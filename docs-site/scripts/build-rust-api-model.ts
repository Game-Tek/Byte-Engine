/**
 * Writes the Rust API model the site renders from a rustdoc JSON file.
 *
 * `scripts/generate-api-docs.sh` runs it after `cargo rustdoc`:
 * `bun docs-site/scripts/build-rust-api-model.ts <rustdoc.json> <model.json>`.
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { buildModel } from '../src/lib/rustdoc/build-model';
import type { Crate } from '../src/lib/rustdoc/format';

const [input, output] = process.argv.slice(2);
if (!input || !output) {
	console.error(
		'Missing arguments. The model builder needs the rustdoc JSON file and the model output path. ' +
			'Run `bun docs-site/scripts/build-rust-api-model.ts target/api-docs-json/doc/byte_engine.json docs-site/generated/rust-api.json`.',
	);
	process.exit(1);
}

const crate = JSON.parse(await readFile(input, 'utf8')) as Crate;
const model = buildModel(crate, (links) => {
	const examples = links.slice(0, 10).map((link) => `  ${link}`).join('\n');
	console.warn(
		`${links.length} doc links point at items without a public API page, so they render as plain code.\n${examples}${links.length > 10 ? '\n  …' : ''}\n` +
			'Their targets are most likely in `#[doc(hidden)]` modules without a `pub use`, private, or in a crate without published docs. ' +
			'Re-export the target from a public module, or change the link to the path readers can import.',
	);
});
await mkdir(dirname(output), { recursive: true });
await writeFile(output, JSON.stringify(model));
console.log(`Wrote ${Object.keys(model.items).length} Rust API items to ${output}.`);
