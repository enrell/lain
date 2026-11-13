<script lang="ts">
	/*
	 * Nothing in a list that sits under its own heading (a tab's section, the
	 * friends): the app's empty state at the size of a row, a disc with the
	 * icon and one or two lines beside it. EmptyState is for a whole page or
	 * tab; a grey line here read as something left over.
	 */
	import type { Snippet } from 'svelte';

	let { title, description, icon }: { title: string; description?: string; icon: Snippet } = $props();
</script>

<div class="flex items-center gap-4 py-5">
	<span class="flex size-11 shrink-0 items-center justify-center rounded-full bg-surface-active text-muted [&>svg]:size-5" aria-hidden="true">
		{@render icon()}
	</span>
	<span class="min-w-0">
		<span class="block text-sm font-semibold text-foreground">{title}</span>
		{#if description}<span class="mt-0.5 block text-sm text-muted">{description}</span>{/if}
	</span>
</div>
