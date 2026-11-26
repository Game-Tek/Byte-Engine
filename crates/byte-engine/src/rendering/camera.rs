/// The `Camera` struct provides scene-owned world-space view settings to render sinks and inspection tools.
///
/// Besides the view and exposure, it describes a thin lens: an f-number, a focus distance, and a sensor height, which
/// together set how far each depth is out of focus. See [`Self::set_f_number`].
///
/// Create one on the world with a transform. To change it later, create it again under its own handle, as a
/// [`CameraUpdate`]. Facet and serde read and write it as the
/// [`CameraUpdate` payload](/docs/reference/inspector#entities-and-messages).
#[derive(Clone, Debug, PartialEq, facet::Facet)]
#[facet(opaque, proxy = SerializableCamera)]
pub struct Camera {
	fov: Degrees,
	/// The distances in meters of the nearest and farthest surfaces the camera shows.
	near: f32,
	far: f32,
	f_number: f32,
	focus_distance: f32,
	sensor_height: f32,
	exposure: f32,
	/// The detail level every mesh draws at, or `None` to select levels by distance; see [`Self::with_detail_level`].
	detail_level: Option<u32>,
}

impl Camera {
	/// Creates a camera with a world-origin position, default perspective settings, neutral exposure, and a pinhole
	/// lens that keeps every depth in focus.
	pub fn new() -> Self {
		Self {
			fov: Degrees::new(45.0),
			near: 0.1,
			far: 100.0,
			f_number: f32::INFINITY,
			focus_distance: 10.0,
			sensor_height: 0.024,
			exposure: 0.0,
			detail_level: None,
		}
	}

	/// Returns the camera's vertical field of view.
	pub fn vertical_fov(&self) -> Degrees {
		self.fov
	}

	/// Returns the distance in meters of the nearest surface the camera shows. See [`Self::set_clip_planes`].
	pub fn near(&self) -> f32 {
		self.near
	}

	/// Returns the distance in meters of the farthest surface the camera shows. See [`Self::set_clip_planes`].
	pub fn far(&self) -> f32 {
		self.far
	}

	/// Sets the clip planes. See [`Self::set_clip_planes`].
	pub fn with_clip_planes(mut self, near: f32, far: f32) -> Self {
		self.set_clip_planes(near, far);
		self
	}

	/// Sets the distances in meters, along the view direction, of the nearest and farthest surfaces the camera
	/// shows. Anything closer than `near` or beyond `far` is cut away. The defaults are `0.1` and `100.0`.
	///
	/// Shadow cascades and depth of field spread over this range, so keep `far` no larger than the scene needs.
	pub fn set_clip_planes(&mut self, near: f32, far: f32) {
		debug_assert!(
			near > 0.0 && near < far && far.is_finite(),
			"Camera clip planes are invalid. The most likely cause is a near plane at or below zero, or not closer than the far plane. Pass 0 < near < far."
		);
		self.near = near;
		self.far = far;
	}

	/// Returns the lens f-number. See [`Self::set_f_number`].
	pub fn f_number(&self) -> f32 {
		self.f_number
	}

	/// Sets the lens f-number. See [`Self::set_f_number`].
	pub fn with_f_number(mut self, f_number: f32) -> Self {
		self.set_f_number(f_number);
		self
	}

	/// Sets the lens f-number: the focal length divided by the aperture's diameter.
	///
	/// Lower numbers open the aperture and blur what lies away from the [focus distance](Self::set_focus_distance)
	/// more: `1.8` gives a portrait lens's shallow focus, `8.0` keeps most of a scene sharp, and `f32::INFINITY`, the
	/// default, is a pinhole that keeps every depth in focus. Numbers below about `0.7` have no real lens, but they
	/// give a scene the shallow focus of a miniature. The depth-of-field pass, installed with
	/// [`crate::application::graphics::setup_depth_of_field_render_pass`], reads it.
	pub fn set_f_number(&mut self, f_number: f32) {
		debug_assert!(
			f_number > 0.0 && !f_number.is_nan(),
			"Camera f-number is invalid. The most likely cause is a zero, negative, or NaN f-number; use `f32::INFINITY` for a pinhole."
		);
		self.f_number = f_number;
	}

	/// Returns the distance in meters that is in focus. See [`Self::set_focus_distance`].
	pub fn focus_distance(&self) -> f32 {
		self.focus_distance
	}

	/// Sets the distance in meters that is in focus. See [`Self::set_focus_distance`].
	pub fn with_focus_distance(mut self, meters: f32) -> Self {
		self.set_focus_distance(meters);
		self
	}

