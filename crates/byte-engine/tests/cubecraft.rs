//! Cubecraft example application
//! This demonstrates a simple first person game, which is definitely not a clone of Minecraft.
//! It uses the Byte-Engine to create a simple game with a player character that can move around and jump.
//! It also includes a simple physics engine to handle collisions and movement.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::rc::Rc;

use byte_engine::constants::FORWARD;
use byte_engine::constants::RIGHT;
use byte_engine::constants::UP;
use byte_engine::core::entity::EntityBuilder;
use byte_engine::core::listener::EntitySubscriber;
use byte_engine::core::listener::Listener;
use byte_engine::core::Entity;
use byte_engine::core::EntityHandle;

use byte_engine::core::Task;
use byte_engine::gameplay::space::Spawn;
use byte_engine::rendering::aces_tonemap_render_pass::AcesToneMapPass;
use byte_engine::rendering::common_shader_generator::CommonShaderGenerator;
use byte_engine::rendering::render_pass::RenderPass;
use byte_engine::rendering::render_pass::RenderPassBuilder;
use byte_engine::rendering::view::View;
use byte_engine::{application::{Application, Parameter}, camera::Camera, input::{Action, ActionBindingDescription, Function}, rendering::directional_light::DirectionalLight, Vector3};
use ghi::raster_pipeline;
use ghi::BoundRasterizationPipelineMode;
use ghi::CommandBufferRecordable;
use ghi::Device;
use ghi::RasterizationRenderPassMode;
use resource_management::glsl;
use utils::hash::HashMap;
use utils::hash::HashMapExt;
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
	let space_handle = app.get_root_space_handle().clone();

	// Create the lookaround action handle
	let lookaround_action_handle = space_handle.spawn(Action::<Vector3>::new("Lookaround", &[
		ActionBindingDescription::new("Mouse.Position").mapped(Vector3::new(1f32, 1f32, 1f32).into(), Function::Sphere),
		ActionBindingDescription::new("Gamepad.RightStick"),
	],));

	// Create the move action
	let move_action_handle = space_handle.spawn(Action::<Vector3>::new("Move", &[
		ActionBindingDescription::new("Keyboard.W").mapped(FORWARD.into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.S").mapped((-FORWARD).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.A").mapped((-RIGHT).into(), Function::Linear),
		ActionBindingDescription::new("Keyboard.D").mapped(RIGHT.into(), Function::Linear),
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

	space_handle.spawn(ChunkLoader::create());

	// Create the camera
	let camera = space_handle.spawn(Camera::new(Vector3::new(0.0, 1.8, 0.0),));

	// Create the directional light
	let _ = space_handle.spawn(DirectionalLight::new(maths_rs::normalize(-UP), 4000f32));

	{
		let camera = camera.clone();

		lookaround_action_handle.write().value().add(move |value: &Vector3| {
			let mut camera = camera.write();
	
			camera.set_orientation(*value);
		});
	}

	const CHUNK_SIZE: i32 = 16;
	const HALF_CHUNK_SIZE: i32 = CHUNK_SIZE / 2;

	{
		let camera = camera.clone();
		let move_action_handle = move_action_handle.clone();

		space_handle.spawn(Task::tick(move || {
			let value = move_action_handle.read().value().get();

			let mut camera = camera.write();
	
			let position = camera.get_position();
			camera.set_position(position + value);
		}));
	}

	{
		let a = space_handle.clone();

		space_handle.spawn(Task::once(move || {
			let blocks = (-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).map(move |x| {
				(-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).map(move |z| {
					(-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).filter_map(move |y| {
						let position = (x, y, z);
						let block = make_block(position);
		
						if block != AIR_BLOCK {
							Some(Block::create(position, block))
						} else {
							None
						}
					})
				}).flatten()
			}).flatten().collect::<Vec<_>>();
		
			a.spawn(blocks);
		}));
	}

	app.do_loop()
}

struct ChunkLoader {
	loaded: Vec<Location>,
	camera: Option<EntityHandle<Camera>>,
}

impl ChunkLoader {
	fn new() -> Self {
		ChunkLoader { loaded: Vec::new(), camera: None, }
	}

	fn create() -> EntityBuilder<'static, Self> {
		EntityBuilder::new(ChunkLoader::new()).listen_to::<Camera>()
	}
}

impl Entity for ChunkLoader {}

impl EntitySubscriber<Camera> for ChunkLoader {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Camera>, params: &'a Camera) -> () {
		self.camera = Some(handle.clone());
	}
}

