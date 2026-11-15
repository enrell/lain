import { afterNavigate } from '$app/navigation';

/*
 * Moving from one page to another eases the new page in (DESIGN.md §2.6): it
 * fades in while its title rises. A plain animation on the page
 * content, never a View Transition: during one the browser takes no clicks,
 * and a click a third of a second after a navigation was lost. It never runs
 * for a change inside the same page (filters, a search query, a tab), for the
 * player or the reader (they open full screen and must start at once), or
 * under prefers-reduced-motion. The header and the bottom bar do not move.
 */
const IMMERSIVE = /^\/(player|read)\//;

export function pageTransitions() {
	afterNavigate((navigation) => {
		// A navigation may come without one of its ends (the first load, a hash
		// typed in the address bar): no motion then, and never an error.
		const from = navigation.from?.url?.pathname;
		const to = navigation.to?.url?.pathname;
		if (!from || !to || from === to) return;
		if (IMMERSIVE.test(from) || IMMERSIVE.test(to)) return;
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
		const page = document.querySelector<HTMLElement>('#main > div');
		// Opacity only: a transform on the page would make it the box of every fixed
		// bar inside it (a staged-changes bar, a sheet) for as long as it ran.
		// The rise is the page title's own (app.css, .page-title).
		page?.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 240, easing: 'cubic-bezier(0.16, 1, 0.3, 1)' });
	});
}
