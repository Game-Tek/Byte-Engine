use std::{
	cell::{Cell, RefCell},
	collections::VecDeque,
	rc::Rc,
	sync::Mutex,
};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send};
use objc2_app_kit::{
	NSApp, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
	NSBackingStoreType, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSScreen, NSView, NSWindow, NSWindowDelegate,
	NSWindowStyleMask,
};
use objc2_foundation::{
	NSAutoreleasePool, NSDate, NSDefaultRunLoopMode, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
	NSString,
};

use crate::window::input::{Keys, MouseKeys};
use crate::window::{
	AppEvents, Event, Events, Features, Seat, Wait, WindowId,
	os::{AppLike, WindowLike},
};

/// Events shared by the pump and the AppKit delegates, in arrival order.
type EventQueue = Rc<RefCell<VecDeque<Event>>>;

pub struct App {
	mtm: MainThreadMarker,
	_delegate: Retained<ApplicationDelegate>,
	events: EventQueue,
	/// Modifier state is keyboard-wide, so it survives focus moving between windows.
	modifier_state: ModifierState,
}

pub struct Window {
	window: Retained<NSWindow>,
	_delegate: Retained<WindowDelegate>,
}

/// The `AppWaker` struct posts an empty application-defined event that ends a waiting poll.
#[derive(Clone)]
pub struct AppWaker;

impl AppWaker {
	pub fn wake(&self) {
		post_wake_event();
	}
}

/// Marks the application-defined events that only exist to end a waiting poll.
const WAKE_EVENT_SUBTYPE: i16 = 0x4245;

/// Posts an event that ends a waiting poll without producing an engine event.
fn post_wake_event() {
	let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
		NSEventType::ApplicationDefined,
		NSPoint::new(0.0, 0.0),
		NSEventModifierFlags::empty(),
		0.0,
		0,
		None,
		WAKE_EVENT_SUBTYPE,
		0,
		0,
	) else {
		return;
	};
	// SAFETY: AppKit documents `postEvent:atStart:` as callable from secondary threads, and the shared application
	// object exists because the app that hands out wakers created it on the main thread.
	let app = NSApplication::sharedApplication(unsafe { MainThreadMarker::new_unchecked() });
	app.postEvent_atStart(&event, false);
}

pub struct Handles {
	pub(crate) view: Retained<NSView>,
}

struct WindowDelegateIvars {
	/// Native callbacks share the app queue so focus transitions keep their arrival order across windows.
	events: EventQueue,
	window: WindowId,
	zoomed: Cell<bool>,
}

struct ApplicationDelegateIvars {
	events: EventQueue,
}

static NEXT_WINDOW_CASCADE_TOP_LEFT: Mutex<Option<(f64, f64)>> = Mutex::new(None);

define_class!(
	#[unsafe(super = NSObject)]
	#[thread_kind = MainThreadOnly]
	#[ivars = WindowDelegateIvars]
	struct WindowDelegate;

	// SAFETY: `WindowDelegate` inherits from NSObject and uses objc2's generated object layout and lifecycle.
	unsafe impl NSObjectProtocol for WindowDelegate {}

	// SAFETY: Every exported selector has the signature required by NSWindowDelegate and runs on the main thread.
	unsafe impl NSWindowDelegate for WindowDelegate {
		#[unsafe(method(windowWillClose:))]
		fn window_will_close(&self, _notification: &NSNotification) {
			self.push(Events::Close);
		}

		#[unsafe(method(windowDidMiniaturize:))]
		fn window_did_miniaturize(&self, _notification: &NSNotification) {
			self.push(Events::Minimize);
		}

		#[unsafe(method(windowDidResize:))]
		fn window_did_resize(&self, notification: &NSNotification) {
			self.update_window_state(notification);
		}

		#[unsafe(method(windowDidChangeBackingProperties:))]
		fn window_did_change_backing_properties(&self, notification: &NSNotification) {
			self.update_window_state(notification);
		}

		#[unsafe(method(windowDidChangeScreen:))]
		fn window_did_change_screen(&self, notification: &NSNotification) {
			let Some(window) = notification.object().and_then(|object| object.downcast::<NSWindow>().ok()) else {
				return;
			};
			self.push(Events::DisplayChanged {
				refresh_interval: screen_refresh_interval(&window),
			});
		}

		#[unsafe(method(windowDidBecomeKey:))]
		fn window_did_become_key(&self, _notification: &NSNotification) {
			self.push(Events::FocusChanged(true));
		}

		#[unsafe(method(windowDidResignKey:))]
		fn window_did_resign_key(&self, _notification: &NSNotification) {
			self.push(Events::FocusChanged(false));
		}

		#[unsafe(method(windowDidEnterFullScreen:))]
		fn window_did_enter_full_screen(&self, _notification: &NSNotification) {
			self.push(Events::Maximize);
			self.ivars().zoomed.set(true);
		}

		#[unsafe(method(windowDidExitFullScreen:))]
		fn window_did_exit_full_screen(&self, _notification: &NSNotification) {
			self.ivars().zoomed.set(false);
		}
	}
);