#[derive(Clone, Copy)]
struct Block {
	position: Location,
	block: u32,
}

impl Block {
	fn new(position: Location, block: u32) -> Self {
		Block { position, block, }
	}

	fn create(position: Location, block: u32) -> EntityBuilder<'static, Self> {
		Block { position, block, }.into()
	}
}

impl Entity for Block {
	fn call_listeners<'a>(&'a self, listener: &'a byte_engine::core::listener::BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		listener.invoke_for(handle.clone(), self);
	}
}

type Location = (i32, i32, i32);

struct RenderParams {
	index_count: u32,
	vertex_count: u32,
	instance_count: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct FaceShaderData {
	block: u16,
}

struct CubeCraftRenderPass {
	vertex_buffer: ghi::BufferHandle<[(f32, f32, f32); 16 * 16 * 256 * 32]>,
	index_buffer: ghi::BufferHandle<[u16; 16 * 16 * 256 * 32 * 3]>,

	camera_data_buffer: ghi::BufferHandle<maths_rs::Mat4f>,

	face_data_buffer: ghi::BufferHandle<[FaceShaderData; 16 * 16 * 256 * 32]>,

	set: ghi::DescriptorSetHandle,
	binding: ghi::DescriptorSetBindingHandle,

	layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,

	render_params: Rc<RefCell<RenderParams>>,

	ghi: Rc<RwLock<ghi::GHI>>,

	blocks: Vec<Block>,

	camera: Option<EntityHandle<Camera>>,
}

impl Entity for CubeCraftRenderPass {}

impl EntitySubscriber<Block> for CubeCraftRenderPass {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Block>, params: &'a Block) -> () {
		self.blocks.push(*params);
	}
}

impl EntitySubscriber<Camera> for CubeCraftRenderPass {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Camera>, params: &'a Camera) -> () {
		self.camera = Some(handle.clone());
	}
}

