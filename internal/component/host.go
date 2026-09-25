package component

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"sync"
	"time"

	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/matrix"
)

// DefaultInvokeTimeout bounds one component call. Long-running
// capabilities (a full-library scan) are poor component candidates in
// this slice; stateless providers finish far inside the bound.
const DefaultInvokeTimeout = 5 * time.Minute

// healthTimeout bounds the liveness op.
const healthTimeout = 3 * time.Second

// handshakeTimeout bounds connect+hello for a fresh child.
const handshakeTimeout = 10 * time.Second

// backoff bounds the respawn ladder after an unexpected exit: it
// doubles from backoffStart to backoffMax, resetting on each clean
// handshake. Restart never stops retrying — an operator who dropped a
// manifest into plugins/ asked for this component — but Health fails
// while it is down, so bindings degrade instead of blocking.
const (
	backoffStart = 500 * time.Millisecond
	backoffMax   = 30 * time.Second
)

// Process is a core.Provider backed by a supervised child process
// speaking the component protocol (D-074). Spawning is eager — Health
// stays a cheap liveness check because the supervisor owns the
// lifecycle in the background.
type Process struct {
	manifest matrix.Manifest
	sockPath string
	argv     []string

	invokeTimeout time.Duration

	ctx    context.Context
	cancel context.CancelFunc

	lis net.Listener

	mu     sync.Mutex // serializes requests; guards conn/ready/cmd
	conn   net.Conn
	br     *bufio.Reader
	bw     *bufio.Writer
	cmd    *exec.Cmd
	ready  bool
	last   string // last lifecycle error, surfaced through Health
	seq    uint64
	closed bool

	wg sync.WaitGroup
}

// Spawn resolves the manifest's entrypoint/args ({sock} and {id}
// placeholders filled) and starts the supervisor: the child is launched
// immediately and respawned with bounded backoff on unexpected exit.
// The returned provider's Health reports handshake/liveness state.
func Spawn(manifest matrix.Manifest, sockDir string) (*Process, error) {
	if manifest.Execution.Kind != "process" {
		return nil, &core.Error{Code: "invalid-message", Msg: "execution kind must be process"}
	}
	argv := make([]string, 0, 1+len(manifest.Execution.Args))
	argv = append(argv, manifest.Execution.Entrypoint)
	sockPath := filepath.Join(sockDir, manifest.ID+".sock")
	for _, a := range manifest.Execution.Args {
		switch a {
		case "{sock}":
			argv = append(argv, sockPath)
		case "{id}":
			argv = append(argv, manifest.ID)
		default:
			argv = append(argv, a)
		}
	}
	if len(argv) == 0 || argv[0] == "" {
		return nil, &core.Error{Code: "invalid-message", Msg: "manifest has no entrypoint"}
	}
	if err := os.MkdirAll(sockDir, 0o755); err != nil {
		return nil, err
	}
	lis, err := net.Listen("unix", sockPath)
	if err != nil {
		return nil, fmt.Errorf("component socket: %w", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	p := &Process{
		manifest:      manifest,
		sockPath:      sockPath,
		argv:          argv,
		invokeTimeout: DefaultInvokeTimeout,
		ctx:           ctx,
		cancel:        cancel,
		lis:           lis,
	}
	p.wg.Add(1)
	go p.supervise()
	return p, nil
}

func (p *Process) ID() string             { return p.manifest.ID }
func (p *Process) Capabilities() []string { return p.manifest.Capabilities }

// Health is cheap: a liveness op round-trip on the established
// connection, or the last lifecycle error when the child is down.
func (p *Process) Health() error {
	p.mu.Lock()
	defer p.mu.Unlock()
	if !p.ready {
		if p.last != "" {
			return &core.Error{Code: "dependency-unavailable", Msg: "component down: " + p.last}
		}
		return &core.Error{Code: "dependency-unavailable", Msg: "component down"}
	}
	if _, err := p.roundTripLocked(healthOp, "", nil, healthTimeout); err != nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "component unhealthy: " + err.Error()}
	}
	return nil
}

