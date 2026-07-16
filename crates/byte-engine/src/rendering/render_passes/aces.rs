use super::tone_map;
use crate::core::Entity;
use crate::rendering::{
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	Sink,
};

const CONFIGURATION: tone_map::Configuration = tone_map::Configuration {
	shader_id: "byte-engine/rendering/aces/tone-mapping",
	shader_name: "ACES Tone Mapping Compute Shader",
	settings_name: "ACES Tonemapping",
	set_layout_name: "Tonemap Pass Set Layout",
	descriptor_set_name: "Tonemap Pass Descriptor Set",
	source: TONE_MAPPING_SHADER,
	shader_error: "Failed to create ACES tone mapping shader. The most likely cause is an incompatible shader interface.",
	syntax_error: "Failed to lex the ACES tone mapping shader. The most likely cause is invalid BESL syntax.",
	entry_point_error:
		"Failed to find the ACES tone mapping entry point. The most likely cause is that the BESL program did not define main.",
};

/// The `BaseAcesToneMapPass` struct provides shared ACES compute pipeline state to per-view passes.
#[derive(Clone)]
pub struct BaseAcesToneMapPass {
	pipeline: tone_map::Pipeline,
}

impl Entity for BaseAcesToneMapPass {}

impl BaseAcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		Self {
			pipeline: tone_map::Pipeline::new(render_pass_builder, &CONFIGURATION),
		}
	}
}

fn create_tone_mapping_program() -> besl::NodeReference {
	tone_map::create_program(&CONFIGURATION)
}

/// The `AcesToneMapPass` struct provides one view with ACES tonemapping descriptor bindings.
pub struct AcesToneMapPass {
	render_pass: BaseAcesToneMapPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl AcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseAcesToneMapPass::new(render_pass_builder);
		let descriptor_set = tone_map::create_descriptor_set(render_pass_builder, &render_pass.pipeline, &CONFIGURATION);

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
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		tone_map::prepare(self.render_pass.pipeline.pipeline, self.descriptor_set, sink, frame_allocator)
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
	use besl::vm::{DescriptorBindings, DescriptorSlot};
	use resource_management::shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator, generator::ShaderGenerationSettings,
	};
	use utils::Extent;

	use super::{create_tone_mapping_program, TONE_MAPPING_SHADER};
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	/// Executes the compiled ACES program for one source color.
	fn run_aces_vm(program: &besl::vm::ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
		let mut source = texture_2d(1, 1, &[source_color]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(DescriptorSlot::new(0, 0), &mut source);
		descriptors.bind_image(DescriptorSlot::new(0, 1), &mut result);
		run_at(program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies reference colors and bounded high-dynamic-range behavior through the VM.
	#[test]
	fn aces_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(create_tone_mapping_program());

		assert_rgba_close(run_aces_vm(&program, [0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(
			run_aces_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.9054924, 0.9054924, 0.9054924, 1.0],
			1e-5,
		);

		for input in [0.18, 4.0, 16.0] {
			let output = run_aces_vm(&program, [input, input, input, 0.0]);
			assert!(
				output[..3]
					.iter()
					.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
				"Invalid ACES VM output. The most likely cause is unstable tone-mapping arithmetic: {output:?}"
			);
		}
	}

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

		assert!(shader.contains("imageLoad(source"));
		assert!(shader.contains("imageStore(result"));
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
