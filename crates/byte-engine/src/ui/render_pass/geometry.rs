//! UI primitive generation: rectangle layers, backdrop blurs, curve pieces, and images, merged with glyphs in painter order.

use super::*;

/// Builds one frame's primitives for the elements touching `damage`; `None` builds everything.
///
/// The draw list keeps one array per element type, each already in painter order, so the arrays
/// are merged by depth and then by element. Every primitive lands in one buffer, and only a
/// backdrop blur, which reads what was drawn below it, splits the frame into more than one draw.
///
/// `text` holds the glyphs a text renderer prepared for the same draw list and damage.
// Keep the merge and every capacity check in one pass so painter order and truncation cannot diverge between kinds.
#[allow(clippy::too_many_lines)]
pub(super) fn build_ui_primitives<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
	mut caches: Option<&mut UiGeometryCaches>,
	masks: &mut UiMaskTable,
	text: Option<&UiTextGeometry<'_>>,
	damage: Option<&[UiPixelRegion]>,
) -> UiPrimitives<'a> {
	// A render cannot contain more images than the engine's 32-bit element IDs allow.
	debug_assert!(u32::try_from(draw_list.images.len()).is_ok());
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let radius_scale = sx.min(sy);
	if let Some(caches) = caches.as_deref_mut() {
		caches.rectangles.begin(viewport, draw_list.layout_size);
		caches.images.begin(viewport, draw_list.layout_size);
	}

	let glyph_count = text.map_or(0, |text| text.primitives.len());
	let capacity =
		(1 + draw_list.elements.len() + draw_list.blurs.len() + draw_list.images.len() + glyph_count).min(MAX_UI_PRIMITIVES);
	let mut output = UiPrimitives {
		primitives: Vec::with_capacity_in(capacity, frame_allocator),
		steps: Vec::with_capacity_in(draw_list.blurs.len() * 2 + 1, frame_allocator),
		images: Vec::with_capacity_in(draw_list.images.len(), frame_allocator),
		truncated: false,
		dropped_glyphs: text.map_or(0, |text| text.dropped_glyphs),
	};
	output.primitives.push(clear_primitive(viewport));
	let mut draw_first = output.primitives.len();

	// Adjacent blurs often share a radius; their dispatch regions still remain independent.
	let mut cached_kernels: Option<(f32, UiBlurKernel, UiBlurKernel)> = None;
	let mut cubics = Vec::new_in(frame_allocator);
	// A blur goes under its own element's layers, and the remaining ties keep the order the types were listed in.
	let (mut blurs, mut elements, mut curves, mut images, mut texts) = (0, 0, 0, 0, 0);
	loop {
		let heads = [
			draw_list.blurs.get(blurs).map(|blur| (blur.depth, blur.order)),
			draw_list.elements.get(elements).map(|element| (element.depth, element.order)),
			draw_list.curves.get(curves).map(|curve| (curve.depth, curve.order)),
			draw_list.images.get(images).map(|image| (image.depth, image.order)),
			draw_list.texts.get(texts).map(|text| (text.depth, text.order)),
		];
		let Some((_, next)) = heads
			.iter()
			.enumerate()
			.filter_map(|(kind, head)| head.map(|head| (head, kind)))
			.min()
		else {
			break;
		};
		// Capacity is checked where a primitive is pushed, so an element that draws nothing never counts against it.
		let full = output.primitives.len() == MAX_UI_PRIMITIVES;

		match next {
			0 => {
				let blur = &draw_list.blurs[blurs];
				blurs += 1;
				let effective_radius = (blur.radius * radius_scale).clamp(0.0, 64.0);
				let sigma_pixels = blur_sigma(effective_radius);
				let resolution_mix = blur_resolution_mix(sigma_pixels);
				let Some(mut primitive) = blur_primitive(blur, viewport, sx, sy, resolution_mix) else {
					continue;
				};
				if full {
					output.truncated = true;
					break;
				}
				primitive.mask = masks.index(None, blur.clip_mask, sx, sy);
				if cached_kernels.as_ref().is_none_or(|(radius, ..)| *radius != effective_radius) {
					cached_kernels = Some((
						effective_radius,
						UiBlurKernel::gaussian(sigma_pixels),
						UiBlurKernel::gaussian(blur_half_sigma(sigma_pixels)),
					));
				}
				let (_, full_kernel, half_kernel) = cached_kernels.unwrap();
				let bounds = primitive.bounds;
				// The blur reads the layer below it, so the primitives so far are drawn first.
				output.steps.push(UiStep::Draw {
					first: draw_first as u32,
					count: (output.primitives.len() - draw_first) as u32,
				});
				draw_first = output.primitives.len();
				output.steps.push(UiStep::Blur(UiBlurDispatch {
					resolution_mix,
					full_kernel,
					half_kernel,
					full_regions: blur_full_dispatch_regions(bounds, viewport),
					half_regions: blur_half_dispatch_regions(bounds, viewport),
					backdrop: UiPixelRegion::from_bounds(bounds, UI_BLUR_FOOTPRINT_MARGIN as f32, viewport)
						.unwrap_or(UiPixelRegion::full(viewport)),
				}));
				output.primitives.push(primitive);
			}
			1 => {
				let element = &draw_list.elements[elements];
				elements += 1;
				let rect_width = (element.size[0] * sx).max(0.0);
				let rect_height = (element.size[1] * sy).max(0.0);
				if rect_width <= 0.0 || rect_height <= 0.0 || element.color[3] <= 0.0 {
					// Omit element if 0 sized in any dimension or if fully transparent
					continue;
				}
				let stroke_width = element.stroke_width * radius_scale;
				if matches!(element.layer_kind, LayerKind::Stroke { .. }) && (!stroke_width.is_finite() || stroke_width <= 0.0)
				{
					continue;
				}
				if !damage_intersects(
					damage,
					turned_bounds(
						element_bounds(element.position, [rect_width, rect_height], sx, sy, UI_DAMAGE_MARGIN_PIXELS),
						element.clip_mask,
						sx,
						sy,
					),
				) {
					continue;
				}
				let primitive = match caches.as_deref_mut() {
					Some(caches) => {
						// Depth only orders the merge. It shifts whenever an element is added earlier in
						// the paint order, which must not invalidate a settled rectangle.
						let key = UiDrawElement { depth: 0, ..*element };
						caches
							.rectangles
							.get(element.order, &key, || rectangle_primitive(element, sx, sy))
					}
					None => rectangle_primitive(element, sx, sy),
				};
				if let Some(mut primitive) = primitive {
					if full {
						output.truncated = true;
						break;
					}
					// Mask indices belong to this frame's table, so a retained primitive gets its own each frame.
					primitive.mask = masks.index(None, element.clip_mask, sx, sy);
					output.primitives.push(primitive);
				}
			}
			2 => {
				let curve = &draw_list.curves[curves];
				curves += 1;
				let stroke_width = curve.stroke_width * radius_scale;
				if curve.color[3] <= 0.0 || !stroke_width.is_finite() || stroke_width <= 0.0 {
					continue;
				}
				let half_width = stroke_width * 0.5;
				if !damage_intersects(
					damage,
					turned_bounds(
						element_bounds(
							curve.position,
							[curve.size[0] * sx, curve.size[1] * sy],
							sx,
							sy,
							half_width + CURVE_AA_WIDTH_PIXELS + UI_DAMAGE_MARGIN_PIXELS,
						),
						curve.clip_mask,
						sx,
						sy,
					),
				) {
					continue;
				}
				let mask = masks.index(curve.clip, curve.clip_mask, sx, sy);
				cubics.clear();
				cubics.extend(
					curve
						.segments
						.iter()
						.filter_map(|segment| segment_cubic(segment, curve.position, sx, sy)),
				);
				for (index, cubic) in cubics.iter().enumerate() {
					// A smooth joint is split between its two segments, and a corner is rounded by the segment that starts at it.
					let smooth_start = index > 0 && joins_smoothly(&cubics[index - 1], cubic);
					let shared_end = cubics.get(index + 1).is_some_and(|next| joined(cubic, next));
					let caps =
						if smooth_start { 0 } else { UI_CURVE_CAP_START } | if shared_end { 0 } else { UI_CURVE_CAP_END };
					let count = curve_piece_count(cubic);
					for piece in 0..count {
						if output.primitives.len() == MAX_UI_PRIMITIVES {
							output.truncated = true;
							break;
						}
						output.primitives.push(UiPrimitive {
							bounds: [cubic[0][0], cubic[0][1], cubic[1][0], cubic[1][1]],
							color: curve.color,
							a: [cubic[2][0], cubic[2][1], cubic[3][0], cubic[3][1]],
							b: [half_width, 0.0, 0.0, 0.0],
							kind: UI_KIND_CURVE,
							mask,
							data0: piece | count << 16,
							data1: caps,
						});
					}
				}
			}
			3 => {
				let image = &draw_list.images[images];
				let source_index = images as u32;
				images += 1;
				if !should_draw_image(image) {
					continue;
				}
				if !damage_intersects(
					damage,
					turned_bounds(
						element_bounds(
							image.position,
							[image.size[0] * sx, image.size[1] * sy],
							sx,
							sy,
							UI_DAMAGE_MARGIN_PIXELS,
						),
						image.clip_mask,
						sx,
						sy,
					),
				) {
					continue;
				}
				let primitive = match caches.as_deref_mut() {
					Some(caches) => {
						let key = (image.position, image.size, image.clip, image.opacity);
						caches.images.get(image.order, &key, || image_primitive(image, sx, sy))
					}
					None => image_primitive(image, sx, sy),
				};
				if let Some(mut primitive) = primitive {
					if full {
						output.truncated = true;
						break;
					}
					primitive.mask = masks.index(None, image.clip_mask, sx, sy);
					output.images.push((output.primitives.len() as u32, source_index));
					output.primitives.push(primitive);
				}
			}
			_ => {
				let label = text.and_then(|text| Some(&text.primitives[text.labels.get(texts)?.clone()]));
				texts += 1;
				let Some(label) = label else {
					continue;
				};
				let count = label.len().min(MAX_UI_PRIMITIVES - output.primitives.len());
				output.primitives.extend_from_slice(&label[..count]);
				output.truncated |= count < label.len();
			}
		}
		if output.truncated {
			break;
		}
	}

	output.steps.push(UiStep::Draw {
		first: draw_first as u32,
		count: (output.primitives.len() - draw_first) as u32,
	});
	output.truncated |= text.is_some_and(|text| text.truncated) || masks.truncated;

	output
}

