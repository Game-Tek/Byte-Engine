use std::{cell::OnceCell, hash::Hash, rc::Rc};

use ghi::Device;
use resource_management::{resources::material::{Material, Shader, Variant, VariantVariable}, types::ShaderTypes, Reference};
use utils::{hash::{HashMap, HashMapExt}, stale_map::{Entry, StaleHashMap}, sync::{RwLock, RwLockUpgradableReadGuard}};

pub struct PipelineManager {
	pipelines: RwLock<HashMap<String, OnceCell<ghi::PipelineHandle>>>,
	shaders: RwLock<StaleHashMap<String, u64, (ghi::ShaderHandle, ghi::ShaderTypes)>>,
}

impl PipelineManager {
	pub fn new() -> Self {
		Self {
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shaders: RwLock::new(StaleHashMap::with_capacity(1024)),
		}
	}

	pub fn load_material(&self, pipeline_layout_handle: &ghi::PipelineLayoutHandle, reference: &mut Reference<Material>, ghi: Rc<RwLock<ghi::GHI>>) -> Option<ghi::PipelineHandle> {
		let v = {
			let mut pipelines = self.pipelines.write();
			let resource_id = reference.id().to_string();
			let v = pipelines.entry(resource_id).or_insert_with(|| OnceCell::new()).clone();
			v
		};

		let r: Result<&ghi::PipelineHandle, ()> = v.get_or_try_init(|| {
			let material = reference.resource_mut();

			let shaders = material.shaders_mut().iter_mut().map(|shader: &mut Reference<Shader>| {
				let hash = shader.get_hash();

				if let Entry::Fresh((old_shader, old_shader_type)) = self.shaders.read().entry(&shader.id, shader.get_hash()) {
					return Ok((*old_shader, *old_shader_type)); // If the shader has not changed, return the old shader
				}

				let shader_binding_descriptors = shader.resource().interface.bindings.iter().map(|binding| {
					ghi::ShaderBindingDescriptor::new(binding.set, binding.binding, if binding.read { ghi::AccessPolicies::READ } else { ghi::AccessPolicies::empty() } | if binding.write { ghi::AccessPolicies::WRITE } else { ghi::AccessPolicies::empty() })
				}).collect::<Vec<_>>();

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
				let load_request = shader.load(read_target).unwrap();

				let buffer = if let Some(b) = load_request.get_buffer() {
					b
				} else {
					return Err(());
				};


				let new_shader = ghi.write().create_shader(Some(shader.id()), ghi::ShaderSource::SPIRV(buffer), stage, &shader_binding_descriptors).unwrap();

				self.shaders.write().insert(shader.id().to_string(), shader.get_hash(), (new_shader, stage));

				Ok((new_shader, stage))
			}).collect::<Result<Vec<_>, ()>>()?;

			let pipeline_handle = ghi.write().create_compute_pipeline(pipeline_layout_handle, ghi::ShaderParameter::new(&shaders[0].0, ghi::ShaderTypes::Compute));

			Ok(pipeline_handle)
		});

		r.ok().map(|v| *v)
	}

	pub fn load_variant(&self, pipeline_layout_handle: &ghi::PipelineLayoutHandle, specilization_map_entries: &[ghi::SpecializationMapEntry], reference: &mut Reference<Variant>, ghi: Rc<RwLock<ghi::GHI>>,) -> Option<ghi::PipelineHandle> {
		let v = {
			let mut pipelines = self.pipelines.write();
			let resource_id = reference.id().to_string();
			let v = pipelines.entry(resource_id).or_insert_with(|| OnceCell::new()).clone();
			v
		};

		let r: Result<&ghi::PipelineHandle, ()> = v.get_or_try_init(|| {
			self.load_material(pipeline_layout_handle, &mut reference.resource_mut().material, ghi.clone()).unwrap();

			let variant = reference.resource_mut();

			let shader_handle = {
				let shader = variant.material.resource().shaders.get(0);
				shader.map(|s| {
					self.shaders.read().get(&s.id().to_string(), s.hash()).map(|(s, _)| s.clone())
				})
			};

			let shader_handle = shader_handle.unwrap().unwrap();

			let pipeline_handle = ghi.write().create_compute_pipeline(pipeline_layout_handle, ghi::ShaderParameter::new(&shader_handle, ghi::ShaderTypes::Compute).with_specialization_map(specilization_map_entries));

			Ok(pipeline_handle)
		});

		r.ok().map(|v| *v)
	}
}
