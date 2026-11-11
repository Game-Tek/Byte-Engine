use std::{
	cell::{Cell, RefCell},
	collections::VecDeque,
	rc::Rc,
	sync::Arc,
	sync::mpsc,
};

use windows::{
	Win32::{
		Devices::HumanInterfaceDevice::{HID_USAGE_GENERIC_KEYBOARD, HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC},
		Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
		Graphics::Gdi::{
			ClientToScreen, DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW, HBRUSH, HMONITOR,
			MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
		},
		System::{
			LibraryLoader::GetModuleHandleW,
			Threading::{CreateEventW, INFINITE, SetEvent},
		},
		UI::{
			HiDpi::{
				AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem,
				SetProcessDpiAwarenessContext,
			},
			Input::{
				GetRawInputBuffer, GetRawInputData, HRAWINPUT,
				KeyboardAndMouse::{GetAsyncKeyState, GetFocus, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON},
				MOUSE_MOVE_ABSOLUTE, MOUSE_VIRTUAL_DESKTOP, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS, RAWINPUTHEADER,
				RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE, RegisterRawInputDevices,
			},
			WindowsAndMessaging::{
				CW_USEDEFAULT, ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
				GWLP_WNDPROC, GetClientRect, GetClipCursor, GetSystemMetrics, GetWindowLongPtrW, HCURSOR, HICON, IDC_ARROW,
				IsIconic, IsWindowVisible, LoadCursorW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW,
				QS_ALLINPUT, RI_KEY_BREAK, RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP,
				RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP,
				RI_MOUSE_WHEEL, RegisterClassW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
				SW_SHOW, SetForegroundWindow, SetWindowLongPtrW, ShowCursor, ShowWindow, TranslateMessage, UnregisterClassW, WHEEL_DELTA, WINDOW_EX_STYLE, WM_CHAR,
				WM_CLOSE, WM_DISPLAYCHANGE, WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
				WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
				WM_RBUTTONUP, WM_SETFOCUS, WM_SIZE, WM_WINDOWPOSCHANGED, WNDCLASS_STYLES, WNDCLASSW, WS_OVERLAPPEDWINDOW,
				WS_POPUP, WS_VISIBLE,
			},
		},
	},
	core::{HSTRING, PCWSTR},
};

use crate::window::{
	AppEvents, Event, Events, Features, PointingDevice, Seat, Wait, WindowId,
	input::{Keys, MouseKeys},
};

/// Events shared by the pump and every window procedure, in arrival order.
mod dialogs;

type EventQueue = Rc<RefCell<VecDeque<Event>>>;

pub struct App {
	class_name: HSTRING,
	hinstance: HINSTANCE,
	events: EventQueue,
	use_raw_mouse: bool,
	use_raw_keyboard: bool,
	wake_event: Arc<WakeEvent>,
	/// The process has one cursor, so every window shares the record of which one holds it.
	cursor_capture: Rc<CursorCapture>,
	/// The process has one mouse, so every window shares its last absolute position.
	absolute_mouse: Rc<AbsoluteMouse>,
	/// Reused by every raw input drain. `u64` elements keep the records 8-byte aligned, as `RAWINPUT` requires.
	raw_input: Box<[u64]>,
	/// Dialog threads send their results here; each poll turns them into events.
	dialog_results: (
		dialogs::Results,
		mpsc::Receiver<(crate::window::DialogId, crate::window::DialogResult)>,
	),
}

/// The latest absolute mouse position in pixels, from devices such as remote desktop sessions, virtual machines, and
/// tablets, so the next one can report the movement between them.
type AbsoluteMouse = Cell<Option<(i32, i32)>>;

/// The `CursorCapture` struct keeps the state of the one cursor that windows created with
/// [`Features::CAPTURE_CURSOR`] hold, so the pump can engage, follow, and release it.
#[derive(Default)]
struct CursorCapture {
	/// The window holding the cursor and the one-pixel rectangle, in screen coordinates, the cursor is clipped to.
	held: Cell<Option<(WindowId, RECT)>>,
}