/// Builds reference primitives without text and without retaining surface data.
#[cfg(test)]
pub(super) fn build_ui_primitives_uncached<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	arena: &'a bumpalo::Bump,
	masks: &mut UiMaskTable,
) -> UiPrimitives<'a> {
	build_ui_primitives(draw_list, viewport, arena, None, masks, None, None)
}

/// Pixel bounds of an element for damage tests: its layout origin scaled, its pixel size, and a margin.
#[inline]
pub(super) fn element_bounds(position: [f32; 2], pixel_size: [f32; 2], sx: f32, sy: f32, margin: f32) -> [f32; 4] {
	let x0 = position[0] * sx;
	let y0 = position[1] * sy;
	[
		x0 - margin,
		y0 - margin,
		x0 + pixel_size[0] + margin,
		y0 + pixel_size[1] + margin,
	]
}

/// Snaps a rectangle to whole pixels and trims it to its clip. Returns the rectangle and the visible part of it.
#[inline]
fn clipped_rect(position: [f32; 2], size: [f32; 2], clip: Option<DrawClip>, sx: f32, sy: f32) -> Option<([f32; 4], [f32; 4])> {
	let original = snapped_rect(position, size, sx, sy);
	let visible = match clip {
		Some(clip) => {
			let clip = snapped_rect(clip.position, clip.size, sx, sy);
			[
				original[0].max(clip[0]),
				original[1].max(clip[1]),
				original[2].min(clip[2]),
				original[3].min(clip[3]),
			]
		}
		None => original,
	};
	(visible[2] > visible[0] && visible[3] > visible[1]).then_some((original, visible))
}

