/*
 * Agent-authored artifacts: validation and formatting.
 *
 * An auditor's prose is a claim; its report file is the evidence. The driver
 * refuses to act on a report that does not validate, because the fix loop
 * consumes these files, not the chat. Validation is hand-rolled (no ajv) — the
 * fleet must stay a dev tooling slice, not a new dependency.
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { atomicWrite } from './runs.mjs';

export const REPORT_SCHEMA = 'lain.qa.report/1';
export const VERDICT_SCHEMA = 'lain.qa.verdict/1';
export const SEVERITIES = ['blocker', 'major', 'minor', 'cosmetic'];
export const TYPES = ['bug', 'usability', 'accessibility', 'visual', 'responsive', 'reliability', 'polish'];
export const AREAS = ['web', 'server', 'docs', 'tooling', 'external'];
export const CONFIDENCES = ['low', 'medium', 'high'];
export const VERDICTS = ['fixed', 'not-fixed', 'partially-fixed', 'cannot-verify', 'not-a-defect', 'wont-fix'];
export const FIX_STATUSES = ['committed', 'needs-decision', 'not-reproduced', 'skipped'];
export const FIX_SCHEMA = 'lain.qa.fix/1';

const isStr = (v) => typeof v === 'string' && v.trim().length > 0;

function stringList(errors, where, value, { min = 0, max = 40 } = {}) {
	if (!Array.isArray(value)) return errors.push(`${where}: expected an array of strings`);
	if (value.length < min) errors.push(`${where}: needs at least ${min}`);
	if (value.length > max) errors.push(`${where}: too many (${value.length} > ${max})`);
	value.forEach((item, i) => {
		if (!isStr(item)) errors.push(`${where}[${i}]: empty or not a string`);
		else if (item.length > 900) errors.push(`${where}[${i}]: longer than 900 characters`);
	});
}

/**
 * @param {unknown} doc parsed report JSON
 * @param {string} seed expected seed (guards against a copied template)
 */
export function validateReport(doc, seed) {
	const errors = [];
	if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return { ok: false, errors: ['root: expected a JSON object'] };
	if (doc.schema !== REPORT_SCHEMA) errors.push(`schema: must be "${REPORT_SCHEMA}"`);
	if (seed && doc.seed !== seed) errors.push(`seed: expected "${seed}", got ${JSON.stringify(doc.seed)}`);
	if (!isStr(doc.title)) errors.push('title: required');
	if (!isStr(doc.summary)) errors.push('summary: required');
	if (!isStr(doc.model)) errors.push('model: required (copy it from your brief)');
	if (!isStr(doc.base_url)) errors.push('base_url: required (copy it from your brief)');
	stringList(errors, 'journeys_tested', doc.journeys_tested, { min: 1, max: 24 });
	if (doc.not_tested !== undefined) stringList(errors, 'not_tested', doc.not_tested, { max: 24 });
	if (!Array.isArray(doc.findings)) errors.push('findings: must be an array (empty is a valid answer)');

	const seen = new Set();
	if (!Array.isArray(doc.findings)) return { ok: false, errors };
	doc.findings.forEach((f, i) => {
		const at = `findings[${i}]`;
		if (!f || typeof f !== 'object' || Array.isArray(f)) return errors.push(`${at}: expected an object`);
		if (!isStr(f.id)) errors.push(`${at}.id: required`);
		else if (!/^F-\d{2,4}$/.test(f.id)) errors.push(`${at}.id: must look like "F-007"`);
		else if (seen.has(f.id)) errors.push(`${at}.id: duplicate ${f.id}`);
		else seen.add(f.id);
		if (!isStr(f.title)) errors.push(`${at}.title: required`);
		if (!isStr(f.detail)) errors.push(`${at}.detail: required`);
		if (!SEVERITIES.includes(f.severity)) errors.push(`${at}.severity: one of ${SEVERITIES.join(', ')}`);
		if (!TYPES.includes(f.type)) errors.push(`${at}.type: one of ${TYPES.join(', ')}`);
		if (!AREAS.includes(f.area)) errors.push(`${at}.area: one of ${AREAS.join(', ')}`);
		if (!CONFIDENCES.includes(f.confidence)) errors.push(`${at}.confidence: one of ${CONFIDENCES.join(', ')}`);
		if (!isStr(f.route)) errors.push(`${at}.route: required (app route, e.g. /settings/libraries)`);
		stringList(errors, `${at}.repro`, f.repro, { min: 1, max: 14 });
		stringList(errors, `${at}.evidence`, f.evidence, { min: 1, max: 14 });
		if (!isStr(f.expected)) errors.push(`${at}.expected: required`);
		if (f.suspected_files !== undefined) stringList(errors, `${at}.suspected_files`, f.suspected_files, { max: 12 });
		if (f.repairable !== undefined && typeof f.repairable !== 'boolean') errors.push(`${at}.repairable: boolean or omitted`);
	});

	for (const key of ['actual', 'impact', 'suggested_fix', 'notes']) {
		if (doc[key] !== undefined && typeof doc[key] !== 'string') errors.push(`${key}: string or omitted`);
	}
	return { ok: errors.length === 0, errors };
}

