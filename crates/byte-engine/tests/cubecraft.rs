//! Cubecraft example application
//! This demonstrates a simple first person game, which is definitely not a clone of Minecraft.
//! It uses the Byte-Engine to create a simple game with a player character that can move around and jump.
//! It also includes a simple physics engine to handle collisions and movement.

use std::borrow::Cow;
use std::rc::Rc;

use byte_engine::core::entity::EntityBuilder;
use byte_engine::core::listener::EntitySubscriber;
use byte_engine::core::listener::Listener;
use byte_engine::core::Entity;
use byte_engine::core::EntityHandle;

use byte_engine::gameplay::space::Spawn;
use byte_engine::rendering::aces_tonemap_render_pass::AcesToneMapPass;
use byte_engine::rendering::common_shader_generator::CommonShaderGenerator;
use byte_engine::rendering::mesh::{MeshGenerator, MeshSource, RenderEntity};
use byte_engine::rendering::render_pass::RenderPass;
use byte_engine::rendering::render_pass::RenderPassBuilder;
use byte_engine::rendering::view::View;
use byte_engine::{application::{Application, Parameter}, camera::Camera, input::{Action, ActionBindingDescription, Function}, rendering::directional_light::DirectionalLight, Vector3};
use ghi::BoundRasterizationPipelineMode;
use ghi::CommandBufferRecordable;
use ghi::Device;
use ghi::RasterizationRenderPassMode;
use maths_rs::mat::MatTranslate;
use utils::sync::RwLock;

#[ignore]
#[test]
fn cubecraft() {
	// Create the Byte-Engine application
	let mut app = byte_engine::application::GraphicsApplication::new("Cubecraft", &[Parameter::new("resources-path", "../../resources"), Parameter::new("assets-path", "../../assets")]);

	{
		let generator = {
			let common_shader_generator = CommonShaderGenerator::new();
			common_shader_generator
		};

		byte_engine::application::graphics_application::setup_default_resource_and_asset_management(&mut app, generator);
	}

	{
		let mut renderer = app.get_renderer_handle().write();
		renderer.add_render_pass::<CubeCraftRenderPass>(app.get_root_space_handle().clone());
		renderer.add_render_pass::<AcesToneMapPass>(app.get_root_space_handle().clone());
	}

	byte_engine::application::graphics_application::setup_default_input(&mut app);
	byte_engine::application::graphics_application::setup_default_window(&mut app);

	// Get the root space handle
	let space_handle = app.get_root_space_handle();

	// Create the lookaround action handle
	let lookaround_action_handle = space_handle.spawn(Action::<Vector3>::new("Lookaround", &[
		ActionBindingDescription::new("Mouse.Position").mapped(Vector3::new(1f32, 1f32, 1f32).into(), Function::Sphere),
		ActionBindingDescription::new("Gamepad.RightStick"),
	],));

	// Create the move action
	let move_action_handle = space_handle.spawn(Action::<Vector3>::new("Move", &[
		ActionBindingDescription::new("Keyboard.W").mapped(Vector3::new(0f32, 0f32, 1f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.S").mapped(Vector3::new(0f32, 0f32, -1f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.A").mapped(Vector3::new(-1f32, 0f32, 0f32).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.D").mapped(Vector3::new(1f32, 0f32, 0f32).into(), Function::Linear),

		ActionBindingDescription::new("Gamepad.LeftStick").mapped(Vector3::new(1f32, 0f32, 1f32).into(), Function::Linear),
	],));

	// Create the jump action
	let jump_action_handle = space_handle.spawn(Action::<bool>::new("Jump", &[
		ActionBindingDescription::new("Keyboard.Space"),
		ActionBindingDescription::new("Gamepad.A"),
	],));

	// Create the right hand action
	let fire_action_handle = space_handle.spawn(Action::<bool>::new("RightHand", &[
		ActionBindingDescription::new("Mouse.LeftButton"),
		ActionBindingDescription::new("Gamepad.RightTrigger"),
	],));

	let exit_action_handle = space_handle.spawn(Action::<bool>::new("Exit", &[
		ActionBindingDescription::new("Keyboard.Escape"),
	],));

	// Create the camera
	let camera = space_handle.spawn(Camera::new(Vector3::new(0.0, 1.0, 0.0),));

	// Create the directional light
	let _ = space_handle.spawn(DirectionalLight::new(maths_rs::normalize(Vector3::new(0.0, -1.0, 0.0)), 4000f32));

	const CHUNK_SIZE: i32 = 16;
	const HALF_CHUNK_SIZE: i32 = CHUNK_SIZE / 2;

	let blocks = (-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).map(move |x| {
		(-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).map(move |z| {
			(-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).filter_map(move |y| {
				let position = (x, y, z);
				let block = make_block(position);

				if block == GRASS_BLOCK {
					Some(Block::new(position, block))
				} else {
					None
				}
			})
		}).flatten()
	}).flatten().collect::<Vec<_>>();

	space_handle.spawn(blocks);

	app.do_loop()
}

struct Block {
	position: (i32, i32, i32),
	block: u32,

	source: MeshSource,
}

impl Block {
	fn new(position: (i32, i32, i32), block: u32) -> EntityBuilder<'static, Self> {
		Block { position, block, source: MeshSource::Generated(Box::new(CubeMeshGenerator {})) }.into()
	}
}

impl Entity for Block {
	fn call_listeners<'a>(&'a self, listener: &'a byte_engine::core::listener::BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		listener.invoke_for(handle.clone() as EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
	}
}

