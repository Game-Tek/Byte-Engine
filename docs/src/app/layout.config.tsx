import { APP_NAME } from '@/lib/consts';
import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export const baseOptions: BaseLayoutProps = {
	nav: {
		title: (
		<>
			<svg
			width="24"
			height="24"
			xmlns="http://www.w3.org/2000/svg"
			aria-label="Logo"
			>
			<circle cx={12} cy={12} r={12} fill="currentColor" />
			</svg>
			{APP_NAME}
		</>
		),
		transparentMode: "top",
	},
	githubUrl: 'https://github.com/Game-Tek/Byte-Engine',
	links: [],
};
