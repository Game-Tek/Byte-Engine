import { notFound } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
import type { Node, Root } from 'fumadocs-core/page-tree';
import {
	deserializePageTree,
	type SerializedPageTree,
} from 'fumadocs-core/source/client';
import type { TOCItemType } from 'fumadocs-core/toc';
import type { OpenAPIPageProps_Spec } from 'fumadocs-openapi/ui';
import { GithubInfo } from 'fumadocs-ui/components/github-info';
import { Step, Steps } from 'fumadocs-ui/components/steps';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import type { LayoutTab } from 'fumadocs-ui/layouts/shared';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import {
	DocsBody,
	DocsDescription,
	DocsPage,
	DocsTitle,
	MarkdownCopyButton,
	ViewOptionsPopover,
} from 'fumadocs-ui/layouts/docs/page';
import {
	type RefObject,
	type ReactNode,
	isValidElement,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from 'react';
import browserCollections from '@/.source/browser';
import { InspectorOpenAPIPage } from '@/components/inspector-openapi-page';
import { getMarkdownUrl } from '@/lib/llm';
import { baseOptions } from '@/lib/layout.shared';
import { source } from '@/lib/source';

type SharedDocsPageData = {
	tree: SerializedPageTree;
	path: string;
	url: string;
};

type MdxDocsPageData = SharedDocsPageData & {
	kind: 'mdx';
};

type OpenAPITocItem = Omit<TOCItemType, 'title'> & {
	title: string;
};

type OpenAPIDocsPageData = SharedDocsPageData & {
	kind: 'openapi';
	title: string;
	toc: OpenAPITocItem[];
	openapiProps: OpenAPIPageProps_Spec;
};

export type DocsPageData = MdxDocsPageData | OpenAPIDocsPageData;

export const loadDocsPage = createServerFn({
	method: 'GET',
})
	.validator((slugs: string[]) => slugs)
	.handler(async ({ data: slugs }) => {
		const page = source.getPage(slugs);
		if (!page) throw notFound();
		const shared = {
			tree: await source.serializePageTree(source.pageTree),
			path: page.path,
			url: page.url,
		};

		if (page.type === 'inspector') {
			return {
				...shared,
				kind: 'openapi' as const,
				title: page.data.title ?? 'Inspector API',
				toc: page.data.toc.map(({ title, ...item }) => ({
					...item,
					title: typeof title === 'string' ? title : '',
				})),
				openapiProps: page.data.getOpenAPIPageProps(),
			};
		}

		return {
			...shared,
			kind: 'mdx' as const,
		};
	});

type DocsContentProps = {
	githubUrl: string;
	markdownUrl: string;
};

const clientLoader = browserCollections.docs.createClientLoader<DocsContentProps>({
	id: 'docs',
	component({ toc, frontmatter, default: MDX }, pageActions) {
		const contentRef = useRef<HTMLDivElement>(null);
		const visibleToc = useVisibleTableOfContents(toc, contentRef);

		return (
			<DocsPage
				toc={visibleToc}
				footer={{ className: 'be-page-navigation' }}
			>
				<div className="flex justify-end gap-2">
					<MarkdownCopyButton markdownUrl={pageActions.markdownUrl} />
					<ViewOptionsPopover
						githubUrl={pageActions.githubUrl}
						markdownUrl={pageActions.markdownUrl}
					/>
				</div>
				<DocsTitle>{frontmatter.title}</DocsTitle>
				<DocsDescription>{frontmatter.description}</DocsDescription>
				<DocsBody ref={contentRef}>
					<MDX
						components={{
							...defaultMdxComponents,
							Step,
							Steps,
							Tab,
							Tabs,
						}}
					/>
				</DocsBody>
			</DocsPage>
		);
	},
});

function useVisibleTableOfContents(
	toc: TOCItemType[],
	contentRef: RefObject<HTMLDivElement | null>,
) {
	const [visibleToc, setVisibleToc] = useState(toc);

	useLayoutEffect(() => {
		const content = contentRef.current;
		if (!content) return;

		const update = () => {
			setVisibleToc(
				toc.filter((item) => {
					if (!item.url.startsWith('#')) return true;

					const heading = document.getElementById(
						decodeURIComponent(item.url.slice(1)),
					);
					if (!heading || !content.contains(heading)) return true;

					return (
						heading.closest('[role="tabpanel"][data-state="inactive"]') ===
						null
					);
				}),
			);
		};

		update();

		// Fumadocs extracts every heading before client-side tabs hide inactive panels.
		// Refilter the TOC whenever a rendered tab panel changes visibility.
		const observer = new MutationObserver(update);
		for (const panel of content.querySelectorAll('[role="tabpanel"]')) {
			observer.observe(panel, {
				attributes: true,
				attributeFilter: ['data-state'],
			});
		}

		return () => observer.disconnect();
	}, [contentRef, toc]);

	return visibleToc;
}

export async function preloadDocsContent(data: DocsPageData) {
	if (data.kind === 'mdx') {
		await clientLoader.preload(data.path);
	}
}

const sectionRoutes = new Map([
	['Introduction', '/docs'],
	['Use', '/docs/use'],
	['Contribute', '/docs/develop'],
	['Reference', '/docs/reference'],
	['API', '/docs/api/latest'],
]);

function addPageUrls(node: Node, urls: Set<string>) {
	if (node.type === 'page') {
		urls.add(node.url);
		return;
	}

	if (node.type === 'folder') {
		if (node.index) urls.add(node.index.url);
		for (const child of node.children) addPageUrls(child, urls);
	}
}

function getPageTreeName(name: ReactNode) {
	if (typeof name === 'string') return name;
	if (
		!isValidElement<{
			dangerouslySetInnerHTML?: { __html?: string };
		}>(name)
	)
		return;

	return name.props.dangerouslySetInnerHTML?.__html;
}

// The sidebar sections are intentionally flattened, so provide the layout tabs
// explicitly instead of relying on root folder nodes that no longer exist.
// Fumadocs wraps serialized names in spans, so read the original HTML label.
function getSectionTabs(tree: Root): LayoutTab[] {
	const tabs: LayoutTab[] = [];
	let currentUrls: Set<string> | undefined;

	for (const node of tree.children) {
		if (node.type === 'separator') {
			const name = getPageTreeName(node.name);
			const url = name ? sectionRoutes.get(name) : undefined;
			if (url) {
				currentUrls = new Set([url]);
				tabs.push({
					title: node.name,
					icon: node.icon,
					url,
					urls: currentUrls,
				});
			}
			continue;
		}

		if (currentUrls) addPageUrls(node, currentUrls);
	}

	return tabs;
}

export function DocsPageContent({ data }: { data: DocsPageData }) {
	const tree = useMemo(
		() => deserializePageTree(data.tree),
		[data.tree],
	);
	const tabs = useMemo(() => getSectionTabs(tree), [tree]);

	return (
		<DocsLayout
			{...baseOptions()}
			tabs={tabs}
			sidebar={{
				footer: (
					<GithubInfo
						className="be-github-info"
						fetchOptions={{
							headers: { 'User-Agent': 'Byte-Engine-Docs' },
						}}
						owner="Game-Tek"
						repo="Byte-Engine"
					/>
				),
			}}
			tree={tree}
		>
			{data.kind === 'openapi' ? (
				<OpenAPIDocsContent data={data} />
			) : (
				<MdxDocsContent data={data} />
			)}
		</DocsLayout>
	);
}

function MdxDocsContent({ data }: { data: MdxDocsPageData }) {
	const Content = clientLoader.getComponent(data.path);
	const markdownUrl = getMarkdownUrl(data.url);
	const githubUrl = `https://github.com/Game-Tek/Byte-Engine/blob/main/docs/${data.path}`;

	return <Content githubUrl={githubUrl} markdownUrl={markdownUrl} />;
}

function OpenAPIDocsContent({ data }: { data: OpenAPIDocsPageData }) {
	const contentRef = useRef<HTMLDivElement>(null);
	const visibleToc = useVisibleTableOfContents(data.toc, contentRef);

	return (
		<DocsPage
			full
			toc={visibleToc}
			footer={{ className: 'be-page-navigation' }}
		>
			<DocsTitle>{data.title}</DocsTitle>
			<DocsBody ref={contentRef}>
				<InspectorOpenAPIPage {...data.openapiProps} />
			</DocsBody>
		</DocsPage>
	);
}
