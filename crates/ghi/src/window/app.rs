use utils::Extent;

use crate::window::{
	Event, Features, Wait, Window,
	os::{self, AppLike as _},
};

/// The `App` struct exists because platforms deliver events for the whole process through one queue.
/// Keep one per process, create every [`Window`] from it, and drain it once per frame with [`App::poll`].
///
/// Drop every [`Window`] before the `App` that created it.
pub struct App {
	os_app: os::App,
}

impl App {
	/// Connects to the platform windowing system. `id_name` identifies the application to the OS.
	///
	/// Next, call [`App::create_window`].
	pub fn new(id_name: &str) -> Result<App, String> {
		Ok(App {
			os_app: os::App::try_new(id_name)?,
		})
	}

	/// Creates and shows a window.
	///
	/// Next, pass [`Window::os_handles`] to [`crate::context::Context::bind_to_window`] and match
	/// [`Window::id`] against the events from [`App::poll`].
	pub fn create_window(&mut self, name: &str, extent: Extent, features: Features) -> Result<Window, String> {
		let os_window = self.os_app.create_window(name, extent, features)?;
		Ok(Window::new(name, extent, os_window))
	}

	/// Waits as `wait` allows for the first event, then drains every pending application and window event.
	///
	/// Pass [`Wait::Immediate`] to only drain. A waiting call also returns when an [`AppWaker`] from
	/// [`Self::waker`] wakes the app, which then yields no event.
	pub fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		self.os_app.poll(wait)
	}

	/// Returns a handle other threads use to interrupt a waiting [`Self::poll`].
	pub fn waker(&self) -> AppWaker {
		AppWaker(self.os_app.waker())
	}
}

/// The `AppWaker` struct interrupts a waiting [`App::poll`] from any thread.
///
/// Waking an app that is not waiting makes its next waiting poll return at once. Repeated wakes before a poll
/// coalesce into one.
#[derive(Clone)]
pub struct AppWaker(os::AppWaker);

impl AppWaker {
	/// Makes a waiting [`App::poll`] return.
	pub fn wake(&self) {
		self.0.wake();
	}
}
