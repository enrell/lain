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
			toasts.success(t('settings.backupPage.saved', { name }));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.backupPage.failed')));
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>{t('settings.backupPage.pageTitle')}</title></svelte:head>

<SettingsGroup id="backup" title={t('settings.backupPage.snapshotTitle')} description={t('settings.groups.backup.snapshot')}>
	{#snippet aside()}
		<Button size="field" loading={busy} onclick={() => void download()}>
			<Download class="size-4" /> {t('settings.backupPage.download')}
		</Button>
	{/snippet}
	<!-- What the archive is, as lain-admin's bundle writes it: a tar of a
	     consistent redb snapshot (lain.redb) and the data folder's documents
	     and avatars, read in one transaction. -->
	<p class="py-4 text-sm leading-6 text-muted">{t('settings.backupPage.what')}</p>
</SettingsGroup>

<SettingsGroup id="restore" title={t('settings.backupPage.restoreTitle')} description={t('settings.groups.backup.restore')}>
	<div class="space-y-3 py-4">
		<!-- One command does the restore: --replace checks the archive, refuses
		     a store a running server holds, and keeps the old data folder
		     beside the new one (lain restore, crates/lain/src/backup.rs). -->
		<p class="text-sm leading-6 text-muted">{t('settings.groups.backup.restoreSteps')}</p>
		<pre
			class="overflow-x-auto rounded-md bg-background px-4 py-3 font-mono text-xs leading-relaxed text-foreground"><code>lain restore ~/Downloads/{archive} --replace</code></pre>
		<p class="text-xs leading-5 text-muted">{t('settings.backupPage.restoreNotes')}</p>
	</div>
</SettingsGroup>
