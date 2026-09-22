//! UI draw-list data, blur planning, and text-overlay preparation.

use super::*;

pub(super) const MAIN_ATTACHMENT_FORMAT: ghi::Formats = crate::rendering::SCENE_COLOR_FORMAT;
/// Bindings of the UI ubershader. They match `ui-vertex.besl` and `ui-fragment.besl`.
pub(super) const UI_PRIMITIVES_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
pub(super) const UI_MASKS_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);
pub(super) const UI_GLYPH_CURVES_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(2);
pub(super) const UI_GLYPH_BANDS_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(3);
/// The full and half resolution blurred backdrops a backdrop blur primitive samples.
pub(super) const UI_BLUR_FULL_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(4);
pub(super) const UI_BLUR_HALF_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(5);
pub(super) const UI_TEXTURES_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(6);
/// Elements of the ubershader's texture array. The first is the glyph atlas and the rest hold images.
pub(super) const UI_TEXTURE_SLOTS: u32 = 32;
pub(super) const UI_ATLAS_TEXTURE_SLOT: u32 = 0;
pub(super) const UI_FIRST_IMAGE_TEXTURE_SLOT: u32 = 1;
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
pub(super) const UI_BLUR_HALF_DOWNSCALE: u32 = 2;
pub(super) const UI_BLUR_GAUSSIAN_SUPPORT: u32 = 22;
pub(super) const UI_BLUR_GAUSSIAN_PAIR_COUNT: usize = 11;
pub(super) const UI_BLUR_SIGMA_SCALE: f32 = 1.689_394_6;
pub(super) const UI_BLUR_FULL_ONLY_SIGMA: f32 = 4.0;
pub(super) const UI_BLUR_HALF_ONLY_SIGMA: f32 = 6.0;
pub(super) const UI_BLUR_HALF_RESAMPLING_VARIANCE: f32 = 2.75;
pub(super) const UI_BLUR_DOWNSAMPLE_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiRegionPush>() as u32;
/// Pixels added around every damaged rectangle for anti-aliasing and glyph atlas padding.
pub(super) const UI_DAMAGE_MARGIN_PIXELS: f32 = 4.0;
/// Pixels a backdrop blur reads around its quad through both the full and half resolution paths.
pub(super) const UI_BLUR_FOOTPRINT_MARGIN: u32 = UI_BLUR_GAUSSIAN_SUPPORT * UI_BLUR_HALF_DOWNSCALE + 8;
/// Every draw is repeated once per damage region, so keep the list short.
pub(super) const MAX_UI_DAMAGE_REGIONS: usize = 4;
/// Damage covering this share of the viewport becomes one full redraw.
pub(super) const UI_FULL_REDRAW_AREA_SHARE: f32 = 0.6;
/// Workgroup edge of the region clear and backdrop resolve compute shaders.
pub(super) const UI_REGION_WORKGROUP: u32 = 16;
pub(super) const UI_BLUR_FILTER_PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<UiBlurFilterPush>() as u32;
pub(super) const UI_BLUR_DOWNSAMPLE_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-downsample.besl";
pub(super) const UI_BLUR_FILTER_SHADER_ID: &str = "byte-engine/rendering/ui/backdrop-blur-filter.besl";

/// Vertices the ubershader pulls per primitive: two triangles with no index buffer.
pub(super) const UI_VERTICES_PER_PRIMITIVE: u32 = 6;
/// Records in the primitive buffer. Rectangle layers, glyphs, images, blurs, and curve pieces all share it.
pub(super) const MAX_UI_PRIMITIVES: usize = 1 << 17;
/// Distinct clip and mask combinations one frame can reference.
pub(super) const MAX_UI_MASKS: usize = 1 << 12;
/// Largest distance between a cubic slice and the quadratic the shader draws in its place.
pub(super) const CURVE_QUADRATIC_TOLERANCE_PIXELS: f32 = 0.05;
/// Largest distance a curve piece may bulge from its chord, which keeps its bounding box tight.
pub(super) const CURVE_PIECE_BULGE_PIXELS: f32 = 4.0;
pub(super) const MAX_CURVE_PIECES: u32 = 64;
pub(super) const CURVE_AA_WIDTH_PIXELS: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) clip_mask: Option<DrawClipMask>,
	pub(super) color: [f32; 4],
	pub(super) corner_radius: f32,
	pub(super) corner_exponent: f32,
	pub(super) sector: Option<Sector>,
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
	pub(super) clip_mask: Option<DrawClipMask>,
	pub(super) color: [f32; 4],
	pub(super) corner_radius: f32,
	pub(super) corner_exponent: f32,
	pub(super) sector: Option<Sector>,
	pub(super) radius: f32,
}

