//! The schedule of a held key's repeats, for platforms that leave repeating to the application, such as Wayland,
//! where the compositor only sends the delay and rate. See [Key repeat](/docs/reference/input#key-repeat).

use std::time::Duration;

/// The `KeyRepeat` struct holds the delay and rate a platform's keyboard settings ask held keys to repeat at, so the
/// backend that makes the repeats can tell how many are due and when to wake for the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyRepeat {
	/// How long a key is held before it first repeats.
	pub(crate) delay: Duration,
	/// Repeats per second after the first; 0 turns repeating off.
	pub(crate) rate: u32,
}

impl KeyRepeat {
	/// Repeating off, until the platform says otherwise.
	pub(crate) const OFF: Self = Self {
		delay: Duration::ZERO,
		rate: 0,
	};

	/// How many repeats a key held for `held` has made: none before the delay, one at the delay, and one more every
	/// `1 / rate` seconds after it.
	pub(crate) fn due(self, held: Duration) -> u32 {
		if self.rate == 0 || held < self.delay {
			return 0;
		}
		let after = (held - self.delay).as_nanos();
		1 + (after * u128::from(self.rate) / 1_000_000_000) as u32
	}

	/// How long after the press the repeat that follows `made` repeats comes, or `None` while repeating is off.
	///
	/// It rounds up, so a wake at that time always finds the repeat [`Self::due`].
	pub(crate) fn next(self, made: u32) -> Option<Duration> {
		let rate = u64::from(self.rate);
		(rate != 0).then(|| self.delay + Duration::from_nanos((u64::from(made) * 1_000_000_000).div_ceil(rate)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const TYPICAL: KeyRepeat = KeyRepeat {
		delay: Duration::from_millis(500),
		rate: 25,
	};

	#[test]
	fn a_held_key_repeats_after_the_delay_at_the_rate() {
		assert_eq!(TYPICAL.due(Duration::from_millis(499)), 0, "nothing before the delay");
		assert_eq!(TYPICAL.due(Duration::from_millis(500)), 1, "the first at the delay");
		assert_eq!(TYPICAL.due(Duration::from_millis(539)), 1);
		assert_eq!(TYPICAL.due(Duration::from_millis(540)), 2, "one every 40 ms after it");
		assert_eq!(
			TYPICAL.due(Duration::from_millis(1500)),
			26,
			"a second of repeats at 25 per second"
		);
	}

	#[test]
	fn the_next_repeat_is_due_when_it_comes() {
		let rate = KeyRepeat {
			delay: Duration::from_millis(250),
			rate: 30,
		};
		for made in 0..100 {
			let at = rate.next(made).expect("repeating is on");
			assert_eq!(rate.due(at), made + 1, "repeat {made}");
		}
	}

	#[test]
	fn a_rate_of_zero_never_repeats() {
		assert_eq!(KeyRepeat::OFF.due(Duration::from_secs(10)), 0);
		assert_eq!(KeyRepeat::OFF.next(0), None);
	}
}
