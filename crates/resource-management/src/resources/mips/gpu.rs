/// The `GPUMipError` enum identifies why offline material mip generation could not use the GPU path.
#[derive(Debug)]
pub enum GPUMipError {
	InstanceCreation(&'static str),
	DeviceCreation(&'static str),
	ContextCreation(&'static str),
	ShaderCompilation(String),
	ShaderCreation,
	WorkerCreation(String),
	WorkerUnavailable,
	UploadSizeMismatch { expected: usize, got: usize },
	ReadbackSizeMismatch { expected: usize, got: usize },
	GPUExecution,
}

impl fmt::Display for GPUMipError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InstanceCreation(error) => write!(formatter, "GPU mip instance creation failed. The most likely cause is that no supported graphics backend is available. Error: {error}"),
			Self::DeviceCreation(error) => write!(formatter, "GPU mip device creation failed. The most likely cause is that no device supports compute and transfer work. Error: {error}"),
			Self::ContextCreation(error) => write!(formatter, "GPU mip context creation failed. The most likely cause is that the selected device could not create an auxiliary context. Error: {error}"),
			Self::ShaderCompilation(error) => write!(formatter, "GPU mip shader compilation failed. The most likely cause is unsupported native shader syntax. Error: {error}"),
			Self::ShaderCreation => formatter.write_str("GPU mip shader creation failed. The most likely cause is that the selected backend rejected the compute shader."),
			Self::WorkerCreation(error) => write!(formatter, "GPU mip worker creation failed. The most likely cause is that the process cannot create another thread. Error: {error}"),
			Self::WorkerUnavailable => formatter.write_str("GPU mip worker is unavailable. The most likely cause is that GPU initialization or command execution terminated the worker."),
			Self::UploadSizeMismatch { expected, got } => write!(formatter, "GPU mip upload has the wrong size: expected {expected}, got {got}. The most likely cause is a staging allocation that does not match the base image."),
			Self::ReadbackSizeMismatch { expected, got } => write!(formatter, "GPU mip readback has the wrong size: expected {expected}, got {got}. The most likely cause is incomplete texture readback."),
			Self::GPUExecution => formatter.write_str("GPU mip generation failed. The most likely cause is a graphics backend validation or command-execution error."),
		}
	}
}

impl Error for GPUMipError {}

/// The `MaterialMipGenerator` struct moves material texture mip filtering and BC7 compression onto the GPU.
///
/// Install it on material importers through [`crate::resources::mips::MipGenerationBackend`]. Requests it can't serve,
/// and requests whose GPU work fails, fall back to [`CPUMipGenerationBackend`].
pub struct MaterialMipGenerator {
	/// Requests carry the base level's width, height, and gamma, and the format the chain is stored in.
	worker: GpuWorker<(u32, u32, Gamma, Formats), Result<(), GPUMipError>>,
}

impl MaterialMipGenerator {
	/// Creates the dedicated offline GPU worker used by material importers.
	pub fn try_with_default_gpu() -> Result<Self, GPUMipError> {
		let worker = GpuWorker::spawn("GPU Material Mip Worker", GPUMipProcessor::try_new, GPUMipProcessor::encode).map_err(
			|error| match error {
				GpuWorkerSpawnError::Initialization(error) => error,
				GpuWorkerSpawnError::WorkerCreation(error) => GPUMipError::WorkerCreation(error.to_string()),
				GpuWorkerSpawnError::WorkerUnavailable => GPUMipError::WorkerUnavailable,
			},
		)?;
		Ok(Self { worker })
	}
}

impl MipGenerationBackend for MaterialMipGenerator {
	fn encode_mip_chain(
		&self,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
	) -> Result<(), MipGenerationError> {
		// The GPU filters RGBA8 levels, and uncommon 16-bit textures keep the CPU filter. The CPU backend also reports
		// buffers whose sizes don't match the request.
		let sizes_match = base_level.len() == width as usize * height as usize * 4
			&& Some(output.len()) == encoded_mip_chain_size(output_format, Extent::rectangle(width, height));
		if filtering_format(output_format) == Formats::RGBA8 && sizes_match {
			let encoded = self
				.worker
				.call((width, height, gamma, output_format), base_level, output)
				.unwrap_or(Err(GPUMipError::WorkerUnavailable));
			match encoded {
				Ok(()) => return Ok(()),
				Err(error) => log::warn!(
					"GPU material mip generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
				),
			}
		}

		CPUMipGenerationBackend.encode_mip_chain(output_format, gamma, width, height, base_level, output)
	}
}

