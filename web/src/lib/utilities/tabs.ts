/*
 * A row of tabs from the keyboard, as WAI-ARIA's tabs pattern has it: the
 * row is one Tab stop (the chosen tab), Left and Right step along it and
 * wrap, Home and End jump to its ends, and the tab reached is chosen at
 * once. The number keys the pages also offer stay as they are.
 */

/** The tab a key moves to from `current`, or null when the key is not one of the row's. */
export function tabStep<T>(key: string, tabs: readonly T[], current: T): T | null {
	const at = tabs.indexOf(current);
	if (at < 0 || tabs.length === 0) return null;
	switch (key) {
		case 'ArrowRight':
			return tabs[(at + 1) % tabs.length];
		case 'ArrowLeft':
			return tabs[(at - 1 + tabs.length) % tabs.length];
		case 'Home':
			return tabs[0];
		case 'End':
			return tabs[tabs.length - 1];
		default:
			return null;
	}
}
