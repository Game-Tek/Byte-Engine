use crate::ghi;

pub trait ToneMapRenderPass {
	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording,);
}