/// The `GPUMipProcessor` struct owns the thread-confined compute context used for offline filtering and compression.
pub struct GPUMipProcessor {
	context: ghi::implementation::Context,
	mip_pipeline: ghi::PipelineHandle,
	/// The BC7 pipeline, compiled on the first BC7 request. `Some(None)` records a failed compilation, after which BC7
	/// chains compress their GPU-filtered levels on the CPU.
	block_pipeline: Option<Option<ghi::PipelineHandle>>,
	queue: ghi::QueueHandle,
	sampler: ghi::SamplerHandle,
	scratch: Vec<GPUMipScratch>,
	_owner: Box<dyn Any>,
}

impl GPUMipProcessor {
	fn try_new() -> Result<Self, GPUMipError> {
		let (context, queue, owner) = create_compute_context().map_err(|error| match error {
			ComputeContextError::Instance(error) => GPUMipError::InstanceCreation(error),
			ComputeContextError::Device(error) => GPUMipError::DeviceCreation(error),
			ComputeContextError::Context(error) => GPUMipError::ContextCreation(error),
		})?;
		// Keep native owners alive after the context on every early-return and unwinding path.
		let mut construction = OwnedContext { context, owner };
		let mip_pipeline = create_compute_kernel(
			&mut construction.context,
			"GPU material mip generation",
			ghi::shader::ShaderSource::PlatformNative {
				glsl: GPU_MIP_GLSL,
				msl: GPU_MIP_MSL,
				msl_entry_point: "generate_mip",
				hlsl: GPU_MIP_HLSL,
				hlsl_entry_point: "generate_mip",
			},
			std::mem::size_of::<PushConstants>(),
		)
		.map_err(kernel_error)?;
		let sampler = construction.context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.max_lod(0.0),
		);
		let OwnedContext { context, owner } = construction;
		Ok(Self {
			context,
			mip_pipeline,
			block_pipeline: None,
			queue,
			sampler,
			scratch: Vec::new(),
			_owner: owner,
		})
	}

	/// Returns the BC7 pipeline, compiling it on first use, or `None` when it can't be compiled.
	fn block_pipeline(&mut self) -> Option<ghi::PipelineHandle> {
		let context = &mut self.context;
		*self.block_pipeline.get_or_insert_with(|| {
			let resources = [
				ghi::ShaderResourceDescriptor::single(
					SOURCE_SLOT,
					ghi::ResourceKind::CombinedImageSampler,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					OUTPUT_SLOT,
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::WRITE,
				)
				.buffer_stride(BLOCK_BYTES as u32),
			];
			create_besl_compute_kernel(
				context,
				"GPU material BC7 encoding",
				BC7_ENCODER,
				Extent::square(BC7_WORKGROUP_SIZE),
				resources,
				std::mem::size_of::<BlockPushConstants>(),
			)
			.map_err(kernel_error)
			.inspect_err(|error| {
				log::warn!("GPU BC7 encoder is unavailable; BC7 textures compress on the CPU after GPU filtering. {error}")
			})
			.ok()
		})
	}

	/// Serves one [`MaterialMipGenerator`] request: filters the RGBA8 base level on the GPU and writes the chain, encoded
	/// as the request's format, into `output`.
	///
	/// `base` and `output` have the sizes the request's extent and format need. BC7 levels are compressed on the GPU and
	/// read back as blocks. Other formats read back the filtered levels and encode them on the CPU.
	fn encode(
		&mut self,
		(width, height, gamma, format): (u32, u32, Gamma, Formats),
		base: &[u8],
		output: &mut [u8],
	) -> Result<(), GPUMipError> {
		let block_pipeline = if matches!(format, Formats::BC7 | Formats::BC7SRGB) {
			self.block_pipeline()
		} else {
			None
		};
		// A lone texel has no lower levels, so without GPU compression there is no GPU work to do.
		if block_pipeline.is_none() && width <= 1 && height <= 1 {
			encode_level_in(format, Extent::rectangle(width, height), base, output, Global);
			return Ok(());
		}

		let (context, scratch_cache) = (&mut self.context, &mut self.scratch);
		let scratch_index = scratch_cache
			.iter()
			.position(|scratch| scratch.width == width && scratch.height == height)
			.unwrap_or_else(|| {
				scratch_cache.push(create_scratch(context, self.queue, self.sampler, width, height));
				scratch_cache.len() - 1
			});
		let scratch = &mut scratch_cache[scratch_index];
		if block_pipeline.is_some() && scratch.blocks.is_none() {
			scratch.blocks = Some(create_block_scratch(context, self.sampler, scratch));
		}
		let scratch = &scratch_cache[scratch_index];

		let upload = context.get_texture_slice_mut(scratch.base_image);
		if upload.len() != base.len() {
			return Err(GPUMipError::UploadSizeMismatch {
				expected: base.len(),
				got: upload.len(),
			});
		}
		upload.copy_from_slice(base);
		context.sync_texture(scratch.base_image);

		// The shader filters in linear light when this push constant is non-zero.
		let srgb = u32::from(gamma == Gamma::SRGB);
		let mut command_buffer = context.command_buffer(scratch.command_buffer);
		let mut recording = command_buffer.create_command_buffer_recording();
		for level in &scratch.levels {
			let command = recording.bind_compute_pipeline(self.mip_pipeline);
			command.bind_descriptor_sets(&[level.descriptor_set]);
			command.write_push_constant(
				0,
				PushConstants {
					source_width: level.source_width,
					source_height: level.source_height,
					destination_width: level.width,
					destination_height: level.height,
					srgb,
				},
			);
			command.dispatch(ghi::DispatchExtent::new(
				Extent::rectangle(level.width, level.height),
				Extent::rectangle(8, 8),
			));
		}
		let block_encoding = block_pipeline.zip(scratch.blocks.as_ref());
		let readbacks = if let Some((pipeline, blocks)) = block_encoding {
			for level in &blocks.levels {
				let command = recording.bind_compute_pipeline(pipeline);
				command.bind_descriptor_sets(&[level.descriptor_set]);
				command.write_push_constant(0, level.push_constants);
				command.dispatch(ghi::DispatchExtent::new(
					Extent::rectangle(level.push_constants.blocks_x, level.push_constants.blocks_y),
					Extent::square(BC7_WORKGROUP_SIZE),
				));
			}
			Vec::new()
		} else {
			scratch
				.levels
				.iter()
				.map(|level| recording.transfer_texture(level.image.into()))
				.collect::<Result<Vec<_>, _>>()
				.map_err(|_| GPUMipError::GPUExecution)?
		};
		recording.execute(scratch.synchronizer);
		context.wait_for_synchronizer(scratch.synchronizer);
		#[cfg(any(debug_assertions, test))]
		if context.has_errors() {
			return Err(GPUMipError::GPUExecution);
		}

		if let Some((_, blocks)) = block_encoding {
			// The buffer holds every level's blocks back to back, which is the stored chain layout.
			let encoded = bytemuck::cast_slice::<[u32; 4], u8>(context.get_buffer_slice(blocks.buffer));
			if encoded.len() != output.len() {
				return Err(GPUMipError::ReadbackSizeMismatch {
					expected: output.len(),
					got: encoded.len(),
				});
			}
			output.copy_from_slice(encoded);
			return Ok(());
		}

		// Without GPU compression, encode the uploaded base level and each filtered level on the CPU.
		let mut remaining = output;
		let mut encode_next = |extent: Extent, texels: &[u8]| {
			let size = encoded_mip_level_size(format, extent).expect("A storable chain has a stored size for every level");
			let (destination, rest) = std::mem::take(&mut remaining).split_at_mut(size);
			encode_level_in(format, extent, texels, destination, Global);
			remaining = rest;
		};
		encode_next(Extent::rectangle(width, height), base);
		for (level, handle) in scratch.levels.iter().zip(readbacks) {
			let size = level.width as usize * level.height as usize * 4;
			let readback = context.get_image_data(handle).map_err(|_| GPUMipError::GPUExecution)?;
			if readback.bytes.len() < size {
				return Err(GPUMipError::ReadbackSizeMismatch {
					expected: size,
					got: readback.bytes.len(),
				});
			}
			encode_next(Extent::rectangle(level.width, level.height), &readback.bytes[..size]);
		}
		Ok(())
	}
}

