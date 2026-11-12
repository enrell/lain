<script lang="ts">
	import { t } from '$lib/i18n';
	/*
	 * The single save model of server settings: changes are staged, then
	 * applied together. Ctrl+S applies and Escape discards from anywhere
	 * on the page, so the bar never has to be reached with a mouse.
	 *
	 * Staged work is never lost by accident. Escape is the key people press
	 * to close things, so it discards only when pressed twice; the first press
	 * says so in the bar. Leaving the page with changes staged (the rail,
	 * `[`/`]`, a `g` chord, Quick find, a header link) asks first: apply,
	 * discard or stay. Closing or reloading the tab gets the browser's own
	 * question.
	 */
	import { beforeNavigate, goto } from '$app/navigation';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';

	let {
		count,
		saving = false,
		onapply,
		ondiscard
	}: { count: number; saving?: boolean; onapply: () => void; ondiscard: () => void } = $props();

	usePageKeys(() =>
		count > 0
			? [
					{ keys: ['Ctrl', 'S'], label: t('pageKeys.apply') },
					{ keys: ['Esc', 'Esc'], label: t('pageKeys.discard') }
				]
			: []
	);

	// Escape once arms the discard for a moment; a second Escape carries it out.
	const ARM_MS = 2500;
	let armed = $state(false);
	let armTimer: ReturnType<typeof setTimeout> | undefined;
	function arm(): void {
		armed = true;
		clearTimeout(armTimer);
		armTimer = setTimeout(() => (armed = false), ARM_MS);
	}
	function discard(): void {
		armed = false;
		clearTimeout(armTimer);
		ondiscard();
	}
	$effect(() => {
		if (count === 0) armed = false;
	});
	$effect(() => () => clearTimeout(armTimer));

	function onKey(e: KeyboardEvent): void {
		if (count === 0) return;
		if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
			e.preventDefault();
			if (!saving) onapply();
		} else if (e.key === 'Escape' && !document.querySelector('[role="dialog"]')) {
			const t = e.target as HTMLElement | null;
			if (t && ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName)) return void t.blur();
			if (armed) discard();
			else arm();
		}
	}

	// ---- leaving with changes staged -------------------------------------
	let leaveTo = $state<URL | null>(null);
	let asking = $state(false);
	// Set while the guard lets its own navigation through.
	let passing = false;

	beforeNavigate((nav) => {
		if (count === 0 || passing) return;
		// Closing the tab, reloading or an external link: cancelling asks the browser's own question.
		if (nav.willUnload || !nav.to) {
			nav.cancel();
			return;
		}
		nav.cancel();
		leaveTo = nav.to.url;
		asking = true;
		// The dialog opens on its primary action, so Enter applies and leaves.
		focusSoon(() => document.getElementById('staged-apply-leave'));
	});
	// Closed with Esc or its X: the viewer stays, and the blocked navigation is forgotten.
	$effect(() => {
		if (!asking && !applying) leaveTo = null;
	});

	function proceed(): void {
		const to = leaveTo;
		asking = false;
		leaveTo = null;
		if (!to) return;
		passing = true;
		void goto(to.pathname + to.search + to.hash).finally(() => (passing = false));
	}

	function discardAndLeave(): void {
		discard();
		proceed();
	}

	// Apply, then leave once the page has saved; a failed save (the page says
	// why) keeps the viewer here with the changes still staged.
	let applying = $state(false);
	let sawSaving = false;
	let applyTimer: ReturnType<typeof setTimeout> | undefined;
	function applyAndLeave(): void {
		asking = false;
		applying = true;
		sawSaving = false;
		onapply();
		// A save that never starts (nothing valid to send) must not hold the guard forever.
		clearTimeout(applyTimer);
		applyTimer = setTimeout(() => {
			if (!sawSaving) applying = false;
		}, 1500);
	}
	$effect(() => {
		if (!applying) return;
		if (saving) {
			sawSaving = true;
			return;
		}
		if (!sawSaving) return;
		applying = false;
		clearTimeout(applyTimer);
		if (count === 0) proceed();
		else leaveTo = null;
	});
	$effect(() => () => clearTimeout(applyTimer));
</script>

<svelte:window onkeydown={onKey} />

{#if count > 0}
	<div
		class="fixed inset-x-0 bottom-16 z-40 border-t border-hairline bg-background/92 backdrop-blur-xl md:bottom-0"
		role="region"
		aria-label={t('settings.staged.region')}
	>
		<div class="mx-auto flex max-w-[1800px] items-center justify-between gap-4 px-5 py-3 sm:px-8 lg:px-10">
			<p class="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-foreground" aria-live="polite">
				<span class="size-1.5 rounded-full bg-warning" aria-hidden="true"></span>
				{armed ? t('settings.staged.armed') : t('settings.staged.count', { count })}
			</p>
			<div class="flex items-center gap-2">
				<Button variant="ghost" size="sm" disabled={saving} onclick={discard} class={armed ? 'text-danger' : ''}>
					{t('settings.staged.discard')} <kbd class="ml-1 font-mono text-[10px] text-muted">Esc</kbd>
				</Button>
				<Button size="sm" loading={saving} onclick={onapply}>
					{t('settings.staged.apply')} <kbd class="ml-1 font-mono text-[10px] opacity-70">Ctrl+S</kbd>
				</Button>
			</div>
		</div>
	</div>
{/if}

<Modal
	bind:open={asking}
	title={t('settings.staged.leaveTitle')}
	description={t('settings.staged.leaveBody', { count })}
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (asking = false)}>{t('settings.staged.stay')} <kbd class="ml-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button variant="secondary" onclick={discardAndLeave}>{t('settings.staged.discardLeave')}</Button>
		<Button id="staged-apply-leave" onclick={applyAndLeave}>{t('settings.staged.applyLeave')} <kbd class="ml-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
