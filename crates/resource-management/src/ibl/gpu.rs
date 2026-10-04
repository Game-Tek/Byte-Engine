const GPU_ATLAS_MAX_DIMENSION: u32 = 8192;

/// The `GPUIBLBakeError` enum identifies why environment-map generation could not use the GPU path.
#[derive(Debug)]
pub enum GPUIBLBakeError {
	InvalidInput(IBLBakeError),
	Worker(GpuWorkerError),
	AtlasTooLarge { width: u32, height: u32 },
	AtlasLayoutOverflow,
	SourceUploadSizeMismatch { expected: usize, got: usize },
	OutputReadbackSizeMismatch { expected: usize, got: usize },
	GPUExecution,
}

impl fmt::Display for GPUIBLBakeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidInput(error) => error.fmt(formatter),
			Self::Worker(error) => error.describe("environment-map", formatter),
			Self::AtlasTooLarge { width, height } => write!(
				formatter,
				"GPU environment-map atlas is too large ({width}x{height}). The most likely cause is a source image that exceeds the portable 8192-pixel atlas limit."
			),
			Self::AtlasLayoutOverflow => formatter.write_str(
				"GPU environment-map atlas layout overflowed. The most likely cause is an environment image with unsupported dimensions.",
			),
			Self::SourceUploadSizeMismatch { expected, got } => write!(
				formatter,
				"GPU environment-map source upload has the wrong size: expected {expected}, got {got}. The most likely cause is a GHI staging allocation that does not match the source atlas."
			),
			Self::OutputReadbackSizeMismatch { expected, got } => write!(
				formatter,
				"GPU environment-map readback has the wrong size: expected at least {expected}, got {got}. The most likely cause is incomplete GHI texture readback."
			),
			Self::GPUExecution => formatter.write_str(
				"GPU environment-map generation failed. The most likely cause is a graphics backend validation or command-execution error.",
			),
		}
	}
}

impl Error for GPUIBLBakeError {}

impl From<IBLBakeError> for GPUIBLBakeError {
	fn from(error: IBLBakeError) -> Self {
		Self::InvalidInput(error)
	}
}

impl From<GpuWorkerError> for GPUIBLBakeError {
	fn from(error: GpuWorkerError) -> Self {
		Self::Worker(error)
	}
}

/// The `OwnedBakedImageIBL` struct carries GPU-generated environment maps from the dedicated worker to asset storage.
pub struct OwnedBakedImageIBL {
	pub root_extent: [u32; 3],
	pub ibl: crate::resources::image::ImageIBL,
	pub streams: Vec<crate::StreamDescription>,
	pub data: Box<[u8]>,
}

/// The `GPUIBLClient` struct queues environment-map requests onto a dedicated GHI context thread.
///
/// Install this client on an environment-map asset handler. The handler can then run on the asset manager's shared worker pool
/// without moving or concurrently accessing the backend context, and it awaits each bake instead of blocking a pool thread.
pub struct GPUIBLClient {
	worker: GpuWorker<GPUIBLProcessor>,
}

impl GPUIBLClient {
	/// Creates a dedicated worker with its own compute device and context.
	pub fn try_new() -> Result<Self, GPUIBLBakeError> {
		Self::from_processor_factory(GPUIBLProcessor::try_new)
	}

	/// Runs a processor factory on the dedicated GPU thread before accepting requests.
	///
	/// Create every thread-affine GHI device and context inside `initialize`. The factory itself must be safe to move,
	/// but the processor it returns remains on the worker for its entire lifetime.
	pub fn from_processor_factory(
		initialize: impl FnOnce() -> Result<GPUIBLProcessor, GPUIBLBakeError> + Send + 'static,
	) -> Result<Self, GPUIBLBakeError> {
		Ok(Self {
			worker: GpuWorker::spawn("GPU Environment Map Worker", initialize)?,
		})
	}

	/// Submits one source image and resolves once the GPU result is safe to consume.
	///
	/// The worker owns a copy of the source while the bake is in flight. The baked maps come back in the result.
	pub async fn bake_image_ibl(
		&self,
		source_extent: Extent,
		source_rgba16f: &[u8],
	) -> Result<OwnedBakedImageIBL, GPUIBLBakeError> {
		self.worker
			.submit(source_extent, source_rgba16f.to_vec())
			.await
			.map_err(|_| GpuWorkerError::Unavailable)?
	}