/// Maps a kernel creation failure to the matching [`GPUMipError`].
fn kernel_error(error: ComputeKernelError) -> GPUMipError {
	match error {
		ComputeKernelError::Compilation(error) => GPUMipError::ShaderCompilation(error),
		ComputeKernelError::Creation => GPUMipError::ShaderCreation,
	}
}

/// The `GPUMipScratch` struct retains one dimension-specific GPU pyramid across material bakes.
struct GPUMipScratch {
	width: u32,
	height: u32,
	base_image: ghi::ImageHandle,
	levels: Vec<GPUMipScratchLevel>,
	/// The BC7 output and bindings, created on the first BC7 request for this extent.
	blocks: Option<BlockScratch>,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
}

#[derive(Clone, Copy)]
struct GPUMipScratchLevel {
	image: ghi::ImageHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	source_width: u32,
	source_height: u32,
	width: u32,
	height: u32,
}

/// The `BlockScratch` struct holds the buffer every level of a pyramid writes its BC7 blocks to.
struct BlockScratch {
	buffer: ghi::BufferHandle<[[u32; 4]]>,
	levels: Vec<BlockScratchLevel>,
}

/// The `BlockScratchLevel` struct binds one level of the pyramid as the source of a BC7 dispatch.
#[derive(Clone, Copy)]
struct BlockScratchLevel {
	descriptor_set: ghi::DescriptorSetHandle,
	push_constants: BlockPushConstants,
}

