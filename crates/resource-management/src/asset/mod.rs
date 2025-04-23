//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

use utils::{json, sync::{File, Read}};

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

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
pub fn read_asset_from_source<'a>(url: ResourceId<'a>, base_path: Option<&'a std::path::Path>) -> Result<(Box<[u8]>, Option<BEADType>, String), ()> {
    let base = url.get_base();
	let resource_origin = if base.as_ref().starts_with("http://") || base.as_ref().starts_with("https://") { "network" } else { "local" };
	let mut source_bytes;
	let format;
	let spec;
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

			let file = File::open(&path);
			let mut file = file.or(Err(()))?;

			spec = {
				// Append ".bead" to the file name to check for a resource file
				let spec_path = path.with_added_extension("bead");
				let file = File::open(spec_path).ok();
				if let Some(mut file) = file {
					let mut spec_bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);
					if let Err(_) = file.read_to_end(&mut spec_bytes) {
						return Err(());
					}
					let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					None
				}
			};

			format = path.extension().unwrap().to_str().unwrap().to_string();

			source_bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);

			if let Err(_) = file.read_to_end(&mut source_bytes) {
				return Err(());
			}
		},
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			return Err(());
		}
	}

	Ok((source_bytes.into(), spec, format))
}