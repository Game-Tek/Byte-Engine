/** Black-box tests of the API model → agent Markdown seam, using the fixture in `build-model.test.ts`. */
import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderRustPageMarkdown, renderSymbolIndex } from '@/lib/rust-api/markdown';
import { buildModel } from '@/lib/rustdoc/build-model';
import type { Crate } from '@/lib/rustdoc/format';

const model = buildModel(
	JSON.parse(readFileSync(new URL('../fixtures/rustdoc-sample/rustdoc.json', import.meta.url), 'utf8')) as Crate,
);

test('renders a struct with its import, declaration, fields, methods, and Markdown links', () => {
	const markdown = renderRustPageMarkdown(model, 'sample::engine::Point');
	expect(markdown).toStartWith('Struct `sample::engine::Point`');
	expect(markdown).toContain('Import: `use sample::engine::Point;`');
	expect(markdown).toContain('```rust\npub struct Point<T> {\n    pub x: T,\n    pub y: T,\n}\n```');
	expect(markdown).toContain('| `x` | `T` | The horizontal coordinate. |');
	expect(markdown).toContain('### `origin`');
	expect(markdown).toContain('[`shape_at`](/docs/api/latest/sample/engine.md#fn.shape_at)');
	expect(markdown).toContain('## Accepted by\n\n- [`shape_at`](/docs/api/latest/sample/engine.md#fn.shape_at): Builds a shape at a point.');
});

test('renders a module with item tables and inline functions', () => {
	const markdown = renderRustPageMarkdown(model, 'sample::engine');
	expect(markdown).toContain('| [`Point`](/docs/api/latest/sample/engine/Point.md) | The `Point` struct holds a position. |');
	expect(markdown).toContain('### `shape_at`\n\n```rust\npub fn shape_at(point: &Point<f32>) -> Shape\n```');
});

test('lists every item and member on one line in the symbol index', () => {
	const lines = renderSymbolIndex(model).split('\n');
	expect(lines).toContain(
		'sample::engine::shape_at | fn | pub fn shape_at(point: &Point<f32>) -> Shape | Builds a shape at a point. | /docs/api/latest/sample/engine.md#fn.shape_at',
	);
	expect(lines).toContain(
		'sample::engine::Area::area | method | fn area(&self) -> f32 | Returns the area. | /docs/api/latest/sample/engine/Area.md#tymethod.area',
	);
	expect(lines).toContain(
		'sample::engine::Shape::Circle | variant | Circle { radius: f32 } | A circle. | /docs/api/latest/sample/engine/Shape.md#variant.Circle',
	);
});
