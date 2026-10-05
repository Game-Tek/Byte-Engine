//! Asynchronous pipeline compilation and frame-boundary publication.

/// The `PipelineRef` struct keeps a stable reference to a requested pipeline.
///
/// The manager derives it from the resource ID passed to [`PipelineManagerClient::request_pipeline`], or from every
/// input of a [`SpecializedComputePipelineRequest`]. Requests with equal inputs share one compiled pipeline.
///
/// Poll it with [`PipelineManagerClient::get`] during frame preparation. A
/// compiled pipeline becomes visible only after the renderer publishes results
/// at the start of a frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineRef(u64);

/// The `SpecializedComputePipelineRequest` struct describes a specialized compute pipeline by everything it compiles from.
///
/// Material variants that use the same shader, specialization constants, and
/// push-constant ranges share one pipeline, even when they bind different
/// textures. Pass this request to
/// [`PipelineManagerClient::request_specialized_compute_pipeline`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SpecializedComputePipelineRequest {
	shader_id: String,
	specialization: Vec<ghi::pipelines::SpecializationMapEntry>,
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
}

impl SpecializedComputePipelineRequest {
	/// Creates a request from the compute shader resource ID and the constants that specialize it.
	pub(crate) fn new(
		shader_id: impl Into<String>,
		specialization: Vec<ghi::pipelines::SpecializationMapEntry>,
		push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	) -> Self {
		Self {
			shader_id: shader_id.into(),
			specialization,
			push_constant_ranges,
		}
	}
}

/// The `PipelineState` enum reports the published state of a pipeline request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineState {
	/// Compilation is queued or still running.
	Pending,
	/// Compilation succeeded and published this pipeline handle.
	Ready(ghi::PipelineHandle),
	/// Loading or compilation failed.
	Failed,
}

/// The `ComputePipeline` struct provides a compiled handle and its reflected dispatch contract.
pub(crate) struct ComputePipeline {
	pub(crate) handle: ghi::PipelineHandle,
	pub(crate) workgroup: utils::Extent,
	pub(crate) bindings: Arc<[Binding]>,
}

/// The `ComputePipelines` struct lends published pipeline entries under one read lock, so descriptor adoption reads
/// reflected bindings without copying them and pollers check many pipeline states with one lock.
pub(crate) struct ComputePipelines<'a>(utils::sync::RwLockReadGuard<'a, HashMap<PipelineRef, PipelineEntry>>);

impl ComputePipelines<'_> {
	/// Returns a published compute pipeline, or `None` while it is unavailable.
	pub(crate) fn get(&self, pipeline: PipelineRef) -> Option<&ComputePipeline> {
		self.0.get(&pipeline).and_then(|entry| entry.compute.as_ref())
	}

	/// Returns the state published for a pipeline, so a caller that polls many pipelines takes the lock once.
	pub(crate) fn state(&self, pipeline: PipelineRef) -> PipelineState {
		self.0.get(&pipeline).map_or(PipelineState::Failed, |entry| entry.state)
	}
}

/// The `PipelineManagerClient` struct lets renderer dependants request and poll
/// asynchronously compiled pipelines without blocking.
///
/// Clone this client for each dependant. Requests with the same inputs
/// are coalesced before they reach a compilation server.
#[derive(Clone)]
pub struct PipelineManagerClient {
	shared: Arc<PipelineManagerShared>,
	requests: kanal::Sender<PipelineRequest>,
}

impl PipelineManagerClient {
	/// Requests a pipeline resource without waiting for storage or compilation.
	pub fn request_pipeline(&self, id: &str) -> PipelineRef {
		self.request(PipelineRequestKind::Resource { id: id.to_string() })
	}

	/// Requests a specialized compute pipeline without waiting for shader loading or compilation.
	///
	/// Equal requests return the same [`PipelineRef`] and compile once.
	pub(crate) fn request_specialized_compute_pipeline(&self, request: SpecializedComputePipelineRequest) -> PipelineRef {
		self.request(PipelineRequestKind::SpecializedCompute(request))
	}

	/// Returns the state published for a pipeline without draining worker results.
	pub fn get(&self, pipeline: PipelineRef) -> PipelineState {
		self.compute_pipelines().state(pipeline)
	}

