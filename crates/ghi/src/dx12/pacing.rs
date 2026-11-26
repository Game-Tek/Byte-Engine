//! Frame pacing for DXGI swapchains: which sync interval holds a capped frame rate on refreshes, when an acquired
//! frame will reach the display, and how DXGI's performance-counter timestamps map onto [`Instant`].
//!
//! Everything here is a pure function of its inputs, so the swapchain code in `context/presentation.rs` decides what
//! to query and this module decides what the answers mean.

use std::time::{Duration, Instant};

/// How far a requested frame interval may stray from a whole number of refreshes, in refreshes, and still be shown
/// with a DXGI sync interval. It absorbs fractional rates such as 59.94 Hz against a 30 fps cap.
const SYNC_INTERVAL_TOLERANCE: f64 = 0.05;

/// The largest sync interval `IDXGISwapChain::Present` accepts.
const MAX_SYNC_INTERVAL: u32 = 4;

/// Returns the DXGI sync interval that shows each frame for `interval`, when `interval` is one to four refreshes of
/// `refresh`.
///
/// Presenting with that sync interval lets the display hold frames apart on its own refreshes, so the cap never drifts
/// against them. Returns `None` for any other interval, which the caller paces by sleeping instead.
pub(crate) fn sync_interval_for(interval: Duration, refresh: Duration) -> Option<u32> {
	if refresh.is_zero() {
		return None;
	}
	let refreshes = interval.as_secs_f64() / refresh.as_secs_f64();
	let rounded = refreshes.round();
	((1.0..=f64::from(MAX_SYNC_INTERVAL)).contains(&rounded) && (refreshes - rounded).abs() <= SYNC_INTERVAL_TOLERANCE)
		.then_some(rounded as u32)
}

/// Returns how many presents a swapchain of `image_count` buffers may queue ahead of the display for `frames` frames in
/// flight.
///
/// One buffer is always on screen, so at most `image_count - 1` can wait in the queue without `Present` blocking.
pub(crate) fn frame_latency(frames: u8, image_count: u8) -> u32 {
	u32::from(frames.min(image_count.saturating_sub(1)).max(1))
}

/// The `ShownFrame` struct describes the most recent present the display showed, as `IDXGISwapChain::GetFrameStatistics`
/// reports it; [`predict_target`] projects the next frame from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShownFrame {
	/// When the refresh that first showed the present began.
	pub(crate) at: Instant,
	/// How many presents were queued after it and not yet shown when the statistics were read.
	pub(crate) pending: u32,
}

/// Predicts when the frame acquired now will be shown, if it presents in time.
///
/// Each queued present takes `sync_interval` refreshes, so the new frame lands that many refreshes after the last one
/// still in the queue. The prediction never lies before the next refresh after `now`, nor less than one present step
/// after `previous`, the target reported for the frame before it: a FIFO queue cannot show two frames on one refresh.
pub(crate) fn predict_target(
	shown: ShownFrame,
	sync_interval: u32,
	refresh: Duration,
	now: Instant,
	previous: Option<Instant>,
) -> Instant {
	let step = refresh * sync_interval;
	let mut target = shown.at + step * (shown.pending + 1);
	if target <= now {
		// Statistics lag the display after a stall; the frame cannot be shown before the next refresh on the same grid.
		let refreshes_since = (now - shown.at).as_nanos().div_ceil(refresh.as_nanos().max(1));
		target = shown.at + refresh * u32::try_from(refreshes_since).unwrap_or(u32::MAX);
	}
	match previous {
		Some(previous) => target.max(previous + step),
		None => target,
	}
}

/// Converts a `QueryPerformanceCounter` reading, such as `DXGI_FRAME_STATISTICS::SyncQPCTime`, into an [`Instant`].
///
/// The counter and `Instant::now` are sampled back to back, so the result is off by at most the time between the two
/// reads. Returns `None` when the counter cannot be read or the time falls outside what `Instant` can hold.
pub(crate) fn qpc_to_instant(qpc: i64) -> Option<Instant> {
	use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

	let mut now_qpc = 0;
	let mut frequency = 0;
	// SAFETY: Both functions only write the provided integers.
	unsafe {
		QueryPerformanceCounter(&mut now_qpc).ok()?;
		QueryPerformanceFrequency(&mut frequency).ok()?;
	}
	let now = Instant::now();
	ticks_to_instant(qpc, now_qpc, frequency, now)
}

