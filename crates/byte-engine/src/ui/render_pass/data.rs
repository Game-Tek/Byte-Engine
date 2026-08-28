//! UI draw-list data, blur planning, and text-overlay preparation.

use super::*;

pub(super) const MAIN_ATTACHMENT_FORMAT: ghi::Formats = crate::rendering::SCENE_COLOR_FORMAT;
pub(super) const TEXT_OVERLAY_FORMAT: ghi::Formats = ghi::Formats::RGBA8UNORM;
pub(super) const TEXT_OVERLAY_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_IMAGE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_BLUR_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_BLUR_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
pub(super) const UI_BLUR_FULL_COMPOSITE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_BLUR_HALF_COMPOSITE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_BLUR_HALF_DOWNSCALE: u32 = 2;
pub(super) const UI_BLUR_GAUSSIAN_SUPPORT: u32 = 22;
pub(super) const UI_BLUR_GAUSSIAN_PAIR_COUNT: usize = 11;
pub(super) const UI_BLUR_SIGMA_SCALE: f32 = 1.689_394_6;
pub(super) const UI_BLUR_FULL_ONLY_SIGMA: f32 = 4.0;
pub(super) const UI_BLUR_HALF_ONLY_SIGMA: f32 = 6.0;
pub(super) const UI_BLUR_HALF_RESAMPLING_VARIANCE: f32 = 2.75;
pub(super) const UI_BLUR_DOWNSAMPLE_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiBlurDownsamplePush>() as u32;
pub(super) const UI_BLUR_FILTER_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiBlurFilterPush>() as u32;
pub(super) const UI_BLUR_DOWNSAMPLE_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-downsample.besl";
pub(super) const UI_BLUR_FILTER_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-filter.besl";
pub(super) const UI_BLUR_COMPOSITE_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-composite.besl";

pub(super) const UI_VERTICES_PER_ELEMENT: usize = 4;
pub(super) const UI_INDICES_PER_ELEMENT: usize = 6;
pub(super) const UI_VERTICES_PER_CURVE_SPAN: usize = 4;
pub(super) const UI_INDICES_PER_CURVE_SPAN: usize = 6;
pub(super) const MAX_UI_VERTICES_PER_DRAW: usize = u16::MAX as usize + 1;
pub(super) const MAX_UI_ELEMENTS: usize = 65_536;
pub(super) const MAX_UI_IMAGES: usize = MAX_UI_ELEMENTS;
pub(super) const MAX_UI_VERTICES: usize = MAX_UI_ELEMENTS * UI_VERTICES_PER_ELEMENT;
pub(super) const MAX_UI_INDICES: usize = MAX_UI_ELEMENTS * UI_INDICES_PER_ELEMENT;
pub(super) const CURVE_FLATTEN_TOLERANCE_PIXELS: f32 = 0.35;
pub(super) const CURVE_AA_WIDTH_PIXELS: f32 = 1.0;

pub(super) const UI_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 14] = [
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
pub(super) struct UiDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) feather_mask: Option<DrawFeatherMask>,
	pub(super) color: [f32; 4],
	pub(super) corner_radius: f32,
	pub(super) corner_exponent: f32,
	pub(super) layer_kind: LayerKind,
	pub(super) stroke_width: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UiBlurDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) feather_mask: Option<DrawFeatherMask>,
	pub(super) color: [f32; 4],
	pub(super) corner_radius: f32,
	pub(super) corner_exponent: f32,
	pub(super) radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct UiTextDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) feather_mask: Option<DrawFeatherMask>,
	pub(super) color: RGBA,
	pub(super) font_size: f32,
	pub(super) text: String,
}

#[derive(Debug, Clone)]
pub(super) struct UiImageDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) image_id: u64,
	pub(super) version: u64,
	pub(super) source_width: u32,
	pub(super) source_height: u32,
	pub(super) pixels: Arc<[u8]>,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) feather_mask: Option<DrawFeatherMask>,
	pub(super) opacity: f32,
}

#[derive(Debug, Clone)]
pub(super) struct UiCurveDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) feather_mask: Option<DrawFeatherMask>,
	pub(super) color: [f32; 4],
	pub(super) stroke_width: f32,
	pub(super) segments: Vec<CurveSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DrawClip {
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DrawFeatherMask {
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) edges: [f32; 4],
	pub(super) corner: [f32; 2],
}

