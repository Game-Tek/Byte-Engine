/// Streams represent the data streams in a resource. They are used to load binary data into the resource.
/// The streams are used to load a select piece of a resource's binary data into memory.

#[derive(Debug)]
pub struct Stream<'a> {
    /// The slice of the buffer to load the resource binary data into.
    buffer: &'a [u8],
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
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
pub struct StreamMut<'a> {
    /// The slice of the buffer to load the resource binary data into.
    buffer: &'a mut [u8],
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
    name: &'a str,
}

impl<'a> StreamMut<'a> {
    pub fn new<T: Copy>(name: &'a str, buffer: &'a mut [T]) -> Self {
		let buffer = unsafe {
			std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, std::mem::size_of::<T>() * buffer.len())
		};
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