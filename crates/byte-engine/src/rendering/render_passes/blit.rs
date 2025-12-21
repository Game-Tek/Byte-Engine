use ghi::command_buffer::{CommandBufferRecordable as _, CommonCommandBufferMode as _};
use utils::{Box};
use crate::rendering::{RenderPass, Viewport, render_pass::{RenderPassCommand, RenderPassView}};

pub struct BlitPass {
}

impl BlitPass {
	pub fn new() -> Self {
		BlitPass {
		}
	}
}

impl RenderPass for BlitPass {
	fn prepare(&mut self, frame: &mut ghi::Frame) -> Option<RenderPassCommand> {
	}
}

struct BlitPassView {
	source: ghi::ImageHandle,
	destination: ghi::ImageHandle,
}

impl BlitPassView {
	pub fn new(source_image: ghi::ImageHandle, destination_image: ghi::ImageHandle) -> Self {
		BlitPassView {
			source: source_image,
			destination: destination_image,
		}
	}
}

impl RenderPassView for BlitPassView {
	fn prepare(&mut self, frame: &mut ghi::Frame) -> Option<RenderPassCommand> {

		let source = self.source;
		let destination = self.destination;

		Some(Box::new(move |command_buffer, viewport, _| {
			command_buffer.region("Blit", |command_buffer| {
				command_buffer.blit_image(source, ghi::Layouts::Transfer, destination, ghi::Layouts::Transfer);
			});
		}))
	}
}
