use crate::input::Keys;
use crate::{os::WindowLike, Events};
use objc2::{rc::Retained, MainThreadMarker};
use objc2::MainThreadOnly as _;
use objc2::Message as _;
use objc2::define_class;
use objc2_app_kit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSEventModifierFlags, NSEventType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSRunLoop, NSRunLoopMode, NSSize, NSString};

pub struct Window {
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
			window,
			app,
		})
	}

	fn poll(&mut self) -> impl Iterator<Item = Events> {
		let event = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, None, unsafe { NSDefaultRunLoopMode }, true);

		let mut events = Vec::new();

		dbg!(&event);

		if let Some(event) = event {
			match event.r#type() {
				NSEventType::MouseMoved => {
					events.push(Events::MouseMove { x: event.absoluteX() as _, y: event.absoluteY() as _, time: event.timestamp().to_bits() });
				}
				NSEventType::KeyDown | NSEventType::KeyUp => {
					let pressed = event.r#type() == NSEventType::KeyDown;

					let key = match event.keyCode() {
						53 => {
							Keys::Escape
						}
						13 => {
							Keys::W
						}
						0 => {
							Keys::A
						}
						1 => {
							Keys::S
						}
						2 => {
							Keys::D
						}
						_ => { Keys::Z }
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
