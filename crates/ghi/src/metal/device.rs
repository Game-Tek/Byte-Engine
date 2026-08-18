/// The `Device` struct owns the Metal GPU entry point for the macOS 26-only backend.
///
/// The backend calls Metal 4 directly and does not provide an older Metal fallback.
pub struct Device {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) queues: Vec<queue::StoredQueue>,
	pub settings: crate::device::Features,
}

impl Device {
	pub fn new(
		settings: crate::device::Features,
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Self, &'static str> {
		let mut created_queues = Vec::with_capacity(queues.len());

		for (selection, output_handle) in queues.iter_mut() {
			let workloads = select_metal_command_queue_workloads(device.as_ref(), selection.r#type)?;
			let queue = device.newMTL4CommandQueue().ok_or(
				"Metal 4 command queue creation failed. The most likely cause is that the device ran out of command queue resources.",
			)?;
			let handle = graphics_hardware_interface::QueueHandle(created_queues.len() as u64);

			created_queues.push(queue::StoredQueue::new(queue, workloads));

			**output_handle = Some(handle);
		}

		Ok(Self {
			device,
			queues: created_queues,
			settings,
		})
	}
}
impl crate::device::Device for Device {
	type Context = crate::metal::context::Context;
	type RasterPipeline = crate::metal::factory::RasterPipeline;
	type ComputePipeline = crate::metal::factory::ComputePipeline;
	type Image = crate::metal::factory::FactoryImage;
	type Sampler = crate::metal::factory::FactorySampler;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Self::Context::new(self.settings, self.device.clone(), self.queues.clone())
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		_shader_source_type: crate::shader::Sources,
		_stage: crate::ShaderTypes,
		_shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		panic!(
			"Metal device shader creation moved to Factory. The most likely cause is that resource construction is using Device instead of Context or Factory."
		);
	}

	fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		panic!(
			"Metal device raster pipeline creation moved to Factory. The most likely cause is that resource construction is using Device instead of Context or Factory."
		);
	}

	fn create_compute_pipeline(&mut self, _builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		panic!(
			"Metal device compute pipeline creation moved to Factory. The most likely cause is that resource construction is using Device instead of Context or Factory."
		);
	}

	fn build_image(&mut self, _builder: crate::image::Builder) -> Self::Image {
		panic!(
			"Metal device image creation moved to Factory. The most likely cause is that resource construction is using Device instead of Context or Factory."
		);
	}

	fn build_sampler(&mut self, _builder: crate::sampler::Builder) -> Self::Sampler {
		panic!(
			"Metal device sampler creation moved to Factory. The most likely cause is that resource construction is using Device instead of Context or Factory."
		);
	}
}

pub(super) fn select_metal_command_queue_workloads(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	requested: crate::WorkloadTypes,
) -> Result<crate::WorkloadTypes, &'static str> {
	if requested.is_empty() {
		return Err("Failed to create a Metal command queue. The requested queue selection did not include any workload type.");
	}

	if requested.intersects(crate::WorkloadTypes::VIDEO) {
		return Err(
			"Failed to create a Metal 4 command queue. Metal video work is not exposed through the graphics command queue in this backend.",
		);
	}

	if requested.intersects(crate::WorkloadTypes::IO) {
		return Err(
			"Failed to create a Metal 4 command queue. Metal IO uses MTLIOCommandQueue and is not compatible with this command-buffer queue path.",
		);
	}

	let mut supported = crate::WorkloadTypes::RASTER | crate::WorkloadTypes::COMPUTE | crate::WorkloadTypes::TRANSFER;

	if requested.intersects(crate::WorkloadTypes::RAY_TRACING) && device.supportsRaytracing() {
		supported |= crate::WorkloadTypes::RAY_TRACING;
	}

	if !supported.contains(requested) {
		return Err(
			"Failed to create a Metal command queue. The requested workload type is not supported by the selected Metal device.",
		);
	}

	Ok(requested)
}

#[derive(Clone)]
pub struct Pipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn MTLDepthStencilState>>>,
	pub(crate) layout: PipelineLayout,
	pub(crate) vertex_layout: Option<VertexLayout>,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
}

unsafe impl Send for Pipeline {}

#[derive(Clone)]
pub struct ComputePipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn MTLDepthStencilState>>>,
	pub(crate) layout: PipelineLayout,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
}

unsafe impl Send for ComputePipeline {}

/// The `Image` struct carries a Metal image built before it has a public GHI handle.
pub struct Image {
	pub(crate) image: crate::metal::image::Image,
}

unsafe impl Send for Image {}

/// The `Sampler` struct carries a Metal sampler built before it has a public GHI handle.
pub struct Sampler {
	pub(crate) sampler: crate::metal::sampler::Sampler,
}

unsafe impl Send for Sampler {}

use objc2_metal::{MTLDepthStencilState, MTLDevice};

use super::*;
