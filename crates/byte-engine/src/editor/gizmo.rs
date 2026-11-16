//! The transform gizmo, which moves, turns, and scales the selection, one [`Tool`] at a time.
//!
//! The move tool has three arrows, three plane handles, and a center dot. The rotate tool has a ring around each world
//! axis and a wider ring that turns around the view. The scale tool has a square-capped handle along each of the
//! selection's own axes, the same plane handles, and the center dot, which scales evenly.
//!
//! The gizmo draws in its own render pass, a pixel step that draws the vector shapes in `editor/gizmo.besl` over the
//! frame. This module places the gizmo on the screen, measures the same shapes on the CPU, so the part the pointer
//! grabs is the part the pixel shows, and turns pointer motion into a new transform for the grabbed part.
//!
//! The step and the editor share a [`State`]: the editor says where the gizmo stands, which tool it shows, and which
//! parts are lit, and the step reports the view it drew through, which the editor picks and drags through.

use std::cell::RefCell;
use std::f32::consts::{FRAC_1_SQRT_2, TAU};
use std::rc::Rc;

use maths_rs::{Quatf, Vec2f, Vec3f, Vec4f};

use crate::core::factory::Handle;
use crate::gameplay::Transform;
use crate::ghi;
use crate::ghi::context::ContextCreate as _;
use crate::ghi::frame::Frame as _;
use crate::math::{self, Orientation, Point, Scale};
use crate::rendering::render_pass::RenderPassBuilder;
use crate::rendering::render_pass::pixel_steps::PixelStep;
use crate::rendering::render_pass::simple_tile::Resource;
use crate::rendering::{Sink, View};
use crate::utils::Extent;

/// The gizmo's render pipeline, under the game's assets.
pub(crate) const PIPELINE: &str = "byte_engine/editor/gizmo.pipeline";

/// How long an arrow looks on screen, in pixels, when it lies across the view, wherever the gizmo is.
const ARROW_PIXELS: f32 = 110.0;
/// How close to a part, in pixels, the pointer grabs it. Wider than the part, so thin shafts are easy to grab.
const GRAB_PIXELS: f32 = 5.0;
/// How visible a part must be for the pointer to grab it. Fainter parts still draw, but are seen too edge-on to drag
/// precisely.
const GRAB_VISIBILITY: f32 = 0.5;
/// The shortest an arrow is drawn, in pixels, so an arrow pointing nearly at the camera still has room for its head
/// and can be grabbed.
const MIN_ARROW_PIXELS: f32 = 50.0;
/// How short, in pixels, an arrow's true projection may be before it fades out entirely, and the length over which it
/// fades in. Below this, the camera looks straight down the axis and the arrow has no direction on screen.
const ARROW_FADE_START: f32 = 4.0;
const ARROW_FADE_LENGTH: f32 = 8.0;
/// The shortest true projection, in pixels, a drag along an arrow scales by, so dragging an arrow that points nearly
/// at the camera doesn't fling the selection.
const MIN_DRAG_PIXELS: f32 = 30.0;
/// The fewest pixels the pointer must move along a ring to turn the selection one radian, so grabbing a ring where it
/// runs toward the camera doesn't spin the selection.
const MIN_TURN_PIXELS: f32 = 60.0;
/// The shortest distance from the center, in pixels, a scale drag measures its stretch against, so grabbing a handle
/// near the center doesn't scale by leaps.
const MIN_STRETCH_PIXELS: f32 = 10.0;
/// The smallest factor a scale drag scales by, so the selection never collapses to nothing or turns inside out.
const MIN_STRETCH: f32 = 0.01;
/// How far around the shapes the shader draws, in pixels: the widest shape's half width and antialiasing.
const MARGIN_PIXELS: f32 = 8.0;

// The shapes, in pixels or, for the plane handles and rings, in fractions of an arrow. These must match `gizmo.besl`.
const CENTER_GAP: f32 = 11.0;
const SHAFT_HALF_WIDTH: f32 = 2.0;
const HEAD_LENGTH: f32 = 15.0;
const HEAD_HALF_WIDTH: f32 = 6.5;
const TIP_ROUNDING: f32 = 1.0;
const BASE_ROUNDING: f32 = 3.0;
const HEAD_TAPER: f32 = 4.0;
const CAP_HALF: f32 = 5.5;
const CAP_ROUNDING: f32 = 1.5;
const CENTER_RADIUS: f32 = 5.0;
const PLANE_CENTER: f32 = 0.3;
const PLANE_HALF: f32 = 0.075;
const PLANE_ROUNDING: f32 = 1.0;
const RING_RADIUS: f32 = 0.85;
const RING_HALF_WIDTH: f32 = 1.75;
const RING_SEGMENTS: usize = 64;
const VIEW_RING_PIXELS: f32 = 112.0;

/// The `Tool` enum names what the gizmo does to the selection, which the toolbar picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tool {
	#[default]
	Move,
	Rotate,
	Scale,
}

impl Tool {
	/// Every tool, in the toolbar's order.
	pub(crate) const ALL: [Tool; 3] = [Tool::Move, Tool::Rotate, Tool::Scale];

	/// The tool's name on its toolbar button.
	pub(crate) fn label(self) -> &'static str {
		match self {
			Tool::Move => "Move",
			Tool::Rotate => "Rotate",
			Tool::Scale => "Scale",
		}
	}

	/// The world directions the gizmo's three axes point along for a selection turned by `orientation`.
	///
	/// Moving and turning work along the world's axes. Scaling works along the selection's own, since those are the
	/// axes a transform's scale stretches.
	fn axes(self, orientation: Orientation) -> [Vec3f; 3] {
		match self {
			Tool::Move | Tool::Rotate => [
				Vec3f::new(1.0, 0.0, 0.0),
				Vec3f::new(0.0, 1.0, 0.0),
				Vec3f::new(0.0, 0.0, 1.0),
			],
			Tool::Scale => orientation.axes::<math::WorldSpace>().map(|axis| axis.into_maths()),
		}
	}
}

/// The `Part` enum names what the pointer can grab on the gizmo, numbered as `gizmo.besl` numbers its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
	/// The arrow or, for the scale tool, the square-capped handle along X.
	AxisX = 1,
	AxisY,
	AxisZ,
	/// The handle in the plane facing X, which moves or scales along Y and Z.
	PlaneX,
	PlaneY,
	PlaneZ,
	/// The dot at the center, which moves in the plane facing the camera, or scales evenly.
	Center,
	/// The ring around X, which turns around X.
	RingX,
	RingY,
	RingZ,
	/// The ring around the others, which turns around the view's direction.
	RingView,
}

