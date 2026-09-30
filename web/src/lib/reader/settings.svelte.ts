import { prefs } from '$lib/auth/storage';
import { resolveDirection, type Direction } from './layout';
import type { ReaderFit, ReaderMode, ReaderProfile } from './profile';

const ZOOM_MIN = 1;
const ZOOM_MAX = 3;

/**
 * Reader preferences. View options are remembered per kind (a manga
 * reader keeps its spread choice for every manga); the reading direction
 * is remembered per title, because a few manga print left to right.
 */
export class ReaderSettings {
	direction = $state<Direction>('ltr');
	mode = $state<ReaderMode>('paged');
	dual = $state(false);
	coverAlone = $state(true);
	fit = $state<ReaderFit>('contain');
	zoom = $state(1);

	#profile: ReaderProfile;
	#titleKey: string;

	constructor(profile: ReaderProfile, titleKey: string, serverDirection: Direction) {
		this.#profile = profile;
		this.#titleKey = titleKey;
		const ns = `reader.${profile.kind}`;
		this.direction = resolveDirection(prefs.get(`reader.dir.${titleKey}`), serverDirection);
		const mode = prefs.get(`${ns}.mode`);
		this.mode = mode === 'strip' || mode === 'paged' ? mode : profile.mode;
		this.dual = prefs.getBool(`${ns}.dual`, profile.dual);
		this.coverAlone = prefs.getBool(`${ns}.cover_alone`, profile.coverAlone);
		const fit = prefs.get(`${ns}.fit`);
		this.fit = fit === 'width' || fit === 'contain' ? fit : profile.fit;
	}

	get #ns(): string {
		return `reader.${this.#profile.kind}`;
	}

	setDirection(d: Direction): void {
		this.direction = d;
		prefs.set(`reader.dir.${this.#titleKey}`, d);
	}
	toggleDirection(): void {
		this.setDirection(this.direction === 'rtl' ? 'ltr' : 'rtl');
	}
	setMode(m: ReaderMode): void {
		this.mode = m;
		this.zoom = 1;
		prefs.set(`${this.#ns}.mode`, m);
	}
	toggleDual(): void {
		this.dual = !this.dual;
		prefs.set(`${this.#ns}.dual`, String(this.dual));
	}
	toggleCoverAlone(): void {
		this.coverAlone = !this.coverAlone;
		prefs.set(`${this.#ns}.cover_alone`, String(this.coverAlone));
	}
	setFit(f: ReaderFit): void {
		this.fit = f;
		this.zoom = 1;
		prefs.set(`${this.#ns}.fit`, f);
	}
	setZoom(z: number): void {
		this.zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 4) / 4));
	}
}
