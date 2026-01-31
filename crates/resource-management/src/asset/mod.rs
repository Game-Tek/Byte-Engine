//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

use std::io::ErrorKind;

use utils::json;

pub mod asset_manager;
pub mod asset_handler;

pub mod audio_asset_handler;
pub mod material_asset_handler;
pub mod image_asset_handler;
pub mod mesh_asset_handler;

pub type BEADType = json::Value;

pub mod storage_backend;
pub mod resource_id;

pub use resource_id::ResourceId;
pub use storage_backend::StorageBackend;
pub use storage_backend::FileStorageBackend;

use crate::r#async::read;

/// Loads an asset from source asynchronously.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
pub async fn read_asset_from_source<'a>(url: ResourceId<'a>, base_path: Option<&'a std::path::Path>) -> Result<(Box<[u8]>, Option<BEADType>, String), ()> {
    let base = url.get_base();
	let resource_origin = if base.as_ref().starts_with("http://") || base.as_ref().starts_with("https://") { "network" } else { "local" };
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

			let spec = {
				// Append ".bead" to the file name to check for a resource file
				let spec_path = path.with_added_extension("bead");
				let spec_bytes = match read(&spec_path).await {
					Ok(bytes) => Some(bytes),
					Err(err) if err.kind() == ErrorKind::NotFound => None,
					Err(_) => return Err(()),
				};
				if let Some(spec_bytes) = spec_bytes {
					let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					None
				}
			};

			let format = path.extension().and_then(|e| e.to_str()).ok_or(())?.to_string();

			let source_bytes = read(&path).await.or(Err(()))?;

			return Ok((source_bytes.into_boxed_slice(), spec, format));
		},
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			return Err(());
		}
	}
}
