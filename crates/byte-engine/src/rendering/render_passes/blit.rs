use ghi::command_buffer::{CommandBufferRecording as _, CommonCommandBufferMode as _};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		render_pass::{simple_compute, RenderPassBuilder, RenderPassReturn},
		RenderPass, Sink,
	},
};

struct BlitPass {
	source: ghi::BaseImageHandle,
	destination: ghi::BaseImageHandle,
}

impl BlitPass {
	pub fn new(source_image: ghi::BaseImageHandle, destination_image: ghi::BaseImageHandle) -> Self {
		BlitPass {
			source: source_image,
			destination: destination_image,
		}
	}
}

impl RenderPass for BlitPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let source = self.source;
		let destination = self.destination;

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Blit"),
					|command_buffer| {
						command_buffer.blit_image(source, ghi::Layouts::Transfer, destination, ghi::Layouts::Transfer);
					},
				);
			},
		))
	}
}

#[derive(Clone)]
pub struct BaseSwapchainBlitPass {
	pipeline: simple_compute::Pipeline,
}

impl Entity for BaseSwapchainBlitPass {}

impl BaseSwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Swapchain Blit",
				"byte-engine/rendering/blit/swapchain",
				"Swapchain Blit Compute Shader",
				create_swapchain_blit_program(),
				Extent::square(32),
			)
			.generation_name("Swapchain Blit")
			.layout_name("Swapchain Blit Pass Set Layout"),
		)
		.expect("Failed to create swapchain blit shader");

		Self { pipeline }
	}
}

fn create_swapchain_blit_program() -> besl::NodeReference {
	let mut program = simple_compute::Program::new();
	program.binding(
		"source",
		besl::BindingTypes::Image {
			format: "rgba16".to_string(),
		},
		0,
		true,
		false,
	);
	program.binding(
		"result",
		besl::BindingTypes::Image {
			format: "unknown".to_string(),
		},
		1,
		false,
		true,
	);
	program
		.compile(SWAPCHAIN_BLIT_SHADER)
		.expect("Failed to compile the swapchain blit shader. The most likely cause is invalid BESL syntax.")
}

const SWAPCHAIN_BLIT_SHADER: &str = r#"
main: fn() -> void {
	let coord: vec2u = thread_id();
	let source_color: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);

	guard_image_bounds(source, coord);
	source_color = image_load(source, coord);
	write(result, coord, source_color);
}
"#;

pub struct SwapchainBlitPass {
	render_pass: simple_compute::Pass,
}

impl SwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let base = BaseSwapchainBlitPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_swapchain = render_pass_builder.render_to_swapchain();
		let render_pass = base
			.pipeline
			.bind(
				render_pass_builder,
				"Swapchain Blit Pass Descriptor Set",
				&[
					simple_compute::Resource::image("source", read_from_main),
					simple_compute::Resource::swapchain("result", render_to_swapchain),
				],
			)
			.expect(
				"Failed to bind swapchain blit resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);

		Self { render_pass }
	}
}

impl Entity for SwapchainBlitPass {}

impl RenderPass for SwapchainBlitPass {
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
	#[cfg(target_os = "linux")]
	use resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator;
	use resource_management::shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator, generator::ShaderGenerationSettings,
	};
	use utils::Extent;

	use super::{create_swapchain_blit_program, SWAPCHAIN_BLIT_SHADER};
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	/// Verifies exact production blits and the dispatch guard through the VM.
	#[test]
	fn swapchain_blit_besl_vm_copies_pixels_and_ignores_out_of_bounds_invocations() {
		let program = crate::rendering::shader_vm_test::compile(create_swapchain_blit_program());
		let expected = [
			[0.1, 0.2, 0.3, 0.4],
			[0.5, 0.6, 0.7, 0.8],
			[0.9, 0.8, 0.7, 0.6],
			[0.4, 0.3, 0.2, 0.1],
		];
		let mut source = texture_2d(2, 2, &expected);
		let mut result = empty_image(2, 2);

		for y in 0..2 {
			for x in 0..2 {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_image(ResourceSlot::new(0), &mut source);
				descriptors.bind_image(ResourceSlot::new(1), &mut result);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}

		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}

		// Dispatch rounding may produce excess invocations, so the production guard must make those invocations true no-ops.
		for coordinate in [[2, 0], [0, 2]] {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_image(ResourceSlot::new(0), &mut source);
			descriptors.bind_image(ResourceSlot::new(1), &mut result);
			run_at(&program, &mut descriptors, coordinate);
		}
		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}
	}

	#[test]
	fn swapchain_blit_besl_parses() {
		besl::parse(SWAPCHAIN_BLIT_SHADER)
			.expect("Failed to parse the swapchain blit BESL shader. The most likely cause is invalid BESL source syntax.");
	}

	#[test]
	fn swapchain_blit_besl_generates_glsl() {
		let main_node = create_swapchain_blit_program();
		let shader = GLSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("Swapchain Blit Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the swapchain blit BESL shader GLSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("imageLoad(source"));
		assert!(shader.contains("imageStore(result"));
	}

	#[test]
	fn swapchain_blit_besl_generates_msl() {
		let main_node = create_swapchain_blit_program();
		let shader = MSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("Swapchain Blit Test".to_string()),
				&main_node,
			)
			.expect("Failed to generate the swapchain blit BESL shader MSL. The most likely cause is invalid BESL lowering.");

		assert!(shader.contains("kernel void besl_main"));
		assert!(shader.contains("resources.source.read(coord)"));
		assert!(shader.contains("resources.result.write("));
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn swapchain_blit_besl_compiles_to_spirv() {
		let main_node = create_swapchain_blit_program();
		SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::square(32)).name("Swapchain Blit Test".to_string()),
				&main_node,
			)
			.expect(
				"Failed to compile the swapchain blit BESL shader to SPIR-V. The most likely cause is invalid GLSL emitted from BESL.",
			);
	}
}
