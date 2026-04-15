use std::{cell::Cell, sync::Mutex};

use crate::input::{Keys, MouseKeys};
use crate::{os::WindowLike, Events};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly, Message as _};
use objc2_app_kit::{
	NSApp, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSEventMask,
	NSEventModifierFlags, NSEventType, NSScreen, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
	NSAutoreleasePool, NSDefaultRunLoopMode, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

pub struct Window {
	mtm: MainThreadMarker,
	app: Retained<NSApplication>,
	window: Retained<NSWindow>,
	_app_delegate: Retained<ApplicationDelegate>,
	delegate: Retained<WindowDelegate>,
	modifier_state: ModifierState,
}

pub struct Handles {
	pub(crate) view: Retained<NSView>,
}

#[derive(Debug, Default)]
struct WindowDelegateIvars {
	close_requested: Cell<bool>,
	minimize_requested: Cell<bool>,
	maximize_requested: Cell<bool>,
	zoomed: Cell<bool>,
}

struct ApplicationDelegateIvars {
	window: Retained<NSWindow>,
}

static NEXT_WINDOW_CASCADE_TOP_LEFT: Mutex<Option<(f64, f64)>> = Mutex::new(None);

define_class!(
	#[unsafe(super = NSObject)]
	#[thread_kind = MainThreadOnly]
	#[ivars = WindowDelegateIvars]
	struct WindowDelegate;

	unsafe impl NSObjectProtocol for WindowDelegate {}

	unsafe impl NSWindowDelegate for WindowDelegate {
		#[unsafe(method(windowWillClose:))]
		fn window_will_close(&self, _notification: &NSNotification) {
			self.ivars().close_requested.set(true);
		}

		#[unsafe(method(windowDidMiniaturize:))]
		fn window_did_miniaturize(&self, _notification: &NSNotification) {
			self.ivars().minimize_requested.set(true);
		}

		#[unsafe(method(windowDidResize:))]
		fn window_did_resize(&self, notification: &NSNotification) {
			self.update_zoom_state(notification);
		}

		#[unsafe(method(windowDidEnterFullScreen:))]
		fn window_did_enter_full_screen(&self, _notification: &NSNotification) {
			self.ivars().maximize_requested.set(true);
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

	unsafe impl NSObjectProtocol for ApplicationDelegate {}

	unsafe impl NSApplicationDelegate for ApplicationDelegate {
		#[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
		fn application_should_handle_reopen(&self, _sender: &NSApplication, has_visible_windows: bool) -> bool {
			if !has_visible_windows || self.ivars().window.isMiniaturized() {
				self.restore_window();
			}

			true
		}

		#[unsafe(method(applicationDidBecomeActive:))]
		fn application_did_become_active(&self, _notification: &NSNotification) {
			self.restore_window();
		}
	}
);

impl WindowDelegate {
	fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(WindowDelegateIvars::default());
		unsafe { msg_send![super(this), init] }
	}

	fn update_zoom_state(&self, notification: &NSNotification) {
		let Some(window) = notification.object() else {
			return;
		};

		let Ok(window) = window.downcast::<NSWindow>() else {
			return;
		};

		let is_zoomed = window.isZoomed();
		let was_zoomed = self.ivars().zoomed.get();

		if is_zoomed != was_zoomed {
			self.ivars().zoomed.set(is_zoomed);

			if is_zoomed {
				self.ivars().maximize_requested.set(true);
			}
		}
	}
}

impl ApplicationDelegate {
	fn new(mtm: MainThreadMarker, window: Retained<NSWindow>) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(ApplicationDelegateIvars { window });
		unsafe { msg_send![super(this), init] }
	}

	fn restore_window(&self) {
		let window = &self.ivars().window;

		if window.isMiniaturized() {
			window.deminiaturize(None);
		}

		if !window.isVisible() || !window.isKeyWindow() {
			window.makeKeyAndOrderFront(None);
		}
	}
}

/// Normalizes a mouse position inside the content view so the window center is `0`
/// and the window edges stay within `-1..=1`.
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

	let x = ((x - half_width) / half_width).clamp(-1.0, 1.0);
	let y = ((y - half_height) / half_height).clamp(-1.0, 1.0);

	Some((x, y))
}

fn pixel_extent_to_window_points(extent: utils::Extent, scale_factor: f64) -> NSSize {
	let scale_factor = scale_factor.max(1.0);

	NSSize::new(
		(extent.width() as f64 / scale_factor) as _,
		(extent.height() as f64 / scale_factor) as _,
	)
}

