use std::path::PathBuf;

use super::{read_asset_from_source, BEADType, ResourceId};

pub trait StorageBackend: Send + Sync {
	fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> utils::SendSyncBoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
        Box::pin(async move {
            read_asset_from_source(url, None).await
        })
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
    fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> utils::SendSyncBoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
        Box::pin(async move {
            read_asset_from_source(url, Some(&self.base_path)).await
        })
    }
}

#[cfg(test)]
use std::{sync::{Arc, Mutex}, collections::HashMap};

#[cfg(test)]
#[derive(Clone)]
pub struct TestStorageBackend(Arc<Mutex<HashMap<String, Box<[u8]>>>>);

#[cfg(test)]
impl TestStorageBackend {
	pub fn new() -> Self {
		Self(Arc::new(Mutex::new(HashMap::new())))
	}

	pub fn add_file(&self, name: &'static str, data: &[u8]) {
		self.0.lock().unwrap().insert(name.to_string(), data.into());
	}
}

#[cfg(test)]
impl StorageBackend for TestStorageBackend {
	fn resolve<'a>(&'a self, url: ResourceId<'a>,) -> utils::SendSyncBoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
		Box::pin(async move {
			if let Some(data) = self.0.lock().unwrap().get(url.as_ref()) {
				Ok((data.clone(), None, url.get_extension().to_string()))
			} else {
				Err(())
			}
		})
	}
}