//! Pairs the GPU timestamps a frame writes with the counters that requested them, independently of the backend.
//!
//! A backend asks [`Counters`] for a slot each time a counter starts or ends, writes its native timestamp into that
//! slot, and hands the resolved ticks back through [`Counters::resolve`] once the frame completes. The backend only
//! owns the native timestamp pool; which counter wrote which slot, and the duration each counter measured, lives here.
//! Clients reach this through [`crate::context::ContextCreate::create_counter`],
//! [`crate::command_buffer::CommonCommandBufferMode::counter`], and [`crate::context::Context::counter_duration`].

use std::time::Duration;

use crate::{CounterHandle, MAX_FRAMES_IN_FLIGHT};

/// The number of timestamp slots each frame sequence owns, so one frame records at most half as many counters.
pub(crate) const COUNTER_SLOTS_PER_FRAME: u32 = 1024;

/// The number of timestamp slots a backend's native pool holds for every frame sequence of a context.
pub(crate) const COUNTER_SLOT_COUNT: u32 = COUNTER_SLOTS_PER_FRAME * MAX_FRAMES_IN_FLIGHT as u32;

/// The `Span` struct records the slots one counter wrote in one frame sequence.
#[derive(Clone, Copy)]
struct Span {
	counter: CounterHandle,
	start: u32,
	end: Option<u32>,
}

/// The `SequenceSlots` struct hands out the timestamp slots of one frame sequence in recording order, so every
/// slot a frame wrote forms one contiguous range the backend can read back and reset at once.
#[derive(Default)]
struct SequenceSlots {
	spans: Vec<Span>,
	next_slot: u32,
}

impl SequenceSlots {
	fn allocate(&mut self) -> u32 {
		assert!(
			self.next_slot < COUNTER_SLOTS_PER_FRAME,
			"Counter slots are exhausted. The most likely cause is that one frame recorded more than {} counters.",
			COUNTER_SLOTS_PER_FRAME / 2,
		);
		let slot = self.next_slot;
		self.next_slot += 1;
		slot
	}
}

/// The `Counters` struct owns every counter of a context: the slots each one wrote in each frame sequence, and the
/// duration its most recently completed frame measured.
///
/// Slots are numbered across the whole native pool: sequence `s` owns `s * COUNTER_SLOTS_PER_FRAME` onward.
pub(crate) struct Counters {
	samples: Vec<Option<Duration>>,
	sequences: [SequenceSlots; MAX_FRAMES_IN_FLIGHT],
}

impl Counters {
	pub(crate) fn new() -> Self {
		Self {
			samples: Vec::new(),
			sequences: std::array::from_fn(|_| SequenceSlots::default()),
		}
	}

	/// Registers one counter and returns its handle.
	pub(crate) fn create(&mut self) -> CounterHandle {
		let handle = CounterHandle(self.samples.len() as u64);
		self.samples.push(None);
		handle
	}

	/// Returns the pool slot the start timestamp of `counter` goes into for the frame on `sequence_index`.
	pub(crate) fn start(&mut self, sequence_index: u8, counter: CounterHandle) -> u32 {
		self.validate(counter);
		let sequence = &mut self.sequences[sequence_index as usize];
		assert!(
			sequence.spans.iter().all(|span| span.counter != counter),
			"Counter {} started twice in one frame. The most likely cause is that start_counter was called again before end_counter.",
			counter.0,
		);
		let start = sequence.allocate();
		sequence.spans.push(Span {
			counter,
			start,
			end: None,
		});
		Self::pool_slot(sequence_index, start)
	}

