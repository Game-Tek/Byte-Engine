use std::{collections::HashMap, sync::Arc};

use besl::parser::Node as ParserNode;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent, RGBA};

use super::{
	element::ElementHandle as _,
	layout::{engine, FeatherMask, Geometry},
	style::{Color, EdgeFeather, LayerKind},
};
use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		shader_store::{ShaderSourceDefinition, ShaderSourceDescriptor},
		Sink,
	},
	ui::{
		components::curve::{CurvePoint, CurveSegment},
		font::TextSystem,
	},
};

const MAIN_ATTACHMENT_FORMAT: ghi::Formats = ghi::Formats::RGBA16UNORM;
const TEXT_OVERLAY_FORMAT: ghi::Formats = ghi::Formats::RGBA8UNORM;
const TEXT_OVERLAY_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);
const UI_IMAGE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);
const UI_BLUR_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const UI_BLUR_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const UI_BLUR_COMPOSITE_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);
const UI_BLUR_COMPOSITE_BLURRED_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);
const UI_BLUR_DOWNSCALE: u32 = 1;
const UI_BLUR_WORKGROUP_SIZE: u32 = 16;

const UI_VERTICES_PER_ELEMENT: usize = 4;
const UI_INDICES_PER_ELEMENT: usize = 6;
const UI_VERTICES_PER_CURVE_SPAN: usize = 4;
const UI_INDICES_PER_CURVE_SPAN: usize = 6;
const MAX_UI_VERTICES_PER_DRAW: usize = u16::MAX as usize + 1;
const MAX_UI_ELEMENTS: usize = 65_536;
const MAX_UI_IMAGES: usize = MAX_UI_ELEMENTS;
const MAX_UI_VERTICES: usize = MAX_UI_ELEMENTS * UI_VERTICES_PER_ELEMENT;
const MAX_UI_INDICES: usize = MAX_UI_ELEMENTS * UI_INDICES_PER_ELEMENT;
const CURVE_FLATTEN_TOLERANCE_PIXELS: f32 = 0.35;
const CURVE_AA_WIDTH_PIXELS: f32 = 1.0;

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
	depth: u32,
	order: u32,
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

