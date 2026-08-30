import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { GlassThemeSwitch } from '@/components/glass-theme-switch';

export function baseOptions(): BaseLayoutProps {
	return {
		nav: {
			title: 'Byte Engine Docs',
		},
		slots: {
			themeSwitch: GlassThemeSwitch,
		},
	};
}
