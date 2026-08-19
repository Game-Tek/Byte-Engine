const SOURCE_ATLAS_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
const OUTPUT_ATLAS_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);
const GPU_ATLAS_MAX_DIMENSION: u32 = 8192;

/// The `GPUIBLBakeError` enum identifies why environment-map generation could not use the GPU path.
#[derive(Debug)]
pub enum GPUIBLBakeError {
	InvalidInput(IBLBakeError),
	InstanceCreation(&'static str),
	DeviceCreation(&'static str),
	ContextCreation(&'static str),
	ShaderCompilation(String),
	ShaderCreation,
	AtlasTooLarge { width: u32, height: u32 },
	AtlasLayoutOverflow,
	SourceUploadSizeMismatch { expected: usize, got: usize },
	OutputReadbackSizeMismatch { expected: usize, got: usize },
	GPUExecution,
	WorkerCreation(String),
	WorkerUnavailable,
}

impl fmt::Display for GPUIBLBakeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidInput(error) => error.fmt(formatter),
			Self::InstanceCreation(error) => write!(
				formatter,
				"GPU environment-map instance creation failed. The most likely cause is that no supported graphics backend is available. Error: {error}"
			),
			Self::DeviceCreation(error) => write!(
				formatter,
				"GPU environment-map device creation failed. The most likely cause is that no device supports compute and transfer work. Error: {error}"
			),
			Self::ContextCreation(error) => write!(
				formatter,
				"GPU environment-map context creation failed. The most likely cause is that the selected device could not create an auxiliary context. Error: {error}"
			),
			Self::ShaderCompilation(error) => write!(
				formatter,
				"GPU environment-map shader compilation failed. The most likely cause is unsupported native shader syntax. Error: {error}"
			),
			Self::ShaderCreation => formatter.write_str(
				"GPU environment-map shader creation failed. The most likely cause is that the selected backend rejected the compute shader.",
			),
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
			Self::WorkerCreation(error) => write!(
				formatter,
				"GPU environment-map worker creation failed. The most likely cause is that the process cannot create another thread. Error: {error}"
			),
			Self::WorkerUnavailable => formatter.write_str(
				"GPU environment-map worker is unavailable. The most likely cause is that GPU initialization or command execution terminated the worker.",
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

/// The `OwnedBakedImageIBL` struct carries GPU-generated environment maps from the dedicated worker to asset storage.
pub struct OwnedBakedImageIBL {
	pub root_extent: [u32; 3],
	pub ibl: crate::resources::image::ImageIBL,
	pub streams: Vec<crate::StreamDescription>,
	pub data: Box<[u8]>,
}

/// The `GPUIBLClient` struct serializes environment-map requests onto a dedicated GHI context thread.
///
/// Install this client on an EXR asset handler. The handler can then run on the asset manager's shared worker pool
/// without moving or concurrently accessing the backend context.
pub struct GPUIBLClient {
	sender: SyncSender<GPUIBLWorkerMessage>,
	responses: Mutex<mpsc::Receiver<Result<OwnedBakedImageIBL, GPUIBLBakeError>>>,
	worker: Option<JoinHandle<()>>,
}

impl GPUIBLClient {
	/// Creates a dedicated worker with its own compute device and context.
	pub fn try_new() -> Result<Self, GPUIBLBakeError> {
		Self::spawn(GPUIBLProcessor::try_new)
	}

	/// Runs a processor factory on the dedicated GPU thread before accepting requests.
	///
	/// Create every thread-affine GHI device and context inside `initialize`. The factory itself must be safe to move,
	/// but the processor it returns remains on the worker for its entire lifetime.
	pub fn from_processor_factory(
		initialize: impl FnOnce() -> Result<GPUIBLProcessor, GPUIBLBakeError> + Send + 'static,
	) -> Result<Self, GPUIBLBakeError> {
		Self::spawn(initialize)
	}

	/// Submits one borrowed source image and waits until the GPU result is safe to consume.
	pub fn bake_image_ibl(&self, source_extent: Extent, source_rgba16f: &[u8]) -> Result<OwnedBakedImageIBL, GPUIBLBakeError> {
		// One outstanding round trip matches the single GPU worker and avoids allocating a response channel per image.
		let responses = self.responses.lock().map_err(|_| GPUIBLBakeError::WorkerUnavailable)?;
		let request = GPUIBLRequest {
			source_extent,
			source: source_rgba16f.as_ptr(),
			source_len: source_rgba16f.len(),
		};
		self.sender
			.send(GPUIBLWorkerMessage::Bake(request))
			.map_err(|_| GPUIBLBakeError::WorkerUnavailable)?;
		responses.recv().map_err(|_| GPUIBLBakeError::WorkerUnavailable)?
	}

	/// Starts the context-owning worker and reports initialization before accepting requests.
	#[cfg(test)]
	pub(crate) fn unavailable_for_test() -> Self {
		let (sender, receiver) = mpsc::sync_channel(1);
		let (response_sender, responses) = mpsc::sync_channel(1);
		drop(receiver);
		drop(response_sender);
		Self {
			sender,
			responses: Mutex::new(responses),
			worker: None,
		}
	}

	fn spawn(
		initialize: impl FnOnce() -> Result<GPUIBLProcessor, GPUIBLBakeError> + Send + 'static,
	) -> Result<Self, GPUIBLBakeError> {
		let (sender, receiver) = mpsc::sync_channel(1);
		let (response_sender, responses) = mpsc::sync_channel(1);
		let (startup, startup_receiver) = mpsc::sync_channel(1);
		let worker = std::thread::Builder::new()
			.name("GPU Environment Map Worker".to_string())
			.spawn(move || {
				let mut processor = match initialize() {
					Ok(processor) => {
						let _ = startup.send(Ok(()));
						processor
					}
					Err(error) => {
						let _ = startup.send(Err(error));
						return;
					}
				};

				while let Ok(message) = receiver.recv() {
					match message {
						GPUIBLWorkerMessage::Bake(request) => {
							// The submitting asset worker blocks on this response, so its borrowed source remains valid.
							let source = unsafe { std::slice::from_raw_parts(request.source, request.source_len) };
							let result = processor.bake_image_ibl(request.source_extent, source);
							if response_sender.send(result).is_err() {
								return;
							}
						}
						GPUIBLWorkerMessage::Shutdown => return,
					}
				}
			})
			.map_err(|error| GPUIBLBakeError::WorkerCreation(error.to_string()))?;

		match startup_receiver.recv() {
			Ok(Ok(())) => Ok(Self {
				sender,
				responses: Mutex::new(responses),
				worker: Some(worker),
			}),
			Ok(Err(error)) => {
				let _ = worker.join();
				Err(error)
			}
			Err(_) => {
				let _ = worker.join();
				Err(GPUIBLBakeError::WorkerUnavailable)
			}
		}
	}
}

impl Drop for GPUIBLClient {
	fn drop(&mut self) {
		let _ = self.sender.send(GPUIBLWorkerMessage::Shutdown);
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

/// The request uses a borrowed pointer only while the submitting thread is synchronously waiting for its response.
struct GPUIBLRequest {
	source_extent: Extent,
	source: *const u8,
	source_len: usize,
}

// SAFETY: `GPUIBLClient::bake_image_ibl` does not return until the worker responds or disconnects. The immutable source
// slice therefore outlives every worker access, and no mutable reference can coexist with the caller's shared borrow.
unsafe impl Send for GPUIBLRequest {}

enum GPUIBLWorkerMessage {
	Bake(GPUIBLRequest),
	Shutdown,
}

/// The `GPUIBLConstruction` struct preserves context-before-owner drop order while GPU initialization can fail.
struct GPUIBLConstruction {
	context: ghi::implementation::Context,
	owner: Box<dyn Any>,
}

/// The `GPUIBLProcessor` struct provides thread-confined environment-map generation with CPU-compatible output.
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
		let features = ghi::device::Features::new().mesh_shading(false);
		let mut instance = ghi::implementation::Instance::new(features).map_err(GPUIBLBakeError::InstanceCreation)?;
		let mut queue = None;
		let device = instance
			.create_device(
				features,
				&mut [(
					ghi::QueueSelection::new(ghi::WorkloadTypes::COMPUTE | ghi::WorkloadTypes::TRANSFER),
					&mut queue,
				)],
			)
			.map_err(GPUIBLBakeError::DeviceCreation)?;
		let context = device.create_context().map_err(GPUIBLBakeError::ContextCreation)?;
		Self::from_parts(
			context,
			queue.expect("GHI device creation must populate the requested compute queue handle."),
			(device, instance),
		)
	}

	/// Uses a caller-created auxiliary context for environment-map generation.
	///
	/// `owner` keeps the device, instance, or other native state alive until after the context is dropped. Create all three
	/// values on the current thread, then continue using the processor on this thread or return it from a worker-local factory.
	pub fn from_context<Owner: 'static>(
		context: ghi::implementation::Context,
		queue: ghi::QueueHandle,
		owner: Owner,
	) -> Result<Self, GPUIBLBakeError> {
		Self::from_parts(context, queue, owner)
	}

	/// Creates the shared compute pipeline before the handler begins processing concurrent assets.
	fn from_parts<Owner: 'static>(
		context: ghi::implementation::Context,
		queue: ghi::QueueHandle,
		owner: Owner,
	) -> Result<Self, GPUIBLBakeError> {
		// Keep native owners alive after the context on every early-return and unwinding path.
		let mut construction = GPUIBLConstruction {
			context,
			owner: Box::new(owner),
		};
		let context = &mut construction.context;
		let compiled = ghi::shader::compile(
			"GPU environment-map generation",
			ghi::shader::ShaderSource::PlatformNative {
				glsl: GPU_IBL_GLSL,
				msl: GPU_IBL_MSL,
				msl_entry_point: "generate_environment_map",
				hlsl: GPU_IBL_HLSL,
				hlsl_entry_point: "generate_environment_map",
			},
		)
		.map_err(GPUIBLBakeError::ShaderCompilation)?;
		let resources = [
			ghi::ShaderResourceDescriptor::single(
				SOURCE_ATLAS_SLOT,
				ghi::ResourceKind::CombinedImageSampler,
				ghi::AccessPolicies::READ,
			),
			ghi::ShaderResourceDescriptor::single(
				OUTPUT_ATLAS_SLOT,
				ghi::ResourceKind::StorageImage,
				ghi::AccessPolicies::WRITE,
			),
		];
		let shader = context
			.create_shader(
				Some("GPU environment-map generation"),
				compiled.as_source(),
				ghi::ShaderTypes::Compute,
				resources,
			)
			.map_err(|_| GPUIBLBakeError::ShaderCreation)?;
		let push_constant_ranges = [ghi::pipelines::PushConstantRange::new(
			0,
			std::mem::size_of::<GPUIBLPushConstants>() as u32,
		)];
		let pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&push_constant_ranges,
				ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
			)
			.name("GPU environment-map generation"),
		);
		let source_sampler = context.build_sampler(ghi::sampler::Builder::new().max_lod(0.0));

		let GPUIBLConstruction { context, owner } = construction;
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

		let copy_handle = self.dispatch(layout, source_level_count, scratch);
		self.context.wait_for_synchronizer(scratch.synchronizer);
		#[cfg(any(debug_assertions, test))]
		if self.context.has_errors() {
			return Err(GPUIBLBakeError::GPUExecution);
		}

		let expected_readback_size = atlas_byte_size(output_atlas_extent)?;
		let readback = self.context.get_image_data(copy_handle);
		if readback.len() < expected_readback_size {
			return Err(GPUIBLBakeError::OutputReadbackSizeMismatch {
				expected: expected_readback_size,
				got: readback.len(),
			});
		}

		let mut data = Vec::new();
		data.try_reserve_exact(layout.total_size())
			.map_err(|_| IBLBakeError::AllocationFailed)?;
		data.resize(layout.total_size(), 0);
		data[..layout.root_size()].copy_from_slice(source_rgba16f);
		copy_output_atlas(layout, readback, output_atlas_extent.width(), &mut data);
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
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage)
				.name("Environment cubemap output atlas")
				.extent(key.output_atlas_extent)
				.device_accesses(ghi::DeviceAccesses::DeviceToHost)
				.use_case(ghi::UseCases::STATIC),
		);
		let descriptor_set = self.context.create_descriptor_set(Some("Environment-map atlases"));
		self.context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				SOURCE_ATLAS_SLOT,
				source_atlas,
				self.source_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_ATLAS_SLOT, output_atlas, ghi::Layouts::General),
		]);
		let command_buffer = self
			.context
			.queue_reference(self.queue)
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
	) -> ghi::TextureCopyHandle {
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

		let copy_handle = recording.transfer_textures(&[scratch.output_atlas.into()])[0];
		recording.execute(scratch.synchronizer);
		copy_handle
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
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
fn source_atlas_layout(mut width: u32, mut height: u32) -> Result<(Extent, u32), GPUIBLBakeError> {
	let atlas_width = width;
	let mut atlas_height = 0_u32;
	let mut level_count = 0_u32;
	loop {
		atlas_height = atlas_height.checked_add(height).ok_or(GPUIBLBakeError::AtlasLayoutOverflow)?;
		level_count += 1;
		if width == 1 && height == 1 {
			break;
		}
		width = (width / 2).max(1);
		height = (height / 2).max(1);
	}
	Ok((Extent::rectangle(atlas_width, atlas_height), level_count))
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
	for (source, destination) in source_rgba16f
		.chunks_exact(BYTES_PER_RGBA16F_PIXEL)
		.zip(atlas[..source_rgba16f.len()].chunks_exact_mut(BYTES_PER_RGBA16F_PIXEL))
	{
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
			destination.push(filter_source_texel(
				source_width,
				source_height,
				destination_width,
				destination_height,
				x,
				y,
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
			let radiance = filter_source_texel(
				source_width,
				source_height,
				destination_width,
				destination_height,
				x,
				y,
				&mut |source_index| pixels[source_index],
			);
			pixels[destination_index] = radiance;
		}
	}
	pixels.truncate(pixel_count);

	Ok((destination_width, destination_height))
}

/// Filters one destination texel with the CPU path's accumulation order and precision.
#[allow(clippy::too_many_arguments)]
fn filter_source_texel(
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
	x: u32,
	y: u32,
	source_pixel: &mut impl FnMut(usize) -> Radiance,
) -> Radiance {
	let source_y_begin = y as u64 * source_height as u64 / destination_height as u64;
	let source_y_end = ((y + 1) as u64 * source_height as u64 / destination_height as u64).max(source_y_begin + 1);
	let source_x_begin = x as u64 * source_width as u64 / destination_width as u64;
	let source_x_end = ((x + 1) as u64 * source_width as u64 / destination_width as u64).max(source_x_begin + 1);
	let mut sum = [0.0_f64; 3];
	let mut total_weight = 0.0_f64;

	for source_y in source_y_begin..source_y_end {
		let weight = lat_long_row_solid_angle(source_width, source_height, source_y as u32) as f64;
		for source_x in source_x_begin..source_x_end {
			let radiance = source_pixel(source_y as usize * source_width as usize + source_x as usize);
			for channel in 0..3 {
				sum[channel] += radiance[channel] as f64 * weight;
			}
			total_weight += weight;
		}
	}

	[
		(sum[0] / total_weight) as f32,
		(sum[1] / total_weight) as f32,
		(sum[2] / total_weight) as f32,
	]
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
		for (radiance, destination) in pixels[source_start..source_start + level_width as usize]
			.iter()
			.zip(atlas[destination_start..destination_end].chunks_exact_mut(BYTES_PER_RGBA16F_PIXEL))
		{
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

#[cfg(test)]
mod tests {
	#[test]
	fn push_constants_match_every_native_shader_layout() {

		assert_eq!(std::mem::size_of::<GPUIBLPushConstants>(), 112);
	}

	#[test]
	fn gpu_client_can_cross_asset_worker_threads() {
		fn assert_send_sync<T: Send + Sync>() {}
		assert_send_sync::<GPUIBLClient>();
	}

	#[test]
	fn gpu_processor_factory_runs_on_the_context_owning_worker() {
		let caller = std::thread::current().id();
		let ran_on_worker = Arc::new(AtomicBool::new(false));
		let worker_result = ran_on_worker.clone();

		let result = GPUIBLClient::from_processor_factory(move || {
			worker_result.store(std::thread::current().id() != caller, Ordering::SeqCst);
			Err(GPUIBLBakeError::WorkerUnavailable)
		});

		assert!(matches!(result, Err(GPUIBLBakeError::WorkerUnavailable)));
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

	#[test]
	fn output_atlas_contains_every_face_and_level_once() {
		let source = vec![0; 512 * 256 * BYTES_PER_RGBA16F_PIXEL];
		let layout = CubemapIBLLayout::new(Extent::rectangle(512, 256), &source).unwrap();
		let extent = output_atlas_extent(layout).unwrap();

		assert_eq!(layout.specular_face_size(), 128);
		assert_eq!(extent, Extent::rectangle(128, 1578));
	}

	#[test]
	fn gpu_base_cubemap_matches_cpu_projection_for_nonconstant_radiance() {
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

		let gpu = client.bake_image_ibl(Extent::rectangle(width, height), &source).unwrap();
		let cpu = bake_image_ibl_in(Extent::rectangle(width, height), &source, &Global).unwrap();
		let gpu_stream = &gpu.streams[1];
		let cpu_stream = &cpu.streams[1];
		let gpu_base = &gpu.data[gpu_stream.offset()..gpu_stream.offset() + gpu_stream.size()];
		let cpu_base = &cpu.data[cpu_stream.offset()..cpu_stream.offset() + cpu_stream.size()];
		for (pixel_index, (gpu_pixel, cpu_pixel)) in gpu_base
			.chunks_exact(BYTES_PER_RGBA16F_PIXEL)
			.zip(cpu_base.chunks_exact(BYTES_PER_RGBA16F_PIXEL))
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

	#[test]
	fn gpu_bake_keeps_a_constant_environment_constant() {
		let client = GPUIBLClient::try_new().expect(
			"GPU IBL setup failed. The most likely cause is invalid native shader code or unavailable compute support on the system device.",
		);
		let color = [4.0_f32, 0.5, 2.0];
		let mut source = vec![0; 4 * 2 * BYTES_PER_RGBA16F_PIXEL];
		for pixel in source.chunks_exact_mut(BYTES_PER_RGBA16F_PIXEL) {
			for (channel, value) in color.into_iter().enumerate() {
				pixel[channel * 2..channel * 2 + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
			}
			pixel[6..8].copy_from_slice(&f16::from_f32(0.25).to_le_bytes());
		}

		let baked = client.bake_image_ibl(Extent::rectangle(4, 2), &source).unwrap();

		assert_eq!(&baked.data[..source.len()], source.as_slice());
		for (pixel_index, pixel) in baked.data[source.len()..].chunks_exact(BYTES_PER_RGBA16F_PIXEL).enumerate() {
			let decoded = std::array::from_fn::<_, 4, _>(|channel| {
				f16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]).to_f32()
			});

			assert_eq!(decoded, [color[0], color[1], color[2], 1.0], "generated pixel {pixel_index}");
		}
	}

	use std::{
		alloc::Global,
		sync::{
			atomic::{AtomicBool, Ordering},
			Arc,
		},
	};

	use exr::prelude::f16;
	use utils::Extent;

	use super::{
		atlas_byte_size, output_atlas_extent, source_atlas_layout, write_source_atlas, write_source_level, GPUIBLBakeError,
		GPUIBLClient, GPUIBLPushConstants, BYTES_PER_RGBA16F_PIXEL,
	};
	use crate::ibl::cpu::{bake_image_ibl_in, build_source_mips, decode_source_radiance, CubemapIBLLayout};
}

use std::{
	any::Any,
	error::Error,
	fmt,
	sync::{
		mpsc::{self, SyncSender},
		Mutex,
	},
	thread::JoinHandle,
};

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
		decode_source_pixel, lat_long_row_solid_angle, write_rgba16f, CubemapIBLLayout, IBLBakeError, Radiance,
		BYTES_PER_RGBA16F_PIXEL, CUBE_FACE_COUNT, DIFFUSE_CUBE_FACE_SIZE,
	},
	gpu_shaders::{GPU_IBL_GLSL, GPU_IBL_HLSL, GPU_IBL_MSL},
};
use crate::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT;
