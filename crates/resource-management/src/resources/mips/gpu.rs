/// The `GPUMipError` enum identifies why offline material mip generation could not use the GPU path.
#[derive(Debug)]
pub enum GPUMipError {
	/// The shared GPU worker could not set up the mip processor or serve the request.
	Worker(GpuWorkerError),
	/// The request is outside what the GPU path serves, such as a 16-bit format or BC7 without its kernel; the CPU path
	/// serves it.
	UnsupportedRequest,
	UploadSizeMismatch {
		expected: usize,
		got: usize,
	},
	ReadbackSizeMismatch {
		expected: usize,
		got: usize,
	},
	GPUExecution,
}

impl fmt::Display for GPUMipError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Worker(error) => error.describe("mip", formatter),
			Self::UnsupportedRequest => formatter.write_str("GPU mip request is unsupported. The most likely cause is a 16-bit filtering format, buffers that don't match the request's extent, or a BC7 request after the BC7 kernel failed to compile."),
			Self::UploadSizeMismatch { expected, got } => write!(formatter, "GPU mip upload has the wrong size: expected {expected}, got {got}. The most likely cause is a staging allocation that does not match the base image."),
			Self::ReadbackSizeMismatch { expected, got } => write!(formatter, "GPU mip readback has the wrong size: expected {expected}, got {got}. The most likely cause is incomplete texture readback."),
			Self::GPUExecution => formatter.write_str("GPU mip generation failed. The most likely cause is a graphics backend validation or command-execution error."),
		}
	}
}

impl Error for GPUMipError {}

impl From<GpuWorkerError> for GPUMipError {
	fn from(error: GpuWorkerError) -> Self {
		Self::Worker(error)
	}
}

/// The `MaterialMipGenerator` struct moves material texture mip filtering and BC7 compression onto the GPU.
///
/// Install it on material importers as [`crate::resources::mips::MipGenerator::Gpu`]. Requests it can't serve report
/// [`GPUMipError::UnsupportedRequest`], and the generator's caller falls back to the CPU path for those and for failed
/// GPU work.
pub struct MaterialMipGenerator {
	worker: GpuWorker<GPUMipProcessor>,
}

impl MaterialMipGenerator {
	/// Creates the dedicated offline GPU worker used by material importers.
	pub fn try_with_default_gpu() -> Result<Self, GPUMipError> {
		Ok(Self {
			worker: GpuWorker::spawn("GPU Material Mip Worker", GPUMipProcessor::try_new)?,
		})
	}

	/// Filters `base_level` on the GPU and writes the chain, encoded as `output_format`, into `output`.
	///
	/// The contract is that of [`crate::resources::mips::MipGenerator::encode_mip_chain`]. The GPU filters RGBA8
	/// levels, so 16-bit filtering formats and mismatched buffer sizes return [`GPUMipError::UnsupportedRequest`] at
	/// once. BC7 chains come back encoded; other formats come back as filtered levels that are encoded here. The call
	/// suspends while the worker serves the request instead of blocking the runtime thread.
	pub(super) async fn encode_mip_chain(
		&self,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
	) -> Result<(), GPUMipError> {
		let extent = Extent::rectangle(width, height);
		let sizes_match = base_level.len() == width as usize * height as usize * 4
			&& Some(output.len()) == encoded_mip_chain_size(output_format, extent);
		if filtering_format(output_format) != Formats::RGBA8 || !sizes_match {
			return Err(GPUMipError::UnsupportedRequest);
		}
		let gpu_encodes = encodes_on_gpu(output_format);
		// Without GPU compression a lone texel has no lower levels, so there is nothing to submit.
		if !gpu_encodes && width <= 1 && height <= 1 {
			encode_levels(output_format, width, height, 4, base_level, &[], output);
			return Ok(());
		}

		// The worker owns its bytes while the request is in flight, so the base level is copied in and the result is
		// copied out. Both copies run on the calling pool thread, which has many siblings, rather than on the single
		// worker thread.
		let bytes = self
			.worker
			.submit((width, height, gamma, output_format), base_level.to_vec())
			.await
			.map_err(|_| GpuWorkerError::Unavailable)??;
		if gpu_encodes {
			output.copy_from_slice(&bytes);
		} else {
			// The base level never left the caller, so it's encoded from the caller's copy; the rest from the readback.
			encode_levels(output_format, width, height, 4, base_level, &bytes, output);
		}
		Ok(())
	}
}

