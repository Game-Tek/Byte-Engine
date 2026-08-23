const SOURCE_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
const OUTPUT_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);

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

/// The `MaterialMipGenerator` struct provides GPU generation with a deterministic CPU fallback for imported material textures.
pub struct MaterialMipGenerator {
	client: GPUMipClient,
}

impl MaterialMipGenerator {
	/// Creates the dedicated offline GPU worker used by material importers.
	pub fn try_with_default_gpu() -> Result<Self, GPUMipError> {
		GPUMipClient::spawn(GPUMipProcessor::try_new).map(|client| Self { client })
	}
}

impl MipGenerationBackend for MaterialMipGenerator {
	fn generate_lower_levels(
		&self,
		format: Formats,
		width: u32,
		height: u32,
		base_level: &[u8],
	) -> Result<OwnedMipChain, MipGenerationError> {
		if format == Formats::RGBA8 {
			match self.client.generate(width, height, base_level) {
				Ok(levels) => return Ok(levels),
				Err(error) => log::warn!(
					"GPU material mip generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
				),
			}
		}

		// The GPU storage path is RGBA8. Preserve support for uncommon 16-bit imported textures through the existing filter.
		generate_owned_lower_mip_chain(format, width, height, base_level)
	}
}

struct GPUMipClient {
	sender: SyncSender<WorkerMessage>,
	responses: Mutex<mpsc::Receiver<Result<OwnedMipChain, GPUMipError>>>,
	worker: Option<JoinHandle<()>>,
}

impl GPUMipClient {
	fn spawn(initialize: impl FnOnce() -> Result<GPUMipProcessor, GPUMipError> + Send + 'static) -> Result<Self, GPUMipError> {
		let (sender, receiver) = mpsc::sync_channel(1);
		let (response_sender, responses) = mpsc::sync_channel(1);
		let (startup, startup_receiver) = mpsc::sync_channel(1);
		let worker = std::thread::Builder::new()
			.name("GPU Material Mip Worker".to_string())
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
						WorkerMessage::Generate(request) => {
							// The caller waits synchronously, so its immutable base-level borrow remains valid.
							let source = unsafe { std::slice::from_raw_parts(request.data, request.len) };
							if response_sender
								.send(processor.generate(request.width, request.height, source))
								.is_err()
							{
								return;
							}
						}
						WorkerMessage::Shutdown => return,
					}
				}
			})
			.map_err(|error| GPUMipError::WorkerCreation(error.to_string()))?;
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
				Err(GPUMipError::WorkerUnavailable)
			}
		}
	}

	fn generate(&self, width: u32, height: u32, data: &[u8]) -> Result<OwnedMipChain, GPUMipError> {
		let responses = self.responses.lock().map_err(|_| GPUMipError::WorkerUnavailable)?;
		self.sender
			.send(WorkerMessage::Generate(GPUMipRequest {
				width,
				height,
				data: data.as_ptr(),
				len: data.len(),
			}))
			.map_err(|_| GPUMipError::WorkerUnavailable)?;
		responses.recv().map_err(|_| GPUMipError::WorkerUnavailable)?
	}
}

