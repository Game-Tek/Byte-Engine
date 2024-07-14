use std::future::Future;

pub use futures::future::join_all;

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