	/// Creates a client whose worker already stopped, so every bake reports it as unavailable.
	#[cfg(test)]
	pub(crate) fn unavailable_for_test() -> Self {
		Self {
			worker: GpuWorker::unavailable(),
		}
	}
}

/// The `GPUIBLProcessor` struct provides thread-confined environment-map generation with CPU-compatible output.
///
/// It serves the worker one bake at a time: [`GpuProcessor::submit`] runs the whole bake and completes inline.
/// Environment maps are rare next to material textures, so they don't pipeline yet.
///
/// Create and use this processor on one thread. To use it from an asset handler, construct it inside the factory passed to
/// [`crate::ibl::IBLGenerator::with_gpu_processor_factory`].
pub struct GPUIBLProcessor {
	// Drop the context before its owner guard. `dyn Any` also keeps this thread-confined processor explicitly non-Send.
	context: ghi::implementation::Context,
	pipeline: ghi::PipelineHandle,
	queue: ghi::QueueHandle,
	source_sampler: ghi::SamplerHandle,
	scratch: Vec<GPUIBLScratch>,
	// Retain the largest lower level and downsample it in place instead of allocating a source pyramid for every bake.
	source_mip: Vec<Radiance>,
	_context_owner: Box<dyn Any>,
}

impl GPUIBLProcessor {
	/// Creates a self-contained compute device and context for offline asset baking.
	///
	/// Call this constructor inside [`crate::ibl::IBLGenerator::with_gpu_processor_factory`] when an asset handler owns the
	/// generation path.
	pub fn try_new() -> Result<Self, GPUIBLBakeError> {
		let (context, queue, owner) = create_compute_context()?;
		Self::from_context(context, queue, owner)
	}

