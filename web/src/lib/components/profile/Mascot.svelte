<script lang="ts">
	/*
	 * The pre-made avatars: flat, geometric, one hue each over a dark
	 * field, drawn to read at 24 px in the nav as well as at 128 px on the
	 * profile page.
	 */
	import { mascotById } from '$lib/profile/mascots';

	let { id, class: className = '' }: { id: string; class?: string } = $props();
	const m = $derived(mascotById(id));
	const hue = $derived(m?.hue ?? 'var(--color-accent)');
	const ink = '#07080b';
</script>

<svg viewBox="0 0 64 64" class={className} role="img" aria-label={m ? `${m.name} mascot` : 'Mascot'} style:--h={hue}>
	<rect width="64" height="64" fill="#0d0f15" />
	<rect width="64" height="64" fill="var(--h)" opacity="0.14" />
	{#if id === 'wired'}
		<path d="M20 14 C18 6 10 6 8 0 M44 14 C46 6 54 6 56 0" stroke="var(--h)" stroke-width="2" fill="none" stroke-linecap="round" />
		<circle cx="32" cy="34" r="20" fill="var(--h)" />
		<rect x="21" y="30" width="7" height="7" fill={ink} />
		<rect x="36" y="30" width="7" height="7" fill={ink} />
		<path d="M26 45 h12" stroke={ink} stroke-width="2.5" stroke-linecap="square" />
		<path d="M0 58 H20 L26 52 H64" stroke="var(--h)" stroke-width="1.5" fill="none" opacity="0.6" />
	{:else if id === 'moth'}
		<path d="M32 22 L6 10 L12 40 L32 34 Z" fill="var(--h)" />
		<path d="M32 22 L58 10 L52 40 L32 34 Z" fill="var(--h)" />
		<path d="M32 34 L16 54 L32 46 L48 54 Z" fill="var(--h)" opacity="0.65" />
		<rect x="29" y="18" width="6" height="30" rx="3" fill={ink} />
		<circle cx="20" cy="24" r="3.5" fill={ink} />
		<circle cx="44" cy="24" r="3.5" fill={ink} />
		<path d="M30 18 L24 8 M34 18 L40 8" stroke="var(--h)" stroke-width="1.8" stroke-linecap="round" />
	{:else if id === 'static'}
		<rect x="10" y="14" width="44" height="36" rx="6" fill="var(--h)" />
		{#each [20, 26, 32, 38, 44] as y (y)}
			<path d="M14 {y} H50" stroke={ink} stroke-width="1" opacity="0.25" />
		{/each}
		<rect x="20" y="26" width="8" height="4" fill={ink} />
		<rect x="36" y="26" width="8" height="4" fill={ink} />
		<path d="M24 40 h4 v-3 h8 v3 h4" stroke={ink} stroke-width="2" fill="none" />
		<path d="M24 50 L20 58 M40 50 L44 58" stroke="var(--h)" stroke-width="2.5" stroke-linecap="round" />
	{:else if id === 'orbit'}
		<ellipse cx="32" cy="34" rx="28" ry="8" fill="none" stroke="var(--h)" stroke-width="2" opacity="0.5" transform="rotate(-18 32 34)" />
		<circle cx="32" cy="32" r="16" fill="var(--h)" />
		<path d="M5 42 A28 8 -18 0 0 59 26" fill="none" stroke="var(--h)" stroke-width="2.5" transform="rotate(0)" />
		<circle cx="26" cy="30" r="2.6" fill={ink} />
		<circle cx="38" cy="30" r="2.6" fill={ink} />
		<path d="M28 37 Q32 40 36 37" stroke={ink} stroke-width="2" fill="none" stroke-linecap="round" />
		<circle cx="54" cy="12" r="2.5" fill="var(--h)" />
	{:else if id === 'glyph'}
		<path d="M14 56 V30 A18 18 0 0 1 50 30 V56 L44 50 L38 56 L32 50 L26 56 L20 50 Z" fill="var(--h)" />
		<ellipse cx="32" cy="31" rx="10" ry="6.5" fill={ink} />
		<circle cx="32" cy="31" r="3.2" fill="var(--h)" />
		<path d="M20 20 L24 23 M44 20 L40 23" stroke={ink} stroke-width="2" stroke-linecap="round" />
	{:else if id === 'shell'}
		<circle cx="36" cy="30" r="18" fill="var(--h)" />
		<path d="M36 30 m0 -4 a4 4 0 1 1 -4 4 a8 8 0 1 1 8 8 a12 12 0 1 1 -12 -12" stroke={ink} stroke-width="2.2" fill="none" opacity="0.7" />
		<path d="M6 52 H48 Q54 52 54 46" stroke="var(--h)" stroke-width="6" stroke-linecap="round" fill="none" />
		<path d="M12 52 V40 M18 52 V42" stroke="var(--h)" stroke-width="2" stroke-linecap="round" />
		<circle cx="12" cy="38" r="2.2" fill="var(--h)" />
		<circle cx="18" cy="40" r="2.2" fill="var(--h)" />
	{:else if id === 'neon'}
		<path d="M14 22 L16 8 L26 17 H38 L48 8 L50 22 V38 A18 18 0 0 1 14 38 Z" fill="none" stroke="var(--h)" stroke-width="3" stroke-linejoin="round" />
		<path d="M22 32 L28 29 M42 32 L36 29" stroke="var(--h)" stroke-width="3" stroke-linecap="round" />
		<path d="M29 41 L32 44 L35 41" stroke="var(--h)" stroke-width="2.5" fill="none" stroke-linecap="round" stroke-linejoin="round" />
		<path d="M4 40 H12 M52 40 H60 M4 46 H11 M53 46 H60" stroke="var(--h)" stroke-width="1.5" opacity="0.6" />
	{:else if id === 'void'}
		<circle cx="32" cy="32" r="24" fill="none" stroke="var(--h)" stroke-width="1.5" opacity="0.35" />
		<circle cx="32" cy="32" r="19" fill="none" stroke="var(--h)" stroke-width="3" stroke-dasharray="9 5" />
		<circle cx="32" cy="32" r="13" fill={ink} />
		<ellipse cx="32" cy="32" rx="7" ry="4.5" fill="var(--h)" />
		<circle cx="32" cy="32" r="2.2" fill={ink} />
	{/if}
</svg>
