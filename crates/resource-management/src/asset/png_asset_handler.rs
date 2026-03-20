use utils::Extent;

use crate::{
	asset,
	processors::image_processor::{gamma_from_semantic, guess_semantic_from_name, process_image, ImageDescription},
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	types::{Formats, Gamma},
	ProcessedAsset,
};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};

struct DecodedImage {
	data: Box<[u8]>,
	description: ImageDescription,
}

pub struct PNGAssetHandler {}

impl PNGAssetHandler {
	pub fn new() -> PNGAssetHandler {
		PNGAssetHandler {}
	}
}

impl AssetHandler for PNGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "png" || r#type == "Image" || r#type == "image/png"
	}

	fn bake<'a>(
		&'a self,
		_: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let (data, _, dt) = asset_storage_backend
				.resolve(url)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			let semantic = guess_semantic_from_name(url.get_base());

			let decoded = spawn_cpu_task(move || -> Result<DecodedImage, LoadErrors> {
				let mut buffer;
				let extent;
				let gamma: Gamma;
				let format;

				match dt.as_str() {
					"png" | "image/png" => {
						let cursor = std::io::Cursor::new(data);
						let decoder = png::Decoder::new(cursor);
						if true { // TODO: make this a setting
							 // decoder.set_transformations(png::Transformations::normalize_to_color8());
						}
						let mut reader = decoder.read_info().map_err(|_| LoadErrors::FailedToProcess)?;

						let Some(size) = reader.output_buffer_size() else {
							return Err(LoadErrors::FailedToProcess);
						};

						buffer = vec![0u8; size];

						let info = reader.next_frame(&mut buffer).map_err(|_| LoadErrors::FailedToProcess)?;

						extent = Extent::rectangle(info.width, info.height);

						gamma = reader
							.info()
							.gama_chunk
							.map(|g: png::ScaledFloat| {
								if g.into_scaled() == 45455 {
									Gamma::SRGB
								} else {
									Gamma::Linear
								}
							})
							.unwrap_or(gamma_from_semantic(semantic));

						match info.bit_depth {
							png::BitDepth::Eight => {}
							png::BitDepth::Sixteen => {
								for i in 0..buffer.len() / 2 {
									buffer.swap(i * 2, i * 2 + 1);
								}
							}
							_ => {
								return Err(LoadErrors::FailedToProcess);
							}
						}

						format = match info.color_type {
							png::ColorType::Rgb => match info.bit_depth {
								png::BitDepth::Eight => Formats::RGB8,
								png::BitDepth::Sixteen => Formats::RGB16,
								_ => {
									return Err(LoadErrors::FailedToProcess);
								}
							},
							png::ColorType::Rgba => match info.bit_depth {
								png::BitDepth::Eight => Formats::RGBA8,
								png::BitDepth::Sixteen => Formats::RGBA16,
								_ => {
									return Err(LoadErrors::FailedToProcess);
								}
							},
							_ => {
								return Err(LoadErrors::FailedToProcess);
							}
						};
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
				};

				Ok(DecodedImage {
					data: buffer.into(),
					description,
				})
			})
			.await
			.map_err(|_| LoadErrors::FailedToProcess)??;

			let DecodedImage { data, description } = decoded;

			process_image(url, description, data)
		})
	}
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
			.bake(&asset_manager, &resource_storage_backend, &asset_storage_backend, url)
			.await
			.expect("Image asset handler did not handle asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
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
