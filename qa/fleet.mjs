#!/usr/bin/env node
/*
 * Lain QA fleet driver.
 *
 *   node qa/fleet.mjs doctor            preflight: model, browser, target, agents
 *   node qa/fleet.mjs e2e [--seed X]    specialists audit a live instance -> reports
 *   node qa/fleet.mjs fix  [--run ID]   engineer fixes, the finder re-tests, repeat
 *   node qa/fleet.mjs status            latest run, per-finding ledger
 *
 * The two commands a contributor types are `just agent-e2e` and
 * `just agent-fix`; this file is the machinery behind them. Reports are the
 * contract between the two halves: an auditor's prose is a claim, its findings
 * file is the evidence, and the fix loop only consumes validated files.
 *
 * The fix loop is cyclic by design: the specialist that filed a finding is the
 * one that re-tests it, inside its own session, so it carries the memory of
 * what it saw and how it proved it. A finding closes only when the auditor says
 * so after watching the fixed behaviour happen.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';

import { resolveConfig, EnvError } from './lib/env.mjs';
import { installAgents, fleetAgents, SEEDS as DEFAULT_SEEDS, auditAgent } from './lib/agents.mjs';
import { runAgent, probeBinary } from './lib/opencode.mjs';
import { startInstance } from './lib/instance.mjs';
import {
	createRun,
	publicAccounts,
	removeRun,
	resolveFinding,
	runDir,
	paths,
	journal,
	writeJson,
	readJson,
	listRuns,
	latestRun,
	newRunState,
	seedState,
	actionable,
	findOpen,
	applyVerdicts,
	applyFixes,
	converged,
	findAwaiting,
	findEscalated,
	auditorSessions,
	summary,
} from './lib/runs.mjs';
import { renderTemplate, browserVerbs, REPORT_EXAMPLE, SEVERITY_TEXT } from './lib/briefs.mjs';
import { validateReport, validateVerdicts, readArtifact, reportToMarkdown, indexToMarkdown, VERDICTS } from './lib/schema.mjs';

const ROOT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const argv = process.argv.slice(2);
const command = argv[0];
const flags = argv.slice(1);
const has = (name) => flags.includes(`--${name}`);
const flag = (name, fallback) => {
	const i = flags.indexOf(`--${name}`);
	if (i < 0) return fallback;
	const next = flags[i + 1];
	return next === undefined || next.startsWith('--') ? true : next;
};
const listFlag = (name) =>
	flag(name, '') === true
		? []
		: String(flag(name, ''))
				.split(',')
				.map((s) => s.trim())
				.filter(Boolean);

const t0 = Date.now();
const log = (...args) => console.error('[fleet]', ...args);

function fail(message, hints = []) {
	console.error(`\nagent fleet: ${message}`);
	for (const hint of hints) console.error(`  ${hint}`);
	process.exit(1);
}

function git(argsArray) {
	return new Promise((resolve) => {
		const child = spawn('git', argsArray, { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
		let out = '';
		child.stdout.on('data', (b) => (out += b.toString()));
		child.on('close', () => resolve(out.trim()));
		child.on('error', () => resolve(''));
	});
}

async function shell(json, argsArray, timeout = 120000) {
	const child = spawn('node', [json, ...argsArray], { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
	let out = '';
	let err = '';
	child.stdout.on('data', (b) => (out += b.toString()));
	child.stderr.on('data', (b) => (err += b.toString()));
	const killer = setTimeout(() => child.kill('SIGKILL'), timeout);
	killer.unref?.();
	const code = await new Promise((resolve) => child.on('close', resolve));
	clearTimeout(killer);
	let parsed = null;
	try {
		parsed = JSON.parse(out.trim().split('\n').pop());
	} catch {
		/* verbs that print prose return null here */
	}
	return { code, parsed, out, err };
}

const BROWSER_SCRIPT = 'qa/browser.mjs';
const RUNS_LABEL = 'qa/runs';
const DEFAULT_BASE = process.env.LAIN_AGENT_BASE_BRANCH || 'main';

function newRunId() {
	const d = new Date();
	const pad = (n) => String(n).padStart(2, '0');
	return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}-${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}`;
}

/* ------------------------------------------------------------------ */
/* doctor                                                              */
/* ------------------------------------------------------------------ */

async function doctor({ json = false } = {}) {
	const checks = [];
	const add = (name, ok, detail, hint) => checks.push({ name, ok, detail, hint });

	let config = null;
	try {
		config = resolveConfig(ROOT);
		add('model', true, `${config.provider}/${config.modelName} via ${config.binary}`, null);
	} catch (err) {
		if (err instanceof EnvError) add('model', false, err.message, err.hints.join(' '));
		else throw err;
	}

	const version = await probeBinary(config?.binary || 'opencode2', ROOT);
	add('opencode cli', version.ok, version.ok ? version.version : version.reason, 'install OpenCode V2 (`bun add -g @opencode-ai/cli`) and set LAIN_AGENT_OPENCODE');

	const modelKnown = config ? await knownModel(config) : { ok: false };
	if (config) {
		add(
			'model available',
			modelKnown.ok,
			modelKnown.ok ? `${config.model} is listed by ${config.binary}` : `${config.model} was not found in \`${config.binary} models\``,
			'check the spelling, or authenticate the provider with `opencode2 auth`'
		);
	}

	const chromium = config?.chromium || process.env.LAIN_AGENT_CHROMIUM || 'chromium';
	const hasChromium = await spawnOk(chromium, ['--version']);
	add('chromium', hasChromium, hasChromium ? `${chromium} is usable for browser automation` : `${chromium} not found`, `install chromium or set LAIN_AGENT_CHROMIUM`);

	const installed = existsSync(join(ROOT, '.opencode', 'agents', 'lain-qa-usability.md'));
	add('agents installed', installed, installed ? `found in .opencode/agents` : 'run `just agent-sync`', null);

	const tracked = fleetAgents(ROOT);
	add('agent sources', tracked.length >= DEFAULT_SEEDS.length + 2, `${tracked.length} agents in qa/agents`, null);

	if (config) {
		const probe = await browserProbe(config);
		add('browser backend', probe.ok, probe.detail, probe.hint);
	}

	if (json) {
		console.log(JSON.stringify({ ok: checks.every((c) => c.ok), checks }, null, 2));
	} else {
		for (const c of checks) {
			console.log(`${c.ok ? 'ok  ' : 'FAIL'} ${c.name.padEnd(16)} ${c.detail || ''}${!c.ok && c.hint ? `\n       → ${c.hint}` : ''}`);
		}
		console.log(checks.every((c) => c.ok) ? '\nagent fleet is ready.' : '\nagent fleet is not ready.');
	}
	process.exit(checks.every((c) => c.ok) ? 0 : 1);
}

