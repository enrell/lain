# V-5: release-name parsers on the same names

DESIGN-v2 §6 (`V-5`, D-158) asked for one measurement before choosing
lain's default `lain.release.parse@1` provider: lain-parser (the
filename-parser model), zantetsu and lain's in-tree tokenizer, on the same
real names, with one metric. This is that run (2026-11-09). Decision:
D-226.

## What ran

| Parser | What exactly |
|---|---|
| `tokenizer` | `lain-release-tokenizer` (`crates/lain-identify`, the Go tokenizer's port, D-185) |
| `zantetsu` | `zantetsu_core::HeuristicParser` at zantetsu `2bbe08d` (main; the parser its README scores, "production-ready"; the neural CRF ships no weights) |
| `model` | lain-parser's raw record: the `e5-s2` int8 artifact from its Hugging Face release channel (hashes verified against `manifest.json`), served by its own `serve.py` on CPU (torch 2.14.0+cpu), asked over its Unix socket |
| `binding` | what lain answers with the model component installed: `lain-release-model` (the model's labels merged with the tokenizer's tags, Go's `merge`), and the tokenizer whenever the model abstains |

## Sets

Real names only; the authored and synthetic suites of filename-parser
were left out.

- **guessit-holdout** — lain-parser's public holdout of GuessIt's own
  test fixtures (`data/public-guessit-holdout*.jsonl` at filename-parser
  `6336b69`, sha256 `b966343917e63840…`, `7e44197d38e2c577…`), 397
  unique names, labelled with the full `parse_filename` schema (title,
  episode_title, year, season, episodes, release_group, resolution), 14
  of them gold abstentions. lain-parser never trained on it.
- **tracker-unlabeled** — lain-parser's `claim-v1` candidate pool
  (`6cd201077a4830dc…`): 944 real names sampled from a public anime
  tracker, layout-disjoint from the model's training data (a few hundred
  of them fed its dev and final suites). No labels: agreement only.
- **library** — the owner's media library (`V:\.media`). **Not scored in
  this run**: the V: drive detached before the prediction pass, and
  nothing on D: is a media library (9 stray clips). The harness reads it
  when it is mounted (`collect.py --media-root /mnt/v/.media`); it labels
  every video that has an Anything `meta.toml` sidecar from the scraped
  page metadata (title = `series`, else `title`; episodes = `[episode]`),
  with no manual step. A structural profile taken earlier in the session,
  without reading names out: about 620 videos, all downloaded from web
  sites by Anything; none has a bracketed group, an `SxxEyy` marker or a
  resolution tag. They are page titles, not release names, so they test
  how a parser treats a plain title, not release grammar.

Names never left the machine: the sets, predictions and gold live in
`~/.cache/lain-bench/parser-v5/`, and only the aggregates below were
written down.

## Metric

`tools/parser-v5/score.py`, stated precisely in its docstring:

- Values are compared after normalization: titles NFKC + casefold, every
  run of non-alphanumeric characters one space; groups casefold + trim;
  episodes as a set of integers; resolution lower-case; year and season as
  integers. Absent equals absent.
- **exact record**: every schema field equal on a positive row, and on a
  gold abstention the parser emits nothing. This is lain-parser's own
  exact-record metric (its README's 69.9% is this metric on its locked
  `real-final-v1` suite, which is not on this machine). zantetsu's
  README number (92.4%) is an average per-field score on its own set, a
  different metric.
- **identity**: title, season and episodes equal (positive rows).
- **per-field**: share of positive rows whose field equals gold ("all"),
  and the same over the rows where gold states the field ("where
  stated"). A parser that emits nothing gets every stated field wrong.
- **false emission**: a title emitted on a gold abstention.
- **agreement**: rows where at least two of tokenizer, zantetsu and
  binding agree on the normalized title, and each parser's agreement with
  that majority.
- Brackets: 95% Wilson intervals. Latency: wall time per name in the
  runner (the model's includes the socket round trip), on a 20-thread
  WSL2 host.

## Results

### guessit-holdout

397 names: 383 with a title, 14 gold abstentions; scored fields: title, episode_title, year, season, episodes, release_group, resolution.

| metric | tokenizer | zantetsu | model | binding |
|---|---|---|---|---|
| **exact record** | 40.1% (159/397; 35.3–44.9) | 16.4% (65/397; 13.1–20.3) | 46.1% (183/397; 41.3–51.0) | 43.8% (174/397; 39.0–48.7) |
| identity (title+season+episodes) | 62.7% (240/383; 57.7–67.4) | 41.5% (159/383; 36.7–46.5) | 77.8% (298/383; 73.4–81.7) | 76.8% (294/383; 72.3–80.7) |
| title (all / where stated) | 65.8% / 65.8% (252/383; 60.9–70.4) | 49.1% / 49.1% (188/383; 44.1–54.1) | 82.2% / 82.2% (315/383; 78.1–85.7) | 82.5% / 82.5% (316/383; 78.4–86.0) |
| episode_title (all / where stated) | 99.0% / n/a | 98.7% / n/a | 76.0% / n/a | 77.3% / n/a |
| year (all / where stated) | 89.8% / 81.5% (97/119; 73.6–87.5) | 91.1% / 92.4% (110/119; 86.2–96.0) | 94.5% / 97.5% (116/119; 92.8–99.1) | 95.6% / 99.2% (118/119; 95.4–99.9) |
| season (all / where stated) | 89.0% / 74.7% (115/154; 67.3–80.9) | 93.2% / 85.7% (132/154; 79.3–90.4) | 97.1% / 96.8% (149/154; 92.6–98.6) | 97.7% / 96.1% (148/154; 91.8–98.2) |
| episodes (all / where stated) | 82.8% / 68.7% (134/195; 61.9–74.8) | 78.6% / 84.1% (164/195; 78.3–88.6) | 91.1% / 91.3% (178/195; 86.5–94.5) | 90.9% / 91.8% (179/195; 87.1–94.9) |
| release_group (all / where stated) | 70.2% / 60.1% (140/233; 53.7–66.2) | 45.2% / 12.0% (28/233; 8.4–16.8) | 81.7% / 84.1% (196/233; 78.9–88.3) | 83.0% / 86.3% (201/233; 81.3–90.1) |
| resolution (all / where stated) | 87.5% / 84.8% (168/198; 79.2–89.2) | 91.9% / 94.4% (187/198; 90.3–96.9) | 93.5% / 99.5% (197/198; 97.2–99.9) | 93.5% / 98.0% (194/198; 94.9–99.2) |
| abstention on positive rows | 1.0% (4/383; 0.4–2.7) | 1.3% (5/383; 0.6–3.0) | 1.3% (5/383; 0.6–3.0) | 0.0% (0/383; 0.0–1.0) |
| false emission on gold abstentions | 92.9% (13/14; 68.5–98.7) | 100.0% (14/14; 78.5–100.0) | 0.0% (0/14; 0.0–21.5) | 92.9% (13/14; 68.5–98.7) |
| agrees with the 2-of-3 title majority | 82.7% (278/336; 78.3–86.4) | 67.6% (227/336; 62.4–72.3) | 87.5% (294/336; 83.5–90.6) | 92.9% (312/336; 89.6–95.2) |

Majority vote (tokenizer, zantetsu, binding) exists on 84.6% (336/397; 80.8–87.8) of names; where it exists it matches the gold title on 81.2% (273/336; 76.7–85.1).

Latency per name: tokenizer p50 0.01 ms / p95 0.02 ms; zantetsu p50 0.57 ms / p95 0.75 ms; model p50 20.37 ms / p95 35.91 ms; binding p50 20.37 ms / p95 35.91 ms.

Model answers: abstain 19, ok 378.

### tracker-unlabeled

944 names, no labels: agreement only.

| metric | tokenizer | zantetsu | model | binding |
|---|---|---|---|---|
| emits a title | 93.4% (882/944; 91.7–94.8) | 93.3% (881/944; 91.6–94.7) | 88.9% (839/944; 86.7–90.7) | 98.7% (932/944; 97.8–99.3) |
| agrees with the 2-of-3 title majority | 86.1% (717/833; 83.6–88.3) | 74.7% (622/833; 71.6–77.5) | 70.0% (583/833; 66.8–73.0) | 81.2% (676/833; 78.4–83.7) |

Latency per name: tokenizer p50 0.01 ms / p95 0.02 ms; zantetsu p50 0.51 ms / p95 0.67 ms; model p50 18.16 ms / p95 26.88 ms; binding p50 18.16 ms / p95 26.88 ms.

Model answers: abstain 105, ok 839.

## Reading

- **The model wins where there is a label.** On exact record it leads the
  tokenizer by 6 points (46.1% vs 40.1%, overlapping intervals), on
  identity by 15 (77.8% vs 62.7%), on title by 16 and on release group by
  24 (where stated). It is the only parser that abstains correctly: 0 of
  14 false emissions, against 13 or 14 for the rule-based parsers.
- **The binding is what users get**, and it keeps almost all of it:
  76.8% identity, 82.5% title. It loses 2 points of exact record to the
  raw model because the tokenizer answers every name the model abstains
  on (the binding never abstains) and fills the year and season the model
  left out. That is the intended fallback (D-225), not a model error.
- **zantetsu is behind on every compound measure**: 16.4% exact record,
  41.5% identity, 49.1% title, 12.0% release group where stated. Its
  heuristic is better than the tokenizer on year, season, episodes and
  resolution where stated, but the model is better than both on each.
- **Agreement is not accuracy.** On the labelled set the 2-of-3 title
  majority matches gold only 81% of the time; on the unlabelled tracker
  names the tokenizer agrees with the majority most (86%), because the
  binding falls back to it and so votes with it. The unlabelled set
  mostly shows that the model abstains on 11% of real tracker names,
  which the binding then answers with the tokenizer.
- **Cost:** the model is ~20 ms per name (p95 ~36 ms) on CPU, three
  orders of magnitude above the tokenizer; fine for scans and searches,
  and why the binding keeps the tokenizer behind it.

## Recommendation (D-226)

- Default `lain.release.parse@1` binding: `lain-release-model` first,
  `lain-release-tokenizer` second (ordered-many). Without the component
  installed, or with the daemon down, the tokenizer answers alone.
- zantetsu is not kept as a parser provider, so it gets no component
  wrapper; its Kitsu title matcher is outside this contract.
- Re-run the library set when V: is attached, before relying on the
  model for web-sourced libraries.

## Re-running

```sh
# 1. lain-parser: install from its HF channel (packaging/lain-parser/install.sh),
#    or run serve.py on a scratch socket.
# 2. Names and gold, outside the repo:
python3 -I tools/parser-v5/collect.py --media-root /mnt/v/.media
# 3. Predictions (builds zantetsu; not part of the workspace):
cargo build --release --manifest-path tools/parser-v5/Cargo.toml
for s in guessit-holdout tracker-unlabeled library; do
  tools/parser-v5/target/release/parser-v5 --parser unix://$HOME/.local/share/lain/parser.sock \
    < ~/.cache/lain-bench/parser-v5/$s.names.jsonl > ~/.cache/lain-bench/parser-v5/$s.pred.jsonl
done
# 4. Aggregates only:
python3 -I tools/parser-v5/score.py ~/.cache/lain-bench/parser-v5 guessit-holdout tracker-unlabeled library
```
