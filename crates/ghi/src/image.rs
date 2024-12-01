use utils::Extent;

use crate::{DeviceAccesses, Formats, UseCases, Uses};

pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
	pub(crate) use_case: UseCases,
	pub(crate) mip_levels: u32,
	pub(crate) array_layers: u32,
}

impl<'a> Builder<'a> {
	/// Creates a new image builder with the given extent, format, and resource uses.
	/// The default device accesses are GPU read and write.
	/// The default use case is static.
	/// The default number of array layers is 1.
	/// The default number of mip levels is 1.
	pub fn new(extent: Extent, format: Formats, resource_uses: Uses) -> Self {
		Self {
			name: None,
			extent,
			format,
			resource_uses,
			device_accesses: DeviceAccesses::GpuRead | DeviceAccesses::GpuWrite,
			use_case: UseCases::STATIC,
			mip_levels: 1,
			array_layers: 1,
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

	pub fn use_case(mut self, use_case: UseCases) -> Self {
		self.use_case = use_case;
		self
	}

	pub fn mip_levels(mut self, mip_levels: u32) -> Self {
		self.mip_levels = mip_levels;
		self
	}

	pub fn array_layers(mut self, array_layers: u32) -> Self {
		self.array_layers = array_layers;
		self
	}
}