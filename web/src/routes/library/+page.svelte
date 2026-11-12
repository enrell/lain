<script lang="ts">
	import { onMount, tick } from 'svelte';
	import LockIcon from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import LibraryBrowser from '$lib/components/media/LibraryBrowser.svelte';
	import LibraryShelves from '$lib/components/media/LibraryShelves.svelte';
	import UnlockLibraryDialog from '$lib/components/media/UnlockLibraryDialog.svelte';
	import { t } from '$lib/i18n';
	import { api } from '$lib/api';
	import { ensureLibraries, nameable } from '$lib/stores/media-cache.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let libraries = $state<Library[]>([]);
	// Private libraries this session has not unlocked (D-156, lain-rs).
	const locked = $derived(libraries.filter((l) => l.private && l.locked));
	const open = $derived(libraries.filter((l) => l.private && !l.locked));
	// What the browser may name: a locked library is not on the page at all.
	const visible = $derived(nameable(libraries));
	// Shelves (one row per library) or every title in one grid. The choice
	// is a per-browser convenience; shelves need more than one library.
	const VIEW_KEY = 'lain.library.view';
	let view = $state<'shelves' | 'all'>('shelves');
	async function setView(next: 'shelves' | 'all', refocus = false): Promise<void> {
		view = next;
		try {
			localStorage.setItem(VIEW_KEY, next);
		} catch {
			/* private window: the choice lasts for this visit */
		}
		// The other view brings its own copy of the switch; focus follows it
		// rather than falling back to the page.
		if (!refocus) return;
		await tick();
		document.querySelector<HTMLElement>('#library-view [aria-pressed="true"]')?.focus();
	}
	let unlockOpen = $state(false);
	// Re-mounts the browser so it refetches once a library unlocks.
	let generation = $state(0);

	// One quiet control for private libraries: it never names them or says
	// how many there are. Locked, it asks for the PIN and opens them all;
	// open, it locks them all again at once.
	async function toggleLock(): Promise<void> {
		if (locked.length > 0) {
			unlockOpen = true;
			return;
		}
		if (open.length === 0) return;
		try {
			for (const lib of open) await api.libraries.lock(lib.id);
			toasts.success(t('privateLibrary.lockedAllToast'));
		} catch (err) {
			toasts.error(errorMessage(err, t('privateLibrary.updateFailed')));
		}
		await unlocked();
	}

	async function unlocked(): Promise<void> {
		libraries = await ensureLibraries(true);
		generation += 1;
	}

	onMount(() => {
		try {
			if (localStorage.getItem(VIEW_KEY) === 'all') view = 'all';
		} catch {
			/* default view */
		}
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
		if (e.ctrlKey || e.metaKey || e.altKey || (e.key !== 's' && e.key !== 'u' && e.key !== 'v')) return;
		const target = e.target as HTMLElement | null;
		if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return;
		if (document.querySelector('[role="dialog"]')) return;
		if (e.key === 'v') {
			if (visible.length > 1) {
				e.preventDefault();
				void setView(view === 'shelves' ? 'all' : 'shelves');
			}
			return;
		}
		if (e.key === 'u') {
			if (locked.length > 0 || open.length > 0) {
				e.preventDefault();
				void toggleLock();
			}
			return;
		}
		void rescan();
	}
</script>

<svelte:window onkeydown={onKey} />

<svelte:head><title>Library — Lain</title></svelte:head>

<div class="space-y-6">
	<header class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="page-title">Library</h1>
			<p class="mt-0.5 text-sm text-muted max-md:hidden">Everything Lain has indexed.</p>
		</div>
		{#if session.isAdmin || locked.length > 0 || open.length > 0}
			<!-- On a phone each action is a chip on one line of its own words: "Manage
			     libraries" broke over two lines inside its box. -->
			<div class="flex items-center gap-1 whitespace-nowrap text-sm max-md:w-full max-md:flex-wrap max-md:gap-2 max-md:[&>*]:min-h-10 max-md:[&>*]:rounded-lg max-md:[&>*]:border max-md:[&>*]:border-line max-md:[&>*]:px-3.5 max-md:[&>a]:flex max-md:[&>a]:items-center">
				{#if locked.length > 0 || open.length > 0}
					<button
						type="button"
						class="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
						title={locked.length > 0 ? t('privateLibrary.unlockHint') : t('privateLibrary.lockHint')}
						aria-label={locked.length > 0 ? t('privateLibrary.unlockHint') : t('privateLibrary.lockHint')}
						onclick={() => void toggleLock()}
					>
						{#if locked.length > 0}<LockIcon class="size-3.5" />{:else}<LockOpen class="size-3.5 text-accent" />{/if}
						{locked.length > 0 ? t('privateLibrary.unlock') : t('privateLibrary.lock')}
						<kbd class="text-muted/70">U</kbd>
					</button>
				{/if}
				{#if session.isAdmin}
					<button
						type="button"
						class="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground disabled:opacity-60"
						disabled={scan.running}
						onclick={() => void rescan()}
					>
						{scan.running ? 'Scanning…' : 'Scan'} <kbd class="text-muted/70">S</kbd>
					</button>
					<a href="/settings/libraries" class="rounded-md px-2.5 py-1.5 text-muted transition-colors hover:bg-surface-hover hover:text-foreground">
						Manage libraries
					</a>
				{/if}
			</div>
		{/if}
	</header>
	{#key `${generation}:${visible.map((l) => l.id).join(',')}`}
		{#if visible.length > 1 && view === 'shelves'}
			<LibraryShelves libraries={visible} view={viewSwitch} />
		{:else}
			<LibraryBrowser libraries={visible} showLibraryFilter={visible.length > 1} view={visible.length > 1 ? viewSwitch : undefined} />
		{/if}
	{/key}
</div>

<!-- Rides at the right end of each view's sticky bar, so it stays in reach
     however far the page is scrolled; above the content it scrolled away. The
     track is the field colour with a hairline: on host palettes where the
     surface is the page colour, a surface track vanished and the unpicked half
     read as loose text beside a button. -->
{#snippet viewSwitch()}
	<div class="ms-auto flex shrink-0 items-center gap-2">
		<div id="library-view" class="inline-flex gap-0.5 rounded-lg bg-field p-0.5 text-[13px] ring-1 ring-hairline" role="group" aria-label={t('library.view')}>
			{#each [['shelves', t('library.byLibrary')], ['all', t('library.allTitles')]] as const as [id, label] (id)}
				<button
					type="button"
					aria-pressed={view === id}
					class={['rounded-md px-2.5 py-2 transition-colors md:py-1', view === id ? 'bg-surface-active font-semibold text-foreground ring-1 ring-accent/50' : 'text-muted hover:bg-surface-hover hover:text-foreground'].join(' ')}
					onclick={() => void setView(id, true)}
				>
					{label}
				</button>
			{/each}
		</div>
		<kbd class="hidden text-muted/70 md:inline-flex" title={t('library.viewKey')}>V</kbd>
	</div>
{/snippet}

<UnlockLibraryDialog bind:open={unlockOpen} libraries={locked} onunlocked={() => void unlocked()} />
