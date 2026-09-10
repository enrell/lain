<script lang="ts">
	let { class: className = '' }: { class?: string } = $props();
</script>

<svg viewBox="0 0 64 64" class={className} fill="none" aria-hidden="true">
	<path
		d="M18 22 L46 18 M18 22 L32 46 M46 18 L32 46"
		stroke="currentColor"
		stroke-width="4"
		stroke-linecap="round"
	/>
	<circle cx="18" cy="22" r="5.5" fill="var(--color-background)" stroke="currentColor" stroke-width="4" />
	<circle cx="46" cy="18" r="5.5" fill="var(--color-background)" stroke="currentColor" stroke-width="4" />
	<circle cx="32" cy="46" r="6" fill="currentColor" />
</svg>
