use utils::Extent;

use crate::{
	core::{factory::Handle, Entity, EntityHandle},
	rendering::Camera,
};

#[derive(Clone)]
pub struct Window {
	name: String,
	extent: Extent,
	camera: Option<Handle>,
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

	pub fn attach(&mut self, camera: Handle) {
		self.camera = Some(camera);
	}

	pub fn camera(&self) -> Option<&Handle> {
		self.camera.as_ref()
	}
}

impl Entity for Window {}