	/// Returns the revision most recently published for a stable pipeline reference.
	pub(crate) fn revision(&self, pipeline: PipelineRef) -> u64 {
		self.shared
			.entries
			.read()
			.get(&pipeline)
			.map_or(0, |entry| entry.published_revision)
	}

	/// Recompiles requests rooted at a resource replaced by development asset baking.
	///
	/// A pipeline resource inherits its shaders' sources, so a shader edit updates the pipeline root without naming
	/// the shader. Clear every prepared shader so the rebuilt pipelines read the new bytes.
	#[cfg(debug_assertions)]
	pub(crate) fn resource_updated(&self, id: &str) {
		self.shared.shaders.lock().clear();
		let requests = {
			let mut entries = self.shared.entries.write();
			entries
				.iter_mut()
				.filter_map(|(key, entry)| {
					entry.kind.depends_on(id).then(|| {
						entry.requested_revision += 1;
						PipelineRequest {
							key: *key,
							revision: entry.requested_revision,
							kind: entry.kind.clone(),
						}
					})
				})
				.collect::<Vec<_>>()
		};

		for request in requests {
			if self.requests.send(request).is_err() {
				log::error!(
					"Pipeline rebuild request failed. The most likely cause is that every pipeline compilation server has stopped."
				);
			}
		}
	}

	/// Returns a published pipeline handle, or `None` while it is unavailable.
	pub fn pipeline(&self, pipeline: PipelineRef) -> Option<ghi::PipelineHandle> {
		match self.get(pipeline) {
			PipelineState::Ready(handle) => Some(handle),
			PipelineState::Pending | PipelineState::Failed => None,
		}
	}

	/// Borrows the published compute pipelines and the metadata needed for descriptor adoption.
	///
	/// Keep the returned view short-lived: it holds a read lock on every pipeline entry that blocks
	/// [`PipelineManager::publish`] and new requests until it is dropped. Do not call other client methods while
	/// holding it.
	pub(crate) fn compute_pipelines(&self) -> ComputePipelines<'_> {
		ComputePipelines(self.shared.entries.read())
	}

	/// Coalesces a request before placing compilation work on the shared queue.
	fn request(&self, kind: PipelineRequestKind) -> PipelineRef {
		use std::hash::{Hash as _, Hasher as _};

		// The request kind is part of the hash, so resource and specialized requests never coalesce.
		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		kind.hash(&mut hasher);
		let key = PipelineRef(hasher.finish());
		{
			let mut entries = self.shared.entries.write();
			if entries.contains_key(&key) {
				return key;
			}
			entries.insert(
				key,
				PipelineEntry {
					state: PipelineState::Pending,
					requested_revision: 0,
					published_revision: 0,
					kind: kind.clone(),
					compute: None,
				},
			);
		}

		if self.requests.send(PipelineRequest { key, revision: 0, kind }).is_err() {
			self.shared.entries.write().get_mut(&key).unwrap().state = PipelineState::Failed;
			log::error!(
				"Pipeline request failed. The most likely cause is that every pipeline compilation server has stopped."
			);
		}

		key
	}
}

/// The `PipelineManagerServer` struct compiles requests using one detached GHI
/// factory.
///
/// Call [`Self::run`] directly from a dedicated thread. The server does not own
/// or spawn that thread, so a future thread pool can run the same work loop.
pub struct PipelineManagerServer {
	factory: ghi::implementation::Factory,
	shared: Arc<PipelineManagerShared>,
	/// Shaders this server's factory already created, keyed by shader resource ID.
	///
	/// Each handle is reused only while the shared cache still hands out the same
	/// prepared shader, so a rebaked shader gets a new handle.
	shaders: HashMap<String, (Arc<PreparedShader>, ghi::ShaderHandle)>,
	requests: kanal::AsyncReceiver<PipelineRequest>,
	completions: kanal::Sender<PipelineCompletion>,
}

impl PipelineManagerServer {
	/// Compiles requests with shaders and pipelines read from `resources` until every client sender is dropped.
	pub async fn run(mut self, resources: crate::core::EntityHandle<resource_management::ResourceManager>) {
		while let Ok(PipelineRequest { key, revision, kind }) = self.requests.recv().await {
			let result = match kind {
				PipelineRequestKind::Resource { id } => self.compile_resource_pipeline(&resources, &id).await,
				PipelineRequestKind::SpecializedCompute(request) => {
					self.compile_specialized_compute_pipeline(&resources, request).await
				}
			};
			if self.completions.send(PipelineCompletion { key, revision, result }).is_err() {
				break;
			}
		}
	}

