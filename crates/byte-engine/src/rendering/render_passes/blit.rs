use crate::rendering::{
	render_pass::{FramePrepare, RenderPassReturn},
	RenderPass, Viewport,
};
use ghi::command_buffer::{CommandBufferRecording as _, CommonCommandBufferMode as _};
use utils::Box;

struct BlitPass {
	source: ghi::BaseImageHandle,
	destination: ghi::BaseImageHandle,
}

impl BlitPass {
	pub fn new(source_image: ghi::BaseImageHandle, destination_image: ghi::BaseImageHandle) -> Self {
		BlitPass {
			source: source_image,
			destination: destination_image,
		}
	}
}

impl RenderPass for BlitPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let source = self.source;
		let destination = self.destination;

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("Blit", |command_buffer| {
				command_buffer.blit_image(source, ghi::Layouts::Transfer, destination, ghi::Layouts::Transfer);
			});
		}))
	}
}
