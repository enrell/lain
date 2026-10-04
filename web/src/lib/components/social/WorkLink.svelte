<script lang="ts">
	import type { WorkRef } from '$lib/api';
	import { kindLabel, workHref } from '$lib/social/format';

	let { work, detail }: { work: WorkRef; detail?: string } = $props();
</script>

<a href={workHref(work)} class="inline-flex min-w-0 items-baseline gap-2 hover:text-accent">
	<span class="truncate font-medium text-foreground">{work.title}</span>
	{#if detail}<span class="shrink-0 font-mono text-[11px] text-muted">{detail}</span>{/if}
	<span class="shrink-0 font-mono text-[9px] uppercase tracking-[0.18em] text-muted">{kindLabel(work.kind)}</span>
</a>
