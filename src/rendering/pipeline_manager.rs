use std::{hash::Hash, rc::Rc};

use gxhash::{HashMap, HashMapExt};

use ghi::GraphicsHardwareInterface;
use resource_management::{material::{Material, Shader, Variant, VariantVariable}, types::ShaderTypes, Reference};
use utils::{r#async::{join_all, try_join_all}, stale_map::{Entry, StaleHashMap}, sync::RwLock};

pub struct PipelineManager {
	pipelines: RwLock<HashMap<String, ghi::PipelineHandle>>,
	shaders: RwLock<StaleHashMap<String, u64, (ghi::ShaderHandle, ghi::ShaderTypes)>>,
}

impl PipelineManager {
	pub fn new() -> Self {
		Self {
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shaders: RwLock::new(StaleHashMap::with_capacity(1024)),
		}
	}

	pub async fn load_material(&self, pipeline_layout_handle: &ghi::PipelineLayoutHandle, shader_binding_descriptors: &[ghi::ShaderBindingDescriptor], reference: &mut Reference<Material>, ghi: Rc<RwLock<ghi::GHI>>) -> Option<ghi::PipelineHandle> {
		if let Some(pipeline) = self.pipelines.read().get(reference.id()) {
			return Some(pipeline.clone());
		}

		let resource_id = reference.id().to_string();
		let material = reference.resource_mut();

		let shaders = try_join_all(material.shaders_mut().iter_mut().map(async |shader: &mut Reference<Shader>| {
			let hash = shader.get_hash();

			if let Entry::Fresh((old_shader, old_shader_type)) = self.shaders.read().entry(&shader.id, shader.get_hash()) {
				return Ok((*old_shader, *old_shader_type)); // If the shader has not changed, return the old shader
			}

			let stage = match shader.resource().stage {
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
			};

			let read_target = shader.into();
			let load_request = shader.load(read_target).await.unwrap();

			let buffer = if let Some(b) = load_request.get_buffer() {
				b
			} else {
				return Err(());
			};

			let new_shader = ghi.write().create_shader(Some(shader.id()), ghi::ShaderSource::SPIRV(buffer), stage, shader_binding_descriptors).unwrap();

			self.shaders.write().insert(shader.id().to_string(), shader.get_hash(), (new_shader, stage));

			Ok((new_shader, stage))
		})).await.unwrap();

		let pipeline_handle = ghi.write().create_compute_pipeline(pipeline_layout_handle, ghi::ShaderParameter::new(&shaders[0].0, ghi::ShaderTypes::Compute));

		self.pipelines.write().insert(resource_id, pipeline_handle.clone());

		Some(pipeline_handle)
	}

	pub async fn load_variant(&self, pipeline_layout_handle: &ghi::PipelineLayoutHandle, shader_binding_descriptors: &[ghi::ShaderBindingDescriptor], specilization_map_entries: &[ghi::SpecializationMapEntry], reference: &mut Reference<Variant>, ghi: Rc<RwLock<ghi::GHI>>) -> Option<ghi::PipelineHandle> {
		if let Some(pipeline) = self.pipelines.read().get(reference.id()) {
			return Some(pipeline.clone());
		}

		self.load_material(pipeline_layout_handle, shader_binding_descriptors, &mut reference.resource_mut().material, ghi.clone()).await.unwrap();

		let resource_id = reference.id().to_string();
		let variant = reference.resource_mut();

		let shader_handle = {
			let shader = variant.material.resource().shaders.get(0);
			shader.map(|s| {
				self.shaders.read().get(&s.id().to_string(), s.hash()).map(|(s, _)| s.clone())
			})
		};

		let shader_handle = shader_handle.unwrap().unwrap();

		let pipeline_handle = ghi.write().create_compute_pipeline(pipeline_layout_handle, ghi::ShaderParameter::new(&shader_handle, ghi::ShaderTypes::Compute).with_specialization_map(specilization_map_entries));

		self.pipelines.write().insert(resource_id, pipeline_handle.clone());

		Some(pipeline_handle)
	}
}