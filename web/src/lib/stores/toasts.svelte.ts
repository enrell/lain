export type ToastKind = 'error' | 'success' | 'info';

export interface Toast {
	id: number;
	kind: ToastKind;
	message: string;
	/** One action beside the message (an Undo), which also dismisses it. */
	action?: { label: string; run: () => void };
}

/** Small global surface for transient feedback (errors from actions). */
class Toasts {
	items = $state<Toast[]>([]);
	#next = 1;

	push(kind: ToastKind, message: string, ttlMs = 5000): number {
		const id = this.#next++;
		this.items = [...this.items, { id, kind, message }];
		setTimeout(() => this.dismiss(id), ttlMs);
		return id;
	}

	error(message: string): number {
		return this.push('error', message, 7000);
	}

	success(message: string): number {
		return this.push('success', message, 4000);
	}

	info(message: string): number {
		return this.push('info', message, 4000);
	}

	/** A done thing that can be taken back: the message with an Undo, for a little longer. */
	undoable(message: string, undo: () => void, label = 'Undo'): number {
		const id = this.push('success', message, 8000);
		this.items = this.items.map((t) => (t.id === id ? { ...t, action: { label, run: undo } } : t));
		return id;
	}

	dismiss(id: number): void {
		this.items = this.items.filter((t) => t.id !== id);
	}
}

export const toasts = new Toasts();
