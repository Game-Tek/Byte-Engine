use std::{collections::VecDeque, sync::Arc, sync::mpsc};

use windows::{
	Win32::{
		Devices::HumanInterfaceDevice::{HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
		Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
		Graphics::Gdi::{
			ClientToScreen, DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW, HBRUSH, HMONITOR,
			MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow, ScreenToClient,
		},
		System::{
			LibraryLoader::GetModuleHandleW,
			Threading::{CreateEventW, INFINITE, SetEvent},
		},
		UI::{
			HiDpi::{
				AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
				SetProcessDpiAwarenessContext,
			},
			Input::{
				GetRawInputBuffer, GetRawInputData, HRAWINPUT,
				KeyboardAndMouse::{GetAsyncKeyState, GetFocus, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON},
				MOUSE_MOVE_ABSOLUTE, MOUSE_VIRTUAL_DESKTOP, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER,
				RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE, RegisterRawInputDevices,
			},
			WindowsAndMessaging::{
				CREATESTRUCTW, CW_USEDEFAULT, ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
				GWLP_USERDATA, GetClientRect, GetClipCursor, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, HCURSOR, HICON,
				HTCLIENT, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, IsIconic,
				LoadCursorW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
				RI_KEY_BREAK, RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
				RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP, RI_MOUSE_WHEEL,
				RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOZORDER,
				SetCursor, SetWindowLongPtrW, SetWindowPos, ShowCursor, TranslateMessage, UnregisterClassW, WHEEL_DELTA,
				WINDOW_EX_STYLE, WM_CHAR, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
				WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
				WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS,
				WM_SETTINGCHANGE, WM_SIZE, WM_WINDOWPOSCHANGED, WNDCLASS_STYLES, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_POPUP,
				WS_VISIBLE, WindowFromPoint,
			},
		},
	},
	core::{HSTRING, PCWSTR},
};

use super::callbacks::Callbacks;
use crate::window::{
	AppEvents, Event, Events, Features, Pointer, PointingDevice, Seat, Wait, WindowId,
	input::{Keys, MouseKeys},
};

mod dialogs;
mod preferences;

/// The pointer every window procedure reaches the app's [`CallbackState`] through, kept in each window's
/// `GWLP_USERDATA`.
///
/// Windows calls window procedures from inside the pump's own calls, such as `DispatchMessageW`, and from inside the
/// calls a procedure makes, such as `DefWindowProcW` and `SetWindowPos`, so no `&mut` to the state may be held across
/// one of them; see [`super::callbacks`].
type State = super::callbacks::Shared<CallbackState>;

/// The `CallbackState` struct holds what the window procedures and the pump both change, so each has one owner: the
/// [`App`], which frees it after it destroyed every window.
struct CallbackState {
	/// Events in arrival order, from every window procedure and the pump.
	events: VecDeque<Event>,
	use_raw_mouse: bool,
	use_raw_keyboard: bool,
	/// The process has one cursor, so every window shares the record of which one holds it.
	cursor_capture: CursorCapture,
	/// The latest absolute mouse position in pixels, from devices such as remote desktop sessions, virtual machines,
	/// and tablets, so the next one can report the movement between them. The process has one mouse, so every window
	/// shares it.
	absolute_mouse: Option<(i32, i32)>,
	/// Every open window's state, in creation order.
	windows: Vec<WindowSlot>,
}

impl CallbackState {
	fn slot(&mut self, window: WindowId) -> Option<&mut WindowSlot> {
		self.windows.iter_mut().find(|slot| slot.id == Some(window))
	}

	fn push(&mut self, window: WindowId, event: Events) {
		self.events.push_back(Event::Window { window, event });
	}
}

/// The `WindowSlot` struct keeps the state one window's procedure needs between messages.
struct WindowSlot {
	/// The window, or `None` while `CreateWindowExW` creates it, until its `WM_NCCREATE` names it.
	id: Option<WindowId>,
	/// The monitor the window was last reported on, so moves report only real display changes.
	monitor: HMONITOR,
	/// Whether the window holds the cursor while it has focus.
	captures_cursor: bool,
	/// The cursor the window shows over its client area; see [`crate::window::App::set_pointer`].
	pointer: HCURSOR,
	/// The cursor's latest client position in pixels, so the next one can report the movement between them.
	cursor_position: Option<(i32, i32)>,
	/// One bit per virtual key the raw keyboard reports down, so its repeats can be told from presses.
	held_keys: [u64; 4],
	/// The first half of a character outside the Basic Multilingual Plane, which `WM_CHAR` delivers in two messages.
	high_surrogate: Option<u16>,
}

pub struct App {
	class_name: HSTRING,
	hinstance: HINSTANCE,
	/// Shared with every [`AppWaker`]; see there.
	wake_event: Arc<WakeEvent>,
	/// Reused by every raw input drain. `u64` elements keep the records 8-byte aligned, as `RAWINPUT` requires.
	raw_input: Box<[u64]>,
	/// Dialog threads send their results here; each poll turns them into events.
	dialog_results: (
		dialogs::Results,
		mpsc::Receiver<(crate::window::DialogId, crate::window::DialogResult)>,
	),
	/// Declared last so it is freed after `Drop` destroyed every window whose procedure reaches it.
	state: Callbacks<CallbackState>,
}

/// The `CursorCapture` struct keeps the state of the one cursor that windows created with
/// [`Features::CAPTURE_CURSOR`] hold, so the pump can engage, follow, and release it.
#[derive(Default)]
struct CursorCapture {
	/// The window holding the cursor and the one-pixel rectangle, in screen coordinates, the cursor is clipped to.
	held: Option<(WindowId, RECT)>,
}

impl CursorCapture {
	/// Clips the cursor to `rect` for `window`, hiding it when the hold starts.
	fn hold(&mut self, window: WindowId, rect: RECT) {
		// Windows drops a clip when another application sets its own or a secure desktop shows, so it is set again
		// whenever it differs.
		let mut clip = RECT::default();
		if unsafe { GetClipCursor(&mut clip) }.is_err() || clip != rect {
			let _ = unsafe { ClipCursor(Some(&rect)) };
		}
		if self.held.replace((window, rect)).is_none() {
			unsafe { ShowCursor(false) };
		}
	}

	/// Frees and shows the cursor where it's held.
	fn release(&mut self) {
		if self.held.take().is_some() {
			let _ = unsafe { ClipCursor(None) };
			unsafe { ShowCursor(true) };
		}
	}

	fn holds(&self, window: WindowId) -> bool {
		self.held.is_some_and(|(held, _)| held == window)
	}
}

/// The `WakeEvent` struct owns the auto-reset event a waiting poll also waits on.
///
/// It's shared through `Arc` because wakers signal it from any thread and may outlive the [`App`], such as one a HID
/// monitor keeps; the handle stays valid for as long as any of them can signal it.
struct WakeEvent(HANDLE);

// SAFETY: Event handles are process-wide kernel objects that Win32 lets any thread signal and wait on.
unsafe impl Send for WakeEvent {}
// SAFETY: `SetEvent` and waits on the same handle are thread-safe kernel calls.
unsafe impl Sync for WakeEvent {}

impl Drop for WakeEvent {
	fn drop(&mut self) {
		// Drop cannot report the failure, and a failed close only leaks the handle.
		unsafe {
			let _ = CloseHandle(self.0);
		}
	}
}

/// The `AppWaker` struct signals the event a waiting poll waits on.
///
/// An event object survives the modal loops Windows runs while moving or resizing a window, where posted
/// thread messages are dropped.
///
/// It shares the event through `Arc` because wakers signal it from any thread and may outlive the [`App`], such as one
/// a HID monitor keeps; the event handle stays open for as long as any of them can signal it.
#[derive(Clone)]
pub struct AppWaker(Arc<WakeEvent>);

impl AppWaker {
	pub fn wake(&self) {
		// A failed signal leaves the poll waiting for its next event or deadline, which is all it can do.
		unsafe {
			let _ = SetEvent(self.0.0);
		}
	}
}

pub struct Handles {
	pub hinstance: HINSTANCE,
	pub hwnd: HWND,
}

impl App {
	/// Connects to the windowing system; see [`crate::window::App::new`].
	pub(crate) fn try_new(id_name: &str) -> Result<App, String> {
		let hinstance = unsafe {
			GetModuleHandleW(PCWSTR::null())
				.map_err(|_| "Failed to acquire the module handle. The most likely cause is that the current process module handle could not be resolved.")?
		};

		// Wide-character windows receive text as UTF-16 in `WM_CHAR`, whatever the system code page.
		let class_name = HSTRING::from(id_name);

		let class_atom = unsafe {
			let wnd_class = WNDCLASSW {
				style: WNDCLASS_STYLES::default(),
				lpfnWndProc: Some(wnd_proc),
				cbClsExtra: 0,
				cbWndExtra: 0,
				hInstance: hinstance.into(),
				hIcon: HICON::default(),
				// Without a class cursor, the cursor keeps whatever shape it had when it entered the window, such as a
				// resize arrow from the border.
				hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default()),
				hbrBackground: HBRUSH::default(),
				lpszMenuName: PCWSTR::null(),
				lpszClassName: PCWSTR(class_name.as_ptr()),
			};

			RegisterClassW(&wnd_class)
		};

		if class_atom == 0 {
			return Err("Failed to register the window class. The most likely cause is that another app with the same id is alive in this process.".to_string());
		}

		// Awareness is process-wide and can only be set once, so a later app in the same process keeps the first value.
		unsafe {
			let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
		}

		// Raw input registration is process-wide. A null target delivers it to the focused window.
		let register_raw_input = |usage| unsafe {
			let rid = RAWINPUTDEVICE {
				usUsagePage: HID_USAGE_PAGE_GENERIC,
				usUsage: usage,
				dwFlags: RAWINPUTDEVICE_FLAGS::default(),
				hwndTarget: HWND::default(),
			};

			RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as _).is_ok()
		};

		let use_raw_mouse = register_raw_input(HID_USAGE_GENERIC_MOUSE);
		let use_raw_keyboard = register_raw_input(HID_USAGE_GENERIC_KEYBOARD);

		// Auto-reset, so one wait consumes every wake signaled before it.
		let wake_event = unsafe { CreateEventW(None, false, false, windows::core::PCWSTR::null()) }.map_err(|error| {
			format!(
				"Failed to create the wake event: {error}. The most likely cause is that the process ran out of kernel handles."
			)
		})?;

		Ok(App {
			class_name,
			hinstance: hinstance.into(),
			wake_event: Arc::new(WakeEvent(wake_event)),
			// 16 KiB holds hundreds of mouse records, enough for an 8 kHz mouse between two frames.
			raw_input: vec![0; 16 * 1024 / 8].into_boxed_slice(),
			dialog_results: mpsc::channel(),
			state: Callbacks::new(CallbackState {
				// The first poll reports the preferences as they stand at launch.
				events: VecDeque::from([preferences::report()]),
				use_raw_mouse,
				use_raw_keyboard,
				cursor_capture: CursorCapture::default(),
				absolute_mouse: None,
				windows: Vec::new(),
			}),
		})
	}

	/// Creates and shows a native window; see [`crate::window::App::create_window`].
	pub(crate) fn create_window(&mut self, name: &str, extent: utils::Extent, features: Features) -> Result<WindowId, String> {
		let name = HSTRING::from(name);

		// A decorated window gets a title bar, a sizing border, and the minimize, maximize, and close buttons.
		let mut window_style = if features.contains(Features::DECORATIONS) {
			WS_OVERLAPPEDWINDOW
		} else {
			WS_POPUP
		};
		// A background window is never shown, so it cannot take focus from the desktop.
		if !features.contains(Features::BACKGROUND) {
			window_style |= WS_VISIBLE;
		}

		// `extent` sizes the client area, so the title bar and border grow the window around it.
		let mut frame = RECT {
			left: 0,
			top: 0,
			right: extent.width() as i32,
			bottom: extent.height() as i32,
		};
		// A failed adjustment leaves the window at `extent`, with a client area smaller by the decorations.
		let _ =
			unsafe { AdjustWindowRectExForDpi(&mut frame, window_style, false, WINDOW_EX_STYLE::default(), GetDpiForSystem()) };

		// The slot waits for the window's `WM_NCCREATE`, which names it, so the messages creation sends already find it.
		let state = self.state.pointer();
		state.with(|state| {
			state.windows.push(WindowSlot {
				id: None,
				monitor: HMONITOR::default(),
				// A background window is never shown, so it never takes focus to hold the cursor.
				captures_cursor: features.contains(Features::CAPTURE_CURSOR) && !features.contains(Features::BACKGROUND),
				pointer: system_cursor(Pointer::Default),
				cursor_position: None,
				held_keys: [0; 4],
				high_surrogate: None,
			})
		});

		// The creation parameter hands the window procedure the callback state from `WM_NCCREATE` on.
		let created = unsafe {
			CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				&self.class_name,
				&name,
				window_style,
				CW_USEDEFAULT,
				CW_USEDEFAULT,
				frame.right - frame.left,
				frame.bottom - frame.top,
				None,
				None,
				Some(self.hinstance),
				Some(state.as_raw().cast_const()),
			)
		};
		let Ok(hwnd) = created else {
			state.with(|state| state.windows.retain(|slot| slot.id.is_some()));
			return Err("Failed to create the window. The most likely cause is that the window class registration or parameters are invalid.".to_owned());
		};

		let id = window_id(hwnd);
		let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
		// Creation-time sizing may have run before the procedure could report it, and a repeat of the same size or
		// display does no harm.
		let size = client_extent(hwnd).map(|(width, height)| Events::Resize {
			width: width as u32,
			height: height as u32,
			scale: display_scale(hwnd),
		});
		let display = Events::DisplayChanged {
			refresh_interval: monitor_refresh_interval(monitor),
		};
		state.with(|state| {
			state.adopt(id, monitor);
			state.events.extend(
				size.into_iter()
					.chain([display])
					.map(|event| Event::Window { window: id, event }),
			);
		});
		Ok(id)
	}

	/// Destroys the window and forgets it; see [`crate::window::App::destroy_window`].
	pub(crate) fn destroy_window(&mut self, id: WindowId) {
		let state = self.state.pointer();
		let open = state.with(|state| {
			if state.cursor_capture.holds(id) {
				state.cursor_capture.release();
			}
			state.slot(id).is_some()
		});
		if !open {
			return;
		}
		// `DestroyWindow` sends `WM_DESTROY` and `WM_NCDESTROY` to the procedure, which still finds the slot.
		// Drop cannot report teardown failures, and Windows may have already destroyed the window during shutdown.
		unsafe {
			let _ = DestroyWindow(window_handle(id));
		}
		state.with(|state| state.windows.retain(|slot| slot.id != Some(id)));
	}

	/// Returns `id` when it names an open window.
	fn open(&self, id: WindowId) -> Option<WindowId> {
		self.state
			.pointer()
			.with(|state| state.windows.iter().any(|slot| slot.id == Some(id)))
			.then_some(id)
	}

	pub(crate) fn handles(&self, id: WindowId) -> Option<Handles> {
		Some(Handles {
			hwnd: window_handle(self.open(id)?),
			hinstance: self.hinstance,
		})
	}

	pub(crate) fn extent(&self, id: WindowId) -> Option<utils::Extent> {
		let (width, height) = client_extent(window_handle(self.open(id)?))?;
		Some(utils::Extent::rectangle(width as u32, height as u32))
	}

	pub(crate) fn refresh_interval(&self, id: WindowId) -> Option<std::time::Duration> {
		monitor_refresh_interval(unsafe { MonitorFromWindow(window_handle(self.open(id)?), MONITOR_DEFAULTTONEAREST) })
	}

	/// Reports minimized windows only; Windows offers no cheap query for a window that others fully cover.
	pub(crate) fn is_visible(&self, id: WindowId) -> bool {
		self.open(id)
			.is_some_and(|id| !unsafe { IsIconic(window_handle(id)) }.as_bool())
	}

	/// Pumps the native queue and yields its events; see [`crate::window::App::poll`].
	pub(crate) fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		let state = self.state.pointer();
		let timeout = match wait {
			Wait::Immediate => None,
			// Round up so the wait does not end just before the deadline and spin once more.
			Wait::Until(deadline) => Some(
				deadline
					.saturating_duration_since(std::time::Instant::now())
					.as_nanos()
					.div_ceil(1_000_000)
					.min(u128::from(INFINITE - 1)) as u32,
			),
			Wait::Forever => Some(INFINITE),
		};
		// Events queued by window procedures outside the pump must not sleep behind the wait.
		if let Some(timeout) = timeout.filter(|_| state.with(|state| state.events.is_empty())) {
			// Input already in the queue ends the wait at once, including input seen but not removed by an earlier peek.
			unsafe {
				let _ = MsgWaitForMultipleObjectsEx(Some(&[self.wake_event.0]), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
			}
		}

		self.drain_raw_input();

		let mut msg = MSG::default();

		// A null window filter also receives thread messages such as `WM_QUIT`.
		while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
			if msg.message == WM_QUIT {
				state.with(|state| state.events.push_back(Event::App(AppEvents::Quit)));
				continue;
			}

			// Dispatching calls the window procedure, which borrows the state itself.
			unsafe {
				// Translation posts the `WM_CHAR` text of key presses, which a later peek of this loop dispatches.
				let _ = TranslateMessage(&msg);
				DispatchMessageW(&msg);
			}
		}

		self.update_cursor_capture();

		let results = &self.dialog_results.1;
		state.with(|state| {
			state.events.extend(
				results
					.try_iter()
					.map(|(id, result)| Event::App(AppEvents::Dialog { id, result })),
			)
		});

		std::iter::from_fn(move || state.with(|state| state.events.pop_front()))
	}

	/// Reads every queued raw input record in batches, instead of one `WM_INPUT` message and system call per record,
	/// so a mouse reporting thousands of times per second does not flood the message queue.
	///
	/// Raw input goes to the focused window, so its records become that window's events. Records the drain misses
	/// still arrive as `WM_INPUT` messages.
	fn drain_raw_input(&mut self) {
		let window = focused_window();
		let state = self.state.pointer();
		let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
		loop {
			let mut size = std::mem::size_of_val(&*self.raw_input) as u32;
			let buffer = self.raw_input.as_mut_ptr().cast::<u8>();
			let count = unsafe { GetRawInputBuffer(Some(buffer.cast()), &mut size, header_size) };
			if count == 0 || count == u32::MAX {
				return;
			}
			// Translating sends no message, so no window procedure runs while the state is borrowed.
			state.with(|state| {
				let window = window.filter(|&id| state.slot(id).is_some());
				let mut offset = 0;
				for _ in 0..count {
					// SAFETY: Each record starts at an 8-byte boundary inside the filled part of the buffer.
					let record = unsafe { &*buffer.add(offset).cast::<RAWINPUT>() };
					if let Some(window) = window {
						translate_raw_input(record, state, window);
					}
					// Records are padded to 8 bytes, as `NEXTRAWINPUTBLOCK` computes.
					offset += (record.header.dwSize as usize).next_multiple_of(8);
				}
			});
		}
	}

	/// Holds the cursor for the focused capturing window, follows that window's center, and releases the cursor when
	/// no capturing window has focus.
	///
	/// A hold waits until no mouse button is down, so the click that focuses a window ends before the cursor
	/// disappears.
	fn update_cursor_capture(&mut self) {
		let focused = focused_window();
		let pressed = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
			.into_iter()
			.any(|button| unsafe { GetAsyncKeyState(button.0 as i32) } < 0);
		// Clipping and hiding the cursor send no message, so no window procedure runs while the state is borrowed.
		self.state.with(|state| {
			let target = focused
				.filter(|&id| state.slot(id).is_some_and(|slot| slot.captures_cursor))
				.and_then(|id| Some((id, center_pixel(window_handle(id))?)));

			let capture = &mut state.cursor_capture;
			match (capture.held, target) {
				// Holding again restores a clip Windows dropped and follows a moved or resized window.
				(Some((held, _)), Some((window, rect))) if held == window => capture.hold(window, rect),
				(_, Some((window, rect))) => {
					capture.release();
					if !pressed {
						capture.hold(window, rect);
					}
				}
				(_, None) => capture.release(),
			}
		});
	}

	pub(crate) fn waker(&self) -> AppWaker {
		AppWaker(Arc::clone(&self.wake_event))
	}

	/// Refuses the clipboard, which this platform doesn't have yet; see [`crate::window::App::set_clipboard_text`].
	pub(crate) fn set_clipboard_text(&mut self, _text: &str) -> Result<(), String> {
		Err("The clipboard was not set. Byte Engine has a clipboard only on macOS so far. Copy the text by hand on this platform.".to_owned())
	}

	/// Sets whether `window` holds the cursor while it has focus; see [`crate::window::App::set_cursor_capture`].
	/// The pump clips the cursor for a capturing window that has focus.
	pub(crate) fn set_cursor_capture(&mut self, window: WindowId, captures: bool) {
		self.state.with(|state| {
			let Some(slot) = state.slot(window) else {
				return;
			};
			slot.captures_cursor = captures;
			if !captures && state.cursor_capture.holds(window) {
				state.cursor_capture.release();
			}
		});
	}

	/// Records the cursor `window` shows over its client area, and shows it at once when the cursor is there; see
	/// [`crate::window::App::set_pointer`]. `WM_SETCURSOR` shows it again on every move over the window.
	pub(crate) fn set_pointer(&mut self, window: WindowId, pointer: Pointer) {
		let cursor = system_cursor(pointer);
		let open = self.state.with(|state| {
			let slot = state.slot(window);
			slot.map(|slot| slot.pointer = cursor).is_some()
		});
		if open && cursor_in_client(window_handle(window)) {
			unsafe {
				SetCursor(Some(cursor));
			}
		}
	}

	/// Starts a common dialog on its own thread; see [`crate::window::App::open_dialog`].
	pub(crate) fn open_dialog(
		&mut self,
		id: crate::window::DialogId,
		dialog: &crate::window::Dialog,
		parent: Option<WindowId>,
	) -> Result<(), crate::window::DialogError> {
		let parent = parent.and_then(|parent| self.open(parent)).map(window_handle);
		dialogs::open(&self.dialog_results.0, self.waker(), id, dialog, parent)
	}
}

