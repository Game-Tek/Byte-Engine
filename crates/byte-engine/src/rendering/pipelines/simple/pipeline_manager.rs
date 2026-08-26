//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub struct PipelineManager {
	/// Buffer containing all vertex positions for meshes.
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); 1024 * 1024]>,
	pub(super) indeces_buffer: ghi::BufferHandle<[u16; 1024 * 1024]>,
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[AffineShaderMatrix; 1024]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pub(super) mesh_buffers_stats: MeshBuffersStats<Handle>,
	pub(super) pipeline: ghi::PipelineHandle,
	// TODO: Replace this temporary map with proper retained component storage.
	renderable_transforms: HashMap<Handle, Transform>,
	sinks: Vec<RenderPass>,
}

const VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 1] =
	[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)];

impl PipelineManager {
	pub fn new(
		context: &mut ghi::implementation::Context,
		resources: &resource_management::resource::resource_manager::ResourceManager,
	) -> Self {
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

		let vertex_shader = load_besl_shader(
			context,
			resources,
			"byte-engine/rendering/simple/vertex.besl",
			"Vertex Shader",
		);

		let fragment_shader = load_besl_shader(
			context,
			resources,
			"byte-engine/rendering/simple/fragment.besl",
			"Fragment Shader",
		);

		let pipeline = context.create_raster_pipeline(
			ghi::pipelines::raster::Builder::new(
				&[ghi::pipelines::PushConstantRange::new(0, 4)],
				&VERTEX_LAYOUT,
				&[
					ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
					ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
				],
				&[
					ghi::pipelines::raster::AttachmentDescriptor::new(crate::rendering::SCENE_COLOR_FORMAT),
					ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32),
				],
			)
			.name("Vertex Shader"),
		);

		Self {
			vertex_positions_buffer,
			indeces_buffer,

			mesh_buffers_stats: MeshBuffersStats::default(),

			instance_data_buffer,
			camera_data_buffer,

			pipeline,

			renderable_transforms: HashMap::new(),
			sinks: Vec::with_capacity(4),
		}
	}

	/// Creates or replaces a mesh instance while preserving transform updates received before creation.
	pub fn create_mesh(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, renderable: RenderableMesh) {
		// Creation messages are upserts, but the latest independently published transform must survive replacement.
		self.remove_mesh_instance(handle);

		let mesh = renderable.source();

		let mesh_id = match mesh {
			MeshSource::Generated(generator) => 'a: {
				let mesh_hash = generator.hash();

				if let Some(mesh_id) = self.mesh_buffers_stats.does_mesh_exist(mesh_hash) {
					break 'a mesh_id;
				}

				let positions = generator.positions();

				let indices = generator.indices();

				debug_assert!(
					indices.iter().all(|&index| u16::try_from(index).is_ok()),
					"Simple mesh index exceeds u16. The most likely cause is submitting geometry that is too large for the simple pipeline."
				);

				let indices = indices.iter().map(|&index| index as u16);

				let vertex_count = positions.len();

				let index_count = indices.len();

				let vertex_buffer = frame.get_mut_buffer_slice(self.vertex_positions_buffer);

				let mesh_ref = self
					.mesh_buffers_stats
					.add_mesh(MeshStats::new(vertex_count, index_count), mesh_hash);

				let vertex_buffer_offset = mesh_ref.vertex_offset();

				let index_buffer_offset = mesh_ref.index_offset();

				debug_assert!(
					vertex_buffer_offset
						.checked_add(vertex_count)
						.is_some_and(|end| end <= vertex_buffer.len()),
					"Simple vertex buffer is too small. The most likely cause is inconsistent mesh allocation statistics."
				);

				vertex_buffer[vertex_buffer_offset..][..vertex_count].copy_from_slice(&positions);

				frame.sync_buffer(self.vertex_positions_buffer);

				let index_buffer = frame.get_mut_buffer_slice(self.indeces_buffer);

				debug_assert!(
					index_buffer_offset
						.checked_add(index_count)
						.is_some_and(|end| end <= index_buffer.len()),
					"Simple index buffer is too small. The most likely cause is inconsistent mesh allocation statistics."
				);

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

		let transform = self.renderable_transforms.get(&handle).cloned().unwrap_or_default();
		instance_data_buffer[instace_id.index()] = transform.get_matrix().into();
	}

	/// Retains the latest transform and applies it immediately when the mesh instance is resident.
	pub fn update_transform(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, transform: &Transform) {
		self.renderable_transforms.insert(handle, transform.clone());

		let Some(idx) = self.mesh_buffers_stats.get_instance_id(handle) else {
			return;
		};

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		instance_data_buffer[idx.index()] = transform.get_matrix().into();
	}

	/// Removes a mesh and any transform retained for later creation.
	pub fn remove_mesh(&mut self, handle: Handle) {
		self.remove_mesh_instance(handle);
		self.renderable_transforms.remove(&handle);
	}

	/// Removes only resident instance state so an upsert can reuse the retained transform.
	fn remove_mesh_instance(&mut self, handle: Handle) {
		let Some(instance_id) = self.mesh_buffers_stats.get_instance_id(handle) else {
			return;
		};

		self.mesh_buffers_stats.remove_instance(instance_id);
	}
}

impl crate::rendering::pipeline_manager::PipelineManager for PipelineManager {
	fn begin_frame(&mut self, _completed_frame: Option<ghi::FrameKey>) -> bool {
		false
	}

