//! Cubecraft example application
//! This demonstrates a simple first person game, which is definitely not a clone of Minecraft.
//! It uses the Byte-Engine to create a simple game with a player character that can move around and jump.
//! It also includes a simple physics engine to handle collisions and movement.

use std::borrow::Cow;

use byte_engine::core::listener::Listener;
use byte_engine::core::Entity;
use byte_engine::core::EntityHandle;

use byte_engine::gameplay::space::Spawn;
use byte_engine::rendering::aces_tonemap_render_pass::AcesToneMapPass;
use byte_engine::rendering::common_shader_generator::CommonShaderGenerator;
use byte_engine::rendering::mesh::{MeshGenerator, MeshSource, RenderEntity};
use byte_engine::{application::{Application, Parameter}, camera::Camera, input::{Action, ActionBindingDescription, Function}, rendering::directional_light::DirectionalLight, Vector3};
use maths_rs::mat::MatTranslate;

#[ignore]
#[test]
fn cubecraft() {
	// Create the Byte-Engine application
	let mut app = byte_engine::application::GraphicsApplication::new("Cubecraft", &[Parameter::new("resources-path", "../../resources"), Parameter::new("assets-path", "../../assets")]);

	{
		let generator = {
			let common_shader_generator = CommonShaderGenerator::new();
			common_shader_generator
		};

		byte_engine::application::graphics_application::setup_default_resource_and_asset_management(&mut app, generator);
	}

	{
		let mut renderer = app.get_renderer_handle().write();
		renderer.add_render_pass::<AcesToneMapPass>(app.get_root_space_handle().clone());
	}

	byte_engine::application::graphics_application::setup_default_input(&mut app);
	byte_engine::application::graphics_application::setup_default_window(&mut app);

	// Get the root space handle
	let space_handle = app.get_root_space_handle();

	// Create the lookaround action handle
	let lookaround_action_handle = space_handle.spawn(Action::<Vector3>::new("Lookaround", &[
		ActionBindingDescription::new("Mouse.Position").mapped(Vector3::new(1f32, 1f32, 1f32).into(), Function::Sphere),
		ActionBindingDescription::new("Gamepad.RightStick"),
	],));

	// Create the move action
	let move_action_handle = space_handle.spawn(Action::<Vector3>::new("Move", &[
		ActionBindingDescription::new("Keyboard.W").mapped(Vector3::new(0f32, 0f32, 1f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.S").mapped(Vector3::new(0f32, 0f32, -1f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.A").mapped(Vector3::new(-1f32, 0f32, 0f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.D").mapped(Vector3::new(1f32, 0f32, 0f32).into(), Function::Linear),

		ActionBindingDescription::new("Gamepad.LeftStick").mapped(Vector3::new(1f32, 0f32, 1f32).into(), Function::Linear),
	],));

	// Create the jump action
	let jump_action_handle = space_handle.spawn(Action::<bool>::new("Jump", &[
		ActionBindingDescription::new("Keyboard.Space"),
		ActionBindingDescription::new("Gamepad.A"),
	],));

	// Create the right hand action
	let fire_action_handle = space_handle.spawn(Action::<bool>::new("RightHand", &[
		ActionBindingDescription::new("Mouse.LeftButton"),
		ActionBindingDescription::new("Gamepad.RightTrigger"),
	],));

	// Create the camera
	let camera = space_handle.spawn(Camera::new(Vector3::new(0.0, 1.0, 0.0),));

	// Create the directional light
	let _ = space_handle.spawn(DirectionalLight::new(maths_rs::normalize(Vector3::new(0.0, -1.0, 0.0)), 4000f32));

	const CHUNK_SIZE: i32 = 16;

	for x in -CHUNK_SIZE..CHUNK_SIZE {
		for z in -CHUNK_SIZE..CHUNK_SIZE {
			for y in -CHUNK_SIZE..CHUNK_SIZE {
				let position = (x, y, z);
				let block = make_block(position);

				if block == GRASS_BLOCK {
					space_handle.spawn(Block::new(position, block));
				}
			}
		}
	}

	app.do_loop()
}

struct Block {
	position: (i32, i32, i32),
	block: u32,

	source: MeshSource,
}

impl Block {
	fn new(position: (i32, i32, i32), block: u32) -> Self {
		Block { position, block, source: MeshSource::Generated(Box::new(CubeMeshGenerator {})) }
	}
}

impl Entity for Block {
	fn call_listeners<'a>(&'a self, listener: &'a byte_engine::core::listener::BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		listener.invoke_for(handle.clone() as EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
	}
}

impl RenderEntity for Block {
	fn get_mesh(&self) -> &byte_engine::rendering::mesh::MeshSource {
		&self.source
	}

	fn get_transform(&self) -> maths_rs::Mat4f {
		maths_rs::Mat4f::from_translation(Vector3::new(self.position.0 as f32, self.position.1 as f32, self.position.2 as f32))
	}
}

struct CubeMeshGenerator {

}

impl MeshGenerator for CubeMeshGenerator {
	fn vertices(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(-0.5, -0.5, -0.5),
			maths_rs::Vec3f::new(0.5, -0.5, -0.5),
			maths_rs::Vec3f::new(0.5, 0.5, -0.5),
			maths_rs::Vec3f::new(-0.5, 0.5, -0.5),
			maths_rs::Vec3f::new(-0.5, -0.5, 0.5),
			maths_rs::Vec3f::new(0.5, -0.5, 0.5),
			maths_rs::Vec3f::new(0.5, 0.5, 0.5),
			maths_rs::Vec3f::new(-0.5, 0.5, 0.5),
		])
	}

	fn normals(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(0.0, 0.0, -1.0),
			maths_rs::Vec3f::new(0.0, 0.0, -1.0),
			maths_rs::Vec3f::new(0.0, 0.0, -1.0),
			maths_rs::Vec3f::new(0.0, 0.0, -1.0),
			maths_rs::Vec3f::new(0.0, 0.0, 1.0),
			maths_rs::Vec3f::new(0.0, 0.0, 1.0),
			maths_rs::Vec3f::new(0.0, 0.0, 1.0),
			maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		])
	}

	fn tangents(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
		])
	}

	fn bitangents(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
		])
	}

	fn uvs(&self) -> std::borrow::Cow<[maths_rs::Vec2f]> {
		Cow::Owned(vec![
			maths_rs::Vec2f::new(0.0, 0.0),
			maths_rs::Vec2f::new(1.0, 0.0),
			maths_rs::Vec2f::new(1.0, 1.0),
			maths_rs::Vec2f::new(0.0, 1.0),
			maths_rs::Vec2f::new(0.0, 0.0),
			maths_rs::Vec2f::new(1.0, 0.0),
			maths_rs::Vec2f::new(1.0, 1.0),
			maths_rs::Vec2f::new(0.0, 1.0),
		])
	}

	fn indices(&self) -> std::borrow::Cow<[u32]> {
		Cow::Owned(vec![
			0, 1, 2,
			0, 2, 3,
			4, 5, 6,
			4, 6, 7,
			0, 1, 5,
			0, 5, 4,
			1, 2, 6,
			1, 6, 5,
			2, 3, 7,
			2, 7, 6,
			3, 0, 4,
			3, 4, 7,
		])
	}
}

const AIR_BLOCK: u32 = 0;
const GRASS_BLOCK: u32 = 1;

fn make_block(position: (i32, i32, i32)) -> u32 {
	if position.1 > 0 {
		AIR_BLOCK
	} else {
		GRASS_BLOCK
	}
}