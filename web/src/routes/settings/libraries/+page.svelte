<script lang="ts">
	import { onMount } from 'svelte';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import Folder from '@lucide/svelte/icons/folder';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
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
		{ keys: ['Delete'], label: t('pageKeys.libraryRemove') }
	]);

	// Row keys: P toggles private, U unlocks or locks, Delete removes.
	function rowKey(e: KeyboardEvent, lib: Library): void {
		if (e.target !== e.currentTarget || e.ctrlKey || e.metaKey || e.altKey) return;
		const key = e.key.toLowerCase();
		if (key === 'p' && roots !== null) {
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
		createOpen = true;
		formError = null;
		form = { name: '', type: 'auto', path: '', private: false };
		nameTouched = false;
		pathDraft = '';
		void loadBrowse();
		// The dialog opens on what it asks for first: the folder.
		focusSoon(() => document.getElementById('library-path'));
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
			if (!nameTouched) form.name = folderName(res.path);
			// A long path keeps its end, the folder being chosen, in view.
			requestAnimationFrame(() => {
				if (pathEl && document.activeElement !== pathEl) pathEl.scrollLeft = pathEl.scrollWidth;
			});
		} catch (err) {
			browseError = errorMessage(err, 'Could not list that folder.');
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

	async function create(scanAfter: boolean): Promise<void> {
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
			libraries = [...libraries, lib].sort((a, b) => a.name.localeCompare(b.name));
			rememberLibrary(lib);
			createOpen = false;
			form = { name: '', type: 'auto', path: '', private: false };
			toasts.success(`Library “${lib.name}” added.`);
			if (scanAfter) await scan.start();
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
			toasts.success(`Library “${target.name}” removed.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not remove the library.'));
		} finally {
			deleting = false;
		}
	}

	async function startScan(): Promise<void> {
		try {
			await scan.start();
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not start the scan.'));
		}
	}

	// Auto takes videos and comic archives alike; the others only filter
	// which files of the folder count.
	const typeOptions = [
		{ value: 'auto', label: 'Auto' },
		{ value: 'anime', label: 'Anime' },
		{ value: 'series', label: 'Series' },
		{ value: 'movie', label: 'Movies' },
		{ value: 'manga', label: 'Manga' },
		{ value: 'comic', label: 'Comics' }
	];
</script>

<svelte:head><title>Libraries — Settings — Lain</title></svelte:head>

<div>
	<SettingsGroup id="scan" title="Scan" description={t('settings.groups.libraries.scan')}>
		{#snippet aside()}
			<Button size="field" onclick={() => void startScan()} loading={scan.running} disabled={scan.running}>
				<ScanLine class="size-4" /> Scan now
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
				<p class="text-danger" role="alert">Scan failed: {scan.status.error}</p>
			{:else if scan.status?.state === 'done' && scan.status.stats}
				{@const stats = scan.status.stats}
				<p class="text-muted">
					<!-- The colon on the last word: on its own line it printed "ago :". -->
					{[`Last scan ${scan.status.finished_at ? formatRelative(scan.status.finished_at) : 'finished'}`, scan.status.trigger === 'watch' ? 'by the filesystem watcher' : ''].filter(Boolean).join(' ')}:
					<span class="text-foreground">{stats.identified}</span> identified ·
					<span class="text-foreground">{stats.unidentified}</span> unidentified ·
					<span class="text-foreground">{stats.enriched}</span> enriched
					{#if stats.missing > 0}· <span class="text-warning">{stats.missing} missing</span>{/if}
					{#if stats.restored > 0}· <span class="text-foreground">{stats.restored} restored</span>{/if}
					{#if stats.walk_errors > 0}· <span class="text-warning">{stats.walk_errors} unreadable</span>{/if}
					{#if stats.errors > 0}· <span class="text-danger">{stats.errors} errors</span>{/if}
				</p>
				<!-- A count nobody can attribute is not a task: name the roots
				     the server could not read, and what to check next. -->
				{#if stats.unreadable?.length}
					<ul class="mt-2 space-y-1.5 text-sm" aria-label="Unreadable library roots">
						{#each stats.unreadable as root (root.library_id)}
							<li class="text-warning">
								<span class="font-medium">{root.name}</span>
								<span class="font-mono text-xs text-muted">{root.path}</span>
								— {root.reason}. Check that the drive is mounted and the path is readable by the
								server, then scan again. Nothing was marked missing for it, so the catalog still
								shows what it last saw.
							</li>
						{/each}
					</ul>
				{/if}
			{:else}
				<p class="text-muted">No scan recorded yet.</p>
			{/if}
		</div>
	</SettingsGroup>

	<SettingsGroup id="libraries" title="Media libraries" description={t('settings.groups.libraries.libraries')}>
		{#snippet aside()}
			<Button variant="tonal" size="field" onclick={() => openCreate()}>
				<FolderPlus class="size-4" /> Add library
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
				<p class="mt-3 text-base font-semibold text-foreground">No libraries configured</p>
				<p class="mt-1 max-w-md text-sm text-muted">
					A library is a directory on the server Lain is allowed to read. Nothing is copied or moved.
				</p>
				<Button class="mt-5" size="field" onclick={() => openCreate()}>
					<FolderPlus class="size-4" /> Add your first library
				</Button>
			</div>
		{:else}
			<!-- One table: name and path, then the Private switch, the Lock/Unlock
			     slot (empty where there is none, so no switch is pushed out of its
			     column) and Remove. The keys act on the focused row, so their caps
			     show on that row only. -->
			<ul class="settings-items">
				{#each libraries as lib (lib.id)}
					<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
					<li
						class="group/row flex flex-wrap items-center gap-x-3 gap-y-2 py-3.5 outline-none focus-visible:bg-surface-active/40 sm:grid sm:grid-cols-[minmax(0,1fr)_auto_auto_auto] sm:gap-x-4"
						tabindex="0"
						onkeydown={(e) => rowKey(e, lib)}
					>
						<div class="min-w-0 basis-full">
							<div class="flex flex-wrap items-center gap-x-2 gap-y-1">
								<p class="truncate text-sm font-medium text-foreground">{lib.name}</p>
								<Badge>{lib.type}</Badge>
								{#if lib.private}
									<Badge tone="warning">{t('privateLibrary.badge')} · {lib.locked ? t('privateLibrary.locked') : t('privateLibrary.unlocked')}</Badge>
								{/if}
								{#if lib.root}
									<Badge>{lib.root}:{lib.root_path || '/'}</Badge>
								{/if}
								{#if scan.status?.stats?.unreadable?.some((r) => r.library_id === lib.id)}
									<Badge tone="warning">unreadable</Badge>
								{/if}
							</div>
							<!-- A long path is cut at its start: the folder's own name is the part that tells libraries apart. -->
							<details class="group/path mt-0.5 text-left font-mono text-xs text-muted">
								<summary class="cursor-pointer list-none truncate hover:text-foreground [&::-webkit-details-marker]:hidden" title="Show the whole path">
									<span class="group-open/path:hidden">…/{lib.path.split(/[\\/]/).filter(Boolean).slice(-2).join('/')}</span>
									<span class="hidden break-all whitespace-normal group-open/path:inline">{lib.path}</span>
								</summary>
							</details>
						</div>
						<div class="flex items-center max-sm:empty:hidden">
							{#if roots !== null}
								<div class="flex h-9 items-center gap-2 pe-2 text-sm text-muted">
									<Switch
										bare
										label={t('privateLibrary.makePrivate')}
										checked={!!lib.private}
										onCheckedChange={(next) => void setPrivate(lib, next)}
									/>
									<span>{t('privateLibrary.makePrivate')}</span>
									<kbd class="transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">P</kbd>
								</div>
							{/if}
						</div>
						<div class="max-sm:empty:hidden sm:w-28">
							{#if lib.private}
								<Button variant="ghost" size="field" onclick={() => void toggleLock(lib)}>
									{#if lib.locked}
										<LockOpen class="size-4" /> {t('privateLibrary.unlock')}
									{:else}
										<LockIcon class="size-4" /> {t('privateLibrary.lock')}
									{/if}
									<kbd class="ms-1 transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">U</kbd>
								</Button>
							{/if}
						</div>
						<!-- Quiet until pointed at: a red bin on every row made five destructive
						     buttons the loudest things on the page, louder than Scan now. -->
						<Button
							variant="ghost"
							size="field"
							class="justify-self-start hover:text-danger! focus-visible:text-danger!"
							aria-label={`Remove ${lib.name}`}
							onclick={() => askRemove(lib)}
						>
							<Trash2 class="size-4" /> Remove
							<kbd class="ms-1 transition-opacity opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100">Del</kbd>
						</Button>
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

<!-- Create -->
<Modal
	bind:open={createOpen}
	title="Add a media library"
	description="Lain indexes files in place. Nothing is copied, moved or modified."
>
	<form
		class="space-y-5"
		onsubmit={(e) => {
			e.preventDefault();
			void create(true);
		}}
	>
		{#if formError}
			<div class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger" role="alert">
				{formError}
			</div>
		{/if}

		<!-- The folder: one path bar, and the folders inside it. -->
		<div class="space-y-1.5">
			<label for="library-path" class="block text-sm font-medium text-muted">Folder</label>
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
				<Button
					variant="secondary"
					disabled={!browseParent || browseLoading}
					onclick={goUp}
					title="Parent folder (Backspace in the list)"
					aria-label="Parent folder"
				>
					<ArrowUp class="size-4" />
				</Button>
			</div>

			<div class="overflow-hidden rounded-md border border-line bg-surface/40">
				{#if browseLoading && browseDirs.length === 0}
					<p class="flex h-56 items-center justify-center gap-2 text-sm text-muted"><Spinner /> Listing folders…</p>
				{:else if browseError}
					<div class="flex h-56 flex-col items-center justify-center gap-2 px-4 text-center text-sm">
						<p class="text-danger" role="alert">{browseError}</p>
						<Button variant="ghost" size="sm" onclick={() => void loadBrowse(browsePath || undefined)}>
							Back to {folderName(browsePath) || 'the start'}
						</Button>
					</div>
				{:else if browseDirs.length === 0}
					<p class="flex h-56 items-center justify-center px-4 text-center text-sm text-muted">
						No folders inside this one. It can still be the library.
					</p>
				{:else}
					<ul
						bind:this={listEl}
						class="h-56 overflow-y-auto py-1 focus:outline-none"
						tabindex="0"
						role="listbox"
						aria-label="Folders in {browsePath}"
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
			<p id="library-path-hint" class="text-xs text-muted">
				The library is the folder in the bar. Click a folder to go into it, or type a path and press
				<kbd class="font-mono text-[10px]">Enter</kbd>. In the list:
				<kbd class="font-mono text-[10px]">↑↓</kbd> move,
				<kbd class="font-mono text-[10px]">Enter</kbd> open,
				<kbd class="font-mono text-[10px]">Backspace</kbd> up. In Docker, these are the container's paths.
			</p>
		</div>

		<div class="grid gap-4 sm:grid-cols-[1fr_11rem]">
			<Input
				label="Name"
				bind:value={form.name}
				oninput={() => (nameTouched = true)}
				placeholder="Anime"
				required
			/>
			<Select label="Type" bind:value={form.type} options={typeOptions} />
		</div>
		<p class="-mt-3 text-xs text-muted">
			{form.type === 'auto'
				? 'Auto reads videos and comic archives, and tells each file apart by itself.'
				: form.type === 'manga' || form.type === 'comic'
					? 'Only comic archives (cbz, cbr, cb7) in the folder are read.'
					: 'Only video files in the folder are read.'}
		</p>

		{#if roots !== null}
			<Switch
				bind:checked={form.private}
				label={t('privateLibrary.makePrivate')}
				description={t('privateLibrary.makePrivateHint')}
			/>
		{/if}

		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (createOpen = false)}>Cancel</Button>
			<Button
				type="button"
				variant="secondary"
				disabled={creating || !form.path.trim() || !form.name.trim()}
				onclick={() => void create(false)}
			>
				Add
			</Button>
			<Button type="submit" loading={creating} disabled={!form.path.trim() || !form.name.trim()}>
				Add and scan <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
			</Button>
		</div>
	</form>
</Modal>

<!-- Delete -->
<Modal
	bind:open={deleteOpen}
	title="Remove library?"
	description="The files on disk stay untouched. Its catalog entries are removed with it."
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>Cancel <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-library" variant="danger" loading={deleting} onclick={() => void remove()}>Remove library <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
	{#if deleteTarget}
		<p class="text-sm text-muted">
			<span class="font-medium text-foreground">{deleteTarget.name}</span> —
			<span class="font-mono text-xs">{deleteTarget.path}</span>
		</p>
	{/if}
</Modal>
