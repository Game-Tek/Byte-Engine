use std::alloc::Allocator;

use utils::Extent;

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};
use crate::{
	asset,
	processors::image_processor::{gamma_from_semantic, guess_semantic_from_name, process_image_in, ImageDescription},
	r#async::BoxedFuture,
	resource,
	types::{Formats, Gamma},
	ProcessedAsset,
};

struct DecodedImage<'a> {
	data: Box<[u8], &'a dyn Allocator>,
	description: ImageDescription,
}

/// The `PNGAssetHandler` struct configures PNG decoding for image assets.
pub struct PNGAssetHandler {
	transformations: png::Transformations,
}

impl Default for PNGAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl PNGAssetHandler {
	pub fn new() -> PNGAssetHandler {
		PNGAssetHandler {
			transformations: png::Transformations::EXPAND,
		}
	}

	/// Creates a PNG asset handler with explicit decoder transformations.
	pub fn with_transformations(transformations: png::Transformations) -> PNGAssetHandler {
		PNGAssetHandler { transformations }
	}
}

impl AssetHandler for PNGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "png" || r#type == "Image" || r#type == "image/png"
	}

	async fn bake<'a>(
		&'a self,
		_: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
		if let Some(dt) = storage_backend.get_type(url) {
			if !self.can_handle(dt) {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let (data, _, dt) = asset_storage_backend
			.resolve_in(url, allocator)
			.await
			.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

		let semantic = guess_semantic_from_name(url.get_base());
		let transformations = self.transformations;

		// Arena-backed source bytes borrow the bake allocator, so decoding stays in this task.
		let decoded = {
			let mut buffer;
			let extent;
			let gamma: Gamma;
			let format;

			match dt.as_str() {
				"png" | "image/png" => {
					let cursor = std::io::Cursor::new(data);
					let mut decoder = png::Decoder::new(cursor);
					decoder.set_transformations(transformations);
					let mut reader = decoder.read_info().map_err(|_| LoadErrors::FailedToProcess)?;

					let Some(size) = reader.output_buffer_size() else {
						return Err(LoadErrors::FailedToProcess);
					};

					buffer = zeroed_vec_in(size, allocator);

					let info = reader.next_frame(&mut buffer).map_err(|_| LoadErrors::FailedToProcess)?;
					buffer.truncate(info.buffer_size());

					extent = Extent::rectangle(info.width, info.height);
					gamma = png_gamma(reader.info(), semantic);
					(buffer, format) = normalize_png_buffer(buffer, info.color_type, info.bit_depth, extent, allocator)?;
				}
				_ => {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let description = ImageDescription {
				format,
				extent,
				semantic,
				gamma,
				generate_mipmaps: false,
			};

			Ok(DecodedImage {
				data: buffer.into(),
				description,
			})
		}?;

		let DecodedImage { data, description } = decoded;

		let (asset, data) = process_image_in(url, description, data, allocator).map_err(|_| LoadErrors::FailedToProcess)?;
		Ok((asset, data.to_vec().into_boxed_slice()))
	}
}

/// Determines the image gamma from PNG metadata before falling back to the asset semantic.
fn png_gamma(info: &png::Info<'_>, semantic: crate::processors::image_processor::Semantic) -> Gamma {
	info.source_gamma
		.map(|g| {
			if g.into_scaled() == 45455 {
				Gamma::SRGB
			} else {
				Gamma::Linear
			}
		})
		.unwrap_or(gamma_from_semantic(semantic))
}

/// Normalizes PNG decoder output into formats supported by the image processor.
fn normalize_png_buffer<'a>(
	mut buffer: Vec<u8, &'a dyn Allocator>,
	color_type: png::ColorType,
	bit_depth: png::BitDepth,
	extent: Extent,
	allocator: &'a dyn Allocator,
) -> Result<(Vec<u8, &'a dyn Allocator>, Formats), LoadErrors> {
	let format = match (color_type, bit_depth) {
		(png::ColorType::Rgb, png::BitDepth::Eight) => Formats::RGB8,
		(png::ColorType::Rgb, png::BitDepth::Sixteen) => Formats::RGB16,
		(png::ColorType::Rgba, png::BitDepth::Eight) => Formats::RGBA8,
		(png::ColorType::Rgba, png::BitDepth::Sixteen) => Formats::RGBA16,
		(png::ColorType::Grayscale, png::BitDepth::Eight) => {
			return Ok((grayscale8_to_rgb8(&buffer, allocator), Formats::RGB8));
		}
		(png::ColorType::Grayscale, png::BitDepth::Sixteen) => {
			swap_16_bit_png_samples(&mut buffer);
			return Ok((grayscale16_to_rgb16(&buffer, extent, allocator), Formats::RGB16));
		}
		(png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => {
			return Ok((grayscale_alpha8_to_rgba8(&buffer, allocator), Formats::RGBA8));
		}
		(png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => {
			swap_16_bit_png_samples(&mut buffer);
			return Ok((grayscale_alpha16_to_rgba16(&buffer, extent, allocator), Formats::RGBA16));
		}
		_ => return Err(LoadErrors::FailedToProcess),
	};

	if bit_depth == png::BitDepth::Sixteen {
		swap_16_bit_png_samples(&mut buffer);
	}

	Ok((buffer, format))
}

fn swap_16_bit_png_samples(buffer: &mut [u8]) {
	for sample in buffer.chunks_exact_mut(2) {
		sample.swap(0, 1);
	}
}

fn zeroed_vec_in(len: usize, allocator: &dyn Allocator) -> Vec<u8, &dyn Allocator> {
	let mut output = Vec::with_capacity_in(len, allocator);
	output.resize(len, 0);
	output
}

fn grayscale8_to_rgb8<'a>(buffer: &[u8], allocator: &'a dyn Allocator) -> Vec<u8, &'a dyn Allocator> {
	let mut output = Vec::with_capacity_in(buffer.len() * 3, allocator);
	for value in buffer {
		output.extend_from_slice(&[*value, *value, *value]);
	}
	output
}

fn grayscale16_to_rgb16<'a>(buffer: &[u8], extent: Extent, allocator: &'a dyn Allocator) -> Vec<u8, &'a dyn Allocator> {
	let mut output = Vec::with_capacity_in(extent.width() as usize * extent.height() as usize * 6, allocator);
	for value in buffer.chunks_exact(2) {
		output.extend_from_slice(value);
		output.extend_from_slice(value);
		output.extend_from_slice(value);
	}
	output
}

