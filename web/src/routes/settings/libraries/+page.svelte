<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import Folder from '@lucide/svelte/icons/folder';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import ScanLine from '@lucide/svelte/icons/scan-line';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import type { BrowseDir, Library } from '$lib/api/types';
	import { api } from '$lib/api';
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
		void load();
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
		{ value: 'movie', label: 'Movies' }
	];
</script>

<svelte:head><title>Libraries — Settings — Lain</title></svelte:head>

<div class="space-y-6">
	<!-- Scan -->
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<h2 class="text-base font-semibold text-foreground">Scan</h2>
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
	<section class="space-y-4">
		<div class="flex items-center justify-between gap-3">
			<h2 class="text-base font-semibold text-foreground">Media libraries</h2>
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
			<ul class="divide-y divide-line overflow-hidden rounded-card border border-line">
				{#each libraries as lib (lib.id)}
					<li class="flex flex-wrap items-center gap-3 bg-surface/40 px-4 py-3.5">
						<div class="min-w-0 flex-1">
							<div class="flex items-center gap-2">
								<p class="truncate font-medium text-foreground">{lib.name}</p>
								<Badge>{lib.type}</Badge>
								{#if scan.status?.stats?.unreadable?.some((r) => r.library_id === lib.id)}
									<Badge tone="warning">unreadable</Badge>
								{/if}
							</div>
							<p class="mt-0.5 truncate font-mono text-xs text-muted">{lib.path}</p>
						</div>
						<Button
							variant="ghost"
							size="sm"
							aria-label={`Remove ${lib.name}`}
							onclick={() => askRemove(lib)}
						>
							<Trash2 class="size-4 text-danger" /> Remove
						</Button>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
</div>

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
					<ul class="mt-1 max-h-44 divide-y divide-line overflow-y-auto rounded-card border border-line">
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
