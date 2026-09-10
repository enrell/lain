import { request } from './client';
import type { BindingView, PluginsInfo, SwapResult } from './types';

export interface SwapInput {
	capability: string;
	providers: string[];
	/** Generation the operator saw; the server fences stale writers. */
	generation: number;
}

/** Gateway routes: GET /api/plugins, POST /api/plugins/swap (admin). */
export const plugins = {
	info: () => request<PluginsInfo>('/api/plugins'),

	swap: (input: SwapInput) =>
		request<SwapResult>('/api/plugins/swap', { method: 'POST', body: input })
};

export function bindingFor(info: PluginsInfo, capability: string): BindingView | undefined {
	return info.composition.find((b) => b.capability === capability);
}
