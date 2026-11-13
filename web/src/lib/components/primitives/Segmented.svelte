<script lang="ts" generics="V extends string">
	/*
	 * One segmented control for the whole app (round 20): Settings' policy
	 * and engine choices and the library's By library / All titles switch
	 * were three looks (mono caps on a bare field, sans on a ringed track,
	 * a lone word beside two links). One look now: a field-coloured track
	 * with a hairline, the picked segment raised on it in the active
	 * surface with an accent ring and heavier type.
	 *
	 * A radio group: Tab reaches the picked segment, the arrow keys move
	 * and pick (as native radios do), Home and End go to the ends. A
	 * disabled option stays visible and says why through its title.
	 */
	let {
		options,
		value,
		label,
		id,
		size = 'md',
		onchange
	}: {
		options: readonly { value: V; label: string; disabled?: boolean; title?: string }[];
		value: V;
		/** The group's accessible name. */
		label: string;
		id?: string;
		/** `sm` sits in a dense toolbar row, `md` beside a settings row. */
		size?: 'sm' | 'md';
		onchange: (value: V) => void;
	} = $props();

	let group = $state<HTMLDivElement>();
	// The one segment Tab reaches: the picked one, or the first that can be
	// picked when the value in force is disabled (Transcoding's engine keeps
	// a backend the probe stopped passing), so the group never drops out of
	// the tab order.
	const tabStop = $derived(
		options.find((o) => o.value === value && !o.disabled)?.value ?? options.find((o) => !o.disabled)?.value
	);

	function pick(next: V): void {
		if (next !== value) onchange(next);
	}

	function onKey(e: KeyboardEvent): void {
		const enabled = options.filter((o) => !o.disabled);
		if (enabled.length === 0) return;
		// From the focused segment: with a disabled value in force, focus sits
		// on the first enabled one, and the next is the one after it.
		const from = (e.target as HTMLElement).closest('[data-value]')?.getAttribute('data-value') ?? value;
		const at = enabled.findIndex((o) => o.value === from);
		let to: number;
		switch (e.key) {
			case 'ArrowRight':
			case 'ArrowDown':
				to = at < 0 ? 0 : (at + 1) % enabled.length;
				break;
			case 'ArrowLeft':
			case 'ArrowUp':
				to = at < 0 ? enabled.length - 1 : (at - 1 + enabled.length) % enabled.length;
				break;
			case 'Home':
				to = 0;
				break;
			case 'End':
				to = enabled.length - 1;
				break;
			default:
				return;
		}
		e.preventDefault();
		pick(enabled[to].value);
		const next = enabled[to].value;
		requestAnimationFrame(() =>
			group?.querySelector<HTMLElement>(`[data-value="${CSS.escape(next)}"]`)?.focus()
		);
	}
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
	bind:this={group}
	{id}
	role="radiogroup"
	aria-label={label}
	tabindex="-1"
	class="segmented inline-flex max-w-full shrink-0 gap-0.5 rounded-lg bg-field p-0.5 ring-1 ring-hairline ring-inset"
	onkeydown={onKey}
>
	{#each options as o (o.value)}
		{@const on = o.value === value}
		<button
			type="button"
			role="radio"
			aria-checked={on}
			data-value={o.value}
			disabled={o.disabled}
			title={o.title}
			tabindex={o.value === tabStop ? 0 : -1}
			class={[
				'min-w-0 truncate rounded-md px-3 text-[13px] tracking-[0.01em] transition-colors disabled:cursor-not-allowed disabled:opacity-40',
				size === 'sm' ? 'h-9 md:h-7' : 'h-8',
				on
					? 'bg-surface-active font-semibold text-foreground ring-1 ring-accent/50'
					: 'font-medium text-muted hover:bg-surface-hover hover:text-foreground'
			].join(' ')}
			onclick={() => pick(o.value)}
		>
			{o.label}
		</button>
	{/each}
</div>
