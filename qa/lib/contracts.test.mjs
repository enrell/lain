/*
 * Fleet contract tests: pure logic, no model, no browser, no server.
 * Run with `just agent-test`.
 *
 * These guard the promises the fleet makes to a contributor: a report is only
 * accepted when every finding is reproducible prose *and* valid JSON, an
 * interrupted run cannot pass for a converged one, and no ledger entry ships a
 * commit that does not exist.
 */
import { strict as assert } from 'node:assert';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { parseDotenv, resolveConfig, EnvError } from './env.mjs';
import { validateReport, validateVerdicts, validateFix, reportToMarkdown, indexToMarkdown, readArtifact } from './schema.mjs';
import {
	createRun,
	newRunState,
	seedState,
	actionable,
	applyFixes,
	applyVerdicts,
	reopenRolledBack,
	converged,
	findOpen,
	findEscalated,
	summary,
	listRuns,
	auditorSessions,
	publicAccounts,
	journal,
	findingKey,
	resolveFinding,
	SETTLED,
} from './runs.mjs';
import { render, renderTemplate, browserVerbs, REPORT_EXAMPLE, SEVERITY_TEXT } from './briefs.mjs';
import { fleetAgents, installAgents, auditAgent, SEEDS } from './agents.mjs';

const ROOT = new URL('../..', import.meta.url).pathname.replace(/\/$/, '');

function goodReport(over = {}) {
	return {
		schema: 'lain.qa.report/1',
		seed: 'usability',
		title: 'Usability audit',
		model: 'bai/qwen3.8-flash',
		base_url: 'http://127.0.0.1:9413',
		summary: 'Covered five journeys.',
		journeys_tested: ['resumed an episode from the home shelf'],
		findings: [
			{
				id: 'F-001',
				title: 'Silent scan failure',
				detail: 'The run reports success while the root was unreadable.',
				severity: 'blocker',
				type: 'usability',
				area: 'web',
				confidence: 'high',
				route: '/settings/libraries',
				repro: ['Add a library with a path that does not exist', 'Run Scan and wait'],
				evidence: ['qa/runs/r1/browser/usability/shots/F-001.png', 'stats.errors was 3'],
				expected: 'Name the unreadable root on the same screen.',
			},
		],
		...over,
	};
}

function baseVars(over = {}) {
	return {
		SEED: 'usability',
		BASE_URL: 'http://127.0.0.1:9413',
		ADMIN_USER: 'qa-admin',
		ADMIN_PASSWORD: 'pw',
		MEMBER_USER: 'qa-member',
		MEMBER_PASSWORD: 'pw',
		REPO_ROOT: ROOT,
		REVISION: 'abc1234',
		LIBRARIES: 'Anime (anime), Movies (movie)',
		CATALOG_SUMMARY: '4 items',
		RUN: 'r1',
		ROUND: '1',
		MODEL: 'bai/qwen3.8-flash',
		REPORT_JSON: 'qa/runs/r1/reports/usability.json',
		REPORT_MD: 'qa/runs/r1/reports/usability.md',
		ARTIFACT_DIR: 'qa/runs/r1/browser/usability/shots',
		BROWSER_STATE: 'qa/runs/r1/browser/usability',
		BROWSER_MODE_NOTE: 'Chromium over CDP',
		BROWSER_VERBS: 'node qa/browser.mjs status',
		REPORT_EXAMPLE: JSON.stringify(REPORT_EXAMPLE, null, 2),
		SEVERITY_TEXT,
		TIME_BUDGET_MIN: '23',
		VERIFY_BUDGET_MIN: '14',
		FINDINGS_BLOCK: '### usability/F-001',
		FIX_BLOCK: '{}',
		FIX_JSON: 'qa/runs/r1/fixes/r1.json',
		FIX_EXAMPLE: '{}',
		VERDICT_JSON: 'qa/runs/r1/verdicts/usability-r1.json',
		VERDICT_EXAMPLE: '{}',
		BRANCH: 'agent/qa/r1',
		...over,
	};
}

test('parseDotenv handles quotes, comments and blank lines', () => {
	const parsed = parseDotenv('# comment\n\nA=1\nB="two words"\nC=\'three\' # trailing\nD=4 # trailing\n');
	assert.equal(parsed.A, '1');
	assert.equal(parsed.B, 'two words');
	assert.equal(parsed.C, 'three');
	assert.equal(parsed.D, '4');
});

