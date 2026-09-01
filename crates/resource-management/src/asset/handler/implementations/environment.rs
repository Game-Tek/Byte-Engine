/// The `EnvironmentMapAssetHandler` struct generates image-based lighting from standalone `.environment.bead` assets.
pub struct EnvironmentMapAssetHandler {
	ibl_generator: IBLGenerator,
}

impl EnvironmentMapAssetHandler {
	/// Creates an environment-map handler that uses `ibl_generator` for offline lighting generation.
	///
	/// Next, register this handler with [`crate::asset::manager::AssetManager::add_asset_handler`].
	pub fn new(ibl_generator: IBLGenerator) -> Self {
		Self { ibl_generator }
	}
}

impl Default for EnvironmentMapAssetHandler {
	fn default() -> Self {
		Self::new(IBLGenerator::new())
	}
}

impl AssetHandler for EnvironmentMapAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("environment.bead")
	}

	/// Resolves the authored source image, converts high-precision samples to linear RGBA16F, and stores its IBL maps.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(resource_type) = context.resource_type(url)
			&& resource_type != "Image"
			&& !self.can_handle(resource_type)
		{
			return Err(LoadErrors::UnsupportedType);
		}

		let (manifest, _, asset_type) = context.resolve(url).await?;

		if !self.can_handle(&asset_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let source_id = parse_environment_source(&manifest).map_err(|cause| {
			context.error(format_args!(
				"Environment-map asset '{}' is invalid. The most likely cause is {cause}. See {}.",
				url.as_ref(),
				crate::online_docs_url(ENVIRONMENT_MAP_DOCS_PATH)
			));

			LoadErrors::FailedToProcess
		})?;
		let source_url = ResourceId::new(&source_id);
		let encoded = context.resolve_raw(source_url).await.map_err(|_| {
			context.error(format_args!(
				"Environment-map source '{source_id}' could not be read. The most likely cause is that the asset-root-relative source ID does not exist or is inaccessible. See {}.",
				crate::online_docs_url(ENVIRONMENT_MAP_DOCS_PATH)
			));

			LoadErrors::AssetCouldNotBeRead
		})?;
		let decoded = decode_rgba16f_in(encoded.as_slice(), context.allocator()).map_err(|error| {
			context.error(format_args!(
				"Environment-map source '{source_id}' is invalid. {error} See {}.",
				crate::online_docs_url(ENVIRONMENT_MAP_DOCS_PATH)
			));

			LoadErrors::FailedToProcess
		})?;

		let extent = decoded.extent();

		if extent.height().checked_mul(2) != Some(extent.width()) {
			context.error(format_args!(
				"Environment-map source '{source_id}' does not use a 2:1 latitude-longitude layout. The most likely cause is that the source is not a full-sphere equirectangular image. See {}.",
				crate::online_docs_url(ENVIRONMENT_MAP_DOCS_PATH)
			));

			return Err(LoadErrors::FailedToProcess);
		}

		self.ibl_generator
			.generate_and_store(context, url, extent, decoded.data())
			.await
	}
}

/// The `EnvironmentMapManifest` struct represents the authored source used for one environment map.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentMapManifest {
	#[serde(rename = "$schema")]
	_schema: Option<String>,
	source: String,
}

/// Reads and validates the strict environment manifest.
fn parse_environment_source(manifest: &[u8]) -> Result<String, &'static str> {
	let manifest = std::str::from_utf8(manifest).map_err(|_| "the manifest is not UTF-8 text")?;
	let EnvironmentMapManifest { source, .. } =
		json5::from_str(manifest).map_err(|_| "the manifest does not match the environment-map schema")?;

	if source.is_empty() {
		return Err("`source` is empty");
	}

	// Resource IDs use asset-root-relative forward-slash paths. Reject ambiguous or escaping paths before storage joins them.
	let source_bytes = source.as_bytes();
	let has_windows_drive_prefix = source_bytes.len() >= 2 && source_bytes[0].is_ascii_alphabetic() && source_bytes[1] == b':';

	if source.starts_with('/')
		|| has_windows_drive_prefix
		|| source.contains('\\')
		|| source.contains('#')
		|| source
			.split('/')
			.any(|component| component.is_empty() || matches!(component, "." | ".."))
	{
		return Err("`source` is not a normalized asset-root-relative resource ID");
	}

	Ok(source)
}

