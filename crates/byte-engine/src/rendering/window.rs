use utils::Extent;

use crate::{camera::Camera, core::{Entity, EntityHandle}};

#[derive(Clone)]
pub struct Window {
	name: String,
	extent: Extent,
	camera: Option<EntityHandle<Camera>>,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Self {
		Window {
			name: name.to_string(),
			extent,
			camera: None,
		}
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}

	pub fn attach(&mut self, camera: EntityHandle<Camera>) {
		self.camera = Some(camera);
	}

	pub fn camera(&self) -> Option<&EntityHandle<Camera>> {
		self.camera.as_ref()
	}
}

impl Entity for Window {}
