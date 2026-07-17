//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub struct PipelineManager {
	/// Buffer containing all vertex positions for meshes.
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); 1024 * 1024]>,
	pub(super) indeces_buffer: ghi::BufferHandle<[u16; 1024 * 1024]>,
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[InstanceShaderData; 1024]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pub(super) mesh_buffers_stats: MeshBuffersStats<Handle>,
	pub(super) pipeline: ghi::PipelineHandle,
	sinks: Vec<RenderPass>,
}

const VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 1] =
	[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)];

impl PipelineManager {
	pub fn new(context: &mut ghi::implementation::Context) -> Self {
		let vertex_positions_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("Vertex Positions")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let indeces_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("Indeces")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let camera_data_buffer = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Camera Data Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let instance_data_buffer = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Instance Data Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let vertex_shader = create_besl_shader(
			context,
			"byte-engine/rendering/simple/vertex",
			"Vertex Shader",
			ResourceShaderTypes::Vertex,
			ShaderGenerationSettings::vertex(),
			create_simple_vertex_program(),
			material::ShaderInterface {
				workgroup_size: None,
				bindings: vec![
					material::Binding::new(0, material::BindingKind::StorageBuffer, 1, true, false),
					material::Binding::new(1, material::BindingKind::StorageBuffer, 1, true, false),
				],
			},
		);

		let fragment_shader = create_besl_shader(
			context,
			"byte-engine/rendering/simple/fragment",
			"Fragment Shader",
			ResourceShaderTypes::Fragment,
			ShaderGenerationSettings::fragment(),
			create_simple_fragment_program(),
			material::ShaderInterface {
				workgroup_size: None,
				bindings: Vec::new(),
			},
		);

		let pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[ghi::pipelines::PushConstantRange::new(0, 4)],
			&VERTEX_LAYOUT,
			&[
				ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
				ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
			],
			&[
				ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::RGBA16UNORM),
				ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32),
			],
		));

		Self {
			vertex_positions_buffer,
			indeces_buffer,

			mesh_buffers_stats: MeshBuffersStats::default(),

			instance_data_buffer,
			camera_data_buffer,

			pipeline,

			sinks: Vec::with_capacity(4),
		}
	}

	pub fn create_mesh(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		handle: Handle,
		entity: EntityHandle<dyn RenderableMesh>,
	) {
		let mesh = entity.get_mesh();

		let mesh_id = match mesh {
			MeshSource::Generated(generator) => 'a: {
				let mesh_hash = generator.hash();

				if let Some(mesh_id) = self.mesh_buffers_stats.does_mesh_exist(mesh_hash) {
					break 'a mesh_id;
				}

				let positions = generator.positions();
				let indices = generator.indices();
				let indices = indices.iter().map(|&index| index as u16);

				let vertex_count = positions.len();
				let index_count = indices.len();

				let vertex_buffer = frame.get_mut_buffer_slice(self.vertex_positions_buffer);

				let mesh_ref = self
					.mesh_buffers_stats
					.add_mesh(MeshStats::new(vertex_count, index_count), mesh_hash);

				let vertex_buffer_offset = mesh_ref.vertex_offset();
				let index_buffer_offset = mesh_ref.index_offset();

				vertex_buffer[vertex_buffer_offset..][..vertex_count].copy_from_slice(&positions);
				frame.sync_buffer(self.vertex_positions_buffer);

				let index_buffer = frame.get_mut_buffer_slice(self.indeces_buffer);

				index_buffer[index_buffer_offset..][..index_count]
					.iter_mut()
					.zip(indices)
					.for_each(|(dst, src)| {
						*dst = src;
					});

				frame.sync_buffer(self.indeces_buffer);

				mesh_ref.id()
			}
			_ => {
				log::warn!("SimpleRenderModel does not support non-generated meshes");
				return;
			}
		};

		let instace_id = self.mesh_buffers_stats.add_instance(mesh_id, handle);

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		let instance_batches = self.mesh_buffers_stats.get_instance_batches();

		instance_data_buffer[instace_id.index()] = InstanceShaderData {
			instance_transform: entity.transform().get_matrix().into(),
		};
	}

	pub fn update_transform(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, transform: Matrix4) {
		let Some(idx) = self.mesh_buffers_stats.get_instance_id(handle) else {
			return;
		};

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		instance_data_buffer[idx.index()] = InstanceShaderData {
			instance_transform: transform.into(),
		};
	}

	pub fn remove_mesh(&mut self, handle: Handle) {
		let Some(instance_id) = self.mesh_buffers_stats.get_instance_id(handle) else {
			return;
		};

		self.mesh_buffers_stats.remove_instance(instance_id);
	}
}

