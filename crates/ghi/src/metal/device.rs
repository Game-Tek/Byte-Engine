use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::ptr::NonNull;
use std::{
	fs,
	time::{SystemTime, UNIX_EPOCH},
};

use ::utils::hash::{HashMap, HashSet};
use objc2::runtime::AnyObject;
use objc2::{msg_send, ClassType};
use objc2_foundation::{NSArray, NSRange, NSString};
use objc2_metal::{
	MTLArgumentEncoder, MTLBlitCommandEncoder, MTLBuffer, MTLCommandBuffer, MTLCommandBufferEncoderInfo, MTLCommandEncoder,
	MTLCommandQueue, MTLDevice, MTLLibrary, MTLResource, MTLTexture,
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

pub struct Device {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::Queue>,
	pub(crate) buffers: ResourceCollection<buffer::Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: ResourceCollection<image::Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: Vec<sampler::Sampler>,
	pub(crate) allocations: Vec<Allocation>,
	pub(crate) descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	pub(crate) pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
	pub(crate) vertex_layouts: Vec<VertexLayout>,
	vertex_layout_indices: HashMap<VertexLayoutKey, VertexLayoutHandle>,
	pub(crate) bindings: Vec<binding::Binding>,
	pub(crate) descriptor_sets: Vec<descriptor_set::DescriptorSet>,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) acceleration_structures: Vec<AccelerationStructure>,
	pub(crate) shaders: Vec<Shader>,
	pub(crate) pipelines: Vec<Pipeline>,
	pub(crate) command_buffers: Vec<CommandBuffer>,
	pub(crate) synchronizers: Vec<synchronizer::Synchronizer>,
	pub(crate) swapchains: Vec<swapchain::Swapchain>,

	pub(crate) resource_to_descriptor: HashMap<PrivateHandles, HashSet<(DescriptorSetBindingHandle, u32, u8)>>,
	pub(crate) descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32, u32, u8), HashSet<PrivateHandles>>,

	pub settings: crate::device::Features,
	pub(crate) states: HashMap<PrivateHandles, TransitionState>,
	pub(crate) pending_buffer_syncs: VecDeque<BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<ImageHandle>,
	pub(crate) tasks: Vec<Task>,
	pub(crate) texture_copies: Vec<Vec<u8>>,

	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,
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

fn metal_command_encoder_label(
	encoder_info: &ProtocolObject<dyn mtl::MTLCommandBufferEncoderInfo>,
) -> Option<Retained<NSString>> {
	unsafe { msg_send![encoder_info, label] }
}

