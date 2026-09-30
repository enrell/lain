<script lang="ts">
	/*
	 * Ctrl+K: the one launcher (AGENTS.md: keyboard-first). Every settings
	 * section and setting row, the common actions, and library titles, in
	 * one fuzzy list. ↑↓ or Ctrl+N/P move, Enter runs, Escape closes.
	 */
	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api';
	import type { CatalogItem } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import { SETTING_ENTRIES, entryLabel, fuzzyScore, sectionHint, sectionLabel, visibleSections } from '$lib/settings/sections';
	import { scan } from '$lib/stores/scan.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	interface Cmd {
		id: string;
		/** Stable group id; the heading text comes from `palette.group.*`. */
		group: 'actions' | 'goTo' | 'settings' | 'titles';
		label: string;
		detail?: string;
		keys?: string;
		run: () => void | Promise<void>;
	}

	let open = $state(false);
	let query = $state('');
	let active = $state(0);
	let input = $state<HTMLInputElement>();
	let listEl = $state<HTMLUListElement>();
	let titles = $state<CatalogItem[]>([]);
	let lastFocus: HTMLElement | null = null;

	const go = (href: string) => () => void goto(href);

	const staticCommands = $derived.by((): Cmd[] => {
		const admin = session.isAdmin;
		const out: Cmd[] = [];
		if (admin) {
			out.push(
				{
					id: 'scan',
					group: 'actions',
					label: t('palette.action.scan'),
					detail: t('palette.action.scanDetail'),
					run: async () => {
						try {
							await scan.start();
							toasts.success(t('palette.action.scanStarted'));
						} catch (err) {
							toasts.error(errorMessage(err, t('palette.action.scanFailed')));
						}
					}
				},
				{ id: 'add-library', group: 'actions', label: t('palette.action.addLibrary'), run: go('/settings/libraries?add') },
				{ id: 'add-user', group: 'actions', label: t('palette.action.addUser'), run: go('/settings/users?add') },
				{ id: 'backup', group: 'actions', label: t('palette.action.backup'), run: go('/settings/backup#backup') }
			);
		}
		out.push({
			id: 'sign-out',
			group: 'actions',
			label: t('palette.action.signOut'),
			run: () => {
				session.logout();
				void goto('/login');
			}
		});
		for (const [href, label] of [
			['/', t('nav.home')],
			['/library', t('nav.library')],
			['/list', t('nav.list')],
			['/search', t('nav.search')]
		]) {
			out.push({ id: `nav-${href}`, group: 'goTo', label, run: go(href) });
		}
		for (const s of visibleSections(admin)) {
			out.push({
				id: `sec-${s.href}`,
				group: 'goTo',
				label: t('palette.sectionPrefix', { section: sectionLabel(s) }),
				detail: sectionHint(s),
				keys: `g${s.key}`,
				run: go(s.href)
			});
		}
		for (const e of SETTING_ENTRIES) {
			if (e.admin && !admin) continue;
			const section = visibleSections(admin).find((s) => s.href === e.href);
			out.push({
				id: `set-${e.href}-${e.anchor}`,
				group: 'settings',
				label: entryLabel(e),
				detail: section ? sectionLabel(section) : undefined,
				run: go(`${e.href}#${e.anchor}`)
			});
		}
		return out;
	});

	function haystack(c: Cmd): string {
		const entry = SETTING_ENTRIES.find((e) => c.id === `set-${e.href}-${e.anchor}`);
		return [c.label, c.detail, entry?.keywords].filter(Boolean).join(' ');
	}

	const results = $derived.by((): Cmd[] => {
		const q = query.trim();
		const ranked = q
			? staticCommands
					.map((c) => ({ c, score: fuzzyScore(q, haystack(c)) }))
					.filter((r) => r.score > 0)
					.sort((a, b) => b.score - a.score)
					.map((r) => r.c)
					.slice(0, 12)
			: staticCommands.filter((c) => c.group !== 'settings').slice(0, 14);
		// One entry per title: a series has many files but one page.
		const seen = new Set<string>();
		const unique = titles.filter((t) => {
			const k = t.title.toLowerCase();
			if (seen.has(k)) return false;
			seen.add(k);
			return true;
		});
		const titleCmds: Cmd[] = unique.slice(0, 5).map((t) => ({
			id: `title-${t.id}`,
			group: 'titles',
			label: t.title,
			detail: t.kind,
			run: go(`/item/${t.id}`)
		}));
		// Grouped rendering needs contiguous groups; rank order holds within each.
		const order = ['actions', 'goTo', 'settings', 'titles'];
		return [...ranked, ...titleCmds].sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group));
	});

	// Titles come from the server; a stale answer never overwrites a newer one.
	let seq = 0;
	$effect(() => {
		const q = query.trim();
		const mine = ++seq;
		if (!open || q.length < 2) {
			titles = [];
			return;
		}
		const t = setTimeout(async () => {
			try {
				const page = await api.search.query({ q, limit: 20 });
				if (mine === seq) titles = page.items;
			} catch {
				if (mine === seq) titles = [];
			}
		}, 140);
		return () => clearTimeout(t);
	});

	$effect(() => {
		void results;
		active = 0;
	});

	async function show(): Promise<void> {
		lastFocus = document.activeElement as HTMLElement | null;
		open = true;
		query = '';
		await tick();
		input?.focus();
	}

	function close(restore = true): void {
		open = false;
		if (restore) lastFocus?.focus?.();
	}

	async function run(cmd: Cmd | undefined): Promise<void> {
		if (!cmd) return;
		close(false);
		await cmd.run();
	}

	function onWindowKey(e: KeyboardEvent): void {
		if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
			e.preventDefault();
			if (open) close();
			else void show();
		}
	}

	function onInputKey(e: KeyboardEvent): void {
		const down = e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n');
		const up = e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p');
		if (down || up) {
			e.preventDefault();
			if (results.length === 0) return;
			active = (active + (down ? 1 : -1) + results.length) % results.length;
			void tick().then(() => listEl?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }));
		} else if (e.key === 'Enter') {
			e.preventDefault();
			void run(results[active]);
		} else if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if (e.key === 'Tab') {
			e.preventDefault(); // focus stays in the launcher
		}
	}