define_class!(
	#[unsafe(super = NSObject)]
	#[thread_kind = MainThreadOnly]
	#[ivars = ApplicationDelegateIvars]
	struct ApplicationDelegate;

	// SAFETY: `ApplicationDelegate` inherits from NSObject and uses objc2's generated object layout and lifecycle.
	unsafe impl NSObjectProtocol for ApplicationDelegate {}

	// SAFETY: Every exported selector has the signature required by NSApplicationDelegate and runs on the main thread.
	unsafe impl NSApplicationDelegate for ApplicationDelegate {
		#[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
		fn application_should_handle_reopen(&self, sender: &NSApplication, has_visible_windows: bool) -> bool {
			if !has_visible_windows || sender.keyWindow().is_none() {
				restore_windows(sender);
			}

			true
		}

		#[unsafe(method(applicationDidBecomeActive:))]
		fn application_did_become_active(&self, notification: &NSNotification) {
			let Some(sender) = notification
				.object()
				.and_then(|object| object.downcast::<NSApplication>().ok())
			else {
				return;
			};
			if sender.keyWindow().is_none() {
				restore_windows(&sender);
			}
		}

		#[unsafe(method(applicationDidChangeScreenParameters:))]
		fn application_did_change_screen_parameters(&self, _notification: &NSNotification) {
			// A display mode change reaches the application, not the windows, so report it for every window.
			let mtm = self.mtm();
			let mut events = self.ivars().events.borrow_mut();
			for window in NSApp(mtm).windows().iter() {
				events.push_back(Event::Window {
					window: window_id(&window),
					event: Events::DisplayChanged {
						refresh_interval: screen_refresh_interval(&window),
					},
				});
			}
			post_wake_event();
		}

		#[unsafe(method(applicationShouldTerminate:))]
		fn application_should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
			// The engine owns shutdown, so AppKit must not exit the process underneath it.
			self.ivars().events.borrow_mut().push_back(Event::App(AppEvents::Quit));
			post_wake_event();
			NSApplicationTerminateReply::TerminateCancel
		}
	}
);

impl WindowDelegate {
	fn new(mtm: MainThreadMarker, events: EventQueue, window: WindowId) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(WindowDelegateIvars {
			events,
			window,
			zoomed: Cell::new(false),
		});
		// SAFETY: `this` is a freshly allocated subclass with initialized ivars and the inherited NSObject initializer.
		unsafe { msg_send![super(this), init] }
	}

	fn push(&self, event: Events) {
		let ivars = self.ivars();
		ivars.events.borrow_mut().push_back(Event::Window {
			window: ivars.window,
			event,
		});
		// Notifications can arrive without an NSEvent, which would leave a waiting poll asleep.
		post_wake_event();
	}

	/// Publishes the drawable pixel size so layout matches the swapchain after resize or display changes.
	fn update_window_state(&self, notification: &NSNotification) {
		let Some(window) = notification.object() else {
			return;
		};

		let Ok(window) = window.downcast::<NSWindow>() else {
			return;
		};
		if let Some(view) = window.contentView() {
			let size = view.convertRectToBacking(view.bounds()).size;
			self.push(Events::Resize {
				width: size.width.round() as u32,
				height: size.height.round() as u32,
			});
		}

		let is_zoomed = window.isZoomed();
		let was_zoomed = self.ivars().zoomed.get();

		if is_zoomed != was_zoomed {
			self.ivars().zoomed.set(is_zoomed);

			if is_zoomed {
				self.push(Events::Maximize);
			}
		}
	}
}

