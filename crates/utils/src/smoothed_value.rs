//! Gradually moves a value toward successive targets.
//!
//! Create a [`SmoothedValue`] with [`SmoothedValue::new`], then call
//! [`SmoothedValue::update`] whenever a new target is available.

/// The `SmoothedValue` struct retains interpolation state between updates so callers can smooth changing values.
/// Create a [`SmoothedValue`] with [`SmoothedValue::new`], then call
/// [`SmoothedValue::update`] whenever a new target is available.
pub struct SmoothedValue<T> {
	current: T,
	target: T,
}

impl<T: Copy + std::ops::AddAssign<O> + std::ops::Sub<T, Output = O>, O: std::ops::Mul<f32, Output = O>> SmoothedValue<T> {
	/// Creates a smoothed value whose current value and target are `initial`.
	pub fn new(initial: T) -> Self {
		Self {
			current: initial,
			target: initial,
		}
	}

	/// Moves the current value by `factor` of the distance to `value` and returns the result.
	///
	/// A factor of `0.0` preserves the current value, while `1.0` reaches the target.
	/// Values outside that range extrapolate rather than being clamped.
	pub fn update(&mut self, value: T, factor: f32) -> T {
		self.target = value;
		self.current += (self.target - self.current) * factor;
		self.current
	}
}

#[cfg(test)]
mod tests {
	use super::SmoothedValue;

	#[test]
	fn updates_preserve_state_and_honor_interpolation_factors() {
		let mut value = SmoothedValue::new(0.0f32);

		assert_eq!(value.update(8.0, 0.25), 2.0);
		assert_eq!(value.update(10.0, 0.5), 6.0);
		assert_eq!(value.update(12.0, 0.0), 6.0);
		assert_eq!(value.update(12.0, 1.0), 12.0);
		assert_eq!(value.update(16.0, 1.5), 18.0);
	}
}
