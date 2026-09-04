use std::{collections::HashMap, sync::Arc};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use utils::{Box, Extent, RGBA};

use super::{
	element::ElementHandle as _,
	layout::{FeatherMask, Geometry, engine},
	style::{Color, EdgeFeather, LayerKind},
};
use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	},
	ui::{
		components::curve::{CurvePoint, CurveSegment},
		font::TextSystem,
	},
};

// Group draw preparation and geometry generation by responsibility.
mod data;
mod geometry;

use data::*;
use geometry::*;

/// The `UiRenderPass` struct centralizes batched UI rectangle rendering and text overlay compositing for the main render target.
pub struct UiRenderPass {
	pipeline_manager: crate::rendering::PipelineManagerClient,
	pipeline: crate::rendering::PipelineRef,
	vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	curve_pipeline: crate::rendering::PipelineRef,
	curve_vertex_buffer: ghi::BufferHandle<[UiCurveVertex; MAX_UI_VERTICES]>,
	curve_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	image_pipeline: crate::rendering::PipelineRef,
	image_vertex_buffer: ghi::BufferHandle<[UiImageVertex; MAX_UI_VERTICES]>,
	image_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	image_sampler: ghi::SamplerHandle,
	image_textures: HashMap<u64, UiImageTexture>,
	text_pipeline: crate::rendering::PipelineRef,
	text_vertex_buffer: ghi::BufferHandle<[[f32; 2]; 3]>,
	text_sampler: ghi::SamplerHandle,
	text_overlays: Vec<UiTextOverlayTexture>,
	blur_downsample_pipeline: crate::rendering::PipelineRef,
	blur_filter_pipeline: crate::rendering::PipelineRef,
	blur_downsample_workgroup: Extent,
	blur_filter_workgroup: Extent,
	blur_composite_pipeline: crate::rendering::PipelineRef,
	blur_vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	blur_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	blur_sampler: ghi::SamplerHandle,
	blur_half_downsample_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_y_descriptor_set: ghi::DescriptorSetHandle,
	blur_half_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_half_y_descriptor_set: ghi::DescriptorSetHandle,
	blur_composite_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_scratch: ghi::BaseImageHandle,
	blur_full_output: ghi::BaseImageHandle,
	blur_half_source: ghi::BaseImageHandle,
	blur_half_scratch: ghi::BaseImageHandle,
	blur_half_output: ghi::BaseImageHandle,
	main_attachment: ghi::BaseImageHandle,
	output_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	data: UiDrawList,
	reported_capacity_limit: bool,
	text_system: TextSystem,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout primitives.
	// Keep the UI pipeline and fixed buffer setup together because every handle is required by frame preparation.
	#[allow(clippy::too_many_lines)]
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let source = render_pass_builder.read_from("main");
		// Backdrop blur samples partially rendered UI, so keep a sampleable working image even when the graph output is the swapchain.
		let main_attachment = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				MAIN_ATTACHMENT_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("UI Working"),
		);
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Storage | ghi::Uses::Image).name("UI"),
		);

		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/rectangle.pipeline");
		let curve_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/curve.pipeline");
		let image_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/image.pipeline");
		let text_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/text.pipeline");
		let blur_downsample_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/ui/backdrop-blur-downsample.pipeline");
		let blur_filter_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/backdrop-blur-filter.pipeline");
		let blur_composite_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/ui/backdrop-blur-composite.pipeline");
		let blur_downsample_workgroup = Extent::square(16);
		let blur_filter_workgroup = Extent::square(16);

		let context = render_pass_builder.context();

		let vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Vertices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("UI Indices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let curve_vertex_buffer: ghi::BufferHandle<[UiCurveVertex; MAX_UI_VERTICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Curve Vertices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let curve_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("UI Curve Indices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let image_vertex_buffer: ghi::BufferHandle<[UiImageVertex; MAX_UI_VERTICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Image Vertices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let image_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("UI Image Indices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let image_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let text_overlay = context.build_dynamic_image(
			ghi::image::Builder::new(TEXT_OVERLAY_FORMAT, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("UI Text Overlay")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let text_vertex_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Text Fullscreen Triangle")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let text_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let blur_vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Backdrop Blur Vertices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let blur_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("UI Backdrop Blur Indices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let blur_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let blur_full_scratch = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Full Scratch"),
		);
		let blur_full_scratch_image: ghi::BaseImageHandle = blur_full_scratch.into();
		let blur_full_output = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Full Output"),
		);
		let blur_full_output_image: ghi::BaseImageHandle = blur_full_output.into();
		let blur_half_source = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Source"),
		);
		let blur_half_source_image: ghi::BaseImageHandle = blur_half_source.into();
		let blur_half_scratch = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Scratch"),
		);
		let blur_half_scratch_image: ghi::BaseImageHandle = blur_half_scratch.into();
		let blur_half_output = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Output"),
		);
		let blur_half_output_image: ghi::BaseImageHandle = blur_half_output.into();
		let main_attachment_image: ghi::BaseImageHandle = main_attachment.into();
		let blur_half_downsample_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half Downsample"));
		let blur_full_x_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Full X"));
		let blur_full_y_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Full Y"));
		let blur_half_x_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half X"));
		let blur_half_y_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half Y"));
		let blur_composite_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Composite"));
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_downsample_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				main_attachment_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_downsample_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_source_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_full_x_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				main_attachment_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_full_x_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_full_scratch_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_full_y_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_full_scratch_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_full_y_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_full_output_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_x_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_half_source_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_x_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_scratch_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_y_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_half_scratch_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_y_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_output_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_composite_descriptor_set,
				UI_BLUR_FULL_COMPOSITE_BINDING.slot(),
				blur_full_output_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_composite_descriptor_set,
				UI_BLUR_HALF_COMPOSITE_BINDING.slot(),
				blur_half_output_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		]);
		let text_overlay_descriptor_set = context.create_descriptor_set(Some("UI Text"));
		context.write(&[ghi::DescriptorWrite::combined_image_sampler(
			text_overlay_descriptor_set,
			TEXT_OVERLAY_BINDING.slot(),
			text_overlay,
			text_sampler,
			ghi::Layouts::Read,
		)]);
		let text_overlays = vec![UiTextOverlayTexture {
			image: text_overlay.into(),
			descriptor_set: text_overlay_descriptor_set,
		}];
		let output_pass =
			crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, main_attachment_image, output);
		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			pipeline_manager,
			pipeline,
			vertex_buffer,
			index_buffer,
			curve_pipeline,
			curve_vertex_buffer,
			curve_index_buffer,
			image_pipeline,
			image_vertex_buffer,
			image_index_buffer,
			image_sampler,
			image_textures: HashMap::new(),
			text_pipeline,
			text_vertex_buffer,
			text_sampler,
			text_overlays,
			blur_downsample_pipeline,
			blur_filter_pipeline,
			blur_downsample_workgroup,
			blur_filter_workgroup,
			blur_composite_pipeline,
			blur_vertex_buffer,
			blur_index_buffer,
			blur_sampler,
			blur_half_downsample_descriptor_set,
			blur_full_x_descriptor_set,
			blur_full_y_descriptor_set,
			blur_half_x_descriptor_set,
			blur_half_y_descriptor_set,
			blur_composite_descriptor_set,
			blur_full_scratch: blur_full_scratch_image,
			blur_full_output: blur_full_output_image,
			blur_half_source: blur_half_source_image,
			blur_half_scratch: blur_half_scratch_image,
			blur_half_output: blur_half_output_image,
			main_attachment: main_attachment_image,
			output_pass,
			bypass_pass,
			data: UiDrawList::default(),
			reported_capacity_limit: false,
			text_system: TextSystem::new(),
		}
	}

	fn ensure_image_texture(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		image: &UiImageDrawElement,
	) -> Option<ghi::DescriptorSetHandle> {
		if !should_draw_image(image) {
			return None;
		}

		let needs_create = !self.image_textures.contains_key(&image.image_id);
		if needs_create {
			let texture = frame.build_image(
				ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
					.name("UI Image")
					.extent(Extent::rectangle(image.source_width, image.source_height))
					.device_accesses(ghi::DeviceAccesses::HostToDevice),
			);
			let texture: ghi::BaseImageHandle = texture.into();
			let descriptor_set = frame.create_descriptor_set(Some("UI Image"));
			frame.write(&[ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				UI_IMAGE_BINDING.slot(),
				texture,
				self.image_sampler,
				ghi::Layouts::Read,
			)]);
			self.image_textures.insert(
				image.image_id,
				UiImageTexture {
					version: u64::MAX,
					extent: (0, 0),
					image: texture,
					descriptor_set,
				},
			);
		}

		let texture = self.image_textures.get_mut(&image.image_id)?;
		if texture.version != image.version || texture.extent != (image.source_width, image.source_height) {
			frame.resize_image(texture.image, Extent::rectangle(image.source_width, image.source_height));
			let texture_slice = frame.get_texture_slice_mut(texture.image);
			texture_slice[..image.pixels.len()].copy_from_slice(&image.pixels);
			frame.sync_texture(texture.image);
			texture.version = image.version;
			texture.extent = (image.source_width, image.source_height);
		}

		Some(texture.descriptor_set)
	}

	fn ensure_text_overlay(&mut self, frame: &mut ghi::implementation::Frame, index: usize) -> ghi::DescriptorSetHandle {
		while self.text_overlays.len() <= index {
			let text_overlay = frame.build_image(
				ghi::image::Builder::new(TEXT_OVERLAY_FORMAT, ghi::Uses::Image | ghi::Uses::TransferDestination)
					.name("UI Text Overlay")
					.device_accesses(ghi::DeviceAccesses::HostToDevice),
			);
			let text_overlay: ghi::BaseImageHandle = text_overlay.into();
			let descriptor_set = frame.create_descriptor_set(Some("UI Text"));
			frame.write(&[ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				TEXT_OVERLAY_BINDING.slot(),
				text_overlay,
				self.text_sampler,
				ghi::Layouts::Read,
			)]);
			self.text_overlays.push(UiTextOverlayTexture {
				image: text_overlay,
				descriptor_set,
			});
		}

		self.text_overlays[index].descriptor_set
	}

	pub fn update(&mut self, render: engine::Render) {
		update_from_render(&render, &mut self.data);
	}
}

