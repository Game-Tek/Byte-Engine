//! UI rectangle, blur, curve, and image geometry generation.

use super::*;

pub(super) fn build_ui_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	frame_allocator: &'a bumpalo::Bump,
) -> UiGeometry<'a> {
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

pub(super) fn build_ui_curve_geometry<'a>(
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

pub(super) fn flatten_curve_segment(
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

pub(super) fn push_scaled_point(
	points: &mut Vec<CurvePoint, &bumpalo::Bump>,
	point: CurvePoint,
	origin: [f32; 2],
	sx: f32,
	sy: f32,
) {
	let point = scaled_curve_point(point, origin, sx, sy);
	if point.is_finite() {
		points.push(point);
	}
}

pub(super) fn scaled_curve_point(point: CurvePoint, origin: [f32; 2], sx: f32, sy: f32) -> CurvePoint {
	CurvePoint::new((origin[0] + point.x) * sx, (origin[1] + point.y) * sy)
}

pub(super) fn flatten_quadratic(
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

pub(super) fn flatten_cubic(
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

pub(super) fn midpoint(a: CurvePoint, b: CurvePoint) -> CurvePoint {
	CurvePoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

pub(super) fn point_line_distance(point: CurvePoint, from: CurvePoint, to: CurvePoint) -> f32 {
	let dx = to.x - from.x;
	let dy = to.y - from.y;
	let length = dx.hypot(dy);
	if length <= 0.0001 {
		return (point.x - from.x).hypot(point.y - from.y);
	}
	((point.x - from.x) * dy - (point.y - from.y) * dx).abs() / length
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

pub(super) fn build_ui_image_geometry<'a>(
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
