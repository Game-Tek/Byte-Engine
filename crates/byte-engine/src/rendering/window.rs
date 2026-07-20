use utils::Extent;

use crate::core::{factory::Handle, Entity};

#[derive(Clone)]
/// The `Window` struct configures a named render surface and its attached camera.
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{core::factory::Factory, rendering::Camera};

	#[test]
	fn feature_builders_compose_and_remove_only_requested_flags() {
		let window = Window::new("Main", Extent::rectangle(1_920, 1_080));
		assert_eq!(window.name(), "Main");
		assert_eq!(window.extent(), Extent::rectangle(1_920, 1_080));
		assert!(window.features().is_empty());

		let decorated = window.with_feature(Features::DECORATIONS);
		assert!(decorated.features().contains(Features::DECORATIONS));
		let undecorated = decorated.without_feature(Features::DECORATIONS);
		assert!(undecorated.features().is_empty());

		let replaced = undecorated.with_features(Features::DECORATIONS);
		assert_eq!(replaced.features(), Features::DECORATIONS);
	}

	#[test]
	fn camera_attachment_preserves_the_factory_identity_across_clones() {
		let mut factory = Factory::new();
		let camera = factory.create(Camera::new());
		let mut window = Window::new("View", Extent::rectangle(800, 600));

		assert!(window.camera().is_none());
		window.attach(camera);
		assert_eq!(window.camera(), Some(&camera));
		assert_eq!(window.clone().camera(), Some(&camera));
	}
}
