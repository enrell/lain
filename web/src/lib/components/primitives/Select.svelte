<script lang="ts">
	import { Select } from 'bits-ui';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';

	export interface Option {
		value: string;
		label: string;
		disabled?: boolean;
	}

	let {
		value = $bindable(''),
		options,
		label,
		placeholder = 'Select…',
		disabled = false,
		class: className = '',
		onValueChange,
		'aria-label': ariaLabel
	}: {
		value?: string;
		options: Option[];
		label?: string;
		placeholder?: string;
		disabled?: boolean;
		class?: string;
		onValueChange?: (value: string) => void;
		'aria-label'?: string;
	} = $props();

	const uid = $props.id();

	function handleChange(v: string): void {
		value = v;
		onValueChange?.(v);
	}
</script>

<div class="space-y-1.5">
	{#if label}
		<span class="block text-sm font-medium text-muted" id={`${uid}-label`}>{label}</span>
	{/if}
	<Select.Root type="single" {value} onValueChange={handleChange} {disabled}>
		<Select.Trigger
			aria-label={ariaLabel}
			aria-labelledby={label ? `${uid}-label` : undefined}
			class={[
				'flex h-10 w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 text-sm text-foreground',
				'hover:border-muted/40 focus:border-accent/60 focus:outline-none disabled:opacity-50',
				className
			].join(' ')}
		>
			<Select.Value {placeholder} />
			<ChevronDown class="size-4 shrink-0 text-muted" aria-hidden="true" />
		</Select.Trigger>
		<Select.Portal>
			<Select.Content
				class="z-50 max-h-72 min-w-[var(--bits-select-anchor-width)] overflow-hidden rounded-md border border-line bg-surface py-1 shadow-xl"
				sideOffset={4}
			>
				<Select.Viewport>
					{#each options as opt (opt.value)}
						<Select.Item
							value={opt.value}
							label={opt.label}
							disabled={opt.disabled}
							class="flex cursor-default items-center justify-between px-3 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover data-[disabled]:opacity-40"
						>
							{opt.label}
						</Select.Item>
					{/each}
				</Select.Viewport>
			</Select.Content>
		</Select.Portal>
	</Select.Root>
</div>
