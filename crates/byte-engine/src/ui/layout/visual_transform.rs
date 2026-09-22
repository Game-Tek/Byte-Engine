use super::{LayoutElement, Location3, Size};
use crate::ui::{Transform, transform::Rotation};

/// Keeps a subtree's unrotated placement apart from its turn, so bounds stay axis aligned.
#[derive(Clone, Copy)]
pub(super) struct Affine2 {
	a: f32,
	b: f32,
	c: f32,
	d: f32,
	tx: f32,
	ty: f32,
	/// The turn from this unrotated space to the screen.
	pub(super) rotation: Rotation,
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
			rotation: Rotation::IDENTITY,
		}
	}

	pub(super) fn from_transform(transform: Transform, element: &LayoutElement) -> Self {
		// Resolve the pivot in layout space before composing inherited transforms.
		let origin_x = element.position.x() + element.size.x() * sanitize_origin(transform.origin.x);
		let origin_y = element.position.y() + element.size.y() * sanitize_origin(transform.origin.y);
		let scale_x = sanitize_scale(transform.scale_x);
		let scale_y = sanitize_scale(transform.scale_y);

		Self {
			a: scale_x,
			b: 0.0,
			c: 0.0,
			d: scale_y,
			tx: origin_x + sanitize_offset(transform.translate_x) - origin_x * scale_x,
			ty: origin_y + sanitize_offset(transform.translate_y) - origin_y * scale_y,
			// Translation follows the turn, so the pivot moves with it.
			rotation: Rotation::about(
				transform.rotation,
				origin_x + sanitize_offset(transform.translate_x),
				origin_y + sanitize_offset(transform.translate_y),
			),
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
			rotation: if rhs.rotation.is_identity() {
				self.rotation
			} else {
				// Carry the local turn through this placement so its pivot lands where the pivot is drawn.
				let Rotation { cos, sin, x, y } = rhs.rotation;
				self.rotation.after(Rotation {
					cos,
					sin,
					x: self.a * x + self.tx - (cos * self.tx - sin * self.ty),
					y: self.d * y + self.ty - (sin * self.tx + cos * self.ty),
				})
			},
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
		// A position may be negative once panned past the viewport; only sizes are clamped.
		let x = sanitize_offset(min_x);
		let y = sanitize_offset(min_y);
		let width = clamp_coordinate(max_x - min_x);
		let height = clamp_coordinate(max_y - min_y);

		(Location3::new(x, y, element.position.z()), Size::new(width, height))
	}
}

fn sanitize_offset(value: f32) -> f32 {
	if value.is_finite() { value } else { 0.0 }
}

fn sanitize_scale(value: f32) -> f32 {
	if value.is_finite() { value.max(0.0) } else { 1.0 }
}

fn sanitize_origin(value: f32) -> f32 {
	if value.is_finite() { value } else { 0.5 }
}

fn clamp_coordinate(value: f32) -> f32 {
	if !value.is_finite() || value <= 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
	use std::num::NonZeroU32;

	use super::*;
	use crate::ui::{Container, Context, ElementContext, Engine, UiPoint, intersection::HitTest};

	#[test]
	fn scaling_origin_keeps_child_rendering_and_retained_hits_on_the_same_bounds() {
		for (origin, expected) in [
			(UiPoint::zero(), UiPoint::new(30.0, 35.0)),
			(UiPoint::new(0.5, 0.5), UiPoint::new(50.0, 50.0)),
			(UiPoint::new(1.5, -0.5), UiPoint::new(90.0, 20.0)),
		] {
			let allocator = bumpalo::Bump::new();
			let mut engine = Engine::new();
			engine.mount(move |ctx| {
				Box::pin(async move {
					let mut root = ctx.element("root").container(Container::default().hit_testable(false));
					let mut parent = root.element("parent").container(
						Container::default()
							.absolute_position(20, 30)
							.width(80.into())
							.height(60.into())
							.hit_testable(false)
							.transform(Transform::identity().origin(origin).scale(0.5).translate(10.0, 5.0)),
					);
					parent
						.element("child")
						.container(Container::default().width(20.into()).height(10.into()));
				})
			});
			let mut snapshot = engine.evaluate(Size::new(200, 150), &allocator);
			let render = engine.render(&mut snapshot);
			let mut hits = HitTest::default();
			snapshot.retain_hit_test(&mut hits);
			let pointer = UiPoint::new((expected.x + 1.0) / 100.0 - 1.0, 1.0 - (expected.y + 1.0) / 75.0);
			let child = hits.query(pointer).expect("the transformed child accepts the pointer");
			let bounds = hits.bounds(child).unwrap();
			let visual = render.elements().find(|element| element.id == child.get()).unwrap();
			assert_eq!((bounds.x(), bounds.y()), (expected.x, expected.y));
			assert_eq!((bounds.width(), bounds.height()), (10.0, 5.0));
			assert_eq!(visual.position, bounds.position);
			assert_eq!(visual.size, bounds.size);
		}
	}

	#[test]
	fn rotation_turns_a_subtree_around_the_translated_pivot_and_keeps_bounds_unrotated() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default().hit_testable(false));
				let mut parent = root.element("parent").container(
					Container::default()
						.absolute_position(20, 30)
						.width(80.into())
						.height(60.into())
						.transform(
							Transform::identity()
								.origin(UiPoint::zero())
								.scale(0.5)
								.rotate(std::f32::consts::FRAC_PI_2)
								.translate(10.0, 5.0),
						),
				);
				parent
					.element("child")
					.container(Container::default().width(20.into()).height(10.into()));
			})
		});
		let mut snapshot = engine.evaluate(Size::new(200, 150), &allocator);
		let render = engine.render(&mut snapshot);
		let child = render
			.elements()
			.find(|element| (element.size.x(), element.size.y()) == (10.0, 5.0))
			.expect("the child keeps its unrotated scaled size");
		assert_eq!((child.position.x(), child.position.y()), (30.0, 35.0));
		let rotation = child.rotation.expect("the child inherits its parent's turn");
		// The pivot is the parent's translated top left corner, so it stays put.
		let (x, y) = rotation.apply(30.0, 35.0);
		assert!((x - 30.0).abs() < 0.001 && (y - 35.0).abs() < 0.001);
		// A quarter turn clockwise sends the child's top right corner straight down.
		let (x, y) = rotation.apply(40.0, 35.0);
		assert!((x - 30.0).abs() < 0.001 && (y - 45.0).abs() < 0.001);
		assert!(render.root().rotation.is_none());
	}

	#[test]
	fn transformed_rect_preserves_subpixel_motion_at_retina_scale() {
		let element = LayoutElement {
			id: NonZeroU32::new(1).expect("expected test value"),
			index: 0,
			position: Location3::new(10, 10, 0),
			size: Size::new(100, 40),
			hit_testable: false,
			sector: None,
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