/// Resolves one rectangle layer's clipped quad. The shader measures the shape from the unclipped rectangle.
#[inline]
fn rectangle_primitive(element: &UiDrawElement, sx: f32, sy: f32) -> Option<UiPrimitive> {
	let (original, bounds) = clipped_rect(element.position, element.size, element.clip, sx, sy)?;
	let rect_width = (original[2] - original[0]).max(0.0);
	let rect_height = (original[3] - original[1]).max(0.0);
	let radius_scale = sx.min(sy);
	// A fill has no stroke width, which is how the shader tells the two apart.
	let stroke_width = element.stroke_width * radius_scale;
	let (b, kind, data0) = match element.sector {
		Some(sector) => (
			sector_parameters(sector, stroke_width),
			UI_KIND_SECTOR,
			sector_inset(sector, radius_scale),
		),
		None => (
			[
				resolved_corner_radius(element.corner_radius * radius_scale, rect_width, rect_height),
				resolved_corner_exponent(element.corner_exponent),
				stroke_width,
				0.0,
			],
			UI_KIND_RECT,
			0,
		),
	};
	Some(UiPrimitive {
		bounds,
		color: element.color,
		a: [original[0], original[1], rect_width, rect_height],
		b,
		kind,
		data0,
		..UiPrimitive::default()
	})
}

