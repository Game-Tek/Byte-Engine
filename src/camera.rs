use crate::{Vec3f, orchestrator::{Property, self, Component, Entity}};

#[derive(component_derive::Component)]
/// Camera struct
pub struct Camera {
	pub position: Vec3f,
	pub direction: Vec3f,
	pub fov: f32,
	pub aspect_ratio: f32,
	pub aperture: f32,
	pub focus_distance: f32,
}

pub struct CameraParameters {
	pub position: Vec3f,
	pub direction: Vec3f,
	pub fov: f32,
	pub aspect_ratio: f32,
	pub aperture: f32,
	pub focus_distance: f32,
}

impl Camera {
	/// Returns the field of view of the camera
	fn get_fov(&self) -> f32 { self.fov }

	/// Returns the aspect ratio of the camera
	fn get_aspect_ratio(&self) -> f32 { self.aspect_ratio }

	/// Returns the aperture of the camera
	fn get_aperture(&self) -> f32 { self.aperture }

	/// Returns the focus distance of the camera
	fn get_focus_distance(&self) -> f32 { self.focus_distance }

	fn get_orientation(&self) -> Vec3f { self.direction }
	fn set_orientation(&mut self, _orchestrator: orchestrator::OrchestratorReference, orientation: Vec3f) { self.direction = orientation; }
	pub const fn orientation() -> Property<Camera, Vec3f> { Property { getter: Self::get_orientation, setter: Self::set_orientation } }

	fn get_position(&self) -> Vec3f { self.position }
	fn set_position(&mut self, _orchestrator: orchestrator::OrchestratorReference, position: Vec3f) { self.position = position; }
	pub const fn position() -> Property<Camera, Vec3f> { Property { getter: Self::get_position, setter: Self::set_position } }
}

impl Entity for Camera {}

impl Component for Camera {
	// type Parameters<'a> = CameraParameters;
}