#![feature(const_mut_refs)]

use byte_engine::{application::Application, Vec3f, input_manager::{self, Action}, Vector3, orchestrator::{Component, EntityHandle, self, Property, DerivedProperty,}, math, rendering::mesh, rendering::point_light::PointLight, audio::audio_system::AudioSystem, ui::{self, Text}};
use maths_rs::prelude::{MatTranslate, MatScale, MatInverse};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");

	let audio_system_handle = app.get_audio_system_handle().clone();

	app.initialize(std::env::args());

	let orchestrator = app.get_mut_orchestrator();

	let lookaround_action_handle: EntityHandle<input_manager::Action<Vector3>> = orchestrator.spawn(input_manager::Action::new("Lookaround", &[
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.Position")).mapped(input_manager::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input_manager::Function::Sphere),
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightStick")),
		],)
	);

	let trigger_action: orchestrator::EntityHandle<input_manager::Action<bool>> = orchestrator.spawn(input_manager::Action::new("Trigger", &[
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.LeftButton")),
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightTrigger")),
		],)
	);

	let mut player: EntityHandle<Player> = orchestrator.spawn_entity(Player::new(lookaround_action_handle, audio_system_handle)).expect("Failed to spawn player");

	orchestrator.subscribe_to(&player, &trigger_action, input_manager::Action::<bool>::value, Player::shoot);

	let scale = maths_rs::Mat4f::from_scale(Vec3f::new(0.1, 0.1, 0.1));

	let duck_1 = orchestrator.spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 2.0)) * scale));
	let duck_2 = orchestrator.spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(2.0, 0.0, 0.0)) * scale));
	let duck_3 = orchestrator.spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(-2.0, 0.0, 0.0)) * scale));
	let duck_4 = orchestrator.spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, -2.0)) * scale));

	let _sun: EntityHandle<PointLight> = orchestrator.spawn(PointLight::new(Vec3f::new(0.0, 2.5, -1.5), 4500.0));

	let magazine_size_text = player.get_mut(|player| {
		orchestrator.spawn(ui::TextComponent::new(&mut player.magazine_as_string))
	});

	app.do_loop();

	app.deinitialize();
}

struct Player {
	mesh: EntityHandle<mesh::Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,

	audio_system: EntityHandle<dyn byte_engine::audio::audio_system::AudioSystem>,

	magazine_size: Property<usize>,
	magazine_as_string: DerivedProperty<usize, String>,

	magazine_capacity: usize,
}

impl orchestrator::Entity for Player {}

impl Component for Player {
	// type Parameters<'a> = EntityHandle<input_manager::Action<Vec3f>>;
}

impl Player {
	fn new(lookaround: EntityHandle<Action<Vec3f>>, audio_system: EntityHandle<dyn AudioSystem>) -> orchestrator::EntityReturn<'static, Self> {
		orchestrator::EntityReturn::new_from_closure(move |orchestrator| {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.25, -0.15, 0.4f32));
			transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.03, 0.2));
	
			let camera_handle = orchestrator.spawn(byte_engine::camera::Camera::new(Vec3f::new(0.0, 0.0, 0.0)));
	
			orchestrator.tie(&camera_handle, byte_engine::camera::Camera::orientation, &lookaround, input_manager::Action::value);

			let mut magazine_size = Property::new(5);
			let magazine_as_string = DerivedProperty::new(&mut magazine_size, |magazine_size| { magazine_size.to_string() });

			Self {
				audio_system: audio_system,
				camera: camera_handle,
				mesh: orchestrator.spawn(mesh::Mesh::new("Box", "solid", transform)),

				magazine_size,
				magazine_as_string,
				magazine_capacity: 5,
			}
		})
	}
}

impl Player {
	fn shoot(&mut self, value: bool) {
		if value {
			self.audio_system.get_mut(|audio_system| audio_system.play("gun"));

			// TODO: spawn bullet

			self.magazine_size.set(|value| {
				if value - 1 == 0 {
					self.magazine_capacity
				} else {
					value - 1
				}
			});
		}
	}
}

struct Bullet {
	mesh: EntityHandle<mesh::Mesh>,
}

impl orchestrator::Entity for Bullet {}
impl orchestrator::System for Bullet {}

impl Bullet {
	fn new(position: Vec3f) -> orchestrator::EntityReturn<'static, Self> {
		orchestrator::EntityReturn::new_from_closure(move |orchestrator| {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 0.0));
			transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.05, 0.05));

			Self {
				mesh: orchestrator.spawn(mesh::Mesh::new("Sphere", "solid", transform,)),
			}
		})
	}
}