use besl::ParserNode;
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
	graphics_hardware_interface::ImageHandleLike as _,
	types::Size as _,
};
use resource_management::{glsl, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{Box, Extent, RGBA};

use crate::{
	core::Entity,
	rendering::{
		common_shader_generator::CommonShaderScope,
		map_shader_binding_to_shader_binding_descriptor,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Viewport,
	},
	ui::font::TextSystem,
};

use super::{element::ElementHandle as _, layout::engine};

const MAIN_ATTACHMENT_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;
const TEXT_OVERLAY_FORMAT: ghi::Formats = ghi::Formats::RGBA8UNORM;
const TEXT_OVERLAY_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::FRAGMENT,
);

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
struct UiTextDrawElement {
	position: [f32; 2],
	size: [f32; 2],
	color: RGBA,
	font_size: f32,
	text: String,
}

#[derive(Debug, Clone)]
struct UiDrawList {
	layout_size: [f32; 2],
	elements: Vec<UiDrawElement>,
	texts: Vec<UiTextDrawElement>,
}

impl Default for UiDrawList {
	fn default() -> Self {
		Self {
			layout_size: [1.0, 1.0],
			elements: Vec::new(),
			texts: Vec::new(),
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

// Whether text rasterization should be ommitted if text is empty, 0 sized in any dimension or if fully transparent
fn should_rasterize_text(text: &UiTextDrawElement) -> bool {
	!text.text.is_empty() && text.color.a > 0.0 && text.size[0] > 0.0 && text.size[1] > 0.0
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
	let texts = render
		.texts()
		.filter_map(|text| {
			let text = UiTextDrawElement {
				position: [text.position.x() as f32, text.position.y() as f32],
				size: [text.size.x() as f32, text.size.y() as f32],
				color: text.color,
				font_size: text.font_size,
				text: text.content.clone(),
			};

			should_rasterize_text(&text).then_some(text)
		})
		.collect();

	UiDrawList {
		layout_size: [root_size.x() as f32, root_size.y() as f32],
		elements,
		texts,
	}
}

/// Rasterizes all visible text elements into the UI overlay texture for the current viewport.
fn rasterize_text_overlay(draw_list: &UiDrawList, viewport: Extent, text_system: &mut TextSystem, target: &mut [u8]) -> bool {
	let viewport_width = viewport.width().max(1);
	let viewport_height = viewport.height().max(1);

	target.fill(0);

	if draw_list.texts.is_empty() {
		return false;
	}

	let sx = viewport_width as f32 / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height as f32 / draw_list.layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let mut drew_text = false;

	for text in &draw_list.texts {
		if !should_rasterize_text(text) {
			continue;
		}

		let position = (
			(text.position[0] * sx).round().max(0.0) as u32,
			(text.position[1] * sy).round().max(0.0) as u32,
		);
		let font_size = (text.font_size * font_scale).max(1.0);

		drew_text |= text_system.rasterize(
			target,
			viewport_width,
			viewport_height,
			position,
			&text.text,
			font_size,
			text.color,
		);
	}

	drew_text
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
		let rect_width = (element.size[0] * sx).max(0.0);
		let rect_height = (element.size[1] * sy).max(0.0);

		if rect_width <= 0.0 || rect_height <= 0.0 || element.color[3] <= 0.0 {
			// Omit element if 0 sized in any dimension or if fully transparent
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
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

/// The `UiRenderPass` struct centralizes batched UI rectangle rendering and text overlay compositing for the main render target.
pub struct UiRenderPass {
	pipeline: ghi::PipelineHandle,
	vertex_buffer: ghi::BufferHandle<[UiVertex; MAX_UI_VERTICES]>,
	index_buffer: ghi::BufferHandle<[u16; MAX_UI_INDICES]>,
	text_pipeline: ghi::PipelineHandle,
	text_descriptor_set: ghi::DescriptorSetHandle,
	text_overlay: ghi::DynamicImageHandle,
	main_attachment: ghi::ImageHandle,
	data: UiDrawList,
	reported_capacity_limit: bool,
	text_system: TextSystem,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout primitives.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let main_attachment = render_pass_builder
			.create_render_target(
				ghi::image::Builder::new(
					MAIN_ATTACHMENT_FORMAT,
					ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
				)
				.name("UI"),
			)
			.into_image_handle();

		render_pass_builder.alias("UI", "main");

		let device = render_pass_builder.device();

		let vertex_shader = create_vertex_shader(device);
		let fragment_shader = create_fragment_shader(device);

		let shaders = [
			ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(MAIN_ATTACHMENT_FORMAT)
			.blend(ghi::pipelines::raster::BlendMode::Alpha)];

		let pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[],
			&[],
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
		let text_descriptor_set_template = device.create_descriptor_set_template(Some("UI Text"), &[TEXT_OVERLAY_BINDING]);
		let text_vertex_shader = create_text_overlay_vertex_shader(device);
		let text_fragment_shader = create_text_overlay_fragment_shader(device);
		let text_shaders = [
			ghi::ShaderParameter::new(&text_vertex_shader, ghi::ShaderTypes::Vertex),
			ghi::ShaderParameter::new(&text_fragment_shader, ghi::ShaderTypes::Fragment),
		];
		let text_pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[text_descriptor_set_template],
			&[],
			&[],
			&text_shaders,
			&attachments,
		));
		let text_overlay = device.build_dynamic_image(
			ghi::image::Builder::new(TEXT_OVERLAY_FORMAT, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("UI Text Overlay")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let text_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let text_descriptor_set = device.create_descriptor_set(Some("UI Text"), &text_descriptor_set_template);
		device.create_descriptor_binding(
			text_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&TEXT_OVERLAY_BINDING,
				text_overlay,
				text_sampler,
				ghi::Layouts::Read,
			),
		);

		Self {
			pipeline,
			vertex_buffer,
			index_buffer,
			text_pipeline,
			text_descriptor_set,
			text_overlay,
			main_attachment,
			data: UiDrawList::default(),
			reported_capacity_limit: false,
			text_system: TextSystem::new(),
		}
	}

	pub fn update(&mut self, render: engine::Render) {
		self.data = update_from_render(&render);
	}
}

impl RenderPass for UiRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let extent = viewport.extent();
		let geometry = build_ui_geometry(&self.data, extent);
		let has_rectangle_batches = !geometry.batches.is_empty();

		if geometry.truncated && !self.reported_capacity_limit {
			log::warn!(
				"UI geometry capacity exceeded. The most likely cause is that the UI contains more than {MAX_UI_ELEMENTS} drawable elements in a single frame."
			);
			self.reported_capacity_limit = true;
		} else if !geometry.truncated {
			self.reported_capacity_limit = false;
		}

		if has_rectangle_batches {
			let vertex_buffer_slice = frame.get_mut_buffer_slice(self.vertex_buffer);
			vertex_buffer_slice[..geometry.vertices.len()].copy_from_slice(&geometry.vertices);
			frame.sync_buffer(self.vertex_buffer);

			let index_buffer_slice = frame.get_mut_buffer_slice(self.index_buffer);
			index_buffer_slice[..geometry.indices.len()].copy_from_slice(&geometry.indices);
			frame.sync_buffer(self.index_buffer);
		}

		let mut draw_text_overlay = false;

		if !self.data.texts.is_empty() {
			assert!(
				extent.width() > 0 && extent.height() > 0,
				"UI text overlay resize requires a non-zero viewport extent. The most likely cause is that text rendering ran before swapchain extent validation."
			);

			frame.resize_image(self.text_overlay, Extent::rectangle(extent.width(), extent.height()));

			let overlay = frame.get_texture_slice_mut(self.text_overlay);
			let expected_overlay_size = extent.width() as usize * extent.height() as usize * TEXT_OVERLAY_FORMAT.size();
			draw_text_overlay = rasterize_text_overlay(&self.data, extent, &mut self.text_system, overlay);

			if draw_text_overlay {
				frame.sync_texture(self.text_overlay);
			}
		}

		if !has_rectangle_batches && !draw_text_overlay {
			return None;
		}

		let pipeline = self.pipeline;
		let vertex_buffer = self.vertex_buffer;
		let index_buffer = self.index_buffer;
		let text_pipeline = self.text_pipeline;
		let text_descriptor_set = self.text_descriptor_set;
		let main_attachment = self.main_attachment;
		let batches = geometry.batches;

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("UI", |command_buffer| {
				assert!(
					!draw_text_overlay || extent.width() > 0 && extent.height() > 0,
					"UI text overlay render pass requires a non-zero attachment extent. The most likely cause is that a stale prepared UI pass survived a viewport resize or minimization."
				);

				let attachments = [ghi::AttachmentInformation::new(
					main_attachment,
					MAIN_ATTACHMENT_FORMAT,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::None,
					true,
					true,
				)];

				if !batches.is_empty() {
					command_buffer.bind_vertex_buffers(&[vertex_buffer.into()]);
					command_buffer.bind_index_buffer(&index_buffer.into());

					let command_buffer = command_buffer.start_render_pass(extent, &attachments);
					let command_buffer = command_buffer.bind_raster_pipeline(pipeline);

					for batch in &batches {
						command_buffer.draw_indexed(batch.index_count, 1, batch.first_index, batch.vertex_offset, 0);
					}

					command_buffer.end_render_pass();
				}

				if draw_text_overlay {
					let command_buffer = command_buffer.start_render_pass(extent, &attachments);
					let command_buffer = command_buffer.bind_raster_pipeline(text_pipeline);
					command_buffer.bind_descriptor_sets(&[text_descriptor_set]);
					command_buffer.draw(3, 1, 0, 0);
					command_buffer.end_render_pass();
				}
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

fn create_text_overlay_vertex_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	let shader_source = glsl::compile(
		r#"
		#version 460
		#pragma shader_stage(vertex)

		layout(location = 0) out vec2 out_uv;

		void main() {
			vec2 positions[3] = vec2[](
				vec2(-1.0, -1.0),
				vec2(-1.0, 3.0),
				vec2(3.0, -1.0)
			);
			vec2 position = positions[gl_VertexIndex];
			gl_Position = vec4(position, 0.0, 1.0);
			out_uv = vec2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
		}
		"#,
		"ui_text_overlay.vert",
	)
	.expect("Failed to compile the UI text overlay vertex shader. The most likely cause is invalid GLSL syntax.");

	device
		.create_shader(
			Some("UI Text Overlay Vertex Shader"),
			ghi::shader::Sources::SPIRV(shader_source.as_binary_u8()),
			ghi::ShaderTypes::Vertex,
			[],
		)
		.expect(
			"Failed to create the UI text overlay vertex shader. The most likely cause is an incompatible shader interface.",
		)
}

fn create_text_overlay_fragment_shader(device: &mut ghi::implementation::Device) -> ghi::ShaderHandle {
	let shader_source = glsl::compile(
		r#"
		#version 460
		#pragma shader_stage(fragment)

		layout(set = 0, binding = 0) uniform sampler2D text_overlay;

		layout(location = 0) in vec2 in_uv;
		layout(location = 0) out vec4 out_color_attachment;

		void main() {
			out_color_attachment = texture(text_overlay, in_uv);
		}
		"#,
		"ui_text_overlay.frag",
	)
	.expect("Failed to compile the UI text overlay fragment shader. The most likely cause is invalid GLSL syntax.");

	device
		.create_shader(
			Some("UI Text Overlay Fragment Shader"),
			ghi::shader::Sources::SPIRV(shader_source.as_binary_u8()),
			ghi::ShaderTypes::Fragment,
			[TEXT_OVERLAY_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ)],
		)
		.expect(
			"Failed to create the UI text overlay fragment shader. The most likely cause is an incompatible shader interface.",
		)
}

#[cfg(test)]
mod tests {
	use super::{
		build_ui_geometry, should_rasterize_text, UiDrawBatch, UiDrawElement, UiDrawList, UiTextDrawElement, MAX_UI_ELEMENTS,
		MAX_UI_VERTICES_PER_DRAW, UI_INDICES_PER_ELEMENT, UI_VERTICES_PER_ELEMENT,
	};
	use utils::{Extent, RGBA};

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
				texts: vec![],
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
				texts: vec![],
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

	#[test]
	fn skips_zero_alpha_elements_before_capacity_checks() {
		let mut elements = Vec::with_capacity(MAX_UI_ELEMENTS + 1);

		elements.extend((0..MAX_UI_ELEMENTS).map(|_| UiDrawElement {
			position: [0.0, 0.0],
			size: [1.0, 1.0],
			color: [1.0, 1.0, 1.0, 0.0],
			corner_radius: 0.0,
		}));
		elements.push(UiDrawElement {
			position: [0.0, 0.0],
			size: [1.0, 1.0],
			color: [1.0, 1.0, 1.0, 1.0],
			corner_radius: 0.0,
		});

		let geometry = build_ui_geometry(
			&UiDrawList {
				layout_size: [1.0, 1.0],
				elements,
				texts: vec![],
			},
			Extent::square(1),
		);

		assert!(!geometry.truncated);
		assert_eq!(geometry.vertices.len(), UI_VERTICES_PER_ELEMENT);
		assert_eq!(geometry.indices.len(), UI_INDICES_PER_ELEMENT);
		assert_eq!(geometry.batches.len(), 1);
	}

	#[test]
	fn skips_zero_alpha_text_before_rasterization() {
		assert!(!should_rasterize_text(&UiTextDrawElement {
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			color: RGBA::new(1.0, 1.0, 1.0, 0.0),
			font_size: 16.0,
			text: "Hidden".to_string(),
		}));

		assert!(should_rasterize_text(&UiTextDrawElement {
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			color: RGBA::new(1.0, 1.0, 1.0, 1.0),
			font_size: 16.0,
			text: "Visible".to_string(),
		}));
	}
}
