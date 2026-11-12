<script lang="ts">
	/*
	 * One group of settings: a heading and a one-line description above a
	 * tonal panel, the way a home rail sits under its title. Rows and list
	 * items inside the panel get its inner padding and the hairlines
	 * between them from app.css (.settings-card). `open` makes the group
	 * collapsible (progressive disclosure, D-062): the heading is then the
	 * summary that opens it, and the panel is what it hides.
	 */
	import type { Snippet } from 'svelte';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';

	let {
		id,
		title,
		description,
		open = $bindable(),
		aside,
		children
	}: {
		id?: string;
		title: string;
		description?: string;
		/** Set (true or false) to make the group collapsible. */
		open?: boolean;
		aside?: Snippet;
		children: Snippet;
	} = $props();

	// Decided once: a group does not switch between collapsible and fixed.
	// svelte-ignore state_referenced_locally
	const collapsible = open !== undefined;
</script>

{#snippet heading()}
	<div class="min-w-0">
		<h2 class="flex items-center gap-2 text-[1.0625rem] font-semibold leading-6 tracking-[-0.01em] text-foreground">
			{title}
			{#if collapsible}
				<ChevronDown class="size-4 shrink-0 text-muted transition-transform group-open/settings:rotate-180" aria-hidden="true" />
			{/if}
		</h2>
		{#if description}<p class="mt-0.5 text-sm leading-5 text-muted">{description}</p>{/if}
	</div>
	{#if aside}<div class="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">{@render aside()}</div>{/if}
{/snippet}

{#if collapsible}
	<details {id} class="settings-group group/settings mb-10 scroll-mt-28" bind:open>
		<summary class="flex cursor-pointer flex-col gap-3 rounded-md pb-3 outline-offset-4 sm:flex-row sm:items-end sm:justify-between sm:gap-4">
			{@render heading()}
		</summary>
		<div class="settings-card">{@render children()}</div>
	</details>
{:else}
	<section {id} class="settings-group mb-10 scroll-mt-28">
		<header class="flex flex-col gap-3 pb-3 sm:flex-row sm:items-end sm:justify-between sm:gap-4">
			{@render heading()}
		</header>
		<div class="settings-card">{@render children()}</div>
	</section>
{/if}
