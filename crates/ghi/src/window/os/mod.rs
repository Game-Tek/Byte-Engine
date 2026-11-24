// The callback block is shared by the backends whose operating system calls back with a raw context pointer.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod callbacks;

#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::{App, AppWaker, Handles};

#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "windows")]
pub use win32::{App, AppWaker, Handles};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::{App, AppWaker, Handles};
