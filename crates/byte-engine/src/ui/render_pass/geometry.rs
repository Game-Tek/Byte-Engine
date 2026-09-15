//! UI rectangle, blur, curve, and image geometry generation.

use super::*;

// Keep rectangle batching, clipping, and capacity accounting in one geometry pass.
#[allow(clippy::too_many_lines)]
pub(super) fn build_ui_geometry_cached<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
	mut cache: Option<&mut SurfaceCache<UiDrawElement, Option<[UiVertex; 4]>>>,
) -> UiGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let radius_scale = sx.min(sy);
	if let Some(cache) = cache.as_deref_mut() {
		cache.begin(viewport, draw_list.layout_size);
	}

	let mut geometry = UiGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_VERTICES_PER_ELEMENT,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_INDICES_PER_ELEMENT,
			frame_allocator,
		),
		batches: Vec::with_capacity_in(draw_list.elements.len().min(MAX_UI_ELEMENTS), frame_allocator),
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

		if let Some(cache) = cache.as_deref_mut() {
			let Some(vertices) = cache.get(element.order, element, || {
				rectangle_vertices(element, viewport, sx, sy, |vertices| vertices)
			}) else {
				continue;
			};
			geometry.vertices.extend_from_slice(&vertices);
		} else if rectangle_vertices(element, viewport, sx, sy, |vertices| {
			geometry.vertices.extend_from_slice(&vertices)
		})
		.is_none()
		{
			continue;
		}

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

// Keep blur region selection and its matching composite geometry in one pass.
#[allow(clippy::too_many_lines)]
/// Builds blur quads and dispatch regions, reusing kernels for adjacent equal radii.
pub(super) fn build_ui_blur_geometry<'a>(
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
		batches: Vec::with_capacity_in(draw_list.blurs.len().min(MAX_UI_ELEMENTS), frame_allocator),
		truncated: false,
	};

	// Adjacent items often share a radius; their dispatch regions still remain independent.
	let mut cached_kernels: Option<(f32, UiBlurKernel, UiBlurKernel)> = None;
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
		if cached_kernels.as_ref().is_none_or(|(radius, ..)| *radius != effective_radius) {
			cached_kernels = Some((
				effective_radius,
				UiBlurKernel::gaussian(sigma_pixels),
				UiBlurKernel::gaussian(blur_half_sigma(sigma_pixels)),
			));
		}
		let (_, full_kernel, half_kernel) = cached_kernels.unwrap();
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