/// Places a counter reading of `qpc` relative to `now`, which was taken when the counter read `now_qpc`.
fn ticks_to_instant(qpc: i64, now_qpc: i64, frequency: i64, now: Instant) -> Option<Instant> {
	if frequency <= 0 {
		return None;
	}
	let ticks = |ticks: i64| {
		let ticks = u128::try_from(ticks).ok()?;
		let nanoseconds = ticks * 1_000_000_000 / frequency as u128;
		Some(Duration::from_nanos(u64::try_from(nanoseconds).ok()?))
	};
	if qpc <= now_qpc {
		now.checked_sub(ticks(now_qpc - qpc)?)
	} else {
		now.checked_add(ticks(qpc - now_qpc)?)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const HZ_60: Duration = Duration::from_nanos(16_666_667);

	#[test]
	fn sync_interval_matches_only_whole_refreshes_dxgi_accepts() {
		assert_eq!(sync_interval_for(HZ_60, HZ_60), Some(1));
		assert_eq!(sync_interval_for(Duration::from_secs_f64(1.0 / 30.0), HZ_60), Some(2));
		// 30 fps on a 59.94 Hz display.
		assert_eq!(
			sync_interval_for(Duration::from_secs_f64(1.0 / 30.0), Duration::from_secs_f64(1.0 / 59.94)),
			Some(2)
		);
		assert_eq!(sync_interval_for(Duration::from_secs_f64(1.0 / 15.0), HZ_60), Some(4));
		// 60 fps on a 144 Hz display is 2.4 refreshes.
		assert_eq!(
			sync_interval_for(Duration::from_secs_f64(1.0 / 60.0), Duration::from_secs_f64(1.0 / 144.0)),
			None
		);
		// Five refreshes is beyond what DXGI accepts.
		assert_eq!(sync_interval_for(HZ_60 * 5, HZ_60), None);
		assert_eq!(sync_interval_for(HZ_60 / 2, HZ_60), None);
		assert_eq!(sync_interval_for(HZ_60, Duration::ZERO), None);
	}

	#[test]
	fn frame_latency_leaves_a_buffer_on_screen() {
		assert_eq!(frame_latency(2, 3), 2);
		assert_eq!(frame_latency(2, 2), 1);
		assert_eq!(frame_latency(3, 8), 3);
		assert_eq!(frame_latency(1, 2), 1);
	}

	#[test]
	fn targets_follow_the_queue_and_stay_ahead() {
		let start = Instant::now();
		let shown = ShownFrame { at: start, pending: 1 };
		// One queued frame ahead, so the new frame lands two refreshes after the shown one.
		assert_eq!(predict_target(shown, 1, HZ_60, start, None), start + HZ_60 * 2);
		// With a sync interval of two, every present takes two refreshes.
		assert_eq!(predict_target(shown, 2, HZ_60, start, None), start + HZ_60 * 4);

		// A target never lies before the next refresh.
		let shown = ShownFrame { at: start, pending: 0 };
		let now = start + HZ_60 * 5 + HZ_60 / 2;
		assert_eq!(predict_target(shown, 1, HZ_60, now, None), start + HZ_60 * 6);

		// Statistics that lag report fewer queued frames than there are; the previous target still holds the order.
		let previous = start + HZ_60 * 3;
		assert_eq!(predict_target(shown, 1, HZ_60, start, Some(previous)), previous + HZ_60);
	}

	#[test]
	fn counter_readings_map_around_now() {
		let now = Instant::now();
		let frequency = 10_000_000;
		assert_eq!(ticks_to_instant(1_000, 1_000, frequency, now), Some(now));
		assert_eq!(
			ticks_to_instant(0, 10_000, frequency, now),
			now.checked_sub(Duration::from_millis(1))
		);
		assert_eq!(
			ticks_to_instant(20_000, 10_000, frequency, now),
			Some(now + Duration::from_millis(1))
		);
		assert_eq!(ticks_to_instant(0, 0, 0, now), None);
	}
}