#[derive(Debug, PartialEq)]
pub(super) struct UiTextDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) clip_mask: Option<DrawClipMask>,
	pub(super) color: RGBA,
	pub(super) font_size: f32,
	pub(super) text: String,
}

impl Clone for UiTextDrawElement {
	/// Copies this surface and its owned local data.
	fn clone(&self) -> Self {
		Self {
			depth: self.depth,
			order: self.order,
			position: self.position,
			size: self.size,
			clip: self.clip,
			clip_mask: self.clip_mask,
			color: self.color,
			font_size: self.font_size,
			text: self.text.clone(),
		}
	}
	/// Reuses owned storage when a surface changes.
	fn clone_from(&mut self, source: &Self) {
		self.depth = source.depth;
		self.order = source.order;
		self.position = source.position;
		self.size = source.size;
		self.clip = source.clip;
		self.clip_mask = source.clip_mask;
		self.color = source.color;
		self.font_size = source.font_size;
		self.text.clone_from(&source.text);
	}
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
	pub(super) clip_mask: Option<DrawClipMask>,
	pub(super) opacity: f32,
}

#[derive(Debug, PartialEq)]
pub(super) struct UiCurveDrawElement {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) clip: Option<DrawClip>,
	pub(super) clip_mask: Option<DrawClipMask>,
	pub(super) color: [f32; 4],
	pub(super) stroke_width: f32,
	pub(super) segments: Vec<CurveSegment>,
}