impl Drop for GPUMipClient {
	fn drop(&mut self) {
		let _ = self.sender.send(WorkerMessage::Shutdown);
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

struct GPUMipRequest {
	width: u32,
	height: u32,
	data: *const u8,
	len: usize,
}
// SAFETY: the submitting method waits for the worker response before returning, and the source is immutably borrowed.
unsafe impl Send for GPUMipRequest {}
enum WorkerMessage {
	Generate(GPUMipRequest),
	Shutdown,
}

struct Construction {
	context: ghi::implementation::Context,
	owner: Box<dyn Any>,
}

/// The `GPUMipProcessor` struct owns the thread-confined compute context used for offline box filtering.
pub struct GPUMipProcessor {
	context: ghi::implementation::Context,
	pipeline: ghi::PipelineHandle,
	queue: ghi::QueueHandle,
	sampler: ghi::SamplerHandle,
	scratch: Vec<GPUMipScratch>,
	_owner: Box<dyn Any>,
}

impl GPUMipProcessor {
	fn try_new() -> Result<Self, GPUMipError> {
		let features = ghi::device::Features::new().mesh_shading(false);
		let mut instance = ghi::implementation::Instance::new(features).map_err(GPUMipError::InstanceCreation)?;
		let mut queue = None;
		let device = instance
			.create_device(
				features,
				&mut [(
					ghi::QueueSelection::new(ghi::WorkloadTypes::COMPUTE | ghi::WorkloadTypes::TRANSFER),
					&mut queue,
				)],
			)
			.map_err(GPUMipError::DeviceCreation)?;
		let context = device.create_context().map_err(GPUMipError::ContextCreation)?;
		Self::from_parts(
			context,
			queue.expect("GHI device creation must populate the compute queue."),
			(device, instance),
		)
	}

	fn from_parts<Owner: 'static>(
		context: ghi::implementation::Context,
		queue: ghi::QueueHandle,
		owner: Owner,
	) -> Result<Self, GPUMipError> {
		let mut construction = Construction {
			context,
			owner: Box::new(owner),
		};
		let compiled = ghi::shader::compile(
			"GPU material mip generation",
			ghi::shader::ShaderSource::PlatformNative {
				glsl: GPU_MIP_GLSL,
				msl: GPU_MIP_MSL,
				msl_entry_point: "generate_mip",
				hlsl: GPU_MIP_HLSL,
				hlsl_entry_point: "generate_mip",
			},
		)
		.map_err(GPUMipError::ShaderCompilation)?;
		let resources = [
			ghi::ShaderResourceDescriptor::single(
				SOURCE_SLOT,
				ghi::ResourceKind::CombinedImageSampler,
				ghi::AccessPolicies::READ,
			),
			ghi::ShaderResourceDescriptor::single(OUTPUT_SLOT, ghi::ResourceKind::StorageImage, ghi::AccessPolicies::WRITE),
		];
		let shader = construction
			.context
			.create_shader(
				Some("GPU material mip generation"),
				compiled.as_source(),
				ghi::ShaderTypes::Compute,
				resources,
			)
			.map_err(|_| GPUMipError::ShaderCreation)?;
		let ranges = [ghi::pipelines::PushConstantRange::new(
			0,
			std::mem::size_of::<PushConstants>() as u32,
		)];
		let pipeline = construction.context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(&ranges, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute))
				.name("GPU material mip generation"),
		);
		let sampler = construction.context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.max_lod(0.0),
		);
		let Construction { context, owner } = construction;
		Ok(Self {
			context,
			pipeline,
			queue,
			sampler,
			scratch: Vec::new(),
			_owner: owner,
		})
	}

	fn generate(&mut self, width: u32, height: u32, base: &[u8]) -> Result<OwnedMipChain, GPUMipError> {
		let expected = width as usize * height as usize * 4;
		if base.len() != expected {
			return Err(GPUMipError::UploadSizeMismatch {
				expected,
				got: base.len(),
			});
		}
		if width <= 1 && height <= 1 {
			return Ok(OwnedMipChain::empty());
		}

		let (context, scratch_cache) = (&mut self.context, &mut self.scratch);
		let scratch_index = scratch_cache
			.iter()
			.position(|scratch| scratch.width == width && scratch.height == height)
			.unwrap_or_else(|| {
				scratch_cache.push(create_scratch(context, self.queue, self.sampler, width, height));
				scratch_cache.len() - 1
			});
		let scratch = &scratch_cache[scratch_index];
		let upload = context.get_texture_slice_mut(scratch.base_image);
		if upload.len() != expected {
			return Err(GPUMipError::UploadSizeMismatch {
				expected,
				got: upload.len(),
			});
		}
		upload.copy_from_slice(base);
		context.sync_texture(scratch.base_image);

		let mut command_buffer = context.command_buffer(scratch.command_buffer);
		let mut recording = command_buffer.create_command_buffer_recording();
		for level in &scratch.levels {
			let command = recording.bind_compute_pipeline(self.pipeline);
			command.bind_descriptor_sets(&[level.descriptor_set]);
			command.write_push_constant(
				0,
				PushConstants {
					source_width: level.source_width,
					source_height: level.source_height,
					destination_width: level.width,
					destination_height: level.height,
				},
			);
			command.dispatch(ghi::DispatchExtent::new(
				Extent::rectangle(level.width, level.height),
				Extent::rectangle(8, 8),
			));
		}
		let handles = scratch
			.readback_images
			.iter()
			.copied()
			.map(|image| recording.transfer_texture(image.into()))
			.collect::<Result<Vec<_>, _>>()
			.map_err(|_| GPUMipError::GPUExecution)?;
		recording.execute(scratch.synchronizer);
		context.wait_for_synchronizer(scratch.synchronizer);
		#[cfg(any(debug_assertions, test))]
		if context.has_errors() {
			return Err(GPUMipError::GPUExecution);
		}

		let total_size = scratch.levels.iter().try_fold(0usize, |size, level| {
			size.checked_add(level.width as usize * level.height as usize * 4)
				.ok_or(GPUMipError::GPUExecution)
		})?;
		let mut data = vec![0_u8; total_size];
		let mut offset = 0usize;
		for (level, handle) in scratch.levels.iter().zip(handles) {
			let size = level.width as usize * level.height as usize * 4;
			let readback = context.get_image_data(handle).map_err(|_| GPUMipError::GPUExecution)?;
			if readback.bytes.len() < size {
				return Err(GPUMipError::ReadbackSizeMismatch {
					expected: size,
					got: readback.bytes.len(),
				});
			}
			data[offset..offset + size].copy_from_slice(&readback.bytes[..size]);
			offset += size;
		}
		OwnedMipChain::from_packed_rgba8_lower_levels(width, height, data.into_boxed_slice())
			.map_err(|_| GPUMipError::GPUExecution)
	}
}

