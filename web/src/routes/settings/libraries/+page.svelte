<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
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
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { ensureLibraries, forgetLibrary, rememberLibrary } from '$lib/stores/media-cache.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';

	let libraries = $state<Library[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let formError = $state<string | null>(null);
	let form = $state({ name: '', type: 'anime', path: '' });

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

	let browsePath = $state('');
	let browseParent = $state('');
	let browseDirs = $state<BrowseDir[]>([]);
	let browseLoading = $state(false);
	let browseError = $state<string | null>(null);

	function openCreate(): void {
		createOpen = true;
		formError = null;
		form = { name: '', type: 'anime', path: '' };
		void loadBrowse();
	}

	async function loadBrowse(path?: string): Promise<void> {
		browseLoading = true;
		browseError = null;
		try {
			const res = await api.libraries.browse(path);
			browsePath = res.path;
			browseParent = res.parent;
			browseDirs = res.dirs;
		} catch (err) {
			browseError = errorMessage(err, 'Could not list server folders.');
		} finally {
			browseLoading = false;
		}
	}

	function useFolder(dir?: BrowseDir): void {
		const picked = dir ? dir.path : browsePath;
		if (!picked) return;
		form.path = picked;
		formError = null;
		if (!form.name.trim()) {
			const base = (dir ? dir.name : picked.split('/').filter(Boolean).pop()) ?? '';
			if (base) form.name = base;
		}
	}

	function askRemove(lib: Library): void {
		deleteTarget = lib;
		deleteOpen = true;
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
		void scan.refresh().then(() => {
			if (scan.running) scan.follow();
		});
	});

	onDestroy(() => scan.stop());

	async function create(scanAfter: boolean): Promise<void> {
		creating = true;
		formError = null;
		try {
			const lib = await api.libraries.create({
				name: form.name.trim(),
				type: form.type,
				path: form.path.trim()
			});
			libraries = [...libraries, lib].sort((a, b) => a.name.localeCompare(b.name));
			rememberLibrary(lib);
			createOpen = false;
			form = { name: '', type: 'anime', path: '' };
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

	const typeOptions = [
		{ value: 'anime', label: 'Anime' },
		{ value: 'series', label: 'Series' },
		{ value: 'movie', label: 'Movies' },
		{ value: 'manga', label: 'Manga' },
		{ value: 'comic', label: 'Comics' }
	];
</script>

<svelte:head><title>Libraries — Settings — Lain</title></svelte:head>

<div class="space-y-6">
	<!-- Scan -->
	<section id="scan" class="settings-section scroll-mt-28 !border-t-0 !pt-0">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<h2 class="settings-heading">Scan</h2>
				<p class="mt-0.5 text-sm text-muted">
					Walks every library root and updates the catalog. Safe to run while watching.
				</p>
			</div>
			<Button onclick={() => void startScan()} loading={scan.running} disabled={scan.running}>
				<ScanLine class="size-4" /> Scan now
			</Button>
		</div>

		<div class="mt-4 text-sm">
			{#if scan.running}
				<div class="flex items-center gap-2 text-accent">
					<Spinner class="size-4" /> Scanning libraries…
				</div>
			{:else if scan.status?.state === 'error'}
				<p class="text-danger" role="alert">Scan failed: {scan.status.error}</p>
			{:else if scan.status?.state === 'done' && scan.status.stats}
				{@const stats = scan.status.stats}
				<p class="text-muted">
					Last scan {scan.status.finished_at ? formatRelative(scan.status.finished_at) : 'finished'}
					{#if scan.status.trigger === 'watch'}by the filesystem watcher{/if}:
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
	</section>

	<!-- Libraries -->
	<section id="libraries" class="scroll-mt-28 space-y-4 pt-8">
		<div class="flex items-center justify-between gap-3">
			<h2 class="settings-heading">Media libraries</h2>
			<Button variant="secondary" size="sm" onclick={() => openCreate()}>
				<FolderPlus class="size-4" /> Add library
			</Button>
		</div>

		{#if loading}
			<div class="h-20 animate-pulse rounded-card bg-surface-active/50"></div>
		{:else if error}
			<ErrorState message={error} retry={() => void load()} />
		{:else if libraries.length === 0}
			<EmptyState
				title="No libraries configured"
				description="A library is a directory on the server Lain is allowed to read. Nothing is copied or moved."
			>
				{#snippet icon()}<FolderPlus class="size-6 text-muted" />{/snippet}
				<Button onclick={() => openCreate()}>
					<FolderPlus class="size-4" /> Add your first library
				</Button>
			</EmptyState>
		{:else}
			<ul class="settings-list">
				{#each libraries as lib (lib.id)}
					<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
					<li
						class="flex flex-wrap items-center gap-3 bg-surface/40 px-4 py-3.5 focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent"
						tabindex="0"
						onkeydown={(e) => rowKey(e, lib)}
					>
						<div class="min-w-0 flex-1">
							<div class="flex items-center gap-2">
								<p class="truncate font-medium text-foreground">{lib.name}</p>
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
							<p class="mt-0.5 truncate font-mono text-xs text-muted">{lib.path}</p>
						</div>
						{#if roots !== null}
							<div class="flex items-center gap-2 text-sm text-muted">
								<Switch
									bare
									label={t('privateLibrary.makePrivate')}
									checked={!!lib.private}
									onCheckedChange={(next) => void setPrivate(lib, next)}
								/>
								<span>{t('privateLibrary.makePrivate')}</span>
								<kbd class="font-mono text-[10px] text-muted">P</kbd>
							</div>
						{/if}
						{#if lib.private}
							<Button variant="ghost" size="sm" onclick={() => void toggleLock(lib)}>
								{#if lib.locked}
									<LockOpen class="size-4" /> {t('privateLibrary.unlock')}
								{:else}
									<LockIcon class="size-4" /> {t('privateLibrary.lock')}
								{/if}
								<kbd class="ms-1 font-mono text-[10px] text-muted">U</kbd>
							</Button>
						{/if}
						<Button
							variant="ghost"
							size="sm"
							aria-label={`Remove ${lib.name}`}
							onclick={() => askRemove(lib)}
						>
							<Trash2 class="size-4 text-danger" /> Remove
							<kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd>
						</Button>
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	{#if roots !== null}
		<section id="roots" class="settings-section scroll-mt-28 space-y-3">
			<div>
				<h2 class="settings-heading">{t('roots.heading')}</h2>
				<p class="mt-0.5 text-sm text-muted">{t('roots.hint')}</p>
			</div>
			{#if roots.length === 0}
				<p class="text-sm text-muted">{t('roots.none')}</p>
			{:else}
				<ul class="settings-list">
					{#each roots as r (r.name)}
						<li class="bg-surface/40 px-4 py-3">
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
		</section>

		<section id="pin" class="settings-section scroll-mt-28 space-y-3">
			<div>
				<h2 class="settings-heading">{t('privateLibrary.pinHeading')}</h2>
				<p class="mt-0.5 text-sm text-muted">
					{t('privateLibrary.pinHint')}
					{pinSet ? t('privateLibrary.pinSet') : t('privateLibrary.pinNotSet')}
				</p>
			</div>
			<form
				class="grid gap-3 sm:grid-cols-[1fr_1fr_auto]"
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
					<Button type="submit" loading={pinBusy} disabled={!pinForm.pin || !pinForm.password}>
						{t('privateLibrary.pinSave')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd>
					</Button>
					{#if pinSet}
						<Button type="button" variant="ghost" disabled={!pinForm.password} onclick={() => void savePin(true)}>
							{t('privateLibrary.pinClear')}
						</Button>
					{/if}
				</div>
			</form>
		</section>
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
		class="space-y-4"
		onsubmit={(e) => {
			e.preventDefault();
			void create(true);
		}}
	>
		{#if formError}
			<div
				class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
				role="alert"
			>
				{formError}
			</div>
		{/if}
		<Input label="Name" bind:value={form.name} placeholder="Anime" required />
		<Select label="Type" bind:value={form.type} options={typeOptions} />
		<div>
			<span class="mb-1 block text-sm font-medium text-foreground">Server folders</span>
			{#if browseLoading}
				<p class="py-2 text-sm text-muted">Listing server folders…</p>
			{:else if browseError}
				<div class="flex items-center gap-2 py-1 text-sm">
					<p class="text-danger" role="alert">{browseError}</p>
					<Button variant="ghost" size="sm" onclick={() => void loadBrowse()}>Retry</Button>
				</div>
			{:else}
				<p class="truncate font-mono text-xs text-muted" title={browsePath}>{browsePath}</p>
				<div class="mt-1 flex gap-2">
					<Button
						variant="ghost"
						size="sm"
						disabled={!browseParent}
						onclick={() => void loadBrowse(browseParent || undefined)}
					>
						<ArrowUp class="size-4" /> Up
					</Button>
					<Button variant="secondary" size="sm" onclick={() => useFolder()}>
						Use this folder
					</Button>
				</div>
				{#if browseDirs.length === 0}
					<p class="py-2 text-sm text-muted">No subfolders here.</p>
				{:else}
					<ul class="settings-list mt-1 max-h-44 overflow-y-auto">
						{#each browseDirs as dir (dir.path)}
							<li class="flex items-center gap-2 bg-surface/40 px-3 py-2">
								<button
									type="button"
									class="flex min-w-0 flex-1 items-center gap-2 text-left"
									onclick={() => void loadBrowse(dir.path)}
									title={`Open ${dir.path}`}
								>
									<Folder class="size-4 shrink-0 text-muted" />
									<span class="truncate text-sm text-foreground">{dir.name}</span>
								</button>
								<Button variant="ghost" size="sm" onclick={() => useFolder(dir)}>Use</Button>
							</li>
						{/each}
					</ul>
				{/if}
			{/if}
		</div>
		<Input
			label="Server directory path"
			bind:value={form.path}
			placeholder="/media/anime"
			hint="Filled in by the folder picker above; type only for paths outside it. In Docker, use the container path."
			required
		/>
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (createOpen = false)}>Cancel</Button>
			<Button type="submit" loading={creating}>Add and scan</Button>
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
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>Cancel</Button>
		<Button variant="danger" loading={deleting} onclick={() => void remove()}>Remove library</Button>
	{/snippet}
	{#if deleteTarget}
		<p class="text-sm text-muted">
			<span class="font-medium text-foreground">{deleteTarget.name}</span> —
			<span class="font-mono text-xs">{deleteTarget.path}</span>
		</p>
	{/if}
</Modal>
