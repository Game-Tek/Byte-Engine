//! Load source assets and use format-specific handlers to bake engine resources.

use std::alloc::Allocator;

use serde_json::{Map, Value};

use crate::{
	processors::{
		image::{ImageProcessorSettings, ImageSource, Semantic, gamma_from_semantic},
		mesh::{LightmapUvSettings, MeshLodSettings, MeshSettings},
	},
	resources::mesh::MAX_PRIMITIVE_LODS,
};

mod authored_levels;
mod bake_memory;
pub mod handler;
pub mod intercept;
pub mod manager;

#[cfg(debug_assertions)]
pub mod resource_trace;

#[cfg(debug_assertions)]
pub use resource_trace::{ResourceTrace, ResourceTraceItem, ResourceTraceLevel};

pub(crate) use authored_levels::{AuthoredLevelPattern, authored_level_pattern};

pub type BEADType = Value;

pub type JsonObject = Map<String, Value>;

/// Parses authored JSON5 text, such as a BEAD sidecar, into a Serde JSON value.
pub fn parse_json(source: &str) -> Result<BEADType, json5::Error> {
	json5::from_str(source)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The `ContainerDefaultResource` enum identifies the BEAD-selected resource for an unfragmented container asset.
pub(crate) enum ContainerDefaultResource {
	Mesh,
	Animation,
}

/// Reads the optional unfragmented resource choice shared by FBX and glTF BEAD manifests.
pub(crate) fn container_default_resource(spec: Option<&BEADType>) -> Result<Option<ContainerDefaultResource>, String> {
	let Some(value) = spec.and_then(|spec| spec.get("default_resource")) else {
		return Ok(None);
	};

	let Some(value) = value.as_str() else {
		return Err("`default_resource` must be the string `mesh` or `animation`".to_string());
	};

	if value.eq_ignore_ascii_case("mesh") {
		Ok(Some(ContainerDefaultResource::Mesh))
	} else if value.eq_ignore_ascii_case("animation") {
		Ok(Some(ContainerDefaultResource::Animation))
	} else {
		Err(format!(
			"`default_resource` is '{value}', but only `mesh` and `animation` are supported; skeletons require an explicit fragment"
		))
	}
}

/// The `MeshLodsManifest` struct is the optional `lods` object of an FBX or glTF BEAD manifest. A key left out keeps
/// its default, and an unknown key is an error, so a misspelled key cannot silently leave the defaults in place.
#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MeshLodsManifest {
	enabled: bool,
	max_levels: u8,
	max_error: f32,
	lock_borders: bool,
	/// Validated here for every model; handlers that support authored levels read it with [`authored_level_pattern`].
	authored: Option<String>,
}

impl Default for MeshLodsManifest {
	fn default() -> Self {
		let MeshLodSettings {
			max_levels,
			max_error,
			lock_borders,
		} = MeshLodSettings::default();
		Self {
			enabled: true,
			max_levels,
			max_error,
			lock_borders,
			authored: None,
		}
	}
}

/// The `LightmapUvsManifest` struct is the optional `lightmap_uvs` object of an FBX or glTF BEAD manifest. A key left
/// out keeps its default, and an unknown key is an error.
#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LightmapUvsManifest {
	enabled: bool,
	resolution: u32,
}

impl Default for LightmapUvsManifest {
	fn default() -> Self {
		Self {
			enabled: true,
			resolution: LightmapUvSettings::default().resolution,
		}
	}
}

/// Reads the mesh settings an FBX or glTF BEAD manifest gives its meshes, or the defaults when it gives none.
///
/// `lightmap_uvs` says whether the bake builds in lightmaps; without them, no primitive gets lightmap UVs whatever the
/// manifest asks.
pub(crate) fn mesh_settings(spec: Option<&BEADType>, lightmap_uvs: bool) -> Result<MeshSettings, String> {
	let lods = mesh_lod_settings(spec)?;
	let manifest = match spec.and_then(|spec| spec.get("lightmap_uvs")) {
		Some(value) => <LightmapUvsManifest as serde::Deserialize>::deserialize(value)
			.map_err(|error| format!("`lightmap_uvs` is invalid: {error}"))?,
		None => LightmapUvsManifest::default(),
	};
	if !(16..=4096).contains(&manifest.resolution) {
		return Err("`lightmap_uvs.resolution` must be from 16 to 4096".to_string());
	}
	Ok(MeshSettings {
		lods,
		lightmap_uvs: (lightmap_uvs && manifest.enabled).then_some(LightmapUvSettings {
			resolution: manifest.resolution,
		}),
	})
}

