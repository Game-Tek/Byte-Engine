#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::Window;
#[cfg(target_os = "linux")]
pub use wayland::Handles;

#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "windows")]
pub use win32::Window;
#[cfg(target_os = "windows")]
pub use win32::Handles;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::Window;
#[cfg(target_os = "macos")]
pub use macos::Handles;

use crate::Events;

pub trait WindowLike: Sized {
	/// Create a new window with the given name, extent, and id name.
	fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Result<Self, String>;

	fn poll<'a>(&'a mut self) -> impl Iterator<Item = Events> + 'a;

	fn handles(&self) -> Handles;
}
