use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
	FrameKey,
};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent};

use crate::core::Entity;
use crate::{
	core::EntityHandle,
	rendering::{
		render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassReturn},
		shader_store::{ShaderSourceDefinition, ShaderSourceDescriptor},
		view::View,
		Sink,
	},
};

#[derive(Clone)]
pub struct BaseAcesToneMapPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for BaseAcesToneMapPass {}

impl BaseAcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let descriptor_set_layout = render_pass_builder.context().create_descriptor_set_template(
			Some("Tonemap Pass Set Layout"),
			&[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE],
		);

		let tone_mapping_shader = create_tone_mapping_shader(render_pass_builder);

		let tone_mapping_pipeline =
			render_pass_builder
				.context()
				.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
					&[descriptor_set_layout],
					&[],
					ghi::ShaderParameter::new(&tone_mapping_shader, ghi::ShaderTypes::Compute),
				));

		Self {
			descriptor_set_layout,
			pipeline: tone_mapping_pipeline,
		}
	}
}

fn create_tone_mapping_shader(render_pass_builder: &mut RenderPassBuilder<'_>) -> ghi::ShaderHandle {
	render_pass_builder
		.create_shader(&ShaderSourceDescriptor {
			id: "byte-engine/rendering/aces/tone-mapping",
			name: "ACES Tone Mapping Compute Shader",
			stage: ResourceShaderTypes::Compute,
			source: ShaderSourceDefinition::Besl {
				settings: ShaderGenerationSettings::compute(Extent::square(32)).name("ACES Tonemapping".to_string()),
				main_node: create_tone_mapping_program(),
			},
			interface: material::ShaderInterface {
				workgroup_size: Some((32, 32, 1)),
				bindings: vec![
					material::Binding::new(0, 0, true, false),
					material::Binding::new(0, 1, false, true),
				],
			},
		})
		.expect("Failed to create ACES tone mapping shader. The most likely cause is an incompatible shader interface.")
}

fn create_tone_mapping_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	root.add_child(
		besl::Node::binding(
			"source",
			besl::BindingTypes::Image {
				format: "rgba16".to_string(),
			},
			0,
			0,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		besl::Node::binding(
			"result",
			besl::BindingTypes::Image {
				format: "unknown".to_string(),
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	);

	let program = besl::compile_to_besl(TONE_MAPPING_SHADER, Some(root))
		.expect("Failed to lex the ACES tone mapping shader. The most likely cause is invalid BESL syntax.");
	program.get_main().expect(
		"Failed to find the ACES tone mapping entry point. The most likely cause is that the BESL program did not define main.",
	)
}

pub struct AcesToneMapPass {
	render_pass: BaseAcesToneMapPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl AcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseAcesToneMapPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to_swapchain();

		let context = render_pass_builder.context();

		let descriptor_set =
			context.create_descriptor_set(Some("Tonemap Pass Descriptor Set"), &render_pass.descriptor_set_layout);

		let source_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main),
		);
		let destination_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::swapchain(&DESTINATION_BINDING_TEMPLATE, render_to_main),
		);

		AcesToneMapPass {
			render_pass,
			descriptor_set,
		}
	}
}

impl Entity for AcesToneMapPass {}

impl RenderPass for AcesToneMapPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pipeline = self.render_pass.pipeline;
		let descriptor_set = self.descriptor_set;

		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |c, _| {
				c.region(
					|label| label.write_str("Tonemap"),
					|c| {
						let r = c.bind_compute_pipeline(pipeline);
						r.bind_descriptor_sets(&[descriptor_set]);
						r.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
					},
				);
			},
		))
	}
}

const TONE_MAPPING_SHADER: &str = r#"
aces_narkowicz: fn(color: vec3f) -> vec3f {
	let a: f32 = 2.51;
	let b: f32 = 0.03;
	let c: f32 = 2.43;
	let d: f32 = 0.59;
	let e: f32 = 0.14;
	return clamp((color * (color * a + vec3f(b, b, b))) / (color * (color * c + vec3f(d, d, d)) + vec3f(e, e, e)), vec3f(0.0, 0.0, 0.0), vec3f(1.0, 1.0, 1.0));
}

main: fn() -> void {
	let coord: vec2u = thread_id();
	let source_color: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let result_color: vec3f = vec3f(0.0, 0.0, 0.0);

	guard_image_bounds(source, coord);
	source_color = image_load(source, coord);
	result_color = aces_narkowicz(vec3f(source_color.x, source_color.y, source_color.z));
	result_color = pow(result_color, vec3f(1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2));
	write(result, coord, vec4f(result_color.x, result_color.y, result_color.z, 1.0));
}
"#;

#[cfg(test)]
mod tests {
	use resource_management::shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator, generator::ShaderGenerationSettings,
	};
	use utils::Extent;

	use super::{create_tone_mapping_program, TONE_MAPPING_SHADER};

	#[test]
	fn aces_tonemap_besl_parses() {
		besl::parse(TONE_MAPPING_SHADER)
			.expect("Failed to parse the ACES BESL shader. The most likely cause is invalid BESL source syntax.");
	}

	#[test]
	fn aces_tonemap_besl_generates_glsl() {
		let main_node = create_tone_mapping_program();
		let shader = GLSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("ACES Tonemapping Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the ACES BESL shader GLSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("imageLoad(source, ivec2(coord))"));
		assert!(shader.contains("imageStore(result, ivec2(coord)"));
	}

	#[test]
	fn aces_tonemap_besl_generates_msl() {
		let main_node = create_tone_mapping_program();
		let shader = MSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("ACES Tonemapping Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the ACES BESL shader MSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("kernel void besl_main"));
		assert!(shader.contains("set0.source.read(coord)"));
		assert!(shader.contains("set0.result.write("));
	}
}