/// Returns whether the GPU encodes chains of `format` itself, instead of reading filtered levels back for the CPU.
fn encodes_on_gpu(format: Formats) -> bool {
	matches!(format, Formats::BC7 | Formats::BC7SRGB)
}

/// The `GPUMipProcessor` struct owns the thread-confined compute context used for offline filtering and compression.
pub struct GPUMipProcessor {
	gpu: OwnedContext,
	mip_pipeline: ghi::PipelineHandle,
	/// The BC7 pipeline, compiled on the first BC7 request. `Some(None)` records a failed compilation, after which BC7
	/// requests are unsupported and take the CPU path.
	block_pipeline: Option<Option<ghi::PipelineHandle>>,
	queue: ghi::QueueHandle,
	sampler: ghi::SamplerHandle,
	/// Pyramids, each idle or bound to one in-flight request. An extent gains another pyramid only while all of its
	/// existing ones are busy, so it holds at most [`GpuProcessor::MAX_IN_FLIGHT`] of them. The GHI can't destroy
	/// images or buffers yet, so pyramids are never freed.
	scratch: Vec<GPUMipScratch>,
}

/// The `InFlight` enum records where a running request's result lands so [`GpuProcessor::poll`] can collect it.
enum InFlight {
	/// The BC7 block buffer holds the encoded chain.
	Blocks(ghi::BufferHandle<[[u32; 4]]>),
	/// The readbacks hold the filtered levels, one per level below the base.
	Levels(Vec<ghi::TextureCopyHandle>),
}

impl GPUMipProcessor {
	fn try_new() -> Result<Self, GPUMipError> {
		let (mut gpu, queue) = create_compute_context()?;
		let mip_pipeline = create_compute_kernel(
			&mut gpu.context,
			"GPU material mip generation",
			ghi::shader::ShaderSource::PlatformNative {
				glsl: GPU_MIP_GLSL,
				msl: GPU_MIP_MSL,
				msl_entry_point: "generate_mip",
				hlsl: GPU_MIP_HLSL,
				hlsl_entry_point: "generate_mip",
			},
			std::mem::size_of::<PushConstants>(),
		)?;
		let sampler = gpu.context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.max_lod(0.0),
		);
		Ok(Self {
			gpu,
			mip_pipeline,
			block_pipeline: None,
			queue,
			sampler,
			scratch: Vec::new(),
		})
	}

	/// Returns the BC7 pipeline, compiling it on first use, or `None` when it can't be compiled.
	fn block_pipeline(&mut self) -> Option<ghi::PipelineHandle> {
		let context = &mut self.gpu.context;
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
			.map_err(GPUMipError::from)
			.inspect_err(|error| {
				log::warn!("GPU BC7 encoder is unavailable; BC7 textures filter and compress on the CPU. {error}")
			})
			.ok()
		})
	}

	/// Returns an idle pyramid for the extent, creating one when every existing one is busy.
	fn acquire_scratch(&mut self, width: u32, height: u32) -> usize {
		let idle = self
			.scratch
			.iter()
			.position(|scratch| scratch.in_flight.is_none() && scratch.width == width && scratch.height == height);
		idle.unwrap_or_else(|| {
			self.scratch
				.push(create_scratch(&mut self.gpu.context, self.queue, self.sampler, width, height));
			self.scratch.len() - 1
		})
	}
}

impl GpuProcessor for GPUMipProcessor {
	/// The base level's width and height, its gamma, and the stored format of the chain.
	type Request = (u32, u32, Gamma, Formats);
	/// The encoded chain for BC7, or the filtered levels below the base level back to back.
	type Result = Result<Vec<u8>, GPUMipError>;
	type Ticket = usize;

	/// Enough to overlap one texture's upload and readback with other textures' dispatches, while bounding how many
	/// pyramids one extent can accumulate.
	const MAX_IN_FLIGHT: usize = 4;

