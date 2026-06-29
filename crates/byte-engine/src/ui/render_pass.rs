use besl::ParserNode;
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use resource_management::shader::{besl::backends::spirv::SPIRVShaderGenerator, generator::ShaderGenerationSettings};
use utils::{Box, Extent, RGBA};

use super::{
	element::ElementHandle as _,
	layout::{engine, FeatherMask, Geometry},
	style::{Color, EdgeFeather, LayerKind},
};
use crate::{
	core::Entity,
	rendering::{
		common_shader_generator::CommonShaderScope,
		map_shader_binding_to_shader_binding_descriptor,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
	ui::font::TextSystem,
};

const MAIN_ATTACHMENT_FORMAT: ghi::Formats = ghi::Formats::RGBA16UNORM;
const TEXT_OVERLAY_FORMAT: ghi::Formats = ghi::Formats::RGBA8UNORM;
const TEXT_OVERLAY_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);

const UI_VERTICES_PER_ELEMENT: usize = 4;
const UI_INDICES_PER_ELEMENT: usize = 6;
const MAX_UI_VERTICES_PER_DRAW: usize = u16::MAX as usize + 1;
const MAX_UI_ELEMENTS: usize = 65_536;
const MAX_UI_VERTICES: usize = MAX_UI_ELEMENTS * UI_VERTICES_PER_ELEMENT;
const MAX_UI_INDICES: usize = MAX_UI_ELEMENTS * UI_INDICES_PER_ELEMENT;

