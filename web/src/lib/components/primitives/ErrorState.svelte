<script lang="ts">
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import Button from './Button.svelte';

	let {
		message,
		retry
	}: {
		message: string;
		retry?: () => void;
	} = $props();
</script>

<div
	class="flex flex-col items-center justify-center rounded-card border border-danger/20 bg-danger/5 px-6 py-10 text-center"
	role="alert"
>
	<TriangleAlert class="mb-3 size-6 text-danger" aria-hidden="true" />
	<p class="max-w-md text-sm text-foreground">{message}</p>
	{#if retry}
		<Button variant="secondary" size="sm" class="mt-4" onclick={retry}>
			<RefreshCw class="size-3.5" /> Try again
		</Button>
	{/if}
</div>
