//! The `debug` module provides placeholder GHI handles for tests that check handle bookkeeping without a GPU.

use crate::{BaseBufferHandle, BaseImageHandle, DynamicImageHandle, ImageGroupHandle, SamplerHandle, image, sampler};

/// The `Device` struct hands out placeholder handles, so unit tests can fill resource tables without a GPU backend.
///
/// Every handle names nothing. Pass them only to code that stores, compares, or validates handles.
#[derive(Default)]
pub struct Device {}

impl Device {
	pub fn new() -> Self {
		Self {}
	}

	pub fn build_dynamic_image(&mut self, _builder: image::Builder) -> DynamicImageHandle {
		DynamicImageHandle(BaseImageHandle(0))
	}

	pub fn create_image_group(&mut self, _name: Option<&str>) -> ImageGroupHandle {
		ImageGroupHandle(0)
	}

	pub fn build_sampler(&mut self, _builder: sampler::Builder) -> SamplerHandle {
		SamplerHandle(0)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> BaseBufferHandle {
		BaseBufferHandle(0)
	}
}
