<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Info from '@lucide/svelte/icons/info';
	import X from '@lucide/svelte/icons/x';
	import { toasts } from '$lib/stores/toasts.svelte';

	const icons = {
		error: CircleAlert,
		success: CircleCheck,
		info: Info
	};
	const tones = {
		error: 'text-danger',
		success: 'text-success',
		info: 'text-accent'
	};
</script>

<!-- Above the mobile bottom nav; polite so it never interrupts playback. -->
<div
	class="pointer-events-none fixed bottom-24 right-4 z-[60] flex w-[min(92vw,24rem)] flex-col gap-2 md:bottom-4"
	aria-live="polite"
>
	{#each toasts.items as toast (toast.id)}
		{@const Icon = icons[toast.kind]}
		<div
			class="pointer-events-auto flex items-start gap-3 rounded-card border border-line bg-surface/95 p-3 shadow-xl backdrop-blur"
		>
			<Icon class={['mt-0.5 size-4 shrink-0', tones[toast.kind]]} />
			<p class="flex-1 text-sm text-foreground">{toast.message}</p>
			<button
				class="text-muted hover:text-foreground"
				aria-label="Dismiss"
				onclick={() => toasts.dismiss(toast.id)}
			>
				<X class="size-3.5" />
			</button>
		</div>
	{/each}
</div>
