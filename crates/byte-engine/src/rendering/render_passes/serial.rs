use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Viewport};
use utils::Box;

pub struct SerialRenderPass {
	render_passes: Vec<Box<dyn RenderPass>>,
}

impl SerialRenderPass {
	pub fn new() -> Self {
		Self {
			render_passes: Vec::new(),
		}
	}

	pub fn add(&mut self, render_pass: Box<dyn RenderPass>) {
		self.render_passes.push(render_pass);
	}
}

impl RenderPass for SerialRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let mut commands: Vec<RenderPassReturn> = Vec::new();

		for render_pass in &mut self.render_passes {
			if let Some(command) = render_pass.prepare(frame, viewport) {
				commands.push(command);
			}
		}

		if commands.is_empty() {
			None
		} else {
			Some(Box::new(move |command_buffer, attachments| {
				for command in &commands {
					command(command_buffer, attachments);
				}
			}))
		}
	}
}