impl crate::rendering::pipeline_manager::PipelineManager for PipelineManager {
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>> {
		let instance_batches = self.mesh_buffers_stats.get_instance_batches_in(frame_allocator);
		let instance_batches = frame_allocator.alloc_slice_copy(&instance_batches);

		let commands = sinks
			.iter()
			.filter_map(|sink| {
				self.sinks
					.iter()
					.find(|sink_state| sink_state.index == sink.index())
					.map(|sink_state| (sink, sink_state))
			})
			.map(|(sink, sink_state)| {
				crate::rendering::render_pass::allocate_render_command(
					frame_allocator,
					sink_state.prepare(frame, sink, self, instance_batches),
				)
			})
			.collect::<SmallVec<[_; 16]>>();

		Some(commands)
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let main = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("main"),
		);
		let depth = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::RenderTarget | ghi::Uses::Image).name("depth"),
		);
		self.sinks.push(RenderPass::new(
			render_pass_builder.context(),
			self.camera_data_buffer.into(),
			self.instance_data_buffer.into(),
			sink_id,
		))
	}
}

/// Builds the simple pipeline fragment BESL program used to visualize object-space grid lines.
fn create_simple_fragment_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	let u32_type = root.get_child("u32").expect("u32 type not found in BESL root");
	let vec3f_type = root.get_child("vec3f").expect("vec3f type not found in BESL root");
	let vec4f_type = root.get_child("vec4f").expect("vec4f type not found in BESL root");

	root.add_children(vec![
		besl::Node::input("in_instance_index", u32_type, 0).into(),
		besl::Node::input("in_local_position", vec3f_type, 1).into(),
		besl::Node::output("out_albedo", vec4f_type, 0).into(),
	]);

	let program = besl::compile_to_besl(SIMPLE_FRAGMENT_SHADER_BESL, Some(root))
		.expect("Failed to compile the simple fragment BESL shader. The most likely cause is invalid BESL syntax.");
	program.get_main().expect(
		"Failed to find the simple fragment entry point. The most likely cause is that the BESL program did not define main.",
	)
}

const SIMPLE_FRAGMENT_SHADER_BESL: &str = r#"
palette_color: fn(index: u32) -> vec3f {
	let color: vec3f = vec3f(0.90, 0.20, 0.20);
	if (index == 1) { color = vec3f(0.20, 0.70, 0.95); }
	if (index == 2) { color = vec3f(0.35, 0.85, 0.35); }
	if (index == 3) { color = vec3f(0.95, 0.75, 0.20); }
	if (index == 4) { color = vec3f(0.75, 0.35, 0.95); }
	if (index == 5) { color = vec3f(0.95, 0.45, 0.20); }
	if (index == 6) { color = vec3f(0.25, 0.90, 0.75); }
	if (index == 7) { color = vec3f(0.85, 0.85, 0.90); }
	return color;
}

main: fn () -> void {
	let instance_index: u32 = in_instance_index;
	let local_grid: vec3f = vec3f(
		abs(fract(in_local_position.x * 4.0 + 0.5) - 0.5),
		abs(fract(in_local_position.y * 4.0 + 0.5) - 0.5),
		abs(fract(in_local_position.z * 4.0 + 0.5) - 0.5)
	);
	let grid_distance: f32 = min(local_grid.x, min(local_grid.y, local_grid.z));
	let grid_line: f32 = 1.0 - smoothstep(0.015, 0.035, grid_distance);
	let base_color: vec3f = palette_color(instance_index % 8);
	let grid_color: vec3f = base_color + (vec3f(1.0, 1.0, 1.0) - base_color) * (grid_line * 0.45);
	out_albedo = vec4f(grid_color.x, grid_color.y, grid_color.z, 1.0);
}
"#;