impl Part {
	const AXES: [Part; 3] = [Part::AxisX, Part::AxisY, Part::AxisZ];
	const PLANES: [Part; 3] = [Part::PlaneX, Part::PlaneY, Part::PlaneZ];
	const RINGS: [Part; 3] = [Part::RingX, Part::RingY, Part::RingZ];

	/// The gizmo axis, 0 to 2, that an axis handle points along, a plane handle faces, or a ring turns around.
	fn axis(self) -> usize {
		match self {
			Part::AxisX | Part::PlaneX | Part::RingX => 0,
			Part::AxisY | Part::PlaneY | Part::RingY => 1,
			Part::AxisZ | Part::PlaneZ | Part::RingZ | Part::Center | Part::RingView => 2,
		}
	}

	/// The number the shader lights a part by, 0 for none.
	fn shader_index(part: Option<Part>) -> f32 {
		part.map_or(0.0, |part| part as u8 as f32)
	}
}

/// The `State` struct is what the editor and the gizmo's render pass share: the editor writes where the gizmo
/// stands, which tool it shows, and which parts are lit, and the pass writes the view it drew through, so the editor
/// picks what it shows.
#[derive(Default)]
pub(crate) struct State {
	/// The selection's transform, which the gizmo stands at, or `None` to draw nothing.
	pub(crate) target: Option<Transform>,
	/// What the gizmo does to the selection.
	pub(crate) tool: Tool,
	/// The camera whose view shows the gizmo, or `None` for every view.
	pub(crate) camera: Option<Handle>,
	/// The part under the pointer.
	pub(crate) hovered: Option<Part>,
	/// The part being dragged.
	pub(crate) active: Option<Part>,
	/// While scaling, how much each axis handle stretches on screen, so the gizmo shows the scale it applies.
	pub(crate) stretch: Option<[f32; 3]>,
	/// The view and pixel extent the gizmo was last drawn through.
	pub(crate) view: Option<(View, Extent)>,
}

impl State {
	/// Places the gizmo for the current tool at `target` in the view it was last drawn through, or returns `None`
	/// before it was drawn or while `target` is behind the camera.
	pub(crate) fn placement(&self, target: &Transform) -> Option<Placement> {
		let (view, extent) = self.view?;
		Placement::new(view, extent, target, self.tool)
	}
}

/// The `Placement` struct is where the gizmo lands in one view, which both the shader and picking measure the gizmo
/// through, so they agree on what the pointer is over.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
	/// The view the gizmo is drawn through.
	view: View,
	/// The tool whose shapes the gizmo shows.
	tool: Tool,
	/// The camera's position in the world.
	camera: Vec3f,
	/// Maps a pixel's coordinate and reverse-Z depth to a homogeneous offset from the camera.
	pixel_to_offset: math::Matrix,
	/// The world length of one arrow.
	size: f32,
	/// The world directions of the gizmo's three axes.
	axes: [Vec3f; 3],
	/// The unit direction from the camera to the gizmo's center.
	view_direction: Vec3f,
	/// The gizmo's center, in pixels.
	center: Vec2f,
	/// Each axis's true projection, in pixels: where one arrow's length along it lands on screen, from the center.
	screen_axes: [Vec2f; 3],
	/// The tip of each arrow as drawn, in pixels, by axis: along its projection, and at least [`MIN_ARROW_PIXELS`] long.
	tips: [Vec2f; 3],
	/// How visible each arrow and then each plane handle is, from 0 to 1, by axis.
	visibility: [f32; 6],
	/// How far each axis points away from the camera, from -1 to 1: the view direction along it.
	facing: [f32; 3],
	/// Per axis, 1 or -1: the side of the center the plane handles sit on, the side facing the camera.
	quadrant: [f32; 3],
	/// The pixel rectangle the gizmo can cover: left, top, right, bottom.
	rect: [f32; 4],
	/// The size of the image the gizmo is drawn into.
	extent: Extent,
}

impl Placement {
	/// Places the gizmo of `tool` at `target` in `view`, drawn at `extent`, so an arrow across the view is
	/// [`ARROW_PIXELS`] long.
	///
	/// Returns `None` when `target` is behind the camera, where there's nothing to draw or grab.
	pub(crate) fn new(view: View, extent: Extent, target: &Transform, tool: Tool) -> Option<Self> {
		let height = extent.height() as f32;
		let target_maths = target.get_position().into_maths();
		let center = to_pixel(view, extent, target_maths)?;
		// The translation column of the inverse view is the camera's world position.
		let camera = Vec3f::from(math::inverse(view.view()).get_column(3));
		let offset = target_maths - camera;
		let distance = maths_rs::length(offset);
		if distance <= view.near() {
			return None;
		}
		// On screen, things shrink with their depth along the view, not their distance from the camera, so the gizmo
		// scales with its depth to keep one size anywhere on the screen.
		let depth = (view.view() * Vec4f::new(target_maths.x, target_maths.y, target_maths.z, 1.0))
			.z
			.abs();
		let pixel_angle = 2.0 * (view.y_fov().to_radians().value() * 0.5).tan() / height;
		let size = depth * pixel_angle * ARROW_PIXELS;
		// Without the view's translation, the inverse lands relative to the camera instead of the world origin.
		let mut rotation = view.view();
		rotation.set_column(3, Vec4f::new(0.0, 0.0, 0.0, 1.0));
		let pixel_to_offset = math::inverse(view.projection() * rotation) * pixel_to_ndc(extent.width() as f32, height);

		// How squarely the camera looks along each axis. A plane handle fades as the camera sees its edge, and an arrow
		// only once it points so straight at the camera that it has no direction on screen.
		let axes = tool.axes(target.get_orientation());
		let view_direction = offset / distance;
		let facing = axes.map(|axis| maths_rs::dot(view_direction, axis));
		let mut visibility = [0.0; 6];
		let mut screen_axes = [Vec2f::new(0.0, 0.0); 3];
		let mut tips = [center; 3];
		// Each axis follows the projection's slope at the center, so perspective doesn't stretch an arrow that points
		// toward the camera: an arrow only shortens by how much it turns toward the view.
		const STEP: f32 = 0.001;
		for axis in 0..3 {
			let projected = to_pixel(view, extent, target_maths + axes[axis] * (size * STEP))
				.map_or(Vec2f::new(0.0, 0.0), |step| (step - center) * (1.0 / STEP));
			let length = maths_rs::length(projected);
			screen_axes[axis] = projected;
			visibility[axis] = ((length - ARROW_FADE_START) / ARROW_FADE_LENGTH).clamp(0.0, 1.0);
			visibility[axis + 3] = ((facing[axis].abs() - 0.15) / 0.15).clamp(0.0, 1.0);
			tips[axis] = center + projected * (length.max(MIN_ARROW_PIXELS) / length.max(0.0001));
		}
		let quadrant = facing.map(|along| if along > 0.0 { -1.0 } else { 1.0 });

		let rect = match tool {
			Tool::Move | Tool::Scale => handle_bounds(center, tips, quadrant),
			// A ring `center + u cos t + v sin t` reaches `sqrt(u.x² + v.x²)` either side of the center across, and
			// likewise down; the view ring reaches its radius.
			Tool::Rotate => {
				let mut reach = Vec2f::new(VIEW_RING_PIXELS, VIEW_RING_PIXELS);
				for axis in 0..3 {
					let [u, v] = ring_axes(&screen_axes, axis);
					reach.x = reach.x.max((u.x * u.x + v.x * v.x).sqrt());
					reach.y = reach.y.max((u.y * u.y + v.y * v.y).sqrt());
				}
				bounds([center - reach, center + reach].into_iter())
			}
		};
		Some(Self {
			view,
			tool,
			camera,
			pixel_to_offset,
			size,
			axes,
			view_direction,
			center,
			screen_axes,
			tips,
			visibility,
			facing,
			quadrant,
			rect,
			extent,
		})
	}