	/// Sets the distance in meters, along the view direction, at which surfaces are in focus. Surfaces nearer or
	/// farther blur by how open the [f-number](Self::set_f_number) is. The default is `10.0`.
	pub fn set_focus_distance(&mut self, meters: f32) {
		debug_assert!(
			meters > 0.0 && meters.is_finite(),
			"Camera focus distance is invalid. The most likely cause is a zero, negative, or non-finite distance."
		);
		self.focus_distance = meters;
	}

	/// Returns the sensor height in meters. See [`Self::set_sensor_height`].
	pub fn sensor_height(&self) -> f32 {
		self.sensor_height
	}

	/// Sets the sensor height in meters. See [`Self::set_sensor_height`].
	pub fn with_sensor_height(mut self, meters: f32) -> Self {
		self.set_sensor_height(meters);
		self
	}

	/// Sets the height in meters of the sensor the lens projects onto. With the field of view it sets the focal
	/// length, so at the same field of view and f-number a larger sensor blurs more. The default, `0.024`, is a full
	/// frame sensor's height.
	pub fn set_sensor_height(&mut self, meters: f32) {
		debug_assert!(
			meters > 0.0 && meters.is_finite(),
			"Camera sensor height is invalid. The most likely cause is a zero, negative, or non-finite height."
		);
		self.sensor_height = meters;
	}

	/// Sets the vertical field of view used by perspective rendering.
	pub fn with_fov(mut self, fov: Degrees) -> Self {
		self.set_fov(fov);
		self
	}

	/// Sets the vertical field of view used by perspective rendering.
	pub fn set_fov(&mut self, fov: Degrees) {
		self.fov = fov;
	}

	/// Returns the camera exposure in stops. See [`Self::set_exposure`].
	pub fn exposure(&self) -> f32 {
		self.exposure
	}

	/// Returns the linear factor that the camera exposure applies to scene light, `2^exposure`.
	pub fn exposure_scale(&self) -> f32 {
		self.exposure.exp2()
	}

	/// Sets the camera exposure in stops. See [`Self::set_exposure`].
	pub fn with_exposure(mut self, stops: f32) -> Self {
		self.set_exposure(stops);
		self
	}

	/// Sets how much the camera brightens or darkens scene light before it is mapped to the display.
	///
	/// Each stop doubles or halves the light: `0.0` shows scene values unchanged, `-1.0` halves them, and `1.0`
	/// doubles them. Scenes lit with real-world values need a matching photographic exposure: a camera at EV100 `e`
	/// uses `-(e + log2(1.2))` stops, so a sunny day at EV100 15 is about `-15.26`.
	///
	/// The PBR visibility pipeline, set up by
	/// [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`], and the atmosphere sky apply it
	/// as they write scene light. Storing light already exposed keeps real-world intensities within the half-float
	/// range of the scene color target. Scenes drawn with other pipelines ignore it.
	pub fn set_exposure(&mut self, stops: f32) {
		debug_assert!(
			stops.is_finite() && stops.abs() <= 64.0,
			"Camera exposure is invalid. The most likely cause is a non-finite or out-of-range number of stops."
		);
		self.exposure = stops;
	}

	/// Returns the detail level the camera draws meshes at, or `None` while it selects them by distance. See
	/// [`Self::with_detail_level`].
	pub fn detail_level(&self) -> Option<u32> {
		self.detail_level
	}

	/// Draws every mesh at detail `level`, from 0, full detail, toward coarser ones, or by distance again for `None`,
	/// such as to inspect a mesh's levels in a preview. It applies while the camera shows the first sink of its scene,
	/// whose view selects levels, and takes precedence over the `render.lod.force` runtime setting. See
	/// [detail level rules](/docs/reference/rendering#the-visibility-pipeline).
	pub fn with_detail_level(mut self, level: Option<u32>) -> Self {
		self.detail_level = level;
		self
	}
}

impl Default for Camera {
	fn default() -> Self {
		Self::new()
	}
}

use math::Degrees;

use crate::core::factory::CreateMessage;

/// The `CameraUpdate` type replaces a camera's settings: a camera created again under the handle of the camera it
/// replaces. The inspector posts it as `CameraUpdate`.
pub type CameraUpdate = CreateMessage<Camera>;

/// The `SerializableCamera` struct is the checked reflection boundary of [`Camera`], in the `CameraUpdate` payload's
/// shape.
///
/// A missing field takes a new camera's value, so a payload names only what it changes, and a null `f_number` is a
/// pinhole, since JSON has no infinity.
#[derive(facet::Facet, serde::Deserialize, serde::Serialize)]
#[facet(deny_unknown_fields)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct SerializableCamera {
	#[facet(default = 45.0f32)]
	fov: f32,
	#[facet(default = 0.1f32)]
	near: f32,
	#[facet(default = 100.0f32)]
	far: f32,
	#[facet(default)]
	f_number: Option<f32>,
	#[facet(default = 10.0f32)]
	focus_distance: f32,
	#[facet(default = 0.024f32)]
	sensor_height: f32,
	#[facet(default)]
	exposure: f32,
	#[facet(default)]
	detail_level: Option<u32>,
}

