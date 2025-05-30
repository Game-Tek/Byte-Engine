import tsConfigPaths from 'vite-tsconfig-paths';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from '@tanstack/react-start/config';

export default defineConfig({
	server: {
		hooks: {
			'prerender:routes': async (routes) => {
				const { source } = await import('./lib/source');
				const pages = source.getPages();

				for (const page of pages) {
					routes.add(page.url);
				}
			},
		},
		prerender: {
			routes: ['/'],
			crawlLinks: true,
		},
	},
	vite: {
		build: {
			rollupOptions: {
				external: ['shiki'],
				onwarn(warning, warn) {
					if (warning.code === 'MODULE_LEVEL_DIRECTIVE') {
						return;
					}
					warn(warning);
				},
			},
		},
		plugins: [
			// cloudflare(),
			tsConfigPaths({
				projects: ['./tsconfig.json'],
			}),
			tailwindcss(),
		],
	},
});