	/// Uses a caller-created auxiliary context for environment-map generation.
	///
	/// `owner` keeps the device, instance, or other native state alive until after the context is dropped. Create all three
	/// values on the current thread, then continue using the processor on this thread or return it from a worker-local factory.
	/// The shared compute pipeline is created here, before the handler begins processing concurrent assets.
	pub fn from_context<Owner: 'static>(
		context: ghi::implementation::Context,
		queue: ghi::QueueHandle,
		owner: Owner,
	) -> Result<Self, GPUIBLBakeError> {
		// Keep native owners alive after the context on every early-return and unwinding path.
		let mut construction = OwnedContext {
			context,
			owner: Box::new(owner),
		};
		let context = &mut construction.context;
		let pipeline = create_compute_kernel(
			context,
			"GPU environment-map generation",
			ghi::shader::ShaderSource::PlatformNative {
				glsl: GPU_IBL_GLSL,
				msl: GPU_IBL_MSL,
				msl_entry_point: "generate_environment_map",
				hlsl: GPU_IBL_HLSL,
				hlsl_entry_point: "generate_environment_map",
			},
			std::mem::size_of::<GPUIBLPushConstants>(),
		)?;
		let source_sampler = context.build_sampler(ghi::sampler::Builder::new().max_lod(0.0));

		let OwnedContext { context, owner } = construction;
		Ok(Self {
			context,
			pipeline,
			queue,
			source_sampler,
			scratch: Vec::with_capacity(2),
			source_mip: Vec::new(),
			_context_owner: owner,
		})
	}

	/// Generates cubemap IBL streams and repacks them into the CPU processor's stable resource layout.
	pub fn bake_image_ibl(
		&mut self,
		source_extent: Extent,
		source_rgba16f: &[u8],
	) -> Result<OwnedBakedImageIBL, GPUIBLBakeError> {
		let layout = CubemapIBLLayout::new(source_extent, source_rgba16f)?;
		let (source_width, source_height) = layout.source_dimensions();
		let (source_atlas_extent, source_level_count) = source_atlas_layout(source_width, source_height)?;
		let output_atlas_extent = output_atlas_extent(layout)?;
		validate_atlas_extent(source_atlas_extent)?;
		validate_atlas_extent(output_atlas_extent)?;

		let key = GPUIBLScratchKey {
			source_atlas_extent,
			output_atlas_extent,
		};
		let scratch = if let Some(scratch) = self.scratch.iter().find(|scratch| scratch.key == key).copied() {
			scratch
		} else {
			let scratch = self.create_scratch(key);
			self.scratch.push(scratch);
			scratch
		};

		let upload = self.context.get_texture_slice_mut(scratch.source_atlas);
		let expected_upload_size = atlas_byte_size(source_atlas_extent)?;
		if upload.len() != expected_upload_size {
			return Err(GPUIBLBakeError::SourceUploadSizeMismatch {
				expected: expected_upload_size,
				got: upload.len(),
			});
		}
		write_source_atlas(source_width, source_height, source_rgba16f, upload, &mut self.source_mip)?;
		self.context.sync_texture(scratch.source_atlas);

		let copy_handle = self.dispatch(layout, source_level_count, scratch)?;
		self.context.wait_for_synchronizer(scratch.synchronizer);
		#[cfg(debug_assertions)]
		if self.context.has_errors() {
			return Err(GPUIBLBakeError::GPUExecution);
		}

		let expected_readback_size = atlas_byte_size(output_atlas_extent)?;
		let readback = self
			.context
			.get_image_data(copy_handle)
			.map_err(|_| GPUIBLBakeError::GPUExecution)?;
		if readback.bytes.len() < expected_readback_size {
			return Err(GPUIBLBakeError::OutputReadbackSizeMismatch {
				expected: expected_readback_size,
				got: readback.bytes.len(),
			});
		}

		let mut data = Vec::new();
		data.try_reserve_exact(layout.total_size())
			.map_err(|_| IBLBakeError::AllocationFailed)?;
		data.resize(layout.total_size(), 0);
		data[..layout.root_size()].copy_from_slice(source_rgba16f);
		copy_output_atlas(layout, &readback.bytes, output_atlas_extent.width(), &mut data);
		let (root_extent, ibl, streams) = layout.metadata();
		Ok(OwnedBakedImageIBL {
			root_extent,
			ibl,
			streams,
			data: data.into_boxed_slice(),
		})
	}

	/// Allocates one reusable source/output atlas pair for a dimension combination.
	fn create_scratch(&mut self, key: GPUIBLScratchKey) -> GPUIBLScratch {
		let source_atlas = self.context.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image)
				.name("Environment source mip atlas")
				.extent(key.source_atlas_extent)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.use_case(ghi::UseCases::STATIC),
		);
		let output_atlas = self.context.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage | ghi::Uses::TransferSource)
				.name("Environment cubemap output atlas")
				.extent(key.output_atlas_extent)
				.device_accesses(ghi::DeviceAccesses::DeviceToHost)
				.use_case(ghi::UseCases::STATIC),
		);
		let descriptor_set = self.context.create_descriptor_set(Some("Environment-map atlases"));
		self.context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				SOURCE_SLOT,
				source_atlas,
				self.source_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_SLOT, output_atlas, ghi::Layouts::General),
		]);
		let command_buffer = self
			.context
			.queue(self.queue)
			.create_command_buffer(Some("Generate environment maps"));
		let synchronizer = self.context.create_synchronizer(Some("Environment maps generated"), true);

		GPUIBLScratch {
			key,
			source_atlas,
			output_atlas,
			descriptor_set,
			command_buffer,
			synchronizer,
		}
	}

	/// Records every roughness level and the diffuse map before one compact atlas readback.
	fn dispatch(
		&mut self,
		layout: CubemapIBLLayout,
		source_level_count: u32,
		scratch: GPUIBLScratch,
	) -> Result<ghi::TextureCopyHandle, GPUIBLBakeError> {
		let (source_width, source_height) = layout.source_dimensions();
		let source_level_y_offsets = source_level_y_offsets(source_height);
		let source_row_angle_step = std::f32::consts::PI / source_height as f32;
		let source_solid_angle_scale =
			(std::f32::consts::TAU / source_width as f32) * 2.0 * (std::f32::consts::PI / (2.0 * source_height as f32)).sin();
		let mut command_buffer = self.context.command_buffer(scratch.command_buffer);
		let mut recording = command_buffer.create_command_buffer_recording();
		{
			let command = recording.bind_compute_pipeline(self.pipeline);
			command.bind_descriptor_sets(&[scratch.descriptor_set]);
			let mut output_y_offset = 0;
			for (level, face_size) in layout.specular_face_sizes().into_iter().enumerate() {
				let push_constants = GPUIBLPushConstants {
					source_width,
					source_height,
					source_level_count,
					output_face_size: face_size,
					output_y_offset,
					mode: (level != 0) as u32,
					roughness: level as f32 / (IBL_PREFILTERED_SPECULAR_MIP_COUNT - 1) as f32,
					source_row_angle_step,
					source_solid_angle_scale,
					_padding: [0; 3],
					source_level_y_offsets,
				};
				command.write_push_constant(0, push_constants);
				command.dispatch(ghi::DispatchExtent::new(
					Extent::new(face_size * face_size * CUBE_FACE_COUNT as u32, 1, 1),
					Extent::new(64, 1, 1),
				));
				output_y_offset += face_size * CUBE_FACE_COUNT as u32;
			}

			let push_constants = GPUIBLPushConstants {
				source_width,
				source_height,
				source_level_count,
				output_face_size: DIFFUSE_CUBE_FACE_SIZE,
				output_y_offset,
				mode: 2,
				roughness: 1.0,
				source_row_angle_step,
				source_solid_angle_scale,
				_padding: [0; 3],
				source_level_y_offsets,
			};
			command.write_push_constant(0, push_constants);
			command.dispatch(ghi::DispatchExtent::new(
				Extent::new(DIFFUSE_CUBE_FACE_SIZE * DIFFUSE_CUBE_FACE_SIZE * CUBE_FACE_COUNT as u32, 1, 1),
				Extent::new(64, 1, 1),
			));
		}

		let copy_handle = recording
			.transfer_texture(scratch.output_atlas.into())
			.map_err(|_| GPUIBLBakeError::GPUExecution)?;
		recording.execute(scratch.synchronizer);
		Ok(copy_handle)
	}
}