/// Resolves one backdrop blur's clipped quad inside the viewport, which its dispatch regions are planned from.
fn blur_primitive(blur: &UiBlurDrawElement, viewport: Extent, sx: f32, sy: f32, resolution_mix: f32) -> Option<UiPrimitive> {
	if (blur.size[0] * sx).max(0.0) <= 0.0 || (blur.size[1] * sy).max(0.0) <= 0.0 || blur.radius <= 0.0 {
		return None;
	}
	let (original, visible) = clipped_rect(blur.position, blur.size, blur.clip, sx, sy)?;
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let bounds = [
		visible[0].clamp(0.0, viewport_width),
		visible[1].clamp(0.0, viewport_height),
		visible[2].clamp(0.0, viewport_width),
		visible[3].clamp(0.0, viewport_height),
	];
	if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
		return None;
	}
	let (rect_width, rect_height) = (original[2] - original[0], original[3] - original[1]);
	let (b, kind, data0) = match blur.sector {
		Some(sector) => (
			sector_parameters(sector, resolution_mix),
			UI_KIND_SECTOR_BLUR,
			sector_inset(sector, sx.min(sy)),
		),
		None => (
			[
				resolved_corner_radius(blur.corner_radius * sx.min(sy), rect_width, rect_height),
				resolved_corner_exponent(blur.corner_exponent),
				0.0,
				resolution_mix,
			],
			UI_KIND_BLUR,
			0,
		),
	};
	Some(UiPrimitive {
		bounds,
		color: blur.color,
		a: [original[0], original[1], rect_width, rect_height],
		b,
		kind,
		data0,
		..UiPrimitive::default()
	})
}

/// Packs a sector's edge inset in viewport pixels as fixed point with [`SECTOR_INSET_SCALE`] steps per pixel.
fn sector_inset(sector: Sector, scale: f32) -> u32 {
	let inset = if sector.inset.is_finite() {
		sector.inset.max(0.0)
	} else {
		0.0
	};
	(inset * scale * SECTOR_INSET_SCALE).round().min(u32::MAX as f32) as u32
}

/// Packs a sector's shape for the shader: inner radius ratio, start angle, sweep angle, and the
/// kind's fourth value. Ratios and angles are scale free, so the viewport scale does not touch them.
fn sector_parameters(sector: Sector, fourth: f32) -> [f32; 4] {
	let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
	[
		finite(sector.inner, 0.0).clamp(0.0, 1.0),
		finite(sector.start, 0.0),
		finite(sector.sweep, 0.0).clamp(0.0, Sector::FULL_TURN),
		fourth,
	]
}

/// Resolves one image's clipped quad and the part of the texture it shows. The pass fills in the texture slot.
#[inline]
fn image_primitive(image: &UiImageDrawElement, sx: f32, sy: f32) -> Option<UiPrimitive> {
	let (original, bounds) = clipped_rect(image.position, image.size, image.clip, sx, sy)?;
	let (rect_width, rect_height) = (original[2] - original[0], original[3] - original[1]);
	if rect_width <= 0.0 || rect_height <= 0.0 {
		return None;
	}
	Some(UiPrimitive {
		bounds,
		color: [1.0, 1.0, 1.0, image.opacity],
		a: [
			((bounds[0] - original[0]) / rect_width).clamp(0.0, 1.0),
			((bounds[1] - original[1]) / rect_height).clamp(0.0, 1.0),
			((bounds[2] - original[0]) / rect_width).clamp(0.0, 1.0),
			((bounds[3] - original[1]) / rect_height).clamp(0.0, 1.0),
		],
		kind: UI_KIND_IMAGE,
		..UiPrimitive::default()
	})
}

