import react from '@vitejs/plugin-react';
import { tanstackStart } from '@tanstack/react-start/plugin/vite';
import { defineConfig } from 'vite';
import tsConfigPaths from 'vite-tsconfig-paths';
import tailwindcss from '@tailwindcss/vite';
import { cloudflare } from '@cloudflare/vite-plugin';
import mdx from 'fumadocs-mdx/vite';

export default defineConfig({
	server: {
		port: 3000,
	},
	// resolve: {
	// 	alias: {
	// 		'react-dom/server': 'react-dom/server.node'
	// 	}
	// },
	// optimizeDeps: {
	// 	include: ['react-dom/server']
	// },
	plugins: [
		mdx(await import('./source.config')),
		tailwindcss(),
		cloudflare(process.env.ENV === 'production' ? { viteEnvironment: { name: "ssr" } } : undefined),
		tsConfigPaths({
			projects: ['./tsconfig.json'],
		}),
		tanstackStart({
			prerender: {
				enabled: true,
			},
		}),
		react(),
	],
});
