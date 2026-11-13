// The demo build only: the client's links are absolute paths (`/library`),
// and the demo routes by hash under a sub-path. Every anchor is kept in its
// hash form as the page changes, so a click, a middle click, a hover preload
// and "copy link" all see a link that stays inside the demo.
function fix(a: HTMLAnchorElement): void {
	const href = a.getAttribute('href');
	if (href && href.startsWith('/') && !href.startsWith('//')) a.setAttribute('href', `#${href}`);
}

function sweep(root: ParentNode): void {
	if (root instanceof HTMLAnchorElement) fix(root);
	for (const a of root.querySelectorAll<HTMLAnchorElement>('a[href^="/"]')) fix(a);
}

export function keepLinksInside(): () => void {
	sweep(document);
	const watch = new MutationObserver((changes) => {
		for (const c of changes) {
			if (c.type === 'attributes') {
				if (c.target instanceof HTMLAnchorElement) fix(c.target);
			} else {
				for (const n of c.addedNodes) if (n instanceof Element) sweep(n);
			}
		}
	});
	watch.observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ['href'] });
	return () => watch.disconnect();
}
