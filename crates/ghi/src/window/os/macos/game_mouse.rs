//! Reads mouse motion and buttons before pointer acceleration from the Game Controller framework, for the
//! [`PointingDevice::Mouse`] events.

use std::{cell::Cell, ptr::NonNull, rc::Rc};

use block2::RcBlock;
use objc2::{
	MainThreadMarker,
	rc::Retained,
	runtime::{AnyObject, Bool, ProtocolObject},
};
use objc2_app_kit::NSApp;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSProcessInfo};
use objc2_game_controller::{
	GCControllerButtonInput, GCMouse, GCMouseDidConnectNotification, GCMouseDidDisconnectNotification, GCMouseInput,
};

use super::{EventQueue, post_wake_event, window_id};
use crate::window::{Event, Events, PointingDevice, Seat, input::MouseKeys};

/// The `GameMice` struct keeps every Game Controller mouse reporting into the app's event queue, so the pump knows
/// whether [`PointingDevice::Mouse`] has a source of its own or repeats the cursor's events.
///
/// Create it once with the app; dropping it stops the reports.
pub(super) struct GameMice {
	shared: Rc<Shared>,
	/// The connect and disconnect observers, removed on drop.
	observers: [Retained<ProtocolObject<dyn NSObjectProtocol>>; 2],
}

/// The `Shared` struct is the state the Game Controller handlers and the pump both reach.
struct Shared {
	events: EventQueue,
	/// How many Game Controller mice are connected.
	connected: Cell<usize>,
	/// Whether a wake event is already queued for the events handlers added since the last poll, so a waiting poll
	/// wakes once instead of once per motion.
	wake_posted: Cell<bool>,
}

impl GameMice {
	/// Starts reading every connected Game Controller mouse and the ones that connect later.
	///
	/// Returns `None` before macOS 14, whose Game Controller mice report broken motion and button events.
	pub(super) fn new(events: EventQueue) -> Option<Self> {
		if !objc2::available!(macos = 14.0) {
			return None;
		}

		let shared = Rc::new(Shared {
			events,
			connected: Cell::new(0),
			wake_posted: Cell::new(false),
		});

		// The handlers and observers run on the main queue, which the pump drains while it reads AppKit events, so
		// they share the pump's queue without locks.
		let connect = {
			let shared = shared.clone();
			RcBlock::new(move |notification: NonNull<NSNotification>| {
				// SAFETY: Notification Center passes a valid notification for the duration of the call.
				let notification = unsafe { notification.as_ref() };
				if let Some(mouse) = notification.object().and_then(|object| object.downcast::<GCMouse>().ok()) {
					shared.attach(&mouse);
				}
			})
		};
		let disconnect = {
			let shared = shared.clone();
			RcBlock::new(move |_: NonNull<NSNotification>| shared.count())
		};

		let center = NSNotificationCenter::defaultCenter();
		let queue = NSOperationQueue::mainQueue();
		// SAFETY: The names are Game Controller's own constants, and the blocks match the observer signature.
		let observers = unsafe {
			[
				center.addObserverForName_object_queue_usingBlock(
					Some(GCMouseDidConnectNotification),
					None,
					Some(&queue),
					&connect,
				),
				center.addObserverForName_object_queue_usingBlock(
					Some(GCMouseDidDisconnectNotification),
					None,
					Some(&queue),
					&disconnect,
				),
			]
		};

		// Mice connected before the observers report no notification.
		for mouse in unsafe { GCMouse::mice() }.iter() {
			shared.attach(&mouse);
		}

		Some(Self { shared, observers })
	}

	/// Returns whether a Game Controller mouse is connected, so its events, not the cursor's, are the
	/// [`PointingDevice::Mouse`] events.
	pub(super) fn connected(&self) -> bool {
		self.shared.connected.get() > 0
	}

