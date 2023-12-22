use crate::{Extent, ghi};

struct ScreenSpaceAmbientOcclusionPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	blur_x_pipeline: ghi::PipelineHandle,
	blur_y_pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	blur_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_y_descriptor_set: ghi::DescriptorSetHandle,
	depth_binding: ghi::DescriptorSetBindingHandle,
	result: ghi::ImageHandle,
	x_blur_target: ghi::ImageHandle,
	y_blur_target: ghi::ImageHandle,

	// Not owned by this render pass
	depth_target: ghi::ImageHandle,
}

impl ScreenSpaceAmbientOcclusionPass {
	fn new(ghi: &mut dyn ghi::GraphicsHardwareInterface, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, depth_target: ghi::ImageHandle) -> ScreenSpaceAmbientOcclusionPass {
		let depth_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let source_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let result_binding_template = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

		let descriptor_set_layout = ghi.create_descriptor_set_template(Some("HBAO Pass Set Layout"), &[depth_binding_template.clone(), source_binding_template.clone(), result_binding_template.clone()]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_layout], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("HBAO Descriptor Set"), &descriptor_set_layout);
		let blur_x_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur X Descriptor Set"), &descriptor_set_layout);
		let blur_y_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur Y Descriptor Set"), &descriptor_set_layout);

		let depth_binding = ghi.create_descriptor_binding(descriptor_set, &depth_binding_template);
		let result_binding = ghi.create_descriptor_binding(descriptor_set, &result_binding_template);

		let blur_x_source_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, &source_binding_template);
		let blur_x_result_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, &result_binding_template);

		let blur_y_source_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, &source_binding_template);
		let blur_y_result_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, &result_binding_template);

		let shader = ghi.create_shader(ghi::ShaderSource::GLSL(HBAO_SHADER), ghi::ShaderTypes::Compute, &[
			ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::WRITE),
		]);

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, (&shader, ghi::ShaderTypes::Compute, vec![]));

		let result = ghi.create_image(Some("HBAO Result"), Extent::new(1920, 1080, 1), ghi::Formats::RGBA16(ghi::Encodings::IEEE754), None, ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
		let x_blur_target = ghi.create_image(Some("X Blur"), Extent::new(1920, 1080, 1), ghi::Formats::RGBA16(ghi::Encodings::IEEE754), None, ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
		let y_blur_target = ghi.create_image(Some("Y Blur"), Extent::new(1920, 1080, 1), ghi::Formats::RGBA16(ghi::Encodings::IEEE754), None, ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		ghi.write(&[
			ghi::DescriptorWrite {
				binding_handle: depth_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: depth_target, sampler_handle: sampler, layout: ghi::Layouts::Read },
			},
			ghi::DescriptorWrite {
				binding_handle: result_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::Image{ handle: result, layout: ghi::Layouts::General },
			},
		]);

		ghi.write(&[
			ghi::DescriptorWrite {
				binding_handle: blur_x_source_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: result, sampler_handle: sampler, layout: ghi::Layouts::Read },
			},
			ghi::DescriptorWrite {
				binding_handle: blur_x_result_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::Image{ handle: x_blur_target, layout: ghi::Layouts::General },
			},
		]);

		ghi.write(&[
			ghi::DescriptorWrite {
				binding_handle: blur_y_source_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: x_blur_target, sampler_handle: sampler, layout: ghi::Layouts::Read },
			},
			ghi::DescriptorWrite {
				binding_handle: blur_y_result_binding,
				array_element: 0,
				descriptor: ghi::Descriptor::Image{ handle: y_blur_target, layout: ghi::Layouts::General },
			},
		]);

		let blur_shader = ghi.create_shader(ghi::ShaderSource::GLSL(BLUR_SHADER), ghi::ShaderTypes::Compute, &[
			ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::WRITE),
		]);

		let blur_x_pipeline = ghi.create_compute_pipeline(&pipeline_layout, (&blur_shader, ghi::ShaderTypes::Compute, vec![Box::new(ghi::GenericSpecializationMapEntry{ constant_id: 0 as u32, r#type: "vec2f".to_string(), value: [1f32, 0f32,] })]));
		let blur_y_pipeline = ghi.create_compute_pipeline(&pipeline_layout, (&blur_shader, ghi::ShaderTypes::Compute, vec![Box::new(ghi::GenericSpecializationMapEntry{ constant_id: 0 as u32, r#type: "vec2f".to_string(), value: [0f32, 1f32,] })]));

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

	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
		command_buffer_recording.bind_compute_pipeline(&self.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent { width: 32, height: 32, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.bind_compute_pipeline(&self.blur_x_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_x_descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent { width: 128, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.bind_compute_pipeline(&self.blur_y_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_y_descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent { width: 128, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });
	}
}

const HBAO_SHADER: &'static str = include_str!("../../assets/engine/shaders/ssao.comp");
const BLUR_SHADER: &'static str = include_str!("../../assets/engine/shaders/blur.comp");