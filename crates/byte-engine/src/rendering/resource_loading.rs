//! Transitional synchronous access to asynchronous resource requests.
//!
//! Renderer resource adoption is still synchronous. Keep its blocking bridge in
//! one private module so the next asynchronous renderer-loading change can
//! delete it without retaining a public blocking resource interface.

use std::future::Future;

use resource_management::{
	r#async::Executor, Reference, ReferenceModel, Resource, ResourceManager, SerializableResource, Solver,
};

/// The `LoadedShader` struct contains a persisted shader handle and its interface metadata.
pub(crate) struct LoadedShader {
	pub(crate) handle: ghi::ShaderHandle,
	pub(crate) stage: resource_management::types::ShaderTypes,
	pub(crate) interface: resource_management::resources::material::ShaderInterface,
}

thread_local! {
	// Reuse one local executor per rendering thread instead of constructing a
	// runtime for every resource request during the transition.
	static RESOURCE_REQUEST_EXECUTOR: Executor = Executor::new().expect(
		"Failed to create the renderer resource executor. The most likely cause is that the platform I/O driver could not be initialized."
	);
}

/// Waits for one asynchronous resource request from synchronous renderer code.
pub(crate) fn request<T>(resource_manager: &ResourceManager, id: &str) -> Result<Reference<T>, String>
where
	T: Resource + 'static,
	for<'de> ReferenceModel<T::Model>: Solver<'de, Reference<T>>,
	SerializableResource: TryInto<ReferenceModel<T::Model>>,
{
	block_on(resource_manager.request(id))
}

/// Waits for one resource future while renderer resource adoption remains synchronous.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
	RESOURCE_REQUEST_EXECUTOR.with(|executor| executor.block_on(future))
}

/// Loads a baked shader resource and creates its GHI shader handle.
pub(crate) fn load_shader(
	context: &mut ghi::implementation::Context,
	resource_manager: &ResourceManager,
	id: &str,
	name: &str,
) -> Result<LoadedShader, String> {
	use ghi::context::ContextCreate as _;
	use resource_management::resource::ReadStorageBackend as _;

	let mut shader: Reference<resource_management::resources::material::Shader> = request(resource_manager, id).map_err(|error| {
		format!(
			"Failed to load baked shader resource '{id}': {error}. The most likely cause is that BELD did not bake the shader or its source asset is unavailable."
		)
	})?;
	let stage = shader.resource.stage;
	let interface = shader.resource.interface.clone();
	let artifact = shader.resource.artifact.clone();
	let backing = block_on(shader.consume_reader().into_backing_storage()).map_err(|_| {
		format!("Failed to load baked shader bytes for '{id}'. The most likely cause is an unsupported shader resource reader.")
	})?;
	let source = shader_artifact_source(&artifact, interface.workgroup_size, backing.as_slice())?;
	let descriptors = interface.bindings.iter().map(binding_to_descriptor).collect::<Vec<_>>();
	let handle = context
		.create_shader(Some(name), source, shader_type_to_ghi(stage), descriptors.iter().copied())
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

pub(crate) fn shader_artifact_source<'a>(
	artifact: &'a resource_management::resources::material::ShaderArtifact,
	workgroup_size: Option<(u32, u32, u32)>,
	bytes: &'a [u8],
) -> Result<ghi::shader::Sources<'a>, String> {
	use resource_management::resources::material::ShaderArtifact;

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
			threadgroup_size: workgroup_size.map(|(width, height, depth)| utils::Extent::new(width, height, depth)),
		}),
	}
}

pub(crate) fn binding_to_descriptor(
	binding: &resource_management::resources::material::Binding,
) -> ghi::ShaderResourceDescriptor {
	use resource_management::resources::material::{BindingKind, TextureView};

	let kind = match binding.kind {
		BindingKind::StorageBuffer => ghi::ResourceKind::StorageBuffer,
		BindingKind::CombinedImageSampler { .. } => ghi::ResourceKind::CombinedImageSampler,
		BindingKind::StorageImage => ghi::ResourceKind::StorageImage,
	};
	let access = (if binding.read {
		ghi::AccessPolicies::READ
	} else {
		ghi::AccessPolicies::empty()
	}) | if binding.write {
		ghi::AccessPolicies::WRITE
	} else {
		ghi::AccessPolicies::empty()
	};
	let descriptor = ghi::ShaderResourceDescriptor::new(ghi::ResourceSlot::new(binding.slot), kind, binding.count, access);
	let descriptor =
		match binding.kind {
			BindingKind::StorageBuffer => descriptor.buffer_stride(binding.buffer_stride.expect(
				"Missing persisted storage-buffer stride. The most likely cause is a stale shader interface resource.",
			)),
			_ => descriptor,
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

pub(crate) fn shader_type_to_ghi(stage: resource_management::types::ShaderTypes) -> ghi::ShaderTypes {
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
