<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';

	let {
		label,
		hint,
		error = null,
		id,
		value = $bindable(''),
		class: className = '',
		...rest
	}: HTMLInputAttributes & {
		label?: string;
		hint?: string;
		error?: string | null;
		class?: string;
	} = $props();

	const uid = $props.id();
	const inputId = $derived(id ?? uid);
</script>

<div class="space-y-1.5">
	{#if label}
		<label for={inputId} class="block text-sm font-medium text-muted">{label}</label>
	{/if}
	<input
		id={inputId}
		bind:value
		aria-invalid={error ? 'true' : undefined}
		class={[
			'h-10 w-full rounded-md border bg-surface px-3 text-sm text-foreground',
			'placeholder:text-muted/60 focus:border-accent/60 focus:outline-none',
			error ? 'border-danger/60' : 'border-line',
			className
		].join(' ')}
		{...rest}
	/>
	{#if error}
		<p class="text-xs text-danger">{error}</p>
	{:else if hint}
		<p class="text-xs text-muted">{hint}</p>
	{/if}
</div>