impl RenderPass for CubeCraftRenderPass {
	fn create<'a>(render_pass_builder: &'a mut RenderPassBuilder) -> EntityBuilder<'static, Self> where Self: Sized {
		let ghi = render_pass_builder.ghi();
		let mut ghi = ghi.write();

		render_pass_builder.render_to("main");
		render_pass_builder.render_to("depth");

		let vertex_buffer = ghi.create_buffer(Some("vertices"), ghi::Uses::Vertex, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
		let index_buffer = ghi.create_buffer(Some("indices"), ghi::Uses::Index, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("template"), &[
			ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX),
			ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::FRAGMENT),
			ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::FRAGMENT),
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
		#extension GL_EXT_shader_16bit_storage: require
		#extension GL_EXT_fragment_shader_barycentric: require

		layout(location = 0) out vec4 out_color;

		layout(set = 0, binding = 1) readonly buffer Faces {
			uint16_t block[];
		} faces;

		layout(set = 0, binding = 2) readonly uniform sampler2D[] textures;

		void main() {
			uint in_block = uint(faces.block[gl_PrimitiveID / 2]);

			if (in_block == 1) {
				out_color = vec4(0.0, 1.0, 0.0, 1.0);
			} else {
				if (in_block == 2) {
					out_color = vec4(0.25, 0.25, 0.25, 1.0);
				} else if (in_block == 3) {
					out_color = vec4(0.5, 0.25, 0.0, 1.0);
				} else {
					out_color = vec4(1.0, 1.0, 1.0, 1.0);
				}
			}
		}
		"#;

		let v_shader_artifact = glsl::compile(v_shader_source, "Cube Vertex Shader").unwrap();
		let f_shader_artifact = glsl::compile(f_shader_source, "Cube Fragment Shader").unwrap();

		let v_shader = ghi.create_shader(None, ghi::ShaderSource::SPIRV(v_shader_artifact.borrow().into()), ghi::ShaderTypes::Vertex, &[ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),]).unwrap();
		let f_shader = ghi.create_shader(None, ghi::ShaderSource::SPIRV(f_shader_artifact.borrow().into()), ghi::ShaderTypes::Fragment, &[ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ), ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ)]).unwrap();

		// TODO: notify user if provided shaders don't consume any bindings in the layout
		let pipeline = ghi.create_raster_pipeline(raster_pipeline::Builder::new(layout, &[ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)], &[ghi::ShaderParameter::new(&v_shader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&f_shader, ghi::ShaderTypes::Fragment)], &[ghi::PipelineAttachmentInformation::new(ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized),), ghi::PipelineAttachmentInformation::new(ghi::Formats::Depth32,)]));

		let camera_data_buffer = ghi.create_buffer(Some("camera"), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
		let face_data_buffer = ghi.create_buffer(Some("face_data_buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let set = ghi.create_descriptor_set(None, &descriptor_set_template);

		// let grass_texture = ghi.create_image(None, Extent::square(16), ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC, 0);

		let camera_data_buffer_binding = ghi.create_descriptor_binding(set, ghi::BindingConstructor::buffer(&ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX), camera_data_buffer.into()));
		let face_data_buffer_binding = ghi.create_descriptor_binding(set, ghi::BindingConstructor::buffer(&ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX), face_data_buffer.into()));
		// let texture_binding = ghi.create_descriptor_binding(set, ghi::BindingConstructor::combined_image_sampler(&ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::FRAGMENT), grass_texture, sampler, ghi::Layouts::Read));

		drop(ghi);

		let render_params = RenderParams {
			index_count: 0,
			vertex_count: 0,
			instance_count: 0,
		};

		EntityBuilder::new(Self {
			vertex_buffer,
			index_buffer,

			camera_data_buffer,

			face_data_buffer,

			set,
			binding: camera_data_buffer_binding,

			layout,
			pipeline,

			render_params: Rc::new(RefCell::new(render_params)),

			ghi: render_pass_builder.ghi(),

			blocks: Vec::with_capacity(8192 * 32),

			camera: None,
		}).listen_to::<Block>().listen_to::<Camera>()
	}

	fn get_read_attachments() -> Vec<&'static str> where Self: Sized {
		vec![]
	}

	fn get_write_attachments() -> Vec<&'static str> where Self: Sized {
		vec!["main"]
	}

	fn prepare(&self, ghi: &mut ghi::GHI, extent: utils::Extent) {
		if let Some(camera) = &self.camera {
			let camera = camera.read();

			let view = View::new_perspective(camera.get_fov(), extent.aspect_ratio(), 0.1f32, 100f32, camera.get_position(), camera.get_orientation());
	
			*ghi.get_mut_buffer_slice(self.camera_data_buffer) = view.view_projection();
		}

		let (vertices, indices, faces) = build_cubes(&self.blocks);

		ghi.get_mut_buffer_slice(self.vertex_buffer)[..vertices.len()].copy_from_slice(&vertices);
		ghi.get_mut_buffer_slice(self.index_buffer)[..indices.len()].copy_from_slice(&indices);

		let faces = faces.iter().map(|&face| FaceShaderData { block: face as u16, }).collect::<Vec<_>>();

		ghi.get_mut_buffer_slice(self.face_data_buffer)[..faces.len()].copy_from_slice(&faces);

		let mut render_params = self.render_params.borrow_mut();

		render_params.index_count = indices.len() as u32;
		render_params.vertex_count = vertices.len() as u32;
		render_params.instance_count = 1;
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: utils::Extent, attachments: &[ghi::AttachmentInformation],) {
		let (vertex_count, index_count, instance_count) = {
			let render_params = self.render_params.borrow_mut();
			(render_params.vertex_count, render_params.index_count, render_params.instance_count)
		};

		command_buffer_recording.bind_vertex_buffers(&[ghi::BufferDescriptor::new(self.vertex_buffer.into(), 0, vertex_count as usize, 0)]);
		command_buffer_recording.bind_index_buffer(&ghi::BufferDescriptor::new(self.index_buffer.into(), 0, index_count as usize, 0));
		let render_pass = command_buffer_recording.start_render_pass(extent, attachments);
		render_pass.bind_descriptor_sets(&self.layout, &[self.set]);
		let pipeline = render_pass.bind_raster_pipeline(&self.pipeline);
		pipeline.draw_indexed(index_count, instance_count, 0, 0, 0);
		render_pass.end_render_pass();
	}
}