// Invoke relays one capability call over the wire and decodes the
// output back to the contract's concrete type.
func (p *Process) Invoke(cap string, input any) (any, error) {
	spec, err := resolveOpForInput(cap, input)
	if err != nil {
		return nil, &core.Error{Code: "invalid-message", Msg: err.Error()}
	}
	var raw json.RawMessage
	if input != nil {
		if raw, err = json.Marshal(input); err != nil {
			return nil, &core.Error{Code: "invalid-message", Msg: err.Error()}
		}
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if !p.ready {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "component down"}
	}
	out, err := p.roundTripLocked(spec.op, cap, raw, p.invokeTimeout)
	if err != nil {
		return nil, err
	}
	if spec.spec.Out == nil {
		return nil, nil
	}
	dst := spec.spec.Out()
	if err := json.Unmarshal(out, dst); err != nil {
		return nil, &core.Error{Code: "internal", Msg: "bad component output: " + err.Error()}
	}
	return deref(dst), nil
}

// Close stops the supervisor, kills the child and removes the socket.
func (p *Process) Close() error {
	p.cancel()
	p.mu.Lock()
	p.closed = true
	p.mu.Unlock()
	_ = p.lis.Close()
	p.killLocked()
	p.wg.Wait()
	_ = os.Remove(p.sockPath)
	return nil
}

// supervise owns the child lifecycle: spawn, handshake, wait, respawn.
func (p *Process) supervise() {
	defer p.wg.Done()
	backoff := backoffStart
	for {
		if p.ctx.Err() != nil {
			return
		}
		cmd := exec.CommandContext(p.ctx, p.argv[0], p.argv[1:]...)
		if err := cmd.Start(); err != nil {
			p.failLocked(err.Error())
			if !p.sleep(backoff) {
				return
			}
			backoff = nextBackoff(backoff)
			continue
		}
		p.setCmd(cmd)
		ul, ok := p.lis.(*net.UnixListener)
		if ok {
			_ = ul.SetDeadline(time.Now().Add(handshakeTimeout))
		}
		conn, err := p.lis.Accept()
		if err != nil {
			p.dropCmd(cmd)
			if p.ctx.Err() != nil {
				return
			}
			p.failLocked("child never connected")
			if !p.sleep(backoff) {
				return
			}
			backoff = nextBackoff(backoff)
			continue
		}
		_ = conn.SetDeadline(time.Now().Add(handshakeTimeout))
		br, bw := bufio.NewReader(conn), bufio.NewWriter(conn)
		var hello Hello
		if err := readFrame(br, &hello); err != nil || !p.checkHello(&hello) {
			p.failLocked("handshake failed")
			conn.Close()
			p.dropCmd(cmd)
			if !p.sleep(backoff) {
				return
			}
			backoff = nextBackoff(backoff)
			continue
		}
		_ = conn.SetDeadline(time.Time{})
		backoff = backoffStart
		p.setConn(conn, br, bw)
		_ = cmd.Wait()
		p.dropConn()
		if !p.sleep(backoff) {
			return
		}
		backoff = nextBackoff(backoff)
	}
}

func (p *Process) checkHello(h *Hello) bool {
	if h.Op != "hello" || h.Protocol != ProtocolVersion || h.Provider != p.manifest.ID {
		return false
	}
	// The component must serve every capability its manifest declares.
	have := map[string]bool{}
	for _, c := range h.Capabilities {
		have[c] = true
	}
	for _, c := range p.manifest.Capabilities {
		if !have[c] {
			return false
		}
	}
	return true
}

func (p *Process) sleep(d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-p.ctx.Done():
		return false
	case <-t.C:
		return true
	}
}

