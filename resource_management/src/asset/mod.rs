//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

use smol::io::AsyncReadExt;

pub mod asset_manager;
pub mod asset_handler;

pub mod audio_asset_handler;
pub mod material_asset_handler;
pub mod image_asset_handler;
pub mod mesh_asset_handler;

pub type BEADType = json::JsonValue;

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
pub fn read_asset_from_source<'a>(url: &'a str, base_path: Option<&'a std::path::Path>) -> utils::SendSyncBoxedFuture<'a, Result<(Vec<u8>, Option<BEADType>, String), ()>> { Box::pin(async move {
	let resource_origin = if url.starts_with("http://") || url.starts_with("https://") { "network" } else { "local" };
	let mut source_bytes;
	let format;
	let spec;
	match resource_origin {
		"network" => {
			let request = if let Ok(request) = ureq::get(url).call() { request } else { return Err(()); };
			let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return Err(()); };
			format = content_type;

			source_bytes = Vec::new();

			spec = None;

			request.into_reader().read_to_end(&mut source_bytes).or(Err(()))?;
		},
		"local" => {
			let path = base_path.unwrap_or(std::path::Path::new(""));

			let path = path.join(url);

			let mut file = smol::fs::File::open(&path).await.or(Err(()))?;

			spec = {
				// Append ".bead" to the file name to check for a resource file
				let mut spec_path = path.clone().as_os_str().to_os_string();
				spec_path.push(".bead");
				let file = smol::fs::File::open(spec_path).await.ok();
				if let Some(mut file) = file {
					let mut spec_bytes = Vec::with_capacity(file.metadata().await.unwrap().len() as usize);
					if let Err(_) = file.read_to_end(&mut spec_bytes).await {
						return Err(());
					}
					let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
					let spec = json::parse(spec).or(Err(()))?;
					Some(spec)
				} else {
					None
				}
			};

			format = path.extension().unwrap().to_str().unwrap().to_string();

			source_bytes = Vec::with_capacity(file.metadata().await.unwrap().len() as usize);

			if let Err(_) = file.read_to_end(&mut source_bytes).await {
				return Err(());
			}
		},
		_ => {
			// Could not resolve how to get raw resource, return empty bytes
			return Err(());
		}
	}

	Ok((source_bytes, spec, format))
}) }

enum ResolvedAsset {
	Loaded((Vec<u8>, Option<BEADType>, String)),
	AlreadyExists,
}

pub fn get_base<'a>(url: &'a str) -> Option<&'a str> {
	let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	} 
	let path = std::path::Path::new(url);
	Some(path.to_str()?)
}

fn get_fragment(url: &str) -> Option<String> {
	let mut split = url.split('#');
	let _ = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	let fragment = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	if split.count() == 0 {
		Some(fragment.to_string())
	} else {
		None
	}
}

#[cfg(test)]
pub mod tests {
    use crate::asset::get_fragment;

    use super::get_base;

	#[test]
	fn test_base_url_parse() {
		assert_eq!(get_base("name.extension").unwrap(), "name.extension");
		assert_eq!(get_base("name.extension#").unwrap(), "name.extension");
		assert_eq!(get_base("#fragment"), None);
		assert_eq!(get_base("name.extension#fragment").unwrap(), "name.extension");
		assert_eq!(get_base("dir/name.extension").unwrap(), "dir/name.extension");
		assert_eq!(get_base("dir/name.extension#").unwrap(), "dir/name.extension");
		assert_eq!(get_base("dir/#fragment").unwrap(), "dir/");
		assert_eq!(get_base("dir/name.extension#fragment").unwrap(), "dir/name.extension");
	}

	#[test]
	fn test_fragment_parse() {
		assert_eq!(get_fragment("name.extension"), None);
		assert_eq!(get_fragment("name.extension#"), None);
		assert_eq!(get_fragment("#fragment"), None);
		assert_eq!(get_fragment("name.extension#fragment").unwrap(), "fragment");
	}
}