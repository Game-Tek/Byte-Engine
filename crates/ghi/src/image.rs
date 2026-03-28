use std::num::NonZeroU32;

use utils::Extent;

use crate::{DeviceAccesses, Formats, HandleLike, Image, Next, PrivateHandles, UseCases, Uses};

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

impl Into<PrivateHandles> for ImageHandle {
	fn into(self) -> PrivateHandles {
		PrivateHandles::Image(self)
	}
}

impl HandleLike for ImageHandle {
	type Item = Image;

	fn build(value: u64) -> Self {
		ImageHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Image {
		&collection[self.0 as usize]
	}
}

impl Next for Image {
	type Handle = ImageHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}
