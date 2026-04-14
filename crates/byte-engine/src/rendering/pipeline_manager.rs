use std::{
	hash::Hash,
	panic::{catch_unwind, AssertUnwindSafe},
	sync::mpsc::{self, Receiver, Sender},
	thread,
	time::Duration,
};

use ghi::device::{Device as _, DeviceCreate as _};
use resource_management::{
	resources::material::{Material, Shader, Variant},
	types::ShaderTypes,
	Reference,
};
use utils::{
	hash::{HashMap, HashMapExt},
	stale_map::{Entry, StaleHashMap},
	sync::RwLock,
};

enum PipelineStatus {
	Pending,
	Ready(ghi::PipelineHandle),
	Failed,
}

#[derive(Clone)]
enum OwnedShaderSource {
	MTLB { binary: Box<[u8]>, entry_point: String },
	MTL { source: String, entry_point: String },
	SPIRV(Box<[u8]>),
}

impl OwnedShaderSource {
	fn sources(&self) -> ghi::shader::Sources<'_> {
		match self {
			OwnedShaderSource::MTLB { binary, entry_point } => ghi::shader::Sources::MTLB { binary, entry_point },
			OwnedShaderSource::MTL { source, entry_point } => ghi::shader::Sources::MTL { source, entry_point },
			OwnedShaderSource::SPIRV(binary) => ghi::shader::Sources::SPIRV(binary),
		}
	}
}

#[derive(Clone)]
struct OwnedShader {
	name: Option<String>,
	source: OwnedShaderSource,
	stage: ghi::ShaderTypes,
	binding_descriptors: Vec<ghi::shader::BindingDescriptor>,
}

struct ComputePipelineRequest {
	key: String,
	descriptor_set_templates: Vec<ghi::DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	shader: OwnedShader,
	specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
}

enum ComputePipelineResult {
	Ready {
		key: String,
		pipeline: ghi::implementation::ComputePipeline,
	},
	Failed {
		key: String,
	},
}

#[cfg(debug_assertions)]
const DEBUG_PIPELINE_CREATION_DELAY: Duration = Duration::from_millis(250);

pub struct PipelineManager {
	pipelines: RwLock<HashMap<String, PipelineStatus>>,
	shaders: RwLock<StaleHashMap<String, u64, (ghi::ShaderHandle, ghi::ShaderTypes)>>,
	// Async requests cannot reload shader bytes after a sync load consumes the read target,
	// so we keep an owned copy of the shader payload keyed by resource hash.
	shader_requests: RwLock<StaleHashMap<String, u64, OwnedShader>>,
	compute_pipeline_requests: Option<Sender<ComputePipelineRequest>>,
	compute_pipeline_results: Option<Receiver<ComputePipelineResult>>,
}

impl PipelineManager {
	/// The `PipelineManager` struct keeps material pipeline creation backend-agnostic while allowing backends with a factory to compile on a worker thread.
	pub fn new(device: &mut ghi::implementation::Device) -> Self {
		let (compute_pipeline_requests, compute_pipeline_results) = if let Some(factory) = device.create_pipeline_factory() {
			let (requests, results) = Self::spawn_compute_worker(factory);
			(Some(requests), Some(results))
		} else {
			(None, None)
		};

		Self {
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shaders: RwLock::new(StaleHashMap::with_capacity(1024)),
			shader_requests: RwLock::new(StaleHashMap::with_capacity(1024)),
			compute_pipeline_requests,
			compute_pipeline_results,
		}
	}

	fn spawn_compute_worker(
		factory: ghi::implementation::PipelineFactory,
	) -> (Sender<ComputePipelineRequest>, Receiver<ComputePipelineResult>) {
		let (request_sender, request_receiver) = mpsc::channel::<ComputePipelineRequest>();
		let (result_sender, result_receiver) = mpsc::channel::<ComputePipelineResult>();

		thread::spawn(move || {
			use ghi::pipelines::factory::Factory as _;

			let mut factory = factory;

			while let Ok(request) = request_receiver.recv() {
				let key = request.key.clone();
				let result = catch_unwind(AssertUnwindSafe(|| Self::compile_compute_pipeline(&mut factory, request)));

				let message = match result {
					Ok(Ok(pipeline)) => ComputePipelineResult::Ready { key, pipeline },
					Ok(Err(())) | Err(_) => ComputePipelineResult::Failed { key },
				};

				if result_sender.send(message).is_err() {
					break;
				}
			}
		});

		(request_sender, result_receiver)
	}

