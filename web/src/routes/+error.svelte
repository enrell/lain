<script lang="ts">
	/*
	 * Any address the app does not know, or a page that failed to load:
	 * inside the shell, so the header and its search stay in reach, and
	 * drawn as the first-run pages open (an accent mono eyebrow, a large
	 * tight line, plain words, the way on as the main action), not a bare
	 * status line or a centred placeholder.
	 *
	 * The way on is focused on arrival, so Enter takes it. Back is offered
	 * only when this app sent the viewer here: on an address typed or
	 * opened from elsewhere, Back would leave Lain.
	 */
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import { afterNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { isTypingTarget } from '$lib/utilities/guards';

	const missing = $derived(page.status === 404);
	// A link to an address no route knows is followed as a full page load
	// (the router hands it to the server), so the page Lain came from is the
	// referrer then, not the navigation's `from`.
	let referred = false;
	try {
		referred = document.referrer !== '' && new URL(document.referrer).origin === location.origin && history.length > 1;
	} catch {
		/* no referrer to judge */
	}
	let cameFromApp = $state(referred);
	afterNavigate((nav) => {
		if (nav.from !== null && nav.type !== 'enter') cameFromApp = true;
	});

	$effect(() => {
		document.querySelector<HTMLElement>('[data-error-main]')?.focus();
	});

	usePageKeys(() => [
		...(cameFromApp ? [{ keys: ['Backspace'], label: t('errorPage.back') }] : []),
		{ keys: ['L'], label: t('nav.library') },
		...(missing ? [] : [{ keys: ['R'], label: t('errorPage.reload') }])
	]);

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		const key = e.key.toLowerCase();
		if (e.key === 'Backspace' && cameFromApp) {
			e.preventDefault();
			history.back();
		} else if (key === 'l') {
			e.preventDefault();
			void goto('/library');
		} else if (key === 'r' && !missing) {
			e.preventDefault();
			location.reload();
		}
	}
</script>

<svelte:head><title>{missing ? t('errorPage.notFoundTitle') : t('errorPage.failedTitle')} — Lain</title></svelte:head>
<svelte:window onkeydown={onKey} />

<section class="relative isolate flex min-h-[calc(100dvh-14rem)] items-center md:min-h-[calc(100dvh-16rem)]">
	<!-- The node lattice behind empty screens, faded out on every side. -->
	<div
		class="lattice pointer-events-none absolute inset-y-0 -right-10 -z-10 w-full opacity-40 [mask-image:radial-gradient(closest-side,black,transparent)] md:w-2/3"
		aria-hidden="true"
	></div>
	<!-- The app's mark, large and faint where the lattice is densest: the
	     signal that did not find its node. -->
	<SignalMark class="pointer-events-none absolute right-[14%] top-1/2 -z-10 hidden size-56 -translate-y-1/2 text-accent/20 lg:block" />


	<div class="max-w-2xl py-6">
		<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent sm:text-[11px]">
			{missing ? t('errorPage.notFoundEyebrow') : t('errorPage.failedEyebrow', { status: page.status })}
		</p>
		<h1 class="mt-4 max-w-[18ch] text-[2rem] font-semibold leading-[1.04] tracking-[-0.035em] text-balance text-foreground sm:text-5xl sm:leading-[1.02]">
			{missing ? t('errorPage.notFoundTitle') : t('errorPage.failedTitle')}
		</h1>
		{#if missing}
			<p class="mt-4 max-w-xl text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7">{t('errorPage.notFoundBody')}</p>
			<!-- The address as typed: the mistake is easier to see set apart. -->
			<p class="mt-3 inline-block max-w-full truncate rounded-md bg-field px-2.5 py-1 font-mono text-xs text-foreground/80">{page.url.pathname}</p>
		{:else}
			<p class="mt-4 max-w-xl text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7">{page.error?.message || t('errorPage.failedBody')}</p>
		{/if}

		<div class="mt-7 flex flex-wrap items-center gap-3">
			{#if missing}
				<LinkButton href="/" size="rowMain" data-error-main>{t('errorPage.home')}</LinkButton>
			{:else}
				<Button size="rowMain" data-error-main onclick={() => location.reload()}>{t('errorPage.reload')} <kbd>R</kbd></Button>
			{/if}
			{#if cameFromApp}
				<Button variant="tonal" size="row" onclick={() => history.back()}>
					<ArrowLeft class="size-4" aria-hidden="true" />{t('errorPage.back')} <kbd>⌫</kbd>
				</Button>
			{/if}
			<LinkButton href="/library" variant="tonal" size="row">{t('nav.library')} <kbd>L</kbd></LinkButton>
			{#if !missing}
				<LinkButton href="/" variant="tonal" size="row">{t('errorPage.home')}</LinkButton>
			{/if}
		</div>
		<p class="mt-8 text-xs text-muted max-md:hidden">{t('errorPage.searchHint')}</p>
	</div>
</section>
