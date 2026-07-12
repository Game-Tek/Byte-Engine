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
	/// Creates a sink for render passes that target a specific view and extent.
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self { view, extent, index }
	}

	/// Returns the camera or light view used by this render target.
	pub fn view(&self) -> View {
		self.view
	}

	/// Returns the pixel extent available to render passes for this sink.
	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the combined projection and view matrix for shader setup.
	pub fn view_projection(&self) -> Matrix4 {
		self.view.view_projection()
	}

	/// Returns the renderer-local sink index.
	pub fn index(&self) -> usize {
		self.index
	}
}

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::*;

	#[test]
	fn sink_keeps_view_extent_index_and_derived_matrix_consistent() {
		let view = View::new_perspective(
			60.0,
			16.0 / 9.0,
			0.1,
			500.0,
			Vector3::new(0.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
		);
		let extent = Extent::rectangle(1_920, 1_080);
		let sink = Sink::new(view, extent, 3);

		assert_eq!(sink.view(), view);
		assert_eq!(sink.extent(), extent);
		assert_eq!(sink.index(), 3);
		assert_eq!(sink.view_projection(), view.view_projection());
	}
}
