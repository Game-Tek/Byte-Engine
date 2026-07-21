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
const TEXT_OVERLAY_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const UI_IMAGE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const UI_BLUR_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const UI_BLUR_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const UI_BLUR_FULL_COMPOSITE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const UI_BLUR_HALF_COMPOSITE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const UI_BLUR_HALF_DOWNSCALE: u32 = 2;
const UI_BLUR_GAUSSIAN_SUPPORT: u32 = 22;
const UI_BLUR_GAUSSIAN_PAIR_COUNT: usize = 11;
const UI_BLUR_SIGMA_SCALE: f32 = 1.689_394_6;
const UI_BLUR_FULL_ONLY_SIGMA: f32 = 4.0;
const UI_BLUR_HALF_ONLY_SIGMA: f32 = 6.0;
const UI_BLUR_HALF_RESAMPLING_VARIANCE: f32 = 2.75;
const UI_BLUR_DOWNSAMPLE_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiBlurDownsamplePush>() as u32;
const UI_BLUR_FILTER_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiBlurFilterPush>() as u32;
const UI_BLUR_DOWNSAMPLE_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-downsample.besl";
const UI_BLUR_FILTER_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-filter.besl";
const UI_BLUR_COMPOSITE_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-composite.besl";

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

const UI_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 14] = [
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
	ghi::pipelines::VertexElement::new("BLUR_RESOLUTION_MIX", ghi::DataTypes::Float, 0),
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
	blur_resolution_mix: f32,
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

/// The `UiBlurDispatchRegion` struct limits one compute stage to the padded part of the blur target it must produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiBlurDispatchRegion {
	origin: [u32; 2],
	extent: Extent,
}

/// The `UiBlurDownsamplePush` struct carries one regional half-resolution dispatch to the production shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiBlurDownsamplePush {
	origin: [u32; 2],
	extent: [u32; 2],
}

/// The `UiBlurFilterPush` struct keeps the complete Gaussian kernel and dispatch region in one aligned GPU record.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct UiBlurFilterPush {
	filter_data: [f32; 4],
	origin: [u32; 2],
	extent: [u32; 2],
	pair_weights_0_3: [f32; 4],
	pair_weights_4_7: [f32; 4],
	pair_weights_8_10_pad: [f32; 4],
	pair_offsets_0_3: [f32; 4],
	pair_offsets_4_7: [f32; 4],
	pair_offsets_8_10_pad: [f32; 4],
}

/// The `UiBlurKernel` struct stores one normalized Gaussian without allocating transient coefficient buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
struct UiBlurKernel {
	center_weight: f32,
	pair_weights: [f32; UI_BLUR_GAUSSIAN_PAIR_COUNT],
	pair_offsets: [f32; UI_BLUR_GAUSSIAN_PAIR_COUNT],
}

impl UiBlurKernel {
	// Generates the normalized integer taps first, then packs adjacent positive
	// taps for bilinear filtering without scaling their weighted offsets.
	fn gaussian(sigma: f32) -> Self {
		let mut taps = [0.0f64; UI_BLUR_GAUSSIAN_SUPPORT as usize + 1];
		taps[0] = 1.0;
		if sigma.is_finite() && sigma > 0.0 {
			let variance_scale = -0.5 / f64::from(sigma * sigma);
			for (index, tap) in taps.iter_mut().enumerate().skip(1) {
				*tap = (index as f64 * index as f64 * variance_scale).exp();
			}
		}
		let normalization = taps[0] + 2.0 * taps.iter().skip(1).sum::<f64>();
		for tap in &mut taps {
			*tap /= normalization;
		}

		let mut pair_weights = [0.0; UI_BLUR_GAUSSIAN_PAIR_COUNT];
		let mut pair_offsets = [0.0; UI_BLUR_GAUSSIAN_PAIR_COUNT];
		for pair_index in 0..UI_BLUR_GAUSSIAN_PAIR_COUNT {
			let first_index = pair_index * 2 + 1;
			let first_weight = taps[first_index];
			let second_weight = taps[first_index + 1];
			let pair_weight = first_weight + second_weight;
			pair_weights[pair_index] = pair_weight as f32;
			pair_offsets[pair_index] = if pair_weight > 0.0 {
				((first_index as f64 * first_weight + (first_index + 1) as f64 * second_weight) / pair_weight) as f32
			} else {
				first_index as f32 + 0.5
			};
		}

		Self {
			center_weight: taps[0] as f32,
			pair_weights,
			pair_offsets,
		}
	}

