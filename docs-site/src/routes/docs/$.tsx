import { createFileRoute, redirect } from '@tanstack/react-router';
import {
	DocsPageContent,
	loadDocsPage,
	preloadDocsContent,
} from '@/lib/docs-page';

export const Route = createFileRoute('/docs/$')({
	component: Page,
	loader: async ({ params }) => {
		// Keep the current API version out of navigation while retaining a stable
		// entry URL for readers and old links.
		if (params._splat?.replace(/\/$/, '') === 'api') {
			throw redirect({ href: '/docs/api/latest' });
		}

		const slugs = params._splat?.split('/') ?? [];
		const data = await loadDocsPage({ data: slugs });
		await preloadDocsContent(data);
		return data;
	},
});

function Page() {
	const data = Route.useLoaderData();

	return <DocsPageContent data={data} />;
}
