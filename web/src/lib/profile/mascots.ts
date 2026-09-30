/**
 * Pre-made avatars (D-086). The ids must match internal/auth.Mascots;
 * the artwork lives only here, in lib/components/profile/Mascot.svelte.
 */
export const MASCOTS = [
	{ id: 'wired', name: 'Wired', hue: 'oklch(0.72 0.14 190)' },
	{ id: 'moth', name: 'Moth', hue: 'oklch(0.78 0.12 85)' },
	{ id: 'static', name: 'Static', hue: 'oklch(0.7 0.02 260)' },
	{ id: 'orbit', name: 'Orbit', hue: 'oklch(0.7 0.16 280)' },
	{ id: 'glyph', name: 'Glyph', hue: 'oklch(0.74 0.15 330)' },
	{ id: 'shell', name: 'Shell', hue: 'oklch(0.72 0.15 40)' },
	{ id: 'neon', name: 'Neon', hue: 'oklch(0.8 0.17 145)' },
	{ id: 'void', name: 'Void', hue: 'oklch(0.62 0.18 300)' }
] as const;

export type MascotId = (typeof MASCOTS)[number]['id'];

export function mascotById(id: string | undefined) {
	return MASCOTS.find((m) => m.id === id) ?? null;
}

/** Name shown for an account: the display name when set, else the username. */
export function displayName(user: { username: string; profile?: { display_name?: string } } | null): string {
	return user?.profile?.display_name?.trim() || user?.username || '';
}