impl CursorCapture {
	/// Clips the cursor to `rect` for `window`, hiding it when the hold starts.
	fn hold(&self, window: WindowId, rect: RECT) {
		// Windows drops a clip when another application sets its own or a secure desktop shows, so it is set again
		// whenever it differs.
		let mut clip = RECT::default();
		if unsafe { GetClipCursor(&mut clip) }.is_err() || clip != rect {
			let _ = unsafe { ClipCursor(Some(&rect)) };
		}
		if self.held.replace(Some((window, rect))).is_none() {
			unsafe { ShowCursor(false) };
		}
	}

	/// Frees and shows the cursor where it's held.
	fn release(&self) {
		if self.held.take().is_some() {
			let _ = unsafe { ClipCursor(None) };
			unsafe { ShowCursor(true) };
		}
	}

	fn holds(&self, window: WindowId) -> bool {
		self.held.get().is_some_and(|(held, _)| held == window)
	}
}

/// The `WakeEvent` struct owns the auto-reset event a waiting poll also waits on.
///
/// Wakers share it so the handle stays valid for as long as any of them can signal it.
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

pub struct Window {
	hinstance: HINSTANCE,
	hwnd: HWND,
	/// Kept alive and at a stable address for the window procedure until the window is destroyed.
	data: Box<WindowData>,
}

pub struct Handles {
	pub hinstance: HINSTANCE,
	pub hwnd: HWND,
}

/// The `WindowData` struct gives the window procedure its window id and queue through `GWLP_USERDATA`.
struct WindowData {
	id: WindowId,
	events: EventQueue,
	use_raw_mouse: bool,
	use_raw_keyboard: bool,
	/// The monitor the window was last reported on, so moves report only real display changes.
	monitor: Cell<HMONITOR>,
	/// Whether the window holds the cursor while it has focus.
	captures_cursor: bool,
	/// The cursor's latest client position in pixels, so the next one can report the movement between them.
	cursor_position: Cell<Option<(i32, i32)>>,
	cursor_capture: Rc<CursorCapture>,
	absolute_mouse: Rc<AbsoluteMouse>,
	/// The first half of a character outside the Basic Multilingual Plane, which `WM_CHAR` delivers in two messages.
	high_surrogate: Cell<Option<u16>>,
}