const UI_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 13] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("PIXEL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("LOCAL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("RECT_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("COLOR", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("CORNER_RADIUS", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("CORNER_EXPONENT", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("LAYER_KIND", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("STROKE_WIDTH", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_EDGES", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_CORNER", ghi::DataTypes::Float2, 0),
];
#[derive(Debug, Clone, Copy)]
struct UiDrawElement {
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	color: [f32; 4],
	corner_radius: f32,
	corner_exponent: f32,
	layer_kind: LayerKind,
	stroke_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct UiTextDrawElement {
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	color: RGBA,
	font_size: f32,
	text: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DrawClip {
	position: [f32; 2],
	size: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DrawFeatherMask {
	position: [f32; 2],
	size: [f32; 2],
	edges: [f32; 4],
	corner: [f32; 2],
}

#[derive(Debug, Clone)]
struct UiDrawList {
	layout_size: [f32; 2],
	elements: Vec<UiDrawElement>,
	texts: Vec<UiTextDrawElement>,
}

impl Default for UiDrawList {
	fn default() -> Self {
		Self {
			layout_size: [1.0, 1.0],
			elements: Vec::new(),
			texts: Vec::new(),
		}
	}
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UiVertex {
	position: [f32; 2],
	pixel_position: [f32; 2],
	local_position: [f32; 2],
	rect_size: [f32; 2],
	color: [f32; 4],
	corner_radius: f32,
	corner_exponent: f32,
	layer_kind: f32,
	stroke_width: f32,
	feather_mask_position: [f32; 2],
	feather_mask_size: [f32; 2],
	feather_mask_edges: [f32; 4],
	feather_mask_corner: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiDrawBatch {
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
}

#[derive(Debug)]
struct UiGeometry<'a> {
	vertices: Vec<UiVertex, &'a bumpalo::Bump>,
	indices: Vec<u16, &'a bumpalo::Bump>,
	batches: Vec<UiDrawBatch, &'a bumpalo::Bump>,
	truncated: bool,
}

// Whether text rasterization should be ommitted if text is empty, 0 sized in any dimension or if fully transparent
fn should_rasterize_text(text: &UiTextDrawElement) -> bool {
	!text.text.is_empty() && text.color.a > 0.0 && text.size[0] > 0.0 && text.size[1] > 0.0
}

fn resolved_corner_radius(radius: f32, rect_width: f32, rect_height: f32) -> f32 {
	radius.max(0.0).min(rect_width.min(rect_height) * 0.5)
}

fn resolved_corner_exponent(exponent: f32) -> f32 {
	if !exponent.is_finite() || exponent < 1.0 {
		2.0
	} else {
		exponent.clamp(1.0, 8.0)
	}
}

fn layer_kind_value(kind: LayerKind) -> f32 {
	match kind {
		LayerKind::Fill => 0.0,
		LayerKind::Stroke { .. } => 1.0,
	}
}

fn stroke_width(kind: LayerKind) -> f32 {
	match kind {
		LayerKind::Fill => 0.0,
		LayerKind::Stroke { width } if width.is_finite() && width > 0.0 => width,
		LayerKind::Stroke { .. } => 0.0,
	}
}

fn draw_clip_from_geometry(clip: Option<Geometry>) -> Option<DrawClip> {
	clip.map(|clip| DrawClip {
		position: [clip.x() as f32, clip.y() as f32],
		size: [clip.width() as f32, clip.height() as f32],
	})
}

fn draw_feather_mask_from_layout(mask: Option<FeatherMask>) -> Option<DrawFeatherMask> {
	mask.map(|mask| DrawFeatherMask {
		position: [mask.geometry.x() as f32, mask.geometry.y() as f32],
		size: [mask.geometry.width() as f32, mask.geometry.height() as f32],
		edges: [mask.feather.top, mask.feather.right, mask.feather.bottom, mask.feather.left],
		corner: [mask.corner_radius, mask.corner_exponent],
	})
}

fn scaled_feather_mask(mask: Option<DrawFeatherMask>, sx: f32, sy: f32) -> DrawFeatherMask {
	mask.map(|mask| DrawFeatherMask {
		position: [mask.position[0] * sx, mask.position[1] * sy],
		size: [mask.size[0] * sx, mask.size[1] * sy],
		edges: [mask.edges[0] * sy, mask.edges[1] * sx, mask.edges[2] * sy, mask.edges[3] * sx],
		corner: [mask.corner[0] * sx.min(sy), mask.corner[1]],
	})
	.unwrap_or(DrawFeatherMask {
		position: [0.0, 0.0],
		size: [0.0, 0.0],
		edges: [0.0, 0.0, 0.0, 0.0],
		corner: [0.0, 2.0],
	})
}

fn update_from_render(render: &engine::Render, draw_list: &mut UiDrawList) {
	let root_size = render.root().size;

	draw_list.layout_size = [root_size.x() as f32, root_size.y() as f32];
	draw_list.elements.clear();
	draw_list.texts.clear();

	for element in render.elements() {
		let position = element.position;
		let size = element.size;

		for layer in element.style.layers() {
			let mut color = match &layer.color {
				Color::Value(rgba) => *rgba,
				Color::Sample(_) => RGBA::white(),
			};
			color.a *= element.opacity;
			let stroke_width = stroke_width(layer.kind);
			if matches!(layer.kind, LayerKind::Stroke { .. }) && stroke_width <= 0.0 {
				continue;
			}

			draw_list.elements.push(UiDrawElement {
				position: [position.x() as f32, position.y() as f32],
				size: [size.x() as f32, size.y() as f32],
				clip: draw_clip_from_geometry(element.clip),
				feather_mask: draw_feather_mask_from_layout(element.feather_mask),
				color: color.into(),
				corner_radius: element.corner_radius,
				corner_exponent: element.corner_exponent,
				layer_kind: layer.kind,
				stroke_width,
			});
		}
	}

	for text in render.texts() {
		let mut color = text.color;
		color.a *= text.opacity;
		let text = UiTextDrawElement {
			position: [text.position.x() as f32, text.position.y() as f32],
			size: [text.size.x() as f32, text.size.y() as f32],
			clip: draw_clip_from_geometry(text.clip),
			feather_mask: draw_feather_mask_from_layout(text.feather_mask),
			color,
			font_size: text.font_size,
			text: text.content.clone(),
		};

		if should_rasterize_text(&text) {
			draw_list.texts.push(text);
		}
	}
}

/// Rasterizes all visible text elements into the UI overlay texture for the current viewport.
fn rasterize_text_overlay(draw_list: &UiDrawList, viewport: Extent, text_system: &mut TextSystem, target: &mut [u8]) -> bool {
	let viewport_width = viewport.width().max(1);
	let viewport_height = viewport.height().max(1);

	target.fill(0);

	if draw_list.texts.is_empty() {
		return false;
	}

	let sx = viewport_width as f32 / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height as f32 / draw_list.layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let mut drew_text = false;

	for text in &draw_list.texts {
		if !should_rasterize_text(text) {
			continue;
		}

		let position = (
			(text.position[0] * sx).round().max(0.0) as u32,
			(text.position[1] * sy).round().max(0.0) as u32,
		);
		let font_size = (text.font_size * font_scale).max(1.0);
		let clip = text.clip.and_then(|clip| {
			let x = (clip.position[0] * sx).round().max(0.0) as u32;
			let y = (clip.position[1] * sy).round().max(0.0) as u32;
			let width = (clip.size[0] * sx).round().max(0.0) as u32;
			let height = (clip.size[1] * sy).round().max(0.0) as u32;
			(width > 0 && height > 0).then_some(crate::ui::font::TextClipRect::new(x, y, width, height))
		});
		let feather_mask = text.feather_mask.and_then(|mask| {
			let scaled = scaled_feather_mask(Some(mask), sx, sy);
			let x = scaled.position[0].round().max(0.0) as u32;
			let y = scaled.position[1].round().max(0.0) as u32;
			let width = scaled.size[0].round().max(0.0) as u32;
			let height = scaled.size[1].round().max(0.0) as u32;
			(width > 0 && height > 0).then_some(crate::ui::font::TextFeatherMask::new(
				x,
				y,
				width,
				height,
				EdgeFeather::edges(scaled.edges[0], scaled.edges[1], scaled.edges[2], scaled.edges[3]),
				scaled.corner[0],
				scaled.corner[1],
			))
		});

		drew_text |= text_system.rasterize(
			target,
			viewport_width,
			viewport_height,
			position,
			&text.text,
			font_size,
			text.color,
			clip,
			feather_mask,
		);
	}

	drew_text
}

/// Builds the packed UI geometry for the current viewport and splits it into `u16`-safe draw ranges.
fn build_ui_geometry<'a>(draw_list: &UiDrawList, viewport: Extent, frame_allocator: &'a bumpalo::Bump) -> UiGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let radius_scale = sx.min(sy);

	let mut geometry = UiGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_VERTICES_PER_ELEMENT,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_INDICES_PER_ELEMENT,
			frame_allocator,
		),
		batches: Vec::new_in(frame_allocator),
		truncated: false,
	};

	let mut batch_first_index = 0usize;
	let mut batch_vertex_offset = 0usize;
	let mut batch_vertex_count = 0usize;
	let mut batch_index_count = 0usize;

	for element in &draw_list.elements {
		let rect_width = (element.size[0] * sx).max(0.0);
		let rect_height = (element.size[1] * sy).max(0.0);

		if rect_width <= 0.0 || rect_height <= 0.0 || element.color[3] <= 0.0 {
			// Omit element if 0 sized in any dimension or if fully transparent
			continue;
		}

		let stroke_width = element.stroke_width * radius_scale;
		if matches!(element.layer_kind, LayerKind::Stroke { .. }) && (!stroke_width.is_finite() || stroke_width <= 0.0) {
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		if batch_vertex_count + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES_PER_DRAW {
			geometry.batches.push(UiDrawBatch {
				index_count: batch_index_count as u32,
				first_index: batch_first_index as u32,
				vertex_offset: batch_vertex_offset as i32,
			});

			batch_first_index = geometry.indices.len();
			batch_vertex_offset = geometry.vertices.len();
			batch_vertex_count = 0;
			batch_index_count = 0;
		}

		let original_x0 = element.position[0] * sx;
		let original_y0 = element.position[1] * sy;
		let original_x1 = original_x0 + rect_width;
		let original_y1 = original_y0 + rect_height;
		let (x0, y0, x1, y1) = match element.clip {
			Some(clip) => {
				let clip_x0 = clip.position[0] * sx;
				let clip_y0 = clip.position[1] * sy;
				let clip_x1 = clip_x0 + clip.size[0] * sx;
				let clip_y1 = clip_y0 + clip.size[1] * sy;
				(
					original_x0.max(clip_x0),
					original_y0.max(clip_y0),
					original_x1.min(clip_x1),
					original_y1.min(clip_y1),
				)
			}
			None => (original_x0, original_y0, original_x1, original_y1),
		};
		if x1 <= x0 || y1 <= y0 {
			continue;
		}
		let local_x0 = x0 - original_x0;
		let local_y0 = y0 - original_y0;
		let local_x1 = x1 - original_x0;
		let local_y1 = y1 - original_y0;
		let color = element.color;
		let corner_radius = resolved_corner_radius(element.corner_radius * radius_scale, rect_width, rect_height);
		let corner_exponent = resolved_corner_exponent(element.corner_exponent);
		let layer_kind = layer_kind_value(element.layer_kind);
		let feather_mask = scaled_feather_mask(element.feather_mask, sx, sy);

		let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
		let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;

		geometry.vertices.extend_from_slice(&[
			UiVertex {
				position: [to_clip_x(x0), to_clip_y(y0)],
				pixel_position: [x0, y0],
				local_position: [local_x0, local_y0],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
				corner_exponent,
				layer_kind,
				stroke_width,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiVertex {
				position: [to_clip_x(x1), to_clip_y(y0)],
				pixel_position: [x1, y0],
				local_position: [local_x1, local_y0],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
				corner_exponent,
				layer_kind,
				stroke_width,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiVertex {
				position: [to_clip_x(x1), to_clip_y(y1)],
				pixel_position: [x1, y1],
				local_position: [local_x1, local_y1],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
				corner_exponent,
				layer_kind,
				stroke_width,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiVertex {
				position: [to_clip_x(x0), to_clip_y(y1)],
				pixel_position: [x0, y1],
				local_position: [local_x0, local_y1],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
				corner_exponent,
				layer_kind,
				stroke_width,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
		]);

		let base_vertex = batch_vertex_count as u16;
		geometry.indices.extend_from_slice(&[
			base_vertex,
			base_vertex + 1,
			base_vertex + 2,
			base_vertex + 2,
			base_vertex + 3,
			base_vertex,
		]);

		batch_vertex_count += UI_VERTICES_PER_ELEMENT;
		batch_index_count += UI_INDICES_PER_ELEMENT;
	}

	if batch_index_count > 0 {
		geometry.batches.push(UiDrawBatch {
			index_count: batch_index_count as u32,
			first_index: batch_first_index as u32,
			vertex_offset: batch_vertex_offset as i32,
		});
	}

	geometry
}

/// The `UiRenderPass` struct centralizes batched UI rectangle rendering and text overlay compositing for the main render target.
pub struct UiRenderPass {
	pipeline: ghi::PipelineHandle,
	vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	text_pipeline: ghi::PipelineHandle,
	text_descriptor_set: ghi::DescriptorSetHandle,
	text_overlay: ghi::BaseImageHandle,
	main_attachment: ghi::BaseImageHandle,
	data: UiDrawList,
	reported_capacity_limit: bool,
	text_system: TextSystem,
	text_overlay_has_been_used: bool,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout primitives.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let main_attachment = render_pass_builder
			.create_render_target(ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::RenderTarget).name("UI"));

		render_pass_builder.alias("UI", "main");

		let context = render_pass_builder.context();

		let vertex_shader = create_vertex_shader(context);
		let fragment_shader = create_fragment_shader(context);

		let shaders = [
			ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(MAIN_ATTACHMENT_FORMAT)
			.blend(ghi::pipelines::raster::BlendMode::Alpha)];

		let pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[],
			&[],
			&UI_VERTEX_LAYOUT,
			&shaders,
			&attachments,
		));

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
		let text_descriptor_set_template = context.create_descriptor_set_template(Some("UI Text"), &[TEXT_OVERLAY_BINDING]);
		let text_vertex_shader = create_text_overlay_vertex_shader(context);
		let text_fragment_shader = create_text_overlay_fragment_shader(context);
		let text_shaders = [
			ghi::ShaderParameter::new(&text_vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&text_fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let text_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[text_descriptor_set_template],
			&[],
			&[],
			&text_shaders,
			&attachments,
		));
		let text_overlay = context.build_dynamic_image(
			ghi::image::Builder::new(TEXT_OVERLAY_FORMAT, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("UI Text Overlay")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let text_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let text_descriptor_set = context.create_descriptor_set(Some("UI Text"), &text_descriptor_set_template);
		context.create_descriptor_binding(
			text_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&TEXT_OVERLAY_BINDING,
				text_overlay,
				text_sampler,
				ghi::Layouts::Read,
			),
		);

		Self {
			pipeline,
			vertex_buffer,
			index_buffer,
			text_pipeline,
			text_descriptor_set,
			text_overlay: text_overlay.into(),
			main_attachment: main_attachment.into(),
			data: UiDrawList::default(),
			reported_capacity_limit: false,
			text_system: TextSystem::new(),
			text_overlay_has_been_used: false,
		}
	}

	pub fn update(&mut self, render: engine::Render) {
		update_from_render(&render, &mut self.data);
	}
}

impl RenderPass for UiRenderPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extent = sink.extent();
		let geometry = build_ui_geometry(&self.data, extent, frame_allocator);
		let has_rectangle_batches = !geometry.batches.is_empty();

		if geometry.truncated && !self.reported_capacity_limit {
			log::warn!(
				"UI geometry capacity exceeded. The most likely cause is that the UI contains more than {MAX_UI_ELEMENTS} drawable elements in a single frame."
			);
			self.reported_capacity_limit = true;
		} else if !geometry.truncated {
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

		let mut draw_text_overlay = false;

		if !self.data.texts.is_empty() {
			assert!(
				extent.width() > 0 && extent.height() > 0,
				"UI text overlay resize requires a non-zero viewport extent. The most likely cause is that text rendering ran before swapchain extent validation."
			);

			frame.resize_image(self.text_overlay, Extent::rectangle(extent.width(), extent.height()));

			let overlay = frame.get_texture_slice_mut(self.text_overlay);
			let drew_text = rasterize_text_overlay(&self.data, extent, &mut self.text_system, overlay);

			if drew_text || self.text_overlay_has_been_used {
				frame.sync_texture(self.text_overlay);
				draw_text_overlay = true;
			}
			self.text_overlay_has_been_used |= drew_text;
		} else if self.text_overlay_has_been_used && extent.width() > 0 && extent.height() > 0 {
			frame.resize_image(
				self.text_overlay,
				Extent::rectangle(extent.width().max(1), extent.height().max(1)),
			);
			frame.get_texture_slice_mut(self.text_overlay).fill(0);
			frame.sync_texture(self.text_overlay);
			draw_text_overlay = true;
		}

		if !has_rectangle_batches && !draw_text_overlay {
			return None;
		}

		let pipeline = self.pipeline;
		let vertex_buffer = self.vertex_buffer;
		let index_buffer = self.index_buffer;
		let text_pipeline = self.text_pipeline;
		let text_descriptor_set = self.text_descriptor_set;
		let main_attachment = self.main_attachment;
		let batches: &'a [UiDrawBatch] = frame_allocator.alloc_slice_copy(&geometry.batches);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(|label| label.write_str("UI"), |command_buffer| {
				assert!(
					!draw_text_overlay || extent.width() > 0 && extent.height() > 0,
					"UI text overlay render pass requires a non-zero attachment extent. The most likely cause is that a stale prepared UI pass survived a viewport resize or minimization."
				);

				let mut needs_clear = true;

				if !batches.is_empty() {
					let attachments = [ghi::AttachmentInformation::new(
						main_attachment,
						ghi::Layouts::RenderTarget,
						ghi::ClearValue::None,
						false,
						true,
					)];

					needs_clear = false;

					command_buffer.bind_vertex_buffers(&[vertex_buffer.into()]);
					command_buffer.bind_index_buffer(&(Into::<ghi::BufferDescriptor>::into(index_buffer).index_type(ghi::DataTypes::U16)));

					let command_buffer = command_buffer.start_render_pass(extent, &attachments);
					let command_buffer = command_buffer.bind_raster_pipeline(pipeline);

					for batch in batches {
						command_buffer.draw_indexed(batch.index_count, 1, batch.first_index, batch.vertex_offset, 0);
					}

					command_buffer.end_render_pass();
				}

				if draw_text_overlay {
					let attachments = [ghi::AttachmentInformation::new(
						main_attachment,
						ghi::Layouts::RenderTarget,
						ghi::ClearValue::None,
						!needs_clear,
						true,
					)];

					let command_buffer = command_buffer.start_render_pass(extent, &attachments);
					let command_buffer = command_buffer.bind_raster_pipeline(text_pipeline);
					command_buffer.bind_descriptor_sets(&[text_descriptor_set]);
					command_buffer.draw(3, 1, 0, 0);
					command_buffer.end_render_pass();
				}
			});
			},
		))
	}
}