</script>

<svelte:window onkeydown={onWindowKey} />

{#if open}
	<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
	<div class="fixed inset-0 z-[60] bg-black/55 backdrop-blur-[2px]" onclick={() => close()}></div>
	<div
		class="fixed left-1/2 top-[14vh] z-[61] w-[min(92vw,38rem)] -translate-x-1/2 overflow-hidden rounded-xl border border-hairline bg-background/95 shadow-[0_30px_80px_rgba(0,0,0,0.6)] backdrop-blur-xl"
		role="dialog"
		aria-modal="true"
		aria-label={t('palette.label')}
	>
		<div class="flex items-center gap-3 border-b border-hairline px-4">
			<span class="font-mono text-xs text-accent" aria-hidden="true">›</span>
			<input
				bind:this={input}
				bind:value={query}
				onkeydown={onInputKey}
				role="combobox"
				aria-expanded="true"
				aria-controls="palette-list"
				aria-activedescendant={results[active] ? `palette-${results[active].id}` : undefined}
				placeholder={t('palette.placeholder')}
				class="h-12 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted/70"
				spellcheck="false"
				autocomplete="off"
			/>
			<kbd class="font-mono text-[10px] text-muted">Esc</kbd>
		</div>
		<ul bind:this={listEl} id="palette-list" role="listbox" class="max-h-[58vh] overflow-y-auto py-1.5" aria-label={t('palette.results')}>
			{#each results as cmd, i (cmd.id)}
				{#if i === 0 || results[i - 1].group !== cmd.group}
					<li role="presentation" class="px-4 pb-1 pt-2.5 font-mono text-[9px] uppercase tracking-[0.26em] text-muted">{t(`palette.group.${cmd.group}`)}</li>
				{/if}
				<li
					id="palette-{cmd.id}"
					role="option"
					aria-selected={i === active}
					class="mx-1.5 flex cursor-default items-center justify-between gap-3 rounded-md px-2.5 py-2 text-sm {i === active
						? 'bg-surface-active text-foreground'
						: 'text-foreground/85'}"
					onpointermove={() => (active = i)}
					onclick={() => void run(cmd)}
					onkeydown={() => {}}
				>
					<span class="min-w-0 truncate">
						{cmd.label}
						{#if cmd.detail}<span class="ml-2 text-xs text-muted">{cmd.detail}</span>{/if}
					</span>
					{#if cmd.keys}<kbd class="shrink-0 font-mono text-[10px] text-muted">{cmd.keys}</kbd>{/if}
				</li>
			{:else}
				<li class="px-4 py-6 text-center text-sm text-muted">{t('palette.empty', { query: query.trim() })}</li>
			{/each}
		</ul>
		<div class="flex items-center gap-4 border-t border-hairline px-4 py-2 font-mono text-[10px] text-muted">
			<span><kbd>↑↓</kbd> {t('palette.move')}</span><span><kbd>↵</kbd> {t('palette.run')}</span><span><kbd>Ctrl K</kbd> {t('palette.toggle')}</span>
		</div>
	</div>
{/if}
