# Agent QA fleet — specialists that drive the real WebUI

Five specialist agents open a real Chromium against a running Lain, walk
journeys only they would think of, and file schema-validated English reports.
One more agent — the remediation engineer — patches what they found, and the
specialist that filed a finding re-tests it in its own browser session until
both agree or the round budget runs out (D-034, D-035).

This is a *review aid*, not a CI gate: the deterministic contract stays
`web/e2e/smoke.mjs`, and a human reads the branch before anything is merged.

## Recipes

| Recipe | What it does |
|---|---|
| `just agent-sync` | Install `qa/agents/*.md` into the ignored `.opencode/agents/` discovery dir and re-probe the browser backend |
| `just agent-doctor` | Preflight: model, OpenCode CLI, chromium, agent definitions, backend |
| `just agent-e2e [seeds]` | Boot a disposable instance, run every specialist, write reports |
| `just agent-fix [run]` | Fix → verify loop over the blocking findings of a run |
| `just agent-status [run]` | Print the convergence ledger: what closed, what is parked |
| `just agent-test` | Fleet self-tests (`node --test qa/lib/*.test.mjs`), no model, no browser |
| `just agent-clean <run>` (or `--all`) | Delete local run artifacts |

`just agent-e2e a11y` or `just agent-e2e a11y,ui` narrows the roster.
`just agent-sync` must run before the first use: OpenCode only discovers agents
flat in `.opencode/agents/`, and that directory is ignored on purpose (D-036).

## Roster

| Agent | Specialisation | Edits code |
|---|---|---|
| `lain-qa-usability` | task completion, wording, feedback, dead ends, recovery | no |
| `lain-qa-ui` | rendered pixels: grid, rhythm, hierarchy, states, theme tokens | no |
| `lain-qa-a11y` | WCAG 2.2 AA: keyboard, focus, names/roles, contrast, target size | no |
| `lain-qa-responsive` | viewport × input modes: reflow, overflow, reachability, stability | no |
| `lain-qa-reliability` | console/network errors, races, stale state, deep links, auth edges | no |
| `lain-qa-fixer` | reproduces, fixes, keeps `just check` green, commits to the review branch | yes |
| `lain-qa-probe` | reports which browser backend this session actually has | no |

Each specialist is a `primary` session with its own system prompt, a step
budget, and explicit `permissions`: read-only repo access, no `.env`, no
`git commit`, no subagents, no questions (nobody is watching), and writes only
under `*qa/runs/*`. Judgement lives in the agent file; every operational fact
(target, credentials, verbs, output paths, JSON shape) is injected by the
generated brief in `qa/prompts/`, so changing the harness never rewrites a
specialisation prompt (D-036).

## Target policy

By default the fleet builds `just web` when the embedded bundle is stale,
compiles its own binary, generates synthetic media with
`web/e2e/fixtures.sh`, boots a throwaway instance on a free port with its own
bbolt directory, seeds two libraries plus a member account, and tears it down
unless `LAIN_AGENT_KEEP_INSTANCE=1`. Nothing in this path can touch a real
library (D-035).

`LAIN_AGENT_TARGET_URL` points the fleet at an instance you already run. That
is opt-in and loud: the runner prints what it is about to mutate, waits 10s,
and refuses to start without `LAIN_AGENT_ADMIN_PASSWORD`, because an external
target is not seeded and a specialist that cannot log in would audit the sign-in
page and report nothing.

## Browser hands

Tooling policy: specialists drive **Chromium over the DevTools Protocol**
through `qa/browser.mjs` (Node built-ins only, same approach as
`web/e2e/smoke.mjs`). OpenCode's integrated `browser.*` tools are used only
when a desktop client is attached; headless sessions do not have them, which
`just agent-doctor` proves by asking `lain-qa-probe` (D-034). Set
`LAIN_AGENT_BROWSER=cdp` to skip the probe, or `integrated` to demand the
opposite. The choice adds no dependency and stays reversible through one env
key; if "the integrated browser specifically" was the requirement, that is a
product-direction call and belongs to the user, not to the harness.

Verbs: `start status stop open goto reload snapshot click fill select check
press scroll wait shot console resources contrast audit eval viewport
localset cookie`. Every call takes `--state <dir>`, so each specialist gets its
own browser, its own screenshots and its own element refs (`@eN`). Refs die on
a full page load — re-`snapshot` after navigating or resizing. Screenshots go
to `<state>/shots/<name>.png`, which is why the brief tells an agent to name a
file after the finding it is proving.

