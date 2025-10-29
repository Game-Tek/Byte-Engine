use crate::input::{Keys, MouseKeys};
use crate::{os::WindowLike, Events};
use objc2::{rc::Retained, MainThreadMarker};
use objc2::MainThreadOnly as _;
use objc2::Message as _;
use objc2::define_class;
use objc2_app_kit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSEventModifierFlags, NSEventType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSRunLoop, NSRunLoopMode, NSSize, NSString};

pub struct Window {
	mtm: MainThreadMarker,
	app: Retained<NSApplication>,
	window: Retained<NSWindow>,
}

pub struct Handles {
	pub(crate) view: Retained<NSView>,
}

impl WindowLike for Window {
	fn try_new(name: &str, extent: utils::Extent, _: &str) -> Result<Self, String> {
		let _pool = unsafe { NSAutoreleasePool::new() };

		let mtm = MainThreadMarker::new().ok_or_else(|| "Failed to create MainThreadMarker. Window is probably being created on a non-main thread.")?;

		let app = NSApp(mtm);

		let frame = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(extent.width() as _, extent.height() as _));
		let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable;

		let window = unsafe {
			let window = NSWindow::alloc(mtm);
			NSWindow::initWithContentRect_styleMask_backing_defer(
				window,
				frame,
				style,
				NSBackingStoreType::Buffered,
				false,
			)
		};

		window.setTitle(&NSString::from_str(name));
		window.makeKeyAndOrderFront(None);

		app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
		app.activate();

		Ok(Window {
			mtm,
			window,
			app,
		})
	}

	fn poll(&mut self) -> impl Iterator<Item = Events> {
		let mut events = Vec::new();

		while let Some(event) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, None, unsafe { NSDefaultRunLoopMode }, true) {
			let time = (event.timestamp() * 1000.0) as u64;

			match event.r#type() {
				NSEventType::MouseMoved => {
					let point = event.locationInWindow();

					if let Some(window) = event.window(self.mtm) {
						if window == self.window {
							let screen = window.screen().unwrap();
							let monitor_extent = screen.frame().size;
							let window_extent = window.frame().size;
							let width = window_extent.width as f32;
							let height = window_extent.height as f32;
							let half_width = width / 2.0;
							let half_height = height / 2.0;
							let (x, y) = (point.x as f32 - half_width, point.y as f32 - half_height);
							let (x, y) = (x / half_width, y / half_height);
							events.push(Events::MouseMove { x, y, time });
						}
					}
				}
				NSEventType::LeftMouseDown | NSEventType::LeftMouseUp => {
					let pressed = event.r#type() == NSEventType::LeftMouseDown;

					events.push(Events::Button { pressed, button: MouseKeys::Left });
				}
				NSEventType::RightMouseDown | NSEventType::RightMouseUp => {
					let pressed = event.r#type() == NSEventType::RightMouseDown;

					events.push(Events::Button { pressed, button: MouseKeys::Right });
				}
				NSEventType::KeyDown | NSEventType::KeyUp => {
					let pressed = event.r#type() == NSEventType::KeyDown;

					let key = match event.keyCode() {
						53 => Keys::Escape,
						13 => Keys::W,
						0 => Keys::A,
						1 => Keys::S,
						2 => Keys::D,
						_ => Keys::Z
					};

					events.push(Events::Key { pressed, key });
				}
				NSEventType::FlagsChanged => {
				}
				NSEventType::AppKitDefined => {
				}
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
