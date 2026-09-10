// Pure SPA: every route is rendered in the browser and the Go server
// serves the same static shell for any client-side path.
export const ssr = false;
export const prerender = false;
export const trailingSlash = 'never';