#[derive(Debug, Clone)]
pub(super) struct UiDrawList {
	pub(super) layout_size: [f32; 2],
	pub(super) elements: Vec<UiDrawElement>,
	pub(super) blurs: Vec<UiBlurDrawElement>,
	pub(super) curves: Vec<UiCurveDrawElement>,
	pub(super) images: Vec<UiImageDrawElement>,
	pub(super) texts: Vec<UiTextDrawElement>,
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
pub(super) struct UiVertex {
	pub(super) position: [f32; 2],
	pub(super) pixel_position: [f32; 2],
	pub(super) local_position: [f32; 2],
	pub(super) rect_size: [f32; 2],
	pub(super) color: [f32; 4],
	pub(super) corner_radius: f32,
	pub(super) corner_exponent: f32,
	pub(super) layer_kind: f32,
	pub(super) stroke_width: f32,
	pub(super) feather_mask_position: [f32; 2],
	pub(super) feather_mask_size: [f32; 2],
	pub(super) feather_mask_edges: [f32; 4],
	pub(super) feather_mask_corner: [f32; 2],
	pub(super) blur_resolution_mix: f32,
}

pub(super) const UI_IMAGE_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 7] = [
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
pub(super) struct UiImageVertex {
	pub(super) position: [f32; 2],
	pub(super) uv: [f32; 2],
	pub(super) opacity: f32,
	pub(super) feather_mask_position: [f32; 2],
	pub(super) feather_mask_size: [f32; 2],
	pub(super) feather_mask_edges: [f32; 4],
	pub(super) feather_mask_corner: [f32; 2],
}

pub(super) const UI_CURVE_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 10] = [
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
pub(super) struct UiCurveVertex {
	pub(super) position: [f32; 2],
	pub(super) pixel_position: [f32; 2],
	pub(super) segment_from: [f32; 2],
	pub(super) segment_to: [f32; 2],
	pub(super) color: [f32; 4],
	pub(super) half_width: f32,
	pub(super) feather_mask_position: [f32; 2],
	pub(super) feather_mask_size: [f32; 2],
	pub(super) feather_mask_edges: [f32; 4],
	pub(super) feather_mask_corner: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiDrawBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) index_count: u32,
	pub(super) first_index: u32,
	pub(super) vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiImageDrawBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) image_id: u64,
	pub(super) version: u64,
	pub(super) index_count: u32,
	pub(super) first_index: u32,
	pub(super) vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiCurveDrawBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) index_count: u32,
	pub(super) first_index: u32,
	pub(super) vertex_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiPreparedImageBatch {
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	pub(super) batch: UiImageDrawBatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiPreparedTextBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
}

/// The `UiBlurDispatchRegion` struct limits one compute stage to the padded part of the blur target it must produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiBlurDispatchRegion {
	pub(super) origin: [u32; 2],
	pub(super) extent: Extent,
}

/// The `UiBlurDownsamplePush` struct carries one regional half-resolution dispatch to the production shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiBlurDownsamplePush {
	pub(super) origin: [u32; 2],
	pub(super) extent: [u32; 2],
}

/// The `UiBlurFilterPush` struct keeps the complete Gaussian kernel and dispatch region in one aligned GPU record.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiBlurFilterPush {
	pub(super) filter_data: [f32; 4],
	pub(super) origin: [u32; 2],
	pub(super) extent: [u32; 2],
	pub(super) pair_weights_0_3: [f32; 4],
	pub(super) pair_weights_4_7: [f32; 4],
	pub(super) pair_weights_8_10_pad: [f32; 4],
	pub(super) pair_offsets_0_3: [f32; 4],
	pub(super) pair_offsets_4_7: [f32; 4],
	pub(super) pair_offsets_8_10_pad: [f32; 4],
}

/// The `UiBlurKernel` struct stores one normalized Gaussian without allocating transient coefficient buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiBlurKernel {
	pub(super) center_weight: f32,
	pub(super) pair_weights: [f32; UI_BLUR_GAUSSIAN_PAIR_COUNT],
	pub(super) pair_offsets: [f32; UI_BLUR_GAUSSIAN_PAIR_COUNT],
}

