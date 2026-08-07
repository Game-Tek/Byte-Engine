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

/// The `LoadedShader` struct keeps a baked shader with the persisted interface
/// needed to build and bind its pipeline.
pub struct LoadedShader {
	pub handle: ghi::ShaderHandle,
	pub stage: ShaderTypes,
	pub interface: ShaderInterface,
}

/// Resolves a baked shader through the resource manager and creates its GHI handle.
pub fn load_shader_resource(
	context: &mut ghi::implementation::Context,
	resource_manager: &resource_management::resource::resource_manager::ResourceManager,
	id: &str,
	name: &str,
) -> Result<LoadedShader, String> {
	let mut shader: Reference<Shader> = crate::rendering::resource_loading::request(resource_manager, id).map_err(|error| {
		format!(
			"Failed to load baked shader resource '{id}': {error}. The most likely cause is that BELD did not bake the shader or its source asset is unavailable."
		)
	})?;
	let stage = shader.resource.stage;
	let interface = shader.resource.interface.clone();
	let artifact = shader.resource.artifact.clone();
	let backing =
		crate::rendering::resource_loading::block_on(shader.consume_reader().into_backing_storage()).map_err(|_| {
			format!(
				"Failed to load baked shader bytes for '{id}'. The most likely cause is an unsupported shader resource reader."
			)
		})?;
	let source = shader_artifact_source(&artifact, interface.workgroup_size, backing.as_slice())?;
	let handle = context
		.create_shader(
			Some(name),
			source,
			shader_type_to_ghi(stage),
			interface.bindings.iter().map(binding_to_descriptor),
		)
		.map_err(|_| {
			format!(
				"Failed to create baked shader '{id}'. The most likely cause is an incompatible persisted shader interface."
			)
		})?;

	Ok(LoadedShader {
		handle,
		stage,
		interface,
	})
}

