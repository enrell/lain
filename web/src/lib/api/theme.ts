import { request } from './client';
import type { ThemePalette } from './types';

export const theme = {
	get(signal?: AbortSignal): Promise<ThemePalette> {
		return request<ThemePalette>('/api/theme', { signal, suppressAuthRedirect: true });
	}
};
