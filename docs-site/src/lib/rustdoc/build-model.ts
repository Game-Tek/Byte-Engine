/**
 * Converts rustdoc's unstable JSON into the stable {@link ApiModel} the site renders.
 *
 * `docs-site/scripts/build-rust-api-model.ts` runs it from `scripts/generate-api-docs.sh`.
 * This is the only module besides `format.ts` that reads rustdoc JSON.
 */
import {
	type Crate,
	type FunctionSignature,
	type GenericArgs,
	type GenericBound,
	type GenericParamDef,
	type Generics,
	type Id,
	type Impl,
	type Item,
	type Path,
	type Term,
	type Type,
	FormatDriftError,
	match,
	RUSTDOC_FORMAT_VERSION,
	RUSTDOC_TOOLCHAIN,
	tagOf,
	variantOf,
} from './format';
import {
	API_ROUTE_PREFIX,
	type ApiChild,
	type ApiExternalReexport,
	type ApiField,
	type ApiImplBlock,
	type ApiImplementor,
	type ApiItem,
	type ApiItemKind,
	type ApiMember,
	type ApiMemberKind,
	type ApiModel,
	type ApiReference,
	type ApiSignature,
	type ApiTraitImpl,
	type ApiVariant,
	type SourceLocation,
	type Token,
	type TokenKind,
} from './model';

const PAGE_KINDS = new Set<ApiItemKind>(['module', 'struct', 'enum', 'union', 'trait', 'trait_alias', 'type_alias']);
const STD_CRATES = new Set(['std', 'core', 'alloc']);
/** Unstable std traits that `#[derive]` adds as compiler plumbing; rustdoc's own pages hide them too. */
const HIDDEN_STD_TRAITS = new Set(['StructuralPartialEq', 'TrivialClone', 'UseCloned']);

/**
 * Builds the API model from a parsed rustdoc JSON crate.
 *
 * Throws when the format version differs from {@link RUSTDOC_FORMAT_VERSION}, and
 * {@link FormatDriftError} when the JSON holds a shape `format.ts` doesn't describe.
 * `onUnresolvedLinks` receives doc links whose targets have no public page; they render as plain code.
 */
export function buildModel(crate: Crate, onUnresolvedLinks?: (links: string[]) => void): ApiModel {
	if (crate.format_version !== RUSTDOC_FORMAT_VERSION) {
		throw new Error(
			`rustdoc JSON has format version ${crate.format_version}, but the API model reads version ${RUSTDOC_FORMAT_VERSION}. ` +
				`The toolchain in scripts/generate-api-docs.sh most likely changed. ` +
				`Pin TOOLCHAIN back to ${RUSTDOC_TOOLCHAIN}, or update RUSTDOC_FORMAT_VERSION and the types in docs-site/src/lib/rustdoc/format.ts.`,
		);
	}

	const builder = new ModelBuilder(crate);
	const model = builder.build();
	if (builder.unresolvedLinks.size > 0) onUnresolvedLinks?.([...builder.unresolvedLinks].sort());
	return model;
}

/** A public item found while walking the module tree, before its page content exists. */
interface Placement {
	item: Item;
	kind: ApiItemKind;
	path: string[];
	parent: string | null;
}

class ModelBuilder {
	private readonly crate: Crate;
	private readonly crateName: string;
	/** Canonical placement of every public local item, by rustdoc id. */
	private readonly placements = new Map<Id, Placement>();
	/** Children listed by each module, by module id. */
	private readonly moduleChildren = new Map<Id, { id: Id; name: string; reexport: boolean }[]>();
	private readonly moduleExternal = new Map<Id, ApiExternalReexport[]>();
	/** Site URLs of members (fields, variants, methods), by rustdoc id, so links can reach them. */
	private readonly memberHrefs = new Map<Id, string>();
	private readonly routes = new Map<Id, string>();
	private readonly anchors = new Map<Id, string>();
	private readonly acceptedBy = new Map<Id, ApiReference[]>();
	private readonly returnedBy = new Map<Id, ApiReference[]>();
	/** Doc links that rustdoc resolved to items without a public page, as `item: destination`. */
	readonly unresolvedLinks = new Set<string>();
	/** The key of the item being built, which names the owner of member docs in warnings. */
	private currentKey = '';

	constructor(crate: Crate) {
		this.crate = crate;
		const root = this.item(crate.root);
		if (!root?.name) throw new FormatDriftError('a root module without a name', 'the crate root');
		this.crateName = root.name;
	}

	build(): ApiModel {
		this.placeItems();
		this.assignRoutes();
		this.assignMemberAnchors();
		this.collectCrossReferences();

		const items: Record<string, ApiItem> = {};
		for (const placement of this.placements.values()) {
			const apiItem = this.buildItem(placement);
			items[apiItem.key] = apiItem;
		}

		return {
			crate: this.crateName,
			version: this.crate.crate_version,
			formatVersion: this.crate.format_version,
			root: this.crateName,
			items,
		};
	}

	private item(id: Id | null | undefined): Item | undefined {
		return id == null ? undefined : this.crate.index[String(id)];
	}

	/** Looks up items by id, skipping ids rustdoc stripped from the index. */
	private items(ids: (Id | null)[]): Item[] {
		return ids.flatMap((id) => this.item(id) ?? []);
	}

	private key(id: Id): string | undefined {
		return this.placements.get(id)?.path.join('::');
	}

	// ---------------------------------------------------------------------
	// Public paths

	/**
	 * Gives every public item one canonical path, in two breadth-first passes over the
	 * module tree. The first pass places items where they are defined, following only
	 * public modules that aren't `#[doc(hidden)]`. The second follows `pub use`
	 * re-exports: it lists already placed items as re-exports, and places items defined
	 * in hidden modules (such as `input::Action`) at their shortest re-export path.
	 * Items reachable only through hidden paths are left out.
	 */
	private placeItems() {
		const root = this.item(this.crate.root)!;
		this.placements.set(root.id, { item: root, kind: 'module', path: [this.crateName], parent: null });

		const modules: Id[] = [root.id];
		for (let index = 0; index < modules.length; index++) {
			const module = this.placements.get(modules[index])!;
			for (const entryId of moduleItems(module.item)) {
				const entry = this.item(entryId);
				if (!entry?.name || !isPublic(entry) || tagOf(entry.inner) === 'use') continue;
				this.placeChild(entry, entry.name, module, modules);
			}
		}

		// Re-exports can place new modules, which join the end of the same queue.
		for (let index = 0; index < modules.length; index++) {
			const module = this.placements.get(modules[index])!;
			for (const entryId of moduleItems(module.item)) {
				this.placeReexport(entryId, module, modules, new Set());
			}
		}
	}

