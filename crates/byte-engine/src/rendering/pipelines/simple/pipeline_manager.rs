//! Simple scene rendering and the render-thread half of asynchronous mesh loading.
//!
//! This module shows where the shared resource lifecycle meets renderer frame
//! callbacks. Scene creation enters through [`PipelineManager::request_mesh`].
//! [`crate::rendering::pipeline_manager::PipelineManager::begin_frame`] retires
//! completed GPU work and drains prepared meshes. Its reciprocal
//! `record_frame_uploads` callback delegates exact placement to
//! `SimpleResourceStore`. Finally, `prepare`
//! resolves pending scene instances and builds draws only from resident meshes.
//!
//! Keep this orchestration layer when adapting the example, but replace the
//! resource protocol and store with the future renderer's formats and resident
//! tables. Application task and staging setup lives in
//! [`crate::application::graphics::setup_simple_render_pipeline`].

/// The `PipelineManager` struct coordinates Simple scene state with shared loading and renderer-owned storage.
///
/// It owns all render-thread halves of the integration: logical request state,
/// pending scene instances, the frame upload queue, resident lookup, exact GPU
/// storage, and sink-local passes. Worker tasks own only preparation.
pub struct PipelineManager {
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[AffineShaderMatrix; 1024]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pipeline: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	resource_loader: ResourceLoader<SimpleMeshResource>,
	uploads: FrameUploadQueue<PreparedSimpleMesh, ResidentSimpleMesh>,
	pub(super) resource_store: SimpleResourceStore,
	resident_meshes: HashMap<MeshKey, ResidentSimpleMesh>,
	pending_renderables: Vec<PendingRenderable>,
	// TODO: Replace this temporary map with proper retained component storage.
	renderable_transforms: HashMap<Handle, Transform>,
	sinks: Vec<RenderPass>,
}

/// The `PendingRenderable` struct keeps scene identity separate from one coalesced mesh request.
///
/// Multiple values may point to the same resource reference. This is why
/// deleting one instance cancels loading only when no other pending instance
/// still consumes that resource.
struct PendingRenderable {
	handle: Handle,
	key: MeshKey,
	resource: ResourceRef,
}

impl PipelineManager {
	/// Creates the Simple scene, renderer-owned mesh store, and shared async loading client.
	///
	/// The application must already have created the loader, its running server,
	/// the staging arena backed by `staging_buffer`, and the asynchronously driven
	/// pipeline compiler represented by `pipeline_manager`. This constructor only
	/// queues the Simple pipeline request; it never waits for shader resources or
	/// creates shaders on the render thread. Next, register this value through
	/// [`crate::rendering::Renderer::add_pipeline_manager`].
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		resource_loader: ResourceLoader<SimpleMeshResource>,
		staging_buffer: ghi::BaseBufferHandle,
	) -> Self {
		let resource_store = SimpleResourceStore::new(context, staging_buffer);

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

		Self {
			instance_data_buffer,
			camera_data_buffer,
			pipeline,
			pipeline_manager,
			resource_loader,
			uploads: FrameUploadQueue::default(),
			resource_store,
			resident_meshes: HashMap::new(),
			pending_renderables: Vec::new(),
			renderable_transforms: HashMap::new(),
			sinks: Vec::with_capacity(4),
		}
	}

	/// Requests or reuses a mesh and delays instance creation until GPU upload completion.
	///
	/// Call this while adopting a scene creation message. Duplicate mesh keys
	/// coalesce in the loader while each handle retains independent pending state.
	/// Failed dependencies retry without changing their stable reference.
	pub fn request_mesh(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, renderable: RenderableMesh) {
		let source = renderable.source().clone();
		let key = source.key();

		// Creation is an upsert. Keep the independently retained transform while
		// replacing resident or pending geometry for this handle.
		self.remove_mesh_instance(handle);
		self.remove_pending_for_upsert(handle, key);

		if let Some(resident) = self.resident_meshes.get(&key).copied() {
			self.add_resident_instance(frame, handle, resident);
			return;
		}

		let resource = match self.resource_loader.request(key, source) {
			Ok(resource) => resource,
			Err(_) => {
				log::error!(
					"Simple mesh request capacity is full for '{key}'. The most likely cause is more unique meshes than the configured loader capacity."
				);
				return;
			}
		};
		if matches!(
			self.resource_loader.state(resource),
			ResourceState::Failed | ResourceState::Cancelled
		) {
			self.resource_loader.retry(resource);
		}
		self.pending_renderables.push(PendingRenderable { handle, key, resource });
		self.resource_loader.submit_requests(usize::MAX);
	}

	/// Retains the latest transform and applies it immediately when the mesh instance is resident.
	///
	/// Updates arriving before residency are not lost; instance creation reads the
	/// retained transform after the upload completes.
	pub fn update_transform(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, transform: &Transform) {
		self.renderable_transforms.insert(handle, transform.clone());

		let Some(idx) = self.resource_store.instance_id(handle) else {
			return;
		};

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		instance_data_buffer[idx.index()] = transform.get_matrix().into();
	}

	/// Removes a mesh and any transform retained for later creation.
	///
	/// Pending loading is cancelled only when this was the last scene consumer
	/// and renderer storage has not claimed the request.
	pub fn remove_mesh(&mut self, handle: Handle) {
		self.remove_mesh_instance(handle);
		self.remove_pending_and_cancel_orphan(handle);
		self.renderable_transforms.remove(&handle);
	}

	/// Removes only resident instance state so an upsert can reuse the retained transform.
	fn remove_mesh_instance(&mut self, handle: Handle) {
		let Some(instance_id) = self.resource_store.instance_id(handle) else {
			return;
		};

		self.resource_store.remove_instance(instance_id);
	}

	/// Replaces pending state without cancelling an identical in-flight request.
	fn remove_pending_for_upsert(&mut self, handle: Handle, replacement_key: MeshKey) {
		let Some(index) = self.pending_renderables.iter().position(|pending| pending.handle == handle) else {
			return;
		};
		let pending = self.pending_renderables.swap_remove(index);
		if pending.key != replacement_key {
			self.cancel_if_orphaned(pending);
		}
	}

	/// Removes pending scene state and cancels work that has no remaining consumer.
	fn remove_pending_and_cancel_orphan(&mut self, handle: Handle) {
		let Some(index) = self.pending_renderables.iter().position(|pending| pending.handle == handle) else {
			return;
		};
		let pending = self.pending_renderables.swap_remove(index);
		self.cancel_if_orphaned(pending);
	}

	/// Cancels an unfinished mesh revision only when no other instance still waits for it.
	fn cancel_if_orphaned(&mut self, pending: PendingRenderable) {
		if !self.resident_meshes.contains_key(&pending.key)
			&& !self
				.pending_renderables
				.iter()
				.any(|other| other.resource == pending.resource)
			&& matches!(
				self.resource_loader.state(pending.resource),
				ResourceState::Queued | ResourceState::Loading
			) {
			self.resource_loader.cancel(pending.resource);
		}
	}

	/// Allocates one resident instance and initializes its retained transform.
	fn add_resident_instance(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, resident: ResidentSimpleMesh) {
		let instance = self.resource_store.add_instance(&resident, handle);
		let instance_data = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);
		if instance.index() >= instance_data.len() {
			self.resource_store.remove_instance(instance);
			log::error!("Simple instance storage is full. The most likely cause is more than 1,024 live renderable instances.");
			return;
		}
		let transform = self.renderable_transforms.get(&handle).cloned().unwrap_or_default();
		instance_data[instance.index()] = transform.get_matrix().into();
	}

	/// Creates scene instances whose shared mesh uploads completed at the frame boundary.
	fn resolve_pending_renderables(&mut self, frame: &mut ghi::implementation::Frame) {
		let mut index = 0usize;
		while index < self.pending_renderables.len() {
			let key = self.pending_renderables[index].key;
			let Some(resident) = self.resident_meshes.get(&key).copied() else {
				index += 1;
				continue;
			};
			let pending = self.pending_renderables.swap_remove(index);
			self.add_resident_instance(frame, pending.handle, resident);
		}
	}
}

