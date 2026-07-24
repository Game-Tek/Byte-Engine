use std::{
	alloc::Allocator,
	error::Error,
	f32::consts::{PI, TAU},
	fmt,
};

use exr::prelude::f16;
use utils::Extent;

use crate::{
	resources::image::{
		ibl_prefiltered_specular_stream_name, ImageIbl, ImageSubresource, IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
		IBL_PREFILTERED_SPECULAR_MIP_COUNT, IMAGE_BASE_MIP_STREAM_NAME,
	},
	types::{Formats, Gamma},
	StreamDescription,
};

const BYTES_PER_RGBA16F_PIXEL: usize = 4 * std::mem::size_of::<f16>();
const MAX_SPECULAR_WIDTH: u32 = 1024;
const MAX_SPECULAR_HEIGHT: u32 = 512;
const DIFFUSE_WIDTH: u32 = 32;
const DIFFUSE_HEIGHT: u32 = 16;
const DIFFUSE_SAMPLE_COUNT: usize = 256;
const SPECULAR_SAMPLE_COUNT: usize = 128;

type Vector3 = [f32; 3];
type Radiance = [f32; 3];

/// The `BakedImageIbl` struct carries the parent image and its embedded lighting maps into resource storage.
pub struct BakedImageIbl<'a> {
	pub root_extent: [u32; 3],
	pub ibl: ImageIbl,
	pub streams: Vec<StreamDescription>,
	pub data: Box<[u8], &'a dyn Allocator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IblBakeError {
	ZeroDimensions,
	BufferSizeMismatch { expected: usize, got: usize },
	DimensionsTooLarge,
	AllocationFailed,
}

impl fmt::Display for IblBakeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::ZeroDimensions => formatter.write_str(
				"Invalid environment dimensions. The most likely cause is an EXR layer with zero width or height.",
			),
			Self::BufferSizeMismatch { expected, got } => write!(
				formatter,
				"Invalid environment buffer size: expected {expected}, got {got}. The most likely cause is mismatched EXR dimensions and RGBA16F pixels."
			),
			Self::DimensionsTooLarge => formatter.write_str(
				"Environment dimensions are too large. The most likely cause is integer overflow while laying out IBL subresources.",
			),
			Self::AllocationFailed => formatter.write_str(
				"Environment IBL allocation failed. The most likely cause is insufficient memory for the baked image subresources.",
			),
		}
	}
}

impl Error for IblBakeError {}