	/// Returns the pool slot the end timestamp of `counter` goes into for the frame on `sequence_index`.
	pub(crate) fn end(&mut self, sequence_index: u8, counter: CounterHandle) -> u32 {
		self.validate(counter);
		let sequence = &mut self.sequences[sequence_index as usize];
		let index = sequence
			.spans
			.iter()
			.position(|span| span.counter == counter)
			.unwrap_or_else(|| {
				panic!(
					"Counter {} ended without a start. The most likely cause is that end_counter was called before start_counter in this frame.",
					counter.0,
				)
			});
		assert!(
			sequence.spans[index].end.is_none(),
			"Counter {} ended twice in one frame. The most likely cause is that end_counter was called again after the counter already ended.",
			counter.0,
		);
		let end = sequence.allocate();
		sequence.spans[index].end = Some(end);
		Self::pool_slot(sequence_index, end)
	}

	/// Returns the pool slots the frame on `sequence_index` has written so far.
	pub(crate) fn written_slots(&self, sequence_index: u8) -> std::ops::Range<u32> {
		let first = Self::pool_slot(sequence_index, 0);
		first..first + self.sequences[sequence_index as usize].next_slot
	}

	/// Publishes the durations the completed frame on `sequence_index` measured and frees its slots for the next frame.
	///
	/// `ticks` returns the timestamp written into a pool slot, or `None` when the GPU never wrote it, and `elapsed`
	/// converts a start and end timestamp into a duration. Every counter the frame did not finish measuring reads
	/// `None` afterwards, which is also what every counter reads once a frame recorded no counters at all.
	pub(crate) fn resolve(
		&mut self,
		sequence_index: u8,
		ticks: impl Fn(u32) -> Option<u64>,
		elapsed: impl Fn(u64, u64) -> Duration,
	) {
		self.samples.fill(None);
		let sequence = &mut self.sequences[sequence_index as usize];
		for span in sequence.spans.drain(..) {
			let Some(end) = span.end else {
				continue;
			};
			let start = ticks(Self::pool_slot(sequence_index, span.start));
			let end = ticks(Self::pool_slot(sequence_index, end));
			if let (Some(start), Some(end)) = (start, end) {
				self.samples[span.counter.0 as usize] = Some(elapsed(start, end));
			}
		}
		sequence.next_slot = 0;
	}

	/// Returns the duration `counter` measured in the most recently resolved frame.
	pub(crate) fn duration(&self, counter: CounterHandle) -> Option<Duration> {
		self.validate(counter);
		self.samples[counter.0 as usize]
	}

	fn validate(&self, counter: CounterHandle) {
		assert!(
			(counter.0 as usize) < self.samples.len(),
			"Invalid counter handle {}. The most likely cause is that the handle came from another context.",
			counter.0,
		);
	}

	fn pool_slot(sequence_index: u8, slot: u32) -> u32 {
		u32::from(sequence_index) * COUNTER_SLOTS_PER_FRAME + slot
	}
}

/// Returns the ticks between two timestamps of which only the low `valid_bits` are meaningful.
///
/// Vulkan reports how many bits a queue family writes; DX12 and Metal always write all 64. An end that reads
/// earlier than its start, which a GPU may report across engines, counts as no time at all instead of a wrap.
pub(crate) fn elapsed_ticks(start: u64, end: u64, valid_bits: u32) -> u64 {
	if valid_bits >= u64::BITS {
		end.saturating_sub(start)
	} else {
		end.wrapping_sub(start) & ((1u64 << valid_bits) - 1)
	}
}

/// Converts GPU ticks to a duration when the GPU clock runs at `frequency` ticks per second, as DX12 and Metal report.
pub(crate) fn duration_from_frequency(ticks: u64, frequency: u64) -> Duration {
	if frequency == 0 {
		return Duration::ZERO;
	}
	Duration::from_secs_f64(ticks as f64 / frequency as f64)
}

