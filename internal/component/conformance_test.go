package component

// Replays the language-neutral NDJSON fixtures in conformance/component
// against Serve: the Go component side is the spec the Rust host and
// Rust components are checked against (D-171).

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/identify"
)

type fixtureLine struct {
	From  string          `json:"from"`
	Note  string          `json:"note"`
	Frame json.RawMessage `json:"frame"`
}

// fixtureProviders maps a fixture's hello provider id to the built-in
// that serves it.
var fixtureProviders = map[string]core.Provider{
	"lain-identify-generic": identify.Generic{},
}

// fixtureMatch compares a received frame with the fixture: "$any"
// matches anything, "$string" any string, the rest must be equal.
func fixtureMatch(want, got any) error {
	if s, ok := want.(string); ok {
		switch s {
		case "$any":
			return nil
		case "$string":
			if _, ok := got.(string); ok {
				return nil
			}
			return fmt.Errorf("want a string, got %v", got)
		}
	}
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			return fmt.Errorf("want object, got %v", got)
		}
		if len(w) != len(g) {
			return fmt.Errorf("keys differ: want %v, got %v", w, g)
		}
		for k, wv := range w {
			gv, ok := g[k]
			if !ok {
				return fmt.Errorf("missing key %q", k)
			}
			if err := fixtureMatch(wv, gv); err != nil {
				return fmt.Errorf("%s: %w", k, err)
			}
		}
		return nil
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return fmt.Errorf("want %v, got %v", w, got)
		}
		for i := range w {
			if err := fixtureMatch(w[i], g[i]); err != nil {
				return fmt.Errorf("[%d]: %w", i, err)
			}
		}
		return nil
	}
	if !reflect.DeepEqual(want, got) {
		return fmt.Errorf("want %v, got %v", want, got)
	}
	return nil
}

func TestConformanceFixtures(t *testing.T) {
	files, err := filepath.Glob(filepath.Join("..", "..", "conformance", "component", "*.ndjson"))
	if err != nil || len(files) == 0 {
		t.Fatalf("no fixtures found: %v", err)
	}
	for _, file := range files {
		t.Run(filepath.Base(file), func(t *testing.T) {
			raw, err := os.ReadFile(file)
			if err != nil {
				t.Fatal(err)
			}
			var lines []fixtureLine
			sc := bufio.NewScanner(bytes.NewReader(raw))
			sc.Buffer(make([]byte, 0, 1<<20), MaxFrame)
			for sc.Scan() {
				if len(sc.Bytes()) == 0 {
					continue
				}
				var l fixtureLine
				if err := json.Unmarshal(sc.Bytes(), &l); err != nil {
					t.Fatalf("bad fixture line %q: %v", sc.Text(), err)
				}
				lines = append(lines, l)
			}
			var hello Hello
			if err := json.Unmarshal(lines[0].Frame, &hello); err != nil || hello.Op != "hello" {
				t.Fatalf("first frame must be hello: %s", lines[0].Frame)
			}
			p, ok := fixtureProviders[hello.Provider]
			if !ok {
				t.Fatalf("no Go provider for %s", hello.Provider)
			}
			sock := filepath.Join(t.TempDir(), "c.sock")
			lis, err := net.Listen("unix", sock)
			if err != nil {
				t.Fatal(err)
			}
			defer lis.Close()
			done := make(chan error, 1)
			go func() { done <- Serve(sock, p) }()
			conn, err := lis.Accept()
			if err != nil {
				t.Fatal(err)
			}
			_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
			br, bw := bufio.NewReader(conn), bufio.NewWriter(conn)
			for i, l := range lines {
				switch l.From {
				case "host":
					if _, err := bw.Write(append([]byte(l.Frame), '\n')); err != nil {
						t.Fatal(err)
					}
					if err := bw.Flush(); err != nil {
						t.Fatal(err)
					}
				case "component":
					var got, want any
					if err := readFrame(br, &got); err != nil {
						t.Fatalf("line %d: read: %v", i+1, err)
					}
					if err := json.Unmarshal(l.Frame, &want); err != nil {
						t.Fatal(err)
					}
					if err := fixtureMatch(want, got); err != nil {
						t.Fatalf("line %d (%s): %v", i+1, l.Note, err)
					}
				default:
					t.Fatalf("line %d: from must be host or component", i+1)
				}
			}
			conn.Close()
			if err := <-done; err != nil {
				t.Fatalf("Serve: %v", err)
			}
		})
	}
}
