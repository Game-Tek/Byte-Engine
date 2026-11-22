//! Reads the accessibility display settings and reports them as [`AppEvents::Preferences`]. See
//! [Presentation preferences](/docs/reference/ghi#presentation-preferences).

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSOperationQueue};

use super::{EventQueue, post_wake_event};
use crate::window::{AppEvents, Event, Preferences};

/// The `PreferenceWatch` struct keeps the app told about the accessibility display settings for as long as it lives:
/// it reports them once when created, and again each time they change.
pub(super) struct PreferenceWatch {
	observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl PreferenceWatch {
	/// Queues the current preferences on `events` and starts reporting changes there.
	pub(super) fn new(events: EventQueue) -> Self {
		events.borrow_mut().push_back(report());
		let changed = RcBlock::new(move |_: NonNull<NSNotification>| {
			events.borrow_mut().push_back(report());
			post_wake_event();
		});
		let center = NSWorkspace::sharedWorkspace().notificationCenter();
		// SAFETY: The name is AppKit's own constant, and the block matches the observer signature.
		let observer = unsafe {
			center.addObserverForName_object_queue_usingBlock(
				Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
				None,
				Some(&NSOperationQueue::mainQueue()),
				&changed,
			)
		};
		Self { observer }
	}
}

impl Drop for PreferenceWatch {
	fn drop(&mut self) {
		let center = NSWorkspace::sharedWorkspace().notificationCenter();
		// SAFETY: The observer came from this center's `addObserverForName:object:queue:usingBlock:`.
		unsafe { center.removeObserver(AsRef::<AnyObject>::as_ref(&*self.observer)) };
	}
}

/// The preferences as the display settings stand now. macOS has no app-wide text size, so it reports none.
fn report() -> Event {
	let workspace = NSWorkspace::sharedWorkspace();
	Event::App(AppEvents::Preferences(Preferences {
		reduce_motion: Some(workspace.accessibilityDisplayShouldReduceMotion()),
		increase_contrast: Some(workspace.accessibilityDisplayShouldIncreaseContrast()),
		text_scale: None,
	}))
}
