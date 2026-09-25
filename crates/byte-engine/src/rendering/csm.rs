//! Cascaded shadow-map calculation and rendering support.

use math::{Matrix, Point, UnitVector, inverse};
use maths_rs::{Vec3f, Vec4f};
use smallvec::SmallVec;

use super::view::View;

/// The `CascadeSplits` struct sets how far a directional light's cascaded shadows reach from the camera and how the
/// cascades divide that range.
///
/// Near cascades cover a short range at fine resolution and far ones a long range at coarse resolution. A logarithmic
/// division keeps each cascade's resolution proportional to its distance but crowds the near cascades close to the
/// camera; an even division spreads them out. Pass it to [`make_csm_views`] and [`make_cascade_split_ranges`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeSplits {
	distance: f32,
	logarithmic_share: f32,
}

impl CascadeSplits {
	/// Shadows reach 100 meters, or the camera's far plane when it is closer.
	pub const DEFAULT_DISTANCE: f32 = 100.0;
	/// The share of the logarithmic division, against an even one. It keeps a camera about five meters behind a
	/// character in the second cascade, whose texels are about 1.4 centimeters with a 75-degree field of view.
	pub const DEFAULT_LOGARITHMIC_SHARE: f32 = 0.8;

	/// Returns splits that reach `distance` meters from the camera, divided `logarithmic_share` logarithmically and the
	/// rest evenly.
	///
	/// # Errors
	///
	/// Returns an error when `distance` is not a positive finite number of meters or `logarithmic_share` lies outside
	/// `0.0..=1.0`.
	pub fn new(distance: f32, logarithmic_share: f32) -> Result<Self, String> {
		if !(distance.is_finite() && distance > 0.0) {
			return Err(format!(
				"Directional shadow distance was not set. The most likely cause is that {distance} is not a positive number of meters."
			));
		}
		if !(0.0..=1.0).contains(&logarithmic_share) {
			return Err(format!(
				"Cascade split blend was not set. The most likely cause is that {logarithmic_share} lies outside 0.0 to 1.0."
			));
		}
		Ok(Self {
			distance,
			logarithmic_share,
		})
	}

	/// Returns how far shadows reach from the camera, in meters, before the camera's far plane limits it.
	pub fn distance(&self) -> f32 {
		self.distance
	}

	/// Returns the share of the logarithmic division, against an even one.
	pub fn logarithmic_share(&self) -> f32 {
		self.logarithmic_share
	}
}

impl Default for CascadeSplits {
	fn default() -> Self {
		Self {
			distance: Self::DEFAULT_DISTANCE,
			logarithmic_share: Self::DEFAULT_LOGARITHMIC_SHARE,
		}
	}
}

/// Returns the camera-space near and far distance for each shadow cascade.
///
/// The cascades cover the camera's range up to `splits`' distance. Past the last one, surfaces receive no sun shadow.
pub(crate) fn make_cascade_split_ranges(
	camera_view: View,
	num_cascades: usize,
	splits: CascadeSplits,
) -> impl ExactSizeIterator<Item = (f32, f32)> {
	let near = camera_view.near();
	let far = camera_view.far().min(splits.distance);
	debug_assert!(
		num_cascades > 0,
		"Cascade count is zero. The most likely cause is creating a shadow pipeline without any cascade layers."
	);
	debug_assert!(
		near.is_finite() && far.is_finite() && near > 0.0 && far > near,
		"Camera depth range is invalid. The most likely cause is a nonpositive near plane, or a far plane or shadow distance that does not follow it."
	);
	let range = far - near;
	let ratio = far / near;
	let mut cascade_near = near;

	(0..num_cascades).map(move |index| {
		let p = (index + 1) as f32 / num_cascades as f32;
		let log = near * ratio.powf(p);
		let uniform = near + range * p;
		let cascade_far = splits.logarithmic_share * (log - uniform) + uniform;
		let cascade_range = (cascade_near, cascade_far);
		cascade_near = cascade_far;
		cascade_range
	})
}

/// How far toward the light, in meters, each cascade's view reaches past its slice of the camera frustum to take in
/// shadow casters.
///
/// It sets most of each cascade's depth range, and so the size of a stored depth step. It is independent of the camera's
/// far plane, so a longer view distance does not coarsen depth precision.
pub(crate) const CASTER_REACH: f32 = 100.0;