impl Clone for UiCurveDrawElement {
	/// Copies this surface and its owned local data.
	fn clone(&self) -> Self {
		Self {
			depth: self.depth,
			order: self.order,
			position: self.position,
			size: self.size,
			clip: self.clip,
			clip_mask: self.clip_mask,
			color: self.color,
			stroke_width: self.stroke_width,
			segments: self.segments.clone(),
		}
	}
	/// Reuses owned storage when a surface changes.
	fn clone_from(&mut self, source: &Self) {
		self.depth = source.depth;
		self.order = source.order;
		self.position = source.position;
		self.size = source.size;
		self.clip = source.clip;
		self.clip_mask = source.clip_mask;
		self.color = source.color;
		self.stroke_width = source.stroke_width;
		self.segments.clone_from(&source.segments);
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DrawClip {
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DrawClipMask {
	pub(super) position: [f32; 2],
	pub(super) size: [f32; 2],
	pub(super) edges: [f32; 4],
	pub(super) corner: [f32; 2],
	/// The turn the element draws with, in layout units. A zero size keeps the turn without masking.
	pub(super) rotation: Rotation,
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

impl UiDrawList {
	pub(super) fn is_empty(&self) -> bool {
		self.elements.is_empty()
			&& self.blurs.is_empty()
			&& self.curves.is_empty()
			&& self.images.is_empty()
			&& self.texts.is_empty()
	}
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

/// How the ubershader shades a primitive. The values match `ui-fragment.besl`.
pub(super) const UI_KIND_RECT: u32 = 0;
pub(super) const UI_KIND_BLUR: u32 = 1;
pub(super) const UI_KIND_CURVE: u32 = 2;
pub(super) const UI_KIND_IMAGE: u32 = 3;
pub(super) const UI_KIND_SLUG_GLYPH: u32 = 4;
pub(super) const UI_KIND_ATLAS_GLYPH: u32 = 5;
pub(super) const UI_KIND_SECTOR: u32 = 6;
pub(super) const UI_KIND_SECTOR_BLUR: u32 = 7;
/// Fixed point steps per pixel of a sector primitive's edge inset in `data0`.
pub(super) const SECTOR_INSET_SCALE: f32 = 256.0;
/// Curve piece flags: the piece's segment rounds off its start or its end.
pub(super) const UI_CURVE_CAP_START: u32 = 1;
pub(super) const UI_CURVE_CAP_END: u32 = 2;

/// The `UiPrimitive` struct is one quad of the UI: a rectangle layer, a glyph, an image, a backdrop blur, or a curve piece.
///
/// The vertex shader pulls six vertices from each record, so the UI needs no vertex or index
/// buffer and every primitive between two backdrop blurs shares one draw.
///
/// | Kind | `bounds` | `a` | `b` | `data0` | `data1` |
/// | --- | --- | --- | --- | --- | --- |
/// | Rectangle, blur | clipped quad | unclipped x, y, width, height | corner radius, corner exponent, stroke width, blur resolution mix | | |
/// | Sector, sector blur | clipped quad | unclipped x, y, width, height | inner radius ratio, start angle, sweep angle, stroke width or blur resolution mix | edge inset in 1/256 pixels | |
/// | Curve piece | control points 0 and 1 | control points 2 and 3 | half width | piece, and piece count above bit 16 | cap flags |
/// | Image | clipped quad | texture rectangle | | texture slot | |
/// | Slug glyph | clipped quad | pen x, pen y, pixels per em | band scale and offset | band data location | last horizontal band, and last vertical band above bit 16 |
/// | Atlas glyph | clipped quad | texture rectangle | | | |
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiPrimitive {
	pub(super) bounds: [f32; 4],
	pub(super) color: [f32; 4],
	pub(super) a: [f32; 4],
	pub(super) b: [f32; 4],
	pub(super) kind: u32,
	/// Index into the frame's [`UiMaskTable`]. Zero is no clip and no mask.
	pub(super) mask: u32,
	pub(super) data0: u32,
	pub(super) data1: u32,
}

/// The `UiClipMaskEntry` struct is one hard clip and feathered mask that primitives share by index.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiClipMaskEntry {
	/// Hard clip as x0, y0, x1, y1. Only curves need it; every other quad is trimmed on the CPU.
	/// The clip and the mask are tested before the turn, where the primitive is laid out.
	pub(super) clip: [f32; 4],
	/// Mask rectangle as x, y, width, height. A zero size disables the mask.
	pub(super) rect: [f32; 4],
	pub(super) edges: [f32; 4],
	/// Corner radius and corner exponent.
	pub(super) corner: [f32; 4],
	/// The turn the vertex shader applies to the quad, as cos, sin, x, y in pixels.
	pub(super) rotation: [f32; 4],
}

impl UiClipMaskEntry {
	pub(super) const UNCLIPPED: [f32; 4] = [-1.0e30, -1.0e30, 1.0e30, 1.0e30];
	pub(super) const NONE: Self = Self {
		clip: Self::UNCLIPPED,
		rect: [0.0; 4],
		edges: [0.0; 4],
		corner: [0.0, 2.0, 0.0, 0.0],
		rotation: [1.0, 0.0, 0.0, 0.0],
	};
}

/// The `UiMaskTable` struct deduplicates the clips and masks of one frame's primitives.
///
/// A mask belongs to a container, so most primitives share a handful of entries instead of each
/// carrying its own copy.
pub(super) struct UiMaskTable {
	entries: Vec<UiClipMaskEntry>,
	// Every element of a list can carry its own mask, so the lookup has to stay cheap next to building a primitive.
	indices: utils::hash::HashMap<[u32; 20], u32>,
	pub(super) truncated: bool,
}

impl Default for UiMaskTable {
	fn default() -> Self {
		Self {
			entries: vec![UiClipMaskEntry::NONE],
			indices: utils::hash::HashMap::default(),
			truncated: false,
		}
	}
}

impl UiMaskTable {
	/// Starts a frame while keeping the table's allocations.
	pub(super) fn clear(&mut self) {
		self.entries.truncate(1);
		self.indices.clear();
		self.truncated = false;
	}

	pub(super) fn entries(&self) -> &[UiClipMaskEntry] {
		&self.entries
	}

	/// Returns the index of a layout-space clip and mask in viewport pixels. Pass the clip only for
	/// primitives the CPU does not trim.
	pub(super) fn index(&mut self, clip: Option<DrawClip>, mask: Option<DrawClipMask>, sx: f32, sy: f32) -> u32 {
		if clip.is_none() && mask.is_none() {
			return 0;
		}
		let mask = scaled_clip_mask(mask, sx, sy);
		let entry = UiClipMaskEntry {
			clip: clip.map_or(UiClipMaskEntry::UNCLIPPED, |clip| {
				snapped_rect(clip.position, clip.size, sx, sy)
			}),
			rect: [mask.position[0], mask.position[1], mask.size[0], mask.size[1]],
			edges: mask.edges,
			corner: [mask.corner[0], mask.corner[1], 0.0, 0.0],
			rotation: [mask.rotation.cos, mask.rotation.sin, mask.rotation.x, mask.rotation.y],
		};
		// Siblings share their container's mask, so the last entry is the usual answer.
		if self.entries.len() > 1 && self.entries.last() == Some(&entry) {
			return self.entries.len() as u32 - 1;
		}
		let key: [u32; 20] = bytemuck::cast(entry);
		if let Some(&index) = self.indices.get(&key) {
			return index;
		}
		if self.entries.len() == MAX_UI_MASKS {
			self.truncated = true;
			return 0;
		}
		let index = self.entries.len() as u32;
		self.entries.push(entry);
		self.indices.insert(key, index);
		index
	}
}

/// The `UiDrawPush` struct tells the vertex shader the viewport and which record a draw starts at.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiDrawPush {
	pub(super) viewport: [f32; 2],
	pub(super) first: u32,
	pub(super) padding: u32,
}

/// The `UiPixelRegion` struct is an integer pixel rectangle: a compute dispatch region, a scissor, or damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiPixelRegion {
	pub(super) origin: [u32; 2],
	pub(super) extent: Extent,
}

/// The `UiRegionPush` struct carries one region-limited dispatch to a production shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiRegionPush {
	pub(super) origin: [u32; 2],
	pub(super) extent: [u32; 2],
}

/// The `UiBlurFilterPush` struct keeps the complete Gaussian kernel and dispatch region in one aligned GPU record.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
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
	pub(super) fn push(self, direction: [f32; 2], region: UiPixelRegion) -> UiBlurFilterPush {
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
	pub(super) horizontal: UiPixelRegion,
	pub(super) vertical: UiPixelRegion,
}

/// The `UiBlurHalfPathRegions` struct adds the binomial prefilter region needed by the half-resolution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiBlurHalfPathRegions {
	pub(super) downsample: UiPixelRegion,
	pub(super) filter: UiBlurPathRegions,
}

