use std::num::NonZeroU32;

use utils::Extent;

use crate::{DeviceAccesses, Formats, PrivateHandle, PrivateHandles, UseCases, Uses};

pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
	pub(crate) use_case: UseCases,
	pub(crate) mip_levels: u32,
	pub(crate) array_layers: Option<NonZeroU32>,
}

impl<'a> Builder<'a> {
	/// Creates a new image builder with the given extent, format, and resource uses.
	/// The default name is None.
	/// The default extent is (0, 0, 0).
	/// The default device accesses are GPU read and write.
	/// The default use case is static.
	/// The default number of array layers is None.
	/// The default number of mip levels is 1.
	pub fn new(format: Formats, resource_uses: Uses) -> Self {
		Self {
			name: None,
			extent: Extent::cube(0, 0, 0),
			format,
			resource_uses,
			device_accesses: DeviceAccesses::DeviceOnly,
			use_case: UseCases::STATIC,
			mip_levels: 1,
			array_layers: None,
		}
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	pub fn extent(mut self, extent: Extent) -> Self {
		self.extent = extent;
		self
	}

	pub fn device_accesses(mut self, device_accesses: DeviceAccesses) -> Self {
		self.device_accesses = device_accesses;
		self
	}

	pub fn use_case(mut self, use_case: UseCases) -> Self {
		self.use_case = use_case;
		self
	}

	pub fn mip_levels(mut self, mip_levels: u32) -> Self {
		self.mip_levels = mip_levels;
		self
	}

	pub fn array_layers(mut self, array_layers: Option<NonZeroU32>) -> Self {
		self.array_layers = array_layers;
		self
	}

	pub fn get_name(&self) -> Option<&'a str> {
		self.name
	}

	pub fn get_format(&self) -> Formats {
		self.format
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ImageHandle(pub(crate) u64);

impl From<ImageHandle> for PrivateHandles {
	fn from(val: ImageHandle) -> Self {
		PrivateHandles::Image(val)
	}
}

impl PrivateHandle for ImageHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[cfg(test)]
mod tests {
	use std::num::NonZeroU32;

	use utils::Extent;

	use super::{Builder, ImageHandle};
	use crate::{DeviceAccesses, Formats, PrivateHandle, PrivateHandles, UseCases, Uses};

	#[test]
	fn builder_defaults_are_valid_for_a_single_static_image() {
		let builder = Builder::new(Formats::RGBA8UNORM, Uses::Image);
		assert_eq!(builder.get_name(), None);
		assert_eq!(builder.get_format(), Formats::RGBA8UNORM);
		assert_eq!(builder.extent, Extent::cube(0, 0, 0));
		assert_eq!(builder.resource_uses, Uses::Image);
		assert_eq!(builder.device_accesses, DeviceAccesses::DeviceOnly);
		assert_eq!(builder.use_case, UseCases::STATIC);
		assert_eq!(builder.mip_levels, 1);
		assert_eq!(builder.array_layers, None);
	}

	#[test]
	fn builder_preserves_all_explicit_image_constraints() {
		let builder = Builder::new(Formats::BC7, Uses::Image | Uses::TransferDestination)
			.name("albedo")
			.extent(Extent::rectangle(64, 32))
			.device_accesses(DeviceAccesses::HostToDevice)
			.use_case(UseCases::DYNAMIC)
			.mip_levels(7)
			.array_layers(NonZeroU32::new(6));

		assert_eq!(builder.get_name(), Some("albedo"));
		assert_eq!(builder.extent, Extent::rectangle(64, 32));
		assert_eq!(builder.resource_uses, Uses::Image | Uses::TransferDestination);
		assert_eq!(builder.device_accesses, DeviceAccesses::HostToDevice);
		assert_eq!(builder.use_case, UseCases::DYNAMIC);
		assert_eq!(builder.mip_levels, 7);
		assert_eq!(builder.array_layers, NonZeroU32::new(6));
	}

	#[test]
	fn private_image_handle_round_trips_index_and_variant() {
		let handle = ImageHandle::new(9);
		assert_eq!(handle.index(), 9);
		assert!(matches!(PrivateHandles::from(handle), PrivateHandles::Image(value) if value == handle));
	}
}
