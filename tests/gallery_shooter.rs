#![feature(const_mut_refs)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use byte_engine::{application::Application, Vec3f, input_manager::{self, Action}, Vector3, core::{self, orchestrator::{self,}, Entity, EntityHandle, property::{Property, DerivedProperty}, event::{Event, FreeEventImplementation}}, math, rendering::mesh, rendering::point_light::PointLight, audio::audio_system::{AudioSystem, DefaultAudioSystem}, ui::{self, Text}, physics};
use maths_rs::{prelude::{MatTranslate, MatScale, MatInverse}, vec::Vec3};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");

	let audio_system_handle = app.get_audio_system_handle().clone();
	let mut physics_world_handle = app.get_physics_world_handle().clone();

	app.initialize(std::env::args());

	let orchestrator_handle = app.get_orchestrator_handle();

	let lookaround_action_handle = core::spawn(input_manager::Action::new("Lookaround", &[
				input_manager::ActionBindingDescription::new("Mouse.Position").mapped(input_manager::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input_manager::Function::Sphere),
				input_manager::ActionBindingDescription::new("Gamepad.RightStick"),
			],));

	let trigger_action = core::spawn(input_manager::Action::new("Trigger", &[
				input_manager::ActionBindingDescription::new("Mouse.LeftButton"),
				input_manager::ActionBindingDescription::new("Gamepad.RightTrigger"),
			],));

	let orchestrator_handle = app.get_orchestrator_handle();

	let scale = maths_rs::Mat4f::from_scale(Vec3f::new(0.1, 0.1, 0.1));
	
	let duck_1 = core::spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 2.0)) * scale));
	
	let physics_duck_1 = core::spawn(physics::Sphere::new(Vec3f::new(0.0, 0.0, 2.0), Vec3f::new(0.0, 0.0, 0.0), 0.1));
	
	let duck_2 = core::spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(2.0, 0.0, 0.0)) * scale));
	let duck_3 = core::spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(-2.0, 0.0, 0.0)) * scale));
	let duck_4 = core::spawn(mesh::Mesh::new("Box", "solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, -2.0)) * scale));
	
	let _sun: EntityHandle<PointLight> = core::spawn(PointLight::new(Vec3f::new(0.0, 2.5, -1.5), 4500.0));

	let mut player = core::spawn(Player::new(orchestrator_handle, lookaround_action_handle, audio_system_handle, physics_world_handle.clone(), physics_duck_1.clone()));

	{
		let mut ta = trigger_action.write_sync();
		ta.subscribe(&player, Player::shoot);
	}

	{
		let mut player = player.write_sync();
		core::spawn(ui::TextComponent::new(&mut player.magazine_as_string));
	}

	{
		let mut events: Vec<Box<dyn Event<bool>>> = Vec::new();
		events.push(Box::new(FreeEventImplementation::new(|_: &bool| {
			log::info!("Duck 1 collided!");
		})));

		for d in events {
			d.fire(&true);
		}
	}

	app.do_loop();

	app.deinitialize();
}

struct Player {
	orchestrator: orchestrator::OrchestratorHandle,

	mesh: EntityHandle<mesh::Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,

	audio_system: EntityHandle<byte_engine::audio::audio_system::DefaultAudioSystem>,
	physics_world: EntityHandle<physics::PhysicsWorld>,

	physics_duck: EntityHandle<physics::Sphere>,

	magazine_size: Property<usize>,
	magazine_as_string: DerivedProperty<usize, String>,

	magazine_capacity: usize,
}

impl Entity for Player {}

impl Player {
	fn new(orchestrator: orchestrator::OrchestratorHandle, lookaround: EntityHandle<Action<Vec3f>>, audio_system: EntityHandle<DefaultAudioSystem>, physics_world_handle: EntityHandle<physics::PhysicsWorld>, physics_duck: EntityHandle<physics::Sphere>) -> orchestrator::EntityReturn<'static, Self> {
		orchestrator::EntityReturn::new_from_closure(move || {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.25, -0.15, 0.4f32));
			transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.03, 0.2));
	
			let camera_handle = core::spawn(byte_engine::camera::Camera::new(Vec3f::new(0.0, 0.0, 0.0)));
	
			// orchestrator.tie(&camera_handle, byte_engine::camera::Camera::orientation, &lookaround, input_manager::Action::value);

			let mut magazine_size = Property::new(5);
			let magazine_as_string = DerivedProperty::new(&mut magazine_size, |magazine_size| { magazine_size.to_string() });

			Self {
				orchestrator,

				physics_duck,

				audio_system: audio_system,
				physics_world: physics_world_handle,

				camera: camera_handle,
				mesh: core::spawn(mesh::Mesh::new("Box", "solid", transform)),

				magazine_size,
				magazine_as_string,
				magazine_capacity: 5,
			}
		})
	}
}

impl Player {
	fn shoot(&mut self, value: &bool) {
		if *value {
			{
				let mut audio_system = self.audio_system.write_sync();
				smol::block_on(audio_system.play("gun"));
			}

			core::spawn::<Bullet>(Bullet::new(&mut self.physics_world, Vec3f::new(0.0, 0.0, 0.0), self.physics_duck.clone()));

			self.magazine_size.set(|value| {
				if value - 1 == 0 {
					self.magazine_capacity
				} else {
					value - 1
				}
			});
		}
	}

	async fn async_shoot(&mut self, value: &bool) {
		if *value {
			{
				let mut audio_system = self.audio_system.write_sync();
				audio_system.play("gun").await;
			}

			core::spawn::<Bullet>(Bullet::new(&mut self.physics_world, Vec3f::new(0.0, 0.0, 0.0), self.physics_duck.clone()));

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
	collision_object: EntityHandle<physics::Sphere>,
	physics_duck: EntityHandle<physics::Sphere>,
}

impl Entity for Bullet {}

impl Bullet {
	fn new(physics_world_handle: &mut EntityHandle<physics::PhysicsWorld>, position: Vec3f, physics_duck: EntityHandle<physics::Sphere>) -> orchestrator::EntityReturn<'_, Self> {
		orchestrator::EntityReturn::new_from_closure(move || {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.0, 0.0));
			transform *= maths_rs::Mat4f::from_scale(Vec3f::new(0.05, 0.05, 0.05));

			let collision_object = core::spawn(physics::Sphere::new(position, Vec3f::new(0.0, 0.0, 0.1), 0.1));

			Self {
				mesh: core::spawn(mesh::Mesh::new("Sphere", "solid", transform,)),
				physics_duck,
				collision_object,
			}
		}).add_post_creation_function(|s| {
			let me = s.clone();
			
			{
				let se = s.write_sync();
				let mut co = se.collision_object.write_sync();
				co.subscribe_to_collision(me, Self::on_collision);
			}
		})
	}

	fn on_collision(&mut self, other: &EntityHandle<physics::Sphere>) {
		if other == &self.physics_duck {
			log::info!("Bullet collided with duck!");
		}
	}
}