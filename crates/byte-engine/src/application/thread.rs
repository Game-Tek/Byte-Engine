//! Worker-thread support for application-owned subsystems.
//!
//! Use [`Thread`] for workers that must stop with the application. The standard
//! audio and Art-Net workers in [`crate::application::graphics`] show how to
//! provide a receiver from the application event bus.

use super::{Receiver, Sender};
use crate::application::Events;

/// The [`Thread`] struct owns a worker that participates in application shutdown.
pub struct Thread {
	handle: std::thread::JoinHandle<()>,
}

impl Thread {
	pub fn new<F>(rx: Receiver<Events>, f: F) -> Self
	where
		F: FnOnce(Receiver<Events>) + Send + 'static,
	{
		let handle = std::thread::spawn(move || f(rx));
		Self { handle }
	}

	pub fn join(self) -> std::thread::Result<()> {
		self.handle.join()
	}
}
