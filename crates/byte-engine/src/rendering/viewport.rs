use math::Matrix4;
use utils::Extent;

use crate::rendering::view::View;

/// The `Viewport` is essentially a view with an extent associated to it. Which allows mapping it to a render surface.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
	view: View,
	extent: Extent,
	index: usize,
}

impl Viewport {
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self {
			view, extent, index,
		}
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}

	pub fn view_projection(&self) -> Matrix4 {
		self.view.view_projection()
	}

	pub fn index(&self) -> usize {
		self.index
	}
}
