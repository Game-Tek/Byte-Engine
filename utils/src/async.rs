use std::future::Future;
use std::sync::Arc;

pub use futures::future::join_all;
pub use futures::future::try_join_all;

use gxhash::HashMap;
use gxhash::HashMapExt;
pub use tokio::spawn;
pub use tokio::task::spawn_blocking;
pub use tokio::task::block_in_place as spawn_blocking_local;

pub use tokio::fs::File;
pub use tokio::io::AsyncReadExt;
pub use tokio::io::AsyncWriteExt;
pub use tokio::io::AsyncSeekExt;

pub use tokio::fs::remove_file;

pub use tokio::sync::Mutex;

pub use tokio::sync::RwLock;
pub use tokio::sync::RwLockReadGuard;
pub use tokio::sync::RwLockWriteGuard;

pub use tokio::sync::OnceCell;

pub use tokio::runtime::Runtime;

pub fn block_on<F>(future: F) -> F::Output where F: Future {
	tokio::runtime::Handle::current().block_on(future)
}

pub fn create_runtime() -> tokio::runtime::Runtime {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.unwrap()
}

pub struct AsyncCacheMap<K: Eq + std::hash::Hash, V> {
	cache: RwLock<HashMap<K, Arc<OnceCell<V>>>>,
}

impl<K: Eq + std::hash::Hash, V> AsyncCacheMap<K, V> {
	pub fn new() -> Self {
		Self {
			cache: RwLock::new(HashMap::new()),
		}
	}

	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			cache: RwLock::new(HashMap::with_capacity(capacity)),
		}
	}

	pub async fn get_or_insert_with<F, R>(&self, key: K, f: F) -> () where F: FnOnce() -> R, R: Future<Output = V> {
		let mut cache = self.cache.write().await;

		let v = cache.entry(key).or_insert_with(|| Arc::new(OnceCell::new()));

		let r = v.get_or_init(f).await;
	}
}