#[derive(Debug, Clone, Copy)]
struct UiBlurDrawElement {
	depth: u32,
	order: u32,
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	color: [f32; 4],
	corner_radius: f32,
	corner_exponent: f32,
	radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct UiTextDrawElement {
	depth: u32,
	order: u32,
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	color: RGBA,
	font_size: f32,
	text: String,
}

#[derive(Debug, Clone)]
struct UiImageDrawElement {
	depth: u32,
	order: u32,
	image_id: u64,
	version: u64,
	source_width: u32,
	source_height: u32,
	pixels: Arc<[u8]>,
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	opacity: f32,
}

#[derive(Debug, Clone)]
struct UiCurveDrawElement {
	depth: u32,
	order: u32,
	position: [f32; 2],
	size: [f32; 2],
	clip: Option<DrawClip>,
	feather_mask: Option<DrawFeatherMask>,
	color: [f32; 4],
	stroke_width: f32,
	segments: Vec<CurveSegment>,
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
	blurs: Vec<UiBlurDrawElement>,
	curves: Vec<UiCurveDrawElement>,
	images: Vec<UiImageDrawElement>,
	texts: Vec<UiTextDrawElement>,
}

impl Default for UiDrawList {
	fn default() -> Self {
		Self {
			layout_size: [1.0, 1.0],
			elements: Vec::new(),
			blurs: Vec::new(),
			curves: Vec::new(),
			images: Vec::new(),
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

const UI_IMAGE_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 7] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("UV", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("OPACITY", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_EDGES", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_CORNER", ghi::DataTypes::Float2, 0),
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UiImageVertex {
	position: [f32; 2],
	uv: [f32; 2],
	opacity: f32,
	feather_mask_position: [f32; 2],
	feather_mask_size: [f32; 2],
	feather_mask_edges: [f32; 4],
	feather_mask_corner: [f32; 2],
}

const UI_CURVE_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 10] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("PIXEL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("SEGMENT_FROM", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("SEGMENT_TO", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("COLOR", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("HALF_WIDTH", ghi::DataTypes::Float, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_EDGES", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_CORNER", ghi::DataTypes::Float2, 0),
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UiCurveVertex {
	position: [f32; 2],
	pixel_position: [f32; 2],
	segment_from: [f32; 2],
	segment_to: [f32; 2],
	color: [f32; 4],
	half_width: f32,
	feather_mask_position: [f32; 2],
	feather_mask_size: [f32; 2],
	feather_mask_edges: [f32; 4],
	feather_mask_corner: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiDrawBatch {
	depth: u32,
	order: u32,
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiImageDrawBatch {
	depth: u32,
	order: u32,
	image_id: u64,
	version: u64,
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiCurveDrawBatch {
	depth: u32,
	order: u32,
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiPreparedImageBatch {
	descriptor_set: ghi::DescriptorSetHandle,
	batch: UiImageDrawBatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiPreparedTextBatch {
	depth: u32,
	order: u32,
	descriptor_set: ghi::DescriptorSetHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiPreparedBlurBatch {
	depth: u32,
	order: u32,
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
	radius_pixels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiPreparedBatch {
	Rect(UiDrawBatch),
	Curve(UiCurveDrawBatch),
	Image(UiPreparedImageBatch),
	Text(UiPreparedTextBatch),
	Blur(UiPreparedBlurBatch),
}

impl UiPreparedBatch {
	fn depth(self) -> u32 {
		match self {
			Self::Rect(batch) => batch.depth,
			Self::Curve(batch) => batch.depth,
			Self::Image(batch) => batch.batch.depth,
			Self::Text(batch) => batch.depth,
			Self::Blur(batch) => batch.depth,
		}
	}

	fn order(self) -> u32 {
		match self {
			Self::Rect(batch) => batch.order,
			Self::Curve(batch) => batch.order,
			Self::Image(batch) => batch.batch.order,
			Self::Text(batch) => batch.order,
			Self::Blur(batch) => batch.order,
		}
	}
}

fn sort_prepared_batches(batches: &mut [UiPreparedBatch]) {
	batches.sort_by_key(|batch| (batch.depth(), batch.order()));
}

#[derive(Debug)]
struct UiGeometry<'a> {
	vertices: Vec<UiVertex, &'a bumpalo::Bump>,
	indices: Vec<u16, &'a bumpalo::Bump>,
	batches: Vec<UiDrawBatch, &'a bumpalo::Bump>,
	truncated: bool,
}

#[derive(Debug)]
struct UiBlurGeometry<'a> {
	vertices: Vec<UiVertex, &'a bumpalo::Bump>,
	indices: Vec<u16, &'a bumpalo::Bump>,
	batches: Vec<UiPreparedBlurBatch, &'a bumpalo::Bump>,
	truncated: bool,
}

#[derive(Debug)]
struct UiImageGeometry<'a> {
	vertices: Vec<UiImageVertex, &'a bumpalo::Bump>,
	indices: Vec<u16, &'a bumpalo::Bump>,
	batches: Vec<UiImageDrawBatch, &'a bumpalo::Bump>,
	truncated: bool,
}

#[derive(Debug)]
struct UiCurveGeometry<'a> {
	vertices: Vec<UiCurveVertex, &'a bumpalo::Bump>,
	indices: Vec<u16, &'a bumpalo::Bump>,
	batches: Vec<UiCurveDrawBatch, &'a bumpalo::Bump>,
	truncated: bool,
}

struct UiImageTexture {
	version: u64,
	extent: (u32, u32),
	image: ghi::BaseImageHandle,
	descriptor_set: ghi::DescriptorSetHandle,
}

struct UiTextOverlayTexture {
	image: ghi::BaseImageHandle,
	descriptor_set: ghi::DescriptorSetHandle,
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

fn backdrop_blur_radius(radius: f32) -> f32 {
	if radius.is_finite() {
		radius.clamp(0.0, 64.0)
	} else {
		0.0
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
	draw_list.blurs.clear();
	draw_list.curves.clear();
	draw_list.images.clear();
	draw_list.texts.clear();

	for element in render.elements() {
		let position = element.position;
		let size = element.size;

		for layer in element.style.layers() {
			if matches!(layer.kind, LayerKind::Fill) && layer.backdrop_blur_radius > 0.0 {
				continue;
			}
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
				depth: position.z(),
				order: element.id,
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

		let radius = backdrop_blur_radius(element.backdrop_blur_radius);
		if radius > 0.0 {
			let mut color = element
				.style
				.layers()
				.iter()
				.find(|layer| matches!(layer.kind, LayerKind::Fill) && layer.backdrop_blur_radius > 0.0)
				.map(|layer| match &layer.color {
					Color::Value(rgba) => *rgba,
					Color::Sample(_) => RGBA::white(),
				})
				.unwrap_or_else(RGBA::transparent);
			color.a *= element.opacity;
			draw_list.blurs.push(UiBlurDrawElement {
				depth: position.z(),
				order: element.id,
				position: [position.x() as f32, position.y() as f32],
				size: [size.x() as f32, size.y() as f32],
				clip: draw_clip_from_geometry(element.clip),
				feather_mask: draw_feather_mask_from_layout(element.feather_mask),
				color: color.into(),
				corner_radius: element.corner_radius,
				corner_exponent: element.corner_exponent,
				radius,
			});
		}
	}

	for curve in render.curves() {
		let position = curve.position;
		let size = curve.size;

		for layer in curve.style.layers() {
			let stroke_width = stroke_width(layer.kind);
			if !matches!(layer.kind, LayerKind::Stroke { .. }) || stroke_width <= 0.0 {
				continue;
			}

			let mut color = match &layer.color {
				Color::Value(rgba) => *rgba,
				Color::Sample(_) => RGBA::white(),
			};
			color.a *= curve.opacity;
			if color.a <= 0.0 {
				continue;
			}

			draw_list.curves.push(UiCurveDrawElement {
				depth: position.z(),
				order: curve.id,
				position: [position.x() as f32, position.y() as f32],
				size: [size.x() as f32, size.y() as f32],
				clip: draw_clip_from_geometry(curve.clip),
				feather_mask: draw_feather_mask_from_layout(curve.feather_mask),
				color: color.into(),
				stroke_width,
				segments: curve.segments.clone(),
			});
		}
	}

	for image in render.images() {
		draw_list.images.push(UiImageDrawElement {
			depth: image.position.z(),
			order: image.id,
			image_id: image.image_id,
			version: image.version,
			source_width: image.source_width,
			source_height: image.source_height,
			pixels: Arc::clone(&image.pixels),
			position: [image.position.x() as f32, image.position.y() as f32],
			size: [image.size.x() as f32, image.size.y() as f32],
			clip: draw_clip_from_geometry(image.clip),
			feather_mask: draw_feather_mask_from_layout(image.feather_mask),
			opacity: image.opacity,
		});
	}

	for text in render.texts() {
		let mut color = text.color;
		color.a *= text.opacity;
		let text = UiTextDrawElement {
			depth: text.position.z(),
			order: text.id,
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

fn should_draw_image(image: &UiImageDrawElement) -> bool {
	image.source_width > 0
		&& image.source_height > 0
		&& image.pixels.len() == image.source_width as usize * image.source_height as usize * 4
		&& image.size[0] > 0.0
		&& image.size[1] > 0.0
		&& image.opacity > 0.0
}

/// Rasterizes all visible text elements into the UI overlay texture for the current viewport.
fn rasterize_text_overlay(
	texts: &[UiTextDrawElement],
	layout_size: [f32; 2],
	viewport: Extent,
	text_system: &mut TextSystem,
	target: &mut [u8],
) -> bool {
	let viewport_width = viewport.width().max(1);
	let viewport_height = viewport.height().max(1);

	target.fill(0);

	if texts.is_empty() {
		return false;
	}

	let sx = viewport_width as f32 / layout_size[0].max(1.0);
	let sy = viewport_height as f32 / layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let mut drew_text = false;

	for text in texts {
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
	let mut batch_depth = 0u32;
	let mut batch_order = 0u32;

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

		if batch_index_count > 0
			&& (batch_vertex_count + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES_PER_DRAW || batch_depth != element.depth)
		{
			geometry.batches.push(UiDrawBatch {
				depth: batch_depth,
				order: batch_order,
				index_count: batch_index_count as u32,
				first_index: batch_first_index as u32,
				vertex_offset: batch_vertex_offset as i32,
			});

			batch_first_index = geometry.indices.len();
			batch_vertex_offset = geometry.vertices.len();
			batch_vertex_count = 0;
			batch_index_count = 0;
		}

		if batch_index_count == 0 {
			batch_depth = element.depth;
			batch_order = element.order;
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
			depth: batch_depth,
			order: batch_order,
			index_count: batch_index_count as u32,
			first_index: batch_first_index as u32,
			vertex_offset: batch_vertex_offset as i32,
		});
	}

	geometry
}

fn build_ui_blur_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
) -> UiBlurGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let radius_scale = sx.min(sy);

	let mut geometry = UiBlurGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.blurs.len().min(MAX_UI_ELEMENTS) * UI_VERTICES_PER_ELEMENT,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.blurs.len().min(MAX_UI_ELEMENTS) * UI_INDICES_PER_ELEMENT,
			frame_allocator,
		),
		batches: Vec::new_in(frame_allocator),
		truncated: false,
	};

	for blur in &draw_list.blurs {
		let rect_width = (blur.size[0] * sx).max(0.0);
		let rect_height = (blur.size[1] * sy).max(0.0);
		if rect_width <= 0.0 || rect_height <= 0.0 || blur.radius <= 0.0 {
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		let original_x0 = blur.position[0] * sx;
		let original_y0 = blur.position[1] * sy;
		let original_x1 = original_x0 + rect_width;
		let original_y1 = original_y0 + rect_height;
		let (x0, y0, x1, y1) = match blur.clip {
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
		let corner_radius = resolved_corner_radius(blur.corner_radius * radius_scale, rect_width, rect_height);
		let corner_exponent = resolved_corner_exponent(blur.corner_exponent);
		let feather_mask = scaled_feather_mask(blur.feather_mask, sx, sy);
		let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
		let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;
		let first_index = geometry.indices.len() as u32;
		let vertex_offset = geometry.vertices.len() as i32;
		let base_vertex = 0u16;

		geometry.vertices.extend_from_slice(&[
			UiVertex {
				position: [to_clip_x(x0), to_clip_y(y0)],
				pixel_position: [x0, y0],
				local_position: [local_x0, local_y0],
				rect_size: [rect_width, rect_height],
				color: blur.color,
				corner_radius,
				corner_exponent,
				layer_kind: 0.0,
				stroke_width: 0.0,
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
				color: blur.color,
				corner_radius,
				corner_exponent,
				layer_kind: 0.0,
				stroke_width: 0.0,
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
				color: blur.color,
				corner_radius,
				corner_exponent,
				layer_kind: 0.0,
				stroke_width: 0.0,
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
				color: blur.color,
				corner_radius,
				corner_exponent,
				layer_kind: 0.0,
				stroke_width: 0.0,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
		]);
		geometry.indices.extend_from_slice(&[
			base_vertex,
			base_vertex + 1,
			base_vertex + 2,
			base_vertex + 2,
			base_vertex + 3,
			base_vertex,
		]);
		geometry.batches.push(UiPreparedBlurBatch {
			depth: blur.depth,
			order: blur.order,
			index_count: UI_INDICES_PER_ELEMENT as u32,
			first_index,
			vertex_offset,
			radius_pixels: (blur.radius * radius_scale / UI_BLUR_DOWNSCALE as f32)
				.round()
				.clamp(1.0, 64.0) as u32,
		});
	}

	geometry
}

fn build_ui_curve_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
) -> UiCurveGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let stroke_scale = sx.min(sy);

	let mut geometry = UiCurveGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.curves.len().min(MAX_UI_ELEMENTS) * UI_VERTICES_PER_CURVE_SPAN,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.curves.len().min(MAX_UI_ELEMENTS) * UI_INDICES_PER_CURVE_SPAN,
			frame_allocator,
		),
		batches: Vec::new_in(frame_allocator),
		truncated: false,
	};

	let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
	let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;
	let mut points = Vec::new_in(frame_allocator);

	for curve in &draw_list.curves {
		let stroke_width = curve.stroke_width * stroke_scale;
		if curve.color[3] <= 0.0 || !stroke_width.is_finite() || stroke_width <= 0.0 {
			continue;
		}

		let half_width = stroke_width * 0.5;
		let expansion = half_width + CURVE_AA_WIDTH_PIXELS;
		let feather_mask = scaled_feather_mask(curve.feather_mask, sx, sy);
		let first_index = geometry.indices.len();
		let vertex_offset = geometry.vertices.len();
		let mut emitted_indices = 0usize;

		for segment in &curve.segments {
			points.clear();
			flatten_curve_segment(segment, curve.position, sx, sy, CURVE_FLATTEN_TOLERANCE_PIXELS, &mut points);

			for span in points.windows(2) {
				let mut from = span[0];
				let mut to = span[1];
				if !clip_curve_span(&mut from, &mut to, curve.clip, sx, sy) {
					continue;
				}
				let dx = to.x - from.x;
				let dy = to.y - from.y;
				let length = dx.hypot(dy);
				if !length.is_finite() || length <= 0.0001 {
					continue;
				}

				if geometry.vertices.len() + UI_VERTICES_PER_CURVE_SPAN > MAX_UI_VERTICES
					|| geometry.indices.len() + UI_INDICES_PER_CURVE_SPAN > MAX_UI_INDICES
				{
					geometry.truncated = true;
					break;
				}

				let tangent = [dx / length, dy / length];
				let normal = [-tangent[1], tangent[0]];
				let corners = [
					[
						from.x - tangent[0] * expansion - normal[0] * expansion,
						from.y - tangent[1] * expansion - normal[1] * expansion,
					],
					[
						to.x + tangent[0] * expansion - normal[0] * expansion,
						to.y + tangent[1] * expansion - normal[1] * expansion,
					],
					[
						to.x + tangent[0] * expansion + normal[0] * expansion,
						to.y + tangent[1] * expansion + normal[1] * expansion,
					],
					[
						from.x - tangent[0] * expansion + normal[0] * expansion,
						from.y - tangent[1] * expansion + normal[1] * expansion,
					],
				];

				let base_vertex = (geometry.vertices.len() - vertex_offset) as u16;
				for corner in corners {
					geometry.vertices.push(UiCurveVertex {
						position: [to_clip_x(corner[0]), to_clip_y(corner[1])],
						pixel_position: corner,
						segment_from: [from.x, from.y],
						segment_to: [to.x, to.y],
						color: curve.color,
						half_width,
						feather_mask_position: feather_mask.position,
						feather_mask_size: feather_mask.size,
						feather_mask_edges: feather_mask.edges,
						feather_mask_corner: feather_mask.corner,
					});
				}
				geometry.indices.extend_from_slice(&[
					base_vertex,
					base_vertex + 1,
					base_vertex + 2,
					base_vertex + 2,
					base_vertex + 3,
					base_vertex,
				]);
				emitted_indices += UI_INDICES_PER_CURVE_SPAN;
			}

			if geometry.truncated {
				break;
			}
		}

		if emitted_indices > 0 {
			geometry.batches.push(UiCurveDrawBatch {
				depth: curve.depth,
				order: curve.order,
				index_count: emitted_indices as u32,
				first_index: first_index as u32,
				vertex_offset: vertex_offset as i32,
			});
		}

		if geometry.truncated {
			break;
		}
	}

	geometry
}

fn flatten_curve_segment(
	segment: &CurveSegment,
	origin: [f32; 2],
	sx: f32,
	sy: f32,
	tolerance: f32,
	points: &mut Vec<CurvePoint, &bumpalo::Bump>,
) {
	match *segment {
		CurveSegment::Line { from, to } => {
			push_scaled_point(points, from, origin, sx, sy);
			push_scaled_point(points, to, origin, sx, sy);
		}
		CurveSegment::Quadratic { from, control, to } => {
			let from = scaled_curve_point(from, origin, sx, sy);
			let control = scaled_curve_point(control, origin, sx, sy);
			let to = scaled_curve_point(to, origin, sx, sy);
			if from.is_finite() && control.is_finite() && to.is_finite() {
				points.push(from);
				flatten_quadratic(from, control, to, tolerance, 0, points);
			}
		}
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		} => {
			let from = scaled_curve_point(from, origin, sx, sy);
			let control0 = scaled_curve_point(control0, origin, sx, sy);
			let control1 = scaled_curve_point(control1, origin, sx, sy);
			let to = scaled_curve_point(to, origin, sx, sy);
			if from.is_finite() && control0.is_finite() && control1.is_finite() && to.is_finite() {
				points.push(from);
				flatten_cubic(from, control0, control1, to, tolerance, 0, points);
			}
		}
	}
}

fn push_scaled_point(points: &mut Vec<CurvePoint, &bumpalo::Bump>, point: CurvePoint, origin: [f32; 2], sx: f32, sy: f32) {
	let point = scaled_curve_point(point, origin, sx, sy);
	if point.is_finite() {
		points.push(point);
	}
}

fn scaled_curve_point(point: CurvePoint, origin: [f32; 2], sx: f32, sy: f32) -> CurvePoint {
	CurvePoint::new((origin[0] + point.x) * sx, (origin[1] + point.y) * sy)
}

fn flatten_quadratic(
	from: CurvePoint,
	control: CurvePoint,
	to: CurvePoint,
	tolerance: f32,
	depth: u32,
	points: &mut Vec<CurvePoint, &bumpalo::Bump>,
) {
	if depth >= 12 || point_line_distance(control, from, to) <= tolerance {
		points.push(to);
		return;
	}

	let from_control = midpoint(from, control);
	let control_to = midpoint(control, to);
	let mid = midpoint(from_control, control_to);
	flatten_quadratic(from, from_control, mid, tolerance, depth + 1, points);
	flatten_quadratic(mid, control_to, to, tolerance, depth + 1, points);
}

fn flatten_cubic(
	from: CurvePoint,
	control0: CurvePoint,
	control1: CurvePoint,
	to: CurvePoint,
	tolerance: f32,
	depth: u32,
	points: &mut Vec<CurvePoint, &bumpalo::Bump>,
) {
	if depth >= 12 || point_line_distance(control0, from, to).max(point_line_distance(control1, from, to)) <= tolerance {
		points.push(to);
		return;
	}

	let p01 = midpoint(from, control0);
	let p12 = midpoint(control0, control1);
	let p23 = midpoint(control1, to);
	let p012 = midpoint(p01, p12);
	let p123 = midpoint(p12, p23);
	let mid = midpoint(p012, p123);
	flatten_cubic(from, p01, p012, mid, tolerance, depth + 1, points);
	flatten_cubic(mid, p123, p23, to, tolerance, depth + 1, points);
}

fn midpoint(a: CurvePoint, b: CurvePoint) -> CurvePoint {
	CurvePoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

fn point_line_distance(point: CurvePoint, from: CurvePoint, to: CurvePoint) -> f32 {
	let dx = to.x - from.x;
	let dy = to.y - from.y;
	let length = dx.hypot(dy);
	if length <= 0.0001 {
		return (point.x - from.x).hypot(point.y - from.y);
	}
	((point.x - from.x) * dy - (point.y - from.y) * dx).abs() / length
}

fn clip_curve_span(from: &mut CurvePoint, to: &mut CurvePoint, clip: Option<DrawClip>, sx: f32, sy: f32) -> bool {
	let Some(clip) = clip else {
		return true;
	};

	let x_min = clip.position[0] * sx;
	let y_min = clip.position[1] * sy;
	let x_max = x_min + clip.size[0] * sx;
	let y_max = y_min + clip.size[1] * sy;
	let dx = to.x - from.x;
	let dy = to.y - from.y;
	let mut t0 = 0.0;
	let mut t1 = 1.0;

	if !clip_line_axis(-dx, from.x - x_min, &mut t0, &mut t1)
		|| !clip_line_axis(dx, x_max - from.x, &mut t0, &mut t1)
		|| !clip_line_axis(-dy, from.y - y_min, &mut t0, &mut t1)
		|| !clip_line_axis(dy, y_max - from.y, &mut t0, &mut t1)
	{
		return false;
	}

	let original_from = *from;
	if t1 < 1.0 {
		*to = CurvePoint::new(original_from.x + dx * t1, original_from.y + dy * t1);
	}
	if t0 > 0.0 {
		*from = CurvePoint::new(original_from.x + dx * t0, original_from.y + dy * t0);
	}
	true
}

fn clip_line_axis(p: f32, q: f32, t0: &mut f32, t1: &mut f32) -> bool {
	if p == 0.0 {
		return q >= 0.0;
	}
	let r = q / p;
	if p < 0.0 {
		if r > *t1 {
			return false;
		}
		if r > *t0 {
			*t0 = r;
		}
	} else {
		if r < *t0 {
			return false;
		}
		if r < *t1 {
			*t1 = r;
		}
	}
	true
}

fn build_ui_image_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
) -> UiImageGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);

	let mut geometry = UiImageGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.images.len().min(MAX_UI_IMAGES) * UI_VERTICES_PER_ELEMENT,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.images.len().min(MAX_UI_IMAGES) * UI_INDICES_PER_ELEMENT,
			frame_allocator,
		),
		batches: Vec::new_in(frame_allocator),
		truncated: false,
	};

	for image in &draw_list.images {
		if !should_draw_image(image) {
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		let rect_width = image.size[0] * sx;
		let rect_height = image.size[1] * sy;
		let original_x0 = image.position[0] * sx;
		let original_y0 = image.position[1] * sy;
		let original_x1 = original_x0 + rect_width;
		let original_y1 = original_y0 + rect_height;
		let (x0, y0, x1, y1) = match image.clip {
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
		if x1 <= x0 || y1 <= y0 || rect_width <= 0.0 || rect_height <= 0.0 {
			continue;
		}

		let u0 = ((x0 - original_x0) / rect_width).clamp(0.0, 1.0);
		let v0 = ((y0 - original_y0) / rect_height).clamp(0.0, 1.0);
		let u1 = ((x1 - original_x0) / rect_width).clamp(0.0, 1.0);
		let v1 = ((y1 - original_y0) / rect_height).clamp(0.0, 1.0);
		let feather_mask = scaled_feather_mask(image.feather_mask, sx, sy);

		let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
		let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;

		let first_index = geometry.indices.len();
		let vertex_offset = geometry.vertices.len();
		geometry.vertices.extend_from_slice(&[
			UiImageVertex {
				position: [to_clip_x(x0), to_clip_y(y0)],
				uv: [u0, v0],
				opacity: image.opacity,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiImageVertex {
				position: [to_clip_x(x1), to_clip_y(y0)],
				uv: [u1, v0],
				opacity: image.opacity,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiImageVertex {
				position: [to_clip_x(x1), to_clip_y(y1)],
				uv: [u1, v1],
				opacity: image.opacity,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
			UiImageVertex {
				position: [to_clip_x(x0), to_clip_y(y1)],
				uv: [u0, v1],
				opacity: image.opacity,
				feather_mask_position: feather_mask.position,
				feather_mask_size: feather_mask.size,
				feather_mask_edges: feather_mask.edges,
				feather_mask_corner: feather_mask.corner,
			},
		]);

		geometry.indices.extend_from_slice(&[0, 1, 2, 2, 3, 0]);
		geometry.batches.push(UiImageDrawBatch {
			depth: image.depth,
			order: image.order,
			image_id: image.image_id,
			version: image.version,
			index_count: UI_INDICES_PER_ELEMENT as u32,
			first_index: first_index as u32,
			vertex_offset: vertex_offset as i32,
		});
	}

	geometry
}

/// The `UiRenderPass` struct centralizes batched UI rectangle rendering and text overlay compositing for the main render target.
pub struct UiRenderPass {
	pipeline: ghi::PipelineHandle,
	vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	curve_pipeline: ghi::PipelineHandle,
	curve_vertex_buffer: ghi::BufferHandle<[UiCurveVertex; MAX_UI_VERTICES]>,
	curve_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	image_pipeline: ghi::PipelineHandle,
	image_vertex_buffer: ghi::BufferHandle<[UiImageVertex; MAX_UI_VERTICES]>,
	image_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	image_descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	image_sampler: ghi::SamplerHandle,
	image_textures: HashMap<u64, UiImageTexture>,
	text_pipeline: ghi::PipelineHandle,
	text_descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	text_sampler: ghi::SamplerHandle,
	text_overlays: Vec<UiTextOverlayTexture>,
	blur_copy_pipeline: ghi::PipelineHandle,
	blur_compute_pipeline_x: ghi::PipelineHandle,
	blur_compute_pipeline_y: ghi::PipelineHandle,
	blur_compute_descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	blur_composite_pipeline: ghi::PipelineHandle,
	blur_vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	blur_index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	blur_composite_descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	blur_sampler: ghi::SamplerHandle,
	blur_full_source_descriptor_set: ghi::DescriptorSetHandle,
	blur_downsample_descriptor_set: ghi::DescriptorSetHandle,
	blur_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_feedback_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_y_descriptor_set: ghi::DescriptorSetHandle,
	blur_composite_descriptor_set: ghi::DescriptorSetHandle,
	blur_composite_source: ghi::BaseImageHandle,
	blur_source: ghi::BaseImageHandle,
	blur_scratch: ghi::BaseImageHandle,
	blur_output: ghi::BaseImageHandle,
	main_attachment: ghi::BaseImageHandle,
	data: UiDrawList,
	reported_capacity_limit: bool,
	text_system: TextSystem,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout primitives.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let main_attachment = render_pass_builder.create_render_target(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::RenderTarget | ghi::Uses::Image).name("UI"),
		);

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
		let curve_vertex_shader = create_curve_vertex_shader(context);
		let curve_fragment_shader = create_curve_fragment_shader(context);
		let curve_shaders = [
			ghi::ShaderParameter::new(&curve_vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&curve_fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let curve_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[],
			&[],
			&UI_CURVE_VERTEX_LAYOUT,
			&curve_shaders,
			&attachments,
		));
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
		let text_descriptor_set_template = context.create_descriptor_set_template(Some("UI Text"), &[TEXT_OVERLAY_BINDING]);
		let image_descriptor_set_template = context.create_descriptor_set_template(Some("UI Image"), &[UI_IMAGE_BINDING]);
		let image_vertex_shader = create_image_vertex_shader(context);
		let image_fragment_shader = create_image_fragment_shader(context);
		let image_shaders = [
			ghi::ShaderParameter::new(&image_vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&image_fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let image_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[image_descriptor_set_template],
			&[],
			&UI_IMAGE_VERTEX_LAYOUT,
			&image_shaders,
			&attachments,
		));
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
		let blur_compute_descriptor_set_template = context.create_descriptor_set_template(
			Some("UI Backdrop Blur Compute"),
			&[UI_BLUR_SOURCE_BINDING, UI_BLUR_OUTPUT_BINDING],
		);
		let blur_composite_descriptor_set_template = context.create_descriptor_set_template(
			Some("UI Backdrop Blur Composite"),
			&[UI_BLUR_COMPOSITE_SOURCE_BINDING, UI_BLUR_COMPOSITE_BLURRED_BINDING],
		);
		let blur_copy_shader = create_blur_copy_compute_shader(context);
		let blur_copy_pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[blur_compute_descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&blur_copy_shader, ghi::ShaderTypes::Compute),
		));
		let blur_compute_shader = create_blur_compute_shader(context);
		let blur_compute_pipeline_x = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[blur_compute_descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&blur_compute_shader, ghi::ShaderTypes::Compute)
				.with_specialization_map(&[ghi::pipelines::SpecializationMapEntry::new(0, "i32".to_string(), 0i32)]),
		));
		let blur_compute_pipeline_y = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[blur_compute_descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&blur_compute_shader, ghi::ShaderTypes::Compute)
				.with_specialization_map(&[ghi::pipelines::SpecializationMapEntry::new(0, "i32".to_string(), 1i32)]),
		));
		let blur_composite_shader = create_blur_composite_fragment_shader(context);
		let blur_composite_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[blur_composite_descriptor_set_template],
			&[],
			&UI_VERTEX_LAYOUT,
			&[
				ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
				ghi::ShaderParameter::new(&blur_composite_shader, ghi::ShaderTypes::Fragment),
			],
			&attachments,
		));
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
		let blur_composite_source = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Composite Source"),
		);
		let blur_composite_source_image: ghi::BaseImageHandle = blur_composite_source.into();
		let blur_source = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Source"),
		);
		let blur_source_image: ghi::BaseImageHandle = blur_source.into();
		let blur_scratch = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Scratch"),
		);
		let blur_scratch_image: ghi::BaseImageHandle = blur_scratch.into();
		let blur_output = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Output"),
		);
		let blur_output_image: ghi::BaseImageHandle = blur_output.into();
		let main_attachment_image: ghi::BaseImageHandle = main_attachment.into();
		let blur_full_source_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur Full Source"), &blur_compute_descriptor_set_template);
		context.create_descriptor_binding(
			blur_full_source_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_SOURCE_BINDING,
				main_attachment_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_full_source_descriptor_set,
			ghi::BindingConstructor::image(&UI_BLUR_OUTPUT_BINDING, blur_composite_source_image),
		);
		let blur_downsample_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur Downsample"), &blur_compute_descriptor_set_template);
		context.create_descriptor_binding(
			blur_downsample_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_SOURCE_BINDING,
				main_attachment_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_downsample_descriptor_set,
			ghi::BindingConstructor::image(&UI_BLUR_OUTPUT_BINDING, blur_source_image),
		);
		let blur_x_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur X"), &blur_compute_descriptor_set_template);
		context.create_descriptor_binding(
			blur_x_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_SOURCE_BINDING,
				blur_source_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_x_descriptor_set,
			ghi::BindingConstructor::image(&UI_BLUR_OUTPUT_BINDING, blur_scratch_image),
		);
		let blur_feedback_x_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur Feedback X"), &blur_compute_descriptor_set_template);
		context.create_descriptor_binding(
			blur_feedback_x_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_SOURCE_BINDING,
				blur_output_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_feedback_x_descriptor_set,
			ghi::BindingConstructor::image(&UI_BLUR_OUTPUT_BINDING, blur_scratch_image),
		);
		let blur_y_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur Y"), &blur_compute_descriptor_set_template);
		context.create_descriptor_binding(
			blur_y_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_SOURCE_BINDING,
				blur_scratch_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_y_descriptor_set,
			ghi::BindingConstructor::image(&UI_BLUR_OUTPUT_BINDING, blur_output_image),
		);
		let blur_composite_descriptor_set =
			context.create_descriptor_set(Some("UI Backdrop Blur Composite"), &blur_composite_descriptor_set_template);
		context.create_descriptor_binding(
			blur_composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_COMPOSITE_SOURCE_BINDING,
				blur_composite_source_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);
		context.create_descriptor_binding(
			blur_composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&UI_BLUR_COMPOSITE_BLURRED_BINDING,
				blur_output_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
		);

		Self {
			pipeline,
			vertex_buffer,
			index_buffer,
			curve_pipeline,
			curve_vertex_buffer,
			curve_index_buffer,
			image_pipeline,
			image_vertex_buffer,
			image_index_buffer,
			image_descriptor_set_template,
			image_sampler,
			image_textures: HashMap::new(),
			text_pipeline,
			text_descriptor_set_template,
			text_sampler,
			text_overlays: vec![UiTextOverlayTexture {
				image: text_overlay.into(),
				descriptor_set: {
					let descriptor_set = context.create_descriptor_set(Some("UI Text"), &text_descriptor_set_template);
					context.create_descriptor_binding(
						descriptor_set,
						ghi::BindingConstructor::combined_image_sampler(
							&TEXT_OVERLAY_BINDING,
							text_overlay,
							text_sampler,
							ghi::Layouts::Read,
						),
					);
					descriptor_set
				},
			}],
			blur_copy_pipeline,
			blur_compute_pipeline_x,
			blur_compute_pipeline_y,
			blur_compute_descriptor_set_template,
			blur_composite_pipeline,
			blur_vertex_buffer,
			blur_index_buffer,
			blur_composite_descriptor_set_template,
			blur_sampler,
			blur_full_source_descriptor_set,
			blur_downsample_descriptor_set,
			blur_x_descriptor_set,
			blur_feedback_x_descriptor_set,
			blur_y_descriptor_set,
			blur_composite_descriptor_set,
			blur_composite_source: blur_composite_source_image,
			blur_source: blur_source_image,
			blur_scratch: blur_scratch_image,
			blur_output: blur_output_image,
			main_attachment: main_attachment_image,
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
			let descriptor_set = frame.create_descriptor_set(Some("UI Image"), &self.image_descriptor_set_template);
			frame.create_descriptor_binding(
				descriptor_set,
				ghi::BindingConstructor::combined_image_sampler(
					&UI_IMAGE_BINDING,
					texture,
					self.image_sampler,
					ghi::Layouts::Read,
				),
			);
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
			let descriptor_set = frame.create_descriptor_set(Some("UI Text"), &self.text_descriptor_set_template);
			frame.create_descriptor_binding(
				descriptor_set,
				ghi::BindingConstructor::combined_image_sampler(
					&TEXT_OVERLAY_BINDING,
					text_overlay,
					self.text_sampler,
					ghi::Layouts::Read,
				),
			);
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
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extent = sink.extent();
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

			let blur_extent = Extent::rectangle(
				(extent.width() / UI_BLUR_DOWNSCALE).max(1),
				(extent.height() / UI_BLUR_DOWNSCALE).max(1),
			);
			frame.resize_image(self.blur_composite_source, extent);
			frame.resize_image(self.blur_source, blur_extent);
			frame.resize_image(self.blur_scratch, blur_extent);
			frame.resize_image(self.blur_output, blur_extent);
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
			geometry.batches.len() + curve_geometry.batches.len() + prepared_image_batches.len() + prepared_text_batches.len(),
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

		let pipeline = self.pipeline;
		let vertex_buffer = self.vertex_buffer;
		let index_buffer = self.index_buffer;
		let curve_pipeline = self.curve_pipeline;
		let curve_vertex_buffer = self.curve_vertex_buffer;
		let curve_index_buffer = self.curve_index_buffer;
		let image_pipeline = self.image_pipeline;
		let image_vertex_buffer = self.image_vertex_buffer;
		let image_index_buffer = self.image_index_buffer;
		let text_pipeline = self.text_pipeline;
		let blur_copy_pipeline = self.blur_copy_pipeline;
		let blur_compute_pipeline_x = self.blur_compute_pipeline_x;
		let blur_compute_pipeline_y = self.blur_compute_pipeline_y;
		let blur_composite_pipeline = self.blur_composite_pipeline;
		let blur_vertex_buffer = self.blur_vertex_buffer;
		let blur_index_buffer = self.blur_index_buffer;
		let blur_full_source_descriptor_set = self.blur_full_source_descriptor_set;
		let blur_downsample_descriptor_set = self.blur_downsample_descriptor_set;
		let blur_x_descriptor_set = self.blur_x_descriptor_set;
		let blur_feedback_x_descriptor_set = self.blur_feedback_x_descriptor_set;
		let blur_y_descriptor_set = self.blur_y_descriptor_set;
		let blur_composite_descriptor_set = self.blur_composite_descriptor_set;
		let main_attachment = self.main_attachment;
		let batches: &'a [UiPreparedBatch] = frame_allocator.alloc_slice_copy(&prepared_batches);
		let blur_extent = Extent::rectangle(
			(extent.width() / UI_BLUR_DOWNSCALE).max(1),
			(extent.height() / UI_BLUR_DOWNSCALE).max(1),
		);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("UI"),
					|command_buffer| {
						let mut needs_clear = true;

						if !batches.is_empty() {
							for batch in batches {
								let attachments = [ghi::AttachmentInformation::new(
									main_attachment,
									ghi::Layouts::RenderTarget,
									ghi::ClearValue::None,
									!needs_clear,
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
										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(text_pipeline);
										command_buffer.bind_descriptor_sets(&[prepared.descriptor_set]);
										command_buffer.draw(3, 1, 0, 0);
										command_buffer.end_render_pass();
									}
									UiPreparedBatch::Blur(batch) => {
										let compute = command_buffer.bind_compute_pipeline(blur_copy_pipeline);
										compute.bind_descriptor_sets(&[blur_full_source_descriptor_set]);
										compute
											.dispatch(ghi::DispatchExtent::new(extent, Extent::square(UI_BLUR_WORKGROUP_SIZE)));

										let compute = command_buffer.bind_compute_pipeline(blur_copy_pipeline);
										compute.bind_descriptor_sets(&[blur_downsample_descriptor_set]);
										compute.dispatch(ghi::DispatchExtent::new(
											blur_extent,
											Extent::square(UI_BLUR_WORKGROUP_SIZE),
										));

										for pass in 0..batch.radius_pixels {
											let compute = command_buffer.bind_compute_pipeline(blur_compute_pipeline_x);
											let x_descriptor_set = if pass == 0 {
												blur_x_descriptor_set
											} else {
												blur_feedback_x_descriptor_set
											};
											compute.bind_descriptor_sets(&[x_descriptor_set]);
											compute.dispatch(ghi::DispatchExtent::new(
												blur_extent,
												Extent::square(UI_BLUR_WORKGROUP_SIZE),
											));

											let compute = command_buffer.bind_compute_pipeline(blur_compute_pipeline_y);
											compute.bind_descriptor_sets(&[blur_y_descriptor_set]);
											compute.dispatch(ghi::DispatchExtent::new(
												blur_extent,
												Extent::square(UI_BLUR_WORKGROUP_SIZE),
											));
										}

										command_buffer.bind_vertex_buffers(&[blur_vertex_buffer.into()]);
										command_buffer.bind_index_buffer(
											&(Into::<ghi::BufferDescriptor>::into(blur_index_buffer)
												.index_type(ghi::DataTypes::U16)),
										);

										let command_buffer = command_buffer.start_render_pass(extent, &attachments);
										let command_buffer = command_buffer.bind_raster_pipeline(blur_composite_pipeline);
										command_buffer.bind_descriptor_sets(&[blur_composite_descriptor_set]);
										command_buffer.draw_indexed(
											batch.index_count,
											1,
											batch.first_index,
											batch.vertex_offset,
											0,
										);
										command_buffer.end_render_pass();
									}
								}
							}
						}
					},
				);
			},
		))
	}
}

