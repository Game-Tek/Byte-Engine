import { notFound } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
import type { Folder, Node, Root } from 'fumadocs-core/page-tree';
import {
	deserializePageTree,
	type SerializedPageTree,
} from 'fumadocs-core/source/client';
import type { TOCItemType } from 'fumadocs-core/toc';
import type { OpenAPIPageProps_Spec } from 'fumadocs-openapi/ui';
import { GithubInfo } from 'fumadocs-ui/components/github-info';
import { Step, Steps } from 'fumadocs-ui/components/steps';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import { buttonVariants } from 'fumadocs-ui/components/ui/button';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { getLayoutTabs } from 'fumadocs-ui/layouts/shared';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { IconBox, IconConnections } from 'nucleo-glass';
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
	docsRsUrl?: string;
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
					{pageActions.docsRsUrl && (
						<a
							className={buttonVariants({
								color: 'secondary',
								size: 'sm',
								className: 'gap-2 [&_svg]:size-3.5',
							})}
							href={pageActions.docsRsUrl}
							rel="noreferrer"
							target="_blank"
						>
							View on docs.rs
							<svg
								aria-hidden="true"
								fill="none"
								stroke="currentColor"
								strokeLinecap="round"
								strokeLinejoin="round"
								strokeWidth="2"
								viewBox="0 0 24 24"
							>
								<path d="M7 17 17 7" />
								<path d="M7 7h10v10" />
							</svg>
						</a>
					)}
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
					if (!heading || !content.contains(heading)) return false;

					return (
						heading.closest('[role="tabpanel"][data-state="inactive"]') ===
						null
					);
				}),
			);
		};

		update();

		// Fumadocs extracts every heading before client-side tabs mount only the
		// active panel. Refilter when panels mount or change visibility.
		const observer = new MutationObserver(update);
		observer.observe(content, {
			attributes: true,
			attributeFilter: ['data-state'],
			childList: true,
			subtree: true,
		});

		return () => observer.disconnect();
	}, [contentRef, toc]);

	return visibleToc;
}

export async function preloadDocsContent(data: DocsPageData) {
	if (data.kind === 'mdx') {
		await clientLoader.preload(data.path);
	}
}

const sections = new Map([
	[
		'Introduction',
		{
			url: '/docs',
			description: 'Start using Byte Engine.',
		},
	],
	[
		'Use',
		{
			url: '/docs/use',
			description: 'Use Byte Engine to develop.',
		},
	],
	[
		'Contribute',
		{
			url: '/docs/develop',
			description: 'Become a Byte Engine developer.',
		},
	],
	[
		'Reference',
		{
			url: '/docs/reference',
			description:
				"Understand the concepts and design principles behind Byte Engine's systems.",
		},
	],
	[
		'API',
		{
			url: '/docs/api/latest',
			description: 'Byte Engine HTTP and Rust API documentation.',
		},
	],
]);

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

function ApiNavigationName({ name, kind }: { name: string; kind: string }) {
	return (
		<span className="be-api-navigation-name">
			<span>{name}</span>
			<span className="be-api-navigation-kind" data-kind={kind}>
				{kind}
			</span>
		</span>
	);
}

function RustModuleNavigationName({ name }: { name: string }) {
	return (
		<span className="be-rust-module-name">
			<span>{name}</span>
			<span className="be-rust-module-kind">mod</span>
		</span>
	);
}

// Generated folders under the crate represent Rust modules. Decorate the
// merged tree so nested modules receive the marker without changing output.
function decorateRustModuleNavigation(node: Node): Node {
	if (node.type !== 'folder') return node;

	const name = getPageTreeName(node.name);
	return {
		...node,
		name: name ? <RustModuleNavigationName name={name} /> : node.name,
		children: node.children.map(decorateRustModuleNavigation),
	};
}

// The HTTP reference is virtual and the Rust reference is generated, so add
// their shared navigation treatment after both sources become one page tree.
function decorateApiNavigation(node: Node): Node {
	if (node.type !== 'folder') return node;

	const name = getPageTreeName(node.name);
	if (name === 'Inspector') {
		return {
			...node,
			name: <ApiNavigationName name={name} kind="http" />,
			icon: (
				<IconConnections
					aria-hidden
					className="be-glass-icon"
					uniqueId="be-inspector-http-"
				/>
			),
		};
	}

	if (node.index?.url === '/docs/api/latest/byte_engine') {
		return {
			...node,
			name: <ApiNavigationName name="byte_engine" kind="rust" />,
			children: node.children.map(decorateRustModuleNavigation),
			icon: (
				<IconBox
					aria-hidden
					className="be-glass-icon"
					uniqueId="be-byte-engine-crate-"
				/>
			),
		};
	}

	return node;
}

// Rebuild root folders from the flattened sections so Fumadocs can scope the
// sidebar and render its native tab selector. Serialized names arrive as spans.
function getSectionTree(tree: Root): Root {
	const children: Folder[] = [];
	let current: Folder | undefined;

	for (const node of tree.children) {
		if (node.type === 'separator') {
			const name = getPageTreeName(node.name);
			const section = name ? sections.get(name) : undefined;
			if (name && section) {
				current = {
					type: 'folder',
					$id: `section:${name}`,
					name: node.name,
					icon: node.icon,
					description: section.description,
					root: true,
					index: {
						type: 'page',
						name: node.name ?? name,
						url: section.url,
					},
					children: [],
				};
				children.push(current);
				continue;
			}
		}

		current?.children.push(node);
	}

	for (const folder of children) {
		if (folder.index?.url === '/docs/api/latest') {
			folder.children = folder.children.map(decorateApiNavigation);
		}

		const index = folder.children.findIndex(
			(node) => node.type === 'page' && node.url === folder.index?.url,
		);
		if (index < 0) continue;

		const [page] = folder.children.splice(index, 1);
		if (page.type === 'page') folder.index = page;
	}

	return {
		...tree,
		children,
	};
}

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

// Use explicit URL sets because Fumadocs' folder matcher does not include the
// folder index. The root folders still control which sidebar tree is visible.
function getSectionTabs(tree: Root) {
	return getLayoutTabs(tree).map(({ $folder, ...tab }) => {
		const urls = new Set([tab.url]);
		if ($folder) {
			for (const child of $folder.children) addPageUrls(child, urls);
		}

		return { ...tab, urls };
	});
}

export function DocsPageContent({ data }: { data: DocsPageData }) {
	const tree = useMemo(
		() => deserializePageTree(data.tree),
		[data.tree],
	);
	const sectionTree = useMemo(() => getSectionTree(tree), [tree]);
	const tabs = useMemo(() => getSectionTabs(sectionTree), [sectionTree]);

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
			tree={sectionTree}
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
	const docsRsUrl = getDocsRsUrl(data.url);

	return (
		<Content
			docsRsUrl={docsRsUrl}
			githubUrl={githubUrl}
			markdownUrl={markdownUrl}
		/>
	);
}

const rustApiBaseUrl = '/docs/api/latest/byte_engine';
const docsRsBaseUrl = 'https://docs.rs/byte-engine/latest/byte_engine/';

function getDocsRsUrl(url: string) {
	if (url === rustApiBaseUrl) return docsRsBaseUrl;
	if (!url.startsWith(`${rustApiBaseUrl}/`)) return;

	const modulePath = url.slice(rustApiBaseUrl.length + 1);
	return `${docsRsBaseUrl}${modulePath}/index.html`;
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