/// Reuses local curve points and unchanged clipped geometry before assembling batches.
pub(super) fn build_ui_curve_geometry_cached<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
	mut cache: Option<&mut CurveGeometryCache>,
) -> UiCurveGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let stroke_scale = sx.min(sy);

	// A curve usually emits several spans. Reuse the last visible count with
	// modest headroom for zoom changes, bounded by the existing frame budget.
	let span_capacity = if draw_list.curves.is_empty() {
		0
	} else {
		cache
			.as_ref()
			.map_or(draw_list.curves.len(), |cache| {
				cache
					.previous_span_count
					.saturating_add(cache.previous_span_count / 8)
					.max(draw_list.curves.len())
			})
			.min(MAX_UI_ELEMENTS)
	};
	let mut geometry = UiCurveGeometry {
		vertices: Vec::with_capacity_in(span_capacity * UI_VERTICES_PER_CURVE_SPAN, frame_allocator),
		indices: Vec::with_capacity_in(span_capacity * UI_INDICES_PER_CURVE_SPAN, frame_allocator),
		batches: Vec::with_capacity_in(draw_list.curves.len().min(MAX_UI_ELEMENTS), frame_allocator),
		truncated: false,
	};

	let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
	let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;
	let mut fallback = CachedCurve::default();
	let mut points = Vec::new_in(frame_allocator);

	let mut previous = None;
	let mut layer = 0;
	for curve in &draw_list.curves {
		layer = if previous == Some(curve.order) { layer + 1 } else { 0 };
		previous = Some(curve.order);
		let stroke_width = curve.stroke_width * stroke_scale;
		if curve.color[3] <= 0.0 || !stroke_width.is_finite() || stroke_width <= 0.0 {
			continue;
		}

		let half_width = stroke_width * 0.5;
		let expansion = half_width + CURVE_AA_WIDTH_PIXELS;
		let feather_mask = scaled_feather_mask(curve.feather_mask, sx, sy);
		let first_index = geometry.indices.len();
		let vertex_offset = geometry.vertices.len();

		let retained = cache.is_some();
		let surface = match cache.as_deref_mut() {
			Some(cache) => cache.surfaces.entry((curve.order, layer)).or_default(),
			None => &mut fallback,
		};
		let stable = surface.input.as_ref() == Some(curve) && surface.viewport == Some((viewport, draw_list.layout_size));
		if retained && surface.valid && stable {
			let spans = (MAX_UI_VERTICES - geometry.vertices.len()) / UI_VERTICES_PER_CURVE_SPAN;
			let spans = spans.min((MAX_UI_INDICES - geometry.indices.len()) / UI_INDICES_PER_CURVE_SPAN);
			let vertices = surface.vertices.len().min(spans * UI_VERTICES_PER_CURVE_SPAN);
			let indices = vertices / UI_VERTICES_PER_CURVE_SPAN * UI_INDICES_PER_CURVE_SPAN;
			geometry.vertices.extend_from_slice(&surface.vertices[..vertices]);
			geometry.indices.extend_from_slice(&surface.indices[..indices]);
			geometry.truncated = vertices < surface.vertices.len();
		} else {
			let flattened = &mut surface.flattened;
			if retained {
				flattened.update(&curve.segments, [sx, sy], CURVE_FLATTEN_TOLERANCE_PIXELS);
			}
			for (index, segment) in curve.segments.iter().enumerate() {
				let local_points = if retained {
					&flattened.points[flattened.ranges[index].clone()]
				} else {
					points.clear();
					flatten_curve_segment(segment, [0.0, 0.0], sx, sy, CURVE_FLATTEN_TOLERANCE_PIXELS, &mut points);
					points.as_slice()
				};
				for span in local_points.windows(2) {
					let mut from = CurvePoint::new(span[0].x + curve.position[0] * sx, span[0].y + curve.position[1] * sy);
					let mut to = CurvePoint::new(span[1].x + curve.position[0] * sx, span[1].y + curve.position[1] * sy);
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
				}

				if geometry.truncated {
					break;
				}
			}

			if retained && !geometry.truncated {
				let store = stable || surface.input.is_none();
				if let Some(input) = &mut surface.input {
					input.clone_from(curve);
				} else {
					surface.input = Some(curve.clone());
				}
				surface.viewport = Some((viewport, draw_list.layout_size));
				surface.valid = store;
				if store {
					surface.vertices.clear();
					surface.vertices.extend_from_slice(&geometry.vertices[vertex_offset..]);
					surface.indices.clear();
					surface.indices.extend_from_slice(&geometry.indices[first_index..]);
				}
			}
		}

		let emitted_indices = geometry.indices.len() - first_index;
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

	if let Some(cache) = cache {
		cache.previous_span_count = geometry.vertices.len() / UI_VERTICES_PER_CURVE_SPAN;
	}

	geometry
}

pub(super) fn flatten_curve_segment(
	segment: &CurveSegment,
	origin: [f32; 2],
	sx: f32,
	sy: f32,
	tolerance: f32,
	points: &mut Vec<CurvePoint, &bumpalo::Bump>,
) {
	segment.flatten(|point| scaled_curve_point(point, origin, sx, sy), tolerance, points);
}

pub(super) fn scaled_curve_point(point: CurvePoint, origin: [f32; 2], sx: f32, sy: f32) -> CurvePoint {
	CurvePoint::new((origin[0] + point.x) * sx, (origin[1] + point.y) * sy)
}

