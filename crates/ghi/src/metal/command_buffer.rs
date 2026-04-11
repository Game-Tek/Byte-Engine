use std::{cell::RefCell, ptr::NonNull};

use ::utils::{hash::HashMap, Extent};
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSRange, NSString};
use objc2_metal::{
	MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLComputeCommandEncoder, MTLRenderCommandEncoder, MTLTexture,
};

use super::*;
use crate::{
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode, RasterizationRenderPassMode,
	},
	descriptors::DescriptorSetHandle,
	HandleLike as _, ImageOrSwapchain, PrivateHandles,
};

const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

fn attachment_texture_view(
	texture: &Retained<ProtocolObject<dyn mtl::MTLTexture>>,
	format: crate::Formats,
	array_layers: u32,
	layer: Option<u32>,
) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
	if let Some(layer) = layer {
		if array_layers > 1 {
			unsafe {
				return texture
					.newTextureViewWithPixelFormat_textureType_levels_slices(
						utils::to_pixel_format(format),
						mtl::MTLTextureType::Type2D,
						NSRange::new(0, 1),
						NSRange::new(layer as usize, 1),
					)
					.expect(
						"Metal texture view creation failed. The most likely cause is an invalid array-layer render target view.",
					);
			}
		}
	}

	texture.clone()
}

fn replace_texture_from_bytes(
	texture: &ProtocolObject<dyn mtl::MTLTexture>,
	format: crate::Formats,
	extent: Extent,
	array_layers: u32,
	bytes: &[u8],
) {
	let Some(bytes_per_pixel) = utils::bytes_per_pixel(format) else {
		return;
	};

	let width = extent.width().max(1) as usize;
	let height = extent.height().max(1) as usize;
	let bytes_per_row = width * bytes_per_pixel;
	let bytes_per_image = bytes_per_row * height;
	let region = mtl::MTLRegion {
		origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
		size: mtl::MTLSize {
			width: width as _,
			height: height as _,
			depth: 1,
		},
	};

	for slice in 0..array_layers as usize {
		let offset = slice * bytes_per_image;
		let end = offset + bytes_per_image;
		let Some(slice_bytes) = bytes.get(offset..end) else {
			break;
		};
		let staging_ptr = NonNull::new(slice_bytes.as_ptr() as *mut std::ffi::c_void)
			.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");

		unsafe {
			if array_layers > 1 {
				texture.replaceRegion_mipmapLevel_slice_withBytes_bytesPerRow_bytesPerImage(
					region,
					0,
					slice,
					staging_ptr,
					bytes_per_row as _,
					bytes_per_image as _,
				);
			} else {
				texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
			}
		}
	}
}

// Encodes a render-pass clear for one Metal texture, clearing every array layer individually when needed.
fn encode_texture_clear(
	command_buffer: &ProtocolObject<dyn mtl::MTLCommandBuffer>,
	texture: &Retained<ProtocolObject<dyn mtl::MTLTexture>>,
	format: crate::Formats,
	array_layers: u32,
	clear_value: graphics_hardware_interface::ClearValue,
) {
	let slice_count = array_layers.max(1);

	for slice in 0..slice_count {
		let rpd = mtl::MTLRenderPassDescriptor::new();
		let texture_view = attachment_texture_view(texture, format, array_layers, (array_layers > 1).then_some(slice));

		if format == crate::Formats::Depth32 {
			let attachment = rpd.depthAttachment();
			attachment.setTexture(Some(texture_view.as_ref()));
			attachment.setLoadAction(mtl::MTLLoadAction::Clear);
			attachment.setStoreAction(mtl::MTLStoreAction::Store);
			attachment.setClearDepth(utils::clear_depth(clear_value));
		} else {
			let attachment = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(0) };
			attachment.setTexture(Some(texture_view.as_ref()));
			attachment.setLoadAction(mtl::MTLLoadAction::Clear);
			attachment.setStoreAction(mtl::MTLStoreAction::Store);
			attachment.setClearColor(utils::clear_color(clear_value));
		}

		let encoder = command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
			"Metal render command encoder creation failed. The most likely cause is that the command buffer could not start an image clear pass.",
		);
		let label = NSString::from_str("Image Clear");
		encoder.setLabel(Some(&label));
		encoder.endEncoding();
	}
}

