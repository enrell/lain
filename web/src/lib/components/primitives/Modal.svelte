<script lang="ts">
	import { Dialog } from 'bits-ui';
	import X from '@lucide/svelte/icons/x';

	let {
		open = $bindable(false),
		title,
		description,
		children,
		footer
	}: {
		open?: boolean;
		title: string;
		description?: string;
		children?: import('svelte').Snippet;
		footer?: import('svelte').Snippet;
	} = $props();

	/*
	 * Keyboard-first: a dialog opens on what it asks for. Left to itself it
	 * focused its first tabbable, the close button, so Add a library and Add
	 * a user needed a Tab before the first keystroke landed. The first field
	 * (or an element marked data-autofocus) takes focus; a dialog with no
	 * field keeps the default.
	 */
	let body = $state<HTMLElement | null>(null);
	function landFocus(event: Event): void {
		const field = body?.querySelector<HTMLElement>(
			'[data-autofocus], input:not([type="hidden"]):not([type="checkbox"]):not([type="radio"]):not(:disabled), textarea:not(:disabled), select:not(:disabled)'
		);
		if (!field) return;
		event.preventDefault();
		field.focus();
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay
			class="fixed inset-0 z-50 bg-black/70 backdrop-blur-[2px] transition-opacity duration-150 data-[state=closed]:opacity-0"
		/>
		<Dialog.Content
			onOpenAutoFocus={landFocus}
			class="fixed inset-x-0 bottom-0 z-50 max-h-[88dvh] overflow-y-auto overscroll-contain rounded-t-2xl border-t border-line bg-surface px-5 pt-3 shadow-2xl pb-[max(1.25rem,env(safe-area-inset-bottom))] sm:inset-x-auto sm:bottom-auto sm:left-1/2 sm:top-1/2 sm:max-h-[90vh] sm:w-[min(92vw,34rem)] sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-card sm:border sm:p-5"
		>
			<!-- On a phone every dialog is a bottom sheet: within reach of a thumb. -->
			<div class="mx-auto mb-3 h-1 w-10 rounded-full bg-white/15 sm:hidden" aria-hidden="true"></div>
			<div class="flex items-start justify-between gap-4">
				<div>
					<Dialog.Title class="text-lg font-semibold text-foreground">{title}</Dialog.Title>
					{#if description}
						<Dialog.Description class="mt-1 text-sm text-muted">{description}</Dialog.Description>
					{/if}
				</div>
				<Dialog.Close
					class="-mr-1 -mt-1 inline-flex size-10 items-center sm:size-8 justify-center rounded-md text-muted hover:bg-surface-hover hover:text-foreground"
					aria-label="Close"
				>
					<X class="size-4" />
				</Dialog.Close>
			</div>
			<div class="mt-4" bind:this={body}>
				{@render children?.()}
			</div>
			{#if footer}
				<div class="mt-6 flex justify-end gap-2 max-sm:[&>*]:flex-1">
					{@render footer()}
				</div>
			{/if}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