fn create_ui_besl_shader(
	context: &mut ghi::implementation::Context,
	id: &str,
	name: &str,
	stage: ResourceShaderTypes,
	settings: ShaderGenerationSettings,
	main_node: besl::NodeReference,
	interface: material::ShaderInterface,
) -> ghi::ShaderHandle {
	crate::rendering::shader_store::create_shader(
		context,
		None,
		&ShaderSourceDescriptor {
			id,
			name,
			stage,
			source: ShaderSourceDefinition::Besl { settings, main_node },
			interface,
		},
	)
	.expect("Failed to create UI BESL shader. The most likely cause is an incompatible shader interface.")
}

/// Lexes a complete UI shader scope and returns the entry point consumed by render pipeline creation.
fn lex_ui_shader(root: ParserNode<'_>, shader_name: &str) -> besl::NodeReference {
	let root = besl::lex(root)
		.unwrap_or_else(|_| panic!("Failed to lex {shader_name}. The most likely cause is invalid BESL syntax."));
	root.get_main().unwrap_or_else(|| {
		panic!("Failed to find {shader_name} entry point. The most likely cause is a missing main function.")
	})
}

/// Builds the UI vertex shader using BESL and compiles it for the active platform.
fn create_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	let main_node = create_ui_vertex_program();
	create_ui_besl_shader(
		context,
		"byte-engine/ui/rect/vertex",
		"UI Vertex Shader",
		ResourceShaderTypes::Vertex,
		ShaderGenerationSettings::vertex(),
		main_node,
		material::ShaderInterface {
			workgroup_size: None,
			bindings: Vec::new(),
		},
	)
}

