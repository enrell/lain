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
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { sectionFor, sectionHint, sectionLabel, visibleSections } from '$lib/settings/sections';

	let { children }: { children: Snippet } = $props();

	const sections = $derived(visibleSections(session.isAdmin));
	const current = $derived(sectionFor(page.url.pathname));
	let pane = $state<HTMLElement>();
	// Phones pick a section from a sheet instead of two strips of tabs.
	let picker = $state(false);
	$effect(() => {
		void page.url.pathname;
		picker = false;
	});
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

<!-- The rail starts at the page's left edge, under the logo, as every other
     page's content does; the reading column beside it is held to a width
     where a label and its control are still one glance apart. The single
     phone column must be allowed to shrink: an auto track is as wide as its
     widest row and pushed every page off the screen. -->
<div class="grid grid-cols-[minmax(0,1fr)] md:grid-cols-[12rem_minmax(0,1fr)] md:gap-12 lg:grid-cols-[13rem_minmax(0,60rem)] lg:gap-16">
	<!-- Rail: the app's navigation turned on its side. The same 13px type and
	     muted links, and the page you are on drawn the way the header draws
	     it: brighter, heavier, with an accent rule (there under the word,
	     here on the rail's edge). A fill plus a bar inset in it read as a
	     text caret, and the heavy surface swallowed the key cap. -->
	<nav
		class="no-scrollbar hidden md:sticky md:top-28 md:block md:max-h-[calc(100dvh-8rem)] md:self-start md:overflow-y-auto md:pb-6"
		aria-label={t('settings.sections')}
	>
		{#each ['you', 'server'] as scope (scope)}
			{@const group = sections.filter((s) => s.scope === scope)}
			{#if group.length > 0}
				<p class="mb-2 mt-7 font-mono text-[10px] uppercase tracking-[0.26em] text-muted first:mt-1">
					{t(scope === 'you' ? 'settings.scope.you' : 'settings.scope.server')}
				</p>
				<ul class="border-s border-hairline">
					{#each group as s (s.href)}
						{@const on = current?.href === s.href}
						<li>
							<a
								href={s.href}
								aria-current={on ? 'page' : undefined}
								aria-keyshortcuts={`g ${s.key}`}
								title={sectionHint(s)}
								class="-ms-px flex h-8 items-center justify-between gap-3 border-s-2 ps-4 text-[13px] tracking-wide transition-colors {on
									? 'border-accent font-semibold text-foreground'
									: 'border-transparent text-muted hover:border-hairline hover:text-foreground'}"
							>
								<span class="truncate">{sectionLabel(s)}</span>
								<kbd class={chord ? 'text-accent' : on ? 'text-muted' : 'text-muted/60'}>g{s.key}</kbd>
							</a>
						</li>
					{/each}
				</ul>
			{/if}
		{/each}
		<!-- Leaving the account sits where accounts are managed, not only behind the avatar's menu. -->
		<button
			type="button"
			class="mt-7 flex h-8 w-full items-center gap-2 ps-[1.125rem] text-left text-[13px] tracking-wide text-muted transition-colors hover:text-foreground"
			onclick={() => {
				session.logout();
				void goto('/login');
			}}
		>
			<LogOut class="size-3.5" aria-hidden="true" /> {t('common.signOut')}
		</button>
		<p class="mt-6 space-y-1.5 font-mono text-[10px] leading-5 text-muted/80">
			<span class="flex items-center gap-2"><kbd>Ctrl K</kbd> {t('settings.hints.find')}</span>
			<span class="flex items-center gap-2"><kbd>g</kbd> {t('settings.hints.jump')}</span>
			<span class="flex items-center gap-2"><kbd>j k</kbd> {t('settings.hints.move')} <kbd>[ ]</kbd> {t('settings.hints.section')}</span>
		</p>
	</nav>

	<div bind:this={pane} class="settings-body min-w-0 pb-24">
		{#if current}
			<!-- The page's own title, like a home section's: the scope as a mono
			     eyebrow, the section in large tight type, what it holds below. On
			     a phone the title is also the section picker. -->
			<header class="mb-8 md:mb-10">
				<p class="font-mono text-[10px] uppercase tracking-[0.26em] text-muted">
					{t('settings.eyebrow', { scope: t(current.scope === 'you' ? 'settings.scope.you' : 'settings.scope.server') })}
				</p>
				<h1 class="mt-2 text-[1.75rem] font-bold leading-tight tracking-[-0.03em] text-foreground md:text-[2rem]">
					<button
						type="button"
						class="-mx-1 inline-flex max-w-full items-center gap-2 rounded-md px-1 text-left md:hidden"
						aria-haspopup="dialog"
						aria-label={t('settings.pick', { section: sectionLabel(current) })}
						onclick={() => (picker = true)}
					>
						<span class="truncate">{sectionLabel(current)}</span>
						<ChevronDown class="size-6 shrink-0 text-muted" aria-hidden="true" />
					</button>
					<span class="hidden md:inline">{sectionLabel(current)}</span>
				</h1>
				<p class="mt-1.5 text-sm text-muted">{sectionHint(current)}</p>
			</header>
		{/if}
		{@render children()}
	</div>
</div>

<Modal bind:open={picker} title={t('settings.title')}>
	{#each ['you', 'server'] as scope (scope)}
		{@const group = sections.filter((s) => s.scope === scope)}
		{#if group.length > 0}
			<p class="mb-1 mt-5 font-mono text-[10px] uppercase tracking-[0.26em] text-muted first:mt-0">{t(scope === 'you' ? 'settings.scope.you' : 'settings.scope.server')}</p>
			<ul class="-mx-2">
				{#each group as s (s.href)}
					{@const on = current?.href === s.href}
					<li>
						<a
							href={s.href}
							aria-current={on ? 'page' : undefined}
							class="flex min-h-12 items-center gap-3 rounded-lg px-3 py-2 {on ? 'bg-foreground/[0.06]' : 'active:bg-surface-hover'}"
						>
							<span class="h-6 w-0.5 shrink-0 rounded-full {on ? 'bg-accent' : 'bg-transparent'}" aria-hidden="true"></span>
							<span class="min-w-0">
								<span class="block text-[15px] {on ? 'font-semibold' : 'font-medium'} text-foreground">{sectionLabel(s)}</span>
								<span class="block truncate text-xs text-muted">{sectionHint(s)}</span>
							</span>
						</a>
					</li>
				{/each}
			</ul>
		{/if}
	{/each}
</Modal>
