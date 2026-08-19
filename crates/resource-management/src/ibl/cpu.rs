pub(super) const BYTES_PER_RGBA16F_PIXEL: usize = 4 * std::mem::size_of::<f16>();
const MAX_SPECULAR_WIDTH: u32 = 1024;
const MAX_SPECULAR_HEIGHT: u32 = 512;
const DIFFUSE_WIDTH: u32 = 32;
const DIFFUSE_HEIGHT: u32 = 16;
pub(super) const MAX_SPECULAR_CUBE_FACE_SIZE: u32 = 256;
pub(super) const DIFFUSE_CUBE_FACE_SIZE: u32 = 8;
pub(super) const CUBE_FACE_COUNT: usize = 6;
const DIFFUSE_SAMPLE_COUNT: usize = 1024;
const SPECULAR_SAMPLE_COUNT: usize = 1024;

type Vector3 = [f32; 3];
pub(super) type Radiance = [f32; 3];

/// The `SourceMIP` struct provides one transient, solid-angle-filtered source level to an IBL integrator.
pub(super) struct SourceMIP<'a> {
	pub(super) width: u32,
	pub(super) height: u32,
	pub(super) pixels: Vec<Radiance, &'a dyn Allocator>,
}

/// The `BakedImageIBL` struct carries the parent image and its embedded lighting maps into resource storage.
pub struct BakedImageIBL<'a> {
	pub root_extent: [u32; 3],
	pub ibl: ImageIBL,
	pub streams: Vec<StreamDescription>,
	pub data: Box<[u8], &'a dyn Allocator>,
}

/// The `CubemapIBLLayout` struct keeps CPU and GPU environment-map generators on one binary resource contract.
#[derive(Clone, Copy)]
pub(super) struct CubemapIBLLayout {
	source_width: u32,
	source_height: u32,
	root_size: usize,
	specular_face_size: u32,
	specular_face_sizes: [u32; IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize],
	specular_offsets: [usize; IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize],
	diffuse_offset: usize,
	diffuse_size: usize,
	total_size: usize,
}

impl CubemapIBLLayout {
	/// Validates the source and computes every tightly packed cubemap stream range.
	pub(super) fn new(source_extent: Extent, source_rgba16f: &[u8]) -> Result<Self, IBLBakeError> {
		let source_width = source_extent.width();
		let source_height = source_extent.height();
		if source_width == 0 || source_height == 0 {
			return Err(IBLBakeError::ZeroDimensions);
		}

		let root_size = image_byte_size(source_width, source_height)?;
		if source_rgba16f.len() != root_size {
			return Err(IBLBakeError::BufferSizeMismatch {
				expected: root_size,
				got: source_rgba16f.len(),
			});
		}

		let specular_face_size = (source_width / 4)
			.min(source_height / 2)
			.clamp(1, MAX_SPECULAR_CUBE_FACE_SIZE);
		let specular_face_sizes = std::array::from_fn(|level| specular_face_size.checked_shr(level as u32).unwrap_or(0).max(1));
		let cube_size = |face_size| {
			image_byte_size(face_size, face_size)?
				.checked_mul(CUBE_FACE_COUNT)
				.ok_or(IBLBakeError::DimensionsTooLarge)
		};

		let mut offset = root_size;
		let mut specular_offsets = [0; IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize];
		for (level, &face_size) in specular_face_sizes.iter().enumerate() {
			specular_offsets[level] = offset;
			offset = offset
				.checked_add(cube_size(face_size)?)
				.ok_or(IBLBakeError::DimensionsTooLarge)?;
		}
		let diffuse_offset = offset;
		let diffuse_size = cube_size(DIFFUSE_CUBE_FACE_SIZE)?;
		let total_size = diffuse_offset
			.checked_add(diffuse_size)
			.ok_or(IBLBakeError::DimensionsTooLarge)?;

		Ok(Self {
			source_width,
			source_height,
			root_size,
			specular_face_size,
			specular_face_sizes,
			specular_offsets,
			diffuse_offset,
			diffuse_size,
			total_size,
		})
	}

	pub(super) fn source_dimensions(self) -> (u32, u32) {
		(self.source_width, self.source_height)
	}

