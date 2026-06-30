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
		let center_x = element.position.x() as f32 + element.size.x() as f32 * 0.5;
		let center_y = element.position.y() as f32 + element.size.y() as f32 * 0.5;
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
		let left = element.position.x() as f32;
		let top = element.position.y() as f32;
		let right = left + element.size.x() as f32;
		let bottom = top + element.size.y() as f32;

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

		let x = clamp_to_u32(min_x.round());
		let y = clamp_to_u32(min_y.round());
		let width = clamp_to_u32((max_x - min_x).round());
		let height = clamp_to_u32((max_y - min_y).round());

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

fn clamp_to_u32(value: f32) -> u32 {
	if !value.is_finite() || value <= 0.0 {
		0
	} else if value >= u32::MAX as f32 {
		u32::MAX
	} else {
		value as u32
	}
}
