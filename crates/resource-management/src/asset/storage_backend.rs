use std::{
	alloc::Allocator,
	path::PathBuf,
};

use super::{read_asset_from_source, read_asset_from_source_in, BEADType, ResourceId};
use crate::r#async::{future, BoxedFuture};

pub trait StorageBackend: Send + Sync {
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
		future(read_asset_from_source(url, None))
	}

	/// Resolves an asset while using the provided allocator for source bytes.
	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> BoxedFuture<'a, Result<(Box<[u8], &'a dyn Allocator>, Option<BEADType>, String), ()>> {
		future(read_asset_from_source_in(url, None, allocator))
	}
}

pub struct FileStorageBackend {
	base_path: PathBuf,
}

impl FileStorageBackend {
	pub fn new(base_path: PathBuf) -> Self {
		std::fs::create_dir_all(&base_path).expect("Failed to create base path");

		Self { base_path }
	}
}

impl StorageBackend for FileStorageBackend {
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
		future(read_asset_from_source(url, Some(&self.base_path)))
	}

	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> BoxedFuture<'a, Result<(Box<[u8], &'a dyn Allocator>, Option<BEADType>, String), ()>> {
		future(read_asset_from_source_in(url, Some(&self.base_path), allocator))
	}
}

fn move_bytes_in<A: Allocator + Clone>(bytes: impl AsRef<[u8]>, allocator: A) -> Box<[u8], A> {
	let bytes = bytes.as_ref();
	let mut output = Vec::with_capacity_in(bytes.len(), allocator);
	output.extend_from_slice(bytes);
	output.into_boxed_slice()
}

#[cfg(test)]
pub mod tests {
	use std::{
		alloc::Allocator,
		collections::HashMap,
		sync::{Arc, Mutex},
	};

	use utils::json;

	use super::StorageBackend;
	use crate::{
		asset::{BEADType, ResourceId},
		r#async::{read, BoxedFuture},
		tests::ASSETS_PATH,
	};

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
		fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> {
			Box::pin(async move {
				if let Some(data) = self.0.lock().unwrap().get(url.as_ref()).cloned() {
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

				let spec = if let Some(data) = spec_data {
					let spec = std::str::from_utf8(&data).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
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

				Ok((source_bytes.into_boxed_slice(), spec, format))
			})
		}

		fn resolve_in<'a>(
			&'a self,
			url: ResourceId<'a>,
			allocator: &'a dyn Allocator,
		) -> BoxedFuture<'a, Result<(Box<[u8], &'a dyn Allocator>, Option<BEADType>, String), ()>> {
			Box::pin(async move {
				if let Some(data) = self.0.lock().unwrap().get(url.as_ref()).cloned() {
					return Ok((super::move_bytes_in(data, allocator), None, url.get_extension().to_string()));
				}

				// NOTE: Don't return value from else because it would be a reborrow of self.0.lock().unwrap()

				let path = std::path::Path::new(ASSETS_PATH);
				let path = path.join(url.get_base().as_ref());

				// Check if the file name exists in our map.
				let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");

				let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();

				// If the file needs to be looked for in the fs use the real path.
				let spec_path = path.with_added_extension("bead");

				let spec = if let Some(data) = spec_data {
					let spec = std::str::from_utf8(&data).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
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

				Ok((super::move_bytes_in(source_bytes, allocator), spec, format))
			})
		}
	}
}
