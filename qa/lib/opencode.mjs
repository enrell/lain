/*
 * Bridge to `opencode run --format json`.
 *
 * The fleet needs three things from a headless agent: the assistant text, the
 * session id (so a later round can continue the *same* conversation and the
 * auditor remembers what it saw), and a readable transcript for humans.
 * Everything else in the event stream is logged and otherwise ignored, so an
 * unknown event type can never break a run.
 */
import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

export class AgentError extends Error {}

/**
 * @param {object} opts
 * @param {string} opts.binary   path to the opencode CLI (v2)
 * @param {string} opts.model    provider/model[#variant]
 * @param {string} [opts.agent]  agent id
 * @param {string} [opts.session] session id to continue
 * @param {string} opts.prompt   the user message
 * @param {string} opts.cwd      repository root
 * @param {string} [opts.transcript] file to receive the raw event stream
 * @param {number} [opts.timeoutMs]
 * @param {string[]} [opts.args] extra CLI args
 */
/**
 * Best-effort "stop thinking" for a session the harness is giving up on.
 * `opencode2 api` talks to the same background service the run used; if it is
 * unreachable the client kill still stands and the caller's quiescence check
 * catches whatever lands late.
 */
export function interruptSession(binary, sessionID, cwd) {
	try {
		const child = spawn(binary, ['api', 'POST', `/api/session/${sessionID}/interrupt`], { cwd, stdio: 'ignore' });
		child.on('error', () => {});
		child.unref?.();
	} catch {
		/* the run is already over: nothing to salvage the interrupt with */
	}
}

export async function runAgent(opts) {
	const { binary, model, agent, session, prompt, cwd, transcript, timeoutMs = 900000, args = [] } = opts;
	const argv = [binary, 'run', '--format', 'json', '--auto', ...args];
	if (agent) argv.push('--agent', agent);
	if (session) argv.push('--session', session);
	if (model) argv.push('-m', model);
	argv.push(prompt);

	const sink = transcript ? createWriteStream(transcript, { flags: 'a' }) : null;
	const child = spawn(argv[0], argv.slice(1), { cwd, env: process.env, stdio: ['ignore', 'pipe', 'pipe'] });
	if (sink) child.stderr.on('data', (chunk) => sink.write(`# stderr: ${chunk}`));

	let stdout = '';
	let stderr = '';
	let done = false;
	// The session id is needed *during* the run: on timeout the runner has to
	// interrupt the server-side turn, not just kill its own client.
	let seenSession = session || null;
	const started = Date.now();
	child.stdout.on('data', (chunk) => {
		const text = chunk.toString();
		if (!seenSession) {
			const seen = text.match(/"sessionID":"(ses_[A-Za-z0-9]+)"/);
			if (seen) seenSession = seen[1];
		}
		stdout += text;
		if (!sink) return;
		const cut = stdout.lastIndexOf('\n');
		if (cut >= 0) {
			sink.write(stdout.slice(0, cut + 1));
			stdout = stdout.slice(cut + 1);
		}
	});
	child.stderr.on('data', (chunk) => (stderr += chunk.toString()));

	const killer = setTimeout(() => {
		if (done) return;
		done = true;
		// The CLI is only the caller. The background server keeps running the
		// turn after its client dies, and a tool call landing minutes later would
		// edit the repository behind the runner's back — interrupt it first.
		if (seenSession) interruptSession(binary, seenSession, cwd);
		child.kill('SIGKILL');
	}, timeoutMs);
	killer.unref?.();

	const code = await new Promise((resolve) => child.on('close', resolve));
	clearTimeout(killer);
	if (sink) {
		if (stdout) sink.write(stdout);
		sink.end();
	}
	const events = parseEvents(stdout);
	const text = collectText(events);
	const sessionID = seenSession || events.map((e) => e.sessionID).find(Boolean) || null;

	// A killed session still has a session id, and its context survives: the
	// caller can continue it to salvage a report from what it already saw.
	if (code !== 0 && !text) {
		const err = new AgentError(
			`${agent || session || 'agent'} exited with code ${code}${done ? ' (timed out)' : ''}: ${(stderr || stdout).slice(-2000)}`
		);
		err.sessionID = sessionID;
		err.timedOut = done;
		err.partialText = text;
		throw err;
	}
	const failure = events.find((e) => e.type === 'error' && e.error);
	if (failure && !text) {
		const err = new AgentError(`${agent || 'agent'} reported an error: ${JSON.stringify(failure.error).slice(0, 1200)}`);
		err.sessionID = sessionID;
		throw err;
	}
	if (done) {
		// Timed out but produced text: report it as a soft failure so the
		// driver can still try to read an artifact and salvage the rest.
		const err = new AgentError(`${agent || session || 'agent'} ran out of time after ${Math.round((Date.now() - started) / 1000)}s`);
		err.sessionID = sessionID;
		err.timedOut = true;
		err.partialText = text;
		err.soft = true;
		throw err;
	}
	return {
		text,
		sessionID,
		agent,
		model,
		exitCode: code,
		ms: Date.now() - started,
		events: events.length,
		usage: collectUsage(events),
		tools: collectTools(events),
		commands: collectCommands(events),
	};
}

