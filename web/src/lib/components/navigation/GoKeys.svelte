<script lang="ts">
	/*
	 * `g` then a digit opens a top-level page (g 1 Home, g 2 Library, ...), and
	 * `g` then a settings section's letter opens that section, from anywhere
	 * (the palette and the settings rail show the chords as `g p`, `g y`, ...);
	 * they used to work only inside Settings. Typing in a field, a modifier or
	 * an open dialog is never a chord.
	 */
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { session } from '$lib/auth/session.svelte';
	import { visibleSections } from '$lib/settings/sections';
	import { pageChords } from './nav-items';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { goKeys } from './go-keys.svelte';

	let timer: ReturnType<typeof setTimeout> | undefined;

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		// The player and the reader have keys of their own (g opens the reader's page jump).
		if (/^\/(player|read)\//.test(page.url.pathname)) return;
		if (isTypingTarget(e.target as Element | null) || document.querySelector('[role="dialog"]')) {
			goKeys.pending = false;
			return;
		}
		if (goKeys.pending) {
			goKeys.pending = false;
			clearTimeout(timer);
			// A digit is a page (g 1 Home, g 2 Library, ...), a letter a settings section.
			const page = pageChords(session.isAdmin).find((p) => p.key === e.key);
			const target = page ?? visibleSections(session.isAdmin).find((s) => s.key === e.key.toLowerCase());
			if (!target) return;
			e.preventDefault();
			void goto(target.href);
			return;
		}
		if (e.key === 'g' && !e.shiftKey) {
			goKeys.pending = true;
			timer = setTimeout(() => (goKeys.pending = false), 1200);
		}
	}
</script>

<svelte:window onkeydown={onKey} />