	/// Uploads the RGBA8 base level and records the filter dispatches, then either the BC7 dispatches into the block
	/// buffer or one readback per filtered level, on an idle pyramid of the request's extent.
	fn submit(&mut self, &(width, height, gamma, format): &Self::Request, base: &[u8]) -> Submission<usize, Self::Result> {
		let block_pipeline = match encodes_on_gpu(format).then(|| self.block_pipeline()) {
			// The kernel's failure was logged once when it was compiled; the caller takes the CPU path quietly.
			Some(None) => return Submission::Complete(Err(GPUMipError::UnsupportedRequest)),
			pipeline => pipeline.flatten(),
		};
		let scratch_index = self.acquire_scratch(width, height);

		let context = &mut self.gpu.context;
		let scratch = &mut self.scratch[scratch_index];
		if block_pipeline.is_some() && scratch.blocks.is_none() {
			scratch.blocks = Some(create_block_scratch(context, self.sampler, scratch));
		}

		let upload = context.get_texture_slice_mut(scratch.base_image);
		if upload.len() != base.len() {
			return Submission::Complete(Err(GPUMipError::UploadSizeMismatch {
				expected: base.len(),
				got: upload.len(),
			}));
		}
		upload.copy_from_slice(base);
		context.sync_texture(scratch.base_image);

		// The shader filters in linear light when this push constant is non-zero.
		let srgb = u32::from(gamma == Gamma::SRGB);
		let mut recording = context.create_command_buffer_recording(scratch.command_buffer);
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
		let in_flight = if let Some((pipeline, blocks)) = block_pipeline.zip(scratch.blocks.as_ref()) {
			for level in &blocks.levels {
				let command = recording.bind_compute_pipeline(pipeline);
				command.bind_descriptor_sets(&[level.descriptor_set]);
				command.write_push_constant(0, level.push_constants);
				command.dispatch(ghi::DispatchExtent::new(
					Extent::rectangle(level.push_constants.blocks_x, level.push_constants.blocks_y),
					Extent::square(BC7_WORKGROUP_SIZE),
				));
			}
			InFlight::Blocks(blocks.buffer)
		} else {
			let readbacks = scratch
				.levels
				.iter()
				.map(|level| recording.transfer_texture(level.image.into()))
				.collect::<Result<Vec<_>, _>>();
			match readbacks {
				Ok(readbacks) => InFlight::Levels(readbacks),
				// Dropping the recording abandons the readbacks it already registered.
				Err(_) => return Submission::Complete(Err(GPUMipError::GPUExecution)),
			}
		};
		recording.execute(scratch.synchronizer);
		scratch.in_flight = Some(in_flight);
		Submission::InFlight(scratch_index)
	}

	/// Copies the result of a completed request out of its pyramid.
	fn poll(&mut self, scratch_index: usize) -> Option<Self::Result> {
		let Self { gpu, scratch, .. } = self;
		let (context, scratch) = (&mut gpu.context, &mut scratch[scratch_index]);
		if !context.poll_synchronizer(scratch.synchronizer) {
			return None;
		}
		let in_flight = scratch.in_flight.take().expect("A polled pyramid has an in-flight request");
		#[cfg(debug_assertions)]
		if context.has_errors() {
			return Some(Err(GPUMipError::GPUExecution));
		}
		match in_flight {
			// The buffer holds every level's blocks back to back, which is the stored chain layout.
			InFlight::Blocks(buffer) => Some(Ok(
				bytemuck::cast_slice::<[u32; 4], u8>(context.get_buffer_slice(buffer)).to_vec()
			)),
			InFlight::Levels(readbacks) => {
				let mut levels = Vec::with_capacity(packed_lower_levels_size(scratch.width, scratch.height, 4));
				for (level, handle) in scratch.levels.iter().zip(readbacks) {
					let size = level.width as usize * level.height as usize * 4;
					let Ok(readback) = context.get_image_data(handle) else {
						return Some(Err(GPUMipError::GPUExecution));
					};
					let Some(source) = readback.bytes.get(..size) else {
						return Some(Err(GPUMipError::ReadbackSizeMismatch {
							expected: size,
							got: readback.bytes.len(),
						}));
					};
					levels.extend_from_slice(source);
				}
				Some(Ok(levels))
			}
		}
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
	/// The request running on this pyramid, until [`GpuProcessor::poll`] collects it.
	in_flight: Option<InFlight>,
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
	// Each level filters the one before it.
	let mut source = base_image;
	let levels = mip_extents(width, height)
		.zip(mip_extents(width, height).skip(1))
		.map(|((source_width, source_height), (width, height))| {
			let image = context.build_image(
				ghi::image::Builder::new(
					ghi::Formats::RGBA8UNORM,
					ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferSource,
				)
				.name("Material mip level")
				.extent(Extent::rectangle(width, height))
				.device_accesses(ghi::DeviceAccesses::DeviceToHost)
				.use_case(ghi::UseCases::STATIC),
			);
			let descriptor_set = context.create_descriptor_set(Some("Material mip level"));
			context.write(&[
				ghi::DescriptorWrite::combined_image_sampler(descriptor_set, SOURCE_SLOT, source, sampler, ghi::Layouts::Read),
				ghi::DescriptorWrite::image(descriptor_set, OUTPUT_SLOT, image, ghi::Layouts::General),
			]);
			source = image;
			GPUMipScratchLevel {
				image,
				descriptor_set,
				source_width,
				source_height,
				width,
				height,
			}
		})
		.collect();
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
		in_flight: None,
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
		MipGenerator,
		bc7::tests::{decode_image, encode_on_cpu, psnr, test_image},
		encoded_mip_level_size, generate_lower_levels, packed_lower_levels,
	};

