use math::Matrix4;
use utils::Extent;

use crate::rendering::view::View;

/// The `Sink` struct represents a per-frame render destination for a resolved camera view.
///
/// A sink exists to keep the view transform, renderable extent, and renderer sink index together
/// while scene managers and render passes prepare work for a specific output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	view: View,
	extent: Extent,
	index: usize,
}

impl Sink {
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self { view, extent, index }
	}

	pub fn view(&self) -> View {
		self.view
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
