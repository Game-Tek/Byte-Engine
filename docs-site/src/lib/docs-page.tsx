import { notFound } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
import type * as PageTree from 'fumadocs-core/page-tree';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import {
	DocsBody,
	DocsDescription,
	DocsPage,
	DocsTitle,
} from 'fumadocs-ui/page';
import { useMemo } from 'react';
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
		return (
			<DocsPage toc={toc}>
				<DocsTitle>{frontmatter.title}</DocsTitle>
				<DocsDescription>{frontmatter.description}</DocsDescription>
				<DocsBody>
					<MDX
						components={{
							...defaultMdxComponents,
						}}
					/>
				</DocsBody>
			</DocsPage>
		);
	},
});

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