/// Builds the portable UI rectangle vertex program shared by VM tests and production backends.
fn create_ui_vertex_program() -> besl::NodeReference {
	let member = ParserNode::member_expression;
	let forward = |output: &'static str, input: &'static str| ParserNode::member_assignment(output, member(input));

	// Express the portable vertex plumbing as BESL nodes so the VM and every backend execute the same program.
	let main = ParserNode::main_function(vec![
		ParserNode::member_assignment(
			"position",
			ParserNode::call(
				"vec4f",
				vec![
					ParserNode::accessor(member("in_position"), member("x")),
					ParserNode::accessor(member("in_position"), member("y")),
					ParserNode::literal_expression("0.0"),
					ParserNode::literal_expression("1.0"),
				],
			),
		),
		forward("out_color", "in_color"),
		forward("out_pixel_position", "in_pixel_position"),
		forward("out_local_position", "in_local_position"),
		forward("out_rect_size", "in_rect_size"),
		forward("out_corner_radius", "in_corner_radius"),
		forward("out_corner_exponent", "in_corner_exponent"),
		forward("out_layer_kind", "in_layer_kind"),
		forward("out_stroke_width", "in_stroke_width"),
		forward("out_feather_mask_position", "in_feather_mask_position"),
		forward("out_feather_mask_size", "in_feather_mask_size"),
		forward("out_feather_mask_edges", "in_feather_mask_edges"),
		forward("out_feather_mask_corner", "in_feather_mask_corner"),
	]);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			ParserNode::input("in_position", "vec2f", 0),
			ParserNode::input("in_pixel_position", "vec2f", 1),
			ParserNode::input("in_local_position", "vec2f", 2),
			ParserNode::input("in_rect_size", "vec2f", 3),
			ParserNode::input("in_color", "vec4f", 4),
			ParserNode::input("in_corner_radius", "f32", 5),
			ParserNode::input("in_corner_exponent", "f32", 6),
			ParserNode::input("in_layer_kind", "f32", 7),
			ParserNode::input("in_stroke_width", "f32", 8),
			ParserNode::input("in_feather_mask_position", "vec2f", 9),
			ParserNode::input("in_feather_mask_size", "vec2f", 10),
			ParserNode::input("in_feather_mask_edges", "vec4f", 11),
			ParserNode::input("in_feather_mask_corner", "vec2f", 12),
			ParserNode::output("position", "vec4f", 0),
			ParserNode::output("out_color", "vec4f", 0),
			ParserNode::output("out_pixel_position", "vec2f", 1),
			ParserNode::output("out_local_position", "vec2f", 2),
			ParserNode::output("out_rect_size", "vec2f", 3),
			ParserNode::output("out_corner_radius", "f32", 4),
			ParserNode::output("out_corner_exponent", "f32", 5),
			ParserNode::output("out_layer_kind", "f32", 6),
			ParserNode::output("out_stroke_width", "f32", 7),
			ParserNode::output("out_feather_mask_position", "vec2f", 8),
			ParserNode::output("out_feather_mask_size", "vec2f", 9),
			ParserNode::output("out_feather_mask_edges", "vec4f", 10),
			ParserNode::output("out_feather_mask_corner", "vec2f", 11),
			main,
		],
	);
	let mut root = ParserNode::root();
	root.add(vec![shader_scope]);
	lex_ui_shader(root, "UI vertex shader")
}

