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
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay
			class="fixed inset-0 z-50 bg-black/70 backdrop-blur-[2px] transition-opacity duration-150 data-[state=closed]:opacity-0"
		/>
		<Dialog.Content
			class="fixed left-1/2 top-1/2 z-50 max-h-[90vh] w-[min(92vw,34rem)] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-card border border-line bg-surface p-5 shadow-2xl"
		>
			<div class="flex items-start justify-between gap-4">
				<div>
					<Dialog.Title class="text-lg font-semibold text-foreground">{title}</Dialog.Title>
					{#if description}
						<Dialog.Description class="mt-1 text-sm text-muted">{description}</Dialog.Description>
					{/if}
				</div>
				<Dialog.Close
					class="-mr-1 -mt-1 inline-flex size-8 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-foreground"
					aria-label="Close"
				>
					<X class="size-4" />
				</Dialog.Close>
			</div>
			<div class="mt-4">
				{@render children?.()}
			</div>
			{#if footer}
				<div class="mt-6 flex justify-end gap-2">
					{@render footer()}
				</div>
			{/if}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
