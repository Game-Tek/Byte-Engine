/// The `Stream` struct provides a borrowed view of one named range in a resource's binary data.
#[derive(Debug)]
pub struct Stream<'a> {
	/// The selected bytes from the resource data.
	buffer: &'a [u8],
	/// The subresource name, such as `Vertex` or `Index`.
	name: &'a str,
	/// The byte offset where this stream starts in the resource data.
	offset: usize,
	/// The maximum number of bytes to read, or the full buffer length when `None`.
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
/// The `StreamMut` struct provides a writable destination for one named resource-data range.
pub struct StreamMut<'a> {
	/// The buffer that receives the resource data.
	buffer: &'a mut [u8],
	/// The subresource name, such as `Vertex` or `Index`.
	name: &'a str,
	/// The byte offset where this stream starts in the resource data.
	offset: usize,
	/// The maximum number of bytes to read, or the full buffer length when `None`.
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

	/// Sets the maximum number of bytes to read into this stream.
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

#[cfg(test)]
mod tests {
	use super::{Stream, StreamMut};

	#[test]
	fn immutable_stream_preserves_name_range_and_buffer() {
		let bytes = [1u8, 2, 3, 4];
		let stream = Stream::new("vertices", &bytes, 12, Some(3));

		assert_eq!(stream.name(), "vertices");
		assert_eq!(stream.buffer(), &bytes);
		assert_eq!(stream.offset(), 12);
		assert_eq!(stream.size(), Some(3));
	}

	#[test]
	fn mutable_typed_stream_exposes_the_complete_object_representation() {
		let mut words = [0x1122u16, 0x3344u16];
		let expected = words;
		{
			let mut stream = StreamMut::new("indices", &mut words).with_size(3);

			assert_eq!(stream.name(), "indices");
			assert_eq!(stream.offset(), 0);
			assert_eq!(stream.size(), Some(3));
			assert_eq!(stream.buffer().len(), std::mem::size_of_val(&expected));
			stream.buffer_mut().fill(0);
		}
		assert_eq!(words, [0, 0]);
	}

	#[test]
	fn mutable_to_immutable_conversion_retains_metadata_and_storage() {
		let mut bytes = [1u8, 2, 3];
		let stream = Stream::from(StreamMut::new("payload", &mut bytes).with_size(2));

		assert_eq!(stream.name(), "payload");
		assert_eq!(stream.buffer(), &[1, 2, 3]);
		assert_eq!(stream.offset(), 0);
		assert_eq!(stream.size(), Some(2));
	}
}
