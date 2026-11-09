<script lang="ts">
	// Unlocks a private library for this session (D-156). Keyboard-first:
	// the dialog traps focus and lands on the secret field, Enter unlocks,
	// Esc closes, Alt+P switches between PIN and password.
	import { tick } from 'svelte';
	import type { Library } from '$lib/api/types';
	import { api } from '$lib/api';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let {
		open = $bindable(false),
		library,
		onunlocked
	}: {
		open?: boolean;
		library: Library | null;
		onunlocked?: (lib: Library) => void;
	} = $props();

	let usePassword = $state(false);
	let secret = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	const fieldId = $props.id();

	$effect(() => {
		if (open) {
			secret = '';
			error = null;
			void tick().then(() => document.getElementById(fieldId)?.focus());
		}
	});

	async function submit(): Promise<void> {
		if (!library || !secret) return;
		busy = true;
		error = null;
		try {
			const lib = await api.libraries.unlock(library.id, usePassword ? { password: secret } : { pin: secret });
			toasts.success(t('privateLibrary.unlockedToast', { name: lib.name }));
			open = false;
			onunlocked?.(lib);
		} catch (err) {
			error = errorMessage(err, t('privateLibrary.unlockFailed'));
		} finally {
			busy = false;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.altKey && e.key.toLowerCase() === 'p') {
			e.preventDefault();
			usePassword = !usePassword;
			secret = '';
			void tick().then(() => document.getElementById(fieldId)?.focus());
		}
	}
</script>

<Modal
	bind:open
	title={t('privateLibrary.unlockTitle', { name: library?.name ?? '' })}
	description={t('privateLibrary.unlockDescription')}
>
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<form
		class="space-y-4"
		onkeydown={onKey}
		onsubmit={(e) => {
			e.preventDefault();
			void submit();
		}}
	>
		<Input
			id={fieldId}
			label={usePassword ? t('privateLibrary.password') : t('privateLibrary.pin')}
			type="password"
			inputmode={usePassword ? undefined : 'numeric'}
			autocomplete={usePassword ? 'current-password' : 'off'}
			bind:value={secret}
			{error}
			required
		/>
		<div class="flex items-center justify-between gap-2">
			<button
				type="button"
				class="text-sm text-muted underline-offset-2 hover:text-foreground hover:underline"
				onclick={() => {
					usePassword = !usePassword;
					secret = '';
				}}
			>
				{usePassword ? t('privateLibrary.usePin') : t('privateLibrary.usePassword')}
				<kbd class="ms-1 font-mono text-[10px] text-muted">Alt+P</kbd>
			</button>
			<div class="flex gap-2">
				<Button type="button" variant="ghost" onclick={() => (open = false)}>
					{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd>
				</Button>
				<Button type="submit" loading={busy} disabled={!secret}>
					{t('privateLibrary.unlock')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd>
				</Button>
			</div>
		</div>
	</form>
</Modal>
