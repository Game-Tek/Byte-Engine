use std::borrow::Borrow as _;

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _};
use utils::Extent;

use crate::rendering::{
	render_pass::{simple_compute, RenderPassBuilder, RenderPassReturn},
	RenderPass, Sink,
};

#[derive(Clone)]
pub struct BaseBilateralBlurPass {
	pipeline_x: simple_compute::Pipeline,
	pipeline_y: simple_compute::Pipeline,
}

impl BaseBilateralBlurPass {
	fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let source_x = bilateral_blur_besl_source((1.0, 0.0));
		let pipeline_x = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Bilateral Blur",
				"byte-engine/rendering/bilateral-blur/x",
				"SSGI Blur X",
				build_bilateral_blur_program(&source_x),
				Extent::line(128),
			)
			.layout_name("SSGI Blur"),
		)
		.expect("Failed to create the X SSGI blur shader. The most likely cause is invalid bilateral blur BESL.");

		let source_y = bilateral_blur_besl_source((0.0, 1.0));
		let pipeline_y = pipeline_x
			.compile_variant(
				render_pass_builder,
				simple_compute::Descriptor::new(
					"Bilateral Blur",
					"byte-engine/rendering/bilateral-blur/y",
					"SSGI Blur Y",
					build_bilateral_blur_program(&source_y),
					Extent::line(128),
				)
				.layout_name("SSGI Blur"),
			)
			.expect("Failed to create the Y SSGI blur shader. The most likely cause is invalid bilateral blur BESL.");

		Self { pipeline_x, pipeline_y }
	}
}

struct BilateralBlurPass {
	pass_x: simple_compute::Pass,
	pass_y: simple_compute::Pass,
}

impl BilateralBlurPass {
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		render_pass: &BaseBilateralBlurPass,
		source: ghi::BaseImageHandle,
	) -> Self {
		let read_depth = render_pass_builder.read_from("depth");
		let depth_image: ghi::BaseImageHandle = (*read_depth.borrow()).into();

		let context = render_pass_builder.context();
		let x_blur_map = context.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let y_blur_map = context.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear),
		);

		let pass_x = render_pass
			.pipeline_x
			.bind(
				render_pass_builder,
				"X SSGI Blur",
				&[
					simple_compute::Resource::combined_image_sampler(
						"depth",
						depth_image,
						depth_sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler("source", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", x_blur_map),
				],
			)
			.expect(
				"Failed to bind X SSGI blur resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);
		let pass_y = render_pass
			.pipeline_y
			.bind(
				render_pass_builder,
				"Y SSGI Blur",
				&[
					simple_compute::Resource::combined_image_sampler(
						"depth",
						depth_image,
						depth_sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler("source", x_blur_map, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", y_blur_map),
				],
			)
			.expect(
				"Failed to bind Y SSGI blur resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);

		Self { pass_x, pass_y }
	}
}

impl RenderPass for BilateralBlurPass {
	fn prepare<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass_x = self.pass_x;
		let pass_y = self.pass_y;
		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Bilateral Blur"),
					|command_buffer| {
						pass_x.record(command_buffer, extent);
						pass_y.record(command_buffer, extent);
					},
				);
			},
		))
	}
}

fn build_bilateral_blur_program(source: &str) -> besl::NodeReference {
	let mut program = simple_compute::Program::new();
	program.binding(
		"depth",
		besl::BindingTypes::CombinedImageSampler { format: String::new() },
		0,
		true,
		false,
	);
	program.binding(
		"source",
		besl::BindingTypes::CombinedImageSampler { format: String::new() },
		1,
		true,
		false,
	);
	program.binding("result", besl::BindingTypes::Image { format: String::new() }, 2, false, true);
	program
		.compile(source)
		.expect("Failed to compile bilateral blur BESL. The most likely cause is invalid BESL syntax.")
}

fn bilateral_blur_besl_source(direction: (f32, f32)) -> String {
	let mut source = String::from(
		r#"main: fn () -> void {
	let coord: vec2u = thread_id();
	guard_image_bounds(result, coord);
	let result_size_u: vec2u = image_size(result);
	let source_size_u: vec2u = texture_size(source);
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5)) / vec2f(f32(result_size_u.x), f32(result_size_u.y));
	let source_size: vec2f = vec2f(f32(source_size_u.x), f32(source_size_u.y));
	let center_depth: f32 = texture_lod(depth, uv).x;
	let center_linear_depth: f32 = (0.1 * 100.0) / (100.0 + center_depth * (0.1 - 100.0));
	let color: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