impl ApplicationDelegate {
	fn new(mtm: MainThreadMarker, events: EventQueue) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(ApplicationDelegateIvars { events });
		// SAFETY: `this` is a freshly allocated subclass with initialized ivars and the inherited NSObject initializer.
		unsafe { msg_send![super(this), init] }
	}
}

/// Brings minimized or hidden windows back and gives the first one key focus.
fn restore_windows(app: &NSApplication) {
	let windows = app.windows();

	for window in windows.iter() {
		if window.isMiniaturized() {
			window.deminiaturize(None);
		}
	}

	if let Some(window) = windows.firstObject() {
		window.makeKeyAndOrderFront(None);
	}
}

/// Returns the refresh interval of the screen showing most of the window.
fn screen_refresh_interval(window: &NSWindow) -> Option<std::time::Duration> {
	let frames_per_second = window.screen()?.maximumFramesPerSecond();
	(frames_per_second > 0).then(|| std::time::Duration::from_secs_f64(1.0 / frames_per_second as f64))
}

fn window_id(window: &NSWindow) -> WindowId {
	WindowId::from_raw(window as *const NSWindow as u64)
}

/// Normalizes the window center to `0` and edges to `-1` and `1`, preserving
/// captured positions outside the window so callers can reject an outside drop.
fn normalize_mouse_position(point: NSPoint, content_frame: NSRect) -> Option<(f32, f32)> {
	let width = content_frame.size.width as f32;
	let height = content_frame.size.height as f32;

	if width <= 0.0 || height <= 0.0 {
		return None;
	}

	let x = point.x as f32 - content_frame.origin.x as f32;
	let y = point.y as f32 - content_frame.origin.y as f32;

	let half_width = width / 2.0;
	let half_height = height / 2.0;

	let x = (x - half_width) / half_width;
	let y = (y - half_height) / half_height;

	Some((x, y))
}

fn pixel_extent_to_window_points(extent: utils::Extent, scale_factor: f64) -> NSSize {
	let scale_factor = scale_factor.max(1.0);

	NSSize::new(
		(extent.width() as f64 / scale_factor) as _,
		(extent.height() as f64 / scale_factor) as _,
	)
}

/// Appends relative and normalized absolute motion from one AppKit mouse event.
fn append_mouse_motion(window: &NSWindow, event: &NSEvent, time: u64, push: &mut impl FnMut(Events)) {
	push(Events::MouseMove {
		seat: Seat::stub(),
		dx: event.deltaX() as f32,
		dy: event.deltaY() as f32,
		time,
	});
	append_mouse_position(window, event, time, push);
}

/// Samples the event's position before a button transition so its drag endpoint
/// stays correct when AppKit coalesces or omits a separate motion event.
fn append_mouse_position(window: &NSWindow, event: &NSEvent, time: u64, push: &mut impl FnMut(Events)) {
	let Some(content_view) = window.contentView() else {
		return;
	};
	let Some((x, y)) = normalize_mouse_position(event.locationInWindow(), content_view.frame()) else {
		return;
	};
	push(Events::MousePosition {
		seat: Seat::stub(),
		x,
		y,
		time,
	});
}

