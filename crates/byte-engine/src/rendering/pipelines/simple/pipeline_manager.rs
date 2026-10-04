//! Simple scene rendering and adoption of loader-resident meshes.
//!
//! Scene creation messages are adopted during the renderer's `update` phase, so loading overlaps window setup.
//! Loader lanes prepare, place, and transfer meshes, then `prepare` adopts their residency, resolves pending scene
//! instances, and builds draws.
//!
//! Keep this orchestration layer when adapting the example, but replace the
//! loader and store with the future renderer's formats and resident tables.
//! Application task and staging setup lives in
//! [`crate::application::graphics::setup_simple_render_pipeline`].

/// The `PipelineManager` struct coordinates Simple scene state with shared loading and renderer-owned storage.
///
/// It owns the world listeners, pending scene instances, resident lookup, instance bookkeeping, and
/// sink-local passes. Loader lanes own mesh preparation, placement, and transfer.
pub struct PipelineManager {
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[ghi::pod::Mat4x3f; MAX_INSTANCES]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[[f32; 3]; super::resource_manager::SIMPLE_VERTEX_CAPACITY]>,
	pub(super) indices_buffer: ghi::BufferHandle<[u16; super::resource_manager::SIMPLE_INDEX_CAPACITY]>,
	pipeline: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	loader: SimpleLoaderClient,
	mesh_listener: DefaultListener<CreateMessage<RenderableMesh>>,
	deletions_listener: DefaultListener<DeleteMessage>,
	transforms_listener: DefaultListener<TransformationUpdate>,
	resident_meshes: HashMap<MeshKey, ResidentSimpleMesh>,
	pending_renderables: Vec<PendingRenderable>,
	/// Live scene instances in draw order. A slot's index is its instance-data index.
	instances: StableVec<(ResidentSimpleMesh, Handle)>,
	/// The instance slot of each renderable that has one.
	instance_slots: HashMap<Handle, StableVecHandle>,
	// TODO: Replace this temporary map with proper retained component storage.
	renderable_transforms: HashMap<Handle, Transform>,
	sinks: Vec<RenderPass>,
}

/// The number of instances Simple's instance-data buffer holds.
const MAX_INSTANCES: usize = 1024;

/// The `PendingRenderable` struct keeps scene identity separate from one coalesced mesh request.
///
/// Multiple values may point to the same coalesced loader key.
struct PendingRenderable {
	handle: Handle,
	key: MeshKey,
}

/// The `InstanceBatch` struct is one indexed draw over consecutive instance slots that share a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InstanceBatch {
	pub(crate) base_index: usize,
	pub(crate) base_vertex: usize,
	pub(crate) instance_count: usize,
	pub(crate) index_count: usize,
	pub(crate) base_instance: usize,
}

/// Groups live instances into frame-allocated draws in slot order.
///
/// A batch ends at a mesh switch or an empty slot, so every draw's instances are contiguous in instance data.
fn instance_batches_in<'a, T>(
	instances: &StableVec<(ResidentSimpleMesh, T)>,
	allocator: &'a bumpalo::Bump,
) -> Vec<InstanceBatch, &'a bumpalo::Bump> {
	let mut batches = Vec::with_capacity_in(instances.len(), allocator);
	let mut current: Option<(ResidentSimpleMesh, InstanceBatch)> = None;
	for slot in 0..instances.slots_len() {
		match (instances.get_slot(slot), &mut current) {
			(Some((mesh, _)), Some((current_mesh, batch))) if mesh == current_mesh => batch.instance_count += 1,
			(Some((mesh, _)), _) => {
				let batch = InstanceBatch {
					base_index: mesh.base_index,
					base_vertex: mesh.base_vertex,
					instance_count: 1,
					index_count: mesh.index_count,
					base_instance: slot,
				};
				batches.extend(current.replace((*mesh, batch)).map(|(_, finished)| finished));
			}
			(None, _) => batches.extend(current.take().map(|(_, finished)| finished)),
		}
	}
	batches.extend(current.map(|(_, finished)| finished));
	batches
}