	/// Converts a pointer position in normalized window coordinates, -1 to 1 with y up, to a pixel of the image the
	/// gizmo is drawn into, as [`Self::pick`] and [`grab`] take it.
	pub(crate) fn pixel(&self, pointer: [f32; 2]) -> [f32; 2] {
		pixel(pointer, self.extent)
	}

	/// Returns the world direction from the camera through `pixel`, which may fall anywhere within a pixel.
	fn direction(&self, pixel: [f32; 2]) -> Vec3f {
		// `pixel_to_offset` puts pixel centers at whole coordinates, and `pixel` puts them half a pixel in.
		let far = self.pixel_to_offset * Vec4f::new(pixel[0] - 0.5, pixel[1] - 0.5, 0.0, 1.0);
		maths_rs::normalize(Vec3f::from(far) / far.w)
	}

	/// The signed distance from `p` to the ring around `axis`, and how far the ring's nearest point lies behind the
	/// center, along the view, as a fraction of the ring's radius; with the angle of that point around the axis.
	fn ring(&self, p: Vec2f, axis: usize) -> (f32, f32, f32) {
		let [u, v] = ring_axes(&self.screen_axes, axis);
		let [first, second] = ring_basis(axis);
		ring(p, self.center, u, v, self.facing[first], self.facing[second])
	}

	/// Returns the part within [`GRAB_PIXELS`] of `pixel`, taking them in the order the shader draws them from the
	/// top: the center, then the nearest axis handle, then the nearest plane handle; or, for the rotate tool, the
	/// nearest ring.
	pub(crate) fn pick(&self, pixel: [f32; 2]) -> Option<Part> {
		let p = Vec2f::new(pixel[0], pixel[1]);
		let nearest = |parts: &mut dyn Iterator<Item = (f32, Part)>| {
			parts
				.filter(|&(distance, _)| distance < GRAB_PIXELS)
				.min_by(|a, b| a.0.total_cmp(&b.0))
				.map(|(_, part)| part)
		};
		if self.tool == Tool::Rotate {
			let view_ring = (maths_rs::length(p - self.center) - VIEW_RING_PIXELS).abs() - RING_HALF_WIDTH;
			let rings = (0..3).map(|axis| (self.ring(p, axis).0 - RING_HALF_WIDTH, Part::RINGS[axis]));
			return nearest(&mut rings.chain([(view_ring, Part::RingView)]));
		}
		if maths_rs::length(p - self.center) - CENTER_RADIUS < GRAB_PIXELS {
			return Some(Part::Center);
		}
		let visible = |first: usize| move |&axis: &usize| self.visibility[first + axis] >= GRAB_VISIBILITY;
		let mut handles = (0..3).filter(visible(0)).map(|axis| {
			let distance = match self.tool {
				Tool::Scale => scale_handle(p, self.center, self.tips[axis], SHAFT_HALF_WIDTH),
				_ => arrow(p, self.center, self.tips[axis], SHAFT_HALF_WIDTH),
			};
			(distance, Part::AXES[axis])
		});
		let mut planes = (0..3).filter(visible(3)).map(|axis| {
			// The handle facing an axis lies in the plane of the other two.
			let (first, second) = ((axis + 1) % 3, (axis + 2) % 3);
			let (first, second) = (first.min(second), first.max(second));
			let distance = plane(
				p,
				self.center,
				[self.tips[first] - self.center, self.tips[second] - self.center],
				[self.quadrant[first], self.quadrant[second]],
			);
			(distance, Part::PLANES[axis])
		});
		nearest(&mut handles).or_else(|| nearest(&mut planes))
	}

	/// The motion of turning around `axis` by the ring point `point`, grabbed at `pixel`: the pointer turns the
	/// selection by how far it moves along the direction the point moves on screen as the ring turns.
	fn turn(&self, axis: Vec3f, origin: Vec3f, point: Vec3f, pixel: Vec2f) -> Option<Motion> {
		const STEP: f32 = 0.01;
		let turned = origin + Quatf::from_axis_angle(axis, STEP) * (point - origin);
		let motion = (to_pixel(self.view, self.extent, turned)? - to_pixel(self.view, self.extent, point)?) / STEP;
		let pixels_per_radian = maths_rs::length(motion);
		Some(Motion::Turn {
			axis,
			pixel,
			tangent: motion / pixels_per_radian.max(0.0001),
			radians_per_pixel: 1.0 / pixels_per_radian.max(MIN_TURN_PIXELS),
		})
	}
}

/// The two gizmo axes whose plane the ring around `axis` lies in, in the order that turns positively around it.
fn ring_basis(axis: usize) -> [usize; 2] {
	[(axis + 1) % 3, (axis + 2) % 3]
}

/// The ring around `axis` on screen, as the two vectors from its center to where its basis axes cross it, in pixels.
fn ring_axes(screen_axes: &[Vec2f; 3], axis: usize) -> [Vec2f; 2] {
	ring_basis(axis).map(|basis| screen_axes[basis] * RING_RADIUS)
}

/// Returns the pixel rectangle the move and scale gizmos cover, as [`Placement::rect`] holds it: the arrows from
/// `center` to `tips`, and the plane handles between each pair of arrows.
///
/// A plane handle sits on the camera's side of each axis, the sides `quadrant` holds, so it can lie opposite an arrow,
/// outside the arrows' own hull. Each handle lies within the parallelogram its two arms span from the center.
fn handle_bounds(center: Vec2f, tips: [Vec2f; 3], quadrant: [f32; 3]) -> [f32; 4] {
	let reach = PLANE_CENTER + PLANE_HALF;
	let arms = [0, 1, 2].map(|axis| (tips[axis] - center) * (reach * quadrant[axis]));
	let corners = [(0, 1), (0, 2), (1, 2)].map(|(first, second)| center + arms[first] + arms[second]);
	bounds(
		[center]
			.into_iter()
			.chain(tips)
			.chain(arms.map(|arm| center + arm))
			.chain(corners),
	)
}

