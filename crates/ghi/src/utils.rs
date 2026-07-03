pub struct StableVec<T: Default, const N: usize> {
	data: [T; N],
	pos: usize,
}

impl<T: Default + Copy, const N: usize> StableVec<T, N> {
	pub fn new() -> Self {
		StableVec {
			data: [T::default(); N],
			pos: 0,
		}
	}

	pub fn append<const M: usize>(&mut self, array: [T; M]) -> &'static [T] {
		assert!(self.pos + M <= N, "StableVec is full");
		let start = self.pos;
		let end = start + M;
		self.data[start..end].copy_from_slice(&array);
		self.pos += M;
		unsafe { std::mem::transmute(&self.data[start..end]) } // SAFETY: this is not correct
	}
}