impl GpuProcessor for GPUIBLProcessor {
	type Request = Extent;
	type Result = Result<OwnedBakedImageIBL, GPUIBLBakeError>;
	/// No request is ever in flight, which the type states so `poll` needs no body.
	type Ticket = std::convert::Infallible;

	const MAX_IN_FLIGHT: usize = 1;

	fn submit(&mut self, source_extent: &Extent, source_rgba16f: &[u8]) -> Submission<Self::Ticket, Self::Result> {
		Submission::Complete(self.bake_image_ibl(*source_extent, source_rgba16f))
	}

	fn poll(&mut self, ticket: Self::Ticket) -> Option<Self::Result> {
		match ticket {}
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GPUIBLPushConstants {
	source_width: u32,
	source_height: u32,
	source_level_count: u32,
	output_face_size: u32,
	output_y_offset: u32,
	mode: u32,
	roughness: f32,
	source_row_angle_step: f32,
	source_solid_angle_scale: f32,
	_padding: [u32; 3],
	source_level_y_offsets: [[u32; 4]; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GPUIBLScratchKey {
	source_atlas_extent: Extent,
	output_atlas_extent: Extent,
}

#[derive(Clone, Copy)]
struct GPUIBLScratch {
	key: GPUIBLScratchKey,
	source_atlas: ghi::ImageHandle,
	output_atlas: ghi::ImageHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
}

/// Computes every packed source-level row offset once so shaders avoid scanning the MIP chain for each sample.
fn source_level_y_offsets(source_height: u32) -> [[u32; 4]; 4] {
	let mut offsets = [[0; 4]; 4];
	let mut offset = 0;
	let mut height = source_height;
	for level in 0..16 {
		offsets[level / 4][level % 4] = offset;
		offset += height;
		height = (height / 2).max(1);
	}
	offsets
}

/// Computes the vertical source-mip atlas without allocating level descriptors.
fn source_atlas_layout(width: u32, height: u32) -> Result<(Extent, u32), GPUIBLBakeError> {
	let mut atlas_height = 0_u32;
	let mut level_count = 0_u32;
	// Levels stack vertically in the full-width atlas.
	for (_, level_height) in mip_extents(width, height) {
		atlas_height = atlas_height
			.checked_add(level_height)
			.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)?;
		level_count += 1;
	}
	Ok((Extent::rectangle(width, atlas_height), level_count))
}

/// Streams sanitized source pixels and filtered lower levels directly into GHI texture staging.
fn write_source_atlas(
	source_width: u32,
	source_height: u32,
	source_rgba16f: &[u8],
	atlas: &mut [u8],
	source_mip: &mut Vec<Radiance>,
) -> Result<(), GPUIBLBakeError> {
	source_mip.clear();

	// The root level spans the full atlas width. Decode while copying so non-finite source values remain sanitized.
	for (source, destination) in source_rgba16f.as_chunks::<BYTES_PER_RGBA16F_PIXEL>().0.iter().zip(
		atlas[..source_rgba16f.len()]
			.as_chunks_mut::<BYTES_PER_RGBA16F_PIXEL>()
			.0
			.iter_mut(),
	) {
		write_rgba16f(destination, decode_source_pixel(source));
	}

	if source_width == 1 && source_height == 1 {
		return Ok(());
	}

	let (mut mip_width, mut mip_height) = generate_source_mip(source_width, source_height, source_mip, |index| {
		let offset = index * BYTES_PER_RGBA16F_PIXEL;
		decode_source_pixel(&source_rgba16f[offset..offset + BYTES_PER_RGBA16F_PIXEL])
	})?;
	let mut level_y_offset = source_height;
	write_source_level(atlas, source_width, level_y_offset, mip_width, mip_height, source_mip);
	level_y_offset += mip_height;

	while mip_width > 1 || mip_height > 1 {
		(mip_width, mip_height) = downsample_source_mip_in_place(mip_width, mip_height, source_mip)?;
		write_source_level(atlas, source_width, level_y_offset, mip_width, mip_height, source_mip);
		level_y_offset += mip_height;
	}

	debug_assert_eq!(
		level_y_offset,
		atlas.len() as u32 / source_width / BYTES_PER_RGBA16F_PIXEL as u32
	);
	Ok(())
}

/// Generates one solid-angle-filtered level while preserving the CPU pyramid's accumulation order and precision.
fn generate_source_mip(
	source_width: u32,
	source_height: u32,
	destination: &mut Vec<Radiance>,
	mut source_pixel: impl FnMut(usize) -> Radiance,
) -> Result<(u32, u32), GPUIBLBakeError> {
	let destination_width = (source_width / 2).max(1);
	let destination_height = (source_height / 2).max(1);
	let pixel_count = (destination_width as usize)
		.checked_mul(destination_height as usize)
		.ok_or(IBLBakeError::DimensionsTooLarge)?;
	destination.clear();
	destination
		.try_reserve_exact(pixel_count)
		.map_err(|_| IBLBakeError::AllocationFailed)?;

	for y in 0..destination_height {
		for x in 0..destination_width {
			destination.push(downsample_source_pixel(
				source_width,
				source_height,
				[x, y],
				[destination_width, destination_height],
				&mut source_pixel,
			));
		}
	}

	Ok((destination_width, destination_height))
}

/// Reuses one level's allocation for its child after each source region has been consumed.
fn downsample_source_mip_in_place(
	source_width: u32,
	source_height: u32,
	pixels: &mut Vec<Radiance>,
) -> Result<(u32, u32), GPUIBLBakeError> {
	let destination_width = (source_width / 2).max(1);
	let destination_height = (source_height / 2).max(1);
	let pixel_count = (destination_width as usize)
		.checked_mul(destination_height as usize)
		.ok_or(IBLBakeError::DimensionsTooLarge)?;
	debug_assert_eq!(pixels.len(), source_width as usize * source_height as usize);

	for y in 0..destination_height {
		for x in 0..destination_width {
			let destination_index = y as usize * destination_width as usize + x as usize;
			// Each filtered region starts at or after its destination index. Write only after reading the complete region so
			// this compacting pass cannot replace a texel needed by a later destination.
			let radiance = downsample_source_pixel(
				source_width,
				source_height,
				[x, y],
				[destination_width, destination_height],
				|source_index| pixels[source_index],
			);
			pixels[destination_index] = radiance;
		}
	}
	pixels.truncate(pixel_count);

	Ok((destination_width, destination_height))
}

/// Writes one compact mip into its rows of the full-width source atlas.
fn write_source_level(
	atlas: &mut [u8],
	atlas_width: u32,
	level_y_offset: u32,
	level_width: u32,
	level_height: u32,
	pixels: &[Radiance],
) {
	debug_assert_eq!(pixels.len(), level_width as usize * level_height as usize);
	for y in 0..level_height as usize {
		let source_start = y * level_width as usize;
		let destination_start = ((level_y_offset as usize + y) * atlas_width as usize) * BYTES_PER_RGBA16F_PIXEL;
		let destination_end = destination_start + level_width as usize * BYTES_PER_RGBA16F_PIXEL;
		for (radiance, destination) in pixels[source_start..source_start + level_width as usize].iter().zip(
			atlas[destination_start..destination_end]
				.as_chunks_mut::<BYTES_PER_RGBA16F_PIXEL>()
				.0
				.iter_mut(),
		) {
			write_rgba16f(destination, *radiance);
		}
	}
}

/// Computes the fixed vertical regions used by all specular levels followed by diffuse irradiance.
fn output_atlas_extent(layout: CubemapIBLLayout) -> Result<Extent, GPUIBLBakeError> {
	let width = layout.specular_face_size().max(DIFFUSE_CUBE_FACE_SIZE);
	let specular_height = layout
		.specular_face_sizes()
		.into_iter()
		.try_fold(0_u32, |height, face_size| {
			height
				.checked_add(
					face_size
						.checked_mul(CUBE_FACE_COUNT as u32)
						.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)?,
				)
				.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)
		})?;
	let diffuse_height = DIFFUSE_CUBE_FACE_SIZE
		.checked_mul(CUBE_FACE_COUNT as u32)
		.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)?;
	let height = specular_height
		.checked_add(diffuse_height)
		.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)?;
	Ok(Extent::rectangle(width, height))
}

