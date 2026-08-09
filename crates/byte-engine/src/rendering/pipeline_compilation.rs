//! Asynchronous pipeline compilation and frame-boundary publication.

/// The `PipelineKey` struct identifies one complete pipeline compilation input.
///
/// The manager derives this value from the resource ID passed to
/// [`PipelineManagerClient::request_pipeline`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineKey(u64);

impl PipelineKey {
	/// Creates a stable key from a hash of the complete pipeline description.
	pub const fn new(value: u64) -> Self {
		Self(value)
	}
}

/// The `PipelineRef` struct keeps a stable reference to a requested pipeline.
///
/// Poll it with [`PipelineManagerClient::get`] during frame preparation. A
/// compiled pipeline becomes visible only after the renderer publishes results
/// at the start of a frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineRef(PipelineKey);

/// The `PipelineState` enum reports the published state of a pipeline request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineState {
	Pending,
	Ready(ghi::PipelineHandle),
	Failed,
}

/// The `ComputePipeline` struct provides a compiled handle and its reflected dispatch contract.
#[derive(Clone)]
pub(crate) struct ComputePipeline {
	pub(crate) handle: ghi::PipelineHandle,
	pub(crate) workgroup: utils::Extent,
	pub(crate) bindings: Arc<[resource_management::shader::besl::evaluation::BindingUsage]>,
}

/// The `PipelineManagerClient` struct lets renderer dependants request and poll
/// asynchronously compiled pipelines without blocking.
///
/// Clone this client for each dependant. Requests with the same [`PipelineKey`]
/// are coalesced before they reach a compilation server.
#[derive(Clone)]
pub struct PipelineManagerClient {
	shared: Arc<PipelineManagerShared>,
	requests: kanal::Sender<PipelineRequest>,
}

impl PipelineManagerClient {
	/// Requests a pipeline resource without waiting for storage or compilation.
	pub fn request_pipeline(&self, id: &str) -> PipelineRef {
		use std::hash::{Hash as _, Hasher as _};

		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		id.hash(&mut hasher);
		self.request(PipelineKey::new(hasher.finish()), id.to_string())
	}

	/// Returns the state published for a pipeline without draining worker results.
	pub fn get(&self, pipeline: PipelineRef) -> PipelineState {
		self.shared
			.entries
			.read()
			.get(&pipeline.0)
			.copied()
			.unwrap_or(PipelineState::Failed)
	}

	/// Returns a published pipeline handle, or `None` while it is unavailable.
	pub fn pipeline(&self, pipeline: PipelineRef) -> Option<ghi::PipelineHandle> {
		match self.get(pipeline) {
			PipelineState::Ready(handle) => Some(handle),
			PipelineState::Pending | PipelineState::Failed => None,
		}
	}

	/// Returns a published compute pipeline with the metadata needed for descriptor adoption.
	pub(crate) fn compute_pipeline(&self, pipeline: PipelineRef) -> Option<ComputePipeline> {
		self.shared.compute_pipelines.read().get(&pipeline.0).cloned()
	}

	/// Coalesces a request before placing compilation work on the shared queue.
	fn request(&self, key: PipelineKey, id: String) -> PipelineRef {
		let reference = PipelineRef(key);
		{
			let mut entries = self.shared.entries.write();
			if entries.contains_key(&key) {
				return reference;
			}
			entries.insert(key, PipelineState::Pending);
		}

		if self.requests.send(PipelineRequest { key, id }).is_err() {
			self.shared.entries.write().insert(key, PipelineState::Failed);
			log::error!(
				"Pipeline request failed. The most likely cause is that every pipeline compilation server has stopped."
			);
		}

		reference
	}
}

/// The `PipelineManagerServer` struct compiles requests using one detached GHI
/// factory.
///
/// Call [`Self::run`] directly from a dedicated thread. The server does not own
/// or spawn that thread, so a future thread pool can run the same work loop.
pub struct PipelineManagerServer {
	factory: ghi::implementation::Factory,
	resource_manager: Option<crate::core::EntityHandle<resource_management::ResourceManager>>,
	requests: kanal::AsyncReceiver<PipelineRequest>,
	completions: kanal::Sender<PipelineCompletion>,
}

impl PipelineManagerServer {
	/// Compiles requests until every client sender is dropped.
	pub async fn run(mut self) {
		while let Ok(request) = self.requests.recv().await {
			let key = request.key;
			let result = self.compile(&request.id).await;
			if self.completions.send(PipelineCompletion { key, result }).is_err() {
				break;
			}
		}
	}

	/// Connects this server to the resource manager before its worker starts.
	pub(crate) fn set_resource_manager(
		&mut self,
		resource_manager: crate::core::EntityHandle<resource_management::ResourceManager>,
	) {
		self.resource_manager = Some(resource_manager);
	}

