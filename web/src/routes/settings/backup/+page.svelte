<script lang="ts">
	import DatabaseBackup from '@lucide/svelte/icons/database-backup';
	import Download from '@lucide/svelte/icons/download';
	import Info from '@lucide/svelte/icons/info';
	import { api } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let busy = $state(false);

	async function download(): Promise<void> {
		busy = true;
		try {
			const name = await api.backup.download();
			toasts.success(`Saved ${name}.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not download the backup.'));
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>Backup — Settings — Lain</title></svelte:head>

<div class="max-w-3xl space-y-5">
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<div class="flex items-start gap-4">
			<div class="flex size-10 shrink-0 items-center justify-center rounded-full bg-accent/10">
				<DatabaseBackup class="size-5 text-accent" />
			</div>
			<div class="min-w-0 flex-1">
				<h2 class="text-base font-semibold text-foreground">Database snapshot</h2>
				<p class="mt-1 text-sm leading-relaxed text-muted">
					Streams a consistent copy of <span class="font-mono text-xs">lain.db</span> from a
					single read transaction: accounts, libraries, catalog and watch progress. Safe while
					scans and streams are running. Media files are never included — they are your files,
					where they already are.
				</p>
				<Button class="mt-4" loading={busy} onclick={() => void download()}>
					<Download class="size-4" /> Download lain.db
				</Button>
			</div>
		</div>
	</section>

	<section class="rounded-card border border-line bg-surface/40 p-5">
		<h2 class="flex items-center gap-2 text-sm font-semibold text-foreground">
			<Info class="size-4 text-muted" /> Restoring
		</h2>
		<p class="mt-2 text-sm leading-relaxed text-muted">
			Restore is deliberately offline: bbolt holds an exclusive lock, so the server must be
			stopped first. The CLI validates the snapshot before touching anything.
		</p>
		<pre
			class="mt-3 overflow-x-auto rounded-md border border-line bg-background px-4 py-3 font-mono text-xs leading-relaxed text-foreground"><code>pkill -x lain
lain restore ~/Downloads/lain-backup.db --data-dir ~/.local/share/lain
lain serve</code></pre>
		<p class="mt-3 text-xs text-muted">
			The composition file (<span class="font-mono">composition.json</span>) sits next to the
			database in the data directory and is inspectable on the Plugins page.
		</p>
	</section>
</div>
