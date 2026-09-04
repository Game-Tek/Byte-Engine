/// The `BitArray` struct provides allocation-free packed storage for a fixed number of bits.
///
/// Set `BYTE_COUNT` to `BIT_COUNT.div_ceil(8)`, then use [`Self::set`] and [`Self::get`]
/// to update and read individual bits.
#[derive(Debug, Clone, Copy)]
pub struct BitArray<const BIT_COUNT: usize, const BYTE_COUNT: usize> {
	data: [u8; BYTE_COUNT],
}

impl<const BIT_COUNT: usize, const BYTE_COUNT: usize> Default for BitArray<BIT_COUNT, BYTE_COUNT> {
	fn default() -> Self {
		Self::new()
	}
}

impl<const BIT_COUNT: usize, const BYTE_COUNT: usize> BitArray<BIT_COUNT, BYTE_COUNT> {
	/// Creates an empty bit array with enough packed storage for all `BIT_COUNT` bits.
	///
	/// Call [`Self::set`] to update a bit.
	pub fn new() -> Self {
		assert!(
			BYTE_COUNT == BIT_COUNT.div_ceil(8),
			"Bit-array byte count is invalid. The most likely cause is a byte count other than BIT_COUNT.div_ceil(8)."
		);
		Self { data: [0; BYTE_COUNT] }
	}

	/// Updates the bit at `index`.
	pub fn set(&mut self, index: usize, value: bool) {
		assert!(
			index < BIT_COUNT,
			"Bit index is out of bounds. The most likely cause is an index greater than or equal to the bit-array length."
		);
		let byte_index = index / 8;
		let bit_index = index % 8;
		let mask = 1 << bit_index;
		if value {
			self.data[byte_index] |= mask;
		} else {
			self.data[byte_index] &= !mask;
		}
	}

	/// Returns the bit at `index`.
	pub fn get(&self, index: usize) -> bool {
		assert!(
			index < BIT_COUNT,
			"Bit index is out of bounds. The most likely cause is an index greater than or equal to the bit-array length."
		);
		let byte_index = index / 8;
		let bit_index = index % 8;
		let mask = 1 << bit_index;
		(self.data[byte_index] & mask) != 0
	}
}

#[cfg(test)]
mod tests {
	use super::BitArray;

	#[test]
	fn bits_are_independent_across_byte_boundaries() {
		let mut bits = BitArray::<17, 3>::new();

		for index in 0..17 {
			assert!(!bits.get(index));
		}

		for index in [0, 7, 8, 15, 16] {
			bits.set(index, true);
		}

		for index in 0..17 {
			assert_eq!(bits.get(index), matches!(index, 0 | 7 | 8 | 15 | 16));
		}

		bits.set(8, false);

		assert!(!bits.get(8));
		assert!(bits.get(7));
		assert!(bits.get(15));
	}

	#[test]
	fn non_byte_aligned_lengths_store_the_last_declared_bit() {
		let mut bits = BitArray::<10, 2>::default();
		bits.set(9, true);

		assert!(bits.get(9));
		assert!(!bits.get(8));
		assert_eq!(size_of_val(&bits), 2);
	}

	#[test]
	#[should_panic(expected = "Bit-array byte count is invalid")]
	fn rejects_incorrect_byte_count() {
		let _ = BitArray::<10, 1>::new();
	}

	#[test]
	#[should_panic(expected = "Bit index is out of bounds")]
	fn rejects_index_at_declared_length() {
		let _ = BitArray::<10, 2>::new().get(10);
	}
}