impl PipelineManager {
	/// Creates the Simple scene, renderer-owned mesh store, and shared async loading client.
	///
	/// The application must already have created the loader, its running lane,
	/// its shared resource store, and the asynchronously driven
	/// pipeline compiler represented by `pipeline_manager`. This constructor subscribes to `world`'s meshes,
	/// deletions, and transforms and queues the Simple pipeline request; it never waits for shader resources or
	/// creates shaders on the render thread. Next, register this value through
	/// [`crate::rendering::Renderer::add_pipeline_manager`].
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		world: &DefaultWorld,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		loader: SimpleLoaderClient,
		resource_store: &SharedSimpleResourceStore,
	) -> Self {
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

		let pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/simple/simple.pipeline");
		let (vertex_positions_buffer, indices_buffer) = {
			let store = resource_store.lock().unwrap_or_else(|error| error.into_inner());
			(store.vertex_positions_buffer, store.indices_buffer)
		};

		Self {
			instance_data_buffer,
			camera_data_buffer,
			vertex_positions_buffer,
			indices_buffer,
			pipeline,
			pipeline_manager,
			loader,
			mesh_listener: world.factory::<RenderableMesh>().listener(),
			deletions_listener: world.deletions_listener(),
			transforms_listener: world.transforms_channel().listener(),
			resident_meshes: HashMap::default(),
			pending_renderables: Vec::new(),
			instances: StableVec::new(),
			instance_slots: HashMap::default(),
			renderable_transforms: HashMap::default(),
			sinks: Vec::with_capacity(4),
		}
	}

	/// Requests or reuses a mesh and delays instance creation until GPU upload completion.
	///
	/// Duplicate mesh keys coalesce in the loader while each handle retains
	/// independent pending state. Failed keys retry when scene demand requests them again.
	fn request_mesh(&mut self, handle: Handle, renderable: RenderableMesh) {
		let source = renderable.source().clone();
		let key = source.key();

		// Creation is an upsert. Keep the independently retained transform while
		// replacing resident or pending geometry for this handle.
		self.remove_mesh_instance(handle);
		self.remove_pending(handle);

		// Instance data lives in frame storage, so even a resident mesh waits for `prepare` to place it.
		if !self.resident_meshes.contains_key(&key) {
			self.loader.request(source);
		}
		self.pending_renderables.push(PendingRenderable { handle, key });
	}

	/// Writes every live instance's transform into this frame's copy of the instance-data buffer.
	///
	/// Each frame in flight reads its own copy, and a copy last written by an earlier frame misses every change
	/// since. So each frame writes its whole copy instead of only the slots that changed.
	fn write_instance_data(&self, frame: &mut ghi::implementation::Frame) {
		let instance_data = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);
		for (slot, (_, handle)) in self.instances.indexed_iter() {
			let transform = self.renderable_transforms.get(handle).cloned().unwrap_or_default();
			instance_data[slot] = transform.get_matrix().into();
		}
	}

	/// Removes a mesh and any transform retained for later creation.
	///
	/// In-flight loader work remains coalesced and may populate the resident cache.
	fn remove_mesh(&mut self, handle: Handle) {
		self.remove_mesh_instance(handle);
		self.remove_pending(handle);
		self.renderable_transforms.remove(&handle);
	}

	/// Removes only resident instance state so an upsert can reuse the retained transform.
	fn remove_mesh_instance(&mut self, handle: Handle) {
		if let Some(slot) = self.instance_slots.remove(&handle) {
			self.instances.remove(slot);
		}
	}

	/// Removes pending scene state for one deleted handle.
	fn remove_pending(&mut self, handle: Handle) {
		if let Some(index) = self.pending_renderables.iter().position(|pending| pending.handle == handle) {
			self.pending_renderables.swap_remove(index);
		}
	}

	/// Creates scene instances whose shared mesh uploads completed at the frame boundary.
	fn resolve_pending_renderables(&mut self) {
		let mut index = 0usize;
		while index < self.pending_renderables.len() {
			let key = self.pending_renderables[index].key;
			let Some(resident) = self.resident_meshes.get(&key).copied() else {
				index += 1;
				continue;
			};
			// Each resident instance gets one slot; `write_instance_data` uploads its retained transform.
			let pending = self.pending_renderables.swap_remove(index);
			let slot = self.instances.push((resident, pending.handle));
			if slot.index() >= MAX_INSTANCES {
				self.instances.remove(slot);
				log::error!(
					"Simple instance storage is full. The most likely cause is more than 1,024 live renderable instances."
				);
				continue;
			}
			self.instance_slots.insert(pending.handle, slot);
		}
	}
}