test('resolveConfig refuses to guess a model', () => {
	assert.throws(
		() => resolveConfig(ROOT, { LAIN_AGENT_MODEL: '' }),
		(err) => err instanceof EnvError && /LAIN_AGENT_MODEL/.test(err.message) && err.hints.length > 0
	);
	assert.throws(() => resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'gpt-5' }), EnvError);
	const config = resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'bai/qwen3.8-flash' });
	assert.equal(config.provider, 'bai');
	assert.equal(config.modelName, 'qwen3.8-flash');
	// Auditor and engineer fall back to the one model the contributor chose,
	// never to a provider default the harness picked.
	assert.equal(config.auditorModel, 'bai/qwen3.8-flash');
	assert.equal(config.fixerModel, 'bai/qwen3.8-flash');
});

test('a #variant picks a reasoning profile without hiding the model name', () => {
	const config = resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'opencode-go/deepseek-v4.1-flash#high' });
	assert.equal(config.provider, 'opencode-go');
	// The provider lists the bare model; the doctor looks it up without the
	// variant, while the CLI still gets the whole string.
	assert.equal(config.modelName, 'deepseek-v4.1-flash');
	assert.equal(config.variant, 'high');
	assert.equal(config.model, 'opencode-go/deepseek-v4.1-flash#high');
});

test('resolveConfig validates the knobs a contributor can mistype', () => {
	const bad = (over, re) =>
		assert.throws(() => resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'p/m', ...over }), (e) => e instanceof EnvError && re.test(e.message), JSON.stringify(over));
	bad({ LAIN_AGENT_SEEDS: 'usability,typo' }, /unknown seed/);
	bad({ LAIN_AGENT_BROWSER: 'firefox' }, /auto, cdp or integrated/);
	bad({ LAIN_AGENT_BLOCKING: 'critical' }, /unknown severities/);
	bad({ LAIN_AGENT_VIEWPORT: 'wide' }, /WIDTHxHEIGHT/);
	bad({ LAIN_AGENT_MAX_ROUNDS: '0' }, /between 1 and 10/);
});

test('the fix branch is namespaced and never the default branch', () => {
	const config = resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'p/m' });
	assert.match(config.branch, /^agent\/qa\//);
	assert.notEqual(config.branch, 'main');
	assert.equal(resolveConfig(ROOT, { LAIN_AGENT_MODEL: 'p/m', LAIN_AGENT_BRANCH: 'agent/experiment' }).branch, 'agent/experiment');
});

test('a report is rejected unless every finding can be re-run', () => {
	assert.equal(validateReport(goodReport(), 'usability').ok, true);
	const cases = [
		[{ schema: 'lain.qa.report/2' }, /schema/],
		[{ seed: 'ui' }, /seed/],
		[{ journeys_tested: [] }, /journeys_tested/],
		[{ summary: '  ' }, /summary/],
		[{ findings: 'many' }, /findings/],
	];
	for (const [over, re] of cases) {
		const result = validateReport(goodReport(over), 'usability');
		assert.equal(result.ok, false, JSON.stringify(over));
		assert.match(result.errors.join('\n'), re);
	}
	const thin = goodReport();
	delete thin.findings[0].repro;
	delete thin.findings[0].evidence;
	const errors = validateReport(thin, 'usability').errors.join('\n');
	assert.match(errors, /repro/);
	assert.match(errors, /evidence/);
});

test('finding ids, enums and duplicates are enforced', () => {
	const dupe = goodReport();
	dupe.findings.push({ ...dupe.findings[0] });
	assert.match(validateReport(dupe, 'usability').errors.join('\n'), /duplicate F-001/);
	const badEnums = goodReport();
	badEnums.findings[0].severity = 'catastrophic';
	badEnums.findings[0].type = 'vibes';
	badEnums.findings[0].confidence = 'certain';
	assert.equal(validateReport(badEnums, 'usability').errors.length, 3);
	const badId = goodReport();
	badId.findings[0].id = '1';
	assert.match(validateReport(badId, 'usability').errors.join('\n'), /must look like/);
});

test('an honest empty report validates', () => {
	assert.equal(validateReport(goodReport({ findings: [] }), 'usability').ok, true);
});

