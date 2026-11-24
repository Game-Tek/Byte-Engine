//! Keeps the state that native callbacks share with the pump in one place both can reach.
//!
//! AppKit delegates and Win32 window procedures are called by the operating system, with only a raw pointer of
//! context, often while the pump is inside a call of its own such as `sendEvent:` or `DispatchMessageW`. The state they
//! share lives in one heap block that the [`super::App`] owns and frees last. Every access forms a `&mut` for the
//! length of one closure, and a closure must never call into the operating system in a way that can call a callback
//! back, because that callback would form a second `&mut`. Debug builds check the rule.

use std::ptr::NonNull;

/// The `Callbacks` struct owns the block that native callbacks and the pump share, so the block outlives every
/// callback that can still reach it.
///
/// Create it before registering any callback, hand [`Self::pointer`] to the operating system, and declare it after
/// every field whose drop unregisters a callback, so it is freed last.
pub(crate) struct Callbacks<T> {
	block: Shared<T>,
}

/// The `Shared` struct is the raw pointer to a [`Callbacks`] block that callbacks receive as their context.
pub(crate) struct Shared<T> {
	block: NonNull<Block<T>>,
}

// The pointer is only an address; copying it creates no access.
impl<T> Clone for Shared<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T> Copy for Shared<T> {}

/// The heap block behind a [`Callbacks`].
struct Block<T> {
	/// Whether a [`Shared::with`] closure is running, so a nested one is caught in debug builds.
	#[cfg(debug_assertions)]
	borrowed: bool,
	value: T,
}

impl<T> Callbacks<T> {
	pub(crate) fn new(value: T) -> Self {
		let block = Box::new(Block {
			#[cfg(debug_assertions)]
			borrowed: false,
			value,
		});
		Self {
			block: Shared {
				block: NonNull::from(Box::leak(block)),
			},
		}
	}

	/// Returns the pointer callbacks reach the block through. It stays valid until this value drops.
	pub(crate) fn pointer(&self) -> Shared<T> {
		self.block
	}

	/// Runs `f` with the block; see [`Shared::with`].
	pub(crate) fn with<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
		self.block.with(f)
	}
}

impl<T> Drop for Callbacks<T> {
	fn drop(&mut self) {
		// SAFETY: The block came from `Box::leak` in `new`, and its owner unregistered every callback before this drop.
		drop(unsafe { Box::from_raw(self.block.block.as_ptr()) });
	}
}

impl<T> Shared<T> {
	/// Returns the pointer a callback received as its context, or `None` for a null one.
	// Only the window procedure receives its context as an untyped pointer.
	#[cfg(target_os = "windows")]
	///
	/// # Safety
	///
	/// A non-null `raw` must come from [`Callbacks::pointer`] of a block that is still alive.
	pub(crate) unsafe fn from_raw(raw: *mut std::ffi::c_void) -> Option<Self> {
		NonNull::new(raw.cast()).map(|block| Self { block })
	}

	/// Returns the pointer to hand the operating system as a callback's context.
	#[cfg(target_os = "windows")]
	pub(crate) fn as_raw(self) -> *mut std::ffi::c_void {
		self.block.as_ptr().cast()
	}

	/// Runs `f` with the only `&mut` to the block that exists while it runs.
	///
	/// `f` must not call anything that can run a callback that reaches this block, such as dispatching a message,
	/// turning the run loop, or showing a window. Debug builds panic when a callback breaks the rule.
	pub(crate) fn with<R>(self, f: impl FnOnce(&mut T) -> R) -> R {
		let block = self.block.as_ptr();
		// SAFETY: The owner keeps the block alive while any callback can run, and every access happens on the one
		// thread that runs the callbacks. The flag is read and written through the raw pointer, before and after the
		// `&mut` below exists.
		#[cfg(debug_assertions)]
		unsafe {
			assert!(
				!(*block).borrowed,
				"The window backend's callback state was reached while it was already in use. The most likely cause is a call that runs native callbacks from inside a `Shared::with` closure. Move that call out of the closure."
			);
			(*block).borrowed = true;
		}
		// SAFETY: As above; the flag guarantees no other `&mut` to the block exists in debug builds, and the rule on
		// `f` guarantees it in release builds.
		let result = f(unsafe { &mut (*block).value });
		#[cfg(debug_assertions)]
		// SAFETY: The `&mut` given to `f` ended with its call.
		unsafe {
			(*block).borrowed = false;
		}
		result
	}
}
