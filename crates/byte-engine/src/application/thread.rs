//! Worker-thread support for application-owned subsystems.
//!
//! Use [`Thread`] for workers that must stop with the application. The standard
//! headed workers show how to provide a listener from the application event bus.

use crate::application::Events;
use crate::core::listener::DefaultListener;

/// The [`Thread`] struct owns a worker that participates in application shutdown.
pub struct Thread {
	handle: std::thread::JoinHandle<()>,
}

impl Thread {
	/// Starts an application-owned worker that receives shutdown events.
	pub fn new<F>(events: DefaultListener<Events>, f: F) -> Self
	where
		F: FnOnce(DefaultListener<Events>) + Send + 'static,
	{
		let handle = std::thread::spawn(move || f(events));
		Self { handle }
	}

	/// Waits for the worker to finish during application shutdown.
	pub fn join(self) -> std::thread::Result<()> {
		self.handle.join()
	}
}