/// Allocates and binds one reusable GPU pyramid for a base extent.
fn create_scratch(
	context: &mut ghi::implementation::Context,
	queue: ghi::QueueHandle,
	sampler: ghi::SamplerHandle,
	width: u32,
	height: u32,
) -> GPUMipScratch {
	let base_image = context.build_image(
		ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image)
			.name("Material mip base")
			.extent(Extent::rectangle(width, height))
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	let mut levels = Vec::with_capacity((u32::BITS - width.max(height).leading_zeros()) as usize);
	let mut source = base_image;
	let (mut source_width, mut source_height) = (width, height);
	while source_width > 1 || source_height > 1 {
		let destination_width = (source_width / 2).max(1);
		let destination_height = (source_height / 2).max(1);
		let destination = context.build_image(
			ghi::image::Builder::new(
				ghi::Formats::RGBA8UNORM,
				ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferSource,
			)
			.name("Material mip level")
			.extent(Extent::rectangle(destination_width, destination_height))
			.device_accesses(ghi::DeviceAccesses::DeviceToHost)
			.use_case(ghi::UseCases::STATIC),
		);
		let descriptor_set = context.create_descriptor_set(Some("Material mip level"));
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(descriptor_set, SOURCE_SLOT, source, sampler, ghi::Layouts::Read),
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_SLOT, destination, ghi::Layouts::General),
		]);
		levels.push(GPUMipScratchLevel {
			image: destination,
			descriptor_set,
			source_width,
			source_height,
			width: destination_width,
			height: destination_height,
		});
		source = destination;
		(source_width, source_height) = (destination_width, destination_height);
	}
	let command_buffer = context.queue(queue).create_command_buffer(Some("Generate material mips"));
	let synchronizer = context.create_synchronizer(Some("Material mips generated"), true);
	GPUMipScratch {
		width,
		height,
		base_image,
		levels,
		blocks: None,
		command_buffer,
		synchronizer,
	}
}

