use windows::{core::PCSTR, Win32::{Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM}, Graphics::Gdi::HBRUSH, System::LibraryLoader::GetModuleHandleA, UI::WindowsAndMessaging::{CreateWindowExA, DestroyWindow, RegisterClassA, UnregisterClassA, CW_USEDEFAULT, HCURSOR, HICON, HMENU, WINDOW_EX_STYLE, WM_CREATE, WM_NCCALCSIZE, WM_NCCREATE, WNDCLASSA, WNDCLASS_STYLES, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_VISIBLE}}};

use crate::{WindowEvents};

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
		_ => {
			LRESULT(0)
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

			(class, CreateWindowExA(WINDOW_EX_STYLE::default(), PCSTR(id_name.as_ptr() as _), PCSTR(name.as_ptr() as _), WS_OVERLAPPEDWINDOW  | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, extent.width() as i32, extent.height() as i32, HWND::default(), HMENU::default(), hinstance, None))
		};

		if hwnd.0 == 0 {
			println!("Last error: {:#?}", unsafe { GetLastError().0 });
			return None;
		}

		Win32Window {
			class_atom: class,
			hwnd,
			hinstance: hinstance.into(),
		}.into()
	}
	
	pub(crate) fn poll(&self) -> WindowIterator {
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

pub struct WindowIterator<'a> {
	window: &'a Win32Window,
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {
		None
	}
}