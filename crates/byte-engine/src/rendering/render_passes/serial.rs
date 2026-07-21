use smallvec::SmallVec;
use utils::Box;

use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Sink};

pub struct SerialRenderPass {
	render_passes: Vec<Box<dyn RenderPass>>,
}

impl Default for SerialRenderPass {
	fn default() -> Self {
		Self::new()
	}
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
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let mut commands: SmallVec<[RenderPassReturn<'a>; 16]> = SmallVec::new();

		for render_pass in &mut self.render_passes {
			if let Some(command) = render_pass.prepare(frame, sink, frame_allocator) {
				commands.push(command);
			}
		}

		if commands.is_empty() {
			None
		} else {
			Some(crate::rendering::render_pass::allocate_render_command(
				frame_allocator,
				move |command_buffer, attachments| {
					for command in &commands {
						command(command_buffer, attachments);
					}
				},
			))
		}
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let mut commands: SmallVec<[RenderPassReturn<'a>; 16]> = SmallVec::new();

		// Preserve each child's forwarding and maintenance work in the same order as the active serial pass.
		for render_pass in &mut self.render_passes {
			if let Some(command) = render_pass.bypass(frame, sink, frame_allocator) {
				commands.push(command);
			}
		}

		if commands.is_empty() {
			None
		} else {
			Some(crate::rendering::render_pass::allocate_render_command(
				frame_allocator,
				move |command_buffer, attachments| {
					for command in &commands {
						command(command_buffer, attachments);
					}
				},
			))
		}
	}
}
