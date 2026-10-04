// Package release serves lain.release.parse@1 (docs/slices/
// acquisition.md, A-6): the lain-parser model over its unix socket,
// with the hand-written tokenizer as fallback and as the source of
// technical tags the model does not label.
package release

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	TokenizerID = "lain-release-tokenizer"
	ModelID     = "lain-release-model"
	maxName     = 1024
)

func input(cap string, in any) (contracts.ReleaseParseInput, error) {
	if cap != contracts.CapReleaseParse {
		return contracts.ReleaseParseInput{}, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	ri, ok := in.(contracts.ReleaseParseInput)
	if !ok {
		return ri, &core.Error{Code: "invalid-message", Msg: "ReleaseParseInput required"}
	}
	if strings.TrimSpace(ri.Name) == "" || len(ri.Name) > maxName || !utf8.ValidString(ri.Name) {
		return ri, &core.Error{Code: "invalid-message", Msg: "name must be non-empty text up to 1024 bytes"}
	}
	return ri, nil
}

// Tokenizer is the always-available fallback.
type Tokenizer struct{}

func (Tokenizer) ID() string             { return TokenizerID }
func (Tokenizer) Capabilities() []string { return []string{contracts.CapReleaseParse} }
func (Tokenizer) Health() error          { return nil }

func (Tokenizer) Invoke(cap string, in any) (any, error) {
	ri, err := input(cap, in)
	if err != nil {
		return nil, err
	}
	return Tokenize(ri.Name, ri.Kind), nil
}

// Model asks the local lain-parser service. It declines (zero Release)
// when the model abstains and errors with dependency-unavailable when
// the service is not reachable, so CallFirst falls through either way.
type Model struct {
	Socket string
	client *http.Client
}

// NewModel builds a client for the parser socket path.
func NewModel(socket string) *Model {
	tr := &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			var d net.Dialer
			return d.DialContext(ctx, "unix", socket)
		},
		MaxIdleConns: 2, IdleConnTimeout: 30 * time.Second,
	}
	return &Model{Socket: socket, client: &http.Client{Transport: tr, Timeout: 3 * time.Second}}
}

func (*Model) ID() string             { return ModelID }
func (*Model) Capabilities() []string { return []string{contracts.CapReleaseParse} }

// Health checks the socket and the service's /health.
func (m *Model) Health() error {
	if _, err := os.Stat(m.Socket); err != nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "parser socket not found"}
	}
	res, err := m.client.Get("http://lain-parser/health")
	if err != nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "parser not answering"}
	}
	res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return &core.Error{Code: "dependency-unavailable", Msg: "parser unhealthy"}
	}
	return nil
}

type modelRecord struct {
	Title        string `json:"title"`
	EpisodeTitle string `json:"episode_title"`
	Year         int    `json:"year"`
	Season       *int   `json:"season"`
	Episodes     []int  `json:"episodes"`
	Group        string `json:"release_group"`
	Resolution   string `json:"resolution"`
}

func (m *Model) Invoke(cap string, in any) (any, error) {
	ri, err := input(cap, in)
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(m.Socket); errors.Is(err, os.ErrNotExist) {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "parser socket not found"}
	}
	body, _ := json.Marshal(map[string]string{"filename": ri.Name})
	res, err := m.client.Post("http://lain-parser/parse", "application/json", bytes.NewReader(body))
	if err != nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "parser not answering"}
	}
	defer res.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(res.Body, 256<<10))
	if err != nil || res.StatusCode != http.StatusOK {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "parser error"}
	}
	var out struct {
		Record *modelRecord `json:"record"`
	}
	if err := json.Unmarshal(raw, &out); err != nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "parser sent bad json"}
	}
	if out.Record == nil || strings.TrimSpace(out.Record.Title) == "" {
		return contracts.Release{}, nil // abstained
	}
	return merge(*out.Record, Tokenize(ri.Name, ri.Kind)), nil
}

// merge trusts the model for what it labels (title, numbers, group,
// resolution) and the tokenizer for the tags the model does not label.
func merge(rec modelRecord, tok contracts.Release) contracts.Release {
	r := tok
	r.Parser = ModelID
	r.Title = cleanTitle(rec.Title)
	r.EpisodeTitle = cleanTitle(rec.EpisodeTitle)
	if rec.Year > 0 {
		r.Year = rec.Year
	}
	if rec.Season != nil {
		r.Season = *rec.Season
	}
	if len(rec.Episodes) > 0 {
		if len(rec.Episodes) > 2000 {
			rec.Episodes = rec.Episodes[:2000]
		}
		r.Episodes = rec.Episodes
		r.SeasonPack = false
		r.Absolute = rec.Season == nil
	}
	if rec.Group != "" {
		r.Group = cleanTitle(rec.Group)
	}
	if v, ok := resolutions[strings.ToLower(rec.Resolution)]; ok {
		r.Resolution = v
	}
	return r
}