"#,
	);

	// BESL does not yet expose constant arrays or loops, so keep the fixed blur
	// kernel data in Rust and emit the platform-agnostic taps explicitly.
	for (index, (offset, weight)) in BLUR_TAPS.iter().enumerate() {
		let offset_expression = if *offset < 0.0 {
			format!("0.0 - {:.17}", offset.abs())
		} else {
			format!("{:.17}", offset)
		};
		source.push_str(&format!(
			"\tlet offset_{index}: vec2f = vec2f({:.17}, {:.17}) * ({offset_expression}) / source_size;\n",
			direction.0, direction.1
		));
		source.push_str(&format!(
			"\tlet sample_depth_{index}: f32 = texture_lod(depth, uv + offset_{index}).x;\n"
		));
		source.push_str(&format!(
			"\tlet sample_linear_depth_{index}: f32 = (0.1 * 100.0) / (100.0 + sample_depth_{index} * (0.1 - 100.0));\n"
		));
		source.push_str(&format!(
			"\tlet depth_diff_{index}: f32 = center_linear_depth - sample_linear_depth_{index};\n"
		));
		source.push_str(&format!(
			"\tlet weight_{index}: f32 = {:.17} * (1.0 - step(0.001, abs(depth_diff_{index})));\n",
			weight
		));
		source.push_str(&format!(
			"\tcolor = color + texture_lod(source, uv + offset_{index}) * weight_{index};\n"
		));
	}

	source.push_str(
		r#"
	write(result, coord, vec4f(color.x, color.x, color.x, 1.0));
}
"#,
	);
	source
}

const BLUR_TAPS: [(f32, f32); 17] = [
	(-15.153_611, 6.531_899_4e-7),
	(-13.184_472, 0.000_014_791_299),
	(-11.219_917, 0.000_217_209_86),
	(-9.260_003, 0.002_070_655_8),
	(-7.304_547, 0.012_826_757_5),
	(-5.353_083_6, 0.051_677_145),
	(-3.404_847_1, 0.135_521_1),
	(-1.458_811_2, 0.231_487_84),
	(0.486_242_68, 0.257_646_32),
	(2.431_625_8, 0.186_864_99),
	(4.378_621, 0.088_296_115),
	(6.328_357, 0.027_166_77),
	(8.281_74, 0.005_438_63),
	(10.239_386, 0.000_707_818_7),
	(12.201_613, 0.000_059_830_993),
	(14.168_479, 0.000_003_281_429_8),
	(16.0, 1.003_370_4e-7),
];

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};
	use resource_management::shader::besl::{backends::glsl::GLSLShaderGenerator, backends::msl::MSLShaderGenerator};
	use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator as _};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	/// Executes one generated bilateral program against deterministic texture fixtures.
	fn run_bilateral_vm(
		direction: (f32, f32),
		extent: [u32; 2],
		depth_texels: &[[f32; 4]],
		source_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let source_code = bilateral_blur_besl_source(direction);
		let program = crate::rendering::shader_vm_test::compile(build_bilateral_blur_program(&source_code));
		let mut depth = texture_2d(extent[0], extent[1], depth_texels);
		let mut source = texture_2d(extent[0], extent[1], source_texels);
		let mut result = empty_image(extent[0], extent[1]);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut depth);
		descriptors.bind_texture(ResourceSlot::new(1), &mut source);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		run_at(&program, &mut descriptors, coordinate);
		drop(descriptors);
		rgba(&result, coordinate)
	}

	/// Verifies that both generated blur axes preserve a constant source.
	#[test]
	fn bilateral_blur_besl_vm_preserves_constant_input_in_both_axes() {
		for direction in [(1.0, 0.0), (0.0, 1.0)] {
			let output = run_bilateral_vm(direction, [1, 1], &[[0.5, 0.0, 0.0, 1.0]], &[[0.375, 0.9, 0.1, 0.2]], [0, 0]);
			assert_rgba_close(output, [0.375, 0.375, 0.375, 1.0], 1e-5);
		}
	}

	/// Verifies that depth rejection blocks horizontal edge bleed while the vertical kernel preserves its column.
	#[test]
	fn bilateral_blur_besl_vm_respects_depth_edges_and_direction() {
		let depth = [
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];
		let source = [
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];

		let horizontal = run_bilateral_vm((1.0, 0.0), [3, 3], &depth, &source, [1, 1]);
		let vertical = run_bilateral_vm((0.0, 1.0), [3, 3], &depth, &source, [1, 1]);

		assert_rgba_close(horizontal, [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(vertical, [0.2, 0.2, 0.2, 1.0], 1e-5);
	}

	#[test]
	fn bilateral_blur_besl_lowers_for_both_axes() {
		for (name, direction) in [("x", (1.0, 0.0)), ("y", (0.0, 1.0))] {
			let source = bilateral_blur_besl_source(direction);
			besl::parse(&source).expect("Generated bilateral blur source should parse before lexing.");
			let main_node = build_bilateral_blur_program(&source);
			let settings =
				ShaderGenerationSettings::compute(Extent::new(128, 1, 1)).name(format!("Bilateral Blur {name} Test"));

			GLSLShaderGenerator::new()
				.generate(&settings, &main_node)
				.expect("Failed to lower bilateral blur BESL to GLSL.");
			MSLShaderGenerator::new()
				.generate(&settings, &main_node)
				.expect("Failed to lower bilateral blur BESL to MSL.");
		}
	}
}
