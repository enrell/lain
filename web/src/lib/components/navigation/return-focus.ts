/*
 * Which link a page was left by, and finding it again (ReturnFocus). A
 * link is told by where it goes, as the router compares addresses (path
 * and query, no hash), and by its place among the links that go there:
 * Home's hero and a card in a rail can open the same title.
 */

export interface Opened {
	/** Path and query the link goes to. */
	href: string;
	/** Which of the page's links to that address it was. */
	index: number;
}

/** Path and query of a link's address, resolved against the page's. */
export function linkTarget(href: string, base: string): string {
	try {
		const url = new URL(href, base);
		return url.pathname + url.search;
	} catch {
		return '';
	}
}

function linksTo(root: HTMLElement, dest: string): HTMLAnchorElement[] {
	const base = document.baseURI;
	return [...root.querySelectorAll<HTMLAnchorElement>('a[href]')].filter((a) => linkTarget(a.getAttribute('href') ?? '', base) === dest);
}

/** The link the page is being left by: the first candidate that goes there, else the first such link. */
export function openedLink(root: HTMLElement, to: URL, candidates: (Element | null)[]): Opened | null {
	const dest = to.pathname + to.search;
	const links = linksTo(root, dest);
	if (links.length === 0) return null;
	const chosen = candidates.find((c): c is HTMLAnchorElement => c instanceof HTMLAnchorElement && links.includes(c)) ?? links[0];
	return { href: dest, index: links.indexOf(chosen) };
}

/** The same link once the page is drawn again; the first to that address if there are fewer now. */
export function matchOpened(root: HTMLElement, opened: Opened): HTMLAnchorElement | null {
	const links = linksTo(root, opened.href);
	return links[opened.index] ?? links[0] ?? null;
}