/// Appends the physical key and any text produced by one AppKit keyboard event.
fn append_key_event(event: &NSEvent, push: &mut impl FnMut(Events)) {
	let pressed = event.r#type() == NSEventType::KeyDown;
	if let Some(key) = keycode_to_key(event.keyCode()) {
		push(Events::Key {
			seat: Seat::stub(),
			pressed,
			key,
		});
	}
	if !pressed || !accepts_text_input(event.modifierFlags()) {
		return;
	}
	let Some(characters) = event.characters() else {
		return;
	};
	for character in characters.to_string().chars().filter(|character| !character.is_control()) {
		push(Events::Character {
			seat: Seat::stub(),
			character,
		});
	}
}

/// Appends a modifier transition when AppKit reports a changed modifier bit.
fn append_modifier_event(modifier_state: &mut ModifierState, event: &NSEvent, push: &mut impl FnMut(Events)) {
	let Some(key) = modifier_keycode_to_key(event.keyCode()) else {
		return;
	};
	let Some(pressed) = modifier_state.update(key, event.modifierFlags()) else {
		return;
	};
	push(Events::Key {
		seat: Seat::stub(),
		pressed,
		key,
	});
}

impl AppLike for App {
	type Window = Window;

	fn try_new(_: &str) -> Result<Self, String> {
		let mtm = MainThreadMarker::new()
			.ok_or("Failed to create MainThreadMarker. The app is probably being created on a non-main thread.")?;

		let app = NSApp(mtm);
		let events = EventQueue::default();
		let delegate = ApplicationDelegate::new(mtm, events.clone());
		app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
		app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

		Ok(App {
			mtm,
			_delegate: delegate,
			events,
			modifier_state: ModifierState::default(),
		})
	}

	fn create_window(&mut self, name: &str, extent: utils::Extent, features: Features) -> Result<Window, String> {
		// SAFETY: Window construction is confined to the main thread and the pool is drained before returning.
		let _pool = unsafe { NSAutoreleasePool::new() };

		let mtm = self.mtm;
		let app = NSApp(mtm);
		let scale_factor = NSScreen::mainScreen(mtm)
			.map(|screen| screen.backingScaleFactor())
			.unwrap_or(1.0);
		let window_size = pixel_extent_to_window_points(extent, scale_factor);

		let frame = NSRect::new(NSPoint::new(0.0, 0.0), window_size);
		let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable;

		let style = style
			| if features.contains(Features::DECORATIONS) {
				NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable
			} else {
				NSWindowStyleMask::empty()
			};

		// SAFETY: The frame, style mask, and backing mode form a valid designated NSWindow initializer call on the main thread.
		let window = unsafe {
			let window = NSWindow::alloc(mtm);
			NSWindow::initWithContentRect_styleMask_backing_defer(window, frame, style, NSBackingStoreType::Buffered, false)
		};
		// The window is owned by `Window`; AppKit must not free it when the user closes it.
		// SAFETY: Disabling release-on-close only changes ownership bookkeeping for a window we retain.
		unsafe { window.setReleasedWhenClosed(false) };

		let id = window_id(&window);
		let delegate = WindowDelegate::new(mtm, self.events.clone(), id);
		window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

		window.setTitle(&NSString::from_str(name));
		window.setCanHide(false);
		window.setHidesOnDeactivate(false);
		window.setAcceptsMouseMovedEvents(true);

		{
			let mut top_left = NEXT_WINDOW_CASCADE_TOP_LEFT
				.lock()
				.expect("Window cascade mutex poisoned while positioning a macOS window.");

			if let Some(seed) = *top_left {
				let next = window.cascadeTopLeftFromPoint(NSPoint::new(seed.0, seed.1));
				*top_left = Some((next.x as f64, next.y as f64));
			} else {
				window.center();

				let frame = window.frame();
				let centered_top_left = (frame.origin.x as f64, frame.origin.y as f64 + frame.size.height as f64);
				let next = window.cascadeTopLeftFromPoint(NSPoint::new(centered_top_left.0, centered_top_left.1));
				*top_left = Some((next.x as f64, next.y as f64));
			}
		};

		window.makeKeyAndOrderFront(None);
		app.activate();
		// Placement can select a display with a different backing scale than the
		// screen used to request the window. Publish the actual pixels before input.
		if let Some(view) = window.contentView() {
			let size = view.convertRectToBacking(view.bounds()).size;
			delegate.push(Events::Resize {
				width: size.width.round() as u32,
				height: size.height.round() as u32,
			});
		}
		delegate.push(Events::DisplayChanged {
			refresh_interval: screen_refresh_interval(&window),
		});

		Ok(Window {
			window,
			_delegate: delegate,
		})
	}

	fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		let app = NSApp(self.mtm);

		// Only the first dequeue waits; a nil date drains what is queued without waiting.
		let mut expiration = match wait {
			Wait::Immediate => None,
			Wait::Until(deadline) => Some(NSDate::dateWithTimeIntervalSinceNow(
				deadline.saturating_duration_since(std::time::Instant::now()).as_secs_f64(),
			)),
			Wait::Forever => Some(NSDate::distantFuture()),
		};
		// Events queued by delegates before the wait must not sleep behind it.
		if !self.events.borrow().is_empty() {
			expiration = None;
		}

		while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
			NSEventMask::Any,
			expiration.take().as_deref(),
			// SAFETY: NSDefaultRunLoopMode is an immutable process-lifetime Foundation constant.
			unsafe { NSDefaultRunLoopMode },
			true,
		) {
			if event.r#type() == NSEventType::ApplicationDefined && event.subtype().0 == WAKE_EVENT_SUBTYPE {
				continue;
			}

			// Input without a target window, such as motion over the desktop, still reaches AppKit below.
			if let Some(window) = event.window(self.mtm) {
				let time = (event.timestamp() * 1000.0) as u64;
				let id = window_id(&window);
				let mut queue = self.events.borrow_mut();
				let push = &mut |event| queue.push_back(Event::Window { window: id, event });

				match event.r#type() {
					NSEventType::MouseMoved
					| NSEventType::LeftMouseDragged
					| NSEventType::RightMouseDragged
					| NSEventType::OtherMouseDragged => {
						append_mouse_motion(&window, &event, time, push);
					}
					NSEventType::LeftMouseDown | NSEventType::LeftMouseUp => {
						let pressed = event.r#type() == NSEventType::LeftMouseDown;
						append_mouse_position(&window, &event, time, push);

						push(Events::Button {
							seat: Seat::stub(),
							pressed,
							button: MouseKeys::Left,
						});
					}
					NSEventType::RightMouseDown | NSEventType::RightMouseUp => {
						let pressed = event.r#type() == NSEventType::RightMouseDown;
						append_mouse_position(&window, &event, time, push);

						push(Events::Button {
							seat: Seat::stub(),
							pressed,
							button: MouseKeys::Right,
						});
					}
					NSEventType::OtherMouseDown | NSEventType::OtherMouseUp => {
						let pressed = event.r#type() == NSEventType::OtherMouseDown;
						append_mouse_position(&window, &event, time, push);

						push(Events::Button {
							seat: Seat::stub(),
							pressed,
							button: MouseKeys::Middle,
						});
					}
					NSEventType::ScrollWheel => {
						let dx = event.scrollingDeltaX() as f32;
						let dy = event.scrollingDeltaY() as f32;

						if dx != 0.0 || dy != 0.0 {
							push(Events::Scroll {
								seat: Seat::stub(),
								dx,
								dy,
								time,
							});
						}
					}
					NSEventType::KeyDown | NSEventType::KeyUp => {
						append_key_event(&event, push);
					}
					NSEventType::FlagsChanged => {
						append_modifier_event(&mut self.modifier_state, &event, push);
					}
					_ => {}
				}
			}

			// AppKit owns native window interactions such as title-bar drags,
			// close controls, and live resize; re-dispatch after translating input.
			// Delegates push into the same queue during `sendEvent`, keeping arrival order.
			// Keyboard events are consumed here because the default responder chain
			// treats unhandled key presses as errors and plays the system beep.
			if !matches!(
				event.r#type(),
				NSEventType::KeyDown | NSEventType::KeyUp | NSEventType::FlagsChanged
			) {
				app.sendEvent(&event);
			}
		}

		std::iter::from_fn(|| self.events.borrow_mut().pop_front())
	}

	fn waker(&self) -> AppWaker {
		AppWaker
	}
}