/// Builds the UI vertex shader using BESL and compiles it to SPIR-V.
fn create_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	if ghi::implementation::USES_METAL {
		let shader_source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct UiVertexIn {
				float2 position [[attribute(0)]];
				float2 pixel_position [[attribute(1)]];
				float2 local_position [[attribute(2)]];
				float2 rect_size [[attribute(3)]];
				float4 color [[attribute(4)]];
				float corner_radius [[attribute(5)]];
				float corner_exponent [[attribute(6)]];
				float layer_kind [[attribute(7)]];
				float stroke_width [[attribute(8)]];
				float2 feather_mask_position [[attribute(9)]];
				float2 feather_mask_size [[attribute(10)]];
				float4 feather_mask_edges [[attribute(11)]];
				float2 feather_mask_corner [[attribute(12)]];
			};

			struct UiVertexOut {
				float4 position [[position]];
				float4 color;
				float2 pixel_position;
				float2 local_position;
				float2 rect_size;
				float corner_radius;
				float corner_exponent;
				float layer_kind;
				float stroke_width;
				float2 feather_mask_position;
				float2 feather_mask_size;
				float4 feather_mask_edges;
				float2 feather_mask_corner;
			};

			vertex UiVertexOut ui_vertex_main(UiVertexIn in [[stage_in]]) {
				UiVertexOut out;
				out.position = float4(in.position, 0.0, 1.0);
				out.color = in.color;
				out.pixel_position = in.pixel_position;
				out.local_position = in.local_position;
				out.rect_size = in.rect_size;
				out.corner_radius = in.corner_radius;
				out.corner_exponent = in.corner_exponent;
				out.layer_kind = in.layer_kind;
				out.stroke_width = in.stroke_width;
				out.feather_mask_position = in.feather_mask_position;
				out.feather_mask_size = in.feather_mask_size;
				out.feather_mask_edges = in.feather_mask_edges;
				out.feather_mask_corner = in.feather_mask_corner;
				return out;
			}
		"#;

		return context
			.create_shader(
				Some("UI Vertex Shader"),
				ghi::shader::Sources::MTL {
					source: shader_source,
					entry_point: "ui_vertex_main",
				},
				ghi::ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create the UI vertex shader. The most likely cause is an incompatible shader interface.");
	}

	let mut shader_generator = SPIRVShaderGenerator::new();
	let mut root = ParserNode::root();

	let main_code = r#"
		gl_Position = vec4(in_position, 0.0, 1.0);
		out_color = in_color;
		out_pixel_position = in_pixel_position;
		out_local_position = in_local_position;
		out_rect_size = in_rect_size;
		out_corner_radius = in_corner_radius;
		out_corner_exponent = in_corner_exponent;
		out_layer_kind = in_layer_kind;
		out_stroke_width = in_stroke_width;
		out_feather_mask_position = in_feather_mask_position;
		out_feather_mask_size = in_feather_mask_size;
		out_feather_mask_edges = in_feather_mask_edges;
		out_feather_mask_corner = in_feather_mask_corner;
	"#
	.trim();

	let main = ParserNode::main_function(vec![ParserNode::glsl(
		main_code,
		&[
			"in_position",
			"in_pixel_position",
			"in_local_position",
			"in_rect_size",
			"in_color",
			"in_corner_radius",
			"in_corner_exponent",
			"in_layer_kind",
			"in_stroke_width",
			"out_color",
			"out_pixel_position",
			"out_local_position",
			"out_rect_size",
			"out_corner_radius",
			"out_corner_exponent",
			"out_layer_kind",
			"out_stroke_width",
			"out_feather_mask_position",
			"out_feather_mask_size",
			"out_feather_mask_edges",
			"out_feather_mask_corner",
			"in_feather_mask_position",
			"in_feather_mask_size",
			"in_feather_mask_edges",
			"in_feather_mask_corner",
		],
		&[],
	)]);
	let position_input = ParserNode::input("in_position", "vec2f", 0);
	let pixel_position_input = ParserNode::input("in_pixel_position", "vec2f", 1);
	let local_position_input = ParserNode::input("in_local_position", "vec2f", 2);
	let rect_size_input = ParserNode::input("in_rect_size", "vec2f", 3);
	let color_input = ParserNode::input("in_color", "vec4f", 4);
	let corner_radius_input = ParserNode::input("in_corner_radius", "f32", 5);
	let corner_exponent_input = ParserNode::input("in_corner_exponent", "f32", 6);
	let layer_kind_input = ParserNode::input("in_layer_kind", "f32", 7);
	let stroke_width_input = ParserNode::input("in_stroke_width", "f32", 8);
	let feather_mask_position_input = ParserNode::input("in_feather_mask_position", "vec2f", 9);
	let feather_mask_size_input = ParserNode::input("in_feather_mask_size", "vec2f", 10);
	let feather_mask_edges_input = ParserNode::input("in_feather_mask_edges", "vec4f", 11);
	let feather_mask_corner_input = ParserNode::input("in_feather_mask_corner", "vec2f", 12);
	let color_output = ParserNode::output("out_color", "vec4f", 0);
	let pixel_position_output = ParserNode::output("out_pixel_position", "vec2f", 1);
	let local_position_output = ParserNode::output("out_local_position", "vec2f", 2);
	let rect_size_output = ParserNode::output("out_rect_size", "vec2f", 3);
	let corner_radius_output = ParserNode::output("out_corner_radius", "f32", 4);
	let corner_exponent_output = ParserNode::output("out_corner_exponent", "f32", 5);
	let layer_kind_output = ParserNode::output("out_layer_kind", "f32", 6);
	let stroke_width_output = ParserNode::output("out_stroke_width", "f32", 7);
	let feather_mask_position_output = ParserNode::output("out_feather_mask_position", "vec2f", 8);
	let feather_mask_size_output = ParserNode::output("out_feather_mask_size", "vec2f", 9);
	let feather_mask_edges_output = ParserNode::output("out_feather_mask_edges", "vec4f", 10);
	let feather_mask_corner_output = ParserNode::output("out_feather_mask_corner", "vec2f", 11);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			position_input,
			pixel_position_input,
			local_position_input,
			rect_size_input,
			color_input,
			corner_radius_input,
			corner_exponent_input,
			layer_kind_input,
			stroke_width_input,
			feather_mask_position_input,
			feather_mask_size_input,
			feather_mask_edges_input,
			feather_mask_corner_input,
			color_output,
			pixel_position_output,
			local_position_output,
			rect_size_output,
			corner_radius_output,
			corner_exponent_output,
			layer_kind_output,
			stroke_width_output,
			feather_mask_position_output,
			feather_mask_size_output,
			feather_mask_edges_output,
			feather_mask_corner_output,
			main,
		],
	);
	root.add(vec![CommonShaderScope::new(), shader_scope]);

	let root_node = besl::lex(root).expect("Failed to lex the UI vertex shader. The most likely cause is invalid BESL syntax.");
	let main_node = root_node.get_main().expect(
		"Failed to find the UI vertex entry point. The most likely cause is that the shader main function was not generated.",
	);
	let generated = shader_generator
		.generate(&ShaderGenerationSettings::vertex(), &main_node)
		.expect("Failed to generate UI vertex shader SPIR-V. The most likely cause is invalid GLSL emitted from BESL.");

	context
		.create_shader(
			Some("UI Vertex Shader"),
			ghi::shader::Sources::SPIRV(generated.binary()),
			ghi::ShaderTypes::Vertex,
			generated
				.bindings()
				.iter()
				.map(map_shader_binding_to_shader_binding_descriptor),
		)
		.expect("Failed to create the UI vertex shader. The most likely cause is an incompatible shader interface.")
}