const AIR_BLOCK: u32 = 0;
const GRASS_BLOCK: u32 = 1;
const STONE_BLOCK: u32 = 2;
const DIRT_BLOCK: u32 = 3;

fn make_block(position: Location) -> u32 {
	if position.1 > 0 {
		AIR_BLOCK
	} else {
		if position.1 == 0 {
			GRASS_BLOCK
		} else if position.1 == -1 {
			DIRT_BLOCK
		} else {
			STONE_BLOCK
		}
	}
}

type Vertex = (f32, f32, f32);

/// Returns a list of vertices and indices for the blocks
/// The vertices are in the format (x, y, z) and the indices are in the format (v1, v2, v3)
/// Triangles for higher Y values are drawn first, as they are more likely to be visible
fn build_cubes(blocks: &[Block]) -> (Vec<Vertex>, Vec<u16>, Vec<u16>) {
	let cube_sides: [(i32, i32, i32); 6] = [
		(1, 0, 0),
		(-1, 0, 0),
		(0, 1, 0),
		(0, -1, 0),
		(0, 0, 1),
		(0, 0, -1),
	];

	// Per cube face data
	let mut faces = Vec::with_capacity(8192 * 6 * 4);

	let mut sides = HashMap::with_capacity(8192 * 6);

	for block in blocks {
		let pos = block.position;

		for &side in &cube_sides {
			let pos = (pos.0 * 2, pos.1 * 2, pos.2 * 2);

			let face = (pos.0 + side.0, pos.1 + side.1, pos.2 + side.2);

			let block = Block::new(pos, block.block);

			// If cube side already exists, then this wall is internal
			sides.entry(face).and_modify(|(_, external): &mut (_, bool)| *external = false).or_insert(((block, side), true));
		}
	}

	let external_sides = sides.values().filter(|(_, external)| *external).map(|(k, _)| *k).collect::<Vec<_>>();

	let face_corners = [
		(-1, 1),
		(1, 1),
		(-1, -1),
		(1, -1),
	];

	let mut corners = HashMap::with_capacity(8192 * 6 * 4 * 3);
	let mut vertices = Vec::with_capacity(corners.len() * 3);

	for &(block, normal) in &external_sides {
		let (cx, cy, cz) = block.position;

		for (fx, fy) in face_corners {
			let (x, y, z) = match (normal.0.abs(), normal.1.abs(), normal.2.abs()) {
				(1, 0, 0) => (0, fx, fy),
				(0, 1, 0) => (fx, 0, fy),
				(0, 0, 1) => (fx, fy, 0),
				_ => unreachable!(),
			};

			let vertex = (cx + normal.0 + x, cy + normal.1 + y, cz + normal.2 + z);

			corners.entry(vertex).or_insert_with(|| {
				let index = vertices.len();
				let (x, y, z) = vertex;
				vertices.push((x as f32 * 0.5, y as f32 * 0.5, z as f32 * 0.5));
				index
			});
		}
	}

	let mut x_sides = external_sides.clone().into_iter().filter(move |&(_, (sx, _, _))| sx.abs() == 1).collect::<Vec<_>>();
	let mut y_sides = external_sides.clone().into_iter().filter(move |&(_, (_, sy, _))| sy.abs() == 1).collect::<Vec<_>>();
	let mut z_sides = external_sides.clone().into_iter().filter(move |&(_, (_, _, sz))| sz.abs() == 1).collect::<Vec<_>>();

	x_sides.sort_by(|(ac, r#as), (bc, bs)| (bc.position.0 + bs.0).cmp(&(ac.position.0 + r#as.0))); // Place higher x sides first, as they are more likely to be visible
	y_sides.sort_by(|(ac, r#as), (bc, bs)| {
		let (_, ay, az) = (ac.position.0 + r#as.0, ac.position.1 + r#as.1, ac.position.2 + r#as.2);
		let (_, by, bz) = (bc.position.0 + bs.0, bc.position.1 + bs.1, bc.position.2 + bs.2);

		// Place higher y sides first, as they are more likely to be visible, and then sort by z so nearer sides are drawn first

		if ay == by {
			return (az).cmp(&bz);
		} else {
			return by.cmp(&ay);
		}
	});
	z_sides.sort_by(|(ac, r#as), (bc, bs)| (bc.position.2 + bs.2).cmp(&(ac.position.2 + r#as.2)));

	let mut indices = Vec::with_capacity(corners.len() * 3);

	// Draw y sides first, as they are more likely to be visible

	for (sides, square_vertices) in [(y_sides, [(-1, 0, 1), (1, 0, 1), (1, 0, -1), (1, 0, -1), (-1, 0, -1), (-1, 0, 1)]), (x_sides, [(0, 1, -1), (0, 1, 1), (0, -1, 1), (0, -1, 1), (0, -1, -1), (0, 1, -1)]), (z_sides, [(1, 1, 0), (-1, 1, 0), (-1, -1, 0), (-1, -1, 0), (1, -1, 0), (1, 1, 0)])] {
		for (block, normal) in sides {			
			let (cx, cy, cz) = block.position;
			let (x, y, z) = (cx + normal.0, cy + normal.1, cz + normal.2);
		
			for (fx, fy, fz) in square_vertices {
				let (fx, fy, fz) = match (normal.0.abs(), normal.1.abs(), normal.2.abs()) {
					(1, 0, 0) => (fx, fy * normal.0, fz),
					(0, 1, 0) => (fx, fy, fz * normal.1),
					(0, 0, 1) => (fx * normal.2, fy, fz),
					_ => unreachable!(),
				};

				let corner = (fx + x, fy + y, fz + z);
		
				let corner_index = corners.get(&corner).expect("Corner must exist!");
		
				indices.push(*corner_index as u16);
			}
		
			faces.push(block.block as u16);
		}
	}

	(vertices, indices, faces)
}

#[cfg(test)]
mod tests {
    use crate::{build_cubes, Block, DIRT_BLOCK, GRASS_BLOCK};

	fn assert_upper_cube_face(vertices: &[(f32, f32, f32)], indices: &[u16], face: usize, offset: (f32, f32, f32)) {
		let (x, y, z) = offset;
		assert_eq!(vertices[indices[face + 0] as usize], (-0.5 + x, 0.5 + y, 0.5 + z));
		assert_eq!(vertices[indices[face + 1] as usize], (0.5 + x, 0.5 + y, 0.5 + z));
		assert_eq!(vertices[indices[face + 2] as usize], (0.5 + x, 0.5 + y, -0.5 + z));
		assert_eq!(vertices[indices[face + 3] as usize], (0.5 + x, 0.5 + y, -0.5 + z));
		assert_eq!(vertices[indices[face + 4] as usize], (-0.5 + x, 0.5 + y, -0.5 + z));
		assert_eq!(vertices[indices[face + 5] as usize], (-0.5 + x, 0.5 + y, 0.5 + z));
	}

	fn assert_lower_cube_face(vertices: &[(f32, f32, f32)], indices: &[u16], face: usize, offset: (f32, f32, f32)) {
		let (x, y, z) = offset;
		assert_eq!(vertices[indices[face + 0] as usize], (-0.5 + x, -0.5 + y, -0.5 + z));
		assert_eq!(vertices[indices[face + 1] as usize], (0.5 + x, -0.5 + y, -0.5 + z));
		assert_eq!(vertices[indices[face + 2] as usize], (0.5 + x, -0.5 + y, 0.5 + z));
		assert_eq!(vertices[indices[face + 3] as usize], (0.5 + x, -0.5 + y, 0.5 + z));
		assert_eq!(vertices[indices[face + 4] as usize], (-0.5 + x, -0.5 + y, 0.5 + z));
		assert_eq!(vertices[indices[face + 5] as usize], (-0.5 + x, -0.5 + y, -0.5 + z));
	}

	#[test]
	fn test_build_single_cube() {
		let blocks = [
			Block::new((0, 0, 0), GRASS_BLOCK),
		];

		let (vertices, indices, faces) = build_cubes(&blocks);

		dbg!(&vertices);
		dbg!(&indices);

		assert_eq!(vertices.len(), 8);
		assert_eq!(indices.len(), 36);
		assert_eq!(faces.len(), 6);

		{
			let mut vertices = vertices.clone();
			vertices.sort_by(|(ax, ay, az), (bx, by, bz)| {
				if ax == bx {
					if ay == by {
						az.partial_cmp(bz).unwrap()
					} else {
						ay.partial_cmp(by).unwrap()
					}
				} else {
					ax.partial_cmp(bx).unwrap()
				}
			});

			assert_eq!(vertices[0], (-0.5, -0.5, -0.5));
			assert_eq!(vertices[1], (-0.5, -0.5, 0.5));
			assert_eq!(vertices[2], (-0.5, 0.5, -0.5));
			assert_eq!(vertices[3], (-0.5, 0.5, 0.5));
			assert_eq!(vertices[4], (0.5, -0.5, -0.5));
			assert_eq!(vertices[5], (0.5, -0.5, 0.5));
			assert_eq!(vertices[6], (0.5, 0.5, -0.5));
			assert_eq!(vertices[7], (0.5, 0.5, 0.5));
		}

		indices.iter().for_each(|index| {
			assert!((*index as usize) < vertices.len());
		});

		indices.chunks(6).for_each(|window| {
			assert_eq!(window[0], window[5]);
			assert_eq!(window[2], window[3]);
		});

		assert_upper_cube_face(&vertices, &indices, 0, (0.0, 0.0, 0.0));

		assert_eq!(faces[0], 1);
		assert_eq!(faces[1], 1);
		assert_eq!(faces[2], 1);
		assert_eq!(faces[3], 1);
		assert_eq!(faces[4], 1);
		assert_eq!(faces[5], 1);

		assert_lower_cube_face(&vertices, &indices, 6, (0.0, 0.0, 0.0));
	}

	#[test]
	fn test_build_two_cubes() {
		let blocks = [
			Block::new((0, 0, 0), GRASS_BLOCK),
			Block::new((0, 0, 1), DIRT_BLOCK),
		];

		let (vertices, indices, faces) = build_cubes(&blocks);

		dbg!(&vertices);
		dbg!(&indices);

		assert_eq!(vertices.len(), 4 + 4 + 4);
		assert_eq!(indices.len(), 5 * 2 * 6);

		indices.iter().for_each(|index| {
			assert!((*index as usize) < vertices.len());
		});

		indices.chunks(6).for_each(|window| {
			assert_eq!(window[0], window[5]);
			assert_eq!(window[2], window[3]);
		});

		assert_upper_cube_face(&vertices, &indices, 0, (0.0, 0.0, 0.0));
		assert_upper_cube_face(&vertices, &indices, 6, (0.0, 0.0, 1.0));

		assert_eq!(faces[0], 1);
	}
}