/// Builds the simple pipeline vertex BESL program that transforms instanced meshes with the
/// bound camera and forwards the instance index and object-space position to the fragment stage.
fn create_simple_vertex_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	let mat4f = root.get_child("mat4f").expect("mat4f type not found in BESL root");
	let vec3f = root.get_child("vec3f").expect("vec3f type not found in BESL root");
	let vec4f = root.get_child("vec4f").expect("vec4f type not found in BESL root");
	let u32_type = root.get_child("u32").expect("u32 type not found in BESL root");

	let camera = root
		.add_child(besl::Node::r#struct("Camera", vec![besl::Node::member("view_projection", mat4f.clone()).into()]).into());
	let instance = root.add_child(besl::Node::r#struct("Instance", vec![besl::Node::member("transform", mat4f).into()]).into());

	root.add_children(vec![
		besl::Node::binding(
			"cameras",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("cameras", camera, 8)],
			},
			0,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"instances",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("instances", instance, 8)],
			},
			1,
			true,
			false,
		)
		.into(),
		besl::Node::input("in_position", vec3f.clone(), 0).into(),
		besl::Node::input("instance_id", u32_type.clone(), 1).into(),
		besl::Node::output("position", vec4f, 0).into(),
		besl::Node::output("out_instance_index", u32_type, 0).into(),
		besl::Node::output("out_local_position", vec3f, 1).into(),
	]);

	// Direct field reads keep the executable VM representation allocation-free while preserving the GPU buffer layout.
	let root_node = besl::compile_to_besl(SIMPLE_VERTEX_SHADER_BESL, Some(root))
		.expect("Failed to lex the simple pipeline vertex shader. The most likely cause is invalid BESL syntax.");
	root_node.get_main().expect(
		"Failed to find the simple pipeline vertex entry point. The most likely cause is that the BESL program did not define main.",
	)
}

const SIMPLE_VERTEX_SHADER_BESL: &str = r#"
main: fn () -> void {
	let instance_index: u32 = instance_id;
	position = cameras.cameras[0].view_projection
		* instances.instances[instance_index].transform
		* vec4f(in_position.x, in_position.y, in_position.z, 1.0);
	out_instance_index = instance_index;
	out_local_position = in_position;
}
"#;

fn create_besl_shader(
	context: &mut ghi::implementation::Context,
	id: &str,
	name: &str,
	stage: ResourceShaderTypes,
	settings: ShaderGenerationSettings,
	main_node: besl::NodeReference,
	interface: material::ShaderInterface,
) -> ghi::ShaderHandle {
	crate::rendering::shader_store::create_shader(
		context,
		None,
		&crate::rendering::shader_store::ShaderSourceDescriptor {
			id,
			name,
			stage,
			source: crate::rendering::shader_store::ShaderSourceDefinition::Besl { settings, main_node },
			interface,
		},
	)
	.expect("Failed to create simple pipeline BESL shader. The most likely cause is an incompatible shader interface.")
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct InstanceShaderData {
	instance_transform: ShaderMatrix4,
}

use std::{
	collections::{hash_map::Entry, VecDeque},
	sync::Arc,
};

use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame,
};
use math::{Matrix4, ShaderMatrix4};
use resource_management::{
	asset::bema_asset_handler::ProgramGenerator, resources::material, shader::generator::ShaderGenerationSettings,
	types::ShaderTypes as ResourceShaderTypes,
};
use smallvec::SmallVec;
use utils::{
	hash::{HashMap, HashMapExt},
	json::{self, JsonContainerTrait as _, JsonValueTrait as _},
	sync::RwLock,
	Box, Extent,
};

use crate::{
	core::{
		channel::DefaultChannel,
		entity::{self},
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		Entity, EntityHandle,
	},
	gameplay::transform::TransformationUpdate,
	rendering::Camera,
	rendering::{
		lights::{Light, Lights},
		make_perspective_view_from_camera,
		pipelines::simple::{render_pass, CameraShaderData, RenderPass},
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassReturn},
		renderable::mesh::MeshSource,
		utils::{InstanceBatch, MeshBuffersStats, MeshStats},
		view::View,
		RenderableMesh, Sink,
	},
};

