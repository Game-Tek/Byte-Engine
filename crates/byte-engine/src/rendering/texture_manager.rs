//! This modules contains code for the `TextureManager` struct, which is responsible for loading and managing textures.

use std::{collections::hash_map::Entry, num::NonZeroU8, sync::Arc};

use resource_management::{resources::image::Image, Reference};
use utils::{hash::{HashMap, HashMapExt}, sync::{Rc, RwLock}, Extent};
use ghi::{graphics_hardware_interface::Device as _, Device};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SamplerState {
	filtering_mode: ghi::FilteringModes,
	reduction_mode: ghi::SamplingReductionModes,
	mip_map_mode: ghi::FilteringModes,
	addressing_mode: ghi::SamplerAddressingModes,
	anisotropy: Option<NonZeroU8>,
	min_lod: u8,
	max_lod: u8,
}

/// The `TextureManager` struct is responsible for loading and managing textures.
pub struct TextureManager {
	samplers: HashMap<SamplerState, ghi::SamplerHandle>,
    textures: HashMap<String, (ghi::ImageHandle, ghi::SamplerHandle)>,
}

impl TextureManager {
	pub fn new() -> Self {
		Self {
			textures: HashMap::with_capacity(1024),
			samplers: HashMap::with_capacity(126),
		}
	}

	pub fn load(&mut self, reference: &mut Reference<Image>, ghi: Arc<RwLock<ghi::Device>>) -> Option<(String, ghi::ImageHandle, ghi::SamplerHandle)> {
		if let Some(r) = self.textures.get(reference.id()) {
			return Some((reference.id().to_string(), r.0, r.1));
		}

		let texture = reference.resource();

		let format = match texture.format {
			resource_management::types::Formats::RG8 => ghi::Formats::RG8(ghi::Encodings::UnsignedNormalized),
			resource_management::types::Formats::RGB8 => ghi::Formats::RGB8(ghi::Encodings::UnsignedNormalized),
			resource_management::types::Formats::RGB16 => ghi::Formats::RGB16(ghi::Encodings::UnsignedNormalized),
			resource_management::types::Formats::RGBA8 => ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized),
			resource_management::types::Formats::RGBA16 => ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized),
			resource_management::types::Formats::BC5 => ghi::Formats::BC5,
			resource_management::types::Formats::BC7 => ghi::Formats::BC7,
		};

		let extent = Extent::from(texture.extent);

		let image;
		let target_buffer;

		{
			let mut ghi = ghi.write();
			image = ghi.create_image(Some(&reference.id()), extent, format, ghi::Uses::Image | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC, 1);
			target_buffer = ghi.get_texture_slice_mut(image);
		}

		let load_target = reference.load(target_buffer.into()).unwrap();

		// let image = if let Some(b) = resource.get_buffer() {
		// 	ghi.get_texture_slice_mut(new_texture).copy_from_slice(b);
		// 	new_texture
		// } else {
		// 	let new_texture = ghi.create_image(Some(&resource.id()), extent, ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
		// 	log::warn!("The image '{}' won't be available because the resource did not provide a buffer.", resource.id());
		// 	let slice = ghi.get_texture_slice_mut(new_texture);

		// 	// Generate checkboard pattern image
		// 	for y in 0..extent.height() {
		// 		for x in 0..extent.width() {
		// 			let color = if (x / 32 + y / 32) % 2 == 0 {
		// 				RGBA::white()
		// 			} else {
		// 				RGBA::black()
		// 			};

		// 			let index = ((y * extent.width() + x) * 4) as usize;
		// 			slice[index + 0] = (color.r * 255.0) as u8;
		// 			slice[index + 1] = (color.g * 255.0) as u8;
		// 			slice[index + 2] = (color.b * 255.0) as u8;
		// 			slice[index + 3] = (color.a * 255.0) as u8;
		// 		}
		// 	}

		// 	new_texture
		// };

		let sampler = {
			let mut ghi = ghi.write();
			self.create_sampler(&mut ghi)
		};

		let v = (image, sampler);

		self.textures.insert(reference.id().to_string(), v.clone());

		// self.pending_texture_loads.push(image);

		Some((reference.id().to_string(), v.0, v.1))
	}

	fn create_sampler(&mut self, ghi: &mut ghi::Device) -> ghi::SamplerHandle {
		let sampler_state = SamplerState {
			filtering_mode: ghi::FilteringModes::Linear,
			reduction_mode: ghi::SamplingReductionModes::WeightedAverage,
			mip_map_mode: ghi::FilteringModes::Linear,
			addressing_mode: ghi::SamplerAddressingModes::Repeat,
			anisotropy: None,
			min_lod: 0,
			max_lod: 0,
		};

		match self.samplers.entry(sampler_state) {
			Entry::Occupied(v) => v.get().clone(),
			Entry::Vacant(v) => {
				let sampler_handler = ghi.create_sampler(sampler_state.filtering_mode, sampler_state.reduction_mode, sampler_state.mip_map_mode, sampler_state.addressing_mode, sampler_state.anisotropy.map(|v| v.get() as f32), sampler_state.min_lod as f32, sampler_state.max_lod as f32);
				v.insert(sampler_handler);
				sampler_handler
			}
		}
	}
}
