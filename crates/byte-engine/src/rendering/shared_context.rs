//! Shared ownership of the GHI context between the render thread and resource loader threads.

use std::sync::{Arc, Mutex, MutexGuard};

/// The `SharedContext` struct grants several threads exclusive turns on one GHI context.
///
/// The render thread holds the context for the length of a frame. A loader thread creates its GPU objects
/// through a detached [`ghi::implementation::Factory`], which needs no context at all, and takes the context
/// only to intern those objects and record their transfers. Keep every guard as short as the work it covers:
/// a guard held across an await point stalls every other thread for that duration.
///
/// A context is `Send` but not `Sync`, because every operation on it needs `&mut`. Exclusion is therefore the
/// only sharing discipline available, and the mutex is what makes the handle shareable at all.
#[derive(Clone)]
pub struct SharedContext(Arc<Mutex<ghi::implementation::Context>>);

impl SharedContext {
	/// Takes shared ownership of a context.
	#[must_use]
	pub fn new(context: ghi::implementation::Context) -> Self {
		Self(Arc::new(Mutex::new(context)))
	}

	/// Takes the context for creation, recording, interning, and submission.
	///
	/// A panicking holder poisons nothing here: the renderer's own panic ends the process, so a poisoned lock
	/// is recovered rather than propagated.
	pub fn lock(&self) -> MutexGuard<'_, ghi::implementation::Context> {
		self.0.lock().unwrap_or_else(|error| error.into_inner())
	}
}