test('verdicts only accept the shared vocabulary', () => {
	const good = {
		schema: 'lain.qa.verdict/1',
		seed: 'usability',
		model: 'm',
		results: [{ id: 'usability/F-001', verdict: 'fixed', observations: 'watched it work' }],
	};
	assert.equal(validateVerdicts(good, 'usability').ok, true);
	good.results[0].verdict = 'looks-good';
	assert.match(validateVerdicts(good, 'usability').errors.join('\n'), /verdict: one of/);
	delete good.results[0].observations;
	assert.match(validateVerdicts(good, 'usability').errors.join('\n'), /observations/);
});

test('a fix result must carry a plausible commit and repeatable verification', () => {
	const doc = {
		schema: 'lain.qa.fix/1',
		run: 'r1',
		round: 1,
		branch: 'agent/qa/r1',
		summary: 'Fixed the silent scan failure.',
		changes: [{ id: 'usability/F-001', status: 'committed', commit: 'deadbeef', files: ['web/src/x.svelte'], verification: 're-ran the repro' }],
	};
	assert.equal(validateFix(doc, 'r1', 1).ok, true);
	doc.changes[0].commit = 'i believe it is a7x9';
	assert.match(validateFix(doc, 'r1', 1).errors.join('\n'), /real 7-40 hex sha/);
	doc.changes[0].commit = 'deadbeef';
	doc.changes[0].status = 'needs-decision';
	assert.match(validateFix(doc, 'r1', 1).errors.join('\n'), /reason: required/);
	doc.changes[0].id = 'F-???';
	assert.match(validateFix(doc, 'r1', 1).errors.join('\n'), /finding id from your brief/);
});

