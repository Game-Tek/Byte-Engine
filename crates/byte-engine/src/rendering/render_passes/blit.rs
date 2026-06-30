use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassReturn},
		shader_store::{ShaderSourceDefinition, ShaderSourceDescriptor},
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
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for BaseSwapchainBlitPass {}

impl BaseSwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let descriptor_set_layout = render_pass_builder.context().create_descriptor_set_template(
			Some("Swapchain Blit Pass Set Layout"),
			&[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE],
		);

		let shader = create_swapchain_blit_shader(render_pass_builder);
		let pipeline = render_pass_builder
			.context()
			.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
				&[descriptor_set_layout],
				&[],
				ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
			));

		Self {
			pipeline,
			descriptor_set_layout,
		}
	}
}

fn create_swapchain_blit_shader(render_pass_builder: &mut RenderPassBuilder<'_>) -> ghi::ShaderHandle {
	render_pass_builder
		.create_shader(&ShaderSourceDescriptor {
			id: "byte-engine/rendering/blit/swapchain",
			name: "Swapchain Blit Compute Shader",
			stage: ResourceShaderTypes::Compute,
			source: ShaderSourceDefinition::Besl {
				settings: ShaderGenerationSettings::compute(Extent::square(32)).name("Swapchain Blit".to_string()),
				main_node: create_swapchain_blit_program(),
			},
			interface: material::ShaderInterface {
				workgroup_size: Some((32, 32, 1)),
				bindings: vec![
					material::Binding::new(0, 0, true, false),
					material::Binding::new(0, 1, false, true),
				],
			},
		})
		.expect("Failed to create swapchain blit shader")
}

fn create_swapchain_blit_program() -> besl::NodeReference {
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

	let program = besl::compile_to_besl(SWAPCHAIN_BLIT_SHADER, Some(root))
		.expect("Failed to lex the swapchain blit shader. The most likely cause is invalid BESL syntax.");
	program.get_main().expect(
		"Failed to find the swapchain blit entry point. The most likely cause is that the BESL program did not define main.",
	)
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
	render_pass: BaseSwapchainBlitPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl SwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseSwapchainBlitPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_swapchain = render_pass_builder.render_to_swapchain();

		let context = render_pass_builder.context();

		let descriptor_set =
			context.create_descriptor_set(Some("Swapchain Blit Pass Descriptor Set"), &render_pass.descriptor_set_layout);

		context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main),
		);
		context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::swapchain(&DESTINATION_BINDING_TEMPLATE, render_to_swapchain),
		);

		Self {
			render_pass,
			descriptor_set,
		}
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
		let pipeline = self.render_pass.pipeline;
		let descriptor_set = self.descriptor_set;
		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Swapchain Blit"),
					|command_buffer| {
						let r = command_buffer.bind_compute_pipeline(pipeline);
						r.bind_descriptor_sets(&[descriptor_set]);
						r.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
					},
				);
			},
		))
	}
}

#[cfg(test)]
mod tests {
	use resource_management::shader::{
		besl::backends::glsl::GLSLShaderGenerator, besl::backends::msl::MSLShaderGenerator,
		besl::backends::spirv::SPIRVShaderGenerator, generator::ShaderGenerationSettings,
	};
	use utils::Extent;

	use super::{create_swapchain_blit_program, SWAPCHAIN_BLIT_SHADER};

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

		assert!(shader.contains("imageLoad(source, ivec2(coord))"));
		assert!(shader.contains("imageStore(result, ivec2(coord), source_color)"));
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
		assert!(shader.contains("set0.source.read(coord)"));
		assert!(shader.contains("set0.result.write(source_color, coord)"));
	}

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
