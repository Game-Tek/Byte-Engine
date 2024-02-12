use crate::core::Entity;
use crate::Vec3f;

pub struct Camera {
	position: Vec3f,
	direction: Vec3f,
	fov: f32,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Camera {
	pub fn new(position: Vec3f) -> Self {
		Self {
			position,
			direction: Vec3f::new(0.0, 0.0, 1.0),
			fov: 90.0,
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		}
	}

	/// Returns the field of view of the camera
	fn get_fov(&self) -> f32 { self.fov }

	/// Returns the aspect ratio of the camera
	fn get_aspect_ratio(&self) -> f32 { self.aspect_ratio }

	/// Returns the aperture of the camera
	fn get_aperture(&self) -> f32 { self.aperture }

	/// Returns the focus distance of the camera
	fn get_focus_distance(&self) -> f32 { self.focus_distance }

	pub fn get_orientation(&self) -> Vec3f { self.direction }
	pub fn set_orientation(&mut self, orientation: Vec3f) { self.direction = orientation; }

	pub fn get_position(&self) -> Vec3f { self.position }
	pub fn set_position(&mut self, position: Vec3f) { self.position = position; }
}

impl Entity for Camera {}