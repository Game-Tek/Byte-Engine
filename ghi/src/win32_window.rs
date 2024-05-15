use windows::{core::PCSTR, Win32::{Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM}, Graphics::Gdi::{UpdateWindow, HBRUSH}, System::LibraryLoader::GetModuleHandleA, UI::{HiDpi::{AdjustWindowRectExForDpi, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2}, WindowsAndMessaging::{AdjustWindowRectEx, CreateWindowExA, DefWindowProcA, DestroyWindow, DispatchMessageA, GetClientRect, GetMessageA, GetWindowLongPtrA, PeekMessageA, PostQuitMessage, RegisterClassA, SetWindowLongPtrA, TranslateMessage, UnregisterClassA, CW_USEDEFAULT, GWLP_USERDATA, GWLP_WNDPROC, HCURSOR, HICON, HMENU, MSG, PM_REMOVE, WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCCREATE, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSA, WNDCLASS_STYLES, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_VISIBLE}}}};

use crate::{MouseKeys, WindowEvents};

pub struct Win32Window {
	class_atom: u16,
	hinstance: HINSTANCE,
	hwnd: HWND,
}

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
			if wparam.0 == false as _ {
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

				GetClientRect(hwnd, &mut lprect);

				((lprect.right - lprect.left) as u32, (lprect.bottom - lprect.top) as u32)
			};

			let y = height - y;

			window_data.payload = Some(WindowEvents::MouseMove {
				x,
				y,
				time: 0,
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

impl Win32Window {
	pub(crate) fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Option<Win32Window> {
		let hinstance = unsafe {
			GetModuleHandleA(PCSTR(std::ptr::null())).ok()?
		};

		let id_name = std::ffi::CString::new(id_name).ok()?;
		let name = std::ffi::CString::new(name).ok()?;

		

		let window_style = WS_OVERLAPPEDWINDOW | WS_VISIBLE;

		let (width, height) = unsafe {
			let mut rect = RECT {
				left: 0,
				top: 0,
				right: extent.width() as i32,
				bottom: extent.height() as i32,
			};

			AdjustWindowRectEx(&mut rect as _, window_style, false, WINDOW_EX_STYLE::default());

			(rect.right - rect.left, rect.bottom - rect.top)
		};

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

			SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

			(class, CreateWindowExA(WINDOW_EX_STYLE::default(), PCSTR(id_name.as_ptr() as _), PCSTR(name.as_ptr() as _), window_style, CW_USEDEFAULT, CW_USEDEFAULT, width, height, HWND::default(), HMENU::default(), hinstance, None))
		};

		if hwnd.0 == 0 {
			println!("Last error: {:#?}", unsafe { GetLastError().0 });
			return None;
		}

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
			if self.hwnd.0 != 0 {
				DestroyWindow(self.hwnd);
			}

			if self.class_atom != 0 {
				UnregisterClassA(PCSTR(self.class_atom as _), self.hinstance);
			}
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

			let res = PeekMessageA(&mut msg, self.window.hwnd, 0, 0, PM_REMOVE);

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