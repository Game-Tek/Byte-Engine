pub use ghi::window::Pointer;
use utils::Extent;

use crate::core::{Entity, factory::Handle};

#[derive(Clone)]
/// The `Window` struct configures a named render surface and its attached camera.
pub struct Window {
	name: String,
	extent: Extent,
	camera: Option<Handle>,
	features: Features,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Self {
		Window {
			name: name.to_string(),
			extent,
			camera: None,
			features: Features::empty(),
		}
	}

	pub fn with_features(self, features: Features) -> Self {
		Self { features, ..self }
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}

	pub fn features(&self) -> Features {
		self.features
	}

	pub fn with_feature(self, feature: Features) -> Self {
		Self {
			features: self.features | feature,
			..self
		}
	}

	pub fn without_feature(self, feature: Features) -> Self {
		Self {
			features: self.features & !feature,
			..self
		}
	}

	pub fn attach(&mut self, camera: Handle) {
		self.camera = Some(camera);
	}

	pub fn camera(&self) -> Option<&Handle> {
		self.camera.as_ref()
	}
}

impl Entity for Window {}

/// The `SetWindowFeatures` struct changes the features of an open window, so an application can switch modes without
/// opening another window, such as an editor freeing the cursor a game captures.
///
/// Send it on the [`crate::application::graphics::GraphicsApplication::messages`] channel with the handle the window was
/// created with. The renderer applies it before the next frame. Only [`Features::CAPTURE_CURSOR`] changes at runtime;
/// see [window features](/docs/reference/rendering#windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetWindowFeatures {
	window: Handle,
	features: Features,
}

impl SetWindowFeatures {
	/// Gives the window created with `window` the features `features`.
	pub fn new(window: Handle, features: Features) -> Self {
		Self { window, features }
	}

	/// The handle the window was created with.
	pub fn window(&self) -> Handle {
		self.window
	}

	/// The features the window takes.
	pub fn features(&self) -> Features {
		self.features
	}
}

/// The `SetWindowPointer` struct changes the system pointer shape an open window shows, so an interface can tell the
/// user what a press does where the pointer is, such as a two-headed arrow over a splitter.
///
/// Send it on the [`crate::application::graphics::GraphicsApplication::messages`] channel with the handle the window was
/// created with. The renderer applies it before the next frame. A UI host sends the shape
/// [`crate::ui::Engine::take_pointer_change`] reports; see [pointer shapes](/docs/reference/rendering#pointer-shapes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetWindowPointer {
	window: Handle,
	pointer: Pointer,
}

impl SetWindowPointer {
	/// Makes the window created with `window` show `pointer`.
	pub fn new(window: Handle, pointer: Pointer) -> Self {
		Self { window, pointer }
	}

	/// The handle the window was created with.
	pub fn window(&self) -> Handle {
		self.window
	}

	/// The shape the window shows.
	pub fn pointer(&self) -> Pointer {
		self.pointer
	}
}

/// The `SetWindowCamera` struct changes the camera an open window shows, so an application can look through another
/// camera without opening another window, such as an editor flying its own camera over the game's.
///
/// Send it on the [`crate::application::graphics::GraphicsApplication::messages`] channel with the handle the window was
/// created with. The renderer applies it before the next frame, to a window created with a camera; see
/// [windows](/docs/reference/rendering#windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetWindowCamera {
	window: Handle,
	camera: Handle,
}

impl SetWindowCamera {
	/// Makes the window created with `window` show `camera`.
	pub fn new(window: Handle, camera: Handle) -> Self {
		Self { window, camera }
	}

	/// The handle the window was created with.
	pub fn window(&self) -> Handle {
		self.window
	}

	/// The camera the window shows.
	pub fn camera(&self) -> Handle {
		self.camera
	}
}

/// The `CloseWindow` struct closes an open window from the application, such as a tool closing its own window.
///
/// Send it on the [`crate::application::graphics::GraphicsApplication::messages`] channel with the handle the window was
/// created with. Closing the main window, the first one opened, quits the application, as its close button does; any
/// other window closes alone before the next frame. See [windows](/docs/reference/rendering#windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseWindow {
	window: Handle,
}

impl CloseWindow {
	/// Closes the window created with `window`.
	pub fn new(window: Handle) -> Self {
		Self { window }
	}

	/// The handle the window was created with.
	pub fn window(&self) -> Handle {
		self.window
	}
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	/// Bit flags for the features of a window.
	pub struct Features : u32 {
		/// The window has decorations (title bar, border, etc.).
		const DECORATIONS = 0b0001;
		/// The window hides the cursor and holds it at its center while it has focus, so the mouse moves without end and
		/// `Mouse.Movement` keeps reporting motion. Use it for camera control; see
		/// [cursor capture](/docs/reference/input#cursor-capture).
		const CAPTURE_CURSOR = 0b0010;
	}
}
