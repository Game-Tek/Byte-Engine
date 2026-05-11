use std::cell::Cell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::ptr::NonNull;
use std::{
	fs,
	time::{SystemTime, UNIX_EPOCH},
};

use ::utils::hash::{HashMap, HashSet};
use dispatch2::DispatchData;
use objc2::runtime::AnyObject;
use objc2::{msg_send, ClassType};
use objc2_foundation::{NSArray, NSAutoreleasePool, NSRange, NSString};
use objc2_metal::{
	MTLArgumentEncoder, MTLBlitCommandEncoder, MTLBuffer, MTLCommandBuffer, MTLCommandBufferEncoderInfo, MTLCommandEncoder,
	MTLCommandQueue, MTLDevice, MTLLibrary, MTLResource,
};

use super::*;
use crate::{
	binding::DescriptorSetBindingHandle,
	buffer::{self as buffer_builder, BufferHandle},
	descriptors::DescriptorSetHandle,
	image::{self as image_builder, ImageHandle},
	metal::swapchain::Swapchain,
	metal::utils::parse_threadgroup_size_metadata,
	pipelines::raster as raster_pipeline,
	sampler::{self as sampler_builder, SamplerHandle},
	window, DeviceAccesses, HandleLike as _, ResourceCollection, Size, Uses,
};

/// The `Device` struct owns the underlying Metal GPU device object.
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
			let queue = device.newCommandQueue().ok_or(
				"Metal command queue creation failed. The most likely cause is that the device ran out of command queue resources.",
			)?;
			let handle = graphics_hardware_interface::QueueHandle(created_queues.len() as u64);

			created_queues.push(queue::StoredQueue { queue, workloads });

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

	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		crate::metal::context::Context::new(self.settings, self.device.clone(), self.queues.clone())
	}
}

fn metal_command_buffer_status_name(status: mtl::MTLCommandBufferStatus) -> &'static str {
	match status {
		mtl::MTLCommandBufferStatus::NotEnqueued => "not_enqueued",
		mtl::MTLCommandBufferStatus::Enqueued => "enqueued",
		mtl::MTLCommandBufferStatus::Committed => "committed",
		mtl::MTLCommandBufferStatus::Scheduled => "scheduled",
		mtl::MTLCommandBufferStatus::Completed => "completed",
		mtl::MTLCommandBufferStatus::Error => "error",
		_ => "unknown",
	}
}

fn metal_command_encoder_error_state_name(state: mtl::MTLCommandEncoderErrorState) -> &'static str {
	match state {
		mtl::MTLCommandEncoderErrorState::Unknown => "unknown",
		mtl::MTLCommandEncoderErrorState::Completed => "completed",
		mtl::MTLCommandEncoderErrorState::Affected => "affected",
		mtl::MTLCommandEncoderErrorState::Pending => "pending",
		mtl::MTLCommandEncoderErrorState::Faulted => "faulted",
		_ => "unknown",
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
			"Failed to create a Metal command queue. Metal video work is not exposed through MTLCommandQueue in this backend.",
		);
	}

	if requested.intersects(crate::WorkloadTypes::IO) {
		return Err(
			"Failed to create a Metal command queue. Metal IO uses MTLIOCommandQueue and is not compatible with this command-buffer queue path.",
		);
	}

	let mut supported = crate::WorkloadTypes::RASTER | crate::WorkloadTypes::COMPUTE | crate::WorkloadTypes::TRANSFER;

	if device.supportsRaytracing() {
		supported |= crate::WorkloadTypes::RAY_TRACING;
	}

	if !supported.contains(requested) {
		return Err(
			"Failed to create a Metal command queue. The requested workload type is not supported by the selected Metal device.",
		);
	}

	Ok(requested)
}

fn metal_command_encoder_label(
	encoder_info: &ProtocolObject<dyn mtl::MTLCommandBufferEncoderInfo>,
) -> Option<Retained<NSString>> {
	unsafe {
		let label: *mut NSString = msg_send![encoder_info, label];
		if label.is_null() {
			None
		} else {
			Retained::from_raw(label)
		}
	}
}

