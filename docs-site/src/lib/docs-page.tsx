import { notFound } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
import type * as PageTree from 'fumadocs-core/page-tree';
import type { TOCItemType } from 'fumadocs-core/toc';
import type { OpenAPIPageProps_Preloaded } from 'fumadocs-openapi/ui';
import { GithubInfo } from 'fumadocs-ui/components/github-info';
import { Step, Steps } from 'fumadocs-ui/components/steps';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { getLayoutTabs } from 'fumadocs-ui/layouts/shared';
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
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from 'react';
import browserCollections from '@/.source/browser';
import { InspectorOpenAPIPage } from '@/components/inspector-openapi-page';
import { getMarkdownUrl } from '@/lib/llm';
import { baseOptions } from '@/lib/layout.shared';
import { inspectorOpenAPI } from '@/lib/openapi';
import { source } from '@/lib/source';

export type DocsPageData = {
	tree: object;
	path: string;
	url: string;
	openapiPreloaded?: OpenAPIPageProps_Preloaded['preloaded'];
};

export const loadDocsPage = createServerFn({
	method: 'GET',
})
	.validator((slugs: string[]) => slugs)
	.handler(async ({ data: slugs }) => {
		const page = source.getPage(slugs);
		if (!page) throw notFound();
		const openapi = '_openapi' in page.data
			? await inspectorOpenAPI.preloadOpenAPIPage(page)
			: undefined;

		return {
			tree: source.pageTree as object,
			path: page.path,
			url: page.url,
			openapiPreloaded: openapi?.preloaded,
		};
	});

type DocsContentProps = {
	githubUrl: string;
	markdownUrl: string;
	openapiPreloaded?: OpenAPIPageProps_Preloaded['preloaded'];
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
							OpenAPIPage: (props) => {
								if (!pageActions.openapiPreloaded) {
									throw new Error('OpenAPI page data was not preloaded.');
								}

								return (
									<InspectorOpenAPIPage
										{...props}
										preloaded={pageActions.openapiPreloaded}
									/>
								);
							},
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

export async function preloadDocsContent(path: string) {
	await clientLoader.preload(path);
}

export function DocsPageContent({ data }: { data: DocsPageData }) {
	const Content = clientLoader.getComponent(data.path);
	const markdownUrl = getMarkdownUrl(data.url);
	const githubUrl = `https://github.com/Game-Tek/Byte-Engine/blob/main/docs/${data.path}`;
	const tree = useMemo(
		() => transformPageTree(data.tree as PageTree.Root),
		[data.tree],
	);
	const tabs = useMemo(
		() => getLayoutTabs(tree).map(({ $folder: _folder, ...tab }) => tab),
		[tree],
	);

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
			<Content
				githubUrl={githubUrl}
				markdownUrl={markdownUrl}
				openapiPreloaded={data.openapiPreloaded}
			/>
		</DocsLayout>
	);
}

function transformPageTree(tree: PageTree.Root): PageTree.Root {
	function transformIcon(icon: PageTree.Item['icon']) {
		if (typeof icon !== 'string') return icon;

		return (
			<span
				dangerouslySetInnerHTML={{
					__html: icon,
				}}
			/>
		);
	}

	function transform<T extends PageTree.Item | PageTree.Separator>(item: T) {
		if (typeof item.icon !== 'string') return item;

		return {
			...item,
			icon: transformIcon(item.icon),
		};
	}

	function transformFolder(folder: PageTree.Folder): PageTree.Folder {
		return {
			...folder,
			icon: transformIcon(folder.icon),
			index: folder.index ? transform(folder.index) : undefined,
			children: folder.children.map((item) => {
				if (item.type === 'folder') return transformFolder(item);
				return transform(item);
			}),
		};
	}

	return {
		...tree,
		fallback: tree.fallback ? transformPageTree(tree.fallback) : undefined,
		children: tree.children.map((item) => {
			if (item.type === 'folder') return transformFolder(item);
			return transform(item);
		}),
	};
}
