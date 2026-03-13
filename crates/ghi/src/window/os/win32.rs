use windows::{
	core::PCSTR,
	Win32::{
		Devices::HumanInterfaceDevice::{HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
		Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
		Graphics::Gdi::{GetMonitorInfoA, MonitorFromWindow, HBRUSH, MONITORINFO, MONITOR_DEFAULTTONEAREST},
		System::LibraryLoader::GetModuleHandleA,
		UI::{
			HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
			Input::{
				GetRawInputData, RegisterRawInputDevices, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, MOUSE_MOVE_RELATIVE, RAWINPUT,
				RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
			},
			WindowsAndMessaging::{
				CreateWindowExA, DefWindowProcA, DestroyWindow, DispatchMessageA, GetClientRect, GetCursorPos,
				GetWindowLongPtrA, PeekMessageA, PostQuitMessage, RegisterClassA, SetWindowLongPtrA, ShowCursor,
				TranslateMessage, UnregisterClassA, CW_USEDEFAULT, GWLP_USERDATA, GWLP_WNDPROC, HCURSOR, HICON, HMENU, MSG,
				PM_REMOVE, RI_KEY_BREAK, WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
				WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_NCCALCSIZE,
				WM_NCCREATE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WNDCLASSA, WNDCLASS_STYLES, WS_OVERLAPPEDWINDOW,
				WS_VISIBLE,
			},
		},
	},
};

use crate::{
	input::{Keys, MouseKeys},
	os::WindowLike,
	Events,
};

pub struct Window {
	class_atom: u16,
	hinstance: HINSTANCE,
	hwnd: HWND,

	state: State,
}

pub struct Handles {
	pub hinstance: HINSTANCE,
	pub hwnd: HWND,
}

impl WindowLike for Window {
	fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Result<Window, String> {
		let hinstance = unsafe {
			GetModuleHandleA(PCSTR(std::ptr::null()))
				.map_err(|_| "Failed to acquire the module handle. The most likely cause is that the current process module handle could not be resolved.")?
		};

		// Create Cstrings becasue Win32 API uses null terminated strings
		let id_name = std::ffi::CString::new(id_name).map_err(|_| {
			"Failed to build the window class name. The most likely cause is that the id string contains an interior null byte."
		})?;
		let name = std::ffi::CString::new(name).map_err(|_| {
			"Failed to build the window title. The most likely cause is that the window name contains an interior null byte."
		})?;

		let window_style = WS_OVERLAPPEDWINDOW | WS_VISIBLE;

		let (width, height) = (extent.width() as i32, extent.height() as i32);

		let (class, hwnd) = unsafe {
			let wnd_class = WNDCLASSA {
				style: WNDCLASS_STYLES::default(),
				lpfnWndProc: Some(wnd_proc),
				cbClsExtra: 0,
				cbWndExtra: 0,
				hInstance: hinstance.into(),
				hIcon: HICON::default(),
				hCursor: HCURSOR::default(),
				hbrBackground: HBRUSH::default(),
				lpszMenuName: PCSTR(std::ptr::null()),
				lpszClassName: PCSTR(id_name.as_ptr() as _),
			};

			let class = RegisterClassA(&wnd_class);

			if class == 0 {
				return Err("Failed to register the window class. The most likely cause is that the class name already exists or is invalid.".to_string());
			}

			SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
				.map_err(|_| "Failed to set the DPI awareness context. The most likely cause is that the process does not have permission to change DPI awareness.")?;

			let hwnd = CreateWindowExA(
				WINDOW_EX_STYLE::default(),
				PCSTR(id_name.as_ptr() as _),
				PCSTR(name.as_ptr() as _),
				window_style,
				CW_USEDEFAULT,
				CW_USEDEFAULT,
				width,
				height,
				None,
				None,
				Some(hinstance.into()),
				None,
			)
			.map_err(|_| {
				"Failed to create the window. The most likely cause is that the window class registration or parameters are invalid."
			})?;
			(class, hwnd)
		};

		// Remove set WNDPROC, we don't want Windows to call this unless we are ready to handle messages
		unsafe {
			SetWindowLongPtrA(hwnd, GWLP_WNDPROC, 0);
		}

		unsafe {
			// ShowCursor(false);
		}

		let use_raw_mouse = unsafe {
			let rid = RAWINPUTDEVICE {
				usUsagePage: HID_USAGE_PAGE_GENERIC,      // Generic desktop controls
				usUsage: HID_USAGE_GENERIC_MOUSE,         // Mouse
				dwFlags: RAWINPUTDEVICE_FLAGS::default(), // Focused window only
				hwndTarget: hwnd,
			};

			RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as _).is_ok()
		};

		let use_raw_keyboard = unsafe {
			let rid = RAWINPUTDEVICE {
				usUsagePage: HID_USAGE_PAGE_GENERIC,      // Generic desktop controls
				usUsage: HID_USAGE_GENERIC_KEYBOARD,      // Keyboard
				dwFlags: RAWINPUTDEVICE_FLAGS::default(), // Focused window only
				hwndTarget: hwnd,
			};

			RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as _).is_ok()
		};

		Ok(Window {
			class_atom: class,
			hwnd,
			hinstance: hinstance.into(),
			state: State {
				use_raw_mouse,
				use_raw_keyboard,
				..State::default()
			},
		})
	}

	fn poll<'a>(&'a mut self) -> impl Iterator<Item = Events> + 'a {
		// Set WNDPROC, we are ready to handle messages
		unsafe {
			SetWindowLongPtrA(self.hwnd, GWLP_WNDPROC, wnd_proc as _);
		}

		WindowIterator {
			state: self.state.clone(),
			window: self,
		}
	}

	fn handles(&self) -> Handles {
		Handles {
			hwnd: self.hwnd,
			hinstance: self.hinstance,
		}
	}

	fn show_cursor(&mut self, _show: bool) {
		// TODO: Wire cursor visibility control through the current win32 input path.
	}

	fn confine_cursor(&mut self, _confine: bool) {
		// TODO: Wire cursor confinement through ClipCursor when the platform abstraction needs it.
	}
}

