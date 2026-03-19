use besl::ParserNode;
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
};
use resource_management::{shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{Box, Extent};

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

const UI_VERTICES_PER_ELEMENT: usize = 4;
const UI_INDICES_PER_ELEMENT: usize = 6;
const MAX_UI_VERTICES_PER_DRAW: usize = u16::MAX as usize + 1;
const MAX_UI_ELEMENTS: usize = 65_536;
const MAX_UI_VERTICES: usize = MAX_UI_ELEMENTS * UI_VERTICES_PER_ELEMENT;
const MAX_UI_INDICES: usize = MAX_UI_ELEMENTS * UI_INDICES_PER_ELEMENT;

const UI_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 5] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("LOCAL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("RECT_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("COLOR", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("CORNER_RADIUS", ghi::DataTypes::Float, 0),
];

#[derive(Debug, Clone, Copy)]
struct UiDrawElement {
	position: [f32; 2],
	size: [f32; 2],
	color: [f32; 4],
	corner_radius: f32,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct UiVertex {
	position: [f32; 2],
	local_position: [f32; 2],
	rect_size: [f32; 2],
	color: [f32; 4],
	corner_radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiDrawBatch {
	index_count: u32,
	first_index: u32,
	vertex_offset: i32,
}

#[derive(Debug, Default, Clone)]
struct UiGeometry {
	vertices: Vec<UiVertex>,
	indices: Vec<u16>,
	batches: Vec<UiDrawBatch>,
	truncated: bool,
}

fn update_from_render(render: &engine::Render) -> UiDrawList {
	let root_size = render.root().size;
	let elements = render
		.elements()
		.map(|element| {
			let position = element.position;
			let size = element.size;

			UiDrawElement {
				position: [position.x() as f32, position.y() as f32],
				size: [size.x() as f32, size.y() as f32],
				color: element.color.into(),
				corner_radius: element.corner_radius,
			}
		})
		.collect();

	UiDrawList {
		layout_size: [root_size.x() as f32, root_size.y() as f32],
		elements,
	}
}

/// Builds the packed UI geometry for the current viewport and splits it into `u16`-safe draw ranges.
fn build_ui_geometry(draw_list: &UiDrawList, viewport: Extent) -> UiGeometry {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let radius_scale = sx.min(sy);

	let mut geometry = UiGeometry {
		vertices: Vec::with_capacity(draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_VERTICES_PER_ELEMENT),
		indices: Vec::with_capacity(draw_list.elements.len().min(MAX_UI_ELEMENTS) * UI_INDICES_PER_ELEMENT),
		batches: Vec::new(),
		truncated: false,
	};

	let mut batch_first_index = 0usize;
	let mut batch_vertex_offset = 0usize;
	let mut batch_vertex_count = 0usize;
	let mut batch_index_count = 0usize;

	for element in &draw_list.elements {
		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		let rect_width = (element.size[0] * sx).max(0.0);
		let rect_height = (element.size[1] * sy).max(0.0);

		if rect_width <= 0.0 || rect_height <= 0.0 || element.color[3] <= 0.0 {
			continue;
		}

		if batch_vertex_count + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES_PER_DRAW {
			geometry.batches.push(UiDrawBatch {
				index_count: batch_index_count as u32,
				first_index: batch_first_index as u32,
				vertex_offset: batch_vertex_offset as i32,
			});

			batch_first_index = geometry.indices.len();
			batch_vertex_offset = geometry.vertices.len();
			batch_vertex_count = 0;
			batch_index_count = 0;
		}

		let x0 = element.position[0] * sx;
		let y0 = element.position[1] * sy;
		let x1 = x0 + rect_width;
		let y1 = y0 + rect_height;
		let color = element.color;
		let corner_radius = element.corner_radius * radius_scale;

		let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
		let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;

		geometry.vertices.extend_from_slice(&[
			UiVertex {
				position: [to_clip_x(x0), to_clip_y(y0)],
				local_position: [0.0, 0.0],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
			},
			UiVertex {
				position: [to_clip_x(x1), to_clip_y(y0)],
				local_position: [rect_width, 0.0],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
			},
			UiVertex {
				position: [to_clip_x(x1), to_clip_y(y1)],
				local_position: [rect_width, rect_height],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
			},
			UiVertex {
				position: [to_clip_x(x0), to_clip_y(y1)],
				local_position: [0.0, rect_height],
				rect_size: [rect_width, rect_height],
				color,
				corner_radius,
			},
		]);

		let base_vertex = batch_vertex_count as u16;
		geometry.indices.extend_from_slice(&[
			base_vertex,
			base_vertex + 1,
			base_vertex + 2,
			base_vertex + 2,
			base_vertex + 3,
			base_vertex,
		]);

		batch_vertex_count += UI_VERTICES_PER_ELEMENT;
		batch_index_count += UI_INDICES_PER_ELEMENT;
	}

	if batch_index_count > 0 {
		geometry.batches.push(UiDrawBatch {
			index_count: batch_index_count as u32,
			first_index: batch_first_index as u32,
			vertex_offset: batch_vertex_offset as i32,
		});
	}

	geometry
}

/// The `UiRenderPass` struct centralizes batched UI rectangle rendering for the main render target.
pub struct UiRenderPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	main_attachment: ghi::ImageHandle,
	data: UiDrawList,
	reported_capacity_limit: bool,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout rectangles.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let main_attachment: ghi::ImageHandle = render_pass_builder
			.create_render_target(
				ghi::image::Builder::new(
					MAIN_ATTACHMENT_FORMAT,
					ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
				)
				.name("UI"),
			)
			.into();

		render_pass_builder.alias("UI", "main");

		let device = render_pass_builder.device();

		let vertex_shader = create_vertex_shader(device);
		let fragment_shader = create_fragment_shader(device);

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		let shaders = [
			ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(MAIN_ATTACHMENT_FORMAT)
			.blend(ghi::pipelines::raster::BlendMode::Alpha)];

		let pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			pipeline_layout,
			&UI_VERTEX_LAYOUT,
			&shaders,
			&attachments,
		));

		let vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]> = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("UI Vertices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]> = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("UI Indices")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		Self {
			pipeline_layout,
			pipeline,
			vertex_buffer,
			index_buffer,
			main_attachment,
			data: UiDrawList::default(),
			reported_capacity_limit: false,
		}
	}

	pub fn update(&mut self, render: engine::Render) {
		self.data = update_from_render(&render);
	}
}

