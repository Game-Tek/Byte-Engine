//! The values the properties panel edits: an entity's position, rotation, and scale, and a camera's settings. This
//! module says how each one reads from an entity, shows, scrubs, reads typed text, and changes the entity.
//!
//! Drawing the fields is the shell's job, and posting a [`Change`] to the game the editor's.

use maths_rs::{Quatf, Vec3f};

use crate::editor::scene::SceneEntity;
use crate::gameplay::Transform;
use crate::math::{self, Orientation, Point, Scale};
use crate::rendering::Camera;

/// The `Change` enum is one edit of an entity, as the message that applies it in the game carries it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Change {
	/// A new transform, posted as a `TransformationUpdate`.
	Transform(Transform),
	/// New camera settings, posted as a `CameraUpdate`.
	Camera(Camera),
}

impl Change {
	/// Applies the change to the editor's copy of `entity`.
	pub(crate) fn apply(&self, entity: &mut SceneEntity) {
		match self {
			Change::Transform(transform) => entity.transform = transform.clone(),
			Change::Camera(camera) => entity.camera = Some(camera.clone()),
		}
	}
}

/// The `Target` enum is the value of an entity a field edits.
#[derive(Clone, Copy, Debug)]
enum Target {
	/// A position axis, 0 to 2, in meters.
	Position(usize),
	/// A rotation angle around an axis, 0 to 2, in degrees; see [`euler_degrees`].
	Rotation(usize),
	/// A scale axis, 0 to 2.
	Scale(usize),
	/// A field of the camera's `CameraUpdate` payload.
	Camera(&'static str),
}

/// The `Scrub` enum is how dragging across a field changes its value.
#[derive(Clone, Copy, Debug)]
enum Scrub {
	/// Adds this much for each pixel the pointer moves right.
	Linear(f32),
	/// Doubles the value for each this many pixels the pointer moves right, for values that span magnitudes, such as
	/// distances and scales.
	Exponential(f32),
}

/// The `Setting` struct is one value the properties panel shows as a field.
#[derive(Debug)]
pub(crate) struct Setting {
	target: Target,
	/// The short name the field shows.
	pub(crate) label: &'static str,
	/// What the stored value is multiplied by to show it, such as 1000 to show meters as millimeters.
	display_scale: f32,
	/// The unit after the shown value.
	unit: &'static str,
	/// The decimals the value shows with.
	decimals: usize,
	scrub: Scrub,
	/// The value a scrub starts from when the setting has none, such as a pinhole's f-number.
	unset_start: f32,
}

/// The transform's settings, a row each for position, rotation, and scale, X, Y, and Z in order.
pub(crate) const TRANSFORM: [(&str, [Setting; 3]); 3] = [
	("Position", axes(Row::Position, " m", 2, Scrub::Linear(0.01))),
	("Rotation", axes(Row::Rotation, "°", 1, Scrub::Linear(0.5))),
	("Scale", axes(Row::Scale, "", 2, Scrub::Exponential(150.0))),
];

/// The `Row` enum names the transform value a row of the transform's settings edits.
#[derive(Clone, Copy)]
enum Row {
	Position,
	Rotation,
	Scale,
}

/// Every camera setting, in the panel's order: two to a row.
pub(crate) const CAMERA: [Setting; 7] = [
	camera("fov", "FOV", 1.0, "°", 1, Scrub::Linear(0.25), 45.0),
	camera("exposure", "Exposure", 1.0, " EV", 2, Scrub::Linear(0.02), 0.0),
	camera("near", "Near", 1.0, " m", 2, Scrub::Exponential(120.0), 0.1),
	camera("far", "Far", 1.0, " m", 0, Scrub::Exponential(120.0), 100.0),
	// A pinhole has no f-number; scrubbing it starts from an aperture that still keeps most of a scene sharp.
	camera("f_number", "f/", 1.0, "", 1, Scrub::Exponential(80.0), 16.0),
	camera("focus_distance", "Focus", 1.0, " m", 2, Scrub::Exponential(120.0), 10.0),
	camera("sensor_height", "Sensor", 1000.0, " mm", 1, Scrub::Exponential(160.0), 0.024),
];

/// The X, Y, and Z settings of one transform value.
const fn axes(row: Row, unit: &'static str, decimals: usize, scrub: Scrub) -> [Setting; 3] {
	[
		axis(row, 0, "X", unit, decimals, scrub),
		axis(row, 1, "Y", unit, decimals, scrub),
		axis(row, 2, "Z", unit, decimals, scrub),
	]
}

/// The setting of one axis of a transform value. A transform value is always set, so it never scrubs from a start.
const fn axis(row: Row, axis: usize, label: &'static str, unit: &'static str, decimals: usize, scrub: Scrub) -> Setting {
	Setting {
		target: match row {
			Row::Position => Target::Position(axis),
			Row::Rotation => Target::Rotation(axis),
			Row::Scale => Target::Scale(axis),
		},
		label,
		display_scale: 1.0,
		unit,
		decimals,
		scrub,
		unset_start: 0.0,
	}
}

const fn camera(
	key: &'static str,
	label: &'static str,
	display_scale: f32,
	unit: &'static str,
	decimals: usize,
	scrub: Scrub,
	unset_start: f32,
) -> Setting {
	Setting {
		target: Target::Camera(key),
		label,
		display_scale,
		unit,
		decimals,
		scrub,
		unset_start,
	}
}

impl Setting {
	/// The world axis, 0 to 2, a transform setting is along, which its label is colored by; `None` for a camera's.
	pub(crate) fn axis(&self) -> Option<usize> {
		match self.target {
			Target::Position(axis) | Target::Rotation(axis) | Target::Scale(axis) => Some(axis),
			Target::Camera(_) => None,
		}
	}

