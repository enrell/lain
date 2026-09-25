package component

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"

	"github.com/enrell/lain/internal/core"
)

// Serve runs one provider as a component: it dials the host socket,
// announces itself with the hello handshake, then answers request
// frames until the host hangs up. Used by `lain plugin-run`; third-party
// components implement the same frames directly.
func Serve(sockPath string, p core.Provider) error {
	conn, err := net.Dial("unix", sockPath)
	if err != nil {
		return err
	}
	defer conn.Close()
	br, bw := bufio.NewReader(conn), bufio.NewWriter(conn)
	if err := writeFrame(bw, Hello{
		Op: "hello", Provider: p.ID(), Protocol: ProtocolVersion,
		Capabilities: p.Capabilities(),
	}); err != nil {
		return fmt.Errorf("hello: %w", err)
	}
	for {
		var req request
		if err := readFrame(br, &req); err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}
		resp := answer(p, req)
		if err := writeFrame(bw, resp); err != nil {
			return err
		}
	}
}

// answer maps one request frame to a response; a panicking Invoke is
// reported as an internal error, never a dead connection.
func answer(p core.Provider, req request) (resp response) {
	resp.Seq = req.Seq
	defer func() {
		if r := recover(); r != nil {
			resp.Output = nil
			resp.Error = &wireError{Code: "internal", Msg: fmt.Sprintf("panic: %v", r)}
		}
	}()
	// The health op maps the provider's own Health contract.
	if req.Cap == "" && req.Op == healthOp {
		if err := p.Health(); err != nil {
			resp.Error = wireOf(err)
			return resp
		}
		resp.Output = json.RawMessage(`{"ok":true}`)
		return resp
	}
	if req.Cap == "" {
		resp.Error = &wireError{Code: "invalid-message", Msg: "cap required"}
		return resp
	}
	spec, err := resolveOp(req.Cap, req.Op)
	if err != nil {
		resp.Error = &wireError{Code: "invalid-message", Msg: err.Error()}
		return resp
	}
	var in any
	if spec.In != nil {
		in = spec.In()
		if err := json.Unmarshal(req.Input, in); err != nil {
			resp.Error = &wireError{Code: "invalid-message", Msg: "bad input: " + err.Error()}
			return resp
		}
		in = deref(in)
	}
	out, err := p.Invoke(req.Cap, in)
	if err != nil {
		resp.Error = wireOf(err)
		return resp
	}
	raw, err := json.Marshal(out)
	if err != nil {
		resp.Error = &wireError{Code: "internal", Msg: "output marshal: " + err.Error()}
		return resp
	}
	resp.Output = raw
	return resp
}

// wireOf preserves a core.Error's stable code; anything else is a
// plain internal failure — invented codes are not allowed on the wire.
func wireOf(err error) *wireError {
	if ce, ok := err.(*core.Error); ok {
		return &wireError{Code: ce.Code, Msg: ce.Msg}
	}
	return &wireError{Code: "internal", Msg: err.Error()}
}
