//! Bakes BESL programs as asset bakes bake them, and creates the GHI device, shaders, and buffers that run them, for
//! the GPU tests beside this module.

use ghi::device::Device as _;
use utils::Extent;

#[cfg(target_os = "macos")]
use crate::shader::besl::backends::msl::MSL_ENTRY_POINT;
use crate::shader::{besl::backends::platform, generator::ShaderGenerationSettings};

/// Bakes `program` for the current platform, as asset bakes do, and returns the Metal library or the SPIR-V.
pub(super) async fn bake(program: &besl::NodeReference, settings: ShaderGenerationSettings) -> Box<[u8]> {
	let name = settings.name.clone();
	platform::Generator::new()
		.lower(&settings, program)
		.unwrap_or_else(|error| panic!("Expected {name} to lower. {error}"))
		.compile()
		.await
		.unwrap_or_else(|error| panic!("Expected {name} to compile. {error}"))
		.binary
}

/// Creates a GHI instance, device, and context with `features`, and one queue for `workloads`. Bind the result in
/// order, so the context drops before the device and the device before the instance.
pub(super) fn gpu_context(
	features: ghi::device::Features,
	workloads: ghi::WorkloadTypes,
) -> (
	ghi::implementation::Instance,
	ghi::implementation::Device,
	ghi::implementation::Context,
	ghi::QueueHandle,
) {
	let mut instance = ghi::implementation::Instance::new(features)
		.expect("Failed to create the GHI instance. The most likely cause is that no GPU device is available.");
	let mut queue_handle = None;
	let device = instance
		.create_device(features, &mut [(ghi::QueueSelection::new(workloads), &mut queue_handle)])
		.expect("Failed to create the GHI device. The most likely cause is a GPU without the features the test needs.");
	let context = device
		.create_context()
		.expect("Failed to create the GHI context. The most likely cause is unavailable command support.");
	let queue_handle = queue_handle.expect("GHI device creation must fill the requested queue handle.");
	(instance, device, context, queue_handle)
}

/// Creates a shader from a baked Metal library, whose entry point every generated shader names alike, or from baked
/// SPIR-V. Metal takes a task or mesh shader's `threadgroup_size` with its library, as the engine passes it from the
/// shader's stage metadata.
pub(super) fn create_baked_shader(
	context: &mut impl ghi::context::Context,
	name: &str,
	binary: &[u8],
	stage: ghi::ShaderTypes,
	threadgroup_size: Option<Extent>,
	sets: impl IntoIterator<Item = ghi::ShaderDescriptorSet>,
) -> ghi::ShaderHandle {
	#[cfg(target_os = "macos")]
	let source = ghi::shader::Sources::MTLB {
		binary,
		entry_point: MSL_ENTRY_POINT,
		threadgroup_size,
	};
	#[cfg(not(target_os = "macos"))]
	let source = {
		let _ = threadgroup_size;
		ghi::shader::Sources::SPIRV(std::borrow::Cow::Borrowed(binary))
	};
	context.create_shader(Some(name), source, stage, sets).unwrap_or_else(|()| {
		panic!("Failed to create shader {name}. The most likely cause is a baked shader the backend can't load.")
	})
}

/// Copies `values` into a new buffer with `uses` that the GPU reads.
pub(super) fn upload<T: bytemuck::Pod>(
	context: &mut impl ghi::context::Context,
	uses: ghi::Uses,
	values: &[T],
) -> ghi::BufferHandle<[T]> {
	let buffer = context.build_buffer::<[T]>(
		ghi::buffer::Builder::new(uses)
			.length(values.len())
			.device_accesses(ghi::DeviceAccesses::HostToDevice),
	);
	context.get_mut_buffer_slice(buffer).copy_from_slice(values);
	context.sync_buffer(buffer);
	buffer
}