impl CallbackState {
	/// Names the slot `create_window` added for the window being created, once Windows created it.
	fn adopt(&mut self, id: WindowId, monitor: HMONITOR) {
		if let Some(slot) = self.windows.iter_mut().find(|slot| slot.id.is_none_or(|slot| slot == id)) {
			slot.id = Some(id);
			slot.monitor = monitor;
		}
	}
}

impl Drop for App {
	fn drop(&mut self) {
		// Each destruction forgets the window's slot, so the loop ends.
		while let Some(window) = self.state.with(|state| state.windows.iter().rev().find_map(|slot| slot.id)) {
			self.destroy_window(window);
		}
		// Drop cannot report teardown failures, and the class stays registered if a window outlived the app. The
		// callback state is freed after this, once no window procedure can reach it.
		unsafe {
			let _ = UnregisterClassW(&self.class_name, Some(self.hinstance));
		}
	}
}

/// Returns the refresh interval of the monitor's current display mode.
pub(crate) fn monitor_refresh_interval(monitor: HMONITOR) -> Option<std::time::Duration> {
	let mut info = MONITORINFOEXW::default();
	info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
	let mut mode = DEVMODEW {
		dmSize: std::mem::size_of::<DEVMODEW>() as u16,
		..Default::default()
	};
	unsafe {
		if !GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
			return None;
		}
		if !EnumDisplaySettingsW(
			windows::core::PCWSTR(info.szDevice.as_ptr()),
			ENUM_CURRENT_SETTINGS,
			&mut mode,
		)
		.as_bool()
		{
			return None;
		}
	}
	// Frequencies of 0 and 1 stand for the hardware default, which Windows does not name.
	(mode.dmDisplayFrequency > 1).then(|| std::time::Duration::from_secs_f64(1.0 / mode.dmDisplayFrequency as f64))
}

