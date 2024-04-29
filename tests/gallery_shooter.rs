#![feature(const_mut_refs)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use core::{self, entity::{EntityBuilder, SpawnerEntity}, event::EventLike, property::{DerivedProperty, Property}, Entity, EntityHandle};
use byte_engine::{application::Application, audio::audio_system::DefaultAudioSystem, gameplay::{self, space::Space}, input, physics::{self, PhysicsEntity}, rendering::{mesh, point_light::PointLight}, Vector3};
use maths_rs::prelude::{MatTranslate, MatScale};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");

	let audio_system_handle = app.get_audio_system_handle().clone();

	app.initialize(std::env::args());

	let space_handle = app.get_root_space_handle();

	let lookaround_action_handle = core::spawn_as_child(space_handle.clone(), input::Action::new("Lookaround", &[
		input::ActionBindingDescription::new("Mouse.Position").mapped(input::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input::Function::Sphere),
		input::ActionBindingDescription::new("Gamepad.RightStick"),
	],));

	let trigger_action = core::spawn_as_child(space_handle.clone(), input::Action::new("Trigger", &[
		input::ActionBindingDescription::new("Mouse.LeftButton"),
		input::ActionBindingDescription::new("Gamepad.RightTrigger"),
	],));

	let scale = maths_rs::Mat4f::from_scale(Vector3::new(0.1, 0.1, 0.1));
	
	let duck_1: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Box.gltf", "white_solid.json", maths_rs::Mat4f::from_translation(Vector3::new(0.0, 0.0, 2.0)) * scale));
	
	let physics_duck_1 = core::spawn_as_child(space_handle.clone(), physics::Sphere::new(Vector3::new(0.0, 0.0, 2.0), Vector3::new(0.0, 0.0, 0.0), 0.1));
	
	let duck_2: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Box.gltf", "green_solid.json", maths_rs::Mat4f::from_translation(Vector3::new(2.0, 0.0, 0.0)) * scale));
	let duck_3: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Box.gltf", "green_solid.json", maths_rs::Mat4f::from_translation(Vector3::new(-2.0, 0.0, 0.0)) * scale));
	let duck_4: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Box.gltf", "red_solid.json", maths_rs::Mat4f::from_translation(Vector3::new(0.0, 0.0, -2.0)) * scale));
	
	let _sun: EntityHandle<PointLight> = core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(0.0, 2.5, -1.5), 4500.0));

	let mut player: EntityHandle<Player> = core::spawn_as_child(space_handle.clone(), Player::new(lookaround_action_handle, trigger_action, audio_system_handle, physics_duck_1.clone()));

	app.do_loop();
}

struct Player {
	parent: EntityHandle<Space>,

	mesh: EntityHandle<mesh::Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,

	audio_system: EntityHandle<byte_engine::audio::audio_system::DefaultAudioSystem>,

	physics_duck: EntityHandle<physics::Sphere>,

	magazine_size: Property<usize>,
	magazine_as_string: DerivedProperty<usize, String>,

	magazine_capacity: usize,
}

impl Entity for Player {}
impl SpawnerEntity<Space> for Player {
	fn get_parent(&self) -> EntityHandle<Space> { self.parent.clone() }
}

impl Player {
	fn new(lookaround: EntityHandle<input::Action<Vector3>>, click: EntityHandle<input::Action<bool>>, audio_system: EntityHandle<DefaultAudioSystem>, physics_duck: EntityHandle<physics::Sphere>) -> EntityBuilder<'static, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			let mut transform = maths_rs::Mat4f::identity();

			transform *= maths_rs::Mat4f::from_translation(Vector3::new(0.25, -0.15, 0.4f32));
			transform *= maths_rs::Mat4f::from_scale(Vector3::new(0.05, 0.03, 0.2));

			let camera_handle = core::spawn_as_child(parent.clone(), byte_engine::camera::Camera::new(Vector3::new(0.0, 0.0, 0.0)));

			let mut magazine_size = Property::new(5);
			let magazine_as_string = DerivedProperty::new(&mut magazine_size, |magazine_size| { magazine_size.to_string() });

			Self {
				parent: parent.downcast().unwrap(),
				physics_duck,

				audio_system: audio_system,

				camera: camera_handle,
				mesh: core::spawn_as_child(parent, mesh::Mesh::new("Box.gltf", "solid.json", transform)),

				magazine_size,
				magazine_as_string,
				magazine_capacity: 5,
			}
		}).add_post_creation_function(move |this| {
			lookaround.write_sync().value_mut().link_to(this.clone(), Player::lookaround);
			click.write_sync().value_mut().link_to(this.clone(), Player::shoot);
		})
	}
}

impl Player {
	fn lookaround(&mut self, value: &Vector3) {
		let mut camera = self.camera.write_sync();
		camera.set_orientation(*value);
	}

	fn shoot(&mut self, value: &bool) {
		if *value {
			log::info!("Shooting!");

			{
				let mut audio_system = self.audio_system.write_sync();
				// smol::block_on(audio_system.play("gun"));
			}

			self.spawn::<Bullet>(Bullet::new(Vector3::new(0.0, 0.0, 0.0), self.physics_duck.clone()));

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
				// audio_system.play("gun").await;
			}

			// core::spawn_in_domain::<Bullet>(Bullet::new(&mut self.physics_world, Vec3f::new(0.0, 0.0, 0.0), self.physics_duck.clone()));

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
	bullet_object: EntityHandle<gameplay::object::Object>,
	physics_duck: EntityHandle<dyn physics::PhysicsEntity>,
}

impl Entity for Bullet {}

impl Bullet {
	fn new<'a>(position: Vector3, physics_duck: EntityHandle<physics::Sphere>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			let bullet_object = core::spawn_as_child(parent, gameplay::object::Object::new(Vector3::new(0f32, 0f32, 0f32), Vector3::new(0f32, 0f32, 0.1f32)));

			Self {
				physics_duck,
				bullet_object,
			}
		}).add_post_creation_function(|s| {
			let me = s.clone();
			
			{
				let se = s.write_sync();
				let mut co = se.bullet_object.write_sync();
				co.on_collision().subscribe(me, Self::on_collision);
			}
		})
	}

	fn on_collision(&mut self, other: &EntityHandle<dyn physics::PhysicsEntity>) {
		if other == &self.physics_duck {
			log::info!("Bullet collided with duck!");
		}
	}
}