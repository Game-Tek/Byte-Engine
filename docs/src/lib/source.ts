import { docs } from '@/.source';
import { icons } from 'lucide-react';
import { loader } from 'fumadocs-core/source';
import { createElement } from 'react';

export const source = loader({
	baseUrl: '/docs',
	source: docs.toFumadocsSource(),
	icon(icon) {
		if (!icon) {
			return;
		}

		if (icon in icons) return createElement(icons[icon as keyof typeof icons]);
	},
});