	private placeReexport(entryId: Id, module: Placement, modules: Id[], globs: Set<Id>) {
		const entry = this.item(entryId);
		const use = entry && isPublic(entry) ? variantOf(entry.inner, 'use') : undefined;
		if (!use) return;
		const target = this.item(use.id);

		if (use.is_glob) {
			// Glob re-exports list every public item of the target module here.
			if (!target || use.id == null || globs.has(use.id)) return;
			globs.add(use.id);
			for (const id of moduleItems(target)) {
				const item = this.item(id);
				if (!item || !isPublic(item)) continue;
				if (tagOf(item.inner) === 'use') this.placeReexport(id, module, modules, globs);
				else if (item.name) this.placeChild(item, item.name, module, modules);
			}
			return;
		}

		if (target && target.crate_id === 0) {
			if (!isHidden(target)) this.placeChild(target, use.name, module, modules);
			return;
		}

		const external = this.moduleExternal.get(module.item.id) ?? [];
		external.push({ name: use.name, source: use.source, href: this.externalHref(use.id) });
		this.moduleExternal.set(module.item.id, external);
	}

	private placeChild(item: Item, name: string, module: Placement, modules: Id[]) {
		const kind = itemKind(item);
		if (!kind) return;

		const children = this.moduleChildren.get(module.item.id) ?? [];
		if (children.some((child) => child.id === item.id && child.name === name)) return;

		const existing = this.placements.get(item.id);
		// A `pub use` that gives an item its canonical path is its home, not a re-export.
		children.push({ id: item.id, name, reexport: existing !== undefined });
		this.moduleChildren.set(module.item.id, children);
		if (existing) return;

		this.placements.set(item.id, {
			item,
			kind,
			path: [...module.path, name],
			parent: module.path.join('::'),
		});
		if (kind === 'module') modules.push(item.id);
	}

	/**
	 * Gives every placed item its URL. Page items get a path segment; other items an
	 * anchor on their module page. Siblings whose names differ only in case get a kind
	 * suffix so prerendered files can't collide on case-insensitive file systems.
	 */
	private assignRoutes() {
		const bySibling = new Map<string, Placement[]>();
		for (const placement of this.placements.values()) {
			if (!PAGE_KINDS.has(placement.kind) || placement.parent === null) continue;
			const group = bySibling.get(placement.parent) ?? [];
			group.push(placement);
			bySibling.set(placement.parent, group);
		}

		const segments = new Map<Id, string>();
		for (const group of bySibling.values()) {
			const counts = new Map<string, number>();
			for (const placement of group) {
				const lower = placement.path.at(-1)!.toLowerCase();
				counts.set(lower, (counts.get(lower) ?? 0) + 1);
			}
			for (const placement of group) {
				const name = placement.path.at(-1)!;
				const collides = (counts.get(name.toLowerCase()) ?? 0) > 1 && placement.kind !== 'module';
				segments.set(placement.item.id, collides ? `${name}-${placement.kind}` : name);
			}
		}

		// Placements are inserted breadth first, so a parent's route exists before its children's.
		const routeByKey = new Map<string, string>();
		for (const placement of this.placements.values()) {
			let route: string;
			if (placement.parent === null) {
				route = `${API_ROUTE_PREFIX}/${this.crateName}`;
			} else if (PAGE_KINDS.has(placement.kind)) {
				route = `${routeByKey.get(placement.parent)}/${segments.get(placement.item.id)}`;
			} else {
				const anchor = `${placement.kind === 'function' ? 'fn' : placement.kind}.${placement.path.at(-1)}`;
				this.anchors.set(placement.item.id, anchor);
				route = `${routeByKey.get(placement.parent)}#${anchor}`;
			}
			routeByKey.set(placement.path.join('::'), route);
			this.routes.set(placement.item.id, route);
		}
	}

	/** Records the URL of every field, variant, and member so docs and signatures can link to them. */
	private assignMemberAnchors() {
		for (const { item, kind } of this.placements.values()) {
			const route = this.routes.get(item.id)!;
			if (kind === 'struct' || kind === 'union') {
				for (const field of this.fieldItems(item)) {
					this.memberHrefs.set(field.item.id, `${route}#structfield.${field.name}`);
				}
			}
			for (const variant of this.items(variantOf(item.inner, 'enum')?.variants ?? [])) {
				this.memberHrefs.set(variant.id, `${route}#variant.${variant.name}`);
			}
			const trait = variantOf(item.inner, 'trait');
			if (trait) {
				for (const member of this.items(trait.items)) {
					const anchor = traitMemberAnchor(member);
					if (anchor) this.memberHrefs.set(member.id, `${route}#${anchor}`);
				}
			}
			for (const block of this.inherentImpls(item)) {
				const used = new Set<string>();
				for (const member of block.members) {
					const anchor = uniqueAnchor(memberAnchor(member)!, used);
					this.memberHrefs.set(member.id, `${route}#${anchor}`);
				}
			}
		}
	}

	// ---------------------------------------------------------------------
	// Items