	fn generator() -> MaterialMipGenerator {
		MaterialMipGenerator::try_with_default_gpu()
			.expect("A compatible GPU is required for the offline GPU mip integration test")
	}

	/// Encodes a whole chain with the GPU generator and returns it with its size checked.
	async fn encode_chain(
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
			.await
			.expect("GPU mip chain encoding should succeed");
		output
	}

	#[crate::r#async::test]
	async fn gpu_worker_generates_complete_uniform_mip_chain() {
		let generator = generator();
		let base = [64_u8, 128, 192, 255].repeat(8 * 4);

		let chain = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 8, 4, &base).await;

		// Levels 8x4, 4x2, 2x1, and 1x1 follow each other.
		assert_eq!(chain.len(), (32 + 8 + 2 + 1) * 4);
		assert!(chain.as_chunks::<4>().0.iter().all(|pixel| *pixel == [64, 128, 192, 255]));

		let reused = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 8, 4, &base).await;

		assert_eq!(reused, chain, "the cached GPU pyramid should serve another bake");
	}

	#[crate::r#async::test]
	async fn gpu_sampler_filters_at_each_two_by_two_block_center() {
		let generator = generator();
		let base = (0_u8..16)
			.flat_map(|value| [value * 4, value * 4, value * 4, 255])
			.collect::<Vec<_>>();

		let chain = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, 4, 4, &base).await;

		let lower_levels = &chain[16 * 4..];
		assert_eq!(
			lower_levels,
			[
				10, 10, 10, 255, 18, 18, 18, 255, 42, 42, 42, 255, 50, 50, 50, 255, 30, 30, 30, 255
			]
		);
	}

	#[crate::r#async::test]
	async fn gpu_worker_filters_srgb_in_linear_light() {
		let generator = generator();
		let base = [0, 0, 0, 0, 255, 255, 255, 64, 0, 0, 0, 128, 255, 255, 255, 255];

		let chain = encode_chain(&generator, Formats::RGBA8SRGB, Gamma::SRGB, 2, 2, &base).await;

		assert_eq!(&chain[16..], [188, 188, 188, 112]);
	}

	#[crate::r#async::test]
	async fn gpu_packed_rg8_chain_matches_the_rgba8_chain_prefix_on_every_level() {
		let generator = generator();
		let (width, height) = (8_u32, 8_u32);
		let base = (0..width * height)
			.flat_map(|index| [(index * 7 % 251) as u8, (index * 13 % 241) as u8, index as u8, 255])
			.collect::<Vec<_>>();

		let packed = encode_chain(&generator, Formats::RG8, Gamma::Linear, width, height, &base).await;
		let unpacked = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, width, height, &base).await;

		assert_eq!(packed.len(), (64 + 16 + 4 + 1) * 2);
		let expected = unpacked
			.as_chunks::<4>()
			.0
			.iter()
			.flat_map(|texel| [texel[0], texel[1]])
			.collect::<Vec<_>>();
		assert_eq!(
			packed, expected,
			"RG8 levels should be the GPU-filtered RGBA8 levels without B and A"
		);
	}

	#[crate::r#async::test]
	async fn gpu_bc7_chain_matches_the_cpu_fast_profile_on_every_level() {
		let generator = generator();
		let (width, height) = (32, 24);
		let base = test_image(width, height, false);

		let chain = encode_chain(&generator, Formats::BC7, Gamma::Linear, width, height, &base).await;
		let mut reference = vec![0; chain.len()];
		MipGenerator::Cpu
			.encode_mip_chain(Formats::BC7, Gamma::Linear, width, height, &base, &mut reference)
			.await
			.expect("CPU mip chain encoding should succeed");

		// Each encoder is measured against the levels its own path filtered. The GPU filter can round a texel
		// differently from the CPU filter, and that difference isn't compression error.
		let gpu_levels = encode_chain(&generator, Formats::RGBA8, Gamma::Linear, width, height, &base).await;
		let (_, cpu_levels) = generate_lower_levels(Formats::RGBA8, Gamma::Linear, width, height, &base)
			.expect("CPU mip filtering should succeed");
		let cpu_sources =
			std::iter::once(base.as_slice()).chain(packed_lower_levels(width, height, 4, &cpu_levels).map(|level| level.data));
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
			eprintln!("level {level_width}x{level_height}: GPU {gpu:.2} dB, CPU {cpu:.2} dB");
			assert!(
				gpu > cpu - 0.5,
				"level {level_width}x{level_height}: GPU {gpu:.2} dB, CPU {cpu:.2} dB"
			);
			offset += size;
			texel_offset += texel_size;
		}
		assert_eq!(offset, chain.len());
	}

	#[crate::r#async::test]
	async fn gpu_bc7_encodes_transparent_texels_and_one_texel_levels() {
		let generator = generator();
		let base = test_image(4, 4, true);

		let chain = encode_chain(&generator, Formats::BC7SRGB, Gamma::SRGB, 4, 4, &base).await;

		// 4x4, 2x2, and 1x1 each take one block.
		assert_eq!(chain.len(), 3 * 16);
		let gpu = psnr(&base, &decode_image(4, 4, chain[..16].as_chunks().0), 4);
		let cpu = psnr(&base, &decode_image(4, 4, &encode_on_cpu(4, 4, &base)), 4);
		assert!(gpu > cpu - 0.5, "GPU {gpu:.2} dB, CPU {cpu:.2} dB");
	}

	/// The mixed-size batch the concurrency tests submit: same-extent requests must share and reuse pyramids, and
	/// different extents must run side by side.
	fn batch() -> Vec<(Formats, Gamma, u32, u32, Vec<u8>)> {
		(0..12_u32)
			.map(|index| match index % 4 {
				0 => (Formats::BC7, Gamma::Linear, 64, 64),
				1 => (Formats::RG8, Gamma::Linear, 32, 32),
				2 => (Formats::BC7SRGB, Gamma::SRGB, 32, 32),
				_ => (Formats::BC5, Gamma::Linear, 64, 64),
			})
			.map(|(format, gamma, width, height)| (format, gamma, width, height, test_image(width, height, false)))
			.collect()
	}

	#[crate::r#async::test]
	async fn concurrent_chains_match_sequential_chains() {
		let generator = generator();
		let batch = batch();

		let mut sequential = Vec::new();
		for (format, gamma, width, height, base) in &batch {
			sequential.push(encode_chain(&generator, *format, *gamma, *width, *height, base).await);
		}
		// Every request is submitted before any completes, so the worker must overlap them and hand each its own slot.
		let concurrent = utils::r#async::join_all(
			batch
				.iter()
				.map(|(format, gamma, width, height, base)| encode_chain(&generator, *format, *gamma, *width, *height, base)),
		)
		.await;

		assert_eq!(concurrent, sequential);
	}

	/// Times a bake-sized batch submitted from eight runtime threads. Run it by hand to compare worker changes:
	/// `cargo nextest run -p byte-engine-resource-management --features gpu-mips concurrent_material_bakes --run-ignored all --no-capture`.
	#[test]
	#[ignore]
	fn concurrent_material_bakes_timing() {
		let generator = std::sync::Arc::new(generator());
		let jobs = (0..32_u32)
			.map(|index| match index % 4 {
				0 => (Formats::BC7, 1024, 1024),
				1 => (Formats::RG8, 512, 512),
				2 => (Formats::BC7SRGB, 512, 512),
				_ => (Formats::BC5, 1024, 1024),
			})
			.map(|(format, width, height)| (format, width, height, test_image(width, height, false)))
			.collect::<Vec<_>>();
		let run = |jobs: &[(Formats, u32, u32, Vec<u8>)]| {
			crate::r#async::Executor::new().unwrap().block_on(async {
				for (format, width, height, base) in jobs {
					encode_chain(&generator, *format, Gamma::Linear, *width, *height, base).await;
				}
			});
		};
		// Warm the pyramids so the measurement excludes scratch creation.
		run(&jobs[..4]);
		let start = std::time::Instant::now();
		run(&jobs);
		eprintln!("32 textures from one thread took {:?}", start.elapsed());
		let start = std::time::Instant::now();
		std::thread::scope(|scope| {
			for chunk in jobs.chunks(4) {
				scope.spawn(|| run(chunk));
			}
		});
		eprintln!("32 textures across 8 threads took {:?}", start.elapsed());
		// Retained uploads and staging show up as resident memory, so the harness reports it next to the time.
		let resident = std::process::Command::new("ps")
			.args(["-o", "rss=", "-p", &std::process::id().to_string()])
			.output()
			.ok()
			.and_then(|output| String::from_utf8(output.stdout).ok())
			.and_then(|rss| rss.trim().parse::<u64>().ok())
			.unwrap_or(0);
		eprintln!("resident memory after the batch: {} MB", resident / 1024);
	}

	/// Splits one 2K texture's bake into its CPU and GPU phases. Run it by hand to decide where to optimize:
	/// `cargo nextest run -p byte-engine-resource-management --features gpu-mips single_texture_phase_timing --run-ignored all --no-capture`.
	#[test]
	#[ignore]
	fn single_texture_phase_timing() {
		use std::time::Instant;

		let mut processor = GPUMipProcessor::try_new().expect("A compatible GPU is required");
		let (width, height) = (2048, 2048);
		// A smooth gradient stands in for the flat regions of real textures, where the search can stop early.
		let smooth = (0..width * height)
			.flat_map(|index| {
				let (x, y) = (index % width, index / width);
				[
					(x * 255 / width) as u8,
					(y * 255 / height) as u8,
					((x + y) * 127 / (width + height)) as u8,
					255,
				]
			})
			.collect::<Vec<u8>>();
		let noisy = test_image(width, height, false);
		for (format, base) in [
			(Formats::BC7, &noisy),
			(Formats::BC7, &smooth),
			(Formats::BC5, &noisy),
			(Formats::RGBA8, &noisy),
		] {
			let request = (width, height, Gamma::Linear, format);
			let mut output = vec![0; encoded_mip_chain_size(format, Extent::rectangle(width, height)).unwrap()];
			// The first request creates the pyramid, so time the second one.
			for pass in 0..2 {
				let start = Instant::now();
				let copy = base.to_vec();
				let copy_in = start.elapsed();
				let start = Instant::now();
				let Submission::InFlight(ticket) = processor.submit(&request, &copy) else {
					panic!("expected in flight")
				};
				let submit = start.elapsed();
				let start = Instant::now();
				while !processor
					.gpu
					.context
					.poll_synchronizer(processor.scratch[ticket].synchronizer)
				{
					std::hint::spin_loop();
				}
				let gpu = start.elapsed();
				let start = Instant::now();
				let readback = processor.poll(ticket).expect("finished").expect("ok");
				let copy_out = start.elapsed();
				let start = Instant::now();
				if encodes_on_gpu(format) {
					output.copy_from_slice(&readback);
				} else {
					encode_levels(format, width, height, 4, base, &readback, &mut output);
				}
				let cpu_encode = start.elapsed();
				if pass == 1 && format == Formats::BC7 {
					let level_size = encoded_mip_level_size(Formats::BC7, Extent::rectangle(width, height)).unwrap();
					let gpu = psnr(&base, &decode_image(width, height, output[..level_size].as_chunks().0), 3);
					let cpu = psnr(&base, &decode_image(width, height, &encode_on_cpu(width, height, &base)), 3);
					eprintln!("BC7 base level quality: GPU {gpu:.2} dB, CPU {cpu:.2} dB");
				}
				if pass == 1 {
					let image = if std::ptr::eq(base, &noisy) { "noisy" } else { "smooth" };
					eprintln!(
						"{format:?} {width}x{height} {image}: copy in {copy_in:?}, submit (staging copy + upload + record) {submit:?}, GPU {gpu:?}, copy out {copy_out:?}, CPU encode/copy {cpu_encode:?}"
					);
				}
			}
		}
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

use std::{error::Error, fmt};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
	device::Device as _,
	queue::Queue as _,
};
use utils::Extent;

use super::{bc7::BC7_ENCODER, encode_levels, encoded_mip_chain_size, filtering_format, mip_extents, packed_lower_levels_size};
use crate::{
	gpu_worker::{
		GpuProcessor, GpuWorker, GpuWorkerError, OUTPUT_SLOT, OwnedContext, SOURCE_SLOT, Submission,
		create_besl_compute_kernel, create_compute_context, create_compute_kernel,
	},
	types::{Formats, Gamma},
};