/** Shell/exec inputs, so a human can replay what the agent claims it did. */
function collectCommands(events) {
	const commands = [];
	for (const event of events) {
		if (event.type !== 'tool_use') continue;
		const part = event.part || {};
		const input = part.state?.input || {};
		const candidate = input.command || input.cmd || input.code;
		if (typeof candidate !== 'string') continue;
		commands.push(candidate.trim().slice(0, 400));
		if (commands.length >= 200) break;
	}
	return commands;
}

function parseEvents(raw) {
	const out = [];
	for (const line of raw.split('\n')) {
		const trimmed = line.trim();
		if (!trimmed.startsWith('{')) continue;
		try {
			const parsed = JSON.parse(trimmed);
			out.push(parsed);
		} catch {
			/* partial trailing line */
		}
	}
	return out;
}

/** Concatenate assistant text parts, oldest first, dropping empty deltas. */
function collectText(events) {
	const chunks = [];
	for (const event of events) {
		const part = event.part || event;
		if (event.type === 'text' && typeof part.text === 'string') chunks.push(part.text);
		else if (part.type === 'text' && typeof part.text === 'string') chunks.push(part.text);
	}
	// Some builds replay the whole message at the end; collapse exact dupes.
	const seen = new Set();
	return chunks
		.filter((chunk) => {
			const key = chunk.trim();
			if (!key || seen.has(key)) return false;
			seen.add(key);
			return true;
		})
		.join('\n\n')
		.trim();
}

/** Sum the per-step `step_finish` accounting so a run can report what it cost. */
function collectUsage(events) {
	const total = { input: 0, output: 0, reasoning: 0, cache_read: 0, cache_write: 0, cost: 0, steps: 0 };
	for (const event of events) {
		if (event.type !== 'step_finish') continue;
		const tokens = event.part?.tokens || {};
		total.steps += 1;
		total.input += tokens.input || 0;
		total.output += tokens.output || 0;
		total.reasoning += tokens.reasoning || 0;
		total.cache_read += tokens.cache?.read || 0;
		total.cache_write += tokens.cache?.write || 0;
		total.cost += event.part?.cost || 0;
	}
	return total.steps ? total : null;
}

/**
 * Tool names the agent actually used — the evidence that it drove the browser
 * rather than reading source and guessing. Errors are counted separately
 * because a denied permission should be visible in the run summary.
 */
function collectTools(events) {
	const calls = new Map();
	for (const event of events) {
		if (event.type !== 'tool_use') continue;
		const part = event.part || {};
		const name = typeof part.tool === 'string' ? part.tool : 'unknown';
		const status = part.state?.status === 'error' ? 'error' : 'ok';
		const key = `${name}:${status}`;
		calls.set(key, (calls.get(key) || 0) + 1);
	}
	return Object.fromEntries([...calls.entries()].sort());
}

/** Cheap availability probe: does the CLI exist and answer? */
export async function probeBinary(binary, cwd) {
	return new Promise((resolve) => {
		const child = spawn(binary, ['--version'], { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
		let out = '';
		child.stdout.on('data', (b) => (out += b.toString()));
		child.on('error', () => resolve({ ok: false, reason: `${binary} is not on PATH` }));
		child.on('close', (code) => resolve({ ok: code === 0, version: out.trim(), reason: code === 0 ? null : `${binary} exited ${code}` }));
		setTimeout(() => {
			child.kill('SIGKILL');
			resolve({ ok: false, reason: `${binary} did not answer within 15s` });
		}, 15000).unref?.();
	});
}

export { sleep };
