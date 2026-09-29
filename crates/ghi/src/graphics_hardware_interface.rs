//! Defines backend-independent handles and resource descriptions for GPU rendering.
//!
//! These types do not require a specific render-pipeline architecture.

mod handles;
mod queue;
mod resources;

pub use handles::*;
pub use queue::*;
pub use resources::*;
#[cfg(test)]
pub(super) mod tests {
	use utils::Extent;

	use super::*;
	use crate::Layouts;

	#[test]
	#[should_panic(expected = "Render-pass attachments use different layer counts")]
	fn render_pass_rejects_mixed_attachment_layer_counts() {
		let target = BaseImageHandle(1);
		let single = AttachmentInformation::new(
			target,
			Layouts::RenderTarget,
			crate::LoadOp::Clear(ClearValue::Depth(0.0)),
			crate::StoreOp::Store,
		);
		let layered = AttachmentInformation::new(
			target,
			Layouts::RenderTarget,
			crate::LoadOp::Clear(ClearValue::Depth(0.0)),
			crate::StoreOp::Store,
		)
		.layers(4);

		AttachmentInformation::render_pass_layer_count(&[single, layered]);
	}

	#[test]
	fn dispatch_extent_rounds_up_partial_groups() {
		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(5, 5, 5));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(2, 2, 2));

		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(3, 3, 3));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(4, 4, 4));
	}
}