/// Returns the pixel rectangle around `points`, widened by [`MARGIN_PIXELS`] for the shapes' widths: left, top, right,
/// bottom.
fn bounds(points: impl Iterator<Item = Vec2f>) -> [f32; 4] {
	let [mut left, mut top, mut right, mut bottom] = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
	for point in points {
		[left, top, right, bottom] = [left.min(point.x), top.min(point.y), right.max(point.x), bottom.max(point.y)];
	}
	[
		left - MARGIN_PIXELS,
		top - MARGIN_PIXELS,
		right + MARGIN_PIXELS,
		bottom + MARGIN_PIXELS,
	]
}

/// Returns the pixel `point` shows in `view`, drawn at `extent`, or `None` behind the camera.
fn to_pixel(view: View, extent: Extent, point: Vec3f) -> Option<Vec2f> {
	let ndc = view.project(Point::from_maths(point))?;
	let [x, y] = pixel(ndc, extent);
	Some(Vec2f::new(x, y))
}

/// Returns the matrix that takes a pixel's coordinate to the normalized device coordinates of its center, whose y
/// points up, for an image `width` by `height` pixels. The z and w components pass through.
fn pixel_to_ndc(width: f32, height: f32) -> math::Matrix {
	math::Matrix::from((
		Vec4f::new(2.0 / width, 0.0, 0.0, 1.0 / width - 1.0),
		Vec4f::new(0.0, -2.0 / height, 0.0, 1.0 - 1.0 / height),
		Vec4f::new(0.0, 0.0, 1.0, 0.0),
		Vec4f::new(0.0, 0.0, 0.0, 1.0),
	))
}

/// The z component of the cross product of `a` and `b`. Mirrors `cross2` in `gizmo.besl`.
fn cross2(a: Vec2f, b: Vec2f) -> f32 {
	a.x * b.y - a.y * b.x
}

/// The distance from `p` to the segment from `a` to `b`. Mirrors `segment` in `gizmo.besl`.
fn segment(p: Vec2f, a: Vec2f, b: Vec2f) -> f32 {
	let (pa, ba) = (p - a, b - a);
	let h = (maths_rs::dot(pa, ba) / maths_rs::dot(ba, ba).max(0.0001)).clamp(0.0, 1.0);
	maths_rs::length(pa - ba * h)
}

/// The signed distance from `p` to the convex quadrilateral `corners`, negative inside, whichever way its corners
/// wind. Mirrors `quadrilateral` in `gizmo.besl`.
fn quadrilateral(p: Vec2f, corners: [Vec2f; 4]) -> f32 {
	let edges = (0..4).map(|index| (corners[index], corners[(index + 1) % 4]));
	let distance = edges.clone().map(|(a, b)| segment(p, a, b)).fold(f32::MAX, f32::min);
	let sides = edges.map(|(a, b)| cross2(b - a, p - a));
	let inside = sides.clone().all(|side| side >= 0.0) || sides.clone().all(|side| side <= 0.0);
	if inside { -distance } else { distance }
}

/// The signed distance from `p` to the arrow from `center` to `tip`. Mirrors `arrow` in `gizmo.besl`.
fn arrow(p: Vec2f, center: Vec2f, tip: Vec2f, half_width: f32) -> f32 {
	let axis = tip - center;
	let direction = axis / maths_rs::length(axis).max(0.0001);
	let side = Vec2f::new(-direction.y, direction.x);
	// The head is the meeting of its two sides and its base, each corner rounded on its own.
	let offset = p - tip;
	let back = -maths_rs::dot(offset, direction);
	let lateral = maths_rs::dot(offset, side);
	let slant = (HEAD_LENGTH * HEAD_LENGTH + HEAD_HALF_WIDTH * HEAD_HALF_WIDTH).sqrt();
	let left = (lateral * HEAD_LENGTH - back * HEAD_HALF_WIDTH) / slant;
	let right = (-lateral * HEAD_LENGTH - back * HEAD_HALF_WIDTH) / slant;
	let tip_sides = -smooth_min(-left, -right, TIP_ROUNDING);
	let head = -smooth_min(-tip_sides, HEAD_LENGTH - back, BASE_ROUNDING);
	let shaft = segment(p, center + direction * CENTER_GAP, tip - direction * HEAD_LENGTH) - half_width;
	smooth_min(shaft, head, HEAD_TAPER)
}

/// The signed distance from `p` to the scale handle from `center` to `tip`: a shaft ending in a rounded square that
/// `tip` closes. Mirrors `scale_handle` in `gizmo.besl`.
fn scale_handle(p: Vec2f, center: Vec2f, tip: Vec2f, half_width: f32) -> f32 {
	let axis = tip - center;
	let direction = axis / maths_rs::length(axis).max(0.0001);
	let side = Vec2f::new(-direction.y, direction.x);
	let cap = tip - direction * CAP_HALF;
	let shaft = segment(p, center + direction * CENTER_GAP, cap) - half_width;
	// The cap is a box along the handle, measured from its middle.
	let offset = p - cap;
	let qx = maths_rs::dot(offset, direction).abs() - (CAP_HALF - CAP_ROUNDING);
	let qy = maths_rs::dot(offset, side).abs() - (CAP_HALF - CAP_ROUNDING);
	let outside = Vec2f::new(qx.max(0.0), qy.max(0.0));
	let cap = maths_rs::length(outside) + qx.max(qy).min(0.0) - CAP_ROUNDING;
	shaft.min(cap)
}

/// The polynomial smooth minimum of `a` and `b`, which fillets where they meet over a width of `k`. Mirrors
/// `smooth_min` in `gizmo.besl`.
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
	let h = (k - (a - b).abs()).max(0.0) / k;
	a.min(b) - h * h * k * 0.25
}

/// The signed distance from `p` to the handle in the plane of `arrows`, given from the center, on their `sides`.
/// Mirrors `plane` in `gizmo.besl`.
fn plane(p: Vec2f, center: Vec2f, [first, second]: [Vec2f; 2], sides: [f32; 2]) -> f32 {
	let near_first = first * ((PLANE_CENTER - PLANE_HALF) * sides[0]);
	let far_first = first * ((PLANE_CENTER + PLANE_HALF) * sides[0]);
	let near_second = second * ((PLANE_CENTER - PLANE_HALF) * sides[1]);
	let far_second = second * ((PLANE_CENTER + PLANE_HALF) * sides[1]);
	quadrilateral(
		p,
		[
			center + near_first + near_second,
			center + far_first + near_second,
			center + far_first + far_second,
			center + near_first + far_second,
		],
	) - PLANE_ROUNDING
}