/** Re-test result written by an auditor asked to verify a fix. */
export function validateVerdicts(doc, seed) {
	const errors = [];
	if (!doc || typeof doc !== 'object') return { ok: false, errors: ['root: expected a JSON object'] };
	if (doc.schema !== VERDICT_SCHEMA) errors.push(`schema: must be "${VERDICT_SCHEMA}"`);
	if (seed && doc.seed !== seed) errors.push(`seed: expected "${seed}"`);
	if (!isStr(doc.model)) errors.push('model: required');
	if (!Array.isArray(doc.results)) errors.push('results: must be an array');
	(doc.results || []).forEach((r, i) => {
		const at = `results[${i}]`;
		if (!r || typeof r !== 'object') return errors.push(`${at}: expected an object`);
		if (!isStr(r.id)) errors.push(`${at}.id: required (the finding you re-tested)`);
		if (!VERDICTS.includes(r.verdict)) errors.push(`${at}.verdict: one of ${VERDICTS.join(', ')}`);
		if (!isStr(r.observations)) errors.push(`${at}.observations: required (what you saw this time)`);
		if (r.evidence !== undefined) stringList(errors, `${at}.evidence`, r.evidence, { max: 12 });
		if (r.remaining !== undefined && !Array.isArray(r.remaining)) errors.push(`${at}.remaining: array or omitted`);
	});
	return { ok: errors.length === 0, errors };
}

/** What the remediation engineer says it did this round. */
export function validateFix(doc, run, round) {
	const errors = [];
	if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return { ok: false, errors: ['root: expected a JSON object'] };
	if (doc.schema !== FIX_SCHEMA) errors.push(`schema: must be "${FIX_SCHEMA}"`);
	if (run && doc.run !== run) errors.push(`run: expected "${run}"`);
	if (doc.round !== round) errors.push(`round: expected ${round}`);
	if (!isStr(doc.branch)) errors.push('branch: required (the branch you committed to)');
	if (!isStr(doc.summary)) errors.push('summary: required (what you did this round, in English)');
	if (!Array.isArray(doc.changes)) errors.push('changes: must be an array (empty is a valid answer)');
	if (Array.isArray(doc.changes)) {
		const seen = new Set();
		doc.changes.forEach((c, i) => {
			const at = `changes[${i}]`;
			if (!c || typeof c !== 'object') {
				errors.push(`${at}: expected an object`);
				return;
			}
			if (!/^([a-z0-9-]+\/)?F-\d{2,4}$/i.test(String(c.id || ''))) errors.push(`${at}.id: must be the finding id from your brief (e.g. "usability/F-007")`);
			else if (seen.has(c.id)) errors.push(`${at}.id: duplicate ${c.id}`);
			else seen.add(c.id);
			if (!FIX_STATUSES.includes(c.status)) errors.push(`${at}.status: one of ${FIX_STATUSES.join(', ')}`);
			if (!isStr(c.verification)) errors.push(`${at}.verification: required (steps the auditor can repeat)`);
			if (c.status === 'committed') {
				if (!/^[0-9a-f]{7,40}$/.test(String(c.commit || ''))) errors.push(`${at}.commit: a committed change needs the real 7-40 hex sha you created`);
				stringList(errors, `${at}.files`, c.files, { min: 1, max: 30 });
			} else if (!isStr(c.reason)) {
				errors.push(`${at}.reason: required when the status is ${c.status}`);
			}
		});
	}
	return { ok: errors.length === 0, errors };
}

/** Load and validate an artifact the agent was supposed to leave behind. */
export function readArtifact(path, kind, seed, extra = {}) {
	if (!existsSync(path)) return { ok: false, errors: [`missing ${kind} file: ${path}`] };
	let doc;
	try {
		doc = JSON.parse(readFileSync(path, 'utf8'));
	} catch (err) {
		return { ok: false, errors: [`${path} is not valid JSON: ${err.message}`] };
	}
	const result =
		kind === 'report' ? validateReport(doc, seed) : kind === 'fix' ? validateFix(doc, extra.run, extra.round) : validateVerdicts(doc, seed);
	return { ...result, doc, path };
}

