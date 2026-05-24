use crate::{
	application::Time,
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Factory},
		listener::{DefaultListener, Listener},
		EntityHandle,
	},
	gameplay::{anchor::AnchorSystem, transform::TransformationUpdate},
	physics::{self, dynabit},
	rendering::{lights::Lights, Camera, RenderableMesh},
};

#[derive(Clone)]
pub struct DefaultWorld {
	body_factory: Factory<EntityHandle<dyn physics::Body>>,
	transforms: DefaultChannel<TransformationUpdate>,
	cameras: Factory<Camera>,
	renderable_factory: Factory<EntityHandle<dyn RenderableMesh>>,
	light_factory: Factory<Lights>,

	anchor_system: AnchorSystem,
	physics_system: dynabit::World,
}

impl DefaultWorld {
	pub fn new() -> Self {
		let body_factory = Factory::new();
		let transforms = DefaultChannel::new();
		let cameras = Factory::new();
		let renderable_factory = Factory::new();

		let anchor_system = AnchorSystem::new();
		let physics_system = dynabit::World::new(body_factory.listener());

		Self {
			body_factory,
			transforms,
			cameras,
			renderable_factory,
			light_factory: Factory::new(),

			anchor_system,
			physics_system,
		}
	}

	pub fn update(&mut self, time: Time, transforms_rx: &mut impl Listener<TransformationUpdate>) {
		self.anchor_system.update();
		self.physics_system.update(time, transforms_rx, &mut self.transforms);
	}

	pub fn body_factory(&self) -> &Factory<EntityHandle<dyn physics::Body>> {
		&self.body_factory
	}

	pub fn body_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn physics::Body>> {
		&mut self.body_factory
	}

	pub fn transforms_channel(&self) -> &DefaultChannel<TransformationUpdate> {
		&self.transforms
	}

	pub fn transforms_channel_mut(&mut self) -> &mut DefaultChannel<TransformationUpdate> {
		&mut self.transforms
	}

	pub fn renderable_factory(&self) -> &Factory<EntityHandle<dyn RenderableMesh>> {
		&self.renderable_factory
	}

	pub fn renderable_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn RenderableMesh>> {
		&mut self.renderable_factory
	}

	pub fn light_factory(&self) -> &Factory<Lights> {
		&self.light_factory
	}

	pub fn light_factory_mut(&mut self) -> &mut Factory<Lights> {
		&mut self.light_factory
	}

	pub fn camera_factory(&self) -> &Factory<Camera> {
		&self.cameras
	}

	pub fn camera_factory_mut(&mut self) -> &mut Factory<Camera> {
		&mut self.cameras
	}
}
