#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::{App, AppWaker, Handles, Window};

#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "windows")]
pub use win32::{App, AppWaker, Handles, Window};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::{App, AppWaker, Handles, Window};
