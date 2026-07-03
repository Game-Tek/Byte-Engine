import { createFileRoute } from '@tanstack/react-router';
import {
	DocsPageContent,
	loadDocsPage,
	preloadDocsContent,
} from '@/lib/docs-page';

export const Route = createFileRoute('/')({
	component: Home,
	loader: async () => {
		const data = await loadDocsPage({ data: [] });
		await preloadDocsContent(data.path);
		return data;
	},
});

function Home() {
	const data = Route.useLoaderData();

	return <DocsPageContent data={data} />;
}
