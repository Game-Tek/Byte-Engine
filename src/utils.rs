pub type BoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

pub struct BufferAllocator<'a> {
	buffer: &'a mut [u8],
	offset: usize,
}

impl<'a> BufferAllocator<'a> {
	pub fn new(buffer: &'a mut [u8]) -> Self {
		Self {
			buffer,
			offset: 0,
		}
	}

	pub fn take(&mut self, size: usize) -> &'a mut [u8] {
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		unsafe { std::mem::transmute(buffer) }
	}
}

pub fn partition<T>(slice: &[T], key_fn: impl Fn(&T) -> usize) -> Vec<(usize, &[T])> {
	let mut partitions = Vec::new();
	let mut slice_start = 0;

	for i in 1..slice.len() {
		if key_fn(&slice[i - 1]) + 1usize != key_fn(&slice[i]) {
			partitions.push((key_fn(&slice[slice_start]), &slice[slice_start..i]));
			slice_start = i;
		}
	}

	if !slice.is_empty() {
		partitions.push((key_fn(&slice[slice_start]), &slice[slice_start..]));
	}

	partitions
}

#[cfg(test)]
mod tests {
	#[test]
	fn test_partition() {
		let input = [];
		let expected: Vec<(usize, &[usize])> = vec![];
		assert_eq!(super::partition(&input,|x| *x,), expected);

		let input = [0];
		let expected: Vec<(usize, &[usize])> = vec![(0, &[0])];
		assert_eq!(super::partition(&input,|x| *x,), expected);

		let input = [0, 1];
		let expected: Vec<(usize, &[usize])> = vec![(0, &[0, 1])];
		assert_eq!(super::partition(&input,|x| *x,), expected);

		let input = [0, 2];
		let expected: Vec<(usize, &[usize])> = vec![(0, &[0]), (2, &[2])];
		assert_eq!(super::partition(&input,|x| *x,), expected);

		let input = [1, 2, 3, 5, 6, 7, 9, 10, 11];
		let expected: Vec<(usize, &[usize])> = vec![(1, &[1, 2, 3]), (5, &[5, 6, 7]), (9, &[9, 10, 11])];
		assert_eq!(super::partition(&input,|x| *x,), expected);
	}
}