When a run finishes you are left on the review branch, because that is where
the work is: `git log main..<branch>` to read it, `git switch main` to leave.

## Report contract

| Schema | Written by | Where |
|---|---|---|
| `lain.qa.report/1` | specialist | `qa/runs/<run>/reports/<seed>.json` + `.md` |
| `lain.qa.fix/1` | engineer | `qa/runs/<run>/fixes/r<N>.json` |
| `lain.qa.verdict/1` | specialist (verify round) | `qa/runs/<run>/verdicts/<seed>-r<N>.json` |
| `lain.qa.state/1` | runner | `qa/runs/<run>/state.json` (the convergence ledger) |

A report is rejected unless every finding carries a namespaced id
(`<seed>/F-001` in the ledger, bare `F-001` inside the report), one of
`blocker|major|minor|cosmetic`, one of the seven `type` values, an
`area`, a `confidence`, numbered `repro` steps, at least one `evidence`
reference and an `expected`. `journeys_tested` must be non-empty — an agent
that filed findings without walking anything is not a report. Anything
invalid gets one repair ask in the same session, then the run records the
failure instead of a silent pass.

Everything under `qa/runs/` is ignored (D-036): reports name routes, files and
screens of a private media server, and they exist to be read locally, not
published.

## Convergence loop

1. `agent-fix` reads the ledger, takes the `LAIN_AGENT_BLOCKING` severities
   (default `blocker,major`), skips low-confidence findings
   (`LAIN_AGENT_INCLUDE_LOW=1` to include them) and hands them to the engineer.
2. The engineer must **reproduce first**, then fix in the right layer, then run
   `just check`. `web/e2e/smoke.mjs` may never be weakened; a red suite is
   reverted by the runner, not by a human cleaning up later.
3. Commits go to `LAIN_AGENT_BRANCH` (default `agent/qa/<UTC timestamp>`), each
   with a `QA-Finding: <id>` body line. The runner attributes work by reading
   `git log`, not by trusting prose: a sha the engineer claimed but that is not
   on the branch leaves the finding open, work the report claims and the branch
   lacks is committed under the finding it names and marked `recovered`, and
   anything nobody reported is written to `recovered/` as a patch and rolled
   back. The review branch is rebased onto the base branch at the start of each
   round, so the diff a human reads is only the fixes.
4. Every specialist that filed an awaiting-verify finding is re-run **in its own
   session** (`--session <id>`), with only its own findings in the brief, and
   must answer `fixed | not-fixed | partially-fixed | cannot-verify |
   not-a-defect | wont-fix` with what it actually saw.
5. Only a verdict closes a finding. The loop repeats until converged or
   `LAIN_AGENT_MAX_ROUNDS` (default 3) is spent.

A run that converges on the blocking severities while a specialist is still
missing coverage reports `converged-with-gaps`: green on what was tested, not
green on the app.

Two outcomes are reported as distinct from "converged":

- **`needs-decision`** — the engineer refused a finding because fixing it would
  break a frozen contract (`D-032`, `D-033`) or a product call. The loop stops
  spending rounds on it and does **not** mark it `wont-fix`; `FINAL.md` lists
  what it conflicts with, and the answer is the user's (Q-024).
- **coverage gaps** — a specialist that timed out, failed, or had its report
  recovered from disk is named in `FINAL.md`. Absence of findings from an
  incomplete audit is never read as health (D-035).

Budget: audits get `LAIN_AGENT_TIMEOUT_MS` (default 25 min), re-tests
`LAIN_AGENT_VERIFY_TIMEOUT_MS` (default 15 min), the engineer
`LAIN_AGENT_FIX_TIMEOUT_MS` (default 30 min), and all three are told to
write `reports/<seed>.json` as soon as their first finding is confirmed, then
overwrite it as they go. If the harness interrupts anyway, the runner continues
that same session once and asks only for the report; the result is a valid
report flagged in the ledger as a gap.

## Interruption is not silence

A timed-out session is not stopped by killing the CLI: the agent turn runs on
OpenCode's background service, so its next tool call can edit files minutes
after the runner gave up. Three things exist because of that, and none of them
are optional:

- `qa/lib/opencode.mjs` calls `POST /api/session/<id>/interrupt` before killing
  its own client, so the server stops thinking.
- the runner waits for the working tree to stop moving (`waitForQuiescence`)
  before it judges it clean.
