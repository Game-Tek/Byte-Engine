use windows::{core::PCSTR, Win32::{Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM}, Graphics::Gdi::HBRUSH, System::LibraryLoader::GetModuleHandleA, UI::{HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2}, WindowsAndMessaging::{CreateWindowExA, DefWindowProcA, DestroyWindow, DispatchMessageA, GetClientRect, GetWindowLongPtrA, PeekMessageA, PostQuitMessage, RegisterClassA, SetWindowLongPtrA, TranslateMessage, UnregisterClassA, CW_USEDEFAULT, GWLP_USERDATA, GWLP_WNDPROC, HCURSOR, HICON, HMENU, MSG, PM_REMOVE, WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCCREATE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WNDCLASSA, WNDCLASS_STYLES, WS_OVERLAPPEDWINDOW, WS_VISIBLE}}}};

use crate::{Keys, MouseKeys, WindowEvents};

pub struct Win32Window {
	class_atom: u16,
	hinstance: HINSTANCE,
	hwnd: HWND,
}

unsafe impl Send for Win32Window {}
unsafe impl Sync for Win32Window {}

pub struct OSHandles {
	pub hinstance: HINSTANCE,
	pub hwnd: HWND,
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	let window_data = GetWindowLongPtrA(hwnd, GWLP_USERDATA) as *mut WindowData;
	let window_data = unsafe {
		if let Some(r) = window_data.as_mut() {
			r
		} else {
			return DefWindowProcA(hwnd, msg, wparam, lparam);
		}
	};

	if window_data.window.hwnd.0 != hwnd.0 { // Check if the window handle is the same as the one we are handling messages for
		return DefWindowProcA(hwnd, msg, wparam, lparam);
	}

	match msg {
		WM_NCCREATE => {
			LRESULT(true as _)
		}
		WM_NCCALCSIZE => {
			if wparam.0 == 0 {
				LRESULT(0)
			} else {
				LRESULT(1)
			}
		}
		WM_CREATE => {
			LRESULT(0)
		}
		WM_CLOSE => {
			window_data.payload = Some(WindowEvents::Close);

			LRESULT(0)
		}
		WM_LBUTTONDOWN => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: true,
				button: MouseKeys::Left,
			});

			LRESULT(0)
		}
		WM_LBUTTONUP => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: false,
				button: MouseKeys::Left,
			});

			LRESULT(0)
		}
		WM_MBUTTONDOWN => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: true,
				button: MouseKeys::Middle,
			});

			LRESULT(0)
		}
		WM_MBUTTONUP => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: false,
				button: MouseKeys::Middle,
			});

			LRESULT(0)
		}
		WM_RBUTTONDOWN => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: true,
				button: MouseKeys::Right,
			});

			LRESULT(0)
		}
		WM_RBUTTONUP => {
			window_data.payload = Some(WindowEvents::Button {
				pressed: false,
				button: MouseKeys::Right,
			});

			LRESULT(0)
		}
		WM_MOUSEMOVE => {
			let x = (lparam.0 & 0xFFFF) as u32;
			let y = (lparam.0 >> 16) as u32;

			let (width, height) = unsafe {
				let mut lprect = RECT {
					left: 0,
					top: 0,
					right: 0,
					bottom: 0,
				};

				let _ = GetClientRect(hwnd, &mut lprect);

				((lprect.right - lprect.left) as u32, (lprect.bottom - lprect.top) as u32)
			};

			let y = height - y;

			let x = x as f32 / width as f32;
			let y = y as f32 / height as f32;

			let x = x * 2f32 - 1f32;
			let y = y * 2f32 - 1f32;

			window_data.payload = Some(WindowEvents::MouseMove {
				x,
				y,
				time: 0,
			});

			LRESULT(0)
		}
		WM_MOUSEHWHEEL => {
			let delta = (wparam.0 & 0xFFFF) as i16;

			window_data.payload = Some(WindowEvents::Button { 
				pressed: true,
				button: if delta > 0 { MouseKeys::ScrollUp } else { MouseKeys::ScrollDown },
			});

			LRESULT(0)
		}
		WM_KEYDOWN => {
			let key = if let Some(k) = wparam_to_key(wparam) {
				k
			} else {
				return DefWindowProcA(hwnd, msg, wparam, lparam);
			};

			window_data.payload = Some(WindowEvents::Key {
				pressed: true,
				key,
			});

			LRESULT(0)
		}
		WM_KEYUP => {
			let key = if let Some(k) = wparam_to_key(wparam) {
				k
			} else {
				return DefWindowProcA(hwnd, msg, wparam, lparam);
			};

			window_data.payload = Some(WindowEvents::Key {
				pressed: false,
				key,
			});

			LRESULT(0)
		}
		WM_SIZE => {
			let width = lparam.0 as u32;
			let height = (lparam.0 >> 16) as u32;

			window_data.payload = Some(WindowEvents::Resize {
				width,
				height,
			});

			LRESULT(0)
		}
		WM_DESTROY => {
			unsafe {
				PostQuitMessage(0);
			}

			LRESULT(0)
		}
		_ => {
			DefWindowProcA(hwnd, msg, wparam, lparam)
		}
	}
}