/// Builds the UI fragment shader using BESL and compiles it to SPIR-V.
fn create_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	let main_node = create_ui_fragment_program();
	create_ui_besl_shader(
		context,
		"byte-engine/ui/rect/fragment",
		"UI Fragment Shader",
		ResourceShaderTypes::Fragment,
		ShaderGenerationSettings::fragment(),
		main_node,
		material::ShaderInterface {
			workgroup_size: None,
			bindings: Vec::new(),
		},
	)
}

/// Builds the portable UI rectangle fragment program shared by VM tests and production backends.
fn create_ui_fragment_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	let vec4f = root.get_child("vec4f").expect("vec4f type not found in BESL root");
	let vec2f = root.get_child("vec2f").expect("vec2f type not found in BESL root");
	let f32 = root.get_child("f32").expect("f32 type not found in BESL root");

	root.add_child(besl::Node::input("in_color", vec4f.clone(), 0).into());
	root.add_child(besl::Node::input("in_pixel_position", vec2f.clone(), 1).into());
	root.add_child(besl::Node::input("in_local_position", vec2f.clone(), 2).into());
	root.add_child(besl::Node::input("in_rect_size", vec2f.clone(), 3).into());
	root.add_child(besl::Node::input("in_corner_radius", f32.clone(), 4).into());
	root.add_child(besl::Node::input("in_corner_exponent", f32.clone(), 5).into());
	root.add_child(besl::Node::input("in_layer_kind", f32.clone(), 6).into());
	root.add_child(besl::Node::input("in_stroke_width", f32, 7).into());
	root.add_child(besl::Node::input("in_feather_mask_position", vec2f.clone(), 8).into());
	root.add_child(besl::Node::input("in_feather_mask_size", vec2f.clone(), 9).into());
	root.add_child(besl::Node::input("in_feather_mask_edges", vec4f.clone(), 10).into());
	root.add_child(besl::Node::input("in_feather_mask_corner", vec2f, 11).into());
	root.add_child(besl::Node::output("out_color_attachment", vec4f, 0).into());

	let program = besl::compile_to_besl(UI_FRAGMENT_SHADER_BESL, Some(root))
		.expect("Failed to compile UI fragment BESL. The most likely cause is invalid BESL syntax.");
	program
		.get_main()
		.expect("Failed to find UI fragment shader entry point. The most likely cause is a missing main function.")
}