async function knownModel(config) {
	return new Promise((resolve) => {
		const child = spawn(config.binary, ['models'], { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
		let out = '';
		child.stdout.on('data', (b) => (out += b.toString()));
		child.on('close', () => resolve({ ok: out.includes(config.modelName) }));
		child.on('error', () => resolve({ ok: false }));
		setTimeout(() => {
			child.kill('SIGKILL');
			resolve({ ok: false });
		}, 60000).unref?.();
	});
}

function spawnOk(command, args) {
	return new Promise((resolve) => {
		const child = spawn(command, args, { stdio: 'ignore' });
		child.on('error', () => resolve(false));
		child.on('close', (code) => resolve(code === 0));
	});
}

/** Ask a real session which browser it can use: integrated, or ours over CDP. */
async function browserProbe(config) {
	if (config.browserMode === 'cdp') {
		const started = await shell(BROWSER_SCRIPT, ['start', '--state', 'qa/.probe']);
		await shell(BROWSER_SCRIPT, ['stop', '--state', 'qa/.probe']);
		return started.code === 0
			? { ok: true, detail: 'Chromium over CDP (LAIN_AGENT_BROWSER=cdp)', hint: null }
			: { ok: false, detail: `CDP browser failed to start: ${started.err.slice(0, 200)}`, hint: 'install chromium or unset LAIN_AGENT_BROWSER' };
	}
	let text = '';
	try {
		const result = await runAgent({
			binary: config.binary,
			model: config.model,
			agent: 'lain-qa-probe',
			prompt: 'Probe now.',
			cwd: ROOT,
			timeoutMs: 180000,
		});
		text = result.text || '';
	} catch (err) {
		return { ok: false, detail: `probe session failed: ${String(err.message).slice(0, 200)}`, hint: 'check the provider credentials' };
	}
	if (text.includes('BROWSER=integrated')) return { ok: true, detail: 'OpenCode integrated browser is attached', hint: null };
	const started = await shell(BROWSER_SCRIPT, ['start', '--state', 'qa/.probe']);
	await shell(BROWSER_SCRIPT, ['stop', '--state', 'qa/.probe']);
	if (started.code !== 0) {
		return { ok: false, detail: 'neither the integrated browser nor CDP Chromium is usable', hint: 'open the OpenCode desktop app, or install chromium' };
	}
	return { ok: true, detail: 'integrated browser absent; using Chromium over CDP', hint: null };
}

/* ------------------------------------------------------------------ */
/* e2e                                                                 */
/* ------------------------------------------------------------------ */

async function e2e() {
	let config;
	try {
		config = resolveConfig(ROOT);
	} catch (err) {
		fail(err.message, err.hints || []);
	}
	const seeds = listFlag('seed').length ? listFlag('seed') : config.seeds;
	const unknown = seeds.filter((s) => !DEFAULT_SEEDS.includes(s));
	if (unknown.length) fail(`unknown seed(s) ${unknown.join(', ')}; the fleet is: ${DEFAULT_SEEDS.join(', ')}`);

	installAgents(ROOT, { log });
	const run = String(flag('run', newRunId()));
	const dir = createRun(ROOT, run);
	const statePath = join(dir, 'state.json');
	const manifest = {
		schema: 'lain.qa.manifest/1',
		run,
		started_at: new Date().toISOString(),
		command: 'e2e',
		seeds,
		model: config.model,
		auditor_model: config.auditorModel,
		fixer_model: config.fixerModel,
		binary: config.binary,
		max_rounds: config.maxRounds,
		branch: config.branch,
		blocking: config.blocking,
		revision: await git(['rev-parse', '--short', 'HEAD']),
		status: 'running',
	};
	writeJson(join(dir, 'manifest.json'), manifest);
	journal(dir, 'run-started', { seeds, model: config.model });

	log(`run ${run}: bringing up the target`);
	let instance;
	try {
		if (config.external) await externalTargetWarning(config.external);
		instance = await startInstance(ROOT, { runId: run, log, external: config.external });
	} catch (err) {
		fail(err.message, ['the target instance could not start; nothing was audited']);
	}
	manifest.base_url = instance.base;
	manifest.instance = instance.external ? 'external' : 'disposable';
	manifest.seed = instance.seed || null;
	manifest.accounts = publicAccounts(instance.secrets || {});
	writeJson(join(dir, 'manifest.json'), manifest);

	const results = [];
	const failures = [];
	const order = config.parallel ? seeds : seeds.slice();
	const runOne = async (seed) => {
		const p = paths(dir, seed);

		const browser = await startBrowser(p.browserState, config);
		if (!browser.ok) {
			failures.push({ seed, error: browser.error });
			return;
		}
		const brief = renderTemplate(ROOT, 'audit', auditVars({ config, seed, run, dir, p, instance, browser, revision: manifest.revision }));
		const started = Date.now();
		let result;
		try {
			result = await runAgent({
				binary: config.binary,
				model: config.auditorModel,
				agent: auditAgent(seed),
				prompt: brief,
				cwd: ROOT,
				transcript: p.transcript,
				timeoutMs: config.timeoutMs,
			});
		} catch (err) {
			const salvage = await salvageReport({ config, seed, run, dir, p, instance, browser, err });
			await stopBrowser(p.browserState);
			if (!salvage.ok) {
				failures.push({ seed, error: String(err.message).slice(0, 500) });
				return;
			}
			results.push({ seed, doc: salvage.doc, session: err.sessionID || null, model: config.auditorModel, salvaged: true });
			log(`${seed}: salvaged ${salvage.doc.findings.length} finding(s) from an interrupted session`);
			return;
		}
		journal(dir, 'auditor-finished', { seed, session: result.sessionID, ms: Date.now() - started, exit: result.exitCode });

		let artifact = readArtifact(p.reportJson, 'report', seed);
		if (!artifact.ok) {
			log(`${seed}: report invalid (${artifact.errors.length} problem(s)) — asking for one repair`);
			journal(dir, 'report-rejected', { seed, errors: artifact.errors });
			const repair = [
				`Your report at ${rel(p.reportJson)} did not validate. Problems:`,
				...artifact.errors.slice(0, 25).map((e) => `- ${e}`),
				'',
				'Rewrite the file so it satisfies the contract exactly, keeping every finding you already verified and deleting anything you cannot support. Do not re-audit from scratch.',
				`Then re-read the file back with your tools and confirm it parses as JSON.`,
			].join('\n');
			try {
				await runAgent({
					binary: config.binary,
					model: config.auditorModel,
					session: result.sessionID,
					agent: auditAgent(seed),
					prompt: repair,
					cwd: ROOT,
					transcript: p.transcript,
					timeoutMs: Math.min(config.timeoutMs, 420000),
				});
			} catch (err) {
				log(`${seed}: repair session failed: ${String(err.message).slice(0, 200)}`);
			}
			artifact = readArtifact(p.reportJson, 'report', seed);
		}
		await stopBrowser(p.browserState);
		if (!artifact.ok) {
			failures.push({ seed, error: `report still invalid: ${artifact.errors.slice(0, 6).join('; ')}` });
			return;
		}
		const doc = artifact.doc;
		writeJson(p.reportJson, doc);
		if (!existsSync(p.reportMd)) {
			writeText(p.reportMd, reportToMarkdown(doc, { run, session: result.sessionID }));
		}
		results.push({ seed, doc, session: result.sessionID, model: config.auditorModel });
		log(`${seed}: ${doc.findings.length} finding(s)`);
	};

	if (config.parallel) {
		await Promise.all(order.map(runOne));
	} else {
		for (const seed of order) await runOne(seed);
	}

	results.sort((a, b) => seeds.indexOf(a.seed) - seeds.indexOf(b.seed));
	const state = seedState(newRunState(manifest), results);
	// A seed that was interrupted, or that produced nothing at all, is a hole in
	// coverage: `just agent-fix` must say so instead of reading silence as clean.
	state.incomplete_seeds = [
		...results.filter((r) => r.salvaged).map((r) => r.seed),
		...failures.map((r) => r.seed),
	];
	const indexed = indexFindings(results);
	writeJson(statePath, state);
	writeJson(join(dir, 'findings.json'), { schema: 'lain.qa.index/1', run, findings: indexed });
	writeText(join(dir, 'index.md'), indexToMarkdown(run, indexed));
	if (failures.length) {
		writeJson(join(dir, 'failures.json'), { schema: 'lain.qa.failures/1', run, failures });
	}
	manifest.status = failures.length && !results.length ? 'failed' : 'audited';
	manifest.finished_at = new Date().toISOString();
	writeJson(join(dir, 'manifest.json'), manifest);
	journal(dir, 'run-finished', { findings: Object.keys(state.findings).length, failures: failures.length });

	const bySeverity = {};
	for (const row of indexed) bySeverity[row.severity] = (bySeverity[row.severity] || 0) + 1;
	console.log(
		[
			`QA run ${run} on ${instance.base} (${config.model})`,
			...results.map((r) => `  ${r.seed.padEnd(12)} ${String(r.doc.findings.length).padStart(3)} findings  ${rel(paths(dir, r.seed).reportMd)}`),
			...failures.map((f) => `  ${f.seed.padEnd(12)}  FAILED      ${f.error.slice(0, 120)}`),
			`  total: ${indexed.length} (${Object.entries(bySeverity).map(([k, v]) => `${v} ${k}`).join(', ') || 'none'})`,
			`  index: ${rel(join(dir, 'index.md'))}`,
		].join('\n')
	);
	await stopInstance(instance);
	process.exit(failures.length ? 2 : 0);
}

/** Human-readable artifact: created lazily, always newline-terminated. */
function writeText(path, text) {
	mkdirSync(path.split('/').slice(0, -1).join('/'), { recursive: true });
	writeFileSync(path, text.endsWith('\n') ? text : `${text}\n`);
}
function rel(path) {
	return path.startsWith(ROOT + '/') ? path.slice(ROOT.length + 1) : path;
}
function auditVars({ config, seed, run, dir, p, instance, browser, revision }) {
	const seedInfo = instance.seed || {};
	const libraries = seedInfo.libraries?.length
		? `${seedInfo.libraries.map((l) => `${l.name} (${l.type})`).join(', ')} — scan: ${seedInfo.scan?.candidates ?? '?'} candidate(s), ${seedInfo.scan?.identified ?? '?'} identified, ${seedInfo.scan?.errors ?? '?'} error(s)`
		: 'unknown (external target, not seeded by the fleet)';
	return {
		SEED: seed,
		BASE_URL: instance.base,
		ADMIN_USER: instance.secrets?.admin?.username || config.admin.username,
		ADMIN_PASSWORD: instance.secrets?.admin?.password || config.admin.password,
		MEMBER_USER: instance.secrets?.member?.username || config.member.username,
		MEMBER_PASSWORD: instance.secrets?.member?.password || config.member.password,
		REPO_ROOT: ROOT,
		REVISION: revision,
		LIBRARIES: libraries,
		CATALOG_SUMMARY: seedInfo.catalog_summary || 'unknown',
		RUN: run,
		MODEL: config.auditorModel,
		REPORT_JSON: rel(p.reportJson),
		REPORT_MD: rel(p.reportMd),
		ARTIFACT_DIR: rel(p.artifactDir),
		BROWSER_STATE: rel(p.browserState),
		BROWSER_MODE_NOTE: browser.detail,
		BROWSER_VERBS: browserVerbs({ script: BROWSER_SCRIPT, state: rel(p.browserState) }),
		TIME_BUDGET_MIN: String(Math.max(5, Math.round(config.timeoutMs / 60000) - 2)),
		REPORT_EXAMPLE: JSON.stringify({ ...REPORT_EXAMPLE, seed, model: config.auditorModel, base_url: instance.base }, null, 2),
		SEVERITY_TEXT,
	};
}

/**
 * An interrupted auditor keeps its context: continue the same session and ask
 * for the report only. Cheap, and it turns a killed run into usable findings.
 */
async function salvageReport({ config, seed, run, dir, p, instance, browser, err }) {
	if (!err.sessionID) return { ok: false };
	log(`${seed}: ${err.timedOut ? 'timed out' : 'failed'} — asking the same session for whatever it verified`);
	journal(dir, 'salvage-started', { seed, session: err.sessionID, reason: String(err.message).slice(0, 200) });
	const prompt = [
		'Your audit was interrupted by the time budget. Do not start new journeys and do not open more pages.',
		'',
		'Write ' + rel(p.reportJson) + ' now, from what you already verified in this session: every finding you reproduced,',
		'with the repro steps and evidence paths you actually collected. Put journeys you did not finish in `not_tested`.',
		'Follow the contract exactly (schema, seed, model, base_url, findings[]). Then write a short ' + rel(p.reportMd) + ' in English.',
		'If you verified nothing yet, still write the file with an empty findings array and say so in `summary`.',
	].join('\n');
	try {
		await runAgent({
			binary: config.binary,
			model: config.auditorModel,
			agent: auditAgent(seed),
			session: err.sessionID,
			prompt,
			cwd: ROOT,
			transcript: p.transcript,
			timeoutMs: Math.min(config.timeoutMs, 420000),
		});
	} catch (inner) {
		return { ok: false, error: String(inner.message).slice(0, 200) };
	}
	const artifact = readArtifact(p.reportJson, 'report', seed);
	if (!artifact.ok) return { ok: false, error: artifact.errors.slice(0, 4).join('; ') };
	writeText(p.reportMd, existsSync(p.reportMd) ? readFileSync(p.reportMd, 'utf8') : reportToMarkdown(artifact.doc, { run, session: err.sessionID, note: 'salvaged after the audit was interrupted' }));
	journal(dir, 'salvage-finished', { seed, findings: (artifact.doc.findings || []).length });
	return { ok: true, doc: artifact.doc };
}

async function startBrowser(state, config) {
	if (config.browserMode === 'integrated') {
		return { ok: true, detail: 'This session has the OpenCode integrated browser; use the `browser` tools. The commands below are the equivalent fallback if a browser call reports it is disconnected.', state };
	}
	const args = ['start', '--state', state, '--width', String(config.width), '--height', String(config.height)];
	const result = await shell(BROWSER_SCRIPT, args);
	if (result.code !== 0) return { ok: false, error: `chromium did not start: ${(result.parsed?.error || result.err || '').slice(0, 300)}` };
	return {
		ok: true,
		state,
		detail: 'Browser hands are the CDP CLI below — every step prints JSON. The OpenCode integrated browser is not attached to a headless session, so use these commands rather than `browser.*` tools.',
	};
}

async function stopBrowser(state) {
	await shell(BROWSER_SCRIPT, ['stop', '--state', state], 30000);
}

async function stopInstance(instance) {
	if (instance.stop) instance.stop();
}

function indexFindings(results) {
	const rows = [];
	for (const { seed, doc, session } of results) {
		for (const f of doc.findings || []) {
			rows.push({ id: f.id, seed, title: f.title, severity: f.severity, type: f.type, area: f.area, confidence: f.confidence, route: f.route, session, report: `reports/${seed}.json` });
		}
	}
	const rank = ['blocker', 'major', 'minor', 'cosmetic'];
	return rows.sort((a, b) => rank.indexOf(a.severity) - rank.indexOf(b.severity) || a.seed.localeCompare(b.seed) || a.id.localeCompare(b.id));
}

/* ------------------------------------------------------------------ */
/* fix (the convergence loop)                                          */
/* ------------------------------------------------------------------ */

async function fix() {
	let config;
	try {
		config = resolveConfig(ROOT);
	} catch (err) {
		fail(err.message, err.hints || []);
	}
	const run = String(flag('run', latestRun(ROOT) || ''));
	if (!run) fail('no QA run to fix; run `just agent-e2e` first');
	const dir = runDir(ROOT, run);
	const manifest = readJson(join(dir, 'manifest.json'));
	if (!manifest) fail(`${rel(dir)} has no manifest`);
	const statePath = join(dir, 'state.json');
	const state = readJson(statePath);
	if (!state) fail(`${rel(statePath)} is missing; re-run \`just agent-e2e\``);
	if (config.branchOverride) manifest.branch = config.branchOverride;
	const branch = manifest.branch || config.branch;

	// A run whose audit was interrupted can still be remediated: whatever valid
	// reports exist on disk are the ledger. Without this, a salvaged report
	// would be thrown away together with the findings in it.
	if (!Object.keys(state.findings).length) {
		const recovered = recoverReports({ config, dir });
		if (recovered.length) {
			seedState(state, recovered);
			state.incomplete_seeds = recovered.map((r) => r.seed);
			log(`ledger rebuilt from ${recovered.length} report(s) already on disk (coverage gap: ${state.incomplete_seeds.join(', ')})`);
			journal(dir, 'ledger-recovered', { seeds: state.incomplete_seeds });
			writeJson(statePath, state);
		}
	}

	const transcriptNotes = [];

	// Git is the only accepted proof of work, so the ledger starts by reading the
	// branch: a round killed after committing but before reporting must not
	// re-offer the finding the engineer already fixed.
	if (Object.keys(state.findings).length) {
		const already = await reconcileCommits(state, { branch, before: new Set(), round: state.round || 0, fixDoc: null });
		if (already.length) {
			log(`attributing ${already.length} commit(s) already on ${branch} to their findings`);
			for (const c of already) if (!c.ids.length) transcriptNotes.push(`commit ${c.sha.slice(0, 8)} carries no QA-Finding: it is outside this ledger`);
		}
	}

	// Verification has to happen in the session that filed the finding, so a
	// ledger written by an older runner (or recovered from disk) is backfilled
	// from the journal before the first round.
	const sessions = auditorSessions(dir);
	for (const finding of Object.values(state.findings)) {
		if (!finding.auditor_session && sessions[finding.seed]) finding.auditor_session = sessions[finding.seed];
	}

	const pending = actionable(state, { severities: config.blocking, includeLowConfidence: config.includeLowConfidence });
	const all = findOpen(state);
	if (!all.length) {
		console.log(`run ${run}: every finding is already settled; nothing to do.`);
		process.exit(0);
	}
	if (!pending.length) {
		console.log(`run ${run}: no blocking findings open (${all.length} minor/cosmetic left; see ${rel(statePath)}).`);
		process.exit(0);
	}

	log(`run ${run}: ${pending.length} blocking finding(s) across ${new Set(pending.map((f) => f.seed)).size} seed(s); branch ${branch}`);
	if (manifest.instance === 'disposable' && !config.external) {
		log('the audited instance is gone; `just agent-fix` starts a fresh one for reproduction');
	}
	let instance;
	try {
		if (config.external) await externalTargetWarning(config.external);
		instance = await startInstance(ROOT, { runId: run, log, external: config.external });
	} catch (err) {
		fail(err.message, ['the fix loop cannot start without a target to reproduce against']);
	}
	manifest.base_url = instance.base;
	writeJson(join(dir, 'manifest.json'), manifest);

	const branchState = await ensureBranch(branch);
	if (!branchState.ok) fail(branchState.error, ['commit or stash your local changes, or set LAIN_AGENT_BRANCH to another branch']);

	const fixerState = join(dir, 'browser', 'fixer');
	const browser = await startBrowser(fixerState, config);
	if (!browser.ok) fail(browser.error);

	const rounds = Number(flag('rounds', config.maxRounds));
	let round = state.round || 0;
	let parked = findEscalated(state).map((f) => f.id);

	for (let attempt = 1; attempt <= rounds; attempt++) {
		round++;
		// Verification must test the branch, not whatever a killed session last
		// wrote, so leftovers from an earlier round are saved and rolled back
		// before anyone looks at the tree.
		const leftover = await preserveDirtyWork({ dir, round, slot: 'pre', log });
		if (leftover) transcriptNotes.push(`round ${round}: uncommitted work from an earlier session preserved in ${rel(leftover)} and reverted`);
		let engineerDied = false;
		const open = actionable(state, { severities: config.blocking, includeLowConfidence: config.includeLowConfidence });
		if (!open.length && !findAwaiting(state).length) {
			// Nothing left for the engineer: either the loop converged, or the
			// only blocking findings are parked behind a decision the fleet is
			// not allowed to make. Those are different answers to the user.
			if (!converged(state, config.blocking)) parked = findEscalated(state).map((f) => f.id);
			break;
		}
		let fixDoc = null;
		if (open.length) {
		log(`round ${round}: ${open.length} finding(s) for the engineer`);
		journal(dir, 'round-start', { round, findings: open.map((f) => f.id) });

		const fixPath = join(dir, 'fixes', `r${round}.json`);
		const brief = renderTemplate(ROOT, 'fix', fixVars({ config, run, dir, instance, browser, branch, round, open, state, fixPath }));
		// Taken before the session starts: reconcile compares this tip with the
		// branch afterwards, so capturing it late would hide every commit the
		// engineer just made.
		const commitsBefore = await commitsOn(branch);
		let fixResult;
		try {
			fixResult = await runAgent({
				binary: config.binary,
				model: config.fixerModel,
				agent: 'lain-qa-fixer',
				session: state.fixer_session || undefined,
				prompt: brief,
				cwd: ROOT,
				transcript: join(dir, 'logs', `fixer-r${round}.jsonl`),
				timeoutMs: config.fixTimeoutMs,
			});
		} catch (err) {
			log(`round ${round}: engineer failed: ${String(err.message).slice(0, 300)}`);
			transcriptNotes.push(`round ${round}: engineer failed (${String(err.message).slice(0, 120)})`);
			engineerDied = true;
		}
		if (fixResult?.sessionID) state.fixer_session = fixResult.sessionID;

		let fixArtifact = engineerDied ? { ok: false, errors: ['the engineer session ended before writing a report'] } : readArtifact(fixPath, 'fix', run, { run, round });
		if (!fixArtifact.ok && !engineerDied) {
			log(`round ${round}: engineer report invalid (${fixArtifact.errors.slice(0, 3).join('; ')}) — one repair ask`);
			journal(dir, 'fix-report-rejected', { round, errors: fixArtifact.errors });
			try {
				await runAgent({
					binary: config.binary,
					model: config.fixerModel,
					agent: 'lain-qa-fixer',
					session: fixResult.sessionID || state.fixer_session || undefined,
					prompt: `Your result file ${rel(fixPath)} did not validate:\n${fixArtifact.errors.slice(0, 20).map((e) => `- ${e}`).join('\n')}\n\nRewrite it to match the contract exactly. Report the commits that really exist (\`git log --oneline -20\`), with their real shas; never invent one.`,
					cwd: ROOT,
					transcript: join(dir, 'logs', `fixer-r${round}.jsonl`),
					timeoutMs: 420000,
				});
			} catch (err) {
				log(`round ${round}: repair ask failed: ${String(err.message).slice(0, 160)}`);
			}
			fixArtifact = readArtifact(fixPath, 'fix', run, { run, round });
		}
		fixDoc = fixArtifact.ok ? fixArtifact.doc : null;
		if (fixDoc) {
			const lines = applyFixes(state, fixDoc, round);
			journal(dir, 'fix-applied', { round, lines });
			log(`round ${round}: engineer reported ${lines.length} change(s)`);
		} else {
			log(`round ${round}: ${engineerDied ? 'no engineer session to report; trusting git for this round' : `no valid ${rel(fixPath)}; trusting git for this round`}`);
			journal(dir, 'fix-report-missing', { round, errors: fixArtifact.errors });
		}

		// The ledger tracks what git says, not what the engineer claims: an agent
		// that "committed" nothing must not be able to advance a finding.
		const created = await reconcileCommits(state, { branch, before: commitsBefore, round, fixDoc });
		journal(dir, 'commits-reconciled', { round, created: created.map((c) => `${c.sha.slice(0, 8)}:${c.id || 'unattributed'}`) });
		if (created.length) log(`round ${round}: ${created.length} new commit(s) on ${branch}`);
		const uncommitted = fixDoc ? fixDoc.changes.filter((c) => c.status === 'committed' && !created.some((rec) => rec.id === resolveFinding(state, c.id)?.id)).map((c) => c.id) : [];
		if (uncommitted.length) {
			log(`round ${round}: ${uncommitted.length} change(s) claimed a commit that is not on ${branch}`);
			transcriptNotes.push(`round ${round}: claimed but absent from the branch: ${uncommitted.join(', ')}`);
		}

		// Whatever is left now is work nobody reported or committed: an engineer
		// killed mid-edit, or a tool call the server finished after its client
		// was killed. It is saved into the run directory and rolled back, so it
		// can never be tested as if it were part of the branch.
		const saved = await preserveDirtyWork({ dir, round, log });
		if (saved) {
			log(`round ${round}: unreported edits saved to ${rel(saved)} and rolled back`);
			transcriptNotes.push(`round ${round}: uncommitted engineer work preserved in ${rel(saved)} and reverted`);
		}

		}

		// Anything already committed is verified whether or not this round had
		// work left to offer: an interrupted round can leave a fix waiting here.
		const toVerify = findAwaiting(state);
		if (!toVerify.length) {
			transcriptNotes.push(`round ${round}: nothing new to verify`);
			break;
		}
		await verifyRound({ config, run, dir, state, round, toVerify, instance, fixDoc });

		if (await checksFailed(dir, round)) {
			transcriptNotes.push(`round ${round}: checks are red after verification`);
			log(`round ${round}: red checks — rolling back to the last green commit`);
			const rolled = await rollback(state, dir, round);
			transcriptNotes.push(`round ${round}: ${rolled}`);
		}
		state.round = round;
		state.updated_at = new Date().toISOString();
		writeJson(statePath, state);
		log(`round ${round} ledger: ${JSON.stringify(summary(state))}`);
		if (converged(state, config.blocking)) break;
	}

	await stopBrowser(fixerState);
	const done = converged(state, config.blocking);
	if (parked.length) {
		manifest.status = done ? 'needs-decision' : 'stalled-needs-decision';
		manifest.parked = parked;
		log(`${parked.length} finding(s) parked behind a decision the fleet must not make (see FINAL.md)`);
	} else {
		manifest.status = done ? (state.incomplete_seeds?.length ? 'converged-with-gaps' : 'converged') : 'stalled';
	}
	manifest.finished_at = new Date().toISOString();
	writeJson(join(dir, 'manifest.json'), manifest);
	state.phase = manifest.status;
	state.round = round;
	writeJson(statePath, state);
	writeText(join(dir, 'FINAL.md'), finalReport({ manifest, state, config, instance, transcriptNotes }));
	journal(dir, 'fix-finished', { status: manifest.status, rounds: round });

	const remaining = findOpen(state);
	console.log(
		[
			`run ${run}: ${manifest.status} after ${round} round(s) on ${branch}`,
			`  commits: ${state.commits.length}`,
			`  settled: ${Object.keys(state.findings).length - remaining.length}/${Object.keys(state.findings).length}`,
			`  open:    ${remaining.map((f) => `${f.id}(${f.status})`).join(', ') || 'none'}`,
			`  report:  ${rel(join(dir, 'FINAL.md'))}`,
		].join('\n')
	);
	await stopInstance(instance);
	process.exit(manifest.status === 'converged' ? 0 : 3);
}

/** Every commit sha on the branch, used as the only accepted proof of work. */
async function commitsOn(branch) {
	const out = await git(['rev-list', branch]);
	return new Set(out.split('\n').filter(Boolean));
}

/**
 * Attach the commits that actually appeared on the branch during this round to
 * the findings the engineer says it fixed. Attribution is by finding id in the
 * commit body (the brief demands it); anything else is recorded unattributed so
 * a human can still see it.
 */
async function reconcileCommits(state, { branch, before, round, fixDoc }) {
	const after = await git(['log', '--format=%H%x09%s', `${DEFAULT_BASE}..${branch}`]).catch(() => '');
	const created = [];
	for (const line of after.split('\n')) {
		const [sha, subject] = line.split('\t');
		if (!sha || before.has(sha)) continue;
		if (state.commits.some((c) => c.sha === sha)) continue;
		const body = await git(['log', '-1', '--format=%b', sha]);
		const claimed = [...body.matchAll(/QA-Finding:\s*([A-Za-z0-9/_-]+)/g)].map((m) => m[1]);
		// The brief demands both a `Fix <id>:` subject and a `QA-Finding:` body
		// line. Either one is enough to attribute the commit; the trailer is what
		// proves the engineer read the contract, so the ledger records which it got.
		const fromSubject = /^(?:Fix|Park)\s+([A-Za-z0-9/_-]+):/.exec(subject || '');
		if (fromSubject && !claimed.includes(fromSubject[1])) claimed.push(fromSubject[1]);
		const ids = claimed.map((raw) => resolveFinding(state, raw.replace(/\]$/, ''))?.id).filter(Boolean);
		const entry = { sha, round, subject: subject || '', ids, branch, via: body.includes('QA-Finding:') ? 'trailer' : ids.length ? 'subject' : null };
		created.push(entry);
		state.commits.push(entry);
		for (const id of ids) {
			const finding = state.findings[id];
			if (finding && (finding.status === 'open' || finding.status === 'not-fixed')) finding.status = 'awaiting-verify';
		}
	}
	if (!created.length) return created;
	// Without a report there is no claim to attribute the tree to, and an
	// "unattributed" commit would be the runner inventing work it did not ask
	// for; `preserveDirtyWork` keeps that case as a patch instead.
	if (!fixDoc) return created;
	// An engineer that fixed something without committing it leaves a working
	// tree behind: commit it under the finding it names, so nothing is lost and
	// nothing pretends to be verified.
	const dirty = await git(['status', '--porcelain']);
	if (dirty) {
		const orphan = fixDoc?.changes?.find((c) => c.status === 'committed' && !created.some((rec) => rec.ids.includes(resolveFinding(state, c.id)?.id)));
		const id = orphan ? resolveFinding(state, orphan.id)?.id || orphan.id : 'unattributed';
		const staged = await gitRun(['add', '-A', '--', '.', ':!qa/runs']);
		if (!staged.ok) return created;
		const message = `Fix ${id}: work left uncommitted by round ${round}\n\nQA-Finding: ${id}\nRecorded by the QA fleet runner because the engineer edited without committing.\n`;
		const committed = await gitRun(['commit', '--no-verify', '-m', message]);
		if (committed.ok) {
			const sha = await git(['rev-parse', 'HEAD']);
			state.commits.push({ sha, round, subject: `Fix ${id}: work left uncommitted`, ids: [id], branch, recovered: true });
			const finding = state.findings[id];
			if (finding && (finding.status === 'open' || finding.status === 'not-fixed')) finding.status = 'awaiting-verify';
			created.push({ sha, round, ids: [id], recovered: true });
		}
	}
	return created;
}

async function verifyRound({ config, run, dir, state, round, toVerify, instance, fixDoc }) {
	const bySeed = new Map();
	for (const finding of toVerify) {
		if (!bySeed.has(finding.seed)) bySeed.set(finding.seed, []);
		bySeed.get(finding.seed).push(finding);
	}
	for (const [seed, findings] of bySeed) {
		const p = paths(dir, seed);
		const session = findings.find((f) => f.auditor_session)?.auditor_session;
		if (!session) {
			log(`${seed}: no auditor session on record; marking ${findings.map((f) => f.id).join(',')} cannot-verify`);
			for (const f of findings) {
				f.status = 'cannot-verify';
				f.last_observation = 'the original auditor session was not recorded, so nobody could re-test it';
			}
			continue;
		}
		const browser = await startBrowser(p.browserState, config);
		if (!browser.ok) {
			log(`${seed}: cannot start browser for re-test (${browser.error})`);
			continue;
		}
		const verdictPath = p.verdictJson(round);
		const brief = renderTemplate(ROOT, 'verify', verifyVars({ config, seed, run, dir, p, findings, instance, browser, round, verdictPath, fixDoc, state }));
		try {
			const result = await runAgent({
				binary: config.binary,
				model: config.auditorModel,
				agent: auditAgent(seed),
				session,
				prompt: brief,
				cwd: ROOT,
				transcript: p.verdictLog(round),
				timeoutMs: Math.min(config.timeoutMs, 600000),
			});
			let verdict = readArtifact(verdictPath, 'verdicts', seed);
			if (!verdict.ok) {
				log(`${seed}: verdict invalid (${verdict.errors.slice(0, 3).join('; ')}) — one repair ask`);
				await runAgent({
					binary: config.binary,
					model: config.auditorModel,
					agent: auditAgent(seed),
					session: result.sessionID || session,
					prompt: `Your verdict file ${rel(verdictPath)} did not validate:\n${verdict.errors.slice(0, 20).map((e) => `- ${e}`).join('\n')}\n\nRewrite it to match the contract exactly, keeping the verdicts you already justified.`,
					cwd: ROOT,
					transcript: p.verdictLog(round),
					timeoutMs: 300000,
				});
				verdict = readArtifact(verdictPath, 'verdicts', seed);
			}
			await stopBrowser(p.browserState);
			if (!verdict.ok) {
				log(`${seed}: verdict still invalid; leaving findings for the next round`);
				for (const f of findings) f.status = 'cannot-verify';
				continue;
			}
			const lines = applyVerdicts(state, seed, verdict.doc, round);
			journal(dir, 'verified', { round, seed, lines });
			log(`round ${round} ${seed}: ${lines.join(' | ')}`);
		} catch (err) {
			await stopBrowser(p.browserState);
			log(`${seed}: re-test session failed: ${String(err.message).slice(0, 200)}`);
			for (const f of findings) f.status = 'cannot-verify';
		}
	}
}

function verifyVars({ config, seed, run, dir, p, findings, instance, browser, round, verdictPath, fixDoc, state }) {
	const blocks = findings.map((f) => {
		const change = (fixDoc?.changes || []).find((c) => c.id === f.id);
		return [
			`### ${f.id} — ${f.title} (${f.severity})`,
			`route: ${f.route}`,
			'',
			'You reported:',
			'```json',
			JSON.stringify({ ...(readReportFinding(config, dir, seed, f.report_id) || f), id: f.id }, null, 2),
			'```',
			'',
			`Use exactly \`${f.id}\` as the id in your verdict.`,

			'',
			'The engineer now claims:',
			'```json',
			JSON.stringify(change || { status: 'unreported', note: 'the engineer did not list this finding' }, null, 2),
			'```',
		].join('\n');
	});
	return {
		SEED: seed,
		ROUND: round,
		RUN: run,
		BASE_URL: instance.base,
		ADMIN_USER: instance.secrets?.admin?.username || config.admin.username,
		ADMIN_PASSWORD: instance.secrets?.admin?.password || config.admin.password,
		MEMBER_USER: instance.secrets?.member?.username || config.member.username,
		MEMBER_PASSWORD: instance.secrets?.member?.password || config.member.password,
		BROWSER_STATE: rel(p.browserState),
		FINDINGS_BLOCK: blocks.join('\n\n'),
		FIX_BLOCK: JSON.stringify(fixDoc || { changes: [] }, null, 2),
		VERDICT_JSON: rel(verdictPath),
		VERDICT_EXAMPLE: JSON.stringify(
			{
				schema: 'lain.qa.verdict/1',
				seed,
				model: config.auditorModel,
				results: findings.map((f) => ({ id: f.id, verdict: VERDICTS[0], observations: 'what you saw when you re-ran the repro', evidence: ['path or quoted output'], remaining: [] })),
			},
			null,
			2
		),
		VERDICT_LIST: VERDICTS.join(' | '),
		REVISION: state.commits.map((c) => c.sha).slice(-1)[0] || 'unknown',
	};
}

function readReportFinding(config, dir, seed, id) {
	const doc = readJson(join(dir, 'reports', `${seed}.json`));
	return (doc?.findings || []).find((f) => f.id === id);
}

function fixVars({ config, run, dir, instance, browser, branch, round, open, state, fixPath }) {
	return {
		RUN: run,
		ROUND: round,
		BRANCH: branch,
		BASE_URL: instance.base,
		ADMIN_USER: instance.secrets?.admin?.username || config.admin.username,
		ADMIN_PASSWORD: instance.secrets?.admin?.password || config.admin.password,
		MEMBER_USER: instance.secrets?.member?.username || config.member.username,
		MEMBER_PASSWORD: instance.secrets?.member?.password || config.member.password,
		BROWSER_STATE: rel(browser.state || fixPath),
		BROWSER_VERBS: browserVerbs({ script: BROWSER_SCRIPT, state: rel(join(dir, 'browser', 'fixer')) }),
		FIX_JSON: rel(fixPath),
		TIME_BUDGET_MIN: String(Math.max(5, Math.round(config.fixTimeoutMs / 60000) - 2)),
		FIX_EXAMPLE: JSON.stringify(
			{
				schema: 'lain.qa.fix/1',
				run,
				round,
				branch,
				changes: open.slice(0, 2).map((f) => ({
					id: f.id,
					status: 'committed',
					commit: 'abc1234',
					files: ['web/src/.../+page.svelte'],
					summary: 'what changed and why',
					verification: 'exact steps you re-ran to confirm the finding no longer reproduces',
				})),
				...open.slice(2, 3).map((f) => ({
					id: f.id,
					status: 'needs-decision',
					blocked_by: 'D-032',
					reason: 'fixing this changes the frozen metadata contract, so it is the user call, not yours',
					verification: 'not attempted: refused on contract grounds',
				})),
				checks: { vet: 'pass', go_test: 'pass', web_test: 'pass', web_check: 'pass' },
			},
			null,
			2
		),
		FINDINGS_BLOCK: [
			'Ids below are namespaced by the specialist that filed them. Echo each `id` exactly as written when you report what you did.',
			'',
		]
			.concat(
				open.map((f) => {
					const full = readReportFinding(config, dir, f.seed, f.report_id) || f;
					return [
						`### ${f.id} (${f.severity}, ${f.seed}) — ${f.title}`,
						'',
						'```json',
						JSON.stringify({ ...full, id: f.id, auditor_session: undefined }, null, 2),
						'```',
					].join('\n');
				})
			)
			.join('\n\n'),
		PREVIOUS_NOTES: state.round
			? JSON.stringify(
					Object.values(state.findings)
						.filter((f) => f.last_observation && f.rounds === state.round)
						.map((f) => ({ id: f.id, verdict: f.last_verdict, observation: f.last_observation, remaining: f.remaining })),
					null,
					2
				)
			: 'first round',
	};
}

async function ensureBranch(branch) {
	const current = await git(['rev-parse', '--abbrev-ref', 'HEAD']);
	const dirty = await git(['status', '--porcelain']);
	if (dirty) return { ok: false, error: 'the working tree is not clean' };
	if (current === branch) return { ok: true, created: false };
	const exists = await git(['rev-parse', '--verify', branch]);
	if (exists) {
		const out = await gitRun(['checkout', branch]);
		return out.ok ? { ok: true, created: false } : { ok: false, error: out.error };
	}
	const out = await gitRun(['checkout', '-b', branch]);
	return out.ok ? { ok: true, created: true } : { ok: false, error: out.error };
}

function gitRun(argsArray) {
	return new Promise((resolve) => {
		const child = spawn('git', argsArray, { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
		let err = '';
		child.stderr.on('data', (b) => (err += b.toString()));
		child.on('close', (code) => resolve(code === 0 ? { ok: true } : { ok: false, error: `git ${argsArray.join(' ')}: ${err.slice(0, 300)}` }));
		child.on('error', (e) => resolve({ ok: false, error: String(e.message) }));
	});
}

async function checksFailed(dir, round) {
	const started = Date.now();
	journal(dir, 'checks-started', { round });
	const child = spawn('just', ['check'], { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
	let out = '';
	child.stdout.on('data', (b) => (out += b.toString()));
	child.stderr.on('data', (b) => (out += b.toString()));
	const code = await new Promise((resolve) => child.on('close', resolve));
	writeText(join(dir, 'logs', `checks-r${round}.log`), out);
	journal(dir, 'checks-finished', { round, exit: code, ms: Date.now() - started });
	return code !== 0;
}

async function rollback(state, dir, round) {
	const last = state.commits.filter((c) => c.round === round).at(-1);
	if (!last) return 'no commit this round to roll back';
	const head = await git(['rev-parse', 'HEAD']);
	if (!head.startsWith(last.sha)) return `HEAD (${head.slice(0, 8)}) is not the round commit (${last.sha}); left alone`;
	const revert = await gitRun(['revert', '--no-edit', ...state.commits.filter((c) => c.round === round).map((c) => c.sha).reverse()]);
	if (!revert.ok) return `revert failed: ${revert.error}`;
	for (const c of state.commits.filter((x) => x.round === round)) {
		const finding = state.findings[c.id];
		if (finding && finding.status === 'awaiting-verify') finding.status = 'not-fixed';
	}
	return 'rolled back';
}

/**
 * Write every uncommitted change (tracked or not) into the run directory as a
 * patch, then return the tree to the branch tip. Returns the patch path, or
 * null when there was nothing to save.
 *
 * The branch is agent-owned work, so rolling back is safe here — the caller has
 * already refused to start on a dirty tree, and everything the engineer reported
 * was committed by now. Two looks because a killed session's last tool call can
 * still land a second after the runner notices.
 */
async function preserveDirtyWork({ dir, round, slot = 'post', log }) {
	// Wait for the tree to stop moving before judging it: an interrupted
	// session's last tool call can still land on the shared server.
	await waitForQuiescence();
	const files = await dirtyPaths();
	if (!files.length) return null;
	const out = join(dir, 'recovered', `r${round}-${slot}-uncommitted.patch`);
	mkdirSync(dirname(out), { recursive: true });
	// intent-to-add makes untracked files show up in `git diff`, without
	// staging them for a commit.
	const intent = await gitRun(['add', '-A', '-N', '--', '.', ':!qa/runs']);
	const diff = await git(['diff', '--binary', 'HEAD']);
	if (intent.ok) await gitRun(['reset', '--quiet']);
	writeText(out, `# uncommitted work found after round ${round}\n# files:\n${files.map((f) => `#   ${f}`).join('\n')}\n\n${diff}\n`);
	log(`${files.length} unreported file(s): ${files.slice(0, 6).join(', ')}${files.length > 6 ? ', …' : ''}`);
	const restored = await gitRun(['restore', '--source=HEAD', '--staged', '--worktree', '--', '.', ':!qa/runs']);
	const cleaned = await gitRun(['clean', '-fdq', '--', '.', ':!qa/runs']);
	if (!restored.ok || !cleaned.ok) log(`warning: could not roll the tree back to ${round}: ${restored.error || cleaned.error}`);
	return out;
}

/**
 * True once `git status` has not changed for `settleMs`, false if the wait ran
 * out. Cheap insurance against reading a half-written working tree.
 */
async function waitForQuiescence({ maxMs = 90000, settleMs = 15000 } = {}) {
	const started = Date.now();
	let last = await git(['status', '--porcelain']);
	let changed = started;
	while (Date.now() - started < maxMs) {
		await sleep(5000);
		const now = await git(['status', '--porcelain']);
		if (now !== last) {
			last = now;
			changed = Date.now();
			continue;
		}
		if (Date.now() - changed >= settleMs) return true;
	}
	return false;
}

async function dirtyPaths() {
	const out = await git(['status', '--porcelain']);
	return out.split('\n').map((line) => line.slice(3).trim()).filter((path) => path && !path.startsWith('qa/runs/'));
}

/** Valid reports already written for this run, in seed order. */
function recoverReports({ config, dir }) {
	const sessions = auditorSessions(dir);
	const reports = [];
	for (const seed of config.seeds) {
		const artifact = readArtifact(join(dir, 'reports', `${seed}.json`), 'report', seed);
		if (artifact.ok) reports.push({ seed, doc: artifact.doc, session: sessions[seed] || null, salvaged: true });
	}
	return reports;
}

/**
 * An external target is somebody's real server. The specialists will add and
 * remove libraries, run scans and change the account they are handed, so the
 * choice gets a pause rather than a log line (D-035).
 */
async function externalTargetWarning(base) {
	console.log('');
	console.log(`!! ${base} is NOT a disposable instance.`);
	console.log('!! The specialists mutate what they can see: they will add and remove');
	console.log('!! libraries, run scans, change this account, and touch real progress.');
	console.log('!! Waiting 10s — Ctrl-C now, or unset LAIN_AGENT_TARGET_URL to use a');
	console.log('!! throwaway instance instead.');
	await sleep(10000);
}

function finalReport({ manifest, state, config, instance, transcriptNotes }) {
	const lines = [];
	lines.push(`# QA convergence report — run \`${manifest.run}\``);
	lines.push('');
	lines.push(`- Status: **${manifest.status}** after ${state.round} fix round(s)`);
	lines.push(`- Branch: \`${manifest.branch}\` (${state.commits.length} commit(s))`);
	lines.push(`- Auditor model: \`${manifest.auditor_model}\` · Engineer model: \`${manifest.fixer_model}\``);
	lines.push(`- Audited revision: \`${manifest.revision}\``);
	lines.push(`- Blocking severities: ${config.blocking.join(', ')}`);
	lines.push('');
	lines.push('## Ledger');
	lines.push('');
	lines.push('| Finding | Seed | Severity | Final status | Rounds | Auditor observation |');
	lines.push('| --- | --- | --- | --- | --- | --- |');
	for (const f of Object.values(state.findings)) {
		lines.push(`| ${f.id} | ${f.seed} | ${f.severity} | ${f.status} | ${f.rounds} | ${(f.last_observation || '').replace(/\|/g, '/').slice(0, 160)} |`);
	}
	const parked = findEscalated(state);
	if (parked.length) {
		lines.push('');
		lines.push('## Parked: needs a decision from you');
		lines.push('');
		lines.push('The engineer refused these on purpose. Fixing them would mean changing a');
		lines.push('decision recorded in the wiki, so the fleet left the code alone. Answer');
		lines.push('them in `docs/advisor/open-questions.md` and re-run.');
		lines.push('');
		for (const f of parked) {
			lines.push(`- \`${f.id}\` (${f.severity}) — ${f.title}`);
			if (f.blocked_by) lines.push(`  - conflicts with: ${f.blocked_by}`);
			lines.push(`  - engineer said: ${f.last_observation}`);
		}
	}
	if (state.incomplete_seeds?.length) {
		lines.push('');
	lines.push('## Coverage gaps');
		lines.push('');
		lines.push(`These specialists did not deliver a complete audit, so "no findings" from`);
		lines.push(`them is not evidence of health: ${state.incomplete_seeds.join(', ')}.`);
		lines.push('Re-run them (`just agent-e2e ' + state.incomplete_seeds.join(' ') + '`) or read');
		lines.push('their transcripts under `qa/runs/' + manifest.run + '/logs/`.');
	}
	if (transcriptNotes.length) {
		lines.push('');
		lines.push('## Round notes');
		lines.push('');
		for (const note of transcriptNotes) lines.push(`- ${note}`);
	}
	lines.push('');
	lines.push('## What this run proves');
	lines.push('');
	lines.push('This is a local, disposable artifact produced by models driving a real');
	lines.push('browser against a throwaway instance. It is not a CI gate: the');
	lines.push('reproducible contract stays `web/e2e/smoke.mjs`, and a human reviews');
	lines.push('the branch before anything is merged.');
	lines.push('');
	return lines.join('\n');
}

/* ------------------------------------------------------------------ */
/* status / sync                                                       */
/* ------------------------------------------------------------------ */

async function status() {
	const run = String(flag('run', latestRun(ROOT) || ''));
	if (!run) fail('no runs yet; try `just agent-e2e`');
	const dir = runDir(ROOT, run);
	const manifest = readJson(join(dir, 'manifest.json'), {});
	const state = readJson(join(dir, 'state.json'), { findings: {}, commits: [] });
	console.log(
		[
			`run ${run}  status=${manifest.status || '?'}  target=${manifest.base_url || '?'}  model=${manifest.model || '?'}`,
			`branch ${manifest.branch}  commits ${state.commits?.length || 0}  round ${state.round || 0}`,
			'  ' + JSON.stringify(summary(state)),
			...Object.values(state.findings || {})
				.filter((f) => !['fixed', 'not-a-defect', 'wont-fix'].includes(f.status))
				.map((f) => `  ${f.id} [${f.severity}] ${f.status}  ${f.title}`),
		].join('\n')
	);
}

/**
 * `just agent-clean <run>` — reports are local, so deleting them is cheap, but
 * wiping every run needs an explicit `--all`: never a default that destroys
 * evidence somebody might still be reading.
 */
async function clean() {
	const wanted = String(flag('run', flags[0] || ''));
	if (!wanted) fail('clean needs a run id, or --all to remove every run', [`known runs: ${listRuns(ROOT).join(', ') || 'none'}`]);
	const runs = wanted === '*' ? (has('all') ? listRuns(ROOT) : fail('--all is required to remove every run')) : [wanted];
	if (!runs.length) {
		console.log('no runs to clean.');
		return;
	}
	for (const run of runs) {
		removeRun(ROOT, run);
		console.log(`removed ${RUNS_LABEL}/${run}`);
	}
}

async function sync() {
	let config = null;
	try {
		config = resolveConfig(ROOT);
	} catch {
		/* syncing definitions does not need a model */
	}
	const installed = installAgents(ROOT, { log: (line) => console.log(line) });
	console.log(`\n${installed.actions.length} agent definition(s) in ${installed.dest}`);
	if (config) {
		const probe = await browserProbe(config);
		console.log(`browser backend: ${probe.detail}${probe.hint ? ` (hint: ${probe.hint})` : ''}`);
	}
}

/* ------------------------------------------------------------------ */

const commands = { doctor, e2e, fix, status, sync, clean };
if (!command || !commands[command]) {
	fail(`unknown command "${command || '(none)'}"`, [`available: ${Object.keys(commands).join(', ')}`]);
}
try {
	await commands[command]();
} catch (err) {
	if (err instanceof EnvError) fail(err.message, err.hints);
	throw err;
}
