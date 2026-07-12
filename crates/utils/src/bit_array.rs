#[derive(Debug, Clone, Copy)]
pub struct BitArray<const N: usize>
where
	[u8; N.div_ceil(8)]:,
{
	data: [u8; N.div_ceil(8)],
}

impl<const N: usize> Default for BitArray<N>
where
	[u8; N.div_ceil(8)]:,
{
	fn default() -> Self {
		Self::new()
	}
}

impl<const N: usize> BitArray<N>
where
	[u8; N.div_ceil(8)]:,
{
	pub fn new() -> Self {
		Self {
			data: [0; N.div_ceil(8)],
		}
	}

	pub fn set(&mut self, index: usize, value: bool) {
		assert!(
			index < N,
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

	pub fn get(&self, index: usize) -> bool {
		assert!(
			index < N,
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
		let mut bits = BitArray::<17>::new();

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
		let mut bits = BitArray::<10>::default();
		bits.set(9, true);

		assert!(bits.get(9));
		assert!(!bits.get(8));
	}

	#[test]
	#[should_panic(expected = "Bit index is out of bounds")]
	fn rejects_index_at_declared_length() {
		let _ = BitArray::<10>::new().get(10);
	}
}
