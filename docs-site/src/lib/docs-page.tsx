import { notFound } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
import type * as PageTree from 'fumadocs-core/page-tree';
import type { TOCItemType } from 'fumadocs-core/toc';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import {
	DocsBody,
	DocsDescription,
	DocsPage,
	DocsTitle,
} from 'fumadocs-ui/page';
import {
	type RefObject,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from 'react';
import browserCollections from '@/.source/browser';
import { baseOptions } from '@/lib/layout.shared';
import { source } from '@/lib/source';

export type DocsPageData = {
	tree: object;
	path: string;
};

export const loadDocsPage = createServerFn({
	method: 'GET',
})
	.validator((slugs: string[]) => slugs)
	.handler(async ({ data: slugs }) => {
		const page = source.getPage(slugs);
		if (!page) throw notFound();

		return {
			tree: source.pageTree as object,
			path: page.path,
		};
	});

const clientLoader = browserCollections.docs.createClientLoader({
	id: 'docs',
	component({ toc, frontmatter, default: MDX }) {
		const contentRef = useRef<HTMLDivElement>(null);
		const visibleToc = useVisibleTableOfContents(toc, contentRef);

		return (
			<DocsPage
				toc={visibleToc}
				footer={{ className: 'be-page-navigation' }}
			>
				<DocsTitle>{frontmatter.title}</DocsTitle>
				<DocsDescription>{frontmatter.description}</DocsDescription>
				<DocsBody ref={contentRef}>
					<MDX
						components={{
							...defaultMdxComponents,
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

export async function preloadDocsContent(path: string) {
	await clientLoader.preload(path);
}

export function DocsPageContent({ data }: { data: DocsPageData }) {
	const Content = clientLoader.getComponent(data.path);
	const tree = useMemo(
		() => transformPageTree(data.tree as PageTree.Folder),
		[data.tree],
	);

	return (
		<DocsLayout {...baseOptions()} tree={tree}>
			<Content />
		</DocsLayout>
	);
}

function transformPageTree(tree: PageTree.Folder): PageTree.Folder {
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

	return {
		...tree,
		icon: transformIcon(tree.icon),
		index: tree.index ? transform(tree.index) : undefined,
		children: tree.children.map((item) => {
			if (item.type === 'folder') return transformPageTree(item);
			return transform(item);
		}),
	};
}
