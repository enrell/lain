<script lang="ts">
	import { t } from '$lib/i18n';
	/*
	 * Settings shell: a fixed rail (YOU / SERVER) beside one reading column,
	 * like two tiled windows. Navigation is keyboard-first (AGENTS.md):
	 *   g+letter  open a section (mnemonic, stable across roles)
	 *   [ / ]     previous / next section
	 *   j / k     next / previous control in the pane
	 * Ctrl+K (global) finds any setting and lands on its row.
	 */
	import { tick, type Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { session } from '$lib/auth/session.svelte';
	import { prefs } from '$lib/auth/storage';
	import { sectionFor, sectionHint, sectionLabel, visibleSections } from '$lib/settings/sections';

	let { children }: { children: Snippet } = $props();

	const sections = $derived(visibleSections(session.isAdmin));
	const current = $derived(sectionFor(page.url.pathname));
	let pane = $state<HTMLElement>();
	let chord = $state(false);
	let chordTimer: ReturnType<typeof setTimeout> | undefined;

	function typing(t: EventTarget | null): boolean {
		const el = t as HTMLElement | null;
		return !!el && (el.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName));
	}

	function focusables(): HTMLElement[] {
		if (!pane) return [];
		return [
			...pane.querySelectorAll<HTMLElement>(
				'a[href], button:not([disabled]), input:not([type="hidden"]):not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex="0"]'
			)
		].filter((el) => el.offsetParent !== null && !el.closest('[aria-hidden="true"]'));
	}

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (typing(e.target) || document.querySelector('[role="dialog"]')) return;
		const at = sections.findIndex((s) => s.href === current?.href);
		if (chord) {
			chord = false;
			clearTimeout(chordTimer);
			const target = sections.find((s) => s.key === e.key.toLowerCase());
			if (target) {
				e.preventDefault();
				void goto(target.href);
			}
			return;
		}
		if (e.key === 'g') {
			chord = true;
			chordTimer = setTimeout(() => (chord = false), 1200);
			return;
		}
		if (e.key === ']' || e.key === '[') {
			e.preventDefault();
			const to = (at + (e.key === ']' ? 1 : -1) + sections.length) % sections.length;
			void goto(sections[to].href);
			return;
		}
		if (e.key === 'j' || e.key === 'k') {
			const list = focusables();
			if (list.length === 0) return;
			e.preventDefault();
			const i = list.indexOf(document.activeElement as HTMLElement);
			const next = i < 0 ? (e.key === 'j' ? 0 : list.length - 1) : Math.min(Math.max(i + (e.key === 'j' ? 1 : -1), 0), list.length - 1);
			list[next].focus();
			list[next].scrollIntoView({ block: 'nearest' });
		}
	}

	// /settings reopens the section you were last in.
	$effect(() => {
		if (current) prefs.set('settings.last', current.href);
	});

	// A #anchor (from Ctrl+K or a link) scrolls to its row and pulses it.
	$effect(() => {
		const hash = page.url.hash.slice(1);
		void page.url.pathname;
		if (!hash) return;
		let tries = 0;
		const find = () => {
			const el = document.getElementById(hash);
			if (!el) {
				if (tries++ < 80) setTimeout(find, 100); // slow pages load their data first
				return;
			}
			el.scrollIntoView({ block: 'start', behavior: 'smooth' });
			el.classList.remove('setting-pulse');
			void el.offsetWidth;
			el.classList.add('setting-pulse');
			void tick().then(() => el.querySelector<HTMLElement>('input, select, textarea, button')?.focus({ preventScroll: true }));
		};
		find();
	});
</script>

<svelte:window onkeydown={onKey} />

<div class="mx-auto grid max-w-[66rem] gap-8 md:grid-cols-[13.5rem_minmax(0,1fr)] md:gap-12">
	<!-- Rail: fixed like a tiled window. On phones it is a scrollable strip. -->
	<nav class="md:sticky md:top-28 md:self-start" aria-label={t('settings.sections')}>
		<p class="hidden font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-foreground md:block">{t('settings.title')}</p>
		{#each ['you', 'server'] as scope (scope)}
			{@const group = sections.filter((s) => s.scope === scope)}
			{#if group.length > 0}
				<p class="mb-1 mt-6 hidden font-mono text-[10px] uppercase tracking-[0.26em] text-muted md:block">{t(scope === 'you' ? 'settings.scope.you' : 'settings.scope.server')}</p>
				<ul class="no-scrollbar -mx-1 flex gap-1 overflow-x-auto md:mx-0 md:block md:space-y-px">
					{#each group as s (s.href)}
						{@const on = current?.href === s.href}
						<li class="shrink-0">
							<a
								href={s.href}
								aria-current={on ? 'page' : undefined}
								aria-keyshortcuts={`g ${s.key}`}
								title={sectionHint(s)}
								class="group flex items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-sm transition-colors {on
									? 'bg-surface-active text-foreground'
									: 'text-muted hover:bg-surface-hover hover:text-foreground'}"
							>
								<span class="flex items-center gap-2">
									<span class="h-3.5 w-0.5 rounded-full {on ? 'bg-accent' : 'bg-transparent'}" aria-hidden="true"></span>
									{sectionLabel(s)}
								</span>
								<kbd class="hidden font-mono text-[10px] {chord ? 'text-accent' : 'text-muted/60'} md:inline">g{s.key}</kbd>
							</a>
						</li>
					{/each}
				</ul>
			{/if}
		{/each}
		<p class="mt-8 hidden font-mono text-[10px] leading-5 text-muted/70 md:block">
			<kbd class="text-muted">Ctrl K</kbd> {t('settings.hints.find')}<br />
			<kbd class="text-muted">j k</kbd> {t('settings.hints.move')} · <kbd class="text-muted">[ ]</kbd> {t('settings.hints.section')}
		</p>
	</nav>

	<div bind:this={pane} class="settings-body min-w-0 pb-24">
		{#if current}
			<p class="mb-6 font-mono text-[10px] uppercase tracking-[0.22em] text-muted" aria-hidden="true">
				{t('settings.breadcrumb', {
					scope: t(current.scope === 'you' ? 'settings.scope.you' : 'settings.scope.server'),
					section: sectionLabel(current)
				}).toLowerCase()}
			</p>
		{/if}
		{@render children()}
	</div>
</div>