impl WindowData {
	fn push(&self, event: Events) {
		self.events.borrow_mut().push_back(Event::Window { window: self.id, event });
	}
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
			events: EventQueue::default(),
			use_raw_mouse,
			use_raw_keyboard,
			wake_event: Arc::new(WakeEvent(wake_event)),
			cursor_capture: Rc::default(),
			absolute_mouse: Rc::default(),
			// 16 KiB holds hundreds of mouse records, enough for an 8 kHz mouse between two frames.
			raw_input: vec![0; 16 * 1024 / 8].into_boxed_slice(),
			dialog_results: mpsc::channel(),
		})
	}

	/// Creates and shows a native window; see [`crate::window::App::create_window`].
	pub(crate) fn create_window(&mut self, name: &str, extent: utils::Extent, features: Features) -> Result<Window, String> {
		let name = HSTRING::from(name);

		// A decorated window gets a title bar, a sizing border, and the minimize, maximize, and close buttons.
		let mut window_style = if features.contains(Features::DECORATIONS) {
			WS_OVERLAPPEDWINDOW
		} else {
			WS_POPUP
		};
		// A background window is never shown, so it cannot take focus from the desktop. A hidden one is shown later.
		if !features.intersects(Features::BACKGROUND | Features::HIDDEN) {
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

		let hwnd = unsafe {
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
				None,
			)
			.map_err(
				|_| "Failed to create the window. The most likely cause is that the window class registration or parameters are invalid.",
			)?
		};

		let data = Box::new(WindowData {
			id: window_id(hwnd),
			events: self.events.clone(),
			use_raw_mouse: self.use_raw_mouse,
			use_raw_keyboard: self.use_raw_keyboard,
			monitor: Cell::new(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }),
			// A background window is never shown, so it never takes focus to hold the cursor.
			captures_cursor: features.contains(Features::CAPTURE_CURSOR) && !features.contains(Features::BACKGROUND),
			cursor_position: Cell::new(None),
			cursor_capture: self.cursor_capture.clone(),
			absolute_mouse: self.absolute_mouse.clone(),
			high_surrogate: Cell::new(None),
		});

		// Messages sent during creation fall through to `DefWindowProcW` because no data is attached yet.
		unsafe {
			SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*data as *const WindowData as _);
		}

		// Creation-time sizing ran before the window procedure could observe it.
		if let Some((width, height)) = client_extent(hwnd) {
			data.push(Events::Resize {
				width: width as u32,
				height: height as u32,
			});
		}
		data.push(Events::DisplayChanged {
			refresh_interval: monitor_refresh_interval(data.monitor.get()),
		});

		Ok(Window {
			hwnd,
			hinstance: self.hinstance,
			data,
		})
	}

	/// Pumps the native queue and yields its events; see [`crate::window::App::poll`].
	pub(crate) fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
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
		if let Some(timeout) = timeout.filter(|_| self.events.borrow().is_empty()) {
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
				self.events.borrow_mut().push_back(Event::App(AppEvents::Quit));
				continue;
			}

			unsafe {
				// Translation posts the `WM_CHAR` text of key presses, which a later peek of this loop dispatches.
				let _ = TranslateMessage(&msg);
				DispatchMessageW(&msg);
			}
		}

		self.update_cursor_capture();

		{
			let mut events = self.events.borrow_mut();
			events.extend(
				self.dialog_results
					.1
					.try_iter()
					.map(|(id, result)| Event::App(AppEvents::Dialog { id, result })),
			);
		}

		std::iter::from_fn(|| self.events.borrow_mut().pop_front())
	}

	/// Reads every queued raw input record in batches, instead of one `WM_INPUT` message and system call per record,
	/// so a mouse reporting thousands of times per second does not flood the message queue.
	///
	/// Raw input goes to the focused window, so its records become that window's events. Records the drain misses
	/// still arrive as `WM_INPUT` messages.
	fn drain_raw_input(&mut self) {
		let window = focused_window_data();
		let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
		loop {
			let mut size = std::mem::size_of_val(&*self.raw_input) as u32;
			let buffer = self.raw_input.as_mut_ptr().cast::<u8>();
			let count = unsafe { GetRawInputBuffer(Some(buffer.cast()), &mut size, header_size) };
			if count == 0 || count == u32::MAX {
				return;
			}
			let mut offset = 0;
			for _ in 0..count {
				// SAFETY: Each record starts at an 8-byte boundary inside the filled part of the buffer.
				let record = unsafe { &*buffer.add(offset).cast::<RAWINPUT>() };
				if let Some(window) = window {
					translate_raw_input(record, window);
				}
				// Records are padded to 8 bytes, as `NEXTRAWINPUTBLOCK` computes.
				offset += (record.header.dwSize as usize).next_multiple_of(8);
			}
		}
	}

	/// Holds the cursor for the focused capturing window, follows that window's center, and releases the cursor when
	/// no capturing window has focus.
	///
	/// A hold waits until no mouse button is down, so the click that focuses a window ends before the cursor
	/// disappears.
	fn update_cursor_capture(&self) {
		let target = focused_window_data()
			.filter(|window| window.captures_cursor)
			.and_then(|window| Some((window.id, center_pixel(window_handle(window.id))?)));

		match (self.cursor_capture.held.get(), target) {
			// Holding again restores a clip Windows dropped and follows a moved or resized window.
			(Some((held, _)), Some((window, rect))) if held == window => self.cursor_capture.hold(window, rect),
			(_, Some((window, rect))) => {
				self.cursor_capture.release();
				let pressed = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
					.into_iter()
					.any(|button| unsafe { GetAsyncKeyState(button.0 as i32) } < 0);
				if !pressed {
					self.cursor_capture.hold(window, rect);
				}
			}
			(_, None) => self.cursor_capture.release(),
		}
	}

	pub(crate) fn waker(&self) -> AppWaker {
		AppWaker(Arc::clone(&self.wake_event))
	}

	/// Starts a common dialog on its own thread; see [`crate::window::App::open_dialog`].
	pub(crate) fn open_dialog(
		&mut self,
		id: crate::window::DialogId,
		dialog: &crate::window::Dialog,
		parent: Option<&Window>,
	) -> Result<(), crate::window::DialogError> {
		dialogs::open(
			&self.dialog_results.0,
			self.waker(),
			id,
			dialog,
			parent.map(|window| window.hwnd),
		)
	}
}

