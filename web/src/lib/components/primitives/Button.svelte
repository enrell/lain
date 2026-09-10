<script lang="ts">
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import { buttonClasses, type ButtonSize, type ButtonVariant } from './button-styles';
	import Spinner from './Spinner.svelte';

	let {
		variant = 'primary',
		size = 'md',
		loading = false,
		disabled = false,
		class: className = '',
		children,
		...rest
	}: HTMLButtonAttributes & {
		variant?: ButtonVariant;
		size?: ButtonSize;
		loading?: boolean;
		class?: string;
	} = $props();

	const classes = $derived(buttonClasses(variant, size, className));
</script>

<button class={classes} disabled={disabled || loading} {...rest}>
	{#if loading}<Spinner class="size-4" />{/if}
	{@render children?.()}
</button>
