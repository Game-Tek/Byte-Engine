//! Creates a Vulkan context's ray tracing pipelines. The `ContextCreate` impl in `traits.rs` writes the shader
//! binding tables, and traces read both in `command_buffer/operations/ray_tracing.rs`.

use super::*;
use crate::vulkan::ray_tracing::RayTracingPipelineState;

impl Context {
	/// Returns what traces need of the ray tracing pipeline `pipeline`, and panics, naming `operation`, when it is
	/// another kind of pipeline.
	pub(crate) fn ray_tracing_pipeline_state(
		&self,
		pipeline: graphics_hardware_interface::PipelineHandle,
		operation: &str,
	) -> &RayTracingPipelineState {
		self.pipelines
			.get(pipeline.index() as usize)
			.and_then(|pipeline| pipeline.ray_tracing.as_deref())
			.unwrap_or_else(|| crate::pipelines::ray_tracing::not_ray_tracing_pipeline("Vulkan", operation))
	}

	/// Destroys the buffer of every shader binding table. Call it only while the context drops, after the device is
	/// idle; the allocations go with the context's other allocations.
	pub(crate) fn destroy_shader_binding_tables(&mut self) {
		for written in self.shader_binding_tables.drain(..).filter_map(|table| table.written) {
			// SAFETY: The device is idle, so no trace reads the table.
			unsafe { self.device.destroy_buffer(written.buffer, None) };
		}
	}
}