impl Drop for Window {
	fn drop(&mut self) {
		unsafe {
			DestroyWindow(self.hwnd);
			UnregisterClassA(PCSTR(self.class_atom as _), Some(self.hinstance));
		}
	}
}

struct WindowData<'a> {
	window: &'a Window,
	state: State,
	payload: Option<Events>,
}

pub struct WindowIterator<'a> {
	window: &'a mut Window,
	state: State,
}

impl Iterator for WindowIterator<'_> {
	type Item = Events;

	fn next(&mut self) -> Option<Events> {
		let mut msg = MSG::default();

		let mut window_data = WindowData {
			window: self.window,
			state: self.state.clone(),
			payload: None,
		};

		unsafe {
			let res = PeekMessageA(&mut msg, Some(self.window.hwnd), 0, 0, PM_REMOVE);

			if res.0 != 0 {
				SetWindowLongPtrA(self.window.hwnd, GWLP_USERDATA, &mut window_data as *mut _ as _); // Only bother setting the window data if there's a message to process
				let _ = TranslateMessage(&msg); // We don't care whether it translated or not
				DispatchMessageA(&msg);
				SetWindowLongPtrA(self.window.hwnd, GWLP_USERDATA, 0); // Clear pointer to window data after processing message
			}
		}

		window_data.payload
	}
}

impl Drop for WindowIterator<'_> {
	fn drop(&mut self) {
		unsafe {
			// We are done handling messages, remove WNDPROC
			SetWindowLongPtrA(self.window.hwnd, GWLP_WNDPROC, 0);
		}

		self.window.state = self.state.clone();
	}
}

/// Represents the state of the window, can be used to store additional data if needed.
#[derive(Debug, Clone, Default)]
struct State {
	use_raw_mouse: bool,
	use_raw_keyboard: bool,
}

fn client_extent(hwnd: HWND) -> Option<(f32, f32)> {
	let mut client_rect = RECT {
		left: 0,
		top: 0,
		right: 0,
		bottom: 0,
	};

	let ok = unsafe { GetClientRect(hwnd, &mut client_rect) }.is_ok();
	if !ok {
		return None;
	}

	let width = (client_rect.right - client_rect.left).max(1) as f32;
	let height = (client_rect.bottom - client_rect.top).max(1) as f32;

	Some((width, height))
}

fn normalize_client_position(hwnd: HWND, x: f32, y: f32) -> Option<(f32, f32)> {
	let (width, height) = client_extent(hwnd)?;

	let x = x / width * 2.0 - 1.0;
	let y = 1.0 - y / height * 2.0;

	Some((x, y))
}

fn cursor_position_in_window(hwnd: HWND) -> Option<(f32, f32)> {
	let _ = hwnd;
	let _ = POINT::default();
	let _ = GetCursorPos;

	// The windows crate bindings available in this workspace do not expose ScreenToClient.
	// Raw absolute mouse input falls back to regular WM_MOUSEMOVE position handling.
	None
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	let window_data = GetWindowLongPtrA(hwnd, GWLP_USERDATA) as *mut WindowData;
	let window_data = unsafe {
		let Some(r) = window_data.as_mut() else {
			return DefWindowProcA(hwnd, msg, wparam, lparam);
		};

		r
	};

	if window_data.window.hwnd.0 != hwnd.0 {
		// Check if the window handle is the same as the one we are handling messages for
		return DefWindowProcA(hwnd, msg, wparam, lparam);
	}

	if let Some((event, result)) = handle_event(hwnd, msg, wparam, lparam, window_data) {
		if let Some(event) = event {
			window_data.payload = Some(event);
		}

		return result;
	}

	DefWindowProcA(hwnd, msg, wparam, lparam)
}