/// Reads the detail level settings an FBX or glTF BEAD manifest gives its meshes, or the defaults when it gives none.
fn mesh_lod_settings(spec: Option<&BEADType>) -> Result<MeshLodSettings, String> {
	let Some(value) = spec.and_then(|spec| spec.get("lods")) else {
		return Ok(MeshLodSettings::default());
	};
	let manifest =
		<MeshLodsManifest as serde::Deserialize>::deserialize(value).map_err(|error| format!("`lods` is invalid: {error}"))?;
	if !(1..=MAX_PRIMITIVE_LODS).contains(&(manifest.max_levels as usize)) {
		return Err(format!("`lods.max_levels` must be from 1 to {MAX_PRIMITIVE_LODS}"));
	}
	if !(manifest.max_error > 0.0 && manifest.max_error <= 1.0) {
		return Err("`lods.max_error` must be above 0 and at most 1".to_string());
	}
	if let Some(pattern) = &manifest.authored {
		AuthoredLevelPattern::parse(pattern)?;
	}
	Ok(MeshLodSettings {
		// One level is full detail alone.
		max_levels: if manifest.enabled { manifest.max_levels } else { 1 },
		max_error: manifest.max_error,
		lock_borders: manifest.lock_borders,
	})
}

/// The `ImageSettingsManifest` struct is the optional image settings object of a BEAD manifest: `image` for a
/// standalone image, `images` for every image a model contains. A key left out keeps the default for the image's
/// semantic, and an unknown key is an error, so a misspelled key cannot silently leave the defaults in place.
#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ImageSettingsManifest {
	mips: Option<bool>,
	compression: Option<bool>,
}

/// The BEAD key that holds a standalone image's settings.
pub(crate) const IMAGE_SETTINGS_KEY: &str = "image";

/// The BEAD key that holds the settings of every image a model contains.
pub(crate) const MODEL_IMAGE_SETTINGS_KEY: &str = "images";

/// Reads the processing settings a BEAD manifest gives images under `key`, starting from the defaults for `semantic`.
fn parse_image_settings(spec: Option<&BEADType>, key: &str, semantic: Semantic) -> Result<ImageProcessorSettings, String> {
	let defaults = ImageProcessorSettings::for_semantic(semantic);
	let Some(value) = spec.and_then(|spec| spec.get(key)) else {
		return Ok(defaults);
	};
	let manifest = <ImageSettingsManifest as serde::Deserialize>::deserialize(value)
		.map_err(|error| format!("`{key}` is invalid: {error}"))?;
	Ok(ImageProcessorSettings {
		mips: manifest.mips.unwrap_or(defaults.mips),
		compression: manifest.compression.unwrap_or(defaults.compression),
	})
}

/// Reads the image settings `spec` gives `id` under `key`, and reports a malformed object on the bake's trace.
pub(crate) fn image_settings(
	context: &handler::BakeContext<'_>,
	id: ResourceId<'_>,
	spec: Option<&BEADType>,
	key: &str,
	semantic: Semantic,
) -> Result<ImageProcessorSettings, handler::LoadErrors> {
	parse_image_settings(spec, key, semantic).map_err(|error| {
		context.error(format_args!(
			"Failed to read image settings for '{}': {error}. The most likely cause is a mistyped `{key}` object in the asset's BEAD file. See {}.",
			id.as_ref(),
			utils::online_docs_url(IMAGE_SETTINGS_DOCS_PATH)
		));
		handler::LoadErrors::FailedToProcess
	})
}

const IMAGE_SETTINGS_DOCS_PATH: &str = "reference/assets#image-settings";

/// The fragment that selects a container's only animation clip.
pub(crate) const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";

/// The fragment prefix that selects one named animation clip of a container.
pub(crate) const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";

/// The fragment prefix that selects one generated material of a container, as `materials/<name>.variant`.
pub(crate) const MATERIAL_FRAGMENT_PREFIX: &str = "materials/";

