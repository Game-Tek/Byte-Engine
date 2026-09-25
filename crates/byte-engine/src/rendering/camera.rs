#[derive(Clone, Debug)]
/// The `Camera` struct provides scene-owned world-space view settings to render sinks and inspection tools.
pub struct Camera {
	fov: Degrees,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
	exposure: f32,
}

impl Camera {
	/// Creates a camera with a world-origin position, default perspective settings, and neutral exposure.
	pub fn new() -> Self {
		Self {
			fov: Degrees::new(45.0),
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
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

	/// Returns the camera aperture.
	pub fn aperture(&self) -> f32 {
		self.aperture
	}

	/// Returns the camera focus distance.
	pub fn focus_distance(&self) -> f32 {
		self.focus_distance
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
		match key {
			"fov" => {
				self.set_fov(Degrees::new(value.parse().map_err(|e| {
					format!("Invalid camera field value. The most likely cause is that fov is not a number: {e}")
				})?));
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_form_a_valid_forward_facing_perspective_camera() {
		let camera = Camera::new();

		assert_eq!(camera.vertical_fov(), Degrees::new(45.0));
		assert_eq!(camera.aspect_ratio(), 1.0);
		assert_eq!(camera.aperture(), 0.0);
		assert_eq!(camera.focus_distance(), 0.0);
		assert_eq!(camera.exposure(), 0.0);
		assert_eq!(camera.exposure_scale(), 1.0);
	}

	#[test]
	fn exposure_stops_double_or_halve_scene_light() {
		assert_eq!(Camera::new().with_exposure(-2.0).exposure_scale(), 0.25);
		assert_eq!(Camera::new().with_exposure(1.0).exposure_scale(), 2.0);
	}
}

use math::{Degrees, Orientation, Point, UnitVector, Vector, direction_from_orientation, orientation_from_direction};

use crate::core::{Entity, EntityHandle};
use crate::inspector::Inspectable;
use crate::space::orientable::Orientable;
