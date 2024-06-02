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
    use std::{collections::HashMap, sync::{Arc, Mutex}};

    use crate::{asset::get_fragment, resource::{resource_handler::ResourceReader, tests::TestResourceReader}, GenericResourceResponse, GenericResourceSerialization, StorageBackend};

    use super::{get_base, read_asset_from_source, BEADType};

	pub struct TestStorageBackend {
		resources: Arc<Mutex<Vec<(GenericResourceSerialization, Box<[u8]>)>>>,
		files: Arc<Mutex<HashMap<&'static str, Box<[u8]>>>>,
	}

	impl TestStorageBackend {
		pub fn new() -> TestStorageBackend {
			TestStorageBackend {
				resources: Arc::new(Mutex::new(Vec::new())),
				files: Arc::new(Mutex::new(HashMap::new())),
			}
		}

		pub fn add_file(&self, name: &'static str, data: &[u8]) {
			self.files.lock().unwrap().insert(name, data.into());
		}

		pub fn get_resources(&self) -> Vec<GenericResourceSerialization> {
			self.resources.lock().unwrap().iter().map(|x| x.0.clone()).collect()
		}

		pub fn get_resource_data_by_name(&self, name: &str) -> Option<Box<[u8]>> {
			Some(self.resources.lock().unwrap().iter().find(|x| x.0.id == name)?.1.clone())
		}
	}

	impl StorageBackend for TestStorageBackend {
		fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>> {
			let resources = self.resources.lock().unwrap();
			let mut names = Vec::with_capacity(resources.len());
			for resource in resources.iter() {
				names.push(resource.0.id.clone());
			}

			Box::pin(async move {
				Ok(names)
			})
		}

		fn delete<'a>(&'a self, id: &'a str) -> utils::BoxedFuture<'a, Result<(), String>> {
			let mut resources = self.resources.lock().unwrap();
			let mut index = None;
			for (i, resource) in resources.iter().enumerate() {
				if resource.0.id == id {
					index = Some(i);
					break;
				}
			}

			if let Some(i) = index {
				resources.remove(i);
				Box::pin(async move {
					Ok(())
				})
			} else {
				Box::pin(async move {
					Err("Resource not found".to_string())
				})
			}
		}
		
		fn store<'a>(&'a self, resource: &GenericResourceSerialization, data: &[u8]) -> utils::SendSyncBoxedFuture<'a, Result<(), ()>> {
			self.resources.lock().unwrap().push((resource.clone(), data.into()));

			Box::pin(async move {
				Ok(())
			})
		}

		fn read<'s, 'a, 'b>(&'s self, id: &'b str) -> utils::SendSyncBoxedFuture<'a, Option<(GenericResourceResponse<'a>, Box<dyn ResourceReader>)>> {
			let mut x = None;

			let resources = self.resources.lock().unwrap();
			for (resource, data) in resources.iter() {
				if resource.id == id {
					// TODO: use actual hash
					x = Some((GenericResourceResponse::new(id, 0, resource.class.clone(), data.len(), resource.resource.clone()), Box::new(TestResourceReader::new(data.clone())) as Box<dyn ResourceReader>));
					break;
				}
			}

			Box::pin(async move {
				x
			})
		}

		fn resolve<'a>(&'a self, url: &'a str) -> utils::SendSyncBoxedFuture<'a, Result<(Vec<u8>, Option<BEADType>, String), ()>> { Box::pin(async {
			if let Ok(x) = read_asset_from_source(get_base(url).ok_or(())?, Some(&std::path::Path::new("../assets"))).await {
				let bead = if let None = x.1 {
					let mut url = get_base(url).ok_or(())?.to_string();
					url.push_str(".bead");
					if let Some(spec) = self.files.lock().unwrap().get(url.as_str()) {
						Some(json::parse(std::str::from_utf8(spec).unwrap()).unwrap())
					} else {
						None
					}
				} else {
					x.1
				};
				Ok((x.0, bead, x.2))
			} else {
				let m = if let Some(f) = self.files.lock().unwrap().get(url) {
					f.to_vec()
				} else {
					return Err(());
				};

				let bead = {
					let mut url = get_base(url).ok_or(())?.to_string();
					url.push_str(".bead");
					if let Some(spec) = self.files.lock().unwrap().get(url.as_str()) {
						Some(json::parse(std::str::from_utf8(spec).unwrap()).unwrap())
					} else {
						None
					}
				};

				// Extract extension from url
				Ok((m, bead, url.split('.').last().unwrap().to_string()))
			}
		}) }
	}

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