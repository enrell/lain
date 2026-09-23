export type RendererBackend = 'webgpu' | 'webgl2';

export interface BrowserProfile {
	id: 'brave' | 'default';
	backends: readonly RendererBackend[];
}

interface BrowserSignals {
	brave?: { isBrave?: () => Promise<boolean> };
	userAgentData?: { brands?: readonly { brand: string }[] };
}

const BRAVE: BrowserProfile = { id: 'brave', backends: ['webgl2', 'webgpu'] };
const DEFAULT: BrowserProfile = { id: 'default', backends: ['webgpu', 'webgl2'] };

/** Browser identity sets preference; renderer and frame probes decide availability. */
export async function detectBrowserProfile(signals: BrowserSignals = navigator as BrowserSignals): Promise<BrowserProfile> {
	try {
		if (await signals.brave?.isBrave?.()) return BRAVE;
	} catch {
		// Brave may hide its detection API on some sites. Client hints are a fallback.
	}
	if (signals.userAgentData?.brands?.some(({ brand }) => brand === 'Brave')) return BRAVE;
	return DEFAULT;
}