impl RenderPass for UiRenderPass {
	fn name(&self) -> &'static str {
		"ui"
	}

	// Keep ordered UI batch recording in one function so clears, blur barriers, and depth order cannot diverge.
	#[allow(clippy::excessive_nesting, clippy::too_many_lines)]
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pipeline = self.pipeline_manager.pipeline(self.pipeline)?;
		let curve_pipeline = self.pipeline_manager.pipeline(self.curve_pipeline)?;
		let image_pipeline = self.pipeline_manager.pipeline(self.image_pipeline)?;
		let text_pipeline = self.pipeline_manager.pipeline(self.text_pipeline)?;
		let blur_downsample_pipeline = self.pipeline_manager.pipeline(self.blur_downsample_pipeline)?;
		let blur_filter_pipeline = self.pipeline_manager.pipeline(self.blur_filter_pipeline)?;
		let blur_composite_pipeline = self.pipeline_manager.pipeline(self.blur_composite_pipeline)?;
		let extent = sink.extent();
		frame
			.get_mut_buffer_slice(self.text_vertex_buffer)
			.copy_from_slice(&[[-1.0, -1.0], [-1.0, 3.0], [3.0, -1.0]]);
		frame.sync_buffer(self.text_vertex_buffer);
		let geometry = build_ui_geometry(&self.data, extent, frame_allocator);
		let blur_geometry = build_ui_blur_geometry(&self.data, extent, frame_allocator);
		let curve_geometry = build_ui_curve_geometry(&self.data, extent, frame_allocator);
		let image_geometry = build_ui_image_geometry(&self.data, extent, frame_allocator);
		let has_rectangle_batches = !geometry.batches.is_empty();
		let has_blur_batches = !blur_geometry.batches.is_empty();
		let has_curve_batches = !curve_geometry.batches.is_empty();
		let has_image_batches = !image_geometry.batches.is_empty();

		if (geometry.truncated || blur_geometry.truncated || curve_geometry.truncated || image_geometry.truncated)
			&& !self.reported_capacity_limit
		{
			log::warn!(
				"UI geometry capacity exceeded. The most likely cause is that the UI contains more than {MAX_UI_ELEMENTS} drawable elements in a single frame."
			);
			self.reported_capacity_limit = true;
		} else if !geometry.truncated && !blur_geometry.truncated && !curve_geometry.truncated && !image_geometry.truncated {
			self.reported_capacity_limit = false;
		}

		if has_rectangle_batches {
			let vertex_buffer_slice = frame.get_mut_buffer_slice(self.vertex_buffer);
			vertex_buffer_slice[..geometry.vertices.len()].copy_from_slice(&geometry.vertices);
			frame.sync_buffer(self.vertex_buffer);

			let index_buffer_slice = frame.get_mut_buffer_slice(self.index_buffer);
			index_buffer_slice[..geometry.indices.len()].copy_from_slice(&geometry.indices);
			frame.sync_buffer(self.index_buffer);
		}

		if has_curve_batches {
			let vertex_buffer_slice = frame.get_mut_buffer_slice(self.curve_vertex_buffer);
			vertex_buffer_slice[..curve_geometry.vertices.len()].copy_from_slice(&curve_geometry.vertices);
			frame.sync_buffer(self.curve_vertex_buffer);

			let index_buffer_slice = frame.get_mut_buffer_slice(self.curve_index_buffer);
			index_buffer_slice[..curve_geometry.indices.len()].copy_from_slice(&curve_geometry.indices);
			frame.sync_buffer(self.curve_index_buffer);
		}

		if has_blur_batches {
			let vertex_buffer_slice = frame.get_mut_buffer_slice(self.blur_vertex_buffer);
			vertex_buffer_slice[..blur_geometry.vertices.len()].copy_from_slice(&blur_geometry.vertices);
			frame.sync_buffer(self.blur_vertex_buffer);

			let index_buffer_slice = frame.get_mut_buffer_slice(self.blur_index_buffer);
			index_buffer_slice[..blur_geometry.indices.len()].copy_from_slice(&blur_geometry.indices);
			frame.sync_buffer(self.blur_index_buffer);

			let half_extent = blur_half_extent(extent);
			frame.resize_image(self.blur_full_scratch, extent);
			frame.resize_image(self.blur_full_output, extent);
			frame.resize_image(self.blur_half_source, half_extent);
			frame.resize_image(self.blur_half_scratch, half_extent);
			frame.resize_image(self.blur_half_output, half_extent);
		}

		if has_image_batches {
			let vertex_buffer_slice = frame.get_mut_buffer_slice(self.image_vertex_buffer);
			vertex_buffer_slice[..image_geometry.vertices.len()].copy_from_slice(&image_geometry.vertices);
			frame.sync_buffer(self.image_vertex_buffer);

			let index_buffer_slice = frame.get_mut_buffer_slice(self.image_index_buffer);
			index_buffer_slice[..image_geometry.indices.len()].copy_from_slice(&image_geometry.indices);
			frame.sync_buffer(self.image_index_buffer);
		}

		let mut prepared_image_batches = Vec::new_in(frame_allocator);
		for batch in &image_geometry.batches {
			let Some(image) = self
				.data
				.images
				.iter()
				.find(|image| image.image_id == batch.image_id && image.version == batch.version)
				.cloned()
			else {
				continue;
			};
			let Some(descriptor_set) = self.ensure_image_texture(frame, &image) else {
				continue;
			};
			prepared_image_batches.push(UiPreparedImageBatch {
				descriptor_set,
				batch: *batch,
			});
		}

		let mut text_groups = Vec::new();
		if !self.data.texts.is_empty() {
			assert!(
				extent.width() > 0 && extent.height() > 0,
				"UI text overlay resize requires a non-zero viewport extent. The most likely cause is that text rendering ran before swapchain extent validation."
			);

			for text in self.data.texts.iter().cloned() {
				if let Some((_, order, texts)) = text_groups
					.iter_mut()
					.find(|(depth, ..): &&mut (u32, u32, std::vec::Vec<UiTextDrawElement>)| *depth == text.depth)
				{
					*order = (*order).min(text.order);
					texts.push(text);
				} else {
					text_groups.push((text.depth, text.order, vec![text]));
				}
			}
			text_groups.sort_by_key(|(depth, order, _)| (*depth, *order));
		}

		let mut prepared_text_batches = Vec::new_in(frame_allocator);
		for (index, (depth, order, texts)) in text_groups.iter().enumerate() {
			let descriptor_set = self.ensure_text_overlay(frame, index);
			let overlay = self.text_overlays[index].image;
			frame.resize_image(overlay, Extent::rectangle(extent.width(), extent.height()));
			let overlay_pixels = frame.get_texture_slice_mut(overlay);
			let drew_text = rasterize_text_overlay(texts, self.data.layout_size, extent, &mut self.text_system, overlay_pixels);
			if drew_text {
				frame.sync_texture(overlay);
				prepared_text_batches.push(UiPreparedTextBatch {
					depth: *depth,
					order: *order,
					descriptor_set,
				});
			}
		}

		let mut prepared_batches = Vec::with_capacity_in(
			geometry.batches.len()
				+ blur_geometry.batches.len()
				+ curve_geometry.batches.len()
				+ prepared_image_batches.len()
				+ prepared_text_batches.len(),
			frame_allocator,
		);
		prepared_batches.extend(geometry.batches.iter().copied().map(UiPreparedBatch::Rect));
		prepared_batches.extend(blur_geometry.batches.iter().copied().map(UiPreparedBatch::Blur));
		prepared_batches.extend(curve_geometry.batches.iter().copied().map(UiPreparedBatch::Curve));
		prepared_batches.extend(prepared_image_batches.iter().copied().map(UiPreparedBatch::Image));
		prepared_batches.extend(prepared_text_batches.iter().copied().map(UiPreparedBatch::Text));
		sort_prepared_batches(&mut prepared_batches);

		if prepared_batches.is_empty() {
			return None;
		}

		let vertex_buffer = self.vertex_buffer;
		let index_buffer = self.index_buffer;
		let curve_vertex_buffer = self.curve_vertex_buffer;
		let curve_index_buffer = self.curve_index_buffer;
		let image_vertex_buffer = self.image_vertex_buffer;
		let image_index_buffer = self.image_index_buffer;
		let text_vertex_buffer = self.text_vertex_buffer;
		let blur_downsample_workgroup = self.blur_downsample_workgroup;
		let blur_filter_workgroup = self.blur_filter_workgroup;
		let blur_vertex_buffer = self.blur_vertex_buffer;
		let blur_index_buffer = self.blur_index_buffer;
		let blur_half_downsample_descriptor_set = self.blur_half_downsample_descriptor_set;
		let blur_full_x_descriptor_set = self.blur_full_x_descriptor_set;
		let blur_full_y_descriptor_set = self.blur_full_y_descriptor_set;
		let blur_half_x_descriptor_set = self.blur_half_x_descriptor_set;
		let blur_half_y_descriptor_set = self.blur_half_y_descriptor_set;
		let blur_composite_descriptor_set = self.blur_composite_descriptor_set;
		let main_attachment = self.main_attachment;
		let output_command = self.output_pass.prepare(frame, sink, frame_allocator)?;
		let batches: &'a [UiPreparedBatch] = frame_allocator.alloc_slice_copy(&prepared_batches);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("UI"),
					|command_buffer| {
						let mut needs_clear = true;

						if !batches.is_empty() {
							for batch in batches {
								let clear_before_batch = needs_clear;
								let attachments = [ghi::AttachmentInformation::new(
									main_attachment,
									ghi::Layouts::RenderTarget,
									ghi::ClearValue::None,
									!clear_before_batch,
									true,
								)];
								needs_clear = false;

								match batch {
									UiPreparedBatch::Rect(batch) => {
										command_buffer.bind_vertex_buffers(&[vertex_buffer.into()]);
										command_buffer.bind_index_buffer(
											&(Into::<ghi::BufferDescriptor>::into(index_buffer)
												.index_type(ghi::DataTypes::U16)),
										);

										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(pipeline);
										command_buffer.draw_indexed(
											batch.index_count,
											1,
											batch.first_index,
											batch.vertex_offset,
											0,
										);
										command_buffer.end_render_pass();
									}
									UiPreparedBatch::Curve(batch) => {
										command_buffer.bind_vertex_buffers(&[curve_vertex_buffer.into()]);
										command_buffer.bind_index_buffer(
											&(Into::<ghi::BufferDescriptor>::into(curve_index_buffer)
												.index_type(ghi::DataTypes::U16)),
										);

										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(curve_pipeline);
										command_buffer.draw_indexed(
											batch.index_count,
											1,
											batch.first_index,
											batch.vertex_offset,
											0,
										);
										command_buffer.end_render_pass();
									}
									UiPreparedBatch::Image(prepared) => {
										command_buffer.bind_vertex_buffers(&[image_vertex_buffer.into()]);
										command_buffer.bind_index_buffer(
											&(Into::<ghi::BufferDescriptor>::into(image_index_buffer)
												.index_type(ghi::DataTypes::U16)),
										);

										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(image_pipeline);
										command_buffer.bind_descriptor_sets(&[prepared.descriptor_set]);
										command_buffer.draw_indexed(
											prepared.batch.index_count,
											1,
											prepared.batch.first_index,
											prepared.batch.vertex_offset,
											0,
										);
										command_buffer.end_render_pass();
									}
									UiPreparedBatch::Text(prepared) => {
										command_buffer.bind_vertex_buffers(&[text_vertex_buffer.into()]);
										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(text_pipeline);
										command_buffer.bind_descriptor_sets(&[prepared.descriptor_set]);
										command_buffer.draw(3, 1, 0, 0);
										command_buffer.end_render_pass();
									}
									UiPreparedBatch::Blur(batch) => {
										// A compute capture cannot perform the first attachment clear. Open an empty
										// render pass first so a blur-only frame never samples prior frame contents.
										if clear_before_batch {
											command_buffer.start_render_pass(extent, &attachments).end_render_pass();
										}
										let loaded_attachments = [ghi::AttachmentInformation::new(
											main_attachment,
											ghi::Layouts::RenderTarget,
											ghi::ClearValue::None,
											true,
											true,
										)];
										command_buffer.region(
											|label| label.write_str("UI Backdrop Blur"),
											|command_buffer| {
												if blur_uses_full_resolution(batch.resolution_mix) {
													let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
													compute.bind_descriptor_sets(&[blur_full_x_descriptor_set]);
													compute.write_push_constant(
														0,
														batch.full_kernel.push([1.0, 0.0], batch.full_regions.horizontal),
													);
													compute.dispatch(ghi::DispatchExtent::new(
														batch.full_regions.horizontal.extent,
														blur_filter_workgroup,
													));

													let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
													compute.bind_descriptor_sets(&[blur_full_y_descriptor_set]);
													compute.write_push_constant(
														0,
														batch.full_kernel.push([0.0, 1.0], batch.full_regions.vertical),
													);
													compute.dispatch(ghi::DispatchExtent::new(
														batch.full_regions.vertical.extent,
														blur_filter_workgroup,
													));
												}

												if blur_uses_half_resolution(batch.resolution_mix) {
													let compute =
														command_buffer.bind_compute_pipeline(blur_downsample_pipeline);
													compute.bind_descriptor_sets(&[blur_half_downsample_descriptor_set]);
													compute.write_push_constant(
														0,
														UiBlurDownsamplePush {
															origin: batch.half_regions.downsample.origin,
															extent: batch.half_regions.downsample.push_extent(),
														},
													);
													compute.dispatch(ghi::DispatchExtent::new(
														batch.half_regions.downsample.extent,
														blur_downsample_workgroup,
													));

													let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
													compute.bind_descriptor_sets(&[blur_half_x_descriptor_set]);
													compute.write_push_constant(
														0,
														batch
															.half_kernel
															.push([1.0, 0.0], batch.half_regions.filter.horizontal),
													);
													compute.dispatch(ghi::DispatchExtent::new(
														batch.half_regions.filter.horizontal.extent,
														blur_filter_workgroup,
													));

													let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
													compute.bind_descriptor_sets(&[blur_half_y_descriptor_set]);
													compute.write_push_constant(
														0,
														batch.half_kernel.push([0.0, 1.0], batch.half_regions.filter.vertical),
													);
													compute.dispatch(ghi::DispatchExtent::new(
														batch.half_regions.filter.vertical.extent,
														blur_filter_workgroup,
													));
												}

												command_buffer.bind_vertex_buffers(&[blur_vertex_buffer.into()]);
												command_buffer.bind_index_buffer(
													&(Into::<ghi::BufferDescriptor>::into(blur_index_buffer)
														.index_type(ghi::DataTypes::U16)),
												);

												let command_buffer =
													command_buffer.start_render_pass(extent, &loaded_attachments);
												let command_buffer =
													command_buffer.bind_raster_pipeline(blur_composite_pipeline);
												command_buffer.bind_descriptor_sets(&[blur_composite_descriptor_set]);
												command_buffer.draw_indexed(
													batch.index_count,
													1,
													batch.first_index,
													batch.vertex_offset,
													0,
												);
												command_buffer.end_render_pass();
											},
										);
									}
								}
							}
						}
					},
				);
				// Resolve the working image only after every raster and blur batch has contributed to it.
				output_command(command_buffer, &[]);
			},
		))
	}

	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
}

#[cfg(test)]
mod tests {
	use std::mem::{align_of, offset_of, size_of};

	use besl::vm::{
		Buffer, DescriptorBindings, ExecutableProgram, Texture, Value, builtin_position_slot, input_slot, output_slot,
	};
	use utils::{Extent, RGBA};

