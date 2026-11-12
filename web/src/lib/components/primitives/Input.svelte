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
	// The message under the field is read with it: the error when there is one, else the hint.
	const describedBy = $derived(error ? `${inputId}-error` : hint ? `${inputId}-hint` : undefined);
</script>

<div class="space-y-1.5">
	{#if label}
		<label for={inputId} class="block text-sm font-medium text-muted">{label}</label>
	{/if}
	<input
		id={inputId}
		bind:value
		aria-invalid={error ? 'true' : undefined}
		aria-describedby={describedBy}
		class={[
			'h-10 w-full rounded-md border bg-field px-3 text-sm text-foreground',
			'placeholder:text-muted/60 focus:border-accent/60 focus:outline-none',
			error ? 'border-danger/60' : 'border-transparent',
			className
		].join(' ')}
		{...rest}
	/>
	{#if error}
		<p id="{inputId}-error" class="text-xs text-danger">{error}</p>
	{:else if hint}
		<p id="{inputId}-hint" class="text-xs text-muted">{hint}</p>
	{/if}
</div>