const ENVIRONMENT_MAP_DOCS_PATH: &str = "develop/resource-management/assets#environment-maps";

#[cfg(test)]
mod tests {
	use std::{
		io::Cursor,
		sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		},
	};

	use exr::prelude::{SpecificChannels, WritableImage as _};
	use image::codecs::hdr::HdrEncoder;

	use super::EnvironmentMapAssetHandler;
	use crate::{
		asset::{
			ResourceId,
			handler::{AssetHandler, BakeContext, LoadErrors},
			manager::AssetManager,
			storage_backend::tests::TestStorageBackend,
		},
		r#async,
		resource::{ReadStorageBackend as _, storage_backend::tests::TestStorageBackend as TestResourceStorage},
		resources::image::{
			IBL_DIFFUSE_IRRADIANCE_STREAM_NAME, IMAGE_BASE_MIP_STREAM_NAME, Image, ibl_prefiltered_specular_stream_name,
		},
		types::{Formats, Gamma},
	};

	/// The `CountingEnvironmentMapAssetHandler` struct exposes whether freshness checks invoke another environment bake.
	struct CountingEnvironmentMapAssetHandler {
		inner: EnvironmentMapAssetHandler,
		bakes: Arc<AtomicUsize>,
	}

	impl AssetHandler for CountingEnvironmentMapAssetHandler {
		fn can_handle(&self, r#type: &str) -> bool {
			self.inner.can_handle(r#type)
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
			self.bakes.fetch_add(1, Ordering::Relaxed);
			self.inner.bake(context, url).await
		}
	}

	/// Encodes a tiny EXR whose values prove that linear highlights and negatives survive the environment path.
	fn exr_fixture(first_red: f32) -> Vec<u8> {
		let channels = SpecificChannels::rgb(|position: exr::prelude::Vec2<usize>| match position.x() {
			0 => (first_red, 0.5_f32, -0.25_f32),
			_ => (16.0_f32, 2.0_f32, 8.0_f32),
		});
		let image = exr::prelude::Image::from_channels((2, 1), channels);
		let mut bytes = Vec::new();

		image
			.write()
			.non_parallel()
			.to_buffered(Cursor::new(&mut bytes))
			.expect("the in-memory EXR fixture must encode");

		bytes
	}

	/// Encodes a high-precision image that is not a 2:1 latitude-longitude environment.
	fn square_exr_fixture() -> Vec<u8> {
		let channels = SpecificChannels::rgb(|_| (1.0_f32, 0.5_f32, 0.25_f32));
		let image = exr::prelude::Image::from_channels((2, 2), channels);
		let mut bytes = Vec::new();

		image
			.write()
			.non_parallel()
			.to_buffered(Cursor::new(&mut bytes))
			.expect("the in-memory square EXR fixture must encode");

		bytes
	}

	/// Encodes a Radiance HDR source, which intentionally has no standalone asset handler.
	fn hdr_fixture() -> Vec<u8> {
		let mut bytes = Vec::new();
		let pixels = [image::Rgb([4.0, 0.5, 0.25]), image::Rgb([16.0, 2.0, 8.0])];

		HdrEncoder::new(&mut bytes)
			.encode(&pixels, 2, 1)
			.expect("the HDR fixture must encode");

		bytes
	}

	/// Encodes one 8-bit source used to exercise the environment precision contract.
	fn eight_bit_png_fixture() -> Vec<u8> {
		let mut bytes = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
			encoder.set_color(png::ColorType::Rgba);
			encoder.set_depth(png::BitDepth::Eight);
			let mut writer = encoder.write_header().expect("the PNG header must encode");
			writer
				.write_image_data(&[32, 128, 224, 255])
				.expect("the PNG pixels must encode");
		}

		bytes
	}

	#[r#async::test]
	async fn standalone_environment_asset_preserves_the_existing_ibl_contract() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file(
			"studio.environment.bead",
			br#"{ "$schema": "byte-engine/schemas/environment.bead.schema.json", source: "studio.exr" }"#,
		);
		source_storage.add_file("studio.exr", &exr_fixture(4.0));

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		// The raw EXR is a decoder input, so this test deliberately registers no EXR asset handler.
		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());
		asset_manager
			.bake("studio.environment.bead")
			.await
			.expect("the environment asset must bake its referenced EXR");

		let (stored, _) = resource_storage
			.read(ResourceId::new("studio.environment.bead"))
			.await
			.expect("the baked environment image must be stored under the manifest ID");
		let image: Image = crate::from_slice(stored.resource()).expect("the stored environment metadata must deserialize");
		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("studio.environment.bead"))
			.expect("the stored environment payload must exist");
		let base_values = data[..16]
			.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();
		let ibl = image.ibl.expect("the environment asset must include baked IBL maps");

		assert_eq!(stored.class(), "Image");
		assert_eq!(image.format, Formats::RGBA16F);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(image.mip_count, 1);
		assert_eq!(base_values, vec![4.0, 0.5, -0.25, 1.0, 16.0, 2.0, 8.0, 1.0]);
		assert_eq!(ibl.diffuse_irradiance.extent, [8, 8, 0]);
		assert_eq!(ibl.diffuse_irradiance.array_layers, 6);
		assert_eq!(ibl.prefiltered_specular.extent, [1, 1, 0]);
		assert_eq!(ibl.prefiltered_specular.mip_count, 8);
		assert_eq!(ibl.prefiltered_specular.array_layers, 6);

		let streams = stored
			.streams()
			.expect("the environment image and IBL maps must be described");

		assert_eq!(streams.len(), 10);
		assert_eq!(streams[0].name(), IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!(streams[0].offset(), 0);
		assert_eq!(streams[0].size(), 16);
		for (index, stream) in streams[1..9].iter().enumerate() {
			assert_eq!(stream.name(), ibl_prefiltered_specular_stream_name(index as u32));
		}
		assert_eq!(streams[1].offset(), 16);
		assert_eq!(streams[1].size(), 48);
		assert_eq!(streams[9].name(), IBL_DIFFUSE_IRRADIANCE_STREAM_NAME);
		assert_eq!(data.len(), 3_472);
	}

	#[r#async::test]
	async fn malformed_source_sidecars_do_not_block_environment_baking() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("raw.environment.bead", br#"{ source: "raw.exr" }"#);
		source_storage.add_file("raw.exr", &exr_fixture(4.0));
		source_storage.add_file("raw.exr.bead", b"this is not JSON5");

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());
		asset_manager
			.bake("raw.environment.bead")
			.await
			.expect("a referenced image sidecar must not participate in environment baking");

		assert!(resource_storage.read(ResourceId::new("raw.environment.bead")).await.is_some());
	}

	#[r#async::test]
	async fn environment_asset_decodes_hdr_without_an_hdr_asset_handler() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("sky.environment.bead", br#"{ source: "sky.hdr" }"#);
		source_storage.add_file("sky.hdr", &hdr_fixture());

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());
		asset_manager
			.bake("sky.environment.bead")
			.await
			.expect("the environment decoder must accept Radiance HDR directly");

		let (stored, _) = resource_storage
			.read(ResourceId::new("sky.environment.bead"))
			.await
			.expect("the HDR-backed environment must be stored");
		let image: Image = crate::from_slice(stored.resource()).expect("the environment metadata must deserialize");

		assert!(image.ibl.is_some());
	}

	#[r#async::test]
	async fn eight_bit_sources_fail_without_storing_an_environment() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("low.environment.bead", br#"{ source: "low.png" }"#);
		source_storage.add_file("low.png", &eight_bit_png_fixture());

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());

		assert!(asset_manager.bake("low.environment.bead").await.is_err());
		assert!(resource_storage.read(ResourceId::new("low.environment.bead")).await.is_none());
	}

	#[r#async::test]
	async fn non_latitude_longitude_sources_fail_without_storing_an_environment() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("square.environment.bead", br#"{ source: "square.exr" }"#);
		source_storage.add_file("square.exr", &square_exr_fixture());

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());

		assert!(asset_manager.bake("square.environment.bead").await.is_err());
		assert!(
			resource_storage
				.read(ResourceId::new("square.environment.bead"))
				.await
				.is_none()
		);
	}

	#[r#async::test]
	async fn invalid_environment_manifests_fail_without_storing_resources() {
		let source_storage = TestStorageBackend::new();
		let cases = [
			("array.environment.bead", "[]"),
			("missing.environment.bead", "{}"),
			("number.environment.bead", r#"{ source: 1 }"#),
			("empty.environment.bead", r#"{ source: "" }"#),
			("schema.environment.bead", r#"{ "$schema": 1, source: "studio.exr" }"#),
			("setting.environment.bead", r#"{ source: "studio.exr", quality: "high" }"#),
			("escape.environment.bead", r#"{ source: "../studio.exr" }"#),
			("drive.environment.bead", r#"{ source: "C:/studio.exr" }"#),
			("drive-relative.environment.bead", r#"{ source: "C:studio.exr" }"#),
		];

		for (id, manifest) in cases {
			source_storage.add_file(id, manifest.as_bytes());
		}

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());

		for (id, _) in cases {
			assert!(
				asset_manager.bake(id).await.is_err(),
				"invalid environment manifest must fail: {id}"
			);
			assert!(resource_storage.read(ResourceId::new(id)).await.is_none());
		}
	}

	#[r#async::test]
	async fn changing_the_referenced_image_rebakes_the_environment_asset() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("tracked.environment.bead", br#"{ source: "tracked.exr" }"#);
		source_storage.add_file("tracked.exr", &exr_fixture(4.0));

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage.clone(), resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::default());
		asset_manager
			.bake_if_not_exists::<Image>("tracked.environment.bead")
			.await
			.expect("the initial environment must bake");

		source_storage.add_file("tracked.exr", &exr_fixture(32.0));

		asset_manager
			.bake_if_not_exists::<Image>("tracked.environment.bead")
			.await
			.expect("the changed source image must rebake its environment");

		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("tracked.environment.bead"))
			.expect("the rebaked environment payload must exist");
		let first_red = exr::prelude::f16::from_le_bytes([data[0], data[1]]).to_f32();

		assert_eq!(first_red, 32.0);
	}

	#[r#async::test]
	async fn changing_only_the_referenced_image_sidecar_does_not_rebake_the_environment_asset() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("stable.environment.bead", br#"{ source: "stable.exr" }"#);
		source_storage.add_file("stable.exr", &exr_fixture(4.0));

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage.clone(), resource_storage);
		let bakes = Arc::new(AtomicUsize::new(0));

		asset_manager.add_asset_handler(CountingEnvironmentMapAssetHandler {
			inner: EnvironmentMapAssetHandler::default(),
			bakes: bakes.clone(),
		});
		asset_manager
			.bake_if_not_exists::<Image>("stable.environment.bead")
			.await
			.expect("the initial environment must bake");

		source_storage.add_file("stable.exr.bead", b"this changed sidecar is deliberately malformed");

		asset_manager
			.bake_if_not_exists::<Image>("stable.environment.bead")
			.await
			.expect("a source-image sidecar edit must leave the stored environment fresh");

		assert_eq!(bakes.load(Ordering::Relaxed), 1);
	}

	#[cfg(feature = "gpu-ibl")]
	#[r#async::test]
	async fn unavailable_gpu_worker_falls_back_to_cpu_baking() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("fallback.environment.bead", br#"{ source: "fallback.exr" }"#);
		source_storage.add_file("fallback.exr", &exr_fixture(4.0));

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EnvironmentMapAssetHandler::new(
			crate::ibl::IBLGenerator::unavailable_for_test(),
		));
		asset_manager
			.bake("fallback.environment.bead")
			.await
			.expect("an unavailable GPU worker must use CPU environment generation");

		let (stored, _) = resource_storage
			.read(ResourceId::new("fallback.environment.bead"))
			.await
			.expect("the CPU fallback environment must be stored");
		let image: Image = crate::from_slice(stored.resource()).expect("the fallback metadata must deserialize");

		assert!(image.ibl.is_some());
	}
}

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{ibl::IBLGenerator, processors::processor::implementations::image::decode_rgba16f_in};
