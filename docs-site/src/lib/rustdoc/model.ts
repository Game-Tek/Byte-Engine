/**
 * The stable API model the documentation site renders.
 *
 * `build-model.ts` produces it from rustdoc's unstable JSON, and the site never
 * reads rustdoc JSON directly. Change this file only when the pages need new
 * information; rustdoc format changes stay inside `format.ts` and `build-model.ts`.
 */

/** The site route under which the crate's pages live. Every {@link ApiItem.route} starts with it. */
export const API_ROUTE_PREFIX = '/docs/api/latest';

/** The highlighting role of a {@link Token}. */
export type TokenKind =
	| 'keyword'
	| 'type'
	| 'primitive'
	| 'generic'
	| 'lifetime'
	| 'fn'
	| 'name'
	| 'literal'
	| 'punct';

/** One piece of a Rust declaration; `href` links a referenced item to its page. */
export interface Token {
	text: string;
	kind?: TokenKind;
	href?: string;
}

/** The kinds of items that appear in the API section. */
export type ApiItemKind =
	| 'module'
	| 'struct'
	| 'enum'
	| 'union'
	| 'trait'
	| 'trait_alias'
	| 'type_alias'
	| 'function'
	| 'constant'
	| 'static'
	| 'macro';

/** The `ApiModel` interface is the document the site loads to build every Rust API page. */
export interface ApiModel {
	crate: string;
	version: string | null;
	formatVersion: number;
	/** The key of the crate root module in {@link items}. */
	root: string;
	/** Every public item, keyed by its canonical Rust path, such as `byte_engine::time::MediaTime`. */
	items: Record<string, ApiItem>;
}

export interface SourceLocation {
	file: string;
	line: number;
}

export interface ApiDeprecation {
	since: string | null;
	note: string | null;
}

/** The `ApiItem` interface describes one public item at its canonical path, with everything its page shows. */
export interface ApiItem {
	key: string;
	kind: ApiItemKind;
	name: string;
	/** The canonical path segments, starting with the crate name. */
	path: string[];
	/** The site URL. Items without their own page link to an anchor on their module page. */
	route: string;
	/** Whether the item has its own page. Functions, constants, statics, and macros live on their module page. */
	page: boolean;
	/** The anchor of an item that lives on its module page. */
	anchor?: string;
	/** The key of the module that lists this item at its canonical path. */
	parent: string | null;
	/** The first sentence of the docs, as plain text. */
	summary: string;
	/** The doc comment as Markdown, with intra-doc links resolved to site or external URLs. */
	docs: string;
	declaration: Token[];
	/** Functions: parameters and return type, for parameter tables. */
	signature?: ApiSignature;
	source: SourceLocation | null;
	deprecation: ApiDeprecation | null;
	/** Modules: the items listed in this module, including re-exports of items whose canonical path is elsewhere. */
	children?: ApiChild[];
	/** Modules: re-exports of items from other crates. */
	externalReexports?: ApiExternalReexport[];
	fields?: ApiField[];
	variants?: ApiVariant[];
	/** Types: inherent `impl` blocks with their public members. */
	inherent?: ApiImplBlock[];
	/** Types: trait implementations, without auto-trait and blanket implementations. */
	traitImpls?: ApiTraitImpl[];
	/** Traits: members an implementation must define. */
	required?: ApiMember[];
	/** Traits: members with a default the implementation can keep. */
	provided?: ApiMember[];
	/** Traits: the implementations in this crate. */
	implementors?: ApiImplementor[];
	/** Types: public functions and methods that take this type as a parameter. */
	acceptedBy?: ApiReference[];
	/** Types: public functions and methods that return this type. */
	returnedBy?: ApiReference[];
}

export interface ApiChild {
	/** The key of the item in {@link ApiModel.items}. */
	key: string;
	/** The name the item has in this module, which differs from the item name for renamed re-exports. */
	name: string;
	/** Whether the canonical path of the item is elsewhere. */
	reexport: boolean;
}

export interface ApiExternalReexport {
	name: string;
	/** The path as written in the `pub use`. */
	source: string;
	href?: string;
}

export interface ApiField {
	name: string;
	anchor: string;
	type: Token[];
	summary: string;
	docs: string;
}

export interface ApiVariant {
	name: string;
	anchor: string;
	declaration: Token[];
	summary: string;
	docs: string;
	fields: ApiField[];
}

export type ApiMemberKind = 'method' | 'assoc_const' | 'assoc_type';

/** The parameters and return type of a function or method. `self` receivers are left out. */
export interface ApiSignature {
	parameters: { name: string; type: Token[] }[];
	returns: Token[] | null;
}

export interface ApiMember {
	name: string;
	kind: ApiMemberKind;
	anchor: string;
	declaration: Token[];
	/** Methods: parameters and return type. */
	signature?: ApiSignature;
	summary: string;
	docs: string;
	source: SourceLocation | null;
	deprecation: ApiDeprecation | null;
}

export interface ApiImplBlock {
	header: Token[];
	members: ApiMember[];
}

/** Where an implemented trait comes from, which decides how prominently a page shows it. */
export type ApiTraitOrigin = 'engine' | 'std' | 'external';

export interface ApiTraitImpl {
	header: Token[];
	traitName: string;
	traitHref?: string;
	origin: ApiTraitOrigin;
	/** Whether a `#[derive]` produced the implementation. */
	derived: boolean;
	/** Members defined by the implementation. Empty for `std` and external traits. */
	members: ApiMember[];
}

export interface ApiImplementor {
	header: Token[];
	href?: string;
}

/** A link to a function or method, used by cross-reference lists. */
export interface ApiReference {
	label: string;
	href: string;
	summary: string;
}
