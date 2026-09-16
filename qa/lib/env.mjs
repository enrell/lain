/*
 * Environment contract for the Lain QA agent fleet.
 *
 * There is deliberately no fallback model: a run without an explicit
 * provider/model in `.env` fails before spending tokens, because a silent
 * default would report against a model the contributor never chose.
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export class EnvError extends Error {
	constructor(message, hints = []) {
		super(message);
		this.name = 'EnvError';
		this.hints = hints;
	}
}

/** Parse the subset of dotenv syntax the project uses (KEY=value, #comments). */
export function parseDotenv(text) {
	const out = {};
	for (const raw of text.split('\n')) {
		const line = raw.trim();
		if (!line || line.startsWith('#')) continue;
		const eq = line.indexOf('=');
		if (eq < 1) continue;
		const key = line.slice(0, eq).trim();
		const value = line.slice(eq + 1).trim();
		// A quoted value ends at its matching quote and whatever follows it is a
		// comment; an unquoted value ends at an unescaped ` #`.
		const quoted = value.match(/^(["'])([\s\S]*?)\1\s*(?:#.*)?$/);
		out[key] = quoted ? quoted[2] : value.split(/\s+#/)[0].trim();
	}
	return out;
}

export function loadFileEnv(root) {
	const file = join(root, '.env');
	if (!existsSync(file)) return { file, values: {}, present: false };
	return { file, values: parseDotenv(readFileSync(file, 'utf8')), present: true };
}

const PROVIDER_HINT = [
	'Add the model you want the fleet to run on, then authenticate:',
	'  1. Edit .env and set LAIN_AGENT_MODEL=<provider>/<model> (optionally #<variant>).',
	'  2. If that provider needs credentials, run `opencode2 auth` and sign in to it.',
	'  3. Re-run the same just recipe.',
	'There is no built-in fallback model on purpose: every report must say',
	'which model produced it, and every contributor picks their own.',
];

/**
 * Resolve the fleet configuration. Throws EnvError with actionable hints.
 * @param {string} root repository root
 */
export function resolveConfig(root, overrides = {}) {
	const { file, values, present } = loadFileEnv(root);
	const get = (key, fallback = '') => overrides[key] ?? process.env[key] ?? values[key] ?? fallback;

	const model = String(get('LAIN_AGENT_MODEL')).trim();
	if (!model) {
		throw new EnvError(
			present
				? `${file} does not define LAIN_AGENT_MODEL`
				: `no .env found at ${file}, so the fleet has no model configured`,
			PROVIDER_HINT
		);
	}
	if (!model.includes('/')) {
		throw EnvError.invalidModel(model);
	}
	const [provider, ...rest] = model.split('/');
	// `#variant` selects a reasoning profile (e.g. #high vs #max). The provider
	// lists the model without it, so the doctor looks up the bare name, while
	// the CLI still receives the whole string.
	const [modelName, ...variantParts] = rest.join('/').split('#');
	const variant = variantParts.join('#');
	if (!provider || !modelName) throw EnvError.invalidModel(model);

	const binary = String(get('LAIN_AGENT_OPENCODE', 'opencode2')).trim();
	const base = String(get('LAIN_AGENT_BASE_URL', 'http://127.0.0.1:9360')).trim().replace(/\/$/, '');
	const maxRounds = intOf(get('LAIN_AGENT_MAX_ROUNDS', '3'), 3);
	if (maxRounds < 1 || maxRounds > 10) {
		throw new EnvError(`LAIN_AGENT_MAX_ROUNDS must be between 1 and 10 (got ${maxRounds})`, [
			'This is the fix/verify convergence budget for `just agent-fix`.',
		]);
	}

	const seeds = csv(get('LAIN_AGENT_SEEDS', 'usability,ui,a11y,responsive,reliability'));
	const known = ['usability', 'ui', 'a11y', 'responsive', 'reliability'];
	const unknown = seeds.filter((seed) => !known.includes(seed));
	if (unknown.length) {
		throw new EnvError(`LAIN_AGENT_SEEDS has unknown seed(s): ${unknown.join(', ')}`, [
			`Valid seeds are: ${known.join(', ')}.`,
			'A seed is one specialist agent; add a new one by writing qa/agents/lain-qa-<seed>.md.',
		]);
	}

	const browserMode = String(get('LAIN_AGENT_BROWSER', 'auto')).trim().toLowerCase();
	if (!['auto', 'cdp', 'integrated'].includes(browserMode)) {
		throw new EnvError(`LAIN_AGENT_BROWSER must be auto, cdp or integrated (got "${browserMode}")`, [
			'cdp drives Chromium through qa/browser.mjs and works anywhere.',
			'integrated uses OpenCode\'s own browser tools, which need the desktop client attached.',
		]);
	}

	const blocking = csv(get('LAIN_AGENT_BLOCKING', 'blocker,major'));
	const badSeverity = blocking.filter((s) => !['blocker', 'major', 'minor', 'cosmetic'].includes(s));
	if (badSeverity.length) {
		throw new EnvError(`LAIN_AGENT_BLOCKING contains unknown severities: ${badSeverity.join(', ')}`, [
			'Choose from blocker, major, minor, cosmetic.',
		]);
	}

	const viewport = String(get('LAIN_AGENT_VIEWPORT', '1440x900')).toLowerCase();
	const [width, height] = viewport.split('x').map((n) => intOf(n, 0));
	if (!width || !height || width < 240 || height < 240) {
		throw new EnvError(`LAIN_AGENT_VIEWPORT must be WIDTHxHEIGHT, got "${viewport}"`, ['Example: LAIN_AGENT_VIEWPORT=1440x900']);
	}

	// A named target turns the fleet into an observer of an instance the
	// contributor already runs; empty means "boot a disposable one".
	const external = String(get('LAIN_AGENT_TARGET_URL', '')).trim().replace(/\/$/, '');
	if (external) {
		// On a disposable instance the fleet invents its own accounts. On someone
		// else's server it cannot: without real credentials every specialist
		// would audit the login screen and report nothing.
		if (!String(get('LAIN_AGENT_ADMIN_PASSWORD', '')).trim()) {
			throw new EnvError(`LAIN_AGENT_TARGET_URL is set but LAIN_AGENT_ADMIN_PASSWORD is empty`, [
				`An external target is not seeded by the fleet, so you must name an account that exists:`,
				`  LAIN_AGENT_ADMIN_USER=<your admin> LAIN_AGENT_ADMIN_PASSWORD=<its password>`,
				`Prefer a throwaway instance: unset LAIN_AGENT_TARGET_URL and let the fleet boot one.`,
			]);
		}
	}

	return {
		envFile: present ? file : null,
		binary,
		model,
		provider,
		modelName,
		variant,
		fixerModel: String(get('LAIN_AGENT_MODEL_FIXER', model)).trim() || model,
		auditorModel: String(get('LAIN_AGENT_MODEL_AUDITOR', model)).trim() || model,
		base,
		external,
		maxRounds,
		timeoutMs: intOf(get('LAIN_AGENT_TIMEOUT_MS', '1500000'), 1500000),
		fixTimeoutMs: intOf(get('LAIN_AGENT_FIX_TIMEOUT_MS', '1800000'), 1800000),
		verifyTimeoutMs: intOf(get('LAIN_AGENT_VERIFY_TIMEOUT_MS', '900000'), 900000),
		retries: intOf(get('LAIN_AGENT_RETRIES', '1'), 1),
		parallel: get('LAIN_AGENT_PARALLEL', '0') === '1',
		seeds,
		blocking,
		includeLowConfidence: get('LAIN_AGENT_INCLUDE_LOW', '0') === '1',
		browserMode,
		chromium: String(get('LAIN_AGENT_CHROMIUM', 'chromium')).trim(),
		width,
		height,
		instance: external ? 'external' : get('LAIN_AGENT_INSTANCE', 'disposable'),
		branch: String(get('LAIN_AGENT_BRANCH', '')).trim() || defaultBranch(),
		branchOverride: overrides.LAIN_AGENT_BRANCH || process.env.LAIN_AGENT_BRANCH || '',
		keepInstance: get('LAIN_AGENT_KEEP_INSTANCE', '0') === '1',
		admin: {
			username: get('LAIN_AGENT_ADMIN_USER', 'qa-admin'),
			password: get('LAIN_AGENT_ADMIN_PASSWORD', '') || generatedPassword(),
		},
		member: {
			username: get('LAIN_AGENT_MEMBER_USER', 'qa-member'),
			password: get('LAIN_AGENT_MEMBER_PASSWORD', '') || generatedPassword(),
		},
	};
}

function csv(raw) {
	return String(raw)
		.split(',')
		.map((item) => item.trim())
		.filter(Boolean);
}

function defaultBranch() {
	const d = new Date();
	const pad = (n) => String(n).padStart(2, '0');
	return `agent/qa/${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}-${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}`;
}

function intOf(raw, fallback) {
	const n = Number.parseInt(String(raw), 10);
	return Number.isFinite(n) ? n : fallback;
}

function generatedPassword() {
	// Disposable instance credentials: entropy here is a convenience, not a
	// security boundary, so it only has to survive one scan and a login.
	return `qa-${Math.random().toString(36).slice(2, 10)}-${Date.now().toString(36)}`;
}

export function validateModelRef(ref) {
	if (typeof ref !== 'string' || !ref.includes('/')) return false;
	const [provider, model] = ref.split('/');
	return Boolean(provider && model);
}

EnvError.invalidModel = (model) =>
	new EnvError(`LAIN_AGENT_MODEL must be provider/model (optionally #variant), got "${model}"`, [
		'Examples: bai/qwen3.8-flash, anthropic/claude-sonnet-4-5#high',
		'List what your configured providers expose with `opencode2 models`.',
	]);
