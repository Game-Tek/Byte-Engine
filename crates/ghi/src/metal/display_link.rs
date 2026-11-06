//! Paces a swapchain by a `CAMetalDisplayLink` instead of by `nextDrawable`.
//!
//! A display link calls back from the run loop it is attached to. The thread that records frames only turns its run
//! loop while it pumps window events, so each link runs on a thread of its own, whose only job is to hand every update
//! to the recording thread through a one-update mailbox. The recording thread takes updates when it acquires the
//! swapchain, which keeps frame pacing in the caller's loop.

use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread as _, DefinedClass, Message as _, define_class, msg_send};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_foundation::{NSDefaultRunLoopMode, NSRunLoop};
use objc2_quartz_core::{
	CAFrameRateRange, CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate, CAMetalLayer,
};

/// How long acquisition waits for an update. It matches the timeout of `nextDrawable`, after which Core Animation also
/// gives up on a window that the display does not show.
pub(crate) const UPDATE_TIMEOUT: Duration = Duration::from_secs(1);

/// The `DisplayLink` struct paces one swapchain by the display, for swapchains created with
/// [`crate::device::Features::display_link`].
///
/// Its thread delivers each display opportunity, a drawable plus the time it will be shown, and the swapchain's
/// acquisition takes the latest one with [`Self::take`]. Dropping it stops the thread.
pub(crate) struct DisplayLink {
	shared: Arc<Shared>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl DisplayLink {
	/// Starts a thread that runs a display link for `layer`, waking `frame_latency` refreshes before each frame is due.
	///
	/// Create it on the thread that acquires the swapchain: that is the thread each update wakes. Next, call
	/// [`Self::take`] each time the swapchain is acquired.
	pub(crate) fn new(layer: Retained<CAMetalLayer>, frame_latency: u8) -> Self {
		let shared = Arc::new(Shared {
			update: AtomicPtr::new(std::ptr::null_mut()),
			consumer: std::thread::current(),
			frame_interval: AtomicU64::new(0),
			stopping: AtomicBool::new(false),
		});
		let thread_shared = shared.clone();
		let layer = SendLayer(layer);
		let thread = std::thread::Builder::new()
			.name("Display Link".into())
			.spawn(move || run_display_link(layer, frame_latency, &thread_shared))
			.expect("Failed to start the display link thread. The most likely cause is that the process reached its thread limit. Turn off `render.display-link`, or close other windows.");
		Self {
			shared,
			thread: Some(thread),
		}
	}

	/// Takes the latest display opportunity, waiting until `deadline` for one.
	///
	/// Returns `None` when the display offered none in time, as happens while the window is occluded.
	pub(crate) fn take(&self, deadline: Instant) -> Option<Retained<CAMetalDisplayLinkUpdate>> {
		assert_eq!(
			self.shared.consumer.id(),
			std::thread::current().id(),
			"A display-linked swapchain was acquired from a thread other than the one that created it. The most likely cause is that the context moved to another thread after binding the window. Acquire every swapchain from the thread that bound it.",
		);
		loop {
			let pointer = self.shared.update.swap(std::ptr::null_mut(), Ordering::AcqRel);
			if let Some(pointer) = NonNull::new(pointer) {
				// SAFETY: `publish` stored a pointer it took from a `Retained`, and the swap moved that ownership here.
				return unsafe { Retained::from_raw(pointer.as_ptr()) };
			}
			// Unparking before this park leaves a token, so an update published after the swap is never missed.
			std::thread::park_timeout(deadline.checked_duration_since(Instant::now())?);
		}
	}

	/// Sets the minimum interval between frames, or removes it with `None`.
	///
	/// The link thread applies it as the link's preferred frame rate range on its next update, so no Core Animation
	/// call crosses threads.
	pub(crate) fn set_frame_interval(&self, interval: Option<Duration>) {
		let nanoseconds = interval.map_or(0, |interval| u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX));
		self.shared.frame_interval.store(nanoseconds, Ordering::Release);
	}
}

