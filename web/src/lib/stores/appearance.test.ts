import { describe, expect, it } from 'vitest';
import type { ThemePalette } from '$lib/api/types';
import { themeVariables } from './appearance';

describe('themeVariables', () => {
	it('maps every semantic API color to its CSS token', () => {
		const palette: ThemePalette = {
			source: 'omarchy',
			mode: 'light',
			background: '#fffcf0',
			surface: '#f2efe4',
			surface_hover: '#e6e4d9',
			surface_active: '#cecdc3',
			foreground: '#100f0f',
			muted: '#878580',
			line: '#cecdc3',
			accent: '#205ea6',
			accent_hover: '#4385be',
			accent_foreground: '#ffffff',
			danger: '#d14d41',
			danger_foreground: '#ffffff',
			success: '#879a39',
			warning: '#d0a215'
		};

		expect(themeVariables(palette)).toEqual({
			'--color-background': '#fffcf0',
			'--color-surface': '#f2efe4',
			'--color-surface-hover': '#e6e4d9',
			'--color-surface-active': '#cecdc3',
			'--color-foreground': '#100f0f',
			'--color-muted': '#878580',
			'--color-line': '#cecdc3',
			'--color-accent': '#205ea6',
			'--color-accent-hover': '#4385be',
			'--color-accent-fg': '#ffffff',
			'--color-danger': '#d14d41',
			'--color-danger-fg': '#ffffff',
			'--color-success': '#879a39',
			'--color-warning': '#d0a215'
		});
	});
});