	private buildItem(placement: Placement): ApiItem {
		const { item, kind, path, parent } = placement;
		this.currentKey = path.join('::');
		const docs = this.docs(item);
		const apiItem: ApiItem = {
			key: path.join('::'),
			kind,
			name: path.at(-1)!,
			path,
			route: this.routes.get(item.id)!,
			page: PAGE_KINDS.has(kind),
			parent,
			summary: summarize(docs),
			docs,
			declaration: this.declaration(item, path.at(-1)!),
			source: sourceLocation(item),
			deprecation: item.deprecation,
		};
		const signature = this.signature(item);
		if (signature) apiItem.signature = signature;
		const anchor = this.anchors.get(item.id);
		if (anchor) apiItem.anchor = anchor;

		// `itemKind` already rejected unknown item shapes, so the kind decides what the page lists.
		switch (kind) {
			case 'module': {
				apiItem.children = (this.moduleChildren.get(item.id) ?? []).flatMap((child): ApiChild[] => {
					const key = this.key(child.id);
					return key ? [{ key, name: child.name, reexport: child.reexport }] : [];
				});
				const external = this.moduleExternal.get(item.id);
				if (external?.length) apiItem.externalReexports = external;
				break;
			}
			case 'struct':
			case 'union':
				apiItem.fields = this.fields(item);
				this.addImpls(apiItem, item);
				break;
			case 'enum':
				apiItem.variants = this.items(variantOf(item.inner, 'enum')!.variants)
					.filter((variant) => !isHidden(variant))
					.map((variant) => this.variant(variant));
				this.addImpls(apiItem, item);
				break;
			case 'trait': {
				const trait = variantOf(item.inner, 'trait')!;
				const members = this.items(trait.items).filter((member) => !isHidden(member));
				const member = (member: Item) => this.member(member, traitMemberAnchor(member)!);
				apiItem.required = members.filter(isRequired).map(member);
				apiItem.provided = members.filter((entry) => !isRequired(entry)).map(member);
				apiItem.implementors = this.items(trait.implementations).map((implItem) => this.implementor(implItem));
				break;
			}
			case 'type_alias':
				this.addImpls(apiItem, item);
				break;
		}

		const accepted = this.acceptedBy.get(item.id);
		if (accepted?.length) apiItem.acceptedBy = accepted;
		const returned = this.returnedBy.get(item.id);
		if (returned?.length) apiItem.returnedBy = returned;

		return apiItem;
	}

	/** Returns the documented fields of a struct, union, or struct-like variant. */
	private fieldItems(item: Item): { name: string; item: Item }[] {
		// `declaration` matches every struct and variant kind, so a new kind fails there.
		const structure = variantOf(item.inner, 'struct');
		const variant = variantOf(item.inner, 'variant');
		const fieldIds: (Id | null)[] = structure
			? (variantOf(structure.kind, 'tuple') ?? variantOf(structure.kind, 'plain')?.fields ?? [])
			: variant
				? (variantOf(variant.kind, 'tuple') ?? variantOf(variant.kind, 'struct')?.fields ?? [])
				: (variantOf(item.inner, 'union')?.fields ?? []);

		// Variant fields have no visibility of their own; they are as public as the enum.
		return this.items(fieldIds)
			.filter((field) => (field.visibility === 'public' || variant !== undefined) && !isHidden(field))
			.map((field) => ({ name: field.name ?? '', item: field }));
	}

	private fields(item: Item, anchorBase = 'structfield'): ApiField[] {
		return this.fieldItems(item).map(({ name, item: field }) => {
			const docs = this.docs(field);
			return {
				name,
				anchor: `${anchorBase}.${name}`,
				type: this.typeTokens(fieldType(field)),
				summary: summarize(docs),
				docs,
			};
		});
	}

	private variant(variant: Item): ApiVariant {
		const inner = variantOf(variant.inner, 'variant')!;
		const name = variant.name!;
		const out = new TokenWriter();
		out.push(name, 'name');
		match(
			inner.kind,
			{
				plain: () => {},
				tuple: (fields) => {
					out.punct('(');
					out.list(fields, ', ', (id) => {
						const field = this.item(id);
						if (field) this.writeType(fieldType(field), out);
						else out.punct('_');
					});
					out.punct(')');
				},
				struct: ({ fields }) => {
					out.punct(' { ');
					out.list(this.items(fields), ', ', (field) => {
						out.push(field.name ?? '', 'name');
						out.punct(': ');
						this.writeType(fieldType(field), out);
					});
					out.punct(' }');
				},
			},
			`variant ${name}`,
		);
		if (inner.discriminant) {
			out.punct(' = ');
			out.push(inner.discriminant.expr, 'literal');
		}

		const docs = this.docs(variant);
		return {
			name,
			anchor: `variant.${name}`,
			declaration: out.tokens,
			summary: summarize(docs),
			docs,
			fields: tagOf(inner.kind) === 'struct' ? this.fields(variant, `variant.${name}.field`) : [],
		};
	}

	// ---------------------------------------------------------------------
	// Implementations

	private implItems(item: Item): { item: Item; impl: Impl }[] {
		const ids =
			variantOf(item.inner, 'struct')?.impls ?? variantOf(item.inner, 'enum')?.impls ?? variantOf(item.inner, 'union')?.impls ?? [];

		// Auto traits and blanket implementations apply to almost every type, so they
		// only add noise to a type's page.
		return this.items(ids)
			.map((implItem) => ({ item: implItem, impl: variantOf(implItem.inner, 'impl')! }))
			.filter(({ impl }) => !impl.is_synthetic && !impl.blanket_impl && !impl.is_negative);
	}

	private inherentImpls(item: Item): { impl: Impl; members: Item[] }[] {
		return this.implItems(item)
			.filter(({ impl }) => impl.trait === null)
			.map(({ impl }) => ({
				impl,
				members: this.items(impl.items).filter(
					(member) => member.visibility === 'public' && !isHidden(member) && memberAnchor(member) !== undefined,
				),
			}))
			.filter((block) => block.members.length > 0);
	}

	private addImpls(apiItem: ApiItem, item: Item) {
		const inherent: ApiImplBlock[] = this.inherentImpls(item).map(({ impl, members }) => ({
			header: this.implHeader(impl),
			members: members.map((member) => this.member(member, this.memberHrefs.get(member.id)!.split('#')[1])),
		}));
		if (inherent.length) apiItem.inherent = inherent;

		const traitImpls: ApiTraitImpl[] = this.implItems(item)
			.filter(({ impl }) => impl.trait !== null)
			.map((entry) => this.traitImpl(entry.item, entry.impl))
			.filter((traitImpl) => !(traitImpl.origin === 'std' && HIDDEN_STD_TRAITS.has(traitImpl.traitName)));
		if (traitImpls.length) apiItem.traitImpls = traitImpls;
	}

	private traitImpl(implItem: Item, impl: Impl): ApiTraitImpl {
		const trait = impl.trait!;
		const origin = this.traitOrigin(trait.id);
		// Derived implementations carry `#[automatically_derived]` on the `impl` item.
		const derived = implItem.attrs.some((attribute) => attribute === 'automatically_derived');
		const members =
			origin === 'engine'
				? this.items(impl.items)
						.filter((member) => memberAnchor(member) !== undefined)
						.map((member) => this.member(member, `impl-${lastSegment(trait.path)}.${memberAnchor(member)}`))
				: [];
		return {
			header: this.implHeader(impl),
			traitName: lastSegment(trait.path),
			traitHref: this.href(trait.id),
			origin,
			derived,
			members,
		};
	}