/// Loads a baked shader from storage, or bakes and stores it when the source descriptor changed.
pub async fn upsert_shader(
	storage_backend: &dyn StorageBackend,
	descriptor: &ShaderSourceDescriptor<'_>,
) -> Result<Shader, String> {
	validate_besl_interface(descriptor)?;
	let source_hash = hash_shader_source(descriptor);
	let resource_id = ResourceId::new(descriptor.id);

	if let Some(_) = storage_backend.read(resource_id).await {
		// A cache schema can change before its source hash can be read. Treat an
		// unreadable entry as stale because this descriptor is the authoritative
		// source and `store` replaces the entry after a successful bake.
		if let Ok(shader) = load_shader_reference(storage_backend, descriptor.id).await {
			let shader = shader.into_resource();
			if shader.source_hash == source_hash {
				return Ok(shader);
			}
		}
	}

	let (shader, bytes) = bake_shader(descriptor, source_hash).await?;
	let processed = ProcessedAsset::new(ResourceId::new(descriptor.id), shader.clone());
	storage_backend.store(processed, &bytes).map_err(|_| {
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
		let runtime =
			compio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime for shader creation: {e}"))?;
		runtime.block_on(upsert_shader(storage_backend, descriptor))?;
		let mut shader = runtime.block_on(load_shader_reference(storage_backend, descriptor.id))?;
		let backing = runtime
			.block_on(shader.consume_reader().into_backing_storage())
			.map_err(|_| {
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

	validate_besl_interface(descriptor)?;
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

/// Rejects structural drift between a BESL program's reachable resources and its retained GHI interface.
fn validate_besl_interface(descriptor: &ShaderSourceDescriptor<'_>) -> Result<(), String> {
	let ShaderSourceDefinition::Besl { main_node, .. } = &descriptor.source else {
		return Ok(());
	};

	let mut declared = descriptor
		.interface
		.bindings
		.iter()
		.map(|binding| (binding.slot, binding.kind, binding.count, binding.buffer_stride))
		.collect::<Vec<_>>();
	declared.sort_by_key(|(slot, ..)| *slot);
	let reflected = resource_management::shader::besl::evaluation::ProgramEvaluation::from_main(main_node)?
		.bindings()
		.iter()
		.map(|binding| (binding.slot, binding.kind, binding.count, binding.buffer_stride))
		.collect::<Vec<_>>();

	if declared != reflected {
		return Err(format!(
			"BESL shader interface mismatch for '{}'. The most likely cause is stale hand-authored resource metadata or obsolete descriptor-layout preservation. declared={declared:?}, reflected={reflected:?}",
			descriptor.name,
		));
	}

	Ok(())
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
			let hlsl_source = HLSLShaderGenerator::new().generate(settings, main_node).map_err(|_| {
				format!(
					"Failed to generate {name} HLSL. The most likely cause is an unsupported BESL construct in the HLSL transpiler."
				)
			})?;

			use_source(ghi::shader::ShaderSource::PlatformNative {
				glsl: &glsl_source,
				msl: &msl_source,
				msl_entry_point: "besl_main",
				hlsl: &hlsl_source,
				hlsl_entry_point: "besl_main",
			})
		}
	}
}

async fn load_shader_reference(storage_backend: &dyn StorageBackend, id: &str) -> Result<Reference<Shader>, String> {
	let (resource, _) = storage_backend
		.read(ResourceId::new(id))
		.await
		.ok_or_else(|| "Failed to load baked shader. The most likely cause is a missing shader resource.".to_string())?;
	let model: ReferenceModel<Shader> = resource.into();
	model
		.solve(storage_backend)
		.await
		.map_err(|_| "Failed to solve baked shader. The most likely cause is invalid shader resource metadata.".to_string())
}

async fn bake_shader(descriptor: &ShaderSourceDescriptor<'_>, source_hash: u64) -> Result<(Shader, Vec<u8>), String> {
	let compiled = with_shader_source(descriptor.name, &descriptor.source, |source| {
		ghi::shader::compile(descriptor.name, source)
	})?;
	let (artifact, bytes) = match compiled {
		ghi::shader::CompiledShaderSource::SPIRV(bytes) => (ShaderArtifact::Spirv, bytes),
		ghi::shader::CompiledShaderSource::HLSL { source, entry_point } => {
			(ShaderArtifact::Hlsl { entry_point }, source.into_bytes())
		}
		ghi::shader::CompiledShaderSource::MTL { source, entry_point } => {
			#[cfg(target_os = "macos")]
			{
				let bytes =
					resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, descriptor.name)
						.await?;
				(ShaderArtifact::Mtlb { entry_point }, bytes.into_vec())
			}
			#[cfg(not(target_os = "macos"))]
			{
				(ShaderArtifact::Msl { entry_point }, source.into_bytes())
			}
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
		ShaderArtifact::Dxil => Ok(ghi::shader::Sources::DXIL(bytes)),
		ShaderArtifact::Hlsl { entry_point } => Ok(ghi::shader::Sources::HLSL {
			source: std::str::from_utf8(bytes).map_err(|_| {
				"Failed to read baked HLSL shader. The most likely cause is invalid UTF-8 shader bytes.".to_string()
			})?,
			entry_point,
		}),
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
			#[cfg(target_os = "linux")]
			ghi::shader::ShaderSource::Glsl(source) => {
				hasher.write(b"glsl");
				hash_text(hasher, source);
			}
			ghi::shader::ShaderSource::Hlsl { source, entry_point } => {
				hasher.write(b"hlsl");
				hash_text(hasher, source);
				hash_text(hasher, entry_point);
			}
			ghi::shader::ShaderSource::Msl { source, entry_point } => {
				hasher.write(b"msl");
				hash_text(hasher, source);
				hash_text(hasher, entry_point);
			}
			ghi::shader::ShaderSource::Platform {
				glsl,
				msl,
				msl_entry_point,
			} => {
				hasher.write(b"platform");
				hash_text(hasher, glsl);
				hash_text(hasher, msl);
				hash_text(hasher, msl_entry_point);
			}
			ghi::shader::ShaderSource::PlatformNative {
				glsl,
				msl,
				msl_entry_point,
				hlsl,
				hlsl_entry_point,
			} => {
				hasher.write(b"platform-native");
				hash_text(hasher, glsl);
				hash_text(hasher, msl);
				hash_text(hasher, msl_entry_point);
				hash_text(hasher, hlsl);
				hash_text(hasher, hlsl_entry_point);
			}
		}
		Ok(())
	})
	.expect("Failed to hash shader source. The most likely cause is invalid generated shader source.");
}

fn hash_text(hasher: &mut DefaultHasher, value: &str) {
	// Length prefixes prevent distinct field partitions from producing the same
	// byte stream without allocating a temporary serialization.
	hasher.write_u64(value.len() as u64);
	hasher.write(value.as_bytes());
}

fn hash_shader_source(descriptor: &ShaderSourceDescriptor<'_>) -> u64 {
	let mut hasher = DefaultHasher::new();
	// v4 invalidates resources that predate retained structured-buffer strides.
	hasher.write(b"shader-store-v4-retained-buffer-strides");
	hash_text(&mut hasher, descriptor.id);
	hash_text(&mut hasher, descriptor.name);
	hasher.write_u8(shader_type_tag(descriptor.stage));
	hash_shader_source_definition(descriptor.name, &descriptor.source, &mut hasher);
	hasher.write_u64(descriptor.interface.bindings.len() as u64);
	for binding in &descriptor.interface.bindings {
		hash_text(&mut hasher, &binding.name);
		hasher.write_u32(binding.slot);
		match binding.kind {
			resource_management::resources::material::BindingKind::StorageBuffer => hasher.write_u8(0),
			resource_management::resources::material::BindingKind::CombinedImageSampler { view } => {
				hasher.write_u8(1);
				hasher.write_u8(match view {
					resource_management::resources::material::TextureView::Texture2D => 0,
					resource_management::resources::material::TextureView::Texture2DArray => 1,
					resource_management::resources::material::TextureView::Texture3D => 2,
					resource_management::resources::material::TextureView::TextureCube => 3,
				});
			}
			resource_management::resources::material::BindingKind::StorageImage => hasher.write_u8(2),
		}
		hasher.write_u32(binding.count);
		hasher.write_u32(binding.buffer_stride.unwrap_or(0));
		hasher.write_u8(binding.read as u8);
		hasher.write_u8(binding.write as u8);
	}
	match descriptor.interface.workgroup_size {
		Some((width, height, depth)) => {
			hasher.write_u8(1);
			hasher.write_u32(width);
			hasher.write_u32(height);
			hasher.write_u32(depth);
		}
		None => hasher.write_u8(0),
	}
	hasher.finish()
}

fn shader_type_tag(shader_type: ShaderTypes) -> u8 {
	match shader_type {
		ShaderTypes::Vertex => 0,
		ShaderTypes::Fragment => 1,
		ShaderTypes::Compute => 2,
		ShaderTypes::Task => 3,
		ShaderTypes::Mesh => 4,
		ShaderTypes::RayGen => 5,
		ShaderTypes::ClosestHit => 6,
		ShaderTypes::AnyHit => 7,
		ShaderTypes::Intersection => 8,
		ShaderTypes::Miss => 9,
		ShaderTypes::Callable => 10,
	}
}

pub(crate) fn binding_to_descriptor(binding: &Binding) -> ghi::ShaderResourceDescriptor {
	use resource_management::resources::material::{BindingKind, TextureView};

	let kind = match binding.kind {
		BindingKind::StorageBuffer => ghi::ResourceKind::StorageBuffer,
		BindingKind::CombinedImageSampler { .. } => ghi::ResourceKind::CombinedImageSampler,
		BindingKind::StorageImage => ghi::ResourceKind::StorageImage,
	};
	let descriptor = ghi::ShaderResourceDescriptor::new(
		ghi::ResourceSlot::new(binding.slot),
		kind,
		binding.count,
		binding_access_policy(binding),
	);
	// Persisted resources bypass the public constructor during deserialization,
	// so enforce the same invariant before GHI can fall back to its default.
	let descriptor = match binding.kind {
		BindingKind::StorageBuffer => {
			let stride = binding.buffer_stride.expect(
				"Missing persisted storage-buffer stride. The most likely cause is a stale or malformed shader interface resource.",
			);
			assert!(
				stride > 0,
				"Invalid persisted storage-buffer stride. The most likely cause is a stale or malformed shader interface resource."
			);
			descriptor.buffer_stride(stride)
		}
		_ => {
			assert!(
				binding.buffer_stride.is_none(),
				"Unexpected persisted buffer stride. The most likely cause is a stale or malformed non-buffer shader binding."
			);
			descriptor
		}
	};

	match binding.kind {
		BindingKind::CombinedImageSampler { view } => descriptor.texture_view_type(match view {
			TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
			TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
			TextureView::TextureCube => ghi::TextureViewTypes::TextureCube,
			TextureView::Texture3D => ghi::TextureViewTypes::Texture3D,
		}),
		_ => descriptor,
	}
}

fn binding_access_policy(binding: &Binding) -> ghi::AccessPolicies {
	(if binding.read {
		ghi::AccessPolicies::READ
	} else {
		ghi::AccessPolicies::empty()
	}) | if binding.write {
		ghi::AccessPolicies::WRITE
	} else {
		ghi::AccessPolicies::empty()
	}
}

pub(crate) fn shader_type_to_ghi(shader_type: ShaderTypes) -> ghi::ShaderTypes {
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
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::ShaderGenerationSettings,
	},
	types::ShaderTypes,
	ProcessedAsset, Reference, ReferenceModel, Solver,
};
use utils::Extent;

