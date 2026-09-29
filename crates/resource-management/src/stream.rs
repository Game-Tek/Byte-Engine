/// The `Stream` struct provides a borrowed view of one named range in a resource's binary data.
#[derive(Debug)]
pub struct Stream<'a> {
	/// The selected bytes from the resource data.
	buffer: &'a [u8],
	/// The subresource name, such as `Vertex` or `Index`.
	name: &'a str,
}

impl<'a> Stream<'a> {
	pub fn new(name: &'a str, buffer: &'a [u8]) -> Self {
		Stream { buffer, name }
	}

	pub fn name(&'a self) -> &'a str {
		self.name
	}

	pub fn buffer(&'a self) -> &'a [u8] {
		self.buffer
	}
}

impl<'a> From<StreamMut<'a>> for Stream<'a> {
	fn from(value: StreamMut<'a>) -> Self {
		Stream::new(value.name, value.buffer)
	}
}

#[derive(Debug)]
/// The `StreamMut` struct provides a writable destination for one named resource-data range.
pub struct StreamMut<'a> {
	/// The buffer that receives the resource data.
	buffer: &'a mut [u8],
	/// The subresource name, such as `Vertex` or `Index`.
	name: &'a str,
}

impl<'a> StreamMut<'a> {
	/// Creates a byte stream over plain data with no padding or invalid byte representations.
	pub fn new<T: bytemuck::Pod>(name: &'a str, buffer: &'a mut [T]) -> Self {
		let buffer = bytemuck::cast_slice_mut(buffer);
		StreamMut { buffer, name }
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
}
