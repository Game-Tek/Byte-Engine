//! This modules contains code for the `TextureManager` struct, which is responsible for loading and managing textures.

use std::{collections::hash_map::Entry, num::NonZeroU8, sync::Arc};

use ghi::{
	device::{Device as _, DeviceCreate as _},
	Frame as _, Size as _,
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
	textures: HashMap<String, (ghi::BaseImageHandle, ghi::SamplerHandle)>,
}

/// The `TextureUpload` struct carries row-padded texture bytes until the transfer queue copies them.
pub struct TextureUpload {
	pub data: Vec<u8>,
	pub source_bytes_per_row: usize,
	pub source_bytes_per_image: usize,
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
	) -> Option<(String, ghi::BaseImageHandle, ghi::SamplerHandle, Option<TextureUpload>)> {
		if let Some(r) = self.textures.get(reference.id()) {
			return Some((reference.id().to_string(), r.0, r.1, None));
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
			resource_management::types::Formats::BC7SRGB => ghi::Formats::BC7SRGB,
		};

		let extent = Extent::from(texture.extent);

		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);

		let mut source = vec![0u8; reference.size];
		let load_target = reference.load(source.as_mut_slice().into()).ok()?;
		let source = load_target.buffer()?;
		let upload = make_texture_upload(format, extent, source)?;

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

		let v = (image.into(), sampler);

		self.textures.insert(reference.id().to_string(), v.clone());

		Some((reference.id().to_string(), v.0, v.1, Some(upload)))
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

	pub fn loaded_textures(&self) -> Vec<(String, ghi::BaseImageHandle, ghi::SamplerHandle)> {
		self.textures
			.iter()
			.map(|(name, (image, sampler))| (name.clone(), *image, *sampler))
			.collect()
	}
}

/// Builds row-padded upload data compatible with the transfer command buffer image copy path.
fn make_texture_upload(format: ghi::Formats, extent: Extent, source: &[u8]) -> Option<TextureUpload> {
	let (source_bytes_per_row, row_count, compact_bytes_per_image) = texture_upload_layout(format, extent)?;
	if source.len() < compact_bytes_per_image {
		return None;
	}
	assert_eq!(
		source.len(),
		compact_bytes_per_image,
		"Texture upload source size mismatch. The most likely cause is that the baked texture payload does not match the runtime texture layout. format={format:?}, extent={extent:?}, source_len={}, source_bytes_per_row={source_bytes_per_row}, row_count={row_count}, expected={compact_bytes_per_image}",
		source.len()
	);

	let padded_bytes_per_row = source_bytes_per_row.next_multiple_of(256);
	let source_bytes_per_image = padded_bytes_per_row * row_count;
	assert_eq!(
		padded_bytes_per_row % 256,
		0,
		"Texture upload row pitch alignment mismatch. The most likely cause is that the Metal upload layout was built without 256-byte row alignment. format={format:?}, extent={extent:?}, source_bytes_per_row={source_bytes_per_row}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	assert!(
		source_bytes_per_image >= compact_bytes_per_image,
		"Texture upload padded image is smaller than compact image. The most likely cause is an invalid row count or row pitch. format={format:?}, extent={extent:?}, compact_bytes_per_image={compact_bytes_per_image}, source_bytes_per_image={source_bytes_per_image}, row_count={row_count}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	let mut data = vec![0u8; source_bytes_per_image];

	for row in 0..row_count {
		let source_offset = row * source_bytes_per_row;
		let destination_offset = row * padded_bytes_per_row;
		let source_end = source_offset + source_bytes_per_row;
		let destination_end = destination_offset + source_bytes_per_row;
		assert!(
			source_end <= source.len(),
			"Texture upload source row is out of bounds. The most likely cause is a bad compact row pitch for this format. format={format:?}, extent={extent:?}, row={row}, row_count={row_count}, source_offset={source_offset}, source_end={source_end}, source_len={}, source_bytes_per_row={source_bytes_per_row}",
			source.len()
		);
		assert!(
			destination_end <= data.len(),
			"Texture upload padded row is out of bounds. The most likely cause is a bad padded row pitch for this format. format={format:?}, extent={extent:?}, row={row}, row_count={row_count}, destination_offset={destination_offset}, destination_end={destination_end}, data_len={}, padded_bytes_per_row={padded_bytes_per_row}",
			data.len()
		);
		let source_row = &source[source_offset..source_end];
		data[destination_offset..destination_end].copy_from_slice(source_row);
	}
	assert_eq!(
		data.len(),
		source_bytes_per_image,
		"Texture upload output size mismatch. The most likely cause is that the padded upload allocation changed during row copy. format={format:?}, extent={extent:?}, data_len={}, expected={source_bytes_per_image}",
		data.len()
	);

	Some(TextureUpload {
		data,
		source_bytes_per_row: padded_bytes_per_row,
		source_bytes_per_image,
	})
}

/// Computes the compact source layout for one mip of the given texture format.
fn texture_upload_layout(format: ghi::Formats, extent: Extent) -> Option<(usize, usize, usize)> {
	let width = extent.width().max(1) as usize;
	let height = extent.height().max(1) as usize;

	match format {
		ghi::Formats::BC5 | ghi::Formats::BC7 | ghi::Formats::BC7SRGB => {
			let block_width = width.div_ceil(4);
			let block_height = height.div_ceil(4);
			let bytes_per_row = block_width * 16;
			Some((bytes_per_row, block_height, bytes_per_row * block_height))
		}
		_ => {
			let bytes_per_row = width * format.size();
			Some((bytes_per_row, height, bytes_per_row * height))
		}
	}
}