impl RenderPass for UiRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		// Upload the current UI geometry and schedule the smallest indexed draw set we can express with `u16` indices.
		if self.data.elements.is_empty() {
			return None;
		}

		let geometry = build_ui_geometry(&self.data, viewport.extent());

		if geometry.batches.is_empty() {
			return None;
		}

		if geometry.truncated && !self.reported_capacity_limit {
			log::warn!(
				"UI geometry capacity exceeded. The most likely cause is that the UI contains more than {MAX_UI_ELEMENTS} drawable elements in a single frame."
			);
			self.reported_capacity_limit = true;
		} else if !geometry.truncated {
			self.reported_capacity_limit = false;
		}

		let vertex_buffer_slice = frame.get_mut_buffer_slice(self.vertex_buffer);
		vertex_buffer_slice[..geometry.vertices.len()].copy_from_slice(&geometry.vertices);
		frame.sync_buffer(self.vertex_buffer);

		let index_buffer_slice = frame.get_mut_buffer_slice(self.index_buffer);
		index_buffer_slice[..geometry.indices.len()].copy_from_slice(&geometry.indices);
		frame.sync_buffer(self.index_buffer);

		let pipeline_layout = self.pipeline_layout;
		let pipeline = self.pipeline;
		let vertex_buffer = self.vertex_buffer;
		let index_buffer = self.index_buffer;
		let main_attachment = self.main_attachment;

		let extent = viewport.extent();
		let batches = geometry.batches;

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

				command_buffer.bind_vertex_buffers(&[vertex_buffer.into()]);
				command_buffer.bind_index_buffer(&index_buffer.into());

				let command_buffer = command_buffer.start_render_pass(extent, &attachments);
				let command_buffer = command_buffer.bind_pipeline_layout(pipeline_layout);
				let command_buffer = command_buffer.bind_raster_pipeline(pipeline);

				for batch in &batches {
					command_buffer.draw_indexed(batch.index_count, 1, batch.first_index, batch.vertex_offset, 0);
				}

				command_buffer.end_render_pass();
			});
		}))
	}
}