const UI_FRAGMENT_SHADER_BESL: &str = r#"
main: fn() -> void {
	let half_size: vec2f = in_rect_size * 0.5;
	let corner_radius: f32 = min(in_corner_radius, min(half_size.x, half_size.y));
	let corner_exponent: f32 = in_corner_exponent;
	let centered_position: vec2f = in_local_position - half_size;
	let rounded_extent: vec2f = half_size - vec2f(corner_radius, corner_radius);
	let corner_delta: vec2f = abs(centered_position) - rounded_extent;
	let abs_corner: vec2f = max(corner_delta, vec2f(0.0, 0.0));
	let corner_sum: f32 = pow(abs_corner.x, corner_exponent) + pow(abs_corner.y, corner_exponent);
	let corner_distance: f32 = pow(corner_sum, 1.0 / corner_exponent);
	let field_distance: f32 = corner_distance + min(max(corner_delta.x, corner_delta.y), 0.0) - corner_radius;
	let edge_width: f32 = max(fwidth(field_distance), 1.0);
	let rounded_shape: f32 = step(0.0001, corner_radius);
	let rounded_fill_coverage: f32 = 1.0 - smoothstep(0.0 - edge_width, edge_width, field_distance);
	let fill_coverage: f32 = mix(1.0, rounded_fill_coverage, rounded_shape);

	let corner_gradient_scale: f32 = pow(max(corner_sum, 0.0001), (1.0 / corner_exponent) - 1.0);
	let corner_gradient: vec2f = vec2f(
		pow(abs_corner.x, corner_exponent - 1.0) * corner_gradient_scale,
		pow(abs_corner.y, corner_exponent - 1.0) * corner_gradient_scale
	);
	let field_gradient_length: f32 = mix(1.0, max(length(vec4f(corner_gradient.x, corner_gradient.y, 0.0, 0.0)), 0.0001), step(0.0001, corner_sum));
	let signed_distance: f32 = field_distance / field_gradient_length;
	let corrected_edge_width: f32 = max(fwidth(signed_distance), 1.0);
	let inner_signed_distance: f32 = signed_distance + in_stroke_width;
	let inner_coverage: f32 = 1.0 - smoothstep(0.0 - corrected_edge_width, corrected_edge_width, inner_signed_distance);
	let stroke_coverage: f32 = max(fill_coverage - inner_coverage, 0.0);
	let coverage: f32 = mix(fill_coverage, stroke_coverage, step(0.5, in_layer_kind));
	let feather_top: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), in_pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	let feather_right: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - in_pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	let feather_bottom: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - in_pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	let feather_left: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), in_pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	let feather_half_size: vec2f = in_feather_mask_size * 0.5;
	let feather_corner_radius: f32 = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	let feather_corner_exponent: f32 = in_feather_mask_corner.y;
	let feather_centered_position: vec2f = in_pixel_position - in_feather_mask_position - feather_half_size;
	let feather_rounded_extent: vec2f = feather_half_size - vec2f(feather_corner_radius, feather_corner_radius);
	let feather_corner_delta: vec2f = abs(feather_centered_position) - feather_rounded_extent;
	let feather_abs_corner: vec2f = max(feather_corner_delta, vec2f(0.0, 0.0));
	let feather_corner_sum: f32 = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	let feather_corner_distance: f32 = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	let feather_field_distance: f32 = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	let feather_mask_enabled: f32 = step(0.0001, min(in_feather_mask_size.x, in_feather_mask_size.y));
	let feather_rounded_shape: f32 = step(0.0001, feather_corner_radius);
	let feather_shape_coverage: f32 = mix(1.0, 1.0 - smoothstep(0.0 - 1.0, 1.0, feather_field_distance), feather_rounded_shape);
	let feather_coverage: f32 = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	out_color_attachment = vec4f(in_color.x, in_color.y, in_color.z, in_color.w * coverage * feather_coverage);
}
"#;

fn create_curve_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Curve Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_CURVE_VERTEX_SHADER_GLSL,
			msl: UI_CURVE_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_curve_vertex_main",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI curve vertex shader. The most likely cause is an incompatible shader interface.")
}

fn create_curve_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Curve Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_CURVE_FRAGMENT_SHADER_GLSL,
			msl: UI_CURVE_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_curve_fragment_main",
		},
		ghi::ShaderTypes::Fragment,
		[],
	)
	.expect("Failed to create the UI curve fragment shader. The most likely cause is an incompatible shader interface.")
}

const UI_CURVE_VERTEX_SHADER_GLSL: &str = r#"
#version 450

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_pixel_position;
layout(location = 2) in vec2 in_segment_from;
layout(location = 3) in vec2 in_segment_to;
layout(location = 4) in vec4 in_color;
layout(location = 5) in float in_half_width;
layout(location = 6) in vec2 in_feather_mask_position;
layout(location = 7) in vec2 in_feather_mask_size;
layout(location = 8) in vec4 in_feather_mask_edges;
layout(location = 9) in vec2 in_feather_mask_corner;

layout(location = 0) out vec2 out_pixel_position;
layout(location = 1) out vec2 out_segment_from;
layout(location = 2) out vec2 out_segment_to;
layout(location = 3) out vec4 out_color;
layout(location = 4) out float out_half_width;
layout(location = 5) out vec2 out_feather_mask_position;
layout(location = 6) out vec2 out_feather_mask_size;
layout(location = 7) out vec4 out_feather_mask_edges;
layout(location = 8) out vec2 out_feather_mask_corner;

void main() {
	gl_Position = vec4(in_position, 0.0, 1.0);
	out_pixel_position = in_pixel_position;
	out_segment_from = in_segment_from;
	out_segment_to = in_segment_to;
	out_color = in_color;
	out_half_width = in_half_width;
	out_feather_mask_position = in_feather_mask_position;
	out_feather_mask_size = in_feather_mask_size;
	out_feather_mask_edges = in_feather_mask_edges;
	out_feather_mask_corner = in_feather_mask_corner;
}
"#;

const UI_CURVE_FRAGMENT_SHADER_GLSL: &str = r#"
#version 450

layout(location = 0) in vec2 in_pixel_position;
layout(location = 1) in vec2 in_segment_from;
layout(location = 2) in vec2 in_segment_to;
layout(location = 3) in vec4 in_color;
layout(location = 4) in float in_half_width;
layout(location = 5) in vec2 in_feather_mask_position;
layout(location = 6) in vec2 in_feather_mask_size;
layout(location = 7) in vec4 in_feather_mask_edges;
layout(location = 8) in vec2 in_feather_mask_corner;

layout(location = 0) out vec4 out_color_attachment;

void main() {
	vec2 segment = in_segment_to - in_segment_from;
	float length_squared = max(dot(segment, segment), 0.0001);
	float segment_length = sqrt(length_squared);
	vec2 tangent = segment / segment_length;
	vec2 normal = vec2(-tangent.y, tangent.x);
	vec2 center = (in_segment_from + in_segment_to) * 0.5;
	vec2 relative_position = in_pixel_position - center;
	vec2 strip_distance = abs(vec2(dot(relative_position, tangent), dot(relative_position, normal))) - vec2(segment_length * 0.5, in_half_width);
	float outside_distance = length(max(strip_distance, vec2(0.0)));
	float inside_distance = min(max(strip_distance.x, strip_distance.y), 0.0);
	float signed_distance = outside_distance + inside_distance;
	float edge_width = max(fwidth(signed_distance), 1.0);
	float coverage = 1.0 - smoothstep(-edge_width, edge_width, signed_distance);

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
}
"#;

const UI_CURVE_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct UiCurveVertexIn {
	float2 position [[attribute(0)]];
	float2 pixel_position [[attribute(1)]];
	float2 segment_from [[attribute(2)]];
	float2 segment_to [[attribute(3)]];
	float4 color [[attribute(4)]];
	float half_width [[attribute(5)]];
	float2 feather_mask_position [[attribute(6)]];
	float2 feather_mask_size [[attribute(7)]];
	float4 feather_mask_edges [[attribute(8)]];
	float2 feather_mask_corner [[attribute(9)]];
};

struct UiCurveVertexOut {
	float4 position [[position]];
	float2 pixel_position;
	float2 segment_from;
	float2 segment_to;
	float4 color;
	float half_width;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

vertex UiCurveVertexOut ui_curve_vertex_main(UiCurveVertexIn in [[stage_in]]) {
	UiCurveVertexOut out;
	out.position = float4(in.position, 0.0, 1.0);
	out.pixel_position = in.pixel_position;
	out.segment_from = in.segment_from;
	out.segment_to = in.segment_to;
	out.color = in.color;
	out.half_width = in.half_width;
	out.feather_mask_position = in.feather_mask_position;
	out.feather_mask_size = in.feather_mask_size;
	out.feather_mask_edges = in.feather_mask_edges;
	out.feather_mask_corner = in.feather_mask_corner;
	return out;
}
"#;

const UI_CURVE_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct UiCurveVertexOut {
	float4 position [[position]];
	float2 pixel_position;
	float2 segment_from;
	float2 segment_to;
	float4 color;
	float half_width;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

fragment float4 ui_curve_fragment_main(UiCurveVertexOut in [[stage_in]]) {
	float2 segment = in.segment_to - in.segment_from;
	float length_squared = max(dot(segment, segment), 0.0001);
	float segment_length = sqrt(length_squared);
	float2 tangent = segment / segment_length;
	float2 normal = float2(-tangent.y, tangent.x);
	float2 center = (in.segment_from + in.segment_to) * 0.5;
	float2 relative_position = in.pixel_position - center;
	float2 strip_distance = abs(float2(dot(relative_position, tangent), dot(relative_position, normal))) - float2(segment_length * 0.5, in.half_width);
	float outside_distance = length(max(strip_distance, float2(0.0)));
	float inside_distance = min(max(strip_distance.x, strip_distance.y), 0.0);
	float signed_distance = outside_distance + inside_distance;
	float edge_width = max(fwidth(signed_distance), 1.0);
	float coverage = 1.0 - smoothstep(-edge_width, edge_width, signed_distance);

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

fn create_image_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Image Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: IMAGE_VERTEX_SHADER_GLSL,
			msl: IMAGE_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_image_vertex",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI image vertex shader. The most likely cause is an incompatible shader interface.")
}

fn create_image_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Image Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: IMAGE_FRAGMENT_SHADER_GLSL,
			msl: IMAGE_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_image_fragment",
		},
		ghi::ShaderTypes::Fragment,
		[UI_IMAGE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ)],
	)
	.expect("Failed to create the UI image fragment shader. The most likely cause is an incompatible shader interface.")
}

fn create_blur_copy_compute_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Backdrop Blur Copy Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_BLUR_COPY_SHADER_GLSL,
			msl: UI_BLUR_COPY_SHADER_MSL,
			msl_entry_point: "ui_backdrop_blur_copy",
		},
		ghi::ShaderTypes::Compute,
		[
			UI_BLUR_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			UI_BLUR_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
		],
	)
	.expect("Failed to create the UI backdrop blur copy shader. The most likely cause is an incompatible shader interface.")
}

fn create_blur_compute_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Backdrop Blur Compute Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_BLUR_COMPUTE_SHADER_GLSL,
			msl: UI_BLUR_COMPUTE_SHADER_MSL,
			msl_entry_point: "ui_backdrop_blur_compute",
		},
		ghi::ShaderTypes::Compute,
		[
			UI_BLUR_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			UI_BLUR_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
		],
	)
	.expect("Failed to create the UI backdrop blur compute shader. The most likely cause is an incompatible shader interface.")
}

fn create_blur_composite_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Backdrop Blur Composite Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_BLUR_COMPOSITE_FRAGMENT_SHADER_GLSL,
			msl: UI_BLUR_COMPOSITE_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_backdrop_blur_composite",
		},
		ghi::ShaderTypes::Fragment,
		[
			UI_BLUR_COMPOSITE_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			UI_BLUR_COMPOSITE_BLURRED_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
		],
	)
	.expect(
		"Failed to create the UI backdrop blur composite shader. The most likely cause is an incompatible shader interface.",
	)
}

