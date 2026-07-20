//! Fixed-rate time used by engine simulation and media timelines.

use std::{
	ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign},
	time::Duration,
};

/// The number of engine ticks in one second.
///
/// This rate exactly represents the engine's conventional frame rates and
/// 44.1, 48, 96, and 192 kHz audio sample boundaries.
pub const TICKS_PER_SECOND: i64 = 28_224_000;

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// The `MediaTime` struct provides a shared, allocation-free time unit for
/// simulation, animation, and media synchronization.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct MediaTime(i64);

impl MediaTime {
	pub const ZERO: Self = Self(0);
	pub const MIN: Self = Self(i64::MIN);
	pub const MAX: Self = Self(i64::MAX);
	pub const TICKS_PER_SECOND: i64 = TICKS_PER_SECOND;

	/// Creates a time value from raw engine ticks.
	pub const fn from_ticks(ticks: i64) -> Self {
		Self(ticks)
	}

	/// Returns the raw engine tick count.
	pub const fn as_ticks(self) -> i64 {
		self.0
	}

	/// Creates a time value from a whole number of seconds.
	pub fn from_seconds(seconds: i64) -> Self {
		Self(
			seconds
				.checked_mul(TICKS_PER_SECOND)
				.expect("Media time seconds overflowed. The most likely cause is an invalid or excessively large duration."),
		)
	}

	/// Creates a time value from fractional seconds, rounded to the nearest tick.
	pub fn from_seconds_f64(seconds: f64) -> Self {
		assert!(
			seconds.is_finite(),
			"Media time seconds must be finite. The most likely cause is a NaN or infinite simulation duration."
		);

		let ticks = seconds * TICKS_PER_SECOND as f64;
		assert!(
			ticks >= -((1_u64 << 63) as f64) && ticks < (1_u64 << 63) as f64,
			"Media time seconds overflowed. The most likely cause is an invalid or excessively large duration."
		);
		Self(ticks.round() as i64)
	}

	/// Creates a time value from fractional seconds, rounded to the nearest tick.
	pub fn from_seconds_f32(seconds: f32) -> Self {
		Self::from_seconds_f64(f64::from(seconds))
	}

	/// Creates a time value from milliseconds.
	pub fn from_millis(milliseconds: i64) -> Self {
		Self::from_ratio(milliseconds, 1_000)
			.expect("Media time milliseconds overflowed. The most likely cause is an invalid or excessively large duration.")
	}

	/// Creates a time value from microseconds, rounded to the nearest tick.
	pub fn from_micros(microseconds: i64) -> Self {
		Self::from_ratio_rounded(microseconds, 1_000_000)
			.expect("Media time microseconds overflowed. The most likely cause is an invalid or excessively large duration.")
	}

	/// Creates the duration occupied by an exact number of frames.
	///
	/// Returns `None` when the frame rate does not divide the engine timebase or
	/// when the resulting tick count would overflow.
	pub fn from_frames(frame_count: i64, frames_per_second: u32) -> Option<Self> {
		Self::from_rate_units(frame_count, frames_per_second)
	}

	/// Creates the duration occupied by an exact number of audio samples.
	///
	/// Returns `None` when the sample rate does not divide the engine timebase or
	/// when the resulting tick count would overflow.
	pub fn from_samples(sample_count: i64, samples_per_second: u32) -> Option<Self> {
		Self::from_rate_units(sample_count, samples_per_second)
	}

	/// Converts a standard duration into engine time, rounded to the nearest tick.
	pub fn from_std(duration: Duration) -> Self {
		let whole_ticks = i128::from(duration.as_secs()) * i128::from(TICKS_PER_SECOND);
		let fractional_ticks =
			(i128::from(duration.subsec_nanos()) * i128::from(TICKS_PER_SECOND) + NANOS_PER_SECOND / 2) / NANOS_PER_SECOND;
		let ticks = whole_ticks + fractional_ticks;

		Self(i64::try_from(ticks).expect(
			"Standard duration does not fit in media time. The most likely cause is a duration longer than the engine timeline range.",
		))
	}

	/// Converts a non-negative engine time into a standard duration.
	pub fn to_std(self) -> Duration {
		assert!(
			self.0 >= 0,
			"Negative media time cannot become a standard duration. The most likely cause is converting a signed timeline offset at an OS time boundary."
		);

		// Use one integer conversion so rounding cannot carry nanoseconds without
		// also carrying the corresponding whole second.
		let total_nanos =
			(i128::from(self.0) * NANOS_PER_SECOND + i128::from(TICKS_PER_SECOND) / 2) / i128::from(TICKS_PER_SECOND);
		let seconds = total_nanos / NANOS_PER_SECOND;
		let nanoseconds = total_nanos % NANOS_PER_SECOND;

		Duration::new(seconds as u64, nanoseconds as u32)
	}

	/// Returns this time value as fractional seconds.
	pub fn as_seconds_f64(self) -> f64 {
		self.0 as f64 / TICKS_PER_SECOND as f64
	}

	/// Returns this time value as fractional seconds.
	pub fn as_seconds_f32(self) -> f32 {
		self.0 as f32 / TICKS_PER_SECOND as f32
	}

	/// Returns the sum, or `None` when it falls outside the timeline range.
	pub const fn checked_add(self, rhs: Self) -> Option<Self> {
		match self.0.checked_add(rhs.0) {
			Some(ticks) => Some(Self(ticks)),
			None => None,
		}
	}