	private traitOrigin(traitId: Id): ApiTraitImpl['origin'] {
		const local = this.item(traitId);
		if (local && local.crate_id === 0) return 'engine';
		const summary = this.crate.paths[String(traitId)];
		if (!summary) return 'external';
		if (summary.crate_id === 0) return 'engine';
		const crateName = this.crate.external_crates[String(summary.crate_id)]?.name;
		return crateName && STD_CRATES.has(crateName) ? 'std' : 'external';
	}

	private implementor(implItem: Item): ApiImplementor {
		const impl = variantOf(implItem.inner, 'impl')!;
		const target = variantOf(impl.for, 'resolved_path');
		return {
			header: this.implHeader(impl),
			href: target ? this.href(target.id) : undefined,
		};
	}

	private member(member: Item, anchor: string): ApiMember {
		const docs = this.docs(member);
		const apiMember: ApiMember = {
			name: member.name ?? '',
			kind: memberKind(member)!,
			anchor,
			declaration: this.declaration(member, member.name ?? ''),
			summary: summarize(docs),
			docs,
			source: sourceLocation(member),
			deprecation: member.deprecation,
		};
		const signature = this.signature(member);
		if (signature) apiMember.signature = signature;
		return apiMember;
	}

	/** Splits a function signature into parameter rows, without the `self` receiver. */
	private signature(item: Item): ApiSignature | undefined {
		const fn = variantOf(item.inner, 'function');
		if (!fn) return undefined;
		return {
			parameters: fn.sig.inputs
				.filter(([name]) => name !== 'self')
				.map(([name, type]) => ({ name, type: this.typeTokens(type) })),
			returns: fn.sig.output ? this.typeTokens(fn.sig.output) : null,
		};
	}

	// ---------------------------------------------------------------------
	// Cross references

	/** Indexes which public functions and methods take or return each type. */
	private collectCrossReferences() {
		const visit = (signature: FunctionSignature, reference: ApiReference, ownerId: Id | undefined) => {
			const record = (map: Map<Id, ApiReference[]>, type: Type) => {
				for (const id of referencedIds(type)) {
					if (id === ownerId || !this.placements.has(id)) continue;
					const list = map.get(id) ?? [];
					if (!list.some((entry) => entry.href === reference.href)) list.push(reference);
					map.set(id, list);
				}
			};
			for (const [name, type] of signature.inputs) {
				if (name !== 'self') record(this.acceptedBy, type);
			}
			if (signature.output) record(this.returnedBy, signature.output);
		};

		for (const { item, kind, path } of this.placements.values()) {
			this.currentKey = path.join('::');
			if (kind === 'function') {
				visit(variantOf(item.inner, 'function')!.sig, { label: path.at(-1)!, href: this.routes.get(item.id)!, summary: summarize(this.docs(item)) }, undefined);
			}
			for (const { members } of this.inherentImpls(item)) {
				for (const member of members) {
					const fn = variantOf(member.inner, 'function');
					if (!fn) continue;
					visit(
						fn.sig,
						{
							label: `${path.at(-1)}::${member.name}`,
							href: this.memberHrefs.get(member.id)!,
							summary: summarize(this.docs(member)),
						},
						item.id,
					);
				}
			}
		}
	}

	// ---------------------------------------------------------------------
	// Links

	/** Returns the URL of any rustdoc id: a page, a member anchor, or an external rustdoc page. */
	private href(id: Id): string | undefined {
		return this.routes.get(id) ?? this.memberHrefs.get(id) ?? this.externalHref(id);
	}

	private externalHref(id: Id | null): string | undefined {
		if (id == null) return undefined;
		const summary = this.crate.paths[String(id)];
		if (!summary || summary.crate_id === 0) return undefined;
		const root = this.crate.external_crates[String(summary.crate_id)]?.html_root_url;
		if (!root) return undefined;
		return `${root.replace(/\/?$/, '/')}${rustdocPagePath(summary.kind, summary.path)}`;
	}

	/**
	 * Returns the doc comment with rustdoc conventions turned into portable Markdown:
	 * intra-doc links point at site or external URLs, and code fences name a language
	 * Shiki knows, with rustdoc's hidden `# ` lines removed.
	 */
	private docs(item: Item): string {
		if (!item.docs) return '';
		const resolve = (destination: string) => {
			const id = item.links[destination];
			if (id === undefined) return undefined;
			const href = this.href(id);
			if (href) return href;
			const owner = this.placements.get(item.id)?.path.join('::') ?? `${this.currentKey}::${item.name}`;
			this.unresolvedLinks.add(`${owner}: ${destination}`);
			return null;
		};
		return normalizeDocs(item.docs, resolve);
	}

	// ---------------------------------------------------------------------
	// Declarations