	/// Lets the handlers post another wake event. Call it at the start of every poll.
	pub(super) fn rearm_wake(&self) {
		self.shared.wake_posted.set(false);
	}
}

impl Drop for GameMice {
	fn drop(&mut self) {
		let center = NSNotificationCenter::defaultCenter();
		for observer in &self.observers {
			// SAFETY: Each observer came from this center's `addObserverForName:object:queue:usingBlock:`.
			unsafe { center.removeObserver(AsRef::<AnyObject>::as_ref(&**observer)) };
		}
		// The handlers keep the shared state alive, so clearing them lets it go.
		for mouse in unsafe { GCMouse::mice() }.iter() {
			if let Some(input) = unsafe { mouse.mouseInput() } {
				detach(&input);
			}
		}
	}
}

impl Shared {
	/// Counts the connected mice again after one connects or disconnects.
	fn count(&self) {
		self.connected.set(unsafe { GCMouse::mice() }.count());
	}

	/// Routes the mouse's motion and buttons into the event queue.
	fn attach(self: &Rc<Self>, mouse: &GCMouse) {
		self.count();
		let Some(input) = (unsafe { mouse.mouseInput() }) else {
			return;
		};

		let moved = {
			let shared = self.clone();
			// Game Controller deltas grow upward, as `MouseMove` does.
			RcBlock::new(move |_: NonNull<GCMouseInput>, dx: f32, dy: f32| {
				shared.push(Events::MouseMove {
					seat: Seat::stub(),
					device: PointingDevice::Mouse,
					dx,
					dy,
					time: now(),
				});
			})
		};
		// SAFETY: The block matches `GCMouseMoved`, and the setter copies it.
		unsafe { input.setMouseMovedHandler(RcBlock::as_ptr(&moved)) };

		let buttons = [
			(Some(unsafe { input.leftButton() }), MouseKeys::Left),
			(unsafe { input.rightButton() }, MouseKeys::Right),
			(unsafe { input.middleButton() }, MouseKeys::Middle),
		];
		for (input, button) in buttons {
			let Some(input) = input else {
				continue;
			};
			let shared = self.clone();
			let changed = RcBlock::new(move |_: NonNull<GCControllerButtonInput>, _: f32, pressed: Bool| {
				shared.push(Events::Button {
					seat: Seat::stub(),
					device: PointingDevice::Mouse,
					pressed: pressed.as_bool(),
					button,
				});
			});
			// SAFETY: The block matches `GCControllerButtonValueChangedHandler`, and the setter copies it.
			unsafe { input.setPressedChangedHandler(RcBlock::as_ptr(&changed)) };
		}
	}

	/// Queues `event` for the focused window and wakes a waiting poll. Input goes to the window with focus, so it's
	/// dropped while the app has none.
	fn push(&self, event: Events) {
		let Some(mtm) = MainThreadMarker::new() else {
			return;
		};
		let app = NSApp(mtm);
		let Some(window) = app.keyWindow().filter(|_| app.isActive()) else {
			return;
		};
		self.events.borrow_mut().push_back(Event::Window {
			window: window_id(&window),
			event,
		});
		if !self.wake_posted.replace(true) {
			post_wake_event();
		}
	}
}

/// Clears the handlers [`Shared::attach`] set.
fn detach(input: &GCMouseInput) {
	// SAFETY: A null handler is the documented way to stop reports.
	unsafe {
		input.setMouseMovedHandler(std::ptr::null_mut());
		input.leftButton().setPressedChangedHandler(std::ptr::null_mut());
		if let Some(button) = input.rightButton() {
			button.setPressedChangedHandler(std::ptr::null_mut());
		}
		if let Some(button) = input.middleButton() {
			button.setPressedChangedHandler(std::ptr::null_mut());
		}
	}
}

/// Returns the time since boot in milliseconds, the clock AppKit stamps its events with.
fn now() -> u64 {
	(NSProcessInfo::processInfo().systemUptime() * 1000.0) as u64
}