/// Builds the UI fragment shader using BESL and compiles it to SPIR-V.
fn create_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	if ghi::implementation::USES_METAL {
		return context
			.create_shader(
				Some("UI Fragment Shader"),
				ghi::shader::Sources::MTL {
					source: UI_FRAGMENT_SHADER_MSL,
					entry_point: "ui_fragment_main",
				},
				ghi::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create the UI fragment shader. The most likely cause is an incompatible shader interface.");
	}

	let mut shader_generator = SPIRVShaderGenerator::new();
	let mut root = ParserNode::root();

	let main = ParserNode::main_function(vec![ParserNode::glsl(
		UI_FRAGMENT_SHADER_GLSL_MAIN,
		&[
			"in_color",
			"in_pixel_position",
			"in_local_position",
			"in_rect_size",
			"in_corner_radius",
			"in_corner_exponent",
			"in_layer_kind",
			"in_stroke_width",
			"in_feather_mask_position",
			"in_feather_mask_size",
			"in_feather_mask_edges",
			"in_feather_mask_corner",
			"out_color_attachment",
		],
		&[],
	)]);
	let input_color = ParserNode::input("in_color", "vec4f", 0);
	let input_pixel_position = ParserNode::input("in_pixel_position", "vec2f", 1);
	let input_local_position = ParserNode::input("in_local_position", "vec2f", 2);
	let input_rect_size = ParserNode::input("in_rect_size", "vec2f", 3);
	let input_corner_radius = ParserNode::input("in_corner_radius", "f32", 4);
	let input_corner_exponent = ParserNode::input("in_corner_exponent", "f32", 5);
	let input_layer_kind = ParserNode::input("in_layer_kind", "f32", 6);
	let input_stroke_width = ParserNode::input("in_stroke_width", "f32", 7);
	let input_feather_mask_position = ParserNode::input("in_feather_mask_position", "vec2f", 8);
	let input_feather_mask_size = ParserNode::input("in_feather_mask_size", "vec2f", 9);
	let input_feather_mask_edges = ParserNode::input("in_feather_mask_edges", "vec4f", 10);
	let input_feather_mask_corner = ParserNode::input("in_feather_mask_corner", "vec2f", 11);
	let output_color = ParserNode::output("out_color_attachment", "vec4f", 0);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			input_color,
			input_pixel_position,
			input_local_position,
			input_rect_size,
			input_corner_radius,
			input_corner_exponent,
			input_layer_kind,
			input_stroke_width,
			input_feather_mask_position,
			input_feather_mask_size,
			input_feather_mask_edges,
			input_feather_mask_corner,
			output_color,
			main,
		],
	);
	root.add(vec![CommonShaderScope::new(), shader_scope]);

	let root_node =
		besl::lex(root).expect("Failed to lex the UI fragment shader. The most likely cause is invalid BESL syntax.");
	let main_node = root_node.get_main().expect(
		"Failed to find the UI fragment entry point. The most likely cause is that the shader main function was not generated.",
	);
	let generated = shader_generator
		.generate(&ShaderGenerationSettings::fragment(), &main_node)
		.expect("Failed to generate UI fragment shader SPIR-V. The most likely cause is invalid GLSL emitted from BESL.");

	context
		.create_shader(
			Some("UI Fragment Shader"),
			ghi::shader::Sources::SPIRV(generated.binary()),
			ghi::ShaderTypes::Fragment,
			generated
				.bindings()
				.iter()
				.map(map_shader_binding_to_shader_binding_descriptor),
		)
		.expect("Failed to create the UI fragment shader. The most likely cause is an incompatible shader interface.")
}

