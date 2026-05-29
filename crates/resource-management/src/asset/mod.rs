//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.
//! Each assert is a file in a specific format, and the asset handlers are responsible for parsing the file and generating the resources from it.

use std::{alloc::Allocator, io::ErrorKind};

use utils::json;

pub mod asset_handler;
pub mod asset_manager;
mod audio_utils;

pub mod bema_asset_handler;
pub mod gltf_asset_handler;
pub mod lut_asset_handler;
pub mod ogg_asset_handler;
pub mod png_asset_handler;
pub mod wav_asset_handler;

pub type BEADType = json::Value;

pub mod resource_id;
pub mod storage_backend;

pub use resource_id::ResourceId;
pub use storage_backend::FileStorageBackend;
pub use storage_backend::{AssetStorageBytes, StorageBackend};

use crate::r#async::read;
use crate::resource::reader::MappedFileBacking;

/// Loads an asset from source asynchronously.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
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
			let format = path.extension().and_then(|e| e.to_str()).ok_or(())?.to_string();

			let spec = read_asset_spec(&spec_path);
			let source_bytes = read_asset_bytes(&path, allocator);

			let (spec, source_bytes) = std::future::join!(spec, source_bytes).await;

			return Ok((source_bytes?, spec?, format));
		}
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			return Err(());
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