/// The `UiBlurDispatch` struct is the compute work that fills the blurred backdrop one blur primitive samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct UiBlurDispatch {
	pub(super) resolution_mix: f32,
	pub(super) full_kernel: UiBlurKernel,
	pub(super) half_kernel: UiBlurKernel,
	pub(super) full_regions: UiBlurPathRegions,
	pub(super) half_regions: UiBlurHalfPathRegions,
	/// Scene and layer pixels both blur paths may read; resolved before filtering.
	pub(super) backdrop: UiPixelRegion,
}

/// The `UiStep` enum is one recorded step of a UI frame, in painter order.
///
/// A frame's steps alternate between draws and blurs, and both begin and end with a draw. Only the
/// first draw can be empty, when the frame starts with a blur or draws nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum UiStep {
	/// One draw of consecutive primitives, recorded as its own pass.
	Draw { first: u32, count: u32 },
	/// A backdrop blur reads the layer drawn so far, so it ends the draw before it. Its own quad
	/// is the first primitive of the draw that follows.
	Blur(UiBlurDispatch),
}

/// The `UiPrimitives` struct carries one frame's primitives in painter order and the steps that draw them.
#[derive(Debug)]
pub(super) struct UiPrimitives<'a> {
	/// The first record is the quad that clears damaged regions; the steps start after it.
	pub(super) primitives: Vec<UiPrimitive, &'a bumpalo::Bump>,
	pub(super) steps: Vec<UiStep, &'a bumpalo::Bump>,
	/// Each image primitive and the draw-list image it shows. The pass writes the texture slot into it.
	pub(super) images: Vec<(u32, u32), &'a bumpalo::Bump>,
	pub(super) truncated: bool,
	pub(super) dropped_glyphs: usize,
}

