use utils::Extent;

use crate::core::Entity;

pub struct Window {
	name: String,
	extent: Extent,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Self {
		Window {
			name: name.to_string(),
			extent
		}
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}
}

impl Entity for Window {}
