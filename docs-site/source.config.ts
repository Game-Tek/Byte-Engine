import { remarkMdxMermaid } from 'fumadocs-core/mdx-plugins';
import { defineConfig, defineDocs } from 'fumadocs-mdx/config';
import { beslGrammar } from './src/lib/besl-grammar';

export const docs = defineDocs({
	dir: '../docs',
	docs: {
		postprocess: {
			// Preserve resolved Markdown so agents receive the same content readers see.
			includeProcessedMarkdown: true,
		},
	},
});

export default defineConfig({
	mdxOptions: {
		// Turn ```mermaid blocks into the client-rendered `Mermaid` component.
		remarkPlugins: [remarkMdxMermaid],
		rehypeCodeOptions: {
			// Shiki doesn't bundle BESL; preload its grammar so ```besl blocks highlight instead of failing the page.
			langs: [beslGrammar],
		},
	},
});