fn grayscale_alpha8_to_rgba8<'a>(buffer: &[u8], allocator: &'a dyn Allocator) -> Vec<u8, &'a dyn Allocator> {
	let mut output = Vec::with_capacity_in(buffer.len() * 2, allocator);
	for pixel in buffer.chunks_exact(2) {
		output.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
	}
	output
}

fn grayscale_alpha16_to_rgba16<'a>(buffer: &[u8], extent: Extent, allocator: &'a dyn Allocator) -> Vec<u8, &'a dyn Allocator> {
	let mut output = Vec::with_capacity_in(extent.width() as usize * extent.height() as usize * 8, allocator);
	for pixel in buffer.chunks_exact(4) {
		let gray = &pixel[0..2];
		let alpha = &pixel[2..4];
		output.extend_from_slice(gray);
		output.extend_from_slice(gray);
		output.extend_from_slice(gray);
		output.extend_from_slice(alpha);
	}
	output
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::{
			self, asset_handler::AssetHandler, asset_manager::AssetManager, png_asset_handler::PNGAssetHandler, ResourceId,
		},
		r#async, resource,
	};

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn load_image() {
		let asset_handler = PNGAssetHandler::new();

		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
		let asset_manager = AssetManager::new(asset_storage_backend.clone());

		let url = ResourceId::new("patterned_brick_floor_02_diff_2k.png");

		let (resource, data) = asset_handler
			.bake(
				&asset_manager,
				&resource_storage_backend,
				&asset_storage_backend,
				url,
				&std::alloc::Global,
			)
			.await
			.expect("Image asset handler did not handle asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, resource, &data)
			.expect("Image asset did not store");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "patterned_brick_floor_02_diff_2k.png");
		assert_eq!(resource.class, "Image");
	}

	// #[test]
	// #[ignore]
	// fn load_16_bit_normal_image() {
	// 	let asset_manager = AssetManager::new("../assets".into(),);
	// 	let asset_handler = ImageAssetHandler::new();

	// 	let url = "Revolver_Normal.png";

	// 	let storage_backend = asset_manager.get_test_storage_backend();

	// 	let _ = smol::block_on(asset_handler.load(&asset_manager, storage_backend, &url,)).expect("Image asset handler did not handle asset");

	// 	let generated_resources = storage_backend.get_resources();

	// 	assert_eq!(generated_resources.len(), 1);

	// 	let resource = &generated_resources[0];

	// 	assert_eq!(resource.id, url);
	// 	assert_eq!(resource.class, "Image");
	// }
}
