/**
 * Lands focus on an element once a dialog has finished its own
 * open-autofocus (which picks the first tabbable, the close button).
 * Keyboard-first rule: a modal opens on its primary action.
 */
export function focusSoon(get: () => HTMLElement | null | undefined): void {
	requestAnimationFrame(() => requestAnimationFrame(() => get()?.focus()));
}
