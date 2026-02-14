use crate::{application::Time, camera::Camera, core::{EntityHandle, channel::DefaultChannel, factory::{CreateMessage, Factory}, listener::DefaultListener}, gameplay::{anchor::AnchorSystem, transform::TransformationUpdate}, physics::{self, dynabit}};

pub struct DefaultWorld {
	body_factory: Factory<EntityHandle<dyn physics::Body>>,
	transforms: (DefaultChannel<TransformationUpdate>, DefaultListener<TransformationUpdate>),
	cameras: (Factory<Camera>, DefaultListener<CreateMessage<Camera>>),

	anchor_system: AnchorSystem,
	physics_system: dynabit::World,
}

impl DefaultWorld {
	pub fn new() -> Self {
		let body_factory = Factory::new();
		let transforms = {
			let channel = DefaultChannel::new();
			let listener = channel.listener();
			(channel, listener)
		};
		let cameras = {
			let factory = Factory::new();
			let listener = factory.listener();
			(factory, listener)
		};

		let anchor_system = AnchorSystem::new();
		let physics_system = dynabit::World::new(body_factory.listener());

		Self {
			body_factory,
			transforms,
			cameras,

			anchor_system,
			physics_system,
		}
	}

	pub fn update(&mut self, time: Time) {
		self.anchor_system.update();
		// self.physics_system.update(time, &mut self.transforms.1);
	}

	pub fn body_factory(&self) -> &Factory<EntityHandle<dyn physics::Body>> {
		&self.body_factory
	}

	pub fn body_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn physics::Body>> {
		&mut self.body_factory
	}

	pub fn transforms(&self) -> &DefaultChannel<TransformationUpdate> {
		&self.transforms.0
	}

	pub fn transforms_mut(&mut self) -> &mut DefaultChannel<TransformationUpdate> {
		&mut self.transforms.0
	}

	pub fn transforms_listener(&self) -> &DefaultListener<TransformationUpdate> {
		&self.transforms.1
	}

	pub fn transforms_listener_mut(&mut self) -> &mut DefaultListener<TransformationUpdate> {
		&mut self.transforms.1
	}

	pub fn camera_factory(&self) -> &Factory<Camera> {
		&self.cameras.0
	}

	pub fn camera_factory_mut(&mut self) -> &mut Factory<Camera> {
		&mut self.cameras.0
	}

	pub fn cameras_listener(&self) -> &DefaultListener<CreateMessage<Camera>> {
		&self.cameras.1
	}

	pub fn cameras_listener_mut(&mut self) -> &mut DefaultListener<CreateMessage<Camera>> {
		&mut self.cameras.1
	}
}