impl UiBlurKernel {
	// Generates the normalized integer taps first, then packs adjacent positive
	// taps for bilinear filtering without scaling their weighted offsets.
	pub(super) fn gaussian(sigma: f32) -> Self {
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
	pub(super) fn push(self, direction: [f32; 2], region: UiBlurDispatchRegion) -> UiBlurFilterPush {
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
pub(super) struct UiBlurPathRegions {
	pub(super) horizontal: UiBlurDispatchRegion,
	pub(super) vertical: UiBlurDispatchRegion,
}

/// The `UiBlurHalfPathRegions` struct adds the binomial prefilter region needed by the half-resolution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiBlurHalfPathRegions {
	pub(super) downsample: UiBlurDispatchRegion,
	pub(super) filter: UiBlurPathRegions,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiPreparedBlurBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) index_count: u32,
	pub(super) first_index: u32,
	pub(super) vertex_offset: i32,
	pub(super) resolution_mix: f32,
	pub(super) full_kernel: UiBlurKernel,
	pub(super) half_kernel: UiBlurKernel,
	pub(super) full_regions: UiBlurPathRegions,
	pub(super) half_regions: UiBlurHalfPathRegions,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum UiPreparedBatch {
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

pub(super) fn sort_prepared_batches(batches: &mut [UiPreparedBatch]) {
	batches.sort_by_key(|batch| (batch.depth(), batch.order()));
}

#[derive(Debug)]
pub(super) struct UiGeometry<'a> {
	pub(super) vertices: Vec<UiVertex, &'a bumpalo::Bump>,
	pub(super) indices: Vec<u16, &'a bumpalo::Bump>,
	pub(super) batches: Vec<UiDrawBatch, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
}

#[derive(Debug)]
pub(super) struct UiBlurGeometry<'a> {
	pub(super) vertices: Vec<UiVertex, &'a bumpalo::Bump>,
	pub(super) indices: Vec<u16, &'a bumpalo::Bump>,
	pub(super) batches: Vec<UiPreparedBlurBatch, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
}

#[derive(Debug)]
pub(super) struct UiImageGeometry<'a> {
	pub(super) vertices: Vec<UiImageVertex, &'a bumpalo::Bump>,
	pub(super) indices: Vec<u16, &'a bumpalo::Bump>,
	pub(super) batches: Vec<UiImageDrawBatch, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
}

#[derive(Debug)]
pub(super) struct UiCurveGeometry<'a> {
	pub(super) vertices: Vec<UiCurveVertex, &'a bumpalo::Bump>,
	pub(super) indices: Vec<u16, &'a bumpalo::Bump>,
	pub(super) batches: Vec<UiCurveDrawBatch, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
}

pub(super) struct UiImageTexture {
	pub(super) version: u64,
	pub(super) extent: (u32, u32),
	pub(super) image: ghi::BaseImageHandle,
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
}

pub(super) struct UiTextOverlayTexture {
	pub(super) image: ghi::BaseImageHandle,
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
}

// Whether text rasterization should be ommitted if text is empty, 0 sized in any dimension or if fully transparent
pub(super) fn should_rasterize_text(text: &UiTextDrawElement) -> bool {
	!text.text.is_empty() && text.color.a > 0.0 && text.size[0] > 0.0 && text.size[1] > 0.0
}

pub(super) fn resolved_corner_radius(radius: f32, rect_width: f32, rect_height: f32) -> f32 {
	radius.max(0.0).min(rect_width.min(rect_height) * 0.5)
}

pub(super) fn resolved_corner_exponent(exponent: f32) -> f32 {
	if !exponent.is_finite() || exponent < 1.0 {
		2.0
	} else {
		exponent.clamp(1.0, 8.0)
	}
}

pub(super) fn layer_kind_value(kind: LayerKind) -> f32 {
	match kind {
		LayerKind::Fill => 0.0,
		LayerKind::Stroke { .. } => 1.0,
	}
}

pub(super) fn stroke_width(kind: LayerKind) -> f32 {
	match kind {
		LayerKind::Fill => 0.0,
		LayerKind::Stroke { width } if width.is_finite() && width > 0.0 => width,
		LayerKind::Stroke { .. } => 0.0,
	}
}

pub(super) fn backdrop_blur_radius(radius: f32) -> f32 {
	if radius.is_finite() { radius.clamp(0.0, 64.0) } else { 0.0 }
}

// Preserves the legacy repeated-blur strength by mapping its variance-domain
// radius to the standard deviation of one Gaussian.
pub(super) fn blur_sigma(radius_pixels: f32) -> f32 {
	UI_BLUR_SIGMA_SCALE * radius_pixels.clamp(0.0, 64.0).sqrt()
}

// Removes the half-resolution prefilter and reconstruction variance before
// converting the remaining full-resolution variance to the half lattice.
pub(super) fn blur_half_sigma(sigma_pixels: f32) -> f32 {
	0.5 * (sigma_pixels * sigma_pixels - UI_BLUR_HALF_RESAMPLING_VARIANCE)
		.max(0.0)
		.sqrt()
}

