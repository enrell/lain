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
		library = null,
		libraries = [],
		onunlocked
	}: {
		open?: boolean;
		/** One library, named in the title (the settings page). */
		library?: Library | null;
		/**
		 * Or every locked one at once, without naming any: the dialog can be
		 * opened with someone else looking at the screen.
		 */
		libraries?: Library[];
		onunlocked?: (lib: Library) => void;
	} = $props();

	const discreet = $derived(!library && libraries.length > 0);

	let usePassword = $state(false);
	let secret = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	const fieldId = $props.id();

	$effect(() => {
		if (open) {
			secret = '';
			error = null;
			// The modal moves focus to its first control once it has opened;
			// the field takes it back after that, so typing goes to the secret.
			const focus = (): void => document.getElementById(fieldId)?.focus();
			void tick().then(focus);
			const later = setTimeout(focus, 80);
			return () => clearTimeout(later);
		}
	});

	async function submit(): Promise<void> {
		const targets = library ? [library] : libraries;
		if (targets.length === 0 || !secret) return;
		busy = true;
		error = null;
		try {
			const credentials = usePassword ? { password: secret } : { pin: secret };
			// One secret opens them all; a wrong one stops at the first, so it
			// costs one failed attempt, not one per library.
			let last: Library | null = null;
			for (const target of targets) last = await api.libraries.unlock(target.id, credentials);
			if (!last) return;
			toasts.success(
				discreet ? t('privateLibrary.unlockedAllToast') : t('privateLibrary.unlockedToast', { name: last.name })
			);
			open = false;
			onunlocked?.(last);
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
	title={discreet ? t('privateLibrary.unlock') : t('privateLibrary.unlockTitle', { name: library?.name ?? '' })}
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
