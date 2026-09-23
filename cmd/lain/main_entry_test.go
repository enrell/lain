package main


import (
	"bytes"
	"errors"
	"io"
	"log/slog"
	"os"
	"strings"
	"testing"
)

func TestMainVersion(t *testing.T) {
	originalArgs, originalStdout := os.Args, os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() { os.Args, os.Stdout = originalArgs, originalStdout }()
	os.Args = []string{"lain", "version"}
	os.Stdout = w
	main()
	w.Close()
	output, err := io.ReadAll(r)
	r.Close()
	if err != nil || !strings.Contains(string(output), "lain "+version) {
		t.Fatalf("version output=%q err=%v", output, err)
	}
}

func TestPrintCommandError(t *testing.T) {
	var output bytes.Buffer
	printCommandError(&output, errors.New("failed"))
	if output.String() != "error: failed\n" {
		t.Fatalf("output=%q", output.String())
	}
}

func TestStartLibraryWatcherReportsFailure(t *testing.T) {
	var output bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&output, nil))
	called := false
	startLibraryWatcher(func() error {
		called = true
		return errors.New("watch unavailable")
	}, logger)
	if !called || !strings.Contains(output.String(), "library watcher disabled") || !strings.Contains(output.String(), "watch unavailable") {
		t.Fatalf("called=%v log=%q", called, output.String())
	}
	output.Reset()
	startLibraryWatcher(func() error { return nil }, logger)
	if output.Len() != 0 {
		t.Fatalf("healthy watcher logged a warning: %q", output.String())
	}
}

func TestCmdServeReportsStartupBeforeListenFailure(t *testing.T) {
	original := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stdout = w
	defer func() { os.Stdout = original; w.Close(); r.Close() }()
	err = cmdServe([]string{"--data-dir", t.TempDir(), "--port", "not-a-port", "--watch", "0"})
	w.Close()
	output, readErr := io.ReadAll(r)
	if err == nil || readErr != nil || !strings.Contains(string(output), "log level: info") {
		t.Fatalf("listen err=%v read err=%v output=%q", err, readErr, output)
	}
}
