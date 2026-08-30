import { source } from '@/lib/source';

/**
 * Return a page in the portable Markdown form exposed to agents.
 */
export async function getLLMText(page: (typeof source)['$inferPage']) {
	const processed = await page.data.getText('processed');

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