fn window_id(hwnd: HWND) -> WindowId {
	WindowId::from_raw(hwnd.0 as usize as u64)
}

/// Returns the handle a window id was made from.
fn window_handle(id: WindowId) -> HWND {
	HWND(id.0.get() as usize as *mut std::ffi::c_void)
}

/// Returns this thread's window with keyboard focus, which receives raw input, unless it's minimized.
///
/// Focus can rest on a window this module didn't create, such as one of another thread that shares this thread's input
/// state while a dialog it owns is open; the callers only act on windows they find among their own.
fn focused_window() -> Option<WindowId> {
	let hwnd = unsafe { GetFocus() };
	if hwnd.is_invalid() || unsafe { IsIconic(hwnd) }.as_bool() {
		return None;
	}
	Some(window_id(hwnd))
}

/// Returns the one-pixel rectangle at the center of the window's client area, in screen coordinates.
fn center_pixel(hwnd: HWND) -> Option<RECT> {
	let mut client = RECT::default();
	unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
	let mut center = POINT {
		x: (client.left + client.right) / 2,
		y: (client.top + client.bottom) / 2,
	};
	if !unsafe { ClientToScreen(hwnd, &mut center) }.as_bool() {
		return None;
	}
	Some(RECT {
		left: center.x,
		top: center.y,
		right: center.x + 1,
		bottom: center.y + 1,
	})
}