#[cfg(test)]
mod tests {
	use besl::vm::{
		builtin_position_slot, input_slot, output_slot, Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value,
	};
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::{ShaderGenerationSettings, ShaderGenerator as _},
	};

	use super::{create_simple_fragment_program, create_simple_vertex_program};

	const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
		}
	}

	/// Executes the production simple fragment shader for one instance and object-space position.
	fn run_fragment(instance_index: u32, local_position: [f32; 3]) -> [f32; 4] {
		let executable = ExecutableProgram::compile(create_simple_fragment_program()).expect(
			"Failed to compile simple fragment shader for the BESL VM. The most likely cause is missing VM shader support.",
		);
		let mut instance = Buffer::new(executable.input_layout(0).expect("Expected instance input layout").clone());
		let mut position = Buffer::new(executable.input_layout(1).expect("Expected position input layout").clone());
		let mut output = Buffer::new(executable.output_layout(0).expect("Expected albedo output layout").clone());
		instance
			.write("in_instance_index", Value::U32(instance_index))
			.expect("Failed to seed the instance index. The most likely cause is a simple fragment interface type mismatch.");
		position
			.write("in_local_position", Value::Vec3F(local_position))
			.expect("Failed to seed the local position. The most likely cause is a simple fragment interface type mismatch.");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut instance);
			descriptors.bind_buffer(input_slot(1), &mut position);
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute simple fragment shader. The most likely cause is incomplete BESL VM support.");
		}

		match output.read("out_albedo").expect("Expected simple fragment albedo output") {
			Value::Vec4F(color) => color,
			value => panic!(
				"Invalid simple fragment output `{value:?}`. The most likely cause is a BESL VM interface type mismatch."
			),
		}
	}

	/// Verifies the production vertex program applies indexed transforms and preserves its varyings.
	#[test]
	fn simple_vertex_besl_vm_transforms_and_forwards_inputs() {
		let executable = ExecutableProgram::compile(create_simple_vertex_program()).expect(
			"Failed to compile simple vertex shader for the BESL VM. The most likely cause is missing VM shader support.",
		);
		let mut cameras = Buffer::new(
			executable
				.buffer_layout(ResourceSlot::new(0))
				.expect("Expected camera buffer layout")
				.clone(),
		);
		let mut instances = Buffer::new(
			executable
				.buffer_layout(ResourceSlot::new(1))
				.expect("Expected instance buffer layout")
				.clone(),
		);
		let mut input_position = Buffer::new(executable.input_layout(0).expect("Expected vertex position input").clone());
		let mut input_instance = Buffer::new(executable.input_layout(1).expect("Expected vertex instance input").clone());
		let mut output_position = Buffer::new(
			executable
				.builtin_position_layout()
				.expect("Expected builtin position output")
				.clone(),
		);
		let mut output_instance = Buffer::new(executable.output_layout(0).expect("Expected instance varying output").clone());
		let mut output_local = Buffer::new(
			executable
				.output_layout(1)
				.expect("Expected local-position varying output")
				.clone(),
		);

		cameras
			.write_indexed_field("cameras", 0, "view_projection", Value::Mat4F(IDENTITY_MATRIX))
			.expect("Failed to seed camera matrix. The most likely cause is a struct buffer layout mismatch.");
		let translated = [
			1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0,
		];
		instances
			.write_indexed_field("instances", 3, "transform", Value::Mat4F(translated))
			.expect("Failed to seed instance transform. The most likely cause is a struct buffer layout mismatch.");
		input_position
			.write("in_position", Value::Vec3F([1.0, 2.0, 3.0]))
			.expect("Failed to seed vertex position. The most likely cause is an interface type mismatch.");
		input_instance
			.write("instance_id", Value::U32(3))
			.expect("Failed to seed instance ID. The most likely cause is an interface type mismatch.");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(ResourceSlot::new(0), &mut cameras);
			descriptors.bind_buffer(ResourceSlot::new(1), &mut instances);
			descriptors.bind_buffer(input_slot(0), &mut input_position);
			descriptors.bind_buffer(input_slot(1), &mut input_instance);
			descriptors.bind_buffer(builtin_position_slot(), &mut output_position);
			descriptors.bind_buffer(output_slot(0), &mut output_instance);
			descriptors.bind_buffer(output_slot(1), &mut output_local);
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute simple vertex shader. The most likely cause is incomplete BESL VM support.");
		}

		assert_eq!(
			output_position.read("position").expect("Expected transformed position"),
			Value::Vec4F([11.0, 22.0, 33.0, 1.0])
		);
		assert_eq!(
			output_instance.read("out_instance_index").expect("Expected instance varying"),
			Value::U32(3)
		);
		assert_eq!(
			output_local
				.read("out_local_position")
				.expect("Expected local position varying"),
			Value::Vec3F([1.0, 2.0, 3.0])
		);
	}

	/// Verifies palette selection, grid blending, and wrapped instance indices in the VM.
	#[test]
	fn simple_fragment_besl_vm_produces_palette_and_grid_colors() {
		assert_vec4_close(run_fragment(0, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);
		assert_vec4_close(run_fragment(0, [0.0; 3]), [0.945, 0.56, 0.56, 1.0]);
		assert_vec4_close(run_fragment(8, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);
	}

	/// Verifies both portable simple shaders remain accepted by every production source backend.
	#[test]
	fn simple_besl_shaders_lower_to_every_source_backend() {
		for (program, settings) in [
			(create_simple_vertex_program(), ShaderGenerationSettings::vertex()),
			(create_simple_fragment_program(), ShaderGenerationSettings::fragment()),
		] {
			GLSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect("Failed to lower a simple BESL shader to GLSL. The most likely cause is unsupported portable syntax.");
			HLSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect("Failed to lower a simple BESL shader to HLSL. The most likely cause is unsupported portable syntax.");
			MSLShaderGenerator::new()
				.generate(&settings, &program)
				.expect("Failed to lower a simple BESL shader to MSL. The most likely cause is unsupported portable syntax.");
		}
	}

	/// Verifies both production simple shaders remain valid after MSL raster-interface lowering.
	#[test]
	fn simple_besl_shaders_compile_to_metal() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		let vertex_source = MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::vertex(), &create_simple_vertex_program())
			.expect("Failed to lower the simple vertex shader to MSL. The most likely cause is unsupported portable syntax.");
		let fragment_source = MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::fragment(), &create_simple_fragment_program())
			.expect("Failed to lower the simple fragment shader to MSL. The most likely cause is unsupported portable syntax.");

		if ghi::implementation::USES_METAL {
			let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
				.expect("Failed to create a Metal instance. The most likely cause is unavailable Metal device support.");
			let mut queue = None;
			let mut context = instance
				.create_device(
					ghi::device::Features::new(),
					&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
				)
				.expect("Failed to create a Metal device. The most likely cause is unavailable graphics queue support.")
				.create_context()
				.expect("Failed to create a Metal context. The most likely cause is unavailable Metal command support.");

			for (name, source, stage) in [
				("Simple Vertex Shader", vertex_source.as_str(), ghi::ShaderTypes::Vertex),
				("Simple Fragment Shader", fragment_source.as_str(), ghi::ShaderTypes::Fragment),
			] {
				context
					.create_shader(
						Some(name),
						ghi::shader::Sources::MTL {
							source,
							entry_point: "besl_main",
						},
						stage,
						Vec::<ghi::ShaderResourceDescriptor>::new(),
					)
					.unwrap_or_else(|_| {
						panic!(
							"Failed to compile `{name}` as MSL. The most likely cause is invalid raster-interface lowering. Shader: {source}"
						)
					});
			}
		}

		assert!(
			vertex_source.contains("resources.cameras->cameras[0].view_projection"),
			"Generated MSL does not qualify the camera binding through its argument buffer. The most likely cause is missing raster binding context. Shader: {vertex_source}"
		);
		assert!(
			vertex_source.contains("resources.instances->instances["),
			"Generated MSL does not qualify the instance binding through its argument buffer. The most likely cause is missing raster binding context. Shader: {vertex_source}"
		);
	}
}
