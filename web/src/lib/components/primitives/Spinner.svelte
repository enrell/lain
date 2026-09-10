<script lang="ts">
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';

	let { class: className = '', label = 'Loading' }: { class?: string; label?: string } = $props();
</script>

<LoaderCircle class={['animate-spin', className].join(' ')} aria-label={label} role="status" />
