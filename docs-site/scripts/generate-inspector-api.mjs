import { rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { generateFiles } from 'fumadocs-openapi';
import { createOpenAPI } from 'fumadocs-openapi/server';

import schema from '../openapi/inspector.json' with { type: 'json' };

const output = fileURLToPath(new URL('../../docs/api/inspector', import.meta.url));
const openapi = createOpenAPI({
	input: { inspector: schema },
});

// Remove stale operation pages when the schema renames or removes a tag.
await rm(output, { recursive: true, force: true });

await generateFiles({
	input: openapi,
	output,
	per: 'tag',
	includeDescription: true,
	addGeneratedComment: 'Generated from docs-site/openapi/inspector.json. Do not edit directly.',
	index: {
		url(file) {
			return `/docs/api/inspector/${file.replace(/\.mdx$/, '')}`;
		},
		items: [
			{
				path: 'index.mdx',
				title: 'HTTP Inspector API',
				description: 'Inspect and control a running Byte Engine application over loopback HTTP.',
				only: ['inspector'],
			},
		],
	},
	meta: true,
	beforeWrite(files) {
		const metadata = files.find((file) => file.path === 'meta.json');
		if (!metadata) throw new Error('Fumadocs OpenAPI did not generate inspector metadata.');

		const value = JSON.parse(metadata.content);
		metadata.content = JSON.stringify(
			{
				...value,
				title: 'HTTP Inspector API',
				description: 'Loopback HTTP interface for runtime inspection and control.',
				icon: 'Bug',
				pagesIndex: 'index',
				pages: ['!index', ...value.pages],
			},
			null,
			2,
		);
	},
});
