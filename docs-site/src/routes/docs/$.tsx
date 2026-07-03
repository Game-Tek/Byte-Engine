import { createFileRoute } from '@tanstack/react-router';
import {
	DocsPageContent,
	loadDocsPage,
	preloadDocsContent,
} from '@/lib/docs-page';

export const Route = createFileRoute('/docs/$')({
	component: Page,
	loader: async ({ params }) => {
		const slugs = params._splat?.split('/') ?? [];
		const data = await loadDocsPage({ data: slugs });
		await preloadDocsContent(data.path);
		return data;
	},
});

function Page() {
	const data = Route.useLoaderData();

	return <DocsPageContent data={data} />;
}
