use math::Base as _;

use super::{LayoutElement, Location3, Size};
use crate::ui::Transform;

#[derive(Clone, Copy)]
pub(super) struct Affine2 {
	a: f32,
	b: f32,
	c: f32,
	d: f32,
	tx: f32,
	ty: f32,
}

impl Affine2 {
	pub(super) fn identity() -> Self {
		Self {
			a: 1.0,
			b: 0.0,
			c: 0.0,
			d: 1.0,
			tx: 0.0,
			ty: 0.0,
		}
	}

	pub(super) fn from_transform(transform: Transform, element: &LayoutElement) -> Self {
		let center_x = element.position.x() + element.size.x() * 0.5;
		let center_y = element.position.y() + element.size.y() * 0.5;
		let scale_x = sanitize_scale(transform.scale_x);
		let scale_y = sanitize_scale(transform.scale_y);

		Self {
			a: scale_x,
			b: 0.0,
			c: 0.0,
			d: scale_y,
			tx: center_x + sanitize_offset(transform.translate_x) - center_x * scale_x,
			ty: center_y + sanitize_offset(transform.translate_y) - center_y * scale_y,
		}
	}

	pub(super) fn compose(self, rhs: Self) -> Self {
		Self {
			a: self.a * rhs.a + self.c * rhs.b,
			b: self.b * rhs.a + self.d * rhs.b,
			c: self.a * rhs.c + self.c * rhs.d,
			d: self.b * rhs.c + self.d * rhs.d,
			tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
			ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
		}
	}

	fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
		(self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
	}

	pub(super) fn transform_rect(self, element: &LayoutElement) -> (Location3, Size) {
		let left = element.position.x();
		let top = element.position.y();
		let right = left + element.size.x();
		let bottom = top + element.size.y();

		let corners = [
			self.transform_point(left, top),
			self.transform_point(right, top),
			self.transform_point(right, bottom),
			self.transform_point(left, bottom),
		];

		let mut min_x = f32::INFINITY;
		let mut min_y = f32::INFINITY;
		let mut max_x = f32::NEG_INFINITY;
		let mut max_y = f32::NEG_INFINITY;

		for (x, y) in corners {
			min_x = min_x.min(x);
			min_y = min_y.min(y);
			max_x = max_x.max(x);
			max_y = max_y.max(y);
		}

		// Preserve fractional visual bounds so display scaling cannot magnify logical-pixel snapping.
		let x = clamp_coordinate(min_x);
		let y = clamp_coordinate(min_y);
		let width = clamp_coordinate(max_x - min_x);
		let height = clamp_coordinate(max_y - min_y);

		(Location3::new(x, y, element.position.z()), Size::new(width, height))
	}
}

fn sanitize_offset(value: f32) -> f32 {
	if value.is_finite() {
		value
	} else {
		0.0
	}
}

fn sanitize_scale(value: f32) -> f32 {
	if value.is_finite() {
		value.max(0.0)
	} else {
		1.0
	}
}

fn clamp_coordinate(value: f32) -> f32 {
	if !value.is_finite() || value <= 0.0 {
		0.0
	} else {
		value
	}
}

#[cfg(test)]
mod tests {
	use std::num::NonZeroU32;

	use super::*;

	#[test]
	fn transformed_rect_preserves_subpixel_motion_at_retina_scale() {
		let element = LayoutElement {
			id: NonZeroU32::new(1).unwrap(),
			position: Location3::new(10, 10, 0),
			size: Size::new(100, 40),
			hit_testable: false,
		};
		let physical_positions = (0..=10)
			.map(|step| {
				let transform = Transform::identity().translate_x(step as f32 * 0.1);
				let (position, _) = Affine2::from_transform(transform, &element).transform_rect(&element);
				position.x() * 2.0
			})
			.collect::<Vec<_>>();

		assert!(
			physical_positions
				.windows(2)
				.all(|positions| (positions[1] - positions[0] - 0.2).abs() < 0.0001),
			"smooth motion was quantized into physical positions: {physical_positions:?}"
		);
	}
}