/// Builds the UI vertex shader using BESL and compiles it to SPIR-V.
fn create_vertex_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	let mut shader_generator = SPIRVShaderGenerator::new();
	let mut root = ParserNode::root();

	let main_code = r#"
		gl_Position = vec4(in_position, 0.0, 1.0);
		out_color = in_color;
		out_local_position = in_local_position;
		out_rect_size = in_rect_size;
		out_corner_radius = in_corner_radius;
	"#
	.trim();

	let main = ParserNode::main_function(vec![ParserNode::glsl(
		main_code,
		&[
			"in_position",
			"in_local_position",
			"in_rect_size",
			"in_color",
			"in_corner_radius",
			"out_color",
			"out_local_position",
			"out_rect_size",
			"out_corner_radius",
		],
		&[],
	)]);
	let position_input = ParserNode::input("in_position", "vec2f", 0);
	let local_position_input = ParserNode::input("in_local_position", "vec2f", 1);
	let rect_size_input = ParserNode::input("in_rect_size", "vec2f", 2);
	let color_input = ParserNode::input("in_color", "vec4f", 3);
	let corner_radius_input = ParserNode::input("in_corner_radius", "f32", 4);
	let color_output = ParserNode::output("out_color", "vec4f", 0);
	let local_position_output = ParserNode::output("out_local_position", "vec2f", 1);
	let rect_size_output = ParserNode::output("out_rect_size", "vec2f", 2);
	let corner_radius_output = ParserNode::output("out_corner_radius", "f32", 3);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			position_input,
			local_position_input,
			rect_size_input,
			color_input,
			corner_radius_input,
			color_output,
			local_position_output,
			rect_size_output,
			corner_radius_output,
			main,
		],
	);
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

	let main_code = r#"
		vec2 half_size = in_rect_size * 0.5;
		float corner_radius = min(in_corner_radius, min(half_size.x, half_size.y));
		vec2 centered_position = in_local_position - half_size;
		vec2 corner_delta = abs(centered_position) - (half_size - vec2(corner_radius));
		float signed_distance = length(max(corner_delta, vec2(0.0))) + min(max(corner_delta.x, corner_delta.y), 0.0) - corner_radius;
		float edge_width = max(fwidth(signed_distance), 0.5);
		float alpha = 1.0 - smoothstep(0.0, edge_width, signed_distance);

		out_color_attachment = vec4(in_color.rgb, in_color.a * alpha);
	"#
	.trim();
	let main = ParserNode::main_function(vec![ParserNode::glsl(
		main_code,
		&[
			"in_color",
			"in_local_position",
			"in_rect_size",
			"in_corner_radius",
			"out_color_attachment",
		],
		&[],
	)]);
	let input_color = ParserNode::input("in_color", "vec4f", 0);
	let input_local_position = ParserNode::input("in_local_position", "vec2f", 1);
	let input_rect_size = ParserNode::input("in_rect_size", "vec2f", 2);
	let input_corner_radius = ParserNode::input("in_corner_radius", "f32", 3);
	let output_color = ParserNode::output("out_color_attachment", "vec4f", 0);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			input_color,
			input_local_position,
			input_rect_size,
			input_corner_radius,
			output_color,
			main,
		],
	);
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

#[cfg(test)]
mod tests {
	use super::{
		build_ui_geometry, UiDrawBatch, UiDrawElement, UiDrawList, MAX_UI_VERTICES_PER_DRAW, UI_INDICES_PER_ELEMENT,
		UI_VERTICES_PER_ELEMENT,
	};
	use utils::Extent;

	fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
		assert!((actual[0] - expected[0]).abs() < 0.0001);
		assert!((actual[1] - expected[1]).abs() < 0.0001);
	}

	#[test]
	fn builds_a_single_batched_quad() {
		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [100.0, 100.0],
				elements: vec![UiDrawElement {
					position: [10.0, 20.0],
					size: [30.0, 40.0],
					color: [0.25, 0.5, 0.75, 1.0],
					corner_radius: 8.0,
				}],
			},
			Extent::rectangle(200, 100),
		);

		assert_eq!(geometry.vertices.len(), 4);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(
			geometry.batches,
			vec![UiDrawBatch {
				index_count: UI_INDICES_PER_ELEMENT as u32,
				first_index: 0,
				vertex_offset: 0,
			}]
		);
		assert_vec2_close(geometry.vertices[0].position, [-0.8, 0.6]);
		assert_vec2_close(geometry.vertices[2].position, [-0.2, -0.2]);
		assert_eq!(geometry.vertices[2].local_position, [60.0, 40.0]);
		assert_eq!(geometry.vertices[0].rect_size, [60.0, 40.0]);
		assert_eq!(geometry.vertices[0].corner_radius, 8.0);
	}

	#[test]
	fn splits_large_batches_to_stay_within_u16_indices() {
		let element_count = MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT + 1;
		let elements = (0..element_count)
			.map(|_| UiDrawElement {
				position: [0.0, 0.0],
				size: [1.0, 1.0],
				color: [1.0, 1.0, 1.0, 1.0],
				corner_radius: 0.0,
			})
			.collect();

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [1.0, 1.0],
				elements,
			},
			Extent::square(1),
		);

		assert_eq!(geometry.batches.len(), 2);
		assert_eq!(
			geometry.batches[0].index_count as usize,
			MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT * UI_INDICES_PER_ELEMENT
		);
		assert_eq!(geometry.batches[0].first_index, 0);
		assert_eq!(geometry.batches[0].vertex_offset, 0);
		assert_eq!(geometry.batches[1].index_count, UI_INDICES_PER_ELEMENT as u32);
		assert_eq!(
			geometry.batches[1].first_index as usize,
			MAX_UI_VERTICES_PER_DRAW / UI_VERTICES_PER_ELEMENT * UI_INDICES_PER_ELEMENT
		);
		assert_eq!(geometry.batches[1].vertex_offset as usize, MAX_UI_VERTICES_PER_DRAW);
	}
}
