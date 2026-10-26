import type { LoaderPlugin, StaticSource, VirtualFile } from 'fumadocs-core/source';
import type { StructuredData } from 'fumadocs-core/mdx-plugins/remark-structure';
import type { ReactNode } from 'react';
import type { ApiItem, ApiModel } from '@/lib/rustdoc/model';
import { describe, KINDS, moduleChildren } from './items';

/** The content directory where the Rust API pages live, matching `docs/api/meta.json`. */
const BASE_DIR = 'api/latest';

/** The page data the Rust API source attaches to each virtual page. */
export interface RustPageData {
	title: string;
	description: string;
	structuredData: StructuredData;
	/** The model key of the item the page shows, or `null` for the crate landing page. */
	rustKey: string | null;
	rustKind: ApiItem['kind'] | 'landing';
}

type RustSourceConfig = {
	pageData: RustPageData;
	metaData: { title: string; description?: string; pages: string[]; defaultOpen?: boolean };
};

/**
 * Builds the Rust API pages and sidebar folders from the model. Fumadocs serves them
 * like MDX pages; `loadDocsPage` renders them from the model instead of compiled MDX.
 */
export function rustApiSource(model: ApiModel | undefined): StaticSource<RustSourceConfig> {
	if (!model) return { files: [] };

	const root = model.items[model.root];
	const files: VirtualFile<RustSourceConfig>[] = [
		{
			type: 'page',
			path: `${BASE_DIR}/index.mdx`,
			data: {
				title: 'Latest Rust API',
				description: 'Browse the generated API reference for the latest Byte Engine revision.',
				structuredData: { headings: [], contents: [] },
				rustKey: null,
				rustKind: 'landing',
			},
		},
		{
			type: 'meta',
			path: `${BASE_DIR}/meta.json`,
			data: {
				title: 'Latest',
				description: 'Generated API reference for the latest Byte Engine revision.',
				pages: ['!index', root.name],
			},
		},
	];

	for (const item of Object.values(model.items)) {
		if (!item.page) continue;
		const directory = `${BASE_DIR}/${pageDirectory(item)}`;
		const pagePath = item.kind === 'module' ? `${directory}/index.mdx` : `${directory}.mdx`;
		files.push({
			type: 'page',
			path: pagePath,
			data: {
				title: item.name,
				description: describe(item),
				structuredData: structuredData(model, item),
				rustKey: item.key,
				rustKind: item.kind,
			},
		});

		if (item.kind === 'module') {
			files.push({
				type: 'meta',
				path: `${directory}/meta.json`,
				data: {
					title: item.name,
					description: describe(item),
					pages: sidebarPages(model, item),
					defaultOpen: item.key === model.root,
				},
			});
		}
	}

	return { files };
}

/** The directory of a page relative to {@link BASE_DIR}, taken from its route. */
function pageDirectory(item: ApiItem): string {
	return item.route.slice(item.route.indexOf(`/${BASE_DIR}/`) + BASE_DIR.length + 2);
}

/** Lists a module's own pages for the sidebar: submodules, then types in kind order. Each entry carries its kind tag. */
function sidebarPages(model: ApiModel, module: ApiItem): string[] {
	return moduleChildren(model, module)
		// Re-exports and items without pages are listed on the module page, not in the sidebar.
		.filter(({ item, reexport }) => !reexport && item.page && item.parent === module.key)
		.map(({ item }) => item.route.split('/').at(-1)!);
}

/** Indexes the headings and summaries a reader searches for: the item, its members, and listed children. */
function structuredData(model: ApiModel, item: ApiItem): StructuredData {
	const headings: StructuredData['headings'] = [];
	const contents: StructuredData['contents'] = [{ heading: undefined, content: describe(item) }];
	const add = (id: string, title: string, summary: string) => {
		headings.push({ id, content: title });
		if (summary) contents.push({ heading: id, content: summary });
	};

	for (const field of item.fields ?? []) add(field.anchor, field.name, field.summary);
	for (const variant of item.variants ?? []) add(variant.anchor, variant.name, variant.summary);
	for (const block of item.inherent ?? []) {
		for (const member of block.members) add(member.anchor, `${item.name}::${member.name}`, member.summary);
	}
	for (const member of [...(item.required ?? []), ...(item.provided ?? [])]) {
		add(member.anchor, `${item.name}::${member.name}`, member.summary);
	}
	if (item.kind === 'module') {
		for (const { item: child } of moduleChildren(model, item)) {
			if (!child.page && child.anchor && child.parent === item.key) add(child.anchor, child.name, child.summary);
		}
	}
	return { headings, contents };
}

/**
 * Marks Rust items and module folders in the sidebar with their kind, the way the
 * OpenAPI plugin marks operations with their HTTP method.
 */
export function rustApiLoaderPlugin(): LoaderPlugin {
	return {
		name: 'byte-engine:rust-api',
		transformPageTree: {
			file(node, filePath) {
				const kind = filePath ? rustKind(this.storage.read(filePath)) : undefined;
				if (kind && kind !== 'module') node.name = kindName(node.name, kind);
				return node;
			},
			folder(node, folderPath) {
				if (rustKind(this.storage.read(`${folderPath}/index.mdx`)) === 'module') node.name = kindName(node.name, 'module');
				return node;
			},
		},
	};
}

function rustKind(file: { format: string; data: unknown } | undefined): ApiItem['kind'] | undefined {
	if (file?.format !== 'page') return undefined;
	const kind = (file.data as Partial<RustPageData>).rustKind;
	return kind === 'landing' ? undefined : kind;
}

function kindName(name: ReactNode, kind: ApiItem['kind']): ReactNode {
	return (
		<span className="be-rust-module-name">
			<span>{name}</span>
			<span className="be-rust-module-kind" data-kind={kind}>
				{KINDS[kind].label}
			</span>
		</span>
	);
}
