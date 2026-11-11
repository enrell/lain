<script lang="ts" module>
	import type { Component } from 'svelte';

	export type MenuEntry = {
		label: string;
		onSelect: () => void;
		icon?: Component<{ class?: string }>;
		/** The key that does the same without the menu. */
		kbd?: string;
		danger?: boolean;
		disabled?: boolean;
		/** A hairline above this entry. */
		separated?: boolean;
	};
</script>

<script lang="ts">
	/*
	 * A button that opens a short list of actions: what a page can do
	 * beyond its main action (manage a title, open it elsewhere) without a
	 * row of equal-weight buttons beside Play. Arrow keys, Enter and
	 * Escape work as in any menu; each entry shows its own key.
	 */
	import { DropdownMenu } from 'bits-ui';
	import type { Snippet } from 'svelte';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import { buttonClasses, type ButtonSize, type ButtonVariant } from './button-styles';

	let {
		label,
		entries,
		icon,
		variant = 'secondary',
		size = 'sm',
		align = 'end',
		hint = undefined,
		class: className = ''
	}: {
		label: string;
		entries: MenuEntry[];
		icon?: Snippet;
		variant?: ButtonVariant;
		size?: ButtonSize;
		align?: 'start' | 'end';
		/** What the menu is for, as a tooltip. */
		hint?: string;
		class?: string;
	} = $props();
</script>

{#if entries.length > 0}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger class={buttonClasses(variant, size, className)} title={hint}>
			{@render icon?.()}
			{label}
			<ChevronDown class="size-3.5 opacity-70" aria-hidden="true" />
		</DropdownMenu.Trigger>
		<DropdownMenu.Portal>
			<DropdownMenu.Content {align} sideOffset={6} class="z-50 min-w-56 rounded-lg border border-line bg-surface p-1 shadow-2xl">
				{#each entries as entry (entry.label)}
					{#if entry.separated}<DropdownMenu.Separator class="my-1 h-px bg-hairline" />{/if}
					<DropdownMenu.Item
						disabled={entry.disabled}
						onSelect={entry.onSelect}
						class={[
							'flex min-h-10 cursor-default items-center gap-2.5 rounded-md px-2.5 text-sm outline-none data-[disabled]:opacity-40 data-[highlighted]:bg-surface-hover md:min-h-9',
							entry.danger ? 'text-danger' : 'text-foreground'
						].join(' ')}
					>
						{#if entry.icon}<entry.icon class="size-4 shrink-0 opacity-80" />{/if}
						<span class="flex-1">{entry.label}</span>
						{#if entry.kbd}<kbd class="key-hint">{entry.kbd}</kbd>{/if}
					</DropdownMenu.Item>
				{/each}
			</DropdownMenu.Content>
		</DropdownMenu.Portal>
	</DropdownMenu.Root>
{/if}