/// Raw input's button transitions, as `usButtonFlags` bits, and the button and state each reports.
const RAW_BUTTONS: [(u32, MouseKeys, bool); 6] = [
	(RI_MOUSE_LEFT_BUTTON_DOWN, MouseKeys::Left, true),
	(RI_MOUSE_LEFT_BUTTON_UP, MouseKeys::Left, false),
	(RI_MOUSE_RIGHT_BUTTON_DOWN, MouseKeys::Right, true),
	(RI_MOUSE_RIGHT_BUTTON_UP, MouseKeys::Right, false),
	(RI_MOUSE_MIDDLE_BUTTON_DOWN, MouseKeys::Middle, true),
	(RI_MOUSE_MIDDLE_BUTTON_UP, MouseKeys::Middle, false),
];

/// Queues the events one raw input record carries for `window`, an open window: the [`PointingDevice::Mouse`]
/// movement, buttons, and wheel, or a key.
///
/// Mouse records report movement in mouse counts. Devices that report absolute positions, such as remote desktop
/// sessions, virtual machines, and tablets, report the pixels between consecutive positions instead. Returns whether
/// the record was keyboard or mouse input.
fn translate_raw_input(record: &RAWINPUT, state: &mut CallbackState, window: WindowId) -> bool {
	let seat = Seat::stub();
	if record.header.dwType == RIM_TYPEMOUSE.0 {
		if !state.use_raw_mouse {
			return true;
		}
		let mouse = unsafe { &record.data.mouse };
		let movement = if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 != 0 {
			let (width, height) = if mouse.usFlags.0 & MOUSE_VIRTUAL_DESKTOP.0 != 0 {
				(SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN)
			} else {
				(SM_CXSCREEN, SM_CYSCREEN)
			};
			// Absolute positions span the screen from 0 to 65535.
			let scale = |value: i32, metric| (value as i64 * unsafe { GetSystemMetrics(metric) } as i64 / 65535) as i32;
			let position = (scale(mouse.lLastX, width), scale(mouse.lLastY, height));
			state
				.absolute_mouse
				.replace(position)
				.map(|previous| (position.0 - previous.0, position.1 - previous.1))
		} else {
			Some((mouse.lLastX, mouse.lLastY))
		};
		// Button-only records carry no movement.
		if let Some((dx, dy)) = movement.filter(|&movement| movement != (0, 0)) {
			state.push(
				window,
				Events::MouseMove {
					seat,
					device: PointingDevice::Mouse,
					dx: dx as f32,
					// Raw input grows downward.
					dy: -dy as f32,
					time: 0,
				},
			);
		}

		let (flags, data) = unsafe {
			(
				mouse.Anonymous.Anonymous.usButtonFlags as u32,
				mouse.Anonymous.Anonymous.usButtonData,
			)
		};
		for (_, button, pressed) in RAW_BUTTONS.into_iter().filter(|(bit, ..)| flags & bit != 0) {
			state.push(
				window,
				Events::Button {
					seat,
					device: PointingDevice::Mouse,
					pressed,
					button,
				},
			);
		}
		// The wheel's turn is a signed count of `WHEEL_DELTA` notches.
		let notches = data as i16 as f32 / WHEEL_DELTA as f32;
		if flags & RI_MOUSE_WHEEL != 0 {
			state.push(window, scroll(PointingDevice::Mouse, 0.0, notches));
		}
		if flags & RI_MOUSE_HWHEEL != 0 {
			state.push(window, scroll(PointingDevice::Mouse, notches, 0.0));
		}
		true
	} else if record.header.dwType == RIM_TYPEKEYBOARD.0 {
		if !state.use_raw_keyboard {
			return true;
		}
		let keyboard = unsafe { &record.data.keyboard };
		if let Some(key) = wparam_to_key(WPARAM(keyboard.VKey as usize)) {
			let pressed = (keyboard.Flags as u32 & RI_KEY_BREAK) == 0;
			// Raw input carries no repeat flag: a make code for a key that is already down is the keyboard's repeat.
			let Some(slot) = state.slot(window) else {
				return true;
			};
			let repeat = hold(&mut slot.held_keys, keyboard.VKey, pressed);
			state.push(
				window,
				Events::Key {
					seat,
					pressed,
					repeat,
					key,
				},
			);
		}
		true
	} else {
		false
	}
}