// Handles windows messages/events.
// If the event cannot be handled or we wish to let the OS handle it we return None.
// This function is used inside the actual wnd_proc function.
fn handle_event(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	window_data: &mut WindowData,
) -> Option<(Option<Events>, LRESULT)> {
	let result = match msg {
		WM_NCCREATE => LRESULT(true as _),
		WM_NCCALCSIZE => {
			if wparam.0 == 0 {
				LRESULT(0)
			} else {
				LRESULT(1)
			}
		}
		WM_CREATE => LRESULT(0),
		WM_CLOSE => {
			return Some((Some(Events::Close), LRESULT(0)));
		}
		WM_LBUTTONDOWN => {
			return Some((
				Some(Events::Button {
					pressed: true,
					button: MouseKeys::Left,
				}),
				LRESULT(0),
			));
		}
		WM_LBUTTONUP => {
			return Some((
				Some(Events::Button {
					pressed: false,
					button: MouseKeys::Left,
				}),
				LRESULT(0),
			));
		}
		WM_MBUTTONDOWN => {
			return Some((
				Some(Events::Button {
					pressed: true,
					button: MouseKeys::Middle,
				}),
				LRESULT(0),
			));
		}
		WM_MBUTTONUP => {
			return Some((
				Some(Events::Button {
					pressed: false,
					button: MouseKeys::Middle,
				}),
				LRESULT(0),
			));
		}
		WM_RBUTTONDOWN => {
			return Some((
				Some(Events::Button {
					pressed: true,
					button: MouseKeys::Right,
				}),
				LRESULT(0),
			));
		}
		WM_RBUTTONUP => {
			return Some((
				Some(Events::Button {
					pressed: false,
					button: MouseKeys::Right,
				}),
				LRESULT(0),
			));
		}
		WM_MOUSEMOVE => {
			let x = (lparam.0 & 0xFFFF) as i16 as f32;
			let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;

			let Some((x, y)) = normalize_client_position(hwnd, x, y) else {
				return None;
			};

			return Some((Some(Events::MousePosition { x, y, time: 0 }), LRESULT(0)));
		}
		WM_INPUT => {
			let mut raw_input = [0u64; 1024 / 8]; // Buffer needs to be aligned to 8 bytes
			let mut raw_input_size = std::mem::size_of_val(&raw_input) as u32;

			let res = unsafe {
				GetRawInputData(
					HRAWINPUT(std::mem::transmute(lparam)),
					RID_INPUT,
					Some(std::mem::transmute(&mut raw_input)),
					&mut raw_input_size,
					std::mem::size_of::<RAWINPUTHEADER>() as u32,
				)
			};

			if res == u32::MAX {
				// Error occurred
				return None;
			}

			let raw_input = unsafe { &*(raw_input.as_ptr() as *const RAWINPUT) };

			if raw_input.header.dwType == RIM_TYPEMOUSE.0 && window_data.state.use_raw_mouse {
				let mouse_data = unsafe { &raw_input.data.mouse };

				if mouse_data.usFlags == MOUSE_MOVE_RELATIVE {
					let Some((width, height)) = client_extent(hwnd) else {
						return None;
					};

					return Some((
						Some(Events::MouseMove {
							dx: mouse_data.lLastX as f32 / width * 2.0,
							dy: -(mouse_data.lLastY as f32) / height * 2.0,
							time: 0,
						}),
						LRESULT(0),
					));
				} else if (mouse_data.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0) == MOUSE_MOVE_ABSOLUTE.0 {
					let Some((x, y)) = cursor_position_in_window(hwnd) else {
						return None;
					};

					return Some((Some(Events::MousePosition { x, y, time: 0 }), LRESULT(0)));
				}
			} else if raw_input.header.dwType == RIM_TYPEKEYBOARD.0 && window_data.state.use_raw_keyboard {
				let keyboard_data = unsafe { &raw_input.data.keyboard };
				let pressed = (keyboard_data.Flags as u32 & RI_KEY_BREAK) == 0;

				if let Some(key) = wparam_to_key(WPARAM(keyboard_data.VKey as usize)) {
					return Some((Some(Events::Key { pressed, key }), LRESULT(0)));
				}
			} else {
				return None;
			}

			LRESULT(0)
		}
		WM_MOUSEHWHEEL => {
			let delta = (wparam.0 & 0xFFFF) as i16;

			return Some((
				Some(Events::Button {
					pressed: true,
					button: if delta > 0 {
						MouseKeys::ScrollUp
					} else {
						MouseKeys::ScrollDown
					},
				}),
				LRESULT(0),
			));
		}
		WM_KEYDOWN => {
			if window_data.state.use_raw_keyboard {
				return None;
			}

			let Some(key) = wparam_to_key(wparam) else {
				return None;
			};

			return Some((Some(Events::Key { pressed: true, key }), LRESULT(0)));
		}
		WM_KEYUP => {
			if window_data.state.use_raw_keyboard {
				return None;
			}

			let Some(key) = wparam_to_key(wparam) else {
				return None;
			};

			return Some((Some(Events::Key { pressed: false, key }), LRESULT(0)));
		}
		WM_SIZE => {
			let width = lparam.0 as u32;
			let height = (lparam.0 >> 16) as u32;

			return Some((Some(Events::Resize { width, height }), LRESULT(0)));
		}
		WM_DESTROY => {
			unsafe {
				PostQuitMessage(0);
			}

			LRESULT(0)
		}
		_ => {
			return None;
		}
	};

	Some((None, result))
}