/// Texels each cascade keeps between its slice of the camera frustum and the map's edge. A receiver's occluder search
/// and penumbra filter read up to sixteen texels around it, and snapping to the texel grid moves the map by up to one.
pub(crate) const EDGE_TEXELS: f32 = 17.0;

/// Steps per doubling to which a cascade's size is rounded up. Turning the camera changes the size its slice needs, and
/// every size change moves shadow edges across texels; rounding keeps the size, and the edges, still until the slice
/// outgrows the step, at the cost of up to 9% of the texels.
pub(crate) const SIZE_STEPS_PER_OCTAVE: f32 = 8.0;

/// The `CascadeFitting` enum chooses what each directional shadow cascade covers.
///
/// Pass it to [`crate::rendering::pipelines::visibility::VisibilityPipelineSettings::with_cascade_fitting`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CascadeFitting {
	/// Each cascade shrinks to the surfaces the camera sees in its slice, so its texels are as small as the view
	/// allows: the sky, and space in front of the nearest surface or behind a wall, take no texels. Two small GPU passes
	/// read the camera's depth to find those surfaces, so the shadow maps are drawn after the camera's depth.
	#[default]
	Receivers,
	/// Each cascade covers its whole slice of the camera frustum. It needs no GPU work before the shadow maps.
	Frustum,
}

impl std::str::FromStr for CascadeFitting {
	type Err = String;

	fn from_str(name: &str) -> Result<Self, Self::Err> {
		match name {
			"receivers" => Ok(Self::Receivers),
			"frustum" => Ok(Self::Frustum),
			_ => Err(format!(
				"Cascade fitting was not set. The most likely cause is that `{name}` is neither `receivers` nor `frustum`."
			)),
		}
	}
}

/// The `CascadeFrame` struct is one cascade view from [`make_cascade_frames`] together with where it lies in the light's
/// view, in meters, so the GPU can shrink it without rebuilding its orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CascadeFrame {
	pub(crate) view: View,
	/// The light-space x and y of the map's center, measured from the world origin.
	pub(crate) center: [f32; 2],
	/// Half the width of the square the map covers.
	pub(crate) half_extent: f32,
	/// The distance along the light from the view's light-facing side to its far side.
	pub(crate) depth: f32,
	/// The camera-space distance at which the cascade's slice of the camera frustum ends.
	pub(crate) slice_far: f32,
}

/// Returns the world-space views for cascaded shadow mapping.
///
/// Each view is the smallest square, in the light's view, that holds its cascade's slice of the camera frustum, so
/// every texel covers as little of the scene as the slice allows. Its size changes only in steps as the camera turns,
/// and it lies on a texel grid fixed in the world, so moving the camera does not move shadow edges across texels.
pub fn make_csm_views(
	camera_view: View,
	light_direction: UnitVector,
	num_cascades: usize,
	shadow_map_resolution: u32,
	splits: CascadeSplits,
) -> impl ExactSizeIterator<Item = View> {
	make_cascade_frames(camera_view, light_direction, num_cascades, shadow_map_resolution, splits).map(|frame| frame.view)
}

/// Returns the cascade views of [`make_csm_views`] with where each lies in the light's view.
pub(crate) fn make_cascade_frames(
	camera_view: View,
	light_direction: UnitVector,
	num_cascades: usize,
	shadow_map_resolution: u32,
	splits: CascadeSplits,
) -> impl ExactSizeIterator<Item = CascadeFrame> {
	assert!(
		shadow_map_resolution as f32 > 2.0 * EDGE_TEXELS,
		"Shadow map resolution is too small. The most likely cause is a resolution of {shadow_map_resolution}, which leaves no texels inside the cascade's edge margin."
	);
	// The light's orientation at the world origin. Light-space coordinates relative to the origin are fixed in the
	// world, so snapping them to texels keeps the texel grid still as the camera moves.
	let light_rotation = View::new_orthographic(-1.0, 1.0, -1.0, 1.0, 0.0, 1.0, Point::origin(), light_direction).view();

	make_cascade_split_ranges(camera_view, num_cascades, splits).map(move |(cascade_near, cascade_far)| {
		let corners = camera_view.from_from_z_planes(cascade_near, cascade_far).get_frustum_corners();
		fit_cascade_view(&corners, cascade_far, light_rotation, light_direction, shadow_map_resolution)
	})
}