	#[cfg(feature = "gpu-ibl")]
	pub(super) fn specular_face_size(self) -> u32 {
		self.specular_face_size
	}

	pub(super) fn specular_face_sizes(self) -> [u32; IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize] {
		self.specular_face_sizes
	}

	pub(super) fn specular_range(self, level: usize) -> std::ops::Range<usize> {
		let start = self.specular_offsets[level];
		let end = self.specular_offsets.get(level + 1).copied().unwrap_or(self.diffuse_offset);
		start..end
	}

	pub(super) fn diffuse_range(self) -> std::ops::Range<usize> {
		self.diffuse_offset..self.total_size
	}

	#[cfg(feature = "gpu-ibl")]
	pub(super) fn root_size(self) -> usize {
		self.root_size
	}

	#[cfg(feature = "gpu-ibl")]
	pub(super) fn total_size(self) -> usize {
		self.total_size
	}

	/// Builds the metadata shared by allocator-backed and owned bake results.
	pub(super) fn metadata(self) -> ([u32; 3], ImageIBL, Vec<StreamDescription>) {
		let mut streams = Vec::with_capacity(IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize + 2);
		streams.push(StreamDescription::new(IMAGE_BASE_MIP_STREAM_NAME, self.root_size, 0));
		for level in 0..IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize {
			let range = self.specular_range(level);
			streams.push(StreamDescription::new(
				ibl_prefiltered_specular_stream_name(level as u32),
				range.len(),
				range.start,
			));
		}
		streams.push(StreamDescription::new(
			IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
			self.diffuse_size,
			self.diffuse_offset,
		));

		let subresource = |face_size, mip_count| ImageSubresource {
			format: Formats::RGBA16F,
			gamma: Gamma::Linear,
			extent: [face_size, face_size, 1],
			mip_count,
			array_layers: CUBE_FACE_COUNT as u32,
		};
		(
			[self.source_width, self.source_height, 1],
			ImageIBL {
				diffuse_irradiance: subresource(DIFFUSE_CUBE_FACE_SIZE, 1),
				prefiltered_specular: subresource(self.specular_face_size, IBL_PREFILTERED_SPECULAR_MIP_COUNT),
			},
			streams,
		)
	}

