#[derive(Clone, Debug)]
/// The `Camera` struct provides scene-owned world-space view settings to render sinks and inspection tools.
///
/// Besides the view and exposure, it describes a thin lens: an f-number, a focus distance, and a sensor height, which
/// together set how far each depth is out of focus. See [`Self::set_f_number`].
pub struct Camera {
	fov: Degrees,
	aspect_ratio: f32,
	f_number: f32,
	focus_distance: f32,
	sensor_height: f32,
	exposure: f32,
}

impl Camera {
	/// Creates a camera with a world-origin position, default perspective settings, neutral exposure, and a pinhole
	/// lens that keeps every depth in focus.
	pub fn new() -> Self {
		Self {
			fov: Degrees::new(45.0),
			aspect_ratio: 1.0,
			f_number: f32::INFINITY,
			focus_distance: 10.0,
			sensor_height: 0.024,
			exposure: 0.0,
		}
	}

	/// Returns the camera's vertical field of view.
	pub fn vertical_fov(&self) -> Degrees {
		self.fov
	}

	/// Returns the camera's width-to-height aspect ratio.
	pub fn aspect_ratio(&self) -> f32 {
		self.aspect_ratio
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
}

impl Inspectable for Camera {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		let number = |value: &str| {
			value.parse::<f32>().map_err(|e| {
				format!(
					"Invalid camera field value. The most likely cause is that {key} is not a number: {e}. Send a decimal number."
				)
			})
		};
		let positive = |value: &str| {
			number(value).and_then(|number| {
				(number > 0.0 && number.is_finite()).then_some(number).ok_or_else(|| {
					format!("Invalid camera field value. The most likely cause is that {key} is not a positive, finite number. Send a length in meters above zero.")
				})
			})
		};
		match key {
			"fov" => {
				self.set_fov(Degrees::new(number(value)?));
				Ok(())
			}
			// `inf` parses as a pinhole.
			"f_number" => {
				let f_number = number(value)?;
				if f_number > 0.0 {
					self.set_f_number(f_number);
					Ok(())
				} else {
					Err("Invalid camera field value. The most likely cause is that f_number is not above zero. Send an f-number such as 1.8, or inf for a pinhole.".to_string())
				}
			}
			"focus_distance" => {
				self.set_focus_distance(positive(value)?);
				Ok(())
			}
			"sensor_height" => {
				self.set_sensor_height(positive(value)?);
				Ok(())
			}
			_ => Err(format!(
				"Unknown camera field. The most likely cause is an unsupported inspector key: {key}"
			)),
		}
	}
}

impl Default for Camera {
	fn default() -> Self {
		Self::new()
	}
}

use math::Degrees;

use crate::inspector::Inspectable;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn inspector_sets_the_lens() {
		let mut camera = Camera::new();
		camera.set("f_number", "1.8").unwrap();
		camera.set("focus_distance", "2.5").unwrap();
		camera.set("sensor_height", "0.015").unwrap();
		assert_eq!(
			(camera.f_number(), camera.focus_distance(), camera.sensor_height()),
			(1.8, 2.5, 0.015)
		);
		camera.set("f_number", "inf").unwrap();
		assert_eq!(camera.f_number(), f32::INFINITY);
	}

	#[test]
	fn inspector_rejects_lens_values_outside_their_range() {
		let mut camera = Camera::new();
		for (key, value) in [
			("f_number", "0"),
			("f_number", "NaN"),
			("focus_distance", "-1"),
			("focus_distance", "inf"),
			("sensor_height", "0"),
			("sensor_height", "wide"),
		] {
			assert!(camera.set(key, value).is_err(), "{key} = {value}");
		}
		assert_eq!(camera.f_number(), f32::INFINITY);
	}
}