	/// Loads one complete pipeline dependency graph before performing native compilation.
	async fn compile_resource_pipeline(
		&mut self,
		resources: &resource_management::ResourceManager,
		id: &str,
	) -> Result<DetachedPipeline, String> {
		use ghi::{Device as _, pipelines::raster};
		use resource_management::resources::pipeline::{CullMode, FaceWinding, FillMode, PipelineKind};

		let pipeline: resource_management::Reference<resource_management::resources::pipeline::Pipeline> =
			resources.request(id).await.map_err(|_| {
				format!(
					"Pipeline resource '{id}' could not be loaded. The most likely cause is that the pipeline asset was not baked."
				)
			})?;
		let pipeline = pipeline.resource();
		let (PipelineKind::Compute { push_constants, .. } | PipelineKind::Raster { push_constants, .. }) = &pipeline.kind;
		let ranges = push_constants
			.iter()
			.map(|range| ghi::pipelines::PushConstantRange::new(range.offset, range.size))
			.collect::<Vec<_>>();
		match &pipeline.kind {
			PipelineKind::Compute { shader, .. } => {
				let prepared = shared_shader(&self.shared, resources, shader).await?;
				self.compile_compute(&prepared, &ranges, &[], &pipeline.name, || {
					format!(
						"Compute pipeline '{id}' has no workgroup size. The most likely cause is missing shader workgroup metadata."
					)
				})
			}
			PipelineKind::Raster {
				shaders,
				vertex_elements,
				attachments,
				face_winding,
				cull_mode,
				fill_mode,
				depth_write,
				..
			} => {
				// Shader reads and debug bakes are independent of mutable GHI state, so
				// prepare every shader before adopting handles in descriptor order.
				let prepared =
					utils::r#async::try_join_all(shaders.iter().map(|shader| shared_shader(&self.shared, resources, shader)))
						.await?;
				let loaded = prepared
					.iter()
					.map(|shader| self.adopt_shader(shader))
					.collect::<Result<Vec<_>, _>>()?;
				let parameters = loaded
					.iter()
					.map(|(handle, stage)| ghi::ShaderParameter::new(handle, *stage))
					.collect::<Vec<_>>();
				let vertices = vertex_elements
					.iter()
					.map(|element| {
						ghi::pipelines::VertexElement::new(&element.name, data_type(element.format), element.binding)
					})
					.collect::<Vec<_>>();
				let targets = attachments.iter().map(attachment).collect::<Vec<_>>();
				let builder = raster::Builder::new(&ranges, &vertices, &parameters, &targets)
					.name(&pipeline.name)
					.face_winding(match face_winding {
						FaceWinding::Clockwise => raster::FaceWinding::Clockwise,
						FaceWinding::CounterClockwise => raster::FaceWinding::CounterClockwise,
					})
					.cull_mode(match cull_mode {
						CullMode::None => raster::CullMode::None,
						CullMode::Front => raster::CullMode::Front,
						CullMode::Back => raster::CullMode::Back,
					})
					.fill_mode(match fill_mode {
						FillMode::Solid => raster::FillMode::Solid,
						FillMode::Wireframe => raster::FillMode::Wireframe,
					})
					.depth_write(*depth_write);
				Ok(DetachedPipeline::Raster(self.factory.create_raster_pipeline(builder)))
			}
		}
	}

	/// Creates a specialized detached compute pipeline from its shader and constants.
	async fn compile_specialized_compute_pipeline(
		&mut self,
		resources: &resource_management::ResourceManager,
		request: SpecializedComputePipelineRequest,
	) -> Result<DetachedPipeline, String> {
		let SpecializedComputePipelineRequest {
			shader_id,
			specialization,
			push_constant_ranges,
		} = request;
		let prepared = shared_shader(&self.shared, resources, &shader_id).await?;
		if !matches!(prepared.stage, ghi::ShaderTypes::Compute) {
			return Err(format!(
				"Specialized compute pipeline uses non-compute shader '{shader_id}'. The most likely cause is that the material variant references the wrong shader stage."
			));
		}
		self.compile_compute(&prepared, &push_constant_ranges, &specialization, &shader_id, || {
			format!(
				"Specialized compute shader '{shader_id}' has no workgroup size. The most likely cause is missing shader workgroup metadata."
			)
		})
	}