/// Allocates the block buffer for a pyramid and binds every level, base level first, as a BC7 source.
fn create_block_scratch(
	context: &mut ghi::implementation::Context,
	sampler: ghi::SamplerHandle,
	scratch: &GPUMipScratch,
) -> BlockScratch {
	let chain_size = encoded_mip_chain_size(Formats::BC7, Extent::rectangle(scratch.width, scratch.height))
		.expect("A GPU pyramid's extent should fit a BC7 chain");
	let buffer = context.build_buffer::<[[u32; 4]]>(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.name("Material BC7 blocks")
			.device_accesses(ghi::DeviceAccesses::DeviceToHost)
			.length(chain_size / BLOCK_BYTES),
	);
	// Each level's blocks follow the previous level's, which is the stored chain layout.
	let images = std::iter::once(scratch.base_image).chain(scratch.levels.iter().map(|level| level.image));
	let mut first_block = 0;
	let levels = images
		.zip(mip_extents(scratch.width, scratch.height))
		.map(|(image, (width, height))| {
			let descriptor_set = context.create_descriptor_set(Some("Material BC7 level"));
			context.write(&[
				ghi::DescriptorWrite::combined_image_sampler(descriptor_set, SOURCE_SLOT, image, sampler, ghi::Layouts::Read),
				ghi::DescriptorWrite::buffer(descriptor_set, OUTPUT_SLOT, buffer.into()),
			]);
			let (blocks_x, blocks_y) = (width.div_ceil(4), height.div_ceil(4));
			let push_constants = BlockPushConstants {
				width,
				height,
				blocks_x,
				blocks_y,
				first_block,
			};
			first_block += blocks_x * blocks_y;
			BlockScratchLevel {
				descriptor_set,
				push_constants,
			}
		})
		.collect();
	BlockScratch { buffer, levels }
}

/// Bytes in one BC7 block.
const BLOCK_BYTES: usize = 16;

/// The workgroup size [`BC7_ENCODER`] is compiled and dispatched with, in blocks.
const BC7_WORKGROUP_SIZE: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PushConstants {
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
	srgb: u32,
}

/// The `BlockPushConstants` struct mirrors the push constant of [`BC7_ENCODER`], field for field in declaration order.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlockPushConstants {
	width: u32,
	height: u32,
	blocks_x: u32,
	blocks_y: u32,
	first_block: u32,
}

const GPU_MIP_GLSL: &str = r#"#version 460
#pragma shader_stage(compute)
layout(local_size_x=8, local_size_y=8, local_size_z=1) in;
layout(set=0,binding=0) uniform sampler2D source_image;
layout(rgba8,set=0,binding=1) uniform writeonly image2D destination_image;
layout(push_constant) uniform PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; uint srgb; } pc;
vec3 srgb_to_linear(vec3 color) {
	vec3 low = color / 12.92;
	vec3 high = pow((color + 0.055) / 1.055, vec3(2.4));
	return mix(high, low, lessThanEqual(color, vec3(0.04045)));
}
vec3 linear_to_srgb(vec3 color) {
	vec3 low = color * 12.92;
	vec3 high = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
	return mix(high, low, lessThanEqual(color, vec3(0.0031308)));
}
void generate_mip() {
	uvec2 p=gl_GlobalInvocationID.xy; if(any(greaterThanEqual(p,uvec2(pc.destination_width,pc.destination_height)))) return;
	uvec2 maximum=uvec2(pc.source_width-1u,pc.source_height-1u); uvec2 origin=p*2u;
	vec4 a=texelFetch(source_image,ivec2(min(origin,maximum)),0); vec4 b=texelFetch(source_image,ivec2(min(origin+uvec2(1u,0u),maximum)),0);
	vec4 c=texelFetch(source_image,ivec2(min(origin+uvec2(0u,1u),maximum)),0); vec4 d=texelFetch(source_image,ivec2(min(origin+uvec2(1u,1u),maximum)),0);
	vec4 result=(a+b+c+d)*0.25;
	if(pc.srgb!=0u) result.rgb=linear_to_srgb((srgb_to_linear(a.rgb)+srgb_to_linear(b.rgb)+srgb_to_linear(c.rgb)+srgb_to_linear(d.rgb))*0.25);
	imageStore(destination_image,ivec2(p),result);
}"#;