/// Fits an orthographic light view around one cascade's slice of the camera frustum. The slice's bounds in the light's
/// view give a square that is padded by the edge margin, rounded up to a size step, and centered on a texel corner.
fn fit_cascade_view(
	corners: &[Point; 8],
	slice_far: f32,
	light_rotation: Matrix,
	light_direction: UnitVector,
	shadow_map_resolution: u32,
) -> CascadeFrame {
	let (minimum, maximum) = corners.iter().fold(
		(Vec3f::new(f32::MAX, f32::MAX, f32::MAX), Vec3f::new(f32::MIN, f32::MIN, f32::MIN)),
		|(minimum, maximum), corner| {
			let light_corner = light_rotation * Vec4f::from((corner.into_maths(), 1.0));
			let light_corner = Vec3f::new(light_corner.x, light_corner.y, light_corner.z);
			(maths_rs::min(minimum, light_corner), maths_rs::max(maximum, light_corner))
		},
	);

	let resolution = shadow_map_resolution as f32;
	let fitted_half_extent = (maximum.x - minimum.x).max(maximum.y - minimum.y) / 2.0;
	// A margin of m texels out of r leaves the slice (r - 2m) / r of the map.
	let padded_half_extent = fitted_half_extent * resolution / (resolution - 2.0 * EDGE_TEXELS);
	let half_extent = ((padded_half_extent.log2() * SIZE_STEPS_PER_OCTAVE).ceil() / SIZE_STEPS_PER_OCTAVE).exp2();
	let texel_size = 2.0 * half_extent / resolution;
	let snap = |coordinate: f32| (coordinate / texel_size).round() * texel_size;
	let center_x = snap((minimum.x + maximum.x) / 2.0);
	let center_y = snap((minimum.y + maximum.y) / 2.0);

	// The view starts CASTER_REACH meters toward the light from the slice and ends at its far side.
	let front = minimum.z - CASTER_REACH;
	let depth = maximum.z - front;
	let light_position = inverse(light_rotation) * Vec4f::new(center_x, center_y, front, 1.0);
	CascadeFrame {
		view: View::new_orthographic(
			-half_extent,
			half_extent,
			-half_extent,
			half_extent,
			0.0,
			depth,
			Point::from_maths(Vec3f::new(light_position.x, light_position.y, light_position.z)),
			light_direction,
		),
		center: [center_x, center_y],
		half_extent,
		depth,
		slice_far,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cascade_split_ranges_partition_the_camera_frustum() {
		let camera_view = View::new_perspective(
			math::Degrees::new(90.0),
			1.0,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let ranges = make_cascade_split_ranges(camera_view, 4, CascadeSplits::default()).collect::<SmallVec<[(f32, f32); 4]>>();

		assert_eq!(ranges.len(), 4);
		assert!((ranges[0].0 - camera_view.near()).abs() < 0.0001);
		assert!((ranges[3].1 - camera_view.far()).abs() < 0.0001);
		assert!(ranges.windows(2).all(|ranges| (ranges[0].1 - ranges[1].0).abs() < 0.0001));
	}

	#[test]
	fn cascade_splits_end_at_the_shadow_distance_and_spread_with_a_smaller_logarithmic_share() {
		let camera_view = View::new_perspective(
			math::Degrees::new(75.0),
			1.0,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let far_of = |splits| make_cascade_split_ranges(camera_view, 4, splits).map(|(_, far)| far).collect::<SmallVec<[f32; 4]>>();

		let short = far_of(CascadeSplits::new(50.0, 0.8).expect("valid splits"));
		assert!((short[3] - 50.0).abs() < 0.0001, "{short:?}");
		let beyond_far = far_of(CascadeSplits::new(500.0, 0.8).expect("valid splits"));
		assert!((beyond_far[3] - camera_view.far()).abs() < 0.0001, "{beyond_far:?}");

		let logarithmic = far_of(CascadeSplits::new(100.0, 1.0).expect("valid splits"));
		let blended = far_of(CascadeSplits::new(100.0, 0.8).expect("valid splits"));
		assert!(
			blended[..3].iter().zip(&logarithmic[..3]).all(|(blended, logarithmic)| blended > logarithmic),
			"{blended:?} {logarithmic:?}"
		);
		assert!(CascadeSplits::new(0.0, 0.8).is_err());
		assert!(CascadeSplits::new(100.0, 1.5).is_err());
	}

	/// Returns a 16:9 camera away from the origin, turned `yaw_degrees` about the vertical axis.
	fn turned_camera(position: Point, yaw_degrees: f32) -> View {
		let yaw = yaw_degrees.to_radians();
		View::new_perspective(
			math::Degrees::new(75.0),
			16.0 / 9.0,
			0.1,
			100.0,
			position,
			math::Vector::new(yaw.sin(), -0.2, yaw.cos()).normalized().expect("nonzero camera direction"),
		)
	}

	fn diagonal_light() -> UnitVector {
		math::Vector::new(0.5, -1.0, 0.3).normalized().expect("nonzero light direction")
	}

	/// Returns a world point's shadow-map texel coordinates and light-space depth in one cascade view.
	fn shadow_texel(view: View, point: Point, resolution: u32) -> (f32, f32, f32) {
		let point = Vec4f::from((point.into_maths(), 1.0));
		let clip = view.view_projection() * point;
		let half = resolution as f32 / 2.0;
		((clip.x / clip.w + 1.0) * half, (clip.y / clip.w + 1.0) * half, (view.view() * point).z)
	}

	#[test]
	fn cascade_views_hold_their_slice_inside_the_edge_margin() {
		let camera_view = turned_camera(Point::new(0.37, 1.7, 2.83), 20.0);
		let resolution = 1024;
		let ranges = make_cascade_split_ranges(camera_view, 4, CascadeSplits::default());
		let views = make_csm_views(camera_view, diagonal_light(), 4, resolution, CascadeSplits::default());

		for ((near, far), view) in ranges.zip(views) {
			for corner in camera_view.from_from_z_planes(near, far).get_frustum_corners() {
				let (x, y, depth) = shadow_texel(view, corner, resolution);
				let inner = (EDGE_TEXELS - 1.0)..=(resolution as f32 - EDGE_TEXELS + 1.0);
				assert!(inner.contains(&x) && inner.contains(&y), "corner at texel ({x}, {y})");
				assert!((CASTER_REACH - 0.001..=view.far() + 0.001).contains(&depth), "corner at depth {depth}, far {}", view.far());
			}
		}
	}

	#[test]
	fn cascade_texel_grid_stays_fixed_in_the_world_as_the_camera_moves() {
		let resolution = 1024;
		for position in [Point::new(0.37, 1.7, 2.83), Point::new(0.52, 1.7, 3.61)] {
			for view in make_csm_views(turned_camera(position, 20.0), diagonal_light(), 4, resolution, CascadeSplits::default()) {
				let (x, y, _) = shadow_texel(view, Point::origin(), resolution);
				assert!((x - x.round()).abs() < 0.01 && (y - y.round()).abs() < 0.01, "origin at texel ({x}, {y})");
			}
		}
	}

	#[test]
	fn cascade_size_changes_in_eighth_octave_steps_as_the_camera_turns() {
		let sizes = (0..90)
			.map(|step| {
				let camera_view = turned_camera(Point::new(0.37, 1.7, 2.83), step as f32 * 0.5);
				let view = make_csm_views(camera_view, diagonal_light(), 1, 1024, CascadeSplits::default())
					.next()
					.expect("a shadow cascade view");
				// The orthographic projection scales x by one over the half extent.
				1.0 / view.projection()[0]
			})
			.collect::<Vec<_>>();

		for size in &sizes {
			let step = size.log2() * SIZE_STEPS_PER_OCTAVE;
			assert!((step - step.round()).abs() < 0.001, "half extent {size}");
		}
		assert!(sizes.windows(2).filter(|pair| pair[0] != pair[1]).count() < 10, "{sizes:?}");
	}

	#[test]
	fn shadow_view_matrices_are_orthonormal_for_cardinal_and_diagonal_directions() {
		use maths_rs::{Vec3f, dot, length};

		let camera_view = View::new_perspective(
			math::Degrees::new(90.0),
			1.0,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let directions = [
			UnitVector::y_axis(),
			-UnitVector::y_axis(),
			UnitVector::x_axis(),
			UnitVector::z_axis(),
			diagonal_light(),
		];

		for direction in directions {
			for view in make_csm_views(camera_view, direction, 4, 2048, CascadeSplits::default()) {
				let matrix = view.view();
				// `View` is a raw-matrix boundary, so inspect its basis as `maths_rs` vectors.
				let x = Vec3f::new(matrix[0], matrix[1], matrix[2]);
				let y = Vec3f::new(matrix[4], matrix[5], matrix[6]);
				let z = Vec3f::new(matrix[8], matrix[9], matrix[10]);

				assert!((length(x) - 1.0).abs() < 1e-5);
				assert!((length(y) - 1.0).abs() < 1e-5);
				assert!((length(z) - 1.0).abs() < 1e-5);
				assert!(dot(x, y).abs() < 1e-5);
				assert!(dot(x, z).abs() < 1e-5);
				assert!(dot(y, z).abs() < 1e-5);
			}
		}
	}

	#[test]
	fn zenith_light_keeps_vertical_casters_on_one_shadow_texel_column() {
		let camera_view = View::new_perspective(
			math::Degrees::new(75.0),
			1.0,
			0.1,
			100.0,
			Point::new(0.0, 2.0, 0.0),
			UnitVector::z_axis(),
		);
		let floor = Vec4f::new(1.25, 0.0, 5.0, 1.0);
		let caster = Vec4f::new(1.25, 3.0, 5.0, 1.0);

		for view in make_csm_views(camera_view, -UnitVector::y_axis(), 4, 2048, CascadeSplits::default()) {
			let floor_clip = view.view_projection() * floor;
			let caster_clip = view.view_projection() * caster;

			assert!((floor_clip.x / floor_clip.w - caster_clip.x / caster_clip.w).abs() < 1e-5);
			assert!((floor_clip.y / floor_clip.w - caster_clip.y / caster_clip.w).abs() < 1e-5);
		}
	}

	/// The GPU cascade fit moves each view within the light's view by its frame, so a frame must say exactly where its
	/// view lies: its center, half extent, and depth range in the light's view, and where its slice ends.
	#[test]
	fn cascade_frames_describe_where_their_views_lie() {
		let camera_view = turned_camera(Point::new(3.0, 2.0, -7.0), 30.0);
		let light = diagonal_light();
		let light_rotation = View::new_orthographic(-1.0, 1.0, -1.0, 1.0, 0.0, 1.0, Point::origin(), light).view();
		let light_to_world = inverse(light_rotation);
		let world = |x: f32, y: f32, z: f32| {
			let point = light_to_world * Vec4f::new(x, y, z, 1.0);
			Vec4f::new(point.x, point.y, point.z, 1.0)
		};
		let ranges = make_cascade_split_ranges(camera_view, 4, CascadeSplits::default());
		for (frame, (_, slice_far)) in make_cascade_frames(camera_view, light, 4, 2048, CascadeSplits::default()).zip(ranges) {
			let [center_x, center_y] = frame.center;
			let view_projection = frame.view.view_projection();
			let center = view_projection * world(center_x, center_y, 0.0);
			let corner = view_projection * world(center_x + frame.half_extent, center_y - frame.half_extent, 0.0);
			let further = view_projection * world(center_x, center_y, frame.depth);
			assert!(center.x.abs() < 1e-4 && center.y.abs() < 1e-4, "The frame's center maps to {center:?}.");
			assert!((corner.x - 1.0).abs() < 1e-4 && (corner.y + 1.0).abs() < 1e-4, "The frame's corner maps to {corner:?}.");
			assert!((center.z - further.z - 1.0).abs() < 1e-4, "The frame's depth range spans {} of stored depth.", center.z - further.z);
			assert_eq!(frame.slice_far, slice_far);
		}
	}

	#[test]
	fn cascade_fitting_parses_its_parameter_names() {
		assert_eq!("receivers".parse(), Ok(CascadeFitting::Receivers));
		assert_eq!("frustum".parse(), Ok(CascadeFitting::Frustum));
		assert!("sphere".parse::<CascadeFitting>().is_err());
	}
}