	/// Creates a detached compute pipeline from a prepared shader, failing with `missing_workgroup` when the shader
	/// has no workgroup size.
	fn compile_compute(
		&mut self,
		prepared: &Arc<PreparedShader>,
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		specialization: &[ghi::pipelines::SpecializationMapEntry],
		name: &str,
		missing_workgroup: impl FnOnce() -> String,
	) -> Result<DetachedPipeline, String> {
		use ghi::Device as _;

		let workgroup = prepared.workgroup.ok_or_else(missing_workgroup)?;
		let (shader, stage) = self.adopt_shader(prepared)?;
		let shader = ghi::ShaderParameter::new(&shader, stage).with_specialization_map(specialization);

		Ok(DetachedPipeline::Compute {
			pipeline: self
				.factory
				.create_compute_pipeline(ghi::pipelines::compute::Builder::new(push_constant_ranges, shader).name(name)),
			workgroup,
			bindings: prepared.bindings.clone(),
		})
	}

	/// Returns this factory's shader handle for a prepared shader, creating the native shader only once.
	fn adopt_shader(&mut self, prepared: &Arc<PreparedShader>) -> Result<(ghi::ShaderHandle, ghi::ShaderTypes), String> {
		use ghi::Device as _;

		if let Some((adopted, handle)) = self.shaders.get(&prepared.id)
			&& Arc::ptr_eq(adopted, prepared)
		{
			return Ok((*handle, prepared.stage));
		}
		let source = shader_artifact_source(&prepared.artifact, prepared.workgroup, prepared.backing.as_slice())?;
		let handle = self
			.factory
			.create_shader(
				Some(&prepared.id),
				source,
				prepared.stage,
				prepared.descriptors.iter().copied(),
			)
			.map_err(|_| {
				format!(
					"Shader '{}' could not be created. The most likely cause is an incompatible persisted interface.",
					prepared.id
				)
			})?;
		self.shaders.insert(prepared.id.clone(), (Arc::clone(prepared), handle));
		Ok((handle, prepared.stage))
	}
}

/// The `PreparedShader` struct keeps resource-owned shader inputs ready for ordered GHI adoption.
///
/// It is immutable once prepared, so every compilation server shares one copy through [`shared_shader`].
struct PreparedShader {
	id: String,
	stage: ghi::ShaderTypes,
	artifact: resource_management::resources::material::ShaderArtifact,
	workgroup: Option<utils::Extent>,
	descriptors: Vec<ghi::shader::ShaderResourceDescriptor>,
	bindings: Arc<[Binding]>,
	backing: resource_management::resource::reader::ResourceReaderBacking,
}

/// The `ShaderPreparation` type is one shader's pending or finished preparation, shared by every compilation server.
type ShaderPreparation = announcement::Announcement<Result<Arc<PreparedShader>, String>>;

/// Returns the prepared shader for `id`, reading the resource only when no server prepared it yet.
///
/// The first server to ask reads the shader. Servers that ask while it reads wait for its result, and later
/// requests reuse it, failures included. Development rebakes clear the cache.
async fn shared_shader(
	shared: &PipelineManagerShared,
	resources: &resource_management::ResourceManager,
	id: &str,
) -> Result<Arc<PreparedShader>, String> {
	let role = {
		let mut shaders = shared.shaders.lock();
		match shaders.get(id) {
			Some(preparation) => Err(preparation.listener()),
			None => {
				let (announcer, preparation) = ShaderPreparation::new();
				shaders.insert(id.to_owned(), preparation);
				Ok(announcer)
			}
		}
	};

	match role {
		Err(listener) => listener.listen().await.map_err(|_| {
			format!(
				"Shader '{id}' preparation stopped before it finished. The most likely cause is that the compilation server preparing it shut down."
			)
		})?,
		Ok(announcer) => {
			let result = prepare_shader(resources, id).await.map(Arc::new);
			let _ = announcer.announce(result.clone());
			result
		}
	}
}