/// The distance from `p` to the ring `center + u cos t + v sin t`, traced as [`RING_SEGMENTS`] segments; how far its
/// nearest point lies behind the center, from the view's direction along the ring's basis, `first` and `second`; and
/// that point's angle `t`. Mirrors `ring` in `gizmo.besl`.
fn ring(p: Vec2f, center: Vec2f, u: Vec2f, v: Vec2f, first: f32, second: f32) -> (f32, f32, f32) {
	let (step_sin, step_cos) = (TAU / RING_SEGMENTS as f32).sin_cos();
	let (mut cos, mut sin) = (1.0f32, 0.0f32);
	let mut previous = center + u;
	let mut nearest = (f32::MAX, 0.0, 0.0);
	for index in 0..RING_SEGMENTS {
		// Turning by the step each segment instead of evaluating the angle keeps the shader free of trigonometry.
		let (next_cos, next_sin) = (cos * step_cos - sin * step_sin, sin * step_cos + cos * step_sin);
		let next = center + u * next_cos + v * next_sin;
		let distance = segment(p, previous, next);
		if distance < nearest.0 {
			let (middle_cos, middle_sin) = ((cos + next_cos) * 0.5, (sin + next_sin) * 0.5);
			let angle = (index as f32 + 0.5) * TAU / RING_SEGMENTS as f32;
			nearest = (distance, middle_cos * first + middle_sin * second, angle);
		}
		(previous, cos, sin) = (next, next_cos, next_sin);
	}
	nearest
}

/// The `Motion` enum is how a drag turns pointer motion into a new transform: moving along an axis or across a plane,
/// turning around an axis, or stretching along some axes.
#[derive(Clone, Copy, Debug)]
enum Motion {
	/// Moves along `axis` by how far the pointer moves along the arrow on screen from where it grabbed it, `pixel`.
	///
	/// Measuring on screen keeps the drag steady however squarely the arrow points at the camera, where the pointer's
	/// ray barely crosses the axis.
	Axis {
		axis: Vec3f,
		pixel: Vec2f,
		/// The arrow's direction on screen.
		direction: Vec2f,
		/// How far the selection moves for each pixel the pointer moves along `direction`.
		world_per_pixel: f32,
	},
	/// Moves within the plane through `origin` facing `normal`, keeping under the pointer the point it grabbed, `grab`.
	Plane { origin: Vec3f, normal: Vec3f, grab: Vec3f },
	/// Turns around `axis` by how far the pointer moves along `tangent`, the grabbed ring's direction on screen, from
	/// where it grabbed it, `pixel`.
	///
	/// Following the ring's direction where it was grabbed turns steadily wherever the ring is grabbed, even seen
	/// edge-on, where the angle around the center on screen says nothing.
	Turn {
		axis: Vec3f,
		pixel: Vec2f,
		tangent: Vec2f,
		radians_per_pixel: f32,
	},
	/// Scales the axes `mask` holds by the factor the pointer stretches: how far it moves along `direction` from where
	/// it grabbed, `pixel`, against `base`, the grab's distance from the center along it, so the handle follows the
	/// pointer.
	Stretch {
		mask: Vec3f,
		pixel: Vec2f,
		direction: Vec2f,
		base: f32,
	},
}

/// Returns where the ray from `camera` along `direction` meets the plane through `origin` facing `normal`, or `None`
/// when it runs parallel to the plane or meets it behind the camera.
fn plane_hit(origin: Vec3f, normal: Vec3f, camera: Vec3f, direction: Vec3f) -> Option<Vec3f> {
	let facing = maths_rs::dot(direction, normal);
	if facing.abs() < 1e-4 {
		return None;
	}
	let t = maths_rs::dot(origin - camera, normal) / facing;
	(t > 0.0).then(|| camera + direction * t)
}

/// The `Drag` struct is one drag of a gizmo part, from the press that grabbed it to the release, which changes the
/// selection's transform by how far the pointer moved along the part.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
	pub(crate) part: Part,
	/// The selection's transform when the drag began.
	start: Transform,
	motion: Motion,
}

impl Drag {
	/// Returns the selection's transform after the pointer moved to `pixel`, or `None` while the pointer's ray misses
	/// the drag's plane, during which the selection should stay as it is.
	pub(crate) fn transform(&self, placement: &Placement, pixel: [f32; 2]) -> Option<Transform> {
		let p = Vec2f::new(pixel[0], pixel[1]);
		let mut transform = self.start.clone();
		let start = self.start.get_position().into_maths();
		match self.motion {
			Motion::Axis {
				axis,
				pixel: grabbed,
				direction,
				world_per_pixel,
			} => {
				let along = maths_rs::dot(p - grabbed, direction);
				transform.set_position(Point::from_maths(start + axis * (along * world_per_pixel)));
			}
			Motion::Plane { origin, normal, grab } => {
				let moved = plane_hit(origin, normal, placement.camera, placement.direction(pixel))? - grab;
				transform.set_position(Point::from_maths(start + moved));
			}
			Motion::Turn {
				axis,
				pixel: grabbed,
				tangent,
				radians_per_pixel,
			} => {
				let angle = maths_rs::dot(p - grabbed, tangent) * radians_per_pixel;
				let turn = Orientation::try_from_maths(Quatf::from_axis_angle(axis, angle)).ok()?;
				transform.set_orientation(turn.compose(self.start.get_orientation()));
			}
			Motion::Stretch { mask, .. } => {
				let factor = self.stretch_factor(p);
				let scale = self.start.scale().into_maths();
				let scaled = Vec3f::new(1.0, 1.0, 1.0) + mask * (factor - 1.0);
				transform.set_scale(Scale::from_maths(scale * scaled));
			}
		}
		Some(transform)
	}

	/// While scaling, how much each axis handle stretches on screen with the pointer at `pixel`, for
	/// [`State::stretch`]; `None` for the other tools.
	pub(crate) fn stretch(&self, pixel: [f32; 2]) -> Option<[f32; 3]> {
		let Motion::Stretch { mask, .. } = self.motion else {
			return None;
		};
		let factor = self.stretch_factor(Vec2f::new(pixel[0], pixel[1]));
		Some([0, 1, 2].map(|axis| 1.0 + mask[axis] * (factor - 1.0)))
	}

	/// The factor a stretch scales by with the pointer at `p`, 1 where it grabbed.
	fn stretch_factor(&self, p: Vec2f) -> f32 {
		let Motion::Stretch {
			pixel, direction, base, ..
		} = self.motion
		else {
			return 1.0;
		};
		(1.0 + maths_rs::dot(p - pixel, direction) / base).max(MIN_STRETCH)
	}
}

