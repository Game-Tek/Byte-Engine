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

#[cfg(test)]
mod tests {
	use std::{sync::mpsc, time::Duration};

	use super::Thread;
	use crate::{
		application::Events,
		core::{
			channel::{Channel as _, DefaultChannel},
			listener::Listener as _,
		},
	};

	/// Verifies one lifecycle publication reaches every registered worker.
	#[test]
	fn workers_share_the_shutdown_broadcast() {
		let events = DefaultChannel::new();
		let (completed, completions) = mpsc::channel();
		let spawn_worker = |listener, completed: mpsc::Sender<()>| {
			Thread::new(listener, move |mut events| {
				loop {
					if matches!(events.read(), Some(Events::Close)) {
						completed.send(()).expect("report worker shutdown");
						return;
					}
					std::thread::yield_now();
				}
			})
		};
		let first = spawn_worker(events.listener(), completed.clone());
		let second = spawn_worker(events.listener(), completed);

		events.send(Events::Close);

		completions
			.recv_timeout(Duration::from_secs(1))
			.expect("first worker shutdown");
		completions
			.recv_timeout(Duration::from_secs(1))
			.expect("second worker shutdown");
		first.join().expect("join first worker");
		second.join().expect("join second worker");
	}
}
