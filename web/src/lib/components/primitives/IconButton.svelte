<script lang="ts">
	import { Tooltip } from 'bits-ui';
	import type { Snippet } from 'svelte';

	let {
		label,
		side = 'top',
		disabled = false,
		type = 'button',
		class: className = '',
		onclick,
		children
	}: {
		/** Accessible name and tooltip text: both are required for icon buttons. */
		label: string;
		side?: 'top' | 'right' | 'bottom' | 'left';
		disabled?: boolean;
		type?: 'button' | 'submit' | 'reset';
		class?: string;
		onclick?: (event: MouseEvent) => void;
		children?: Snippet;
	} = $props();

	const classes = $derived(
		[
			'inline-flex size-9 items-center justify-center rounded-md text-muted transition-colors duration-150',
			'hover:bg-surface-hover hover:text-foreground disabled:opacity-40 disabled:pointer-events-none',
			className
		].join(' ')
	);
</script>

<Tooltip.Root delayDuration={450}>
	<Tooltip.Trigger {type} {disabled} aria-label={label} class={classes} {onclick}>
		{@render children?.()}
	</Tooltip.Trigger>
	<Tooltip.Content
		{side}
		sideOffset={6}
		class="z-50 rounded-md border border-line bg-surface px-2 py-1 text-xs text-foreground shadow-lg"
	>
		{label}
	</Tooltip.Content>
</Tooltip.Root>