	use super::{
		DrawClip, DrawFeatherMask, MAX_UI_ELEMENTS, MAX_UI_VERTICES_PER_DRAW, UI_BLUR_GAUSSIAN_PAIR_COUNT,
		UI_BLUR_GAUSSIAN_SUPPORT, UI_BLUR_HALF_DOWNSCALE, UI_INDICES_PER_CURVE_SPAN, UI_INDICES_PER_ELEMENT,
		UI_VERTICES_PER_CURVE_SPAN, UI_VERTICES_PER_ELEMENT, UiBlurDispatchRegion, UiBlurDrawElement, UiBlurFilterPush,
		UiBlurKernel, UiCurveDrawElement, UiDrawBatch, UiDrawElement, UiDrawList, UiImageDrawElement, UiTextDrawElement,
		blur_composite_region, blur_full_dispatch_regions, blur_half_dispatch_regions, blur_half_extent, blur_half_sigma,
		blur_resolution_mix, blur_sigma, blur_uses_full_resolution, blur_uses_half_resolution, build_ui_blur_geometry,
		build_ui_curve_geometry, build_ui_geometry, build_ui_image_geometry, flatten_curve_segment, should_draw_image,
		should_rasterize_text, update_from_render,
	};
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, compile as compile_shader_vm, empty_image, rgba, run_at, texture_2d},
	};
	use crate::ui::{
		Container, Text,
		components::{
			curve::{CurvePoint, CurveSegment},
			image::Image,
		},
		flow::Size,
		layout::{
			context::{Context, ElementContext},
			engine::Engine,
		},
		style::{ConcreteLayer, ConcreteStyle, LayerKind},
	};

	const UI_BLUR_DOWNSAMPLE_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-downsample.besl");
	const UI_BLUR_FILTER_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-filter.besl");
	const UI_BLUR_COMPOSITE_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-composite.besl");
	const UI_RECT_VERTEX_BESL: &str = include_str!("../../assets/rendering/ui/rect-vertex.besl");
	const UI_RECT_FRAGMENT_BESL: &str = include_str!("../../assets/rendering/ui/rect-fragment.besl");
	const UI_CURVE_VERTEX_BESL: &str = include_str!("../../assets/rendering/ui/curve-vertex.besl");
	const UI_CURVE_FRAGMENT_BESL: &str = include_str!("../../assets/rendering/ui/curve-fragment.besl");
	const UI_IMAGE_VERTEX_BESL: &str = include_str!("../../assets/rendering/ui/image-vertex.besl");
	const UI_IMAGE_FRAGMENT_BESL: &str = include_str!("../../assets/rendering/ui/image-fragment.besl");
	const UI_TEXT_VERTEX_BESL: &str = include_str!("../../assets/rendering/ui/text-vertex.besl");
	const UI_TEXT_FRAGMENT_BESL: &str = include_str!("../../assets/rendering/ui/text-fragment.besl");

	fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
		assert!((actual[0] - expected[0]).abs() < 0.0001);
		assert!((actual[1] - expected[1]).abs() < 0.0001);
	}

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
		}
	}

	// Compiles one checked-in UI blur shader through the same shared scope used
	// by production standalone compute shaders.
	fn compile_ui_blur_shader(source: &str) -> ExecutableProgram {
		compile_shader_vm(simple_compute::compile_test_program(source))
	}

	// Links one checked-in raster shader through the production BESL frontend.
	fn ui_raster_program(source: &str, shader_name: &str) -> besl::NodeReference {
		let program = besl::compile_to_besl(source, None).unwrap_or_else(|error| {
			panic!(
				"Failed to link {shader_name}: {error:?}. The most likely cause is invalid syntax in the checked-in BESL asset."
			)
		});
		program.get_main().unwrap_or_else(|| {
			panic!(
				"Missing {shader_name} entry point. The most likely cause is that the checked-in BESL asset has no `main` function."
			)
		})
	}

	// Initializes the shared origin/extent contract used by regional compute stages.
	fn blur_region_push_constant(executable: &ExecutableProgram, origin: [u32; 2], extent: [u32; 2]) -> Buffer {
		let mut push_constant = Buffer::new(
			executable
				.push_constant_layout()
				.expect("Missing blur region push constants. The most likely cause is a changed production shader interface.")
				.clone(),
		);
		push_constant
			.write("origin", Value::Vec2U(origin))
			.expect("Failed to initialize the blur region origin. The most likely cause is a changed push constant type.");
		push_constant
			.write("extent", Value::Vec2U(extent))
			.expect("Failed to initialize the blur region extent. The most likely cause is a changed push constant type.");
		push_constant
	}

	// Mirrors the aligned host record through named VM fields so production
	// shader tests validate both the coefficients and the reflected interface.
	fn blur_filter_push_constant(executable: &ExecutableProgram, push: UiBlurFilterPush) -> Buffer {
		let mut push_constant = Buffer::new(
			executable
				.push_constant_layout()
				.expect("Missing blur filter push constants. The most likely cause is a changed production shader interface.")
				.clone(),
		);
		for (name, value) in [
			("filter_data", Value::Vec4F(push.filter_data)),
			("origin", Value::Vec2U(push.origin)),
			("extent", Value::Vec2U(push.extent)),
			("pair_weights_0_3", Value::Vec4F(push.pair_weights_0_3)),
			("pair_weights_4_7", Value::Vec4F(push.pair_weights_4_7)),
			("pair_weights_8_10", Value::Vec4F(push.pair_weights_8_10_pad)),
			("pair_offsets_0_3", Value::Vec4F(push.pair_offsets_0_3)),
			("pair_offsets_4_7", Value::Vec4F(push.pair_offsets_4_7)),
			("pair_offsets_8_10", Value::Vec4F(push.pair_offsets_8_10_pad)),
		] {
			push_constant.write(name, value).unwrap_or_else(|error| {
				panic!(
					"Failed to initialize blur filter field `{name}`: {error}. The most likely cause is a changed push constant type."
				)
			});
		}
		push_constant
	}

	// Reconstructs the integer Gaussian taps represented by the bilinear pairs
	// so tests can compare the actual discrete variance with the requested one.
	fn blur_kernel_variance(kernel: UiBlurKernel) -> f32 {
		let mut second_moment = 0.0;
		for pair_index in 0..UI_BLUR_GAUSSIAN_PAIR_COUNT {
			let first = (pair_index * 2 + 1) as f32;
			let weight = kernel.pair_weights[pair_index];
			let offset = kernel.pair_offsets[pair_index];
			let first_weight = weight * (first + 1.0 - offset);
			let second_weight = weight * (offset - first);
			second_moment += 2.0 * (first_weight * first * first + second_weight * (first + 1.0) * (first + 1.0));
		}
		second_moment
	}

	// Executes the production composite shader with a full-coverage rectangle.
	fn run_blur_composite_vm(
		full_texels: &[[f32; 4]],
		full_extent: [u32; 2],
		half_texels: &[[f32; 4]],
		half_extent: [u32; 2],
		pixel_position: [f32; 2],
		resolution_mix: f32,
		feather_edges: [f32; 4],
	) -> [f32; 4] {
		let executable = compile_ui_blur_shader(UI_BLUR_COMPOSITE_BESL);
		let mut full_blurred = texture_2d(full_extent[0], full_extent[1], full_texels);
		let mut half_blurred = texture_2d(half_extent[0], half_extent[1], half_texels);
		run_blur_composite_textures_vm(
			&executable,
			&mut full_blurred,
			&mut half_blurred,
			pixel_position,
			resolution_mix,
			feather_edges,
		)
	}

	// Executes one fragment against reusable textures and a precompiled shader,
	// which keeps production-chain parameter sweeps fast enough for unit tests.
	fn run_blur_composite_textures_vm(
		executable: &ExecutableProgram,
		full_blurred: &mut Texture,
		half_blurred: &mut Texture,
		pixel_position: [f32; 2],
		resolution_mix: f32,
		feather_edges: [f32; 4],
	) -> [f32; 4] {
		let mut inputs = [
			(10, "_besl_interface_pixel_position", Value::Vec2F(pixel_position)),
			(9, "_besl_interface_local_position", Value::Vec2F([1.0, 1.0])),
			(11, "_besl_interface_rect_size", Value::Vec2F([2.0, 2.0])),
			(3, "_besl_interface_corner_radius", Value::F32(0.0)),
			(2, "_besl_interface_corner_exponent", Value::F32(2.0)),
			(6, "_besl_interface_feather_mask_position", Value::Vec2F([0.0, 0.0])),
			(7, "_besl_interface_feather_mask_size", Value::Vec2F([8.0, 4.0])),
			(5, "_besl_interface_feather_mask_edges", Value::Vec4F(feather_edges)),
			(0, "_besl_interface_blur_resolution_mix", Value::F32(resolution_mix)),
		]
		.map(|(location, name, value)| {
			let mut input = Buffer::new(
				executable
					.input_layout(location)
					.expect("Missing blur composite input. The most likely cause is a changed production shader interface.")
					.clone(),
			);
			input
				.write(name, value)
				.expect("Failed to initialize blur composite input. The most likely cause is a changed input type.");
			(location, input)
		});
		let mut output = Buffer::new(
			executable
				.output_layout(0)
				.expect("Missing blur composite output. The most likely cause is a changed production shader interface.")
				.clone(),
		);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), full_blurred);
			descriptors.bind_texture(besl::vm::ResourceSlot::new(1), half_blurred);
			for (location, input) in &mut inputs {
				descriptors.bind_buffer(input_slot(*location), input);
			}
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute the blur composite shader. The most likely cause is incomplete BESL VM support.");
		}

		match output
			.read("_besl_output_color_attachment")
			.expect("Failed to read blur composite output. The most likely cause is a changed output interface.")
		{
			Value::Vec4F(color) => color,
			value => {
				panic!("Invalid blur composite output `{value:?}`. The most likely cause is a changed production shader type.")
			}
		}
	}

	// Executes one regional production downsample dispatch into a caller-owned
	// image so tests can seed untouched texels with stale sentinels.
	fn run_blur_downsample_region_vm(
		executable: &ExecutableProgram,
		source: &mut Texture,
		result: &mut Texture,
		region: UiBlurDispatchRegion,
	) {
		let mut push_constant = blur_region_push_constant(executable, region.origin, region.push_extent());
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), result);
		descriptors.bind_push_constant(&mut push_constant);
		for y in 0..region.extent.height() {
			for x in 0..region.extent.width() {
				run_at(executable, &mut descriptors, [x, y]);
			}
		}
	}

	// Executes one regional production Gaussian dispatch using the same packed
	// coefficients and local-thread convention as command recording.
	fn run_blur_filter_region_vm(
		executable: &ExecutableProgram,
		source: &mut Texture,
		result: &mut Texture,
		kernel: UiBlurKernel,
		direction: [f32; 2],
		region: UiBlurDispatchRegion,
	) {
		let mut push_constant = blur_filter_push_constant(executable, kernel.push(direction, region));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), result);
		descriptors.bind_push_constant(&mut push_constant);
		for y in 0..region.extent.height() {
			for x in 0..region.extent.width() {
				run_at(executable, &mut descriptors, [x, y]);
			}
		}
	}

	fn full_blur_region(extent: Extent) -> UiBlurDispatchRegion {
		UiBlurDispatchRegion { origin: [0, 0], extent }
	}

	// Runs every production BESL stage selected for one radius and returns the
	// composited center scanline. Radius zero follows production's skipped path.
	fn run_adaptive_blur_scanline_vm(
		downsample: &ExecutableProgram,
		filter: &ExecutableProgram,
		composite: &ExecutableProgram,
		texels: &[[f32; 4]],
		extent: Extent,
		radius: f32,
		display_scale: f32,
	) -> Vec<[f32; 4]> {
		let width = extent.width();
		let height = extent.height();
		if radius <= 0.0 {
			let row = height / 2;
			return texels[(row * width) as usize..((row + 1) * width) as usize].to_vec();
		}

		let sigma = blur_sigma((radius * display_scale).clamp(0.0, 64.0));
		let resolution_mix = blur_resolution_mix(sigma);
		let full_region = full_blur_region(extent);
		let half_extent = blur_half_extent(extent);
		let half_region = full_blur_region(half_extent);
		let mut source = texture_2d(width, height, texels);
		let mut full_output = empty_image(width, height);
		if blur_uses_full_resolution(resolution_mix) {
			let mut horizontal = empty_image(width, height);
			run_blur_filter_region_vm(
				filter,
				&mut source,
				&mut horizontal,
				UiBlurKernel::gaussian(sigma),
				[1.0, 0.0],
				full_region,
			);
			run_blur_filter_region_vm(
				filter,
				&mut horizontal,
				&mut full_output,
				UiBlurKernel::gaussian(sigma),
				[0.0, 1.0],
				full_region,
			);
		}

		let mut half_output = empty_image(half_extent.width(), half_extent.height());
		if blur_uses_half_resolution(resolution_mix) {
			let mut half_source = empty_image(half_extent.width(), half_extent.height());
			run_blur_downsample_region_vm(downsample, &mut source, &mut half_source, half_region);
			let mut horizontal = empty_image(half_extent.width(), half_extent.height());
			let half_kernel = UiBlurKernel::gaussian(blur_half_sigma(sigma));
			run_blur_filter_region_vm(
				filter,
				&mut half_source,
				&mut horizontal,
				half_kernel,
				[1.0, 0.0],
				half_region,
			);
			run_blur_filter_region_vm(
				filter,
				&mut horizontal,
				&mut half_output,
				half_kernel,
				[0.0, 1.0],
				half_region,
			);
		}

		let row = height / 2;
		(0..width)
			.map(|x| {
				run_blur_composite_textures_vm(
					composite,
					&mut full_output,
					&mut half_output,
					[x as f32 + 0.5, row as f32 + 0.5],
					resolution_mix,
					[0.0; 4],
				)
			})
			.collect()
	}

	#[derive(Clone, Copy)]
	enum BlurChainPattern {
		Impulse,
		ThinLine,
		Checkerboard,
		Constant,
	}

	// Builds bounded semantic inputs that expose ringing, energy drift, and
	// failure to preserve constant colors without requiring a full-size frame.
	fn blur_chain_fixture(pattern: BlurChainPattern, extent: Extent) -> Vec<[f32; 4]> {
		let mut texels = vec![[0.0, 0.0, 0.0, 1.0]; (extent.width() * extent.height()) as usize];
		for y in 0..extent.height() {
			for x in 0..extent.width() {
				let color = match pattern {
					BlurChainPattern::Impulse if x == extent.width() / 2 && y == extent.height() / 2 => [1.0; 4],
					BlurChainPattern::ThinLine if x == extent.width() / 2 => [1.0; 4],
					BlurChainPattern::Checkerboard if (x + y) % 2 == 0 => [1.0; 4],
					BlurChainPattern::Constant => [0.25, 0.5, 0.75, 1.0],
					_ => [0.0, 0.0, 0.0, 1.0],
				};
				texels[(y * extent.width() + x) as usize] = color;
			}
		}
		texels
	}

	/// Verifies the half-resolution prefilter uses the positive binomial marginal and guards extra lanes.
	#[test]
	fn backdrop_blur_downsample_besl_vm_uses_binomial_prefilter() {
		let executable = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let mut texels = [[0.0; 4]; 6];
		texels[1] = [1.0, 0.0, 0.0, 0.0];
		texels[2] = [0.0, 1.0, 0.0, 0.0];
		texels[3] = [0.0, 0.0, 1.0, 0.0];
		texels[4] = [0.0, 0.0, 0.0, 1.0];
		let mut source = texture_2d(6, 1, &texels);
		let mut result = empty_image(3, 1);
		let mut push_constant = blur_region_push_constant(&executable, [1, 0], [1, 1]);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
		descriptors.bind_push_constant(&mut push_constant);
		run_at(&executable, &mut descriptors, [0, 0]);
		run_at(&executable, &mut descriptors, [1, 0]);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [1, 0]), [0.125, 0.375, 0.375, 0.125], 1e-6);
		assert_rgba_close(rgba(&result, [0, 0]), [0.0; 4], 1e-6);
		assert_rgba_close(rgba(&result, [2, 0]), [0.0; 4], 1e-6);
	}

	#[test]
	fn backdrop_blur_filter_push_layout_matches_the_production_shader() {
		assert_eq!(size_of::<UiBlurFilterPush>(), 128);
		assert_eq!(align_of::<UiBlurFilterPush>(), 16);
		assert_eq!(offset_of!(UiBlurFilterPush, filter_data), 0);
		assert_eq!(offset_of!(UiBlurFilterPush, origin), 16);
		assert_eq!(offset_of!(UiBlurFilterPush, extent), 24);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_0_3), 32);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_4_7), 48);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_8_10_pad), 64);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_0_3), 80);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_4_7), 96);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_8_10_pad), 112);

		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let layout = executable
			.push_constant_layout()
			.expect("Missing production blur push constants. The most likely cause is a changed filter interface.");

		assert_eq!(layout.size(), 128);
		for (name, expected_offset) in [
			("filter_data", 0),
			("origin", 16),
			("extent", 24),
			("pair_weights_0_3", 32),
			("pair_weights_4_7", 48),
			("pair_weights_8_10", 64),
			("pair_offsets_0_3", 80),
			("pair_offsets_4_7", 96),
			("pair_offsets_8_10", 112),
		] {
			let actual = layout
				.members()
				.iter()
				.find(|member| member.name() == name)
				.unwrap_or_else(|| panic!("Missing reflected blur field `{name}`"))
				.offset();

			assert_eq!(actual, expected_offset, "Unexpected reflected offset for `{name}`");
		}
	}

	#[test]
	fn backdrop_blur_gaussian_coefficients_are_normalized_and_preserve_variance() {
		let smallest_test_sigma = blur_sigma(0.25);
		let largest_half_sigma = blur_half_sigma(blur_sigma(64.0));
		for sigma in [0.0, smallest_test_sigma, 4.0, 5.0, 6.0, largest_half_sigma] {
			let kernel = UiBlurKernel::gaussian(sigma);
			let energy = kernel.center_weight + 2.0 * kernel.pair_weights.iter().sum::<f32>();

			assert!(
				(energy - 1.0).abs() <= 2e-6,
				"Gaussian energy drifted to {energy} at sigma {sigma}"
			);
			assert!(kernel.center_weight.is_finite() && kernel.center_weight >= 0.0);

			let mut second_moment = 0.0f32;
			for pair_index in 0..UI_BLUR_GAUSSIAN_PAIR_COUNT {
				let first = (pair_index * 2 + 1) as f32;
				let weight = kernel.pair_weights[pair_index];
				let offset = kernel.pair_offsets[pair_index];

				assert!(weight.is_finite() && weight >= 0.0);
				assert!(offset.is_finite() && (first..=first + 1.0).contains(&offset));
				let first_weight = weight * (first + 1.0 - offset);
				let second_weight = weight * (offset - first);
				second_moment += 2.0 * (first_weight * first * first + second_weight * (first + 1.0) * (first + 1.0));
			}
			if sigma >= smallest_test_sigma {
				let relative_error = (second_moment - sigma * sigma).abs() / (sigma * sigma);

				assert!(
					relative_error < 0.02,
					"Gaussian variance error {relative_error} at sigma {sigma}"
				);
			} else {
				assert_eq!(second_moment, 0.0);
			}
		}
	}

	#[test]
	fn backdrop_blur_variance_mapping_preserves_strength_at_one_and_two_x_scale() {
		for display_scale in [1.0f32, 2.0] {
			for radius in [0.25, 1.0, 4.0, 18.0, 32.0, 64.0] {
				let sigma = blur_sigma((radius * display_scale).clamp(0.0, 64.0));
				let resolution_mix = blur_resolution_mix(sigma);
				if blur_uses_full_resolution(resolution_mix) {
					let observed = blur_kernel_variance(UiBlurKernel::gaussian(sigma));
					let relative_error = (observed - sigma * sigma).abs() / (sigma * sigma);

					assert!(
						relative_error < 0.02,
						"Full-resolution variance error {relative_error} at radius {radius} and scale {display_scale}"
					);
				}
				if blur_uses_half_resolution(resolution_mix) {
					let half_variance = blur_kernel_variance(UiBlurKernel::gaussian(blur_half_sigma(sigma)));
					let observed = 4.0 * half_variance + 2.75;
					let relative_error = (observed - sigma * sigma).abs() / (sigma * sigma);

					assert!(
						relative_error < 0.05,
						"Half-resolution variance error {relative_error} at radius {radius} and scale {display_scale}"
					);
				}
			}
		}
	}

	/// Verifies the production Gaussian preserves constants, selects one axis, and guards extra lanes.
	#[test]
	fn backdrop_blur_filter_besl_vm_preserves_constants_and_direction() {
		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let region = UiBlurDispatchRegion {
			origin: [1, 1],
			extent: Extent::rectangle(1, 1),
		};
		let mut push_constant = blur_filter_push_constant(&executable, UiBlurKernel::gaussian(5.0).push([1.0, 0.0], region));
		let constant = [0.25, 0.5, 0.75, 1.0];
		let mut source = texture_2d(5, 5, &[constant; 25]);
		let mut result = empty_image(5, 5);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&executable, &mut descriptors, [0, 0]);
			run_at(&executable, &mut descriptors, [1, 0]);
		}
		assert_rgba_close(rgba(&result, [1, 1]), constant, 1e-5);
		assert_rgba_close(rgba(&result, [2, 1]), [0.0; 4], 1e-5);

		let width = 65;
		let center = width / 2;
		let mut impulse = vec![[0.0; 4]; width as usize * 3];
		impulse[(width + center) as usize] = [1.0; 4];
		let mut source = texture_2d(width, 3, &impulse);
		let mut result = empty_image(width, 3);
		let region = UiBlurDispatchRegion {
			origin: [0, 0],
			extent: Extent::rectangle(width, 3),
		};
		let mut push_constant = blur_filter_push_constant(&executable, UiBlurKernel::gaussian(5.0).push([1.0, 0.0], region));
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&executable, &mut descriptors, [center - 1, 1]);
			run_at(&executable, &mut descriptors, [center, 0]);
		}

		assert!(rgba(&result, [center - 1, 1])[0] > 0.0);
		assert_eq!(rgba(&result, [center, 0])[0], 0.0);
	}

	/// Verifies the effective-radius-36 production profile has one Gaussian peak without secondary bands.
	#[test]
	fn backdrop_blur_filter_besl_vm_has_no_secondary_lobe_at_effective_radius_36() {
		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let width = 65;
		let center = width / 2;
		let sigma = blur_half_sigma(blur_sigma(36.0));
		let kernel = UiBlurKernel::gaussian(sigma);
		let region = UiBlurDispatchRegion {
			origin: [0, 0],
			extent: Extent::rectangle(width, 1),
		};
		let mut push_constant = blur_filter_push_constant(&executable, kernel.push([1.0, 0.0], region));
		let mut impulse = vec![[0.0; 4]; width as usize];
		impulse[center as usize] = [1.0; 4];
		let mut source = texture_2d(width, 1, &impulse);
		let mut result = empty_image(width, 1);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			for x in 0..width {
				run_at(&executable, &mut descriptors, [x, 0]);
			}
		}

		let profile = (0..width).map(|x| rgba(&result, [x, 0])[0]).collect::<Vec<_>>();
		let normalization = 1.0
			+ 2.0
				* (1..=UI_BLUR_GAUSSIAN_SUPPORT)
					.map(|distance| (-0.5 * (distance as f32 / sigma).powi(2)).exp())
					.sum::<f32>();
		for distance in 0..=UI_BLUR_GAUSSIAN_SUPPORT {
			let positive = profile[(center + distance) as usize];
			let negative = profile[(center - distance) as usize];
			let expected = (-0.5 * (distance as f32 / sigma).powi(2)).exp() / normalization;

			assert!(
				(positive - negative).abs() < 2e-6,
				"Asymmetric Gaussian at distance {distance}"
			);
			assert!(
				(positive - expected).abs() < 2e-5,
				"Unexpected Gaussian tap at distance {distance}"
			);
			if distance > 0 {
				assert!(profile[(center + distance - 1) as usize] >= positive);
			}
		}
		let energy = profile.iter().sum::<f32>();

		assert!((energy - 1.0).abs() < 2e-5, "Production Gaussian energy drifted to {energy}");
	}

	/// Verifies full-resolution composite sampling uses the texture's exact pixel lattice.
	#[test]
	fn backdrop_blur_composite_besl_vm_samples_full_resolution_lattice() {
		let output = run_blur_composite_vm(
			&[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
			[2, 1],
			&[[0.0; 4]],
			[1, 1],
			[1.5, 0.5],
			0.0,
			[0.0; 4],
		);
		assert_rgba_close(output, [0.0, 1.0, 0.0, 1.0], 1e-6);
	}

	/// Verifies skipped resolution paths cannot contaminate a composite through stale values.
	#[test]
	fn backdrop_blur_composite_besl_vm_does_not_sample_inactive_resolution() {
		let nan = [f32::NAN; 4];
		let full = [0.2, 0.4, 0.6, 1.0];
		let half = [0.8, 0.6, 0.4, 1.0];
		let full_only = run_blur_composite_vm(&[full], [1, 1], &[nan], [1, 1], [0.5, 0.5], 0.0, [0.0; 4]);
		let half_only = run_blur_composite_vm(&[nan], [1, 1], &[half], [1, 1], [0.5, 0.5], 1.0, [0.0; 4]);
		assert_rgba_close(full_only, full, 1e-6);
		assert_rgba_close(half_only, half, 1e-6);
	}

	#[test]
	fn backdrop_blur_composite_besl_vm_blends_paths_and_preserves_feather_coverage() {
		let blended = run_blur_composite_vm(
			&[[1.0, 0.0, 0.0, 1.0]],
			[1, 1],
			&[[0.0, 0.0, 1.0, 1.0]],
			[1, 1],
			[0.5, 0.5],
			0.5,
			[0.0; 4],
		);
		assert_rgba_close(blended, [0.5, 0.0, 0.5, 1.0], 1e-6);

		let feathered = run_blur_composite_vm(
			&[[0.25, 0.5, 0.75, 1.0]],
			[1, 1],
			&[[0.0; 4]],
			[1, 1],
			[2.0, 2.0],
			0.0,
			[4.0, 0.0, 0.0, 0.0],
		);
		assert_rgba_close(feathered, [0.25, 0.5, 0.75, 0.5], 1e-6);
	}

	#[test]
	fn backdrop_blur_composite_besl_vm_keeps_awkward_widths_on_the_fixed_half_lattice() {
		for full_width in [2_801u32, 2_802, 2_803] {
			let half_width = full_width.div_ceil(UI_BLUR_HALF_DOWNSCALE);
			let full = vec![[0.0; 4]; full_width as usize];
			let half = (0..half_width)
				.map(|index| [index as f32 / (half_width - 1) as f32, 0.0, 0.0, 1.0])
				.collect::<Vec<_>>();
			let pixel_position = [full_width as f32 * 0.5, 0.5];
			let expected_coordinate = pixel_position[0] * 0.5 - 0.5;
			let output = run_blur_composite_vm(&full, [full_width, 1], &half, [half_width, 1], pixel_position, 1.0, [0.0; 4]);
			let expected = expected_coordinate / (half_width - 1) as f32;

			assert!(
				(output[0] - expected).abs() < 2e-5,
				"Half-lattice phase drift at width {full_width}"
			);
		}
	}

	#[test]
	fn backdrop_blur_resolution_crossover_selects_two_three_or_five_dispatches() {
		let dispatch_count = |sigma| {
			let resolution_mix = blur_resolution_mix(sigma);
			usize::from(blur_uses_full_resolution(resolution_mix)) * 2
				+ usize::from(blur_uses_half_resolution(resolution_mix)) * 3
		};

		assert_eq!(blur_resolution_mix(4.0), 0.0);
		assert_eq!(blur_resolution_mix(5.0), 0.5);
		assert_eq!(blur_resolution_mix(6.0), 1.0);
		assert_eq!(dispatch_count(4.0), 2);
		assert_eq!(dispatch_count(5.0), 5);
		assert_eq!(dispatch_count(6.0), 3);
		assert!(blur_resolution_mix(4.001) < 0.000_001);
		assert!(1.0 - blur_resolution_mix(5.999) < 0.000_001);

		let mut previous = 0.0;
		for step in 0..=512 {
			let resolution_mix = blur_resolution_mix(blur_sigma(step as f32 * 0.125));

			assert!(
				resolution_mix >= previous,
				"Resolution crossover stepped backward at sweep index {step}"
			);
			previous = resolution_mix;
		}
	}

	#[test]
	fn backdrop_blur_half_extent_keeps_every_awkward_edge_texel() {
		assert_eq!(blur_half_extent(Extent::rectangle(1920, 1080)), Extent::rectangle(960, 540));
		assert_eq!(blur_half_extent(Extent::rectangle(1919, 1079)), Extent::rectangle(960, 540));
		assert_eq!(blur_half_extent(Extent::rectangle(2802, 1)), Extent::rectangle(1401, 1));
		assert_eq!(blur_half_extent(Extent::rectangle(1, 1)), Extent::rectangle(1, 1));
	}

	#[test]
	fn backdrop_blur_dispatch_regions_pad_each_adaptive_path() {
		let viewport = Extent::rectangle(1920, 1080);
		let bounds = [400.0, 300.0, 800.0, 600.0];
		let full = blur_full_dispatch_regions(bounds, viewport);

		assert_eq!(
			full.vertical,
			UiBlurDispatchRegion {
				origin: [399, 299],
				extent: Extent::rectangle(402, 302),
			}
		);
		assert_eq!(
			full.horizontal,
			UiBlurDispatchRegion {
				origin: [398, 277],
				extent: Extent::rectangle(404, 346),
			}
		);

		let half = blur_half_dispatch_regions(bounds, viewport);

		assert_eq!(
			half.filter.vertical,
			UiBlurDispatchRegion {
				origin: [198, 148],
				extent: Extent::rectangle(204, 154),
			}
		);
		assert_eq!(
			half.filter.horizontal,
			UiBlurDispatchRegion {
				origin: [197, 126],
				extent: Extent::rectangle(206, 198),
			}
		);
		assert_eq!(
			half.downsample,
			UiBlurDispatchRegion {
				origin: [175, 125],
				extent: Extent::rectangle(250, 200),
			}
		);
	}

	#[test]
	// The footprint assertion intentionally visits every downsampled texel and its full tent support.
	#[allow(clippy::excessive_nesting)]
	fn backdrop_blur_half_region_contains_every_tent_sample_on_fixed_lattice() {
		let tent_offsets = [
			[-1.0, 0.0],
			[-0.5, 0.5],
			[0.0, 1.0],
			[0.5, 0.5],
			[1.0, 0.0],
			[0.5, -0.5],
			[0.0, -1.0],
			[-0.5, -0.5],
		];
		for width in [19, 2_801, 2_802, 2_803] {
			let viewport = Extent::rectangle(width, 13);
			let target = blur_half_extent(viewport);
			let bounds = [2.25, 1.75, width as f32 - 1.6, 11.2];
			let region = blur_half_dispatch_regions(bounds, viewport).filter.vertical;
			let end = [
				region.origin[0] + region.extent.width(),
				region.origin[1] + region.extent.height(),
			];
			let sample_xs = if width == 19 {
				(0..width).collect::<Vec<_>>()
			} else {
				vec![2, 3, width / 2, width - 3]
			};
			for y in 0..viewport.height() {
				for &x in &sample_xs {
					let pixel = [x as f32 + 0.5, y as f32 + 0.5];
					if pixel[0] < bounds[0] || pixel[0] >= bounds[2] || pixel[1] < bounds[1] || pixel[1] >= bounds[3] {
						continue;
					}
					let base = [pixel[0] * 0.5 - 0.5, pixel[1] * 0.5 - 0.5];
					for offset in tent_offsets {
						let sample = [base[0] + offset[0], base[1] + offset[1]];
						for sampled_y in [sample[1].floor(), sample[1].ceil()] {
							for sampled_x in [sample[0].floor(), sample[0].ceil()] {
								let sampled_x = sampled_x.clamp(0.0, target.width().saturating_sub(1) as f32) as u32;
								let sampled_y = sampled_y.clamp(0.0, target.height().saturating_sub(1) as f32) as u32;

								assert!((region.origin[0]..end[0]).contains(&sampled_x));
								assert!((region.origin[1]..end[1]).contains(&sampled_y));
							}
						}
					}
				}
			}
		}
	}

	/// Verifies every adaptive path executes the production shaders over representative UI signals.
	#[test]
	// The sweep keeps all radius, scale, and sampled-color assertions in one production-chain regression.
	#[allow(clippy::excessive_nesting)]
	fn backdrop_blur_production_besl_chain_sweep_preserves_positive_filtering() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let composite = compile_ui_blur_shader(UI_BLUR_COMPOSITE_BESL);
		let extent = Extent::rectangle(49, 5);
		let radii = [0.0, 0.25, 1.0, 4.0, 18.0, 32.0, 64.0];
		for pattern in [
			BlurChainPattern::Impulse,
			BlurChainPattern::ThinLine,
			BlurChainPattern::Checkerboard,
			BlurChainPattern::Constant,
		] {
			let texels = blur_chain_fixture(pattern, extent);
			let row = extent.height() / 2;
			let input = &texels[(row * extent.width()) as usize..((row + 1) * extent.width()) as usize];
			let input_variation = input.windows(2).map(|pair| (pair[1][0] - pair[0][0]).abs()).sum::<f32>();
			for display_scale in [1.0, 2.0] {
				for radius in radii {
					let output =
						run_adaptive_blur_scanline_vm(&downsample, &filter, &composite, &texels, extent, radius, display_scale);
					for color in &output {
						for channel in color.iter().take(3) {
							assert!(
								channel.is_finite() && (0.0..=1.0).contains(channel),
								"Adaptive blur introduced an invalid color at radius {radius} and scale {display_scale}"
							);
						}
					}
					let output_variation = output.windows(2).map(|pair| (pair[1][0] - pair[0][0]).abs()).sum::<f32>();

					assert!(
						output_variation <= input_variation + 1e-4,
						"Positive blur increased scanline variation at radius {radius} and scale {display_scale}"
					);
					if matches!(pattern, BlurChainPattern::Constant) {
						for color in output {
							assert_rgba_close(color, [0.25, 0.5, 0.75, 1.0], 2e-5);
						}
					} else if radius == 0.0 {
						assert_eq!(output, input);
					} else {
						assert!(
							output
								.iter()
								.zip(input)
								.any(|(actual, source)| (actual[0] - source[0]).abs() > 1e-5)
						);
					}
				}
			}
		}
	}

	#[test]
	fn backdrop_blur_production_chain_changes_continuously_across_radius_sweep() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let composite = compile_ui_blur_shader(UI_BLUR_COMPOSITE_BESL);
		let extent = Extent::rectangle(49, 1);
		let texels = blur_chain_fixture(BlurChainPattern::ThinLine, extent);
		let sample_center = |radius| {
			run_adaptive_blur_scanline_vm(&downsample, &filter, &composite, &texels, extent, radius, 1.0)
				[extent.width() as usize / 2][0]
		};

		let at_zero = sample_center(0.0);
		let near_zero = sample_center(1e-6);

		assert!((at_zero - near_zero).abs() < 1e-6, "Blur popped when leaving radius zero");

		let mut previous = at_zero;
		let mut plateau_steps = 0;
		let mut largest_step = 0.0f32;
		for step in 1..=512 {
			let current = sample_center(step as f32 * 0.125);
			let delta = (current - previous).abs();

			assert!(current.is_finite());
			largest_step = largest_step.max(delta);
			plateau_steps += usize::from(delta <= 1e-7);
			previous = current;
		}

		assert!(
			largest_step < 0.4,
			"Radius sweep contained a visible output jump of {largest_step}"
		);
		assert!(plateau_steps <= 1, "Radius sweep retained {plateau_steps} quantized plateaus");

		let sigma_scale = blur_sigma(1.0);
		for crossover_sigma in [4.0f32, 6.0] {
			let crossover_radius = (crossover_sigma / sigma_scale).powi(2);
			let before = sample_center(crossover_radius - 0.001);
			let after = sample_center(crossover_radius + 0.001);

			assert!(
				(before - after).abs() < 5e-4,
				"Resolution crossover at sigma {crossover_sigma} introduced a discontinuity"
			);
		}
	}

	#[test]
	fn backdrop_blur_awkward_width_impulse_centroid_stays_phase_aligned() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let composite = compile_ui_blur_shader(UI_BLUR_COMPOSITE_BESL);
		for width in [2_801, 2_802] {
			let extent = Extent::rectangle(width, 1);
			let texels = blur_chain_fixture(BlurChainPattern::Impulse, extent);
			let output = run_adaptive_blur_scanline_vm(&downsample, &filter, &composite, &texels, extent, 18.0, 2.0);
			let energy = output.iter().map(|color| color[0]).sum::<f32>();
			let centroid = output.iter().enumerate().map(|(x, color)| x as f32 * color[0]).sum::<f32>() / energy;
			let source_centroid = (width / 2) as f32;

			assert!(
				(centroid - source_centroid).abs() <= 0.25,
				"Blur centroid drifted from {source_centroid} to {centroid} at width {width}"
			);
		}
	}

	#[test]
	fn backdrop_blur_regional_production_chain_never_samples_stale_texels() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let composite = compile_ui_blur_shader(UI_BLUR_COMPOSITE_BESL);
		let viewport = Extent::rectangle(129, 33);
		let bounds = [45.25, 10.25, 83.75, 22.75];
		let regions = blur_half_dispatch_regions(bounds, viewport);
		let target = blur_half_extent(viewport);
		let constant = [0.2, 0.4, 0.6, 1.0];
		let source_texels = vec![constant; (viewport.width() * viewport.height()) as usize];
		let stale_texels = vec![[f32::NAN; 4]; (target.width() * target.height()) as usize];
		let mut source = texture_2d(viewport.width(), viewport.height(), &source_texels);
		let mut downsampled = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_downsample_region_vm(&downsample, &mut source, &mut downsampled, regions.downsample);
		for y in regions.downsample.origin[1]..regions.downsample.origin[1] + regions.downsample.extent.height() {
			for x in regions.downsample.origin[0]..regions.downsample.origin[0] + regions.downsample.extent.width() {
				assert!(
					rgba(&downsampled, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale downsample texel at [{x}, {y}]"
				);
			}
		}

		let sigma = blur_sigma(36.0);
		let kernel = UiBlurKernel::gaussian(blur_half_sigma(sigma));
		let mut horizontal = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_filter_region_vm(
			&filter,
			&mut downsampled,
			&mut horizontal,
			kernel,
			[1.0, 0.0],
			regions.filter.horizontal,
		);
		for y in
			regions.filter.horizontal.origin[1]..regions.filter.horizontal.origin[1] + regions.filter.horizontal.extent.height()
		{
			for x in regions.filter.horizontal.origin[0]
				..regions.filter.horizontal.origin[0] + regions.filter.horizontal.extent.width()
			{
				assert!(
					rgba(&horizontal, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale horizontal texel at [{x}, {y}]"
				);
			}
		}
		let mut vertical = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_filter_region_vm(
			&filter,
			&mut horizontal,
			&mut vertical,
			kernel,
			[0.0, 1.0],
			regions.filter.vertical,
		);
		for y in regions.filter.vertical.origin[1]..regions.filter.vertical.origin[1] + regions.filter.vertical.extent.height()
		{
			for x in
				regions.filter.vertical.origin[0]..regions.filter.vertical.origin[0] + regions.filter.vertical.extent.width()
			{
				assert!(
					rgba(&vertical, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale vertical texel at [{x}, {y}]"
				);
			}
		}

		let full_stale = vec![[f32::NAN; 4]; (viewport.width() * viewport.height()) as usize];
		let mut full = texture_2d(viewport.width(), viewport.height(), &full_stale);
		for y in 0..viewport.height() {
			for x in 0..viewport.width() {
				let pixel = [x as f32 + 0.5, y as f32 + 0.5];
				if pixel[0] < bounds[0] || pixel[0] >= bounds[2] || pixel[1] < bounds[1] || pixel[1] >= bounds[3] {
					continue;
				}
				let output = run_blur_composite_textures_vm(&composite, &mut full, &mut vertical, pixel, 1.0, [0.0; 4]);
				assert_rgba_close(output, constant, 2e-5);
			}
		}
	}

	/// The `UiFragmentVmInputs` struct provides one deterministic fragment invocation to the BESL VM tests.
	struct UiFragmentVmInputs {
		color: [f32; 4],
		pixel_position: [f32; 2],
		local_position: [f32; 2],
		rect_size: [f32; 2],
		corner_radius: f32,
		corner_exponent: f32,
		layer_kind: f32,
		stroke_width: f32,
		feather_mask_position: [f32; 2],
		feather_mask_size: [f32; 2],
		feather_mask_edges: [f32; 4],
		feather_mask_corner: [f32; 2],
	}

	impl Default for UiFragmentVmInputs {
		/// Provides a centered fill invocation whose output should preserve the input color.
		fn default() -> Self {
			Self {
				color: [0.2, 0.4, 0.6, 0.8],
				pixel_position: [50.0, 50.0],
				local_position: [50.0, 50.0],
				rect_size: [100.0, 100.0],
				corner_radius: 12.0,
				corner_exponent: 2.0,
				layer_kind: 0.0,
				stroke_width: 0.0,
				feather_mask_position: [0.0, 0.0],
				feather_mask_size: [0.0, 0.0],
				feather_mask_edges: [0.0; 4],
				feather_mask_corner: [0.0, 2.0],
			}
		}
	}

	/// Executes the production UI fragment shader for one set of interpolated inputs.
	fn run_ui_fragment_vm(values: UiFragmentVmInputs) -> [f32; 4] {
		let executable = ExecutableProgram::compile(ui_raster_program(UI_RECT_FRAGMENT_BESL, "UI rectangle fragment shader"))
			.expect(
				"Failed to compile UI fragment shader for the BESL VM. The most likely cause is missing VM shader support.",
			);
		let mut inputs = [
			(1, "_besl_interface_color", Value::Vec4F(values.color)),
			(10, "_besl_interface_pixel_position", Value::Vec2F(values.pixel_position)),
			(9, "_besl_interface_local_position", Value::Vec2F(values.local_position)),
			(11, "_besl_interface_rect_size", Value::Vec2F(values.rect_size)),
			(3, "_besl_interface_corner_radius", Value::F32(values.corner_radius)),
			(2, "_besl_interface_corner_exponent", Value::F32(values.corner_exponent)),
			(8, "_besl_interface_layer_kind", Value::F32(values.layer_kind)),
			(13, "_besl_interface_stroke_width", Value::F32(values.stroke_width)),
			(
				6,
				"_besl_interface_feather_mask_position",
				Value::Vec2F(values.feather_mask_position),
			),
			(7, "_besl_interface_feather_mask_size", Value::Vec2F(values.feather_mask_size)),
			(
				5,
				"_besl_interface_feather_mask_edges",
				Value::Vec4F(values.feather_mask_edges),
			),
			(
				4,
				"_besl_interface_feather_mask_corner",
				Value::Vec2F(values.feather_mask_corner),
			),
		]
		.map(|(location, name, value)| {
			let mut input = Buffer::new(
				executable
					.input_layout(location)
					.expect("Missing UI fragment input layout. The most likely cause is an unused or unresolved shader input.")
					.clone(),
			);
			input
				.write(name, value)
				.expect("Failed to seed a UI fragment VM input. The most likely cause is an interface type mismatch.");
			(location, input)
		});

		let mut output = Buffer::new(
			executable
				.output_layout(0)
				.expect("Missing UI fragment output layout. The most likely cause is an unresolved shader output.")
				.clone(),
		);
		{
			let mut descriptors = DescriptorBindings::new();
			for (location, input) in &mut inputs {
				descriptors.bind_buffer(input_slot(*location), input);
			}
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute UI fragment shader. The most likely cause is incomplete BESL VM support.");
		}

		match output
			.read("_besl_output_color_attachment")
			.expect("Failed to read UI fragment output. The most likely cause is an interface layout mismatch.")
		{
			Value::Vec4F(color) => color,
			value => panic!(
				"Invalid UI fragment output type `{value:?}`. The most likely cause is a BESL VM interface type mismatch."
			),
		}
	}

	fn draw_element(corner_radius: f32, corner_exponent: f32) -> UiDrawElement {
		UiDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [50.0, 50.0],
			clip: None,
			feather_mask: None,
			color: [1.0, 1.0, 1.0, 1.0],
			corner_radius,
			corner_exponent,
			layer_kind: LayerKind::Fill,
			stroke_width: 0.0,
		}
	}

	fn image_pixels(width: u32, height: u32) -> Vec<u8> {
		vec![255; width as usize * height as usize * 4]
	}

	fn triangle_area(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
		(b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
	}

	fn curve_element(segments: Vec<CurveSegment>) -> UiCurveDrawElement {
		UiCurveDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [100.0, 100.0],
			clip: None,
			feather_mask: None,
			color: [1.0, 1.0, 1.0, 1.0],
			stroke_width: 4.0,
			segments,
		}
	}

	#[test]
	fn builds_a_single_batched_quad() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![UiDrawElement {
					depth: 0,
					order: 0,
					position: [10.0, 20.0],
					size: [30.0, 40.0],
					clip: None,
					feather_mask: None,
					color: [0.25, 0.5, 0.75, 1.0],
					corner_radius: 8.0,
					corner_exponent: 2.0,
					layer_kind: LayerKind::Fill,
					stroke_width: 0.0,
				}],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(200, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices.len(), 4);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(
			geometry.batches.as_slice(),
			[UiDrawBatch {
				depth: 0,
				order: 0,
				index_count: UI_INDICES_PER_ELEMENT as u32,
				first_index: 0,
				vertex_offset: 0,
			}]
		);
		assert_vec2_close(geometry.vertices[0].position, [-0.8, 0.6]);
		assert_vec2_close(geometry.vertices[2].position, [-0.2, -0.2]);

		assert_eq!(geometry.vertices[2].local_position, [60.0, 40.0]);
		assert_eq!(geometry.vertices[0].rect_size, [60.0, 40.0]);
		assert_eq!(geometry.vertices[0].corner_radius, 8.0);
		assert_eq!(geometry.vertices[0].corner_exponent, 2.0);
		assert_eq!(geometry.vertices[0].layer_kind, 0.0);
		assert_eq!(geometry.vertices[0].stroke_width, 0.0);
	}

	#[test]
	fn blur_geometry_builds_an_adaptive_composite_quad_at_display_scale() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_blur_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: Vec::new(),
				blurs: vec![UiBlurDrawElement {
					depth: 2,
					order: 7,
					position: [10.0, 20.0],
					size: [30.0, 40.0],
					clip: None,
					feather_mask: None,
					color: [0.0, 0.0, 0.0, 0.45],
					corner_radius: 8.0,
					corner_exponent: 2.0,
					radius: 18.0,
				}],
				curves: Vec::new(),
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(200, 200),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices.len(), 4);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(geometry.batches.len(), 1);
		assert_eq!(geometry.batches[0].depth, 2);
		assert_eq!(geometry.batches[0].order, 7);
		let expected_sigma = blur_sigma(36.0);

		assert_eq!(geometry.batches[0].resolution_mix, 1.0);
		assert_eq!(geometry.batches[0].full_kernel, UiBlurKernel::gaussian(expected_sigma));
		assert_eq!(
			geometry.batches[0].half_kernel,
			UiBlurKernel::gaussian(blur_half_sigma(expected_sigma))
		);
		assert_eq!(
			geometry.batches[0].half_regions.filter.vertical,
			UiBlurDispatchRegion {
				origin: [8, 18],
				extent: Extent::rectangle(34, 44),
			}
		);
		assert!(geometry.vertices.iter().all(|vertex| vertex.blur_resolution_mix == 1.0));
		assert_vec2_close(geometry.vertices[0].position, [-0.8, 0.6]);

		assert_eq!(geometry.vertices[0].color, [0.0, 0.0, 0.0, 0.45]);
	}

	#[test]
	fn blurred_fill_layer_is_not_added_to_normal_rectangles() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.style(ConcreteLayer::default().backdrop_blur(18.0)),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let mut draw_list = UiDrawList::default();
		update_from_render(&render, &mut draw_list);

		assert!(draw_list.elements.is_empty());
		assert_eq!(draw_list.blurs.len(), 1);
		assert_eq!(draw_list.blurs[0].radius, 18.0);
	}

	#[test]
	fn rectangle_batches_split_when_depth_changes() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![
					UiDrawElement {
						depth: 0,
						order: 0,
						position: [0.0, 0.0],
						size: [10.0, 10.0],
						clip: None,
						feather_mask: None,
						color: [1.0, 1.0, 1.0, 1.0],
						corner_radius: 0.0,
						corner_exponent: 2.0,
						layer_kind: LayerKind::Fill,
						stroke_width: 0.0,
					},
					UiDrawElement {
						depth: 1,
						order: 1,
						position: [0.0, 0.0],
						size: [10.0, 10.0],
						clip: None,
						feather_mask: None,
						color: [1.0, 1.0, 1.0, 1.0],
						corner_radius: 0.0,
						corner_exponent: 2.0,
						layer_kind: LayerKind::Fill,
						stroke_width: 0.0,
					},
				],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::square(100),
			&frame_allocator,
		);

		assert_eq!(geometry.batches.len(), 2);
		assert_eq!(geometry.batches[0].depth, 0);
		assert_eq!(geometry.batches[1].depth, 1);
	}

	#[test]
	fn scales_corner_radius_to_viewport_pixels() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(6.0, 2.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(200, 300),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_radius, 12.0);
	}

	#[test]
	fn clamps_corner_radius_to_half_the_shortest_edge() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![UiDrawElement {
					depth: 0,
					order: 0,
					position: [0.0, 0.0],
					size: [80.0, 20.0],
					clip: None,
					feather_mask: None,
					color: [1.0, 1.0, 1.0, 1.0],
					corner_radius: 80.0,
					corner_exponent: 2.0,
					layer_kind: LayerKind::Fill,
					stroke_width: 0.0,
				}],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_radius, 10.0);
	}

	#[test]
	fn clipped_geometry_trims_vertices_but_preserves_local_position() {
		let frame_allocator = bumpalo::Bump::new();
		let mut element = draw_element(0.0, 2.0);
		element.position = [20.0, 20.0];
		element.size = [40.0, 40.0];
		element.clip = Some(DrawClip {
			position: [30.0, 10.0],
			size: [20.0, 30.0],
		});

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![element],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices.len(), UI_VERTICES_PER_ELEMENT);
		assert_vec2_close(geometry.vertices[0].local_position, [10.0, 0.0]);
		assert_vec2_close(geometry.vertices[1].local_position, [30.0, 0.0]);
		assert_vec2_close(geometry.vertices[2].local_position, [30.0, 20.0]);
		assert_vec2_close(geometry.vertices[3].local_position, [10.0, 20.0]);
		assert_vec2_close(geometry.vertices[0].rect_size, [40.0, 40.0]);
	}

	#[test]
	fn feather_mask_scales_to_viewport_pixels() {
		let frame_allocator = bumpalo::Bump::new();
		let mut element = draw_element(0.0, 2.0);
		element.feather_mask = Some(DrawFeatherMask {
			position: [10.0, 20.0],
			size: [30.0, 40.0],
			edges: [1.0, 2.0, 3.0, 4.0],
			corner: [5.0, 3.0],
		});

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![element],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(200, 300),
			&frame_allocator,
		);

		assert_vec2_close(geometry.vertices[0].feather_mask_position, [20.0, 60.0]);
		assert_vec2_close(geometry.vertices[0].feather_mask_size, [60.0, 120.0]);

		assert_eq!(geometry.vertices[0].feather_mask_edges, [3.0, 4.0, 9.0, 8.0]);
		assert_eq!(geometry.vertices[0].feather_mask_corner, [10.0, 3.0]);
	}

	#[test]
	fn fully_clipped_geometry_is_skipped_before_capacity_checks() {
		let frame_allocator = bumpalo::Bump::new();
		let mut element = draw_element(0.0, 2.0);
		element.position = [20.0, 20.0];
		element.size = [10.0, 10.0];
		element.clip = Some(DrawClip {
			position: [40.0, 40.0],
			size: [10.0, 10.0],
		});

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![element],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert!(geometry.vertices.is_empty());
		assert!(geometry.indices.is_empty());
		assert!(geometry.batches.is_empty());
	}

	#[test]
	fn negative_corner_radius_resolves_to_square_corners() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(-8.0, 2.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_radius, 0.0);
	}

	#[test]
	fn explicit_corner_exponent_is_uploaded_to_vertices() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(8.0, 4.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_exponent, 4.0);
	}

	#[test]
	fn fill_layer_uploads_fill_kind() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(0.0, 2.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].layer_kind, 0.0);
		assert_eq!(geometry.vertices[0].stroke_width, 0.0);
	}

	#[test]
	fn stroke_layer_uploads_scaled_stroke_width() {
		let frame_allocator = bumpalo::Bump::new();
		let mut element = draw_element(0.0, 2.0);
		element.layer_kind = LayerKind::Stroke { width: 3.0 };
		element.stroke_width = 3.0;

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![element],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(200, 300),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].layer_kind, 1.0);
		assert_eq!(geometry.vertices[0].stroke_width, 6.0);
	}

	#[test]
	fn invalid_stroke_widths_are_skipped() {
		for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
			let frame_allocator = bumpalo::Bump::new();
			let mut element = draw_element(0.0, 2.0);
			element.layer_kind = LayerKind::Stroke { width };
			element.stroke_width = width;

			let geometry = build_ui_geometry(
				&UiDrawList {
					layout_size: [100.0, 100.0],
					elements: vec![element],
					blurs: Vec::new(),
					curves: Vec::new(),
					images: Vec::new(),
					texts: vec![],
				},
				Extent::rectangle(100, 100),
				&frame_allocator,
			);

			assert!(geometry.vertices.is_empty());
			assert!(geometry.indices.is_empty());
		}
	}

	#[test]
	fn line_curve_segment_flattens_to_one_span() {
		let frame_allocator = bumpalo::Bump::new();
		let mut points = Vec::new_in(&frame_allocator);
		flatten_curve_segment(
			&CurveSegment::Line {
				from: CurvePoint::new(1.0, 2.0),
				to: CurvePoint::new(5.0, 6.0),
			},
			[10.0, 20.0],
			2.0,
			3.0,
			0.35,
			&mut points,
		);

		assert_eq!(points.len(), 2);
		assert_eq!(points[0], CurvePoint::new(22.0, 66.0));
		assert_eq!(points[1], CurvePoint::new(30.0, 78.0));
	}

	#[test]
	fn quadratic_and_cubic_curves_flatten_adaptively() {
		let frame_allocator = bumpalo::Bump::new();
		let mut quadratic = Vec::new_in(&frame_allocator);
		flatten_curve_segment(
			&CurveSegment::Quadratic {
				from: CurvePoint::new(0.0, 0.0),
				control: CurvePoint::new(50.0, 100.0),
				to: CurvePoint::new(100.0, 0.0),
			},
			[0.0, 0.0],
			1.0,
			1.0,
			0.35,
			&mut quadratic,
		);

		let mut cubic = Vec::new_in(&frame_allocator);
		flatten_curve_segment(
			&CurveSegment::Cubic {
				from: CurvePoint::new(0.0, 0.0),
				control0: CurvePoint::new(20.0, 100.0),
				control1: CurvePoint::new(80.0, -100.0),
				to: CurvePoint::new(100.0, 0.0),
			},
			[0.0, 0.0],
			1.0,
			1.0,
			0.35,
			&mut cubic,
		);

		assert!(quadratic.len() > 2);
		assert!(cubic.len() > 2);
		assert_eq!(quadratic[0], CurvePoint::new(0.0, 0.0));
		assert_eq!(quadratic[quadratic.len() - 1], CurvePoint::new(100.0, 0.0));
		assert_eq!(cubic[0], CurvePoint::new(0.0, 0.0));
		assert_eq!(cubic[cubic.len() - 1], CurvePoint::new(100.0, 0.0));
	}

	#[test]
	fn curve_geometry_builds_anti_aliased_span_quad() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_curve_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: Vec::new(),
				blurs: Vec::new(),
				curves: vec![curve_element(vec![CurveSegment::Line {
					from: CurvePoint::new(10.0, 20.0),
					to: CurvePoint::new(30.0, 20.0),
				}])],
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(200, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices.len(), UI_VERTICES_PER_CURVE_SPAN);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_CURVE_SPAN);
		assert_eq!(geometry.batches.len(), 1);
		assert_eq!(geometry.vertices[0].segment_from, [20.0, 20.0]);
		assert_eq!(geometry.vertices[0].segment_to, [60.0, 20.0]);
		assert_eq!(geometry.vertices[0].half_width, 2.0);
		assert!(geometry.vertices[0].pixel_position[0] < 20.0);
		assert!(geometry.vertices[0].pixel_position[1] < 20.0);
	}

	#[test]
	fn curve_quad_winding_matches_rectangle_winding() {
		let frame_allocator = bumpalo::Bump::new();
		let rect_geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(0.0, 2.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);
		let curve_geometry = build_ui_curve_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: Vec::new(),
				blurs: Vec::new(),
				curves: vec![curve_element(vec![CurveSegment::Line {
					from: CurvePoint::new(10.0, 20.0),
					to: CurvePoint::new(30.0, 20.0),
				}])],
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		let rect_area = triangle_area(
			rect_geometry.vertices[0].position,
			rect_geometry.vertices[1].position,
			rect_geometry.vertices[2].position,
		);
		let curve_area = triangle_area(
			curve_geometry.vertices[0].position,
			curve_geometry.vertices[1].position,
			curve_geometry.vertices[2].position,
		);

		assert!(rect_area < 0.0);
		assert!(curve_area < 0.0);
	}

	#[test]
	fn curve_geometry_clips_partially_visible_spans() {
		let frame_allocator = bumpalo::Bump::new();
		let mut curve = curve_element(vec![CurveSegment::Line {
			from: CurvePoint::new(0.0, 10.0),
			to: CurvePoint::new(100.0, 10.0),
		}]);
		curve.clip = Some(DrawClip {
			position: [25.0, 0.0],
			size: [50.0, 20.0],
		});
		let geometry = build_ui_curve_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: Vec::new(),
				blurs: Vec::new(),
				curves: vec![curve],
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].segment_from, [25.0, 10.0]);
		assert_eq!(geometry.vertices[0].segment_to, [75.0, 10.0]);
	}

	#[test]
	fn curve_geometry_skips_invalid_or_non_positive_strokes() {
		for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
			let frame_allocator = bumpalo::Bump::new();
			let mut curve = curve_element(vec![CurveSegment::Line {
				from: CurvePoint::new(0.0, 0.0),
				to: CurvePoint::new(10.0, 0.0),
			}]);
			curve.stroke_width = width;
			let geometry = build_ui_curve_geometry(
				&UiDrawList {
					layout_size: [100.0, 100.0],
					elements: Vec::new(),
					blurs: Vec::new(),
					curves: vec![curve],
					images: Vec::new(),
					texts: Vec::new(),
				},
				Extent::rectangle(100, 100),
				&frame_allocator,
			);

			assert!(geometry.vertices.is_empty());
			assert!(geometry.indices.is_empty());
		}
	}

	#[test]
	fn checked_in_ui_raster_besl_sources_link() {
		for (shader_name, source) in [
			("UI rectangle vertex shader", UI_RECT_VERTEX_BESL),
			("UI rectangle fragment shader", UI_RECT_FRAGMENT_BESL),
			("UI curve vertex shader", UI_CURVE_VERTEX_BESL),
			("UI curve fragment shader", UI_CURVE_FRAGMENT_BESL),
			("UI image vertex shader", UI_IMAGE_VERTEX_BESL),
			("UI image fragment shader", UI_IMAGE_FRAGMENT_BESL),
			("UI text vertex shader", UI_TEXT_VERTEX_BESL),
			("UI text fragment shader", UI_TEXT_FRAGMENT_BESL),
			("UI backdrop blur composite fragment shader", UI_BLUR_COMPOSITE_BESL),
		] {
			ui_raster_program(source, shader_name);
		}
	}

	/// Verifies the production UI vertex shader preserves every geometry and styling varying.
	#[test]
	fn ui_vertex_besl_vm_forwards_position_and_varyings() {
		let executable = ExecutableProgram::compile(ui_raster_program(UI_RECT_VERTEX_BESL, "UI rectangle vertex shader"))
			.expect("Failed to compile UI vertex shader for the BESL VM. The most likely cause is missing VM shader support.");
		let mut inputs: [Buffer; 14] = std::array::from_fn(|location| {
			Buffer::new(
				executable
					.input_layout(location as u8)
					.expect("Missing UI vertex input layout. The most likely cause is an unresolved shader input.")
					.clone(),
			)
		});
		let input_names = [
			"in_position",
			"in_pixel_position",
			"in_local_position",
			"in_rect_size",
			"in_color",
			"in_corner_radius",
			"in_corner_exponent",
			"in_layer_kind",
			"in_stroke_width",
			"in_feather_mask_position",
			"in_feather_mask_size",
			"in_feather_mask_edges",
			"in_feather_mask_corner",
			"in_blur_resolution_mix",
		];
		let input_values = [
			Value::Vec2F([0.25, -0.75]),
			Value::Vec2F([10.0, 20.0]),
			Value::Vec2F([3.0, 4.0]),
			Value::Vec2F([100.0, 80.0]),
			Value::Vec4F([0.1, 0.2, 0.3, 0.4]),
			Value::F32(12.0),
			Value::F32(3.0),
			Value::F32(1.0),
			Value::F32(2.5),
			Value::Vec2F([5.0, 6.0]),
			Value::Vec2F([70.0, 60.0]),
			Value::Vec4F([1.0, 2.0, 3.0, 4.0]),
			Value::Vec2F([9.0, 2.0]),
			Value::F32(0.375),
		];
		for ((input, name), value) in inputs.iter_mut().zip(input_names).zip(input_values) {
			input
				.write(name, value)
				.expect("Failed to seed a UI vertex VM input. The most likely cause is an interface type mismatch.");
		}

		let mut position = Buffer::new(
			executable
				.builtin_position_layout()
				.expect("Missing UI vertex position layout. The most likely cause is an unresolved position output.")
				.clone(),
		);
		let mut outputs: [Buffer; 14] = std::array::from_fn(|location| {
			Buffer::new(
				executable
					.output_layout(location as u8)
					.expect("Missing UI vertex varying layout. The most likely cause is an unresolved shader output.")
					.clone(),
			)
		});
		{
			let mut descriptors = DescriptorBindings::new();
			for (location, input) in inputs.iter_mut().enumerate() {
				descriptors.bind_buffer(input_slot(location as u8), input);
			}
			descriptors.bind_buffer(builtin_position_slot(), &mut position);
			for (location, output) in outputs.iter_mut().enumerate() {
				descriptors.bind_buffer(output_slot(location as u8), output);
			}
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute UI vertex shader. The most likely cause is incomplete BESL VM support.");
		}

		assert_eq!(
			position.read("_besl_interface_position").expect("Expected position output"),
			Value::Vec4F([0.25, -0.75, 0.0, 1.0])
		);
		for ((output, name), expected) in outputs
			.iter()
			.zip([
				"_besl_interface_blur_resolution_mix",
				"_besl_interface_color",
				"_besl_interface_corner_exponent",
				"_besl_interface_corner_radius",
				"_besl_interface_feather_mask_corner",
				"_besl_interface_feather_mask_edges",
				"_besl_interface_feather_mask_position",
				"_besl_interface_feather_mask_size",
				"_besl_interface_layer_kind",
				"_besl_interface_local_position",
				"_besl_interface_pixel_position",
				"_besl_interface_rect_size",
				"_besl_interface_screen_uv",
				"_besl_interface_stroke_width",
			])
			.zip([
				Value::F32(0.375),
				Value::Vec4F([0.1, 0.2, 0.3, 0.4]),
				Value::F32(3.0),
				Value::F32(12.0),
				Value::Vec2F([9.0, 2.0]),
				Value::Vec4F([1.0, 2.0, 3.0, 4.0]),
				Value::Vec2F([5.0, 6.0]),
				Value::Vec2F([70.0, 60.0]),
				Value::F32(1.0),
				Value::Vec2F([3.0, 4.0]),
				Value::Vec2F([10.0, 20.0]),
				Value::Vec2F([100.0, 80.0]),
				Value::Vec2F([0.625, 0.875]),
				Value::F32(2.5),
			]) {
			assert_eq!(output.read(name).expect("Expected UI vertex varying output"), expected);
		}
	}

	/// Verifies a centered fill fragment retains its source color.
	#[test]
	fn ui_fragment_besl_vm_preserves_centered_fill_color() {
		let expected = UiFragmentVmInputs::default().color;
		assert_vec4_close(run_ui_fragment_vm(UiFragmentVmInputs::default()), expected);
	}

	/// Verifies rounded-corner coverage rejects a fragment outside the rounded boundary.
	#[test]
	fn ui_fragment_besl_vm_rejects_rounded_corner_exterior() {
		let output = run_ui_fragment_vm(UiFragmentVmInputs {
			local_position: [0.0, 0.0],
			corner_radius: 20.0,
			..Default::default()
		});

		assert!(
			output[3] < 0.001,
			"Expected rounded corner alpha near zero, found {}",
			output[3]
		);
	}

	/// Verifies stroke coverage removes fragments that lie inside the hollow center.
	#[test]
	fn ui_fragment_besl_vm_stroke_excludes_the_center() {
		let output = run_ui_fragment_vm(UiFragmentVmInputs {
			layer_kind: 1.0,
			stroke_width: 3.0,
			..Default::default()
		});

		assert!(
			output[3] < 0.001,
			"Expected stroke center alpha near zero, found {}",
			output[3]
		);
	}

	/// Verifies the feather mask suppresses fragments outside its clipped region.
	#[test]
	fn ui_fragment_besl_vm_feather_mask_suppresses_outside_pixels() {
		let output = run_ui_fragment_vm(UiFragmentVmInputs {
			pixel_position: [10.0, 10.0],
			feather_mask_position: [25.0, 25.0],
			feather_mask_size: [50.0, 50.0],
			feather_mask_edges: [5.0; 4],
			..Default::default()
		});

		assert!(
			output[3] < 0.001,
			"Expected feathered pixel alpha near zero, found {}",
			output[3]
		);
	}

	#[test]
	fn curve_geometry_reports_capacity_truncation() {
		let frame_allocator = bumpalo::Bump::new();
		let curves = (0..=MAX_UI_ELEMENTS)
			.map(|_| {
				curve_element(vec![CurveSegment::Line {
					from: CurvePoint::new(0.0, 0.0),
					to: CurvePoint::new(1.0, 0.0),
				}])
			})
			.collect();
		let geometry = build_ui_curve_geometry(
			&UiDrawList {
				layout_size: [1.0, 1.0],
				elements: Vec::new(),
				blurs: Vec::new(),
				curves,
				images: Vec::new(),
				texts: Vec::new(),
			},
			Extent::rectangle(1, 1),
			&frame_allocator,
		);

		assert!(geometry.truncated);
		assert_eq!(geometry.vertices.len(), MAX_UI_ELEMENTS * UI_VERTICES_PER_CURVE_SPAN);
	}

	#[test]
	fn invalid_corner_exponents_resolve_to_round_corners() {
		for exponent in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5] {
			let frame_allocator = bumpalo::Bump::new();
			let geometry = build_ui_geometry(
				&UiDrawList {
					layout_size: [100.0, 100.0],
					elements: vec![draw_element(8.0, exponent)],
					blurs: Vec::new(),
					curves: Vec::new(),
					images: Vec::new(),
					texts: vec![],
				},
				Extent::rectangle(100, 100),
				&frame_allocator,
			);

			assert_eq!(geometry.vertices[0].corner_exponent, 2.0);
		}
	}

	#[test]
	fn high_corner_exponents_are_clamped() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(8.0, 12.0)],
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_exponent, 8.0);
	}

	#[test]
	fn splits_large_batches_to_stay_within_u16_indices() {
		let frame_allocator = bumpalo::Bump::new();
		let element_count = MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT + 1;
		let elements = (0..element_count)
			.map(|_| UiDrawElement {
				depth: 0,
				order: 0,
				position: [0.0, 0.0],
				size: [1.0, 1.0],
				clip: None,
				feather_mask: None,
				color: [1.0, 1.0, 1.0, 1.0],
				corner_radius: 0.0,
				corner_exponent: 2.0,
				layer_kind: LayerKind::Fill,
				stroke_width: 0.0,
			})
			.collect();

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [1.0, 1.0],
				elements,
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::square(1),
			&frame_allocator,
		);

		assert_eq!(geometry.batches.len(), 2);
		assert_eq!(
			geometry.batches[0].index_count as usize,
			MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT * UI_INDICES_PER_ELEMENT
		);
		assert_eq!(geometry.batches[0].first_index, 0);
		assert_eq!(geometry.batches[0].vertex_offset, 0);
		assert_eq!(geometry.batches[1].index_count, UI_INDICES_PER_ELEMENT as u32);
		assert_eq!(
			geometry.batches[1].first_index as usize,
			MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT * UI_INDICES_PER_ELEMENT
		);
		assert_eq!(geometry.batches[1].vertex_offset as usize, MAX_UI_VERTICES_PER_DRAW);
	}

	#[test]
	fn skips_zero_alpha_elements_before_capacity_checks() {
		let frame_allocator = bumpalo::Bump::new();
		let mut elements = Vec::with_capacity(MAX_UI_ELEMENTS + 1);

		elements.extend((0..MAX_UI_ELEMENTS).map(|_| UiDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [1.0, 1.0],
			clip: None,
			feather_mask: None,
			color: [1.0, 1.0, 1.0, 0.0],
			corner_radius: 0.0,
			corner_exponent: 2.0,
			layer_kind: LayerKind::Fill,
			stroke_width: 0.0,
		}));
		elements.push(UiDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [1.0, 1.0],
			clip: None,
			feather_mask: None,
			color: [1.0, 1.0, 1.0, 1.0],
			corner_radius: 0.0,
			corner_exponent: 2.0,
			layer_kind: LayerKind::Fill,
			stroke_width: 0.0,
		});

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [1.0, 1.0],
				elements,
				blurs: Vec::new(),
				curves: Vec::new(),
				images: Vec::new(),
				texts: vec![],
			},
			Extent::square(1),
			&frame_allocator,
		);

		assert!(!geometry.truncated);
		assert_eq!(geometry.vertices.len(), UI_VERTICES_PER_ELEMENT);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(geometry.batches.len(), 1);
	}

	#[test]
	fn skips_zero_alpha_text_before_rasterization() {
		assert!(!should_rasterize_text(&UiTextDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			clip: None,
			feather_mask: None,
			color: RGBA::new(1.0, 1.0, 1.0, 0.0),
			font_size: 16.0,
			text: "Hidden".to_string(),
		}));
		assert!(should_rasterize_text(&UiTextDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			clip: None,
			feather_mask: None,
			color: RGBA::new(1.0, 1.0, 1.0, 1.0),
			font_size: 16.0,
			text: "Visible".to_string(),
		}));
	}

	#[test]
	fn update_from_render_clears_removed_text_entries() {
		let frame_allocator = bumpalo::Bump::new();
		let mut draw_list = UiDrawList::default();

		let mut text_engine = Engine::new();
		text_engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("label").text(Text::new("Option"));
			})
		});
		let mut text_snapshot = text_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let text_render = text_engine.render(&mut text_snapshot);
		update_from_render(&text_render, &mut draw_list);

		assert_eq!(draw_list.texts.len(), 1);

		let mut no_text_engine = Engine::new();
		no_text_engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(Container::default());
			})
		});
		let mut no_text_snapshot = no_text_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let no_text_render = no_text_engine.render(&mut no_text_snapshot);
		update_from_render(&no_text_render, &mut draw_list);

		assert!(draw_list.texts.is_empty());
	}

	#[test]
	fn update_from_render_clears_removed_image_entries() {
		let frame_allocator = bumpalo::Bump::new();
		let mut draw_list = UiDrawList::default();

		let mut image_engine = Engine::new();
		image_engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("preview").image(Image::from_rgba(2, 2, image_pixels(2, 2)));
			})
		});
		let mut image_snapshot = image_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let image_render = image_engine.render(&mut image_snapshot);
		update_from_render(&image_render, &mut draw_list);

		assert_eq!(draw_list.images.len(), 1);

		let mut no_image_engine = Engine::new();
		no_image_engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(Container::default());
			})
		});
		let mut no_image_snapshot = no_image_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let no_image_render = no_image_engine.render(&mut no_image_snapshot);
		update_from_render(&no_image_render, &mut draw_list);

		assert!(draw_list.images.is_empty());
	}

	#[test]
	fn draw_list_multiplies_effective_opacity_into_layers_and_text() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default().opacity(0.5).style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default().color(RGBA::new(1.0, 0.0, 0.0, 0.8).into()))
							.layer(
								ConcreteLayer::default()
									.color(RGBA::new(0.0, 1.0, 0.0, 0.6).into())
									.stroke(2.0),
							),
					),
				);
				frame
					.element("label")
					.text(Text::new("Visible").style(ConcreteLayer::default().color(RGBA::new(1.0, 1.0, 1.0, 0.4).into())));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let mut draw_list = UiDrawList::default();
		update_from_render(&render, &mut draw_list);

		assert_eq!(draw_list.elements[0].color[3], 0.4);
		assert_eq!(draw_list.elements[1].color[3], 0.3);
		assert_eq!(draw_list.texts[0].color, RGBA::new(1.0, 1.0, 1.0, 0.2));
	}

	#[test]
	fn draw_list_multiplies_effective_opacity_into_images() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().opacity(0.5));
				frame
					.element("preview")
					.image(Image::from_rgba(4, 4, image_pixels(4, 4)).opacity(0.4));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let mut draw_list = UiDrawList::default();
		update_from_render(&render, &mut draw_list);

		assert_eq!(draw_list.images.len(), 1);
		assert!((draw_list.images[0].opacity - 0.2).abs() < 0.0001);
	}

	#[test]
	fn image_geometry_trims_uvs_to_clip() {
		let frame_allocator = bumpalo::Bump::new();
		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			elements: Vec::new(),
			blurs: Vec::new(),
			curves: Vec::new(),
			images: vec![UiImageDrawElement {
				depth: 7,
				order: 0,
				image_id: 1,
				version: 0,
				source_width: 10,
				source_height: 10,
				pixels: image_pixels(10, 10).into(),
				position: [10.0, 20.0],
				size: [40.0, 20.0],
				clip: Some(DrawClip {
					position: [20.0, 25.0],
					size: [20.0, 10.0],
				}),
				feather_mask: None,
				opacity: 1.0,
			}],
			texts: Vec::new(),
		};

		let geometry = build_ui_image_geometry(&draw_list, Extent::rectangle(100, 100), &frame_allocator);

		assert_eq!(geometry.vertices.len(), UI_VERTICES_PER_ELEMENT);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(geometry.batches.len(), 1);
		assert_eq!(geometry.batches[0].depth, 7);
		assert_vec2_close(geometry.vertices[0].uv, [0.25, 0.25]);
		assert_vec2_close(geometry.vertices[2].uv, [0.75, 0.75]);
	}

	#[test]
	fn image_geometry_skips_invalid_or_transparent_images() {
		let frame_allocator = bumpalo::Bump::new();
		let hidden = UiImageDrawElement {
			depth: 0,
			order: 0,
			image_id: 1,
			version: 0,
			source_width: 2,
			source_height: 2,
			pixels: image_pixels(2, 2).into(),
			position: [0.0, 0.0],
			size: [20.0, 20.0],
			clip: None,
			feather_mask: None,
			opacity: 0.0,
		};

		assert!(!should_draw_image(&hidden));

		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			elements: Vec::new(),
			blurs: Vec::new(),
			curves: Vec::new(),
			images: vec![hidden],
			texts: Vec::new(),
		};
		let geometry = build_ui_image_geometry(&draw_list, Extent::rectangle(100, 100), &frame_allocator);

		assert!(geometry.vertices.is_empty());
		assert!(geometry.indices.is_empty());
		assert!(geometry.batches.is_empty());
	}
}
