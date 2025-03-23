#![feature(closure_lifetime_binder)]

use byte_engine::core::{entity::{DomainType, EntityBuilder, SelfDestroyingEntity, SpawnerEntity,}, property::{DerivedProperty, Property}, spawn_as_child, Entity, EntityHandle};
use std::f32::consts::PI;
use byte_engine::{application::application::Application, audio::audio_system::{AudioSystem, DefaultAudioSystem}, gameplay::{self, space::Space, Transform}, input, math::from_normal, physics::{self, PhysicsEntity}, rendering::{directional_light::DirectionalLight, mesh::{self}}, Vector3};
use maths_rs::{mat::MatInverse, swizz::Vec3Swizzle, vec::Vec4};
use utils::r#async::block_on;

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter", &[]);

	let audio_system_handle = app.get_audio_system_handle().clone();

	let runtime = app.get_runtime();

	let space_handle = app.get_root_space_handle();

	let lookaround_action_handle = runtime.block_on(spawn_as_child(space_handle.clone(), input::Action::new("Lookaround", &[
		input::ActionBindingDescription::new("Mouse.Position").mapped(input::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input::Function::Sphere),
		input::ActionBindingDescription::new("Gamepad.RightStick"),
	],)));

	let trigger_action = runtime.block_on(spawn_as_child(space_handle.clone(), input::Action::new("Trigger", &[
		input::ActionBindingDescription::new("Mouse.LeftButton"),
		input::ActionBindingDescription::new("Gamepad.RightTrigger"),
	],)));

	let scale = Vector3::new(0.1, 0.1, 0.1);

	let floor: EntityHandle<gameplay::object::Object> = runtime.block_on(spawn_as_child(space_handle.clone(), gameplay::object::Object::new("Box.glb", Transform::default().position(Vector3::new(0.0, -1.25f32, 1.0)).scale(Vector3::new(10f32, 1f32, 10f32)), physics::BodyTypes::Static, Vector3::new(0f32, 0f32, 0f32))));

	let duck_1: EntityHandle<gameplay::object::Object> = runtime.block_on(spawn_as_child(space_handle.clone(), gameplay::object::Object::new("Box.glb", Transform::default().position(Vector3::new(0.0, 0.0, 2.0)).scale(scale), physics::BodyTypes::Kinematic, Vector3::new(0f32, 0f32, 0f32))));
	let duck_2: EntityHandle<gameplay::object::Object> = runtime.block_on(spawn_as_child(space_handle.clone(), gameplay::object::Object::new("Box.glb", Transform::default().position(Vector3::new(2.0, 0.0, 0.0)).scale(scale), physics::BodyTypes::Kinematic, Vector3::new(0f32, 0f32, 0f32))));
	let duck_3: EntityHandle<gameplay::object::Object> = runtime.block_on(spawn_as_child(space_handle.clone(), gameplay::object::Object::new("Box.glb", Transform::default().position(Vector3::new(-2.0, 0.0, 0.0)).scale(scale), physics::BodyTypes::Kinematic, Vector3::new(0f32, 0f32, 0f32))));
	let duck_4: EntityHandle<gameplay::object::Object> = runtime.block_on(spawn_as_child(space_handle.clone(), gameplay::object::Object::new("Box.glb", Transform::default().position(Vector3::new(0.0, 0.0, -2.0)).scale(scale), physics::BodyTypes::Kinematic, Vector3::new(0f32, 0f32, 0f32))));

	app.get_tick_handle().write_sync().add(move |v| {
		let mut ducks = vec![duck_1.write_sync(), duck_2.write_sync(), duck_3.write_sync(), duck_4.write_sync(),];

		let alpha = 2.0f32 * PI / ducks.len() as f32;

		for (i, duck) in ducks.iter_mut().enumerate() {
			let x = alpha * i as f32 + v.elapsed().as_secs_f32();
			let z = alpha * i as f32 + v.elapsed().as_secs_f32();

			duck.set_position(Vector3::new(x.cos(), 0.0, z.sin()));
		}
	});

	// let _sun: EntityHandle<PointLight> = crate::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(0.0, 1.5, -0.5), 4500.0));
	let _sun: EntityHandle<DirectionalLight> = runtime.block_on(spawn_as_child(space_handle.clone(), DirectionalLight::new(maths_rs::normalize(Vector3::new(-1.0, -1.0, 1.0)), 4500.0)));

	let mut game_state = runtime.block_on(spawn_as_child(space_handle.clone(), GameState::new()));

	let mut player: EntityHandle<Player> = runtime.block_on(spawn_as_child(space_handle.clone(), Player::new(game_state, lookaround_action_handle, trigger_action, audio_system_handle)));

	app.do_loop();

	app.close();
}

/// This struct represents the state of the game.
/// It contains match duration, score, and other game-related information.
struct GameState {
	/// The score of the player.
	points: Property<usize>,