	/// Allocates final storage once and preserves the decoded EXR as the root stream.
	pub(super) fn allocate_data<'a>(
		self,
		source_rgba16f: &[u8],
		allocator: &'a dyn Allocator,
	) -> Result<Vec<u8, &'a dyn Allocator>, IBLBakeError> {
		let mut data = Vec::new_in(allocator);
		data.try_reserve_exact(self.total_size)
			.map_err(|_| IBLBakeError::AllocationFailed)?;
		data.resize(self.total_size, 0);
		data[..self.root_size].copy_from_slice(source_rgba16f);
		Ok(data)
	}

	/// Adds stable stream metadata after an integrator fills all derived ranges.
	pub(super) fn finish<'a>(self, data: Vec<u8, &'a dyn Allocator>) -> BakedImageIBL<'a> {
		debug_assert_eq!(data.len(), self.total_size);
		let (root_extent, ibl, streams) = self.metadata();
		BakedImageIBL {
			root_extent,
			ibl,
			streams,
			data: data.into_boxed_slice(),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IBLBakeError {
	ZeroDimensions,
	BufferSizeMismatch { expected: usize, got: usize },
	DimensionsTooLarge,
	AllocationFailed,
}

impl fmt::Display for IBLBakeError {
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

impl Error for IBLBakeError {}

/// Bakes normalized diffuse irradiance and eight GGX-prefiltered specular levels beside an EXR base image.
pub fn bake_image_ibl_lat_long_in<'a>(
	source_extent: Extent,
	source_rgba16f: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<BakedImageIBL<'a>, IBLBakeError> {
	let source_width = source_extent.width();
	let source_height = source_extent.height();
	if source_width == 0 || source_height == 0 {
		return Err(IBLBakeError::ZeroDimensions);
	}

	let expected_source_size = image_byte_size(source_width, source_height)?;
	if source_rgba16f.len() != expected_source_size {
		return Err(IBLBakeError::BufferSizeMismatch {
			expected: expected_source_size,
			got: source_rgba16f.len(),
		});
	}

	// Sampling from decoded f32 radiance avoids repeating four half-float conversions for every
	// bilinear tap during the comparatively expensive convolution loops.
	let source = decode_source_radiance(source_rgba16f, allocator)?;
	let source_mips = build_source_mips(source_width, source_height, source, allocator)?;
	let specular_width = source_width.min(MAX_SPECULAR_WIDTH);
	let specular_height = source_height.min(MAX_SPECULAR_HEIGHT);
	let specular_extents = specular_extents(specular_width, specular_height);
	let root_size = expected_source_size;
	let diffuse_size = image_byte_size(DIFFUSE_WIDTH, DIFFUSE_HEIGHT)?;

	let mut total_size = root_size;
	for &(width, height) in &specular_extents {
		total_size = total_size
			.checked_add(image_byte_size(width, height)?)
			.ok_or(IBLBakeError::DimensionsTooLarge)?;
	}
	total_size = total_size.checked_add(diffuse_size).ok_or(IBLBakeError::DimensionsTooLarge)?;

	let mut data = Vec::new_in(allocator);
	data.try_reserve_exact(total_size)
		.map_err(|_| IBLBakeError::AllocationFailed)?;
	data.resize(total_size, 0);

	let mut streams = Vec::with_capacity(IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize + 2);
	streams.push(StreamDescription::new(IMAGE_BASE_MIP_STREAM_NAME, root_size, 0));
	data[..root_size].copy_from_slice(source_rgba16f);

	let mut offset = root_size;
	for (level, &(width, height)) in specular_extents.iter().enumerate() {
		let level_size = image_byte_size(width, height)?;
		let level_end = offset.checked_add(level_size).ok_or(IBLBakeError::DimensionsTooLarge)?;
		if level == 0 {
			if width == source_width && height == source_height {
				write_sanitized_source(&source_mips[0].pixels, &mut data[offset..level_end]);
			} else {
				resample_environment(
					&source_mips[0].pixels,
					source_width,
					source_height,
					width,
					height,
					&mut data[offset..level_end],
				);
			}
		} else {
			let roughness = level as f32 / (IBL_PREFILTERED_SPECULAR_MIP_COUNT - 1) as f32;
			prefilter_specular_level(&source_mips, width, height, roughness, &mut data[offset..level_end]);
		}
		streams.push(StreamDescription::new(
			ibl_prefiltered_specular_stream_name(level as u32),
			level_size,
			offset,
		));
		offset = level_end;
	}

	let diffuse_end = offset.checked_add(diffuse_size).ok_or(IBLBakeError::DimensionsTooLarge)?;
	convolve_diffuse_irradiance(&source_mips, DIFFUSE_WIDTH, DIFFUSE_HEIGHT, &mut data[offset..diffuse_end]);
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
		array_layers: 1,
	};

	Ok(BakedImageIBL {
		root_extent: [source_width, source_height, 1],
		ibl: ImageIBL {
			diffuse_irradiance: subresource([DIFFUSE_WIDTH, DIFFUSE_HEIGHT, 1], 1),
			prefiltered_specular: subresource([specular_width, specular_height, 1], IBL_PREFILTERED_SPECULAR_MIP_COUNT),
		},
		streams,
		data: data.into_boxed_slice(),
	})
}

/// Bakes native cubemaps while retaining [`bake_image_ibl_lat_long_in`] for tools that need equirectangular maps.
pub fn bake_image_ibl_in<'a>(
	source_extent: Extent,
	source_rgba16f: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<BakedImageIBL<'a>, IBLBakeError> {
	let layout = CubemapIBLLayout::new(source_extent, source_rgba16f)?;
	let (source_width, source_height) = layout.source_dimensions();
	let source = decode_source_radiance(source_rgba16f, allocator)?;
	let source_mips = build_source_mips(source_width, source_height, source, allocator)?;
	let mut data = layout.allocate_data(source_rgba16f, allocator)?;

	for (level, face_size) in layout.specular_face_sizes().into_iter().enumerate() {
		let range = layout.specular_range(level);
		let roughness = level as f32 / (IBL_PREFILTERED_SPECULAR_MIP_COUNT - 1) as f32;
		if level == 0 {
			resample_environment_cubemap(
				&source_mips[0].pixels,
				source_width,
				source_height,
				face_size,
				&mut data[range],
			);
		} else {
			prefilter_specular_cubemap(&source_mips, face_size, roughness, &mut data[range]);
		}
	}
	convolve_diffuse_irradiance_cubemap(&source_mips, DIFFUSE_CUBE_FACE_SIZE, &mut data[layout.diffuse_range()]);

	Ok(layout.finish(data))
}

