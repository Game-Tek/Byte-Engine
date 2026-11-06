//! Holds what a DX12 ray tracing pipeline and its shader binding tables keep: the state object, the identifiers of its
//! shaders, and the records a trace reads.
//!
//! The context creates them in `context/ray_tracing.rs`. As on Vulkan, rays are found with inline ray queries, so a
//! pipeline holds no miss or hit group shaders: its ray generation shader runs closest hits as callable shaders, by the
//! record of the hit. See the ray tracing design note (`docs/develop/rendering/ray-tracing.mdx`).

use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::Win32::Graphics::Direct3D12::{ID3D12Resource, ID3D12StateObject};

use crate::PipelineHandle;

/// The size of a shader identifier, which is all a record holds, since no shader reads a local root signature.
pub(crate) const IDENTIFIER_SIZE: usize = d3d12::D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize;

/// The offset of the callable region in a table: the ray generation record, then padding to the table alignment.
pub(crate) const CALLABLE_OFFSET: u64 = (IDENTIFIER_SIZE as u64).next_multiple_of(TABLE_ALIGNMENT);

/// The distance between two callable records.
pub(crate) const RECORD_STRIDE: u64 = (IDENTIFIER_SIZE as u64)
	.next_multiple_of(d3d12::D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as u64);

/// The alignment of each region of a table.
const TABLE_ALIGNMENT: u64 = d3d12::D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as u64;

/// The most invocations one trace runs, which D3D12 sets.
pub(crate) const MAX_TRACE_INVOCATIONS: u64 = 1 << 30;

/// The `RayTracingState` struct is what a DX12 ray tracing pipeline holds besides its layout.
pub(crate) struct RayTracingState {
	pub(crate) state_object: ID3D12StateObject,
	pub(crate) ray_generation: [u8; IDENTIFIER_SIZE],
	/// The identifier of each hit group's closest hit, by hit group.
	pub(crate) hit_groups: Vec<[u8; IDENTIFIER_SIZE]>,
}

/// The `ShaderBindingTable` struct is a table the application maps hit records to hit groups with.
pub(crate) struct ShaderBindingTable {
	pub(crate) name: Option<String>,
	pub(crate) written: Option<WrittenTable>,
}

/// The `WrittenTable` struct is what the last write of a table left: the records, the pipeline they were written for,
/// and the regions a trace passes.
pub(crate) struct WrittenTable {
	pub(crate) pipeline: PipelineHandle,
	/// An upload buffer the GPU reads the records from. Each write makes a new one, and recordings that traced with the
	/// old one keep it alive.
	pub(crate) resource: ID3D12Resource,
	pub(crate) ray_generation: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
	pub(crate) callable: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
}

/// Returns the size of a table with `record_count` callable records.
pub(crate) fn table_size(record_count: usize) -> u64 {
	CALLABLE_OFFSET + RECORD_STRIDE * record_count.max(1) as u64
}

/// Panics unless every record of `hit_groups` names one of the `hit_group_count` hit groups of the pipeline.
pub(crate) fn check_hit_groups(hit_group_count: usize, hit_groups: &[u32]) {
	for (record, &group) in hit_groups.iter().enumerate() {
		assert!(
			(group as usize) < hit_group_count,
			"DX12 shader binding table write failed because record {record} names hit group {group}, and the pipeline has {hit_group_count} hit groups. The most likely cause is that the records were made for another version of the pipeline, with more hit groups. Write the table for the pipeline whose hit groups the records name."
		);
	}
}

/// Writes the records of a table into `memory`, which holds [`table_size`] bytes: the ray generation shader's
/// identifier, then the closest hit of `hit_groups[i]` at callable record `i`.
pub(crate) fn fill_shader_binding_table(state: &RayTracingState, hit_groups: &[u32], memory: &mut [u8]) {
	memory[..IDENTIFIER_SIZE].copy_from_slice(&state.ray_generation);
	for (record, &group) in hit_groups.iter().enumerate() {
		let offset = (CALLABLE_OFFSET + RECORD_STRIDE * record as u64) as usize;
		memory[offset..offset + IDENTIFIER_SIZE].copy_from_slice(&state.hit_groups[group as usize]);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn records_follow_the_ray_generation_record_at_the_table_alignment() {
		assert_eq!(CALLABLE_OFFSET, 64);
		assert_eq!(RECORD_STRIDE, 32);
		assert_eq!(table_size(3), 64 + 3 * 32);
	}
}