pub(super) fn clip_curve_span(from: &mut CurvePoint, to: &mut CurvePoint, clip: Option<DrawClip>, sx: f32, sy: f32) -> bool {
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

pub(super) fn clip_line_axis(p: f32, q: f32, t0: &mut f32, t1: &mut f32) -> bool {
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

/// Builds clipped image quads and retains their source indices for texture preparation.
/// Reuses image quads while resolving batch sources from the current draw list.
pub(super) fn build_ui_image_geometry_cached<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
	mut cache: Option<&mut ImageGeometryCache>,
) -> UiImageGeometry<'a> {
	// A render cannot contain more images than the engine's 32-bit element IDs allow.
	debug_assert!(u32::try_from(draw_list.images.len()).is_ok());
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	if let Some(cache) = cache.as_deref_mut() {
		cache.begin(viewport, draw_list.layout_size);
	}

	let mut geometry = UiImageGeometry {
		vertices: Vec::with_capacity_in(
			draw_list.images.len().min(MAX_UI_IMAGES) * UI_VERTICES_PER_ELEMENT,
			frame_allocator,
		),
		indices: Vec::with_capacity_in(
			draw_list.images.len().min(MAX_UI_IMAGES) * UI_INDICES_PER_ELEMENT,
			frame_allocator,
		),
		batches: Vec::with_capacity_in(draw_list.images.len().min(MAX_UI_ELEMENTS), frame_allocator),
		truncated: false,
	};

	for (source_index, image) in draw_list.images.iter().enumerate() {
		if !should_draw_image(image) {
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		let key = (image.position, image.size, image.clip, image.feather_mask, image.opacity);
		let first_index = geometry.indices.len();
		let vertex_offset = geometry.vertices.len();
		if let Some(cache) = cache.as_deref_mut() {
			let Some(vertices) = cache.get(image.order, &key, || {
				image_vertices(image, viewport, sx, sy, |vertices| vertices)
			}) else {
				continue;
			};
			geometry.vertices.extend_from_slice(&vertices);
		} else if image_vertices(image, viewport, sx, sy, |vertices| {
			geometry.vertices.extend_from_slice(&vertices)
		})
		.is_none()
		{
			continue;
		}

		geometry.indices.extend_from_slice(&[0, 1, 2, 2, 3, 0]);
		geometry.batches.push(UiImageDrawBatch {
			source_index: source_index as u32,
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

/// Resolves one rectangle's clipped quad independently of draw batching.
#[inline]
fn rectangle_vertices<R>(
	element: &UiDrawElement,
	viewport: Extent,
	sx: f32,
	sy: f32,
	consume: impl FnOnce([UiVertex; 4]) -> R,
) -> Option<R> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let rect_width = (element.size[0] * sx).max(0.0);
	let rect_height = (element.size[1] * sy).max(0.0);
	let radius_scale = sx.min(sy);
	let stroke_width = element.stroke_width * radius_scale;
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
		return None;
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

	Some(consume([
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
	]))
}

/// Builds reference geometry without retaining surface data.
#[cfg(test)]
pub(super) fn build_ui_geometry<'a>(draw_list: &UiDrawList, viewport: Extent, arena: &'a bumpalo::Bump) -> UiGeometry<'a> {
	build_ui_geometry_cached(draw_list, viewport, arena, None)
}

/// Builds curve geometry without retaining local tessellation between calls.
#[cfg(test)]
pub(super) fn build_ui_curve_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	arena: &'a bumpalo::Bump,
) -> UiCurveGeometry<'a> {
	build_ui_curve_geometry_cached(draw_list, viewport, arena, None)
}

/// Resolves one image's clipped quad without rebuilding neighboring surfaces.
#[inline]
fn image_vertices<R>(
	image: &UiImageDrawElement,
	viewport: Extent,
	sx: f32,
	sy: f32,
	consume: impl FnOnce([UiImageVertex; 4]) -> R,
) -> Option<R> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
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
		return None;
	}

	let u0 = ((x0 - original_x0) / rect_width).clamp(0.0, 1.0);
	let v0 = ((y0 - original_y0) / rect_height).clamp(0.0, 1.0);
	let u1 = ((x1 - original_x0) / rect_width).clamp(0.0, 1.0);
	let v1 = ((y1 - original_y0) / rect_height).clamp(0.0, 1.0);
	let feather_mask = scaled_feather_mask(image.feather_mask, sx, sy);

	let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
	let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;

	Some(consume([
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
	]))
}

/// Builds reference image geometry without retaining surface quads.
#[cfg(test)]
pub(super) fn build_ui_image_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	arena: &'a bumpalo::Bump,
) -> UiImageGeometry<'a> {
	build_ui_image_geometry_cached(draw_list, viewport, arena, None)
}