	fn record_frame_uploads(
		&mut self,
		_frame: ghi::FrameKey,
		_recording: &mut ghi::implementation::CommandBufferRecording<'_>,
	) {
	}

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
				crate::rendering::SCENE_COLOR_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("main"),
		);

		let depth = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::RenderTarget | ghi::Uses::Image)
				.name("depth")
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);

		self.sinks.push(RenderPass::new(
			render_pass_builder.context(),
			self.camera_data_buffer.into(),
			self.instance_data_buffer.into(),
			sink_id,
		))
	}
}

fn load_besl_shader(
	context: &mut ghi::implementation::Context,
	resources: &resource_management::resource::resource_manager::ResourceManager,
	id: &str,
	name: &str,
) -> ghi::ShaderHandle {
	crate::rendering::resource_loading::load_shader(context, resources, id, name)
		.unwrap_or_else(|error| panic!("{error}"))
		.handle
}

use std::{
	collections::{VecDeque, hash_map::Entry},
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
use math::{AffineShaderMatrix, ShaderMatrix};
use resource_management::asset::handler::implementations::bema::ProgramGenerator;
use smallvec::SmallVec;
use utils::{
	Box, Extent,
	hash::{HashMap, HashMapExt},
	json::{self, JsonContainerTrait as _, JsonValueTrait as _},
	sync::RwLock,
};

use crate::{
	core::{
		Entity,
		channel::DefaultChannel,
		entity::{self},
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
	},
	gameplay::transform::Transform,
	rendering::Camera,
	rendering::{
		RenderableMesh, Sink,
		lights::{Light, Lights},
		make_perspective_view_from_camera,
		pipelines::simple::{CameraShaderData, RenderPass, render_pass},
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassReturn},
		renderable::mesh::MeshSource,
		utils::{InstanceBatch, MeshBuffersStats, MeshStats},
		view::View,
	},
};

#[cfg(test)]

mod tests {

	use besl::vm::{DescriptorBindings, ResourceSlot, Value, builtin_position_slot, input_slot, output_slot};
	use resource_management::shader::{
		besl::backends::{hlsl::HLSLTranspiler, msl::MSLTranspiler},
		generator::ShaderGenerationSettings,
	};

	use crate::rendering::shader_vm_test::{buffer, builtin_position_buffer, compile, input_buffer, output_buffer, run_at};

	fn create_simple_fragment_program() -> besl::NodeReference {
		besl::compile_to_besl(
			include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/simple/fragment.besl")),
			None,
		)
		.expect("Simple fragment asset should compile")
		.get_main()
		.expect("Simple fragment asset should contain main")
	}

	fn create_simple_vertex_program() -> besl::NodeReference {
		besl::compile_to_besl(
			include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/simple/vertex.besl")),
			None,
		)
		.expect("Simple vertex asset should compile")
		.get_main()
		.expect("Simple vertex asset should contain main")
	}

	const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
		}
	}

	/// Executes the production simple fragment shader for one instance and object-space position.
	fn run_fragment(instance_index: u32, local_position: [f32; 3]) -> [f32; 4] {
		let program = compile(create_simple_fragment_program());

		let mut instance = input_buffer(&program, 0);

		let mut position = input_buffer(&program, 1);

		let mut output = output_buffer(&program, 0);

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

			run_at(&program, &mut descriptors, [0, 0]);
		}

		let Ok(Value::Vec4F(color)) = output.read("out_albedo") else {
			panic!("Expected vec4 fragment output")
		};

		color
	}

	/// Verifies the production vertex program applies indexed transforms and preserves its varyings.
	#[test]
	fn simple_vertex_besl_vm_transforms_and_forwards_inputs() {
		let program = compile(create_simple_vertex_program());

		let mut cameras = buffer(&program, ResourceSlot::new(0));

		let mut instances = buffer(&program, ResourceSlot::new(1));

		let mut input_position = input_buffer(&program, 0);

		let mut input_instance = input_buffer(&program, 1);

		let mut output_position = builtin_position_buffer(&program);

		let mut output_instance = output_buffer(&program, 0);

		let mut output_local = output_buffer(&program, 1);

		cameras
			.write_indexed_field("cameras", 0, "view_projection", Value::Mat4F(IDENTITY_MATRIX))
			.expect("Failed to seed camera matrix. The most likely cause is a struct buffer layout mismatch.");

		instances
			.write_indexed(
				"transforms",
				3,
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0]),
			)
			.expect("Failed to seed instance transform. The most likely cause is a compact transform buffer layout mismatch.");

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

			run_at(&program, &mut descriptors, [0, 0]);
		}

		assert_eq!(output_position.read("position"), Ok(Value::Vec4F([11.0, 22.0, 33.0, 1.0])));
		assert_eq!(output_instance.read("out_instance_index"), Ok(Value::U32(3)));
		assert_eq!(output_local.read("out_local_position"), Ok(Value::Vec3F([1.0, 2.0, 3.0])));
	}

	/// Verifies palette selection, grid blending, and wrapped instance indices in the VM.
	#[test]
	fn simple_fragment_besl_vm_produces_palette_and_grid_colors() {
		assert_vec4_close(run_fragment(0, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);

		assert_vec4_close(run_fragment(0, [0.0; 3]), [0.945, 0.56, 0.56, 1.0]);

		assert_vec4_close(run_fragment(8, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);
	}
}