	/// Returns the difference, or `None` when it falls outside the timeline range.
	pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
		match self.0.checked_sub(rhs.0) {
			Some(ticks) => Some(Self(ticks)),
			None => None,
		}
	}

	/// Returns the difference clamped to the timeline range.
	pub const fn saturating_sub(self, rhs: Self) -> Self {
		Self(self.0.saturating_sub(rhs.0))
	}

	/// Creates a time value from units whose rate exactly divides the timebase.
	fn from_rate_units(unit_count: i64, units_per_second: u32) -> Option<Self> {
		if units_per_second == 0 || TICKS_PER_SECOND % i64::from(units_per_second) != 0 {
			return None;
		}

		unit_count
			.checked_mul(TICKS_PER_SECOND / i64::from(units_per_second))
			.map(Self)
	}

	/// Converts an integer ratio when it maps to an exact tick count.
	fn from_ratio(value: i64, denominator: i64) -> Option<Self> {
		let scaled = i128::from(value) * i128::from(TICKS_PER_SECOND);
		if scaled % i128::from(denominator) != 0 {
			return None;
		}
		i64::try_from(scaled / i128::from(denominator)).ok().map(Self)
	}

	/// Converts an integer ratio to the nearest tick without floating-point loss.
	fn from_ratio_rounded(value: i64, denominator: i64) -> Option<Self> {
		let scaled = i128::from(value) * i128::from(TICKS_PER_SECOND);
		let half = i128::from(denominator) / 2;
		let rounded = if scaled >= 0 {
			(scaled + half) / i128::from(denominator)
		} else {
			(scaled - half) / i128::from(denominator)
		};
		i64::try_from(rounded).ok().map(Self)
	}
}

impl std::fmt::Debug for MediaTime {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("MediaTime")
			.field("ticks", &self.0)
			.field("seconds", &self.as_seconds_f64())
			.finish()
	}
}

impl From<Duration> for MediaTime {
	fn from(duration: Duration) -> Self {
		Self::from_std(duration)
	}
}

impl Add for MediaTime {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		self.checked_add(rhs).expect(
			"Media time addition overflowed. The most likely cause is accumulating an invalid or excessively large duration.",
		)
	}
}

impl AddAssign for MediaTime {
	fn add_assign(&mut self, rhs: Self) {
		*self = *self + rhs;
	}
}

impl Sub for MediaTime {
	type Output = Self;

	fn sub(self, rhs: Self) -> Self::Output {
		self.checked_sub(rhs).expect(
			"Media time subtraction overflowed. The most likely cause is subtracting an invalid or excessively large timeline offset.",
		)
	}
}

impl SubAssign for MediaTime {
	fn sub_assign(&mut self, rhs: Self) {
		*self = *self - rhs;
	}
}

impl Mul<i64> for MediaTime {
	type Output = Self;

	fn mul(self, rhs: i64) -> Self::Output {
		Self(self.0.checked_mul(rhs).expect(
			"Media time multiplication overflowed. The most likely cause is an invalid or excessively large timeline scale.",
		))
	}
}

impl Div<i64> for MediaTime {
	type Output = Self;

	fn div(self, rhs: i64) -> Self::Output {
		assert!(
			rhs != 0,
			"Media time division by zero is invalid. The most likely cause is averaging an empty timeline."
		);
		Self(self.0 / rhs)
	}
}

impl Neg for MediaTime {
	type Output = Self;

	fn neg(self) -> Self::Output {
		Self(
			self.0
				.checked_neg()
				.expect("Media time negation overflowed. The most likely cause is negating the minimum timeline value."),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{MediaTime, TICKS_PER_SECOND};

	#[test]
	fn media_timebase_exactly_represents_supported_frame_rates() {
		let rates = [
			1, 2, 4, 8, 12, 16, 24, 30, 48, 50, 60, 90, 96, 120, 140, 144, 200, 240, 360, 480,
		];

		for rate in rates {
			let frame = MediaTime::from_frames(1, rate).unwrap();
			assert_eq!(frame.as_ticks() * i64::from(rate), TICKS_PER_SECOND);
		}
	}

	#[test]
	fn media_timebase_rejects_rates_that_require_rational_timing() {
		for rate in [122, 165, 244, 365] {
			assert_eq!(MediaTime::from_frames(1, rate), None);
		}
	}

	#[test]
	fn media_timebase_exactly_represents_supported_audio_rates() {
		for rate in [44_100, 48_000, 96_000, 192_000] {
			let sample = MediaTime::from_samples(1, rate).unwrap();
			assert_eq!(sample.as_ticks() * i64::from(rate), TICKS_PER_SECOND);
		}
	}

	#[test]
	fn standard_duration_conversion_rounds_without_float_loss() {
		let standard = std::time::Duration::new(12, 345_678_901);
		let media = MediaTime::from_std(standard);
		let round_trip = media.to_std();
		let error = round_trip.abs_diff(standard);

		assert!(error <= std::time::Duration::from_nanos(18));
	}

	#[test]
	fn arithmetic_preserves_signed_timeline_offsets() {
		let frame = MediaTime::from_frames(1, 24).unwrap();
		let offset = frame * 3 - MediaTime::from_seconds(1);

		assert_eq!(offset.as_ticks(), -24_696_000);
		assert_eq!((offset + MediaTime::from_seconds(1)).as_ticks(), frame.as_ticks() * 3);
	}
}
