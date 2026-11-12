<script lang="ts">
	import Download from '@lucide/svelte/icons/download';
	import { api } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let busy = $state(false);
	// The archive the restore lines name: the one just saved, once there is one.
	let archive = $state('lain-backup-YYYYMMDD-HHMMSS.tar');

	async function download(): Promise<void> {
		busy = true;
		try {
			const name = await api.backup.download();
			archive = name;
			toasts.success(`Saved ${name}.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not download the backup.'));
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>Backup — Settings — Lain</title></svelte:head>

<SettingsGroup id="backup" title="Database snapshot" description={t('settings.groups.backup.snapshot')}>
	{#snippet aside()}
		<Button size="field" loading={busy} onclick={() => void download()}>
			<Download class="size-4" /> Download a backup
		</Button>
	{/snippet}
	<p class="py-4 text-sm leading-6 text-muted">
		One archive (<span class="font-mono text-xs">.tar</span>) read in a single transaction:
		accounts, libraries, catalog and watch progress, with the plugin composition, library
		roots and avatars. Safe while scans and streams are running. Media files are never
		included — they are your files, where they already are. Restore it into an empty data
		folder with <span class="font-mono text-xs">lain restore</span>.
	</p>
</SettingsGroup>

<SettingsGroup id="restore" title="Restoring" description={t('settings.groups.backup.restore')}>
	<div class="space-y-3 py-4">
		<!-- The steps as they work: lain restore writes only into an empty data folder,
		     so the live one is moved aside first (and kept until the restore is checked). -->
		<p class="text-sm leading-6 text-muted">{t('settings.groups.backup.restoreSteps')}</p>
		<pre
			class="overflow-x-auto rounded-md bg-background px-4 py-3 font-mono text-xs leading-relaxed text-foreground"><code>pkill -x lain
mv ~/.local/share/lain ~/.local/share/lain.before-restore
lain restore ~/Downloads/{archive} --data-dir ~/.local/share/lain
lain serve</code></pre>
		<p class="text-xs text-muted">
			The composition file (<span class="font-mono">composition.json</span>) sits next to the
			database in the data directory and is inspectable on the Plugins page.
		</p>
	</div>
</SettingsGroup>
