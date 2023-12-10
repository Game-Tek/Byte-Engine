use super::render_system;

pub trait WorldRenderDomain {
	fn get_descriptor_set_template(&self) -> render_system::DescriptorSetTemplateHandle;
	fn get_result_image(&self) -> render_system::ImageHandle;
}