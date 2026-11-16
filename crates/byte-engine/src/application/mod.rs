//! Application lifecycle and composition.
//!
//! Use [`BaseApplication`] when building a custom runtime that only needs parameter
//! handling and frame-local storage. Headed applications should start with
//! `GraphicsApplication` and choose either `default_setup` or individual
//! graphics setup functions.
//!
//! The `triangle` example demonstrates the standard setup path, while the
//! `window` example demonstrates selecting only one setup component.

use crate::time::MediaTime;

pub mod application;
// Only the headed console reads the kept lines, and a Tracy build sends records to Tracy instead.
#[cfg_attr(any(not(feature = "headed"), feature = "tracy"), allow(dead_code))]
pub(crate) mod log_capture;

pub mod thread;
pub mod waker;
pub use application::BaseApplication;
// The parameters live with the asset pipeline, so BELD resolves them the same way without the engine. Inline them so
// the engine's API documents them where applications read them.
#[doc(inline)]
pub use resource_management::parameters::{self, Parameter, ParameterError};
pub use waker::LoopWaker;

#[cfg(feature = "headed")]
pub mod graphics;

/// The [`Time`] struct provides frame timing to application systems without exposing
/// the lifecycle clock owned by the application.
#[derive(Debug, Clone, Copy)]
pub struct Time {
	elapsed: MediaTime,
	delta: MediaTime,
}

impl Time {
	/// Creates frame timing data for systems that run inside an application tick.
	pub fn new(elapsed: MediaTime, delta: MediaTime) -> Self {
		Self { elapsed, delta }
	}

	/// Returns the total time since the application started.
	pub fn elapsed(&self) -> MediaTime {
		self.elapsed
	}

	/// Returns the time since the previous application tick.
	pub fn delta(&self) -> MediaTime {
		self.delta
	}
}

/// The [`Events`] enum defines messages shared with application-owned worker threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Events {
	/// Requests that the application close.
	Close,
}