impl RenderEntity for Block {
	fn get_mesh(&self) -> &byte_engine::rendering::mesh::MeshSource {
		&self.source
	}

	fn get_transform(&self) -> maths_rs::Mat4f {
		maths_rs::Mat4f::from_translation(Vector3::new(self.position.0 as f32, self.position.1 as f32, self.position.2 as f32))
	}
}

struct CubeMeshGenerator {

}

impl MeshGenerator for CubeMeshGenerator {
	fn vertices(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(vec![
			(-0.5, -0.5, -0.5),
			(0.5, -0.5, -0.5),
			(0.5, 0.5, -0.5),
			(-0.5, 0.5, -0.5),
			(-0.5, -0.5, 0.5),
			(0.5, -0.5, 0.5),
			(0.5, 0.5, 0.5),
			(-0.5, 0.5, 0.5),
		])
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(vec![
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
		])
	}

	fn tangents(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
			maths_rs::Vec3f::new(-1.0, 0.0, 0.0),
		])
	}

	fn bitangents(&self) -> std::borrow::Cow<[maths_rs::Vec3f]> {
		Cow::Owned(vec![
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, 1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
			maths_rs::Vec3f::new(0.0, -1.0, 0.0),
		])
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		Cow::Owned(vec![
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
		])
	}

	fn indices(&self) -> std::borrow::Cow<[u32]> {
		Cow::Owned(vec![
			0, 1, 2,
			0, 2, 3,
			4, 5, 6,
			4, 6, 7,
			0, 1, 5,
			0, 5, 4,
			1, 2, 6,
			1, 6, 5,
			2, 3, 7,
			2, 7, 6,
			3, 0, 4,
			3, 4, 7,
		])
	}
}

struct CubeCraftRenderPass {
	vertex_buffer: ghi::BufferHandle<[(f32, f32, f32); 16 * 16 * 256 * 32]>,
	index_buffer: ghi::BufferHandle<[u16; 16 * 16 * 256 * 32 * 3]>,

	camera: ghi::BufferHandle<maths_rs::Mat4f>,

	set: ghi::DescriptorSetHandle,
	binding: ghi::DescriptorSetBindingHandle,

	layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,

	index_count: u32,
	vertex_count: u32,
	instance_count: u32,

	ghi: Rc<RwLock<ghi::GHI>>,
}

impl Entity for CubeCraftRenderPass {}

impl EntitySubscriber<Block> for CubeCraftRenderPass {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Block>, params: &'a Block) -> () {
		let mesh = params.get_mesh();

		if self.instance_count > 0 {
			self.instance_count += 1;
			return;
		}

		let mesh = match mesh { MeshSource::Generated(g) => g, _ => panic!("Mesh is not generated") };

		let vertices = mesh.vertices();
		let indices = mesh.indices();

		let ghi = self.ghi.write();
		ghi.get_mut_buffer_slice(self.vertex_buffer)[self.vertex_count as usize..][..vertices.len()].copy_from_slice(&vertices);
		ghi.get_mut_buffer_slice(self.index_buffer)[self.index_count as usize..][..indices.len()].copy_from_slice(&indices.iter().map(|i| *i as u16).collect::<Vec<_>>());

		self.vertex_count += vertices.len() as u32;
		self.index_count += indices.len() as u32;
		self.instance_count += 1;

		// ghi.get_mut_buffer_slice(self.vertex_buffer)[self.vertex_count as usize..][..4].copy_from_slice(&[(-0.5, -0.5, 0f32), (0.5, -0.5, 0f32), (0.5, 0.5, 0f32), (-0.5, 0.5, 0f32)]);
		// ghi.get_mut_buffer_slice(self.index_buffer)[self.index_count as usize..][..6].copy_from_slice(&[0, 1, 2, 0, 2, 3]);

		// self.vertex_count += 4;
		// self.index_count += 6;
		// self.instance_count += 1;
	}
}