const UI_FRAGMENT_SHADER_GLSL_MAIN: &str = r#"
vec2 half_size = in_rect_size * 0.5;
float corner_radius = min(in_corner_radius, min(half_size.x, half_size.y));
float corner_exponent = in_corner_exponent;
vec2 centered_position = in_local_position - half_size;
vec2 rounded_extent = half_size - vec2(corner_radius);
vec2 corner_delta = abs(centered_position) - rounded_extent;
vec2 abs_corner = max(corner_delta, vec2(0.0));
float corner_sum = pow(abs_corner.x, corner_exponent) + pow(abs_corner.y, corner_exponent);
float corner_distance = pow(corner_sum, 1.0 / corner_exponent);
float field_distance = corner_distance + min(max(corner_delta.x, corner_delta.y), 0.0) - corner_radius;
float edge_width = max(fwidth(field_distance), 1.0);
float rounded_shape = step(0.0001, corner_radius);
float rounded_fill_coverage = 1.0 - smoothstep(-edge_width, edge_width, field_distance);
float fill_coverage = mix(1.0, rounded_fill_coverage, rounded_shape);

float corner_gradient_scale = pow(max(corner_sum, 0.0001), (1.0 / corner_exponent) - 1.0);
vec2 corner_gradient = vec2(
	pow(abs_corner.x, corner_exponent - 1.0) * corner_gradient_scale,
	pow(abs_corner.y, corner_exponent - 1.0) * corner_gradient_scale
);
float field_gradient_length = mix(1.0, max(length(corner_gradient), 0.0001), step(0.0001, corner_sum));
float signed_distance = field_distance / field_gradient_length;
float corrected_edge_width = max(fwidth(signed_distance), 1.0);
float inner_signed_distance = signed_distance + in_stroke_width;
float inner_coverage = 1.0 - smoothstep(-corrected_edge_width, corrected_edge_width, inner_signed_distance);
float stroke_coverage = max(fill_coverage - inner_coverage, 0.0);
float coverage = mix(fill_coverage, stroke_coverage, step(0.5, in_layer_kind));
float feather_top = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), in_pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
float feather_right = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - in_pixel_position.x), step(0.0001, in_feather_mask_edges.y));
float feather_bottom = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - in_pixel_position.y), step(0.0001, in_feather_mask_edges.z));
float feather_left = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), in_pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
vec2 feather_half_size = in_feather_mask_size * 0.5;
float feather_corner_radius = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
float feather_corner_exponent = in_feather_mask_corner.y;
vec2 feather_centered_position = in_pixel_position - in_feather_mask_position - feather_half_size;
vec2 feather_rounded_extent = feather_half_size - vec2(feather_corner_radius);
vec2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
vec2 feather_abs_corner = max(feather_corner_delta, vec2(0.0));
float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
float feather_mask_enabled = step(0.0001, min(in_feather_mask_size.x, in_feather_mask_size.y));
float feather_rounded_shape = step(0.0001, feather_corner_radius);
float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);

