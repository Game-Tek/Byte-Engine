use utils::Extent;

use crate::wayland_window;
use crate::wayland_window::WaylandWindow;
use crate::x11_window::{self, X11Window};

#[derive(Debug, Clone, Copy)]
/// The keys that can be pressed on a keyboard.
pub enum Keys {
	/// The A key.
	A,
	/// The B key.
	B,
	/// The C key.
	C,
	/// The D key.
	D,
	/// The E key.
	E,
	/// The F key.
	F,
	/// The G key.
	G,
	/// The H key.
	H,
	/// The I key.
	I,
	/// The J key.
	J,
	/// The K key.
	K,
	/// The L key.
	L,
	/// The M key.
	M,
	/// The N key.
	N,
	/// The O key.
	O,
	/// The P key.
	P,
	/// The Q key.
	Q,
	/// The R key.
	R,
	/// The S key.
	S,
	/// The T key.
	T,
	/// The U key.
	U,
	/// The V key.
	V,
	/// The W key.
	W,
	/// The X key.
	X,
	/// The Y key.
	Y,
	/// The Z key.
	Z,

	/// The number 0 key.
	Num0,
	/// The number 1 key.
	Num1,
	/// The number 2 key.
	Num2,
	/// The number 3 key.
	Num3,
	/// The number 4 key.
	Num4,
	/// The number 5 key.
	Num5,
	/// The number 6 key.
	Num6,
	/// The number 7 key.
	Num7,
	/// The number 8 key.
	Num8,
	/// The number 9 key.
	Num9,

	/// The numpad 0 key.
	NumPad0,
	/// The numpad 1 key.
	NumPad1,
	/// The numpad 2 key.
	NumPad2,
	/// The numpad 3 key.
	NumPad3,
	/// The numpad 4 key.
	NumPad4,
	/// The numpad 5 key.
	NumPad5,
	/// The numpad 6 key.
	NumPad6,
	/// The numpad 7 key.
	NumPad7,
	/// The numpad 8 key.
	NumPad8,
	/// The numpad 9 key.
	NumPad9,

	/// The numpad add key.
	NumPadAdd,
	/// The numpad subtract key.
	NumPadSubtract,
	/// The numpad multiply key.
	NumPadMultiply,
	/// The numpad divide key.
	NumPadDivide,
	/// The numpad decimal key.
	NumPadDecimal,
	/// The numpad enter key.
	NumPadEnter,

	/// The backspace key.
	Backspace,
	/// The tab key.
	Tab,
	/// The enter key.
	Enter,
	/// The shift left key.
	ShiftLeft,
	/// The shift right key.
	ShiftRight,
	/// The control left key.
	ControlLeft,
	/// The control right key.
	ControlRight,
	/// The alt left key.
	AltLeft,
	/// The alt right key.
	AltRight,
	/// The menu key.
	Menu,
	/// The spacebar key.
	Space,
	/// The insert key.
	Insert,
	/// The delete key.
	Delete,
	/// The home key.
	Home,
	/// The end key.
	End,
	/// The page up key.
	PageUp,
	/// The page down key.
	PageDown,
	/// The arrow up key.
	ArrowUp,
	/// The arrow down key.
	ArrowDown,
	/// The arrow left key.
	ArrowLeft,
	/// The arrow right key.
	ArrowRight,

	/// The escape key.
	Escape,
	/// The F1 key.
	F1,
	/// The F2 key.
	F2,
	/// The F3 key.
	F3,
	/// The F4 key.
	F4,
	/// The F5 key.
	F5,
	/// The F6 key.
	F6,
	/// The F7 key.
	F7,
	/// The F8 key.
	F8,
	/// The F9 key.
	F9,
	/// The F10 key.
	F10,
	/// The F11 key.
	F11,
	/// The F12 key.
	F12,

	/// The num lock key.
	NumLock,
	/// The scroll lock key.
	ScrollLock,
	/// The caps lock key.
	CapsLock,
	/// The print screen key.
	PrintScreen,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseKeys {
	Left,
	Middle,
	Right,
	ScrollUp,
	ScrollDown,
}

#[derive(Debug, Clone, Copy)]
/// The events that can be received from a window.
pub enum WindowEvents {
	/// The window has been resized.
	Resize,
	/// The window has been minimized.
	Minimize,
	/// The window has been maximized.
	Maximize,
	/// The window has been closed.
	Close,
	/// A key has been pressed or released.
	Key {
		pressed: bool,
		key: Keys,
	},
	/// A mouse button has been pressed or released.
	Button {
		pressed: bool,
		button: MouseKeys,
	},
	MouseMove {
		x: u32,
		y: u32,
		time: u64,
	},
}

impl TryFrom<u8> for Keys {
	type Error = ();