	// Combines the reusable kernel with one axis and one regional dispatch.
	fn push(self, direction: [f32; 2], region: UiBlurDispatchRegion) -> UiBlurFilterPush {
		UiBlurFilterPush {
			filter_data: [direction[0], direction[1], self.center_weight, 0.0],
			origin: region.origin,
			extent: region.push_extent(),
			pair_weights_0_3: [
				self.pair_weights[0],
				self.pair_weights[1],
				self.pair_weights[2],
				self.pair_weights[3],
			],
			pair_weights_4_7: [
				self.pair_weights[4],
				self.pair_weights[5],
				self.pair_weights[6],
				self.pair_weights[7],
			],
			pair_weights_8_10_pad: [self.pair_weights[8], self.pair_weights[9], self.pair_weights[10], 0.0],
			pair_offsets_0_3: [
				self.pair_offsets[0],
				self.pair_offsets[1],
				self.pair_offsets[2],
				self.pair_offsets[3],
			],
			pair_offsets_4_7: [
				self.pair_offsets[4],
				self.pair_offsets[5],
				self.pair_offsets[6],
				self.pair_offsets[7],
			],
			pair_offsets_8_10_pad: [self.pair_offsets[8], self.pair_offsets[9], self.pair_offsets[10], 0.0],
		}
	}
}

/// The `UiBlurPathRegions` struct describes the two separable Gaussian stages for one resolution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiBlurPathRegions {
	horizontal: UiBlurDispatchRegion,
	vertical: UiBlurDispatchRegion,
}