pub(super) struct UiImageTexture {
	pub(super) version: u64,
	pub(super) extent: (u32, u32),
	pub(super) image: ghi::BaseImageHandle,
	/// Element of the ubershader's texture array this image is bound to.
	pub(super) slot: u32,
}

/// The `UiPreparedFrame` struct retains the steps recorded for one render revision, viewport, and damage set.
///
/// Geometry, uploads, and glyph residency are only redone when the render
/// revision, the viewport extent, the text renderer's glyph generation, or the damaged
/// regions change. Frames that repeat the same key reuse the GPU buffers already in place.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UiPreparedFrame {
	pub(super) revision: Option<engine::RenderRevision>,
	pub(super) extent: Extent,
	pub(super) glyph_generation: u64,
	/// Only elements touching these regions have primitives in this frame.
	pub(super) damage: Vec<UiPixelRegion>,
	pub(super) steps: Vec<UiStep>,
}

impl UiPreparedFrame {
	pub(super) fn matches(
		&self,
		revision: Option<engine::RenderRevision>,
		extent: Extent,
		glyph_generation: u64,
		damage: &[UiPixelRegion],
	) -> bool {
		self.revision == revision && self.extent == extent && self.glyph_generation == glyph_generation && self.damage == damage
	}
}

impl UiPixelRegion {
	pub(super) fn full(viewport: Extent) -> Self {
		Self {
			origin: [0, 0],
			extent: viewport,
		}
	}

	pub(super) fn is_empty(self) -> bool {
		self.extent.width() == 0 || self.extent.height() == 0
	}

	pub(super) fn end(self) -> [u32; 2] {
		[self.origin[0] + self.extent.width(), self.origin[1] + self.extent.height()]
	}

	pub(super) fn area(self) -> u64 {
		u64::from(self.extent.width()) * u64::from(self.extent.height())
	}

	/// Rounds fractional pixel bounds outward with a margin, clamped to the viewport.
	pub(super) fn from_bounds(bounds: [f32; 4], margin: f32, viewport: Extent) -> Option<Self> {
		let clamp = |value: f32, limit: u32| value.clamp(0.0, limit as f32) as u32;
		let x0 = clamp((bounds[0] - margin).floor(), viewport.width());
		let y0 = clamp((bounds[1] - margin).floor(), viewport.height());
		let x1 = clamp((bounds[2] + margin).ceil(), viewport.width());
		let y1 = clamp((bounds[3] + margin).ceil(), viewport.height());
		(x1 > x0 && y1 > y0).then(|| Self {
			origin: [x0, y0],
			extent: Extent::rectangle(x1 - x0, y1 - y0),
		})
	}

	pub(super) fn intersects(self, other: Self) -> bool {
		let (end, other_end) = (self.end(), other.end());
		self.origin[0] < other_end[0] && other.origin[0] < end[0] && self.origin[1] < other_end[1] && other.origin[1] < end[1]
	}

	/// Reports whether fractional pixel bounds touch this region.
	pub(super) fn intersects_bounds(self, bounds: [f32; 4]) -> bool {
		let end = self.end();
		bounds[0] < end[0] as f32
			&& bounds[2] > self.origin[0] as f32
			&& bounds[1] < end[1] as f32
			&& bounds[3] > self.origin[1] as f32
	}

	pub(super) fn union(self, other: Self) -> Self {
		let (end, other_end) = (self.end(), other.end());
		let origin = [self.origin[0].min(other.origin[0]), self.origin[1].min(other.origin[1])];
		Self {
			origin,
			extent: Extent::rectangle(end[0].max(other_end[0]) - origin[0], end[1].max(other_end[1]) - origin[1]),
		}
	}
}

