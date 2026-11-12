<script lang="ts">
	import { Switch } from 'bits-ui';

	let {
		checked = $bindable(false),
		label,
		description,
		disabled = false,
		bare = false,
		onCheckedChange
	}: {
		checked?: boolean;
		label: string;
		description?: string;
		disabled?: boolean;
		/** Only the toggle: the surrounding row already shows the label. */
		bare?: boolean;
		onCheckedChange?: (checked: boolean) => void;
	} = $props();

	const handleChange = (next: boolean): void => {
		checked = next;
		onCheckedChange?.(next);
	};
</script>

<div class={bare ? 'contents' : 'flex items-start justify-between gap-4'}>
	{#if !bare}
		<div>
			<p class="text-sm font-medium text-foreground">{label}</p>
			{#if description}<p class="mt-0.5 text-xs text-muted">{description}</p>{/if}
		</div>
	{/if}
	<!-- Off is a visible track with a muted knob; on is the accent track with a knob in
	     the accent's ink (the app's toggle, and the player's). A field-coloured track
	     vanished on the page, and an on state of a dim track with a rose knob was
	     easy to read as off. -->
	<Switch.Root
		aria-label={bare ? label : undefined}
		checked={checked}
		onCheckedChange={handleChange}
		{disabled}
		class="relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full border border-hairline bg-surface-active transition-colors data-[state=checked]:border-accent data-[state=checked]:bg-accent disabled:opacity-50"
	>
		<Switch.Thumb
			class="size-4 translate-x-1 rounded-full bg-muted transition-transform data-[state=checked]:translate-x-5 data-[state=checked]:bg-accent-fg"
		/>
	</Switch.Root>
</div>