/// Begins dragging the part under `pixel` of the gizmo standing at `target`, or returns `None` when the pointer is over
/// no part.
pub(crate) fn grab(placement: &Placement, target: &Transform, pixel: [f32; 2]) -> Option<Drag> {
	let part = placement.pick(pixel)?;
	let origin = target.get_position().into_maths();
	let ray = placement.direction(pixel);
	let p = Vec2f::new(pixel[0], pixel[1]);
	let from_center = p - placement.center;
	let plane =
		|normal: Vec3f| plane_hit(origin, normal, placement.camera, ray).map(|grab| Motion::Plane { origin, normal, grab });
	let axis = part.axis();
	let motion = match (placement.tool, part) {
		(Tool::Move, Part::AxisX | Part::AxisY | Part::AxisZ) => {
			let projected = placement.tips[axis] - placement.center;
			Motion::Axis {
				axis: placement.axes[axis],
				pixel: p,
				direction: projected / maths_rs::length(projected).max(0.0001),
				world_per_pixel: placement.size / maths_rs::length(placement.screen_axes[axis]).max(MIN_DRAG_PIXELS),
			}
		}
		(Tool::Move, Part::PlaneX | Part::PlaneY | Part::PlaneZ) => plane(placement.axes[axis])?,
		// The dot moves in the plane facing the pointer's ray, so it stays under the pointer.
		(Tool::Move, Part::Center) => plane(ray)?,
		(Tool::Rotate, Part::RingView) => {
			// The view ring turns around the view's direction, through the point under the pointer in the plane facing
			// the view.
			let normal = placement.view_direction;
			let point = plane_hit(origin, normal, placement.camera, ray)?;
			placement.turn(normal, origin, point, p)?
		}
		(Tool::Rotate, Part::RingX | Part::RingY | Part::RingZ) => {
			let (_, _, angle) = placement.ring(p, axis);
			let [first, second] = ring_basis(axis).map(|basis| placement.axes[basis]);
			let point = origin + (first * angle.cos() + second * angle.sin()) * (placement.size * RING_RADIUS);
			placement.turn(placement.axes[axis], origin, point, p)?
		}
		(Tool::Scale, Part::AxisX | Part::AxisY | Part::AxisZ) => {
			let projected = placement.tips[axis] - placement.center;
			let direction = projected / maths_rs::length(projected).max(0.0001);
			let mut mask = Vec3f::new(0.0, 0.0, 0.0);
			mask[axis] = 1.0;
			Motion::Stretch {
				mask,
				pixel: p,
				direction,
				base: maths_rs::dot(from_center, direction).max(MIN_STRETCH_PIXELS),
			}
		}
		// A plane handle scales its two axes by how far the pointer moves away from the center.
		(Tool::Scale, Part::PlaneX | Part::PlaneY | Part::PlaneZ) => {
			let mut mask = Vec3f::new(1.0, 1.0, 1.0);
			mask[axis] = 0.0;
			let distance = maths_rs::length(from_center);
			Motion::Stretch {
				mask,
				pixel: p,
				direction: from_center / distance.max(0.0001),
				base: distance.max(MIN_STRETCH_PIXELS),
			}
		}
		// The dot sits under the pointer, so it scales evenly as the pointer moves right or up: an arrow's length on
		// screen doubles the size.
		(Tool::Scale, Part::Center) => Motion::Stretch {
			mask: Vec3f::new(1.0, 1.0, 1.0),
			pixel: p,
			direction: Vec2f::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
			base: ARROW_PIXELS * FRAC_1_SQRT_2,
		},
		_ => return None,
	};
	Some(Drag {
		part,
		start: target.clone(),
		motion,
	})
}

/// Converts a pointer position in normalized window coordinates, -1 to 1 with y up, to a pixel of `extent`.
fn pixel(pointer: [f32; 2], extent: Extent) -> [f32; 2] {
	[
		(pointer[0] + 1.0) * 0.5 * extent.width() as f32,
		(1.0 - pointer[1]) * 0.5 * extent.height() as f32,
	]
}

/// The gizmo's constants as the shader reads them; see `GizmoParameters` in `gizmo.besl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
	center: [f32; 4],
	x_tip: [f32; 4],
	y_tip: [f32; 4],
	z_tip: [f32; 4],
	planes: [f32; 4],
	quadrant: [f32; 4],
	rect: [f32; 4],
	state: [f32; 4],
	xy_axes: [f32; 4],
	z_axis: [f32; 4],
	facing: [f32; 4],
}

/// The `GizmoStep` struct is the gizmo's render pass, a pixel step that draws the gizmo over the frame of the view
/// the editor edits through.
///
/// Register it with [`register`] before the game opens its first window.
pub(crate) struct GizmoStep {
	resources: [Resource; 1],
	parameters: ghi::BufferHandle<Parameters>,
	state: Rc<RefCell<State>>,
}

/// Adds the gizmo's render pass to `renderer`, drawing what `state` says.
///
/// Register it after the game's passes, so the gizmo draws over the finished frame, and before the editor's UI, so
/// the panels draw over the gizmo.
pub(crate) fn register(renderer: &mut crate::rendering::Renderer, state: &Rc<RefCell<State>>) {
	let state = Rc::clone(state);
	renderer.add_pixel_step_for_all_sinks(PIPELINE, move |builder: &mut RenderPassBuilder| {
		crate::utils::Box::new(GizmoStep::new(builder, Rc::clone(&state)))
	});
}

impl GizmoStep {
	fn new(builder: &mut RenderPassBuilder, state: Rc<RefCell<State>>) -> Self {
		let context = builder.context();
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Editor Gizmo Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		Self {
			resources: [Resource::buffer("gizmo_parameters", parameters)],
			parameters,
			state,
		}
	}
}

