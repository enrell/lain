/*
 * The run store. Everything the fleet produces lives under qa/runs/<run>/ and
 * is disposable: reports, verdicts, screenshots, transcripts and the ledger
 * that drives the fix/verify loop.
 *
 * Writes are atomic because an agent can be killed mid-report, and a torn
 * finding file must never be read as truth by the fixer.
 */
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

export const RUNS_DIR = 'qa/runs';

export function runDir(root, id) {
	return join(root, RUNS_DIR, id);
}

export function createRun(root, id, { seeds } = {}) {
	const dir = runDir(root, id);
	for (const sub of ['reports', 'verdicts', 'fixes', 'logs', 'browser']) mkdirSync(join(dir, sub), { recursive: true });
	return dir;
}

export function atomicWrite(path, contents) {
	mkdirSync(dirname(path), { recursive: true });
	const tmp = `${path}.tmp-${process.pid}-${Date.now()}`;
	writeFileSync(tmp, contents);
	renameSync(tmp, path);
	return path;
}

export function writeJson(path, value) {
	return atomicWrite(path, `${JSON.stringify(value, null, 2)}\n`);
}

export function readJson(path, fallback = null) {
	try {
		return JSON.parse(readFileSync(path, 'utf8'));
	} catch {
		return fallback;
	}
}

/** Append-only ledger of what the fleet did, for humans reading the run later. */
export function journal(dir, event, data = {}) {
	appendFileSync(join(dir, 'journal.jsonl'), `${JSON.stringify({ at: new Date().toISOString(), event, ...data })}\n`);
}

export function listRuns(root) {
	const base = join(root, RUNS_DIR);
	if (!existsSync(base)) return [];
	return readdirSync(base)
		.filter((name) => {
			try {
				return statSync(join(base, name)).isDirectory() && existsSync(join(base, name, 'manifest.json'));
			} catch {
				return false;
			}
		})
		.sort();
}

export function latestRun(root) {
	const runs = listRuns(root);
	return runs.length ? runs[runs.length - 1] : null;
}

export function paths(dir, seed) {
	return {
		reportJson: join(dir, 'reports', `${seed}.json`),
		reportMd: join(dir, 'reports', `${seed}.md`),
		artifactDir: join(dir, 'browser', seed, 'shots'),
		transcript: join(dir, 'logs', `${seed}.jsonl`),
		verdictJson: (round) => join(dir, 'verdicts', `${seed}-r${round}.json`),
		verdictLog: (round) => join(dir, 'logs', `${seed}-r${round}-verify.jsonl`),
		browserState: join(dir, 'browser', seed),
	};
}

/* ------------------------------------------------------------------ */
/* convergence ledger                                                  */
/* ------------------------------------------------------------------ */

/**
 * The engineer stops working on a finding and hands the judgement up. This is
 * *not* a settlement: the loop may not spend more rounds on it, but the run is
 * not converged either, because `qa/runs/` is local and the decision is the
 * user's to make.
 */
export const ESCALATE = 'needs-decision';

/** Verdicts that end a finding's life. */
export const SETTLED = new Set(['fixed', 'not-a-defect', 'wont-fix']);
/** Verdicts that send the finding back to the fixer. */
export const OPENING = new Set(['not-fixed', 'partially-fixed', 'cannot-verify']);

/**
 * What a manifest may say about accounts: names, never secrets. A disposable
 * instance's password is derived from the run id (see qa/lib/instance.mjs), so
 * the run directory can still log in without holding a bearer token on disk —
 * the same ban `D-026` puts on logs applies to these artifacts.
 */
export function publicAccounts(secrets = {}) {
	const name = (account) => (account?.username ? { username: account.username } : null);
	return { admin: name(secrets.admin), member: name(secrets.member) };
}

/**
 * The session each auditor was running when it last produced something, read
 * back from the journal. A report recovered from disk can only be re-tested by
 * the agent that filed it, so the loop needs this mapping.
 */
export function auditorSessions(dir) {
	const file = join(dir, 'journal.jsonl');
	if (!existsSync(file)) return {};
	const sessions = {};
	for (const line of readFileSync(file, 'utf8').split('\n')) {
		let event;
		try {
			event = JSON.parse(line);
		} catch {
			continue;
		}
		// `journal` spreads its payload at the top level, so the last event that
		// carried both a seed and a session id wins — that is the session the
		// auditor was still in when it wrote the report.
		if (event?.seed && event?.session) sessions[event.seed] = event.session;
	}
	return sessions;
}

export function newRunState(manifest) {
	return {
		schema: 'lain.qa.state/1',
		run: manifest.run,
		phase: 'created',
		round: 0,
		findings: {},
		fixer_session: null,
		commits: [],
		incomplete_seeds: [],
		updated_at: new Date().toISOString(),
	};
}

/** Fold the auditors' reports into the ledger the fix loop consumes. */
/**
 * Two specialists both filing "F-001" is normal, so the ledger keys findings by
 * the seed that saw them. The namespaced id is what the engineer and the
 * verdicts use; the bare id stays inside the report file.
 */
export function findingKey(seed, id) {
	return String(id).includes('/') ? String(id) : `${seed}/${id}`;
}

/** Resolve a ledger key, tolerating an engineer who dropped the seed prefix. */
export function resolveFinding(state, id, seed = null) {
	if (state.findings[id]) return state.findings[id];
	if (seed && state.findings[`${seed}/${id}`]) return state.findings[`${seed}/${id}`];
	const matches = Object.values(state.findings).filter((f) => f.id === id || f.report_id === id || f.id.endsWith(`/${id}`));
	return matches.length === 1 ? matches[0] : null;
}