impl From<UiPixelRegion> for UiRegionPush {
	fn from(region: UiPixelRegion) -> Self {
		Self {
			origin: region.origin,
			extent: region.push_extent(),
		}
	}
}

/// A viewport-covering rectangle that writes transparent black; the scissor limits it to one damaged region.
pub(super) fn clear_primitive(viewport: Extent) -> UiPrimitive {
	let size = [viewport.width().max(1) as f32, viewport.height().max(1) as f32];
	UiPrimitive {
		bounds: [0.0, 0.0, size[0], size[1]],
		a: [0.0, 0.0, size[0], size[1]],
		b: [0.0, 2.0, 0.0, 0.0],
		kind: UI_KIND_RECT,
		..UiPrimitive::default()
	}
}

/// Reports whether an element with these pixel bounds must be drawn for the damage; no damage list means everything.
pub(super) fn damage_intersects(damage: Option<&[UiPixelRegion]>, bounds: [f32; 4]) -> bool {
	damage.is_none_or(|damage| damage.iter().any(|region| region.intersects_bounds(bounds)))
}

/// Converts layout-unit damage into margin-padded pixel regions.
pub(super) fn pixel_damage(rects: &[Geometry], layout_size: [f32; 2], viewport: Extent, out: &mut Vec<UiPixelRegion>) {
	let sx = viewport.width().max(1) as f32 / layout_size[0].max(1.0);
	let sy = viewport.height().max(1) as f32 / layout_size[1].max(1.0);
	out.extend(rects.iter().filter_map(|rect| {
		let bounds = [rect.x() * sx, rect.y() * sy, rect.right() * sx, rect.bottom() * sy];
		UiPixelRegion::from_bounds(bounds, UI_DAMAGE_MARGIN_PIXELS, viewport)
	}));
}

/// Adds the pixels every visible backdrop blur reads and writes, since blurs follow the scene each frame.
pub(super) fn blur_footprints(draw_list: &UiDrawList, viewport: Extent, out: &mut Vec<UiPixelRegion>) {
	let sx = viewport.width().max(1) as f32 / draw_list.layout_size[0].max(1.0);
	let sy = viewport.height().max(1) as f32 / draw_list.layout_size[1].max(1.0);
	out.extend(draw_list.blurs.iter().filter_map(|blur| {
		if blur.radius <= 0.0 {
			return None;
		}
		let mut bounds = [
			blur.position[0] * sx,
			blur.position[1] * sy,
			(blur.position[0] + blur.size[0]) * sx,
			(blur.position[1] + blur.size[1]) * sy,
		];
		if let Some(clip) = blur.clip {
			bounds = [
				bounds[0].max(clip.position[0] * sx),
				bounds[1].max(clip.position[1] * sy),
				bounds[2].min((clip.position[0] + clip.size[0]) * sx),
				bounds[3].min((clip.position[1] + clip.size[1]) * sy),
			];
		}
		UiPixelRegion::from_bounds(bounds, UI_BLUR_FOOTPRINT_MARGIN as f32, viewport)
	}));
}

/// Merges overlapping regions, bounds the count, and promotes large damage to one full-viewport region.
pub(super) fn merge_damage(damage: &mut Vec<UiPixelRegion>, viewport: Extent) {
	damage.retain(|region| !region.is_empty());
	let mut merged = true;
	while merged {
		merged = false;
		'outer: for index in 0..damage.len() {
			for other in index + 1..damage.len() {
				if damage[index].intersects(damage[other]) {
					damage[index] = damage[index].union(damage[other]);
					damage.swap_remove(other);
					merged = true;
					break 'outer;
				}
			}
		}
	}
	if damage.len() > MAX_UI_DAMAGE_REGIONS {
		let union = damage.iter().copied().reduce(UiPixelRegion::union).unwrap();
		damage.clear();
		damage.push(union);
	}
	let full = UiPixelRegion::full(viewport);
	let area: u64 = damage.iter().map(|region| region.area()).sum();
	if !damage.is_empty() && area as f32 >= full.area() as f32 * UI_FULL_REDRAW_AREA_SHARE {
		damage.clear();
		damage.push(full);
	}
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

