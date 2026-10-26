import type { ApiModel } from '@/lib/rustdoc/model';

// `scripts/generate-api-docs.sh` writes the model, and Git ignores it. A glob import
// lets the site build without it, with an empty Rust API section, so pages outside
// the API can be previewed without the pinned nightly toolchain.
const generated = import.meta.glob<ApiModel>('../../../generated/rust-api.json', {
	eager: true,
	import: 'default',
});

/** The Rust API model, or `undefined` when the API docs haven't been generated. */
export const rustApiModel: ApiModel | undefined = Object.values(generated)[0];