	/// The setting's value in `entity`, or `None` when it has none: a camera setting of an entity that isn't a camera,
	/// or a pinhole's f-number.
	///
	/// Fields read it every frame, so it reads the values straight, without going through a payload.
	pub(crate) fn value(&self, entity: &SceneEntity) -> Option<f32> {
		let transform = &entity.transform;
		match self.target {
			Target::Position(axis) => Some(transform.get_position().to_array()[axis]),
			Target::Rotation(axis) => Some(euler_degrees(transform.get_orientation())[axis]),
			Target::Scale(axis) => Some(transform.scale().to_array()[axis]),
			Target::Camera(key) => {
				let camera = entity.camera.as_ref()?;
				let value = match key {
					"fov" => camera.vertical_fov().value(),
					"near" => camera.near(),
					"far" => camera.far(),
					"f_number" => camera.f_number(),
					"focus_distance" => camera.focus_distance(),
					"sensor_height" => camera.sensor_height(),
					"exposure" => camera.exposure(),
					_ => unreachable!("Camera setting '{key}' is unknown. Add it to `Setting::value`."),
				};
				value.is_finite().then_some(value)
			}
		}
	}

	/// The change that sets the setting of `entity` to `value`, or unsets it for `None`, or `None` when the value is
	/// refused: a camera value the camera's rules refuse, such as a near plane past the far one, or a scale of zero.
	pub(crate) fn with(&self, entity: &SceneEntity, value: Option<f32>) -> Option<Change> {
		let mut transform = entity.transform.clone();
		match self.target {
			Target::Position(axis) => {
				let mut position = transform.get_position().to_array();
				position[axis] = value.filter(|value| value.is_finite())?;
				transform.set_position(Point::new(position[0], position[1], position[2]));
			}
			Target::Rotation(axis) => {
				let mut angles = euler_degrees(transform.get_orientation());
				angles[axis] = value.filter(|value| value.is_finite())?;
				transform.set_orientation(orientation_from_euler_degrees(angles)?);
			}
			Target::Scale(axis) => {
				let mut scale = transform.scale().to_array();
				scale[axis] = value.filter(|value| value.is_finite() && *value != 0.0)?;
				transform.set_scale(Scale::from_array(scale));
			}
			// Camera edits go through the payload, so the engine's own rules decide which values a camera takes.
			Target::Camera(key) => {
				let mut payload = serde_json::to_value(entity.camera.as_ref()?).ok()?;
				payload[key] = value.map_or(serde_json::Value::Null, Into::into);
				return serde_json::from_value(payload).ok().map(Change::Camera);
			}
		}
		Some(Change::Transform(transform))
	}