fn validate_atlas_extent(extent: Extent) -> Result<(), GPUIBLBakeError> {
	if extent.width() > GPU_ATLAS_MAX_DIMENSION || extent.height() > GPU_ATLAS_MAX_DIMENSION {
		return Err(GPUIBLBakeError::AtlasTooLarge {
			width: extent.width(),
			height: extent.height(),
		});
	}
	Ok(())
}

fn atlas_byte_size(extent: Extent) -> Result<usize, GPUIBLBakeError> {
	(extent.width() as usize)
		.checked_mul(extent.height() as usize)
		.and_then(|pixel_count| pixel_count.checked_mul(BYTES_PER_RGBA16F_PIXEL))
		.ok_or(GPUIBLBakeError::AtlasLayoutOverflow)
}

/// Removes unused atlas columns while retaining mip-major, face-major stream order.
fn copy_output_atlas(layout: CubemapIBLLayout, atlas: &[u8], atlas_width: u32, destination: &mut [u8]) {
	let mut atlas_y_offset = 0_u32;
	for (level, face_size) in layout.specular_face_sizes().into_iter().enumerate() {
		copy_output_region(
			atlas,
			atlas_width,
			atlas_y_offset,
			face_size,
			&mut destination[layout.specular_range(level)],
		);
		atlas_y_offset += face_size * CUBE_FACE_COUNT as u32;
	}
	copy_output_region(
		atlas,
		atlas_width,
		atlas_y_offset,
		DIFFUSE_CUBE_FACE_SIZE,
		&mut destination[layout.diffuse_range()],
	);
}