	private declaration(item: Item, name: string): Token[] {
		const out = new TokenWriter();
		const visibility = item.visibility === 'public' ? 'pub ' : '';
		match(
			item.inner,
			{
				module: () => {
					out.push(`${visibility}mod `, 'keyword');
					out.push(name, 'name');
				},
				struct: (structure) => {
					out.push(`${visibility}struct `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(structure.generics, out);
					match(
						structure.kind,
						{
							unit: () => {
								this.writeWhere(structure.generics, out);
								out.punct(';');
							},
							tuple: (fields) => {
								out.punct('(');
								out.list(fields, ', ', (id) => {
									const field = this.item(id);
									if (field && field.visibility === 'public' && !isHidden(field)) {
										out.push('pub ', 'keyword');
										this.writeType(fieldType(field), out);
									} else {
										out.punct('_');
									}
								});
								out.punct(')');
								this.writeWhere(structure.generics, out);
								out.punct(';');
							},
							plain: ({ fields, has_stripped_fields }) => {
								this.writeWhere(structure.generics, out);
								this.writeFieldBlock(fields, has_stripped_fields, out);
							},
						},
						`declaration of ${name}`,
					);
				},
				union: (union) => {
					out.push(`${visibility}union `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(union.generics, out);
					this.writeWhere(union.generics, out);
					this.writeFieldBlock(union.fields, union.has_stripped_fields, out);
				},
				enum: (enumeration) => {
					out.push(`${visibility}enum `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(enumeration.generics, out);
					this.writeWhere(enumeration.generics, out);
					const variants = this.items(enumeration.variants).filter((variant) => !isHidden(variant));
					if (variants.length === 0 && !enumeration.has_stripped_variants) {
						out.punct(' {}');
						return;
					}
					out.punct(' {\n');
					for (const variant of variants) {
						out.punct('    ');
						out.append(this.variant(variant).declaration);
						out.punct(',\n');
					}
					if (enumeration.has_stripped_variants) out.punct('    // some variants omitted\n');
					out.punct('}');
				},
				trait: (trait) => {
					out.push(`${visibility}${trait.is_unsafe ? 'unsafe ' : ''}trait `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(trait.generics, out);
					if (trait.bounds.length) {
						out.punct(': ');
						this.writeBounds(trait.bounds, out);
					}
					this.writeWhere(trait.generics, out);
					out.punct(' { … }');
				},
				trait_alias: (alias) => {
					out.push(`${visibility}trait `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(alias.generics, out);
					out.punct(' = ');
					this.writeBounds(alias.params, out);
					this.writeWhere(alias.generics, out);
					out.punct(';');
				},
				type_alias: (alias) => {
					out.push(`${visibility}type `, 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(alias.generics, out);
					this.writeWhere(alias.generics, out);
					out.punct(' = ');
					this.writeType(alias.type, out);
					out.punct(';');
				},
				function: (fn) => {
					const header = fn.header;
					out.push(
						`${visibility}${header.is_const ? 'const ' : ''}${header.is_async ? 'async ' : ''}${header.is_unsafe ? 'unsafe ' : ''}fn `,
						'keyword',
					);
					out.push(name, 'fn');
					this.writeGenericParams(fn.generics, out);
					this.writeSignature(fn.sig, out);
					this.writeWhere(fn.generics, out);
				},
				constant: (constant) => {
					out.push(`${visibility}const `, 'keyword');
					out.push(name, 'name');
					out.punct(': ');
					this.writeType(constant.type, out);
					const value = constant.const.value ?? constant.const.expr;
					if (value && value !== '_') {
						out.punct(' = ');
						out.push(value, 'literal');
					}
					out.punct(';');
				},
				static: (value) => {
					out.push(`${visibility}static ${value.is_mutable ? 'mut ' : ''}`, 'keyword');
					out.push(name, 'name');
					out.punct(': ');
					this.writeType(value.type, out);
					out.punct(';');
				},
				macro: (source) => {
					out.push(source);
				},
				proc_macro: () => {
					out.push(name, 'fn');
				},
				assoc_const: (constant) => {
					out.push('const ', 'keyword');
					out.push(name, 'name');
					out.punct(': ');
					this.writeType(constant.type, out);
					if (constant.value && constant.value !== '_') {
						out.punct(' = ');
						out.push(constant.value, 'literal');
					}
				},
				assoc_type: (assoc) => {
					out.push('type ', 'keyword');
					out.push(name, 'type');
					this.writeGenericParams(assoc.generics, out);
					if (assoc.bounds.length) {
						out.punct(': ');
						this.writeBounds(assoc.bounds, out);
					}
					if (assoc.type) {
						out.punct(' = ');
						this.writeType(assoc.type, out);
					}
				},
				extern_crate: () => out.push(name),
				use: () => out.push(name),
				struct_field: (type) => {
					out.push(name, 'name');
					out.punct(': ');
					this.writeType(type, out);
				},
				variant: () => out.append(this.variant(item).declaration),
				impl: (impl) => out.append(this.implHeader(impl)),
				extern_type: () => out.push(name, 'type'),
				primitive: () => out.push(name, 'primitive'),
			},
			`declaration of ${name}`,
		);
		return out.tokens;
	}

	private writeFieldBlock(fieldIds: Id[], hasStrippedFields: boolean, out: TokenWriter) {
		const fields = this.items(fieldIds).filter((field) => !isHidden(field));
		if (fields.length === 0 && !hasStrippedFields) {
			out.punct(' {}');
			return;
		}
		if (fields.length === 0) {
			out.punct(' { /* private fields */ }');
			return;
		}
		out.punct(' {\n');
		for (const field of fields) {
			out.push('    pub ', 'keyword');
			out.push(field.name ?? '', 'name');
			out.punct(': ');
			this.writeType(fieldType(field), out);
			out.punct(',\n');
		}
		if (hasStrippedFields) out.punct('    /* private fields */\n');
		out.punct('}');
	}

	private implHeader(impl: Impl): Token[] {
		const out = new TokenWriter();
		out.push(`${impl.is_unsafe ? 'unsafe ' : ''}impl`, 'keyword');
		this.writeGenericParams(impl.generics, out);
		out.punct(' ');
		if (impl.trait) {
			if (impl.is_negative) out.punct('!');
			this.writePath(impl.trait, out, 'type');
			out.push(' for ', 'keyword');
		}
		this.writeType(impl.for, out);
		this.writeWhere(impl.generics, out);
		return out.tokens;
	}

	private writeSignature(signature: FunctionSignature, out: TokenWriter) {
		out.punct('(');
		out.list(signature.inputs, ', ', ([name, type]) => {
			if (name === 'self') return this.writeSelfReceiver(type, out);
			out.push(name, 'name');
			out.punct(': ');
			this.writeType(type, out);
		});
		if (signature.is_c_variadic) out.punct(', ...');
		out.punct(')');
		if (signature.output) {
			out.punct(' -> ');
			this.writeType(signature.output, out);
		}
	}

	/** Writes `self`, `&self`, and `&mut self` the way they are written in source. */
	private writeSelfReceiver(type: Type, out: TokenWriter) {
		const reference = variantOf(type, 'borrowed_ref');
		// Only `Self`, `&Self`, and `&mut Self` have a shorthand; `self: Box<Self>` stays explicit.
		if (variantOf(reference?.type ?? type, 'generic') !== 'Self') {
			out.push('self', 'keyword');
			out.punct(': ');
			this.writeType(type, out);
			return;
		}
		if (reference) {
			out.punct('&');
			if (reference.lifetime) out.push(`${reference.lifetime} `, 'lifetime');
			if (reference.is_mutable) out.push('mut ', 'keyword');
		}
		out.push('self', 'keyword');
	}

	private writeGenericParams(generics: Generics, out: TokenWriter) {
		const params = generics.params.filter((param) => !isSyntheticParam(param));
		if (params.length === 0) return;
		out.punct('<');
		out.list(params, ', ', (param) => this.writeGenericParam(param, out));
		out.punct('>');
	}

	private writeGenericParam(param: GenericParamDef, out: TokenWriter) {
		match(
			param.kind,
			{
				lifetime: ({ outlives }) => {
					out.push(param.name, 'lifetime');
					if (outlives.length) {
						out.punct(': ');
						out.push(outlives.join(' + '), 'lifetime');
					}
				},
				type: ({ bounds, default: defaultType }) => {
					out.push(param.name, 'generic');
					if (bounds.length) {
						out.punct(': ');
						this.writeBounds(bounds, out);
					}
					if (defaultType) {
						out.punct(' = ');
						this.writeType(defaultType, out);
					}
				},
				const: ({ type, default: defaultValue }) => {
					out.push('const ', 'keyword');
					out.push(param.name, 'generic');
					out.punct(': ');
					this.writeType(type, out);
					if (defaultValue) {
						out.punct(' = ');
						out.push(defaultValue, 'literal');
					}
				},
			},
			`generic parameter ${param.name}`,
		);
	}

	private writeWhere(generics: Generics, out: TokenWriter) {
		if (generics.where_predicates.length === 0) return;
		out.push('\nwhere\n', 'keyword');
		out.list(generics.where_predicates, ',\n', (predicate) => {
			out.punct('    ');
			match(
				predicate,
				{
					bound_predicate: ({ type, bounds, generic_params }) => {
						this.writeHigherRanked(generic_params, out);
						this.writeType(type, out);
						out.punct(': ');
						this.writeBounds(bounds, out);
					},
					lifetime_predicate: ({ lifetime, outlives }) => {
						out.push(lifetime, 'lifetime');
						out.punct(': ');
						out.push(outlives.join(' + '), 'lifetime');
					},
					eq_predicate: ({ lhs, rhs }) => {
						this.writeType(lhs, out);
						out.punct(' = ');
						this.writeTerm(rhs, out);
					},
				},
				'where predicate',
			);
		});
	}

	private writeBounds(bounds: GenericBound[], out: TokenWriter) {
		out.list(bounds, ' + ', (bound) =>
			match(
				bound,
				{
					trait_bound: ({ trait, generic_params, modifier }) => {
						this.writeHigherRanked(generic_params, out);
						if (modifier === 'maybe') out.punct('?');
						if (modifier === 'maybe_const') out.push('~const ', 'keyword');
						this.writePath(trait, out, 'type');
					},
					outlives: (lifetime) => out.push(lifetime, 'lifetime'),
					use: (args) => {
						out.push('use', 'keyword');
						out.punct('<');
						out.list(args, ', ', (arg) =>
							match(
								arg,
								{
									lifetime: (lifetime) => out.push(lifetime, 'lifetime'),
									param: (param) => out.push(param, 'generic'),
								},
								'precise capturing argument',
							),
						);
						out.punct('>');
					},
				},
				'generic bound',
			),
		);
	}

	/** Writes the `for<'a>` binder of a higher-ranked bound or predicate. */
	private writeHigherRanked(params: GenericParamDef[], out: TokenWriter) {
		if (params.length === 0) return;
		out.push('for', 'keyword');
		out.punct('<');
		out.list(params, ', ', (param) => this.writeGenericParam(param, out));
		out.punct('> ');
	}

	private writeTerm(term: Term, out: TokenWriter) {
		match(
			term,
			{
				type: (type) => this.writeType(type, out),
				constant: (constant) => out.push(constant.expr, 'literal'),
			},
			'term',
		);
	}

	private writePath(path: Path, out: TokenWriter, kind: TokenKind) {
		out.push(lastSegment(path.path), kind, this.href(path.id));
		if (path.args) this.writeGenericArgs(path.args, out);
	}

	private writeGenericArgs(args: GenericArgs, out: TokenWriter) {
		match(
			args,
			{
				angle_bracketed: ({ args: list, constraints }) => {
					if (list.length === 0 && constraints.length === 0) return;
					out.punct('<');
					out.list(list, ', ', (arg) =>
						match(
							arg,
							{
								lifetime: (lifetime) => out.push(lifetime, 'lifetime'),
								type: (type) => this.writeType(type, out),
								const: (constant) => out.push(constant.expr, 'literal'),
								infer: () => out.punct('_'),
							},
							'generic argument',
						),
					);
					if (list.length > 0 && constraints.length > 0) out.punct(', ');
					out.list(constraints, ', ', (constraint) => {
						out.push(constraint.name, 'type');
						if (constraint.args) this.writeGenericArgs(constraint.args, out);
						match(
							constraint.binding,
							{
								equality: (term) => {
									out.punct(' = ');
									this.writeTerm(term, out);
								},
								constraint: (bounds) => {
									out.punct(': ');
									this.writeBounds(bounds, out);
								},
							},
							'associated item constraint',
						);
					});
					out.punct('>');
				},
				parenthesized: ({ inputs, output }) => {
					out.punct('(');
					out.list(inputs, ', ', (type) => this.writeType(type, out));
					out.punct(')');
					if (output) {
						out.punct(' -> ');
						this.writeType(output, out);
					}
				},
				return_type_notation: () => out.punct('(..)'),
			},
			'generic arguments',
		);
	}

	private typeTokens(type: Type): Token[] {
		const out = new TokenWriter();
		this.writeType(type, out);
		return out.tokens;
	}

	private writeType(type: Type, out: TokenWriter) {
		match(
			type,
			{
				resolved_path: (path) => this.writePath(path, out, 'type'),
				dyn_trait: ({ traits, lifetime }) => {
					out.push('dyn ', 'keyword');
					out.list(traits, ' + ', ({ trait }) => this.writePath(trait, out, 'type'));
					if (lifetime) {
						out.punct(' + ');
						out.push(lifetime, 'lifetime');
					}
				},
				generic: (name) => out.push(name, 'generic'),
				primitive: (name) => out.push(name, 'primitive', primitiveHref(name)),
				function_pointer: (pointer) => {
					out.push(`${pointer.header.is_unsafe ? 'unsafe ' : ''}fn`, 'keyword');
					this.writeSignature(pointer.sig, out);
				},
				tuple: (types) => {
					out.punct('(');
					out.list(types, ', ', (inner) => this.writeType(inner, out));
					if (types.length === 1) out.punct(',');
					out.punct(')');
				},
				slice: (inner) => {
					out.punct('[');
					this.writeType(inner, out);
					out.punct(']');
				},
				array: ({ type: inner, len }) => {
					out.punct('[');
					this.writeType(inner, out);
					out.punct('; ');
					out.push(len, 'literal');
					out.punct(']');
				},
				pat: ({ type: inner }) => this.writeType(inner, out),
				impl_trait: (bounds) => {
					out.push('impl ', 'keyword');
					this.writeBounds(bounds, out);
				},
				infer: () => out.punct('_'),
				raw_pointer: ({ is_mutable, type: inner }) => {
					out.punct('*');
					out.push(is_mutable ? 'mut ' : 'const ', 'keyword');
					this.writeType(inner, out);
				},
				borrowed_ref: ({ lifetime, is_mutable, type: inner }) => {
					out.punct('&');
					if (lifetime) out.push(`${lifetime} `, 'lifetime');
					if (is_mutable) out.push('mut ', 'keyword');
					this.writeType(inner, out);
				},
				qualified_path: ({ name, args, self_type, trait }) => {
					if (trait && trait.path !== '') {
						out.punct('<');
						this.writeType(self_type, out);
						out.push(' as ', 'keyword');
						this.writePath(trait, out, 'type');
						out.punct('>');
					} else {
						this.writeType(self_type, out);
					}
					out.punct('::');
					out.push(name, 'type');
					if (args) this.writeGenericArgs(args, out);
				},
			},
			'type',
		);
	}
}

/**
 * The `TokenWriter` class accumulates declaration tokens, merging neighbors with the
 * same role so the model stays small.
 */
class TokenWriter {
	readonly tokens: Token[] = [];

	push(text: string, kind?: TokenKind, href?: string) {
		if (text === '') return;
		const last = this.tokens.at(-1);
		if (last && !last.href && !href && last.kind === kind) {
			last.text += text;
			return;
		}
		const token: Token = { text };
		if (kind) token.kind = kind;
		if (href) token.href = href;
		this.tokens.push(token);
	}

	punct(text: string) {
		this.push(text, 'punct');
	}

	/** Writes each item with `write`, with `separator` between them. */
	list<T>(items: readonly T[], separator: string, write: (item: T) => void) {
		items.forEach((item, index) => {
			if (index > 0) this.punct(separator);
			write(item);
		});
	}

	append(tokens: Token[]) {
		for (const token of tokens) this.push(token.text, token.kind, token.href);
	}
}

// -------------------------------------------------------------------------
// Helpers over raw items

function isPublic(item: Item): boolean {
	return item.visibility === 'public' && !isHidden(item);
}

function isHidden(item: Item): boolean {
	return item.attrs.some((attribute) => typeof attribute !== 'string' && 'other' in attribute && /^#\[doc\(hidden\)\]$/.test(attribute.other));
}

function moduleItems(item: Item): Id[] {
	return variantOf(item.inner, 'module')?.items ?? [];
}

/** Maps a raw item to an API item kind, or `undefined` for kinds the API section doesn't list. */
function itemKind(item: Item): ApiItemKind | undefined {
	return match<typeof item.inner, ApiItemKind | undefined>(
		item.inner,
		{
			module: () => 'module',
			struct: () => 'struct',
			enum: () => 'enum',
			union: () => 'union',
			trait: () => 'trait',
			trait_alias: () => 'trait_alias',
			type_alias: () => 'type_alias',
			function: () => 'function',
			constant: () => 'constant',
			static: () => 'static',
			macro: () => 'macro',
			proc_macro: () => 'macro',
			extern_crate: () => undefined,
			use: () => undefined,
			struct_field: () => undefined,
			variant: () => undefined,
			impl: () => undefined,
			extern_type: () => undefined,
			primitive: () => undefined,
			assoc_const: () => undefined,
			assoc_type: () => undefined,
		},
		`kind of ${item.name ?? item.id}`,
	);
}

const MEMBER_KINDS: Partial<Record<string, ApiMemberKind>> = {
	function: 'method',
	assoc_const: 'assoc_const',
	assoc_type: 'assoc_type',
};

/** rustdoc's anchor prefix for each member kind. */
const MEMBER_ANCHOR_PREFIXES: Record<ApiMemberKind, string> = {
	method: 'method',
	assoc_const: 'associatedconstant',
	assoc_type: 'associatedtype',
};

function memberKind(member: Item): ApiMemberKind | undefined {
	return MEMBER_KINDS[tagOf(member.inner)];
}

/** The rustdoc-style anchor of an inherent or trait implementation member. */
function memberAnchor(member: Item): string | undefined {
	const kind = memberKind(member);
	return kind && `${MEMBER_ANCHOR_PREFIXES[kind]}.${member.name}`;
}

/** Trait members an implementation must define use rustdoc's `tymethod.` anchor. */
function traitMemberAnchor(member: Item): string | undefined {
	const anchor = memberAnchor(member);
	return anchor && isRequired(member) && memberKind(member) === 'method' ? anchor.replace(/^method\./, 'tymethod.') : anchor;
}

/** Whether a trait member has no default, so every implementation must define it. */
function isRequired(member: Item): boolean {
	const fn = variantOf(member.inner, 'function');
	if (fn) return !fn.has_body;
	const constant = variantOf(member.inner, 'assoc_const');
	if (constant) return constant.value === null;
	return variantOf(member.inner, 'assoc_type')?.type === null;
}

function uniqueAnchor(anchor: string, used: Set<string>): string {
	let candidate = anchor;
	for (let index = 1; used.has(candidate); index++) candidate = `${anchor}-${index}`;
	used.add(candidate);
	return candidate;
}

/** rustdoc's file name prefix for each item kind whose prefix differs from the kind. */
const RUSTDOC_PAGE_PREFIXES: Partial<Record<string, string>> = {
	function: 'fn',
	type_alias: 'type',
	trait_alias: 'traitalias',
	proc_attribute: 'attr',
	proc_derive: 'derive',
};

/**
 * Returns the page of an item in rustdoc's HTML output relative to the docs root,
 * such as `byte_engine/time/struct.MediaTime.html`. `kind` is a rustdoc item kind,
 * which every {@link ApiItemKind} is.
 */
export function rustdocPagePath(kind: string, path: string[]): string {
	if (kind === 'module') return `${path.join('/')}/index.html`;
	return `${path.slice(0, -1).join('/')}/${RUSTDOC_PAGE_PREFIXES[kind] ?? kind}.${path.at(-1)}.html`;
}

function primitiveHref(name: string): string {
	return `https://doc.rust-lang.org/std/primitive.${name}.html`;
}

function fieldType(field: Item): Type {
	const type = variantOf(field.inner, 'struct_field');
	if (!type) throw new FormatDriftError(`a field without a type (${tagOf(field.inner)})`, field.name ?? String(field.id));
	return type;
}

function isSyntheticParam(param: GenericParamDef): boolean {
	return variantOf(param.kind, 'type')?.is_synthetic === true;
}

function lastSegment(path: string): string {
	return path.split('::').at(-1) ?? path;
}

function sourceLocation(item: Item): SourceLocation | null {
	return item.span ? { file: item.span.filename, line: item.span.begin[0] } : null;
}

/** Returns every local or external item id a type mentions, at any depth. */
function referencedIds(type: Type): Id[] {
	const ids: Id[] = [];
	const visit = (value: unknown) => {
		if (Array.isArray(value)) {
			for (const entry of value) visit(entry);
			return;
		}
		if (!value || typeof value !== 'object') return;
		const record = value as Record<string, unknown>;
		if ('resolved_path' in record) {
			const path = record.resolved_path as Path;
			ids.push(path.id);
			visit(path.args);
			return;
		}
		for (const entry of Object.values(record)) visit(entry);
	};
	visit(type);
	return ids;
}

// -------------------------------------------------------------------------
// Markdown

const RUSTDOC_FENCE_ATTRIBUTES = /^(?:rust|ignore|no_run|compile_fail|should_panic|test_harness|standalone_crate|edition\d{4}|E\d{4})$/;

/**
 * Maps an intra-doc link destination, as written, to a URL. Returns `null` for a link
 * rustdoc resolved to an item without a public page, and `undefined` for text that
 * isn't an intra-doc link.
 */
export type LinkResolver = (destination: string) => string | null | undefined;

/**
 * Rewrites a doc comment into portable Markdown: intra-doc links become URLs, and links
 * to items without a page keep only their label.
 */
export function normalizeDocs(markdown: string, resolve: LinkResolver): string {
	const lines = markdown.split('\n');
	const output: string[] = [];
	let fence: { marker: string; rust: boolean } | undefined;

	for (const line of lines) {
		const fenceMatch = line.match(/^(\s*)(`{3,}|~{3,})(.*)$/);
		if (fence) {
			if (fenceMatch && fenceMatch[2].startsWith(fence.marker) && fenceMatch[3].trim() === '') {
				fence = undefined;
				output.push(line);
				continue;
			}
			// rustdoc hides lines that start with `# ` in Rust examples, and `##` escapes a literal `#`.
			if (fence.rust) {
				if (/^\s*#(?:\s|$)/.test(line)) continue;
				output.push(line.replace(/^(\s*)##/, '$1#'));
			} else {
				output.push(line);
			}
			continue;
		}

		if (fenceMatch) {
			const [, indentation, marker, info] = fenceMatch;
			const attributes = info.split(/[,\s]+/).filter(Boolean);
			const language = attributes.find((attribute) => !RUSTDOC_FENCE_ATTRIBUTES.test(attribute));
			const rust = language === undefined;
			fence = { marker, rust };
			output.push(`${indentation}${marker}${rust ? 'rust' : language}`);
			continue;
		}

		output.push(rewriteLinks(line, resolve));
	}

	return output.join('\n').trim();
}

/** Resolves intra-doc links in one line of Markdown outside code blocks. */
function rewriteLinks(line: string, resolve: LinkResolver): string {
	// Reference definitions: `[label]: destination`. A definition without a page is
	// dropped, so its references fall back to plain text.
	const definition = line.match(/^(\s*\[[^\]]+\]:\s*)(\S+)(.*)$/);
	if (definition) {
		const href = resolve(definition[2]);
		if (href === null) return '';
		return href ? `${definition[1]}${href}${definition[3]}` : line;
	}

	return line.replace(
		/\[((?:[^\[\]`]|`[^`]*`)+)\](?:\(([^()\s]+(?:\([^()]*\))?)\)|\[([^\]]*)\])?/g,
		(whole, label: string, inline: string | undefined, reference: string | undefined) => {
			if (inline !== undefined) {
				if (inline.startsWith('#') || inline.startsWith('/')) return whole;
				// Resolve before treating the destination as a URL: an intra-doc path such
				// as `crate::a::B` looks like a URL with a `crate:` scheme.
				const href = resolve(inline);
				if (href) return `[${label}](${href})`;
				if (href === undefined && /^[a-z][a-z0-9+.-]*:\/\//i.test(inline)) return whole;
				return label;
			}
			const destination = reference === undefined || reference === '' ? label : reference;
			const href = resolve(destination);
			if (href) return `[${label}](${href})`;
			// rustdoc resolved the link, but its target has no page: keep the label.
			if (href === null) return label;
			return whole;
		},
	);
}

/** Returns the first sentence of the first paragraph as plain text. */
export function summarize(markdown: string): string {
	return splitSummary(markdown)[0]
		.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
		.replace(/\[([^\]]+)\]\[[^\]]*\]/g, '$1')
		.replace(/\s+/g, ' ')
		.trim();
}

/**
 * Returns the docs after the sentence that {@link summarize} returns, for pages that
 * already show the summary as their description.
 */
export function withoutSummary(markdown: string): string {
	return splitSummary(markdown)[1];
}

/** Splits docs into their first sentence and the rest, both as Markdown. */
function splitSummary(markdown: string): [string, string] {
	const paragraphEnd = markdown.search(/\n\s*\n/);
	const paragraph = paragraphEnd < 0 ? markdown : markdown.slice(0, paragraphEnd);
	// Docs that open with a code block or heading have no summary sentence.
	if (/^\s*(?:```|~~~|#)/.test(paragraph)) return ['', markdown];
	// A sentence ends at punctuation followed by the start of another sentence or the
	// end of the paragraph. The punctuation in a link destination is never followed by
	// whitespace, so links don't end a sentence early.
	const sentence = paragraph.match(/^[\s\S]*?[.!?](?=\s+[A-Z`\[(]|\s*$)/);
	const length = sentence ? sentence[0].length : paragraph.length;
	return [markdown.slice(0, length), markdown.slice(length).trimStart()];
}