fn metal_command_encoder_debug_signposts(
	encoder_info: &ProtocolObject<dyn mtl::MTLCommandBufferEncoderInfo>,
) -> Option<Retained<NSArray<NSString>>> {
	unsafe { msg_send![encoder_info, debugSignposts] }
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

impl Device {
	// Creates a Metal command buffer with enhanced encoder execution status enabled.
	pub(super) fn create_metal_command_buffer(
		&self,
		queue: &ProtocolObject<dyn mtl::MTLCommandQueue>,
		label: Option<&str>,
		error_message: &'static str,
	) -> Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>> {
		let descriptor = mtl::MTLCommandBufferDescriptor::new();
		descriptor.setRetainedReferences(true);
		descriptor.setErrorOptions(mtl::MTLCommandBufferErrorOption::EncoderExecutionStatus);

		let command_buffer = queue.commandBufferWithDescriptor(&descriptor).expect(error_message);

		if let Some(label) = label {
			command_buffer.setLabel(Some(&NSString::from_str(label)));
		}

		command_buffer
	}

	// Submits the Metal command buffer and validates its completion status with enhanced diagnostics.
	pub(super) fn submit_metal_command_buffer(&self, command_buffer: &ProtocolObject<dyn mtl::MTLCommandBuffer>) {
		wait_for_metal_command_buffer(command_buffer);
	}

	pub fn new(
		settings: crate::device::Features,
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Device, &'static str> {
		let mut created_queues = Vec::with_capacity(queues.len());

		for (_selection, output_handle) in queues.iter_mut() {
			let queue = device.newCommandQueue().ok_or(
				"Metal command queue creation failed. The most likely cause is that the device ran out of command queue resources.",
			)?;
			let handle = graphics_hardware_interface::QueueHandle(created_queues.len() as u64);

			created_queues.push(queue::Queue { queue });

			**output_handle = Some(handle);
		}

		Ok(Device {
			device,
			frames: MAX_FRAMES_IN_FLIGHT as u8,
			queues: created_queues,
			buffers: ResourceCollection::with_capacity(1024),
			images: ResourceCollection::with_capacity(1024),
			samplers: Vec::new(),
			allocations: Vec::new(),
			descriptor_sets_layouts: Vec::new(),
			pipeline_layouts: Vec::new(),
			pipeline_layout_indices: HashMap::default(),
			vertex_layouts: Vec::new(),
			vertex_layout_indices: HashMap::default(),
			bindings: Vec::new(),
			descriptor_sets: Vec::new(),
			meshes: Vec::new(),
			acceleration_structures: Vec::new(),
			shaders: Vec::new(),
			pipelines: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers: Vec::new(),
			swapchains: Vec::new(),
			resource_to_descriptor: HashMap::default(),
			descriptor_set_to_resource: HashMap::default(),
			settings,
			states: HashMap::default(),
			pending_buffer_syncs: VecDeque::new(),
			pending_image_syncs: VecDeque::new(),
			tasks: Vec::new(),
			texture_copies: Vec::new(),

			#[cfg(debug_assertions)]
			names: HashMap::default(),
		})
	}

	fn create_buffer_resource(
		&mut self,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> buffer::Buffer {
		let options = utils::resource_options_from_access(device_accesses);
		let name = name.map(str::to_owned);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal buffer creation failed. The most likely cause is that the device is out of memory.");

		let staging = if device_accesses == crate::DeviceAccesses::DeviceOnly {
			Some(
				self.device
					.newBufferWithLength_options(size as _, mtl::MTLResourceOptions::StorageModeShared)
					.expect("Metal staging buffer creation failed. The most likely cause is that the device is out of memory."),
			)
		} else {
			None
		};

		if let Some(name) = name.as_deref() {
			buffer.setLabel(Some(&NSString::from_str(name)));
			if let Some(staging) = staging.as_ref() {
				staging.setLabel(Some(&NSString::from_str(&format!("{name}_staging"))));
			}
		}

		let pointer = staging
			.as_ref()
			.map(|staging| staging.contents().as_ptr() as *mut u8)
			.unwrap_or_else(|| buffer.contents().as_ptr() as *mut u8);
		let gpu_address = buffer.gpuAddress() as u64;
		let staging = staging.map(|staging| {
			let mut creator = self.buffers.creator();
			let handle = creator.add(buffer::Buffer {
				name: name.as_ref().map(|name| format!("{name}_staging")),
				staging: None,
				buffer: staging,
				size,
				gpu_address: 0,
				pointer,
				uses: resource_uses,
				access: crate::DeviceAccesses::HostToDevice,
			});
			handle
		});

		buffer::Buffer {
			name,
			buffer,
			staging,
			size,
			gpu_address,
			pointer,
			uses: resource_uses,
			access: device_accesses,
		}
	}

	pub(super) fn create_image_resource(
		&self,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		array_layers: u32,
	) -> image::Image {
		let pixel_format = utils::to_pixel_format(format);
		let name = name.map(str::to_owned);

		let width = extent.width().max(1);
		let height = extent.height().max(1);
		let mipmapped = false;

		let descriptor = unsafe {
			mtl::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
				pixel_format,
				width as _,
				height as _,
				mipmapped,
			)
		};
		if extent.depth() > 1 {
			descriptor.setTextureType(mtl::MTLTextureType::Type3D);
		} else if array_layers > 1 {
			descriptor.setTextureType(mtl::MTLTextureType::Type2DArray);
		}
		descriptor.setUsage(utils::texture_usage_from_uses(resource_uses));
		descriptor.setStorageMode(utils::storage_mode_from_access(device_accesses));
		unsafe {
			descriptor.setArrayLength(array_layers as _);
		}

		let texture = self
			.device
			.newTextureWithDescriptor(&descriptor)
			.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

		if let Some(name) = name.as_deref() {
			texture.setLabel(Some(&NSString::from_str(name)));
		}

		let staging = utils::bytes_per_pixel(format).map(|bytes_per_pixel| {
			let depth = extent.depth().max(1) as usize;
			let size = width as usize * height as usize * depth * bytes_per_pixel * array_layers as usize;
			vec![0u8; size]
		});

		image::Image {
			name,
			texture,
			extent,
			format,
			uses: resource_uses,
			access: device_accesses,
			array_layers,
			staging,
		}
	}

	fn upload_texture_from_staging(
		&self,
		texture: &ProtocolObject<dyn mtl::MTLTexture>,
		format: crate::Formats,
		extent: Extent,
		array_layers: u32,
		staging: &[u8],
	) {
		let Some(bytes_per_pixel) = utils::bytes_per_pixel(format) else {
			return;
		};

		let width = extent.width().max(1) as usize;
		let height = extent.height().max(1) as usize;
		let bytes_per_row = width * bytes_per_pixel;
		let bytes_per_image = bytes_per_row * height;
		let aligned_bytes_per_row = bytes_per_row.next_multiple_of(256);
		let aligned_bytes_per_image = aligned_bytes_per_row * height;
		let upload_size = aligned_bytes_per_image * array_layers as usize;

		let upload_buffer = self
			.device
			.newBufferWithLength_options(upload_size as _, mtl::MTLResourceOptions::StorageModeShared)
			.expect("Metal upload buffer creation failed. The most likely cause is that the device is out of memory.");
		let destination = upload_buffer.contents().as_ptr() as *mut u8;

		for slice in 0..array_layers as usize {
			let source_offset = slice * bytes_per_image;
			let destination_offset = slice * aligned_bytes_per_image;
			let Some(source_bytes) = staging.get(source_offset..source_offset + bytes_per_image) else {
				break;
			};

			for row in 0..height {
				let source_row_offset = row * bytes_per_row;
				let destination_row_offset = destination_offset + row * aligned_bytes_per_row;

				unsafe {
					std::ptr::copy_nonoverlapping(
						source_bytes.as_ptr().add(source_row_offset),
						destination.add(destination_row_offset),
						bytes_per_row,
					);
				}
			}
		}

		let queue = &self.queues[0];
		let command_buffer = self.create_metal_command_buffer(
			queue.queue.as_ref(),
			Some("Texture Upload"),
			"Metal texture upload command buffer creation failed. The most likely cause is that the transfer queue did not provide a command buffer.",
		);
		let blit_encoder = command_buffer.blitCommandEncoder().expect(
			"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		blit_encoder.setLabel(Some(&NSString::from_str("Texture Upload")));

		let source_size = mtl::MTLSize {
			width: width as _,
			height: height as _,
			depth: 1,
		};
		let destination_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

		for slice in 0..array_layers as usize {
			unsafe {
				blit_encoder.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
					upload_buffer.as_ref(),
					(slice * aligned_bytes_per_image) as _,
					aligned_bytes_per_row as _,
					aligned_bytes_per_image as _,
					source_size,
					texture,
					slice,
					0,
					destination_origin,
				);
			}
		}

		blit_encoder.endEncoding();
		self.submit_metal_command_buffer(command_buffer.as_ref());
	}

	/// Stores a resolved descriptor for one binding slot, re-encodes the argument buffer, and refreshes resource tracking.
	pub(crate) fn update_descriptor_for_binding(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let binding = self.bindings[binding_handle.0 as usize].clone();
		let set_handle = binding.descriptor_set_handle;
		let binding_index = binding.index;

		self.clear_descriptor_tracking(set_handle, binding_handle, binding_index, array_element, frame_index);

		{
			let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
			let bindings = descriptor_set.descriptors.entry(binding_index).or_default();
			bindings.insert(array_element, descriptor);
		}

		self.encode_descriptor_binding(set_handle, binding_index, descriptor, frame_index, array_element);
		self.register_descriptor_tracking(
			set_handle,
			binding_handle,
			binding_index,
			descriptor,
			array_element,
			frame_index,
		);
	}

	/// Removes reverse-tracking entries for the descriptor currently associated with one binding element in one frame.
	fn clear_descriptor_tracking(
		&mut self,
		set_handle: DescriptorSetHandle,
		binding_handle: DescriptorSetBindingHandle,
		binding_index: u32,
		array_element: u32,
		frame_index: u8,
	) {
		let key = (set_handle, binding_index, array_element, frame_index);
		let Some(resources) = self.descriptor_set_to_resource.remove(&key) else {
			return;
		};

		for resource in resources {
			let should_remove = if let Some(descriptor_bindings) = self.resource_to_descriptor.get_mut(&resource) {
				descriptor_bindings.remove(&(binding_handle, array_element, frame_index));
				descriptor_bindings.is_empty()
			} else {
				false
			};

			if should_remove {
				self.resource_to_descriptor.remove(&resource);
			}
		}
	}

	/// Registers reverse-tracking for resource-backed descriptors so later resource changes can re-encode the affected bindings.
	fn register_descriptor_tracking(
		&mut self,
		set_handle: DescriptorSetHandle,
		binding_handle: DescriptorSetBindingHandle,
		binding_index: u32,
		descriptor: Descriptor,
		array_element: u32,
		frame_index: u8,
	) {
		let Some(resource) = descriptor.tracked_resource() else {
			return;
		};

		self.descriptor_set_to_resource
			.entry((set_handle, binding_index, array_element, frame_index))
			.or_default()
			.insert(resource);
		self.resource_to_descriptor
			.entry(resource)
			.or_default()
			.insert((binding_handle, array_element, frame_index));
	}

	/// Writes a resolved descriptor into the Metal argument buffer for one frame and array element.
	/// Call this to write a descriptor binding into the argument buffer.
	pub(crate) fn encode_descriptor_binding(
		&mut self,
		set_handle: DescriptorSetHandle,
		binding_index: u32,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let descriptor_set_layout_handle = self.descriptor_sets[set_handle.0 as usize].descriptor_set_layout;
		let (argument_encoder, layout_binding) = {
			let layout = &self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize];
			(
				layout.argument_encoder.clone(),
				layout.binding(binding_index).cloned().expect(
					"Descriptor set binding not found in Metal layout. The most likely cause is that a descriptor write targeted a binding that was not declared in the descriptor set template.",
				),
			)
		};

		let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];

		unsafe {
			argument_encoder.setArgumentBuffer_offset(Some(descriptor_set.argument_buffer.as_ref()), 0);
		}

		match (layout_binding.slot_for_array_element(array_element), descriptor) {
			(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
				let buffer = self.buffers.resource(buffer);
				argument_encoder.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, .. }) => unsafe {
				let image = self.images.resource(image);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::CombinedImageSampler { image, .. }) => unsafe {
				let image = self.images.resource(image);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Sampler(slot), Descriptor::Sampler { sampler }) => unsafe {
				let sampler = &self.samplers[sampler.0 as usize];
				argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
			},
			(
				DescriptorBindingSlot::CombinedImageSampler { texture, sampler },
				Descriptor::CombinedImageSampler {
					image,
					sampler: sampler_handle,
					..
				},
			) => unsafe {
				let image = self.images.resource(image);
				let sampler_state = &self.samplers[sampler_handle.0 as usize];
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), texture as _);
				argument_encoder.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::Swapchain { handle }) => unsafe {
				let swapchain = &self.swapchains[handle.0 as usize];
				let proxy_image_handle = swapchain.images[frame_index as usize].expect(
					"Swapchain proxy image not found. The most likely cause is that the swapchain was not created with proxy images.",
				);
				let image = self.images.resource(proxy_image_handle);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			_ => panic!(
				"Descriptor write does not match the Metal descriptor set layout. The most likely cause is that a descriptor type was written to a binding declared with a different descriptor type."
			),
		}
	}

	pub(crate) fn encode_binding(
		&self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let binding = &self.bindings[binding_handle.0 as usize];
		let descriptor_set_handle = binding.descriptor_set_handle;
		let index = binding.index;

		let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
		let descriptor_set_template_handle = descriptor_set.descriptor_set_layout;

		let (argument_encoder, layout_binding) = {
			let layout = &self.descriptor_sets_layouts[descriptor_set_template_handle.0 as usize];
			(
				layout.argument_encoder.clone(),
				layout.binding(index).cloned().expect(
					"Descriptor set binding not found in Metal layout. The most likely cause is that a descriptor write targeted a binding that was not declared in the descriptor set template.",
				),
			)
		};

		unsafe {
			argument_encoder.setArgumentBuffer_offset(Some(descriptor_set.argument_buffer.as_ref()), 0);
		}

		match (layout_binding.slot_for_array_element(array_element), descriptor) {
			(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
				let buffer = self.buffers.resource(buffer);
				argument_encoder.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, .. }) => unsafe {
				let image = self.images.resource(image);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::CombinedImageSampler { image, .. }) => unsafe {
				let image = self.images.resource(image);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Sampler(slot), Descriptor::Sampler { sampler }) => unsafe {
				let sampler = &self.samplers[sampler.0 as usize];
				argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
			},
			(
				DescriptorBindingSlot::CombinedImageSampler { texture, sampler },
				Descriptor::CombinedImageSampler {
					image,
					sampler: sampler_handle,
					..
				},
			) => unsafe {
				let image = self.images.resource(image);
				let sampler_state = &self.samplers[sampler_handle.0 as usize];
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), texture as _);
				argument_encoder.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::Swapchain { handle }) => unsafe {
				let swapchain = &self.swapchains[handle.0 as usize];
				let proxy_image_handle = swapchain.images[frame_index as usize].expect(
					"Swapchain proxy image not found. The most likely cause is that the swapchain was not created with proxy images.",
				);
				let image = self.images.resource(proxy_image_handle);
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			_ => panic!(
				"Descriptor write does not match the Metal descriptor set layout. The most likely cause is that a descriptor type was written to a binding declared with a different descriptor type."
			),
		}
	}

	/// Pre-encodes immutable samplers into a descriptor set.
	fn encode_immutable_samplers(&mut self, set_handle: DescriptorSetHandle) {
		let descriptor_set_layout_handle = self.descriptor_sets[set_handle.0 as usize].descriptor_set_layout;
		let (argument_encoder, bindings) = {
			let layout = &self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize];
			(layout.argument_encoder.clone(), layout.bindings.clone())
		};

		unsafe {
			argument_encoder
				.setArgumentBuffer_offset(Some(self.descriptor_sets[set_handle.0 as usize].argument_buffer.as_ref()), 0);
		}

		for binding in &bindings {
			let Some(immutable_samplers) = &binding.immutable_samplers else {
				continue;
			};

			for (array_element, sampler_handle) in immutable_samplers.iter().enumerate() {
				let slot = binding.slot_for_array_element(array_element as u32);
				let sampler = &self.samplers[SamplerHandle(sampler_handle.0).0 as usize];

				match slot {
					DescriptorBindingSlot::Sampler(slot) => unsafe {
						argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
					},
					DescriptorBindingSlot::CombinedImageSampler {
						sampler: sampler_slot, ..
					} => unsafe {
						argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), sampler_slot as _);
					},
					_ => {}
				}
			}
		}
	}

	/// Resolves a descriptor write into the concrete per-frame Metal resources referenced by the current sequence.
	/// TODO: fix delta indexing in this function
	fn resolve_descriptor_for_frame(
		&self,
		descriptor: crate::descriptors::WriteData,
		sequence_index: u8,
		frame_offset: i32,
	) -> Option<Descriptor> {
		match descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => {
				let index = (sequence_index as i32 - frame_offset) as usize;
				let handle = self.buffers.nth_handle(handle, index)?;
				Some(Descriptor::Buffer { buffer: handle, size })
			}
			crate::descriptors::WriteData::Image { handle, layout } => {
				let handle = self
					.images
					.nth_handle(handle, (sequence_index as i64 - frame_offset as i64) as usize)?;
				Some(Descriptor::Image { image: handle, layout })
			}
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				..
			} => {
				let handle = self
					.images
					.nth_handle(image_handle, (sequence_index as i64 - frame_offset as i64) as usize)?;
				Some(Descriptor::CombinedImageSampler {
					image: handle,
					sampler: SamplerHandle(sampler_handle.0),
					layout,
				})
			}
			crate::descriptors::WriteData::Sampler(handle) => Some(Descriptor::Sampler {
				sampler: SamplerHandle(handle.0),
			}),
			crate::descriptors::WriteData::StaticSamplers => None,
			crate::descriptors::WriteData::CombinedImageSamplerArray => None,
			crate::descriptors::WriteData::AccelerationStructure { .. } => None,
			crate::descriptors::WriteData::Swapchain(swapchain_handle) => Some(Descriptor::Swapchain {
				handle: crate::swapchain::SwapchainHandle(swapchain_handle.0),
			}),
		}
	}

	/// Resolves and applies a descriptor write for a single frame when the referenced resources are available.
	fn apply_descriptor_write_for_frame(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
		sequence_index: u8,
	) {
		if let Some(descriptor) = self.resolve_descriptor_for_frame(descriptor, sequence_index, frame_offset) {
			self.update_descriptor_for_binding(binding_handle, descriptor, sequence_index, array_element);
		}
	}

	/// Applies the same descriptor write across every frame tracked by the Metal device.
	/// Call this to update a descriptor binding for all frames.
	fn apply_descriptor_write_to_all_frames(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
	) {
		let binding_handles = binding_handle.root(&self.bindings).get_all(&self.bindings);

		for (sequence_index, &binding_handle) in binding_handles.iter().enumerate() {
			self.apply_descriptor_write_for_frame(
				binding_handle,
				descriptor,
				array_element,
				frame_offset,
				sequence_index as u8,
			);
		}
	}

	/// Re-encodes every tracked descriptor binding that references a resource after its Metal backing changes.
	pub(crate) fn rewrite_descriptors_for_handle(&mut self, handle: PrivateHandles) {
		let Some(descriptor_bindings) = self.resource_to_descriptor.get(&handle).cloned() else {
			return;
		};

		for (binding_handle, array_element, frame_index) in descriptor_bindings {
			let binding = self.bindings[binding_handle.0 as usize].clone();
			let set_handle = binding.descriptor_set_handle;
			let descriptor = self.descriptor_sets[set_handle.0 as usize]
				.descriptors
				.get(&binding.index)
				.and_then(|descriptors| descriptors.get(&array_element))
				.copied();

			if let Some(descriptor) = descriptor {
				self.encode_descriptor_binding(set_handle, binding.index, descriptor, frame_index, array_element);
			}
		}
	}

	/// Resizes every swapchain proxy image in place so existing descriptors can keep their image handles.
	pub(crate) fn resize_swapchain_images(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		extent: Extent,
	) {
		let image_handles = self.swapchains[swapchain_handle.0 as usize]
			.images
			.into_iter()
			.flatten()
			.collect::<Vec<_>>();

		for image_handle in image_handles {
			let (name, current_extent, format, uses, access, array_layers) = {
				let image = self.images.resource(image_handle);
				(
					image.name.clone(),
					image.extent,
					image.format,
					image.uses,
					image.access,
					image.array_layers,
				)
			};

			if current_extent == extent {
				continue;
			}

			let replacement = self.create_image_resource(name.as_deref(), extent, format, uses, access, array_layers);
			*self.images.resource_mut(image_handle) = replacement;
			self.rewrite_descriptors_for_handle(PrivateHandles::Image(image_handle));
		}
	}

	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let mut tasks = self.tasks.split_off(0);

		tasks.retain(|task| {
			if let Some(frame) = task.frame() {
				if frame != sequence_index {
					return true;
				}
			}

			match task.task() {
				Tasks::UpdateBufferDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(PrivateHandles::Buffer(*handle));
				}
				Tasks::UpdateImageDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(PrivateHandles::Image(*handle));
				}
				Tasks::UpdateDescriptor { descriptor_write } => {
					let binding_handles = DescriptorSetBindingHandle(descriptor_write.binding_handle.0)
						.root(&self.bindings)
						.get_all(&self.bindings);
					let binding_index = (sequence_index as usize).rem_euclid(binding_handles.len());

					let Some(&binding_handle) = binding_handles.get(binding_index) else {
						return false;
					};

					self.apply_descriptor_write_for_frame(
						binding_handle,
						descriptor_write.descriptor,
						descriptor_write.array_element,
						descriptor_write.frame_offset.unwrap_or(0),
						sequence_index,
					);
				}
				Tasks::WriteDescriptor {
					binding_handle,
					descriptor,
				} => match descriptor {
					Descriptors::Buffer { handle, size } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Buffer {
							buffer: *handle,
							size: *size,
						},
						sequence_index,
						0,
					),
					Descriptors::Image { handle, layout } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Image {
							image: *handle,
							layout: *layout,
						},
						sequence_index,
						0,
					),
					Descriptors::CombinedImageSampler {
						image_handle,
						sampler_handle,
						layout,
						..
					} => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::CombinedImageSampler {
							image: *image_handle,
							sampler: *sampler_handle,
							layout: *layout,
						},
						sequence_index,
						0,
					),
					Descriptors::Sampler { handle } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Sampler { sampler: *handle },
						sequence_index,
						0,
					),
					Descriptors::CombinedImageSamplerArray => {}
				},
				Tasks::DeleteMetalTexture { .. }
				| Tasks::DeleteMetalBuffer { .. }
				| Tasks::ResizeImage { .. }
				| Tasks::BuildImage(_)
				| Tasks::BuildBuffer(_) => {}
			}

			false
		});

		self.tasks = tasks;
	}

	pub(super) fn copy_texture_to_cpu(&mut self, image_handle: ImageHandle) -> graphics_hardware_interface::TextureCopyHandle {
		let image = self.images.resource(image_handle);

		let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
			self.texture_copies.push(Vec::new());
			return graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64);
		};

		let extent = image.extent;
		let width = extent.width() as usize;
		let height = extent.height() as usize;
		let bytes_per_row = width * bytes_per_pixel;
		let size = bytes_per_row * height;

		let mut data = vec![0u8; size];
		let data_ptr = NonNull::new(data.as_mut_ptr() as *mut std::ffi::c_void)
			.expect("Texture readback buffer was null. The most likely cause is an empty allocation.");
		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: mtl::MTLSize {
				width: width as _,
				height: height as _,
				depth: 1,
			},
		};

		unsafe {
			image
				.texture
				.getBytes_bytesPerRow_fromRegion_mipmapLevel(data_ptr, bytes_per_row as _, region, 0);
		}

		self.texture_copies.push(data);
		graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64)
	}
}