	/// Loads one complete pipeline dependency graph before performing native compilation.
	async fn compile(&mut self, id: &str) -> Result<DetachedPipeline, String> {
		use ghi::Device as _;
		use resource_management::resources::pipeline::PipelineKind;

		let resources = self.resource_manager.as_ref().ok_or_else(|| {
			"Pipeline compilation failed. The most likely cause is that the renderer did not configure its resource manager.".to_string()
		})?;
		let pipeline: resource_management::Reference<resource_management::resources::pipeline::Pipeline> =
			resources.request(id).await.map_err(|_| {
				format!("Pipeline resource '{id}' could not be loaded. The most likely cause is that the pipeline asset was not baked.")
			})?;
		match &pipeline.resource().kind {
			PipelineKind::Compute { shader, push_constants } => {
				let prepared = prepare_shader(resources, shader).await?;
				let workgroup = prepared.workgroup.ok_or_else(|| {
					format!("Compute pipeline '{id}' has no workgroup size. The most likely cause is missing shader workgroup metadata.")
				})?;
				let bindings = prepared.bindings.clone();
				let (shader, stage) = adopt_shader(&mut self.factory, prepared)?;
				let ranges = push_constants
					.iter()
					.map(|range| ghi::pipelines::PushConstantRange::new(range.offset, range.size))
					.collect::<Vec<_>>();
				Ok(DetachedPipeline::Compute {
					pipeline: self.factory.create_compute_pipeline(
						ghi::pipelines::compute::Builder::new(&ranges, ghi::ShaderParameter::new(&shader, stage))
							.name(&pipeline.resource().name),
					),
					workgroup: utils::Extent::new(workgroup.0, workgroup.1, workgroup.2),
					bindings,
				})
			}
			PipelineKind::Raster {
				shaders,
				push_constants,
				vertex_elements,
				attachments,
				face_winding,
				cull_mode,
				depth_write,
			} => {
				// Resource reads and debug bakes are independent of mutable GHI state, so
				// prepare every shader before adopting handles in descriptor order.
				let prepared =
					utils::r#async::try_join_all(shaders.iter().map(|shader| prepare_shader(resources, shader))).await?;
				let loaded = prepared
					.into_iter()
					.map(|shader| adopt_shader(&mut self.factory, shader))
					.collect::<Result<Vec<_>, _>>()?;
				let parameters = loaded
					.iter()
					.map(|(handle, stage)| ghi::ShaderParameter::new(handle, *stage))
					.collect::<Vec<_>>();
				let ranges = push_constants
					.iter()
					.map(|range| ghi::pipelines::PushConstantRange::new(range.offset, range.size))
					.collect::<Vec<_>>();
				let vertices = vertex_elements
					.iter()
					.map(|element| {
						ghi::pipelines::VertexElement::new(&element.name, data_type(element.format), element.binding)
					})
					.collect::<Vec<_>>();
				let targets = attachments.iter().map(attachment).collect::<Vec<_>>();
				let builder = ghi::pipelines::raster::Builder::new(&ranges, &vertices, &parameters, &targets)
					.name(&pipeline.resource().name)
					.face_winding(match face_winding {
						resource_management::resources::pipeline::FaceWinding::Clockwise => {
							ghi::pipelines::raster::FaceWinding::Clockwise
						}
						resource_management::resources::pipeline::FaceWinding::CounterClockwise => {
							ghi::pipelines::raster::FaceWinding::CounterClockwise
						}
					})
					.cull_mode(match cull_mode {
						resource_management::resources::pipeline::CullMode::None => ghi::pipelines::raster::CullMode::None,
						resource_management::resources::pipeline::CullMode::Front => ghi::pipelines::raster::CullMode::Front,
						resource_management::resources::pipeline::CullMode::Back => ghi::pipelines::raster::CullMode::Back,
					})
					.depth_write(*depth_write);
				Ok(DetachedPipeline::Raster(self.factory.create_raster_pipeline(builder)))
			}
		}
	}
}

/// The `PreparedShader` struct keeps resource-owned shader inputs ready for ordered GHI adoption.
struct PreparedShader {
	id: String,
	stage: ghi::ShaderTypes,
	artifact: resource_management::resources::material::ShaderArtifact,
	workgroup: Option<(u32, u32, u32)>,
	descriptors: Vec<ghi::shader::ShaderResourceDescriptor>,
	bindings: Arc<[resource_management::shader::besl::evaluation::BindingUsage]>,
	backing: resource_management::resource::reader::ResourceReaderBacking,
}