impl PixelStep for GizmoStep {
	fn name(&self) -> &'static str {
		"editor-gizmo"
	}

	fn output(&self) -> &'static str {
		"Editor Gizmo"
	}

	fn resources(&self) -> &[Resource] {
		&self.resources
	}

	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> bool {
		let mut state = self.state.borrow_mut();
		if state.camera.is_some() && sink.camera() != state.camera {
			return false;
		}
		state.view = Some((sink.view(), sink.extent()));
		let Some(placement) = state
			.target
			.as_ref()
			.and_then(|target| Placement::new(sink.view(), sink.extent(), target, state.tool))
		else {
			return false;
		};
		let Placement {
			tool,
			center,
			screen_axes: [x_axis, y_axis, z_axis],
			mut tips,
			visibility,
			facing: [fx, fy, fz],
			quadrant: [qx, qy, qz],
			mut rect,
			..
		} = placement;
		// A scale drag stretches the handles, and the plane handles between them, with the scale it applies, so the
		// drawing area follows them.
		if let Some(stretch) = state.stretch {
			for axis in 0..3 {
				tips[axis] = center + (tips[axis] - center) * stretch[axis];
			}
			rect = handle_bounds(center, tips, [qx, qy, qz]);
		}
		let tip = |axis: usize| [tips[axis].x, tips[axis].y, visibility[axis], 0.0];
		*frame.map(self.parameters) = Parameters {
			center: [center.x, center.y, 0.0, 0.0],
			x_tip: tip(0),
			y_tip: tip(1),
			z_tip: tip(2),
			planes: [visibility[3], visibility[4], visibility[5], 0.0],
			quadrant: [qx, qy, qz, 0.0],
			rect,
			state: [
				Part::shader_index(state.hovered),
				Part::shader_index(state.active),
				tool as u8 as f32,
				0.0,
			],
			xy_axes: [x_axis.x, x_axis.y, y_axis.x, y_axis.y],
			z_axis: [z_axis.x, z_axis.y, 0.0, 0.0],
			facing: [fx, fy, fz, 0.0],
		};
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::Degrees;

	/// A 800 by 600 view from 5 m behind the origin, looking along +Z.
	fn view() -> (View, Extent) {
		view_from(Point::new(0.0, 0.0, -5.0))
	}

	/// A 800 by 600 view from `camera`, looking at the origin.
	fn view_from(camera: Point) -> (View, Extent) {
		let direction = (Point::origin() - camera).normalized().unwrap();
		let view = View::new_perspective(Degrees::new(60.0), 800.0 / 600.0, 0.1, 100.0, camera, direction);
		(view, Extent::rectangle(800, 600))
	}

	/// Returns the pixel `point` shows in [`view`].
	fn pixel_of(point: Point) -> [f32; 2] {
		let (view, extent) = view();
		pixel(view.project(point).unwrap(), extent)
	}

	fn at(target: Point) -> Transform {
		Transform::from_position(target)
	}

	fn placement_for(target: Point, tool: Tool) -> Placement {
		let (view, extent) = view();
		Placement::new(view, extent, &at(target), tool).unwrap()
	}

	fn placement(target: Point) -> Placement {
		placement_for(target, Tool::Move)
	}

	#[test]
	fn plane_handles_sit_on_the_side_facing_the_camera() {
		// From the left, above, and behind, the handles sit to the left of, above, and behind the center.
		let (view, extent) = view_from(Point::new(-4.0, 3.0, -5.0));
		let placement = Placement::new(view, extent, &at(Point::origin()), Tool::Move).unwrap();
		let handle = PLANE_CENTER * placement.size;
		let at = |x: f32, y: f32| pixel(view.project(Point::new(x, y, 0.0)).unwrap(), extent);

		assert_eq!(placement.pick(at(-handle, handle)), Some(Part::PlaneZ));
		assert_eq!(placement.pick(at(handle, handle)), None);
	}

	#[test]
	fn the_drawn_area_holds_every_part_from_any_side() {
		// From each corner around the selection, the plane handles sit on the camera's side, opposite some arrows.
		for x in [-4.0, 4.0] {
			for y in [-3.0, 3.0] {
				for z in [-5.0, 5.0] {
					let (view, extent) = view_from(Point::new(x, y, z));
					for tool in [Tool::Move, Tool::Scale, Tool::Rotate] {
						let placement = Placement::new(view, extent, &at(Point::origin()), tool).unwrap();
						// A part is grabbed up to `GRAB_PIXELS` outside its shape, so its grab area may reach that far
						// past the drawn area.
						let [left, top, right, bottom] = placement.rect;
						let [left, top, right, bottom] = [
							left - GRAB_PIXELS,
							top - GRAB_PIXELS,
							right + GRAB_PIXELS,
							bottom + GRAB_PIXELS,
						];
						// Every fourth pixel finds every part, which are each wider than that.
						for px in (0..800).step_by(4) {
							for py in (0..600).step_by(4) {
								let pixel = [px as f32, py as f32];
								if let Some(part) = placement.pick(pixel) {
									let inside = (left..=right).contains(&pixel[0]) && (top..=bottom).contains(&pixel[1]);
									assert!(
										inside,
										"{part:?} at {pixel:?} lies outside {:?} from {:?}",
										placement.rect,
										(x, y, z)
									);
								}
							}
						}
					}
				}
			}
		}
	}

	#[test]
	fn an_arrow_pointing_nearly_at_the_camera_can_be_grabbed_and_dragged() {
		// From above and behind, the camera looks about 14 degrees off the Z axis, so the Z arrow's true projection
		// is only about 26 pixels long.
		let (view, extent) = view_from(Point::new(0.0, 3.0, -12.0));
		let placement = Placement::new(view, extent, &at(Point::origin()), Tool::Move).unwrap();
		assert!(maths_rs::length(placement.screen_axes[2]) < 30.0);
		let projected = placement.tips[2] - placement.center;
		let on_arrow = placement.center + projected * 0.75;
		let pixel = [on_arrow.x, on_arrow.y];
		assert_eq!(placement.pick(pixel), Some(Part::AxisZ));

		// Dragging toward the tip moves the selection toward +Z, and only along Z.
		let drag = grab(&placement, &at(Point::origin()), pixel).unwrap();
		let farther = on_arrow + projected / maths_rs::length(projected) * 13.0;
		let moved = drag.transform(&placement, [farther.x, farther.y]).unwrap().get_position();
		assert!(moved.z() > 0.0, "{moved:?}");
		assert_eq!((moved.x(), moved.y()), (0.0, 0.0));
	}

	#[test]
	fn an_arrow_pointing_at_the_camera_cannot_be_grabbed() {
		// The Z arrow points straight away from the camera, so it has no direction on screen and fades out.
		let placement = placement(Point::origin());
		let behind_center = pixel_of(Point::new(0.0, 0.0, 0.9 * placement.size));
		assert_ne!(placement.pick(behind_center), Some(Part::AxisZ));
	}

	#[test]
	fn the_pointer_grabs_the_part_it_is_over() {
		let placement = placement(Point::origin());
		let size = placement.size;
		let along = |x: f32, y: f32, z: f32| pixel_of(Point::new(x * size, y * size, z * size));

		assert_eq!(placement.pick(along(0.0, 0.0, 0.0)), Some(Part::Center));
		assert_eq!(placement.pick(along(0.5, 0.0, 0.0)), Some(Part::AxisX));
		assert_eq!(placement.pick(along(0.0, 0.9, 0.0)), Some(Part::AxisY));
		assert_eq!(placement.pick(along(PLANE_CENTER, PLANE_CENTER, 0.0)), Some(Part::PlaneZ));
		assert_eq!(placement.pick(along(0.6, 0.6, 0.0)), None);
	}

	#[test]
	fn arrows_keep_their_length_on_screen_at_any_distance_and_place() {
		// Near, far, and off to the side and up, an arrow across the view is as long on screen.
		for target in [
			Point::new(0.0, 0.0, 0.0),
			Point::new(0.0, 0.0, 20.0),
			Point::new(3.0, 2.0, 0.0),
		] {
			let placement = placement(target);
			let length = maths_rs::length(placement.tips[0] - placement.center);
			assert!((length - ARROW_PIXELS).abs() < 0.5, "{target:?}: {length}");
		}
	}

	#[test]
	fn dragging_an_arrow_moves_only_along_its_axis() {
		let target = Point::new(1.0, 0.5, 2.0);
		let placement = placement(target);
		let size = placement.size;
		let drag = grab(&placement, &at(target), pixel_of(Point::new(1.0 + 0.5 * size, 0.5, 2.0))).unwrap();
		assert_eq!(drag.part, Part::AxisX);

		// Along the axis, the selection follows the pointer exactly.
		let moved = drag
			.transform(&placement, pixel_of(Point::new(1.5 + 0.5 * size, 0.5, 2.0)))
			.unwrap();
		let moved = moved.get_position();
		assert!((moved.x() - 1.5).abs() < 1e-3, "{moved:?}");
		// Off the axis, it still moves only along it.
		let moved = drag
			.transform(&placement, pixel_of(Point::new(1.5 + 0.5 * size, 0.9, 2.0)))
			.unwrap();
		let moved = moved.get_position();
		assert!(moved.x() > 1.0, "{moved:?}");
		assert_eq!((moved.y(), moved.z()), (0.5, 2.0));
	}

	#[test]
	fn dragging_a_plane_handle_moves_within_its_plane() {
		let target = Point::new(0.0, 0.0, 0.0);
		let placement = placement(target);
		let handle = PLANE_CENTER * placement.size;
		let drag = grab(&placement, &at(target), pixel_of(Point::new(handle, handle, 0.0))).unwrap();
		assert_eq!(drag.part, Part::PlaneZ);

		let moved = drag
			.transform(&placement, pixel_of(Point::new(handle + 0.25, handle - 0.5, 0.0)))
			.unwrap();
		let moved = moved.get_position();
		assert!((moved.x() - 0.25).abs() < 1e-3 && (moved.y() + 0.5).abs() < 1e-3, "{moved:?}");
		assert!(moved.z().abs() < 1e-4, "{moved:?}");
	}

	#[test]
	fn dragging_a_ring_turns_around_its_axis_only() {
		// Looking along +Z, the Z ring faces the camera as a circle, and the X and Y rings are seen edge-on, as lines
		// through the center; between them, the Z ring is alone.
		let placement = placement_for(Point::origin(), Tool::Rotate);
		let radius = placement.size * RING_RADIUS * std::f32::consts::FRAC_1_SQRT_2;
		let top = pixel_of(Point::new(radius, radius, 0.0));
		assert_eq!(placement.pick(top), Some(Part::RingZ));

		// Dragging the ring along the direction it turns, a quarter of its circumference, turns a quarter turn around
		// Z, leaving the position and scale.
		let drag = grab(&placement, &at(Point::origin()), top).unwrap();
		let Motion::Turn { tangent, .. } = drag.motion else {
			panic!("{:?}", drag.motion)
		};
		let quarter = maths_rs::length(placement.screen_axes[0]) * RING_RADIUS * std::f32::consts::FRAC_PI_2;
		let moved = Vec2f::new(top[0], top[1]) + tangent * quarter;
		let turned = drag.transform(&placement, [moved.x, moved.y]).unwrap();
		let x = turned.get_orientation().axes::<math::WorldSpace>()[0].into_maths();
		assert!((x.y - 1.0).abs() < 0.02, "X turned to {x:?}");
		assert_eq!(turned.get_position(), Point::origin());
		assert_eq!(turned.scale(), Scale::identity());
	}

	#[test]
	fn the_view_ring_lies_outside_the_axis_rings() {
		let placement = placement_for(Point::origin(), Tool::Rotate);
		let outside = [placement.center.x + VIEW_RING_PIXELS, placement.center.y];
		assert_eq!(placement.pick(outside), Some(Part::RingView));
		// Inside the Z ring and off the edge-on X and Y rings, nothing is grabbed.
		assert_eq!(placement.pick([placement.center.x + 30.0, placement.center.y + 30.0]), None);
	}

	#[test]
	fn dragging_a_scale_handle_scales_its_own_axis_by_how_far_its_tip_follows() {
		// Turned a quarter around Z, the selection's X axis points up, so its X handle does too.
		let turned =
			Orientation::try_from_maths(Quatf::from_axis_angle(Vec3f::new(0.0, 0.0, 1.0), std::f32::consts::FRAC_PI_2))
				.unwrap();
		let target = at(Point::origin()).rotation(turned);
		let (view, extent) = view();
		let placement = Placement::new(view, extent, &target, Tool::Scale).unwrap();
		let tip = placement.tips[0];
		assert!(tip.y < placement.center.y - ARROW_PIXELS * 0.9, "{tip:?}");
		let on_cap = tip - (tip - placement.center) * (CAP_HALF / ARROW_PIXELS);
		assert_eq!(placement.pick([on_cap.x, on_cap.y]), Some(Part::AxisX));

		// Dragging the cap out to twice its distance from the center doubles X and leaves the others.
		let drag = grab(&placement, &target, [on_cap.x, on_cap.y]).unwrap();
		let doubled = placement.center + (on_cap - placement.center) * 2.0;
		let scaled = drag.transform(&placement, [doubled.x, doubled.y]).unwrap().scale().to_array();
		assert!(
			(scaled[0] - 2.0).abs() < 1e-3 && scaled[1] == 1.0 && scaled[2] == 1.0,
			"{scaled:?}"
		);
		assert_eq!(
			drag.stretch([doubled.x, doubled.y]).map(|s| s.map(|s| (s * 100.0).round())),
			Some([200.0, 100.0, 100.0])
		);
	}

	#[test]
	fn the_center_scales_evenly_and_never_through_zero() {
		let placement = placement_for(Point::origin(), Tool::Scale);
		let center = [placement.center.x, placement.center.y];
		let drag = grab(&placement, &at(Point::origin()), center).unwrap();
		assert_eq!(drag.part, Part::Center);

		let right = drag.transform(&placement, [center[0] + ARROW_PIXELS, center[1]]).unwrap();
		assert_eq!(right.scale().to_array(), [2.0, 2.0, 2.0]);
		let far_left = drag
			.transform(&placement, [center[0] - ARROW_PIXELS * 3.0, center[1]])
			.unwrap();
		assert!(
			far_left.scale().to_array().iter().all(|s| (s - MIN_STRETCH).abs() < 1e-6),
			"{far_left:?}"
		);
	}
}
