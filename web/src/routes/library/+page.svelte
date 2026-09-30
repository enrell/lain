<script lang="ts">
	import { onMount } from 'svelte';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import LibraryBrowser from '$lib/components/media/LibraryBrowser.svelte';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let libraries = $state<Library[]>([]);

	onMount(() => {
		void ensureLibraries()
			.then((libs) => (libraries = libs))
			.catch(() => {
				// The browser itself reports catalog failures; the
				// filter degrades to "all libraries" if this fails.
			});
	});

	// Scanning is an action, not a setting: it lives where the media is.
	async function rescan(): Promise<void> {
		if (!session.isAdmin || scan.running) return;
		try {
			await scan.start();
			toasts.success('Scan started.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not start the scan.'));
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || e.key !== 's') return;
		const t = e.target as HTMLElement | null;
		if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return;
		if (document.querySelector('[role="dialog"]')) return;
		void rescan();
	}
</script>

<svelte:window onkeydown={onKey} />

<svelte:head><title>Library — Lain</title></svelte:head>

<div class="space-y-6">
	<header class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="text-xl font-semibold tracking-tight text-foreground">Library</h1>
			<p class="mt-0.5 text-sm text-muted">Everything Lain has indexed.</p>
		</div>
		{#if session.isAdmin}
			<div class="flex items-center gap-1 font-mono text-[11px] uppercase tracking-[0.14em]">
				<button
					type="button"
					class="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground disabled:opacity-60"
					disabled={scan.running}
					onclick={() => void rescan()}
				>
					{scan.running ? 'Scanning…' : 'Scan'} <kbd class="text-muted/70">S</kbd>
				</button>
				<a href="/settings/libraries" class="rounded-md px-2.5 py-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground">
					Manage ›
				</a>
			</div>
		{/if}
	</header>
	<LibraryBrowser {libraries} showLibraryFilter />
</div>