/// Loads one shader resource without borrowing mutable GHI factory state.
async fn prepare_shader(resources: &resource_management::ResourceManager, id: &str) -> Result<PreparedShader, String> {
	use resource_management::resource::ReadStorageBackend as _;

	let mut shader: resource_management::Reference<resource_management::resources::material::Shader> =
		resources.request(id).await.map_err(|_| {
			format!("Shader resource '{id}' could not be loaded. The most likely cause is that the shader asset was not baked.")
		})?;
	let stage = crate::rendering::resource_loading::shader_type_to_ghi(shader.resource().stage);
	let artifact = shader.resource().artifact.clone();
	let workgroup = shader.resource().interface.workgroup_size;
	let descriptors = shader
		.resource()
		.interface
		.bindings
		.iter()
		.map(crate::rendering::resource_loading::binding_to_descriptor)
		.collect::<Vec<_>>();
	let bindings = shader
		.resource()
		.interface
		.bindings
		.iter()
		.map(|binding| resource_management::shader::besl::evaluation::BindingUsage {
			name: binding.name.clone(),
			kind: binding.kind,
			count: binding.count,
			slot: binding.slot,
			buffer_stride: binding.buffer_stride,
			read: binding.read,
			write: binding.write,
		})
		.collect::<Vec<_>>()
		.into();
	let backing = shader.consume_reader().into_backing_storage().await.map_err(|_| {
		format!("Shader bytes for '{id}' could not be loaded. The most likely cause is an unsupported resource reader.")
	})?;
	// Validate persisted source metadata before mutable GHI adoption begins.
	let _ = crate::rendering::resource_loading::shader_artifact_source(&artifact, workgroup, backing.as_slice())?;
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

/// Creates one shader handle after asynchronous resource preparation has completed.
fn adopt_shader(
	factory: &mut ghi::implementation::Factory,
	prepared: PreparedShader,
) -> Result<(ghi::ShaderHandle, ghi::ShaderTypes), String> {
	use ghi::Device as _;

	let source = crate::rendering::resource_loading::shader_artifact_source(
		&prepared.artifact,
		prepared.workgroup,
		prepared.backing.as_slice(),
	)?;
	let handle = factory
		.create_shader(Some(&prepared.id), source, prepared.stage, prepared.descriptors)
		.map_err(|_| {
			format!(
				"Shader '{}' could not be created. The most likely cause is an incompatible persisted interface.",
				prepared.id
			)
		})?;
	Ok((handle, prepared.stage))
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
		Format::Depth16 => ghi::Formats::Depth16,
		Format::Depth32 => ghi::Formats::Depth32,
		Format::U32 => ghi::Formats::U32,
		_ => panic!("Pipeline attachment format is invalid. The most likely cause is a vertex format used as an attachment."),
	};
	let mut descriptor = ghi::pipelines::raster::AttachmentDescriptor::new(format).blend(match value.blend {
		BlendMode::None => ghi::pipelines::raster::BlendMode::None,
		BlendMode::Alpha => ghi::pipelines::raster::BlendMode::Alpha,
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
				shared: Arc::new(PipelineManagerShared {
					entries: RwLock::new(HashMap::new()),
					compute_pipelines: RwLock::new(HashMap::new()),
				}),
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

	#[test]
	fn unknown_pipeline_is_not_available() {
		let (client, _requests) = client();

		assert_eq!(client.get(PipelineRef(PipelineKey::new(7))), PipelineState::Failed);
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
		use ghi::context::ContextCreate as _;

		let (request_sender, request_receiver) = kanal::unbounded_async();
		let (completion_sender, completion_receiver) = kanal::unbounded();
		let shared = Arc::new(PipelineManagerShared {
			entries: RwLock::new(HashMap::new()),
			compute_pipelines: RwLock::new(HashMap::new()),
		});
		let servers = (0..server_count.max(1))
			.filter_map(|_| {
				context.create_factory().map(|factory| PipelineManagerServer {
					factory,
					resource_manager: None,
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
	pub(crate) fn publish(&mut self, frame: &mut ghi::implementation::Frame) {
		while let Ok(Some(completion)) = self.completions.try_recv() {
			let state = match completion.result {
				Ok(DetachedPipeline::Compute {
					pipeline,
					workgroup,
					bindings,
				}) => {
					let handle = frame.intern_compute_pipeline(pipeline);
					self.shared.compute_pipelines.write().insert(
						completion.key,
						ComputePipeline {
							handle,
							workgroup,
							bindings,
						},
					);
					PipelineState::Ready(handle)
				}
				Ok(DetachedPipeline::Raster(pipeline)) => PipelineState::Ready(frame.intern_raster_pipeline(pipeline)),
				Err(reason) => {
					log::error!("Pipeline compilation failed: {reason}");
					PipelineState::Failed
				}
			};
			self.shared.entries.write().insert(completion.key, state);
		}
	}
}

struct PipelineManagerShared {
	entries: RwLock<HashMap<PipelineKey, PipelineState>>,
	compute_pipelines: RwLock<HashMap<PipelineKey, ComputePipeline>>,
}

struct PipelineRequest {
	key: PipelineKey,
	id: String,
}

struct PipelineCompletion {
	key: PipelineKey,
	result: Result<DetachedPipeline, String>,
}

enum DetachedPipeline {
	Compute {
		pipeline: ghi::factory::ComputePipeline,
		workgroup: utils::Extent,
		bindings: Arc<[resource_management::shader::besl::evaluation::BindingUsage]>,
	},
	Raster(ghi::factory::RasterPipeline),
}

use std::sync::Arc;

use ghi::frame::Frame as _;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
};