export function seedState(state, reports) {
	for (const { seed, doc, session } of reports) {
		for (const finding of doc.findings || []) {
			const key = findingKey(seed, finding.id);
			state.findings[key] = {
				id: key,
				report_id: finding.id,
				seed,
				agent: `lain-qa-${seed}`,
				title: finding.title,
				severity: finding.severity,
				type: finding.type,
				area: finding.area,
				confidence: finding.confidence,
				route: finding.route,
				repairable: finding.repairable !== false,
				status: 'open',
				rounds: 0,
				auditor_session: session || null,
				last_verdict: null,
				last_observation: null,
				remaining: [],
				report_path: `qa/runs/${state.run}/reports/${seed}.json`,
			};
		}
	}
	state.phase = 'audited';
	state.updated_at = new Date().toISOString();
	return state;
}

export function findOpen(state) {
	return Object.values(state.findings).filter((f) => !SETTLED.has(f.status));
}

/** Findings parked for a human: they end the loop without converging it. */
export function findEscalated(state) {
	return Object.values(state.findings).filter((f) => f.status === ESCALATE);
}

/** Findings the fixer should work this round, worst first. */
/**
 * Findings to hand the engineer: requested severity, not low-confidence, and
 * not already parked somewhere else. `awaiting-verify` belongs to the auditor
 * that filed it and `cannot-verify` belongs to a human; re-offering either
 * spends a round on work that already has an owner.
 */
/** Findings committed but not yet re-tested by the auditor that filed them. */
export function findAwaiting(state) {
	return Object.values(state.findings).filter((f) => f.status === 'awaiting-verify');
}

export function actionable(state, { severities, includeLowConfidence = false } = {}) {
	const elsewhere = new Set([ESCALATE, 'awaiting-verify', 'cannot-verify']);
	const list = findOpen(state)
		.filter((f) => !elsewhere.has(f.status))
		.filter((f) => (severities || ['blocker', 'major']).includes(f.severity));
	const filtered = includeLowConfidence ? list : list.filter((f) => f.confidence !== 'low');
	const rank = ['blocker', 'major', 'minor', 'cosmetic'];
	return filtered.sort((a, b) => rank.indexOf(a.severity) - rank.indexOf(b.severity) || a.id.localeCompare(b.id));
}

export function applyVerdicts(state, seed, verdicts, round) {
	const lines = [];
	for (const result of verdicts.results || []) {
		const finding = resolveFinding(state, result.id, seed);
		if (!finding) {
			lines.push(`${result.id}: verdict for an unknown finding (ignored)`);
			continue;
		}
		if (finding.seed !== seed) {
			lines.push(`${result.id}: verdict came from ${seed} but the finding belongs to ${finding.seed} (ignored)`);
			continue;
		}
		finding.status = result.verdict;
		finding.last_verdict = result.verdict;
		finding.last_observation = result.observations;
		finding.remaining = Array.isArray(result.remaining) ? result.remaining : [];
		finding.rounds = round;
		lines.push(`${result.id} [${finding.severity}] ${result.verdict}`);
	}
	state.updated_at = new Date().toISOString();
	return lines;
}

export function applyFixes(state, fix, round) {
	const lines = [];
	// `skipped` is not part of the contract, but an agent that reaches for it
	// must not be able to make a finding disappear: fold it into the same list.
	const changes = [...(fix.changes || []), ...(Array.isArray(fix.skipped) ? fix.skipped : [])];
	for (const change of changes) {
		const finding = resolveFinding(state, change.id, change.seed || null);
		if (!finding) {
			lines.push(`${change.id}: fix reported for an unknown finding (ignored)`);
			continue;
		}
		finding.fix_status = change.status;
		finding.fix_round = round;
		if (change.status === 'needs-decision') {
			// A frozen contract (D-032/D-033) or a product call: the engineer is
			// right to stop, but the fleet must not turn that into `wont-fix`.
			finding.status = ESCALATE;
			finding.last_observation = change.reason || change.summary || 'needs-decision';
			finding.blocked_by = change.blocked_by || null;
			lines.push(`${finding.id} -> needs-decision (a human must answer)`);
			continue;
		}
		// The claim is kept on the finding so the auditor can read it. Commit
		// records themselves come from git only: a sha the runner has not seen on
		// the branch must never sit in the ledger as if it were recorded work.
		if (change.commit) finding.fix_claim = { sha: change.commit, summary: change.summary, verification: change.verification };
		// 'skipped' means the engineer ran out of budget: keep it open for the
		// next round. 'not-reproduced' goes back to the auditor rather than
		// closing, so a disagreement is settled by the evidence of whoever is
		// right, not by whoever gave up first.
		if (change.status === 'not-reproduced') finding.status = 'awaiting-verify';
		else finding.status = change.status === 'skipped' ? 'open' : change.commit ? 'awaiting-verify' : 'not-fixed';
		finding.last_observation = change.verification || change.reason || change.summary || change.status;
		lines.push(`${change.id} -> ${change.status}${change.commit ? ` (${String(change.commit).slice(0, 8)})` : ''}`);
	}
	state.updated_at = new Date().toISOString();
	return lines;
}

/**
 * Convergence: every finding is settled, or the only ones left are severities
 * the run does not treat as blocking.
 */
export function converged(state, blocking) {
	return findOpen(state).every((f) => !blocking.includes(f.severity));
}

export function summary(state) {
	const counts = {};
	for (const f of Object.values(state.findings)) counts[f.status] = (counts[f.status] || 0) + 1;
	return counts;
}

export function removeRun(root, id) {
	rmSync(runDir(root, id), { recursive: true, force: true });
}
