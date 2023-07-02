use byte_engine::{self, application::Application, Vec3f, Vector2, input_manager, Vector3, orchestrator::{Orchestrator, ComponentHandle, Component}, mesh::Mesh};

#[ignore]
#[test]
fn gallery_shooter() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	let input_system = app.get_input_system_handle();

	let lookaround_action = app.get_orchestrator().get_mut_and(&input_system, |input_system| {
		byte_engine::input_manager::Action::<Vector3>::new(app.get_orchestrator(), input_system, "Lookaround", &[
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.Position")),
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Gamepad.RightStick")),
		])
	});

	let camera = byte_engine::camera::Camera::new(app.get_mut_orchestrator(), Vec3f::new(0.0, 0.0, 0.0),90.0,);

	app.get_mut_orchestrator().tie(&camera, byte_engine::camera::Camera::orientation, &lookaround_action, input_manager::Action::value);

	let trigger_action = app.get_orchestrator().get_mut_and(&input_system, |input_system| {
		byte_engine::input_manager::Action::<bool>::new(app.get_orchestrator(), input_system, "Trigger", &[
			input_manager::ActionBindingDescription::new(input_manager::InputSourceAction::Name("Mouse.LeftButton"))
		])		
	});

	//app.get_orchestrator().tie(&weapon, byte_engine::camera::Weapon::trigger, &trigger_action, byte_engine::input_manager::Action::value);

	let weapon = Weapon::new(app.get_mut_orchestrator());

	app.do_loop();

	app.deinitialize();
}

struct Weapon {
	mesh: ComponentHandle<Mesh>,
}

impl Component<Weapon> for Weapon {
	fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Weapon> {
		let weapon = Self {
			mesh: Mesh::new(orchestrator, "cube"),
		};

		orchestrator.make_object(weapon)
	}
}

impl Weapon {
	pub fn fire(&self) {

	}
}