pub struct CommandBufferRecording<'a> {
	device: &'a mut device::Device,
	command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	sequence_index: u8,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	debug_regions: RefCell<Vec<String>>,
	states: HashMap<PrivateHandles, TransitionState>,
	active_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<(u32, DescriptorSetHandle)>,
	bound_vertex_buffers: Vec<(graphics_hardware_interface::BaseBufferHandle, usize)>,
	bound_vertex_layout: Option<VertexLayoutHandle>,
	bound_index_buffer: Option<(graphics_hardware_interface::BaseBufferHandle, usize, crate::DataTypes)>,
	push_constant_data: Vec<u8>,
	active_compute_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>>>,
	active_render_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLRenderCommandEncoder>>>,
	drawables: Vec<(
		graphics_hardware_interface::SwapchainHandle,
		Retained<ProtocolObject<dyn CAMetalDrawable>>,
	)>,
}

pub struct FinishedCommandBuffer<'a> {
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	pub(crate) command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	pub(crate) states: HashMap<PrivateHandles, TransitionState>,
	pub(crate) present_keys: &'a [graphics_hardware_interface::PresentKey],
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut device::Device,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		drawables: Vec<(
			graphics_hardware_interface::SwapchainHandle,
			Retained<ProtocolObject<dyn CAMetalDrawable>>,
		)>,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let states = device.states.clone();

		Self {
			device,
			command_buffer_handle,
			sequence_index,
			command_buffer,
			debug_regions: RefCell::new(Vec::new()),
			states,
			drawables,
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

	fn current_encoder_label(&self, suffix: &str) -> Retained<NSString> {
		NSString::from_str(suffix)
	}

	fn refresh_active_encoder_labels(&self) {
		if let Some(encoder) = self.active_compute_encoder.as_ref() {
			let label = self.current_encoder_label("Compute Pass");
			encoder.setLabel(Some(&label));
		}

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			let label = self.current_encoder_label("Render Pass");
			encoder.setLabel(Some(&label));
		}
	}

	fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>> {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		if self.active_compute_encoder.is_none() {
			let encoder = self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			let label = self.current_encoder_label("Compute Pass");
			encoder.setLabel(Some(&label));
			self.active_compute_encoder = Some(encoder);
		}

		self.active_compute_encoder.as_ref().unwrap()
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device.images.nth_handle(handle, self.sequence_index as _).unwrap()
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
			let descriptor_set = &self.device.descriptor_sets[descriptor_set_handle.0 as usize];
			let Some(descriptors) = descriptor_set.descriptors.get(&binding_index) else {
				continue;
			};

			for descriptor in descriptors.values().copied() {
				let (handle, layout) = match descriptor {
					Descriptor::Buffer { buffer, .. } => (PrivateHandles::Buffer(buffer), crate::Layouts::General),
					Descriptor::Image { image, layout, .. } => (PrivateHandles::Image(image), layout),
					Descriptor::CombinedImageSampler { image, layout, .. } => (PrivateHandles::Image(image), layout),
					Descriptor::Sampler { .. } => continue,
					Descriptor::Swapchain { handle } => {
						let swapchain = &self.device.swapchains[handle.0 as usize];
						if let Some(proxy_image_handle) = swapchain.images[self.sequence_index as usize] {
							(PrivateHandles::Image(proxy_image_handle), crate::Layouts::General)
						} else {
							continue;
						}
					}
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
			let buffer = &self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
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
				encoder.setObjectBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setMeshBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
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

		self.device.submit_metal_command_buffer(self.command_buffer.as_ref());

		if let Some(synchronizer) = self.device.synchronizers.get_mut(synchronizer.0 as usize) {
			synchronizer.signaled = true;
		}

		self.device.states = self.states;
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

		let attachments = attachments.iter().map(|attachment| match attachment.target {
			ImageOrSwapchain::Image(image) => {
				let image = self.device.images.resource(self.get_internal_image_handle(image));

				(attachment, image.texture.clone(), image.format, image.array_layers)
			}
			ImageOrSwapchain::Swapchain(swapchain) => {
				let drawable = self
					.drawables
					.iter()
					.find(|(handle, _)| *handle == swapchain)
					.expect("Swapchain image not found");

				(attachment, drawable.1.texture(), crate::Formats::BGRAu8, 1) // TODO: get actual format
			}
		});

		// let consumptions = attachments
		// 	.filter_map(|(attachment, _, _)| Some(Consumption {
		// 		handle: {
		// 			match attachment.target {
		// 				ImageOrSwapchain::Image(image) => {
		// 					PrivateHandles::Image(self.get_internal_image_handle(image))
		// 				}
		// 				ImageOrSwapchain::Swapchain(_) => {
		// 					return None;
		// 				},
		// 			}
		// 		},
		// 		stages: crate::Stages::FRAGMENT,
		// 		access: if attachment.load {
		// 			crate::AccessPolicies::READ_WRITE
		// 		} else {
		// 			crate::AccessPolicies::WRITE
		// 		},
		// 		layout: attachment.layout,
		// 	}))
		// 	.collect::<Vec<_>>();
		// self.consume_resources(consumptions);

		let rpd = mtl::MTLRenderPassDescriptor::new();

		for (i, (attachment, image, format, array_layers)) in attachments
			.clone()
			.filter(|(_, _, format, _)| *format != crate::Formats::Depth32)
			.enumerate()
		{
			let att = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(i) };
			let texture_view = attachment_texture_view(&image, format, array_layers, attachment.layer);

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearColor(utils::clear_color(attachment.clear));
		}

		if let Some((attachment, image, format, array_layers)) = attachments
			.clone()
			.find(|(_, _, format, _)| format == &crate::Formats::Depth32)
		{
			let att = unsafe { rpd.depthAttachment() };
			let texture_view = attachment_texture_view(&image, format, array_layers, attachment.layer);

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearDepth(utils::clear_depth(attachment.clear));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).unwrap();
		let label = self.current_encoder_label("Render Pass");
		rce.setLabel(Some(&label));

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

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		let consumptions = textures
			.iter()
			.map(|(handle, _)| Consumption {
				handle: PrivateHandles::Image(self.get_internal_image_handle(*handle)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		for (handle, clear_value) in textures {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);

			encode_texture_clear(
				self.command_buffer.as_ref(),
				&image.texture,
				image.format,
				image.array_layers,
				*clear_value,
			);
		}
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		let consumptions = buffer_handles
			.iter()
			.map(|buffer_handle| Consumption {
				handle: PrivateHandles::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		let blit_encoder = self.command_buffer.blitCommandEncoder().expect(
			"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		let label = self.current_encoder_label("Buffer Clear");
		blit_encoder.setLabel(Some(&label));

		for buffer_handle in buffer_handles {
			let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(*buffer_handle));
			unsafe {
				blit_encoder.fillBuffer_range_value(buffer.buffer.as_ref(), NSRange::new(0, buffer.size), 0);
			}
		}

		blit_encoder.endEncoding();
	}

	fn transfer_textures(
		&mut self,
		texture_handles: &[graphics_hardware_interface::BaseImageHandle],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		let consumptions = texture_handles
			.iter()
			.map(|handle| Consumption {
				handle: PrivateHandles::Image(self.get_internal_image_handle(*handle)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		texture_handles
			.iter()
			.map(|handle| self.device.copy_texture_to_cpu(self.get_internal_image_handle(*handle)))
			.collect()
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle);

		self.consume_resources([Consumption {
			handle: PrivateHandles::Image(image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}]);

		let image = self.device.images.resource_mut(image_handle);

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

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;

		replace_texture_from_bytes(texture.as_ref(), format, extent, array_layers, staging);

		self.consume_resources([Consumption {
			handle: PrivateHandles::Image(image_handle),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}]);
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		_source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		_destination_layout: crate::Layouts,
	) {
		let source_internal = self.get_internal_image_handle(source_image);
		let destination_internal = self.get_internal_image_handle(destination_image);

		self.consume_resources([
			Consumption {
				handle: PrivateHandles::Image(source_internal),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			},
			Consumption {
				handle: PrivateHandles::Image(destination_internal),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			},
		]);

		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		let source_texture = &self.device.images.resource(source_internal).texture;
		let destination_texture = &self.device.images.resource(destination_internal).texture;

		let blit_encoder = self.command_buffer.blitCommandEncoder().expect(
			"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		blit_encoder.setLabel(Some(&NSString::from_str("Blit Pass")));

		unsafe {
			blit_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
		}

		blit_encoder.endEncoding();
	}

	fn execute(self, _synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.finish(_synchronizer);
	}

	fn end<'a>(mut self, present_keys: &'a [graphics_hardware_interface::PresentKey]) -> Self::Result<'a> {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		if let Some(encoder) = self.active_compute_encoder.take() {
			encoder.endEncoding();
		}

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer,
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
		if let Some(encoder) = self.active_compute_encoder.as_ref() {
			encoder.memoryBarrierWithScope(mtl::MTLBarrierScope::Buffers | mtl::MTLBarrierScope::Textures);
		}

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
		self.debug_regions.borrow_mut().push(name.to_owned());
		self.command_buffer.pushDebugGroup(&NSString::from_str(name));

		if let Some(encoder) = self.active_compute_encoder.as_ref() {
			encoder.pushDebugGroup(&NSString::from_str(name));
		}

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			encoder.pushDebugGroup(&NSString::from_str(name));
		}

		self.refresh_active_encoder_labels();
	}

	fn end_region(&self) {
		if let Some(encoder) = self.active_compute_encoder.as_ref() {
			encoder.popDebugGroup();
		}

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			encoder.popDebugGroup();
		}

		self.command_buffer.popDebugGroup();
		self.debug_regions.borrow_mut().pop();
		self.refresh_active_encoder_labels();
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
		let depth_stencil_state = pipeline.depth_stencil_state.clone();
		let face_winding = pipeline.face_winding;
		let cull_mode = pipeline.cull_mode;

		self.active_pipeline_layout = Some(pipeline_layout);
		self.bound_pipeline_layout = None;
		self.resize_push_constants_for_layout(pipeline_layout);

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			encoder.setFrontFacingWinding(utils::winding(face_winding));
			encoder.setCullMode(utils::cull_mode(cull_mode));

			if let Some(depth_stencil_state) = depth_stencil_state.as_ref() {
				encoder.setDepthStencilState(Some(depth_stencil_state.as_ref()));
			}

			if let PipelineState::Raster(Some(render_pipeline_state)) = &pipeline_state {
				encoder.setRenderPipelineState(render_pipeline_state);
			}

			self.bound_vertex_layout = pipeline_vertex_layout;
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
				handle: PrivateHandles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer)),
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
			handle: PrivateHandles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer)),
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
			let descriptor_set_handles = DescriptorSetHandle(descriptor_set_handle.0)
				.root(&self.device.descriptor_sets)
				.get_all(&self.device.descriptor_sets);
			let descriptor_set_handle =
				descriptor_set_handles[(self.sequence_index as usize).rem_euclid(descriptor_set_handles.len())];
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
			let binding_index = ARGUMENT_BUFFER_BINDING_BASE + set_index;

			match &pipeline.pipeline {
				PipelineState::Raster(_) => {
					if let Some(encoder) = self.active_render_encoder.as_ref() {
						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::TASK))
						{
							unsafe {
								encoder.setObjectBuffer_offset_atIndex(
									Some(descriptor_set.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}

						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::MESH))
						{
							unsafe {
								encoder.setMeshBuffer_offset_atIndex(
									Some(descriptor_set.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}

						if descriptor_set_layout
							.bindings
							.iter()
							.any(|binding| binding.stages.intersects(crate::Stages::VERTEX))
						{
							unsafe {
								encoder.setVertexBuffer_offset_atIndex(
									Some(descriptor_set.argument_buffer.as_ref()),
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
									Some(descriptor_set.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}

						// Make resources referenced through argument buffers resident so the GPU can access them.
						let usage = mtl::MTLResourceUsage(mtl::MTLResourceUsage::Read.0 | mtl::MTLResourceUsage::Write.0);
						for descriptors_at_binding in descriptor_set.descriptors.values() {
							for descriptor in descriptors_at_binding.values() {
								match *descriptor {
									Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
										let tex: &ProtocolObject<dyn mtl::MTLTexture> =
											&self.device.images.resource(image).texture;
										encoder.useResource_usage(ProtocolObject::from_ref(tex), usage);
									}
									Descriptor::Buffer { buffer, .. } => {
										let buf: &ProtocolObject<dyn mtl::MTLBuffer> =
											&self.device.buffers.resource(buffer).buffer;
										encoder.useResource_usage(ProtocolObject::from_ref(buf), usage);
									}
									Descriptor::Swapchain { handle } => {
										if let Some(proxy_handle) =
											self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
										{
											let tex: &ProtocolObject<dyn mtl::MTLTexture> =
												&self.device.images.resource(proxy_handle).texture;
											encoder.useResource_usage(ProtocolObject::from_ref(tex), usage);
										}
									}
									Descriptor::Sampler { .. } => {}
								}
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
									Some(descriptor_set.argument_buffer.as_ref()),
									0,
									binding_index as _,
								);
							}
						}

						// Make resources referenced through argument buffers resident so the GPU can access them.
						let usage = mtl::MTLResourceUsage(mtl::MTLResourceUsage::Read.0 | mtl::MTLResourceUsage::Write.0);
						for descriptors_at_binding in descriptor_set.descriptors.values() {
							for descriptor in descriptors_at_binding.values() {
								match *descriptor {
									Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
										let tex: &ProtocolObject<dyn mtl::MTLTexture> =
											&self.device.images.resource(image).texture;
										encoder.useResource_usage(ProtocolObject::from_ref(tex), usage);
									}
									Descriptor::Buffer { buffer, .. } => {
										let buf: &ProtocolObject<dyn mtl::MTLBuffer> =
											&self.device.buffers.resource(buffer).buffer;
										encoder.useResource_usage(ProtocolObject::from_ref(buf), usage);
									}
									Descriptor::Swapchain { handle } => {
										if let Some(proxy_handle) =
											self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
										{
											let tex: &ProtocolObject<dyn mtl::MTLTexture> =
												&self.device.images.resource(proxy_handle).texture;
											encoder.useResource_usage(ProtocolObject::from_ref(tex), usage);
										}
									}
									Descriptor::Sampler { .. } => {}
								}
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
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		let mesh = &self.device.meshes[mesh_handle.0 as usize];
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that draw_mesh was called outside start_render_pass.");

		unsafe {
			for (binding, vertex_buffer) in mesh.vertex_buffers.iter().enumerate() {
				if let Some(vertex_buffer) = vertex_buffer.as_ref() {
					encoder.setVertexBuffer_offset_atIndex(Some(vertex_buffer.as_ref()), 0, binding as _);
				}
			}
			encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
				mtl::MTLPrimitiveType::Triangle,
				mesh.index_count as _,
				mtl::MTLIndexType::UInt16,
				mesh.index_buffer.as_ref(),
				0,
			);
		}
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
		let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
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

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		let bound_pipeline = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that dispatch_meshes was called before bind_raster_pipeline.");
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let mesh_threadgroup_size = pipeline.mesh_threadgroup_size.expect(
			"Metal mesh dispatch requires mesh threadgroup metadata. The most likely cause is that the mesh shader was not generated with Metal mesh threadgroup size metadata.",
		);
		let object_threadgroup_size = pipeline.object_threadgroup_size.unwrap_or(Extent::new(1, 1, 1));

		self.active_render_encoder
			.as_ref()
			.expect(
				"No active render pass. The most likely cause is that dispatch_meshes was called outside start_render_pass.",
			)
			.drawMeshThreadgroups_threadsPerObjectThreadgroup_threadsPerMeshThreadgroup(
				mtl::MTLSize {
					width: x as _,
					height: y as _,
					depth: z as _,
				},
				mtl::MTLSize {
					width: object_threadgroup_size.width() as _,
					height: object_threadgroup_size.height() as _,
					depth: object_threadgroup_size.depth() as _,
				},
				mtl::MTLSize {
					width: mesh_threadgroup_size.width() as _,
					height: mesh_threadgroup_size.height() as _,
					depth: mesh_threadgroup_size.depth() as _,
				},
			);
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
		buffer_handle: graphics_hardware_interface::BufferHandle<[[u32; 4]; N]>,
		entry_index: usize,
	) {
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle.into());
		let buffer = self.device.buffers.resource(internal_buffer).buffer.clone();

		self.consume_resources([Consumption {
			handle: PrivateHandles::Buffer(internal_buffer),
			stages: crate::Stages::COMPUTE,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Indirect,
		}]);

		let bound_pipeline = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that indirect_dispatch was called before bind_compute_pipeline.",
		);
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let threadgroup_extent = pipeline.compute_threadgroup_size.unwrap_or(Extent::line(128));

		unsafe {
			self.ensure_compute_encoder()
				.dispatchThreadgroupsWithIndirectBuffer_indirectBufferOffset_threadsPerThreadgroup(
					buffer.as_ref(),
					(entry_index * std::mem::size_of::<[u32; 4]>()) as _,
					mtl::MTLSize {
						width: threadgroup_extent.width() as _,
						height: threadgroup_extent.height() as _,
						depth: threadgroup_extent.depth() as _,
					},
				);
		}
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: Encode Metal ray tracing dispatch.
	}
}
