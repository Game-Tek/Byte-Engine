use utils::Extent;

use crate::ghi;

pub trait ToneMapRenderPass where Self: Sized {
	fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, extent: Extent);
}