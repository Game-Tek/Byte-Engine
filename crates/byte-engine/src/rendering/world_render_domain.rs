use crate::ghi;

#[derive(Clone, Copy)]
/// The `VisibilityInfo` struct provides scene workload counts used to size visibility dispatches.
pub struct VisibilityInfo {
	pub instance_count: u32,
	pub triangle_count: u32,
	pub meshlet_count: u32,
	pub vertex_count: u32,
	pub primitives_count: u32,
}

/// The `WorldRenderDomain` trait exposes retained scene resources and visibility counts to world render passes.
pub trait WorldRenderDomain {
	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle;
	fn get_visibility_info(&self) -> VisibilityInfo;
}
