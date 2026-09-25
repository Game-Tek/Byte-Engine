#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::Handles;
#[cfg(target_os = "linux")]
pub use wayland::{App, Window};

#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "windows")]
pub use win32::Handles;
#[cfg(target_os = "windows")]
pub use win32::{App, Window};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::AppWaker;
#[cfg(target_os = "macos")]
pub use macos::Handles;
#[cfg(target_os = "macos")]
pub use macos::{App, Window};
#[cfg(target_os = "linux")]
pub use wayland::AppWaker;
#[cfg(target_os = "windows")]
pub use win32::AppWaker;

use crate::window::{Event, Features, Wait, WindowId};

/// The platform connection that owns the process event queue.
pub trait AppLike: Sized {
	type Window: WindowLike;

	/// Connects to the windowing system with the given application ID.
	fn try_new(id_name: &str) -> Result<Self, String>;

	/// Creates a window with the given name, extent, and features.
	fn create_window(&mut self, name: &str, extent: utils::Extent, features: Features) -> Result<Self::Window, String>;

	/// Waits as `wait` allows for the first event, then pumps the native queue and yields every pending event in
	/// arrival order.
	fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_;

	/// Returns a handle that interrupts a waiting [`Self::poll`] from any thread.
	fn waker(&self) -> AppWaker;
}

pub trait WindowLike: Sized {
	fn id(&self) -> WindowId;

	fn handles(&self) -> Handles;

	/// Returns the refresh interval of the display the window is on, when the platform reports it.
	fn refresh_interval(&self) -> Option<std::time::Duration>;
}