const GPU_MIP_MSL: &str = r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(compute)
// besl-threadgroup-size:8,8,1
struct Resources { texture2d<float,access::sample> source_image [[id(0)]]; sampler source_sampler [[id(1)]]; texture2d<float,access::write> destination_image [[id(2)]]; };
struct PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; uint srgb; };
float3 srgb_to_linear(float3 color) { return select(pow((color+0.055f)/1.055f,float3(2.4f)),color/12.92f,color<=0.04045f); }
float3 linear_to_srgb(float3 color) { return select(1.055f*pow(color,float3(1.0f/2.4f))-0.055f,color*12.92f,color<=0.0031308f); }
kernel void generate_mip(uint3 invocation_id [[thread_position_in_grid]], constant PushConstants& pc [[buffer(15)]], constant Resources& resources [[buffer(16)]]) {
	uint2 p=invocation_id.xy; if(p.x>=pc.destination_width||p.y>=pc.destination_height) return;
	uint2 maximum=uint2(pc.source_width-1u,pc.source_height-1u); uint2 origin=p*2u;
	float4 a=resources.source_image.read(min(origin,maximum)); float4 b=resources.source_image.read(min(origin+uint2(1u,0u),maximum));
	float4 c=resources.source_image.read(min(origin+uint2(0u,1u),maximum)); float4 d=resources.source_image.read(min(origin+uint2(1u,1u),maximum));
	float4 result=(a+b+c+d)*0.25f;
	if(pc.srgb!=0u) result.rgb=linear_to_srgb((srgb_to_linear(a.rgb)+srgb_to_linear(b.rgb)+srgb_to_linear(c.rgb)+srgb_to_linear(d.rgb))*0.25f);
	resources.destination_image.write(result,p);
}"#;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::resources::mips::{
		bc7::tests::{decode_image, encode_on_cpu, psnr, test_image},
		generate_owned_lower_mip_chain,
	};

	fn generator() -> MaterialMipGenerator {
		MaterialMipGenerator::try_with_default_gpu()
			.expect("A compatible GPU is required for the offline GPU mip integration test")
	}

	/// Encodes a whole chain with the GPU backend and returns it with its size checked.
	fn encode_chain(
		generator: &MaterialMipGenerator,
		format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base: &[u8],
	) -> Vec<u8> {
		let size = encoded_mip_chain_size(format, Extent::rectangle(width, height)).expect("the format should be storable");
		let mut output = vec![0; size];
		generator
			.encode_mip_chain(format, gamma, width, height, base, &mut output)
			.expect("GPU mip chain encoding should succeed");
		output
	}

	#[test]
	fn gpu_worker_generates_complete_uniform_mip_chain() {
		let generator = generator();
		let base = [64_u8, 128, 192, 255].repeat(8 * 4);

		let chain = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 8, 4, &base);

		// Levels 8x4, 4x2, 2x1, and 1x1 follow each other.
		assert_eq!(chain.len(), (32 + 8 + 2 + 1) * 4);
		assert!(chain.as_chunks::<4>().0.iter().all(|pixel| *pixel == [64, 128, 192, 255]));

		let reused = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 8, 4, &base);

		assert_eq!(reused, chain, "the cached GPU pyramid should serve another bake");
	}

	#[test]
	fn gpu_sampler_filters_at_each_two_by_two_block_center() {
		let generator = generator();
		let base = (0_u8..16)
			.flat_map(|value| [value * 4, value * 4, value * 4, 255])
			.collect::<Vec<_>>();

		let chain = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 4, 4, &base);

		let lower_levels = &chain[16 * 4..];
		assert_eq!(
			lower_levels,
			[
				10, 10, 10, 255, 18, 18, 18, 255, 42, 42, 42, 255, 50, 50, 50, 255, 30, 30, 30, 255
			]
		);
	}

	#[test]
	fn gpu_worker_filters_srgb_in_linear_light() {
		let generator = generator();
		let base = [0, 0, 0, 0, 255, 255, 255, 64, 0, 0, 0, 128, 255, 255, 255, 255];

		let chain = encode_chain(&generator, Formats::RGBA8SRGB, Gamma::SRGB, 2, 2, &base);

		assert_eq!(&chain[16..], [188, 188, 188, 112]);
	}

	#[test]
	fn gpu_bc7_chain_matches_the_cpu_fast_profile_on_every_level() {
		let generator = generator();
		let (width, height) = (32, 24);
		let base = test_image(width, height, false);

		let chain = encode_chain(&generator, Formats::BC7, Gamma::Linear, width, height, &base);
		let mut reference = vec![0; chain.len()];
		CPUMipGenerationBackend
			.encode_mip_chain(Formats::BC7, Gamma::Linear, width, height, &base, &mut reference)
			.expect("CPU mip chain encoding should succeed");

		// Each encoder is measured against the levels its own backend filtered. The GPU filter can round a texel
		// differently from the CPU filter, and that difference isn't compression error.
		let gpu_levels = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, width, height, &base);
		let cpu_levels = generate_owned_lower_mip_chain(Formats::RGBA8, Gamma::Linear, width, height, &base)
			.expect("CPU mip filtering should succeed");
		let cpu_sources = std::iter::once(base.as_slice()).chain(cpu_levels.levels().map(|level| level.data));
		let (mut offset, mut texel_offset) = (0, 0);
		for ((level_width, level_height), cpu_source) in mip_extents(width, height).zip(cpu_sources) {
			let extent = Extent::rectangle(level_width, level_height);
			let size = encoded_mip_level_size(Formats::BC7, extent).unwrap();
			let texel_size = encoded_mip_level_size(Formats::RGBA8, extent).unwrap();
			let gpu_source = &gpu_levels[texel_offset..texel_offset + texel_size];
			let gpu = psnr(
				gpu_source,
				&decode_image(level_width, level_height, chain[offset..offset + size].as_chunks().0),
				3,
			);
			let cpu = psnr(
				cpu_source,
				&decode_image(level_width, level_height, reference[offset..offset + size].as_chunks().0),
				3,
			);
			assert!(
				gpu > cpu - 0.5,
				"level {level_width}x{level_height}: GPU {gpu:.2} dB, CPU {cpu:.2} dB"
			);
			offset += size;
			texel_offset += texel_size;
		}
		assert_eq!(offset, chain.len());
	}

	#[test]
	fn gpu_bc7_encodes_transparent_texels_and_one_texel_levels() {
		let generator = generator();
		let base = test_image(4, 4, true);

		let chain = encode_chain(&generator, Formats::BC7SRGB, Gamma::SRGB, 4, 4, &base);

		// 4x4, 2x2, and 1x1 each take one block.
		assert_eq!(chain.len(), 3 * 16);
		let gpu = psnr(&base, &decode_image(4, 4, chain[..16].as_chunks().0), 4);
		let cpu = psnr(&base, &decode_image(4, 4, &encode_on_cpu(4, 4, &base)), 4);
		assert!(gpu > cpu - 0.5, "GPU {gpu:.2} dB, CPU {cpu:.2} dB");
	}
}

