import { describe, expect, it } from 'vitest';
import { detectBrowserProfile } from './browser-profile';

describe('browser renderer profile', () => {
	it('prefers WebGL2 in Brave even when WebGPU exists', async () => {
		const profile = await detectBrowserProfile({ brave: { isBrave: async () => true } });
		expect(profile).toEqual({ id: 'brave', backends: ['webgl2', 'webgpu'] });
	});

	it('prefers WebGPU in ordinary Chromium', async () => {
		const profile = await detectBrowserProfile({ userAgentData: { brands: [{ brand: 'Chromium' }] } });
		expect(profile).toEqual({ id: 'default', backends: ['webgpu', 'webgl2'] });
	});

	it('uses Brave client hints if its detection API is unavailable', async () => {
		const profile = await detectBrowserProfile({
			brave: { isBrave: async () => { throw new Error('hidden'); } },
			userAgentData: { brands: [{ brand: 'Brave' }] }
		});
		expect(profile.id).toBe('brave');
	});
});
