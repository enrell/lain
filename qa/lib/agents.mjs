/*
 * Agent roster and installation.
 *
 * `.opencode/` is gitignored in this repository, so the tracked copies of the
 * specialists live in `qa/agents/` and `just agent-sync` installs them. That
 * keeps contributor-onboarding explicit (`clone -> just agent-sync`) and keeps
 * personal agent experiments out of Git.
 */
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export const AGENT_SRC = 'qa/agents';
export const AGENT_DEST = '.opencode/agents';

/** Seeds the fleet runs by default: one specialist per modality. */
export const SEEDS = ['usability', 'ui', 'a11y', 'responsive', 'reliability'];

export function agentFile(root, name) {
	return join(root, AGENT_SRC, `${name}.md`);
}

export function fleetAgents(root) {
	if (!existsSync(join(root, AGENT_SRC))) return [];
	return readdirSync(join(root, AGENT_SRC))
		.filter((f) => f.endsWith('.md'))
		.map((f) => f.replace(/\.md$/, ''))
		.sort();
}

/** Parse the `description:` frontmatter value (folded block or single line). */
export function agentDescription(file) {
	const text = readFileSync(file, 'utf8');
	const match = text.match(/^description:\s*(.*)$/m);
	if (!match) return null;
	if (match[1].trim().startsWith('>-')) {
		const lines = [];
		for (const line of text.split('\n').slice(text.split('\n').findIndex((l) => l.startsWith('description:')) + 1)) {
			if (/^\S/.test(line)) break;
			lines.push(line.trim());
		}
		return lines.join(' ').trim();
	}
	return match[1].trim();
}

export function auditAgent(seed) {
	return `lain-qa-${seed}`;
}

export function isAuditor(agent) {
	return SEEDS.some((seed) => auditAgent(seed) === agent);
}

/**
 * Copy tracked agents into the ignored discovery directory, replacing stale
 * copies of our own files only. Returns the actions taken.
 */
export function installAgents(root, { log = () => {} } = {}) {
	const src = join(root, AGENT_SRC);
	const dest = join(root, AGENT_DEST);
	mkdirSync(dest, { recursive: true });
	const actions = [];
	for (const name of fleetAgents(root)) {
		const from = join(src, `${name}.md`);
		const to = join(dest, `${name}.md`);
		const needs = !existsSync(to) || statSync(to).mtimeMs < statSync(from).mtimeMs || readFileSync(to, 'utf8') !== readFileSync(from, 'utf8');
		if (needs) {
			copyFileSync(from, to);
			actions.push({ name, action: 'installed' });
			log(`installed ${name} -> ${AGENT_DEST}/${name}.md`);
		} else {
			actions.push({ name, action: 'current' });
		}
	}
	return { dest, actions };
}

/** Remove our installed copies (leaves anything the contributor added alone). */
export function uninstallAgents(root) {
	const removed = [];
	for (const name of fleetAgents(root)) {
		const to = join(root, AGENT_DEST, `${name}.md`);
		if (existsSync(to)) {
			rmSync(to);
			removed.push(name);
		}
	}
	return removed;
}

export function writeStub(path, contents) {
	mkdirSync(path.split('/').slice(0, -1).join('/'), { recursive: true });
	writeFileSync(path, contents);
}