/// Builds an area-preserving lat-long pyramid used to filter each Monte Carlo sample to its spherical footprint.
pub(super) fn build_source_mips<'a>(
	width: u32,
	height: u32,
	pixels: Vec<Radiance, &'a dyn Allocator>,
	allocator: &'a dyn Allocator,
) -> Result<Vec<SourceMIP<'a>, &'a dyn Allocator>, IBLBakeError> {
	let level_count = width.max(height).ilog2() as usize + 1;
	let mut mips = Vec::new_in(allocator);
	mips.try_reserve_exact(level_count)
		.map_err(|_| IBLBakeError::AllocationFailed)?;
	mips.push(SourceMIP { width, height, pixels });

	while mips.last().is_some_and(|level| level.width > 1 || level.height > 1) {
		let source = mips.last().expect("the source pyramid always contains its base level");
		let destination_width = (source.width / 2).max(1);
		let destination_height = (source.height / 2).max(1);
		let pixel_count = (destination_width as usize)
			.checked_mul(destination_height as usize)
			.ok_or(IBLBakeError::DimensionsTooLarge)?;
		let mut destination = Vec::new_in(allocator);
		destination
			.try_reserve_exact(pixel_count)
			.map_err(|_| IBLBakeError::AllocationFailed)?;

		for y in 0..destination_height {
			let source_y_begin = y as u64 * source.height as u64 / destination_height as u64;
			let source_y_end = ((y + 1) as u64 * source.height as u64 / destination_height as u64).max(source_y_begin + 1);
			for x in 0..destination_width {
				let source_x_begin = x as u64 * source.width as u64 / destination_width as u64;
				let source_x_end = ((x + 1) as u64 * source.width as u64 / destination_width as u64).max(source_x_begin + 1);
				let mut sum = [0.0_f64; 3];
				let mut total_weight = 0.0_f64;

				for source_y in source_y_begin..source_y_end {
					let weight = lat_long_row_solid_angle(source.width, source.height, source_y as u32) as f64;
					for source_x in source_x_begin..source_x_end {
						let radiance = source.pixels[source_y as usize * source.width as usize + source_x as usize];
						for channel in 0..3 {
							sum[channel] += radiance[channel] as f64 * weight;
						}
						total_weight += weight;
					}
				}

				destination.push([
					(sum[0] / total_weight) as f32,
					(sum[1] / total_weight) as f32,
					(sum[2] / total_weight) as f32,
				]);
			}
		}

		mips.push(SourceMIP {
			width: destination_width,
			height: destination_height,
			pixels: destination,
		});
	}

	Ok(mips)
}

fn image_byte_size(width: u32, height: u32) -> Result<usize, IBLBakeError> {
	(width as usize)
		.checked_mul(height as usize)
		.and_then(|pixels| pixels.checked_mul(BYTES_PER_RGBA16F_PIXEL))
		.ok_or(IBLBakeError::DimensionsTooLarge)
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

pub(super) fn decode_source_radiance<'a>(
	source: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<Vec<Radiance, &'a dyn Allocator>, IBLBakeError> {
	let pixel_count = source.len() / BYTES_PER_RGBA16F_PIXEL;
	let mut radiance = Vec::new_in(allocator);
	radiance
		.try_reserve_exact(pixel_count)
		.map_err(|_| IBLBakeError::AllocationFailed)?;

	for pixel in source.chunks_exact(BYTES_PER_RGBA16F_PIXEL) {
		radiance.push(decode_source_pixel(pixel));
	}

	Ok(radiance)
}

