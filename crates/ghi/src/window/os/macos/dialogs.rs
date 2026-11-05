//! Opens AppKit panels and alerts for [`crate::window::App::open_dialog`].
//!
//! Every dialog starts without blocking: completion handlers and the color panel's actions run while
//! [`super::App::poll`] dispatches events, and push their results into the pump's event queue.

use std::cell::Cell;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send, sel};
use objc2_app_kit::{
	NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApp, NSColor, NSColorPanel, NSColorSpace, NSModalResponse,
	NSModalResponseAbort, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{NSArray, NSNotification, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;

use super::{EventQueue, post_wake_event};
use crate::window::{AppEvents, Button, Buttons, Dialog, DialogError, DialogId, DialogResult, Event, FileFilter, MessageLevel};

/// Queues a dialog result and wakes a waiting poll, since handlers may run outside an `NSEvent` dispatch.
fn push(events: &EventQueue, id: DialogId, result: DialogResult) {
	events.borrow_mut().push_back(Event::App(AppEvents::Dialog { id, result }));
	post_wake_event();
}

/// Opens `dialog` and arranges for its results to reach `events`.
///
/// `color` holds the target of the one color panel AppKit offers, so a second color dialog is refused while the first
/// is open.
pub(super) fn open(
	mtm: MainThreadMarker,
	events: &EventQueue,
	color: &mut Option<Retained<ColorTarget>>,
	id: DialogId,
	dialog: &Dialog,
	parent: Option<&NSWindow>,
) -> Result<(), DialogError> {
	match *dialog {
		Dialog::Open {
			title,
			directory,
			filters,
			multiple,
			folders,
		} => {
			let panel = NSOpenPanel::openPanel(mtm);
			panel.setCanChooseFiles(!folders);
			panel.setCanChooseDirectories(folders);
			panel.setAllowsMultipleSelection(multiple);
			configure_save_panel(&panel, title, directory, if folders { &[] } else { filters });
			let reader = panel.clone();
			begin(&panel, parent, events, id, move || {
				reader.URLs().iter().filter_map(|url| url.to_file_path()).collect()
			});
		}
		Dialog::Save {
			title,
			directory,
			file_name,
			filters,
		} => {
			let panel = NSSavePanel::savePanel(mtm);
			panel.setNameFieldStringValue(&NSString::from_str(file_name));
			configure_save_panel(&panel, title, directory, filters);
			let reader = panel.clone();
			begin(&panel, parent, events, id, move || {
				reader.URL().and_then(|url| url.to_file_path()).into_iter().collect()
			});
		}
		Dialog::Color { initial, alpha } => {
			if color.as_ref().is_some_and(|target| !target.ivars().done.get()) {
				return Err(DialogError::Busy);
			}
			let panel = NSColorPanel::sharedColorPanel(mtm);
			let target = ColorTarget::new(mtm, events.clone(), id, (!alpha).then_some(initial[3]));
			panel.setShowsAlpha(alpha);
			panel.setContinuous(true);
			panel.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
				initial[0] as f64,
				initial[1] as f64,
				initial[2] as f64,
				initial[3] as f64,
			));
			// SAFETY: `target` implements `changeColor:` and outlives the panel's use of it: `color` keeps it until
			// the next color dialog replaces it, and the panel drops its references when it closes.
			unsafe {
				panel.setTarget(Some(&target));
				panel.setAction(Some(sel!(changeColor:)));
			}
			panel.setDelegate(Some(ProtocolObject::from_ref(&*target)));
			panel.makeKeyAndOrderFront(None);
			*color = Some(target);
		}
		Dialog::Message {
			level,
			title,
			text,
			buttons,
		} => {
			let alert = NSAlert::new(mtm);
			alert.setAlertStyle(match level {
				MessageLevel::Info => NSAlertStyle::Informational,
				MessageLevel::Warning => NSAlertStyle::Warning,
				MessageLevel::Error => NSAlertStyle::Critical,
			});
			alert.setMessageText(&NSString::from_str(title));
			alert.setInformativeText(&NSString::from_str(text));
			let order = button_order(buttons);
			for button in order {
				alert.addButtonWithTitle(&NSString::from_str(button_title(*button)));
			}
			// The first, second, and third buttons added answer with consecutive responses.
			let result = move |response: NSModalResponse| {
				let index = (response - NSAlertFirstButtonReturn) as usize;
				DialogResult::Button(order.get(index).copied().unwrap_or(Button::Cancel))
			};
			// An alert has no modeless form, so it attaches to the key window when it has no parent and only runs
			// modally, holding up this call, when the application has no window to attach to.
			match parent.map(|window| window.retain()).or_else(|| NSApp(mtm).keyWindow()) {
				Some(window) => {
					let events = events.clone();
					let handler = RcBlock::new(move |response| push(&events, id, result(response)));
					alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
				}
				None => push(events, id, result(alert.runModal())),
			}
		}
	}
	Ok(())
}

