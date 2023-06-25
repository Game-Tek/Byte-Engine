//! The window system module implements logic to handle creation and management of OS windows.

use std::ffi::c_void;

use xcb::{Xid, x};

use crate::{Extent, orchestrator::System};

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
enum MouseKeys {
	Left,
	Middle,
	Right,
	ScrollUp,
	ScrollDown,
}

#[derive(Debug, Clone, Copy)]
/// The events that can be received from a window.
enum WindowEvents {
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
	}
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

struct Window {
	connection: xcb::Connection,
	window: xcb::x::Window,
	wm_del_window: xcb::x::Atom,
}

struct WindowIterator<'a> {
	connection: &'a xcb::Connection,
	wm_del_window: xcb::x::Atom,
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {
		let connection = &self.connection;

		loop {
			let event = connection.poll_for_event();

			let event = if let Ok(event) = event {
				event
			} else {
				return None;
			};

			let event = if let Some(event) = event {
				event
			} else {
				return None;
			};

			let ev = match event {
				xcb::Event::X(x::Event::KeyPress(ev)) => {
					let key: Result<Keys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Key { pressed: true, key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::KeyRelease(ev)) => {
					let key: Result<Keys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						println!("release {:?}", key);
						Some(WindowEvents::Key { pressed: false, key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ButtonPress(ev)) => {
					let key: Result<MouseKeys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Button { pressed: true, button: key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ButtonRelease(ev)) => {
					let key: Result<MouseKeys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Button { pressed: false, button: key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ClientMessage(ev)) => {
					// We have received a message from the server
					if let x::ClientMessageData::Data32([atom, ..]) = ev.data() {
						if atom == self.wm_del_window.resource_id() {
							let event = WindowEvents::Close;
							println!("Window event: {:?}", event);
							Some(event)
						} else {
							None
						}
					} else {
						None
					}
				},
				_ => { None }
			};

			if let Some(ev) = ev {
				return Some(ev);
			} else {
				continue;
			}
		}
	}
}

impl Window {
	pub fn new_with_params(name: &str, extent: Extent, id_name: &str) -> Option<Window> {
		let (connection, screen_index) = xcb::Connection::connect(None).unwrap();

		let setup = connection.get_setup();
		let screen = setup.roots().nth(screen_index as usize).unwrap();

		let window: xcb::x::Window = connection.generate_id();

		// We can now create a window. For this we pass a `Request`
		// object to the `send_request_checked` method. The method
		// returns a cookie that will be used to check for success.
		let cookie = connection.send_request_checked(&xcb::x::CreateWindow {
			depth: xcb::x::COPY_FROM_PARENT as u8,
			wid: window,
			parent: screen.root(),
			x: 0,
			y: 0,
			width: extent.width as u16, height: extent.height as u16,
			border_width: 0,
			class: xcb::x::WindowClass::InputOutput,
			visual: screen.root_visual(),
			// this list must be in same order than `Cw` enum order
			value_list: &[
				xcb::x::Cw::BackPixel(screen.black_pixel()),
				xcb::x::Cw::EventMask(xcb::x::EventMask::EXPOSURE | xcb::x::EventMask::BUTTON_PRESS | xcb::x::EventMask::BUTTON_RELEASE | xcb::x::EventMask::POINTER_MOTION | xcb::x::EventMask::ENTER_WINDOW | xcb::x::EventMask::LEAVE_WINDOW | xcb::x::EventMask::KEY_PRESS | xcb::x::EventMask::KEY_RELEASE | xcb::x::EventMask::RESIZE_REDIRECT | xcb::x::EventMask::STRUCTURE_NOTIFY),
			],
		});

		if let Err(_) = connection.check_request(cookie) {
			return None;
		}

		// Let's change the window title
		let cookie = connection.send_request_checked(&xcb::x::ChangeProperty {
			mode: xcb::x::PropMode::Replace,
			window,
			property: xcb::x::ATOM_WM_NAME,
			r#type: xcb::x::ATOM_STRING,
			data: name.as_bytes(),
		});

		if let Err(_) = connection.check_request(cookie) {
			return None;
		}

		// We need a few atoms for our application.
		// We send a few requests in a row and wait for the replies after.
		let (wm_protocols, wm_del_window, _wm_state, _wm_state_maxv, _wm_state_maxh) = {
			let cookies = (
				connection.send_request(&xcb::x::InternAtom {
					only_if_exists: true,
					name: b"WM_PROTOCOLS",
				}),
				connection.send_request(&xcb::x::InternAtom {
					only_if_exists: true,
					name: b"WM_DELETE_WINDOW",
				}),
				connection.send_request(&xcb::x::InternAtom {
					only_if_exists: true,
					name: b"_NET_WM_STATE",
				}),
				connection.send_request(&xcb::x::InternAtom {
					only_if_exists: true,
					name: b"_NET_WM_STATE_MAXIMIZED_VERT",
				}),
				connection.send_request(&xcb::x::InternAtom {
					only_if_exists: true,
					name: b"_NET_WM_STATE_MAXIMIZED_HORZ",
				}),
			);
			(
				connection.wait_for_reply(cookies.0).unwrap().atom(),
				connection.wait_for_reply(cookies.1).unwrap().atom(),
				connection.wait_for_reply(cookies.2).unwrap().atom(),
				connection.wait_for_reply(cookies.3).unwrap().atom(),
				connection.wait_for_reply(cookies.4).unwrap().atom(),
			)
		};

		// We now activate the window close event by sending the following request.
		// If we don't do this we can still close the window by clicking on the "x" button,
		// but the event loop is notified through a connection shutdown error.
		let cookie = connection.send_request_checked(&xcb::x::ChangeProperty {
			mode: xcb::x::PropMode::Replace,
			window,
			property: wm_protocols,
			r#type: xcb::x::ATOM_ATOM,
			data: &[wm_del_window],
		});

		if let Err(_) = connection.check_request(cookie) {
			return None;
		}

		// We now show ("map" in X terminology) the window.
		// This time we do not check for success, so we discard the cookie.
		connection.send_request(&xcb::x::MapWindow { window, });

		// Previous request was checked, so a flush is not necessary in this case.
		// Otherwise, here is how to perform a connection flush.
		let flush_result = connection.flush();

		if let Err(_) = flush_result {
			return None;
		}

		Some(Window {
			connection,
			window,
			wm_del_window,
		})
	}

	pub fn poll(&self) -> WindowIterator {
		WindowIterator {
			connection: &self.connection,
			wm_del_window: self.wm_del_window,
		}
	}

	pub fn update(&self) -> Option<WindowEvents> {
		let connection = &self.connection;

		loop {
			let event = connection.poll_for_event();

			let event = if let Ok(event) = event {
				event
			} else {
				return None;
			};

			let event = if let Some(event) = event {
				event
			} else {
				return None;
			};

			let ev = match event {
				xcb::Event::X(x::Event::KeyPress(ev)) => {
					let key: Result<Keys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Key { pressed: true, key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::KeyRelease(ev)) => {
					let key: Result<Keys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						println!("release {:?}", key);
						Some(WindowEvents::Key { pressed: false, key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ButtonPress(ev)) => {
					let key: Result<MouseKeys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Button { pressed: true, button: key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ButtonRelease(ev)) => {
					let key: Result<MouseKeys, _> = ev.detail().try_into();

					if let Ok(key) = key {
						Some(WindowEvents::Button { pressed: false, button: key })
					} else {
						None
					}
				},
				xcb::Event::X(x::Event::ClientMessage(ev)) => {
					// We have received a message from the server
					if let x::ClientMessageData::Data32([atom, ..]) = ev.data() {
						if atom == self.wm_del_window.resource_id() {
							let event = WindowEvents::Close;
							println!("Window event: {:?}", event);
							Some(event)
						} else {
							None
						}
					} else {
						None
					}
				},
				_ => { None }
			};

			if let Some(ev) = ev {
				return Some(ev);
			} else {
				continue;
			}
		}
	}
}

/// The window system.
pub struct WindowSystem {
	windows: Vec<Window>,
}

impl System for WindowSystem {
	fn as_any(&self) -> &dyn std::any::Any { self }
}

/// The handle of a window.
pub struct WindowHandle(u64);

/// The operating system handles for a window.
pub struct WindowOsHandles {
	#[cfg(target_os = "linux")]
	/// The XCB connection.
	pub xcb_connection: *mut c_void,
	#[cfg(target_os = "linux")]
	/// The XCB window.
	pub xcb_window: u32,
}

impl WindowSystem {
	/// Creates a new window system.
	pub fn new() -> WindowSystem {
		WindowSystem { windows: Vec::new() }
	}

	pub fn update(&mut self) -> bool {
		for window in &self.windows {
			for event in window.poll() {
				println!("event {:?}", event);

				match event {
					WindowEvents::Close => {
						return false;
					},
					_ => { return true; }
				}
			}
		}

		return true;
	}

	/// Creates a new OS window.
	/// 
	/// # Arguments
	/// - `name` - The name of the window.
	/// - `extent` - The size of the window.
	/// - `id_name` - The name of the window for identification purposes.
	pub fn create_window(&mut self, name: &str, extent: Extent, id_name: &str) -> WindowHandle {
		let window = Window::new_with_params(name, extent, id_name);

		if let Some(window) = window {
			let window_handle = WindowHandle(self.windows.len() as u64);
			self.windows.push(window);
			window_handle
		} else {
			panic!("Failed to create window")
		}
	}

	/// Gets the OS handles for a window.
	/// 
	/// # Arguments
	/// - `window_handle` - The handle of the window to get the OS handles for.
	/// 
	/// # Returns
	/// The operationg system handles for the window.
	pub fn get_os_handles(&self, window_handle: WindowHandle,) -> WindowOsHandles {
		if window_handle.0 > self.windows.len() as u64 { return WindowOsHandles{ xcb_connection: std::ptr::null_mut(), xcb_window: 0 }; }

		let window = &self.windows[window_handle.0 as usize];

		let connection = window.connection.get_raw_conn() as *mut std::ffi::c_void;

		let window = window.window.to_owned().resource_id();

		return WindowOsHandles{ xcb_connection: connection, xcb_window: window };
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn create_window() {
		let mut window_system = WindowSystem::new();

		let window_handle = window_system.create_window("Main Window", Extent { width: 1920, height: 1080, depth: 1 }, "main_window");

		let os_handles = window_system.get_os_handles(window_handle);

		assert_ne!(os_handles.xcb_connection, std::ptr::null_mut());
		assert_ne!(os_handles.xcb_window, 0);
	}

	#[ignore]
	#[test]
	fn test_window_loop() {
		let mut window_system = WindowSystem::new();

		let window_handle = window_system.create_window("Main Window", Extent { width: 1920, height: 1080, depth: 1 }, "main_window");

		loop {
			if window_system.update() == false {
				break;
			}

			std::thread::sleep(std::time::Duration::from_millis(16));
		}
	}
}