/// Bakes normalized diffuse irradiance and eight GGX-prefiltered specular levels beside an EXR base image.
pub fn bake_image_ibl_in<'a>(
	source_extent: Extent,
	source_rgba16f: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<BakedImageIbl<'a>, IblBakeError> {
	let source_width = source_extent.width();
	let source_height = source_extent.height();
	if source_width == 0 || source_height == 0 {
		return Err(IblBakeError::ZeroDimensions);
	}

	let expected_source_size = image_byte_size(source_width, source_height)?;
	if source_rgba16f.len() != expected_source_size {
		return Err(IblBakeError::BufferSizeMismatch {
			expected: expected_source_size,
			got: source_rgba16f.len(),
		});
	}

	// Sampling from decoded f32 radiance avoids repeating four half-float conversions for every
	// bilinear tap during the comparatively expensive convolution loops.
	let source = decode_source_radiance(source_rgba16f, allocator)?;
	let specular_width = source_width.min(MAX_SPECULAR_WIDTH);
	let specular_height = source_height.min(MAX_SPECULAR_HEIGHT);
	let specular_extents = specular_extents(specular_width, specular_height);
	let root_size = expected_source_size;
	let diffuse_size = image_byte_size(DIFFUSE_WIDTH, DIFFUSE_HEIGHT)?;

	let mut total_size = root_size;
	for &(width, height) in &specular_extents {
		total_size = total_size
			.checked_add(image_byte_size(width, height)?)
			.ok_or(IblBakeError::DimensionsTooLarge)?;
	}
	total_size = total_size.checked_add(diffuse_size).ok_or(IblBakeError::DimensionsTooLarge)?;

	let mut data = Vec::new_in(allocator);
	data.try_reserve_exact(total_size)
		.map_err(|_| IblBakeError::AllocationFailed)?;
	data.resize(total_size, 0);

	let mut streams = Vec::with_capacity(IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize + 2);
	streams.push(StreamDescription::new(IMAGE_BASE_MIP_STREAM_NAME, root_size, 0));
	data[..root_size].copy_from_slice(source_rgba16f);

	let mut offset = root_size;
	for (level, &(width, height)) in specular_extents.iter().enumerate() {
		let level_size = image_byte_size(width, height)?;
		let level_end = offset.checked_add(level_size).ok_or(IblBakeError::DimensionsTooLarge)?;
		if level == 0 {
			if width == source_width && height == source_height {
				write_sanitized_source(&source, &mut data[offset..level_end]);
			} else {
				resample_environment(
					&source,
					source_width,
					source_height,
					width,
					height,
					&mut data[offset..level_end],
				);
			}
		} else {
			let roughness = level as f32 / (IBL_PREFILTERED_SPECULAR_MIP_COUNT - 1) as f32;
			prefilter_specular_level(
				&source,
				source_width,
				source_height,
				width,
				height,
				roughness,
				&mut data[offset..level_end],
			);
		}
		streams.push(StreamDescription::new(
			&ibl_prefiltered_specular_stream_name(level as u32),
			level_size,
			offset,
		));
		offset = level_end;
	}

	let diffuse_end = offset.checked_add(diffuse_size).ok_or(IblBakeError::DimensionsTooLarge)?;
	convolve_diffuse_irradiance(
		&source,
		source_width,
		source_height,
		DIFFUSE_WIDTH,
		DIFFUSE_HEIGHT,
		&mut data[offset..diffuse_end],
	);
	streams.push(StreamDescription::new(
		IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
		diffuse_size,
		offset,
	));
	debug_assert_eq!(diffuse_end, data.len());

	let subresource = |extent, mip_count| ImageSubresource {
		format: Formats::RGBA16F,
		gamma: Gamma::Linear,
		extent,
		mip_count,
	};

	Ok(BakedImageIbl {
		root_extent: [source_width, source_height, 1],
		ibl: ImageIbl {
			diffuse_irradiance: subresource([DIFFUSE_WIDTH, DIFFUSE_HEIGHT, 1], 1),
			prefiltered_specular: subresource([specular_width, specular_height, 1], IBL_PREFILTERED_SPECULAR_MIP_COUNT),
		},
		streams,
		data: data.into_boxed_slice(),
	})
}

fn image_byte_size(width: u32, height: u32) -> Result<usize, IblBakeError> {
	(width as usize)
		.checked_mul(height as usize)
		.and_then(|pixels| pixels.checked_mul(BYTES_PER_RGBA16F_PIXEL))
		.ok_or(IblBakeError::DimensionsTooLarge)
}

fn specular_extents(mut width: u32, mut height: u32) -> [(u32, u32); IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize] {
	let mut extents = [(1, 1); IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize];
	for extent in &mut extents {
		*extent = (width, height);
		width = (width / 2).max(1);
		height = (height / 2).max(1);
	}
	extents
}

fn decode_source_radiance<'a>(
	source: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<Vec<Radiance, &'a dyn Allocator>, IblBakeError> {
	let pixel_count = source.len() / BYTES_PER_RGBA16F_PIXEL;
	let mut radiance = Vec::new_in(allocator);
	radiance
		.try_reserve_exact(pixel_count)
		.map_err(|_| IblBakeError::AllocationFailed)?;

	for pixel in source.chunks_exact(BYTES_PER_RGBA16F_PIXEL) {
		radiance.push([
			decode_finite_half(&pixel[0..2]),
			decode_finite_half(&pixel[2..4]),
			decode_finite_half(&pixel[4..6]),
		]);
	}

	Ok(radiance)
}

