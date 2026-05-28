use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
};
use resource_management::{
	resources::material,
	shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator, generator::ShaderGenerationSettings,
	},
	types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent};

use crate::core::Entity;
use crate::{
	core::EntityHandle,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		view::View,
		Sink,
	},
};

/// The `BaseAgxToneMapPass` struct defines the shared GPU state required for AGX tonemapping.
#[derive(Clone)]
pub struct BaseAgxToneMapPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for BaseAgxToneMapPass {}

impl BaseAgxToneMapPass {
	/// Creates the shared AGX compute pipeline resources used by per-view tonemap passes.
	pub fn new<'a>(render_pass_builder: &'a mut RenderPassBuilder<'_>) -> Self {
		let descriptor_set_layout = render_pass_builder.context().create_descriptor_set_template(
			Some("AGX Tonemap Pass Set Layout"),
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
	let main_node = create_tone_mapping_program();
	let settings = ShaderGenerationSettings::compute(Extent::square(32)).name("AGX Tonemapping".to_string());
	let glsl_source = GLSLShaderGenerator::new()
		.generate(&settings, &main_node)
		.expect("Failed to generate AGX tone mapping GLSL. The most likely cause is invalid BESL syntax.");
	let msl_source = MSLShaderGenerator::new().generate(&settings, &main_node).expect(
		"Failed to generate the AGX MSL shader. The most likely cause is an unsupported BESL construct in the Metal transpiler.",
	);

	render_pass_builder
		.create_shader(&crate::rendering::shader_store::ShaderSourceDescriptor {
			id: "byte-engine/rendering/agx/tone-mapping",
			name: "AGX Tone Mapping Compute Shader",
			stage: ResourceShaderTypes::Compute,
			source: ghi::shader::ShaderSource::Platform {
				glsl: &glsl_source,
				msl: &msl_source,
				msl_entry_point: "besl_main",
			},
			interface: material::ShaderInterface {
				workgroup_size: Some((32, 32, 1)),
				bindings: vec![
					material::Binding::new(0, 0, true, false),
					material::Binding::new(0, 1, false, true),
				],
			},
		})
		.expect("Failed to create AGX tone mapping shader")
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
		.expect("Failed to lex the AGX tone mapping shader. The most likely cause is invalid BESL syntax.");
	program.get_main().expect(
		"Failed to find the AGX tone mapping entry point. The most likely cause is that the BESL program did not define main.",
	)
}

const TONE_MAPPING_SHADER: &str = r#"
splat3: fn(value: f32) -> vec3f {
	return vec3f(value, value, value);
}

linear_srgb_to_linear_rec2020: fn(color: vec3f) -> vec3f {
	let x: f32 = 0.6274 * color.x + 0.3293 * color.y + 0.0433 * color.z;
	let y: f32 = 0.0691 * color.x + 0.9195 * color.y + 0.0113 * color.z;
	let z: f32 = 0.0164 * color.x + 0.0880 * color.y + 0.8956 * color.z;
	return vec3f(x, y, z);
}

linear_rec2020_to_linear_srgb: fn(color: vec3f) -> vec3f {
	let x: f32 = 1.6605 * color.x - 0.5876 * color.y - 0.0728 * color.z;
	let y: f32 = 0.0 - 0.1246 * color.x + 1.1329 * color.y - 0.0083 * color.z;
	let z: f32 = 0.0 - 0.0182 * color.x - 0.1006 * color.y + 1.1187 * color.z;
	return vec3f(x, y, z);
}

agx_inset: fn(color: vec3f) -> vec3f {
	let x: f32 = 0.856627153315983 * color.x + 0.0951212405381588 * color.y + 0.0482516061458583 * color.z;
	let y: f32 = 0.137318972929847 * color.x + 0.761241990602591 * color.y + 0.101439036467562 * color.z;
	let z: f32 = 0.11189821299995 * color.x + 0.0767994186031903 * color.y + 0.811302368396859 * color.z;
	return vec3f(x, y, z);
}

agx_outset: fn(color: vec3f) -> vec3f {
	let x: f32 = 1.1271005818144368 * color.x - 0.11060664309660323 * color.y - 0.016493938717834573 * color.z;
	let y: f32 = 0.0 - 0.1413297634984383 * color.x + 1.157823702216272 * color.y - 0.016493938717834257 * color.z;
	let z: f32 = 0.0 - 0.14132976349843826 * color.x - 0.11060664309660294 * color.y + 1.2519364065950405 * color.z;
	return vec3f(x, y, z);
}

agx: fn(color: vec3f) -> vec3f {
	let agx_min_ev: f32 = 0.0 - 12.47393;
	let agx_max_ev: f32 = 4.026069;
	let x2: vec3f = vec3f(0.0, 0.0, 0.0);
	let x4: vec3f = vec3f(0.0, 0.0, 0.0);
	let term1: vec3f = vec3f(0.0, 0.0, 0.0);
	let term2: vec3f = vec3f(0.0, 0.0, 0.0);
	let term3: vec3f = vec3f(0.0, 0.0, 0.0);
	let term4: vec3f = vec3f(0.0, 0.0, 0.0);
	let term5: vec3f = vec3f(0.0, 0.0, 0.0);
	let term6: vec3f = vec3f(0.0, 0.0, 0.0);
	let term7: vec3f = vec3f(0.0, 0.0, 0.0);

	color = linear_srgb_to_linear_rec2020(color);
	color = agx_inset(color);
	color = max(color, splat3(0.0000000001));
	color = clamp(log2(color), splat3(agx_min_ev), splat3(agx_max_ev));
	color = color - splat3(agx_min_ev);
	color = color / splat3(agx_max_ev - agx_min_ev);
	color = clamp(color, splat3(0.0), splat3(1.0));

	x2 = color * color;
	x4 = x2 * x2;
	term1 = 15.5 * x4 * x2;
	term2 = 40.14 * x4 * color;
	term3 = 31.96 * x4;
	term4 = 6.868 * x2 * color;
	term5 = 0.4298 * x2;
	term6 = 0.1191 * color;
	term7 = splat3(0.00232);
	color = term1 - term2;
	color = color + term3;
	color = color - term4;
	color = color + term5;
	color = color + term6;
	color = color - term7;

	color = agx_outset(color);
	color = pow(max(splat3(0.0), color), splat3(2.2));
	color = linear_rec2020_to_linear_srgb(color);

	return clamp(color, splat3(0.0), splat3(1.0));
}

main: fn() -> void {
	let coord: vec2u = thread_id();
	let source_color: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let result_color: vec3f = vec3f(0.0, 0.0, 0.0);

	guard_image_bounds(source, coord);
	source_color = image_load(source, coord);
	result_color = agx(vec3f(source_color.x, source_color.y, source_color.z));
	write(result, coord, vec4f(result_color.x, result_color.y, result_color.z, 1.0));
}
"#;

/// The `AgxToneMapPass` struct defines a per-view AGX tonemapping pass instance.
pub struct AgxToneMapPass {
	render_pass: BaseAgxToneMapPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl AgxToneMapPass {
	/// Creates the per-view descriptor bindings for the AGX tonemap pass.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseAgxToneMapPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to_swapchain();

		let context = render_pass_builder.context();

		let descriptor_set =
			context.create_descriptor_set(Some("AGX Tonemap Pass Descriptor Set"), &render_pass.descriptor_set_layout);

		let source_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main),
		);
		let destination_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::swapchain(&DESTINATION_BINDING_TEMPLATE, render_to_main),
		);

		AgxToneMapPass {
			render_pass,
			descriptor_set,
		}
	}
}