impl UiPixelRegion {
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
pub(super) fn blur_composite_region(bounds: [f32; 4], target: Extent, downscale: u32) -> UiPixelRegion {
	let axis = |minimum: f32, maximum: f32, target_size: u32| {
		let lattice_scale = 1.0 / downscale as f32;
		let start = (minimum * lattice_scale - 0.5).floor().clamp(0.0, target_size as f32) as u32;
		let end = (maximum * lattice_scale + 0.5).ceil().clamp(0.0, target_size as f32) as u32;
		(start, end.max(start.saturating_add(1).min(target_size)))
	};
	let (start_x, end_x) = axis(bounds[0], bounds[2], target.width());
	let (start_y, end_y) = axis(bounds[1], bounds[3], target.height());
	UiPixelRegion {
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

pub(super) fn draw_clip_mask_from_layout(mask: Option<ClipMask>, rotation: Option<Rotation>) -> Option<DrawClipMask> {
	// A turned element without a mask still needs a table entry to carry its turn.
	(mask.is_some() || rotation.is_some()).then(|| DrawClipMask {
		position: mask.map_or([0.0; 2], |mask| [mask.geometry.x(), mask.geometry.y()]),
		size: mask.map_or([0.0; 2], |mask| [mask.geometry.width(), mask.geometry.height()]),
		edges: mask.map_or([0.0; 4], |mask| {
			[mask.feather.top, mask.feather.right, mask.feather.bottom, mask.feather.left]
		}),
		corner: mask.map_or([0.0, 2.0], |mask| [mask.corner_radius, mask.corner_exponent]),
		rotation: rotation.unwrap_or(Rotation::IDENTITY),
	})
}

/// Moves unrotated pixel bounds to where an element's turn draws them, for damage tests.
#[inline]
pub(super) fn turned_bounds(bounds: [f32; 4], mask: Option<DrawClipMask>, sx: f32, sy: f32) -> [f32; 4] {
	match mask {
		Some(mask) if !mask.rotation.is_identity() => mask.rotation.scaled(sx, sy).bounds(bounds),
		_ => bounds,
	}
}

/// Converts a layout rectangle to viewport pixels with its edges on whole pixels, as `[x0, y0, x1, y1]`.
///
/// Every rectangle, clip, and mask goes through here, so shared edges stay shared
/// and a one pixel border covers one pixel instead of straddling two. Edges round
/// rather than sizes, and a visible span never rounds away.
pub(super) fn snapped_rect(position: [f32; 2], size: [f32; 2], sx: f32, sy: f32) -> [f32; 4] {
	let span = |position: f32, size: f32, scale: f32| {
		let start = (position * scale).round();
		let end = ((position + size) * scale).round();
		(start, if size * scale > 0.0 { end.max(start + 1.0) } else { end })
	};
	let (x0, x1) = span(position[0], size[0], sx);
	let (y0, y1) = span(position[1], size[1], sy);
	[x0, y0, x1, y1]
}

pub(super) fn scaled_clip_mask(mask: Option<DrawClipMask>, sx: f32, sy: f32) -> DrawClipMask {
	mask.map(|mask| {
		let [x0, y0, x1, y1] = snapped_rect(mask.position, mask.size, sx, sy);
		(mask, [x0, y0], [x1 - x0, y1 - y0])
	})
	.map(|(mask, position, size)| DrawClipMask {
		position,
		size,
		edges: [mask.edges[0] * sy, mask.edges[1] * sx, mask.edges[2] * sy, mask.edges[3] * sx],
		corner: [mask.corner[0] * sx.min(sy), mask.corner[1]],
		rotation: mask.rotation.scaled(sx, sy),
	})
	.unwrap_or(DrawClipMask {
		position: [0.0, 0.0],
		size: [0.0, 0.0],
		edges: [0.0, 0.0, 0.0, 0.0],
		corner: [0.0, 2.0],
		rotation: Rotation::IDENTITY,
	})
}

// Keep the render snapshot conversion as one pass so all draw-list arrays share the same ordering and opacity rules.
#[allow(clippy::too_many_lines)]
/// Adopts a snapshot while retaining the owned text and curve buffers of surviving slots.
pub(super) fn update_from_render(render: &engine::Render, draw_list: &mut UiDrawList) {
	draw_list.layout_size = [render.viewport_size.x(), render.viewport_size.y()];
	draw_list.elements.clear();
	draw_list.blurs.clear();
	draw_list.images.clear();
	let mut curve_count = 0;
	let mut text_count = 0;

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
				clip_mask: draw_clip_mask_from_layout(element.clip_mask, element.rotation),
				color: color.into(),
				corner_radius: element.corner_radius,
				corner_exponent: element.corner_exponent,
				sector: element.sector,
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
				clip_mask: draw_clip_mask_from_layout(element.clip_mask, element.rotation),
				color: color.into(),
				corner_radius: element.corner_radius,
				corner_exponent: element.corner_exponent,
				sector: element.sector,
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

			// A zoomed subtree scales its wires like its rectangles: points from the
			// element's origin and the stroke follow the inherited scale.
			let mut entry = UiCurveDrawElement {
				depth: position.z(),
				order: curve.id,
				position: [position.x(), position.y()],
				size: [size.x(), size.y()],
				clip: draw_clip_from_geometry(curve.clip),
				clip_mask: draw_clip_mask_from_layout(curve.clip_mask, curve.rotation),
				color: color.into(),
				stroke_width: stroke_width * curve.scale[0].min(curve.scale[1]),
				segments: Vec::new(),
			};
			// Reuse by output slot; filtered layers must not consume a retained buffer.
			if let Some(previous) = draw_list.curves.get_mut(curve_count) {
				entry.segments = std::mem::take(&mut previous.segments);
				*previous = entry;
			} else {
				draw_list.curves.push(entry);
			}
			let segments = &mut draw_list.curves[curve_count].segments;
			segments.clear();
			segments.extend(curve.segments.iter().map(|segment| scale_segment(segment, curve.scale)));
			curve_count += 1;
		}
	}
	draw_list.curves.truncate(curve_count);

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
			clip_mask: draw_clip_mask_from_layout(image.clip_mask, image.rotation),
			opacity: image.opacity,
		});
	}

	for text in render.texts() {
		let mut color = text.color;
		color.a *= text.opacity;
		// Filter before touching retained strings so hidden text cannot discard reusable storage.
		if text.content.is_empty() || !(color.a > 0.0 && text.size.x() > 0.0 && text.size.y() > 0.0) {
			continue;
		}
		let mut entry = UiTextDrawElement {
			depth: text.position.z(),
			order: text.id,
			position: [text.position.x(), text.position.y()],
			size: [text.size.x(), text.size.y()],
			clip: draw_clip_from_geometry(text.clip),
			clip_mask: draw_clip_mask_from_layout(text.clip_mask, text.rotation),
			color,
			font_size: text.font_size * text.scale,
			text: String::new(),
		};
		if let Some(previous) = draw_list.texts.get_mut(text_count) {
			entry.text = std::mem::take(&mut previous.text);
			*previous = entry;
		} else {
			draw_list.texts.push(entry);
		}
		draw_list.texts[text_count].text.clone_from(&text.content);
		text_count += 1;
	}
	draw_list.texts.truncate(text_count);
}

/// Scales a segment's points about the curve element's origin.
fn scale_segment(segment: &CurveSegment, scale: [f32; 2]) -> CurveSegment {
	if scale == [1.0, 1.0] {
		return segment.clone();
	}
	let scaled = |point: CurvePoint| CurvePoint::new(point.x * scale[0], point.y * scale[1]);
	match *segment {
		CurveSegment::Line { from, to } => CurveSegment::Line {
			from: scaled(from),
			to: scaled(to),
		},
		CurveSegment::Quadratic { from, control, to } => CurveSegment::Quadratic {
			from: scaled(from),
			control: scaled(control),
			to: scaled(to),
		},
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		} => CurveSegment::Cubic {
			from: scaled(from),
			control0: scaled(control0),
			control1: scaled(control1),
			to: scaled(to),
		},
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