/// The reference section that lists the fragments a glTF or FBX container answers to.
pub(crate) const ASSETS_FRAGMENTS_DOCS_PATH: &str = "reference/assets#container-fragments";

/// The fragment that selects the skeleton a container generates for its skins and animations.
pub(crate) const SKELETON_FRAGMENT: &str = "skeleton";

/// Returns the resource ID of the skeleton generated for the container at `base`.
pub(crate) fn generated_skeleton_id(base: &str) -> String {
	format!("{base}#{SKELETON_FRAGMENT}")
}

/// Picks the resource an unfragmented glTF or FBX request bakes.
///
/// A BEAD `default_resource` wins. Otherwise a container with a mesh bakes the mesh, and one with no mesh and exactly
/// one animation bakes that animation. `format` and `clips` name the container format and its animations in errors.
pub(crate) fn select_unfragmented_resource(
	spec: Option<&BEADType>,
	has_mesh: bool,
	animation_count: usize,
	format: &str,
	clips: &str,
) -> Result<ContainerDefaultResource, String> {
	match container_default_resource(spec)? {
		Some(ContainerDefaultResource::Animation) if animation_count != 1 => Err(format!(
			"BEAD selects animation, but the {format} contains {animation_count} {clips}; use an explicit animation fragment"
		)),
		Some(selected) => Ok(selected),
		None if has_mesh => Ok(ContainerDefaultResource::Mesh),
		None if animation_count == 1 => Ok(ContainerDefaultResource::Animation),
		None => Err(format!(
			"the {format} contains no mesh and {animation_count} {clips}; use an explicit fragment"
		)),
	}
}

/// Writes a processed container mesh into resource storage and commits it as the requested primary resource.
///
/// `materials` holds the resolved material of each slot the mesh's primitives reference.
pub(crate) async fn commit_mesh(
	context: handler::BakeContext<'_>,
	url: ResourceId<'_>,
	mesh: crate::processors::mesh::MeshProcessorSession,
	materials: &[crate::ReferenceModel<crate::resources::material::VariantModel>],
) -> Result<(), handler::LoadErrors> {
	let mut transaction = context.begin_resource(url, mesh.payload_size()).await?;
	let (mesh, stream_descriptions) = mesh
		.finish_into_resource(materials, &mut transaction)
		.await
		.map_err(|_| handler::LoadErrors::FailedToStore)?;

	context
		.commit_primary(
			transaction,
			crate::ProcessedAsset::new(url, mesh).with_streams(stream_descriptions),
		)
		.await
}

/// Stores one generated model and returns the serialized reference used by its parent resource.
pub(crate) async fn store_model<M: crate::Model>(
	context: handler::BakeContext<'_>,
	id: &str,
	model: M,
	data: &[u8],
) -> Result<crate::ReferenceModel<M>, handler::LoadErrors> {
	context
		.store_resource(crate::ProcessedAsset::new(ResourceId::new(id), model), data)
		.await
		.map(Into::into)
}

/// Stores one generated model by moving an owned payload into resource storage.
pub(crate) async fn store_model_owned<M: crate::Model, T: compio::buf::IoBuf>(
	context: handler::BakeContext<'_>,
	id: &str,
	model: M,
	data: T,
) -> Result<crate::ReferenceModel<M>, handler::LoadErrors> {
	context
		.store_resource_owned(crate::ProcessedAsset::new(ResourceId::new(id), model), data)
		.await
		.map(Into::into)
}

/// Processes one texture a mesh importer extracted through the shared image processor and stores its image resource.
///
/// `spec` is the model's BEAD manifest, whose `images` object applies to every image the model contains.
pub(crate) async fn store_imported_image(
	context: handler::BakeContext<'_>,
	id: ResourceId<'_>,
	semantic: Semantic,
	spec: Option<&BEADType>,
	source: ImageSource<'_>,
) -> Result<(), handler::LoadErrors> {
	let gamma = gamma_from_semantic(semantic);
	let settings = image_settings(&context, id, spec, MODEL_IMAGE_SETTINGS_KEY, semantic)?;
	let image = context
		.image_processor()
		.process_in(semantic, gamma, settings, source, context.allocator())
		.await?;
	let (resource, data) = image.into_asset(id);

	context.store_resource(resource, &data).await.map(|_| ())
}