impl crate::rendering::pipeline_manager::PipelineManager for PipelineManager {
	fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool {
		for (token, resident) in self.uploads.retire_frame(completed_frame, &mut self.resource_loader) {
			let key = *self
				.resource_loader
				.key(token.reference())
				.expect("Completed Simple uploads retain their logical loader key.");
			self.resident_meshes.insert(key, resident);
		}

		for _ in 0..64 {
			let Some(completion) = self.resource_loader.take_completion() else {
				break;
			};
			let token = completion.token();
			match completion.into_result() {
				Ok(prepared) => self.uploads.enqueue(token, prepared),
				Err(error) => {
					let key = self
						.resource_loader
						.key(token.reference())
						.map(ToString::to_string)
						.unwrap_or_else(|| "unknown".to_string());
					log::error!("Simple mesh preparation failed for '{key}': {error}");
				}
			}
		}
		self.resource_loader.submit_requests(64);
		self.uploads.has_pending()
	}

	fn record_frame_uploads(&mut self, frame: ghi::FrameKey, recording: &mut ghi::implementation::CommandBufferRecording<'_>) {
		for (token, error) in self
			.uploads
			.record_frame(frame, recording, &mut self.resource_loader, &mut self.resource_store)
		{
			let key = self
				.resource_loader
				.key(token.reference())
				.map(ToString::to_string)
				.unwrap_or_else(|| "unknown".to_string());
			log::error!("Simple mesh upload failed for '{key}': {error}");
		}
	}

	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>> {
		let pipeline = self.pipeline_manager.pipeline(self.pipeline)?;
		self.resolve_pending_renderables(frame);
		let instance_batches = self.resource_store.instance_batches_in(frame_allocator);

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
					sink_state.prepare(frame, sink, self, pipeline, instance_batches),
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

use ghi::context::{Context as _, ContextCreate as _};
use math::AffineShaderMatrix;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};

use crate::{
	core::factory::Handle,
	gameplay::transform::Transform,
	rendering::{
		RenderableMesh, Sink,
		pipelines::simple::{
			CameraShaderData, RenderPass,
			resource_manager::{PreparedSimpleMesh, ResidentSimpleMesh, SimpleMeshResource, SimpleResourceStore},
		},
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassReturn},
		renderable::mesh::MeshKey,
		resource_loading::{FrameUploadQueue, ResourceLoader, ResourceRef, ResourceState},
	},
};

#[cfg(test)]
mod tests {

	use besl::vm::{
		DescriptorBindings, ResourceSlot, Value, builtin_instance_index_slot, builtin_position_slot, input_slot, output_slot,
	};
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
		let program = compile(create_simple_vertex_program());

		let mut cameras = buffer(&program, ResourceSlot::new(0));

		let mut instances = buffer(&program, ResourceSlot::new(1));

		let mut input_position = input_buffer(&program, 0);

		let mut input_instance = buffer(&program, builtin_instance_index_slot());

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
