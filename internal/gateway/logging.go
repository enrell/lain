package gateway

import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"strings"
	"time"
)

// Structured server logging (D-026/D-027): stdlib slog to stdout as
// JSONL shaped like the OpenTelemetry Logs Data Model (Timestamp,
// SeverityText, SeverityNumber, Body, Resource, flat Attributes) —
// machine-first for AI agents, stdlib only, no SDK. Levels via
// --log-level / LAIN_LOG_LEVEL, one access line per request plus
// targeted diagnostics where failures used to vanish silently
// (enrichment fan-out, ffmpeg transforms, scans). No log files, no
// new endpoints in this slice.
//
// What never enters a log line: query strings (media ?token= rides
// there), request/response bodies (login carries passwords), and
// media bytes. Handlers log ids, paths, counts and error causes.

type ctxKey string

// reqIDKey carries the access-log request id through the request
// context. Server-side only until Q-015 decides on the wire header.
const reqIDKey ctxKey = "reqID"

// ParseLogLevel maps debug/info/warn/error (case-insensitive).
// Empty means info so flag/env plumbing stays trivial.
func ParseLogLevel(s string) (slog.Level, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "debug":
		return slog.LevelDebug, nil
	case "info", "":
		return slog.LevelInfo, nil
	case "warn", "warning":
		return slog.LevelWarn, nil
	case "error":
		return slog.LevelError, nil
	default:
		return slog.LevelInfo, fmt.Errorf("unknown log level %q (want debug, info, warn or error)", s)
	}
}

// SetLogger replaces the server logger. Tests inject a buffer; serve
// installs JSON stdout. Nil restores discard so tests stay silent.
func (s *Server) SetLogger(l *slog.Logger) {
	if l == nil {
		l = slog.New(slog.DiscardHandler)
	}
	s.log.Store(l)
	if s.transcode != nil {
		s.transcode.SetLogger(l)
	}
}

func (s *Server) logger() *slog.Logger {
	if l := s.log.Load(); l != nil {
		return l
	}
	return slog.New(slog.DiscardHandler)
}

// otelSeverity maps slog levels to OpenTelemetry SeverityNumbers
// (spec ranges: DEBUG 5-8, INFO 9-12, WARN 13-16, ERROR 17-20).
func otelSeverity(l slog.Level) int {
	switch {
	case l < slog.LevelInfo:
		return 5
	case l < slog.LevelWarn:
		return 9
	case l < slog.LevelError:
		return 13
	default:
		return 17
	}
}

// NewAgentLogger builds the machine-first JSONL logger: one
// OpenTelemetry Logs Data Model-shaped record per line (Timestamp,
// SeverityText, SeverityNumber, Body, Resource, flat Attributes),
// stdlib only, no SDK. Event attributes stay top-level after the
// envelope — exactly the Zap→OTel mapping the spec documents — so
// agents can parse every line with one stable schema. TraceId/SpanId
// are reserved for a future tracing slice; correlation today is the
// server-side req id (Q-015 decides the wire header).
func NewAgentLogger(w io.Writer, level slog.Level, service, version string) *slog.Logger {
	h := slog.NewJSONHandler(w, &slog.HandlerOptions{
		Level: level,
		ReplaceAttr: func(groups []string, a slog.Attr) slog.Attr {
			if len(groups) > 0 {
				return a
			}
			switch a.Key {
			case slog.TimeKey:
				return slog.String("Timestamp", a.Value.Time().UTC().Format(time.RFC3339Nano))
			case slog.LevelKey:
				if l, ok := a.Value.Any().(slog.Level); ok {
					return slog.Any("", slog.GroupValue(
						slog.String("SeverityText", strings.ToUpper(l.String())),
						slog.Int("SeverityNumber", otelSeverity(l)),
					))
				}
				return a
			case slog.MessageKey:
				return slog.String("Body", a.Value.String())
			}
			return a
		},
	})
	return slog.New(h).With(slog.Group("Resource",
		slog.String("service.name", service),
		slog.String("service.version", version),
	))
}

// reqIDOf returns the access-log id for this request, "" outside HTTP.
func reqIDOf(r *http.Request) string {
	if v, _ := r.Context().Value(reqIDKey).(string); v != "" {
		return v
	}
	return ""
}

type statusWriter struct {
	http.ResponseWriter
	status int
}

func (w *statusWriter) WriteHeader(code int) {
	w.status = code
	w.ResponseWriter.WriteHeader(code)
}

// withAccessLog is the single outermost wrapper: one line per request
// with method, path, status and duration. Errors surface here even
// when a handler only returns an HTTP status: 5xx at error, 4xx at
// warn, success at debug. Path only, never the query.
func (s *Server) withAccessLog(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := fmt.Sprintf("req-%d", s.reqSeq.Add(1))
		start := time.Now()
		sw := &statusWriter{ResponseWriter: w, status: 200}
		next.ServeHTTP(sw, r.WithContext(context.WithValue(r.Context(), reqIDKey, id)))
		attrs := []any{"req", id, "method", r.Method, "path", r.URL.Path, "status", sw.status, "dur_ms", time.Since(start).Milliseconds()}
		switch {
		case sw.status >= 500:
			s.logger().Error("request", attrs...)
		case sw.status >= 400:
			s.logger().Warn("request", attrs...)
		default:
			s.logger().Debug("request", attrs...)
		}
	})
}
