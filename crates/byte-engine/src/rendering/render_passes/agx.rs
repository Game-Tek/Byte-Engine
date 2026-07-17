use super::tone_map;
use crate::core::Entity;
use crate::rendering::{
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	Sink,
};

const CONFIGURATION: tone_map::Configuration = tone_map::Configuration {
	shader_id: "byte-engine/rendering/agx/tone-mapping",
	shader_name: "AGX Tone Mapping Compute Shader",
	settings_name: "AGX Tonemapping",
	set_layout_name: "AGX Tonemap Pass Set Layout",
	descriptor_set_name: "AGX Tonemap Pass Descriptor Set",
	source: TONE_MAPPING_SHADER,
	shader_error: "Failed to create AGX tone mapping shader",
	syntax_error: "Failed to lex the AGX tone mapping shader. The most likely cause is invalid BESL syntax.",
};

/// The `BaseAgxToneMapPass` struct defines the shared GPU state required for AGX tonemapping.
#[derive(Clone)]
pub struct BaseAgxToneMapPass {
	pipeline: crate::rendering::render_pass::simple_compute::Pipeline,
}

impl Entity for BaseAgxToneMapPass {}

impl BaseAgxToneMapPass {
	/// Creates the shared AGX compute pipeline resources used by per-view tonemap passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		Self {
			pipeline: tone_map::create_pipeline(render_pass_builder, &CONFIGURATION),
		}
	}
}

fn create_tone_mapping_program() -> besl::NodeReference {
	tone_map::create_program(&CONFIGURATION)
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
	render_pass: crate::rendering::render_pass::simple_compute::Pass,
}

impl AgxToneMapPass {
	/// Creates the per-view descriptor bindings for the AGX tonemap pass.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let base = BaseAgxToneMapPass::new(render_pass_builder);
		let render_pass = tone_map::create_pass(render_pass_builder, &base.pipeline, &CONFIGURATION);
		AgxToneMapPass { render_pass }
	}
}

impl Entity for AgxToneMapPass {}

impl RenderPass for AgxToneMapPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.render_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};

	use super::{create_tone_mapping_program, TONE_MAPPING_SHADER};
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	/// Executes the compiled AGX program for one source color.
	fn run_agx_vm(program: &besl::vm::ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
		let mut source = texture_2d(1, 1, &[source_color]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies reference colors, channel ordering, and bounded output through the VM.
	#[test]
	fn agx_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(create_tone_mapping_program());

		assert_rgba_close(run_agx_vm(&program, [0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(
			run_agx_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.5902294, 0.5901361, 0.5901023, 1.0],
			2e-5,
		);

		let warm = run_agx_vm(&program, [1.0, 0.5, 0.25, 0.0]);
		assert!(
			warm[0] > warm[1] && warm[1] > warm[2],
			"Invalid AGX channel ordering. The most likely cause is an incorrect color-space transform: {warm:?}"
		);
		assert!(
			warm.iter()
				.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
			"Invalid AGX VM output. The most likely cause is unstable tone-mapping arithmetic: {warm:?}"
		);
	}

	#[test]
	fn agx_tonemap_besl_parses() {
		besl::parse(TONE_MAPPING_SHADER)
			.expect("Failed to parse the AGX BESL shader. The most likely cause is invalid BESL source syntax.");
	}
}
