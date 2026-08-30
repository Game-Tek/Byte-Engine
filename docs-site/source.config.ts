import { defineConfig, defineDocs } from 'fumadocs-mdx/config';

export const docs = defineDocs({
	dir: '../docs',
	docs: {
		postprocess: {
			// Preserve resolved Markdown so agents receive the same content readers see.
			includeProcessedMarkdown: true,
		},
	},
});

export default defineConfig();
