//! Transitional synchronous access to asynchronous resource requests.
//!
//! Renderer resource adoption is still synchronous. Keep its blocking bridge in
//! one private module so the next asynchronous renderer-loading change can
//! delete it without retaining a public blocking resource interface.

use std::future::Future;

use resource_management::{
	r#async::Executor, Reference, ReferenceModel, Resource, ResourceManager, SerializableResource, Solver,
};

thread_local! {
	// Reuse one local executor per rendering thread instead of constructing a
	// runtime for every resource request during the transition.
	static RESOURCE_REQUEST_EXECUTOR: Executor = Executor::new().expect(
		"Failed to create the renderer resource executor. The most likely cause is that the platform I/O driver could not be initialized."
	);
}

/// Waits for one asynchronous resource request from synchronous renderer code.
pub(crate) fn request<T>(resource_manager: &ResourceManager, id: &str) -> Result<Reference<T>, String>
where
	T: Resource + 'static,
	for<'de> ReferenceModel<T::Model>: Solver<'de, Reference<T>>,
	SerializableResource: TryInto<ReferenceModel<T::Model>>,
{
	block_on(resource_manager.request(id))
}

/// Waits for one resource future while renderer resource adoption remains synchronous.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
	RESOURCE_REQUEST_EXECUTOR.with(|executor| executor.block_on(future))
}
