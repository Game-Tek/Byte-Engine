/**
 * Black-box tests of the rustdoc JSON → API model seam.
 *
 * The fixture is the rustdoc JSON of `test/fixtures/rustdoc-sample`. After changing
 * the fixture crate or the pinned toolchain, regenerate it from that directory with
 * `cargo +nightly-2026-08-31 rustdoc --lib -Z unstable-options --output-format json -- -Z unstable-options --document-hidden-items`
 * and copy `target/doc/sample.json` to `rustdoc.json`.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { buildModel, withoutSummary } from '@/lib/rustdoc/build-model';
import { FormatDriftError, type Crate } from '@/lib/rustdoc/format';

const fixture = () =>
	JSON.parse(readFileSync(new URL('../fixtures/rustdoc-sample/rustdoc.json', import.meta.url), 'utf8')) as Crate;
const model = buildModel(fixture());
const item = (key: string) => {
	const found = model.items[key];
	if (!found) throw new Error(`The model has no ${key}.`);
	return found;
};

describe('public paths', () => {
	test('places an item from a hidden module at its re-export path', () => {
		expect(item('sample::engine::Hidden').route).toBe('/docs/api/latest/sample/engine/Hidden');
		expect(item('sample::engine').children).toContainEqual({ key: 'sample::engine::Hidden', name: 'Hidden', reexport: false });
	});

	test('leaves out items reachable only through hidden modules', () => {
		expect(Object.keys(model.items).some((key) => key.endsWith('Unreachable'))).toBe(false);
		expect(Object.keys(model.items).some((key) => key.includes('::hidden'))).toBe(false);
	});

	test('keeps a public definition path canonical and lists other paths as re-exports', () => {
		expect(item('sample::engine::Point').page).toBe(true);
		expect(model.items['sample::Point']).toBeUndefined();
		expect(item('sample').children).toContainEqual({ key: 'sample::engine::Point', name: 'Point', reexport: true });
	});

	test('puts functions and constants on their module page', () => {
		expect(item('sample::engine::shape_at')).toMatchObject({ page: false, route: '/docs/api/latest/sample/engine#fn.shape_at' });
		expect(item('sample::engine::LIMIT').route).toBe('/docs/api/latest/sample/engine#constant.LIMIT');
	});
});

describe('implementations', () => {
	test('lists derived traits and leaves out auto and blanket implementations', () => {
		const traits = item('sample::engine::Point').traitImpls!.map((traitImpl) => traitImpl.traitName);
		expect(traits).toEqual(expect.arrayContaining(['Clone', 'Debug', 'PartialEq']));
		for (const hidden of ['Send', 'Sync', 'Unpin', 'Borrow', 'Into', 'From', 'Any', 'StructuralPartialEq']) {
			expect(traits).not.toContain(hidden);
		}
	});

	test('splits trait members into required and provided, and lists implementors', () => {
		const area = item('sample::engine::Area');
		expect(area.required!.map((member) => member.anchor)).toEqual(['tymethod.area']);
		expect(area.provided!.map((member) => member.anchor)).toEqual(['method.double']);
		expect(area.implementors!.map((implementor) => implementor.href)).toEqual(['/docs/api/latest/sample/engine/Shape']);
	});

	test('records which functions accept and return each type', () => {
		expect(item('sample::engine::Point').acceptedBy!.map((reference) => reference.label)).toEqual(['shape_at']);
		expect(item('sample::engine::Shape').returnedBy!.map((reference) => reference.label)).toEqual(['shape_at']);
	});

	test('describes fields and variants with linked types', () => {
		const point = item('sample::engine::Point');
		expect(point.fields!.map((field) => [field.name, field.summary])).toEqual([
			['x', 'The horizontal coordinate.'],
			['y', 'The vertical coordinate.'],
		]);
		const shape = item('sample::engine::Shape');
		expect(shape.variants!.map((variant) => variant.name)).toEqual(['Circle', 'Square', 'Empty']);
		expect(shape.variants![0].fields.map((field) => field.anchor)).toEqual(['variant.Circle.field.radius']);
		const origin = point.inherent![0].members[0];
		expect(origin.declaration.map((token) => token.text).join('')).toBe('pub fn origin() -> Self');
	});
});

describe('docs', () => {
	test('resolves intra-doc links to site routes and member anchors', () => {
		const docs = item('sample::engine::Point').docs;
		expect(docs).toContain('[`shape_at`](/docs/api/latest/sample/engine#fn.shape_at)');
		expect(docs).toContain('[`Point::origin`](/docs/api/latest/sample/engine/Point#method.origin)');
	});

	test('resolves an inline link to a Rust path and keeps a web link', () => {
		const docs = item('sample::engine::Shape').docs;
		expect(docs).toContain('[`shape_at`](/docs/api/latest/sample/engine#fn.shape_at)');
		expect(docs).toContain('[Rust book](https://doc.rust-lang.org/book/)');
	});

	test('keeps only the label of a link whose target has no page, and reports it', () => {
		const reported: string[] = [];
		const docs = buildModel(fixture(), (links) => reported.push(...links)).items['sample::engine::Point'].docs;
		expect(docs).toContain('It replaces `hidden::Unreachable`, which has no page.');
		expect(reported).toEqual(['sample::engine::Point: `hidden::Unreachable`']);
	});

	test('drops hidden lines from Rust examples', () => {
		const docs = item('sample::engine::LIMIT').docs;
		expect(docs).toContain('```rust\nlet visible = sample::engine::LIMIT;\n```');
		expect(docs).not.toContain('hidden_setup');
	});

	test('summarizes an item with the first sentence of its docs', () => {
		expect(item('sample::engine::Point').summary).toBe('The `Point` struct holds a position.');
	});

	test('leaves the docs after the summary for pages that show the summary separately', () => {
		const docs = item('sample::engine::Point').docs;
		const rest = withoutSummary(docs);
		expect(docs.startsWith('The `Point` struct holds a position.')).toBe(true);
		expect(rest).not.toContain('holds a position');
		expect(docs.endsWith(rest)).toBe(true);
		expect(withoutSummary('```rust\nlet x = 1;\n```')).toBe('```rust\nlet x = 1;\n```');
	});
});

describe('format drift', () => {
	test('rejects JSON from another format version and names the fix', () => {
		const crate = { ...fixture(), format_version: 999 };
		expect(() => buildModel(crate)).toThrow(/format version 999.*Pin TOOLCHAIN back to nightly-2026-08-31/);
	});

	test('fails on an item shape the model does not know instead of dropping it', () => {
		const crate = fixture();
		const point = Object.values(crate.index).find((entry) => entry.name === 'Point' && 'struct' in (entry.inner as object))!;
		point.inner = { future_kind: {} } as unknown as typeof point.inner;
		expect(() => buildModel(crate)).toThrow(FormatDriftError);
	});
});