/// Converts one persisted shader stage into its GHI stage.
fn shader_type_to_ghi(stage: resource_management::types::ShaderTypes) -> ghi::ShaderTypes {
	match stage {
		resource_management::types::ShaderTypes::Vertex => ghi::ShaderTypes::Vertex,
		resource_management::types::ShaderTypes::Fragment => ghi::ShaderTypes::Fragment,
		resource_management::types::ShaderTypes::Compute => ghi::ShaderTypes::Compute,
		resource_management::types::ShaderTypes::Task => ghi::ShaderTypes::Task,
		resource_management::types::ShaderTypes::Mesh => ghi::ShaderTypes::Mesh,
		resource_management::types::ShaderTypes::RayGen => ghi::ShaderTypes::RayGen,
		resource_management::types::ShaderTypes::ClosestHit => ghi::ShaderTypes::ClosestHit,
		resource_management::types::ShaderTypes::AnyHit => ghi::ShaderTypes::AnyHit,
		resource_management::types::ShaderTypes::Intersection => ghi::ShaderTypes::Intersection,
		resource_management::types::ShaderTypes::Miss => ghi::ShaderTypes::Miss,
		resource_management::types::ShaderTypes::Callable => ghi::ShaderTypes::Callable,
	}
}

/// Converts one persisted binding into the descriptor used for detached shader creation.
fn binding_to_descriptor(binding: &Binding) -> ghi::ShaderResourceDescriptor {
	use resource_management::resources::material::{BindingKind, TextureView};

	let mut access = ghi::AccessPolicies::empty();
	access.set(ghi::AccessPolicies::READ, binding.read);
	access.set(ghi::AccessPolicies::WRITE, binding.write);
	let descriptor =
		|kind| ghi::ShaderResourceDescriptor::new(ghi::ResourceSlot::new(binding.slot), kind, binding.count, access);
	match binding.kind {
		BindingKind::StorageBuffer => descriptor(ghi::ResourceKind::StorageBuffer).buffer_stride(
			binding
				.buffer_stride
				.expect("Missing persisted storage-buffer stride. The most likely cause is a stale shader interface resource."),
		),
		BindingKind::CombinedImageSampler { view } => {
			descriptor(ghi::ResourceKind::CombinedImageSampler).texture_view_type(match view {
				TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
				TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
				TextureView::TextureCube => ghi::TextureViewTypes::TextureCube,
				TextureView::TextureCubeArray => ghi::TextureViewTypes::TextureCubeArray,
				TextureView::Texture3D => ghi::TextureViewTypes::Texture3D,
			})
		}
		BindingKind::StorageImage => descriptor(ghi::ResourceKind::StorageImage),
	}
}

/// Borrows persisted shader bytes in the source representation expected by GHI.
fn shader_artifact_source<'a>(
	artifact: &'a resource_management::resources::material::ShaderArtifact,
	workgroup_size: Option<utils::Extent>,
	bytes: &'a [u8],
) -> Result<ghi::shader::Sources<'a>, String> {
	use resource_management::resources::material::ShaderArtifact;

	let text = |language: &str| {
		std::str::from_utf8(bytes).map_err(|_| {
			format!("Failed to read baked {language} shader. The most likely cause is invalid UTF-8 shader bytes.")
		})
	};
	match artifact {
		ShaderArtifact::Spirv => Ok(ghi::shader::Sources::SPIRV(bytes)),
		ShaderArtifact::Dxil => Ok(ghi::shader::Sources::DXIL(bytes)),
		ShaderArtifact::Hlsl { entry_point } => Ok(ghi::shader::Sources::HLSL {
			source: text("HLSL")?,
			entry_point,
		}),
		ShaderArtifact::Msl { entry_point } => Ok(ghi::shader::Sources::MTL {
			source: text("MSL")?,
			entry_point,
		}),
		ShaderArtifact::Mtlb { entry_point } => Ok(ghi::shader::Sources::MTLB {
			binary: bytes,
			entry_point,
			threadgroup_size: workgroup_size,
		}),
	}
}

