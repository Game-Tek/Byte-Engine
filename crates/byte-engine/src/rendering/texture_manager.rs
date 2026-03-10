//! This modules contains code for the `TextureManager` struct, which is responsible for loading and managing textures.

use std::{collections::hash_map::Entry, num::NonZeroU8, sync::Arc};

use ghi::{
	device::{Device as _, DeviceCreate as _},
	Frame as _,
};
use resource_management::{resources::image::Image, Reference};
use utils::{
	hash::{HashMap, HashMapExt},
	sync::{Rc, RwLock},
	Extent,
};

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

	pub fn load(
		&mut self,
		reference: &mut Reference<Image>,
		device: &mut ghi::implementation::Frame,
	) -> Option<(String, ghi::ImageHandle, ghi::SamplerHandle)> {
		if let Some(r) = self.textures.get(reference.id()) {
			return Some((reference.id().to_string(), r.0, r.1));
		}

		let texture = reference.resource();

		let format = match texture.format {
			resource_management::types::Formats::RG8 => ghi::Formats::RG8UNORM,
			resource_management::types::Formats::RGB8 => ghi::Formats::RGB8UNORM,
			resource_management::types::Formats::RGB16 => ghi::Formats::RGB16UNORM,
			resource_management::types::Formats::RGBA8 => ghi::Formats::RGBA8UNORM,
			resource_management::types::Formats::RGBA16 => ghi::Formats::RGBA16UNORM,
			resource_management::types::Formats::BC5 => ghi::Formats::BC5,
			resource_management::types::Formats::BC7 => ghi::Formats::BC7,
		};

		let extent = Extent::from(texture.extent);

		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.use_case(ghi::UseCases::STATIC),
		);
		let target_buffer = device.get_texture_slice_mut(image);

		device.sync_texture(image);

		let load_target = reference.load(target_buffer.into()).unwrap();

		// let image = if let Some(b) = resource.get_buffer() {
		// 	ghi.get_texture_slice_mut(new_texture).copy_from_slice(b);
		// 	new_texture
		// } else {
		// 	let new_texture = ghi.build_image(ghi::image::Builder::new(ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image | ghi::Uses::TransferDestination).name(&resource.id()).extent(extent).device_accesses(ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead).use_case(ghi::UseCases::STATIC));
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

		let sampler = self.build_sampler(device);

		let v = (image, sampler);

		self.textures.insert(reference.id().to_string(), v.clone());

		// self.pending_texture_loads.push(image);

		Some((reference.id().to_string(), v.0, v.1))
	}

	fn build_sampler(&mut self, device: &mut ghi::implementation::Frame) -> ghi::SamplerHandle {
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
				let mut sampler_builder = ghi::sampler::Builder::new()
					.filtering_mode(sampler_state.filtering_mode)
					.reduction_mode(sampler_state.reduction_mode)
					.mip_map_mode(sampler_state.mip_map_mode)
					.addressing_mode(sampler_state.addressing_mode)
					.min_lod(sampler_state.min_lod as f32)
					.max_lod(sampler_state.max_lod as f32);

				if let Some(anisotropy) = sampler_state.anisotropy {
					sampler_builder = sampler_builder.anisotropy(anisotropy.get() as f32);
				}

				let sampler_handler = device.build_sampler(sampler_builder);
				v.insert(sampler_handler);
				sampler_handler
			}
		}
	}
}