export function writeJson(path, value) {
	atomicWrite(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** Render a report as English Markdown for humans and contributors. */
export function reportToMarkdown(report, meta = {}) {
	const lines = [];
	const counts = {};
	for (const f of report.findings || []) counts[f.severity] = (counts[f.severity] || 0) + 1;
	lines.push(`# ${report.title}`);
	lines.push('');
	lines.push(`- **Run:** \`${meta.run ?? 'unknown'}\`  `);
	lines.push(`- **Seed:** \`${report.seed}\`  `);
	lines.push(`- **Model:** \`${report.model}\`  `);
	lines.push(`- **Target:** ${report.base_url}  `);
	if (meta.started_at) lines.push(`- **Session:** \`${meta.session ?? 'n/a'}\` started ${meta.started_at}`);
	lines.push(`- **Findings:** ${(report.findings || []).length} (${SEVERITIES.filter((s) => counts[s]).map((s) => `${counts[s]} ${s}`).join(', ') || 'none'})`);
	lines.push('');
	lines.push('## Summary');
	lines.push('');
	lines.push(report.summary.trim());
	lines.push('');
	if (report.journeys_tested?.length) {
		lines.push('## Journeys exercised');
		lines.push('');
		for (const j of report.journeys_tested) lines.push(`- ${j}`);
		lines.push('');
	}
	if (report.not_tested?.length) {
		lines.push('## Deliberately not covered');
		lines.push('');
		for (const j of report.not_tested) lines.push(`- ${j}`);
		lines.push('');
	}
	if ((report.findings || []).length) {
		lines.push('## Findings');
		lines.push('');
		const sorted = [...report.findings].sort((a, b) => SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity) || order(a.id) - order(b.id));
		for (const f of sorted) {
			lines.push(`### ${f.id} — ${f.title}`);
			lines.push('');
			lines.push(`**Severity:** ${f.severity} · **Type:** ${f.type} · **Area:** ${f.area} · **Confidence:** ${f.confidence} · **Route:** \`${f.route}\``);
			lines.push('');
			lines.push(f.detail.trim());
			lines.push('');
			if (f.actual) {
				lines.push(`**Observed:** ${f.actual.trim()}`);
				lines.push('');
			}
			lines.push(`**Expected:** ${f.expected.trim()}`);
			lines.push('');
			lines.push('**Reproduce:**');
			lines.push('');
			(f.repro || []).forEach((step, i) => lines.push(`${i + 1}. ${step}`));
			lines.push('');
			lines.push('**Evidence:**');
			lines.push('');
			for (const e of f.evidence || []) lines.push(`- ${e}`);
			lines.push('');
			if (f.suspected_files?.length) {
				lines.push('**Likely source:**');
				lines.push('');
				for (const file of f.suspected_files) lines.push(`- \`${file}\``);
				lines.push('');
			}
			if (f.suggested_fix) {
				lines.push(`**Suggested direction:** ${f.suggested_fix.trim()}`);
				lines.push('');
			}
			lines.push(`*Repairable by the fixer: ${f.repairable === false ? 'no (needs a human decision)' : 'yes'}*`);
			lines.push('');
		}
	} else {
		lines.push('_No findings were filed by this seed._');
		lines.push('');
	}
	if (report.notes) {
		lines.push('## Notes');
		lines.push('');
		lines.push(report.notes.trim());
		lines.push('');
	}
	return lines.join('\n');
}

/** Markdown table of the whole fleet's findings for a run. */
export function indexToMarkdown(run, rows) {
	const lines = [];
	lines.push(`# QA index — run \`${run}\``);
	lines.push('');
	if (!rows.length) {
		lines.push('_No findings across any seed._');
		return `${lines.join('\n')}\n`;
	}
	lines.push('| Finding | Seed | Severity | Type | Area | Title |');
	lines.push('| --- | --- | --- | --- | --- | --- |');
	for (const row of rows) lines.push(`| ${row.id} | ${row.seed} | ${row.severity} | ${row.type} | ${row.area} | ${row.title} |`);
	return `${lines.join('\n')}\n`;
}

export function reportPath(root, run, seed) {
	return join(root, 'qa', 'runs', run, 'reports', `${seed}.json`);
}