/// Loads one shader resource without borrowing mutable GHI factory state.
async fn prepare_shader(resources: &resource_management::ResourceManager, id: &str) -> Result<PreparedShader, String> {
	let mut shader: resource_management::Reference<resource_management::resources::material::Shader> = resources
		.request(id)
		.await
		.map_err(|error| format!("Could not load shader '{id}'. {error}"))?;
	let resource = shader.resource();
	let stage = shader_type_to_ghi(resource.stage);
	let artifact = resource.artifact.clone();
	let workgroup = resource
		.interface
		.workgroup_size
		.map(|(width, height, depth)| utils::Extent::new(width, height, depth));
	let descriptors = resource
		.interface
		.bindings
		.iter()
		.map(binding_to_descriptor)
		.collect::<Vec<_>>();
	let bindings = Arc::from(resource.interface.bindings.as_slice());
	let backing = shader.consume_reader().into_backing_storage().await.map_err(|_| {
		format!("Shader bytes for '{id}' could not be loaded. The most likely cause is an unsupported resource reader.")
	})?;
	// Validate persisted source metadata before mutable GHI adoption begins.
	let _ = shader_artifact_source(&artifact, workgroup, backing.as_slice())?;
	Ok(PreparedShader {
		id: id.to_string(),
		stage,
		artifact,
		workgroup,
		descriptors,
		bindings,
		backing,
	})
}

fn data_type(format: resource_management::resources::pipeline::Format) -> ghi::DataTypes {
	use resource_management::resources::pipeline::Format;
	match format {
		Format::Float => ghi::DataTypes::Float,
		Format::Float2 => ghi::DataTypes::Float2,
		Format::Float3 => ghi::DataTypes::Float3,
		Format::Float4 => ghi::DataTypes::Float4,
		Format::U16 => ghi::DataTypes::U16,
		_ => panic!("Pipeline vertex format is invalid. The most likely cause is an image format used as a vertex element."),
	}
}

