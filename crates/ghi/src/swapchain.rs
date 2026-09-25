use std::time::{Duration, Instant};

use crate::{PrivateHandle, PrivateHandles};

/// Sleeps until the next paced present slot and advances it by `interval`.
///
/// Backends without a timed present call this before acquisition so the frame cannot start earlier than the
/// interval allows; the following FIFO present then lands on the next refresh. A missed slot restarts from now
/// instead of catching up. Without an interval the slot is cleared and the call returns immediately.
pub(crate) fn pace_present(next_slot: &mut Option<Instant>, interval: Option<Duration>) {
	let Some(interval) = interval else {
		*next_slot = None;
		return;
	};
	let now = Instant::now();
	let slot = match *next_slot {
		Some(slot) if slot > now => {
			std::thread::sleep(slot - now);
			slot
		}
		_ => now,
	};
	*next_slot = Some(slot + interval);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SwapchainHandle(pub(crate) u64);

impl From<SwapchainHandle> for PrivateHandles {
	fn from(val: SwapchainHandle) -> Self {
		PrivateHandles::Swapchain(val)
	}
}

impl PrivateHandle for SwapchainHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}
