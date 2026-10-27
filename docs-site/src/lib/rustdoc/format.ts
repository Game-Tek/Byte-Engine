/**
 * Types for the subset of rustdoc's unstable JSON output that the API model reads.
 *
 * rustdoc JSON changes shape between nightly toolchains. This file and
 * `build-model.ts` are the only code that knows that shape; everything else in
 * the site reads the stable model in `model.ts`. When `scripts/generate-api-docs.sh`
 * moves to a toolchain with a new format version, update these types and
 * {@link RUSTDOC_FORMAT_VERSION} together, then fix what `build-model.ts` reports.
 *
 * Field names follow the JSON, which serializes Rust enums as externally tagged
 * snake_case objects (`{ "resolved_path": { … } }`) and unit variants as strings.
 */

/** The rustdoc JSON format version produced by the toolchain that `generate-api-docs.sh` pins. */
export const RUSTDOC_FORMAT_VERSION = 61;

/** The toolchain that produces {@link RUSTDOC_FORMAT_VERSION}. Keep it equal to `TOOLCHAIN` in `generate-api-docs.sh`. */
export const RUSTDOC_TOOLCHAIN = 'nightly-2026-08-31';

/**
 * An externally tagged enum: exactly one key of `Variants` holding that variant's payload,
 * or the variant name as a string for unit variants listed in `Units`.
 */
export type Tagged<Variants, Units extends string = never> =
	| { [K in keyof Variants]: { [P in K]: Variants[P] } }[keyof Variants]
	| Units;

export type Id = number;

export interface Crate {
	root: Id;
	crate_version: string | null;
	includes_private: boolean;
	index: Record<string, Item>;
	paths: Record<string, ItemSummary>;
	external_crates: Record<string, ExternalCrate>;
	format_version: number;
}

export interface ExternalCrate {
	name: string;
	html_root_url: string | null;
}

export interface ItemSummary {
	crate_id: number;
	path: string[];
	kind: ItemKind;
}

export type ItemKind =
	| 'module'
	| 'extern_crate'
	| 'use'
	| 'struct'
	| 'struct_field'
	| 'union'
	| 'enum'
	| 'variant'
	| 'function'
	| 'type_alias'
	| 'constant'
	| 'trait'
	| 'trait_alias'
	| 'impl'
	| 'static'
	| 'extern_type'
	| 'macro'
	| 'proc_attribute'
	| 'proc_derive'
	| 'assoc_const'
	| 'assoc_type'
	| 'primitive'
	| 'keyword'
	| 'attribute';

export interface Item {
	id: Id;
	crate_id: number;
	name: string | null;
	span: Span | null;
	visibility: Visibility;
	docs: string | null;
	links: Record<string, Id>;
	attrs: Attribute[];
	deprecation: Deprecation | null;
	inner: ItemEnum;
}

export interface Span {
	filename: string;
	begin: [number, number];
	end: [number, number];
}

export interface Deprecation {
	since: string | null;
	note: string | null;
}

export type Visibility = Tagged<{ restricted: { parent: Id; path: string } }, 'public' | 'default' | 'crate'>;

export type Attribute = Tagged<
	{
		must_use: { reason: string | null };
		export_name: string;
		link_section: string;
		repr: unknown;
		target_feature: unknown;
		other: string;
	},
	'non_exhaustive' | 'macro_export' | 'automatically_derived' | 'no_mangle'
>;

export type ItemEnum = Tagged<{
	module: Module;
	extern_crate: { name: string; rename: string | null };
	use: Use;
	union: Union;
	struct: Struct;
	struct_field: Type;
	enum: Enum;
	variant: Variant;
	function: Function;
	trait: Trait;
	trait_alias: TraitAlias;
	impl: Impl;
	type_alias: TypeAlias;
	constant: { type: Type; const: Constant };
	static: Static;
	extern_type: null;
	macro: string;
	proc_macro: ProcMacro;
	primitive: { name: string; impls: Id[] };
	assoc_const: { type: Type; value: string | null };
	assoc_type: { generics: Generics; bounds: GenericBound[]; type: Type | null };
}>;

