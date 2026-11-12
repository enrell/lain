/*
 * Enter in a form field presses the form's first submit button, and a
 * <button> (or our <Button>, which passes its attributes through) is a
 * submit button unless it says otherwise. A helper button placed before
 * the real one therefore takes over Enter: the Add library dialog's
 * "parent folder" button did, and Enter in the Name field went up a
 * folder and renamed the library instead of adding it. Every button
 * written inside a <form> states its type.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

// vitest runs from web/, as the i18n catalog test assumes.
const ROOT = process.cwd();

function svelteFiles(dir: string): string[] {
	return readdirSync(dir).flatMap((name) => {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) return svelteFiles(path);
		return name.endsWith('.svelte') ? [path] : [];
	});
}

/** Buttons inside a <form> that do not state their type, as file:line. */
function untypedFormButtons(source: string, file = ''): string[] {
	const found: string[] = [];
	for (const form of source.matchAll(/<form\b[\s\S]*?<\/form>/g)) {
		for (const button of form[0].matchAll(/<(Button|button|IconButton)\b((?:=>|[^>])*)>/g)) {
			if (/\btype=/.test(button[2])) continue;
			const at = (form.index ?? 0) + (button.index ?? 0);
			found.push(`${file}:${source.slice(0, at).split('\n').length}`);
		}
	}
	return found;
}

describe('buttons in forms', () => {
	it('finds an untyped button before the submit button', () => {
		const sample =
			'<form>\n<Button onclick={() => up()}>Up</Button>\n<Button onclick={() => go()} type="button">Go</Button>\n<Button type="submit">Add</Button>\n</form>';
		expect(untypedFormButtons(sample, 'x')).toEqual(['x:2']);
	});

	it('every button inside a form in src states its type', () => {
		const offenders = svelteFiles(join(ROOT, 'src')).flatMap((path) =>
			untypedFormButtons(readFileSync(path, 'utf8'), relative(ROOT, path))
		);
		expect(offenders).toEqual([]);
	});
});