fn wparam_to_key(wparam: WPARAM) -> Option<Keys> {
	match wparam.0 as u8 {
		0x41 => Some(Keys::A),
		0x42 => Some(Keys::B),
		0x43 => Some(Keys::C),
		0x44 => Some(Keys::D),
		0x45 => Some(Keys::E),
		0x46 => Some(Keys::F),
		0x47 => Some(Keys::G),
		0x48 => Some(Keys::H),
		0x49 => Some(Keys::I),
		0x4A => Some(Keys::J),
		0x4B => Some(Keys::K),
		0x4C => Some(Keys::L),
		0x4D => Some(Keys::M),
		0x4E => Some(Keys::N),
		0x4F => Some(Keys::O),
		0x50 => Some(Keys::P),
		0x51 => Some(Keys::Q),
		0x52 => Some(Keys::R),
		0x53 => Some(Keys::S),
		0x54 => Some(Keys::T),
		0x55 => Some(Keys::U),
		0x56 => Some(Keys::V),
		0x57 => Some(Keys::W),
		0x58 => Some(Keys::X),
		0x59 => Some(Keys::Y),
		0x5A => Some(Keys::Z),
		0x30 => Some(Keys::Num0),
		0x31 => Some(Keys::Num1),
		0x32 => Some(Keys::Num2),
		0x33 => Some(Keys::Num3),
		0x34 => Some(Keys::Num4),
		0x35 => Some(Keys::Num5),
		0x36 => Some(Keys::Num6),
		0x37 => Some(Keys::Num7),
		0x38 => Some(Keys::Num8),
		0x39 => Some(Keys::Num9),
		0x60 => Some(Keys::NumPad0),
		0x61 => Some(Keys::NumPad1),
		0x62 => Some(Keys::NumPad2),
		0x63 => Some(Keys::NumPad3),
		0x64 => Some(Keys::NumPad4),
		0x65 => Some(Keys::NumPad5),
		0x66 => Some(Keys::NumPad6),
		0x67 => Some(Keys::NumPad7),
		0x68 => Some(Keys::NumPad8),
		0x69 => Some(Keys::NumPad9),
		0x6A => Some(Keys::NumPadMultiply),
		0x6B => Some(Keys::NumPadAdd),
		0x6D => Some(Keys::NumPadSubtract),
		0x6E => Some(Keys::NumPadDecimal),
		0x6F => Some(Keys::NumPadDivide),
		0x70 => Some(Keys::F1),
		0x71 => Some(Keys::F2),
		0x72 => Some(Keys::F3),
		0x73 => Some(Keys::F4),
		0x74 => Some(Keys::F5),
		0x75 => Some(Keys::F6),
		0x76 => Some(Keys::F7),
		0x77 => Some(Keys::F8),
		0x78 => Some(Keys::F9),
		0x79 => Some(Keys::F10),
		0x7A => Some(Keys::F11),
		0x7B => Some(Keys::F12),
		0x08 => Some(Keys::Backspace),
		0x09 => Some(Keys::Tab),
		0x0D => Some(Keys::Enter),
		0x1B => Some(Keys::Escape),
		0x20 => Some(Keys::Space),
		0x2D => Some(Keys::Insert),
		0x2E => Some(Keys::Delete),
		0x25 => Some(Keys::ArrowLeft),
		0x26 => Some(Keys::ArrowUp),
		0x27 => Some(Keys::ArrowRight),
		0x28 => Some(Keys::ArrowDown),
		0x14 => Some(Keys::CapsLock),
		0x10 => Some(Keys::ShiftLeft),
		0x11 => Some(Keys::ControlLeft),
		0x12 => Some(Keys::AltLeft),
		_ => None,
	}
}