fn metal_command_encoder_debug_signposts(
	encoder_info: &ProtocolObject<dyn mtl::MTLCommandBufferEncoderInfo>,
) -> Option<Retained<NSArray<NSString>>> {
	unsafe {
		let debug_signposts: *mut NSArray<NSString> = msg_send![encoder_info, debugSignposts];
		if debug_signposts.is_null() {
			None
		} else {
			Retained::from_raw(debug_signposts)
		}
	}
}

// Formats the detailed Metal failure report, including per-encoder execution status when Metal provides it.
fn describe_metal_command_buffer_failure(command_buffer: &ProtocolObject<dyn mtl::MTLCommandBuffer>) -> String {
	let status = command_buffer.status();
	let mut report = String::from(
		"Metal command buffer execution failed. The most likely cause is that a Metal encoder triggered a GPU validation, resource lifetime, or shader execution fault.",
	);

	if let Some(label) = command_buffer.label().filter(|label| !label.to_string().is_empty()) {
		let _ = write!(report, "\nCommand buffer: {}", label);
	}

	let _ = write!(report, "\nStatus: {}", metal_command_buffer_status_name(status));

	let Some(error) = command_buffer.error() else {
		return report;
	};

	let _ = write!(report, "\nDomain: {}", error.domain());
	let _ = write!(report, "\nCode: {}", error.code());
	let _ = write!(report, "\nDescription: {}", error.localizedDescription());

	if let Some(reason) = error.localizedFailureReason().filter(|reason| !reason.to_string().is_empty()) {
		let _ = write!(report, "\nFailure reason: {}", reason);
	}

	let user_info = error.userInfo();
	let encoder_info_key = unsafe { mtl::MTLCommandBufferEncoderInfoErrorKey };
	let Some(encoder_info_value) = user_info.objectForKeyedSubscript(encoder_info_key) else {
		return report;
	};

	let encoder_infos = unsafe { objc2::rc::Retained::cast_unchecked::<NSArray<AnyObject>>(encoder_info_value) };
	if encoder_infos.count() == 0 {
		return report;
	}

	report.push_str("\nEncoders:");
	for index in 0..encoder_infos.count() {
		let encoder_info = unsafe {
			objc2::rc::Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLCommandBufferEncoderInfo>>(
				encoder_infos.objectAtIndex(index),
			)
		};
		let label = metal_command_encoder_label(encoder_info.as_ref())
			.map(|label| label.to_string())
			.unwrap_or_default();
		let label = if label.is_empty() { "<unlabeled>" } else { label.as_str() };
		let state = metal_command_encoder_error_state_name(encoder_info.errorState());
		let _ = write!(report, "\n  {}. {} [{}]", index, label, state);

		if let Some(signposts) =
			metal_command_encoder_debug_signposts(encoder_info.as_ref()).filter(|signposts| signposts.count() > 0)
		{
			let joined_signposts = signposts.componentsJoinedByString(&NSString::from_str(" > "));
			let _ = write!(report, "\n     Signposts: {}", joined_signposts);
		}
	}

	report
}

// Waits for the Metal command buffer and turns Metal's enhanced error payload into a readable panic.
fn wait_for_metal_command_buffer(command_buffer: &ProtocolObject<dyn mtl::MTLCommandBuffer>) {
	command_buffer.commit();
	command_buffer.waitUntilCompleted();

	if command_buffer.status() != mtl::MTLCommandBufferStatus::Completed || command_buffer.error().is_some() {
		panic!("{}", describe_metal_command_buffer_failure(command_buffer));
	}
}

pub(super) fn submit_metal_command_buffer(command_buffer: &ProtocolObject<dyn mtl::MTLCommandBuffer>) {
	wait_for_metal_command_buffer(command_buffer);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SwapchainDescriptorBinding {
	pub(crate) binding_handle: DescriptorSetBindingHandle,
	pub(crate) array_element: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct SwapchainDescriptorSource {
	pub(crate) swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	pub(crate) frame_offset: i32,
}
