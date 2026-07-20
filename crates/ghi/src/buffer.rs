use crate::{graphics_hardware_interface, DeviceAccesses, PrivateHandle, PrivateHandles, Uses};

/// The `Builder` struct configures buffer creation parameters that can be shared across static and dynamic buffer constructors.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
}

impl<'a> Builder<'a> {
	/// Creates a buffer builder with GPU read and write access.
	///
	/// The default name is `None`.
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

#[cfg(test)]
mod tests {
	use super::{BufferHandle, Builder};
	use crate::{DeviceAccesses, PrivateHandle, PrivateHandles, Uses};

	#[test]
	fn builder_defaults_to_device_only_and_preserves_requested_uses() {
		let builder = Builder::new(Uses::Vertex | Uses::TransferDestination);
		assert_eq!(builder.name, None);
		assert_eq!(builder.resource_uses, Uses::Vertex | Uses::TransferDestination);
		assert_eq!(builder.device_accesses, DeviceAccesses::DeviceOnly);
	}

	#[test]
	fn builder_overrides_are_independent() {
		let builder = Builder::new(Uses::Uniform)
			.name("camera")
			.device_accesses(DeviceAccesses::HostToDevice);
		assert_eq!(builder.name, Some("camera"));
		assert_eq!(builder.resource_uses, Uses::Uniform);
		assert_eq!(builder.device_accesses, DeviceAccesses::HostToDevice);
	}

	#[test]
	fn private_buffer_handle_round_trips_index_and_variant() {
		let handle = BufferHandle::new(17);
		assert_eq!(handle.index(), 17);
		assert!(matches!(PrivateHandles::from(handle), PrivateHandles::Buffer(value) if value == handle));
	}
}