impl RenderPass for CubeCraftRenderPass {
	fn create<'a>(render_pass_builder: &'a mut RenderPassBuilder) -> EntityBuilder<'static, Self> where Self: Sized {
		let ghi = render_pass_builder.ghi();
		let mut ghi = ghi.write();

		render_pass_builder.render_to("main");

		let vertex_buffer = ghi.create_buffer(Some("vertices"), ghi::Uses::Vertex, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
		let index_buffer = ghi.create_buffer(Some("indices"), ghi::Uses::Index, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("template"), &[
			ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX),
		]);
		let layout = ghi.create_pipeline_layout(&[descriptor_set_template], &[]);

		let v_shader_source = r#"#version 450 core
		#pragma shader_stage(vertex)
		// Row major matrices in buffers
		layout(row_major) buffer;

		layout(location = 0) in vec3 in_position;

		layout(set = 0, binding = 0) readonly buffer Camera {
			mat4 vp;
		} camera;
		
		void main() {
			gl_Position = camera.vp * vec4(in_position, 1.0);
		}
		"#;

		let f_shader_source = r#"#version 450 core
		#pragma shader_stage(fragment)
		layout(location = 0) out vec4 out_color;
		void main() {
			out_color = vec4(1.0, 1.0, 1.0, 1.0);
		}
		"#;

		let v_shader = ghi.create_shader(None, ghi::ShaderSource::GLSL(v_shader_source.into()), ghi::ShaderTypes::Vertex, &[ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ)]).unwrap();
		let f_shader = ghi.create_shader(None, ghi::ShaderSource::GLSL(f_shader_source.into()), ghi::ShaderTypes::Fragment, &[]).unwrap();

		let pipeline = ghi.create_raster_pipeline(&[
			ghi::PipelineConfigurationBlocks::InputAssembly {  },
			ghi::PipelineConfigurationBlocks::Shaders { shaders: &[ghi::ShaderParameter::new(&v_shader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&f_shader, ghi::ShaderTypes::Fragment)] },
			ghi::PipelineConfigurationBlocks::VertexInput { vertex_elements: &[ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)] },
			ghi::PipelineConfigurationBlocks::Layout { layout: &layout }, // TODO: notify user if provided shaders don't consume any bindings in the layout
			ghi::PipelineConfigurationBlocks::RenderTargets { targets: &[ghi::PipelineAttachmentInformation::new(ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget, ghi::ClearValue::None, false, true)] },
		]);
		
		let camera = ghi.create_buffer(Some("camera"), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let view = View::new_perspective(45f32, 16f32 / 9f32, 0.1f32, 100f32, maths_rs::Vec3f::new(0f32, 1f32, -2f32), maths_rs::Vec3f::new(0f32, 0f32, 1f32));

		*ghi.get_mut_buffer_slice(camera) = view.view_projection();

		let set = ghi.create_descriptor_set(None, &descriptor_set_template);

		let binding = ghi.create_descriptor_binding(set, ghi::BindingConstructor::buffer(&ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX), camera.into()));

		drop(ghi);

		EntityBuilder::new(Self {
			vertex_buffer,
			index_buffer,

			camera,

			set,
			binding,

			layout,
			pipeline,

			index_count: 0,
			vertex_count: 0,
			instance_count: 0,

			ghi: render_pass_builder.ghi(),
		}).listen_to::<Block>()
	}

	fn get_read_attachments() -> Vec<&'static str> where Self: Sized {
		vec![]
	}

	fn get_write_attachments() -> Vec<&'static str> where Self: Sized {
		vec!["main"]
	}

	fn prepare(&self, ghi: &mut ghi::GHI, extent: utils::Extent) {
		
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: utils::Extent, attachments: &[ghi::AttachmentInformation],) {
		command_buffer_recording.bind_vertex_buffers(&[ghi::BufferDescriptor::new(self.vertex_buffer.into(), 0, (self.vertex_count * 12) as u64, 0)]);
		command_buffer_recording.bind_index_buffer(&ghi::BufferDescriptor::new(self.index_buffer.into(), 0, (self.index_count * 2) as u64, 0));
		let render_pass = command_buffer_recording.start_render_pass(extent, attachments);
		render_pass.bind_descriptor_sets(&self.layout, &[self.set]);
		let pipeline = render_pass.bind_raster_pipeline(&self.pipeline);
		pipeline.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);
		render_pass.end_render_pass();
	}
}

const AIR_BLOCK: u32 = 0;
const GRASS_BLOCK: u32 = 1;

fn make_block(position: (i32, i32, i32)) -> u32 {
	if position.1 > 0 {
		AIR_BLOCK
	} else {
		GRASS_BLOCK
	}
}