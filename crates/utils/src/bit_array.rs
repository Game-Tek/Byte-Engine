#[derive(Clone, Copy)]
pub struct BitArray<const N: usize> where [u8; N / 8]: {
	data: [u8; N / 8],
}

impl<const N: usize> BitArray<N> where [u8; N / 8]: {
	pub fn new() -> Self {
		Self {
			data: [0; N / 8],
		}
	}

	pub fn set(&mut self, index: usize, value: bool) {
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
		let byte_index = index / 8;
		let bit_index = index % 8;
		let mask = 1 << bit_index;
		(self.data[byte_index] & mask) != 0
	}
}
