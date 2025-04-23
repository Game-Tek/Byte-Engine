use std::path::PathBuf;

use super::{read_asset_from_source, BEADType, ResourceId};

pub trait StorageBackend: Send + Sync {
	fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> Result<(Box<[u8]>, Option<BEADType>, String), ()> {
		read_asset_from_source(url, None)
    }
}

pub struct FileStorageBackend {
	base_path: PathBuf,
}

impl FileStorageBackend {
	pub fn new(base_path: PathBuf) -> Self {
		Self {
			base_path,
		}
	}
}

impl StorageBackend for FileStorageBackend {
    fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> Result<(Box<[u8]>, Option<BEADType>, String), ()> {
		read_asset_from_source(url, Some(&self.base_path))
    }
}

#[cfg(test)]
pub mod tests {
	use std::{collections::HashMap, io::Read, sync::{Arc, Mutex}};

	use crate::{asset::{BEADType, ResourceId}, tests::ASSETS_PATH};

	use utils::{json, sync::File};

	use super::StorageBackend;
	
	/// A storage backend that can be used for tests.
	/// It allows you to add files to the storage backend. This way you can test custom files without having to create them on the filesystem.
	/// For any requested file that was not "mocked" it will try to read the file from the assets directory.
	#[derive(Clone)]
	pub struct TestStorageBackend(Arc<Mutex<HashMap<String, Box<[u8]>>>>);
	
	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::new())))
		}
	
		pub fn add_file(&self, name: &'static str, data: &[u8]) {
			self.0.lock().unwrap().insert(name.to_string(), data.into());
		}
	}
	
	impl StorageBackend for TestStorageBackend {
		fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> Result<(Box<[u8]>, Option<BEADType>, String), ()> {
			if let Some(data) = self.0.lock().unwrap().get(url.as_ref()) {
				let data = data.clone();
				return Ok((data.clone(), None, url.get_extension().to_string()));
			}
	
			// NOTE: Don't return value from else because it would be a reborrow of self.0.lock().unwrap()
	
			let path = std::path::Path::new(ASSETS_PATH);
			let path = path.join(url.get_base().as_ref());
			
			// Check if the file name exitst in our map
			let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");
	
			let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();
	
			// If case file needs to be looked for in the fs use the real path
			let spec_path = path.with_added_extension("bead");
	
			let file = File::open(&path);
			let mut file = file.or(Err(()))?;
	
			let spec = if let Some(data) = spec_data {
				let spec = std::str::from_utf8(&data).or(Err(()))?;
				let spec: json::Value = json::from_str(spec).or(Err(()))?;
				Some(spec)
			} else {
				let file = File::open(&spec_path).ok();
	
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
	
			let format = path.extension().unwrap().to_str().unwrap().to_string();
	
			let mut source_bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);
	
			if let Err(_) = file.read_to_end(&mut source_bytes) {
				return Err(());
			} else {
				Ok((source_bytes.into(), spec, format))
			}
		}
	}
}