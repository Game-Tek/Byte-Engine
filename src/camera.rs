use crate::{Vec3f, orchestrator::{Property, ComponentHandle, self}};

/// Camera struct
pub struct Camera {
	position: Vec3f,
	direction: Vec3f,
	fov: f32,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Camera {
	/// Creates a new camera
	pub fn new(orchestrator: &mut orchestrator::Orchestrator, position: Vec3f, fov: f32) -> ComponentHandle<Camera> {
		let camera  =Camera {
			position,
			direction: Vec3f::new(0.0, 0.0, 1.0),
			fov,
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		};

		orchestrator.make_object(camera)
	}

	/// Returns the position of the camera
	fn get_position(&self) -> Vec3f { self.position }

	/// Returns the direction of the camera
	fn get_direction(&self) -> Vec3f { self.direction }

	/// Returns the field of view of the camera
	fn get_fov(&self) -> f32 { self.fov }

	/// Returns the aspect ratio of the camera
	fn get_aspect_ratio(&self) -> f32 { self.aspect_ratio }

	/// Returns the aperture of the camera
	fn get_aperture(&self) -> f32 { self.aperture }

	/// Returns the focus distance of the camera
	fn get_focus_distance(&self) -> f32 { self.focus_distance }

	fn get_orientation(&self) -> Vec3f { self.direction }
	fn set_orientation(&mut self, orientation: Vec3f) { self.direction = orientation; }
	pub const fn orientation() -> Property<(), Camera, Vec3f> { Property::Component { getter: Self::get_orientation, setter: Self::set_orientation } }
}