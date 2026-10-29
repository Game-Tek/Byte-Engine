use super::*;

impl crate::command_buffer::BoundTilePipelineMode for CommandBufferRecording<'_> {
	fn dispatch_tiles(&mut self) {
		crate::pipelines::tile::unsupported("Vulkan")
	}
}
