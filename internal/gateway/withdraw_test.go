package gateway

import (
	"encoding/json"
	"testing"
)

func compositionProviders(t *testing.T, srv *Server, admin, capability string) []string {
	t.Helper()
	rec := do(t, srv, "GET", "/api/plugins", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("plugins: %d", rec.Code)
	}
	var body struct {
		Composition []struct {
			Capability string   `json:"capability"`
			Providers  []string `json:"providers"`
		} `json:"composition"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	for _, b := range body.Composition {
		if b.Capability == capability {
			return b.Providers
		}
	}
	t.Fatalf("capability %s not in composition", capability)
	return nil
}

func TestWithdrawEndpoint(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	rec := do(t, srv, "POST", "/api/plugins/withdraw", map[string]any{"provider": "ghost"}, admin)
	if rec.Code != 400 {
		t.Fatalf("unknown provider withdraw: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "POST", "/api/plugins/withdraw", map[string]any{}, admin)
	if rec.Code != 400 {
		t.Fatalf("empty provider: %d", rec.Code)
	}

	// ordered-many: withdrawing one identifier falls back to the rest.
	rec = do(t, srv, "POST", "/api/plugins/withdraw", map[string]any{"provider": "lain-identify-anime"}, admin)
	if rec.Code != 200 {
		t.Fatalf("withdraw: %d %s", rec.Code, rec.Body.String())
	}
	got := compositionProviders(t, srv, admin, "lain.media.identify@1")
	if len(got) != 2 || got[0] != "lain-identify-comic" || got[1] != "lain-identify-generic" {
		t.Fatalf("identify binding after withdraw: %v", got)
	}

	// exactly-one with no alternative fails closed: the binding keeps
	// the provider (degraded) rather than serving nothing silently.
	rec = do(t, srv, "POST", "/api/plugins/withdraw", map[string]any{"provider": "lain-catalog-bolt"}, admin)
	if rec.Code != 200 {
		t.Fatalf("withdraw exactly-one: %d %s", rec.Code, rec.Body.String())
	}
	got = compositionProviders(t, srv, admin, "lain.catalog.read@1")
	if len(got) != 1 || got[0] != "lain-catalog-bolt" {
		t.Fatalf("catalog binding must stay degraded-bound, got %v", got)
	}

	// The withdraw is persisted: composition.json on disk matches.
	var saved struct {
		Bindings map[string]struct {
			Providers []string `json:"providers"`
		} `json:"bindings"`
	}
	if err := srv.st.Load("composition.json", &saved); err != nil {
		t.Fatalf("composition.json: %v", err)
	}
	got = saved.Bindings["lain.media.identify@1"].Providers
	if len(got) != 2 || got[0] != "lain-identify-comic" || got[1] != "lain-identify-generic" {
		t.Fatalf("persisted identify binding: %v", got)
	}
}