impl WindowLike for Window {
	fn id(&self) -> WindowId {
		window_id(&self.window)
	}

	fn handles(&self) -> Handles {
		Handles {
			view: self.window.contentView().unwrap().retain(),
		}
	}

	fn refresh_interval(&self) -> Option<std::time::Duration> {
		screen_refresh_interval(&self.window)
	}
}

impl Drop for Window {
	fn drop(&mut self) {
		self.window.setDelegate(None);
		self.window.close();
	}
}

fn accepts_text_input(flags: NSEventModifierFlags) -> bool {
	!flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control)
}

#[derive(Debug, Default, Clone, Copy)]
struct ModifierState {
	shift_left: bool,
	shift_right: bool,
	control_left: bool,
	control_right: bool,
	alt_left: bool,
	alt_right: bool,
	caps_lock: bool,
}

impl ModifierState {
	fn update(&mut self, key: Keys, flags: NSEventModifierFlags) -> Option<bool> {
		match key {
			Keys::ShiftLeft => update_modifier_side(
				&mut self.shift_left,
				&mut self.shift_right,
				flags.contains(NSEventModifierFlags::Shift),
			),
			Keys::ShiftRight => update_modifier_side(
				&mut self.shift_right,
				&mut self.shift_left,
				flags.contains(NSEventModifierFlags::Shift),
			),
			Keys::ControlLeft => update_modifier_side(
				&mut self.control_left,
				&mut self.control_right,
				flags.contains(NSEventModifierFlags::Control),
			),
			Keys::ControlRight => update_modifier_side(
				&mut self.control_right,
				&mut self.control_left,
				flags.contains(NSEventModifierFlags::Control),
			),
			Keys::AltLeft => update_modifier_side(
				&mut self.alt_left,
				&mut self.alt_right,
				flags.contains(NSEventModifierFlags::Option),
			),
			Keys::AltRight => update_modifier_side(
				&mut self.alt_right,
				&mut self.alt_left,
				flags.contains(NSEventModifierFlags::Option),
			),
			Keys::CapsLock => {
				let pressed = flags.contains(NSEventModifierFlags::CapsLock);

				if pressed == self.caps_lock {
					None
				} else {
					self.caps_lock = pressed;
					Some(pressed)
				}
			}
			_ => None,
		}
	}
}

fn update_modifier_side(current: &mut bool, other: &mut bool, flag_on: bool) -> Option<bool> {
	let next = if !flag_on {
		*other = false;
		false
	} else if !*other {
		true
	} else {
		!*current
	};

	if *current == next {
		return None;
	}

	*current = next;
	Some(next)
}

fn modifier_keycode_to_key(code: u16) -> Option<Keys> {
	match code {
		56 => Some(Keys::ShiftLeft),
		60 => Some(Keys::ShiftRight),
		59 => Some(Keys::ControlLeft),
		62 => Some(Keys::ControlRight),
		58 => Some(Keys::AltLeft),
		61 => Some(Keys::AltRight),
		57 => Some(Keys::CapsLock),
		_ => None,
	}
}