func nextBackoff(d time.Duration) time.Duration {
	d *= 2
	if d > backoffMax {
		return backoffMax
	}
	return d
}

// roundTripLocked writes one request and reads its response; the caller
// holds mu. Any transport failure drops the connection and kills the
// child, so the supervisor respawns a clean peer instead of risking a
// desynchronized stream.
func (p *Process) roundTripLocked(op, cap string, input json.RawMessage, timeout time.Duration) (json.RawMessage, error) {
	p.seq++
	req := request{Seq: p.seq, Cap: cap, Op: op, Input: input}
	if err := p.conn.SetDeadline(time.Now().Add(timeout)); err != nil {
		p.resetLocked()
		return nil, err
	}
	if err := writeFrame(p.bw, req); err != nil {
		p.resetLocked()
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "component write: " + err.Error()}
	}
	var resp response
	if err := readFrame(p.br, &resp); err != nil {
		p.resetLocked()
		if isTimeout(err) {
			return nil, &core.Error{Code: "outcome-unknown", Msg: "component call timed out; it may or may not have run"}
		}
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "component read: " + err.Error()}
	}
	_ = p.conn.SetDeadline(time.Time{})
	if resp.Seq != req.Seq {
		p.resetLocked()
		return nil, &core.Error{Code: "internal", Msg: "component replied out of order"}
	}
	if resp.Error != nil {
		return nil, &core.Error{Code: resp.Error.Code, Msg: resp.Error.Msg}
	}
	return resp.Output, nil
}

// resetLocked drops the broken connection and kills the child; the
// supervisor respawns it on the backoff ladder.
func (p *Process) resetLocked() {
	if p.conn != nil {
		_ = p.conn.Close()
		p.conn = nil
	}
	p.ready = false
	p.killLocked()
	p.last = "connection reset"
}

func (p *Process) killLocked() {
	if p.cmd != nil && p.cmd.Process != nil {
		_ = p.cmd.Process.Kill()
	}
}

func (p *Process) setCmd(cmd *exec.Cmd) {
	p.mu.Lock()
	p.cmd = cmd
	p.mu.Unlock()
}

func (p *Process) dropCmd(cmd *exec.Cmd) {
	p.mu.Lock()
	if p.cmd == cmd {
		p.cmd = nil
	}
	p.ready = false
	p.mu.Unlock()
	_ = cmd.Process.Kill()
	_ = cmd.Wait()
}

func (p *Process) setConn(conn net.Conn, br *bufio.Reader, bw *bufio.Writer) {
	p.mu.Lock()
	p.conn, p.br, p.bw = conn, br, bw
	p.ready = true
	p.last = ""
	p.mu.Unlock()
}

func (p *Process) dropConn() {
	p.mu.Lock()
	if p.conn != nil {
		_ = p.conn.Close()
		p.conn = nil
	}
	p.ready = false
	if !p.closed {
		p.last = "component exited"
	}
	p.cmd = nil
	p.mu.Unlock()
}

func (p *Process) failLocked(msg string) {
	p.mu.Lock()
	p.ready = false
	p.last = msg
	p.mu.Unlock()
}

func isTimeout(err error) bool {
	var ne net.Error
	return errors.As(err, &ne) && ne.Timeout()
}

// resolved names one (cap, input) pair's wire op and table entry.
type resolved struct {
	op   string
	spec opSpec
}

func resolveOpForInput(cap string, input any) (resolved, error) {
	op, err := opForInput(cap, input)
	if err != nil {
		return resolved{}, err
	}
	s, err := resolveOp(cap, op)
	if err != nil {
		return resolved{}, err
	}
	return resolved{op: op, spec: s}, nil
}

// deref unwraps the pointer an Out factory returned after decoding.
func deref(v any) any {
	rv := reflect.ValueOf(v)
	if rv.Kind() == reflect.Ptr && !rv.IsNil() {
		return rv.Elem().Interface()
	}
	return v
}