- whatever is still uncommitted is written to
  `qa/runs/<run>/recovered/r<N>-<pre|post>-uncommitted.patch` and rolled back,
  so verification always tests the branch rather than a half-finished edit.

Unreported work is never committed by the runner: an engineer that died without
a result file leaves a patch, not a commit. Work that the result file claims but
does not name is saved the same way. Attribution therefore comes from
`git log` only — by `QA-Finding:` trailer, falling back to the `Fix <id>:`
subject, and recorded in the ledger as `via: trailer` or `via: subject` so a
human can see which contract the engineer actually honoured.

## Configuration

`.env` is loaded by `just` (`set dotenv-load`), and the fleet also reads it
directly so `node qa/fleet.mjs` works from the repo root.

| Key | Default | Meaning |
|---|---|---|
| `LAIN_AGENT_MODEL` | — (required) | `provider/model[#variant]` for the auditors. No fallback: a run on a model nobody chose is not a report (D-036) |
| `LAIN_AGENT_MODEL_AUDITOR` / `_FIXER` | `LAIN_AGENT_MODEL` | per-role override |
| `LAIN_AGENT_OPENCODE` | `opencode2` | the V2 CLI; V1 rejects this repo's config |
| `LAIN_AGENT_BROWSER` | `auto` | `auto` probes, `cdp` forces the DevTools CLI, `integrated` demands OpenCode's tools |
| `LAIN_AGENT_CHROMIUM` | `chromium` | binary used by `qa/browser.mjs` |
| `LAIN_AGENT_VIEWPORT` | `1440x900` | audit viewport |
| `LAIN_AGENT_SEEDS` | all five | which specialists run |
| `LAIN_AGENT_BLOCKING` | `blocker,major` | severities that must converge |
| `LAIN_AGENT_INCLUDE_LOW` | `0` | offer low-confidence findings to the engineer |
| `LAIN_AGENT_MAX_ROUNDS` | `3` | fix/verify budget |
| `LAIN_AGENT_TIMEOUT_MS` | `1500000` | per auditor session |
| `LAIN_AGENT_FIX_TIMEOUT_MS` | `1800000` | per engineer session |
| `LAIN_AGENT_VERIFY_TIMEOUT_MS` | `900000` | per re-test session |
| `LAIN_AGENT_BRANCH` | `agent/qa/<run id>` | review branch, never pushed |
| `LAIN_AGENT_TARGET_URL` | empty | audit an instance you already run |
| `LAIN_AGENT_ADMIN_USER` / `_PASSWORD` | `qa-admin` / `Admin-<run>-pw` | seeded (or supplied, for an external target) |
| `LAIN_AGENT_MEMBER_USER` / `_PASSWORD` | `qa-member` / `Member-<run>-pw` | non-admin account |
| `LAIN_AGENT_KEEP_INSTANCE` | `0` | leave the throwaway instance and its data behind |

Provider credentials never enter this repository: they live in OpenCode's own
auth store (`opencode2 auth`), and no agent can read `.env`.

## Layout

```
qa/
  agents/*.md      the specialists (tracked; installed to .opencode/agents by agent-sync)
  prompts/*.md     audit / verify / fix brief templates
  lib/*.mjs        env contract, schemas, run store, briefs, roster, CLI bridge, instance seeder
  browser.mjs      dependency-free CDP driver
  fleet.mjs        doctor | e2e | fix | status | sync | clean
  runs/ .cache/    local artifacts and disposable instances (ignored)
```

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `opencode` rejects `opencode.json` | V1 on `PATH`; set `LAIN_AGENT_OPENCODE=/path/to/opencode2` |
| probe answers `BROWSER=absent` | Normal headless: `auto` falls back to CDP. Only `integrated` mode fails |
| `[browser.disconnected]` from `browser.*` | The integrated tools need the desktop client attached — use CDP |
| audit found nothing but the UI is clearly broken | Check `FINAL.md` coverage gaps and `qa/runs/<run>/logs/*.jsonl` |
| `exited with code null (timed out)` | Raise `LAIN_AGENT_TIMEOUT_MS`, or narrow `LAIN_AGENT_SEEDS` |
| engineer reports commits that do not exist | Intended guard: the ledger reopens those findings and says so in `FINAL.md` |
| reports reference an old `base_url` | Reports name the instance they were filed against; each `agent-fix` round boots a fresh one |
