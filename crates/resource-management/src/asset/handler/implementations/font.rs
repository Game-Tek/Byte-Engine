/// The `FontAssetHandler` struct lets an asset manager bake `.ttf` and `.otf` files into [`Font`] resources, so an
/// application ships its fonts in the resource store with the rest of its assets.
///
/// Register it with [`crate::asset::manager::AssetManager::add_asset_handler`]; the standard graphics setup does.
#[derive(Default)]
pub struct FontAssetHandler;

impl FontAssetHandler {
	pub fn new() -> Self {
		Self
	}
}

impl AssetHandler for FontAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		matches!(r#type, "ttf" | "otf" | "font/ttf" | "font/otf")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (data, dt) = context.resolve(url).await?;

		if !self.can_handle(&dt) {
			return Err(LoadErrors::UnsupportedType);
		}

		// The payload is the file itself, so a font the UI can't parse must fail here instead of at its first frame.
		if ttf_parser::Face::parse(&data, 0).is_err() {
			context.error(format_args!(
				"Font '{}' could not be parsed. The most likely cause is a corrupt file or one that isn't a TrueType or OpenType font. Re-export it as a single .ttf or .otf font.",
				url.as_ref()
			));
			return Err(LoadErrors::FailedToProcess);
		}

		context.store_primary(ProcessedAsset::new(url, Font {}), &data).await
	}
}

#[cfg(test)]
mod tests {

	use super::FontAssetHandler;
	use crate::{
		asset::{self, ResourceId, manager::AssetManager},
		r#async, resource,
	};

	/// Builds the smallest font `ttf_parser` accepts: a table directory with `head`, `hhea`, and `maxp`.
	fn minimal_font() -> Vec<u8> {
		let mut head = vec![0u8; 54];
		head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // Version 1.0.
		head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes()); // Magic number.
		head[18..20].copy_from_slice(&1000u16.to_be_bytes()); // Units per em.
		let mut hhea = vec![0u8; 36];
		hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
		let mut maxp = vec![0u8; 6];
		maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes()); // Version 0.5, for CFF outlines.
		maxp[4..6].copy_from_slice(&1u16.to_be_bytes()); // One glyph.

		let tables: [(&[u8; 4], Vec<u8>); 3] = [(b"head", head), (b"hhea", hhea), (b"maxp", maxp)];
		let mut font = Vec::new();
		font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
		font.extend_from_slice(&(tables.len() as u16).to_be_bytes());
		font.extend_from_slice(&[0; 6]); // Search hints, which parsers don't need.
		let mut offset = 12 + 16 * tables.len();
		for (tag, table) in &tables {
			font.extend_from_slice(*tag);
			font.extend_from_slice(&[0; 4]); // Checksum.
			font.extend_from_slice(&(offset as u32).to_be_bytes());
			font.extend_from_slice(&(table.len() as u32).to_be_bytes());
			offset += table.len().next_multiple_of(4);
		}
		for (_, table) in &tables {
			font.extend_from_slice(table);
			font.resize(font.len().next_multiple_of(4), 0);
		}
		font
	}

	#[r#async::test]
	async fn fonts_bake_into_font_resources_holding_the_file_unchanged() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::MemoryStorageBackend::new();
		let font = minimal_font();

		asset_storage_backend.add_file("fonts/display.ttf", &font);
		asset_storage_backend.add_file("fonts/broken.otf", b"not a font");

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.add_asset_handler(FontAssetHandler::new());

		asset_manager
			.bake("fonts/display.ttf")
			.await
			.expect("a valid font should bake");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("fonts/display.ttf"))
			.expect("the baked font should be stored");
		assert_eq!(resource.class, "Font");
		assert_eq!(
			resource_storage_backend
				.get_resource_data_by_name(ResourceId::new("fonts/display.ttf"))
				.as_deref(),
			Some(font.as_slice())
		);

		assert!(
			asset_manager.bake("fonts/broken.otf").await.is_err(),
			"a file that isn't a font must fail the bake"
		);
	}
}

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{ProcessedAsset, resources::font::Font};