fn copy_output_region(atlas: &[u8], atlas_width: u32, atlas_y_offset: u32, face_size: u32, destination: &mut [u8]) {
	let compact_row_size = face_size as usize * BYTES_PER_RGBA16F_PIXEL;
	for row in 0..face_size as usize * CUBE_FACE_COUNT {
		let source_start = ((atlas_y_offset as usize + row) * atlas_width as usize) * BYTES_PER_RGBA16F_PIXEL;
		let destination_start = row * compact_row_size;
		destination[destination_start..destination_start + compact_row_size]
			.copy_from_slice(&atlas[source_start..source_start + compact_row_size]);
	}
}

/// `GPUIBLPushConstants` must keep the size every native IBL shader declares.
///
/// The prefilter, irradiance, and BRDF shaders all bind this one push-constant block, so its size is a
/// shared ABI rather than an internal detail.
///
/// This is a compile-time check so a layout change fails the build at the definition rather than later
/// in a shader that silently reads the wrong bytes.
const _: () = assert!(std::mem::size_of::<GPUIBLPushConstants>() == 112);

#[cfg(test)]
mod tests {
	#[test]
	fn gpu_processor_factory_runs_on_the_context_owning_worker() {
		let caller = std::thread::current().id();
		let ran_on_worker = Arc::new(AtomicBool::new(false));
		let worker_result = ran_on_worker.clone();

		let result = GPUIBLClient::from_processor_factory(move || {
			worker_result.store(std::thread::current().id() != caller, Ordering::SeqCst);
			Err(GPUIBLBakeError::Worker(GpuWorkerError::Unavailable))
		});

		assert!(matches!(result, Err(GPUIBLBakeError::Worker(GpuWorkerError::Unavailable))));
		assert!(ran_on_worker.load(Ordering::SeqCst));
	}