/// Converts a segment to a cubic in viewport pixels, or `None` when it has no length.
///
/// Lines and quadratics are stored as the cubic that traces the same path, so the shader has one
/// curve type. Its quadratic fit recovers both exactly.
fn segment_cubic(segment: &CurveSegment, origin: [f32; 2], sx: f32, sy: f32) -> Option<[[f32; 2]; 4]> {
	let pixel = |point: CurvePoint| {
		let point = scaled_curve_point(point, origin, sx, sy);
		[point.x, point.y]
	};
	let toward = |from: [f32; 2], to: [f32; 2], t: f32| [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t];
	let cubic = match *segment {
		CurveSegment::Line { from, to } => {
			let (from, to) = (pixel(from), pixel(to));
			[from, toward(from, to, 1.0 / 3.0), toward(from, to, 2.0 / 3.0), to]
		}
		CurveSegment::Quadratic { from, control, to } => {
			let (from, control, to) = (pixel(from), pixel(control), pixel(to));
			[from, toward(from, control, 2.0 / 3.0), toward(to, control, 2.0 / 3.0), to]
		}
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		} => [pixel(from), pixel(control0), pixel(control1), pixel(to)],
	};
	let finite = cubic.iter().flatten().all(|value| value.is_finite());
	let extent = cubic[1..].iter().map(|point| distance(cubic[0], *point)).fold(0.0, f32::max);
	(finite && extent > 0.0001).then_some(cubic)
}

/// Picks how many pieces the shader draws a cubic in.
///
/// Each piece is drawn as one quadratic inside one bounding box, so a piece must be close to a
/// quadratic and must not bulge far from its chord. Both limits follow from the control points:
/// the fit's error shrinks with the cube of the piece count, and the bulge with its square.
pub(super) fn curve_piece_count(cubic: &[[f32; 2]; 4]) -> u32 {
	let [c0, c1, c2, c3] = *cubic;
	let second = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1]);
	let third = (c3[0] - 3.0 * c2[0] + 3.0 * c1[0] - c0[0]).hypot(c3[1] - 3.0 * c2[1] + 3.0 * c1[1] - c0[1]);
	let fit = (third * (3.0f32.sqrt() / 54.0) / CURVE_QUADRATIC_TOLERANCE_PIXELS).cbrt();
	let bulge = (second(c0, c1, c2).max(second(c1, c2, c3)) * 0.75 / CURVE_PIECE_BULGE_PIXELS).sqrt();
	(fit.max(bulge).ceil() as u32).clamp(1, MAX_CURVE_PIECES)
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
	(b[0] - a[0]).hypot(b[1] - a[1])
}

/// Reports whether `next` starts where `previous` ends.
fn joined(previous: &[[f32; 2]; 4], next: &[[f32; 2]; 4]) -> bool {
	distance(previous[3], next[0]) < 0.01
}

/// Reports whether `next` continues `previous` without a corner.
fn joins_smoothly(previous: &[[f32; 2]; 4], next: &[[f32; 2]; 4]) -> bool {
	// Repeated control points leave an end without a tangent, so look further along the curve.
	let direction = |from: [f32; 2], candidates: [[f32; 2]; 3], sign: f32| {
		candidates
			.into_iter()
			.map(|to| [(to[0] - from[0]) * sign, (to[1] - from[1]) * sign])
			.find(|delta| delta[0].hypot(delta[1]) > 0.0001)
			.map(|delta| {
				let length = delta[0].hypot(delta[1]);
				[delta[0] / length, delta[1] / length]
			})
	};
	let (Some(leaving), Some(entering)) = (
		direction(previous[3], [previous[2], previous[1], previous[0]], -1.0),
		direction(next[0], [next[1], next[2], next[3]], 1.0),
	) else {
		return false;
	};
	joined(previous, next)
		&& (leaving[0] * entering[1] - leaving[1] * entering[0]).abs() < 0.01
		&& leaving[0] * entering[0] + leaving[1] * entering[1] > 0.0
}

pub(super) fn scaled_curve_point(point: CurvePoint, origin: [f32; 2], sx: f32, sy: f32) -> CurvePoint {
	CurvePoint::new((origin[0] + point.x) * sx, (origin[1] + point.y) * sy)
}
