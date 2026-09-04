//! Play ordered image sequences at a fixed frame rate.
//!
//! Create a [`Flipbook`] from image identifiers or handles, then call
//! [`Flipbook::frame`] with elapsed playback time. The caller owns the clock,
//! so pausing, restarting, and choosing an idle image stay at the gameplay layer.

use crate::time::MediaTime;

/// The `Flipbook` struct provides allocation-free playback of a borrowed image sequence.
///
/// Images may be resource paths, loaded handles, or another renderer's image
/// identifiers. Load [`Self::images`] before playback, then use [`Self::frame`]
/// to choose the image to draw. Sampling loops and does not load resources.
///
/// ```
/// use byte_engine::{animation::flipbook::Flipbook, time::MediaTime};
///
/// let run = Flipbook::new(12, &["run0.png", "run1.png", "run2.png"]);
/// assert_eq!(*run.frame(MediaTime::from_frames(1, 12).unwrap()), "run1.png");
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flipbook<'a, Image> {
	images: &'a [Image],
	frames_per_second: u32,
}

impl<'a, Image> Flipbook<'a, Image> {
	/// Creates a looping sequence. Next, sample it with [`Self::frame`].
	///
	/// # Panics
	/// Panics if `images` is empty or `frames_per_second` is zero.
	pub const fn new(frames_per_second: u32, images: &'a [Image]) -> Self {
		assert!(
			!images.is_empty(),
			"Flipbook has no images. The most likely cause is an empty animation sequence."
		);
		assert!(
			frames_per_second > 0,
			"Flipbook FPS is zero. The most likely cause is an unset animation frame rate."
		);
		Self {
			images,
			frames_per_second,
		}
	}

	/// Returns the ordered images to load before playback.
	pub const fn images(&self) -> &'a [Image] {
		self.images
	}

	/// Returns the configured number of images displayed per second.
	pub const fn frames_per_second(&self) -> u32 {
		self.frames_per_second
	}

	/// Selects the looping frame at `elapsed`, wrapping negative time backward.
	pub fn frame_index(&self, elapsed: MediaTime) -> usize {
		// Widen before multiplying to preserve exact boundaries at every whole
		// FPS, including rates that do not divide the engine timebase.
		let frame = (i128::from(elapsed.as_ticks()) * i128::from(self.frames_per_second))
			.div_euclid(i128::from(MediaTime::TICKS_PER_SECOND));
		frame.rem_euclid(self.images.len() as i128) as usize
	}

	/// Returns the image at `elapsed`; skipped updates do not slow playback.
	pub fn frame(&self, elapsed: MediaTime) -> &'a Image {
		&self.images[self.frame_index(elapsed)]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn samples_boundaries_and_loops_in_sequence_order() {
		let clip = Flipbook::new(10, &["a", "b", "c"]);
		for (millis, image) in [(0, "a"), (99, "a"), (100, "b"), (299, "c"), (300, "a"), (850, "c"), (-1, "c")] {
			assert_eq!(*clip.frame(MediaTime::from_millis(millis)), image);
		}
	}

	#[test]
	fn supports_rates_that_do_not_divide_the_timebase() {
		let clip = Flipbook::new(13, &[0, 1, 2, 3]);
		let boundary = (MediaTime::TICKS_PER_SECOND + 12) / 13;
		assert_eq!(clip.frame_index(MediaTime::from_ticks(boundary - 1)), 0);
		assert_eq!(clip.frame_index(MediaTime::from_ticks(boundary)), 1);
		assert_eq!(clip.frame_index(MediaTime::from_seconds(1)), 1);
	}

	#[test]
	fn a_single_image_is_stable_across_the_timeline() {
		let clip = Flipbook::new(u32::MAX, &["idle"]);
		for time in [MediaTime::MIN, MediaTime::ZERO, MediaTime::MAX] {
			assert_eq!(*clip.frame(time), "idle");
		}
	}

	#[test]
	#[should_panic(expected = "Flipbook has no images")]
	fn rejects_empty_sequences() {
		Flipbook::<&str>::new(12, &[]);
	}

	#[test]
	#[should_panic(expected = "Flipbook FPS is zero")]
	fn rejects_zero_fps() {
		Flipbook::new(0, &["idle"]);
	}
}