/// Converts GPU ticks to a duration when each tick lasts `period_nanoseconds`, as Vulkan reports.
pub(crate) fn duration_from_period(ticks: u64, period_nanoseconds: f32) -> Duration {
	Duration::from_secs_f64(ticks as f64 * f64::from(period_nanoseconds) * 1e-9)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn no_time(_: u64, _: u64) -> Duration {
		Duration::ZERO
	}

	fn tick_difference(start: u64, end: u64) -> Duration {
		Duration::from_nanos(end - start)
	}

	#[test]
	fn counters_read_nothing_until_a_frame_resolves_them() {
		let mut counters = Counters::new();
		let counter = counters.create();
		assert_eq!(counters.duration(counter), None);

		let start = counters.start(0, counter);
		let end = counters.end(0, counter);
		assert_eq!((start, end), (0, 1));
		assert_eq!(counters.written_slots(0), 0..2);
		assert_eq!(counters.duration(counter), None);

		counters.resolve(0, |slot| Some([100, 350][slot as usize]), tick_difference);
		assert_eq!(counters.duration(counter), Some(Duration::from_nanos(250)));
		assert_eq!(counters.written_slots(0), 0..0);
	}

	#[test]
	fn sequences_own_disjoint_slot_ranges() {
		let mut counters = Counters::new();
		let counter = counters.create();
		assert_eq!(counters.start(2, counter), 2 * COUNTER_SLOTS_PER_FRAME);
		assert_eq!(counters.end(2, counter), 2 * COUNTER_SLOTS_PER_FRAME + 1);
		assert_eq!(counters.start(0, counter), 0);
		assert_eq!(
			counters.written_slots(2),
			2 * COUNTER_SLOTS_PER_FRAME..2 * COUNTER_SLOTS_PER_FRAME + 2
		);
	}

	#[test]
	fn unfinished_and_unwritten_spans_resolve_to_nothing() {
		let mut counters = Counters::new();
		let finished = counters.create();
		let unfinished = counters.create();
		let unwritten = counters.create();
		counters.start(0, finished);
		counters.end(0, finished);
		counters.start(0, unfinished);
		counters.start(0, unwritten);
		counters.end(0, unwritten);

		counters.resolve(
			0,
			|slot| if slot < 3 { Some(u64::from(slot) * 10) } else { None },
			tick_difference,
		);
		assert_eq!(counters.duration(finished), Some(Duration::from_nanos(10)));
		assert_eq!(counters.duration(unfinished), None);
		assert_eq!(counters.duration(unwritten), None);
	}

	#[test]
	fn a_frame_without_counters_clears_earlier_samples() {
		let mut counters = Counters::new();
		let counter = counters.create();
		counters.start(1, counter);
		counters.end(1, counter);
		counters.resolve(1, |slot| Some(u64::from(slot)), tick_difference);
		assert!(counters.duration(counter).is_some());

		counters.resolve(0, |_| None, no_time);
		assert_eq!(counters.duration(counter), None);
	}

	#[test]
	#[should_panic(expected = "started twice in one frame")]
	fn starting_a_counter_twice_in_one_frame_fails() {
		let mut counters = Counters::new();
		let counter = counters.create();
		counters.start(0, counter);
		counters.start(0, counter);
	}

	#[test]
	#[should_panic(expected = "ended without a start")]
	fn ending_a_counter_that_never_started_fails() {
		let mut counters = Counters::new();
		let counter = counters.create();
		counters.end(0, counter);
	}

	#[test]
	#[should_panic(expected = "Invalid counter handle")]
	fn a_handle_from_another_context_is_rejected() {
		let counters = Counters::new();
		counters.duration(CounterHandle(0));
	}

	#[test]
	fn elapsed_ticks_respect_the_valid_bits() {
		assert_eq!(elapsed_ticks(10, 25, 64), 15);
		assert_eq!(elapsed_ticks(25, 10, 64), 0);
		assert_eq!(elapsed_ticks((1 << 36) - 5, 3, 36), 8);
	}

	#[test]
	fn ticks_convert_through_frequency_and_period() {
		assert_eq!(duration_from_frequency(1_500, 1_000_000), Duration::from_micros(1_500));
		assert_eq!(duration_from_frequency(7, 0), Duration::ZERO);
		assert_eq!(duration_from_period(2_000, 0.5), Duration::from_micros(1));
	}
}
