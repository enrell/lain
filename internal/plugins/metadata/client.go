// Package metadata holds replaceable metadata providers behind
// lain.metadata.search@1 / resolve@1 (merge-many): local NFO first,
// then Kitsu, AniList and Jikan over plain HTTPS with the standard
// library only. No Rust, no FFI: we consume the same upstream sources
// animedb normalizes, with our own small clients.
//
// A failing provider degrades the merge instead of failing the call;
// locals answer offline; remotes are cached with TTL (see cache.go).
package metadata

import (
	"bytes"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

// politeClient is one HTTP client per provider with a minimum interval
// between requests (upstream rate limits), an identifying User-Agent,
// timeouts, and a single retry honoring Retry-After on 429.
type politeClient struct {
	http      *http.Client
	mu        sync.Mutex
	last      time.Time
	minGap    time.Duration
	userAgent string
}

func newPoliteClient(minGap time.Duration) *politeClient {
	return &politeClient{
		http:      &http.Client{Timeout: 15 * time.Second},
		minGap:    minGap,
		userAgent: "Lain/0.1 (local media server; contact via gateway admin)",
	}
}

func (c *politeClient) waitTurn() {
	c.mu.Lock()
	defer c.mu.Unlock()
	if wait := c.minGap - time.Since(c.last); wait > 0 {
		time.Sleep(wait)
	}
	c.last = time.Now()
}

// get performs one GET with a single 429-aware retry.
func (c *politeClient) get(url string) ([]byte, error) {
	for attempt := 0; attempt < 2; attempt++ {
		c.waitTurn()
		req, err := http.NewRequest("GET", url, nil)
		if err != nil {
			return nil, err
		}
		req.Header.Set("User-Agent", c.userAgent)
		resp, err := c.http.Do(req)
		if err != nil {
			return nil, err
		}
		raw, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if resp.StatusCode == 429 && attempt == 0 {
			if wait := parseRetryAfter(resp.Header.Get("Retry-After")); wait > 0 && wait < 30*time.Second {
				time.Sleep(wait)
				continue
			}
			return nil, fmt.Errorf("rate limited (429)")
		}
		if resp.StatusCode/100 != 2 {
			return nil, fmt.Errorf("GET %s: status %d", url, resp.StatusCode)
		}
		return raw, nil
	}
	return nil, fmt.Errorf("rate limited (429)")
}

// postJSON performs one POST with a JSON body.
func (c *politeClient) postJSON(url string, body []byte) ([]byte, error) {
	c.waitTurn()
	req, err := http.NewRequest("POST", url, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", c.userAgent)
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	if resp.StatusCode/100 != 2 {
		return nil, fmt.Errorf("POST %s: status %d: %.200s", url, resp.StatusCode, raw)
	}
	return raw, nil
}

func parseRetryAfter(h string) time.Duration {
	if h == "" {
		return 0
	}
	var secs int
	if _, err := fmt.Sscanf(h, "%d", &secs); err != nil {
		return 0
	}
	return time.Duration(secs) * time.Second
}