fn attachment(value: &resource_management::resources::pipeline::Attachment) -> ghi::pipelines::raster::AttachmentDescriptor {
	use resource_management::resources::pipeline::{BlendMode, Format};
	let format = match value.format {
		Format::Rgba8Unorm => ghi::Formats::RGBA8UNORM,
		Format::Rgba16Unorm => ghi::Formats::RGBA16UNORM,
		Format::Rgba16Float => ghi::Formats::RGBA16F,
		Format::Rg11b10Float => ghi::Formats::RGBu11u11u10,
		Format::Depth16 => ghi::Formats::Depth16,
		Format::Depth32 => ghi::Formats::Depth32,
		Format::U32 => ghi::Formats::U32,
		_ => panic!("Pipeline attachment format is invalid. The most likely cause is a vertex format used as an attachment."),
	};
	let mut descriptor = ghi::pipelines::raster::AttachmentDescriptor::new(format).blend(match value.blend {
		BlendMode::None => ghi::pipelines::raster::BlendMode::None,
		BlendMode::Alpha => ghi::pipelines::raster::BlendMode::Alpha,
		BlendMode::Premultiplied => ghi::pipelines::raster::BlendMode::Premultiplied,
	});
	if let Some(layer) = value.layer {
		descriptor = descriptor.layer(layer);
	}
	descriptor
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Creates a client without a GHI factory so request behavior can be tested independently.
	fn client() -> (PipelineManagerClient, kanal::Receiver<PipelineRequest>) {
		let (requests, receiver) = kanal::unbounded();
		(
			PipelineManagerClient {
				shared: Arc::new(PipelineManagerShared::default()),
				requests,
			},
			receiver,
		)
	}

	#[test]
	fn duplicate_requests_enqueue_one_compilation() {
		let (client, requests) = client();
		let first = client.request_pipeline("pipeline/test");
		let second = client.request_pipeline("pipeline/test");

		assert_eq!(first, second);
		assert!(matches!(client.get(first), PipelineState::Pending));
		assert!(matches!(requests.try_recv(), Ok(Some(_))));
		assert!(matches!(requests.try_recv(), Ok(None)));
	}

	/// Builds a specialized request with one scalar constant, like a material variant with one factor.
	fn specialized(shader_id: &str, factor: f32) -> SpecializedComputePipelineRequest {
		SpecializedComputePipelineRequest::new(
			shader_id,
			vec![ghi::pipelines::SpecializationMapEntry::new(0, factor)],
			vec![ghi::pipelines::PushConstantRange::new(0, 16)],
		)
	}

	#[test]
	fn specialized_requests_with_equal_inputs_enqueue_one_compilation() {
		let (client, requests) = client();

		// Two material variants that differ only in their textures send equal requests.
		let first = client.request_specialized_compute_pipeline(specialized("shader/test", 1.0));
		let second = client.request_specialized_compute_pipeline(specialized("shader/test", 1.0));

		assert_eq!(first, second);
		assert!(matches!(client.get(first), PipelineState::Pending));
		let queued = requests
			.try_recv()
			.expect("specialized request receive")
			.expect("specialized request");
		let PipelineRequestKind::SpecializedCompute(request) = queued.kind else {
			panic!(
				"Unexpected pipeline request kind. The most likely cause is that the specialized client route sent a resource request."
			);
		};

		assert_eq!(request.shader_id, "shader/test");
		assert_eq!(request.push_constant_ranges.len(), 1);
		assert!(matches!(requests.try_recv(), Ok(None)));
	}

	#[test]
	fn specialized_requests_with_different_inputs_compile_separately() {
		let (client, requests) = client();

		let base = client.request_specialized_compute_pipeline(specialized("shader/test", 1.0));
		let other_constant = client.request_specialized_compute_pipeline(specialized("shader/test", 2.0));
		let other_shader = client.request_specialized_compute_pipeline(specialized("shader/other", 1.0));
		let other_push_constants = client.request_specialized_compute_pipeline(SpecializedComputePipelineRequest::new(
			"shader/test",
			vec![ghi::pipelines::SpecializationMapEntry::new(0, 1.0f32)],
			vec![ghi::pipelines::PushConstantRange::new(0, 32)],
		));

		assert_ne!(base, other_constant);
		assert_ne!(base, other_shader);
		assert_ne!(base, other_push_constants);
		assert_eq!(requests.len(), 4);
	}

	#[test]
	fn specialized_and_resource_requests_use_distinct_namespaces() {
		let (client, requests) = client();
		let resource = client.request_pipeline("shared/id");
		let specialized = client.request_specialized_compute_pipeline(SpecializedComputePipelineRequest::new(
			"shared/id",
			Vec::new(),
			Vec::new(),
		));

		assert_ne!(resource, specialized);
		let resource_request = requests
			.try_recv()
			.expect("resource request receive")
			.expect("resource request");

		assert!(matches!(
			resource_request.kind,
			PipelineRequestKind::Resource { ref id } if id == "shared/id"
		));
		let specialized_request = requests
			.try_recv()
			.expect("specialized request receive")
			.expect("specialized request");

		assert!(matches!(
			specialized_request.kind,
			PipelineRequestKind::SpecializedCompute(ref request)
				if request.shader_id == "shared/id"
		));
	}

	#[cfg(debug_assertions)]
	#[test]
	fn resource_updates_keep_stable_references_and_enqueue_new_revisions() {
		let (client, requests) = client();
		let reference = client.request_pipeline("pipeline/test");
		let initial = requests.try_recv().unwrap().unwrap();

		assert_eq!(initial.revision, 0);

		client.resource_updated("unrelated");

		assert!(matches!(requests.try_recv(), Ok(None)));
		client.resource_updated("pipeline/test");

		let rebuilt = requests.try_recv().unwrap().unwrap();

		assert_eq!(rebuilt.key, reference);
		assert_eq!(rebuilt.revision, 1);
		assert!(matches!(client.get(reference), PipelineState::Pending));
	}

	#[cfg(debug_assertions)]
	#[test]
	fn shader_updates_rebuild_every_specialized_pipeline_that_reads_the_shader() {
		let (client, requests) = client();
		let first = client.request_specialized_compute_pipeline(specialized("shader/test", 1.0));
		let second = client.request_specialized_compute_pipeline(specialized("shader/test", 2.0));
		let unrelated = client.request_specialized_compute_pipeline(specialized("shader/other", 1.0));
		while let Ok(Some(_)) = requests.try_recv() {}

		client.resource_updated("shader/test");

		let mut rebuilt = [
			requests.try_recv().unwrap().unwrap().key,
			requests.try_recv().unwrap().unwrap().key,
		];
		rebuilt.sort_by_key(|key| key.0);
		let mut expected = [first, second];
		expected.sort_by_key(|key| key.0);

		assert_eq!(rebuilt, expected);
		assert!(!rebuilt.contains(&unrelated));
		assert!(matches!(requests.try_recv(), Ok(None)));
	}
}

/// The `PipelineManager` struct owns compilation result publication for the
/// renderer.
pub(crate) struct PipelineManager {
	shared: Arc<PipelineManagerShared>,
	completions: kanal::Receiver<PipelineCompletion>,
}

