//! Creates a Vulkan context's ray tracing pipelines and writes its shader binding tables. Traces read them in
//! `command_buffer/operations/ray_tracing.rs`.

use super::*;
use crate::vulkan::ray_tracing::{
	RayTracingPipelineState, ShaderBindingTable, TableLayout, WrittenTable, fill_shader_binding_table,
};

impl Context {
	/// Creates a shader binding table that holds no records.
	pub(crate) fn create_shader_binding_table_internal(
		&mut self,
		name: Option<&str>,
	) -> graphics_hardware_interface::ShaderBindingTableHandle {
		self.ray_tracing("create a shader binding table");
		self.shader_binding_tables.push(ShaderBindingTable {
			name: name.map(str::to_owned),
			written: None,
		});
		graphics_hardware_interface::ShaderBindingTableHandle::from_index(self.shader_binding_tables.len() as u64 - 1)
	}

	/// Fills a new buffer with one record per entry of `hit_groups` for `pipeline`, and makes it the table's.
	///
	/// The buffer the table had lives on until the frames recorded before this call complete, so their traces keep
	/// their records.
	pub(crate) fn write_shader_binding_table_internal(
		&mut self,
		table: graphics_hardware_interface::ShaderBindingTableHandle,
		pipeline: graphics_hardware_interface::PipelineHandle,
		hit_groups: &[u32],
	) {
		let layout = TableLayout::new(self.ray_tracing("write a shader binding table"), hit_groups.len());
		if table.index() as usize >= self.shader_binding_tables.len() {
			crate::pipelines::ray_tracing::unknown_table_write("Vulkan");
		}
		crate::pipelines::ray_tracing::check_records(
			"Vulkan",
			self.ray_tracing_pipeline_state(pipeline, "write_shader_binding_table")
				.hit_group_count(),
			hit_groups,
		);

		// The GPU reads a record every time a closest hit runs, so the table goes in device memory the CPU can write
		// when the device has it. The table lends the buffer its name for the call.
		let name = std::mem::take(&mut self.shader_binding_tables[table.index() as usize].name);
		let (buffer, allocation, address, pointer) = self.create_dedicated_buffer(
			name.as_deref(),
			layout.size as usize,
			vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::HostAccess::Write,
		);
		self.shader_binding_tables[table.index() as usize].name = name;
		// SAFETY: The buffer is mapped coherent memory of `layout.size` bytes that only this write uses so far.
		let memory = unsafe { std::slice::from_raw_parts_mut(pointer, layout.size as usize) };
		let state = self.ray_tracing_pipeline_state(pipeline, "write_shader_binding_table");
		let (ray_generation, callable) = fill_shader_binding_table(state, &layout, hit_groups, memory, address);

		let written = WrittenTable {
			pipeline,
			buffer: buffer.resource,
			allocation,
			ray_generation,
			callable,
		};
		if let Some(replaced) = self.shader_binding_tables[table.index() as usize].written.replace(written) {
			self.retire_dedicated_buffer(replaced.buffer, replaced.allocation);
		}
	}

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