impl Default for SerializableCamera {
	/// A new camera's payload, which every missing field takes its value from.
	fn default() -> Self {
		Self::from(&Camera::new())
	}
}

impl TryFrom<SerializableCamera> for Camera {
	type Error = String;

	/// Checks every field against the camera's rules before building it, so a payload never reaches the setters'
	/// assertions.
	fn try_from(value: SerializableCamera) -> Result<Self, Self::Error> {
		let invalid = |field: &str, rule: &str| {
			format!("Camera payload is invalid. The most likely cause is that `{field}` is outside its range. Send {rule}.")
		};
		let positive = |number: f32| number > 0.0 && number.is_finite();
		if !(value.fov > 0.0 && value.fov < 180.0) {
			return Err(invalid("fov", "a vertical field of view in degrees above 0 and below 180"));
		}
		if !(positive(value.near) && positive(value.far) && value.near < value.far) {
			return Err(invalid(
				"near` or `far",
				"a near plane above 0 and below a finite far plane, in meters",
			));
		}
		if value.f_number.is_some_and(|f_number| !positive(f_number)) {
			return Err(invalid("f_number", "an f-number above 0, or null for a pinhole"));
		}
		if !positive(value.focus_distance) {
			return Err(invalid("focus_distance", "a distance in meters above 0"));
		}
		if !positive(value.sensor_height) {
			return Err(invalid("sensor_height", "a height in meters above 0"));
		}
		if !(value.exposure.is_finite() && value.exposure.abs() <= 64.0) {
			return Err(invalid("exposure", "a number of stops within ±64"));
		}
		Ok(Camera {
			fov: Degrees::new(value.fov),
			near: value.near,
			far: value.far,
			f_number: value.f_number.unwrap_or(f32::INFINITY),
			focus_distance: value.focus_distance,
			sensor_height: value.sensor_height,
			exposure: value.exposure,
			detail_level: value.detail_level,
		})
	}
}

impl From<&Camera> for SerializableCamera {
	fn from(camera: &Camera) -> Self {
		Self {
			fov: camera.fov.value(),
			near: camera.near,
			far: camera.far,
			f_number: camera.f_number.is_finite().then_some(camera.f_number),
			focus_distance: camera.focus_distance,
			sensor_height: camera.sensor_height,
			exposure: camera.exposure,
			detail_level: camera.detail_level,
		}
	}
}

impl serde::Serialize for Camera {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		SerializableCamera::from(self).serialize(serializer)
	}
}

impl<'de> serde::Deserialize<'de> for Camera {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		Camera::try_from(SerializableCamera::deserialize(deserializer)?).map_err(serde::de::Error::custom)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_camera_payload_round_trips_and_names_only_what_it_changes() {
		let camera = Camera::new()
			.with_f_number(1.8)
			.with_exposure(-2.0)
			.with_clip_planes(0.5, 300.0);
		let json = serde_json::to_value(&camera).unwrap();
		assert_eq!(json["f_number"], 1.8f32);
		assert_eq!(serde_json::from_value::<Camera>(json).unwrap(), camera);

		// A missing field keeps a new camera's value, and null is a pinhole.
		let camera: Camera = serde_json::from_str(r#"{ "fov": 60, "f_number": null }"#).unwrap();
		assert_eq!(camera.vertical_fov(), Degrees::new(60.0));
		assert_eq!(camera.f_number(), f32::INFINITY);
		assert_eq!((camera.near(), camera.far()), (0.1, 100.0));
		assert!(serde_json::to_value(&camera).unwrap()["f_number"].is_null());
	}

	#[test]
	fn a_camera_payload_outside_the_camera_rules_is_rejected() {
		for payload in [
			r#"{ "fov": 0 }"#,
			r#"{ "fov": 180 }"#,
			r#"{ "near": 0 }"#,
			r#"{ "near": 5, "far": 5 }"#,
			r#"{ "f_number": 0 }"#,
			r#"{ "focus_distance": -1 }"#,
			r#"{ "sensor_height": 0 }"#,
			r#"{ "exposure": 65 }"#,
			r#"{ "aspect_ratio": 1.5 }"#,
		] {
			assert!(serde_json::from_str::<Camera>(payload).is_err(), "{payload}");
		}
	}
}
