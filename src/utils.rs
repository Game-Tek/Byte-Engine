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