	#[test]
	fn source_mips_stream_into_staging_without_changing_filter_results() {
		let (width, height) = (5, 3);
		let mut source = Vec::with_capacity(width * height * BYTES_PER_RGBA16F_PIXEL);
		for pixel_index in 0..width * height {
			for channel in 0..3 {
				source.extend_from_slice(&f16::from_f32((pixel_index * 3 + channel) as f32).to_le_bytes());
			}
			source.extend_from_slice(&f16::from_f32(0.25).to_le_bytes());
		}
		let pixels = decode_source_radiance(&source, &Global).unwrap();
		let mips = build_source_mips(width as u32, height as u32, pixels, &Global).unwrap();
		let (extent, level_count) = source_atlas_layout(width as u32, height as u32).unwrap();
		let mut atlas = vec![0; atlas_byte_size(extent).unwrap()];
		let mut source_mip = Vec::new();
		write_source_atlas(width as u32, height as u32, &source, &mut atlas, &mut source_mip).unwrap();

		let mut expected = vec![0; atlas.len()];
		let mut level_y_offset = 0;
		for mip in &mips {
			write_source_level(
				&mut expected,
				extent.width(),
				level_y_offset,
				mip.width,
				mip.height,
				&mip.pixels,
			);
			level_y_offset += mip.height;
		}

		assert_eq!(level_count as usize, mips.len());
		assert_eq!(atlas, expected);
		assert_eq!(f16::from_le_bytes([atlas[6], atlas[7]]).to_f32(), 1.0);
	}

