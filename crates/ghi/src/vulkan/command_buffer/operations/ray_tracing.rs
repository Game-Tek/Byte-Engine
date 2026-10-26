use super::*;

impl crate::command_buffer::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _: crate::ShaderBindingTableHandle, _: ::utils::Extent) {
		crate::rt::unsupported("Vulkan")
	}
}