impl crate::rendering::pipeline_manager::PipelineManager for PipelineManager {
	/// Adopts mesh creation messages so their loads overlap window setup.
	fn update(&mut self) {
		while let Some(message) = self.mesh_listener.read() {
			self.request_mesh(message.handle(), message.into_data());
		}
	}

	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		_alpha: f32,
		_time: crate::time::MediaTime,
	) -> SmallVec<[(usize, RenderPassReturn<'a>); 16]> {
		// Transforms apply before deletions, so a renderable moved and deleted in one tick ends deleted. A transform that
		// arrives before residency is kept, and the instance reads it once it exists.
		while let Some(message) = self.transforms_listener.read() {
			self.renderable_transforms
				.insert(message.handle(), message.transform().clone());
		}
		while let Some(message) = self.deletions_listener.read() {
			self.remove_mesh(message.into_handle());
		}
		while let Some(event) = self.loader.poll() {
			match event {
				crate::rendering::loading::Event::Ready { key, resident } => {
					self.resident_meshes.insert(key, resident);
				}
				crate::rendering::loading::Event::Failed { key, error } => {
					log::error!("Simple mesh '{key}' could not be loaded: {error}");
				}
			}
		}
		let Some(pipeline) = self.pipeline_manager.pipeline(self.pipeline) else {
			return SmallVec::new();
		};
		self.resolve_pending_renderables();
		self.write_instance_data(frame);
		let instance_batches: &[InstanceBatch] = instance_batches_in(&self.instances, frame_allocator).leak();

		sinks
			.iter()
			.filter_map(|sink| {
				let sink_state = self.sinks.iter().find(|sink_state| sink_state.index == sink.index())?;
				let command = sink_state.prepare(frame, sink, self, pipeline, instance_batches, frame_allocator);
				Some((
					sink.index(),
					crate::rendering::render_pass::allocate_render_command(frame_allocator, command),
				))
			})
			.collect()
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

		let background = render_pass_builder.create_scene_background(crate::rendering::render_pass::SceneBackgroundTargets {
			color: main.into(),
			depth: depth.into(),
		});
		self.sinks.push(RenderPass::new(
			render_pass_builder.context(),
			self.camera_data_buffer.into(),
			self.instance_data_buffer.into(),
			sink_id,
			[main.into(), depth.into()],
			background,
		))
	}
}

use ghi::{context::ContextCreate as _, frame::Frame as _};
use smallvec::SmallVec;
use utils::{StableVec, StableVecHandle, hash::HashMap};

use crate::{
	core::{
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
	},
	gameplay::{
		transform::{Transform, TransformationUpdate},
		world::DefaultWorld,
	},
	rendering::{
		RenderableMesh, Sink,
		pipelines::simple::{
			CameraShaderData, RenderPass,
			resource_manager::{ResidentSimpleMesh, SharedSimpleResourceStore, SimpleLoaderClient},
		},
		render_pass::{RenderPassBuilder, RenderPassReturn},
		renderable::mesh::MeshKey,
	},
};

#[cfg(test)]
mod tests {
	use utils::StableVec;

	use super::{ResidentSimpleMesh, instance_batches_in};

	fn mesh(base_vertex: usize, base_index: usize, index_count: usize) -> ResidentSimpleMesh {
		ResidentSimpleMesh {
			index_count,
			base_vertex,
			base_index,
		}
	}

	#[test]
	fn batches_split_at_mesh_switches_and_holes() {
		let first_mesh = mesh(0, 0, 30);
		let second_mesh = mesh(10, 30, 60);
		let mut instances = StableVec::new();
		instances.push((first_mesh, "first"));
		let removed = instances.push((first_mesh, "removed"));
		instances.push((second_mesh, "second-a"));
		instances.push((second_mesh, "second-b"));
		instances.push((first_mesh, "last"));
		instances.remove(removed);
		let allocator = bumpalo::Bump::new();

		let batches = instance_batches_in(&instances, &allocator);

		assert_eq!(batches.len(), 3);
		assert_eq!((batches[0].index_count, batches[0].base_instance), (30, 0));
		assert_eq!((batches[1].index_count, batches[1].instance_count), (60, 2));
		assert_eq!((batches[1].base_vertex, batches[1].base_index), (10, 30));
		assert_eq!((batches[2].index_count, batches[2].base_instance), (30, 4));
	}

	use besl::vm::{
		DescriptorBindings, ResourceSlot, Value, builtin_instance_index_slot, builtin_position_slot, input_slot, output_slot,
	};

	use crate::rendering::shader_vm_test::{
		IDENTITY_MATRIX, buffer, builtin_position_buffer, compile, input_buffer, link_program, output_buffer, run_at,
	};

