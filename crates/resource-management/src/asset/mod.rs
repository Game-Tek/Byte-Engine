//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

use std::fmt::Debug;

use utils::{r#async::AsyncReadExt, json, File};

pub mod asset_manager;
pub mod asset_handler;

pub mod audio_asset_handler;
pub mod material_asset_handler;
pub mod image_asset_handler;
pub mod mesh_asset_handler;

pub type BEADType = json::Value;

pub mod storage_backend;

pub use storage_backend::StorageBackend;
pub use storage_backend::FileStorageBackend;

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
pub fn read_asset_from_source<'a>(url: ResourceId<'a>, base_path: Option<&'a std::path::Path>) -> utils::SendSyncBoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> { Box::pin(async move {
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

			let file = File::open(&path).await;
			let mut file = file.or(Err(()))?;

			spec = {
				// Append ".bead" to the file name to check for a resource file
				let spec_path = path.with_added_extension("bead");
				let file = File::open(spec_path).await.ok();
				if let Some(mut file) = file {
					let mut spec_bytes = Vec::with_capacity(file.metadata().await.unwrap().len() as usize);
					if let Err(_) = file.read_to_end(&mut spec_bytes).await {
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

	Ok((source_bytes.into(), spec, format))
}) }

pub fn get_base<'a>(url: &'a str) -> Option<&'a str> {
	let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	}
	let path = std::path::Path::new(url);
	Some(path.to_str()?)
}

pub fn get_extension<'a>(url: &'a str) -> Option<&'a str> {
    let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	}
	let path = std::path::Path::new(url);
	Some(path.extension()?.to_str()?)
}

fn get_fragment(url: &str) -> Option<&str> {
	let mut split = url.split('#');
	let _ = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	let fragment = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	if split.count() == 0 {
		Some(fragment)
	} else {
		None
	}
}

/// A `ResourceId` encapsulates and provides methods for interacting with a full resource id.
/// A resource id is composed of up to three parts.
/// The base, the extension and the fragment.
///
/// "meshes/Box.gltf#texture"
///
/// "mehses/Box.gltf" is the base
/// "gltf" is the extension
/// "texture" is the fragment
///
/// Fragments like in HTTP urls, allow referencing subresources, they are useful to address elements in container formats.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceId<'a> {
    full: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceIdBase<'a> {
    base: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceIdFragment<'a> {
    fragment: &'a str,
}

impl<'a> ResourceId<'a> {
    pub fn new(full: &'a str) -> Self {
        Self { full }
    }

    pub fn get_base(&self) -> ResourceIdBase<'a> {
        ResourceIdBase { base: get_base(self.full).unwrap() }
    }

    pub fn get_extension(&self) -> &'a str {
        let mut split = self.full.split('#');
    	let url = split.next().unwrap();
    	let path = std::path::Path::new(url);
    	path.extension().unwrap().to_str().unwrap()
    }

    pub fn get_fragment(&self) -> Option<ResourceIdFragment<'a>> {
        get_fragment(self.full).map(|fragment| ResourceIdFragment { fragment })
    }
}

impl Debug for ResourceId<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.full)
    }
}

impl ToString for ResourceId<'_> {
    fn to_string(&self) -> String {
        self.full.to_string()
    }
}

impl AsRef<str> for ResourceId<'_> {
    fn as_ref(&self) -> &str {
        self.full
    }
}

impl Debug for ResourceIdBase<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.base)
    }
}

impl ToString for ResourceIdBase<'_> {
    fn to_string(&self) -> String {
        self.base.to_string()
    }
}

impl AsRef<str> for ResourceIdBase<'_> {
    fn as_ref(&self) -> &str {
        self.base
    }
}

impl Debug for ResourceIdFragment<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.fragment)
    }
}

impl ToString for ResourceIdFragment<'_> {
    fn to_string(&self) -> String {
        self.fragment.to_string()
    }
}

impl AsRef<str> for ResourceIdFragment<'_> {
    fn as_ref(&self) -> &str {
        self.fragment
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
