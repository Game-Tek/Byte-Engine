use std::sync::Arc;

use besl::ParserNode;
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	device::{Device as _, DeviceCreate as _},
};
use resource_management::{
	asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings,
	spirv_shader_generator::SPIRVShaderGenerator,
};
use utils::{sync::RwLock, Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		common_shader_generator::CommonShaderScope,
		map_shader_binding_to_shader_binding_descriptor,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Viewport,
	},
};

use super::{element::ElementHandle as _, layout::engine};

const MAIN_ATTACHMENT_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;

const UI_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 1] =
	[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0)];

#[derive(Debug, Clone, Copy)]
struct UiDrawElement {
	position: [f32; 2],
	size: [f32; 2],
	color: [f32; 4],
}

#[derive(Debug, Clone)]
struct UiDrawList {
	layout_size: [f32; 2],
	elements: Vec<UiDrawElement>,
}

impl Default for UiDrawList {
	fn default() -> Self {
		Self {
			layout_size: [1.0, 1.0],
			elements: Vec::new(),
		}
	}
}

/// The `UiRenderData` struct stores the layout data that the UI pass consumes for rendering.
#[derive(Clone)]
pub struct UiRenderData {
	data: Arc<RwLock<UiDrawList>>,
}

impl UiRenderData {
	pub fn new() -> Self {
		Self {
			data: Arc::new(RwLock::new(UiDrawList::default())),
		}
	}

	pub fn clear(&self) {
		*self.data.write() = UiDrawList::default();
	}

	/// Converts a layout render tree into an internal draw list for the UI pass.
	pub fn update_from_render(&self, render: &engine::Render<'_>) {
		let root_size = render.root().size;
		let elements = render
			.elements()
			.map(|element| {
				let position = element.position;
				let size = element.size;

				UiDrawElement {
					position: [position.x() as f32, position.y() as f32],
					size: [size.x() as f32, size.y() as f32],
					color: random_color_from_id(element.id),
				}
			})
			.collect();

		*self.data.write() = UiDrawList {
			layout_size: [root_size.x() as f32, root_size.y() as f32],
			elements,
		};
	}

	fn snapshot(&self) -> UiDrawList {
		self.data.read().clone()
	}
}

impl Default for UiRenderData {
	fn default() -> Self {
		Self::new()
	}
}

/// The `UiRenderPass` struct renders UI layout rectangles to the main render target.
pub struct UiRenderPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	quad_mesh: ghi::MeshHandle,
	main_attachment: ghi::ImageHandle,
	data: UiRenderData,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout rectangles.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, data: UiRenderData) -> Self {
		let main_attachment: ghi::ImageHandle = render_pass_builder.render_to("main").into();
		let device = render_pass_builder.device();

		let vertex_shader = create_vertex_shader(device);
		let fragment_shader = create_fragment_shader(device);

		let push_constant_size = std::mem::size_of::<UiPushConstants>() as u32;
		let pipeline_layout =
			device.create_pipeline_layout(&[], &[ghi::pipelines::PushConstantRange::new(0, push_constant_size)]);

		let shaders = [
			ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(MAIN_ATTACHMENT_FORMAT)];

		let pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			pipeline_layout,
			&UI_VERTEX_LAYOUT,
			&shaders,
			&attachments,
		));

		let quad_mesh = create_quad_mesh(device);

		Self {
			pipeline_layout,
			pipeline,
			quad_mesh,
			main_attachment,
			data,
		}
	}
}

impl RenderPass for UiRenderPass {
	fn prepare(&mut self, _frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let draw_list = self.data.snapshot();

		if draw_list.elements.is_empty() {
			return None;
		}

		let pipeline_layout = self.pipeline_layout;
		let pipeline = self.pipeline;
		let quad_mesh = self.quad_mesh;
		let main_attachment = self.main_attachment;

		let extent = viewport.extent();
		let layout_size = draw_list.layout_size;
		let elements = draw_list.elements;

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("UI", |command_buffer| {
				let attachments = [ghi::AttachmentInformation::new(
					main_attachment,
					MAIN_ATTACHMENT_FORMAT,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::None,
					true,
					true,
				)];

				let command_buffer = command_buffer.start_render_pass(extent, &attachments);
				let command_buffer = command_buffer.bind_pipeline_layout(pipeline_layout);
				let command_buffer = command_buffer.bind_raster_pipeline(pipeline);

				for element in &elements {
					let push_constants = UiPushConstants::new(*element, layout_size, extent);
					command_buffer.write_push_constant(0, push_constants);
					command_buffer.draw_mesh(&quad_mesh);
				}

				command_buffer.end_render_pass();
			});
		}))
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UiPushConstants {
	rect: [f32; 4],
	color: [f32; 4],
	viewport: [f32; 4],
}

impl UiPushConstants {
	fn new(element: UiDrawElement, layout_size: [f32; 2], viewport: Extent) -> Self {
		let viewport_width = viewport.width() as f32;
		let viewport_height = viewport.height() as f32;

		let sx = viewport_width / layout_size[0].max(1.0);
		let sy = viewport_height / layout_size[1].max(1.0);

		let rect = [
			element.position[0] * sx,
			element.position[1] * sy,
			element.size[0] * sx,
			element.size[1] * sy,
		];

		Self {
			rect,
			color: element.color,
			viewport: [viewport_width, viewport_height, 0.0, 0.0],
		}
	}
}

