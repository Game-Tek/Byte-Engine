use core::EntityHandle;

use utils::Extent;

pub trait RenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>);

	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}
	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent);

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent);
}