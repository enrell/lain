// Package downloads moves bytes from an HTTP origin onto local disk:
// a resumable fetcher (this file), a byte budget, safe file naming and
// the server-side download manager. It is not a plugin — it owns
// goroutines and writes media bytes, which plugins never do (D-006,
// D-007) — and it is stdlib-only (D-001). The CLI offline store reuses
// the fetcher and the budget.
package downloads

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"
)

// Error is a typed failure with a stable code, so callers (API, CLI)
// can branch without string matching.
type Error struct {
	Code string
	Msg  string
}

func (e *Error) Error() string { return e.Code + ": " + e.Msg }

// Stable error codes.
const (
	CodeQuota    = "quota-exceeded"
	CodeDiskFull = "disk-full"
	CodeHTTP     = "http-error"
	CodeIO       = "io-error"
	CodeInvalid  = "invalid-request"
	CodeNotFound = "not-found"
	CodeState    = "invalid-state"
)

// CodeOf returns the stable code of err, "" for untyped errors.
func CodeOf(err error) string {
	var e *Error
	if errors.As(err, &e) {
		return e.Code
	}
	return ""
}

// Request describes one transfer. Part is the partial file the bytes
// accumulate in; it survives a pause or crash so the next Fetch
// continues from its size. Validator is the ETag or Last-Modified the
// previous attempt saw: a resume is only trusted when the origin still
// serves the same representation (If-Range), else it restarts from 0
// rather than splicing two different files together.
type Request struct {
	URL       string
	Header    http.Header
	Part      string
	Validator string
	// Reserve is consulted before bytes are written (see Budget); nil
	// means unlimited.
	Reserve func(n int64) error
	// Progress, when set, is called with (done, total) as bytes land;
	// total is -1 while unknown.
	Progress func(done, total int64)
}

// Result reports a finished or interrupted transfer.
type Result struct {
	Bytes     int64  // bytes now in Part
	Total     int64  // expected total, -1 if the origin never said
	Validator string // to pass back on the next resume
	Filename  string // origin-suggested name (Content-Disposition), may be ""
	Restarted bool   // a partial file was discarded
}

// chunk is the write granularity; budget checks happen per chunk.
const chunk = 256 << 10

// Fetch downloads Request.URL into Request.Part, resuming from the
// part's current size. It returns when the body is exhausted, the
// context is canceled (the part is kept for a later resume) or a write
// is refused by the budget. A canceled fetch returns ctx.Err().
func Fetch(ctx context.Context, client *http.Client, in Request) (Result, error) {
	res := Result{Total: -1, Validator: in.Validator}
	if client == nil {
		client = http.DefaultClient
	}
	have := int64(0)
	if fi, err := os.Stat(in.Part); err == nil {
		have = fi.Size()
	} else if !os.IsNotExist(err) {
		return res, &Error{CodeIO, err.Error()}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, in.URL, nil)
	if err != nil {
		return res, &Error{CodeInvalid, err.Error()}
	}
	for k, vs := range in.Header {
		for _, v := range vs {
			req.Header.Add(k, v)
		}
	}
	// Ask for the remainder only when the previous representation can be
	// named; without a validator a 206 could belong to another file.
	if have > 0 && in.Validator != "" {
		req.Header.Set("Range", "bytes="+strconv.FormatInt(have, 10)+"-")
		req.Header.Set("If-Range", in.Validator)
	}
	resp, err := client.Do(req)
	if err != nil {
		if ctx.Err() != nil {
			res.Bytes = have
			return res, ctx.Err()
		}
		return res, &Error{CodeHTTP, err.Error()}
	}
	defer resp.Body.Close()

	flags := os.O_WRONLY | os.O_CREATE
	switch resp.StatusCode {
	case http.StatusPartialContent:
		start, total, ok := parseContentRange(resp.Header.Get("Content-Range"))
		if !ok || start != have {
			return res, &Error{CodeHTTP, "origin answered an unexpected range"}
		}
		res.Total = total
		flags |= os.O_APPEND
	case http.StatusOK:
		res.Restarted = have > 0
		have = 0
		res.Total = resp.ContentLength
		flags |= os.O_TRUNC
	case http.StatusRequestedRangeNotSatisfiable:
		// The part already holds every byte (a crash after the last
		// write but before the job was marked done).
		if _, total, ok := parseContentRange(resp.Header.Get("Content-Range")); ok && total == have {
			res.Bytes, res.Total = have, total
			return res, nil
		}
		return res, &Error{CodeHTTP, "origin refused the resume range"}
	default:
		return res, &Error{CodeHTTP, fmt.Sprintf("origin answered %d", resp.StatusCode)}
	}
	if v := resp.Header.Get("ETag"); v != "" && !strings.HasPrefix(v, "W/") {
		res.Validator = v
	} else if v := resp.Header.Get("Last-Modified"); v != "" {
		res.Validator = v
	} else {
		res.Validator = ""
	}
	res.Filename = dispositionName(resp.Header.Get("Content-Disposition"))

	if in.Reserve != nil && res.Total > 0 {
		// Refuse up front when the whole remainder cannot fit; a size the
		// origin never stated is checked chunk by chunk instead.
		if err := in.Reserve(res.Total - have); err != nil {
			res.Bytes = have
			return res, err
		}
	}
	f, err := os.OpenFile(in.Part, flags, 0o644)
	if err != nil {
		return res, &Error{CodeIO, err.Error()}
	}
	done := have
	report := func() {
		if in.Progress != nil {
			in.Progress(done, res.Total)
		}
	}
	report()
	buf := make([]byte, chunk)
	for {
		n, rerr := resp.Body.Read(buf)
		if n > 0 {
			if in.Reserve != nil && res.Total <= 0 {
				if err := in.Reserve(int64(n)); err != nil {
					f.Close()
					res.Bytes = done
					return res, err
				}
			}
			if _, err := f.Write(buf[:n]); err != nil {
				f.Close()
				res.Bytes = done
				return res, &Error{CodeIO, err.Error()}
			}
			done += int64(n)
			report()
		}
		if rerr == io.EOF {
			break
		}
		if rerr != nil {
			f.Close()
			res.Bytes = done
			if ctx.Err() != nil {
				return res, ctx.Err()
			}
			return res, &Error{CodeHTTP, rerr.Error()}
		}
	}
	if err := f.Close(); err != nil {
		res.Bytes = done
		return res, &Error{CodeIO, err.Error()}
	}
	res.Bytes = done
	if res.Total > 0 && done != res.Total {
		return res, &Error{CodeHTTP, fmt.Sprintf("short body: %d of %d bytes", done, res.Total)}
	}
	if res.Total < 0 {
		res.Total = done
	}
	return res, nil
}

// parseContentRange reads "bytes start-end/total"; total is -1 for "*".
func parseContentRange(v string) (start, total int64, ok bool) {
	v = strings.TrimSpace(v)
	if !strings.HasPrefix(v, "bytes ") {
		return 0, 0, false
	}
	v = strings.TrimPrefix(v, "bytes ")
	rng, tot, found := strings.Cut(v, "/")
	if !found {
		return 0, 0, false
	}
	total = -1
	if tot != "*" {
		t, err := strconv.ParseInt(tot, 10, 64)
		if err != nil || t < 0 {
			return 0, 0, false
		}
		total = t
	}
	if rng == "*" {
		return 0, total, true
	}
	s, _, found := strings.Cut(rng, "-")
	if !found {
		return 0, 0, false
	}
	start, err := strconv.ParseInt(s, 10, 64)
	if err != nil || start < 0 {
		return 0, 0, false
	}
	return start, total, true
}