impl Entity for AgxToneMapPass {}

impl RenderPass for AgxToneMapPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> Option<RenderPassReturn> {
		let pipeline = self.render_pass.pipeline;
		let descriptor_set = self.descriptor_set;

		let extent = sink.extent();

		Some(Box::new(move |c, _| {
			c.region("Tonemap", |c| {
				let r = c.bind_compute_pipeline(pipeline);
				r.bind_descriptor_sets(&[descriptor_set]);
				r.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
			});
		}))
	}
}

#[cfg(test)]
mod tests {
	use resource_management::shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator,
		besl::backends::spirv::SPIRVShaderGenerator, generator::ShaderGenerationSettings,
		msl_shader_compiler::MSLShaderCompiler,
	};
	use utils::Extent;

	use super::{create_tone_mapping_program, TONE_MAPPING_SHADER};

	#[test]
	fn agx_tonemap_besl_parses() {
		besl::parse(TONE_MAPPING_SHADER)
			.expect("Failed to parse the AGX BESL shader. The most likely cause is invalid BESL source syntax.");
	}

	#[test]
	fn agx_tonemap_besl_generates_glsl() {
		let main_node = create_tone_mapping_program();

		let shader = GLSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("AGX Tonemapping Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the AGX BESL shader GLSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("clamp(log2(color)"));
		assert!(shader.contains("uvec2(gl_GlobalInvocationID.xy)"));
		assert!(shader.contains("imageLoad(source, ivec2(coord))"));
	}

	#[test]
	fn agx_tonemap_besl_generates_msl() {
		let main_node = create_tone_mapping_program();

		let shader = MSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("AGX Tonemapping Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the AGX BESL shader MSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("kernel void besl_main"));
		assert!(shader.contains("constant _set0& set0 [[buffer(16)]]"));
		assert!(shader.contains("set0.source.read(coord)"));
		assert!(shader.contains("set0.result.write("));
	}

	#[test]
	fn agx_tonemap_besl_compiles_to_spirv() {
		let main_node = create_tone_mapping_program();

		SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("AGX Tonemapping Test".to_string()),
				&main_node,
			)
			.expect(
				"Failed to compile the AGX BESL shader to SPIR-V. The most likely cause is invalid GLSL emitted from BESL.",
			);
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn agx_tonemap_besl_compiles_to_metal() {
		let main_node = create_tone_mapping_program();

		MSLShaderCompiler::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("AGX Tonemapping Test".to_string()),
				&main_node,
			)
			.expect("Failed to compile the AGX BESL shader to Metal. The most likely cause is invalid MSL emitted from BESL.");
	}
}