test('readArtifact distinguishes missing from malformed', () => {
	const dir = mkdtempSync(join(tmpdir(), 'lain-qa-art-'));
	try {
		assert.match(readArtifact(join(dir, 'nope.json'), 'report', 'usability').errors.join(' '), /missing report file/);
		const broken = join(dir, 'broken.json');
		writeFileSync(broken, '{');
		assert.match(readArtifact(broken, 'report', 'usability').errors.join(' '), /not valid JSON/);
		const ok = join(dir, 'ok.json');
		writeFileSync(ok, JSON.stringify(goodReport()));
		assert.equal(readArtifact(ok, 'report', 'usability').ok, true);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('the run store lays out one directory per run and lists it', () => {
	const dir = mkdtempSync(join(tmpdir(), 'lain-qa-run-'));
	try {
		const run = createRun(dir, 'r1');
		for (const sub of ['reports', 'verdicts', 'fixes', 'logs', 'browser']) assert.ok(existsSync(join(run, sub)), sub);
		assert.deepEqual(listRuns(dir), []);
		writeFileSync(join(run, 'manifest.json'), '{}');
		assert.deepEqual(listRuns(dir), ['r1']);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('two seeds filing F-001 become two distinct ledger entries', () => {
	const state = seedState(newRunState({ run: 'r1' }), [
		{ seed: 'usability', doc: goodReport(), session: 'ses_u' },
		{
			seed: 'a11y',
			doc: goodReport({ seed: 'a11y', findings: [{ ...goodReport().findings[0], id: 'F-001', type: 'accessibility', severity: 'minor' }] }),
			session: 'ses_a',
		},
	]);
	assert.deepEqual(Object.keys(state.findings).sort(), ['a11y/F-001', 'usability/F-001']);
	assert.equal(state.findings['usability/F-001'].report_id, 'F-001');
	assert.equal(findingKey('ui', 'F-002'), 'ui/F-002');
	assert.equal(findingKey('ui', 'ui/F-002'), 'ui/F-002');
	// Only the blocking one is offered to the engineer.
	assert.deepEqual(
		actionable(state, { severities: ['blocker', 'major'] }).map((f) => f.id),
		['usability/F-001']
	);
	assert.equal(converged(state, ['blocker', 'major']), false);
	assert.equal(converged(state, ['cosmetic']), true);
});

test('a finding closes only on the auditor’s verdict', () => {
	const state = seedState(newRunState({ run: 'r1' }), [{ seed: 'usability', doc: goodReport(), session: 'ses_u' }]);
	const key = 'usability/F-001';
	applyFixes(state, { changes: [{ id: key, status: 'committed', commit: 'abc1234', verification: 're-ran the repro' }] }, 1);
	assert.equal(state.findings[key].status, 'awaiting-verify');
	assert.equal(state.findings[key].fix_claim.sha, 'abc1234');
	// A claimed sha is a claim, not a record: `state.commits` is filled by
	// reading the branch, so an invented sha can never become ledger truth.
	assert.deepEqual(state.commits, []);
	applyVerdicts(state, 'usability', { results: [{ id: key, verdict: 'not-fixed', observations: 'still silent' }] }, 1);
	assert.equal(state.findings[key].status, 'not-fixed');
	assert.deepEqual(findOpen(state).map((f) => f.id), [key]);
	assert.equal(converged(state, ['blocker']), false);
	applyVerdicts(state, 'usability', { results: [{ id: key, verdict: 'fixed', observations: 'it names the path now' }] }, 2);
	assert.equal(state.findings[key].rounds, 2);
	assert.equal(findOpen(state).length, 0);
	assert.deepEqual(summary(state), { fixed: 1 });
	assert.ok(SETTLED.has('fixed'));
});

test('an engineer that reports no commit cannot advance the ledger', () => {
	const state = seedState(newRunState({ run: 'r1' }), [{ seed: 'usability', doc: goodReport(), session: 's' }]);
	applyFixes(state, { changes: [{ id: 'usability/F-001', status: 'committed', verification: 'x' }] }, 1);
	assert.equal(state.findings['usability/F-001'].status, 'not-fixed');
	assert.equal(state.commits.length, 0);
	applyFixes(state, { changes: [{ id: 'usability/F-001', status: 'skipped', reason: 'out of budget' }] }, 2);
	assert.equal(state.findings['usability/F-001'].status, 'open');
});

test('a rollback reopens only the findings whose commits left the branch', () => {
	const state = seedState(newRunState({ run: 'r1' }), [
		{ seed: 'usability', doc: goodReport(), session: 's' },
		{ seed: 'ui', doc: goodReport({ seed: 'ui', findings: [{ ...goodReport().findings[0], type: 'ui' }] }), session: 's2' },
	]);
	const u = 'usability/F-001';
	const ui = 'ui/F-001';
	state.commits.push(
		{ sha: 'c-usability', round: 1, subject: '', ids: [u], branch: 'b' },
		{ sha: 'c-ui', round: 1, subject: '', ids: [ui], branch: 'b' }
	);
	state.findings[u].status = 'fixed';
	state.findings[ui].status = 'fixed';
	// Roll back round 1: `c-usability` left the branch, `c-ui` is still live.
	const reopened = reopenRolledBack(state, (sha) => sha !== 'c-usability');
	assert.deepEqual(reopened, [u]);
	assert.equal(state.findings[u].status, 'not-fixed');
	assert.equal(state.findings[ui].status, 'fixed');

	// A finding fixed with no attributed commit in this ledger (fixed by an
	// earlier run on a stacked branch) has no proof it was undone: leave it.
	state.commits.length = 0;
	state.findings[u].status = 'fixed';
	assert.deepEqual(reopenRolledBack(state, () => false), []);
	assert.equal(state.findings[u].status, 'fixed');
});

test('a verdict from the wrong seed cannot close another agent’s finding', () => {
	const state = seedState(newRunState({ run: 'r1' }), [{ seed: 'usability', doc: goodReport(), session: 's' }]);
	const lines = applyVerdicts(state, 'ui', { results: [{ id: 'F-001', verdict: 'fixed', observations: 'x' }] }, 1);
	assert.match(lines.join(' '), /belongs to usability/);
	assert.equal(state.findings['usability/F-001'].status, 'open');
	// A bare id is tolerated while it is unambiguous, and refused once two
	// specialists have filed the same local number.
	assert.equal(resolveFinding(state, 'F-001').id, 'usability/F-001');
	seedState(state, [
		{ seed: 'ui', doc: goodReport({ seed: 'ui', findings: [{ ...goodReport().findings[0], type: 'ui' }] }), session: 's2' },
	]);
	assert.equal(resolveFinding(state, 'F-001'), null);
	assert.equal(resolveFinding(state, 'ui/F-001').id, 'ui/F-001');
});

test('a commit is attributed by trailer, and by subject when the trailer is missing', () => {
	// The runner reads `Fix <id>:` / `QA-Finding: <id>` out of git and never out
	// of the engineer's prose (see reconcileCommits).
	const claim = (body, subject) => {
		const claimed = [...body.matchAll(/QA-Finding:\s*([A-Za-z0-9/_-]+)/g)].map((m) => m[1]);
		const fromSubject = /^(?:Fix|Park)\s+([A-Za-z0-9/_-]+):/.exec(subject || '');
		if (fromSubject && !claimed.includes(fromSubject[1])) claimed.push(fromSubject[1]);
		return claimed;
	};
	assert.deepEqual(claim('QA-Finding: usability/F-001\n', 'Fix usability/F-001: x'), ['usability/F-001']);
	assert.deepEqual(claim('', 'Fix usability/F-002: count unreadable files'), ['usability/F-002']);
	assert.deepEqual(claim('QA-Finding: usability/F-003\n', 'Fix usability/F-003: y'), ['usability/F-003']);
	assert.deepEqual(claim('', 'Rebuild the hero'), []);
});

test('needs-decision parks the finding without settling it', () => {
	const state = seedState(newRunState({ run: 'r1' }), [{ seed: 'usability', doc: goodReport(), session: 's' }]);
	applyFixes(state, { changes: [{ id: 'usability/F-001', status: 'needs-decision', reason: 'conflicts with D-032', blocked_by: 'D-032' }] }, 1);
	assert.equal(state.findings['usability/F-001'].status, 'needs-decision');
	assert.equal(state.findings['usability/F-001'].blocked_by, 'D-032');
	assert.equal(state.commits.length, 0);
	// The loop stops spending rounds on it, but the run is not converged: the
	// decision is the user's, and reports live in a gitignored directory.
	assert.deepEqual(actionable(state, { severities: ['blocker'] }), []);
	assert.equal(converged(state, ['blocker', 'major']), false);
	assert.deepEqual(findEscalated(state).map((f) => f.id), ['usability/F-001']);
});

test('"not reproduced" is a disagreement, not a closure', () => {
	const state = seedState(newRunState({ run: 'r1' }), [{ seed: 'usability', doc: goodReport(), session: 's' }]);
	applyFixes(state, { changes: [{ id: 'usability/F-001', status: 'not-reproduced', reason: 'scan reported errors' }] }, 1);
	assert.equal(state.findings['usability/F-001'].status, 'awaiting-verify');
	applyVerdicts(state, 'usability', { results: [{ id: 'usability/F-001', verdict: 'not-fixed', observations: 'still silent' }] }, 1);
	assert.equal(state.findings['usability/F-001'].status, 'not-fixed');
});

test('a new run state records which audits were incomplete', () => {
	const state = newRunState({ run: 'r1' });
	assert.deepEqual(state.incomplete_seeds, []);
});

test('run artifacts name accounts but never hold secrets', () => {
	const accounts = publicAccounts({
		admin: { username: 'qa-admin', password: 'Admin-r1-pw', token: 'eyJhbGciOi...' },
		member: { username: 'qa-member', password: 'Member-r1-pw', token: 'eyJhbGciOi...' },
	});
	assert.deepEqual(accounts, { admin: { username: 'qa-admin' }, member: { username: 'qa-member' } });
	assert.equal(JSON.stringify(accounts).includes('password'), false);
	assert.equal(JSON.stringify(accounts).includes('token'), false);
	assert.deepEqual(publicAccounts({}), { admin: null, member: null });
});

test('a recovered report is still tied to the session that filed it', () => {
	const dir = mkdtempSync(join(tmpdir(), 'lain-qa-journal-'));
	try {
		journal(dir, 'run-started', { seeds: ['usability'] });
		journal(dir, 'auditor-finished', { seed: 'usability', session: 'ses_original' });
		journal(dir, 'salvage-started', { seed: 'usability', session: 'ses_salvaged' });
		journal(dir, 'fix-applied', { round: 1, lines: [] });
		assert.deepEqual(auditorSessions(dir), { usability: 'ses_salvaged' });
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('brief templates resolve every placeholder they mention', () => {
	for (const name of ['audit', 'verify', 'fix']) {
		const out = renderTemplate(ROOT, name, baseVars());
		assert.ok(out.length > 1500, `${name} is too thin to steer a specialist`);
		assert.equal(/\{\{[A-Z_]+\}\}/.test(out), false, `${name} left a placeholder unresolved`);
	}
	assert.throws(() => render('hello {{NOPE}}', {}), /unknown placeholder/);
});

test('the re-test brief keeps the auditor on the harness target', () => {
	const brief = renderTemplate(ROOT, 'verify', baseVars());
	// Learned the hard way: an auditor with time to spare built and ran its own
	// Lain binary, then timed out before writing a single verdict.
	assert.match(brief, /belongs to the harness/);
	assert.match(brief, /Never build,\s+install, start or stop a server/);
	assert.match(brief, /after the \*\*first\*\* finding/);
	assert.match(brief, /127\.0\.0\.1:9413/);
});

test('the audit brief fences the agent to its own run directory', () => {
	const brief = renderTemplate(ROOT, 'audit', baseVars());
	assert.match(brief, /qa\/runs\/r1\/reports\/usability\.json/);
	assert.match(brief, /as soon as your first finding is confirmed/);
	assert.doesNotMatch(brief, /just (agent-fix|agent-sync|web|build|check)/);
	assert.match(browserVerbs({ script: 'qa/browser.mjs', state: 'qa/runs/r1/browser/usability' }), /--state qa\/runs\/r1\/browser\/usability/);
});

test('every specialist is described, fenced and installable', () => {
	const agents = fleetAgents(ROOT);
	for (const seed of SEEDS) assert.ok(agents.includes(`lain-qa-${seed}`), `missing agent for seed ${seed}`);
	assert.ok(agents.includes('lain-qa-fixer'));
	assert.ok(agents.includes('lain-qa-probe'));
	for (const name of agents) {
		const text = readFileSync(join(ROOT, 'qa', 'agents', `${name}.md`), 'utf8');
		assert.match(text, /^---\n/, `${name} must start with frontmatter`);
		assert.match(text, /^description: >-/m, `${name} needs a folded description for routing`);
		assert.match(text, /^mode: primary$/m, `${name} must run as its own session`);
		if (name === 'lain-qa-probe') {
			// The probe is the one agent that denies everything and re-opens
			// exactly one capability, so the per-specialist rules below do not
			// apply to it.
			assert.match(text, /action: "\*"\n\s*resource: "\*"\n\s*effect: deny/);
			assert.match(text, /action: "execute"/);
			continue;
		}
		assert.match(text, /action: "subagent"\n\s*resource: "\*"\n\s*effect: deny/, `${name} must not launch subagents`);
		assert.match(text, /resource: "\*\.env"/, `${name} must not read .env`);
		assert.ok(text.length > 3000, `${name} is too thin to specialise the model`);
		assert.match(text, /action: "question"\n\s*resource: "\*"\n\s*effect: deny/, `${name} must not ask a question nobody can answer`);
		if (name === 'lain-qa-fixer') {
			assert.match(text, /git push \*/);
			assert.match(text, /smoke\.mjs/);
			assert.match(text, /D-032/);
			assert.match(text, /resource: "\*qa\/runs\/\*\/fixes\/\*"\n\s*effect: allow/, 'the engineer must be able to write its own result');
		} else {
			assert.match(text, /action: "edit"\n\s*resource: "\*"\n\s*effect: deny/, `${name} may only write its own report`);
			assert.match(text, /resource: "\*qa\/runs\/\*"\n\s*effect: allow/);
			assert.match(text, /git commit \*/, `${name} must not commit`);
		}
	}
	const dir = mkdtempSync(join(tmpdir(), 'lain-qa-install-'));
	try {
		const fake = join(dir, 'qa', 'agents');
		mkdirSync(fake, { recursive: true });
		copyFileSync(join(ROOT, 'qa', 'agents', 'lain-qa-ui.md'), join(fake, 'lain-qa-ui.md'));
		assert.equal(installAgents(dir).actions[0].action, 'installed');
		assert.ok(existsSync(join(dir, '.opencode', 'agents', 'lain-qa-ui.md')));
		assert.equal(installAgents(dir).actions[0].action, 'current', 're-sync must not churn mtimes');
		assert.equal(auditAgent('ui'), 'lain-qa-ui');
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('reports render as English prose a contributor can act on', () => {
	const md = reportToMarkdown(goodReport(), { run: 'r1', session: 'ses_1' });
	assert.match(md, /^# Usability audit/);
	assert.match(md, /### F-001 — Silent scan failure/);
	assert.match(md, /1\. Add a library with a path that does not exist/);
	assert.match(md, /qa\/runs\/r1\/browser\/usability\/shots\/F-001\.png/);
	assert.match(md, /\*\*Findings:\*\* 1 \(1 blocker\)/);
	assert.match(md, /\*\*Model:\*\* `bai\/qwen3.8-flash`/);
	const index = indexToMarkdown('r1', [{ id: 'usability/F-001', seed: 'usability', severity: 'blocker', type: 'usability', area: 'web', title: 'Silent scan failure' }]);
	assert.match(index, /\| usability\/F-001 \| usability \| blocker \| usability \| web \| Silent scan failure \|/);
});
