import type { Element, Root } from 'hast';
import { createRehypeCode, rehypeCodeDefaultOptions } from 'fumadocs-core/mdx-plugins/rehype-code.core';
import { remarkGfm } from 'fumadocs-core/mdx-plugins/remark-gfm';
import remarkParse from 'remark-parse';
import remarkRehype from 'remark-rehype';
import { createHighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';
import bash from 'shiki/langs/bash.mjs';
import json from 'shiki/langs/json.mjs';
import rust from 'shiki/langs/rust.mjs';
import toml from 'shiki/langs/toml.mjs';
import githubDark from 'shiki/themes/github-dark.mjs';
import githubLight from 'shiki/themes/github-light.mjs';
import { unified } from 'unified';
import { beslGrammar } from '@/lib/besl-grammar';
import type {
	ApiDeprecation,
	ApiField,
	ApiImplBlock,
	ApiItem,
	ApiMember,
	ApiModel,
	ApiTraitImpl,
	ApiVariant,
	Token,
} from '@/lib/rustdoc/model';
import { rustdocPagePath, withoutSummary } from '@/lib/rustdoc/build-model';
import { describe, KIND_ORDER, KINDS, moduleChildren } from './items';

const GITHUB_SOURCE_URL = 'https://github.com/Game-Tek/Byte-Engine/blob/master';
const DOCS_RS_URL = 'https://docs.rs/byte-engine/latest';

/** Doc comment Markdown converted to a highlighted HAST tree that the client renders with the MDX components. */
export type DocsTree = Root;

/** Replaces each `docs` string of a model type with its rendered tree. */
type WithDocsTree<T> = Omit<T, 'docs'> & { docs: DocsTree | null };

/** One section with a heading anchor: a member of a type or trait, or a function, constant, or macro on its module page. */
export interface SectionView {
	name: string;
	anchor: string;
	declaration: Token[];
	deprecation: ApiDeprecation | null;
	docs: DocsTree | null;
	sourceUrl?: string;
}

export type MemberView = SectionView & Pick<ApiMember, 'kind' | 'signature' | 'summary'>;
export type FieldView = WithDocsTree<ApiField>;
export type VariantView = Omit<WithDocsTree<ApiVariant>, 'fields'> & { fields: FieldView[] };
export type ImplBlockView = Omit<ApiImplBlock, 'members'> & { members: MemberView[] };
export type TraitImplView = Omit<ApiTraitImpl, 'members'> & { members: MemberView[] };

/** One entry of a module listing. Items without their own page carry their full section. */
export interface ChildView {
	name: string;
	kind: ApiItem['kind'];
	route: string;
	summary: string;
	reexport: boolean;
	deprecated: boolean;
	section?: SectionView;
}

export interface ItemView {
	kind: ApiItem['kind'];
	name: string;
	declaration: Token[];
	deprecation: ApiDeprecation | null;
	docs: DocsTree | null;
	breadcrumbs: { name: string; href: string }[];
	sourceUrl?: string;
	docsRsUrl: string;
	children?: { kind: ApiItem['kind']; title: string; entries: ChildView[] }[];
	externalReexports?: ApiItem['externalReexports'];
	fields?: FieldView[];
	variants?: VariantView[];
	inherent?: ImplBlockView[];
	traitImpls?: TraitImplView[];
	required?: MemberView[];
	provided?: MemberView[];
	implementors?: ApiItem['implementors'];
	acceptedBy?: ApiItem['acceptedBy'];
	returnedBy?: ApiItem['returnedBy'];
}

export interface LandingView {
	crate: string;
	version: string | null;
	root: ChildView;
	modules: ChildView[];
}

export interface RustTocItem {
	title: string;
	url: string;
	depth: number;
}

export type RustPageView =
	| { kind: 'landing'; landing: LandingView; toc: RustTocItem[] }
	| { kind: 'item'; item: ItemView; toc: RustTocItem[] };

/** Builds the serializable view of one Rust API page; `key` is `null` for the crate landing page. */
export async function buildRustPageView(model: ApiModel, key: string | null): Promise<RustPageView> {
	if (key === null) {
		const root = model.items[model.root];
		const modules = moduleChildren(model, root).filter(({ item }) => item.kind === 'module');
		return {
			kind: 'landing',
			landing: {
				crate: model.crate,
				version: model.version,
				root: childView(root, root.name, false),
				modules: modules.map(({ item, name, reexport }) => childView(item, name, reexport)),
			},
			toc: [],
		};
	}

	const view = await itemView(model, model.items[key]);
	return { kind: 'item', item: view, toc: tableOfContents(view) };
}

async function itemView(model: ApiModel, item: ApiItem): Promise<ItemView> {
	const view: ItemView = {
		kind: item.kind,
		name: item.name,
		declaration: item.declaration,
		deprecation: item.deprecation,
		// The page description already shows the summary sentence, so the body starts after it.
		docs: await docsTree(item.summary ? withoutSummary(item.docs) : item.docs, 1),
		breadcrumbs: item.path.slice(0, -1).map((name, index) => ({
			name,
			href: model.items[item.path.slice(0, index + 1).join('::')]?.route ?? '',
		})),
		sourceUrl: sourceUrl(item.source),
		docsRsUrl: `${DOCS_RS_URL}/${rustdocPagePath(item.kind, item.path)}`,
		externalReexports: item.externalReexports,
		implementors: item.implementors,
		acceptedBy: item.acceptedBy,
		returnedBy: item.returnedBy,
	};

	if (item.kind === 'module') {
		const groups = new Map<ApiItem['kind'], ChildView[]>();
		for (const { item: child, name, reexport } of moduleChildren(model, item)) {
			const entry = childView(child, name, reexport);
			// Functions, constants, and macros defined here render in full on this page.
			if (!child.page && !reexport && child.parent === item.key) {
				entry.section = await sectionView({ ...child, anchor: child.anchor! });
			}
			const group = groups.get(child.kind) ?? [];
			group.push(entry);
			groups.set(child.kind, group);
		}
		view.children = KIND_ORDER.filter((kind) => groups.has(kind)).map((kind) => ({
			kind,
			title: KINDS[kind].title,
			entries: groups.get(kind)!,
		}));
	}

	if (item.fields) view.fields = await Promise.all(item.fields.map(fieldView));
	if (item.variants) {
		view.variants = await Promise.all(
			item.variants.map(async (variant) => ({
				...variant,
				docs: await docsTree(variant.docs, 3),
				fields: await Promise.all(variant.fields.map(fieldView)),
			})),
		);
	}
	if (item.inherent) {
		view.inherent = await Promise.all(
			item.inherent.map(async (block) => ({ ...block, members: await Promise.all(block.members.map(memberView)) })),
		);
	}
	if (item.traitImpls) {
		view.traitImpls = await Promise.all(
			item.traitImpls.map(async (traitImpl) => ({ ...traitImpl, members: await Promise.all(traitImpl.members.map(memberView)) })),
		);
	}
	if (item.required) view.required = await Promise.all(item.required.map(memberView));
	if (item.provided) view.provided = await Promise.all(item.provided.map(memberView));
	return view;
}

function childView(item: ApiItem, name: string, reexport: boolean): ChildView {
	return {
		name,
		kind: item.kind,
		route: item.route,
		summary: describe(item),
		reexport,
		deprecated: item.deprecation !== null,
	};
}

async function sectionView(
	section: Pick<ApiMember, 'name' | 'anchor' | 'declaration' | 'deprecation' | 'docs' | 'source'>,
): Promise<SectionView> {
	return {
		name: section.name,
		anchor: section.anchor,
		declaration: section.declaration,
		deprecation: section.deprecation,
		docs: await docsTree(section.docs, 3),
		sourceUrl: sourceUrl(section.source),
	};
}

async function memberView(member: ApiMember): Promise<MemberView> {
	return { ...(await sectionView(member)), kind: member.kind, signature: member.signature, summary: member.summary };
}

async function fieldView(field: ApiField): Promise<FieldView> {
	return { ...field, docs: await docsTree(field.docs, 3) };
}

/** Lists the page sections and members in reading order for the table of contents. */
function tableOfContents(view: ItemView): RustTocItem[] {
	const toc: RustTocItem[] = [];
	const section = (id: string, title: string) => toc.push({ title, url: `#${id}`, depth: 2 });
	const entry = (id: string, title: string) => toc.push({ title, url: `#${id}`, depth: 3 });

	for (const group of view.children ?? []) {
		section(`section-${group.kind}`, group.title);
		for (const child of group.entries) {
			if (child.section) entry(child.section.anchor, child.name);
		}
	}
	if (view.externalReexports?.length) section('section-reexports', 'Re-exports');
	if (view.fields?.length) {
		section('fields', 'Fields');
		for (const field of view.fields) entry(field.anchor, field.name);
	}
	if (view.variants?.length) {
		section('variants', 'Variants');
		for (const variant of view.variants) entry(variant.anchor, variant.name);
	}
	if (view.required?.length) {
		section('required-members', 'Required members');
		for (const member of view.required) entry(member.anchor, member.name);
	}
	if (view.provided?.length) {
		section('provided-members', 'Provided members');
		for (const member of view.provided) entry(member.anchor, member.name);
	}
	if (view.inherent?.length) {
		section('implementations', 'Methods');
		for (const block of view.inherent) for (const member of block.members) entry(member.anchor, member.name);
	}
	if (view.implementors?.length) section('implementors', 'Implementors');
	if (view.traitImpls?.length) section('trait-implementations', 'Trait implementations');
	if (view.acceptedBy?.length || view.returnedBy?.length) section('used-by', 'Used by');
	return toc;
}

function sourceUrl(source: ApiItem['source']): string | undefined {
	return source ? `${GITHUB_SOURCE_URL}/${source.file}#L${source.line}` : undefined;
}

// -------------------------------------------------------------------------
// Markdown

const LANGUAGES = [rust, beslGrammar, json, bash, toml];
const KNOWN_LANGUAGES = new Set(['rust', 'besl', 'json', 'bash', 'sh', 'shell', 'toml', 'text', 'plaintext', 'txt']);

// Doc comments use a few fence languages. Loading only those keeps every other Shiki
// grammar out of the deployed Worker, as the inspector page does.
const rehypeCode = createRehypeCode(async (options) => ({
	highlighter: await createHighlighterCore({
		engine: createJavaScriptRegexEngine(),
		langs: LANGUAGES,
		themes: [githubLight, githubDark],
	}),
	options: { ...rehypeCodeDefaultOptions(), ...options, lazy: false, langs: [] },
}));

const processor = unified()
	.use(remarkParse)
	.use(remarkGfm)
	.use(remarkRehype)
	.use(rehypeCode, {
		themes: { light: 'github-light', dark: 'github-dark' },
	});

/**
 * Renders doc comment Markdown to a highlighted HAST tree. Headings move down by
 * `headingShift` levels so a doc comment's `# Examples` nests under the page's own headings.
 */
export async function docsTree(markdown: string, headingShift: number): Promise<DocsTree | null> {
	if (!markdown) return null;
	const tree = processor.parse(plainFences(markdown));
	const hast = (await processor.run(tree)) as Root;
	visit(hast, (node) => {
		delete (node as { position?: unknown }).position;
		if (node.type === 'element') shiftHeading(node, headingShift);
	});
	return hast;
}

/** Turns fences in languages the Worker doesn't load into plain text fences. */
function plainFences(markdown: string): string {
	return markdown.replace(/^(\s*(?:`{3,}|~{3,}))([\w+-]+)/gm, (whole, fence: string, language: string) =>
		KNOWN_LANGUAGES.has(language) ? whole : `${fence}text`,
	);
}

function shiftHeading(node: Element, shift: number) {
	const level = node.tagName.match(/^h([1-6])$/)?.[1];
	if (!level) return;
	node.tagName = `h${Math.min(6, Number(level) + shift)}`;
}

function visit(node: Root | Root['children'][number], callback: (node: Root | Root['children'][number]) => void) {
	callback(node);
	if ('children' in node) for (const child of node.children) visit(child as Root['children'][number], callback);
}