fn wparam_to_key(wparam: WPARAM) -> Option<Keys> {
	match wparam.0 as u8 {
		0x41 => Some(Keys::A), 0x42 => Some(Keys::B), 0x43 => Some(Keys::C), 0x44 => Some(Keys::D), 0x45 => Some(Keys::E), 0x46 => Some(Keys::F),
		0x47 => Some(Keys::G), 0x48 => Some(Keys::H), 0x49 => Some(Keys::I), 0x4A => Some(Keys::J), 0x4B => Some(Keys::K), 0x4C => Some(Keys::L),
		0x4D => Some(Keys::M), 0x4E => Some(Keys::N), 0x4F => Some(Keys::O), 0x50 => Some(Keys::P), 0x51 => Some(Keys::Q), 0x52 => Some(Keys::R),
		0x53 => Some(Keys::S), 0x54 => Some(Keys::T), 0x55 => Some(Keys::U), 0x56 => Some(Keys::V), 0x57 => Some(Keys::W), 0x58 => Some(Keys::X),
		0x59 => Some(Keys::Y), 0x5A => Some(Keys::Z),
		0x30 => Some(Keys::Num0), 0x31 => Some(Keys::Num1), 0x32 => Some(Keys::Num2), 0x33 => Some(Keys::Num3), 0x34 => Some(Keys::Num4),
		0x35 => Some(Keys::Num5),0x36 => Some(Keys::Num6), 0x37 => Some(Keys::Num7), 0x38 => Some(Keys::Num8), 0x39 => Some(Keys::Num9),
		0x60 => Some(Keys::NumPad0), 0x61 => Some(Keys::NumPad1), 0x62 => Some(Keys::NumPad2), 0x63 => Some(Keys::NumPad3), 0x64 => Some(Keys::NumPad4),
		0x65 => Some(Keys::NumPad5), 0x66 => Some(Keys::NumPad6), 0x67 => Some(Keys::NumPad7), 0x68 => Some(Keys::NumPad8), 0x69 => Some(Keys::NumPad9),
		0x6A => Some(Keys::NumPadMultiply), 0x6B => Some(Keys::NumPadAdd), 0x6D => Some(Keys::NumPadSubtract), 0x6E => Some(Keys::NumPadDecimal), 0x6F => Some(Keys::NumPadDivide),
		0x70 => Some(Keys::F1), 0x71 => Some(Keys::F2), 0x72 => Some(Keys::F3), 0x73 => Some(Keys::F4), 0x74 => Some(Keys::F5), 0x75 => Some(Keys::F6),
		0x76 => Some(Keys::F7), 0x77 => Some(Keys::F8), 0x78 => Some(Keys::F9), 0x79 => Some(Keys::F10), 0x7A => Some(Keys::F11), 0x7B => Some(Keys::F12),
		0x08 => Some(Keys::Backspace), 0x09 => Some(Keys::Tab), 0x0D => Some(Keys::Enter), 0x1B => Some(Keys::Escape), 0x20 => Some(Keys::Space),
		0x2D => Some(Keys::Insert), 0x2E => Some(Keys::Delete),
		0x25 => Some(Keys::ArrowLeft), 0x26 => Some(Keys::ArrowUp), 0x27 => Some(Keys::ArrowRight), 0x28 => Some(Keys::ArrowDown),
		0x14 => Some(Keys::CapsLock), 0x10 => Some(Keys::ShiftLeft), 0x11 => Some(Keys::ControlLeft), 0x12 => Some(Keys::AltLeft),
		_ => None,
	}
}

impl Win32Window {
	pub(crate) fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Option<Win32Window> {
		let hinstance = unsafe {
			GetModuleHandleA(PCSTR(std::ptr::null())).ok()?
		};

		// Create Cstrings becasue Win32 API uses null terminated strings
		let id_name = std::ffi::CString::new(id_name).ok()?;
		let name = std::ffi::CString::new(name).ok()?;

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
				return None;
			}

			SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).ok()?;

			let hwnd = CreateWindowExA(WINDOW_EX_STYLE::default(), PCSTR(id_name.as_ptr() as _), PCSTR(name.as_ptr() as _), window_style, CW_USEDEFAULT, CW_USEDEFAULT, width, height, None, None, Some(hinstance.into()), None).ok()?;
			(class, hwnd)
		};

		// Remove set WNDPROC, we don't want Windows to call this unless we are ready to handle messages
		unsafe {
		 	SetWindowLongPtrA(hwnd, GWLP_WNDPROC, 0);
		}

		Win32Window {
			class_atom: class,
			hwnd,
			hinstance: hinstance.into(),
		}.into()
	}
	
	pub(crate) fn poll(&self) -> WindowIterator {
		// Set WNDPROC, we are ready to handle messages
		unsafe {
			SetWindowLongPtrA(self.hwnd, GWLP_WNDPROC, wnd_proc as _);
	   }

		WindowIterator {
			window: self,
		}
	}
	
	pub(crate) fn get_os_handles(&self) -> OSHandles {
		OSHandles {
			hwnd: self.hwnd,
			hinstance: self.hinstance,
		}
	}
}

impl Drop for Win32Window {
	fn drop(&mut self) {
		unsafe {
			DestroyWindow(self.hwnd);

			UnregisterClassA(PCSTR(self.class_atom as _), Some(self.hinstance));
		}
	}

}

struct WindowData<'a> {
	window: &'a Win32Window,
	payload: Option<WindowEvents>,
}

pub struct WindowIterator<'a> {
	window: &'a Win32Window,
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {
		let mut msg = MSG::default();

		let mut window_data = WindowData {
			window: self.window,
			payload: None,
		};

		let res = unsafe {
			SetWindowLongPtrA(self.window.hwnd, GWLP_USERDATA, &mut window_data as *mut _ as _);

			let res = PeekMessageA(&mut msg, Some(self.window.hwnd), 0, 0, PM_REMOVE);

			TranslateMessage(&msg);
			DispatchMessageA(&msg);

			SetWindowLongPtrA(self.window.hwnd, GWLP_USERDATA, 0);

			res
		};

		if res.0 == 0 {
			return None;
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
	}
}