	/// The value after dragging `pixels` right from where a scrub of `start` began.
	pub(crate) fn scrubbed(&self, start: Option<f32>, pixels: f32) -> f32 {
		let start = start.unwrap_or(self.unset_start);
		match self.scrub {
			Scrub::Linear(per_pixel) => start + pixels * per_pixel,
			Scrub::Exponential(per_doubling) => start * (pixels / per_doubling).exp2(),
		}
	}

	/// Writes the text the field shows for `value` into `text`: its number and unit, or `∞` when it's unset.
	pub(crate) fn display(&self, value: Option<f32>, text: &mut String) {
		use std::fmt::Write as _;
		text.clear();
		match value {
			Some(value) => {
				let shown = value * self.display_scale;
				// A value that rounds to zero shows as zero, never as `-0.0`.
				let shown = if shown.abs() < 0.5 * 10f32.powi(-(self.decimals as i32)) {
					0.0
				} else {
					shown
				};
				let _ = write!(text, "{:.*}{}", self.decimals, shown, self.unit);
			}
			None => text.push('∞'),
		}
	}

	/// Reads typed `text` as a value in the field's shown unit: `Some(None)` unsets the setting, for `∞`, `inf`, or
	/// `off`, and `None` means the text isn't a number.
	pub(crate) fn parse(&self, text: &str) -> Option<Option<f32>> {
		let text = text.trim().trim_end_matches(self.unit.trim()).trim();
		if ["∞", "inf", "off"].iter().any(|unset| text.eq_ignore_ascii_case(unset)) {
			return Some(None);
		}
		text.parse::<f32>().ok().map(|value| Some(value / self.display_scale))
	}
}

/// The angles in degrees that turn around X, then Y, then Z, all around the world's axes, to `orientation`.
///
/// Near a quarter turn around Y, the X and Z turns become one and the split between them is arbitrary.
pub(crate) fn euler_degrees(orientation: Orientation) -> [f32; 3] {
	// The rotation matrix's columns are where the axes turn to; R = Rz Ry Rx gives its entries as these angles.
	let [x, y, z] = orientation.axes::<math::WorldSpace>().map(|axis| axis.into_maths());
	let around_y = (-x.z).clamp(-1.0, 1.0).asin();
	let around_x = y.z.atan2(z.z);
	let around_z = x.y.atan2(x.x);
	[around_x, around_y, around_z].map(f32::to_degrees)
}

/// The orientation that turns `degrees` around X, then Y, then Z, all around the world's axes. The reverse of
/// [`euler_degrees`].
fn orientation_from_euler_degrees(degrees: [f32; 3]) -> Option<Orientation> {
	let [x, y, z] = degrees.map(f32::to_radians);
	let turn = |axis: Vec3f, angle: f32| Quatf::from_axis_angle(axis, angle);
	let rotation = turn(Vec3f::new(0.0, 0.0, 1.0), z) * turn(Vec3f::new(0.0, 1.0, 0.0), y) * turn(Vec3f::new(1.0, 0.0, 0.0), x);
	Orientation::try_from_maths(rotation).ok()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::factory::Factory;

	fn entity(transform: Transform, camera: Option<Camera>) -> SceneEntity {
		SceneEntity {
			handle: Factory::<u8>::new().create(0),
			name: String::new(),
			kind: String::new(),
			transform,
			camera,
		}
	}

	fn camera_setting(key: &str) -> &'static Setting {
		CAMERA
			.iter()
			.find(|setting| matches!(setting.target, Target::Camera(name) if name == key))
			.unwrap()
	}