/// The `GPUMipScratch` struct retains one dimension-specific GPU pyramid across material bakes.
struct GPUMipScratch {
	width: u32,
	height: u32,
	base_image: ghi::ImageHandle,
	levels: Vec<GPUMipScratchLevel>,
	readback_images: Vec<ghi::BaseImageHandle>,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
}

#[derive(Clone, Copy)]
struct GPUMipScratchLevel {
	descriptor_set: ghi::DescriptorSetHandle,
	source_width: u32,
	source_height: u32,
	width: u32,
	height: u32,
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
	let mut readback_images = Vec::with_capacity(levels.capacity());
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
			descriptor_set,
			source_width,
			source_height,
			width: destination_width,
			height: destination_height,
		});
		readback_images.push(destination.into());
		source = destination;
		(source_width, source_height) = (destination_width, destination_height);
	}
	let command_buffer = context
		.queue_reference(queue)
		.create_command_buffer(Some("Generate material mips"));
	let synchronizer = context.create_synchronizer(Some("Material mips generated"), true);
	GPUMipScratch {
		width,
		height,
		base_image,
		levels,
		readback_images,
		command_buffer,
		synchronizer,
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PushConstants {
	source_width: u32,
	source_height: u32,
	destination_width: u32,
	destination_height: u32,
}

const GPU_MIP_GLSL: &str = r#"#version 460
#pragma shader_stage(compute)
layout(local_size_x=8, local_size_y=8, local_size_z=1) in;
layout(set=0,binding=0) uniform sampler2D source_image;
layout(rgba8,set=0,binding=1) uniform writeonly image2D destination_image;
layout(push_constant) uniform PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; } pc;
void generate_mip() {
	uvec2 p=gl_GlobalInvocationID.xy; if(any(greaterThanEqual(p,uvec2(pc.destination_width,pc.destination_height)))) return;
	// The normalized center between four source texels lets the fixed-function bilinear sampler perform the box filter.
	vec2 uv=vec2(p*2u+1u)/vec2(pc.source_width,pc.source_height);
	imageStore(destination_image,ivec2(p),textureLod(source_image,uv,0.0));
}"#;