/// The `UiBlurHalfPathRegions` struct adds the binomial prefilter region needed by the half-resolution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiBlurHalfPathRegions {
	downsample: UiBlurDispatchRegion,
	filter: UiBlurPathRegions,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UiPreparedBlurBatch {
	depth: u32,
	order: u32,
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
	resolution_mix: f32,
	full_kernel: UiBlurKernel,
	half_kernel: UiBlurKernel,
	full_regions: UiBlurPathRegions,
	half_regions: UiBlurHalfPathRegions,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

// Preserves the legacy repeated-blur strength by mapping its variance-domain
// radius to the standard deviation of one Gaussian.
fn blur_sigma(radius_pixels: f32) -> f32 {
	UI_BLUR_SIGMA_SCALE * radius_pixels.clamp(0.0, 64.0).sqrt()
}

// Removes the half-resolution prefilter and reconstruction variance before
// converting the remaining full-resolution variance to the half lattice.
fn blur_half_sigma(sigma_pixels: f32) -> f32 {
	0.5 * (sigma_pixels * sigma_pixels - UI_BLUR_HALF_RESAMPLING_VARIANCE)
		.max(0.0)
		.sqrt()
}

// Blends continuously between the full and half paths while leaving their
// quality-stable ranges at exactly zero and one.
fn blur_resolution_mix(sigma_pixels: f32) -> f32 {
	let t = ((sigma_pixels - UI_BLUR_FULL_ONLY_SIGMA) / (UI_BLUR_HALF_ONLY_SIGMA - UI_BLUR_FULL_ONLY_SIGMA)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

fn blur_uses_full_resolution(resolution_mix: f32) -> bool {
	resolution_mix < 1.0
}

fn blur_uses_half_resolution(resolution_mix: f32) -> bool {
	resolution_mix > 0.0
}

// Keeps partial edge texels when an odd full-resolution dimension maps to the
// fixed two-pixel half-resolution lattice.
fn blur_half_extent(extent: Extent) -> Extent {
	Extent::rectangle(
		extent.width().div_ceil(UI_BLUR_HALF_DOWNSCALE).max(1),
		extent.height().div_ceil(UI_BLUR_HALF_DOWNSCALE).max(1),
	)
}

impl UiBlurDispatchRegion {
	// Expands one region without crossing the selected blur target's edges.
	fn expanded(self, horizontal: u32, vertical: u32, target: Extent) -> Self {
		let start_x = self.origin[0].saturating_sub(horizontal);
		let start_y = self.origin[1].saturating_sub(vertical);
		let end_x = self.origin[0]
			.saturating_add(self.extent.width())
			.saturating_add(horizontal)
			.min(target.width());
		let end_y = self.origin[1]
			.saturating_add(self.extent.height())
			.saturating_add(vertical)
			.min(target.height());
		Self {
			origin: [start_x, start_y],
			extent: Extent::rectangle(end_x - start_x, end_y - start_y),
		}
	}

	fn push_extent(self) -> [u32; 2] {
		[self.extent.width(), self.extent.height()]
	}
}

// Converts screen bounds through a fixed full- or half-resolution lattice.
// It never derives UV scale from ceil-divided image dimensions, which keeps odd
// viewport widths phase-aligned with the composite shader.
fn blur_composite_region(bounds: [f32; 4], target: Extent, downscale: u32) -> UiBlurDispatchRegion {
	let axis = |minimum: f32, maximum: f32, target_size: u32| {
		let lattice_scale = 1.0 / downscale as f32;
		let start = (minimum * lattice_scale - 0.5).floor().clamp(0.0, target_size as f32) as u32;
		let end = (maximum * lattice_scale + 0.5).ceil().clamp(0.0, target_size as f32) as u32;
		(start, end.max(start.saturating_add(1).min(target_size)))
	};
	let (start_x, end_x) = axis(bounds[0], bounds[2], target.width());
	let (start_y, end_y) = axis(bounds[1], bounds[3], target.height());
	UiBlurDispatchRegion {
		origin: [start_x, start_y],
		extent: Extent::rectangle(end_x - start_x, end_y - start_y),
	}
}

// Plans the full-resolution producer regions backward from the composite
// footprint using the fixed 22-texel Gaussian support. The orthogonal one-texel
// pad covers normalized-UV roundoff around a nominal bilinear texel center.
fn blur_full_dispatch_regions(bounds: [f32; 4], viewport: Extent) -> UiBlurPathRegions {
	let vertical = blur_composite_region(bounds, viewport, 1);
	let horizontal = vertical.expanded(1, UI_BLUR_GAUSSIAN_SUPPORT, viewport);
	UiBlurPathRegions { horizontal, vertical }
}

// Plans the half-resolution stages backward through the eight-read tent and
// both Gaussian axes. Each producer also keeps one orthogonal texel because a
// normalized center coordinate can round onto both bilinear neighbors.
fn blur_half_dispatch_regions(bounds: [f32; 4], viewport: Extent) -> UiBlurHalfPathRegions {
	let target = blur_half_extent(viewport);
	let vertical = blur_composite_region(bounds, target, UI_BLUR_HALF_DOWNSCALE).expanded(1, 1, target);
	let horizontal = vertical.expanded(1, UI_BLUR_GAUSSIAN_SUPPORT, target);
	let downsample = horizontal.expanded(UI_BLUR_GAUSSIAN_SUPPORT, 1, target);
	UiBlurHalfPathRegions {
		downsample,
		filter: UiBlurPathRegions { horizontal, vertical },
	}
}

// Reads the dispatch contract persisted beside a production compute shader.
fn blur_shader_workgroup(shader: &crate::rendering::shader_store::LoadedShader, name: &str) -> Extent {
	assert!(
		matches!(shader.stage, ResourceShaderTypes::Compute),
		"Invalid {name} shader stage. The most likely cause is incorrect BESL sidecar metadata."
	);
	let (width, height, depth) = shader
		.interface
		.workgroup_size
		.unwrap_or_else(|| panic!("Missing {name} workgroup. The most likely cause is an incomplete BESL compute sidecar."));
	Extent::new(width, height, depth)
}

fn draw_clip_from_geometry(clip: Option<Geometry>) -> Option<DrawClip> {
	clip.map(|clip| DrawClip {
		position: [clip.x(), clip.y()],
		size: [clip.width(), clip.height()],
	})
}

fn draw_feather_mask_from_layout(mask: Option<FeatherMask>) -> Option<DrawFeatherMask> {
	mask.map(|mask| DrawFeatherMask {
		position: [mask.geometry.x(), mask.geometry.y()],
		size: [mask.geometry.width(), mask.geometry.height()],
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

	draw_list.layout_size = [root_size.x(), root_size.y()];
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
				position: [position.x(), position.y()],
				size: [size.x(), size.y()],
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
				position: [position.x(), position.y()],
				size: [size.x(), size.y()],
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
				position: [position.x(), position.y()],
				size: [size.x(), size.y()],
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
			position: [image.position.x(), image.position.y()],
			size: [image.size.x(), image.size.y()],
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
			position: [text.position.x(), text.position.y()],
			size: [text.size.x(), text.size.y()],
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
				blur_resolution_mix: 0.0,
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
				blur_resolution_mix: 0.0,
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
				blur_resolution_mix: 0.0,
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
				blur_resolution_mix: 0.0,
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
		let x0 = x0.clamp(0.0, viewport_width);
		let y0 = y0.clamp(0.0, viewport_height);
		let x1 = x1.clamp(0.0, viewport_width);
		let y1 = y1.clamp(0.0, viewport_height);
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
		let effective_radius = (blur.radius * radius_scale).clamp(0.0, 64.0);
		let sigma_pixels = blur_sigma(effective_radius);
		let resolution_mix = blur_resolution_mix(sigma_pixels);
		let full_kernel = UiBlurKernel::gaussian(sigma_pixels);
		let half_kernel = UiBlurKernel::gaussian(blur_half_sigma(sigma_pixels));
		let full_regions = blur_full_dispatch_regions([x0, y0, x1, y1], viewport);
		let half_regions = blur_half_dispatch_regions([x0, y0, x1, y1], viewport);

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
				blur_resolution_mix: resolution_mix,
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
				blur_resolution_mix: resolution_mix,
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
				blur_resolution_mix: resolution_mix,
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
				blur_resolution_mix: resolution_mix,
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
			resolution_mix,
			full_kernel,
			half_kernel,
			full_regions,
			half_regions,
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
	image_sampler: ghi::SamplerHandle,
	image_textures: HashMap<u64, UiImageTexture>,
	text_pipeline: ghi::PipelineHandle,
	text_sampler: ghi::SamplerHandle,
	text_overlays: Vec<UiTextOverlayTexture>,
	blur_downsample_pipeline: ghi::PipelineHandle,
	blur_filter_pipeline: ghi::PipelineHandle,
	blur_downsample_workgroup: Extent,
	blur_filter_workgroup: Extent,
	blur_composite_pipeline: ghi::PipelineHandle,
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

		let blur_downsample_shader = render_pass_builder
			.load_shader(UI_BLUR_DOWNSAMPLE_SHADER_ID, "UI Backdrop Blur Downsample Shader")
			.expect(
				"Failed to load the UI backdrop downsample shader. The most likely cause is that the BESL asset was not baked.",
			);
		let blur_filter_shader = render_pass_builder
			.load_shader(UI_BLUR_FILTER_SHADER_ID, "UI Backdrop Blur Filter Shader")
			.expect(
				"Failed to load the UI backdrop filter shader. The most likely cause is that the BESL asset was not baked.",
			);
		let blur_composite_shader = render_pass_builder
			.load_shader(UI_BLUR_COMPOSITE_SHADER_ID, "UI Backdrop Blur Composite Shader")
			.expect(
				"Failed to load the UI backdrop composite shader. The most likely cause is that the BESL asset was not baked.",
			);
		let blur_downsample_workgroup = blur_shader_workgroup(&blur_downsample_shader, "UI backdrop downsample");
		let blur_filter_workgroup = blur_shader_workgroup(&blur_filter_shader, "UI backdrop filter");
		assert!(
			matches!(blur_composite_shader.stage, ResourceShaderTypes::Fragment),
			"Invalid UI backdrop composite shader stage. The most likely cause is incorrect BESL sidecar metadata."
		);

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
		let image_vertex_shader = create_image_vertex_shader(context);
		let image_fragment_shader = create_image_fragment_shader(context);
		let image_shaders = [
			ghi::ShaderParameter::new(&image_vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&image_fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let image_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
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
		let text_pipeline =
			context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(&[], &[], &text_shaders, &attachments));
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
		let blur_downsample_pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[ghi::pipelines::PushConstantRange::new(
				0,
				UI_BLUR_DOWNSAMPLE_PUSH_CONSTANT_SIZE,
			)],
			ghi::ShaderParameter::new(&blur_downsample_shader.handle, ghi::ShaderTypes::Compute),
		));
		let blur_filter_pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[ghi::pipelines::PushConstantRange::new(0, UI_BLUR_FILTER_PUSH_CONSTANT_SIZE)],
			ghi::ShaderParameter::new(&blur_filter_shader.handle, ghi::ShaderTypes::Compute),
		));
		let blur_composite_pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[],
			&UI_VERTEX_LAYOUT,
			&[
				ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
				ghi::ShaderParameter::new(&blur_composite_shader.handle, ghi::ShaderTypes::Fragment),
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
			image_sampler,
			image_textures: HashMap::new(),
			text_pipeline,
			text_sampler,
			text_overlays: vec![UiTextOverlayTexture {
				image: text_overlay.into(),
				descriptor_set: {
					let descriptor_set = context.create_descriptor_set(Some("UI Text"));
					context.write(&[ghi::DescriptorWrite::combined_image_sampler(
						descriptor_set,
						TEXT_OVERLAY_BINDING.slot(),
						text_overlay,
						text_sampler,
						ghi::Layouts::Read,
					)]);
					descriptor_set
				},
			}],
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
		let blur_downsample_pipeline = self.blur_downsample_pipeline;
		let blur_filter_pipeline = self.blur_filter_pipeline;
		let blur_downsample_workgroup = self.blur_downsample_workgroup;
		let blur_filter_workgroup = self.blur_filter_workgroup;
		let blur_composite_pipeline = self.blur_composite_pipeline;
		let blur_vertex_buffer = self.blur_vertex_buffer;
		let blur_index_buffer = self.blur_index_buffer;
		let blur_half_downsample_descriptor_set = self.blur_half_downsample_descriptor_set;
		let blur_full_x_descriptor_set = self.blur_full_x_descriptor_set;
		let blur_full_y_descriptor_set = self.blur_full_y_descriptor_set;
		let blur_half_x_descriptor_set = self.blur_half_x_descriptor_set;
		let blur_half_y_descriptor_set = self.blur_half_y_descriptor_set;
		let blur_composite_descriptor_set = self.blur_composite_descriptor_set;
		let main_attachment = self.main_attachment;
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
			},
		))
	}

	fn bypass<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		None
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
		forward("out_blur_resolution_mix", "in_blur_resolution_mix"),
		ParserNode::member_assignment(
			"out_screen_uv",
			ParserNode::call(
				"vec2f",
				vec![
					ParserNode::operator(
						"+",
						ParserNode::operator(
							"*",
							ParserNode::accessor(member("in_position"), member("x")),
							ParserNode::literal_expression("0.5"),
						),
						ParserNode::literal_expression("0.5"),
					),
					ParserNode::operator(
						"-",
						ParserNode::literal_expression("0.5"),
						ParserNode::operator(
							"*",
							ParserNode::accessor(member("in_position"), member("y")),
							ParserNode::literal_expression("0.5"),
						),
					),
				],
			),
		),
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
			ParserNode::input("in_blur_resolution_mix", "f32", 13),
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
			ParserNode::output("out_screen_uv", "vec2f", 12),
			ParserNode::output("out_blur_resolution_mix", "f32", 13),
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
		[TEXT_OVERLAY_BINDING],
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
		[UI_IMAGE_BINDING],
	)
	.expect("Failed to create the UI image fragment shader. The most likely cause is an incompatible shader interface.")
}

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
	use std::mem::{align_of, offset_of, size_of};

	use besl::vm::{
		builtin_position_slot, input_slot, output_slot, Buffer, DescriptorBindings, ExecutableProgram, Texture, Value,
	};
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::{Generator as _, ShaderGenerationSettings},
	};
	use utils::{Extent, RGBA};

	use super::{
		blur_composite_region, blur_full_dispatch_regions, blur_half_dispatch_regions, blur_half_extent, blur_half_sigma,
		blur_resolution_mix, blur_sigma, blur_uses_full_resolution, blur_uses_half_resolution, build_ui_blur_geometry,
		build_ui_curve_geometry, build_ui_geometry, build_ui_image_geometry, flatten_curve_segment, should_draw_image,
		should_rasterize_text, update_from_render, DrawClip, DrawFeatherMask, UiBlurDispatchRegion, UiBlurDrawElement,
		UiBlurFilterPush, UiBlurKernel, UiCurveDrawElement, UiDrawBatch, UiDrawElement, UiDrawList, UiImageDrawElement,
		UiTextDrawElement, MAX_UI_ELEMENTS, MAX_UI_VERTICES_PER_DRAW, UI_BLUR_GAUSSIAN_PAIR_COUNT, UI_BLUR_GAUSSIAN_SUPPORT,
		UI_BLUR_HALF_DOWNSCALE, UI_INDICES_PER_CURVE_SPAN, UI_INDICES_PER_ELEMENT, UI_VERTICES_PER_CURVE_SPAN,
		UI_VERTICES_PER_ELEMENT,
	};
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, compile as compile_shader_vm, empty_image, rgba, run_at, texture_2d},
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

	const UI_BLUR_DOWNSAMPLE_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-downsample.besl");
	const UI_BLUR_FILTER_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-filter.besl");
	const UI_BLUR_COMPOSITE_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-composite.besl");

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
				panic!("Failed to initialize blur filter field `{name}`: {error}. The most likely cause is a changed push constant type.")
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

	// Exercises every production backend before platform-specific shader baking
	// so one portable blur asset cannot silently drift on an inactive backend.
	fn assert_ui_blur_shader_lowers(source: &str, settings: &ShaderGenerationSettings, name: &str) {
		let main = simple_compute::compile_test_program(source);
		GLSLShaderGenerator::new()
			.generate(settings, &main)
			.unwrap_or_else(|error| panic!("Failed to lower {name} to GLSL: {error:?}"));
		HLSLShaderGenerator::new()
			.generate(settings, &main)
			.unwrap_or_else(|error| panic!("Failed to lower {name} to HLSL: {error:?}"));
		MSLShaderGenerator::new()
			.generate(settings, &main)
			.unwrap_or_else(|error| panic!("Failed to lower {name} to MSL: {error:?}"));
	}

	/// Verifies every checked-in blur stage lowers from one portable BESL source to all production backends.
	#[test]
	fn backdrop_blur_besl_lowers_for_every_backend() {
		let compute = ShaderGenerationSettings::compute(Extent::square(16));
		assert_ui_blur_shader_lowers(UI_BLUR_DOWNSAMPLE_BESL, &compute, "UI backdrop downsample");
		assert_ui_blur_shader_lowers(UI_BLUR_FILTER_BESL, &compute, "UI backdrop filter");
		assert_ui_blur_shader_lowers(
			UI_BLUR_COMPOSITE_BESL,
			&ShaderGenerationSettings::fragment(),
			"UI backdrop composite",
		);
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
			(1, "in_pixel_position", Value::Vec2F(pixel_position)),
			(2, "in_local_position", Value::Vec2F([1.0, 1.0])),
			(3, "in_rect_size", Value::Vec2F([2.0, 2.0])),
			(4, "in_corner_radius", Value::F32(0.0)),
			(5, "in_corner_exponent", Value::F32(2.0)),
			(8, "in_feather_mask_position", Value::Vec2F([0.0, 0.0])),
			(9, "in_feather_mask_size", Value::Vec2F([8.0, 4.0])),
			(10, "in_feather_mask_edges", Value::Vec4F(feather_edges)),
			(13, "in_blur_resolution_mix", Value::F32(resolution_mix)),
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
			.read("out_color_attachment")
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
						assert!(output
							.iter()
							.zip(input)
							.any(|(actual, source)| (actual[0] - source[0]).abs() > 1e-5));
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
	fn primary_ui_besl_shaders_build_besl_programs() {
		let vertex_main = super::create_ui_vertex_program();
		let fragment_main = super::create_ui_fragment_program();

		assert!(matches!(vertex_main.borrow().node(), besl::Nodes::Function { .. }));
		assert!(matches!(fragment_main.borrow().node(), besl::Nodes::Function { .. }));
	}

	/// Verifies the production UI vertex shader preserves every geometry and styling varying.
	#[test]
	fn ui_vertex_besl_vm_forwards_position_and_varyings() {
		let executable = ExecutableProgram::compile(super::create_ui_vertex_program())
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
				"out_screen_uv",
				"out_blur_resolution_mix",
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
				Value::Vec2F([0.625, 0.875]),
				Value::F32(0.375),
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
