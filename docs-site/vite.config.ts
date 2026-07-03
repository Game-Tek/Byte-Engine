import react from '@vitejs/plugin-react';
import { tanstackStart } from '@tanstack/react-start/plugin/vite';
import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import mdx from 'fumadocs-mdx/vite';
import { cloudflare } from '@cloudflare/vite-plugin';

export default defineConfig({
	server: {
		port: 3000,
	},
	resolve: {
		tsconfigPaths: true,
	},
	plugins: [
		mdx(await import('./source.config')),
		tailwindcss(),
		// cloudflare({ viteEnvironment: { name: "ssr" } }),
		tanstackStart({
			prerender: {
				enabled: true,
			},
		}),
		react(),
	],
});
