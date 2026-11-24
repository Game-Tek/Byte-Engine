//! Paces a display-synced swapchain by a `CAMetalDisplayLink` on the main thread's run loop.
//!
//! A display link calls back only while its run loop turns. The main thread turns its run loop while the application
//! waits for window events, so the link fires inside that wait: each update goes into a one-update slot, and the link
//! posts the event that ends the wait. The tick then waits for the display and handles window events in the same
//! call, and acquisition takes the update without waiting. However long the display holds drawables back, as it can
//! for a window that another window just covered, the application keeps handling events.

#![expect(
	clippy::disallowed_types,
	reason = "objc2 hands the display link's ivars out only by shared reference, so the update that the OS callback and the swapchain both replace sits in a `Cell`; see `Slot`"
)]

use std::cell::Cell;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread as _, DefinedClass, MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send};
use objc2_foundation::{NSDefaultRunLoopMode, NSRunLoop};
use objc2_quartz_core::{
	CAFrameRateRange, CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalLayer,
};

/// The latest display opportunity nobody took yet, kept in the delegate's ivars, its only owner.
///
/// It's a `Cell` because objc2 hands ivars out only by shared reference, while both the link's callback and the
/// swapchain replace the update. Only the main thread reaches it, and `Cell` never lends a reference into it, so a
/// callback during a swapchain call can't see it half-changed.
type Slot = Cell<Option<Retained<CAMetalDisplayLinkUpdate>>>;

/// The `DisplayLink` struct paces one display-synced swapchain by the display.
///
/// Each display opportunity, a drawable plus the time it will be shown, arrives during the window event wait. Ask
/// [`Self::ready`] whether one arrived, and take it with [`Self::take`] when the swapchain is acquired. Dropping it
/// stops the link.
pub(crate) struct DisplayLink {
	link: Retained<CAMetalDisplayLink>,
	/// The link holds its delegate weakly, so the swapchain keeps it alive for as long as the link runs. It keeps the
	/// update slot.
	delegate: Retained<LinkDelegate>,
}

impl DisplayLink {
	/// Starts a display link for `layer` on the main run loop, waking `frame_latency` refreshes before each frame is due.
	pub(crate) fn new(layer: &CAMetalLayer, frame_latency: u8, mtm: MainThreadMarker) -> Self {
		let link = CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), layer);
		let delegate = LinkDelegate::new(mtm);
		link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
		link.setPreferredFrameLatency(frame_latency as f32);
		// The window event wait turns the main run loop in the default mode, so the link fires there.
		// SAFETY: The main run loop is this thread's own, and the link is added to it from the main thread.
		unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSDefaultRunLoopMode) };
		Self { link, delegate }
	}

	/// Returns whether an update is waiting, so acquiring the swapchain now would not skip the frame.
	pub(crate) fn ready(&self) -> bool {
		let slot = self.delegate.ivars();
		// `Cell` lends no reference, so the update is taken out to look at and put back.
		let update = slot.take();
		let ready = update.is_some();
		slot.set(update);
		ready
	}

	/// Takes the latest display opportunity, or `None` when the display offered none since the last one was taken.
	pub(crate) fn take(&self) -> Option<Retained<CAMetalDisplayLinkUpdate>> {
		self.delegate.ivars().take()
	}

	/// Sets the minimum interval between frames, or removes it with `None`.
	///
	/// A fixed range lets the display pick a refresh rate that shows each frame for the same time, instead of holding
	/// frames on a faster refresh.
	pub(crate) fn set_frame_interval(&self, interval: Option<Duration>) {
		let rate = interval.map_or(0.0, |interval| (1.0 / interval.as_secs_f64()) as f32);
		self.link.setPreferredFrameRateRange(CAFrameRateRange {
			minimum: rate,
			maximum: rate,
			preferred: rate,
		});
	}
}

impl Drop for DisplayLink {
	fn drop(&mut self) {
		// The run loop retains the link until it is invalidated.
		self.link.invalidate();
	}
}

define_class!(
	#[unsafe(super = NSObject)]
	#[thread_kind = MainThreadOnly]
	#[ivars = Slot]
	struct LinkDelegate;

	// SAFETY: `LinkDelegate` inherits from NSObject and uses objc2's generated object layout and lifecycle.
	unsafe impl NSObjectProtocol for LinkDelegate {}

	// SAFETY: The exported selector has the signature `CAMetalDisplayLinkDelegate` requires. The link runs on the main
	// run loop, so it calls back on the main thread.
	unsafe impl CAMetalDisplayLinkDelegate for LinkDelegate {
		#[unsafe(method(metalDisplayLink:needsUpdate:))]
		fn metal_display_link_needs_update(&self, _link: &CAMetalDisplayLink, update: &CAMetalDisplayLinkUpdate) {
			// An update nobody took yet is released: it stands for a refresh that has passed or is about to, so
			// presenting for it would show the frame late and report a display time that is already wrong. Releasing it
			// returns its drawable to the layer.
			drop(self.ivars().replace(Some(update.retain())));
			// Ends the window event wait, which checks the slot.
			crate::window::os::macos::AppWaker.wake();
		}
	}
);

impl LinkDelegate {
	fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(Slot::default());
		// SAFETY: `this` is a freshly allocated subclass with initialized ivars and the inherited NSObject initializer.
		unsafe { msg_send![super(this), init] }
	}
}
