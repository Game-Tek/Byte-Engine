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
pub use macos::Handles;
#[cfg(target_os = "macos")]
pub use macos::{App, Window};

use crate::window::{Event, Features, WindowId};

/// The platform connection that owns the process event queue.
pub trait AppLike: Sized {
	type Window: WindowLike;

	/// Connects to the windowing system with the given application ID.
	fn try_new(id_name: &str) -> Result<Self, String>;

	/// Creates a window with the given name, extent, and features.
	fn create_window(&mut self, name: &str, extent: utils::Extent, features: Features) -> Result<Self::Window, String>;

	/// Pumps the native queue and yields every pending event in arrival order.
	fn poll(&mut self) -> impl Iterator<Item = Event> + '_;
}

pub trait WindowLike: Sized {
	fn id(&self) -> WindowId;

	fn handles(&self) -> Handles;
}
