<script lang="ts">
	import { onMount } from 'svelte';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import Folder from '@lucide/svelte/icons/folder';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import ChevronUp from '@lucide/svelte/icons/chevron-up';
	import Pencil from '@lucide/svelte/icons/pencil';
	import ScanLine from '@lucide/svelte/icons/scan-line';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import LockIcon from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import type { BrowseDir, Library, LibraryRoot } from '$lib/api/types';
	import { api } from '$lib/api';
	import { t } from '$lib/i18n';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import UnlockLibraryDialog from '$lib/components/media/UnlockLibraryDialog.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { ensureLibraries, forgetLibrary, rememberLibrary } from '$lib/stores/media-cache.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { focusSoon } from '$lib/components/social/focus';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { formatRelative } from '$lib/utilities/format';

	let libraries = $state<Library[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	// The dialog adds a library, or edits this one (same fields, same picker).
	let editTarget = $state<Library | null>(null);
	let formError = $state<string | null>(null);
	let form = $state({ name: '', type: 'auto', path: '', private: false });
	// The name follows the folder until the user types one.
	let nameTouched = $state(false);

	let deleteTarget = $state<Library | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state(false);

	// lain-rs features (D-156, D-157): null until the server proves it
	// serves them (the Go server has no /api/roots).
	let roots = $state<LibraryRoot[] | null>(null);
	let pinSet = $state(false);
	let pinForm = $state({ pin: '', password: '' });
	let pinBusy = $state(false);
	let unlockTarget = $state<Library | null>(null);
	let unlockOpen = $state(false);

	async function loadExtras(): Promise<void> {
		try {
			roots = (await api.libraries.roots()).roots;
			pinSet = (await api.libraries.pin.get()).set;
		} catch {
			roots = null;
		}
	}

	function replace(lib: Library): void {
		libraries = libraries.map((l) => (l.id === lib.id ? lib : l));
		rememberLibrary(lib);
	}

	async function setPrivate(lib: Library, next: boolean): Promise<void> {
		try {
			const updated = await api.libraries.update(lib.id, { private: next });
			replace(updated);
			toasts.success(t(next ? 'privateLibrary.madePrivate' : 'privateLibrary.madePublic', { name: updated.name }));
		} catch (err) {
			toasts.error(errorMessage(err, t('privateLibrary.updateFailed')));
		}
	}

	async function toggleLock(lib: Library): Promise<void> {
		if (!lib.private) return;
		if (lib.locked) {
			unlockTarget = lib;
			unlockOpen = true;
			return;
		}
		try {
			const updated = await api.libraries.lock(lib.id);
			replace(updated);
			toasts.success(t('privateLibrary.lockedToast', { name: updated.name }));
		} catch (err) {
			toasts.error(errorMessage(err, t('privateLibrary.updateFailed')));
		}
	}

	async function savePin(clear: boolean): Promise<void> {
		pinBusy = true;
		try {
			pinSet = clear
				? (await api.libraries.pin.clear(pinForm.password)).set
				: (await api.libraries.pin.set(pinForm.pin, pinForm.password)).set;
			pinForm = { pin: '', password: '' };
			toasts.success(t(clear ? 'privateLibrary.pinCleared' : 'privateLibrary.pinSaved'));
		} catch (err) {
			toasts.error(errorMessage(err, t('privateLibrary.pinFailed')));
		} finally {
			pinBusy = false;
		}
	}

	usePageKeys(() => [
		...(roots !== null ? [{ keys: ['P'], label: t('pageKeys.libraryPrivate') }] : []),
		{ keys: ['U'], label: t('pageKeys.libraryLock') },
		{ keys: ['E'], label: t('settings.librariesPage.keyEdit') },
		{ keys: ['Alt', '↑↓'], label: t('settings.librariesPage.keyMove') },
		{ keys: ['Delete'], label: t('pageKeys.libraryRemove') }
	]);

	// Row keys: P toggles private, U unlocks or locks, E edits, Alt+arrows
	// move it in the order, Delete removes.
	function rowKey(e: KeyboardEvent, lib: Library): void {
		if (e.target !== e.currentTarget || e.ctrlKey || e.metaKey) return;
		if (e.altKey) {
			if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
				e.preventDefault();
				void move(lib, e.key === 'ArrowUp' ? -1 : 1, true);
			}
			return;
		}
		const key = e.key.toLowerCase();
		if (key === 'e') {
			e.preventDefault();
			openEdit(lib);
		} else if (key === 'p' && roots !== null) {
			e.preventDefault();
			void setPrivate(lib, !lib.private);
		} else if (key === 'u' && lib.private) {
			e.preventDefault();
			void toggleLock(lib);
		} else if (e.key === 'Delete') {
			e.preventDefault();
			askRemove(lib);
		}
	}

	// The folder picker: the path bar is the choice. Opening a folder moves
	// the bar there; a typed path is checked by listing it.
	let browsePath = $state('');
	let browseParent = $state('');
	let browseDirs = $state<BrowseDir[]>([]);
	let browseLoading = $state(false);
	let browseError = $state<string | null>(null);
	let pathDraft = $state('');
	let cursor = $state(0);
	let listEl = $state<HTMLUListElement | null>(null);
	let pathEl = $state<HTMLInputElement | null>(null);

	function openCreate(): void {
		editTarget = null;
		createOpen = true;
		formError = null;
		form = { name: '', type: 'auto', path: '', private: false };
		nameTouched = false;
		pathDraft = '';
		void loadBrowse();
		// The dialog opens on what it asks for first: the folder.
		focusSoon(() => document.getElementById('library-path'));
	}

	// Editing opens the same dialog on the library as it is: its folder in
	// the picker, its name kept (it does not follow the folder any more).
	function openEdit(lib: Library): void {
		editTarget = lib;
		createOpen = true;
		formError = null;
		form = { name: lib.name, type: lib.type, path: lib.path, private: !!lib.private };
		nameTouched = true;
		pathDraft = lib.path;
		void loadBrowse(lib.path);
		focusSoon(() => document.getElementById('library-name'));
	}

	// Moves a library one place up or down in the server's order (Home and
	// the library page follow it). The list moves at once; a failure puts
	// it back.
	async function move(lib: Library, delta: number, refocus = false): Promise<void> {
		const from = libraries.findIndex((l) => l.id === lib.id);
		const to = from + delta;
		if (from < 0 || to < 0 || to >= libraries.length) return;
		const before = libraries;
		const next = [...libraries];
		[next[from], next[to]] = [next[to], next[from]];
		libraries = next;
		if (refocus) requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-library="${lib.id}"]`)?.focus());
		try {
			libraries = await api.libraries.order(next.map((l) => l.id));
			void ensureLibraries(true);
		} catch (err) {
			libraries = before;
			toasts.error(errorMessage(err, t('settings.librariesPage.moveFailed')));
		}
	}

	function folderName(path: string): string {
		return path.split(/[\\/]/).filter(Boolean).pop() ?? '';
	}

	async function loadBrowse(path?: string): Promise<void> {
		browseLoading = true;
		browseError = null;
		try {
			const res = await api.libraries.browse(path);
			browsePath = res.path;
			browseParent = res.parent;
			browseDirs = res.dirs;
			cursor = 0;
			pathDraft = res.path;
			form.path = res.path;
			formError = null;
			// Named after a folder the viewer went to, not after the one the
			// picker opens on: that one is the server's starting point, and
			// the dialog came up with a library already called "media".
			if (!nameTouched && path !== undefined) form.name = folderName(res.path);
			// A long path keeps its end, the folder being chosen, in view.
			requestAnimationFrame(() => {
				if (pathEl && document.activeElement !== pathEl) pathEl.scrollLeft = pathEl.scrollWidth;
			});
		} catch (err) {
			browseError = errorMessage(err, t('settings.librariesPage.listFailed'));
			// Keep what was typed: the server has the last word on Add.
			if (path) form.path = path;
		} finally {
			browseLoading = false;
		}
	}

	function goUp(): void {
		if (browseParent) void loadBrowse(browseParent);
	}

	function pathKey(e: KeyboardEvent): void {
		if (e.key === 'Enter') {
			// Enter here opens the typed path; it does not submit the form.
			e.preventDefault();
			void loadBrowse(pathDraft.trim() || undefined);
		} else if (e.key === 'ArrowDown' && browseDirs.length > 0) {
			e.preventDefault();
			listEl?.focus();
		}
	}

	function listKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const last = browseDirs.length - 1;
		if (e.key === 'ArrowDown') cursor = Math.min(last, cursor + 1);
		else if (e.key === 'ArrowUp') cursor = Math.max(0, cursor - 1);
		else if (e.key === 'Home') cursor = 0;
		else if (e.key === 'End') cursor = Math.max(0, last);
		else if (e.key === 'Enter' || e.key === 'ArrowRight') {
			const dir = browseDirs[cursor];
			if (dir) void loadBrowse(dir.path);
		} else if (e.key === 'Backspace' || e.key === 'ArrowLeft') goUp();
		else return;
		e.preventDefault();
		listEl?.querySelector(`[data-index="${cursor}"]`)?.scrollIntoView({ block: 'nearest' });
	}

	function askRemove(lib: Library): void {
		deleteTarget = lib;
		deleteOpen = true;
		focusSoon(() => document.getElementById('confirm-remove-library'));
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			libraries = await ensureLibraries(true);
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		// Ctrl+K "Add library" lands here with ?add.
		if (new URL(window.location.href).searchParams.has('add')) openCreate();
		void load();
		void loadExtras();
		void scan.watch();
	});

	// What an edit changes; a new folder or type is read by the next scan.
	const editPatch = $derived.by(() => {
		const lib = editTarget;
		if (!lib) return {};
		const patch: { name?: string; type?: string; path?: string; private?: boolean } = {};
		if (form.name.trim() && form.name.trim() !== lib.name) patch.name = form.name.trim();
		if (form.type !== lib.type) patch.type = form.type;
		if (form.path.trim() && form.path.trim() !== lib.path) patch.path = form.path.trim();
		if (roots !== null && form.private !== !!lib.private) patch.private = form.private;
		return patch;
	});
	const editNeedsScan = $derived(editPatch.type !== undefined || editPatch.path !== undefined);

	async function saveEdit(scanAfter: boolean): Promise<void> {
		const lib = editTarget;
		if (!lib) return;
		creating = true;
		formError = null;
		try {
			// Private first: a library made private must not be looked up
			// online by the scan that follows.
			const { private: priv, ...rest } = editPatch;
			let updated = lib;
			if (priv === true) updated = await api.libraries.update(lib.id, { private: true });
			if (Object.keys(rest).length > 0) updated = await api.libraries.update(lib.id, rest);
			if (priv === false) updated = await api.libraries.update(lib.id, { private: false });
			replace(updated);
			createOpen = false;
			editTarget = null;
			if (!scanAfter) {
				toasts.success(t('settings.librariesPage.saved', { name: updated.name }));
				return;
			}
			try {
				const how = await scan.start();
				toasts.success(t(how === 'queued' ? 'settings.librariesPage.savedQueued' : 'settings.librariesPage.savedScanning', { name: updated.name }));
			} catch (err) {
				toasts.success(t('settings.librariesPage.saved', { name: updated.name }));
				toasts.error(errorMessage(err, t('scan.startFailed')));
			}
		} catch (err) {
			formError = errorMessage(err);
		} finally {
			creating = false;
		}
	}

	async function create(scanAfter: boolean): Promise<void> {
		if (editTarget) return saveEdit(scanAfter);
		creating = true;
		formError = null;
		try {
			let lib = await api.libraries.create({
				name: form.name.trim(),
				type: form.type,
				path: form.path.trim()
			});
			// Private before the first scan: a scan of a public library may
			// look its titles up online.
			if (form.private) lib = await api.libraries.update(lib.id, { private: true });
			rememberLibrary(lib);
			// The server's order: a new library goes after the ordered ones.
			libraries = await ensureLibraries(true).catch(() => [...libraries, lib]);
			createOpen = false;
			form = { name: '', type: 'auto', path: '', private: false };
			if (!scanAfter) {
				toasts.success(t('scan.libraryAdded', { name: lib.name }));
				return;
			}
			// The dialog is closed by now: a failed scan says so in a toast,
			// not in the form nobody can see.
			try {
				const how = await scan.start();
				toasts.success(t(how === 'queued' ? 'scan.libraryAddedQueued' : 'scan.libraryAdded', { name: lib.name }));
			} catch (err) {
				toasts.success(t('scan.libraryAdded', { name: lib.name }));
				toasts.error(errorMessage(err, t('scan.startFailed')));
			}
		} catch (err) {
			formError = errorMessage(err);
		} finally {
			creating = false;
		}
	}

	async function remove(): Promise<void> {
		const target = deleteTarget;
		if (!target) return;
		deleting = true;
		try {
			await api.libraries.remove(target.id);
			libraries = libraries.filter((lib) => lib.id !== target.id);
			forgetLibrary(target.id);
			deleteOpen = false;
			deleteTarget = null;
				toasts.success(t('settings.librariesPage.removed', { name: target.name }));
			} catch (err) {
				toasts.error(errorMessage(err, t('settings.librariesPage.removeFailed')));
		} finally {
			deleting = false;
		}
	}

	async function startScan(): Promise<void> {
		try {
			if ((await scan.start()) === 'queued') toasts.success(t('scan.queued'));
		} catch (err) {
			toasts.error(errorMessage(err, t('scan.startFailed')));
		}
	}

	// Auto takes videos and comic archives alike; the others only filter
	// which files of the folder count.
	const TYPES = ['auto', 'anime', 'series', 'movie', 'manga', 'comic'] as const;
	const typeOptions = $derived([
		...TYPES.map((value) => ({ value, label: t(`settings.librariesPage.types.${value}`) })),
		// A type set elsewhere (the API, an older client) stays selectable as it is.
		...(form.type && !(TYPES as readonly string[]).includes(form.type) ? [{ value: form.type, label: form.type }] : [])
	]);
	function typeLabel(type: string): string {
		return (TYPES as readonly string[]).includes(type) ? t(`settings.librariesPage.types.${type as (typeof TYPES)[number]}`) : type;
	}
</script>

<svelte:head><title>{t('settings.librariesPage.pageTitle')}</title></svelte:head>

<div>
	<SettingsGroup id="scan" title={t('settings.librariesPage.scanTitle')} description={t('settings.groups.libraries.scan')}>
		{#snippet aside()}
			<!-- With no library there is nothing to walk: Add library is the step.
			     While a scan runs, the button queues one more on the server (D-286),
			     once: what is added meanwhile is read by that one. -->
			<Button
				size="field"
				variant={scan.running ? 'tonal' : 'primary'}
				onclick={() => void startScan()}
				disabled={scan.queued || (!loading && libraries.length === 0)}
			>
				<ScanLine class="size-4" />
				{scan.queued ? t('settings.librariesPage.scanQueued') : scan.running ? t('settings.librariesPage.scanAfter') : t('settings.librariesPage.scanNow')}
			</Button>
		{/snippet}
		<div class="py-4 text-sm">
			{#if scan.running}
				{@const fraction = scan.fraction}
				<div class="flex items-center gap-2 text-accent" role="status">
					<Spinner class="size-4" />
					<span class="tabular-nums">{scan.line}</span>
					{#if scan.elapsed}
						<span class="font-mono text-xs tabular-nums text-muted">
							{t('scan.elapsed', { time: scan.elapsed })}
						</span>
					{/if}
				</div>
				{#if scan.status?.progress?.phase === 'enrich'}
					<!-- This phase asks the network about every title and can take
					     long; nothing waits on it. -->
					<p class="mt-2 text-xs text-muted">{t('scan.enrichHint')}</p>
				{/if}
				{#if scan.queued}
					<p class="mt-1 text-xs text-muted">{t('scan.queuedLine')}</p>
				{/if}
				<!-- Indeterminate (full, pulsing) until the phase has a total. -->
				<div
					class="mt-2 h-0.5 w-full max-w-md overflow-hidden rounded-full bg-hairline"
					role="progressbar"
					aria-label={t('scan.progress')}
					aria-valuemin={0}
					aria-valuemax={100}
					aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
				>
					<div
						class="h-full bg-accent transition-[width] duration-500 {fraction === null ? 'animate-pulse' : ''}"
						style:width="{fraction === null ? 100 : fraction * 100}%"
					></div>
				</div>
			{:else if scan.status?.state === 'error'}
				<p class="text-danger" role="alert">{t('settings.librariesPage.scanFailed', { error: scan.status.error ?? '' })}</p>
			{:else if scan.status?.state === 'done' && scan.status.stats}
				{@const stats = scan.status.stats}
				<p class="text-muted">
					<!-- One sentence for when and who, then the counts, each its own phrase. -->
					{scan.status.finished_at
						? t(scan.status.trigger === 'watch' ? 'settings.librariesPage.lastScanWatch' : 'settings.librariesPage.lastScan', { when: formatRelative(scan.status.finished_at) })
						: t('settings.librariesPage.lastScanDone')}
					<span class="text-foreground">{t('settings.librariesPage.statIdentified', { count: stats.identified })}</span> ·
					<span class="text-foreground">{t('settings.librariesPage.statUnidentified', { count: stats.unidentified })}</span> ·
					<span class="text-foreground">{t('settings.librariesPage.statEnriched', { count: stats.enriched })}</span>
					{#if stats.missing > 0}· <span class="text-warning">{t('settings.librariesPage.statMissing', { count: stats.missing })}</span>{/if}
					{#if stats.restored > 0}· <span class="text-foreground">{t('settings.librariesPage.statRestored', { count: stats.restored })}</span>{/if}
					{#if stats.walk_errors > 0}· <span class="text-warning">{t('settings.librariesPage.statUnreadable', { count: stats.walk_errors })}</span>{/if}
					{#if stats.errors > 0}· <span class="text-danger">{t('settings.librariesPage.statErrors', { count: stats.errors })}</span>{/if}
				</p>
				<!-- A count nobody can attribute is not a task: name the roots
				     the server could not read, and what to check next. -->
				{#if stats.unreadable?.length}
					<ul class="mt-2 space-y-1.5 text-sm" aria-label={t('settings.librariesPage.unreadableRoots')}>
						{#each stats.unreadable as root (root.library_id)}
							<li class="text-warning">
								<span class="font-medium">{root.name}</span>
								<span class="font-mono text-xs text-muted">{root.path}</span>
								— {t('settings.librariesPage.unreadableHint', { reason: root.reason })}
							</li>
						{/each}
					</ul>
				{/if}
			{:else}
				<p class="text-muted">{t('settings.librariesPage.noScan')}</p>
			{/if}
		</div>
	</SettingsGroup>

	<SettingsGroup id="libraries" title={t('settings.librariesPage.librariesTitle')} description={t('settings.groups.libraries.libraries')}>
		{#snippet aside()}
			<Button variant="tonal" size="field" onclick={() => openCreate()}>
				<FolderPlus class="size-4" /> {t('settings.librariesPage.add')}
			</Button>
		{/snippet}
		{#if loading}
			<div class="py-4"><div class="h-12 animate-pulse rounded-md bg-field"></div></div>
		{:else if error}
			<div class="py-4"><ErrorState message={error} retry={() => void load()} /></div>
		{:else if libraries.length === 0}
			<!-- The panel is the frame here: a boxed empty state inside it was a box in a box. -->
			<div class="flex flex-col items-center px-6 py-10 text-center">
				<FolderPlus class="size-6 text-muted" />
				<p class="mt-3 text-base font-semibold text-foreground">{t('settings.librariesPage.emptyTitle')}</p>
				<p class="mt-1 max-w-md text-sm text-muted">{t('settings.librariesPage.emptyBody')}</p>
				<Button class="mt-5" size="field" onclick={() => openCreate()}>
					<FolderPlus class="size-4" /> {t('settings.librariesPage.addFirst')}
				</Button>
			</div>
		{:else}
			<!-- One table: the order handle, name and path, then the Private
			     switch, the Lock/Unlock slot (empty where there is none, so no
			     switch is pushed out of its column) and the row's two actions.
			     The keys act on the focused row, so their caps show on that row
			     only. On a phone the controls take a second line, and the actions
			     are icons at its end: a worded Remove wrapped under the rest. -->
			<ul class="settings-items">
				{#each libraries as lib, index (lib.id)}
					<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
					<li
						data-library={lib.id}
						class="group/row grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2 py-3.5 outline-none focus-visible:bg-surface-active/40 lg:grid-cols-[auto_minmax(0,1fr)_auto_7rem_auto] lg:gap-x-4"
						tabindex="0"
						aria-label={lib.name}
						onkeydown={(e) => rowKey(e, lib)}
					>
						<!-- The order: Home and the library page list libraries this way. -->
						<div class="row-span-2 flex flex-col self-center lg:row-span-1">
							<button
								type="button"
								tabindex="-1"
								class="flex h-8 w-7 items-center justify-center rounded text-muted/60 transition-colors group-hover/row:text-muted group-focus-within/row:text-muted hover:bg-surface-hover hover:text-foreground! disabled:invisible lg:h-5 lg:w-6"
								aria-label={t('settings.librariesPage.moveUp', { name: lib.name })}
								title={t('settings.librariesPage.moveUpKey')}
								disabled={index === 0}
								onclick={() => void move(lib, -1)}
							>
								<ChevronUp class="size-3.5" />
							</button>
							<button
								type="button"
								tabindex="-1"
								class="flex h-8 w-7 items-center justify-center rounded text-muted/60 transition-colors group-hover/row:text-muted group-focus-within/row:text-muted hover:bg-surface-hover hover:text-foreground! disabled:invisible lg:h-5 lg:w-6"
								aria-label={t('settings.librariesPage.moveDown', { name: lib.name })}
								title={t('settings.librariesPage.moveDownKey')}
								disabled={index === libraries.length - 1}
								onclick={() => void move(lib, 1)}
							>
								<ChevronDown class="size-3.5" />
							</button>
						</div>
						<div class="min-w-0">
							<!-- One tag style for what a library is (its type, private, its
							     root); only a problem (unreadable) takes a colour. -->
							<div class="flex flex-wrap items-center gap-x-2 gap-y-1">
								<p class="truncate text-sm font-medium text-foreground">{lib.name}</p>
								<Badge>{typeLabel(lib.type)}</Badge>
								{#if lib.private}
									<Badge>
										{#if lib.locked}<LockIcon class="size-3" aria-hidden="true" />{:else}<LockOpen class="size-3" aria-hidden="true" />{/if}
										{lib.locked ? t('settings.librariesPage.privateLocked') : t('settings.librariesPage.privateOpen')}
									</Badge>
								{/if}
								{#if lib.root}
									<Badge>{lib.root}:{lib.root_path || '/'}</Badge>
								{/if}
								{#if scan.status?.stats?.unreadable?.some((r) => r.library_id === lib.id)}
									<Badge tone="warning">{t('settings.librariesPage.unreadable')}</Badge>
								{/if}
							</div>
							<!-- A long path is cut at its start: the folder's own name is the part that tells libraries apart. -->
							<details class="group/path mt-0.5 text-start font-mono text-xs text-muted">
								<summary class="cursor-pointer list-none truncate hover:text-foreground [&::-webkit-details-marker]:hidden" title={t('settings.librariesPage.wholePath')}>
									<span class="group-open/path:hidden">…/{lib.path.split(/[\\/]/).filter(Boolean).slice(-2).join('/')}</span>
									<span class="hidden break-all whitespace-normal group-open/path:inline">{lib.path}</span>
								</summary>
							</details>
						</div>
						<div class="col-start-2 flex min-w-0 items-center gap-1 lg:contents">
							<div class="flex items-center">
								{#if roots !== null}
									<div class="flex h-9 items-center gap-2 pe-2 text-sm text-muted">
										<Switch
											bare
											label={t('privateLibrary.makePrivateFor', { name: lib.name })}
											checked={!!lib.private}
											onCheckedChange={(next) => void setPrivate(lib, next)}
										/>
										<span>{t('privateLibrary.makePrivate')}</span>
										<kbd class="max-lg:hidden transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">P</kbd>
									</div>
								{/if}
							</div>
							<div class="min-w-0">
								{#if lib.private}
									<Button variant="ghost" size="field" onclick={() => void toggleLock(lib)}>
										{#if lib.locked}
											<LockOpen class="size-4" /> {t('privateLibrary.unlock')}
										{:else}
											<LockIcon class="size-4" /> {t('privateLibrary.lock')}
										{/if}
										<kbd class="ms-1 max-lg:hidden transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">U</kbd>
									</Button>
								{/if}
							</div>
							<!-- Quiet until pointed at: a red bin on every row made five
							     destructive buttons the loudest things on the page. -->
							<div class="ms-auto flex items-center gap-0.5 lg:ms-0">
								<Button variant="ghost" size="field" class="max-lg:px-3" aria-label={t('settings.librariesPage.editFor', { name: lib.name })} onclick={() => openEdit(lib)}>
									<Pencil class="size-4" /> <span class="max-lg:sr-only">{t('settings.librariesPage.edit')}</span>
									<kbd class="ms-1 max-lg:hidden transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">E</kbd>
								</Button>
								<Button
									variant="ghost"
									size="field"
									class="hover:text-danger! focus-visible:text-danger! max-lg:px-3"
									aria-label={t('settings.librariesPage.removeFor', { name: lib.name })}
									onclick={() => askRemove(lib)}
								>
									<Trash2 class="size-4" /> <span class="max-lg:sr-only">{t('settings.librariesPage.remove')}</span>
									<kbd class="ms-1 max-lg:hidden transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">Del</kbd>
								</Button>
							</div>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</SettingsGroup>

	{#if roots !== null}
		<SettingsGroup id="roots" title={t('roots.heading')} description={t('roots.hint')}>
			{#if roots.length === 0}
				<p class="py-4 text-sm text-muted">{t('roots.none')}</p>
			{:else}
				<ul class="settings-items">
					{#each roots as r (r.name)}
						<li class="py-3">
							<div class="flex items-center gap-2">
								<span class="font-mono text-sm text-foreground">{r.name}</span>
								{#if !r.available}<Badge tone="warning">{t('roots.unavailable')}</Badge>{/if}
							</div>
							<p class="mt-0.5 truncate font-mono text-xs text-muted">{r.path}</p>
							{#if r.aliases.length}
								<p class="mt-0.5 truncate font-mono text-xs text-muted">
									{t('roots.aliases', { paths: r.aliases.join(', ') })}
								</p>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</SettingsGroup>

		<SettingsGroup
			id="pin"
			title={t('privateLibrary.pinHeading')}
			description={`${t('privateLibrary.pinHint')} ${pinSet ? t('privateLibrary.pinSet') : t('privateLibrary.pinNotSet')}`}
		>
			<form
				class="grid gap-3 py-4 sm:grid-cols-[1fr_1fr_auto]"
				onsubmit={(e) => {
					e.preventDefault();
					void savePin(false);
				}}
			>
				<Input
					label={t('privateLibrary.pinNew')}
					type="password"
					inputmode="numeric"
					autocomplete="off"
					bind:value={pinForm.pin}
				/>
				<Input
					label={t('privateLibrary.password')}
					type="password"
					autocomplete="current-password"
					bind:value={pinForm.password}
					required
				/>
				<div class="flex items-end gap-2">
					<Button type="submit" size="field" loading={pinBusy} disabled={!pinForm.pin || !pinForm.password}>
						{t('privateLibrary.pinSave')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd>
					</Button>
					{#if pinSet}
						<Button type="button" variant="ghost" size="field" disabled={!pinForm.password} onclick={() => void savePin(true)}>
							{t('privateLibrary.pinClear')}
						</Button>
					{/if}
				</div>
			</form>
		</SettingsGroup>
	{/if}
</div>

<UnlockLibraryDialog bind:open={unlockOpen} library={unlockTarget} onunlocked={(lib) => replace(lib)} />

<!-- Add, or edit: one dialog, one folder picker. -->
<Modal
	bind:open={createOpen}
	title={editTarget ? t('settings.librariesPage.editTitle', { name: editTarget.name }) : t('settings.librariesPage.addTitle')}
	description={editTarget ? t('settings.librariesPage.editBody') : t('settings.librariesPage.addBody')}
>
	<form
		class="space-y-5"
		onsubmit={(e) => {
			e.preventDefault();
			void create(editTarget ? editNeedsScan : true);
		}}
	>
		{#if formError}
			<div class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger" role="alert">
				{formError}
			</div>
		{/if}

		<!-- The folder: one path bar, and the folders inside it. -->
		<div class="space-y-1.5">
			<label for="library-path" class="block text-sm font-medium text-muted">{t('settings.librariesPage.folder')}</label>
			<div class="flex gap-2">
				<input
					id="library-path"
					bind:this={pathEl}
					bind:value={pathDraft}
					onkeydown={pathKey}
					onblur={() => {
						if (pathDraft.trim() && pathDraft.trim() !== browsePath) void loadBrowse(pathDraft.trim());
					}}
					placeholder="/media/anime"
					spellcheck="false"
					autocomplete="off"
					aria-invalid={browseError ? 'true' : undefined}
					aria-describedby="library-path-hint"
					class={[
						'h-10 min-w-0 flex-1 rounded-md border bg-field px-3 font-mono text-sm text-foreground',
						'placeholder:text-muted/60 focus:border-accent/60 focus:outline-none',
						browseError ? 'border-danger/60' : 'border-transparent'
					].join(' ')}
				/>
				<!-- type="button": the first submit button of a form is what Enter in
				     any field presses, and this one went up a folder and renamed the
				     library instead of adding it. -->
				<Button
					type="button"
					variant="secondary"
					disabled={!browseParent || browseLoading}
					onclick={goUp}
					title={t('settings.librariesPage.parentKey')}
										aria-label={t('settings.librariesPage.parent')}
				>
					<ArrowUp class="size-4" />
				</Button>
			</div>

			<div class="overflow-hidden rounded-md border border-line bg-surface/40">
				{#if browseLoading && browseDirs.length === 0}
					<p class="flex h-56 items-center justify-center gap-2 text-sm text-muted"><Spinner /> {t('settings.librariesPage.listing')}</p>
				{:else if browseError}
					<div class="flex h-56 flex-col items-center justify-center gap-2 px-4 text-center text-sm">
						<p class="text-danger" role="alert">{browseError}</p>
						<Button type="button" variant="ghost" size="sm" onclick={() => void loadBrowse(browsePath || undefined)}>
							{folderName(browsePath) ? t('settings.librariesPage.backTo', { name: folderName(browsePath) }) : t('settings.librariesPage.backToStart')}
						</Button>
					</div>
				{:else if browseDirs.length === 0}
					<p class="flex h-56 items-center justify-center px-4 text-center text-sm text-muted">{t('settings.librariesPage.noFolders')}</p>
				{:else}
					<ul
						bind:this={listEl}
						class="h-56 overflow-y-auto py-1 focus:outline-none"
						tabindex="0"
						role="listbox"
						aria-label={t('settings.librariesPage.foldersIn', { path: browsePath })}
						aria-activedescendant="library-dir-{cursor}"
						onkeydown={listKey}
					>
						{#each browseDirs as dir, index (dir.path)}
							<li id="library-dir-{index}" data-index={index} role="option" aria-selected={index === cursor}>
								<button
									type="button"
									tabindex="-1"
									class={[
										'flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-sm',
										index === cursor ? 'bg-surface-hover text-foreground' : 'text-foreground/90 hover:bg-surface-hover'
									].join(' ')}
									onclick={() => void loadBrowse(dir.path)}
									onmousemove={() => (cursor = index)}
								>
									<Folder class="size-4 shrink-0 text-muted" />
									<span class="truncate">{dir.name}</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
			<p id="library-path-hint" class="text-xs leading-5 text-muted">
				{t('settings.librariesPage.pathHint')}
				<span class="whitespace-nowrap"><kbd class="font-mono text-[10px]">↑↓</kbd> {t('settings.librariesPage.keyListMove')}</span> ·
				<span class="whitespace-nowrap"><kbd class="font-mono text-[10px]">Enter</kbd> {t('settings.librariesPage.keyListOpen')}</span> ·
				<span class="whitespace-nowrap"><kbd class="font-mono text-[10px]">Backspace</kbd> {t('settings.librariesPage.keyListUp')}</span>
			</p>
			{#if editTarget && editPatch.path !== undefined}
				<p class="text-xs leading-5 text-warning">{t('settings.librariesPage.pathChanged')}</p>
			{/if}
		</div>

		<div class="grid gap-4 sm:grid-cols-[1fr_11rem]">
			<Input
				id="library-name"
				label={t('settings.librariesPage.name')}
				bind:value={form.name}
				oninput={() => (nameTouched = true)}
				placeholder={t('settings.librariesPage.namePlaceholder')}
				required
			/>
			<Select label={t('settings.librariesPage.type')} bind:value={form.type} options={typeOptions} />
		</div>
		<p class="-mt-3 text-xs text-muted">
			{form.type === 'auto'
				? t('settings.librariesPage.typeAuto')
				: form.type === 'manga' || form.type === 'comic'
					? t('settings.librariesPage.typeComics')
					: t('settings.librariesPage.typeVideo')}
		</p>

		{#if roots !== null}
			<Switch
				bind:checked={form.private}
				label={t('privateLibrary.makePrivate')}
				description={t('privateLibrary.makePrivateHint')}
			/>
		{/if}

		<div class="flex flex-wrap justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
			{#if editTarget}
				<!-- A new folder or type is read by the next scan: then saving
				     offers the scan first, and a plain save stays one step away. -->
				{#if editNeedsScan}
					<Button type="button" variant="secondary" disabled={creating || !form.name.trim()} onclick={() => void create(false)}>
						{t('settings.librariesPage.saveOnly')}
					</Button>
				{/if}
				<Button type="submit" loading={creating} disabled={!form.path.trim() || !form.name.trim() || Object.keys(editPatch).length === 0}>
					{editNeedsScan ? t('settings.librariesPage.saveAndScan') : t('settings.librariesPage.save')}
					<kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
				</Button>
			{:else}
				<Button
					type="button"
					variant="secondary"
					disabled={creating || !form.path.trim() || !form.name.trim()}
					onclick={() => void create(false)}
				>
					{t('settings.librariesPage.addOnly')}
				</Button>
				<Button type="submit" loading={creating} disabled={!form.path.trim() || !form.name.trim()}>
					{t('settings.librariesPage.addAndScan')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
				</Button>
			{/if}
		</div>
	</form>
</Modal>

<!-- Delete -->
<Modal
	bind:open={deleteOpen}
	title={t('settings.groups.libraries.removeTitle')}
	description={t('settings.groups.libraries.removeBody')}
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-library" variant="danger" loading={deleting} onclick={() => void remove()}>{t('settings.groups.libraries.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
	{#if deleteTarget}
		<p class="text-sm text-muted">
			<span class="font-medium text-foreground">{deleteTarget.name}</span> —
			<span class="font-mono text-xs">{deleteTarget.path}</span>
		</p>
	{/if}
</Modal>
