//! The window system module implements logic to handle creation and management of OS windows.

use component_derive::component;
use utils::Extent;

use crate::core::{entity::EntityBuilder, listener::{EntitySubscriber, Listener}, orchestrator, Entity, EntityHandle};

/// The window system.
pub struct WindowSystem {
	windows: gxhash::GxHashMap<EntityHandle<Window>, ghi::Window>,
}

impl Entity for WindowSystem {}

/// The handle of a window.
pub struct WindowHandle(u64);

/// We are not using the handles to mutate data, so it is safe to send them between threads.
/// The pointers are only mutable because the XCB library defines them as such.
// #[cfg(target_os = "linux")]
// unsafe impl Send for WindowOsHandles {}

pub struct Window {}
impl Window {
	pub fn new(name: &str, extent: Extent) -> EntityBuilder<'static, Window> {
		EntityBuilder::new(Window {})
	}
}
impl Entity for Window {}

impl WindowSystem {
	/// Creates a new window system.
	pub fn new() -> WindowSystem {
		if let Some(_) = std::env::vars().find(|(key, _)| key == "WAYLAND_DISPLAY") {
			if let Some((_, v)) = std::env::vars().find(|(key, _)| key == "XDG_SESSION_TYPE") {
				if v == "wayland" {
					log::debug!("Wayland detected. Using Wayland backend.");
				} else {
					log::debug!("Wayland detected, but not using Wayland backend. Using XCB backend.");
				}
			}
		}

		WindowSystem { windows: gxhash::GxHashMap::default() }
	}

	pub fn new_as_system<'a>() -> EntityBuilder<'a, WindowSystem> {
		EntityBuilder::new(Self::new()).listen_to::<Window>()
	}

	pub fn update(&mut self) -> bool {
		for (_, window) in &self.windows {
			for event in window.poll() {
				match event {
					ghi::WindowEvents::Close => {
						return false;
					},
					_ => { return true; }
				}
			}
		}

		true
	}

	pub fn update_window(&self, window_handle: &EntityHandle<Window>) -> Option<ghi::WindowEvents> {
		self.windows[window_handle].update()
	}

	pub fn update_windows(&self, mut function: impl FnMut(&EntityHandle<Window>, ghi::WindowEvents)) {
		for (handle, window) in &self.windows {
			for event in window.poll() {
				function(handle, event);
			}
		}
	}

	/// Creates a new OS window.
	/// 
	/// # Arguments
	/// - `name` - The name of the window.
	/// - `extent` - The size of the window.
	/// - `id_name` - The name of the window for identification purposes.
	pub fn create_window(&mut self, window_handle: EntityHandle<Window>, name: &str, extent: Extent, id_name: &str) {
		let window = ghi::Window::new_with_params(name, extent, id_name);

		if let Some(window) = window {
			log::trace!("Created window. Name: {}, Extent: {:?}", name, extent);

			self.windows.insert(window_handle, window);
		} else {
			panic!("Failed to create window")
		}
	}

	pub fn close_window(&mut self, window_handle: &EntityHandle<Window>) {
		self.windows[window_handle].close();
	}

	/// Gets the OS handles for a window.
	/// 
	/// # Arguments
	/// - `window_handle` - The handle of the window to get the OS handles for.
	/// 
	/// # Returns
	/// The operationg system handles for the window.
	pub fn get_os_handles(&self, window_handle: &EntityHandle<Window>,) -> ghi::WindowOsHandles {
		let window = &self.windows[window_handle];
		window.get_os_handles()
	}
}

impl EntitySubscriber<Window> for WindowSystem {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Window>, _window: &Window) -> utils::BoxedFuture<()> {
		let h = self.create_window(handle, "Main Window", Extent::rectangle(1920, 1080), "main_window");

		Box::pin(async move { })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// #[ignore = "Ignore until we have a way to disable this test in CI where windows are not supported"]
	// #[test]
	// fn create_window() {
	// 	let mut window_system = WindowSystem::new();

	// 	let window_handle = window_system.create_window("Main Window", Extent { width: 1920, height: 1080, depth: 1 }, "main_window");

	// 	let os_handles = window_system.get_os_handles(&window_handle);

	// 	assert_ne!(os_handles.xcb_connection, std::ptr::null_mut());
	// 	assert_ne!(os_handles.xcb_window, 0);
	// }

	// #[test]
	// fn test_window_loop() {
	// 	let mut window_system = WindowSystem::new();

	// 	let window_handle = window_system.create_window("Main Window", Extent { width: 1920, height: 1080, depth: 1 }, "main_window");

	// 	std::thread::sleep(std::time::Duration::from_millis(500));

	// 	window_system.close_window(&window_handle);
	// }
}