impl WindowLike for Window {
	fn try_new(name: &str, extent: utils::Extent, _: &str) -> Result<Self, String> {
		let _pool = unsafe { NSAutoreleasePool::new() };

		let mtm = MainThreadMarker::new()
			.ok_or_else(|| "Failed to create MainThreadMarker. Window is probably being created on a non-main thread.")?;

		let app = NSApp(mtm);
		let scale_factor = NSScreen::mainScreen(mtm)
			.map(|screen| screen.backingScaleFactor() as f64)
			.unwrap_or(1.0);
		let window_size = pixel_extent_to_window_points(extent, scale_factor);

		let frame = NSRect::new(NSPoint::new(0.0, 0.0), window_size);
		let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable;

		// let style = NSWindowStyleMask::Titled
		// 	| NSWindowStyleMask::Closable
		// 	| NSWindowStyleMask::Miniaturizable
		// 	| NSWindowStyleMask::Resizable;

		let window = unsafe {
			let window = NSWindow::alloc(mtm);
			NSWindow::initWithContentRect_styleMask_backing_defer(window, frame, style, NSBackingStoreType::Buffered, false)
		};

		let app_delegate = ApplicationDelegate::new(mtm, window.clone());
		let delegate = WindowDelegate::new(mtm);
		app.setDelegate(Some(ProtocolObject::from_ref(&*app_delegate)));
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

		app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
		app.activate();

		Ok(Window {
			mtm,
			window,
			app,
			_app_delegate: app_delegate,
			delegate,
			modifier_state: ModifierState::default(),
		})
	}

	fn show_cursor(&mut self, show: bool) {
		todo!()
	}

	fn confine_cursor(&mut self, confine: bool) {
		todo!()
	}

	fn poll(&mut self) -> impl Iterator<Item = Events> {
		let mut events = Vec::new();

		if self.delegate.ivars().close_requested.replace(false) {
			events.push(Events::Close);
		}

		if self.delegate.ivars().minimize_requested.replace(false) {
			events.push(Events::Minimize);
		}

		if self.delegate.ivars().maximize_requested.replace(false) {
			events.push(Events::Maximize);
		}

		while let Some(event) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
			NSEventMask::Any,
			None,
			unsafe { NSDefaultRunLoopMode },
			true,
		) {
			let time = (event.timestamp() * 1000.0) as u64;

			match event.r#type() {
				NSEventType::MouseMoved | NSEventType::LeftMouseDragged | NSEventType::RightMouseDragged => {
					let dx = event.deltaX() as f32;
					let dy = event.deltaY() as f32;

					events.push(Events::MouseMove { dx, dy, time });

					let point = event.locationInWindow();

					if let Some(window) = event.window(self.mtm) {
						if window == self.window {
							if let Some(content_view) = window.contentView() {
								if let Some((x, y)) = normalize_mouse_position(point, content_view.frame()) {
									events.push(Events::MousePosition { x, y, time });
								}
							}
						}
					}
				}
				NSEventType::LeftMouseDown | NSEventType::LeftMouseUp => {
					let pressed = event.r#type() == NSEventType::LeftMouseDown;

					events.push(Events::Button {
						pressed,
						button: MouseKeys::Left,
					});
				}
				NSEventType::RightMouseDown | NSEventType::RightMouseUp => {
					let pressed = event.r#type() == NSEventType::RightMouseDown;

					events.push(Events::Button {
						pressed,
						button: MouseKeys::Right,
					});
				}
				NSEventType::KeyDown | NSEventType::KeyUp => {
					let pressed = event.r#type() == NSEventType::KeyDown;

					if let Some(key) = keycode_to_key(event.keyCode()) {
						events.push(Events::Key { pressed, key });
					}
				}
				NSEventType::FlagsChanged => {
					if let Some(key) = modifier_keycode_to_key(event.keyCode()) {
						if let Some(pressed) = self.modifier_state.update(key, event.modifierFlags()) {
							events.push(Events::Key { pressed, key });
						}
					}
				}
				NSEventType::AppKitDefined => {}
				_ => {}
			}
		}

		events.into_iter()
	}

	fn handles(&self) -> Handles {
		Handles {
			view: self.window.contentView().unwrap().retain(),
		}
	}
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
	use super::normalize_mouse_position;
	use objc2_foundation::{NSPoint, NSRect, NSSize};

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
}