export interface Module {
	is_crate: boolean;
	items: Id[];
	is_stripped: boolean;
}

export interface Use {
	source: string;
	name: string;
	id: Id | null;
	is_glob: boolean;
}

export interface Union {
	generics: Generics;
	has_stripped_fields: boolean;
	fields: Id[];
	impls: Id[];
}

export interface Struct {
	kind: StructKind;
	generics: Generics;
	impls: Id[];
}

export type StructKind = Tagged<
	{
		tuple: (Id | null)[];
		plain: { fields: Id[]; has_stripped_fields: boolean };
	},
	'unit'
>;

export interface Enum {
	generics: Generics;
	has_stripped_variants: boolean;
	variants: Id[];
	impls: Id[];
}

export interface Variant {
	kind: VariantKind;
	discriminant: { expr: string; value: string } | null;
}

export type VariantKind = Tagged<
	{
		tuple: (Id | null)[];
		struct: { fields: Id[]; has_stripped_fields: boolean };
	},
	'plain'
>;

export interface FunctionHeader {
	is_const: boolean;
	is_unsafe: boolean;
	is_async: boolean;
	abi: unknown;
}

export interface Function {
	sig: FunctionSignature;
	generics: Generics;
	header: FunctionHeader;
	has_body: boolean;
}

export interface FunctionSignature {
	inputs: [string, Type][];
	output: Type | null;
	is_c_variadic: boolean;
}

export interface Generics {
	params: GenericParamDef[];
	where_predicates: WherePredicate[];
}

export interface GenericParamDef {
	name: string;
	kind: GenericParamDefKind;
}

export type GenericParamDefKind = Tagged<{
	lifetime: { outlives: string[] };
	type: { bounds: GenericBound[]; default: Type | null; is_synthetic: boolean };
	const: { type: Type; default: string | null };
}>;

export type WherePredicate = Tagged<{
	bound_predicate: { type: Type; bounds: GenericBound[]; generic_params: GenericParamDef[] };
	lifetime_predicate: { lifetime: string; outlives: string[] };
	eq_predicate: { lhs: Type; rhs: Term };
}>;

export type GenericBound = Tagged<{
	trait_bound: { trait: Path; generic_params: GenericParamDef[]; modifier: 'none' | 'maybe' | 'maybe_const' };
	outlives: string;
	use: Tagged<{ lifetime: string; param: string }>[];
}>;

export type Term = Tagged<{ type: Type; constant: Constant }>;

export interface Constant {
	expr: string;
	value: string | null;
	is_literal: boolean;
}

export type Type = Tagged<
	{
		resolved_path: Path;
		dyn_trait: DynTrait;
		generic: string;
		primitive: string;
		function_pointer: FunctionPointer;
		tuple: Type[];
		slice: Type;
		array: { type: Type; len: string };
		pat: { type: Type };
		impl_trait: GenericBound[];
		raw_pointer: { is_mutable: boolean; type: Type };
		borrowed_ref: { lifetime: string | null; is_mutable: boolean; type: Type };
		qualified_path: { name: string; args: GenericArgs | null; self_type: Type; trait: Path | null };
	},
	'infer'
>;

export interface Path {
	path: string;
	id: Id;
	args: GenericArgs | null;
}

export interface DynTrait {
	traits: { trait: Path; generic_params: GenericParamDef[] }[];
	lifetime: string | null;
}

export interface FunctionPointer {
	sig: FunctionSignature;
	generic_params: GenericParamDef[];
	header: FunctionHeader;
}

export type GenericArgs = Tagged<
	{
		angle_bracketed: { args: GenericArg[]; constraints: AssocItemConstraint[] };
		parenthesized: { inputs: Type[]; output: Type | null };
	},
	'return_type_notation'