impl PipelineManager {
	/// Creates a client and independent servers that may be moved directly onto threads.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		server_count: usize,
	) -> (PipelineManagerClient, Self, Vec<PipelineManagerServer>) {
		let (request_sender, request_receiver) = kanal::unbounded_async();
		let (completion_sender, completion_receiver) = kanal::unbounded();
		let shared = Arc::new(PipelineManagerShared::default());
		let servers = (0..server_count.max(1))
			.filter_map(|_| {
				context.create_factory().map(|factory| PipelineManagerServer {
					factory,
					shared: shared.clone(),
					shaders: HashMap::default(),
					requests: request_receiver.clone(),
					completions: completion_sender.clone(),
				})
			})
			.collect();

		(
			PipelineManagerClient {
				shared: shared.clone(),
				requests: request_sender.to_sync(),
			},
			Self {
				shared,
				completions: completion_receiver,
			},
			servers,
		)
	}

	/// Interns all completed work and publishes one stable availability snapshot.
	///
	/// Each completion takes the entry lock once. A completion for a superseded revision is dropped, and a failed
	/// rebuild keeps the previously published pipeline.
	pub(crate) fn publish(&mut self, frame: &mut ghi::implementation::Frame) {
		while let Ok(Some(completion)) = self.completions.try_recv() {
			let mut entries = self.shared.entries.write();
			let Some(entry) = entries
				.get_mut(&completion.key)
				.filter(|entry| entry.requested_revision == completion.revision)
			else {
				continue;
			};
			match completion.result {
				Ok(DetachedPipeline::Compute {
					pipeline,
					workgroup,
					bindings,
				}) => {
					let handle = frame.intern_compute_pipeline(pipeline);
					entry.compute = Some(ComputePipeline {
						handle,
						workgroup,
						bindings,
					});
					entry.state = PipelineState::Ready(handle);
					entry.published_revision = completion.revision;
				}
				Ok(DetachedPipeline::Raster(pipeline)) => {
					entry.compute = None;
					entry.state = PipelineState::Ready(frame.intern_raster_pipeline(pipeline));
					entry.published_revision = completion.revision;
				}
				Err(reason) => {
					log::error!("Pipeline compilation failed: {reason}");
					if !matches!(entry.state, PipelineState::Ready(_)) {
						entry.state = PipelineState::Failed;
					}
				}
			}
		}
	}
}

#[derive(Default)]
struct PipelineManagerShared {
	entries: RwLock<HashMap<PipelineRef, PipelineEntry>>,
	/// Prepared shaders keyed by resource ID, see [`shared_shader`].
	shaders: Mutex<HashMap<String, ShaderPreparation>>,
}

/// The `PipelineEntry` struct is the published state of one coalesced request, kept under one lock.
struct PipelineEntry {
	state: PipelineState,
	requested_revision: u64,
	published_revision: u64,
	kind: PipelineRequestKind,
	/// The dispatch contract of the latest compute pipeline; `None` until one publishes or for raster pipelines.
	compute: Option<ComputePipeline>,
}

struct PipelineRequest {
	key: PipelineRef,
	revision: u64,
	kind: PipelineRequestKind,
}

#[derive(Clone, Hash)]
enum PipelineRequestKind {
	Resource { id: String },
	SpecializedCompute(SpecializedComputePipelineRequest),
}

impl PipelineRequestKind {
	/// Returns whether a replaced root resource supplies this request's compilation inputs.
	#[cfg(debug_assertions)]
	fn depends_on(&self, id: &str) -> bool {
		match self {
			Self::Resource { id: pipeline_id } => pipeline_id == id,
			Self::SpecializedCompute(request) => request.shader_id == id,
		}
	}
}

struct PipelineCompletion {
	key: PipelineRef,
	revision: u64,
	result: Result<DetachedPipeline, String>,
}

enum DetachedPipeline {
	Compute {
		pipeline: ghi::factory::ComputePipeline,
		workgroup: utils::Extent,
		bindings: Arc<[Binding]>,
	},
	Raster(ghi::factory::RasterPipeline),
}

use std::sync::Arc;

use resource_management::resources::material::Binding;
use utils::{
	hash::HashMap,
	sync::{Mutex, RwLock},
};