	#[crate::r#async::test]
	async fn gpu_base_cubemap_matches_cpu_projection_for_nonconstant_radiance() {
		let client = GPUIBLClient::try_new().expect(
			"GPU IBL setup failed. The most likely cause is invalid native shader code or unavailable compute support on the system device.",
		);
		let (width, height) = (8_u32, 4_u32);
		let mut source = vec![0; width as usize * height as usize * BYTES_PER_RGBA16F_PIXEL];
		for y in 0..height {
			for x in 0..width {
				let pixel_index = (y * width + x) as usize;
				let pixel = &mut source[pixel_index * BYTES_PER_RGBA16F_PIXEL..(pixel_index + 1) * BYTES_PER_RGBA16F_PIXEL];
				for (channel, value) in [x as f32, y as f32 * 2.0, (x + y) as f32].into_iter().enumerate() {
					pixel[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
				}
				pixel[6..8].copy_from_slice(&f16::from_f32(1.0).to_le_bytes());
			}
		}

		let gpu = client
			.bake_image_ibl(Extent::rectangle(width, height), &source)
			.await
			.unwrap();
		let cpu = bake_image_ibl_in(Extent::rectangle(width, height), &source, &Global).unwrap();
		let gpu_stream = &gpu.streams[1];
		let cpu_stream = &cpu.streams[1];
		let gpu_base = &gpu.data[gpu_stream.offset()..gpu_stream.offset() + gpu_stream.size()];
		let cpu_base = &cpu.data[cpu_stream.offset()..cpu_stream.offset() + cpu_stream.size()];
		for (pixel_index, (gpu_pixel, cpu_pixel)) in gpu_base
			.as_chunks::<BYTES_PER_RGBA16F_PIXEL>()
			.0
			.iter()
			.zip(cpu_base.as_chunks::<BYTES_PER_RGBA16F_PIXEL>().0.iter())
			.enumerate()
		{
			for channel in 0..3 {
				let gpu_value = f16::from_le_bytes([gpu_pixel[channel * 2], gpu_pixel[channel * 2 + 1]]).to_f32();
				let cpu_value = f16::from_le_bytes([cpu_pixel[channel * 2], cpu_pixel[channel * 2 + 1]]).to_f32();

				assert!(
					(gpu_value - cpu_value).abs() <= 0.01,
					"GPU base cubemap pixel {pixel_index} channel {channel} differs from CPU: GPU={gpu_value}, CPU={cpu_value}"
				);
			}
		}
	}

	#[crate::r#async::test]
	async fn gpu_bake_keeps_a_constant_environment_constant() {
		let client = GPUIBLClient::try_new().expect(
			"GPU IBL setup failed. The most likely cause is invalid native shader code or unavailable compute support on the system device.",
		);
		let color = [4.0_f32, 0.5, 2.0];
		let mut source = vec![0; 4 * 2 * BYTES_PER_RGBA16F_PIXEL];
		for pixel in source.as_chunks_mut::<BYTES_PER_RGBA16F_PIXEL>().0 {
			for (channel, value) in color.into_iter().enumerate() {
				pixel[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
			}
			pixel[6..8].copy_from_slice(&f16::from_f32(0.25).to_le_bytes());
		}

		let baked = client.bake_image_ibl(Extent::rectangle(4, 2), &source).await.unwrap();

		assert_eq!(&baked.data[..source.len()], source.as_slice());
		for (pixel_index, pixel) in baked.data[source.len()..]
			.as_chunks::<BYTES_PER_RGBA16F_PIXEL>()
			.0
			.iter()
			.enumerate()
		{
			let decoded = std::array::from_fn::<_, 4, _>(|channel| {
				f16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]).to_f32()
			});

			assert_eq!(decoded, [color[0], color[1], color[2], 1.0], "generated pixel {pixel_index}");
		}
	}

	use std::{
		alloc::Global,
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
		},
	};

	use exr::prelude::f16;
	use utils::Extent;

	use super::{
		BYTES_PER_RGBA16F_PIXEL, GPUIBLBakeError, GPUIBLClient, GpuWorkerError, atlas_byte_size, source_atlas_layout,
		write_source_atlas, write_source_level,
	};
	use crate::ibl::cpu::{bake_image_ibl_in, build_source_mips, decode_source_radiance};
}

use std::{any::Any, error::Error, fmt};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBuffer as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
	device::Device as _,
	queue::Queue as _,
};
use utils::Extent;

use super::{
	cpu::{
		BYTES_PER_RGBA16F_PIXEL, CUBE_FACE_COUNT, CubemapIBLLayout, DIFFUSE_CUBE_FACE_SIZE, IBLBakeError, Radiance,
		decode_source_pixel, downsample_source_pixel, lat_long_row_solid_angle, write_rgba16f,
	},
	gpu_shaders::{GPU_IBL_GLSL, GPU_IBL_HLSL, GPU_IBL_MSL},
};
use crate::{
	gpu_worker::{
		GpuProcessor, GpuWorker, GpuWorkerError, OUTPUT_SLOT, OwnedContext, SOURCE_SLOT, Submission, create_compute_context,
		create_compute_kernel,
	},
	resources::{image::IBL_PREFILTERED_SPECULAR_MIP_COUNT, mips::mip_extents},
};