#[cfg(test)]
mod tests {
	use super::*;

	fn inline_descriptor<'a>(
		id: &'a str,
		name: &'a str,
		source: &'a str,
		entry_point: &'a str,
		stage: ShaderTypes,
		interface: ShaderInterface,
	) -> ShaderSourceDescriptor<'a> {
		ShaderSourceDescriptor {
			id,
			name,
			stage,
			source: ShaderSourceDefinition::Inline(ghi::shader::ShaderSource::Msl { source, entry_point }),
			interface,
		}
	}

	fn interface(workgroup_size: Option<(u32, u32, u32)>, bindings: Vec<Binding>) -> ShaderInterface {
		ShaderInterface {
			workgroup_size,
			bindings,
		}
	}

	fn storage_binding(slot: u32, count: u32, read: bool, write: bool) -> Binding {
		Binding::new(
			slot,
			resource_management::resources::material::BindingKind::StorageBuffer,
			count,
			Some(4),
			read,
			write,
		)
	}

	fn canonical_descriptor() -> ShaderSourceDescriptor<'static> {
		inline_descriptor(
			"shader/id",
			"Shader Name",
			"kernel void main0() {}",
			"main0",
			ShaderTypes::Compute,
			interface(Some((8, 4, 2)), vec![storage_binding(3, 1, true, false)]),
		)
	}

	/// Verifies that changing one persisted descriptor input changes its source hash.
	fn assert_hash_changes(change: impl FnOnce(&mut ShaderSourceDescriptor<'static>)) {
		let base = canonical_descriptor();
		let mut changed = canonical_descriptor();
		change(&mut changed);
		assert_ne!(hash_shader_source(&changed), hash_shader_source(&base));
	}

	#[test]
	fn unreadable_cached_shader_is_rebaked_from_the_authoritative_descriptor() {
		use resource_management::{
			resource::{storage_backend::redb_storage_backend::RedbStorageBackend, WriteStorageBackend as _},
			resources::audio::Audio,
			types::BitDepths,
		};

		let runtime = compio::runtime::Runtime::new().expect("Failed to create runtime for test");

		let unique = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("System clock should follow the Unix epoch")
			.as_nanos();
		let storage_path =
			std::env::temp_dir().join(format!("byte-engine-shader-cache-migration-{}-{unique}", std::process::id()));
		let storage = RedbStorageBackend::new_writable(storage_path.clone());
		let descriptor = inline_descriptor(
			"shader/stale-schema",
			"Stale Schema",
			"kernel void besl_main() {}",
			"besl_main",
			ShaderTypes::Compute,
			interface(None, Vec::new()),
		);
		storage
			.store(
				ProcessedAsset::new(
					ResourceId::new(descriptor.id),
					Audio {
						bit_depth: BitDepths::Sixteen,
						channel_count: 1,
						sample_rate: 48_000,
						sample_count: 0,
					},
				),
				&[],
			)
			.expect("Expected stale cache fixture to store");

		let expected_hash = hash_shader_source(&descriptor);
		let shader = runtime
			.block_on(upsert_shader(&storage, &descriptor))
			.expect("An unreadable cached resource should be replaced from the shader descriptor");

		assert_eq!(shader.source_hash, expected_hash);
		let loaded_hash = runtime
			.block_on(load_shader_reference(&storage, descriptor.id))
			.expect("Expected the replacement shader to load")
			.resource
			.source_hash;
		assert_eq!(loaded_hash, expected_hash);

		drop(storage);
		std::fs::remove_dir_all(&storage_path).expect("Expected shader cache migration fixture cleanup");
	}

	#[test]
	fn persisted_storage_buffer_stride_reaches_the_ghi_descriptor() {
		let binding = Binding::new(
			8,
			resource_management::resources::material::BindingKind::StorageBuffer,
			1,
			Some(64),
			true,
			false,
		);
		let descriptor = binding_to_descriptor(&binding);

		assert_eq!(descriptor.slot(), ghi::ResourceSlot::new(8));
		assert_eq!(descriptor.buffer_element_stride(), 64);
	}

	#[test]
	#[should_panic(expected = "Missing persisted storage-buffer stride")]
	fn deserialized_storage_buffer_without_stride_is_rejected() {
		let invalid = Binding {
			name: "meshlets".to_string(),
			slot: 8,
			kind: resource_management::resources::material::BindingKind::StorageBuffer,
			count: 1,
			buffer_stride: None,
			read: true,
			write: false,
		};
		let json = serde_json::to_string(&invalid).expect("Malformed binding fixture should serialize");
		let archived: Binding = serde_json::from_str(&json).expect("Malformed binding fixture should deserialize");

		let _ = binding_to_descriptor(&archived);
	}

	/// Verifies structural BESL reflection drift is rejected before a backend can materialize incompatible IDs.
	#[test]
	fn besl_interface_validation_rejects_missing_reachable_resources() {
		let material_offset = besl::compile_to_besl(
			include_str!(concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/assets/rendering/visibility/material-offset.besl"
			)),
			None,
		)
		.expect("Material-offset BESL asset should link")
		.get_main()
		.expect("Material-offset BESL asset should declare main");
		let descriptor = ShaderSourceDescriptor {
			id: "shader/material-offset-mismatch",
			name: "Material Offset Mismatch",
			stage: ShaderTypes::Compute,
			source: ShaderSourceDefinition::Besl {
				settings: ShaderGenerationSettings::compute(Extent::square(1)),
				main_node: material_offset,
			},
			interface: interface(Some((1, 1, 1)), Vec::new()),
		};

		let error = validate_besl_interface(&descriptor)
			.expect_err("Missing flat resources should fail before shader creation or cache lookup");
		assert!(error.contains("BESL shader interface mismatch"));
		assert!(error.contains("1033"));
		assert!(error.contains("1036"));
	}

	#[test]
	fn source_hash_is_stable_and_covers_every_persisted_input() {
		assert_eq!(
			hash_shader_source(&canonical_descriptor()),
			hash_shader_source(&canonical_descriptor())
		);

		assert_hash_changes(|shader| shader.id = "shader/other");
		assert_hash_changes(|shader| shader.name = "Other Name");
		assert_hash_changes(|shader| {
			shader.source = ShaderSourceDefinition::Inline(ghi::shader::ShaderSource::Msl {
				source: "kernel void changed() {}",
				entry_point: "main0",
			});
		});
		assert_hash_changes(|shader| {
			shader.source = ShaderSourceDefinition::Inline(ghi::shader::ShaderSource::Msl {
				source: "kernel void main0() {}",
				entry_point: "other",
			});
		});
		assert_hash_changes(|shader| shader.stage = ShaderTypes::Fragment);
		assert_hash_changes(|shader| shader.interface.workgroup_size = Some((4, 4, 2)));
		assert_hash_changes(|shader| shader.interface.bindings[0].slot = 4);
		assert_hash_changes(|shader| shader.interface.bindings[0].count = 2);
		assert_hash_changes(|shader| {
			shader.interface.bindings[0].read = false;
			shader.interface.bindings[0].write = true;
		});
		assert_hash_changes(|shader| shader.interface.bindings[0].name = "renamed_scene".into());
		assert_hash_changes(|shader| shader.interface.bindings[0].buffer_stride = Some(16));

		let sampled_hash = |view| {
			let mut shader = canonical_descriptor();
			shader.interface.bindings[0] = Binding::new(
				3,
				resource_management::resources::material::BindingKind::CombinedImageSampler { view },
				1,
				None,
				true,
				false,
			);
			hash_shader_source(&shader)
		};
		assert_ne!(
			sampled_hash(resource_management::resources::material::TextureView::Texture2D),
			sampled_hash(resource_management::resources::material::TextureView::Texture3D)
		);

		let partitioned_left = inline_descriptor("ab", "c", "de", "f", ShaderTypes::Compute, interface(None, Vec::new()));
		let partitioned_right = inline_descriptor("a", "bc", "d", "ef", ShaderTypes::Compute, interface(None, Vec::new()));
		assert_ne!(hash_shader_source(&partitioned_left), hash_shader_source(&partitioned_right));
	}

	#[test]
	fn every_shader_stage_has_a_unique_hash_tag_and_matching_ghi_stage() {
		let mappings = [
			(ShaderTypes::Vertex, ghi::Stages::VERTEX),
			(ShaderTypes::Fragment, ghi::Stages::FRAGMENT),
			(ShaderTypes::Compute, ghi::Stages::COMPUTE),
			(ShaderTypes::Task, ghi::Stages::TASK),
			(ShaderTypes::Mesh, ghi::Stages::MESH),
			(ShaderTypes::RayGen, ghi::Stages::RAYGEN),
			(ShaderTypes::ClosestHit, ghi::Stages::CLOSEST_HIT),
			(ShaderTypes::AnyHit, ghi::Stages::ANY_HIT),
			(ShaderTypes::Intersection, ghi::Stages::INTERSECTION),
			(ShaderTypes::Miss, ghi::Stages::MISS),
			(ShaderTypes::Callable, ghi::Stages::CALLABLE),
		];
		let mut tags = mappings.iter().map(|(stage, _)| shader_type_tag(*stage)).collect::<Vec<_>>();
		tags.sort_unstable();
		tags.dedup();
		assert_eq!(tags.len(), mappings.len());

		for (resource_stage, expected) in mappings {
			assert_eq!(ghi::Stages::from(shader_type_to_ghi(resource_stage)), expected);
		}
	}

	#[test]
	fn binding_access_flags_preserve_all_read_write_combinations() {
		assert_eq!(
			binding_access_policy(&storage_binding(0, 1, false, false)),
			ghi::AccessPolicies::empty()
		);
		assert_eq!(
			binding_access_policy(&storage_binding(0, 1, true, false)),
			ghi::AccessPolicies::READ
		);
		assert_eq!(
			binding_access_policy(&storage_binding(0, 1, false, true)),
			ghi::AccessPolicies::WRITE
		);
		assert_eq!(
			binding_access_policy(&storage_binding(0, 1, true, true)),
			ghi::AccessPolicies::READ_WRITE
		);
	}

	#[test]
	fn baked_artifacts_reconstruct_the_expected_backend_variant_and_metadata() {
		let binary = [1, 2, 3, 4];
		assert!(matches!(
			shader_artifact_source(&ShaderArtifact::Spirv, Some((8, 4, 2)), &binary).expect("SPIR-V source"),
			ghi::shader::Sources::SPIRV(bytes) if bytes == binary
		));
		assert!(matches!(
			shader_artifact_source(&ShaderArtifact::Dxil, Some((8, 4, 2)), &binary).expect("DXIL source"),
			ghi::shader::Sources::DXIL(bytes) if bytes == binary
		));

		let hlsl = ShaderArtifact::Hlsl {
			entry_point: "compute_main".to_string(),
		};
		assert!(matches!(
			shader_artifact_source(&hlsl, None, b"[numthreads(1, 1, 1)] void compute_main() {}").expect("HLSL source"),
			ghi::shader::Sources::HLSL { entry_point, .. } if entry_point == "compute_main"
		));

		let msl = ShaderArtifact::Msl {
			entry_point: "main0".to_string(),
		};
		assert!(matches!(
			shader_artifact_source(&msl, None, b"kernel void main0() {}").expect("MSL source"),
			ghi::shader::Sources::MTL { entry_point, .. } if entry_point == "main0"
		));

		let mtlb = ShaderArtifact::Mtlb {
			entry_point: "main0".to_string(),
		};
		assert!(matches!(
			shader_artifact_source(&mtlb, Some((8, 4, 2)), &binary).expect("metallib source"),
			ghi::shader::Sources::MTLB {
				binary: bytes,
				entry_point: "main0",
				threadgroup_size: Some(extent),
			} if bytes == binary && extent == Extent::new(8, 4, 2)
		));
	}

	#[test]
	fn text_artifacts_reject_invalid_utf8_with_actionable_errors() {
		let invalid = [0xFF];
		let hlsl = ShaderArtifact::Hlsl {
			entry_point: "main".to_string(),
		};
		let msl = ShaderArtifact::Msl {
			entry_point: "main".to_string(),
		};

		let hlsl_error = match shader_artifact_source(&hlsl, None, &invalid) {
			Err(error) => error,
			Ok(_) => panic!("invalid HLSL bytes must fail"),
		};
		let msl_error = match shader_artifact_source(&msl, None, &invalid) {
			Err(error) => error,
			Ok(_) => panic!("invalid MSL bytes must fail"),
		};
		assert!(hlsl_error.contains("most likely cause"));
		assert!(msl_error.contains("most likely cause"));
	}
}
