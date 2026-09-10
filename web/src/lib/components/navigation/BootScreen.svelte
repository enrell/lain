<script lang="ts">
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import SignalMark from '../primitives/SignalMark.svelte';
	import Button from '../primitives/Button.svelte';
	import Spinner from '../primitives/Spinner.svelte';
	import { session } from '$lib/auth/session.svelte';

	function retry(): void {
		void session.retry();
	}
</script>

<div class="flex min-h-dvh flex-col items-center justify-center gap-5 px-6 text-center">
	<div class="flex items-center gap-3">
		<SignalMark class="size-8 text-accent" />
		<span class="text-2xl font-semibold tracking-[0.24em] text-foreground">lain</span>
	</div>
	{#if session.bootError}
		<p class="max-w-sm text-sm text-muted" role="alert">{session.bootError}</p>
		<Button variant="secondary" onclick={retry}>
			<RefreshCw class="size-4" /> Try again
		</Button>
	{:else}
		<div class="flex items-center gap-2 text-sm text-muted">
			<Spinner class="size-4" />
			Connecting to your server…
		</div>
	{/if}
</div>
