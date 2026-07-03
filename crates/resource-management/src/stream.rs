/// Streams represent the data streams in a resource. They are used to load binary data into the resource.
/// The streams are used to load a select piece of a resource's binary data into memory.

#[derive(Debug)]
pub struct Stream<'a> {
	/// The slice of the buffer to load the resource binary data into.
	buffer: &'a [u8],
	/// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
	name: &'a str,
	/// Byte offset into the source resource data to start reading this stream from.
	offset: usize,
	/// Maximum bytes to read for this stream. Defaults to the full buffer length when `None`.
	size: Option<usize>,
}

impl<'a> Stream<'a> {
	pub fn new(name: &'a str, buffer: &'a [u8], offset: usize, size: Option<usize>) -> Self {
		Stream {
			buffer,
			name,
			offset,
			size,
		}
	}

	pub fn name(&'a self) -> &'a str {
		self.name
	}

	pub fn buffer(&'a self) -> &'a [u8] {
		self.buffer
	}

	pub fn offset(&self) -> usize {
		self.offset
	}

	pub fn size(&self) -> Option<usize> {
		self.size
	}
}

impl<'a> From<StreamMut<'a>> for Stream<'a> {
	fn from(value: StreamMut<'a>) -> Self {
		Stream::new(value.name, value.buffer, value.offset, value.size)
	}
}

#[derive(Debug)]
pub struct StreamMut<'a> {
	/// The slice of the buffer to load the resource binary data into.
	buffer: &'a mut [u8],
	/// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
	name: &'a str,
	/// Byte offset into the source resource data to start reading this stream from.
	offset: usize,
	/// Maximum bytes to read for this stream. Defaults to the buffer length when `None`.
	size: Option<usize>,
}

impl<'a> StreamMut<'a> {
	pub fn new<T: Copy>(name: &'a str, buffer: &'a mut [T]) -> Self {
		let buffer = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, std::mem::size_of_val(buffer)) };
		StreamMut {
			buffer,
			name,
			offset: 0,
			size: None,
		}
	}

	/// Sets the maximum bytes to read for this stream.
	pub fn with_size(self, size: usize) -> Self {
		StreamMut {
			size: Some(size),
			..self
		}
	}

	pub fn buffer(&self) -> &'_ [u8] {
		self.buffer
	}

	pub fn buffer_mut(&mut self) -> &'_ mut [u8] {
		self.buffer
	}

	pub fn name(&self) -> &'_ str {
		self.name
	}

	pub fn offset(&self) -> usize {
		self.offset
	}

	pub fn size(&self) -> Option<usize> {
		self.size
	}
}
