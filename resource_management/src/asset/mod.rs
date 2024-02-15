//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

pub mod asset_manager;
pub mod asset_handler;

pub mod audio_asset_handler;
pub mod material_asset_handler;
pub mod image_asset_handler;
pub mod mesh_asset_handler;

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
pub async fn read_asset_from_source(url: &str, base_path: Option<&std::path::Path>) -> Result<(Vec<u8>, String), ()> {
	let resource_origin = if url.starts_with("http://") || url.starts_with("https://") { "network" } else { "local" };
	let mut source_bytes;
	let format;
	match resource_origin {
		"network" => {
			let request = if let Ok(request) = ureq::get(url).call() { request } else { return Err(None); };
			let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return Err(None); };
			format = content_type;

			source_bytes = Vec::new();

			request.into_reader().read_to_end(&mut source_bytes).or(Err(()))?;
		},
		"local" => {
			let path = base_path.ok_or(None)?;

			let mut file = smol::fs::File::open(&path).await.or(Err(()));

			format = path.extension().unwrap().to_str().unwrap().to_string();

			source_bytes = Vec::with_capacity(file.metadata().await.unwrap().len() as usize);

			if let Err(_) = file.read_to_end(&mut source_bytes).await {
				return Err(None);
			}
		},
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			return Err(None);
		}
	}

	Ok((source_bytes, format))
}