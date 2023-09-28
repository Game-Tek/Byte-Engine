#![feature(const_mut_refs)]

use byte_engine::{application::Application, Vec3f, input_manager, Vector3, orchestrator::{Component, EntityHandle, self, System,}, render_domain::{Mesh, MeshParameters}, math};
use maths_rs::prelude::{MatTranslate, MatScale, MatInverse};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	let orchestrator = app.get_mut_orchestrator();

	let lookaround_action_handle: EntityHandle<input_manager::Action<Vector3>> = orchestrator.spawn(input_manager::Action{ name: "Lookaround", bindings: vec![
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.Position")).mapped(input_manager::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input_manager::Function::Sphere),
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightStick")),
		],
		phantom: std::marker::PhantomData,
	});

	let _trigger_action: orchestrator::EntityHandle<input_manager::Action<bool>> = orchestrator.spawn(input_manager::Action{ name: "Trigger", bindings: vec![
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.LeftButton")),
			// input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightTrigger")),
		],
		phantom: std::marker::PhantomData,
	});

	let player: EntityHandle<Player> = orchestrator.spawn_entity(Player::new(lookaround_action_handle)).expect("Failed to spawn player");

	let scale = maths_rs::Mat4f::from_scale(Vec3f::new(0.1, 0.1, 0.1));

	let duck_1: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 2.0)) * scale, });
	let duck_2: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(2.0, 0.0, 0.0)) * scale, });
	let duck_3: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(-2.0, 0.0, 0.0)) * scale, });
	let duck_4: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, -2.0)) * scale, });

	app.do_loop();

	app.deinitialize();
}

struct Player {
	mesh: EntityHandle<Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,
}

impl orchestrator::Entity for Player {}

impl Component for Player {
	// type Parameters<'a> = EntityHandle<input_manager::Action<Vec3f>>;
}

impl Player {
	fn new(lookaround: EntityHandle<input_manager::Action<Vec3f>>) -> orchestrator::EntityReturn<Self> {
		orchestrator::EntityReturn::new_from_closure(move |orchestrator| {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.25, -0.15, 0.4f32));
			transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.03, 0.2));
	
			let camera_handle = orchestrator.spawn(byte_engine::camera::Camera{
				position: Vec3f::new(0.0, 0.0, 0.0),
				direction: Vec3f::new(0.0, 0.0, 1.0),
				fov: 90.0,
				aspect_ratio: 1.0,
				aperture: 0.0,
				focus_distance: 0.0,
			});
	
			orchestrator.tie(&camera_handle, byte_engine::camera::Camera::orientation, &lookaround, input_manager::Action::value);
	
			// orchestrator.tie_self(Player::lookaround, &handle, input_manager::Action::value);

			Self {
				camera: camera_handle,
				mesh: orchestrator.spawn(Mesh{ resource_id: "Box", transform, }),
			}
		})
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