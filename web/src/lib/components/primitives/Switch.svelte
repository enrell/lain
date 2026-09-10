<script lang="ts">
	import { Switch } from 'bits-ui';

	let {
		checked = $bindable(false),
		label,
		description,
		disabled = false,
		onCheckedChange
	}: {
		checked?: boolean;
		label: string;
		description?: string;
		disabled?: boolean;
		onCheckedChange?: (checked: boolean) => void;
	} = $props();

	const handleChange = (next: boolean): void => {
		checked = next;
		onCheckedChange?.(next);
	};
</script>

<div class="flex items-start justify-between gap-4">
	<div>
		<p class="text-sm font-medium text-foreground">{label}</p>
		{#if description}<p class="mt-0.5 text-xs text-muted">{description}</p>{/if}
	</div>
	<Switch.Root
		checked
		onCheckedChange={handleChange}
		{disabled}
		class="relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full border border-line bg-surface-active transition-colors data-[state=checked]:border-accent/40 data-[state=checked]:bg-accent/25 disabled:opacity-50"
	>
		<Switch.Thumb
			class="size-4 translate-x-1 rounded-full bg-muted transition-transform data-[state=checked]:translate-x-5 data-[state=checked]:bg-accent"
		/>
	</Switch.Root>
</div>