/// Records virtual key `key` as down or up in `held`, and returns whether a down finds it already down: a repeat.
fn hold(held: &mut [u64; 4], key: u16, pressed: bool) -> bool {
	let (word, bit) = (usize::from(key >> 6) & 3, 1u64 << (key & 63));
	let was = held[word] & bit != 0;
	if pressed {
		held[word] |= bit;
	} else {
		held[word] &= !bit;
	}
	pressed && was
}

fn scroll(device: PointingDevice, dx: f32, dy: f32) -> Events {
	Events::Scroll {
		seat: Seat::stub(),
		device,
		dx,
		dy,
		time: 0,
	}
}

/// The display scale `hwnd` draws at: its DPI over the 96 that Windows draws at 100%.
fn display_scale(hwnd: HWND) -> f32 {
	let dpi = unsafe { GetDpiForWindow(hwnd) };
	dpi as f32 / 96.0
}

fn client_extent(hwnd: HWND) -> Option<(f32, f32)> {
	let mut client_rect = RECT::default();
	unsafe { GetClientRect(hwnd, &mut client_rect) }.ok()?;

	let width = (client_rect.right - client_rect.left).max(1) as f32;
	let height = (client_rect.bottom - client_rect.top).max(1) as f32;

	Some((width, height))
}

fn normalize_client_position(hwnd: HWND, x: f32, y: f32) -> Option<(f32, f32)> {
	let (width, height) = client_extent(hwnd)?;
	Some((x / width * 2.0 - 1.0, 1.0 - y / height * 2.0))
}