/// Tells whether `url` names the material or variant a container generates under `base_id`.
///
/// A generated material stores `<base_id>.material` and `<base_id>.variant`, so a material fragment names one of them.
pub(crate) fn names_generated_material(url: ResourceId<'_>, base_id: &str) -> bool {
	url.as_ref()
		.strip_prefix(base_id)
		.is_some_and(|suffix| suffix == ".variant" || suffix == ".material")
}

/// Reports a material fragment that names no generated material of its container, and returns the load error.
pub(crate) fn missing_generated_material(context: handler::BakeContext<'_>, url: ResourceId<'_>) -> handler::LoadErrors {
	let base = url.get_base();
	context.error(format_args!(
		"Material '{}' isn't generated by its container. The most likely cause is an ID without its `.variant` suffix, or a material that no mesh uses or that a BEAD sidecar overrides. Use an ID the container's mesh lists, such as `{}#materials/<name>.variant`; `beld bake --dry-run --format json {}` shows them. See {}.",
		url.as_ref(),
		base.as_ref(),
		base.as_ref(),
		crate::online_docs_url(ASSETS_FRAGMENTS_DOCS_PATH)
	));
	handler::LoadErrors::FailedToProcess
}

/// Converts authored material names into stable resource-ID path components.
pub(crate) fn sanitize_material_name(name: &str) -> String {
	let sanitized = name
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
				character
			} else {
				'_'
			}
		})
		.collect::<String>();

	if sanitized.is_empty() {
		"material".to_string()
	} else {
		sanitized
	}
}

#[cfg(test)]
mod container_default_resource_tests {

	use super::{ContainerDefaultResource, container_default_resource};

