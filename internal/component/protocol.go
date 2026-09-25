// Package component is the component-mode transport (D-074): a
// core.Provider backed by a supervised child process speaking the lain
// component protocol over a unix socket, plus the server side used by
// `lain plugin-run`. Payloads are bounded JSON only — media bytes never
// cross the boundary (D-007).
//
// Wire shape, newline-delimited JSON frames (no literal newlines appear
// inside a marshaled frame):
//
//	component -> host: {"op":"hello","provider":"<id>","protocol":1,"capabilities":[...]}
//	host -> component: {"seq":N,"cap":"lain.x@1","op":"get","input":{...}}
//	                  {"seq":N,"op":"health","input":null}
//	component -> host: {"seq":N,"output":{...}}
//	                  {"seq":N,"error":{"code":"not-found","msg":"..."}}
//
// `op` selects the input shape inside a multi-input capability
// (documented per capability in docs/CONTRACTS.md); single-input
// capabilities may omit it. Error codes are the stable core.Error
// codes; a transport timeout surfaces as `outcome-unknown` because the
// call may or may not have executed.
package component

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
)

// ProtocolVersion is the only wire version this build speaks.
const ProtocolVersion = 1

// MaxFrame bounds one NDJSON frame. Payloads are small JSON contract
// shapes; anything larger is a peer bug or an attack, not a request.
const MaxFrame = 8 << 20

// Hello is the component's first frame: identity plus what it serves.
// The host validates it against the manifest before serving.
type Hello struct {
	Op           string   `json:"op"` // "hello"
	Provider     string   `json:"provider"`
	Protocol     int      `json:"protocol"`
	Capabilities []string `json:"capabilities"`
}

// request is one invocation (or the transport-level "health" op).
type request struct {
	Seq   uint64          `json:"seq"`
	Cap   string          `json:"cap,omitempty"`
	Op    string          `json:"op,omitempty"`
	Input json.RawMessage `json:"input"`
}

// response carries the invoke output or a typed error.
type response struct {
	Seq    uint64          `json:"seq"`
	Output json.RawMessage `json:"output,omitempty"`
	Error  *wireError      `json:"error,omitempty"`
}

type wireError struct {
	Code string `json:"code"`
	Msg  string `json:"msg"`
}

// healthOp is the transport-level liveness op; it is not a capability
// and never reaches a provider's Invoke.
const healthOp = "health"

// writeFrame emits one NDJSON frame. Marshal never produces literal
// newlines inside a JSON document, so '\n' is a safe delimiter.
func writeFrame(w *bufio.Writer, v any) error {
	raw, err := json.Marshal(v)
	if err != nil {
		return err
	}
	if _, err := w.Write(raw); err != nil {
		return err
	}
	if err := w.WriteByte('\n'); err != nil {
		return err
	}
	return w.Flush()
}

// readFrame reads one bounded NDJSON frame.
func readFrame(r *bufio.Reader, dst any) error {
	var buf []byte
	for {
		frag, err := r.ReadSlice('\n')
		buf = append(buf, frag...)
		if len(buf) > MaxFrame {
			return fmt.Errorf("frame exceeds %d bytes", MaxFrame)
		}
		if err == bufio.ErrBufferFull {
			continue
		}
		if err != nil {
			return err
		}
		break
	}
	if len(buf) == 0 {
		return io.EOF
	}
	return json.Unmarshal(buf, dst)
}