const UI_BLUR_COPY_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(compute)

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform sampler2D source_texture;
layout(set = 0, binding = 1, rgba16) uniform writeonly image2D output_texture;

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 output_size = imageSize(output_texture);
	if (pixel.x >= output_size.x || pixel.y >= output_size.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + vec2(0.5)) / vec2(output_size);
	imageStore(output_texture, pixel, textureLod(source_texture, uv, 0.0));
}
"#;

const UI_BLUR_COPY_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct BlurCopySet0 {
	texture2d<float> source_texture [[id(0)]];
	sampler source_sampler [[id(1)]];
	texture2d<float, access::write> output_texture [[id(2)]];
};

kernel void ui_backdrop_blur_copy(
	constant BlurCopySet0& set0 [[buffer(16)]],
	uint2 pixel [[thread_position_in_grid]]
) {
	uint width = set0.output_texture.get_width();
	uint height = set0.output_texture.get_height();
	if (pixel.x >= width || pixel.y >= height) {
		return;
	}

	constexpr sampler copy_sampler(coord::normalized, address::clamp_to_edge, filter::linear);
	float2 uv = (float2(pixel) + float2(0.5)) / float2(width, height);
	set0.output_texture.write(set0.source_texture.sample(copy_sampler, uv, level(0.0)), pixel);
}
"#;

const UI_BLUR_COMPUTE_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(compute)

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;
layout(constant_id = 0) const int BLUR_AXIS = 0;

layout(set = 0, binding = 0) uniform sampler2D source_texture;
layout(set = 0, binding = 1, rgba16) uniform writeonly image2D output_texture;

const float WEIGHTS[5] = float[](0.22702703, 0.19459459, 0.12162162, 0.05405405, 0.01621622);

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 output_size = imageSize(output_texture);
	if (pixel.x >= output_size.x || pixel.y >= output_size.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + vec2(0.5)) / vec2(output_size);
	vec2 texel_size = 1.0 / vec2(textureSize(source_texture, 0));
	vec2 direction = BLUR_AXIS == 0 ? vec2(texel_size.x, 0.0) : vec2(0.0, texel_size.y);
	vec4 color = textureLod(source_texture, uv, 0.0) * WEIGHTS[0];
	for (int i = 1; i < 5; i++) {
		vec2 offset = direction * float(i);
		color += textureLod(source_texture, uv + offset, 0.0) * WEIGHTS[i];
		color += textureLod(source_texture, uv - offset, 0.0) * WEIGHTS[i];
	}
	imageStore(output_texture, pixel, color);
}
"#;

const UI_BLUR_COMPUTE_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

constant int BLUR_AXIS [[function_constant(0)]];

struct BlurSet0 {
	texture2d<float> source_texture [[id(0)]];
	sampler source_sampler [[id(1)]];
	texture2d<float, access::write> output_texture [[id(2)]];
};

kernel void ui_backdrop_blur_compute(
	constant BlurSet0& set0 [[buffer(16)]],
	uint2 pixel [[thread_position_in_grid]]
) {
	uint width = set0.output_texture.get_width();
	uint height = set0.output_texture.get_height();
	if (pixel.x >= width || pixel.y >= height) {
		return;
	}

	constexpr sampler blur_sampler(coord::normalized, address::clamp_to_edge, filter::linear);
	float weights[5] = {0.22702703, 0.19459459, 0.12162162, 0.05405405, 0.01621622};
	float2 uv = (float2(pixel) + float2(0.5)) / float2(width, height);
	float2 texel_size = 1.0 / float2(set0.source_texture.get_width(), set0.source_texture.get_height());
	float2 direction = BLUR_AXIS == 0 ? float2(texel_size.x, 0.0) : float2(0.0, texel_size.y);
	float4 color = set0.source_texture.sample(blur_sampler, uv, level(0.0)) * weights[0];
	for (int i = 1; i < 5; i++) {
		float2 offset = direction * float(i);
		color += set0.source_texture.sample(blur_sampler, uv + offset, level(0.0)) * weights[i];
		color += set0.source_texture.sample(blur_sampler, uv - offset, level(0.0)) * weights[i];
	}
	set0.output_texture.write(color, pixel);
}
"#;

const UI_BLUR_COMPOSITE_FRAGMENT_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(fragment)

layout(set = 0, binding = 0) uniform sampler2D source_texture;
layout(set = 0, binding = 1) uniform sampler2D blurred_texture;

layout(location = 0) in vec4 in_color;
layout(location = 1) in vec2 in_pixel_position;
layout(location = 2) in vec2 in_local_position;
layout(location = 3) in vec2 in_rect_size;
layout(location = 4) in float in_corner_radius;
layout(location = 5) in float in_corner_exponent;
layout(location = 6) in float in_layer_kind;
layout(location = 7) in float in_stroke_width;
layout(location = 8) in vec2 in_feather_mask_position;
layout(location = 9) in vec2 in_feather_mask_size;
layout(location = 10) in vec4 in_feather_mask_edges;
layout(location = 11) in vec2 in_feather_mask_corner;
layout(location = 0) out vec4 out_color_attachment;

void main() {
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
	float coverage = mix(1.0, rounded_fill_coverage, rounded_shape);
	float feather_top = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), in_pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - in_pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - in_pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), in_pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	float feather_coverage = feather_top * feather_right * feather_bottom * feather_left;
	vec2 source_uv = gl_FragCoord.xy / vec2(textureSize(source_texture, 0));
	vec2 blur_uv = gl_FragCoord.xy / vec2(textureSize(blurred_texture, 0));
	vec4 source = texture(source_texture, source_uv);
	vec4 blurred = texture(blurred_texture, blur_uv);
	float blur_strength = clamp(coverage * feather_coverage, 0.0, 1.0);
	vec3 color = mix(source.rgb, blurred.rgb, blur_strength);
	out_color_attachment = vec4(color, 1.0);
}
"#;

const UI_BLUR_COMPOSITE_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

// This struct consumes the BESL-generated UI vertex shader outputs, which are bound by
// [[user(locnN)]] location, so each field must carry the matching location attribute.
struct UiVertexOut {
	float4 position [[position]];
	float4 color [[user(locn0)]];
	float2 pixel_position [[user(locn1)]];
	float2 local_position [[user(locn2)]];
	float2 rect_size [[user(locn3)]];
	float corner_radius [[user(locn4)]];
	float corner_exponent [[user(locn5)]];
	float layer_kind [[user(locn6)]];
	float stroke_width [[user(locn7)]];
	float2 feather_mask_position [[user(locn8)]];
	float2 feather_mask_size [[user(locn9)]];
	float4 feather_mask_edges [[user(locn10)]];
	float2 feather_mask_corner [[user(locn11)]];
};

struct BlurCompositeSet0 {
	texture2d<float> source_texture [[id(0)]];
	sampler source_sampler [[id(1)]];
	texture2d<float> blurred_texture [[id(2)]];
	sampler blur_sampler [[id(3)]];
};

fragment float4 ui_backdrop_blur_composite(
	UiVertexOut in [[stage_in]],
	constant BlurCompositeSet0& set0 [[buffer(16)]]
) {
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
	float coverage = mix(1.0, rounded_fill_coverage, rounded_shape);
	float feather_top = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.x, 0.0001), in.pixel_position.y - in.feather_mask_position.y), step(0.0001, in.feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.y, 0.0001), in.feather_mask_position.x + in.feather_mask_size.x - in.pixel_position.x), step(0.0001, in.feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.z, 0.0001), in.feather_mask_position.y + in.feather_mask_size.y - in.pixel_position.y), step(0.0001, in.feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.w, 0.0001), in.pixel_position.x - in.feather_mask_position.x), step(0.0001, in.feather_mask_edges.w));
	float feather_coverage = feather_top * feather_right * feather_bottom * feather_left;
	float2 source_extent = float2(set0.source_texture.get_width(), set0.source_texture.get_height());
	float2 source_uv = in.position.xy / source_extent;
	float2 blur_uv = in.position.xy / float2(set0.blurred_texture.get_width(), set0.blurred_texture.get_height());
	float4 source = set0.source_texture.sample(set0.source_sampler, source_uv);
	float4 blurred = set0.blurred_texture.sample(set0.blur_sampler, blur_uv);
	float blur_strength = clamp(coverage * feather_coverage, 0.0, 1.0);
	float3 color = mix(source.rgb, blurred.rgb, blur_strength);
	return float4(color, 1.0);
}
"#;

const IMAGE_VERTEX_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(vertex)

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_uv;
layout(location = 2) in float in_opacity;
layout(location = 3) in vec2 in_feather_mask_position;
layout(location = 4) in vec2 in_feather_mask_size;
layout(location = 5) in vec4 in_feather_mask_edges;
layout(location = 6) in vec2 in_feather_mask_corner;

layout(location = 0) out vec2 out_uv;
layout(location = 1) out float out_opacity;
layout(location = 2) out vec2 out_feather_mask_position;
layout(location = 3) out vec2 out_feather_mask_size;
layout(location = 4) out vec4 out_feather_mask_edges;
layout(location = 5) out vec2 out_feather_mask_corner;

void main() {
	gl_Position = vec4(in_position, 0.0, 1.0);
	out_uv = in_uv;
	out_opacity = in_opacity;
	out_feather_mask_position = in_feather_mask_position;
	out_feather_mask_size = in_feather_mask_size;
	out_feather_mask_edges = in_feather_mask_edges;
	out_feather_mask_corner = in_feather_mask_corner;
}
"#;

const IMAGE_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct ImageVertexIn {
	float2 position [[attribute(0)]];
	float2 uv [[attribute(1)]];
	float opacity [[attribute(2)]];
	float2 feather_mask_position [[attribute(3)]];
	float2 feather_mask_size [[attribute(4)]];
	float4 feather_mask_edges [[attribute(5)]];
	float2 feather_mask_corner [[attribute(6)]];
};

