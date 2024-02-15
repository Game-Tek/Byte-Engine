//! This module contains the asset management system.
//! This system is responsible for loading assets from different sources (network, local, etc.) and generating the resources from them.

use smol::{future::FutureExt, io::AsyncReadExt};

pub mod asset_manager;
pub mod asset_handler;

pub mod audio_asset_handler;
pub mod material_asset_handler;
pub mod image_asset_handler;
pub mod mesh_asset_handler;

/// Loads an asset from source.\
/// Expects an asset name in the form of a path relative to the assets directory, or a network address.\
/// If the asset is not found it will return None.
async fn read_asset_from_source(url: &str, base_path: Option<&std::path::Path>) -> Result<(Vec<u8>, String), ()> {
	let resource_origin = if url.starts_with("http://") || url.starts_with("https://") { "network" } else { "local" };
	let mut source_bytes;
	let format;
	match resource_origin {
		"network" => {
			let request = if let Ok(request) = ureq::get(url).call() { request } else { return Err(()); };
			let content_type = if let Some(e) = request.header("content-type") { e.to_string() } else { return Err(()); };
			format = content_type;

			source_bytes = Vec::new();

			request.into_reader().read_to_end(&mut source_bytes).or(Err(()))?;
		},
		"local" => {
			let path = base_path.ok_or(())?;

			let path = path.join(url);

			let mut file = smol::fs::File::open(&path).await.or(Err(()))?;

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

	Ok((source_bytes, format))
}

pub trait AssetResolver: Sync + Send {
	/// Returns the type of the asset, if attainable from the url.
	/// Can serve as a filter for the asset handler to not attempt to load assets it can't handle.
	fn get_type(&self, url: &str) -> Option<String> {
		let path = std::path::Path::new(url);
		Some(path.extension()?.to_string_lossy().to_string())
	}

	fn resolve<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Vec<u8>, String)>> + Send + 'a>> {
		async move {
			read_asset_from_source(url, None).await.ok()
		}.boxed()
	}
}

#[cfg(test)]
mod tests {
    use smol::future::FutureExt;

    use super::{read_asset_from_source, AssetResolver};

	pub struct TestAssetResolver {

	}

	impl TestAssetResolver {
		pub fn new() -> TestAssetResolver {
			TestAssetResolver {}
		}
	}

	impl AssetResolver for TestAssetResolver {
		fn resolve<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<(Vec<u8>, String)>> + Send + 'a>> {
			async move {
				read_asset_from_source(url, Some(&std::path::Path::new("../assets"))).await.ok()
			}.boxed()
		}
	}
}