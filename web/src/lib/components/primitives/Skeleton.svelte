<script lang="ts">
	let { class: className = '' }: { class?: string } = $props();
</script>

<div class={['animate-pulse rounded-md bg-surface-active/70', className].join(' ')} aria-hidden="true"></div>