/// Returns the system cursor that stands for `pointer`. Windows has no hand that grabs, so `Grab` shows the pointing
/// hand and `Grabbing` the four-headed move arrow.
fn system_cursor(pointer: Pointer) -> HCURSOR {
	let id = match pointer {
		Pointer::Default => IDC_ARROW,
		Pointer::Hand | Pointer::Grab => IDC_HAND,
		Pointer::Text => IDC_IBEAM,
		Pointer::Grabbing => IDC_SIZEALL,
		Pointer::Crosshair => IDC_CROSS,
		Pointer::NotAllowed => IDC_NO,
		Pointer::ResizeColumn => IDC_SIZEWE,
		Pointer::ResizeRow => IDC_SIZENS,
	};
	// System cursors are shared and never freed, so loading one again returns the same handle.
	unsafe { LoadCursorW(None, id).unwrap_or_default() }
}

/// Returns whether the cursor is over `hwnd`'s client area, with no other window on top of it there.
fn cursor_in_client(hwnd: HWND) -> bool {
	let mut point = POINT::default();
	let mut rect = RECT::default();
	let read = unsafe {
		GetCursorPos(&mut point).is_ok()
			&& WindowFromPoint(point) == hwnd
			&& ScreenToClient(hwnd, &mut point).as_bool()
			&& GetClientRect(hwnd, &mut rect).is_ok()
	};
	read && (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
}

/// What [`wnd_proc`] does once its translation, and the borrow of the callback state that it held, ended.
enum Reply {
	/// Leave the message to `DefWindowProcW`.
	Default,
	/// Consume the message.
	Handled,
	/// Show this cursor and consume the message.
	SetCursor(HCURSOR),
	/// Move the window to this frame, which Windows suggested for a display of another DPI, then report its new size.
	MoveTo(RECT),
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	unsafe {
		// `create_window` passes the callback state as the creation parameter, so the messages creation sends after
		// this one already find it.
		if msg == WM_NCCREATE {
			let create = &*(lparam.0 as *const CREATESTRUCTW);
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
			if let Some(state) = State::from_raw(create.lpCreateParams) {
				let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
				state.with(|state| state.adopt(window_id(hwnd), monitor));
			}
		}
		// SAFETY: Only `WM_NCCREATE` above sets the pointer, to the state of the app that created the window, which
		// destroys the window before it frees the state.
		let Some(state) = State::from_raw(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut std::ffi::c_void) else {
			return DefWindowProcW(hwnd, msg, wparam, lparam);
		};
		if msg == WM_NCDESTROY {
			// The window's last message; nothing reads the pointer after it.
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
			return DefWindowProcW(hwnd, msg, wparam, lparam);
		}

		let id = window_id(hwnd);
		// The borrow covers only the translation, which sends no message. The calls below may send messages that
		// re-enter this procedure, so they run after it ends.
		let reply = state.with(|state| translate(state, id, hwnd, msg, wparam, lparam));
		match reply {
			Reply::Default => DefWindowProcW(hwnd, msg, wparam, lparam),
			Reply::Handled => LRESULT(0),
			Reply::SetCursor(cursor) => {
				SetCursor(Some(cursor));
				LRESULT(1)
			}
			Reply::MoveTo(suggested) => {
				let _ = SetWindowPos(
					hwnd,
					None,
					suggested.left,
					suggested.top,
					suggested.right - suggested.left,
					suggested.bottom - suggested.top,
					SWP_NOZORDER | SWP_NOACTIVATE,
				);
				// The resize reports the new scale even when the pixel size stayed, which sends no `WM_SIZE`.
				if let Some((width, height)) = client_extent(hwnd) {
					let event = Events::Resize {
						width: width as u32,
						height: height as u32,
						scale: display_scale(hwnd),
					};
					state.with(|state| state.push(id, event));
				}
				LRESULT(0)
			}
		}
	}
}

/// Translates one message to the window `id` into its events and returns what [`wnd_proc`] does next.
fn translate(state: &mut CallbackState, id: WindowId, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
	let Some(slot) = state.slot(id) else {
		return Reply::Default;
	};
	// Windows asks which cursor to show on every move. Over the client area it is the window's own; over the border
	// the default procedure shows the resize arrows.
	if msg == WM_SETCURSOR && (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
		return Reply::SetCursor(slot.pointer);
	}
	// The window reached a display of another DPI. Windows suggests the frame that keeps its physical size there,
	// which a per-monitor aware window must take itself; the resize then reports the new scale.
	if msg == WM_DPICHANGED {
		// SAFETY: `WM_DPICHANGED` carries a pointer to the suggested frame in `lparam`.
		return Reply::MoveTo(unsafe { *(lparam.0 as *const RECT) });
	}
	match handle_event(hwnd, msg, wparam, lparam, state, id) {
		None => Reply::Default,
		Some(None) => Reply::Handled,
		Some(Some(event)) => {
			state.push(id, event);
			Reply::Handled
		}
	}
}

/// Translates one window message to the open window `id` into an event for [`translate`].
///
/// Returns `None` to leave the message to `DefWindowProcW`, and `Some(None)` to consume it without an event.
fn handle_event(
	hwnd: HWND,
	msg: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	state: &mut CallbackState,
	id: WindowId,
) -> Option<Option<Events>> {
	let seat = Seat::stub();
	let captured = state.cursor_capture.holds(id);
	let use_raw_keyboard = state.use_raw_keyboard;
	let event = match msg {
		WM_CLOSE => Events::Close,
		WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => Events::Button {
			seat,
			device: PointingDevice::Pointer,
			pressed: matches!(msg, WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN),
			button: match msg {
				WM_LBUTTONDOWN | WM_LBUTTONUP => MouseKeys::Left,
				WM_MBUTTONDOWN | WM_MBUTTONUP => MouseKeys::Middle,
				_ => MouseKeys::Right,
			},
		},
		// A held cursor stays at the window's center, so its position means nothing.
		// The cursor can't move from that pixel, so it reports no movement either, and the next movement starts from
		// where the hold leaves it.
		WM_MOUSEMOVE if captured => {
			state.slot(id)?.cursor_position = None;
			return Some(None);
		}
		WM_MOUSEMOVE => {
			let position = ((lparam.0 & 0xFFFF) as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as i16 as i32);
			// The cursor's movement is the pixels between its consecutive positions, after pointer acceleration.
			if let Some(previous) = state.slot(id)?.cursor_position.replace(position) {
				let (dx, dy) = (position.0 - previous.0, position.1 - previous.1);
				if (dx, dy) != (0, 0) {
					state.push(
						id,
						Events::MouseMove {
							seat,
							device: PointingDevice::Pointer,
							dx: dx as f32,
							// Client coordinates grow downward.
							dy: -dy as f32,
							time: 0,
						},
					);
				}
			}
			let (x, y) = normalize_client_position(hwnd, position.0 as f32, position.1 as f32)?;
			Events::PointerPosition { seat, x, y, time: 0 }
		}
		WM_INPUT => {
			let mut raw_input = [0u64; 1024 / 8]; // Buffer needs to be aligned to 8 bytes
			let mut raw_input_size = std::mem::size_of_val(&raw_input) as u32;

			let res = unsafe {
				GetRawInputData(
					HRAWINPUT(lparam.0 as *mut std::ffi::c_void),
					RID_INPUT,
					Some(raw_input.as_mut_ptr().cast()),
					&mut raw_input_size,
					std::mem::size_of::<RAWINPUTHEADER>() as u32,
				)
			};

			if res == u32::MAX {
				// Error occurred
				return None;
			}

			let raw_input = unsafe { &*(raw_input.as_ptr() as *const RAWINPUT) };
			return translate_raw_input(raw_input, state, id).then_some(None);
		}
		// The high word holds the wheel's signed turn in `WHEEL_DELTA` notches; positive turns away from the user, or
		// to the right.
		WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
			let notches = ((wparam.0 >> 16) & 0xFFFF) as i16 as f32 / WHEEL_DELTA as f32;
			if msg == WM_MOUSEWHEEL {
				scroll(PointingDevice::Pointer, 0.0, notches)
			} else {
				scroll(PointingDevice::Pointer, notches, 0.0)
			}
		}
		// Raw keyboard input reports keys through `WM_INPUT` instead, so these fall through to default handling.
		WM_KEYDOWN | WM_KEYUP if !use_raw_keyboard => Events::Key {
			seat,
			pressed: msg == WM_KEYDOWN,
			// Bit 30 holds the key's previous state, so a key down with it set is a repeat.
			repeat: msg == WM_KEYDOWN && lparam.0 & (1 << 30) != 0,
			key: wparam_to_key(wparam)?,
		},
		// Text arrives as UTF-16 code units. A character outside the Basic Multilingual Plane takes two messages, the
		// high surrogate first.
		WM_CHAR => {
			let unit = wparam.0 as u16;
			let slot = state.slot(id)?;
			if (0xD800..0xDC00).contains(&unit) {
				slot.high_surrogate = Some(unit);
				return Some(None);
			}
			let character = match slot.high_surrogate.take() {
				Some(high) => char::decode_utf16([high, unit]).next(),
				None => char::decode_utf16([unit]).next(),
			};
			match character {
				Some(Ok(character)) if !character.is_control() => Events::Character { seat, character },
				// Keys such as Backspace and Enter also type control characters; they arrive as `Events::Key`. An
				// unpaired surrogate carries no text.
				_ => return Some(None),
			}
		}
		WM_SIZE => Events::Resize {
			width: (lparam.0 & 0xffff) as u32,
			height: ((lparam.0 >> 16) & 0xffff) as u32,
			scale: display_scale(hwnd),
		},
		// Both messages still need default handling: moves generate `WM_SIZE` and `WM_MOVE` from it.
		WM_WINDOWPOSCHANGED | WM_DISPLAYCHANGE => {
			let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
			let slot = state.slot(id)?;
			if msg == WM_DISPLAYCHANGE || std::mem::replace(&mut slot.monitor, monitor) != monitor {
				state.push(
					id,
					Events::DisplayChanged {
						refresh_interval: monitor_refresh_interval(monitor),
					},
				);
			}
			return None;
		}
		// The broadcast reaches every top-level window, so each reports; the app passes on only a changed report.
		WM_SETTINGCHANGE => {
			state.events.push_back(preferences::report());
			return None;
		}
		WM_SETFOCUS | WM_KILLFOCUS => {
			// Keys released while another window had the keyboard never report their release here.
			state.slot(id)?.held_keys = [0; 4];
			Events::FocusChanged(msg == WM_SETFOCUS)
		}
		_ => return None,
	};

	Some(Some(event))
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
		// VK_OEM_PERIOD
		0xBE => Some(Keys::Period),
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
		// VK_LWIN and VK_RWIN, the Windows keys.
		0x5B => Some(Keys::MetaLeft),
		0x5C => Some(Keys::MetaRight),
		_ => None,
	}
}
