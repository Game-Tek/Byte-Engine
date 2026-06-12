use crate::{graphics_hardware_interface, DeviceAccesses, PrivateHandle, PrivateHandles, Uses};

/// The `Builder` struct configures buffer creation parameters that can be shared across static and dynamic buffer constructors.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
}

impl<'a> Builder<'a> {
	/// Creates a new buffer builder with the given resource uses.
	/// The default name is None.
	/// The default device accesses are GPU read and write.
	pub fn new(resource_uses: Uses) -> Self {
		Self {
			name: None,
			resource_uses,
			device_accesses: DeviceAccesses::DeviceOnly,
		}
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	pub fn device_accesses(mut self, device_accesses: DeviceAccesses) -> Self {
		self.device_accesses = device_accesses;
		self
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BufferHandle(pub(crate) u64);

impl From<BufferHandle> for graphics_hardware_interface::Handles {
	fn from(val: BufferHandle) -> Self {
		graphics_hardware_interface::Handles::Buffer(graphics_hardware_interface::BaseBufferHandle(val.0))
	}
}

impl From<BufferHandle> for PrivateHandles {
	fn from(val: BufferHandle) -> Self {
		PrivateHandles::Buffer(val)
	}
}

impl PrivateHandle for BufferHandle {
	fn new(i: u64) -> Self {
		BufferHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}