struct ImageVertexOut {
	float4 position [[position]];
	float2 uv;
	float opacity;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

vertex ImageVertexOut ui_image_vertex(ImageVertexIn in [[stage_in]]) {
	ImageVertexOut out;
	out.position = float4(in.position, 0.0, 1.0);
	out.uv = in.uv;
	out.opacity = in.opacity;
	out.feather_mask_position = in.feather_mask_position;
	out.feather_mask_size = in.feather_mask_size;
	out.feather_mask_edges = in.feather_mask_edges;
	out.feather_mask_corner = in.feather_mask_corner;
	return out;
}
"#;

const IMAGE_FRAGMENT_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(fragment)

layout(set = 0, binding = 0) uniform sampler2D image_texture;

layout(location = 0) in vec2 in_uv;
layout(location = 1) in float in_opacity;
layout(location = 2) in vec2 in_feather_mask_position;
layout(location = 3) in vec2 in_feather_mask_size;
layout(location = 4) in vec4 in_feather_mask_edges;
layout(location = 5) in vec2 in_feather_mask_corner;
layout(location = 0) out vec4 out_color_attachment;

void main() {
	vec2 pixel_position = gl_FragCoord.xy;
	float feather_top = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	vec2 feather_half_size = in_feather_mask_size * 0.5;
	float feather_corner_radius = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in_feather_mask_corner.y;
	vec2 feather_centered_position = pixel_position - in_feather_mask_position - feather_half_size;
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
	vec4 color = texture(image_texture, in_uv);
	out_color_attachment = vec4(color.rgb, color.a * in_opacity * feather_coverage);
}
"#;

const IMAGE_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct ImageVertexOut {
	float4 position [[position]];
	float2 uv;
	float opacity;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

struct ImageSet0 {
	texture2d<float> image_texture [[id(0)]];
	sampler image_sampler [[id(1)]];
};

fragment float4 ui_image_fragment(
	ImageVertexOut in [[stage_in]],
	constant ImageSet0& set0 [[buffer(16)]]
) {
	float2 pixel_position = in.position.xy;
	float feather_top = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.x, 0.0001), pixel_position.y - in.feather_mask_position.y), step(0.0001, in.feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.y, 0.0001), in.feather_mask_position.x + in.feather_mask_size.x - pixel_position.x), step(0.0001, in.feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.z, 0.0001), in.feather_mask_position.y + in.feather_mask_size.y - pixel_position.y), step(0.0001, in.feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.w, 0.0001), pixel_position.x - in.feather_mask_position.x), step(0.0001, in.feather_mask_edges.w));
	float2 feather_half_size = in.feather_mask_size * 0.5;
	float feather_corner_radius = min(in.feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in.feather_mask_corner.y;
	float2 feather_centered_position = pixel_position - in.feather_mask_position - feather_half_size;
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
	float4 color = set0.image_texture.sample(set0.image_sampler, in.uv);
	return float4(color.rgb, color.a * in.opacity * feather_coverage);
}
"#;

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
	use besl::vm::{builtin_position_slot, input_slot, output_slot, Buffer, DescriptorBindings, ExecutableProgram, Value};
	use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator as _};
	use utils::{Extent, RGBA};

	use super::{
		build_ui_blur_geometry, build_ui_curve_geometry, build_ui_geometry, build_ui_image_geometry, flatten_curve_segment,
		should_draw_image, should_rasterize_text, update_from_render, DrawClip, DrawFeatherMask, UiBlurDrawElement,
		UiCurveDrawElement, UiDrawBatch, UiDrawElement, UiDrawList, UiImageDrawElement, UiTextDrawElement, MAX_UI_ELEMENTS,
		MAX_UI_VERTICES_PER_DRAW, UI_BLUR_COMPOSITE_FRAGMENT_SHADER_GLSL, UI_BLUR_COMPOSITE_FRAGMENT_SHADER_MSL,
		UI_BLUR_COMPUTE_SHADER_GLSL, UI_BLUR_COMPUTE_SHADER_MSL, UI_BLUR_COPY_SHADER_GLSL, UI_BLUR_COPY_SHADER_MSL,
		UI_CURVE_FRAGMENT_SHADER_GLSL, UI_CURVE_FRAGMENT_SHADER_MSL, UI_FRAGMENT_SHADER_BESL, UI_INDICES_PER_CURVE_SPAN,
		UI_INDICES_PER_ELEMENT, UI_VERTICES_PER_CURVE_SPAN, UI_VERTICES_PER_ELEMENT,
	};
	use crate::ui::{
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
		Container, Text,
	};

	fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
		assert!((actual[0] - expected[0]).abs() < 0.0001);
		assert!((actual[1] - expected[1]).abs() < 0.0001);
	}

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
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
		let executable = ExecutableProgram::compile(super::create_ui_fragment_program()).expect(
			"Failed to compile UI fragment shader for the BESL VM. The most likely cause is missing VM shader support.",
		);
		let mut inputs: [Buffer; 12] = std::array::from_fn(|location| {
			Buffer::new(
				executable
					.input_layout(location as u8)
					.expect("Missing UI fragment input layout. The most likely cause is an unused or unresolved shader input.")
					.clone(),
			)
		});
		let input_values = [
			Value::Vec4F(values.color),
			Value::Vec2F(values.pixel_position),
			Value::Vec2F(values.local_position),
			Value::Vec2F(values.rect_size),
			Value::F32(values.corner_radius),
			Value::F32(values.corner_exponent),
			Value::F32(values.layer_kind),
			Value::F32(values.stroke_width),
			Value::Vec2F(values.feather_mask_position),
			Value::Vec2F(values.feather_mask_size),
			Value::Vec4F(values.feather_mask_edges),
			Value::Vec2F(values.feather_mask_corner),
		];
		let input_names = [
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
		];
		for ((input, name), value) in inputs.iter_mut().zip(input_names).zip(input_values) {
			input
				.write(name, value)
				.expect("Failed to seed a UI fragment VM input. The most likely cause is an interface type mismatch.");
		}

		let mut output = Buffer::new(
			executable
				.output_layout(0)
				.expect("Missing UI fragment output layout. The most likely cause is an unresolved shader output.")
				.clone(),
		);
		{
			let mut descriptors = DescriptorBindings::new();
			for (location, input) in inputs.iter_mut().enumerate() {
				descriptors.bind_buffer(input_slot(location as u8), input);
			}
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute UI fragment shader. The most likely cause is incomplete BESL VM support.");
		}

		match output
			.read("out_color_attachment")
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
	fn blur_geometry_builds_a_composite_quad_and_radius() {
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
			Extent::rectangle(200, 100),
			&frame_allocator,
		);

		assert_eq!(geometry.vertices.len(), 4);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(geometry.batches.len(), 1);
		assert_eq!(geometry.batches[0].depth, 2);
		assert_eq!(geometry.batches[0].order, 7);
		assert_eq!(geometry.batches[0].radius_pixels, 18);
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
	fn primary_ui_besl_shaders_build_besl_programs() {
		let vertex_main = super::create_ui_vertex_program();
		let fragment_main = super::create_ui_fragment_program();

		assert!(matches!(vertex_main.borrow().node(), besl::Nodes::Function { .. }));
		assert!(matches!(fragment_main.borrow().node(), besl::Nodes::Function { .. }));
	}

	/// Verifies the primary portable UI shaders remain accepted by every source backend.
	#[test]
	fn primary_ui_besl_shaders_lower_to_every_source_backend() {
		for (program, settings) in [
			(super::create_ui_vertex_program(), ShaderGenerationSettings::vertex()),
			(super::create_ui_fragment_program(), ShaderGenerationSettings::fragment()),
		] {
			resource_management::shader::besl::backends::glsl::GLSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect(
					"Failed to lower a primary UI BESL shader to GLSL. The most likely cause is unsupported portable syntax.",
				);
			resource_management::shader::besl::backends::hlsl::HLSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect(
					"Failed to lower a primary UI BESL shader to HLSL. The most likely cause is unsupported portable syntax.",
				);
			resource_management::shader::besl::backends::msl::MSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect(
					"Failed to lower a primary UI BESL shader to MSL. The most likely cause is unsupported portable syntax.",
				);
		}
	}

	/// Verifies the production UI vertex shader preserves every geometry and styling varying.
	#[test]
	fn ui_vertex_besl_vm_forwards_position_and_varyings() {
		let executable = ExecutableProgram::compile(super::create_ui_vertex_program())
			.expect("Failed to compile UI vertex shader for the BESL VM. The most likely cause is missing VM shader support.");
		let mut inputs: [Buffer; 13] = std::array::from_fn(|location| {
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
		let mut outputs: [Buffer; 12] = std::array::from_fn(|location| {
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
			position.read("position").expect("Expected position output"),
			Value::Vec4F([0.25, -0.75, 0.0, 1.0])
		);
		for ((output, name), expected) in outputs
			.iter()
			.zip([
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
			])
			.zip([
				Value::Vec4F([0.1, 0.2, 0.3, 0.4]),
				Value::Vec2F([10.0, 20.0]),
				Value::Vec2F([3.0, 4.0]),
				Value::Vec2F([100.0, 80.0]),
				Value::F32(12.0),
				Value::F32(3.0),
				Value::F32(1.0),
				Value::F32(2.5),
				Value::Vec2F([5.0, 6.0]),
				Value::Vec2F([70.0, 60.0]),
				Value::Vec4F([1.0, 2.0, 3.0, 4.0]),
				Value::Vec2F([9.0, 2.0]),
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
	fn blur_composite_msl_fragment_inputs_match_besl_vertex_output_locations() {
		let vertex_main = super::create_ui_vertex_program();
		let vertex_msl = resource_management::shader::besl::backends::msl::MSLShaderGenerator::new()
			.generate(
				&resource_management::shader::generator::ShaderGenerationSettings::vertex(),
				&vertex_main,
			)
			.expect("Failed to lower primary UI vertex shader to MSL.");

		// The blur composite fragment consumes the BESL vertex outputs, so every varying it
		// declares must bind to the same [[user(locnN)]] slot the vertex shader writes.
		for (field, location) in [
			("color", 0),
			("pixel_position", 1),
			("local_position", 2),
			("rect_size", 3),
			("corner_radius", 4),
			("corner_exponent", 5),
			("layer_kind", 6),
			("stroke_width", 7),
			("feather_mask_position", 8),
			("feather_mask_size", 9),
			("feather_mask_edges", 10),
			("feather_mask_corner", 11),
		] {
			assert!(
				super::UI_BLUR_COMPOSITE_FRAGMENT_SHADER_MSL.contains(&format!("{field} [[user(locn{location})]]")),
				"Blur composite fragment field `{field}` must bind to user(locn{location})"
			);
			assert!(
				vertex_msl.contains(&format!("out_{field} [[user(locn{location})]]")),
				"BESL vertex output `out_{field}` must bind to user(locn{location})"
			);
		}
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
