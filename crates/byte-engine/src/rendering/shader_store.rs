/// The `ShaderSourceDefinition` enum describes the source program used to create a shader resource.
pub enum ShaderSourceDefinition<'a> {
	Inline(ghi::shader::ShaderSource<'a>),
	Besl {
		settings: ShaderGenerationSettings,
		main_node: besl::NodeReference,
	},
}

/// The `ShaderSourceDescriptor` struct describes an inline shader that can be baked into a `Shader` resource.
pub struct ShaderSourceDescriptor<'a> {
	pub id: &'a str,
	pub name: &'a str,
	pub stage: ShaderTypes,
	pub source: ShaderSourceDefinition<'a>,
	pub interface: ShaderInterface,
}

/// Loads a baked shader from storage, or bakes and stores it when the source descriptor changed.
pub fn upsert_shader(storage_backend: &dyn StorageBackend, descriptor: &ShaderSourceDescriptor<'_>) -> Result<Shader, String> {
	let source_hash = hash_shader_source(descriptor);
	let resource_id = ResourceId::new(descriptor.id);

	if storage_backend.read(resource_id).is_some() {
		let shader = load_shader_reference(storage_backend, descriptor.id)?.into_resource();
		if shader.source_hash == source_hash {
			return Ok(shader);
		}
	}

	let (shader, bytes) = bake_shader(descriptor, source_hash)?;
	let processed = ProcessedAsset::new(ResourceId::new(descriptor.id), shader.clone());
	storage_backend.store(&processed, &bytes).map_err(|_| {
		"Failed to store baked shader. The most likely cause is a resource storage backend failure.".to_string()
	})?;

	Ok(shader)
}

/// Creates a GPU shader from storage, baking the inline descriptor first when needed.
pub fn create_shader(
	context: &mut ghi::implementation::Context,
	storage_backend: Option<&dyn StorageBackend>,
	descriptor: &ShaderSourceDescriptor<'_>,
) -> Result<ghi::ShaderHandle, String> {
	if let Some(storage_backend) = storage_backend {
		upsert_shader(storage_backend, descriptor)?;
		let mut shader = load_shader_reference(storage_backend, descriptor.id)?;
		let backing = shader.consume_reader().into_backing_storage().map_err(|_| {
			"Failed to load baked shader bytes. The most likely cause is an unsupported shader resource reader.".to_string()
		})?;
		let bytes = backing.as_slice();
		let source = shader_artifact_source(&shader.resource.artifact, shader.resource.interface.workgroup_size, bytes)?;
		return context
			.create_shader(
				Some(descriptor.name),
				source,
				shader_type_to_ghi(shader.resource.stage),
				shader.resource.interface.bindings.iter().map(binding_to_descriptor),
			)
			.map_err(|_| {
				"Failed to create baked shader. The most likely cause is an incompatible shader interface.".to_string()
			});
	}

	with_shader_source(descriptor.name, &descriptor.source, |source| {
		crate::rendering::create_shader_from_source(
			context,
			Some(descriptor.name),
			source,
			shader_type_to_ghi(descriptor.stage),
			descriptor.interface.bindings.iter().map(binding_to_descriptor),
		)
	})
}

pub fn create_shader_from_baked_or_inline(
	context: &mut ghi::implementation::Context,
	storage_backend: Option<&dyn StorageBackend>,
	descriptor: &ShaderSourceDescriptor<'_>,
) -> Result<ghi::ShaderHandle, String> {
	create_shader(context, storage_backend, descriptor)
}

fn with_shader_source<T>(
	name: &str,
	definition: &ShaderSourceDefinition<'_>,
	use_source: impl FnOnce(ghi::shader::ShaderSource<'_>) -> Result<T, String>,
) -> Result<T, String> {
	match definition {
		ShaderSourceDefinition::Inline(source) => use_source(*source),
		ShaderSourceDefinition::Besl { settings, main_node } => {
			// BESL generation is centralized here so render passes do not need to know about platform shader sources.
			let glsl_source = GLSLShaderGenerator::new()
				.generate(settings, main_node)
				.map_err(|_| format!("Failed to generate {name} GLSL. The most likely cause is invalid BESL syntax."))?;
			let msl_source = MSLShaderGenerator::new().generate(settings, main_node).map_err(|_| {
				format!(
					"Failed to generate {name} MSL. The most likely cause is an unsupported BESL construct in the Metal transpiler."
				)
			})?;

			use_source(ghi::shader::ShaderSource::Platform {
				glsl: &glsl_source,
				msl: &msl_source,
				msl_entry_point: "besl_main",
			})
		}
	}
}

fn load_shader_reference(storage_backend: &dyn StorageBackend, id: &str) -> Result<Reference<Shader>, String> {
	let (resource, _) = storage_backend
		.read(ResourceId::new(id))
		.ok_or_else(|| "Failed to load baked shader. The most likely cause is a missing shader resource.".to_string())?;
	let model: ReferenceModel<Shader> = resource.into();
	model
		.solve(storage_backend)
		.map_err(|_| "Failed to solve baked shader. The most likely cause is invalid shader resource metadata.".to_string())
}

fn bake_shader(descriptor: &ShaderSourceDescriptor<'_>, source_hash: u64) -> Result<(Shader, Vec<u8>), String> {
	let compiled = with_shader_source(descriptor.name, &descriptor.source, |source| {
		ghi::shader::compile(descriptor.name, source)
	})?;
	let (artifact, bytes) = match compiled {
		ghi::shader::CompiledShaderSource::SPIRV(bytes) => (ShaderArtifact::Spirv, bytes),
		ghi::shader::CompiledShaderSource::HLSL { source, entry_point } => {
			todo!("Handle HLSL shader baking");
		}
		ghi::shader::CompiledShaderSource::MTL { source, entry_point } => {
			let bytes =
				resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, descriptor.name)?;
			(ShaderArtifact::Mtlb { entry_point }, bytes.into_vec())
		}
	};

	Ok((
		Shader {
			id: descriptor.id.to_string(),
			stage: descriptor.stage,
			interface: descriptor.interface.clone(),
			artifact,
			source_hash,
		},
		bytes,
	))
}