pub(super) fn decode_source_pixel(pixel: &[u8]) -> Radiance {
	[
		decode_finite_half(&pixel[0..2]),
		decode_finite_half(&pixel[2..4]),
		decode_finite_half(&pixel[4..6]),
	]
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

fn resample_environment_cubemap(
	source: &[Radiance],
	source_width: u32,
	source_height: u32,
	face_size: u32,
	destination: &mut [u8],
) {
	for face in 0..CUBE_FACE_COUNT as u32 {
		for y in 0..face_size {
			for x in 0..face_size {
				let direction = cubemap_texel_direction(face, x, y, face_size);
				let radiance = sample_direction(source, source_width, source_height, direction);
				let offset = (((face * face_size + y) * face_size + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
				write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
			}
		}
	}
}

/// Stores irradiance divided by pi, allowing Lambertian shading to multiply this map by albedo directly.
fn convolve_diffuse_irradiance(
	source_mips: &[SourceMIP<'_>],
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
				let pdf = local_direction[2] / PI;
				let radiance = sample_filtered_direction(source_mips, direction, pdf, DIFFUSE_SAMPLE_COUNT);
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

fn convolve_diffuse_irradiance_cubemap(source_mips: &[SourceMIP<'_>], face_size: u32, destination: &mut [u8]) {
	let samples = cosine_hemisphere_samples();
	for face in 0..CUBE_FACE_COUNT as u32 {
		for y in 0..face_size {
			for x in 0..face_size {
				let normal = cubemap_texel_direction(face, x, y, face_size);
				let (tangent, bitangent) = orthonormal_basis(normal);
				let mut sum = [0.0_f64; 3];
				for &local_direction in &samples {
					let direction = tangent_to_world(local_direction, tangent, bitangent, normal);
					let radiance =
						sample_filtered_direction(source_mips, direction, local_direction[2] / PI, DIFFUSE_SAMPLE_COUNT);
					for channel in 0..3 {
						sum[channel] += radiance[channel] as f64;
					}
				}
				let scale = 1.0 / DIFFUSE_SAMPLE_COUNT as f64;
				let offset = (((face * face_size + y) * face_size + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
				write_rgba16f(
					&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL],
					[(sum[0] * scale) as f32, (sum[1] * scale) as f32, (sum[2] * scale) as f32],
				);
			}
		}
	}
}

fn prefilter_specular_level(
	source_mips: &[SourceMIP<'_>],
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

				let normal_dot_half = dot(normal, half_vector).max(0.0);
				let pdf = ggx_light_pdf(normal_dot_half, view_dot_half, roughness);
				let radiance = sample_filtered_direction(source_mips, light, pdf, SPECULAR_SAMPLE_COUNT);
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
				sample_direction(&source_mips[0].pixels, source_mips[0].width, source_mips[0].height, normal)
			};
			let offset = ((y * destination_width + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
			write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
		}
	}
}

fn prefilter_specular_cubemap(source_mips: &[SourceMIP<'_>], face_size: u32, roughness: f32, destination: &mut [u8]) {
	let samples = ggx_half_vector_samples(roughness);
	for face in 0..CUBE_FACE_COUNT as u32 {
		for y in 0..face_size {
			for x in 0..face_size {
				let normal = cubemap_texel_direction(face, x, y, face_size);
				let (tangent, bitangent) = orthonormal_basis(normal);
				let mut sum = [0.0_f64; 3];
				let mut total_weight = 0.0_f64;
				for &local_half_vector in &samples {
					let half_vector = normalize(tangent_to_world(local_half_vector, tangent, bitangent, normal));
					let view_dot_half = dot(normal, half_vector).max(0.0);
					let light = normalize(sub(scale(half_vector, 2.0 * view_dot_half), normal));
					let normal_dot_light = dot(normal, light).max(0.0);
					if normal_dot_light <= 0.0 {
						continue;
					}
					let pdf = ggx_light_pdf(dot(normal, half_vector).max(0.0), view_dot_half, roughness);
					let radiance = sample_filtered_direction(source_mips, light, pdf, SPECULAR_SAMPLE_COUNT);
					for channel in 0..3 {
						sum[channel] += radiance[channel] as f64 * normal_dot_light as f64;
					}
					total_weight += normal_dot_light as f64;
				}
				let radiance = if total_weight > 0.0 {
					[
						(sum[0] / total_weight) as f32,
						(sum[1] / total_weight) as f32,
						(sum[2] / total_weight) as f32,
					]
				} else {
					sample_direction(&source_mips[0].pixels, source_mips[0].width, source_mips[0].height, normal)
				};
				let offset = (((face * face_size + y) * face_size + x) as usize) * BYTES_PER_RGBA16F_PIXEL;
				write_rgba16f(&mut destination[offset..offset + BYTES_PER_RGBA16F_PIXEL], radiance);
			}
		}
	}
}

/// Returns the GGX half-vector density transformed into a light-direction density.
fn ggx_light_pdf(normal_dot_half: f32, view_dot_half: f32, roughness: f32) -> f32 {
	let alpha = roughness * roughness;
	let alpha_squared = alpha * alpha;
	let denominator_term = normal_dot_half * normal_dot_half * (alpha_squared - 1.0) + 1.0;
	let distribution = alpha_squared / (PI * denominator_term * denominator_term).max(f32::MIN_POSITIVE);
	(distribution * normal_dot_half / (4.0 * view_dot_half).max(f32::MIN_POSITIVE)).max(f32::MIN_POSITIVE)
}

/// Filters a directional sample according to the solid angle represented by its Monte Carlo PDF.
fn sample_filtered_direction(source_mips: &[SourceMIP<'_>], direction: Vector3, pdf: f32, sample_count: usize) -> Radiance {
	let base = &source_mips[0];
	let sample_solid_angle = 1.0 / (sample_count as f32 * pdf.max(f32::MIN_POSITIVE));
	let texel_solid_angle = direction_texel_solid_angle(base.width, base.height, direction);
	let lod = (0.5 * (sample_solid_angle / texel_solid_angle).max(1.0).log2()).clamp(0.0, (source_mips.len() - 1) as f32);
	let lower_level = lod.floor() as usize;
	let upper_level = (lower_level + 1).min(source_mips.len() - 1);
	let blend = lod - lower_level as f32;
	let lower = sample_mip_direction(&source_mips[lower_level], direction);
	let upper = sample_mip_direction(&source_mips[upper_level], direction);
	lerp_radiance(lower, upper, blend)
}

fn sample_mip_direction(mip: &SourceMIP<'_>, direction: Vector3) -> Radiance {
	sample_direction(&mip.pixels, mip.width, mip.height, direction)
}

/// Returns the exact spherical area of the base lat-long texel containing a direction.
fn direction_texel_solid_angle(width: u32, height: u32, direction: Vector3) -> f32 {
	let direction = normalize(direction);
	let v = 0.5 - direction[1].clamp(-1.0, 1.0).asin() / PI;
	let row = (v * height as f32).floor().clamp(0.0, height.saturating_sub(1) as f32) as u32;
	lat_long_row_solid_angle(width, height, row)
}

/// Returns one texel's solid angle for a row of an equirectangular image.
pub(super) fn lat_long_row_solid_angle(width: u32, height: u32, row: u32) -> f32 {
	let latitude_top = PI * (0.5 - row as f32 / height as f32);
	let latitude_bottom = PI * (0.5 - (row + 1) as f32 / height as f32);
	(TAU / width as f32) * (latitude_top.sin() - latitude_bottom.sin())
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

/// Maps the API-standard +X, -X, +Y, -Y, +Z, -Z face order to a world direction.
fn cubemap_texel_direction(face: u32, x: u32, y: u32, face_size: u32) -> Vector3 {
	let u = 2.0 * (x as f32 + 0.5) / face_size as f32 - 1.0;
	let v = 2.0 * (y as f32 + 0.5) / face_size as f32 - 1.0;
	normalize(match face {
		0 => [1.0, -v, -u],
		1 => [-1.0, -v, u],
		2 => [u, 1.0, v],
		3 => [u, -1.0, -v],
		4 => [u, -v, 1.0],
		5 => [-u, -v, -1.0],
		_ => unreachable!("cubemap face index is always below six"),
	})
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

pub(super) fn write_rgba16f(destination: &mut [u8], radiance: Radiance) {
	for (channel, value) in radiance.into_iter().enumerate() {
		let value = if value.is_finite() { value } else { 0.0 };
		destination[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
	}
	destination[6..8].copy_from_slice(&f16::from_f32(1.0).to_le_bytes());
}

#[cfg(test)]
mod tests {
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

	fn allocator_pixels(pixels: Vec<Radiance>) -> Vec<Radiance, &'static dyn Allocator> {
		let allocator: &'static dyn Allocator = &Global;
		let mut allocated = Vec::new_in(allocator);
		allocated.extend(pixels);
		allocated
	}

	/// Estimates the same split-sum prefilter integral with a configurable sample count for quality comparisons.
	fn estimate_specular(
		source_mips: &[super::SourceMIP<'_>],
		normal: [f32; 3],
		roughness: f32,
		sample_count: usize,
		filtered: bool,
	) -> Radiance {
		let view = normal;
		let (tangent, bitangent) = orthonormal_basis(normal);
		let mut sum = [0.0_f64; 3];
		let mut total_weight = 0.0_f64;
		let alpha = roughness * roughness;
		let alpha_squared = alpha * alpha;

		for index in 0..sample_count {
			let [angular_sample, elevation_sample] = hammersley(index, sample_count);
			let angle = std::f32::consts::TAU * angular_sample;
			let cos_theta = ((1.0 - elevation_sample) / (1.0 + (alpha_squared - 1.0) * elevation_sample))
				.max(0.0)
				.sqrt();
			let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
			let (sin_angle, cos_angle) = angle.sin_cos();
			let local_half = [sin_theta * cos_angle, sin_theta * sin_angle, cos_theta];
			let half = normalize(tangent_to_world(local_half, tangent, bitangent, normal));
			let view_dot_half = dot(view, half).max(0.0);
			let light = normalize(sub(scale(half, 2.0 * view_dot_half), view));
			let normal_dot_light = dot(normal, light).max(0.0);
			if normal_dot_light <= 0.0 {
				continue;
			}

			let radiance = if filtered {
				let pdf = ggx_light_pdf(dot(normal, half).max(0.0), view_dot_half, roughness);
				sample_filtered_direction(source_mips, light, pdf, sample_count)
			} else {
				sample_direction(&source_mips[0].pixels, source_mips[0].width, source_mips[0].height, light)
			};
			for channel in 0..3 {
				sum[channel] += radiance[channel] as f64 * normal_dot_light as f64;
			}
			total_weight += normal_dot_light as f64;
		}

		[
			(sum[0] / total_weight) as f32,
			(sum[1] / total_weight) as f32,
			(sum[2] / total_weight) as f32,
		]
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
		assert_eq!(
			first.ibl.diffuse_irradiance.extent,
			[DIFFUSE_CUBE_FACE_SIZE, DIFFUSE_CUBE_FACE_SIZE, 1]
		);
		assert_eq!(first.ibl.diffuse_irradiance.array_layers, 6);
		assert_eq!(first.ibl.prefiltered_specular.extent, [1, 1, 1]);
		assert_eq!(first.ibl.prefiltered_specular.array_layers, 6);
		assert_eq!(first.ibl.prefiltered_specular.mip_count, IBL_PREFILTERED_SPECULAR_MIP_COUNT);
		assert_eq!(first.streams.len(), IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize + 2);

		let root = &first.streams[0];
		let specular_zero = &first.streams[1];

		assert_eq!(root.name(), IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!(root.offset(), 0);
		assert_eq!(root.size(), 4 * 2 * BYTES_PER_RGBA16F_PIXEL);
		assert_eq!(specular_zero.name(), ibl_prefiltered_specular_stream_name(0));
		assert_eq!(specular_zero.offset(), root.size());
		assert_eq!(specular_zero.size(), CUBE_FACE_COUNT * BYTES_PER_RGBA16F_PIXEL);
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
		let mut expected_face_size = 1_u32;
		for (level, stream) in first.streams[1..1 + IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize]
			.iter()
			.enumerate()
		{
			assert_eq!(stream.name(), ibl_prefiltered_specular_stream_name(level as u32));
			assert_eq!(stream.offset(), expected_offset);
			assert_eq!(
				stream.size(),
				CUBE_FACE_COUNT * expected_face_size as usize * expected_face_size as usize * BYTES_PER_RGBA16F_PIXEL
			);
			expected_offset += stream.size();
			expected_face_size = (expected_face_size / 2).max(1);
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
		assert_eq!(baked.ibl.prefiltered_specular.extent, [1, 1, 1]);
		assert_eq!(root.size(), source.len());
		assert_eq!(&baked.data[root.offset()..root.offset() + root.size()], source.as_slice());
		assert_eq!(specular_zero.size(), CUBE_FACE_COUNT * BYTES_PER_RGBA16F_PIXEL);
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
	fn cubemap_face_centers_follow_the_native_api_face_order() {
		let expected = [
			[1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, -1.0, 0.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, -1.0],
		];
		for (face, expected) in expected.into_iter().enumerate() {
			assert_eq!(super::cubemap_texel_direction(face as u32, 0, 0, 1), expected);
		}
	}

	#[test]
	fn source_pyramid_preserves_spherical_energy_across_latitudes() {
		let width = 8;
		let height = 4;
		let pixels = (0..height)
			.flat_map(|y| (0..width).map(move |_| [1.0 + y as f32 * 3.0, 0.0, 0.0]))
			.collect::<Vec<_>>();
		let expected = (0..height)
			.map(|y| (1.0 + y as f32 * 3.0) * lat_long_row_solid_angle(width, height, y) * width as f32)
			.sum::<f32>()
			/ (4.0 * std::f32::consts::PI);
		let mips = build_source_mips(width, height, allocator_pixels(pixels), &Global).unwrap();
		let last = mips.last().unwrap();

		assert_eq!([last.width, last.height], [1, 1]);
		assert!((last.pixels[0][0] - expected).abs() < 1.0e-5);
	}

	#[test]
	fn filtered_importance_sampling_improves_bright_emitter_accuracy() {
		let width = 64;
		let height = 32;
		let pixels = (0..height)
			.flat_map(|y| {
				(0..width).map(move |x| {
					if (31..=32).contains(&x) && (15..=16).contains(&y) {
						[1000.0, 1000.0, 1000.0]
					} else {
						[0.1, 0.1, 0.1]
					}
				})
			})
			.collect::<Vec<_>>();
		let mips = build_source_mips(width, height, allocator_pixels(pixels), &Global).unwrap();
		let normal = [1.0, 0.0, 0.0];
		let roughness = 0.45;
		let reference = estimate_specular(&mips, normal, roughness, 65_536, false)[0];
		let old = estimate_specular(&mips, normal, roughness, 128, false)[0];
		let filtered = estimate_specular(&mips, normal, roughness, 1024, true)[0];
		let old_error = (old - reference).abs();
		let filtered_error = (filtered - reference).abs();

		assert!(
			filtered_error < old_error * 0.5,
			"filtered error {} must be lower than mip-zero error {}; reference={reference}, filtered={filtered}, old={old}",
			filtered_error,
			old_error,
		);
	}

	#[test]
	fn malformed_source_layout_is_rejected_before_allocation() {
		assert_eq!(
			bake_image_ibl_in(Extent::rectangle(0, 2), &[], &Global).err(),
			Some(IBLBakeError::ZeroDimensions)
		);
		assert_eq!(
			bake_image_ibl_in(Extent::rectangle(2, 1), &[0; 8], &Global).err(),
			Some(IBLBakeError::BufferSizeMismatch { expected: 16, got: 8 })
		);
	}

	use std::alloc::{Allocator, Global};

	use exr::prelude::f16;
	use utils::Extent;

	use super::{
		bake_image_ibl_in, build_source_mips, dot, ggx_light_pdf, hammersley, image_byte_size, lat_long_row_solid_angle,
		normalize, orthonormal_basis, sample_direction, sample_filtered_direction, sample_lat_long_uv, scale, sub,
		tangent_to_world, IBLBakeError, Radiance, BYTES_PER_RGBA16F_PIXEL, CUBE_FACE_COUNT, DIFFUSE_CUBE_FACE_SIZE,
		DIFFUSE_HEIGHT, DIFFUSE_WIDTH,
	};
	use crate::resources::image::{
		ibl_prefiltered_specular_stream_name, IBL_DIFFUSE_IRRADIANCE_STREAM_NAME, IBL_PREFILTERED_SPECULAR_MIP_COUNT,
		IMAGE_BASE_MIP_STREAM_NAME,
	};
}

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
		ibl_prefiltered_specular_stream_name, ImageIBL, ImageSubresource, IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
		IBL_PREFILTERED_SPECULAR_MIP_COUNT, IMAGE_BASE_MIP_STREAM_NAME,
	},
	types::{Formats, Gamma},
	StreamDescription,
};