const GPU_MIP_MSL: &str = r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(compute)
// besl-threadgroup-size:8,8,1
struct Resources { texture2d<float,access::sample> source_image [[id(0)]]; sampler source_sampler [[id(1)]]; texture2d<float,access::write> destination_image [[id(2)]]; };
struct PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; };
kernel void generate_mip(uint3 invocation_id [[thread_position_in_grid]], constant PushConstants& pc [[buffer(15)]], constant Resources& resources [[buffer(16)]]) {
	uint2 p=invocation_id.xy; if(p.x>=pc.destination_width||p.y>=pc.destination_height) return;
	// The normalized center between four source texels lets the fixed-function bilinear sampler perform the box filter.
	float2 uv=float2(p*2u+1u)/float2(pc.source_width,pc.source_height);
	resources.destination_image.write(resources.source_image.sample(resources.source_sampler,uv,level(0.0f)),p);
}"#;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gpu_worker_generates_complete_uniform_mip_chain() {
		let generator = MaterialMipGenerator::try_with_default_gpu()
			.expect("A compatible GPU is required for the offline GPU mip integration test");
		let base = [64_u8, 128, 192, 255].repeat(8 * 4);
		let levels = generator
			.generate_lower_levels(Formats::RGBA8, 8, 4, &base)
			.expect("GPU mip generation should succeed");

		assert_eq!(
			levels.levels().map(|level| (level.width, level.height)).collect::<Vec<_>>(),
			vec![(4, 2), (2, 1), (1, 1)]
		);
		for level in levels.levels() {
			assert!(level.data.chunks_exact(4).all(|pixel| pixel == [64, 128, 192, 255]));
		}

		let reused = generator
			.generate_lower_levels(Formats::RGBA8, 8, 4, &base)
			.expect("the cached GPU pyramid should support another bake");

		assert_eq!(reused.levels().count(), 3);
	}

	#[test]
	fn gpu_sampler_filters_at_each_two_by_two_block_center() {
		let generator = MaterialMipGenerator::try_with_default_gpu()
			.expect("A compatible GPU is required for the offline GPU mip integration test");
		let base = (0_u8..16)
			.flat_map(|value| [value * 4, value * 4, value * 4, 255])
			.collect::<Vec<_>>();
		let levels = generator
			.generate_lower_levels(Formats::RGBA8, 4, 4, &base)
			.expect("GPU sampler mip generation should succeed");
		let mut levels = levels.levels();

		let first = levels.next().expect("the 2x2 level should exist");

		assert_eq!(
			first.data,
			[10, 10, 10, 255, 18, 18, 18, 255, 42, 42, 42, 255, 50, 50, 50, 255,]
		);
		assert_eq!(levels.next().expect("the 1x1 level should exist").data, [30, 30, 30, 255]);
	}
}

const GPU_MIP_HLSL: &str = r#"Texture2D<float4> source_image : register(t0,space0);
SamplerState source_sampler : register(s0,space0);
RWTexture2D<float4> destination_image : register(u1,space0);
struct PushConstants { uint source_width; uint source_height; uint destination_width; uint destination_height; };
ConstantBuffer<PushConstants> pc : register(b0,space0);
[numthreads(8,8,1)] void generate_mip(uint3 invocation_id : SV_DispatchThreadID) {
	uint2 p=invocation_id.xy; if(p.x>=pc.destination_width||p.y>=pc.destination_height) return;
	// The normalized center between four source texels lets the fixed-function bilinear sampler perform the box filter.
	float2 uv=float2(p*2u+1u)/float2(pc.source_width,pc.source_height);
	destination_image[p]=source_image.SampleLevel(source_sampler,uv,0.0);
}"#;

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

use super::{generate_owned_lower_mip_chain, MipGenerationBackend, MipGenerationError, OwnedMipChain};
use crate::types::Formats;
