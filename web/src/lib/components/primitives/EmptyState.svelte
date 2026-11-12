<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		icon,
		children
	}: {
		title: string;
		description?: string;
		icon?: Snippet;
		children?: Snippet;
	} = $props();
</script>

<!-- Open on the page, no panel: a dashed outline read as a drop target, and a
     grey box as a placeholder left in (D-022: no boxed cards). The icon's
     disc and the space around carry it. -->
<div class="flex flex-col items-center justify-center px-6 py-16 text-center">
	<div class="mb-5 flex size-14 items-center justify-center rounded-full bg-surface-active text-foreground">
		{@render icon?.()}
	</div>
	<h2 class="text-base font-semibold text-foreground">{title}</h2>
	{#if description}
		<p class="mt-1 max-w-md text-sm text-muted">{description}</p>
	{/if}
	{#if children}
		<div class="mt-5 flex flex-wrap items-center justify-center gap-2">
			{@render children()}
		</div>
	{/if}
</div>
