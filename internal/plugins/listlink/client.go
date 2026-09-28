// Package listlink holds external list-platform connectors behind
// lain.listlink@1 (ordered-many): each provider serves the platforms it
// knows and declines the rest with the typed code
// "unsupported-platform" — the identify pattern applied to links
// (D-078..D-081). AniList is the first connector: OAuth2
// authorization-code grant for linking (no PKCE exists in AniList's
// flow) and GraphQL MediaListCollection for imports.
package listlink

import (
	"bytes"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

// politeClient is one HTTP client per connector with a minimum interval
// between requests (upstream rate limits), an identifying User-Agent,
// timeouts, and a single retry honoring Retry-After on 429. Same
// discipline as the metadata package's client, with a bearer slot.
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

// postJSON performs one POST with a JSON body; bearer is attached when
// non-empty. A 429 gets a single Retry-After-aware retry.
func (c *politeClient) postJSON(url string, body []byte, bearer string) ([]byte, int, error) {
	for attempt := 0; attempt < 2; attempt++ {
		c.waitTurn()
		req, err := http.NewRequest("POST", url, bytes.NewReader(body))
		if err != nil {
			return nil, 0, err
		}
		req.Header.Set("User-Agent", c.userAgent)
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "application/json")
		if bearer != "" {
			req.Header.Set("Authorization", "Bearer "+bearer)
		}
		resp, err := c.http.Do(req)
		if err != nil {
			return nil, 0, err
		}
		raw, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if resp.StatusCode == 429 && attempt == 0 {
			if wait := parseRetryAfter(resp.Header.Get("Retry-After")); wait > 0 && wait < 30*time.Second {
				time.Sleep(wait)
				continue
			}
			return nil, resp.StatusCode, fmt.Errorf("rate limited (429)")
		}
		return raw, resp.StatusCode, nil
	}
	return nil, 429, fmt.Errorf("rate limited (429)")
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
