use crate::ghi;

#[derive(Clone, Copy)]
pub struct VisibilityInfo {
	pub instance_count: u32,
	pub triangle_count: u32,
	pub meshlet_count: u32,
	pub vertex_count: u32,
	pub primitives_count: u32,
}

pub trait WorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle;
	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle;
	fn get_result_image(&self) -> ghi::ImageHandle;
	fn get_view_depth_image(&self) -> ghi::ImageHandle;
	fn get_view_occlusion_image(&self) -> ghi::ImageHandle;
	fn get_visibility_info(&self) -> VisibilityInfo;
}