/// Creates a reusable quad mesh in local [0, 1] space for UI rectangle rendering.
fn create_quad_mesh(device: &mut ghi::implementation::Device) -> ghi::MeshHandle {
	let vertices: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
	let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];

	unsafe {
		device.add_mesh_from_vertices_and_indices(
			4,
			6,
			std::slice::from_raw_parts(vertices.as_ptr() as *const u8, std::mem::size_of_val(&vertices)),
			std::slice::from_raw_parts(indices.as_ptr() as *const u8, std::mem::size_of_val(&indices)),
			&UI_VERTEX_LAYOUT,
		)
	}
}

/// Builds the UI vertex shader using BESL and compiles it to SPIR-V.
fn create_vertex_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	let mut shader_generator = SPIRVShaderGenerator::new();
	let mut root = ParserNode::root();

	let main_code = r#"
		vec2 pixel = push_constant.rect.xy + in_position * push_constant.rect.zw;

		float x = (pixel.x / push_constant.viewport.x) * 2.0 - 1.0;
		float y = 1.0 - (pixel.y / push_constant.viewport.y) * 2.0;

		gl_Position = vec4(x, y, 0.0, 1.0);
		out_color = push_constant.color;
	"#
	.trim();

	let main = ParserNode::main_function(vec![ParserNode::glsl(
		main_code,
		&["push_constant", "in_position", "out_color"],
		&[],
	)]);
	let push_constant = ParserNode::push_constant(vec![
		ParserNode::member("rect", "vec4f"),
		ParserNode::member("color", "vec4f"),
		ParserNode::member("viewport", "vec4f"),
	]);
	let position_input = ParserNode::input("in_position", "vec2f", 0);
	let color_output = ParserNode::output("out_color", "vec4f", 0);

	let shader_scope = ParserNode::scope("Shader", vec![push_constant, position_input, color_output, main]);
	root.add(vec![CommonShaderScope::new(), shader_scope]);

	let root_node = besl::lex(root).expect("Failed to lex the UI vertex shader. The most likely cause is invalid BESL syntax.");
	let main_node = root_node.get_main().expect(
		"Failed to find the UI vertex entry point. The most likely cause is that the shader main function was not generated.",
	);
	let generated = shader_generator
		.generate(&ShaderGenerationSettings::vertex(), &main_node)
		.expect("Failed to generate UI vertex shader SPIR-V. The most likely cause is invalid GLSL emitted from BESL.");

	device
		.create_shader(
			Some("UI Vertex Shader"),
			ghi::shader::Sources::SPIRV(generated.binary()),
			ghi::ShaderTypes::Vertex,
			generated
				.bindings()
				.iter()
				.map(map_shader_binding_to_shader_binding_descriptor),
		)
		.expect("Failed to create the UI vertex shader. The most likely cause is an incompatible shader interface.")
}

/// Builds the UI fragment shader using BESL and compiles it to SPIR-V.
fn create_fragment_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	let mut shader_generator = SPIRVShaderGenerator::new();
	let mut root = ParserNode::root();

	let main_code = "out_color_attachment = in_color;";
	let main = ParserNode::main_function(vec![ParserNode::glsl(main_code, &["in_color", "out_color_attachment"], &[])]);
	let input_color = ParserNode::input("in_color", "vec4f", 0);
	let output_color = ParserNode::output("out_color_attachment", "vec4f", 0);

	let shader_scope = ParserNode::scope("Shader", vec![input_color, output_color, main]);
	root.add(vec![CommonShaderScope::new(), shader_scope]);

	let root_node =
		besl::lex(root).expect("Failed to lex the UI fragment shader. The most likely cause is invalid BESL syntax.");
	let main_node = root_node.get_main().expect(
		"Failed to find the UI fragment entry point. The most likely cause is that the shader main function was not generated.",
	);
	let generated = shader_generator
		.generate(&ShaderGenerationSettings::fragment(), &main_node)
		.expect("Failed to generate UI fragment shader SPIR-V. The most likely cause is invalid GLSL emitted from BESL.");

	device
		.create_shader(
			Some("UI Fragment Shader"),
			ghi::shader::Sources::SPIRV(generated.binary()),
			ghi::ShaderTypes::Fragment,
			generated
				.bindings()
				.iter()
				.map(map_shader_binding_to_shader_binding_descriptor),
		)
		.expect("Failed to create the UI fragment shader. The most likely cause is an incompatible shader interface.")
}

fn random_color_from_id(id: u32) -> [f32; 4] {
	let mut state = id.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
	state ^= state >> 16;
	state = state.wrapping_mul(2_246_822_519);
	state ^= state >> 13;

	let r = ((state & 0xFF) as f32) / 255.0;
	let g = (((state >> 8) & 0xFF) as f32) / 255.0;
	let b = (((state >> 16) & 0xFF) as f32) / 255.0;

	[0.25 + r * 0.75, 0.25 + g * 0.75, 0.25 + b * 0.75, 1.0]
}

#[cfg(test)]
mod tests {
	use super::UiRenderData;
	use crate::ui::{
		components::container::{BaseContainer, ContainerSettings},
		layout::engine::{Component, Context, Engine},
	};

	struct TestComponent;

	impl Component for TestComponent {
		fn render(&self, ctx: &mut impl Context) {
			let mut ctx = ctx.element(&BaseContainer::new(ContainerSettings::default()));
			ctx.element(&BaseContainer::new(ContainerSettings::default().size(64.into())));
		}
	}

	#[test]
	fn update_from_render_collects_layout_elements() {
		let mut engine = Engine::new();
		let component = TestComponent;
		let render = engine.render(&component);

		let data = UiRenderData::new();
		data.update_from_render(&render);

		let snapshot = data.snapshot();

		assert_eq!(snapshot.elements.len(), render.size());
		assert_eq!(snapshot.layout_size, [1024.0, 1024.0]);
	}
}
