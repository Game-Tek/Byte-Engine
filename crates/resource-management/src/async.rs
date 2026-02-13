use std::pin::Pin;

pub use compio::fs::read;
pub use compio::fs::File;
pub use compio::runtime::spawn;
pub use compio::runtime::Runtime as Executor;

pub use compio::runtime::spawn_blocking;
pub use spawn_blocking as offload;
pub use spawn_blocking as spawn_cpu_task;

pub use compio::test;

use std::future::Future;

pub fn future<'a, T, F>(f: F) -> BoxedFuture<'a, T>
where
    F: Future<Output = T> + 'a,
{
    Box::pin(f)
}

pub type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
