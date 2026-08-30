'use client';

import { IconBrightnessIncrease, IconCloudMoon } from 'nucleo-glass';
import { useId, type ComponentProps } from 'react';
import { useTheme } from 'next-themes';
import { twMerge } from 'tailwind-merge';

type GlassThemeSwitchProps = ComponentProps<'div'> & {
	mode?: 'light-dark' | 'light-dark-system';
};

export function GlassThemeSwitch({
	className,
	mode: _mode,
}: GlassThemeSwitchProps) {
	const { resolvedTheme, setTheme } = useTheme();
	const uniqueId = useId().replaceAll(':', '');

	return (
		<button
			aria-label="Toggle Theme"
			className={twMerge(
				'inline-flex items-center overflow-hidden rounded-full border p-1 *:rounded-full',
				className,
			)}
			data-theme-toggle=""
			onClick={() => setTheme(resolvedTheme === 'light' ? 'dark' : 'light')}
			type="button"
		>
			<IconBrightnessIncrease
				aria-hidden="true"
				className="size-6.5 p-0.5"
				uniqueId={`${uniqueId}-light-`}
			/>
			<IconCloudMoon
				aria-hidden="true"
				className="size-6.5 p-0.5"
				uniqueId={`${uniqueId}-dark-`}
			/>
		</button>
	);
}
