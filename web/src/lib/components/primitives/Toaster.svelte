<script lang="ts">
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import Info from '@lucide/svelte/icons/info';
	import X from '@lucide/svelte/icons/x';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { isTypingTarget } from '$lib/utilities/guards';

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

	// The newest toast with an action (an Undo) answers Ctrl+Z from anywhere but a
	// field, which keeps its own undo: the toast sits far down the Tab order.
	const actionable = $derived([...toasts.items].reverse().find((toast) => toast.action));
	function onKey(e: KeyboardEvent): void {
		if (!actionable || !(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== 'z') return;
		if (isTypingTarget(e.target)) return;
		e.preventDefault();
		actionable.action?.run();
		toasts.dismiss(actionable.id);
	}
</script>

<svelte:window onkeydown={onKey} />

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
			{#if toast.action}
				<button
					type="button"
					class="-my-0.5 shrink-0 rounded-md px-2 py-0.5 text-sm font-semibold text-accent hover:bg-surface-hover"
					onclick={() => {
						toast.action?.run();
						toasts.dismiss(toast.id);
					}}
				>
					{toast.action.label}
					{#if toast === actionable}<kbd class="ms-1 font-mono text-[10px] font-normal opacity-70">Ctrl+Z</kbd>{/if}
				</button>
			{/if}
			<button
				class="text-muted hover:text-foreground"
				aria-label={t('toast.dismiss')}
				onclick={() => toasts.dismiss(toast.id)}
			>
				<X class="size-3.5" />
			</button>
		</div>
	{/each}
</div>
