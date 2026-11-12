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
	 * Where the focus lands when the dialog opens (keyboard first: on what
	 * the dialog is for, not its close button): an element marked
	 * data-autofocus; else the body's first control when it is a field; else,
	 * when the body has no control at all (a question), the footer's primary
	 * action (its last button) unless that destroys something, then the
	 * button beside it. A dialog that opens on choices keeps the default and
	 * its own focus code: the filters sheet's first field is a genre search
	 * far down and the rating dialog's a review under the scores, and
	 * focusing those scrolled the sheet and opened a phone's keyboard over
	 * it. A dialog that focuses something itself a moment later (focusSoon)
	 * still wins.
	 */
	const FIELD =
		'input:not([type="hidden"]):not([type="checkbox"]):not([type="radio"]):not(:disabled):not([readonly]), textarea:not(:disabled):not([readonly]), select:not(:disabled)';
	let content = $state<HTMLElement | null>(null);
	let body = $state<HTMLElement | null>(null);
	let footerEl = $state<HTMLElement | null>(null);
	function landing(): HTMLElement | null {
		const marked = content?.querySelector<HTMLElement>('[data-autofocus]');
		if (marked) return marked;
		const first = body?.querySelector<HTMLElement>(`${FIELD}, button:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])`);
		if (first) return first.matches(FIELD) ? first : null;
		const buttons = [...(footerEl?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? [])];
		const last = buttons.at(-1);
		if (last?.dataset.variant === 'danger') return buttons.at(-2) ?? null;
		return last ?? null;
	}
	function onOpenAutoFocus(e: Event): void {
		const to = landing();
		if (!to) return;
		e.preventDefault();
		to.focus();
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay
			class="fixed inset-0 z-50 bg-black/70 backdrop-blur-[2px] transition-opacity duration-150 data-[state=closed]:opacity-0"
		/>
		<Dialog.Content
			bind:ref={content}
			{onOpenAutoFocus}
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
				<div bind:this={footerEl} class="mt-6 flex justify-end gap-2 max-sm:[&>*]:flex-1">
					{@render footer()}
				</div>
			{/if}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
