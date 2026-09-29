<script lang="ts">
	/*
	 * DOM cue overlay (D-082): while the GPU renderer presents frames the
	 * subtitle <track> runs in `hidden` mode — the browser still parses the
	 * WebVTT sidecar and fires cuechange — and this paints the active cues
	 * over the shader canvas. When the renderer is off the parent switches
	 * the track back to `showing` and the browser renders cues natively.
	 */
	import { cuePlacement, pictureRect, type Rect } from '$lib/player/cue-overlay';

	let {
		track,
		video,
		size
	}: {
		track: TextTrack;
		video: HTMLVideoElement | null;
		size: 'small' | 'medium' | 'large';
	} = $props();

	let cues = $state<VTTCue[]>([]);
	let rect = $state<Rect | null>(null);
	let boxHeight = $state(0);

	function measure(): void {
		const el = video;
		if (!el) {
			rect = null;
			return;
		}
		boxHeight = el.clientHeight;
		rect = pictureRect(el.videoWidth, el.videoHeight, el.clientWidth, el.clientHeight);
	}

	// cuechange only fires on transitions, so entering must read the cues
	// that are already active rather than waiting for the next one.
	$effect(() => {
		const update = () => {
			cues = track.activeCues ? (Array.from(track.activeCues) as VTTCue[]) : [];
		};
		update();
		track.addEventListener('cuechange', update);
		return () => track.removeEventListener('cuechange', update);
	});

	$effect(() => {
		const el = video;
		// Recalculate for cue changes too: the first cue often lands before
		// the metadata that videoWidth/videoHeight need.
		void cues.length;
		measure();
		if (!el) return;
		const observer = new ResizeObserver(measure);
		observer.observe(el);
		return () => observer.disconnect();
	});

	const fontPx = $derived(
		rect ? rect.height * 0.05 * (size === 'small' ? 0.8 : size === 'large' ? 1.25 : 1) : 0
	);
	const linePx = $derived(fontPx * 1.35);
	const placed = $derived(
		rect
			? cues
					.map((cue) => ({ cue, at: cuePlacement(cue, rect!, linePx) }))
					.filter((entry): entry is { cue: VTTCue; at: NonNullable<typeof entry.at> } => entry.at !== null)
			: []
	);
	const stacked = $derived(cues.filter((cue) => cuePlacement(cue, rect ?? { left: 0, top: 0, width: 0, height: 0 }, linePx) === null));

	function cueHTML(cue: VTTCue): string {
		const holder = document.createElement('div');
		holder.appendChild(cue.getCueAsHTML());
		return holder.innerHTML;
	}
</script>

{#if rect && (placed.length > 0 || stacked.length > 0)}
	<div class="cue-overlay" aria-hidden="true">
		{#each placed as { cue, at } (cue)}
			<div
				class="cue-line"
				style={`left:${at.left}px;top:${at.top}px;width:${at.width}px;font-size:${fontPx}px;text-align:${at.textAlign}`}
			>
				{@html cueHTML(cue)}
			</div>
		{/each}
		{#if stacked.length > 0}
			<div
				class="cue-stack"
				style={`left:${rect.left}px;width:${rect.width}px;bottom:${boxHeight - (rect.top + rect.height) + rect.height * 0.03}px;font-size:${fontPx}px`}
			>
				{#each stacked as cue (cue)}
					<div class="cue-line">{@html cueHTML(cue)}</div>
				{/each}
			</div>
		{/if}
	</div>
{/if}

<style>
	.cue-overlay {
		position: absolute;
		inset: 0;
		pointer-events: none;
		overflow: hidden;
	}

	.cue-line {
		position: absolute;
		color: #ffffff;
		text-shadow:
			0 0 2px #000000,
			0 0 4px #000000,
			1px 1px 2px #000000,
			-1px -1px 2px #000000,
			1px -1px 2px #000000,
			-1px 1px 2px #000000,
			0 2px 4px #000000;
		font-weight: 600;
		line-height: 1.35;
		white-space: pre-line;
	}

	:global(.subtitles-bg-box) .cue-line {
		background-color: rgba(0, 0, 0, 0.8);
		text-shadow: none;
	}

	.cue-stack {
		position: absolute;
		display: flex;
		flex-direction: column;
		align-items: center;
	}

	.cue-stack .cue-line {
		position: static;
		width: 90%;
		text-align: center;
	}
</style>