>;

export type GenericArg = Tagged<{ lifetime: string; type: Type; const: Constant }, 'infer'>;

export interface AssocItemConstraint {
	name: string;
	args: GenericArgs | null;
	binding: Tagged<{ equality: Term; constraint: GenericBound[] }>;
}

export interface Trait {
	is_auto: boolean;
	is_unsafe: boolean;
	is_dyn_compatible: boolean;
	items: Id[];
	generics: Generics;
	bounds: GenericBound[];
	implementations: Id[];
}

export interface TraitAlias {
	generics: Generics;
	params: GenericBound[];
}

export interface Impl {
	is_unsafe: boolean;
	generics: Generics;
	provided_trait_methods: string[];
	trait: Path | null;
	for: Type;
	items: Id[];
	is_negative: boolean;
	is_synthetic: boolean;
	blanket_impl: Type | null;
}

export interface TypeAlias {
	type: Type;
	generics: Generics;
}

export interface Static {
	type: Type;
	is_mutable: boolean;
	is_unsafe: boolean;
	expr: string;
}

export interface ProcMacro {
	kind: 'bang' | 'attr' | 'derive';
	helpers: string[];
}

/**
 * The `FormatDriftError` class reports rustdoc JSON that doesn't match the types in this file.
 *
 * `build-model.ts` throws it so a toolchain change fails the API build instead of dropping content.
 */
export class FormatDriftError extends Error {
	constructor(found: string, context: string) {
		super(
			`rustdoc JSON contains ${found} at ${context}, which the API model doesn't know. ` +
				`The rustdoc JSON format changed, most likely because the toolchain in scripts/generate-api-docs.sh changed. ` +
				`Pin TOOLCHAIN back to ${RUSTDOC_TOOLCHAIN}, or add the new shape to docs-site/src/lib/rustdoc/format.ts and handle it in build-model.ts.`,
		);
		this.name = 'FormatDriftError';
	}
}

type UnitKeys<T> = T extends string ? T : never;
type PayloadOf<T> = T extends string ? never : T;
type VariantMap<T> = {
	[K in keyof UnionToIntersection<PayloadOf<T>>]: UnionToIntersection<PayloadOf<T>>[K];
} & { [K in UnitKeys<T>]: null };
type UnionToIntersection<U> = (U extends unknown ? (value: U) => void : never) extends (value: infer I) => void
	? I
	: never;

/**
 * Dispatches on an externally tagged enum with one arm per variant.
 *
 * TypeScript rejects a missing arm, and an unknown variant at runtime throws
 * {@link FormatDriftError} naming `context`, so format drift can't silently drop output.
 */
export function match<T extends object | string, R>(
	value: T,
	arms: { [K in keyof VariantMap<T>]: (payload: VariantMap<T>[K]) => R },
	context: string,
): R {
	const tag = typeof value === 'string' ? value : Object.keys(value)[0];
	const arm = (arms as Record<string, ((payload: unknown) => R) | undefined>)[tag];
	if (!arm) throw new FormatDriftError(`the variant "${tag}"`, context);
	return arm(typeof value === 'string' ? null : (value as Record<string, unknown>)[tag]);
}

/** Returns the variant name of an externally tagged enum value. */
export function tagOf(value: object | string): string {
	return typeof value === 'string' ? value : Object.keys(value)[0];
}

/**
 * Returns the payload of one variant, or `undefined` when `value` holds another.
 *
 * Use it where only one variant matters. Code that must handle every variant uses
 * {@link match}, so a new variant fails the build there instead.
 */
export function variantOf<T extends object | string, K extends keyof VariantMap<T> & string>(
	value: T,
	tag: K,
): VariantMap<T>[K] | undefined {
	if (typeof value === 'string' || !(tag in (value as object))) return undefined;
	return (value as Record<string, unknown>)[tag] as VariantMap<T>[K];
}