/// Applies the settings open and save panels share.
fn configure_save_panel(panel: &NSSavePanel, title: &str, directory: Option<&std::path::Path>, filters: &[FileFilter]) {
	panel.setTitle(Some(&NSString::from_str(title)));
	panel.setMessage(Some(&NSString::from_str(title)));
	if let Some(url) = directory.and_then(NSURL::from_directory_path) {
		panel.setDirectoryURL(Some(&url));
	}
	// AppKit panels offer no named filter menu, so every filter's extensions are accepted together.
	let types: Vec<Retained<UTType>> = filters
		.iter()
		.flat_map(|filter| filter.extensions)
		.filter_map(|extension| UTType::typeWithFilenameExtension(&NSString::from_str(extension)))
		.collect();
	if !types.is_empty() {
		panel.setAllowedContentTypes(&NSArray::from_retained_slice(&types));
	}
}

/// Shows a panel as a sheet on `parent`, or on its own without one, and reports `paths()` when the user confirms.
fn begin(
	panel: &NSSavePanel,
	parent: Option<&NSWindow>,
	events: &EventQueue,
	id: DialogId,
	paths: impl Fn() -> Vec<std::path::PathBuf> + 'static,
) {
	let events = events.clone();
	let handler = RcBlock::new(move |response: NSModalResponse| {
		let result = if response == NSModalResponseOK {
			DialogResult::Files(paths())
		} else if response == NSModalResponseAbort {
			DialogResult::Failed(DialogError::Os("the panel could not be shown".into()))
		} else {
			DialogResult::Cancelled
		};
		push(&events, id, result);
	});
	match parent {
		Some(window) => panel.beginSheetModalForWindow_completionHandler(window, &handler),
		None => panel.beginWithCompletionHandler(&handler),
	}
}

/// Lists a button set in the order the alert adds them, which is the order its responses count in.
fn button_order(buttons: Buttons) -> &'static [Button] {
	match buttons {
		Buttons::Ok => &[Button::Ok],
		Buttons::OkCancel => &[Button::Ok, Button::Cancel],
		Buttons::YesNo => &[Button::Yes, Button::No],
		Buttons::YesNoCancel => &[Button::Yes, Button::No, Button::Cancel],
	}
}

fn button_title(button: Button) -> &'static str {
	match button {
		Button::Ok => "OK",
		Button::Cancel => "Cancel",
		Button::Yes => "Yes",
		Button::No => "No",
	}
}

/// Reads `color` as sRGB components, replacing alpha with `fixed_alpha` when the dialog hides it.
fn srgba(color: &NSColor, fixed_alpha: Option<f32>) -> [f32; 4] {
	let color = color
		.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
		.unwrap_or_else(|| color.retain());
	[
		color.redComponent() as f32,
		color.greenComponent() as f32,
		color.blueComponent() as f32,
		fixed_alpha.unwrap_or(color.alphaComponent() as f32),
	]
}

pub(super) struct ColorTargetIvars {
	events: EventQueue,
	id: DialogId,
	/// The alpha every result keeps when the dialog hides alpha.
	fixed_alpha: Option<f32>,
	/// Whether the dialog sent its final result, after which a new color dialog may take the panel.
	done: Cell<bool>,
}

define_class!(
	/// The `ColorTarget` class receives the shared color panel's actions for one color dialog, because AppKit reports
	/// color changes to a target object rather than to a completion handler.
	#[unsafe(super = NSObject)]
	#[thread_kind = MainThreadOnly]
	#[ivars = ColorTargetIvars]
	pub(super) struct ColorTarget;

	// SAFETY: `ColorTarget` inherits from NSObject and uses objc2's generated object layout and lifecycle.
	unsafe impl NSObjectProtocol for ColorTarget {}

	// SAFETY: The exported selector has the signature NSWindowDelegate requires and runs on the main thread.
	unsafe impl NSWindowDelegate for ColorTarget {
		#[unsafe(method(windowWillClose:))]
		fn window_will_close(&self, _notification: &NSNotification) {
			self.report(true);
		}
	}

	impl ColorTarget {
		#[unsafe(method(changeColor:))]
		fn change_color(&self, _sender: Option<&AnyObject>) {
			self.report(false);
		}
	}
);

impl ColorTarget {
	fn new(mtm: MainThreadMarker, events: EventQueue, id: DialogId, fixed_alpha: Option<f32>) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(ColorTargetIvars {
			events,
			id,
			fixed_alpha,
			done: Cell::new(false),
		});
		// SAFETY: `this` is a freshly allocated subclass with initialized ivars and the inherited NSObject initializer.
		unsafe { msg_send![super(this), init] }
	}

	/// Sends the panel's color, and on `done` detaches from the panel so later uses of it reach no closed dialog.
	fn report(&self, done: bool) {
		let ivars = self.ivars();
		if ivars.done.get() {
			return;
		}
		let panel = NSColorPanel::sharedColorPanel(self.mtm());
		let color = srgba(&panel.color(), ivars.fixed_alpha);
		if done {
			ivars.done.set(true);
			// SAFETY: Clearing the target and action leaves the panel without a receiver, which AppKit allows.
			unsafe {
				panel.setTarget(None);
				panel.setAction(None);
			}
			panel.setDelegate(None);
		}
		push(&ivars.events, ivars.id, DialogResult::Color { color, done });
	}
}
