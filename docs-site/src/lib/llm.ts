import { rustApiModel } from '@/lib/rust-api/load';
import { renderRustPageMarkdown } from '@/lib/rust-api/markdown';
import { source } from '@/lib/source';

/**
 * Return a page in the portable Markdown form exposed to agents.
 */
export async function getLLMText(page: (typeof source)['$inferPage']) {
	// Rust API pages render from the API model, with `.md` links between them.
	if (page.type === 'rust' && rustApiModel) {
		return `# ${page.data.title} (${page.url})\n\n${renderRustPageMarkdown(rustApiModel, page.data.rustKey)}`;
	}

	// Virtual OpenAPI pages expose searchable structured text instead of
	// compiled MDX text.
	const processed = 'getText' in page.data
		? await page.data.getText('processed')
		: page.data.structuredData.contents
			.map(({ content }) => content)
			.join('\n\n');

	return `# ${page.data.title} (${page.url})\n\n${processed}`;
}

/**
 * Translate a Markdown endpoint path back to the slugs used by the docs source.
 */
export function decodeMarkdownUrl(segments: string[]) {
	if (segments.length === 0) return [];

	const slugs = [...segments];
	slugs[slugs.length - 1] = slugs.at(-1)?.replace(/\.md$/, '') ?? '';
	if (slugs.length === 1 && slugs[0] === 'index') slugs.pop();

	return slugs;
}

/**
 * Return the stable Markdown endpoint for a rendered documentation page.
 */
export function getMarkdownUrl(pageUrl: string) {
	return pageUrl === '/docs' ? '/docs/index.md' : `${pageUrl}.md`;
}