	const SIMPLE_FRAGMENT_BESL: &str =
		include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/simple/fragment.besl"));
	const SIMPLE_VERTEX_BESL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/simple/vertex.besl"));

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
		}
	}

	/// Executes the production simple fragment shader for one instance and object-space position.
	fn run_fragment(instance_index: u32, local_position: [f32; 3]) -> [f32; 4] {
		let program = compile(link_program(SIMPLE_FRAGMENT_BESL, "Simple fragment shader"));

		let mut instance = input_buffer(&program, 0);

		let mut position = input_buffer(&program, 1);

		let mut output = output_buffer(&program, 0);

		instance
			.write("_besl_interface_instance_index", Value::U32(instance_index))
			.expect("Failed to seed the instance index. The most likely cause is a simple fragment interface type mismatch.");

		position
			.write("_besl_interface_local_position", Value::Vec3F(local_position))
			.expect("Failed to seed the local position. The most likely cause is a simple fragment interface type mismatch.");

		{
			let mut descriptors = DescriptorBindings::new();

			descriptors.bind_buffer(input_slot(0), &mut instance);

			descriptors.bind_buffer(input_slot(1), &mut position);

			descriptors.bind_buffer(output_slot(0), &mut output);

			run_at(&program, &mut descriptors, [0, 0]);
		}

		let Ok(Value::Vec4F(color)) = output.read("_besl_output_albedo") else {
			panic!("Expected vec4 fragment output")
		};

		color
	}

	/// Verifies the production vertex program applies indexed transforms and preserves its varyings.
	#[test]
	fn simple_vertex_besl_vm_transforms_and_forwards_inputs() {
		let program = compile(link_program(SIMPLE_VERTEX_BESL, "Simple vertex shader"));

		let mut cameras = buffer(&program, ResourceSlot::new(0));

		let mut instances = buffer(&program, ResourceSlot::new(1));

		let mut input_position = input_buffer(&program, 0);

		let mut input_instance = buffer(&program, builtin_instance_index_slot());

		let mut output_position = builtin_position_buffer(&program);

		let mut output_instance = output_buffer(&program, 0);

		let mut output_local = output_buffer(&program, 1);

		cameras
			.write_array_member(0, "view_projection", Value::Mat4F(IDENTITY_MATRIX))
			.expect("Failed to seed camera matrix. The most likely cause is a struct buffer layout mismatch.");

		instances
			.write_array_element(
				3,
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0]),
			)
			.expect("Failed to seed instance transform. The most likely cause is a compact transform buffer layout mismatch.");

		input_position
			.write("in_position", Value::Vec3F([1.0, 2.0, 3.0]))
			.expect("Failed to seed vertex position. The most likely cause is an interface type mismatch.");

		input_instance
			.write("instance_index", Value::U32(3))
			.expect("Failed to seed instance ID. The most likely cause is an interface type mismatch.");

		{
			let mut descriptors = DescriptorBindings::new();

			descriptors.bind_buffer(ResourceSlot::new(0), &mut cameras);

			descriptors.bind_buffer(ResourceSlot::new(1), &mut instances);

			descriptors.bind_buffer(input_slot(0), &mut input_position);

			descriptors.bind_buffer(builtin_instance_index_slot(), &mut input_instance);

			descriptors.bind_buffer(builtin_position_slot(), &mut output_position);

			descriptors.bind_buffer(output_slot(0), &mut output_instance);

			descriptors.bind_buffer(output_slot(1), &mut output_local);

			run_at(&program, &mut descriptors, [0, 0]);
		}

		assert_eq!(
			output_position.read("_besl_interface_position"),
			Ok(Value::Vec4F([11.0, 22.0, 33.0, 1.0]))
		);
		assert_eq!(output_instance.read("_besl_interface_instance_index"), Ok(Value::U32(3)));
		assert_eq!(
			output_local.read("_besl_interface_local_position"),
			Ok(Value::Vec3F([1.0, 2.0, 3.0]))
		);
	}

	/// Verifies palette selection, grid blending, and wrapped instance indices in the VM.
	#[test]
	fn simple_fragment_besl_vm_produces_palette_and_grid_colors() {
		assert_vec4_close(run_fragment(0, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);

		assert_vec4_close(run_fragment(0, [0.0; 3]), [0.945, 0.56, 0.56, 1.0]);

		assert_vec4_close(run_fragment(8, [0.125; 3]), [0.9, 0.2, 0.2, 1.0]);
	}
}