impl Device {
	#[cfg(debug_assertions)]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn create_pipeline_factory(&self) -> Option<crate::implementation::PipelineFactory> {
		Some(crate::metal::pipelines::factory::Factory {
			device: self.device.clone(),
			shaders: Vec::with_capacity(64),
		})
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		self.frames = frames.max(1);
		for swapchain in &mut self.swapchains {}
		// TODO: Rebuild dynamic resources for new frame count.
	}

	pub fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		let options = utils::resource_options_from_access(device_accesses);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal allocation failed. The most likely cause is that the device is out of memory.");
		let pointer = buffer.contents().as_ptr() as *mut u8;

		self.allocations.push(Allocation { buffer, pointer, size });
		graphics_hardware_interface::AllocationHandle((self.allocations.len() - 1) as u64)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		// Split interleaved vertices into one packed stream per Metal vertex binding.
		let options = mtl::MTLResourceOptions::StorageModeShared;
		let index_ptr = NonNull::new(indices.as_ptr() as *mut std::ffi::c_void)
			.expect("Index data pointer was null. The most likely cause is an empty index slice.");
		let index_buffer = unsafe {
			self.device
				.newBufferWithBytes_length_options(index_ptr, indices.len() as _, options)
		}
		.expect("Metal index buffer creation failed. The most likely cause is that the device is out of memory.");
		let vertex_size = utils::vertex_layout_size(vertex_layout);
		let max_binding = vertex_layout
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);
		let mut binding_spans = vec![Vec::<(usize, usize, usize)>::new(); max_binding];
		let mut source_offset = 0usize;

		for element in vertex_layout {
			let element_size = utils::data_type_size(element.format);
			let binding = element.binding as usize;
			let destination_offset = binding_spans[binding]
				.last()
				.map(|(_, destination_offset, size)| destination_offset + size)
				.unwrap_or(0);
			binding_spans[binding].push((source_offset, destination_offset, element_size));
			source_offset += element_size;
		}

		let vertex_buffers = binding_spans
			.iter()
			.map(|spans| {
				if spans.is_empty() {
					return None;
				}

				let binding_stride = spans
					.last()
					.map(|(_, destination_offset, size)| destination_offset + size)
					.unwrap_or(0);
				let mut binding_vertices = vec![0u8; binding_stride * vertex_count as usize];

				for vertex_index in 0..vertex_count as usize {
					let source_vertex_offset = vertex_index * vertex_size;
					let destination_vertex_offset = vertex_index * binding_stride;

					for &(span_source_offset, span_destination_offset, span_size) in spans {
						let source_range =
							source_vertex_offset + span_source_offset..source_vertex_offset + span_source_offset + span_size;
						let destination_range = destination_vertex_offset + span_destination_offset
							..destination_vertex_offset + span_destination_offset + span_size;
						binding_vertices[destination_range].copy_from_slice(&vertices[source_range]);
					}
				}

				let vertex_ptr = NonNull::new(binding_vertices.as_ptr() as *mut std::ffi::c_void)
					.expect("Vertex data pointer was null. The most likely cause is an empty vertex slice.");
				Some(
					unsafe {
						self.device
							.newBufferWithBytes_length_options(vertex_ptr, binding_vertices.len() as _, options)
					}
					.expect("Metal vertex buffer creation failed. The most likely cause is that the device is out of memory."),
				)
			})
			.collect::<Vec<_>>();

		self.meshes.push(Mesh {
			vertex_buffers,
			index_buffer,
			vertex_count,
			index_count,
			vertex_size,
		});

		graphics_hardware_interface::MeshHandle((self.meshes.len() - 1) as u64)
	}

	pub fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let (spirv, metal_library, metal_entry_point, threadgroup_size) = match shader_source_type {
			crate::shader::Sources::SPIRV(data) => (Some(data.to_vec()), None, None, None),
			crate::shader::Sources::MTLB { binary, entry_point } => {
				let temporary_name = format!(
					"byte_engine_{}_{}.metallib",
					entry_point,
					SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| ())?.as_nanos()
				);
				let metallib_path = std::env::temp_dir().join(temporary_name);
				fs::write(&metallib_path, binary).map_err(|_| ())?;
				let metallib_path_string = metallib_path.to_string_lossy().into_owned();
				let metallib_path = NSString::from_str(&metallib_path_string);
				let library = self.device.newLibraryWithFile_error(&metallib_path).map_err(|error| {
					eprintln!(
						"Metal shader library load failed: {}",
						error.localizedDescription().to_string()
					);
					()
				})?;
				let _ = fs::remove_file(std::path::Path::new(&metallib_path_string));

				(None, Some(library), Some(entry_point.to_owned()), None)
			}
			crate::shader::Sources::MTL { source, entry_point } => {
				let threadgroup_size = match stage {
					crate::ShaderTypes::Task | crate::ShaderTypes::Mesh | crate::ShaderTypes::Compute => {
						parse_threadgroup_size_metadata(source)
					}
					_ => None,
				};
				let compile_options = mtl::MTLCompileOptions::new();
				let source = NSString::from_str(source);
				let library = self
					.device
					.newLibraryWithSource_options_error(&source, Some(&compile_options))
					.map_err(|error| {
						eprintln!(
							"Metal shader compilation failed: {}",
							error.localizedDescription().to_string()
						);
						()
					})?;

				(None, Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
		};

		let stages = match stage {
			crate::ShaderTypes::Vertex => crate::Stages::VERTEX,
			crate::ShaderTypes::Fragment => crate::Stages::FRAGMENT,
			crate::ShaderTypes::Compute => crate::Stages::COMPUTE,
			crate::ShaderTypes::RayGen => crate::Stages::RAYGEN,
			crate::ShaderTypes::Intersection => crate::Stages::INTERSECTION,
			crate::ShaderTypes::AnyHit => crate::Stages::ANY_HIT,
			crate::ShaderTypes::ClosestHit => crate::Stages::CLOSEST_HIT,
			crate::ShaderTypes::Miss => crate::Stages::MISS,
			crate::ShaderTypes::Callable => crate::Stages::CALLABLE,
			crate::ShaderTypes::Task => crate::Stages::TASK,
			crate::ShaderTypes::Mesh => crate::Stages::MESH,
		};

		self.shaders.push(Shader {
			stage: stages,
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
			metal_library,
			metal_entry_point,
			spirv,
			threadgroup_size,
		});

		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn create_metal_function(
		&self,
		shader_parameter: &crate::pipelines::ShaderParameter,
	) -> Option<Retained<ProtocolObject<dyn mtl::MTLFunction>>> {
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let library = shader.metal_library.as_ref()?;
		let entry_point = shader.metal_entry_point.as_ref()?;
		let entry_point = NSString::from_str(entry_point);

		let constant_values = mtl::MTLFunctionConstantValues::new();

		for specialization_map_entry in shader_parameter.specialization_map {
			self.apply_specialization_map_entry(&constant_values, specialization_map_entry);
		}

		library
			.newFunctionWithName_constantValues_error(&entry_point, &constant_values)
			.map_err(|error| {
				eprintln!(
					"Metal shader specialization failed: {}",
					error.localizedDescription().to_string()
				);
			})
			.ok()
	}

	fn apply_specialization_map_entry(
		&self,
		constant_values: &mtl::MTLFunctionConstantValues,
		specialization_map_entry: &crate::pipelines::SpecializationMapEntry,
	) {
		match specialization_map_entry.get_type().as_str() {
			"bool" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::Bool,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"u32" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::UInt,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"f32" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::Float,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"vec2f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 2),
				);
			},
			"vec3f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 3),
				);
			},
			"vec4f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					mtl::MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 4),
				);
			},
			_ => panic!(
				"Unsupported Metal specialization constant type. The most likely cause is that the Metal backend was not updated for a new specialization entry type."
			),
		}
	}

	/// Builds the Metal argument-buffer layout that backs a descriptor set template.
	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		binding_templates: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
	) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		let mut next_argument_index = 0u32;
		let mut metal_argument_descriptors = Vec::new();
		let bindings = binding_templates
			.iter()
			.map(|template| {
				assert_ne!(
					template.descriptor_count, 0,
					"Metal descriptor set bindings must contain at least one descriptor. The most likely cause is that a descriptor set template declared a binding with descriptor_count = 0.",
				);

				let access = match template.descriptor_type {
					crate::descriptors::DescriptorType::UniformBuffer
					| crate::descriptors::DescriptorType::SampledImage
					| crate::descriptors::DescriptorType::InputAttachment
					| crate::descriptors::DescriptorType::Sampler
					| crate::descriptors::DescriptorType::CombinedImageSampler => mtl::MTLBindingAccess::ReadOnly,
					crate::descriptors::DescriptorType::StorageBuffer
					| crate::descriptors::DescriptorType::StorageImage
					| crate::descriptors::DescriptorType::AccelerationStructure => mtl::MTLBindingAccess::ReadWrite,
				};

				let mut build_slots = |data_type: mtl::MTLDataType| {
					(0..template.descriptor_count)
						.map(|_| {
							let descriptor = mtl::MTLArgumentDescriptor::argumentDescriptor();
							descriptor.setDataType(data_type);
							descriptor.setIndex(next_argument_index as _);
							descriptor.setAccess(access);
							if data_type == mtl::MTLDataType::Texture {
								let texture_type = match template.texture_view_type {
									crate::TextureViewTypes::Texture2D => mtl::MTLTextureType::Type2D,
									crate::TextureViewTypes::Texture2DArray => mtl::MTLTextureType::Type2DArray,
									crate::TextureViewTypes::Texture3D => mtl::MTLTextureType::Type3D,
								};
								descriptor.setTextureType(texture_type);
							}
							metal_argument_descriptors.push(descriptor);
							let slot = next_argument_index;
							next_argument_index += 1;
							slot
						})
						.collect::<Vec<_>>()
				};

				let argument_slots = match template.descriptor_type {
					crate::descriptors::DescriptorType::UniformBuffer
					| crate::descriptors::DescriptorType::StorageBuffer => {
						ArgumentBindingSlots::Buffer(build_slots(mtl::MTLDataType::Pointer))
					}
					crate::descriptors::DescriptorType::SampledImage
					| crate::descriptors::DescriptorType::StorageImage
					| crate::descriptors::DescriptorType::InputAttachment => {
						ArgumentBindingSlots::Texture(build_slots(mtl::MTLDataType::Texture))
					}
					crate::descriptors::DescriptorType::Sampler => {
						ArgumentBindingSlots::Sampler(build_slots(mtl::MTLDataType::Sampler))
					}
					crate::descriptors::DescriptorType::CombinedImageSampler => ArgumentBindingSlots::CombinedImageSampler {
						textures: build_slots(mtl::MTLDataType::Texture),
						samplers: build_slots(mtl::MTLDataType::Sampler),
					},
					crate::descriptors::DescriptorType::AccelerationStructure => {
						ArgumentBindingSlots::Buffer(build_slots(mtl::MTLDataType::Pointer))
					}
				};

				DescriptorSetLayoutBinding {
					binding: template.binding,
					descriptor_type: template.descriptor_type,
					descriptor_count: template.descriptor_count,
					stages: template.stages,
					immutable_samplers: template.immutable_samplers.clone(),
					argument_slots,
				}
			})
			.collect::<Vec<_>>();
		let argument_descriptor_refs = metal_argument_descriptors
			.iter()
			.map(|descriptor| descriptor.as_ref())
			.collect::<Vec<_>>();
		let argument_descriptors = NSArray::from_slice(&argument_descriptor_refs);
		let argument_encoder = self.device.newArgumentEncoderWithArguments(&argument_descriptors).expect(
			"Metal argument encoder creation failed. The most likely cause is that the descriptor set template described an unsupported argument layout.",
		);
		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings,
			encoded_length: argument_encoder.encodedLength() as usize,
			argument_encoder,
		});
		graphics_hardware_interface::DescriptorSetTemplateHandle((self.descriptor_sets_layouts.len() - 1) as u64)
	}

	/// Allocates one Metal descriptor set per in-flight frame and seeds immutable samplers.
	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		descriptor_set_template_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		let encoded_length = self.descriptor_sets_layouts[descriptor_set_template_handle.0 as usize]
			.encoded_length
			.max(1);

		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let descriptor_set_handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			let argument_buffer = self
				.device
				.newBufferWithLength_options(encoded_length as _, mtl::MTLResourceOptions::StorageModeShared)
				.expect("Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.");

			self.descriptor_sets.push(descriptor_set::DescriptorSet {
				next: None,
				descriptor_set_layout: *descriptor_set_template_handle,
				argument_buffer,
				descriptors: HashMap::default(),
			});

			if let Some(previous_handle) = previous_handle {
				self.descriptor_sets[previous_handle.0 as usize].next = Some(descriptor_set_handle);
			}

			self.encode_immutable_samplers(descriptor_set_handle);
			previous_handle = Some(descriptor_set_handle);
		}

		handle
	}

	/// Creates one descriptor binding per frame-local descriptor set and applies the initial contents.
	pub fn create_descriptor_binding(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		binding_constructor: graphics_hardware_interface::BindingConstructor,
	) -> graphics_hardware_interface::DescriptorSetBindingHandle {
		let descriptor_type = binding_constructor.descriptor_set_binding_template.descriptor_type;
		let binding_index = binding_constructor.descriptor_set_binding_template.binding;
		let count = binding_constructor.descriptor_set_binding_template.descriptor_count;
		let descriptor_set_handles = DescriptorSetHandle(descriptor_set.0).get_all(&self.descriptor_sets);
		let mut next = None;

		for descriptor_set_handle in descriptor_set_handles.iter().rev() {
			let binding_handle = DescriptorSetBindingHandle(self.bindings.len() as u64);

			self.bindings.push(binding::Binding {
				next,
				descriptor_set_handle: *descriptor_set_handle,
				descriptor_type,
				index: binding_index,
				count,
			});

			next = Some(binding_handle);
		}

		let binding_handle = next.expect("Descriptor binding creation failed. The most likely cause is that no Metal descriptor sets were created for the requested template.");
		let frame_offset = binding_constructor.frame_offset.map(i32::from).unwrap_or(0);

		self.apply_descriptor_write_to_all_frames(
			binding_handle,
			binding_constructor.descriptor,
			binding_constructor.array_element,
			frame_offset,
		);

		graphics_hardware_interface::DescriptorSetBindingHandle(binding_handle.0)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let key = PipelineLayoutKey {
			descriptor_set_templates: descriptor_set_template_handles.to_vec(),
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		let descriptor_set_template_indices = descriptor_set_template_handles
			.iter()
			.enumerate()
			.map(|(i, handle)| (*handle, i as u32))
			.collect();
		let push_constant_size = push_constant_ranges
			.iter()
			.map(|range| range.offset as usize + range.size as usize)
			.max()
			.unwrap_or(0);
		self.pipeline_layouts.push(PipelineLayout {
			descriptor_set_template_indices,
			push_constant_ranges: push_constant_ranges.to_vec(),
			push_constant_size,
		});
		let handle = graphics_hardware_interface::PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
		self.pipeline_layout_indices.insert(key, handle);
		handle
	}

	fn get_or_create_pipeline_layout_from_prebuilt(
		&mut self,
		layout: &PipelineLayout,
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let mut descriptor_set_templates =
			vec![graphics_hardware_interface::DescriptorSetTemplateHandle(0); layout.descriptor_set_template_indices.len()];

		for (handle, index) in &layout.descriptor_set_template_indices {
			descriptor_set_templates[*index as usize] = *handle;
		}

		self.get_or_create_pipeline_layout(&descriptor_set_templates, &layout.push_constant_ranges)
	}

	fn get_or_create_vertex_layout(&mut self, vertex_elements: &[crate::pipelines::VertexElement]) -> VertexLayoutHandle {
		let elements = vertex_elements
			.iter()
			.map(|element| VertexElementDescriptor {
				name: element.name.to_owned(),
				format: element.format,
				binding: element.binding,
			})
			.collect::<Vec<_>>();
		let key = VertexLayoutKey {
			elements: elements.clone(),
		};

		if let Some(handle) = self.vertex_layout_indices.get(&key) {
			return *handle;
		}

		let max_binding = elements
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);
		let mut strides = vec![0; max_binding];
		let vertex_descriptor = mtl::MTLVertexDescriptor::vertexDescriptor();
		let mut binding_offsets = vec![0usize; max_binding];

		for (attribute_index, element) in elements.iter().enumerate() {
			strides[element.binding as usize] += element.format.size() as u32;

			let offset = binding_offsets[element.binding as usize];
			let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
			attribute.setFormat(utils::vertex_format(element.format));
			unsafe {
				attribute.setOffset(offset as _);
				attribute.setBufferIndex(element.binding as _);
			}

			binding_offsets[element.binding as usize] += utils::data_type_size(element.format);
		}

		for (binding, stride) in strides.iter().copied().enumerate() {
			let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
			unsafe {
				layout.setStride(stride as _);
				layout.setStepRate(1);
			}
			layout.setStepFunction(mtl::MTLVertexStepFunction::PerVertex);
		}

		self.vertex_layouts.push(VertexLayout {
			elements,
			strides,
			vertex_descriptor,
		});
		let handle = VertexLayoutHandle((self.vertex_layouts.len() - 1) as u64);
		self.vertex_layout_indices.insert(key, handle);
		handle
	}

	fn get_or_create_vertex_layout_from_prebuilt(&mut self, vertex_layout: VertexLayout) -> VertexLayoutHandle {
		let key = VertexLayoutKey {
			elements: vertex_layout.elements.clone(),
		};

		if let Some(handle) = self.vertex_layout_indices.get(&key) {
			return *handle;
		}

		self.vertex_layouts.push(vertex_layout);
		let handle = VertexLayoutHandle((self.vertex_layouts.len() - 1) as u64);
		self.vertex_layout_indices.insert(key, handle);
		handle
	}

	fn intern_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.pipelines.push(pipeline);
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn intern_raster_pipeline(
		&mut self,
		pipeline: crate::metal::pipelines::factory::Pipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout_from_prebuilt(&pipeline.layout);
		let vertex_layout = pipeline
			.vertex_layout
			.map(|vertex_layout| self.get_or_create_vertex_layout_from_prebuilt(vertex_layout));

		self.intern_pipeline(Pipeline {
			pipeline: pipeline.pipeline,
			depth_stencil_state: pipeline.depth_stencil_state,
			layout,
			vertex_layout,
			shader_handles: pipeline.shader_handles,
			resource_access: pipeline.resource_access,
			compute_threadgroup_size: pipeline.compute_threadgroup_size,
			object_threadgroup_size: pipeline.object_threadgroup_size,
			mesh_threadgroup_size: pipeline.mesh_threadgroup_size,
			face_winding: pipeline.face_winding,
			cull_mode: pipeline.cull_mode,
		})
	}

	pub fn intern_compute_pipeline(
		&mut self,
		pipeline: crate::metal::pipelines::factory::ComputePipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout_from_prebuilt(&pipeline.layout);

		self.intern_pipeline(Pipeline {
			pipeline: pipeline.pipeline,
			depth_stencil_state: pipeline.depth_stencil_state,
			layout,
			vertex_layout: None,
			shader_handles: pipeline.shader_handles,
			resource_access: pipeline.resource_access,
			compute_threadgroup_size: pipeline.compute_threadgroup_size,
			object_threadgroup_size: pipeline.object_threadgroup_size,
			mesh_threadgroup_size: pipeline.mesh_threadgroup_size,
			face_winding: pipeline.face_winding,
			cull_mode: pipeline.cull_mode,
		})
	}

	pub fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let has_depth_attachment = builder
			.render_targets
			.iter()
			.any(|attachment| attachment.format.channel_layout() == crate::ChannelLayout::Depth);
		let vertex_layout = self.get_or_create_vertex_layout(builder.vertex_elements.as_ref());
		let mut shader_handles = HashMap::default();
		let mut object_function = None;
		let mut vertex_function = None;
		let mut mesh_function = None;
		let mut fragment_function = None;
		let mut object_threadgroup_size = None;
		let mut mesh_threadgroup_size = None;
		let resource_access = builder
			.shaders
			.iter()
			.flat_map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				shader_handles.insert(*shader_parameter.handle, [0; 32]);
				match shader_parameter.stage {
					crate::ShaderTypes::Task => {
						object_function = self.create_metal_function(shader_parameter);
						object_threadgroup_size = shader.threadgroup_size;
					}
					crate::ShaderTypes::Vertex => vertex_function = self.create_metal_function(shader_parameter),
					crate::ShaderTypes::Mesh => {
						mesh_function = self.create_metal_function(shader_parameter);
						mesh_threadgroup_size = shader.threadgroup_size;
					}
					crate::ShaderTypes::Fragment => fragment_function = self.create_metal_function(shader_parameter),
					_ => {}
				}
				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| {
						(
							(descriptor.set, descriptor.binding),
							(shader_parameter.stage.into(), descriptor.access),
						)
					})
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();

		let depth_stencil_state = if has_depth_attachment {
			let descriptor = mtl::MTLDepthStencilDescriptor::new();
			descriptor.setDepthCompareFunction(mtl::MTLCompareFunction::GreaterEqual);
			descriptor.setDepthWriteEnabled(true);
			self.device.newDepthStencilStateWithDescriptor(&descriptor)
		} else {
			None
		};

		let raster_pipeline_state = if let Some(mesh_function) = mesh_function.as_ref() {
			let descriptor = mtl::MTLMeshRenderPipelineDescriptor::new();
			descriptor.setLabel(Some(&NSString::from_str("mesh_pipeline")));
			unsafe {
				descriptor.setObjectFunction(object_function.as_ref().map(|function| function.as_ref()));
				descriptor.setMeshFunction(Some(mesh_function.as_ref()));
				descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			}

			for (index, attachment) in builder.render_targets.iter().enumerate() {
				if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
					descriptor.setDepthAttachmentPixelFormat(utils::to_pixel_format(attachment.format));
				} else {
					let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
					color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
					match attachment.blend {
						crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
						crate::pipelines::raster::BlendMode::Alpha => {
							color_attachment.setBlendingEnabled(true);
							color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setSourceRGBBlendFactor(mtl::MTLBlendFactor::SourceAlpha);
							color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
							color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
							color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
						}
					}
				}
			}

			self.device
				.newRenderPipelineStateWithMeshDescriptor_options_reflection_error(
					&descriptor,
					mtl::MTLPipelineOption::None,
					None,
				)
				.ok()
		} else if let Some(vertex_function) = vertex_function.as_ref() {
			let descriptor = mtl::MTLRenderPipelineDescriptor::new();
			descriptor.setLabel(Some(&NSString::from_str("raster_pipeline")));
			descriptor.setVertexFunction(Some(vertex_function.as_ref()));
			descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			descriptor.setVertexDescriptor(Some(&self.vertex_layouts[vertex_layout.0 as usize].vertex_descriptor));

			for (index, attachment) in builder.render_targets.iter().enumerate() {
				if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
					descriptor.setDepthAttachmentPixelFormat(utils::to_pixel_format(attachment.format));
				} else {
					let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
					color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
					match attachment.blend {
						crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
						crate::pipelines::raster::BlendMode::Alpha => {
							color_attachment.setBlendingEnabled(true);
							color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setSourceRGBBlendFactor(mtl::MTLBlendFactor::SourceAlpha);
							color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
							color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
							color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
						}
					}
				}
			}

			self.device.newRenderPipelineStateWithDescriptor_error(&descriptor).ok()
		} else {
			None
		};

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Raster(raster_pipeline_state),
			depth_stencil_state,
			layout,
			vertex_layout: Some(vertex_layout),
			shader_handles,
			resource_access,
			compute_threadgroup_size: None,
			object_threadgroup_size,
			mesh_threadgroup_size,
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		});

		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_handle = *builder.shader.handle;
		let compute_pipeline_state = {
			let shader_parameter = &builder.shader;
			let shader = &self.shaders[shader_handle.0 as usize];
			assert!(
				shader.stage == crate::Stages::COMPUTE,
				"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
			);
			let function = self.create_metal_function(shader_parameter).expect(
				"Metal compute pipeline creation requires a Metal shader function. The most likely cause is that this compute shader was created from SPIR-V, which this backend does not translate to MSL.",
			);

			Some(
				self.device
					.newComputePipelineStateWithFunction_error(&function)
					.expect("Metal compute pipeline creation failed. The most likely cause is that the shader function was invalid for compute pipeline creation."),
			)
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);
		let resource_access = self.shaders[shader_handle.0 as usize]
			.shader_binding_descriptors
			.iter()
			.map(|descriptor| {
				(
					(descriptor.set, descriptor.binding),
					(crate::Stages::COMPUTE, descriptor.access),
				)
			})
			.collect::<Vec<_>>();
		let compute_threadgroup_size = self.shaders[shader_handle.0 as usize].threadgroup_size;

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			depth_stencil_state: None,
			layout,
			vertex_layout: None,
			shader_handles,
			resource_access,
			compute_threadgroup_size,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let resource_access = builder
			.shaders
			.iter()
			.flat_map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| ((descriptor.set, descriptor.binding), (shader.stage, descriptor.access)))
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();
		self.pipelines.push(Pipeline {
			pipeline: PipelineState::RayTracing,
			depth_stencil_state: None,
			layout,
			vertex_layout: None,
			shader_handles: HashMap::default(),
			resource_access,
			compute_threadgroup_size: None,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		// TODO: Metal ray tracing pipeline mapping.
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		self.command_buffers.push(CommandBuffer {
			queue_handle,
			name: name.map(str::to_owned),
		});
		graphics_hardware_interface::CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.create_command_buffer_recording_with_frame_key(command_buffer_handle, None)
	}

	pub fn create_command_buffer_recording_with_frame_key<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) -> super::CommandBufferRecording<'a> {
		self.flush_pending_uploads();

		let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
		let queue = &self.queues[command_buffer.queue_handle.0 as usize];
		let mtl_command_buffer = self.create_metal_command_buffer(
			queue.queue.as_ref(),
			command_buffer.name.as_deref(),
			"Metal command buffer creation failed. The most likely cause is that the command queue did not provide a command buffer.",
		);

		super::CommandBufferRecording::new(self, command_buffer_handle, mtl_command_buffer, frame_key, Vec::new())
	}

	pub fn build_buffer<T: Copy>(&mut self, builder: buffer_builder::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();
		let buffer = self.create_buffer_resource(builder.name, size, builder.resource_uses, builder.device_accesses);

		let mut creator = self.buffers.creator();
		creator.add(buffer);

		graphics_hardware_interface::BufferHandle::<T>(creator.into(), std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: Copy>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let master = self.buffers.master();

		for _ in 0..self.frames {
			let buffer = self.create_buffer_resource(builder.name, size, builder.resource_uses, builder.device_accesses);
			self.buffers.add_with_master(buffer, master);
		}

		graphics_hardware_interface::DynamicBufferHandle::<T>(master.into(), std::marker::PhantomData)
	}

	pub fn build_dynamic_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
		let mut first_handle: Option<ImageHandle> = None;
		let mut previous_handle: Option<ImageHandle> = None;

		let master = self.images.master();

		for _ in 0..self.frames {
			let image = self.create_image_resource(
				builder.get_name(),
				builder.extent,
				builder.format,
				builder.resource_uses,
				builder.device_accesses,
				layers,
			);

			self.images.add_with_master(image, master);
		}

		graphics_hardware_interface::DynamicImageHandle(master)
	}

	pub fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().gpu_address
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { &*(buffer.pointer as *const T) }
	}

	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { std::mem::transmute(buffer.pointer) }
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let handle = self.buffers.nth_handle(buffer_handle.into(), 0).unwrap();
		let buffer = self.buffers.resource(handle);
		if buffer.staging.is_some() {
			self.pending_buffer_syncs.push_back(handle);
		}
	}

	fn upload_buffer_from_staging(&mut self, buffer_handle: BufferHandle) {
		let buffer = self.buffers.resource(buffer_handle);

		let Some(staging_handle) = buffer.staging else {
			return;
		};

		let staging = self.buffers.resource(staging_handle);
		let queue = &self.queues[0];
		let command_buffer = self.create_metal_command_buffer(
			queue.queue.as_ref(),
			Some("Buffer Upload"),
			"Metal command buffer creation failed. The most likely cause is that the transfer queue did not provide a command buffer.",
		);
		let blit_encoder = command_buffer.blitCommandEncoder().expect(
			"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		blit_encoder.setLabel(Some(&NSString::from_str("Buffer Upload")));

		unsafe {
			blit_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
				staging.buffer.as_ref(),
				0,
				buffer.buffer.as_ref(),
				0,
				buffer.size as _,
			);
		}

		blit_encoder.endEncoding();
		self.submit_metal_command_buffer(command_buffer.as_ref());
	}

	fn upload_image_from_staging(&mut self, image_handle: ImageHandle) {
		let image = self.images.resource_mut(image_handle);

		let Some(staging) = image.staging.as_ref() else {
			return;
		};

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;
		let staging = staging.to_vec();

		self.upload_texture_from_staging(texture.as_ref(), format, extent, array_layers, &staging);
	}

	fn flush_pending_uploads(&mut self) {
		let pending_buffers = self.pending_buffer_syncs.drain(..).collect::<Vec<_>>();
		for buffer_handle in pending_buffers {
			self.upload_buffer_from_staging(buffer_handle);
		}

		let pending_images = self.pending_image_syncs.drain(..).collect::<Vec<_>>();
		for image_handle in pending_images {
			self.upload_image_from_staging(image_handle);
		}
	}

	pub fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let image = self.images.get_single(texture_handle.0).unwrap();

		let Some(staging) = image.staging.as_ref() else {
			return &mut [];
		};

		unsafe { std::slice::from_raw_parts_mut(staging.as_ptr() as *mut u8, staging.len()) }
	}

	pub fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image = self.images.resource_mut(self.images.nth_handle(texture_handle.0, 0).unwrap());

		let Some(staging) = image.staging.as_mut() else {
			return;
		};

		f(staging);

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;
		let staging = staging.to_vec();

		self.upload_texture_from_staging(texture.as_ref(), format, extent, array_layers, &staging);
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		let handle = self.images.nth_handle(image_handle.0, 0).unwrap();
		self.pending_image_syncs.push_back(handle);
	}

	pub fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);

		let image = self.create_image_resource(
			builder.get_name(),
			builder.extent,
			builder.format,
			builder.resource_uses,
			builder.device_accesses,
			layers,
		);

		let image_handle = self.images.add(image);

		graphics_hardware_interface::ImageHandle(image_handle.0)
	}

	pub fn build_sampler(&mut self, builder: sampler_builder::Builder) -> graphics_hardware_interface::SamplerHandle {
		let descriptor = mtl::MTLSamplerDescriptor::new();
		descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
		descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
		descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
		descriptor.setSAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setTAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setRAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setLodMinClamp(builder.min_lod);
		descriptor.setLodMaxClamp(builder.max_lod);
		descriptor.setSupportArgumentBuffers(true);

		if let Some(anisotropy) = builder.anisotropy {
			descriptor.setMaxAnisotropy(anisotropy as _);
		}

		let sampler_state = self
			.device
			.newSamplerStateWithDescriptor(&descriptor)
			.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.");
		self.samplers.push(super::sampler::Sampler { sampler: sampler_state });
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<mtl::MTLAccelerationStructureInstanceDescriptor>();
		let buffer = self.create_buffer_resource(
			name,
			size,
			crate::Uses::AccelerationStructure,
			crate::DeviceAccesses::DeviceOnly,
		);
		let mut creator = self.buffers.creator();

		creator.add(buffer);

		creator.into()
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure and backing buffer.
		graphics_hardware_interface::TopLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		_description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure for mesh or AABB.
		graphics_hardware_interface::BottomLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	/// Applies descriptor writes to the Metal-backed bindings for every frame they target.
	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		for write in descriptor_set_writes {
			self.apply_descriptor_write_to_all_frames(
				DescriptorSetBindingHandle(write.binding_handle.0),
				write.descriptor,
				write.array_element,
				write.frame_offset.unwrap_or(0),
			);
		}
	}

	pub fn write_instance(
		&mut self,
		_instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_instance_index: usize,
		_transform: [[f32; 4]; 3],
		_custom_index: u16,
		_mask: u8,
		_sbt_record_offset: usize,
		_acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		// TODO: Populate MTLAccelerationStructureInstanceDescriptor buffer.
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: graphics_hardware_interface::PipelineHandle,
		_shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		// TODO: Metal ray tracing shader binding table mapping.
	}

	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		_presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();
		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(mtl::MTLPixelFormat::BGRA8Unorm);
		layer.setFramebufferOnly(false); // If true, higher perfomance but can only write to image from raster render pass

		window_os_handles.view.setWantsLayer(true);
		window_os_handles.view.setLayer(Some(layer.as_super()));
		let extent = get_layer_extent(&layer, &window_os_handles.view);

		let format = mtl::MTLPixelFormat::BGRA8Unorm;

		let needs_proxies = {
			true // Force proxy creation, easier to handle descriptors, for now at least
		};

		let format = match format {
			mtl::MTLPixelFormat::BGRA8Unorm => crate::Formats::BGRAu8,
			mtl::MTLPixelFormat::BGRA8Unorm_sRGB => crate::Formats::BGRAsRGB,
			_ => panic!(
				"Unsupported Metal swapchain pixel format. The most likely cause is that the layer pixel format does not have a matching GHI format."
			),
		};

		let mut images = [None; super::MAX_SWAPCHAIN_IMAGES];

		if needs_proxies {
			// Create proxies for every swapchain image

			for image_index in 0..super::MAX_SWAPCHAIN_IMAGES {
				let proxy = self.create_image_resource(
					Some("Swapchain Proxy Image"),
					extent,
					format,
					uses | Uses::BlitSource,
					DeviceAccesses::DeviceOnly,
					1,
				);

				let image_handle = self.images.add(proxy);

				images[image_index] = Some(image_handle.1);
			}
		}

		let handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			layer,
			view: window_os_handles.view.clone(),
			extent,
			images,
		});

		handle
	}

	pub fn get_image_data<'a>(&'a self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		self.texture_copies
			.get(texture_copy_handle.0 as usize)
			.map(|data| data.as_slice())
			.unwrap_or(&[])
	}

	pub fn create_synchronizer(
		&mut self,
		_name: Option<&str>,
		signaled: bool,
	) -> graphics_hardware_interface::SynchronizerHandle {
		self.synchronizers.push(synchronizer::Synchronizer { next: None, signaled });
		graphics_hardware_interface::SynchronizerHandle((self.synchronizers.len() - 1) as u64)
	}

	pub fn start_frame<'a>(
		&'a mut self,
		index: u32,
		_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> super::Frame<'a> {
		let frame_key = graphics_hardware_interface::FrameKey {
			frame_index: index,
			sequence_index: (index % self.frames as u32) as u8,
		};
		self.process_tasks(frame_key.sequence_index);
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let buffer = self.buffers.get_single(buffer_handle).unwrap();

		if buffer.size >= size {
			return;
		}

		let uses = buffer.uses;
		let access = buffer.access;
		let name = buffer.name.clone();

		let replacement = self.create_buffer_resource(name.as_deref(), size, uses, access);

		let handle = self.buffers.nth_handle(buffer_handle, 0).unwrap();

		*self.buffers.resource_mut(handle) = replacement;

		self.rewrite_descriptors_for_handle(PrivateHandles::Buffer(handle));
	}

	pub fn start_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn end_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn wait(&self) {
		// TODO: Track pending command buffers and wait for completion.
	}
}
