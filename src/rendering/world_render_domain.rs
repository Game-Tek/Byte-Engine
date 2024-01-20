use crate::ghi;

pub trait WorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle;
	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle;
	fn get_result_image(&self) -> ghi::ImageHandle;
}