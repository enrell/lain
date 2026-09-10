<script lang="ts">
	import type { HTMLAnchorAttributes } from 'svelte/elements';
	import { buttonClasses, type ButtonSize, type ButtonVariant } from './button-styles';

	let {
		variant = 'primary',
		size = 'md',
		class: className = '',
		href,
		children,
		...rest
	}: HTMLAnchorAttributes & {
		variant?: ButtonVariant;
		size?: ButtonSize;
		class?: string;
		href: string;
	} = $props();
</script>

<a {href} class={buttonClasses(variant, size, className)} {...rest}>
	{@render children?.()}
</a>
