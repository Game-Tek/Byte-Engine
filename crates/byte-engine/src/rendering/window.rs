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
	features: Features,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Self {
		Window {
			name: name.to_string(),
			extent,
			camera: None,
			features: Features::empty(),
		}
	}

	pub fn with_features(self, features: Features) -> Self {
		Self { features, ..self }
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}

	pub fn features(&self) -> Features {
		self.features
	}

	pub fn with_feature(self, feature: Features) -> Self {
		Self {
			features: self.features | feature,
			..self
		}
	}

	pub fn without_feature(self, feature: Features) -> Self {
		Self {
			features: self.features & !feature,
			..self
		}
	}

	pub fn attach(&mut self, camera: Handle) {
		self.camera = Some(camera);
	}

	pub fn camera(&self) -> Option<&Handle> {
		self.camera.as_ref()
	}
}

impl Entity for Window {}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	/// Bit flags for the features of a window.
	pub struct Features : u32 {
		/// The window has decorations (title bar, border, etc.).
		const DECORATIONS = 0b0001;
	}
}