impl Drop for App {
	fn drop(&mut self) {
		// Drop cannot report teardown failures, and the class stays registered if a window outlived the app.
		unsafe {
			let _ = UnregisterClassW(&self.class_name, Some(self.hinstance));
		}
	}
}

impl Window {
	pub(crate) fn id(&self) -> WindowId {
		self.data.id
	}

	pub(crate) fn handles(&self) -> Handles {
		Handles {
			hwnd: self.hwnd,
			hinstance: self.hinstance,
		}
	}

	pub(crate) fn refresh_interval(&self) -> Option<std::time::Duration> {
		monitor_refresh_interval(unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) })
	}

	pub(crate) fn show(&self) {
		// SAFETY: The window handle is valid while `self` lives; both calls only change its visibility and focus.
		unsafe {
			if !IsWindowVisible(self.hwnd).as_bool() {
				let _ = ShowWindow(self.hwnd, SW_SHOW);
				let _ = SetForegroundWindow(self.hwnd);
			}
		}
	}

	/// Reports minimized windows only; Windows offers no cheap query for a window that others fully cover.
	pub(crate) fn is_visible(&self) -> bool {
		!unsafe { IsIconic(self.hwnd) }.as_bool()
	}
}

impl Drop for Window {
	fn drop(&mut self) {
		if self.data.cursor_capture.holds(self.data.id) {
			self.data.cursor_capture.release();
		}
		// `WM_DESTROY` is dispatched synchronously and still reads `data`, which is freed after this body.
		// Drop cannot report teardown failures, and Windows may have already destroyed the window during shutdown.
		unsafe {
			let _ = DestroyWindow(self.hwnd);
		}
	}
}

/// Returns the refresh interval of the monitor's current display mode.
fn monitor_refresh_interval(monitor: HMONITOR) -> Option<std::time::Duration> {
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

/// Returns the data of this thread's window with keyboard focus, which receives raw input.
fn focused_window_data<'a>() -> Option<&'a WindowData> {
	let hwnd = unsafe { GetFocus() };
	if hwnd.is_invalid() || unsafe { IsIconic(hwnd) }.as_bool() {
		return None;
	}
	// Focus can rest on a window this module didn't create, such as one of another thread that shares this thread's
	// input state while a dialog it owns is open. Only windows with this module's procedure carry a `WindowData`.
	if unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) } != wnd_proc as *const () as isize {
		return None;
	}
	// SAFETY: The window runs this module's procedure, and each such window's data outlives it.
	unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowData).as_ref() }
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

