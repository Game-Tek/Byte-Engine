//! Load source assets and use format-specific handlers to bake engine resources.

use std::{alloc::Allocator, io::ErrorKind};

use utils::{json, json::JsonValueTrait};

pub mod asset_handler;
pub mod asset_manager;
mod audio_utils;

pub mod bema_asset_handler;
pub mod besl_shader_asset_handler;
pub mod exr_asset_handler;
pub mod fbx_asset_handler;
pub mod gltf_asset_handler;
pub mod lut_asset_handler;
pub mod ogg_asset_handler;
pub mod png_asset_handler;
pub mod wav_asset_handler;

#[cfg(debug_assertions)]
pub mod resource_trace;

#[cfg(debug_assertions)]
pub use resource_trace::{ResourceTrace, ResourceTraceItem, ResourceTraceLevel};

pub type BEADType = json::Value;

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

/// Stores one generated model and returns the serialized reference used by its parent resource.
pub(crate) fn store_model<M: crate::Model>(
	context: asset_handler::BakeContext<'_>,
	id: &str,
	model: M,
	data: &[u8],
) -> Result<crate::ReferenceModel<M>, asset_handler::LoadErrors> {
	context
		.store_generated(crate::ProcessedAsset::new(ResourceId::new(id), model), data)
		.map(Into::into)
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
	use super::{container_default_resource, ContainerDefaultResource};

	#[test]
	fn bead_default_resource_accepts_mesh_and_animation_but_never_skeleton() {
		for (value, expected) in [
			("mesh", ContainerDefaultResource::Mesh),
			("Animation", ContainerDefaultResource::Animation),
		] {
			let spec = utils::json::from_str(&format!(r#"{{ "default_resource": "{value}" }}"#)).unwrap();
			assert_eq!(container_default_resource(Some(&spec)), Ok(Some(expected)));
		}

		let skeleton = utils::json::from_str(r#"{ "default_resource": "skeleton" }"#).unwrap();
		assert!(container_default_resource(Some(&skeleton)).is_err());
	}
}

pub mod resource_id;
pub mod storage_backend;

pub use resource_id::ResourceId;
pub use storage_backend::FileStorageBackend;
pub use storage_backend::{AssetStorageBytes, StorageBackend};

use crate::r#async::read;
use crate::resource::reader::MappedFileBacking;

/// Loads a source asset and its optional BEAD description.
///
/// Pass a path relative to the assets directory or a network URL. The function
/// returns `Err(())` when it cannot find or read the asset.
pub async fn read_asset_from_source<'a>(
	url: ResourceId<'a>,
	base_path: Option<&'a std::path::Path>,
	allocator: &'a dyn Allocator,
) -> Result<(AssetStorageBytes<'a>, Option<BEADType>, String), ()> {
	let base = url.get_base();

	let resource_origin = if base.as_ref().starts_with("http://") || base.as_ref().starts_with("https://") {
		"network"
	} else {
		"local"
	};

	match resource_origin {
		// "network" => {
		// 	let request = if let Ok(request) = ureq::get(base.as_ref()).call() { request } else { return Err(()); };
		// 	let content_type = if let Some(e) = request.headers().get("content-type") { e.to_str().unwrap().to_string() } else { return Err(()); };
		// 	format = content_type;

		// 	source_bytes = Vec::new();

		// 	spec = None;

		// 	request.body().read_to_end(&mut source_bytes).or(Err(()))?;
		// },
		"local" => {
			let path = base_path.unwrap_or(std::path::Path::new(""));

			let path = path.join(base.as_ref());
			let spec_path = path.with_added_extension("bead");
			let format = path
				.extension()
				.and_then(|extension| extension.to_str())
				.unwrap_or_default()
				.to_string();

			let spec = read_asset_spec(&spec_path);
			let source_bytes = read_asset_bytes(&path, allocator);

			let (spec, source_bytes) = std::future::join!(spec, source_bytes).await;

			return Ok((source_bytes?, spec?, format));
		}
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			Err(())
		}
	}
}

async fn read_asset_spec(spec_path: &std::path::Path) -> Result<Option<BEADType>, ()> {
	// Append ".bead" to the file name to check for a resource file.
	let spec_bytes = match read(spec_path).await {
		Ok(bytes) => Some(bytes),
		Err(err) if err.kind() == ErrorKind::NotFound => None,
		Err(_) => return Err(()),
	};

	if let Some(spec_bytes) = spec_bytes {
		let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
		let spec: json::Value = json::from_str(spec).or(Err(()))?;
		Ok(Some(spec))
	} else {
		Ok(None)
	}
}

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