fn shader_artifact_source<'a>(
	artifact: &'a ShaderArtifact,
	workgroup_size: Option<(u32, u32, u32)>,
	bytes: &'a [u8],
) -> Result<ghi::shader::Sources<'a>, String> {
	match artifact {
		ShaderArtifact::Spirv => Ok(ghi::shader::Sources::SPIRV(bytes)),
		ShaderArtifact::Msl { entry_point } => Ok(ghi::shader::Sources::MTL {
			source: std::str::from_utf8(bytes).map_err(|_| {
				"Failed to read baked MSL shader. The most likely cause is invalid UTF-8 shader bytes.".to_string()
			})?,
			entry_point,
		}),
		ShaderArtifact::Mtlb { entry_point } => Ok(ghi::shader::Sources::MTLB {
			binary: bytes,
			entry_point,
			threadgroup_size: shader_threadgroup_size(artifact, workgroup_size),
		}),
	}
}

fn shader_threadgroup_size(artifact: &ShaderArtifact, workgroup_size: Option<(u32, u32, u32)>) -> Option<Extent> {
	match artifact {
		ShaderArtifact::Mtlb { .. } => workgroup_size.map(|(width, height, depth)| Extent::new(width, height, depth)),
		_ => None,
	}
}

fn hash_shader_source_definition(name: &str, definition: &ShaderSourceDefinition<'_>, hasher: &mut DefaultHasher) {
	with_shader_source(name, definition, |source| {
		match source {
			ghi::shader::ShaderSource::Glsl(source) => {
				hasher.write(b"glsl");
				hasher.write(source.as_bytes());
			}
			ghi::shader::ShaderSource::Hlsl { .. } => {
				todo!("implement hash shader source for hlsl");
			}
			ghi::shader::ShaderSource::Msl { source, entry_point } => {
				hasher.write(b"msl");
				hasher.write(source.as_bytes());
				hasher.write(entry_point.as_bytes());
			}
			ghi::shader::ShaderSource::Platform {
				glsl,
				msl,
				msl_entry_point,
			} => {
				hasher.write(b"platform");
				hasher.write(glsl.as_bytes());
				hasher.write(msl.as_bytes());
				hasher.write(msl_entry_point.as_bytes());
			}
			ghi::shader::ShaderSource::PlatformNative { .. } => {
				todo!("implement whatever this is. damned clankers");
			}
		}
		Ok(())
	})
	.expect("Failed to hash shader source. The most likely cause is invalid generated shader source.");
}

fn hash_shader_source(descriptor: &ShaderSourceDescriptor<'_>) -> u64 {
	let mut hasher = DefaultHasher::new();
	hasher.write(b"shader-store-mtlb-v1");
	hasher.write(descriptor.id.as_bytes());
	hasher.write(descriptor.name.as_bytes());
	hasher.write(format!("{:?}", descriptor.stage).as_bytes());
	hash_shader_source_definition(descriptor.name, &descriptor.source, &mut hasher);
	for binding in &descriptor.interface.bindings {
		hasher.write_u32(binding.set);
		hasher.write_u32(binding.binding);
		hasher.write_u8(binding.read as u8);
		hasher.write_u8(binding.write as u8);
	}
	if let Some((width, height, depth)) = descriptor.interface.workgroup_size {
		hasher.write_u32(width);
		hasher.write_u32(height);
		hasher.write_u32(depth);
	}
	hasher.finish()
}

fn binding_to_descriptor(binding: &Binding) -> ghi::shader::BindingDescriptor {
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
}

fn shader_type_to_ghi(shader_type: ShaderTypes) -> ghi::ShaderTypes {
	match shader_type {
		ShaderTypes::Vertex => ghi::ShaderTypes::Vertex,
		ShaderTypes::Fragment => ghi::ShaderTypes::Fragment,
		ShaderTypes::Compute => ghi::ShaderTypes::Compute,
		ShaderTypes::Task => ghi::ShaderTypes::Task,
		ShaderTypes::Mesh => ghi::ShaderTypes::Mesh,
		ShaderTypes::RayGen => ghi::ShaderTypes::RayGen,
		ShaderTypes::ClosestHit => ghi::ShaderTypes::ClosestHit,
		ShaderTypes::AnyHit => ghi::ShaderTypes::AnyHit,
		ShaderTypes::Intersection => ghi::ShaderTypes::Intersection,
		ShaderTypes::Miss => ghi::ShaderTypes::Miss,
		ShaderTypes::Callable => ghi::ShaderTypes::Callable,
	}
}

use std::{collections::hash_map::DefaultHasher, hash::Hasher as _};

use ghi::context::{Context as _, ContextCreate as _};
use resource_management::{
	asset::ResourceId,
	resource::{ReadStorageBackend as _, StorageBackend},
	resources::material::{Binding, Shader, ShaderArtifact, ShaderInterface},
	shader::{
		besl::backends::{glsl::GLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::ShaderGenerationSettings,
	},
	types::ShaderTypes,
	ProcessedAsset, Reference, ReferenceModel, Solver,
};
use utils::Extent;
