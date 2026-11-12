<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLInputAttributes } from 'svelte/elements';

	let {
		label,
		hint,
		error = null,
		id,
		value = $bindable(''),
		edged = false,
		end,
		class: className = '',
		...rest
	}: HTMLInputAttributes & {
		label?: string;
		hint?: string;
		error?: string | null;
		/**
		 * A field with an edge of its own, a step taller: the sign-in and setup
		 * cards, where a borderless field on the card's surface was hard to find.
		 * The edge carries the focus colour, since the field has no outline.
		 */
		edged?: boolean;
		/** A control inside the field's end (the password's show/hide). */
		end?: Snippet;
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
	<div class="relative">
		<input
			id={inputId}
			bind:value
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy}
			class={[
				'w-full rounded-md border bg-field px-3 text-sm text-foreground',
				'placeholder:text-muted/60 focus:outline-none',
				edged ? 'h-11 focus:border-accent' : 'h-10 focus:border-accent/60',
				error ? 'border-danger/60' : edged ? 'border-line' : 'border-transparent',
				end ? 'pe-11' : '',
				className
			].join(' ')}
			{...rest}
		/>
		{#if end}
			<div class="absolute inset-y-0 end-0 flex items-center">{@render end()}</div>
		{/if}
	</div>
	{#if error}
		<p id="{inputId}-error" class="text-xs text-danger">{error}</p>
	{:else if hint}
		<p id="{inputId}-hint" class="text-xs text-muted">{hint}</p>
	{/if}
</div>
