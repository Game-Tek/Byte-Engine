use crate::Extent;

use super::render_system;

struct ScreenSpaceAmbientOcclusionPass {
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	blur_x_pipeline: render_system::PipelineHandle,
	blur_y_pipeline: render_system::PipelineHandle,
	descriptor_set_layout: render_system::DescriptorSetTemplateHandle,
	descriptor_set: render_system::DescriptorSetHandle,
	blur_x_descriptor_set: render_system::DescriptorSetHandle,
	blur_y_descriptor_set: render_system::DescriptorSetHandle,
	depth_binding: render_system::DescriptorSetBindingHandle,
	result: render_system::ImageHandle,
	x_blur_target: render_system::ImageHandle,
	y_blur_target: render_system::ImageHandle,

	// Not owned by this render pass
	depth_target: render_system::ImageHandle,
}

impl ScreenSpaceAmbientOcclusionPass {
	fn new(render_system: &mut dyn render_system::RenderSystem, parent_descriptor_set_layout: render_system::DescriptorSetTemplateHandle, depth_target: render_system::ImageHandle) -> ScreenSpaceAmbientOcclusionPass {
		let depth_binding_template = render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE);
		let source_binding_template = render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE);
		let result_binding_template = render_system::DescriptorSetBindingTemplate::new(2, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE);

		let descriptor_set_layout = render_system.create_descriptor_set_template(Some("HBAO Pass Set Layout"), &[depth_binding_template.clone(), source_binding_template.clone(), result_binding_template.clone()]);

		let pipeline_layout = render_system.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_layout], &[]);

		let descriptor_set = render_system.create_descriptor_set(Some("HBAO Descriptor Set"), &descriptor_set_layout);
		let blur_x_descriptor_set = render_system.create_descriptor_set(Some("HBAO Blur X Descriptor Set"), &descriptor_set_layout);
		let blur_y_descriptor_set = render_system.create_descriptor_set(Some("HBAO Blur Y Descriptor Set"), &descriptor_set_layout);

		let depth_binding = render_system.create_descriptor_binding(descriptor_set, &depth_binding_template);
		let result_binding = render_system.create_descriptor_binding(descriptor_set, &result_binding_template);

		let blur_x_source_binding = render_system.create_descriptor_binding(blur_x_descriptor_set, &source_binding_template);
		let blur_x_result_binding = render_system.create_descriptor_binding(blur_x_descriptor_set, &result_binding_template);

		let blur_y_source_binding = render_system.create_descriptor_binding(blur_y_descriptor_set, &source_binding_template);
		let blur_y_result_binding = render_system.create_descriptor_binding(blur_y_descriptor_set, &result_binding_template);

		let shader = render_system.create_shader(render_system::ShaderSource::GLSL(HBAO_SHADER), render_system::ShaderTypes::Compute,);

		let pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&shader, render_system::ShaderTypes::Compute, vec![]));

		let result = render_system.create_image(Some("HBAO Result"), Extent::new(1920, 1080, 1), render_system::Formats::RGBA16(render_system::Encodings::IEEE754), None, render_system::Uses::Storage | render_system::Uses::Image, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
		let x_blur_target = render_system.create_image(Some("X Blur"), Extent::new(1920, 1080, 1), render_system::Formats::RGBA16(render_system::Encodings::IEEE754), None, render_system::Uses::Storage | render_system::Uses::Image, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
		let y_blur_target = render_system.create_image(Some("Y Blur"), Extent::new(1920, 1080, 1), render_system::Formats::RGBA16(render_system::Encodings::IEEE754), None, render_system::Uses::Storage | render_system::Uses::Image, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

		let sampler = render_system.create_sampler(render_system::FilteringModes::Linear, render_system::FilteringModes::Linear, render_system::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: depth_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: depth_target, sampler_handle: sampler, layout: render_system::Layouts::Read },
			},
			render_system::DescriptorWrite {
				binding_handle: result_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: result, layout: render_system::Layouts::General },
			},
		]);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: blur_x_source_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: result, sampler_handle: sampler, layout: render_system::Layouts::Read },
			},
			render_system::DescriptorWrite {
				binding_handle: blur_x_result_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: x_blur_target, layout: render_system::Layouts::General },
			},
		]);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: blur_y_source_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: x_blur_target, sampler_handle: sampler, layout: render_system::Layouts::Read },
			},
			render_system::DescriptorWrite {
				binding_handle: blur_y_result_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: y_blur_target, layout: render_system::Layouts::General },
			},
			// render_system::DescriptorWrite { // AO Texture
			// 	binding_handle: occlussion_texture_binding,
			// 	array_element: 0,
			// 	descriptor: render_system::Descriptor::CombinedImageSampler{ image_handle: y_blur_target, sampler_handle: sampler, layout: render_system::Layouts::Read },
			// },
		]);

		let blur_shader = render_system.create_shader(render_system::ShaderSource::GLSL(BLUR_SHADER), render_system::ShaderTypes::Compute,);

		let blur_x_pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&blur_shader, render_system::ShaderTypes::Compute, vec![Box::new(render_system::GenericSpecializationMapEntry{ constant_id: 0 as u32, r#type: "vec2f".to_string(), value: [1f32, 0f32,] })]));
		let blur_y_pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&blur_shader, render_system::ShaderTypes::Compute, vec![Box::new(render_system::GenericSpecializationMapEntry{ constant_id: 0 as u32, r#type: "vec2f".to_string(), value: [0f32, 1f32,] })]));

		ScreenSpaceAmbientOcclusionPass {
			pipeline_layout,
			descriptor_set_layout,
			descriptor_set,
			blur_x_descriptor_set,
			blur_y_descriptor_set,
			blur_x_pipeline,
			blur_y_pipeline,
			pipeline,
			depth_binding,
			result,
			x_blur_target,
			y_blur_target,

			depth_target,
		}
	}

	fn render(&self, command_buffer_recording: &mut dyn render_system::CommandBufferRecording) {
		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.depth_target),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.result),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 32, height: 32, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.result),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.x_blur_target),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);
		command_buffer_recording.bind_compute_pipeline(&self.blur_x_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_x_descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 128, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.x_blur_target),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.y_blur_target),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);
		command_buffer_recording.bind_compute_pipeline(&self.blur_y_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_y_descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 128, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });
	}
}

const HBAO_SHADER: &'static str = include_str!("../../assets/engine/shaders/ssao.comp");
const BLUR_SHADER: &'static str = include_str!("../../assets/engine/shaders/blur.comp");