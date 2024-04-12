use std::ffi::c_void;

use utils::Extent;
use xcb::{x, Xid};

use crate::{Keys, MouseKeys, WindowEvents};

pub struct X11Window {
	connection: xcb::Connection,
	window: xcb::x::Window,
	wm_del_window: xcb::x::Atom,
}

pub struct WindowIterator<'a> {
	connection: &'a xcb::Connection,
	wm_del_window: xcb::x::Atom,
}

impl X11Window {
	pub fn try_new(name: &str, extent: Extent, id_name: &str) -> Option<X11Window> {
		let (connection, screen_index) = xcb::Connection::connect(None).ok()?;

		let setup = connection.get_setup();
		let screen = setup.roots().nth(screen_index as usize)?;

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
			width: extent.width() as u16, height: extent.height() as u16,
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

		Some(X11Window {
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

	pub fn get_os_handles(&self) -> OSHandles {
		OSHandles {
			xcb_connection: self.connection.get_raw_conn() as *mut c_void,
			xcb_window: self.window.to_owned().resource_id(),
		}
	}

	pub fn close(&self) {
		let connection = &self.connection;

		connection.send_request(&xcb::x::DestroyWindow { window: self.window });
	}
}

pub struct OSHandles {
	pub xcb_connection: *mut c_void,
	pub xcb_window: u32,
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

			let event = event?;

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
				xcb::Event::X(x::Event::MotionNotify(ev)) => {
					let x = ev.event_x();
					let y = ev.event_y();

					Some(WindowEvents::MouseMove { x: x as u32, y: 1080 - (y as u32), time: ev.time() as u64 })
				},
				xcb::Event::X(x::Event::ClientMessage(ev)) => {
					// We have received a message from the server
					if let x::ClientMessageData::Data32([atom, ..]) = ev.data() {
						if atom == self.wm_del_window.resource_id() {
							let event = WindowEvents::Close;
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