impl Drop for DisplayLink {
	fn drop(&mut self) {
		// The link thread checks this after each update, and at least every `STOP_CHECK_INTERVAL` while the display
		// sends none.
		self.shared.stopping.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

/// How long the link thread's run loop waits before it checks for a stop while no update arrives.
const STOP_CHECK_INTERVAL: f64 = 0.1;

/// The `Shared` struct is the state a display link thread and the thread that acquires its swapchain both reach.
///
/// Every field is atomic or immutable, so neither thread ever waits on a lock.
struct Shared {
	/// The latest update nobody took yet, as a pointer owned through `Retained`, or null.
	update: AtomicPtr<CAMetalDisplayLinkUpdate>,
	/// The thread that takes updates, which the link thread unparks after publishing.
	consumer: std::thread::Thread,
	/// The requested minimum frame interval in nanoseconds, or 0 to follow the display.
	frame_interval: AtomicU64,
	/// Set when the owner drops, so the link thread leaves its run loop.
	stopping: AtomicBool,
}

impl Shared {
	/// Makes `update` the latest display opportunity and wakes the consumer.
	///
	/// An update nobody took yet is released: it stands for a refresh that has passed or is about to, so presenting
	/// for it would show the frame late and report a display time that is already wrong. Releasing it returns its
	/// drawable to the layer.
	fn publish(&self, update: Retained<CAMetalDisplayLinkUpdate>) {
		let previous = self.update.swap(Retained::into_raw(update), Ordering::AcqRel);
		// SAFETY: The slot owned the previous pointer, and the swap moved that ownership here.
		drop(unsafe { Retained::from_raw(previous) });
		self.consumer.unpark();
	}
}

impl Drop for Shared {
	fn drop(&mut self) {
		// SAFETY: The slot owns its pointer, and nothing else can reach it while the last reference drops.
		drop(unsafe { Retained::from_raw(*self.update.get_mut()) });
	}
}

/// Creates the display link for `layer` on the current thread, then turns this thread's run loop until the owner stops.
fn run_display_link(layer: SendLayer, frame_latency: u8, shared: &Arc<Shared>) {
	let SendLayer(layer) = layer;
	let link = CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), &layer);
	// The link holds its delegate weakly, so this thread keeps it alive for as long as the link runs.
	let delegate = LinkDelegate::new(shared.clone());
	link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
	link.setPreferredFrameLatency(frame_latency as f32);
	// SAFETY: The run loop is this thread's own, and the link is added to it from this thread.
	unsafe { link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSDefaultRunLoopMode) };

	// SAFETY: `kCFRunLoopDefaultMode` is an immutable constant Core Foundation defines.
	let mode = unsafe { kCFRunLoopDefaultMode };
	while !shared.stopping.load(Ordering::Acquire) {
		// Returning after each update lets the thread notice a stop within a refresh while the display runs.
		CFRunLoop::run_in_mode(mode, STOP_CHECK_INTERVAL, true);
	}
	link.invalidate();
}

/// The `LinkDelegateIvars` struct holds what a [`LinkDelegate`] reads on each update.
struct LinkDelegateIvars {
	shared: Arc<Shared>,
	/// The frame interval last applied to the link. Only the link thread reads or writes it.
	applied_frame_interval: std::cell::Cell<u64>,
}

define_class!(
	#[unsafe(super = NSObject)]
	#[ivars = LinkDelegateIvars]
	struct LinkDelegate;

	// SAFETY: `LinkDelegate` inherits from NSObject and uses objc2's generated object layout and lifecycle.
	unsafe impl NSObjectProtocol for LinkDelegate {}

	// SAFETY: The exported selector has the signature `CAMetalDisplayLinkDelegate` requires. It runs on the link thread,
	// the only thread that touches the delegate.
	unsafe impl CAMetalDisplayLinkDelegate for LinkDelegate {
		#[unsafe(method(metalDisplayLink:needsUpdate:))]
		fn metal_display_link_needs_update(&self, link: &CAMetalDisplayLink, update: &CAMetalDisplayLinkUpdate) {
			objc2::rc::autoreleasepool(|_| {
				let ivars = self.ivars();
				let frame_interval = ivars.shared.frame_interval.load(Ordering::Acquire);
				if frame_interval != ivars.applied_frame_interval.replace(frame_interval) {
					link.setPreferredFrameRateRange(frame_rate_range(frame_interval));
				}
				ivars.shared.publish(update.retain());
			});
		}
	}
);

impl LinkDelegate {
	fn new(shared: Arc<Shared>) -> Retained<Self> {
		let this = Self::alloc().set_ivars(LinkDelegateIvars {
			shared,
			// A new link already follows the display, which an interval of 0 asks for.
			applied_frame_interval: std::cell::Cell::new(0),
		});
		// SAFETY: `this` is a freshly allocated subclass with initialized ivars and the inherited NSObject initializer.
		unsafe { msg_send![super(this), init] }
	}
}

/// Returns the frame rate range that holds frames at least `frame_interval` nanoseconds apart, or the range that
/// follows the display for 0.
///
/// A fixed range lets the display pick a refresh rate that shows each frame for the same time, instead of holding
/// frames on a faster refresh.
fn frame_rate_range(frame_interval: u64) -> CAFrameRateRange {
	let rate = if frame_interval == 0 {
		0.0
	} else {
		(1e9 / frame_interval as f64) as f32
	};
	CAFrameRateRange {
		minimum: rate,
		maximum: rate,
		preferred: rate,
	}
}

/// The `SendLayer` struct moves the swapchain's layer to the link thread, which creates its display link.
struct SendLayer(Retained<CAMetalLayer>);

// SAFETY: `CAMetalLayer` is safe to use from a background thread; Metal's own render loops create display links for
// their layers on render threads.
unsafe impl Send for SendLayer {}
