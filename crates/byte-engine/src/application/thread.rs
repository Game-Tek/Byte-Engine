//! Worker-thread support for application-owned subsystems.
//!
//! Use [`Thread`] for workers that must stop with the application. The standard
//! headed workers show how to provide a receiver from the application event bus.

use super::{Receiver, Sender};
use crate::application::Events;

/// The [`Thread`] struct owns a worker that participates in application shutdown.
pub struct Thread {
	handle: std::thread::JoinHandle<()>,
}

impl Thread {
	/// Starts an application-owned worker that receives shutdown events.
	pub fn new<F>(rx: Receiver<Events>, f: F) -> Self
	where
		F: FnOnce(Receiver<Events>) + Send + 'static,
	{
		let handle = std::thread::spawn(move || f(rx));
		Self { handle }
	}

	/// Waits for the worker to finish during application shutdown.
	pub fn join(self) -> std::thread::Result<()> {
		self.handle.join()
	}
}
