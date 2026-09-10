<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import ScanLine from '@lucide/svelte/icons/scan-line';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import type { Library } from '$lib/api/types';
	import { api } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';
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
					Last scan {scan.status.finished_at ? formatRelative(scan.status.finished_at) : 'finished'}:
					<span class="text-foreground">{stats.identified}</span> identified ·
					<span class="text-foreground">{stats.unidentified}</span> unidentified ·
					<span class="text-foreground">{stats.pruned}</span> pruned
					{#if stats.walk_errors > 0}· <span class="text-warning">{stats.walk_errors} unreadable</span>{/if}
					{#if stats.errors > 0}· <span class="text-danger">{stats.errors} errors</span>{/if}
				</p>
			{:else}
				<p class="text-muted">No scan recorded yet.</p>
			{/if}
		</div>
	</section>

	<!-- Libraries -->
	<section class="space-y-4">
		<div class="flex items-center justify-between gap-3">
			<h2 class="text-base font-semibold text-foreground">Media libraries</h2>
			<Button variant="secondary" size="sm" onclick={() => (createOpen = true)}>
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
				<Button onclick={() => (createOpen = true)}>
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
		<Input
			label="Server directory path"
			bind:value={form.path}
			placeholder="/media/anime"
			hint="An absolute path on the machine running Lain — not a path in your browser. In Docker, use the container path (for example /media/videos)."
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
	description="The files on disk stay untouched. Catalog entries are pruned on the next scan."
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
