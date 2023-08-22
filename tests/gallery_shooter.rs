#![feature(const_mut_refs)]

use byte_engine::{application::Application, Vec3f, input_manager, Vector3, orchestrator::{Component, EntityHandle, self}, render_domain::{Mesh, MeshParameters}, Vector2, math};
use maths_rs::prelude::{MatTranslate, MatScale, MatInverse};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	let orchestrator = app.get_mut_orchestrator();

	let lookaround_action_handle: EntityHandle<input_manager::Action<Vector3>> = orchestrator.spawn_component(("Lookaround", [
		input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.Position")).mapped(input_manager::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input_manager::Function::Sphere),
		input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightStick")),
	].as_slice()));

	let x = [input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.LeftButton"))];

	let trigger_action: orchestrator::EntityHandle<input_manager::Action<bool>> = orchestrator.spawn_component(("Trigger", x.as_slice()));

	let player: EntityHandle<Player> = orchestrator.spawn_component((lookaround_action_handle));

	let scale = maths_rs::Mat4f::from_scale(Vec3f::new(0.1, 0.1, 0.1));

	// let duck_1: EntityHandle<Mesh> = orchestrator.spawn_component(MeshParameters{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 2.0)) * scale, });
	// let duck_2: EntityHandle<Mesh> = orchestrator.spawn_component(MeshParameters{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(2.0, 0.0, 0.0)) * scale, });
	// let duck_3: EntityHandle<Mesh> = orchestrator.spawn_component(MeshParameters{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(-2.0, 0.0, 0.0)) * scale, });
	// let duck_4: EntityHandle<Mesh> = orchestrator.spawn_component(MeshParameters{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, -2.0)) * scale, });

	app.do_loop();

	app.deinitialize();
}

struct Player {
	mesh: EntityHandle<Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,
}

impl orchestrator::Entity for Player {}

impl Component for Player {
	type Parameters<'a> = EntityHandle<input_manager::Action<Vec3f>>;
	fn new(orchestrator: orchestrator::OrchestratorReference, params: Self::Parameters<'_>) -> Self {
		let mut transform = maths_rs::Mat4f::identity();

		transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.25, -0.15, 0.4f32));
		transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.03, 0.2));

		let camera_handle = orchestrator.spawn_component(byte_engine::camera::CameraParameters{
			position: Vec3f::new(0.0, 0.0, 0.0),
			direction: Vec3f::new(0.0, 0.0, 1.0),
			fov: 90.0,
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		});

		orchestrator.tie(&camera_handle, byte_engine::camera::Camera::orientation, &params, input_manager::Action::value);

		orchestrator.tie_self(Player::lookaround, &params, input_manager::Action::value);

		Self {
			camera: camera_handle,
			mesh: orchestrator.spawn_component(MeshParameters{ resource_id: "Box", transform, }),
		}
	}
}

impl Player {
	pub const fn lookaround() -> orchestrator::Property<(), Player, Vec3f> { orchestrator::Property::Component { getter: Self::get_lookaround, setter: Self::set_lookaround } }

	fn get_lookaround(&self) -> Vec3f {
		Vec3f::new(0.0, 0.0, 0.0)
	}

	fn set_lookaround(&mut self, orchestrator: orchestrator::OrchestratorReference, direction: Vec3f) {
		let mut transform = maths_rs::Mat4f::identity();

		transform *= maths_rs::Mat4f::from_translation(direction);
		transform *= math::look_at(direction).inverse();
		transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.25, -0.15, 0.0f32));
		transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.03, 0.2));

		orchestrator.set_property(&self.mesh, Mesh::transform, transform);
	}
}