out_color_attachment = vec4(in_color.rgb, in_color.a * coverage * feather_coverage);
"#;

const UI_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct UiVertexOut {
	float4 position [[position]];
	float4 color;
	float2 pixel_position;
	float2 local_position;
	float2 rect_size;
	float corner_radius;
	float corner_exponent;
	float layer_kind;
	float stroke_width;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

fragment float4 ui_fragment_main(UiVertexOut in [[stage_in]]) {
	float2 half_size = in.rect_size * 0.5;
	float corner_radius = min(in.corner_radius, min(half_size.x, half_size.y));
	float corner_exponent = in.corner_exponent;
	float2 centered_position = in.local_position - half_size;
	float2 rounded_extent = half_size - float2(corner_radius);
	float2 corner_delta = abs(centered_position) - rounded_extent;
	float2 abs_corner = max(corner_delta, float2(0.0));
	float corner_sum = pow(abs_corner.x, corner_exponent) + pow(abs_corner.y, corner_exponent);
	float corner_distance = pow(corner_sum, 1.0 / corner_exponent);
	float field_distance = corner_distance + min(max(corner_delta.x, corner_delta.y), 0.0) - corner_radius;
	float edge_width = max(fwidth(field_distance), 1.0);
	float rounded_shape = step(0.0001, corner_radius);
	float rounded_fill_coverage = 1.0 - smoothstep(-edge_width, edge_width, field_distance);
	float fill_coverage = mix(1.0, rounded_fill_coverage, rounded_shape);

	float corner_gradient_scale = pow(max(corner_sum, 0.0001), (1.0 / corner_exponent) - 1.0);
	float2 corner_gradient = float2(
		pow(abs_corner.x, corner_exponent - 1.0) * corner_gradient_scale,
		pow(abs_corner.y, corner_exponent - 1.0) * corner_gradient_scale
	);
	float field_gradient_length = mix(1.0, max(length(corner_gradient), 0.0001), step(0.0001, corner_sum));
	float signed_distance = field_distance / field_gradient_length;
	float corrected_edge_width = max(fwidth(signed_distance), 1.0);
	float inner_signed_distance = signed_distance + in.stroke_width;
	float inner_coverage = 1.0 - smoothstep(-corrected_edge_width, corrected_edge_width, inner_signed_distance);
	float stroke_coverage = max(fill_coverage - inner_coverage, 0.0);
	float coverage = mix(fill_coverage, stroke_coverage, step(0.5, in.layer_kind));
	float feather_top = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.x, 0.0001), in.pixel_position.y - in.feather_mask_position.y), step(0.0001, in.feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.y, 0.0001), in.feather_mask_position.x + in.feather_mask_size.x - in.pixel_position.x), step(0.0001, in.feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.z, 0.0001), in.feather_mask_position.y + in.feather_mask_size.y - in.pixel_position.y), step(0.0001, in.feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.w, 0.0001), in.pixel_position.x - in.feather_mask_position.x), step(0.0001, in.feather_mask_edges.w));
	float2 feather_half_size = in.feather_mask_size * 0.5;
	float feather_corner_radius = min(in.feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in.feather_mask_corner.y;
	float2 feather_centered_position = in.pixel_position - in.feather_mask_position - feather_half_size;
	float2 feather_rounded_extent = feather_half_size - float2(feather_corner_radius);
	float2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
	float2 feather_abs_corner = max(feather_corner_delta, float2(0.0));
	float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	float feather_mask_enabled = step(0.0001, min(in.feather_mask_size.x, in.feather_mask_size.y));
	float feather_rounded_shape = step(0.0001, feather_corner_radius);
	float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
	float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	return float4(in.color.rgb, in.color.a * coverage * feather_coverage);
}
"#;

