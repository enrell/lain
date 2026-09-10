package bench

import (
	"encoding/json"
	"fmt"
	"html/template"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// RunResult is one measured scan execution.
type RunResult struct {
	Run         int     `json:"run"`
	TotalMs     float64 `json:"total_ms"`
	EnumerateMs float64 `json:"enumerate_ms"`
	IdentifyMs  float64 `json:"identify_ms"`
	PersistMs   float64 `json:"persist_ms"`
	PruneMs     float64 `json:"prune_ms"`
	Candidates  int     `json:"candidates"`
	Identified  int     `json:"identified"`
	Dirs        int     `json:"dirs"`
	FilesPerSec float64 `json:"files_per_sec"`
	PeakRSS     uint64  `json:"peak_rss"`
	HeapAlloc   uint64  `json:"heap_alloc"`
}

// Report is the full benchmark document: written as results.json and
// rendered as a self-contained report.html.
type Report struct {
	Tool      string      `json:"tool"`
	Target    string      `json:"target"`
	Path      string      `json:"path"`
	StartedAt time.Time   `json:"started_at"`
	GoVersion string      `json:"go_version"`
	NumCPU    int         `json:"num_cpu"`
	Runs      []RunResult `json:"runs"`
	Note      string      `json:"note"`
}

type stats struct {
	Mean, Min, Max float64
}

func summarize(vals []float64) stats {
	if len(vals) == 0 {
		return stats{}
	}
	s := stats{Min: vals[0], Max: vals[0]}
	for _, v := range vals {
		s.Mean += v
		if v < s.Min {
			s.Min = v
		}
		if v > s.Max {
			s.Max = v
		}
	}
	s.Mean /= float64(len(vals))
	return s
}

func humanBytes(b uint64) string {
	const u = 1024
	if b < u {
		return fmt.Sprintf("%d B", b)
	}
	v := float64(b)
	for _, suf := range []string{"KiB", "MiB", "GiB"} {
		v /= u
		if v < u || suf == "GiB" {
			return fmt.Sprintf("%.1f %s", v, suf)
		}
	}
	return fmt.Sprintf("%.1f GiB", v)
}

// stackedChart renders per-run phase breakdown as inline SVG.
func stackedChart(runs []RunResult) template.HTML {
	const W, rowH, labelW = 760, 34, 90
	maxTotal := 1.0
	for _, r := range runs {
		if r.TotalMs > maxTotal {
			maxTotal = r.TotalMs
		}
	}
	H := len(runs)*rowH + 46
	var sb strings.Builder
	fmt.Fprintf(&sb, `<svg width="%d" height="%d" role="img" aria-label="phase breakdown">`, W, H)
	phases := []struct {
		name  string
		color string
		get   func(RunResult) float64
	}{
		{"enumerate", "#4C7CF0", func(r RunResult) float64 { return r.EnumerateMs }},
		{"identify", "#D9A45B", func(r RunResult) float64 { return r.IdentifyMs }},
		{"persist", "#58B26A", func(r RunResult) float64 { return r.PersistMs }},
		{"prune", "#B058B2", func(r RunResult) float64 { return r.PruneMs }},
	}
	for i, r := range runs {
		y := 8 + i*rowH
		fmt.Fprintf(&sb, `<text x="0" y="%d" font-size="12" class="lbl">run %d</text>`, y+15, r.Run)
		x := float64(labelW)
		barW := float64(W - labelW - 90)
		for _, p := range phases {
			w := p.get(r) / maxTotal * barW
			if w < 0.5 && p.get(r) > 0 {
				w = 0.5
			}
			fmt.Fprintf(&sb, `<rect x="%.1f" y="%d" width="%.1f" height="20" fill="%s"><title>%s %.2f ms</title></rect>`, x, y, w, p.color, p.name, p.get(r))
			x += w
		}
		fmt.Fprintf(&sb, `<text x="%d" y="%d" font-size="12" class="lbl">%.0f ms</text>`, W-84, y+15, r.TotalMs)
	}
	lx := labelW
	for _, p := range phases {
		fmt.Fprintf(&sb, `<rect x="%d" y="%d" width="12" height="12" fill="%s"/><text x="%d" y="%d" font-size="12" class="lbl">%s</text>`, lx, H-24, p.color, lx+16, H-14, p.name)
		lx += 110
	}
	sb.WriteString(`</svg>`)
	return template.HTML(sb.String())
}

// bars renders a simple per-run metric bar chart as inline SVG.
func bars(title string, runs []RunResult, get func(RunResult) float64, unit string) template.HTML {
	const W, rowH, labelW = 760, 30, 90
	maxV := 1.0
	for _, r := range runs {
		if v := get(r); v > maxV {
			maxV = v
		}
	}
	H := len(runs)*rowH + 16
	var sb strings.Builder
	fmt.Fprintf(&sb, `<svg width="%d" height="%d" role="img" aria-label="%s"><text x="0" y="12" font-size="12" class="lbl">%s (%s)</text>`, W, H, title, title, unit)
	for i, r := range runs {
		y := 20 + i*rowH
		w := get(r) / maxV * float64(W-labelW-120)
		fmt.Fprintf(&sb, `<text x="0" y="%d" font-size="12" class="lbl">run %d</text>`, y+14, r.Run)
		fmt.Fprintf(&sb, `<rect x="%d" y="%d" width="%.1f" height="18" fill="#4C7CF0"><title>run %d: %.2f %s</title></rect>`, labelW, y, w, r.Run, get(r), unit)
		fmt.Fprintf(&sb, `<text x="%d" y="%d" font-size="12" class="lbl">%.1f</text>`, W-110, y+14, get(r))
	}
	sb.WriteString(`</svg>`)
	return template.HTML(sb.String())
}

var reportTmpl = template.Must(template.New("report").Funcs(template.FuncMap{"humanBytes": humanBytes}).Parse(`<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<title>lain bench: {{.Report.Target}} — {{.Report.StartedAt.Format "2006-01-02 15:04"}}</title>
<style>
body{font-family:system-ui,sans-serif;max-width:900px;margin:2rem auto;padding:0 1rem;color:#1c1e21;background:#fff}
@media(prefers-color-scheme:dark){body{color:#e4e6eb;background:#18191a}.lbl{fill:#e4e6eb}table{border-color:#3a3b3c}td,th{border-color:#3a3b3c}}
.lbl{fill:#1c1e21}table{border-collapse:collapse;width:100%;margin:1rem 0}td,th{border:1px solid #ccc;padding:.4rem .6rem;text-align:right;font-variant-numeric:tabular-nums}td:first-child,th:first-child{text-align:left}.meta{color:#666;font-size:.9rem}code{background:#f0f0f0;padding:.1rem .3rem;border-radius:3px}@media(prefers-color-scheme:dark){code{background:#333}}</style>
</head><body>
<h1>lain bench: {{.Report.Target}}</h1>
<p class="meta">{{.Report.StartedAt.Format "2006-01-02 15:04:05 MST"}} · {{.Report.GoVersion}} · {{.Report.NumCPU}} cpu · path <code>{{.Report.Path}}</code></p>
<p>{{.Report.Note}}</p>
<h2>Summary ({{len .Report.Runs}} runs)</h2>
<table><tr><th>metric</th><th>mean</th><th>min</th><th>max</th></tr>
<tr><td>total</td><td>{{printf "%.0f ms" .Total.Mean}}</td><td>{{printf "%.0f ms" .Total.Min}}</td><td>{{printf "%.0f ms" .Total.Max}}</td></tr>
<tr><td>enumerate</td><td>{{printf "%.1f ms" .Enumerate.Mean}}</td><td>{{printf "%.1f ms" .Enumerate.Min}}</td><td>{{printf "%.1f ms" .Enumerate.Max}}</td></tr>
<tr><td>identify</td><td>{{printf "%.1f ms" .Identify.Mean}}</td><td>{{printf "%.1f ms" .Identify.Min}}</td><td>{{printf "%.1f ms" .Identify.Max}}</td></tr>
<tr><td>persist</td><td>{{printf "%.1f ms" .Persist.Mean}}</td><td>{{printf "%.1f ms" .Persist.Min}}</td><td>{{printf "%.1f ms" .Persist.Max}}</td></tr>
<tr><td>prune</td><td>{{printf "%.1f ms" .Prune.Mean}}</td><td>{{printf "%.1f ms" .Prune.Min}}</td><td>{{printf "%.1f ms" .Prune.Max}}</td></tr>
<tr><td>throughput</td><td>{{printf "%.0f files/s" .Throughput.Mean}}</td><td>{{printf "%.0f files/s" .Throughput.Min}}</td><td>{{printf "%.0f files/s" .Throughput.Max}}</td></tr>
<tr><td>peak RSS</td><td>{{.PeakRSSMean}}</td><td>{{.PeakRSSMin}}</td><td>{{.PeakRSSMax}}</td></tr>
</table>
<h2>Phase breakdown per run</h2>
{{.Stacked}}
<h2>Throughput per run</h2>
{{.ThroughputChart}}
<h2>Runs</h2>
<table><tr><th>run</th><th>total ms</th><th>enum</th><th>ident</th><th>persist</th><th>prune</th><th>files</th><th>ident</th><th>files/s</th><th>peak RSS</th><th>heap</th></tr>
{{range .Report.Runs}}<tr><td>{{.Run}}</td><td>{{printf "%.0f" .TotalMs}}</td><td>{{printf "%.0f" .EnumerateMs}}</td><td>{{printf "%.0f" .IdentifyMs}}</td><td>{{printf "%.1f" .PersistMs}}</td><td>{{printf "%.1f" .PruneMs}}</td><td>{{.Candidates}}</td><td>{{.Identified}}</td><td>{{printf "%.0f" .FilesPerSec}}</td><td>{{humanBytes .PeakRSS}}</td><td>{{humanBytes .HeapAlloc}}</td></tr>{{end}}
</table>
<p class="meta">Raw data: <code>results.json</code> next to this file. Each run uses a fresh data dir (cold insert path); the media tree is only read, never written.</p>
</body></html>`))

// view is the template data: report plus aggregates and charts.
type view struct {
	Report                              *Report
	Total, Enumerate                    stats
	Identify, Persist                   stats
	Prune, Throughput                   stats
	PeakRSSMean, PeakRSSMin, PeakRSSMax string
	Stacked, ThroughputChart            template.HTML
}

// WriteReport writes results.json and the self-contained report.html
// into dir (created if needed) and returns the html path.
func WriteReport(dir string, rep *Report) (string, error) {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	raw, err := json.MarshalIndent(rep, "", "  ")
	if err != nil {
		return "", err
	}
	if err := os.WriteFile(filepath.Join(dir, "results.json"), append(raw, '\n'), 0o644); err != nil {
		return "", err
	}
	totals := summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.TotalMs }))
	v := view{
		Report: rep, Total: totals,
		Enumerate:  summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.EnumerateMs })),
		Identify:   summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.IdentifyMs })),
		Persist:    summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.PersistMs })),
		Prune:      summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.PruneMs })),
		Throughput: summarize(pluck(rep.Runs, func(r RunResult) float64 { return r.FilesPerSec })),
		Stacked:    stackedChart(rep.Runs),
		ThroughputChart: bars("throughput", rep.Runs,
			func(r RunResult) float64 { return r.FilesPerSec }, "files/s"),
	}
	var peaks []float64
	for _, r := range rep.Runs {
		peaks = append(peaks, float64(r.PeakRSS))
	}
	ps := summarize(peaks)
	v.PeakRSSMean, v.PeakRSSMin, v.PeakRSSMax =
		humanBytes(uint64(ps.Mean)), humanBytes(uint64(ps.Min)), humanBytes(uint64(ps.Max))
	htmlPath := filepath.Join(dir, "report.html")
	f, err := os.Create(htmlPath)
	if err != nil {
		return "", err
	}
	defer f.Close()
	if err := reportTmpl.Execute(f, v); err != nil {
		return "", err
	}
	return htmlPath, nil
}

func pluck(runs []RunResult, f func(RunResult) float64) []float64 {
	out := make([]float64, 0, len(runs))
	for _, r := range runs {
		out = append(out, f(r))
	}
	return out
}
