use super::render_system;

pub trait ToneMapRenderPass {
	fn render(&self, command_buffer_recording: &mut dyn render_system::CommandBufferRecording,);
}