fn keycode_to_key(code: u16) -> Option<Keys> {
	match code {
		0 => Some(Keys::A),
		11 => Some(Keys::B),
		8 => Some(Keys::C),
		2 => Some(Keys::D),
		14 => Some(Keys::E),
		3 => Some(Keys::F),
		5 => Some(Keys::G),
		4 => Some(Keys::H),
		34 => Some(Keys::I),
		38 => Some(Keys::J),
		40 => Some(Keys::K),
		37 => Some(Keys::L),
		46 => Some(Keys::M),
		45 => Some(Keys::N),
		31 => Some(Keys::O),
		35 => Some(Keys::P),
		12 => Some(Keys::Q),
		15 => Some(Keys::R),
		1 => Some(Keys::S),
		17 => Some(Keys::T),
		32 => Some(Keys::U),
		9 => Some(Keys::V),
		13 => Some(Keys::W),
		7 => Some(Keys::X),
		16 => Some(Keys::Y),
		6 => Some(Keys::Z),
		18 => Some(Keys::Num1),
		19 => Some(Keys::Num2),
		20 => Some(Keys::Num3),
		21 => Some(Keys::Num4),
		23 => Some(Keys::Num5),
		22 => Some(Keys::Num6),
		26 => Some(Keys::Num7),
		28 => Some(Keys::Num8),
		25 => Some(Keys::Num9),
		29 => Some(Keys::Num0),
		82 => Some(Keys::NumPad0),
		83 => Some(Keys::NumPad1),
		84 => Some(Keys::NumPad2),
		85 => Some(Keys::NumPad3),
		86 => Some(Keys::NumPad4),
		87 => Some(Keys::NumPad5),
		88 => Some(Keys::NumPad6),
		89 => Some(Keys::NumPad7),
		91 => Some(Keys::NumPad8),
		92 => Some(Keys::NumPad9),
		69 => Some(Keys::NumPadAdd),
		78 => Some(Keys::NumPadSubtract),
		67 => Some(Keys::NumPadMultiply),
		75 => Some(Keys::NumPadDivide),
		65 => Some(Keys::NumPadDecimal),
		76 => Some(Keys::NumPadEnter),
		51 => Some(Keys::Backspace),
		48 => Some(Keys::Tab),
		36 => Some(Keys::Enter),
		49 => Some(Keys::Space),
		114 => Some(Keys::Insert),
		117 => Some(Keys::Delete),
		115 => Some(Keys::Home),
		119 => Some(Keys::End),
		116 => Some(Keys::PageUp),
		121 => Some(Keys::PageDown),
		123 => Some(Keys::ArrowLeft),
		124 => Some(Keys::ArrowRight),
		125 => Some(Keys::ArrowDown),
		126 => Some(Keys::ArrowUp),
		53 => Some(Keys::Escape),
		122 => Some(Keys::F1),
		120 => Some(Keys::F2),
		99 => Some(Keys::F3),
		118 => Some(Keys::F4),
		96 => Some(Keys::F5),
		97 => Some(Keys::F6),
		98 => Some(Keys::F7),
		100 => Some(Keys::F8),
		101 => Some(Keys::F9),
		109 => Some(Keys::F10),
		103 => Some(Keys::F11),
		111 => Some(Keys::F12),
		71 => Some(Keys::NumLock),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use objc2_foundation::{NSPoint, NSRect, NSSize};

	use super::normalize_mouse_position;

	#[test]
	fn normalize_mouse_position_centers_the_origin() {
		let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 100.0));

		let (x, y) = normalize_mouse_position(NSPoint::new(100.0, 50.0), frame).unwrap();

		assert_eq!((x, y), (0.0, 0.0));
	}

	#[test]
	fn normalize_mouse_position_uses_the_content_frame_edges() {
		let frame = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(200.0, 100.0));

		let top_left = normalize_mouse_position(NSPoint::new(10.0, 120.0), frame).unwrap();
		let bottom_right = normalize_mouse_position(NSPoint::new(210.0, 20.0), frame).unwrap();

		assert_eq!(top_left, (-1.0, 1.0));
		assert_eq!(bottom_right, (1.0, -1.0));
	}

	#[test]
	fn normalize_mouse_position_preserves_captured_positions_outside_the_window() {
		let frame = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(200.0, 100.0));
		assert_eq!(
			normalize_mouse_position(NSPoint::new(-90.0, -30.0), frame),
			Some((-2.0, -2.0))
		);
		assert_eq!(normalize_mouse_position(NSPoint::new(310.0, 170.0), frame), Some((2.0, 2.0)));
	}
}
