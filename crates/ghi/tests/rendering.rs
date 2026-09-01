//! Exercises rendering through the active native GHI backend.

// Native rendering scenarios keep complete setup, submission, and readback workflows together.
#![allow(
	clippy::cognitive_complexity,
	clippy::excessive_nesting,
	clippy::multiple_unsafe_ops_per_block,
	clippy::too_many_lines,
	clippy::undocumented_unsafe_blocks
)]

use ghi::implementation::{Context as BackendContext, Instance};
use ghi::{
	BufferDescriptor, BufferStridedRange, DataTypes, DeviceAccesses, Encodings, FilteringModes, Formats, Layouts, QueueHandle,
	SamplerAddressingModes, SamplingReductionModes, ShaderTypes, UseCases, Uses, Window,
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		BoundRayTracingPipelineMode as _, CommandBuffer as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
		RasterizationRenderPassMode as _,
	},
	frame::Frame as _,
	pipelines::{self, PushConstantRange, ShaderParameter, VertexElement, raster::AttachmentDescriptor},
	queue::{FrameRequest, Queue as _, QueueExecution as _},
	rt::{
		BindingTables, BottomLevelAccelerationStructureBuild, BottomLevelAccelerationStructureBuildDescriptions,
		TopLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuildDescriptions,
	},
	shader::{CompiledShaderSource, ShaderSource},
	*,
};
use utils::{Extent, RGBA};

#[path = "rendering/common.rs"]
mod common;
#[path = "rendering/presentation.rs"]
mod presentation;
#[path = "rendering/raster.rs"]
mod raster;
#[path = "rendering/ray_tracing.rs"]
mod ray_tracing;
#[path = "rendering/resources.rs"]
mod resources;

/// Creates the native device and raster queue used by one rendering scenario.
fn create_default_device_setup() -> (Instance, BackendContext, QueueHandle) {
	let features = ghi::device::Features::new().validation(true);
	create_default_device_setup_with_features(features)
}

/// Creates the native device with the capabilities required by one rendering scenario.
fn create_default_device_setup_with_features(features: ghi::device::Features) -> (Instance, BackendContext, QueueHandle) {
	let mut instance = Instance::new(features).expect(
		"Failed to create the GHI test instance. The most likely cause is that the active backend has no available device.",
	);
	let mut queue_handle = None;
	let device = instance
		.create_device(
			features,
			&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue_handle)],
		)
		.expect("Failed to create the GHI test device. The most likely cause is unavailable raster queue support.");
	let context = ghi::device::Device::create_context(&device)
		.expect("Failed to create the GHI test context. The most likely cause is unavailable backend command support.");
	(instance, context, queue_handle.unwrap())
}

#[test]
fn render_triangle_pixels() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	raster::render_triangle(&mut device, queue_handle);
}

#[cfg(target_os = "macos")]
#[test]
fn raster_pipeline_can_disable_depth_writes() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	raster::render_without_depth_writes(&mut device, queue_handle);
}

#[test]
#[ignore = "test is broken because of WSI"]
fn present_to_window() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	presentation::present(&mut device, queue_handle);
}

#[test]
#[ignore = "test is broken because of WSI"]
fn present_multiple_frames_to_window() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	presentation::multiframe_present(&mut device, queue_handle);
}

#[test]
fn render_multiple_frames() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::multiframe_rendering(&mut device, queue_handle);
}

#[test]
fn change_frames_in_flight() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::change_frames(&mut device, queue_handle);
}

#[test]
fn resize_render_target() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::resize(&mut device, queue_handle);
}

#[test]
fn update_dynamic_data() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::dynamic_data(&mut device, queue_handle);
}

#[test]
fn update_dynamic_textures() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::dynamic_textures(&mut device, queue_handle);
}

#[test]
fn render_with_descriptor_sets() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::descriptor_sets(&mut device, queue_handle);
}

#[test]
fn render_with_multiframe_resources() {
	let (_instance, mut device, queue_handle) = create_default_device_setup();
	resources::multiframe_resources(&mut device, queue_handle);
}

#[test]
#[ignore = "not working on supporting ray tracing right now"]
fn render_with_ray_tracing() {
	let (_instance, mut device, queue_handle) =
		create_default_device_setup_with_features(ghi::device::Features::new().validation(true).ray_tracing(true));
	ray_tracing::ray_tracing(&mut device, queue_handle);
}
