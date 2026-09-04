//! Load source assets and use format-specific handlers to bake engine resources.

use std::alloc::Allocator;

use serde_json::{Map, Value};

mod bake_memory;
pub mod handler;
pub mod manager;

#[cfg(debug_assertions)]
pub mod resource_trace;

#[cfg(debug_assertions)]
pub use resource_trace::{ResourceTrace, ResourceTraceItem, ResourceTraceLevel};

pub type BEADType = Value;

pub type JsonObject = Map<String, Value>;

/// Parses authored JSON5 text into a Serde JSON value.
pub(crate) fn parse_json(source: &str) -> Result<BEADType, json5::Error> {
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

/// Stores one generated model and returns the serialized reference used by its parent resource.
pub(crate) async fn store_model<M: crate::Model>(
	context: handler::BakeContext<'_>,
	id: &str,
	model: M,
	data: &[u8],
) -> Result<crate::ReferenceModel<M>, handler::LoadErrors> {
	context
		.store_generated(crate::ProcessedAsset::new(ResourceId::new(id), model), data)
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
		.store_generated_owned(crate::ProcessedAsset::new(ResourceId::new(id), model), data)
		.await
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
