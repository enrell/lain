/*
 * The brief is what turns a specialist agent into a member of the fleet: it
 * carries the target, the browser verbs, the artifact paths and the exact JSON
 * contract. The agent's own system prompt stays about *judgement*; everything
 * operational lives here so a change to the harness never rewrites five prompts.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** Substitute {{NAME}} placeholders; an unresolved one is a harness bug. */
export function render(template, vars) {
	const out = template.replace(/\{\{(\w+)\}\}/g, (_, name) => {
		if (vars[name] === undefined) throw new Error(`brief template used unknown placeholder {{${name}}}`);
		return String(vars[name]);
	});
	return out.trim() + '\n';
}

export function readTemplate(root, name) {
	return readFileSync(join(root, 'qa', 'prompts', `${name}.md`), 'utf8');
}

export function renderTemplate(root, name, vars) {
	return render(readTemplate(root, name), vars);
}

/** Verbatim JSON contract, interpolated into the templates as an example. */
export const REPORT_EXAMPLE = {
	schema: 'lain.qa.report/1',
	seed: 'usability',
	title: 'Usability audit — Lain WebUI',
	model: 'bai/qwen3.8-flash',
	base_url: 'http://127.0.0.1:9413',
	summary: 'Three paragraphs of plain English: what you tested, how it held up, where it hurts.',
	journeys_tested: ['signed in and resumed S02E03 from the home shelf'],
	not_tested: ['backup download (destructive on a real library)'],
	findings: [
		{
			id: 'F-001',
			title: 'Scan that cannot read a library reports success',
			detail: 'With an unreadable root the run finishes as "completed" and the counter stays at zero.',
			severity: 'blocker',
			type: 'usability',
			area: 'web',
			confidence: 'high',
			route: '/settings/libraries',
			repro: ['Open /settings/libraries', 'Add a library whose path does not exist', 'Run Scan and wait for it to finish'],
			evidence: ['qa/runs/<run>/browser/usability/shots/F-001-scan-error.png', 'stats.errors was 3, the UI showed none'],
			expected: 'Name the unreadable root and offer the fix in the same place.',
			actual: 'Completed with no mention of the failure.',
			suspected_files: ['web/src/routes/settings/libraries/+page.svelte'],
		},
	],
	notes: 'Optional context the fixer should know.',
};

/** Severity ladder, quoted to every agent so the vocabulary never drifts. */
export const SEVERITY_TEXT = `blocker = a journey is impossible or data is wrong/lost ·
major = most users hit it and it degrades the product ·
minor = real but survivable · cosmetic = polish.`;

export function browserVerbs({ script, state }) {
	return `node ${script} start --state ${state}   # already running; re-run only if it died
node ${script} open  --state ${state} URL
node ${script} snapshot --state ${state} [--interactive] [--max N]
node ${script} click '@e7' --state ${state}
node ${script} fill '@e7' 'text' --state ${state}
node ${script} press Enter --state ${state}
node ${script} wait text 'Saved' --state ${state}
node ${script} shot F-001 --state ${state}            # writes ${state}/shots/F-001.png
node ${script} console --state ${state} [--errors]
node ${script} resources --state ${state} [--slow 800]
node ${script} contrast '@e7' --state ${state}
node ${script} audit --state ${state}                 # automatic a11y/layout/perf checks
node ${script} viewport 390 844 --state ${state}
node ${script} eval 'document.activeElement?.outerHTML' --state ${state}
node ${script} status --state ${state}`;
}
