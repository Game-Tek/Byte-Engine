use super::{Receiver, Sender};
use crate::application::Events;

/// An application thread that receives events from the application bus.
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
