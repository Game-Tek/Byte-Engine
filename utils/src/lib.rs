pub type BoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;
pub type SendSyncBoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + Sync + 'a>>;
pub type SendBoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + Sync + 'a>>;

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

#[derive(Debug, Clone, Copy, PartialEq,)]
pub struct Extent {
	width: u32,
	height: u32,
	depth: u32,
}

impl Extent {
	pub fn new(width: u32, height: u32, depth: u32) -> Self {
		Self {
			width,
			height,
			depth,
		}
	}

	pub fn line(width: u32) -> Self {
		Self {
			width,
			height: 1,
			depth: 1,
		}
	}

	pub fn square(size: u32) -> Self {
		Self {
			width: size,
			height: size,
			depth: 1,
		}
	}

	pub fn rectangle(width: u32, height: u32) -> Self {
		Self {
			width,
			height,
			depth: 1,
		}
	}

	pub fn cube(width: u32, height: u32, depth: u32) -> Self {
		Self {
			width,
			height,
			depth,
		}
	}

	pub fn as_tuple(&self) -> (u32, u32, u32) {
		(self.width, self.height, self.depth)
	}

	pub fn as_array(&self) -> [u32; 3] {
		[self.width, self.height, self.depth]
	}

	#[inline]
	pub fn width(&self) -> u32 { self.width }
	#[inline]
	pub fn height(&self) -> u32 { self.height }
	#[inline]
	pub fn depth(&self) -> u32 { self.depth }
}

impl From<[u32; 3]> for Extent {
	fn from(array: [u32; 3]) -> Self {
		Self {
			width: array[0],
			height: array[1],
			depth: array[2],
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RGBA {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

impl RGBA {
	pub fn black() -> Self { Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0, } }	
	pub fn white() -> Self { Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0, } }
}

pub fn insert_return_length<T>(collection: &mut Vec<T>, value: T) -> usize {
	let length = collection.len();
	collection.push(value);
	length
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