/// Queues the events one raw input record carries for `window`: the [`PointingDevice::Mouse`] movement, buttons,
/// and wheel, or a key.
///
/// Mouse records report movement in mouse counts. Devices that report absolute positions, such as remote desktop
/// sessions, virtual machines, and tablets, report the pixels between consecutive positions instead. Returns whether
/// the record was keyboard or mouse input.
fn translate_raw_input(record: &RAWINPUT, window: &WindowData) -> bool {
	let seat = Seat::stub();
	if record.header.dwType == RIM_TYPEMOUSE.0 {
		if !window.use_raw_mouse {
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
			window
				.absolute_mouse
				.replace(Some(position))
				.map(|previous| (position.0 - previous.0, position.1 - previous.1))
		} else {
			Some((mouse.lLastX, mouse.lLastY))
		};
		// Button-only records carry no movement.
		if let Some((dx, dy)) = movement.filter(|&movement| movement != (0, 0)) {
			window.push(Events::MouseMove {
				seat,
				device: PointingDevice::Mouse,
				dx: dx as f32,
				// Raw input grows downward.
				dy: -dy as f32,
				time: 0,
			});
		}

		let (flags, data) = unsafe {
			(
				mouse.Anonymous.Anonymous.usButtonFlags as u32,
				mouse.Anonymous.Anonymous.usButtonData,
			)
		};
		for (_, button, pressed) in RAW_BUTTONS.into_iter().filter(|(bit, ..)| flags & bit != 0) {
			window.push(Events::Button {
				seat,
				device: PointingDevice::Mouse,
				pressed,
				button,
			});
		}
		// The wheel's turn is a signed count of `WHEEL_DELTA` notches.
		let notches = data as i16 as f32 / WHEEL_DELTA as f32;
		if flags & RI_MOUSE_WHEEL != 0 {
			window.push(scroll(PointingDevice::Mouse, 0.0, notches));
		}
		if flags & RI_MOUSE_HWHEEL != 0 {
			window.push(scroll(PointingDevice::Mouse, notches, 0.0));
		}
		true
	} else if record.header.dwType == RIM_TYPEKEYBOARD.0 {
		if !window.use_raw_keyboard {
			return true;
		}
		let keyboard = unsafe { &record.data.keyboard };
		if let Some(key) = wparam_to_key(WPARAM(keyboard.VKey as usize)) {
			window.push(Events::Key {
				seat,
				pressed: (keyboard.Flags as u32 & RI_KEY_BREAK) == 0,
				key,
			});
		}
		true
	} else {
		false
	}
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

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	unsafe {
		let window_data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowData;
		let Some(window_data) = window_data.as_ref() else {
			return DefWindowProcW(hwnd, msg, wparam, lparam);
		};

		let Some(event) = handle_event(hwnd, msg, wparam, lparam, window_data) else {
			return DefWindowProcW(hwnd, msg, wparam, lparam);
		};
		if let Some(event) = event {
			window_data.push(event);
		}
		LRESULT(0)
	}
}

/// Translates one window message into an event for [`wnd_proc`].
///
/// Returns `None` to leave the message to `DefWindowProcW`, and `Some(None)` to consume it without an event.
fn handle_event(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, window_data: &WindowData) -> Option<Option<Events>> {
	let seat = Seat::stub();
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
		WM_MOUSEMOVE if window_data.cursor_capture.holds(window_data.id) => {
			window_data.cursor_position.set(None);
			return Some(None);
		}
		WM_MOUSEMOVE => {
			let position = ((lparam.0 & 0xFFFF) as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as i16 as i32);
			// The cursor's movement is the pixels between its consecutive positions, after pointer acceleration.
			if let Some(previous) = window_data.cursor_position.replace(Some(position)) {
				let (dx, dy) = (position.0 - previous.0, position.1 - previous.1);
				if (dx, dy) != (0, 0) {
					window_data.push(Events::MouseMove {
						seat,
						device: PointingDevice::Pointer,
						dx: dx as f32,
						// Client coordinates grow downward.
						dy: -dy as f32,
						time: 0,
					});
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
			return translate_raw_input(raw_input, window_data).then_some(None);
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
		WM_KEYDOWN | WM_KEYUP if !window_data.use_raw_keyboard => Events::Key {
			seat,
			pressed: msg == WM_KEYDOWN,
			key: wparam_to_key(wparam)?,
		},
		// Text arrives as UTF-16 code units. A character outside the Basic Multilingual Plane takes two messages, the
		// high surrogate first.
		WM_CHAR => {
			let unit = wparam.0 as u16;
			if (0xD800..0xDC00).contains(&unit) {
				window_data.high_surrogate.set(Some(unit));
				return Some(None);
			}
			let character = match window_data.high_surrogate.take() {
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
		},
		// Both messages still need default handling: moves generate `WM_SIZE` and `WM_MOVE` from it.
		WM_WINDOWPOSCHANGED | WM_DISPLAYCHANGE => {
			let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
			if msg == WM_DISPLAYCHANGE || monitor != window_data.monitor.replace(monitor) {
				window_data.push(Events::DisplayChanged {
					refresh_interval: monitor_refresh_interval(monitor),
				});
			}
			return None;
		}
		WM_SETFOCUS | WM_KILLFOCUS => Events::FocusChanged(msg == WM_SETFOCUS),
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
