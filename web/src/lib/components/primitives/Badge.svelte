<script lang="ts">
	let {
		tone = 'neutral',
		children
	}: {
		tone?: 'neutral' | 'accent' | 'danger' | 'success' | 'warning';
		children?: import('svelte').Snippet;
	} = $props();

	const tones = {
		neutral: 'border-line bg-surface-active text-muted',
		accent: 'border-accent/30 bg-accent/10 text-accent',
		danger: 'border-danger/30 bg-danger/10 text-danger',
		success: 'border-success/30 bg-success/10 text-success',
		warning: 'border-warning/30 bg-warning/10 text-warning'
	};
</script>

<span
	class={[
		'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium tracking-wide',
		tones[tone]
	].join(' ')}
>
	{@render children?.()}
</span>
