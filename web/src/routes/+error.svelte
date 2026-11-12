<script lang="ts">
	/*
	 * Any address the app does not know, or a page that failed to load:
	 * inside the shell, so the header and its search stay in reach, with
	 * a way on from here instead of a bare status line.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import Compass from '@lucide/svelte/icons/compass';
	import { page } from '$app/state';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';

	const missing = $derived(page.status === 404);
	// The way on, focused on arrival: Enter goes Home.
	$effect(() => {
		document.querySelector<HTMLElement>('[data-error-home]')?.focus();
	});
</script>

<svelte:head><title>{missing ? t('errorPage.notFoundTitle') : t('errorPage.failedTitle')} — Lain</title></svelte:head>

<EmptyState
	title={missing ? t('errorPage.notFoundTitle') : t('errorPage.failedTitle')}
	description={missing ? t('errorPage.notFoundBody', { path: page.url.pathname }) : (page.error?.message ?? t('errorPage.failedBody'))}
>
	{#snippet icon()}
		{#if missing}<Compass class="size-6" aria-hidden="true" />{:else}<CircleAlert class="size-6 text-danger" aria-hidden="true" />{/if}
	{/snippet}
	<LinkButton href="/" data-error-home>{t('errorPage.home')}</LinkButton>
	{#if missing}
		<LinkButton href="/library" variant="tonal">{t('nav.library')}</LinkButton>
	{:else}
		<Button variant="tonal" onclick={() => location.reload()}>{t('errorPage.reload')}</Button>
	{/if}
</EmptyState>
<p class="-mt-10 text-center text-xs text-muted max-md:hidden">{t('errorPage.searchHint')}</p>