	/// The duration of the match.
	duration: std::time::Duration,
}

impl Entity for GameState {}

impl GameState {
	fn new() -> EntityBuilder<'static, Self> {
		EntityBuilder::new_from_closure(|| {
			Self {
				points: Property::new(0),
				duration: std::time::Duration::new(30, 0),
			}
		})
	}

	/// This function is called when a duck is hit.
	fn on_duck_hit(&mut self, other: &EntityHandle<dyn physics::PhysicsEntity>) {
		self.points.set(|points| points + 1);

		log::info!("Duck hit! Points: {}", self.points.get());
	}
}

struct Player {
	parent: EntityHandle<Space>,
	game_state: EntityHandle<GameState>,

	mesh: EntityHandle<mesh::Mesh>,
	camera: EntityHandle<byte_engine::camera::Camera>,

	audio_system: EntityHandle<byte_engine::audio::audio_system::DefaultAudioSystem>,

	magazine_size: Property<usize>,
	magazine_as_string: DerivedProperty<usize, String>,

	magazine_capacity: usize,
}

impl Entity for Player {}
impl SpawnerEntity<Space> for Player {
	fn get_parent(&self) -> EntityHandle<Space> { self.parent.clone() }
}

impl Player {
	fn new(game_state: EntityHandle<GameState>, lookaround: EntityHandle<input::Action<Vector3>>, click: EntityHandle<input::Action<bool>>, audio_system: EntityHandle<DefaultAudioSystem>,) -> EntityBuilder<'static, Self> {
		EntityBuilder::new_from_async_function_with_parent(async move |parent: DomainType| {
			let transform = Transform::default().position(Vector3::new(0.25, -0.15, 0.4f32)).scale(Vector3::new(0.05, 0.03, 0.2));
			let camera_handle = crate::spawn_as_child(parent.clone(), byte_engine::camera::Camera::new(Vector3::new(0.0, 0.0, 0.0))).await;

			let mut magazine_size = Property::new(5);
			let magazine_as_string = DerivedProperty::new(&mut magazine_size, |magazine_size| { magazine_size.to_string() });

			Self {
				parent: parent.downcast().unwrap(),
				game_state,

				audio_system: audio_system,

				camera: camera_handle,
				mesh: crate::spawn_as_child(parent, mesh::Mesh::new("Box.glb", transform)).await,

				magazine_size,
				magazine_as_string,
				magazine_capacity: 5,
			}
		}).then(move |this| {
			lookaround.write_sync().value_mut().link_to(this.clone(), Player::lookaround);
			click.write_sync().value_mut().link_to(this.clone(), Player::shoot);
		})
	}
}

impl Player {
	fn lookaround(&mut self, value: &Vector3) {
		let mut camera = self.camera.write_sync();
		camera.set_orientation(*value);
		let mut mesh = self.mesh.write_sync();
		mesh.transform_mut().set_orientation(*value);
		let nmatrix = from_normal(*value);
		mesh.transform_mut().set_position((nmatrix.inverse() * Vec4::new(0.25, -0.15, 0.4, 1f32)).xyz());
	}

	fn shoot(&mut self, value: &bool) {
		if *value {
			log::info!("Shooting!");

			{
				let mut audio_system = self.audio_system.write_sync();
				block_on(audio_system.play("gun.wav"));
			}

			let direction = {
				let mesh = self.mesh.read_sync();
				mesh.transform().get_orientation()
			};

			self.spawn::<Bullet>(Bullet::new(Vector3::new(0.0, 0.0, 0.0),direction,).then(|s| { s.read_sync().bullet_object.write_sync().on_collision().subscribe(self.game_state.clone(), GameState::on_duck_hit) }));

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

			// crate::core::spawn_in_domain::<Bullet>(Bullet::new(&mut self.physics_world, Vec3f::new(0.0, 0.0, 0.0), self.physics_duck.clone()));

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
}

impl Entity for Bullet {}

impl SelfDestroyingEntity for Bullet {
	fn destroy(&self) {
	}
}

impl Bullet {
	fn new<'a>(position: Vector3, direction: Vector3) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_async_function_with_parent(async move |parent: DomainType| {
			let bullet_object = crate::spawn_as_child(parent, gameplay::object::Object::new("Box.glb", Transform::identity().position(position).rotation(direction).scale(Vector3::new(0.1f32, 0.1f32, 0.1f32)), physics::BodyTypes::Dynamic, direction * 20.0f32,)).await;

			Self {
				bullet_object,
			}
		}).then(|s| {
			let me = s.clone();

			{
				let se = s.write_sync();
				let mut co = se.bullet_object.write_sync();
				co.on_collision().subscribe(me, Self::on_collision);
			}
		})
	}

	fn on_collision(&mut self, other: &EntityHandle<dyn physics::PhysicsEntity>) {
	}
}