	fn try_from(value: u8) -> Result<Self, Self::Error> {
		match value {
			0x26 => Ok(Keys::A), 0x38 => Ok(Keys::B), 0x36 => Ok(Keys::C), 0x28 => Ok(Keys::D), 0x1a => Ok(Keys::E), 0x29 => Ok(Keys::F),
			0x2a => Ok(Keys::G), 0x2b => Ok(Keys::H), 0x1f => Ok(Keys::I), 0x2c => Ok(Keys::J), 0x2d => Ok(Keys::K), 0x2e => Ok(Keys::L),
			0x3a => Ok(Keys::M), 0x39 => Ok(Keys::N), 0x20 => Ok(Keys::O), 0x21 => Ok(Keys::P), 24 => Ok(Keys::Q), 0x1b => Ok(Keys::R),
			0x27 => Ok(Keys::S), 28 => Ok(Keys::T),	30 => Ok(Keys::U), 0x37 => Ok(Keys::V), 25 => Ok(Keys::W), 0x35 => Ok(Keys::X),
			0x1d => Ok(Keys::Y), 0x34 => Ok(Keys::Z),

			90 => Ok(Keys::NumPad0), 87 => Ok(Keys::NumPad1), 88 => Ok(Keys::NumPad2), 89 => Ok(Keys::NumPad3), 0x53 => Ok(Keys::NumPad4),
			0x54 => Ok(Keys::NumPad5), 0x55 => Ok(Keys::NumPad6), 79 => Ok(Keys::NumPad7), 80 => Ok(Keys::NumPad8), 81 => Ok(Keys::NumPad9),

			113 => Ok(Keys::ArrowLeft), 116 => Ok(Keys::ArrowDown), 114 => Ok(Keys::ArrowRight), 111 => Ok(Keys::ArrowUp),
			
			9 => Ok(Keys::Escape), 23 => Ok(Keys::Tab), 50 => Ok(Keys::ShiftLeft), 37 => Ok(Keys::ControlLeft), 64 => Ok(Keys::AltLeft),
			65 => Ok(Keys::Space), 108 => Ok(Keys::AltRight), 105 => Ok(Keys::ControlRight), 62 => Ok(Keys::ShiftRight), 36 => Ok(Keys::Enter),
			22 => Ok(Keys::Backspace),

			_ => Err(()),
		}
	}
}

impl TryFrom<u8> for MouseKeys {
	type Error = ();

	fn try_from(value: u8) -> Result<Self, Self::Error> {
		match value {
			1 => Ok(MouseKeys::Left),
			2 => Ok(MouseKeys::Middle),
			3 => Ok(MouseKeys::Right),
			4 => Ok(MouseKeys::ScrollUp),
			5 => Ok(MouseKeys::ScrollDown),
			_ => Err(()),
		}
	}
}

enum OSWindow {
	#[cfg(target_os = "linux")]
	Wayland(WaylandWindow),
	#[cfg(target_os = "linux")]
	X11(X11Window),
}

pub struct Window {
	pub name: String,
	pub extent: Extent,
	pub id_name: String,
	pub os_window: OSWindow,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Option<Window> {
		Self::new_with_params(name, extent, name)
	}

	pub fn new_with_params(name: &str, extent: Extent, id_name: &str) -> Option<Window> {
		let window_impl = if let Some(_) = std::env::vars().find(|(key, _)| key == "WAYLAND_DISPLAY") {
			if let Some(_) = std::env::vars().find(|(key, value)| key == "XDG_SESSION_TYPE" && value == "wayland") {
				OSWindow::Wayland(WaylandWindow::try_new().ok()?)
			} else {
				OSWindow::X11(X11Window::try_new(name, extent, id_name)?)
			}
		} else {
			OSWindow::X11(X11Window::try_new(name, extent, id_name)?)
		};

		Some(Window {
			name: name.to_owned(),
			extent,
			id_name: id_name.to_owned(),
			os_window: window_impl,
		})
	}

	pub fn poll(&self) -> WindowIterator {
		match self.os_window {
			OSWindow::X11(ref window) => WindowIterator::X11(window.poll()),
			OSWindow::Wayland(ref window) => WindowIterator::Wayland(window.poll()),
		}
	}

	pub fn get_os_handles(&self) -> OSHandles {
		match self.os_window {
			OSWindow::X11(ref window) => OSHandles::X11(window.get_os_handles()),
			OSWindow::Wayland(ref window) => OSHandles::Wayland(window.get_os_handles()),
		}
	}
}

pub enum WindowIterator<'a> {
	#[cfg(target_os = "linux")]
	X11(x11_window::WindowIterator<'a>),
	#[cfg(target_os = "linux")]
	Wayland(wayland_window::WindowIterator<'a>),
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {
		match self {
			#[cfg(target_os = "linux")]
			WindowIterator::X11(window) => window.next(),
			#[cfg(target_os = "linux")]
			WindowIterator::Wayland(window) => window.next(),
		}
	}
}

/// The operating system handles for a window.
pub enum OSHandles {
	#[cfg(target_os = "linux")]
	X11(x11_window::OSHandles),
	#[cfg(target_os = "linux")]
	Wayland(wayland_window::OSHandles),
}