// Blends continuously between the full and half paths while leaving their
// quality-stable ranges at exactly zero and one.
pub(super) fn blur_resolution_mix(sigma_pixels: f32) -> f32 {
	let t = ((sigma_pixels - UI_BLUR_FULL_ONLY_SIGMA) / (UI_BLUR_HALF_ONLY_SIGMA - UI_BLUR_FULL_ONLY_SIGMA)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

pub(super) fn blur_uses_full_resolution(resolution_mix: f32) -> bool {
	resolution_mix < 1.0
}

pub(super) fn blur_uses_half_resolution(resolution_mix: f32) -> bool {
	resolution_mix > 0.0
}

// Keeps partial edge texels when an odd full-resolution dimension maps to the
// fixed two-pixel half-resolution lattice.
pub(super) fn blur_half_extent(extent: Extent) -> Extent {
	Extent::rectangle(
		extent.width().div_ceil(UI_BLUR_HALF_DOWNSCALE).max(1),
		extent.height().div_ceil(UI_BLUR_HALF_DOWNSCALE).max(1),
	)
}

impl UiBlurDispatchRegion {
	// Expands one region without crossing the selected blur target's edges.
	pub(super) fn expanded(self, horizontal: u32, vertical: u32, target: Extent) -> Self {
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

	pub(super) fn push_extent(self) -> [u32; 2] {
		[self.extent.width(), self.extent.height()]
	}
}

// Converts screen bounds through a fixed full- or half-resolution lattice.
// It never derives UV scale from ceil-divided image dimensions, which keeps odd
// viewport widths phase-aligned with the composite shader.
pub(super) fn blur_composite_region(bounds: [f32; 4], target: Extent, downscale: u32) -> UiBlurDispatchRegion {
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
pub(super) fn blur_full_dispatch_regions(bounds: [f32; 4], viewport: Extent) -> UiBlurPathRegions {
	let vertical = blur_composite_region(bounds, viewport, 1);
	let horizontal = vertical.expanded(1, UI_BLUR_GAUSSIAN_SUPPORT, viewport);
	UiBlurPathRegions { horizontal, vertical }
}

// Plans the half-resolution stages backward through the eight-read tent and
// both Gaussian axes. Each producer also keeps one orthogonal texel because a
// normalized center coordinate can round onto both bilinear neighbors.
pub(super) fn blur_half_dispatch_regions(bounds: [f32; 4], viewport: Extent) -> UiBlurHalfPathRegions {
	let target = blur_half_extent(viewport);
	let vertical = blur_composite_region(bounds, target, UI_BLUR_HALF_DOWNSCALE).expanded(1, 1, target);
	let horizontal = vertical.expanded(1, UI_BLUR_GAUSSIAN_SUPPORT, target);
	let downsample = horizontal.expanded(UI_BLUR_GAUSSIAN_SUPPORT, 1, target);
	UiBlurHalfPathRegions {
		downsample,
		filter: UiBlurPathRegions { horizontal, vertical },
	}
}

pub(super) fn draw_clip_from_geometry(clip: Option<Geometry>) -> Option<DrawClip> {
	clip.map(|clip| DrawClip {
		position: [clip.x(), clip.y()],
		size: [clip.width(), clip.height()],
	})
}

pub(super) fn draw_feather_mask_from_layout(mask: Option<FeatherMask>) -> Option<DrawFeatherMask> {
	mask.map(|mask| DrawFeatherMask {
		position: [mask.geometry.x(), mask.geometry.y()],
		size: [mask.geometry.width(), mask.geometry.height()],
		edges: [mask.feather.top, mask.feather.right, mask.feather.bottom, mask.feather.left],
		corner: [mask.corner_radius, mask.corner_exponent],
	})
}

pub(super) fn scaled_feather_mask(mask: Option<DrawFeatherMask>, sx: f32, sy: f32) -> DrawFeatherMask {
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

pub(super) fn update_from_render(render: &engine::Render, draw_list: &mut UiDrawList) {
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

pub(super) fn should_draw_image(image: &UiImageDrawElement) -> bool {
	image.source_width > 0
		&& image.source_height > 0
		&& image.pixels.len() == image.source_width as usize * image.source_height as usize * 4
		&& image.size[0] > 0.0
		&& image.size[1] > 0.0
		&& image.opacity > 0.0
}

/// Rasterizes all visible text elements into the UI overlay texture for the current viewport.
pub(super) fn rasterize_text_overlay(
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
