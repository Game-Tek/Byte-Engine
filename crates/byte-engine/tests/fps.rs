use core::EntityHandle;

use byte_engine::{application::{Application, Parameter}, camera::Camera, gameplay::{Anchor, Transform}, input::{Action, ActionBindingDescription, Function, Value}, math, rendering::{directional_light::DirectionalLight, mesh::Mesh}, Vector3};

#[ignore]
#[test]
fn fps() {
	// Create the Byte-Engine application
	let mut app = byte_engine::application::GraphicsApplication::new("Third Person Shooter", &[Parameter::new("resources-path", "../../resources"), Parameter::new("assets-path", "../../assets")]);

	// Get the root space handle
	let space_handle = app.get_root_space_handle();

	// Get the runtime
	let runtime = app.get_runtime();

	// TODO: spawn straight from the space

	// Create the lookaround action handle
	let lookaround_action_handle = runtime.block_on(core::spawn_as_child(space_handle.clone(), Action::<Vector3>::new("Lookaround", &[
		ActionBindingDescription::new("Mouse.Position").mapped(Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), Function::Sphere),
		ActionBindingDescription::new("Gamepad.RightStick"),
	],)));

	// Create the move action
	let move_action_handle = runtime.block_on(core::spawn_as_child(space_handle.clone(), Action::<Vector3>::new("Move", &[
		ActionBindingDescription::new("Keyboard.W").mapped(Value::Vector3(Vector3::new(0f32, 0f32, 1f32)), Function::Linear),
		ActionBindingDescription::new("Keyboard.S").mapped(Value::Vector3(Vector3::new(0f32, 0f32, -1f32)), Function::Linear),
		ActionBindingDescription::new("Keyboard.A").mapped(Value::Vector3(Vector3::new(-1f32, 0f32, 0f32)), Function::Linear),
		ActionBindingDescription::new("Keyboard.D").mapped(Value::Vector3(Vector3::new(1f32, 0f32, 0f32)), Function::Linear),

		ActionBindingDescription::new("Gamepad.LeftStick").mapped(Value::Vector3(Vector3::new(1f32, 0f32, 1f32)), Function::Linear),
	],)));

	// Create the jump action
	let jump_action_handle = runtime.block_on(core::spawn_as_child(space_handle.clone(), Action::<bool>::new("Jump", &[
		ActionBindingDescription::new("Keyboard.Space"),
		ActionBindingDescription::new("Gamepad.A"),
	],)));

	// Create the camera
	let camera = runtime.block_on(core::spawn_as_child(space_handle.clone(), Camera::new(Vector3::new(0.0, 1.0, 0.0),)));

	// Create the directional light
	let _ = runtime.block_on(core::spawn_as_child(space_handle.clone(), DirectionalLight::new(Vector3::new(0.0, -1.0, -1.0), 4000f32)));

	let anchor = runtime.block_on(core::spawn_as_child(space_handle.clone(), Anchor::new(Transform::identity())));

	// Attach the camera to the anchor, offset from the anchor
	anchor.write_sync().attach_with_offset(camera.clone(), Vector3::new(0.0, 1.0, 0.0));

	{
		let camera = camera.clone();
		let anchor = anchor.clone();
	
		// Subscribe to the move action
		move_action_handle.write_sync().value_mut().add(move |v| {
			let mut anchor = anchor.write_sync();
	
			let camera = camera.read_sync();
			let camera_orientation = camera.get_orientation();
	
			let current_position = anchor.transform().get_position();
	
			anchor.transform_mut().set_position(current_position + math::plane_navigation(camera_orientation, *v));
		});
	}

	{
		let camera = camera.clone();

		// Subscribe to the lookaround action
		// TODO: update orientation before keypress in engine
		lookaround_action_handle.write_sync().value_mut().add(move |v| {
			let mut camera = camera.write_sync();
	
			camera.set_orientation(*v);
		});
	}

	{
		let anchor = anchor.clone();

		// Subscribe to the jump action
		jump_action_handle.write_sync().value_mut().add(move |v| {
			if *v {
				let mut anchor = anchor.write_sync();
	
				let current_position = anchor.transform().get_position();
	
				anchor.transform_mut().set_position(current_position + Vector3::new(0.0, 1.0, 0.0));
			}
		});
	}

	// Create the floor
	let _floor: EntityHandle<Mesh> = runtime.block_on(core::spawn_as_child(space_handle.clone(), Mesh::new("Box.glb", Transform::identity().position(Vector3::new(0.0, -0.5, 1.0)).scale(Vector3::new(15.0, 1.0, 15.0)))));

	let _a: EntityHandle<Mesh> = runtime.block_on(core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne.gltf", Transform::default().position(Vector3::new(0.0, 0.5, 1.0)).scale(Vector3::new(0.4, 0.4, 0.4)))));
	let _a: EntityHandle<Mesh> = runtime.block_on(core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne.gltf", Transform::default().position(Vector3::new(-3.5, 0.5, 4.0)).scale(Vector3::new(0.4, 0.4, 0.4)))));
	let _a: EntityHandle<Mesh> = runtime.block_on(core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne.gltf", Transform::default().position(Vector3::new(3.0, 0.5, 7.5)).scale(Vector3::new(0.4, 0.4, 0.4)))));
	let _a: EntityHandle<Mesh> = runtime.block_on(core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne.gltf", Transform::default().position(Vector3::new(2.75, 0.5, -3.0)).scale(Vector3::new(0.4, 0.4, 0.4)))));

	app.do_loop()
}