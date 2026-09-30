/** Which row just saved: drives the transient "saved" mark of SettingRow. */
export class SavedFlash {
	key = $state('');
	#timer: ReturnType<typeof setTimeout> | undefined;

	mark(key: string): void {
		clearTimeout(this.#timer);
		// Re-trigger the animation when the same row saves twice in a row.
		this.key = '';
		queueMicrotask(() => {
			this.key = key;
			this.#timer = setTimeout(() => (this.key = ''), 1700);
		});
	}

	is(key: string): boolean {
		return this.key === key;
	}
}