	fn compile_compute_pipeline(
		factory: &mut ghi::implementation::PipelineFactory,
		request: ComputePipelineRequest,
	) -> Result<ghi::implementation::ComputePipeline, ()> {
		use ghi::pipelines::factory::Factory as _;

		Self::sleep_for_debug_pipeline_delay();

		let shader = request.shader;
		let shader_handle = factory.create_shader(
			shader.name.as_deref(),
			shader.source.sources(),
			shader.stage,
			shader.binding_descriptors,
		)?;

		Ok(factory.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&request.descriptor_set_templates,
			&request.push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, shader.stage)
				.with_specialization_map(&request.specialization_map_entries),
		)))
	}

	fn queue_compute_pipeline(&self, request: ComputePipelineRequest) {
		let key = request.key.clone();

		let Some(compute_pipeline_requests) = self.compute_pipeline_requests.as_ref() else {
			self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
			log::error!(
				"Async pipeline requests are unavailable for {}. The most likely cause is that the active backend does not expose a pipeline factory.",
				key
			);
			return;
		};

		if compute_pipeline_requests.send(request).is_err() {
			self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
			log::error!(
				"Async pipeline request channel closed for {}. The most likely cause is that the compilation worker terminated unexpectedly.",
				key
			);
		}
	}

	fn sleep_for_debug_pipeline_delay() {
		#[cfg(debug_assertions)]
		thread::sleep(DEBUG_PIPELINE_CREATION_DELAY);
	}

	pub fn poll(&mut self, frame: &mut ghi::implementation::Frame, max_results: usize) -> Vec<(String, ghi::PipelineHandle)> {
		let Some(compute_pipeline_results) = self.compute_pipeline_results.as_ref() else {
			return Vec::new();
		};

		let mut resolved_pipelines = Vec::with_capacity(max_results.min(16));

		while resolved_pipelines.len() < max_results {
			let Ok(result) = compute_pipeline_results.try_recv() else {
				break;
			};

			match result {
				ComputePipelineResult::Ready { key, pipeline } => {
					let handle = frame.intern_compute_pipeline(pipeline);
					self.pipelines.write().insert(key.clone(), PipelineStatus::Ready(handle));
					resolved_pipelines.push((key, handle));
				}
				ComputePipelineResult::Failed { key } => {
					self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
					log::error!(
						"Async pipeline compilation failed for {}. The most likely cause is that shader creation or pipeline specialization failed on the compilation thread.",
						key
					);
				}
			}
		}

		resolved_pipelines
	}

	fn load_shader_handles(
		&self,
		material: &mut Material,
		device: &mut ghi::implementation::Frame,
	) -> Result<Vec<(ghi::ShaderHandle, ghi::ShaderTypes)>, ()> {
		material
			.shaders_mut()
			.iter_mut()
			.map(|shader: &mut Reference<Shader>| {
				if let Entry::Fresh((old_shader, old_shader_type)) = self.shaders.read().entry(&shader.id, shader.get_hash()) {
					return Ok((*old_shader, *old_shader_type));
				}

				let owned_shader = self.load_cached_shader_request(shader)?;

				let new_shader = device
					.create_shader(
						owned_shader.name.as_deref(),
						owned_shader.source.sources(),
						owned_shader.stage,
						owned_shader.binding_descriptors.clone(),
					)
					.unwrap();

				self.shaders
					.write()
					.insert(shader.id().to_string(), shader.get_hash(), (new_shader, owned_shader.stage));

				Ok((new_shader, owned_shader.stage))
			})
			.collect::<Result<Vec<_>, ()>>()
	}

	fn map_shader_type(stage: ShaderTypes) -> ghi::ShaderTypes {
		match stage {
			ShaderTypes::AnyHit => ghi::ShaderTypes::AnyHit,
			ShaderTypes::ClosestHit => ghi::ShaderTypes::ClosestHit,
			ShaderTypes::Compute => ghi::ShaderTypes::Compute,
			ShaderTypes::Fragment => ghi::ShaderTypes::Fragment,
			ShaderTypes::Intersection => ghi::ShaderTypes::Intersection,
			ShaderTypes::Mesh => ghi::ShaderTypes::Mesh,
			ShaderTypes::Miss => ghi::ShaderTypes::Miss,
			ShaderTypes::RayGen => ghi::ShaderTypes::RayGen,
			ShaderTypes::Callable => ghi::ShaderTypes::Callable,
			ShaderTypes::Task => ghi::ShaderTypes::Task,
			ShaderTypes::Vertex => ghi::ShaderTypes::Vertex,
		}
	}

	/// Loads the shader bytes once and keeps an owned copy so sync and async pipeline creation can reuse the same payload.
	fn load_cached_shader_request(&self, shader: &mut Reference<Shader>) -> Result<OwnedShader, ()> {
		if let Entry::Fresh(shader_request) = self.shader_requests.read().entry(&shader.id, shader.get_hash()) {
			return Ok(shader_request.clone());
		}

		let binding_descriptors = shader
			.resource()
			.interface
			.bindings
			.iter()
			.map(|binding| {
				ghi::shader::BindingDescriptor::new(
					binding.set,
					binding.binding,
					if binding.read {
						ghi::AccessPolicies::READ
					} else {
						ghi::AccessPolicies::empty()
					} | if binding.write {
						ghi::AccessPolicies::WRITE
					} else {
						ghi::AccessPolicies::empty()
					},
				)
			})
			.collect::<Vec<_>>();

		let stage = Self::map_shader_type(shader.resource().stage);
		let read_target = shader.into();
		let load_request = shader.load(read_target).map_err(|error| {
			log::error!(
				"Failed to load shader bytes for {}: {:?}. The most likely cause is that the shader resource no longer has an available read target.",
				shader.id(),
				error
			);
		})?;
		let buffer = load_request.buffer().ok_or(())?;

		let shader_request = OwnedShader {
			name: Some(shader.id().to_string()),
			source: if ghi::implementation::USES_METAL {
				OwnedShaderSource::MTLB {
					binary: buffer.to_vec().into_boxed_slice(),
					entry_point: "besl_main".to_string(),
				}
			} else {
				OwnedShaderSource::SPIRV(buffer.to_vec().into_boxed_slice())
			},
			stage,
			binding_descriptors,
		};

		self.shader_requests
			.write()
			.insert(shader.id().to_string(), shader.get_hash(), shader_request.clone());

		Ok(shader_request)
	}

	fn queue_material_pipeline(
		&self,
		resource_id: String,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		material: &mut Material,
	) -> Option<ghi::PipelineHandle> {
		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.pipelines.write().insert(resource_id.clone(), PipelineStatus::Pending);

		let request = match material.shaders_mut().iter_mut().next() {
			Some(shader) => self.load_cached_shader_request(shader).map(|shader| ComputePipelineRequest {
				key: resource_id.clone(),
				descriptor_set_templates: descriptor_set_template_handles.to_vec(),
				push_constant_ranges: push_constant_ranges.to_vec(),
				shader,
				specialization_map_entries: Vec::new(),
			}),
			None => Err(()),
		};

		match request {
			Ok(request) => self.queue_compute_pipeline(request),
			Err(()) => {
				self.pipelines.write().insert(resource_id, PipelineStatus::Failed);
			}
		}

		None
	}

	fn queue_variant_pipeline(
		&self,
		resource_id: String,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		specialization_map_entries: &[ghi::pipelines::SpecializationMapEntry],
		variant: &mut Reference<Variant>,
	) -> Option<ghi::PipelineHandle> {
		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.pipelines.write().insert(resource_id.clone(), PipelineStatus::Pending);

		let request = match variant.resource_mut().material.resource_mut().shaders_mut().iter_mut().next() {
			Some(shader) => self.load_cached_shader_request(shader).map(|shader| ComputePipelineRequest {
				key: resource_id.clone(),
				descriptor_set_templates: descriptor_set_template_handles.to_vec(),
				push_constant_ranges: push_constant_ranges.to_vec(),
				shader,
				specialization_map_entries: specialization_map_entries.to_vec(),
			}),
			None => Err(()),
		};

		match request {
			Ok(request) => self.queue_compute_pipeline(request),
			Err(()) => {
				self.pipelines.write().insert(resource_id, PipelineStatus::Failed);
			}
		}

		None
	}

	pub fn load_material(
		&self,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		reference: &mut Reference<Material>,
		device: &mut ghi::implementation::Frame,
	) -> Option<ghi::PipelineHandle> {
		if self.compute_pipeline_requests.is_some() {
			return self.queue_material_pipeline(
				reference.id().to_string(),
				descriptor_set_template_handles,
				push_constant_ranges,
				reference.resource_mut(),
			);
		}

		let resource_id = reference.id().to_string();

		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		let material = reference.resource_mut();
		let shaders = self.load_shader_handles(material, device).ok()?;
		Self::sleep_for_debug_pipeline_delay();
		let handle = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			descriptor_set_template_handles,
			push_constant_ranges,
			ghi::ShaderParameter::new(&shaders[0].0, ghi::ShaderTypes::Compute),
		));

		self.pipelines.write().insert(resource_id, PipelineStatus::Ready(handle));
		Some(handle)
	}

	pub fn load_variant(
		&self,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		specilization_map_entries: &[ghi::pipelines::SpecializationMapEntry],
		reference: &mut Reference<Variant>,
		device: &mut ghi::implementation::Frame,
	) -> Option<ghi::PipelineHandle> {
		if self.compute_pipeline_requests.is_some() {
			return self.queue_variant_pipeline(
				reference.id().to_string(),
				descriptor_set_template_handles,
				push_constant_ranges,
				specilization_map_entries,
				reference,
			);
		}

		let resource_id = reference.id().to_string();

		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.load_shader_handles(reference.resource_mut().material.resource_mut(), device)
			.ok()?;
		let variant = reference.resource_mut();
		let shader = variant.material.resource().shaders.get(0)?;
		let shader_handle = self
			.shaders
			.read()
			.get(&shader.id().to_string(), shader.hash())
			.map(|(handle, _)| *handle)?;
		Self::sleep_for_debug_pipeline_delay();
		let handle = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			descriptor_set_template_handles,
			push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, ghi::ShaderTypes::Compute)
				.with_specialization_map(specilization_map_entries),
		));

		self.pipelines.write().insert(resource_id, PipelineStatus::Ready(handle));
		Some(handle)
	}
}