fn decode_finite_half(bytes: &[u8]) -> f32 {
	let value = f16::from_le_bytes([bytes[0], bytes[1]]).to_f32();
	if value.is_finite() {
		value
	} else {
		0.0
	}
}

fn write_sanitized_source(source: &[Radiance], destination: &mut [u8]) {
	for (radiance, pixel) in source.iter().zip(destination.chunks_exact_mut(BYTES_PER_RGBA16F_PIXEL)) {
		write_rgba16f(pixel, *radiance);
	}
}

fn resample_environment(
	source: &[Radiance],
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
	destination: &mut [u8],
) {
	for y in 0..destination_height {
		for x in 0..destination_width {
			let direction = texel_direction(x, y, destination_width, destination_height);
			let radiance = sample_direction(source, source_width, source_height, direction);
			let offset = ((y * destination_width + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
			write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
		}
	}
}

/// Stores irradiance divided by pi, allowing Lambertian shading to multiply this map by albedo directly.
fn convolve_diffuse_irradiance(
	source: &[Radiance],
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
	destination: &mut [u8],
) {
	let samples = cosine_hemisphere_samples();

	for y in 0..destination_height {
		for x in 0..destination_width {
			let normal = texel_direction(x, y, destination_width, destination_height);
			let (tangent, bitangent) = orthonormal_basis(normal);
			let mut sum = [0.0_f64; 3];

			for &local_direction in &samples {
				let direction = tangent_to_world(local_direction, tangent, bitangent, normal);
				let radiance = sample_direction(source, source_width, source_height, direction);
				for channel in 0..3 {
					sum[channel] += radiance[channel] as f64;
				}
			}

			let scale = 1.0 / DIFFUSE_SAMPLE_COUNT as f64;
			let radiance = [(sum[0] * scale) as f32, (sum[1] * scale) as f32, (sum[2] * scale) as f32];
			let offset = ((y * destination_width + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
			write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
		}
	}
}

fn prefilter_specular_level(
	source: &[Radiance],
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
	roughness: f32,
	destination: &mut [u8],
) {
	let samples = ggx_half_vector_samples(roughness);

	for y in 0..destination_height {
		for x in 0..destination_width {
			let normal = texel_direction(x, y, destination_width, destination_height);
			let view = normal;
			let (tangent, bitangent) = orthonormal_basis(normal);
			let mut sum = [0.0_f64; 3];
			let mut total_weight = 0.0_f64;

			for &local_half_vector in &samples {
				let half_vector = normalize(tangent_to_world(local_half_vector, tangent, bitangent, normal));
				let view_dot_half = dot(view, half_vector).max(0.0);
				let light = normalize(sub(scale(half_vector, 2.0 * view_dot_half), view));
				let normal_dot_light = dot(normal, light).max(0.0);
				if normal_dot_light <= 0.0 {
					continue;
				}

				let radiance = sample_direction(source, source_width, source_height, light);
				let weight = normal_dot_light as f64;
				for channel in 0..3 {
					sum[channel] += radiance[channel] as f64 * weight;
				}
				total_weight += weight;
			}

			let radiance = if total_weight > 0.0 {
				[
					(sum[0] / total_weight) as f32,
					(sum[1] / total_weight) as f32,
					(sum[2] / total_weight) as f32,
				]
			} else {
				sample_direction(source, source_width, source_height, normal)
			};
			let offset = ((y * destination_width + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
			write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
		}
	}
}

fn cosine_hemisphere_samples() -> [Vector3; DIFFUSE_SAMPLE_COUNT] {
	let mut samples = [[0.0; 3]; DIFFUSE_SAMPLE_COUNT];
	for (index, sample) in samples.iter_mut().enumerate() {
		let [radial_sample, angular_sample] = hammersley(index, DIFFUSE_SAMPLE_COUNT);
		let radius = radial_sample.sqrt();
		let angle = TAU * angular_sample;
		let (sin_angle, cos_angle) = angle.sin_cos();
		*sample = [radius * cos_angle, radius * sin_angle, (1.0 - radial_sample).max(0.0).sqrt()];
	}
	samples
}

fn ggx_half_vector_samples(roughness: f32) -> [Vector3; SPECULAR_SAMPLE_COUNT] {
	let mut samples = [[0.0; 3]; SPECULAR_SAMPLE_COUNT];
	let alpha = roughness * roughness;
	let alpha_squared = alpha * alpha;

	for (index, sample) in samples.iter_mut().enumerate() {
		let [angular_sample, elevation_sample] = hammersley(index, SPECULAR_SAMPLE_COUNT);
		let angle = TAU * angular_sample;
		let cos_theta = ((1.0 - elevation_sample) / (1.0 + (alpha_squared - 1.0) * elevation_sample))
			.max(0.0)
			.sqrt();
		let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
		let (sin_angle, cos_angle) = angle.sin_cos();
		*sample = [sin_theta * cos_angle, sin_theta * sin_angle, cos_theta];
	}

	samples
}

fn hammersley(index: usize, sample_count: usize) -> [f32; 2] {
	[
		index as f32 / sample_count as f32,
		(index as u32).reverse_bits() as f32 * 2.328_306_4e-10,
	]
}

fn texel_direction(x: u32, y: u32, width: u32, height: u32) -> Vector3 {
	let u = (x as f32 + 0.5) / width as f32;
	let v = (y as f32 + 0.5) / height as f32;
	let longitude = (u - 0.5) * TAU;
	let latitude = (0.5 - v) * PI;
	let (sin_longitude, cos_longitude) = longitude.sin_cos();
	let (sin_latitude, cos_latitude) = latitude.sin_cos();
	[cos_latitude * cos_longitude, sin_latitude, cos_latitude * sin_longitude]
}

fn sample_direction(source: &[Radiance], width: u32, height: u32, direction: Vector3) -> Radiance {
	let direction = normalize(direction);
	let u = direction[2].atan2(direction[0]) / TAU + 0.5;
	let v = 0.5 - direction[1].clamp(-1.0, 1.0).asin() / PI;
	sample_lat_long_uv(source, width, height, u, v)
}

fn sample_lat_long_uv(source: &[Radiance], width: u32, height: u32, u: f32, v: f32) -> Radiance {
	let source_x = u * width as f32 - 0.5;
	let x0_unwrapped = source_x.floor() as i64;
	let x_fraction = source_x - x0_unwrapped as f32;
	let x0 = x0_unwrapped.rem_euclid(width as i64) as usize;
	let x1 = (x0 + 1) % width as usize;

	let source_y = (v * height as f32 - 0.5).clamp(0.0, height.saturating_sub(1) as f32);
	let y0 = source_y.floor() as usize;
	let y1 = (y0 + 1).min(height as usize - 1);
	let y_fraction = source_y - y0 as f32;

	let top = lerp_radiance(source[y0 * width as usize + x0], source[y0 * width as usize + x1], x_fraction);
	let bottom = lerp_radiance(source[y1 * width as usize + x0], source[y1 * width as usize + x1], x_fraction);
	lerp_radiance(top, bottom, y_fraction)
}

fn lerp_radiance(a: Radiance, b: Radiance, amount: f32) -> Radiance {
	[
		a[0] + (b[0] - a[0]) * amount,
		a[1] + (b[1] - a[1]) * amount,
		a[2] + (b[2] - a[2]) * amount,
	]
}

/// Builds a stable tangent frame without choosing a different helper axis near the lat-long poles.
fn orthonormal_basis(normal: Vector3) -> (Vector3, Vector3) {
	let sign = if normal[2] >= 0.0 { 1.0 } else { -1.0 };
	let a = -1.0 / (sign + normal[2]);
	let b = normal[0] * normal[1] * a;
	(
		[1.0 + sign * normal[0] * normal[0] * a, sign * b, -sign * normal[0]],
		[b, sign + normal[1] * normal[1] * a, -normal[1]],
	)
}

fn tangent_to_world(local: Vector3, tangent: Vector3, bitangent: Vector3, normal: Vector3) -> Vector3 {
	[
		tangent[0] * local[0] + bitangent[0] * local[1] + normal[0] * local[2],
		tangent[1] * local[0] + bitangent[1] * local[1] + normal[1] * local[2],
		tangent[2] * local[0] + bitangent[2] * local[1] + normal[2] * local[2],
	]
}

fn dot(a: Vector3, b: Vector3) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(vector: Vector3, scale: f32) -> Vector3 {
	[vector[0] * scale, vector[1] * scale, vector[2] * scale]
}

fn sub(a: Vector3, b: Vector3) -> Vector3 {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize(vector: Vector3) -> Vector3 {
	let length_squared = dot(vector, vector);
	if length_squared > 0.0 && length_squared.is_finite() {
		scale(vector, length_squared.sqrt().recip())
	} else {
		[1.0, 0.0, 0.0]
	}
}

fn write_rgba16f(destination: &mut [u8], radiance: Radiance) {
	for (channel, value) in radiance.into_iter().enumerate() {
		let value = if value.is_finite() { value } else { 0.0 };
		destination[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
	}
	destination[6..8].copy_from_slice(&f16::from_f32(1.0).to_le_bytes());
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use exr::prelude::f16;
	use utils::Extent;

	use super::{
		bake_image_ibl_in, image_byte_size, sample_lat_long_uv, IblBakeError, Radiance, BYTES_PER_RGBA16F_PIXEL,
		DIFFUSE_HEIGHT, DIFFUSE_WIDTH,
	};
	use crate::resources::image::{
		ibl_prefiltered_specular_stream_name, IBL_DIFFUSE_IRRADIANCE_STREAM_NAME, IBL_PREFILTERED_SPECULAR_MIP_COUNT,
		IMAGE_BASE_MIP_STREAM_NAME,
	};

	fn constant_source(width: u32, height: u32, color: Radiance) -> Vec<u8> {
		let mut source = vec![0; image_byte_size(width, height).unwrap()];
		for pixel in source.chunks_exact_mut(BYTES_PER_RGBA16F_PIXEL) {
			for (channel, value) in color.into_iter().enumerate() {
				pixel[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
			}
			pixel[6..8].copy_from_slice(&f16::from_f32(0.25).to_le_bytes());
		}
		source
	}

	fn decode_pixel(pixel: &[u8]) -> [f32; 4] {
		let mut values = [0.0; 4];
		for (channel, bytes) in pixel.chunks_exact(2).enumerate() {
			values[channel] = f16::from_le_bytes([bytes[0], bytes[1]]).to_f32();
		}
		values
	}

	#[test]
	fn constant_environment_stays_constant_in_every_ibl_stream() {
		let color = [4.0, 0.5, 2.0];
		let source = constant_source(4, 2, color);
		let first = bake_image_ibl_in(Extent::rectangle(4, 2), &source, &Global).unwrap();
		let second = bake_image_ibl_in(Extent::rectangle(4, 2), &source, &Global).unwrap();

		assert_eq!(
			first.data.as_ref(),
			second.data.as_ref(),
			"fixed sampling must bake stable bytes"
		);
		assert_eq!(first.root_extent, [4, 2, 1]);
		assert_eq!(first.ibl.diffuse_irradiance.extent, [DIFFUSE_WIDTH, DIFFUSE_HEIGHT, 1]);
		assert_eq!(first.ibl.prefiltered_specular.extent, [4, 2, 1]);
		assert_eq!(first.ibl.prefiltered_specular.mip_count, IBL_PREFILTERED_SPECULAR_MIP_COUNT);
		assert_eq!(first.streams.len(), IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize + 2);

		let root = &first.streams[0];
		let specular_zero = &first.streams[1];
		assert_eq!(root.name(), IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!(root.offset(), 0);
		assert_eq!(root.size(), 4 * 2 * BYTES_PER_RGBA16F_PIXEL);
		assert_eq!(specular_zero.name(), ibl_prefiltered_specular_stream_name(0));
		assert_eq!(specular_zero.offset(), root.size());
		assert_eq!(specular_zero.size(), root.size());
		assert_eq!(first.streams.last().unwrap().name(), IBL_DIFFUSE_IRRADIANCE_STREAM_NAME);

		for pixel in first.data[root.offset()..root.offset() + root.size()].chunks_exact(BYTES_PER_RGBA16F_PIXEL) {
			assert_eq!(decode_pixel(pixel), [color[0], color[1], color[2], 0.25]);
		}
		for stream in &first.streams[1..] {
			let bytes = &first.data[stream.offset()..stream.offset() + stream.size()];
			for pixel in bytes.chunks_exact(BYTES_PER_RGBA16F_PIXEL) {
				assert_eq!(decode_pixel(pixel), [color[0], color[1], color[2], 1.0]);
			}
		}

		let mut expected_offset = root.size();
		let mut expected_width = 4_u32;
		let mut expected_height = 2_u32;
		for (level, stream) in first.streams[1..1 + IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize]
			.iter()
			.enumerate()
		{
			assert_eq!(stream.name(), ibl_prefiltered_specular_stream_name(level as u32));
			assert_eq!(stream.offset(), expected_offset);
			assert_eq!(
				stream.size(),
				expected_width as usize * expected_height as usize * BYTES_PER_RGBA16F_PIXEL
			);
			expected_offset += stream.size();
			expected_width = (expected_width / 2).max(1);
			expected_height = (expected_height / 2).max(1);
		}
		assert_eq!(first.streams.last().unwrap().offset(), expected_offset);
		assert_eq!(expected_offset + first.streams.last().unwrap().size(), first.data.len());
	}

	#[test]
	fn specular_cap_does_not_change_the_parent_exr_image() {
		let source = constant_source(1025, 1, [2.0, 3.0, 4.0]);
		let baked = bake_image_ibl_in(Extent::rectangle(1025, 1), &source, &Global).unwrap();
		let root = &baked.streams[0];
		let specular_zero = &baked.streams[1];

		assert_eq!(baked.root_extent, [1025, 1, 1]);
		assert_eq!(baked.ibl.prefiltered_specular.extent, [1024, 1, 1]);
		assert_eq!(root.size(), source.len());
		assert_eq!(&baked.data[root.offset()..root.offset() + root.size()], source.as_slice());
		assert_eq!(specular_zero.size(), 1024 * BYTES_PER_RGBA16F_PIXEL);
		for pixel in baked.data[specular_zero.offset()..specular_zero.offset() + specular_zero.size()]
			.chunks_exact(BYTES_PER_RGBA16F_PIXEL)
		{
			assert_eq!(decode_pixel(pixel), [2.0, 3.0, 4.0, 1.0]);
		}
	}

	#[test]
	fn horizontal_sampling_wraps_at_the_lat_long_seam() {
		let source = vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0], [5.0, 0.0, 0.0], [7.0, 0.0, 0.0]];
		let at_zero = sample_lat_long_uv(&source, 4, 1, 0.0, 0.5);
		let at_one = sample_lat_long_uv(&source, 4, 1, 1.0, 0.5);

		assert_eq!(at_zero, [4.0, 0.0, 0.0]);
		assert_eq!(at_one, at_zero);
	}

	#[test]
	fn malformed_source_layout_is_rejected_before_allocation() {
		assert_eq!(
			bake_image_ibl_in(Extent::rectangle(0, 2), &[], &Global).err(),
			Some(IblBakeError::ZeroDimensions)
		);
		assert_eq!(
			bake_image_ibl_in(Extent::rectangle(2, 1), &[0; 8], &Global).err(),
			Some(IblBakeError::BufferSizeMismatch { expected: 16, got: 8 })
		);
	}
}