	#[test]
	fn bead_default_resource_accepts_mesh_and_animation_but_never_skeleton() {
		for (value, expected) in [
			("mesh", ContainerDefaultResource::Mesh),
			("Animation", ContainerDefaultResource::Animation),
		] {
			let spec = super::parse_json(&format!(r#"{{ "default_resource": "{value}" }}"#)).unwrap();

			assert_eq!(container_default_resource(Some(&spec)), Ok(Some(expected)));
		}

		let skeleton = super::parse_json(r#"{ "default_resource": "skeleton" }"#).unwrap();

		assert!(container_default_resource(Some(&skeleton)).is_err());
	}
}

#[cfg(test)]
mod image_settings_tests {
	use super::parse_image_settings;
	use crate::processors::image::{ImageProcessorSettings, Semantic};

	fn settings(source: &str, semantic: Semantic) -> Result<ImageProcessorSettings, String> {
		parse_image_settings(Some(&super::parse_json(source).unwrap()), "image", semantic)
	}

	#[test]
	fn image_settings_override_only_the_keys_they_name() {
		let albedo = ImageProcessorSettings::for_semantic(Semantic::Albedo);

		assert_eq!(parse_image_settings(None, "image", Semantic::Albedo), Ok(albedo));
		assert_eq!(settings("{ image: {} }", Semantic::Albedo), Ok(albedo));
		assert_eq!(
			settings("{ image: { mips: false } }", Semantic::Albedo),
			Ok(ImageProcessorSettings { mips: false, ..albedo })
		);
		assert_eq!(
			settings("{ image: { compression: true } }", Semantic::Roughness),
			Ok(ImageProcessorSettings {
				mips: true,
				compression: true
			})
		);
	}

	#[test]
	fn image_settings_reject_unknown_keys_and_non_boolean_values() {
		assert!(settings("{ image: { mipmaps: false } }", Semantic::Albedo).is_err());
		assert!(settings("{ image: { mips: 0 } }", Semantic::Albedo).is_err());
		assert!(settings("{ image: false }", Semantic::Albedo).is_err());
	}
}

#[cfg(test)]
mod mesh_lod_settings_tests {
	use super::mesh_lod_settings;
	use crate::processors::mesh::MeshLodSettings;

	fn settings(source: &str) -> Result<MeshLodSettings, String> {
		mesh_lod_settings(Some(&super::parse_json(source).unwrap()))
	}

	#[test]
	fn bead_lods_default_when_absent_and_override_each_key() {
		assert_eq!(mesh_lod_settings(None), Ok(MeshLodSettings::default()));
		assert_eq!(settings("{ default_resource: 'mesh' }"), Ok(MeshLodSettings::default()));
		assert_eq!(settings("{ lods: {} }"), Ok(MeshLodSettings::default()));

		assert_eq!(
			settings("{ lods: { max_levels: 3, max_error: 0.05, lock_borders: true } }"),
			Ok(MeshLodSettings {
				max_levels: 3,
				max_error: 0.05,
				lock_borders: true,
			})
		);
	}

	#[test]
	fn bead_lods_turn_generation_off_and_reject_what_they_cannot_apply() {
		assert_eq!(
			settings("{ lods: { enabled: false, max_levels: 4 } }").map(|lods| lods.max_levels),
			Ok(1)
		);

		for source in [
			"{ lods: true }",
			"{ lods: { levels: 4 } }",
			"{ lods: { enabled: 1 } }",
			"{ lods: { max_levels: 0 } }",
			"{ lods: { max_levels: 9 } }",
			"{ lods: { max_error: 0 } }",
			"{ lods: { max_error: 1.5 } }",
		] {
			assert!(settings(source).is_err(), "{source} should be rejected");
		}
	}

	#[test]
	fn bead_lightmap_uvs_follow_the_bake_and_the_manifest() {
		let parse = |source: &str, built_in| super::mesh_settings(Some(&super::parse_json(source).unwrap()), built_in);
		let resolution = |resolution| Some(super::LightmapUvSettings { resolution });

		assert_eq!(parse("{}", false).map(|settings| settings.lightmap_uvs), Ok(None));
		assert_eq!(parse("{}", true).map(|settings| settings.lightmap_uvs), Ok(resolution(64)));
		assert_eq!(
			parse("{ lightmap_uvs: { resolution: 256 } }", true).map(|settings| settings.lightmap_uvs),
			Ok(resolution(256))
		);
		assert_eq!(
			parse("{ lightmap_uvs: { enabled: false } }", true).map(|settings| settings.lightmap_uvs),
			Ok(None)
		);
		for source in ["{ lightmap_uvs: { resolution: 8 } }", "{ lightmap_uvs: { size: 64 } }"] {
			assert!(parse(source, true).is_err(), "{source} should be rejected");
		}
	}
}

pub mod resource_id;
pub mod storage_backend;

pub use resource_id::ResourceId;
pub use storage_backend::{
	AssetSource, AssetStorageBytes, AssetVersion, DynStorageBackend, FileStorageBackend, StorageBackend,
};

use crate::r#async::read;
use crate::resource::reader::MappedFileBacking;

/// Loads the exact source file without looking for or parsing an adjacent BEAD sidecar.
///
/// Paths are relative to `base_path`. Request settings separately through [`StorageBackend::load_sidecar`].
pub async fn read_asset_from_source<'a>(
	url: ResourceId<'a>,
	base_path: Option<&'a std::path::Path>,
	allocator: &'a dyn Allocator,
) -> Result<(AssetStorageBytes<'a>, String), ()> {
	let base = url.get_base();

	if base.as_ref().starts_with("http://") || base.as_ref().starts_with("https://") {
		return Err(());
	}

	let path = base_path.unwrap_or(std::path::Path::new("")).join(base.as_ref());
	let source_bytes = read_asset_bytes(&path, allocator).await?;

	Ok((source_bytes, url.get_asset_type().to_string()))
}

/// Maps source bytes when possible, falling back to an allocated asynchronous read.
async fn read_asset_bytes<'a>(path: &std::path::Path, allocator: &'a dyn Allocator) -> Result<AssetStorageBytes<'a>, ()> {
	match std::fs::File::open(path)
		.map_err(|_| ())
		.and_then(|file| MappedFileBacking::new(&file))
	{
		Ok(mapped_file) => Ok(AssetStorageBytes::MappedFile(mapped_file)),
		Err(_) => {
			let source_bytes = read(path).await.or(Err(()))?;

			let mut source_data = Vec::with_capacity_in(source_bytes.len(), allocator);

			source_data.extend_from_slice(&source_bytes);

			Ok(AssetStorageBytes::Allocated(source_data.into_boxed_slice()))
		}
	}
}
