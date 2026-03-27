use std::ptr::NonNull;

use ::utils::hash::HashMap;
use objc2_foundation::NSString;
use objc2_metal::{
	MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLComputeCommandEncoder, MTLRenderCommandEncoder, MTLTexture,
};

use super::*;
use crate::command_buffer::{
	BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
	CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode, RasterizationRenderPassMode,
};

const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

pub struct CommandBufferRecording<'a> {
	device: &'a mut device::Device,
	command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	sequence_index: u8,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	present_drawables: Vec<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
	states: HashMap<Handle, TransitionState>,
	active_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<(u32, descriptor_set::DescriptorSetHandle)>,
	bound_vertex_buffers: Vec<(graphics_hardware_interface::BaseBufferHandle, usize)>,
	bound_vertex_layout: Option<VertexLayoutHandle>,
	bound_index_buffer: Option<(graphics_hardware_interface::BaseBufferHandle, usize, crate::DataTypes)>,
	push_constant_data: Vec<u8>,
	active_compute_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>>>,
	active_render_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLRenderCommandEncoder>>>,
}

pub struct FinishedCommandBuffer<'a> {
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	pub(crate) command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	pub(crate) present_drawables: Vec<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
	pub(crate) states: HashMap<Handle, TransitionState>,
	pub(crate) present_keys: &'a [graphics_hardware_interface::PresentKey],
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut device::Device,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let states = device.states.clone();

		Self {
			device,
			command_buffer_handle,
			sequence_index,
			command_buffer,
			present_drawables: Vec::new(),
			states,
			active_pipeline_layout: None,
			bound_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_set_handles: Vec::new(),
			bound_vertex_buffers: Vec::new(),
			bound_vertex_layout: None,
			bound_index_buffer: None,
			push_constant_data: Vec::new(),
			active_compute_encoder: None,
			active_render_encoder: None,
		}
	}

	fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>> {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		if self.active_compute_encoder.is_none() {
			self.active_compute_encoder = Some(self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			));
		}

		self.active_compute_encoder.as_ref().unwrap()
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> buffer::BufferHandle {
		let handles = buffer::BufferHandle(handle.0).get_all(&self.device.buffers);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> image::ImageHandle {
		if let Some(swapchain) = self
			.device
			.swapchains
			.iter()
			.find(|swapchain| swapchain.images[0].map(|image| image.0 == handle.0).unwrap_or(false))
		{
			return swapchain.images[swapchain.acquired_image_indices[self.sequence_index as usize] as usize].expect(
				"Missing Metal swapchain proxy image for the acquired drawable. The most likely cause is that get_swapchain_image was not called before using the swapchain image.",
			);
		}

		let handles = image::ImageHandle(handle.0).get_all(&self.device.images);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	fn consume_resources(&mut self, consumptions: impl IntoIterator<Item = Consumption>) {
		for consumption in consumptions {
			self.states.insert(
				consumption.handle,
				TransitionState {
					layout: consumption.layout,
				},
			);
		}
	}

	fn consume_bound_descriptor_resources(&mut self) {
		let Some(bound_pipeline_handle) = self.bound_pipeline else {
			return;
		};

		let pipeline = &self.device.pipelines[bound_pipeline_handle.0 as usize];
		let mut consumptions = Vec::with_capacity(pipeline.resource_access.len());

		for &((set_index, binding_index), (stages, access)) in &pipeline.resource_access {
			let Some(&(_, descriptor_set_handle)) = self.bound_descriptor_set_handles.get(set_index as usize) else {
				continue;
			};
			let frame_state =
				&self.device.descriptor_sets[descriptor_set_handle.0 as usize].frames[self.sequence_index as usize];
			let Some(descriptors) = frame_state.descriptors.get(&binding_index) else {
				continue;
			};

			for descriptor in descriptors.values().copied() {
				let Some(handle) = descriptor.tracked_resource() else {
					continue;
				};
				let layout = match descriptor {
					Descriptor::Buffer { .. } => crate::Layouts::General,
					Descriptor::Image { layout, .. } | Descriptor::CombinedImageSampler { layout, .. } => layout,
					Descriptor::Sampler { .. } => continue,
				};

				consumptions.push(Consumption {
					handle,
					stages,
					access,
					layout,
				});
			}
		}

		self.consume_resources(consumptions);
	}

	fn resize_push_constants_for_layout(&mut self, pipeline_layout: graphics_hardware_interface::PipelineLayoutHandle) {
		let push_constant_size = self.device.pipeline_layouts[pipeline_layout.0 as usize].push_constant_size;
		self.push_constant_data.clear();
		self.push_constant_data.resize(push_constant_size, 0);
	}

	fn apply_bound_vertex_buffers(&self) {
		let Some(encoder) = self.active_render_encoder.as_ref() else {
			return;
		};

		for (binding, (buffer_handle, offset)) in self.bound_vertex_buffers.iter().copied().enumerate() {
			let buffer = &self.device.buffers[self.get_internal_buffer_handle(buffer_handle).0 as usize];
			unsafe {
				encoder.setVertexBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), offset as _, binding as _);
			}
		}
	}

	fn apply_push_constants(&self) {
		if self.push_constant_data.is_empty() {
			return;
		}

		let pointer = NonNull::new(self.push_constant_data.as_ptr() as *mut std::ffi::c_void)
			.expect("Push constant data pointer was null. The most likely cause is an empty push constant buffer upload.");

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			unsafe {
				encoder.setVertexBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setFragmentBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
			}
		}

		if let Some(encoder) = self.active_compute_encoder.as_ref() {
			unsafe {
				encoder.setBytes_length_atIndex(pointer, self.push_constant_data.len() as _, PUSH_CONSTANT_BINDING_INDEX as _);
			}
		}
	}

	fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		for drawable in &self.present_drawables {
			let drawable_ref: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			self.command_buffer.presentDrawable(drawable_ref);
		}

		self.command_buffer.commit();
		self.command_buffer.waitUntilCompleted();

		if let Some(synchronizer) = self.device.synchronizers.get_mut(synchronizer.0 as usize) {
			synchronizer.signaled = true;
		}

		self.device.states = self.states;
	}

	fn take_drawable(
		&mut self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
		let swapchain = &mut self.device.swapchains[present_key.swapchain.0 as usize];
		swapchain.take_drawable(present_key.image_index)
	}

	fn collect_present_drawables(&mut self, present_keys: &[graphics_hardware_interface::PresentKey]) {
		for &present_key in present_keys {
			if let Some(drawable) = self.take_drawable(present_key) {
				self.present_drawables.push(drawable);
			}
		}
	}
}

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	type Result<'a> = FinishedCommandBuffer<'a>;

	fn build_top_level_acceleration_structure(
		&mut self,
		_acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		let consumptions = attachments
			.iter()
			.map(|attachment| Consumption {
				handle: Handle::Image(self.get_internal_image_handle(attachment.image)),
				stages: crate::Stages::FRAGMENT,
				access: if attachment.load {
					crate::AccessPolicies::READ_WRITE
				} else {
					crate::AccessPolicies::WRITE
				},
				layout: attachment.layout,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		let rpd = mtl::MTLRenderPassDescriptor::new();

		for (i, attachment) in attachments
			.iter()
			.filter(|attachment| attachment.format != crate::Formats::Depth32)
			.enumerate()
		{
			let att = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(i) };
			let image = &self.device.images[self.get_internal_image_handle(attachment.image).0 as usize];

			att.setTexture(Some(image.texture.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearColor(utils::clear_color(attachment.clear));
		}

		if let Some(attachment) = attachments
			.iter()
			.find(|attachment| attachment.format == crate::Formats::Depth32)
		{
			let att = unsafe { rpd.depthAttachment() };
			let image = &self.device.images[self.get_internal_image_handle(attachment.image).0 as usize];

			att.setTexture(Some(image.texture.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearDepth(utils::clear_depth(attachment.clear));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).unwrap();

		rce.setViewport(mtl::MTLViewport {
			originX: 0.0,
			originY: 0.0,
			width: extent.width() as f64,
			height: extent.height() as f64,
			znear: 0.0,
			zfar: 1.0,
		});
		rce.setScissorRect(mtl::MTLScissorRect {
			x: 0,
			y: 0,
			width: extent.width() as _,
			height: extent.height() as _,
		});

		self.active_render_encoder = Some(rce);
		self.apply_bound_vertex_buffers();
		self.apply_push_constants();

		self
	}

	fn clear_images<I: graphics_hardware_interface::ImageHandleLike>(
		&mut self,
		textures: &[(I, graphics_hardware_interface::ClearValue)],
	) {
		let consumptions = textures
			.iter()
			.map(|(handle, _)| Consumption {
				handle: Handle::Image(self.get_internal_image_handle((*handle).into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		// TODO: Encode blit clears for textures.
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		let consumptions = buffer_handles
			.iter()
			.map(|buffer_handle| Consumption {
				handle: Handle::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		// TODO: Encode fillBuffer on MTLBlitCommandEncoder.
	}

	fn transfer_textures(
		&mut self,
		texture_handles: &[impl graphics_hardware_interface::ImageHandleLike],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		let consumptions = texture_handles
			.iter()
			.map(|handle| Consumption {
				handle: Handle::Image(self.get_internal_image_handle((*handle).into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		texture_handles
			.iter()
			.map(|handle| {
				self.device
					.copy_texture_to_cpu(self.get_internal_image_handle(handle.into_image_handle()))
			})
			.collect()
	}

	fn write_image_data(
		&mut self,
		image_handle: impl graphics_hardware_interface::ImageHandleLike,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle.into_image_handle());
		self.consume_resources([Consumption {
			handle: Handle::Image(image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}]);

		let image = &mut self.device.images[image_handle.0 as usize];
		let Some(staging) = image.staging.as_mut() else {
			return;
		};
		let bytes = unsafe {
			std::slice::from_raw_parts(
				data.as_ptr() as *const u8,
				data.len() * std::mem::size_of::<graphics_hardware_interface::RGBAu8>(),
			)
		};
		let length = staging.len().min(bytes.len());
		staging[..length].copy_from_slice(&bytes[..length]);

		let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
			return;
		};
		let width = image.extent.width() as usize;
		let height = image.extent.height() as usize;
		let bytes_per_row = width * bytes_per_pixel;
		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: mtl::MTLSize {
				width: width as _,
				height: height as _,
				depth: 1,
			},
		};
		let staging_ptr = NonNull::new(staging.as_ptr() as *mut std::ffi::c_void)
			.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");

		unsafe {
			image
				.texture
				.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
		}

		self.consume_resources([Consumption {
			handle: Handle::Image(image_handle),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}]);
	}

	fn blit_image(
		&mut self,
		source_image: impl graphics_hardware_interface::ImageHandleLike,
		_source_layout: crate::Layouts,
		destination_image: impl graphics_hardware_interface::ImageHandleLike,
		_destination_layout: crate::Layouts,
	) {
		self.consume_resources([
			Consumption {
				handle: Handle::Image(self.get_internal_image_handle(source_image.into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			},
			Consumption {
				handle: Handle::Image(self.get_internal_image_handle(destination_image.into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			},
		]);

		// TODO: Encode MTLBlitCommandEncoder copyFromTexture.
	}

	fn execute(self, _synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.finish(_synchronizer);
	}

	fn end<'a>(mut self, present_keys: &'a [graphics_hardware_interface::PresentKey]) -> Self::Result<'a> {
		self.collect_present_drawables(present_keys);

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer,
			present_drawables: self.present_drawables,
			states: self.states,
			present_keys,
		}
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let pipeline_layout = pipeline.layout;
		let pipeline_state = pipeline.pipeline.clone();
		self.active_pipeline_layout = Some(pipeline_layout);
		self.bound_pipeline_layout = None;
		self.resize_push_constants_for_layout(pipeline_layout);

		if let PipelineState::Compute(Some(compute_pipeline_state)) = &pipeline_state {
			self.ensure_compute_encoder().setComputePipelineState(compute_pipeline_state);
		}

		self.apply_push_constants();

		self
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.active_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self.bound_pipeline_layout = None;
		self
	}

	fn start_region(&self, name: &str) {
		self.command_buffer.pushDebugGroup(&NSString::from_str(name));
	}

	fn end_region(&self) {
		self.command_buffer.popDebugGroup();
	}

	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
		self.start_region(name);
		f(self);
		self.end_region();
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let pipeline_layout = pipeline.layout;
		let pipeline_vertex_layout = pipeline.vertex_layout;
		let pipeline_state = pipeline.pipeline.clone();
		let face_winding = pipeline.face_winding;
		let cull_mode = pipeline.cull_mode;

		self.active_pipeline_layout = Some(pipeline_layout);
		self.bound_pipeline_layout = None;
		self.resize_push_constants_for_layout(pipeline_layout);

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			encoder.setFrontFacingWinding(utils::winding(face_winding));
			encoder.setCullMode(utils::cull_mode(cull_mode));

			if self.bound_vertex_layout != pipeline_vertex_layout {
				if let PipelineState::Raster(Some(render_pipeline_state)) = &pipeline_state {
					encoder.setRenderPipelineState(render_pipeline_state);
				}
				self.bound_vertex_layout = pipeline_vertex_layout;
			}
		}
		self.apply_bound_vertex_buffers();
		self.apply_push_constants();

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		self.bound_vertex_buffers = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| (buffer_descriptor.buffer, buffer_descriptor.offset))
			.collect();

		let consumptions = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| Consumption {
				handle: Handle::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer)),
				stages: crate::Stages::VERTEX,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::General,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		self.apply_bound_vertex_buffers();
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		self.consume_resources([Consumption {
			handle: Handle::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer)),
			stages: crate::Stages::INDEX,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::General,
		}]);

		let index_type = buffer_descriptor.index_type.expect(
			"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32).",
		);

		self.bound_index_buffer = Some((buffer_descriptor.buffer, buffer_descriptor.offset, index_type));
	}

	fn end_render_pass(&mut self) {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		if sets.is_empty() {
			return self;
		}

		let pipeline_layout_handle = self.active_pipeline_layout.expect(
			"No pipeline layout is active. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];

		for descriptor_set_handle in sets {
			let descriptor_set_handle = descriptor_set::DescriptorSetHandle(descriptor_set_handle.0);
			let descriptor_set = &self.device.descriptor_sets[descriptor_set_handle.0 as usize];
			let set_index = *pipeline_layout
				.descriptor_set_template_indices
				.get(&descriptor_set.descriptor_set_layout)
				.expect(
					"Descriptor set layout not found in the active Metal pipeline layout. The most likely cause is that a descriptor set incompatible with the currently bound pipeline was bound.",
				);

			if (set_index as usize) < self.bound_descriptor_set_handles.len() {
				self.bound_descriptor_set_handles[set_index as usize] = (set_index, descriptor_set_handle);
				self.bound_descriptor_set_handles.truncate(set_index as usize + 1);
			} else {
				assert_eq!(set_index as usize, self.bound_descriptor_set_handles.len());
				self.bound_descriptor_set_handles.push((set_index, descriptor_set_handle));
			}
		}

		let bound_pipeline = self.bound_pipeline.expect(
			"No pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		let pipeline = self.device.pipelines[bound_pipeline.0 as usize].clone();

		for &(set_index, descriptor_set_handle) in &self.bound_descriptor_set_handles {
			let descriptor_set = &self.device.descriptor_sets[descriptor_set_handle.0 as usize];
			let descriptor_set_layout = &self.device.descriptor_sets_layouts[descriptor_set.descriptor_set_layout.0 as usize];
			let frame_state = &descriptor_set.frames[self.sequence_index as usize];
			let binding_index = ARGUMENT_BUFFER_BINDING_BASE + set_index;

			match &pipeline.pipeline {
				PipelineState::Raster(_) => {
					if let Some(encoder) = self.active_render_encoder.as_ref() {
						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::VERTEX))
						{
							unsafe {
								encoder.setVertexBuffer_offset_atIndex(
									Some(frame_state.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}

						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::FRAGMENT))
						{
							unsafe {
								encoder.setFragmentBuffer_offset_atIndex(
									Some(frame_state.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}
					}
				}
				PipelineState::Compute(_) => {
					if let Some(encoder) = self.active_compute_encoder.as_ref() {
						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::COMPUTE))
						{
							unsafe {
								encoder.setBuffer_offset_atIndex(
									Some(frame_state.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}
					}
				}
				PipelineState::RayTracing => {}
			}
		}

		self.consume_bound_descriptor_resources();
		self.bound_pipeline_layout = self.active_pipeline_layout;
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let pipeline_layout_handle = self.active_pipeline_layout.expect(
			"No pipeline bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let end = offset as usize + std::mem::size_of::<T>();

		assert!(
			end <= pipeline_layout.push_constant_size,
			"Push constant write exceeds the Metal pipeline layout push constant storage. The most likely cause is that the write offset or type size does not match the pipeline's declared push constant ranges.",
		);

		if self.push_constant_data.len() < pipeline_layout.push_constant_size {
			self.resize_push_constants_for_layout(pipeline_layout_handle);
		}

		unsafe {
			std::ptr::copy_nonoverlapping(
				&data as *const T as *const u8,
				self.push_constant_data[offset as usize..end].as_mut_ptr(),
				std::mem::size_of::<T>(),
			);
		}

		self.apply_push_constants();
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, _mesh_handle: &graphics_hardware_interface::MeshHandle) {
		// TODO: Issue draw call using mesh buffers.
	}

	fn draw(&mut self, vertex_count: u32, _instance_count: u32, first_vertex: u32, _first_instance: u32) {
		unsafe {
			self.active_render_encoder
				.as_ref()
				.unwrap()
				.drawPrimitives_vertexStart_vertexCount(mtl::MTLPrimitiveType::Triangle, first_vertex as _, vertex_count as _);
		}
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		let (buffer_handle, offset, index_type) = self
			.bound_index_buffer
			.expect("No index buffer bound. The most likely cause is that draw_indexed was called before bind_index_buffer.");
		let buffer = &self.device.buffers[self.get_internal_buffer_handle(buffer_handle).0 as usize];
		let (metal_index_type, index_size) = match index_type {
			crate::DataTypes::U16 => (mtl::MTLIndexType::UInt16, std::mem::size_of::<u16>()),
			crate::DataTypes::U32 => (mtl::MTLIndexType::UInt32, std::mem::size_of::<u32>()),
			_ => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
		};
		let index_buffer_offset = offset + first_index as usize * index_size;

		unsafe {
			self.active_render_encoder
				.as_ref()
				.unwrap()
				.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(
					mtl::MTLPrimitiveType::Triangle,
					index_count as _,
					metal_index_type,
					buffer.buffer.as_ref(),
					index_buffer_offset as _,
					instance_count as _,
					vertex_offset as _,
					first_instance as _,
				);
		}
	}

	fn dispatch_meshes(&mut self, _x: u32, _y: u32, _z: u32) {
		// TODO: Map mesh shading to Metal mesh shaders when supported.
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let threadgroups = dispatch.get_extent();
		let threads_per_threadgroup = dispatch.get_workgroup_extent();

		self.ensure_compute_encoder().dispatchThreadgroups_threadsPerThreadgroup(
			mtl::MTLSize {
				width: threadgroups.width() as _,
				height: threadgroups.height() as _,
				depth: threadgroups.depth() as _,
			},
			mtl::MTLSize {
				width: threads_per_threadgroup.width() as _,
				height: threads_per_threadgroup.height() as _,
				depth: threads_per_threadgroup.depth() as _,
			},
		);
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		_buffer: graphics_hardware_interface::BufferHandle<[(u32, u32, u32); N]>,
		_entry_index: usize,
	) {
		// TODO: Encode indirect dispatch.
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: Encode Metal ray tracing dispatch.
	}
}