	fn changed_camera(change: Option<Change>) -> Option<Camera> {
		match change? {
			Change::Camera(camera) => Some(camera),
			Change::Transform(_) => None,
		}
	}

	#[test]
	fn camera_settings_read_and_write_the_camera_through_its_payload() {
		let camera = entity(Transform::identity(), Some(Camera::new()));
		let fov = camera_setting("fov");
		assert_eq!(fov.value(&camera), Some(45.0));
		let wider = entity(Transform::identity(), changed_camera(fov.with(&camera, Some(70.0))));
		assert_eq!(fov.value(&wider), Some(70.0));
		// Only the edited setting changes.
		assert_eq!(camera_setting("far").value(&wider), Some(100.0));

		// The camera's rules refuse a near plane past the far one, and a pinhole has no f-number.
		assert!(camera_setting("near").with(&camera, Some(200.0)).is_none());
		assert_eq!(camera_setting("f_number").value(&camera), None);
		// An entity that isn't a camera has no camera settings.
		assert_eq!(fov.value(&entity(Transform::identity(), None)), None);
	}

	#[test]
	fn transform_settings_change_one_axis_and_keep_the_rest() {
		let moved = entity(Transform::from_position(Point::new(1.0, 2.0, 3.0)), None);
		let [_, y, _] = &TRANSFORM[0].1;
		let Some(Change::Transform(transform)) = y.with(&moved, Some(5.0)) else {
			panic!()
		};
		assert_eq!(transform.get_position().to_array(), [1.0, 5.0, 3.0]);

		let [_, _, z] = &TRANSFORM[2].1;
		assert!(z.with(&moved, Some(0.0)).is_none(), "a scale of zero is refused");
		let Some(Change::Transform(transform)) = z.with(&moved, Some(2.0)) else {
			panic!()
		};
		assert_eq!(transform.scale().to_array(), [1.0, 1.0, 2.0]);
	}

	#[test]
	fn rotation_angles_round_trip_through_the_orientation() {
		for degrees in [[30.0, -20.0, 75.0], [0.0, 0.0, 0.0], [-120.0, 45.0, 10.0]] {
			let orientation = orientation_from_euler_degrees(degrees).unwrap();
			let back = euler_degrees(orientation);
			for (back, degrees) in back.into_iter().zip(degrees) {
				assert!((back - degrees).abs() < 1e-3, "{degrees:?} came back as {back:?}");
			}
		}
		// Turning a quarter around Z sends X to Y.
		let quarter = orientation_from_euler_degrees([0.0, 0.0, 90.0]).unwrap();
		let x = quarter.axes::<math::WorldSpace>()[0].into_maths();
		assert!((x.y - 1.0).abs() < 1e-5, "{x:?}");
	}

	#[test]
	fn scrubbing_adds_or_doubles_and_starts_an_unset_value_from_its_start() {
		assert_eq!(camera_setting("fov").scrubbed(Some(45.0), 40.0), 55.0);
		assert_eq!(camera_setting("far").scrubbed(Some(100.0), 120.0), 200.0);
		assert_eq!(camera_setting("f_number").scrubbed(None, -80.0), 8.0);
		assert_eq!(TRANSFORM[2].1[0].scrubbed(Some(1.0), 150.0), 2.0);
	}

	#[test]
	fn fields_show_and_read_values_in_their_own_unit() {
		let sensor = camera_setting("sensor_height");
		let mut text = String::new();
		sensor.display(Some(0.024), &mut text);
		assert_eq!(text, "24.0 mm");
		assert_eq!(sensor.parse("36 mm"), Some(Some(0.036)));
		camera_setting("f_number").display(None, &mut text);
		assert_eq!(text, "∞");
		assert_eq!(camera_setting("f_number").parse("inf"), Some(None));
		assert_eq!(TRANSFORM[1].1[0].parse("45°"), Some(Some(45.0)));
		TRANSFORM[1].1[1].display(Some(-0.0001), &mut text);
		assert_eq!(text, "0.0°");
		assert_eq!(camera_setting("fov").parse("wide"), None);
	}
}
