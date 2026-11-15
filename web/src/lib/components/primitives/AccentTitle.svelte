<script lang="ts">
	/*
	 * A heading in the design system's voice (DESIGN.md §2.2): the bold sans,
	 * with its closing word in the serif italic accent ("Continue *watching*").
	 * A one-word heading stays plain: the accent closes a phrase, it is not a
	 * word set apart.
	 */
	let { text }: { text: string } = $props();
	const parts = $derived.by(() => {
		const at = text.trimEnd().lastIndexOf(' ');
		return at > 0 ? { lead: text.slice(0, at + 1), last: text.slice(at + 1).trim() } : null;
	});
</script>

{#if parts}{parts.lead}<em class="accent-word">{parts.last}</em>{:else}{text}{/if}