const GPU_MIP_HLSL: &str = r#"Texture2D<float4> source_image : register(t0,space0);
SamplerState source_sampler : register(s0,space0);
RWTexture2D<float4> destination_image : register(u1,space0);
struct PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; uint srgb; };
ConstantBuffer<PushConstants> pc : register(b0,space0);
float3 srgb_to_linear(float3 color) { float3 low=color/12.92; float3 high=pow((color+0.055)/1.055,2.4); return lerp(high,low,step(color,0.04045)); }
float3 linear_to_srgb(float3 color) { float3 low=color*12.92; float3 high=1.055*pow(color,1.0/2.4)-0.055; return lerp(high,low,step(color,0.0031308)); }
[numthreads(8,8,1)] void generate_mip(uint3 invocation_id : SV_DispatchThreadID) {
	uint2 p=invocation_id.xy; if(p.x>=pc.destination_width||p.y>=pc.destination_height) return;
	uint2 maximum=uint2(pc.source_width-1u,pc.source_height-1u); uint2 origin=p*2u;
	float4 a=source_image.Load(int3(min(origin,maximum),0)); float4 b=source_image.Load(int3(min(origin+uint2(1u,0u),maximum),0));
	float4 c=source_image.Load(int3(min(origin+uint2(0u,1u),maximum),0)); float4 d=source_image.Load(int3(min(origin+uint2(1u,1u),maximum),0));
	float4 result=(a+b+c+d)*0.25;
	if(pc.srgb!=0u) result.rgb=linear_to_srgb((srgb_to_linear(a.rgb)+srgb_to_linear(b.rgb)+srgb_to_linear(c.rgb)+srgb_to_linear(d.rgb))*0.25);
	destination_image[p]=result;
}"#;

use std::{alloc::Global, any::Any, error::Error, fmt};

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
	CPUMipGenerationBackend, MipGenerationBackend, MipGenerationError, bc7::BC7_ENCODER, encode_level_in,
	encoded_mip_chain_size, encoded_mip_level_size, filtering_format, mip_extents,
};
use crate::{
	gpu_worker::{
		ComputeContextError, ComputeKernelError, GpuWorker, GpuWorkerSpawnError, OUTPUT_SLOT, OwnedContext, SOURCE_SLOT,
		create_besl_compute_kernel, create_compute_context, create_compute_kernel,
	},
	types::{Formats, Gamma},
};
