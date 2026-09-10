<script lang="ts">
	/*
	 * Deterministic artwork placeholder: same item always gets the same
	 * quiet gradient + monogram, so a partially enriched library still
	 * reads as a designed grid instead of broken images.
	 */
	let {
		title,
		seed,
		class: className = ''
	}: {
		title: string;
		seed: string;
		class?: string;
	} = $props();

	const palettes: [string, string][] = [
		['#12323b', '#0a161c'],
		['#1b2a44', '#0c1220'],
		['#2c2438', '#140f1d'],
		['#20302a', '#0d1512'],
		['#332a22', '#191410'],
		['#252b3a', '#10131b']
	];

	function hash(input: string): number {
		let h = 2166136261;
		for (let i = 0; i < input.length; i++) {
			h ^= input.charCodeAt(i);
			h = Math.imul(h, 16777619);
		}
		return h >>> 0;
	}

	const h = $derived(hash(seed || title));
	const palette = $derived(palettes[h % palettes.length]);
	const monogram = $derived(
		title
			.split(/\s+/)
			.filter((w) => /[\p{L}\p{N}]/u.test(w))
			.slice(0, 2)
			.map((w) => [...w][0]?.toUpperCase() ?? '')
			.join('') || '?'
	);
</script>

<div
	class={['relative flex items-center justify-center', className].join(' ')}
	style={`background: linear-gradient(155deg, ${palette[0]}, ${palette[1]});`}
	aria-hidden="true"
>
	<div
		class="absolute inset-0 opacity-40"
		style="background-image: radial-gradient(circle, rgba(255,255,255,0.10) 1px, transparent 1px); background-size: 18px 18px;"
	></div>
	<span class="relative font-mono text-3xl font-semibold tracking-widest text-white/25">
		{monogram}
	</span>
</div>