fn create_text_overlay_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Text Overlay Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: TEXT_OVERLAY_VERTEX_SHADER_GLSL,
			msl: TEXT_OVERLAY_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_text_overlay_vertex",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI text overlay vertex shader. The most likely cause is an incompatible shader interface.")
}

fn create_text_overlay_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Text Overlay Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: TEXT_OVERLAY_FRAGMENT_SHADER_GLSL,
			msl: TEXT_OVERLAY_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_text_overlay_fragment",
		},
		ghi::ShaderTypes::Fragment,
		[TEXT_OVERLAY_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ)],
	)
	.expect("Failed to create the UI text overlay fragment shader. The most likely cause is an incompatible shader interface.")
}

const TEXT_OVERLAY_VERTEX_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(vertex)

layout(location = 0) out vec2 out_uv;

void main() {
	vec2 positions[3] = vec2[](
		vec2(-1.0, -1.0),
		vec2(-1.0, 3.0),
		vec2(3.0, -1.0)
	);
	vec2 position = positions[gl_VertexIndex];
	gl_Position = vec4(position, 0.0, 1.0);
	out_uv = vec2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
}
"#;

const TEXT_OVERLAY_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct TextOverlayVertexOut {
	float4 position [[position]];
	float2 uv;
};

vertex TextOverlayVertexOut ui_text_overlay_vertex(uint vertex_id [[vertex_id]]) {
	float2 positions[3] = {
		float2(-1.0, -1.0),
		float2(-1.0, 3.0),
		float2(3.0, -1.0)
	};
	float2 position = positions[vertex_id];
	TextOverlayVertexOut out;
	out.position = float4(position, 0.0, 1.0);
	out.uv = float2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
	return out;
}
"#;

const TEXT_OVERLAY_FRAGMENT_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(fragment)

layout(set = 0, binding = 0) uniform sampler2D text_overlay;

layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color_attachment;

void main() {
	out_color_attachment = texture(text_overlay, in_uv);
}
"#;

const TEXT_OVERLAY_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct TextOverlayVertexOut {
	float4 position [[position]];
	float2 uv;
};

struct TextOverlaySet0 {
	texture2d<float> text_overlay [[id(0)]];
	sampler text_overlay_sampler [[id(1)]];
};

fragment float4 ui_text_overlay_fragment(
	TextOverlayVertexOut in [[stage_in]],
	constant TextOverlaySet0& set0 [[buffer(16)]]
) {
	return set0.text_overlay.sample(set0.text_overlay_sampler, in.uv);
}
"#;

#[cfg(test)]
mod tests {
	use utils::{Extent, RGBA};

	use super::{
		build_ui_geometry, should_rasterize_text, update_from_render, DrawClip, DrawFeatherMask, UiDrawBatch, UiDrawElement,
		UiDrawList, UiTextDrawElement, MAX_UI_ELEMENTS, MAX_UI_VERTICES_PER_DRAW, UI_FRAGMENT_SHADER_GLSL_MAIN,
		UI_FRAGMENT_SHADER_MSL, UI_INDICES_PER_ELEMENT, UI_VERTICES_PER_ELEMENT,
	};
	use crate::ui::{
		flow::Size,
		layout::{
			context::{Context, ElementContext},
			engine::Engine,
		},
		style::{ConcreteLayer, ConcreteStyle, LayerKind},
		Container, Text,
	};

	fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
		assert!((actual[0] - expected[0]).abs() < 0.0001);
		assert!((actual[1] - expected[1]).abs() < 0.0001);
	}

	fn draw_element(corner_radius: f32, corner_exponent: f32) -> UiDrawElement {
		UiDrawElement {
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

	#[test]
	fn builds_a_single_batched_quad() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![UiDrawElement {
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
	fn scales_corner_radius_to_viewport_pixels() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![draw_element(6.0, 2.0)],
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
	fn invalid_corner_exponents_resolve_to_round_corners() {
		for exponent in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5] {
			let frame_allocator = bumpalo::Bump::new();
			let geometry = build_ui_geometry(
				&UiDrawList {
					layout_size: [100.0, 100.0],
					elements: vec![draw_element(8.0, exponent)],
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
				texts: vec![],
			},
			Extent::rectangle(100, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices[0].corner_exponent, 8.0);
	}

	#[test]
	fn rounded_rect_glsl_shader_uses_derivative_anti_aliasing() {
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("fwidth(field_distance)"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN
			.contains("rounded_fill_coverage = 1.0 - smoothstep(-edge_width, edge_width, field_distance)"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("fwidth(signed_distance)"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("in_color.a * coverage * feather_coverage"));
	}

	#[test]
	fn rounded_rect_msl_shader_uses_derivative_anti_aliasing() {
		assert!(UI_FRAGMENT_SHADER_MSL.contains("fwidth(field_distance)"));
		assert!(UI_FRAGMENT_SHADER_MSL
			.contains("rounded_fill_coverage = 1.0 - smoothstep(-edge_width, edge_width, field_distance)"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("fwidth(signed_distance)"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("in.color.a * coverage * feather_coverage"));
	}

	#[test]
	fn rounded_rect_shaders_apply_feather_mask_coverage() {
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("feather_top"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("feather_right"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("feather_bottom"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("feather_left"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("feather_shape_coverage"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("feather_top"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("feather_right"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("feather_bottom"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("feather_left"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("feather_shape_coverage"));
	}

	#[test]
	fn square_fill_layers_do_not_antialias_shared_edges() {
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("rounded_shape = step(0.0001, corner_radius)"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("fill_coverage = mix(1.0, rounded_fill_coverage, rounded_shape)"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("rounded_shape = step(0.0001, corner_radius)"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("fill_coverage = mix(1.0, rounded_fill_coverage, rounded_shape)"));
	}

	#[test]
	fn rounded_rect_shaders_use_superellipse_corner_distance() {
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("pow(abs_corner.x, corner_exponent)"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("1.0 / corner_exponent"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("pow(abs_corner.x, corner_exponent)"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("1.0 / corner_exponent"));
	}

	#[test]
	fn rounded_rect_shaders_support_stroke_band_coverage() {
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("field_gradient_length"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("corner_gradient"));
		assert!(
			UI_FRAGMENT_SHADER_GLSL_MAIN.contains("mix(1.0, max(length(corner_gradient), 0.0001), step(0.0001, corner_sum))")
		);
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("field_distance / field_gradient_length"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("signed_distance + in_stroke_width"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("fill_coverage - inner_coverage"));
		assert!(UI_FRAGMENT_SHADER_GLSL_MAIN.contains("step(0.5, in_layer_kind)"));
		assert!(!UI_FRAGMENT_SHADER_GLSL_MAIN.contains("gradient_sample"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("field_gradient_length"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("corner_gradient"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("mix(1.0, max(length(corner_gradient), 0.0001), step(0.0001, corner_sum))"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("field_distance / field_gradient_length"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("signed_distance + in.stroke_width"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("fill_coverage - inner_coverage"));
		assert!(UI_FRAGMENT_SHADER_MSL.contains("step(0.5, in.layer_kind)"));
		assert!(!UI_FRAGMENT_SHADER_MSL.contains("gradient_sample"));
	}

	#[test]
	fn splits_large_batches_to_stay_within_u16_indices() {
		let frame_allocator = bumpalo::Bump::new();
		let element_count = MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT + 1;
		let elements = (0..element_count)
			.map(|_| UiDrawElement {
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
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			clip: None,
			feather_mask: None,
			color: RGBA::new(1.0, 1.0, 1.0, 0.0),
			font_size: 16.0,
			text: "Hidden".to_string(),
		}));

		assert!(should_rasterize_text(&UiTextDrawElement {
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
}
