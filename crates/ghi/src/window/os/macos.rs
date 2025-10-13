use crate::{os::WindowLike, Events};
use objc2::{rc::Retained, MainThreadMarker};
use objc2::MainThreadOnly as _;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask, NSView};
use objc2_foundation::{ns_string, NSAutoreleasePool, NSPoint, NSRect, NSSize};

pub struct Window {
	window: Retained<NSWindow>,
}

pub struct Handles {

}

impl WindowLike for Window {
	fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Result<Self, String> {
		let _pool = unsafe { NSAutoreleasePool::new() };

		let mtm = MainThreadMarker::new().ok_or_else(|| "Failed to create MainThreadMarker")?;

		let app = NSApp(mtm);
		app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
		app.activate();

		let frame = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(800.0, 600.0));

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

		window.setTitle(ns_string!("Hello!"));
		window.makeKeyAndOrderFront(None);

		app.run();

		Ok(Window {
			window,
		})
	}

	fn poll(&mut self) -> impl Iterator<Item = Events> {
		std::iter::empty()
	}

	fn handles(&self) -> Handles {
		Handles {}
	}
}
