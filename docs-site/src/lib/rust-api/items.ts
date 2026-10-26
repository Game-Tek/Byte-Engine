import { API_ROUTE_PREFIX, type ApiItem, type ApiModel, type Token } from '@/lib/rustdoc/model';

/** The URL of the symbol index that `renderSymbolIndex` serves to agents. */
export const SYMBOL_INDEX_URL = `${API_ROUTE_PREFIX}/symbols.txt`;

/**
 * How each item kind is named, in the order module pages and the sidebar list kinds:
 * `title` heads a module page section, `name` starts a sentence, and `label` is the
 * Rust keyword shown on chips.
 */
export const KINDS: Record<ApiItem['kind'], { title: string; name: string; label: string }> = {
	module: { title: 'Modules', name: 'Module', label: 'mod' },
	struct: { title: 'Structs', name: 'Struct', label: 'struct' },
	enum: { title: 'Enums', name: 'Enum', label: 'enum' },
	union: { title: 'Unions', name: 'Union', label: 'union' },
	trait: { title: 'Traits', name: 'Trait', label: 'trait' },
	trait_alias: { title: 'Trait aliases', name: 'Trait alias', label: 'trait' },
	type_alias: { title: 'Type aliases', name: 'Type alias', label: 'type' },
	function: { title: 'Functions', name: 'Function', label: 'fn' },
	macro: { title: 'Macros', name: 'Macro', label: 'macro' },
	constant: { title: 'Constants', name: 'Constant', label: 'const' },
	static: { title: 'Statics', name: 'Static', label: 'static' },
};

/** The order of item kinds on module pages and in the sidebar. */
export const KIND_ORDER = Object.keys(KINDS) as ApiItem['kind'][];

/** Returns the items a module lists, in the order its page shows them. */
export function moduleChildren(model: ApiModel, module: ApiItem): { item: ApiItem; name: string; reexport: boolean }[] {
	return (module.children ?? [])
		.map((child) => ({ item: model.items[child.key], name: child.name, reexport: child.reexport }))
		.filter((child) => child.item !== undefined)
		.sort((left, right) => KIND_ORDER.indexOf(left.item.kind) - KIND_ORDER.indexOf(right.item.kind) || left.name.localeCompare(right.name));
}

/** Returns a one-sentence description for navigation, `llms.txt`, and search, even for items without docs. */
export function describe(item: ApiItem): string {
	if (item.summary) return item.summary;
	if (item.parent === null) return `The \`${item.name}\` crate root.`;
	return item.kind === 'module'
		? `The \`${item.path.join('::')}\` module.`
		: `The \`${item.name}\` ${KINDS[item.kind].label} in \`${item.path.slice(0, -1).join('::')}\`.`;
}

/** Returns declaration tokens as plain Rust source. */
export function tokenText(tokens: Token[]): string {
	return tokens.map((token) => token.text).join('');
}
