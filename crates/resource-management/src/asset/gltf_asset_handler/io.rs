use super::*;
pub(crate) async fn load_gltf_buffers(
	asset_storage_backend: &dyn asset::DynStorageBackend,
	source: ResourceId<'_>,
	gltf: &gltf::Gltf,
	mut binary_blob: Option<std::borrow::Cow<'_, [u8]>>,
	required: Option<&[bool]>,
	allocator: &dyn std::alloc::Allocator,
) -> Result<Vec<gltf::buffer::Data>, LoadErrors> {
	use utils::r#async::StreamExt as _;

	let requests = gltf.buffers().map(|buffer| {
		let skipped = required.is_some_and(|required| !required.get(buffer.index()).copied().unwrap_or(false));
		let binary_data = if !skipped && matches!(buffer.source(), gltf::buffer::Source::Bin) {
			binary_blob.take()
		} else {
			None
		};

		async move {
			if skipped {
				return Ok((buffer.index(), gltf::buffer::Data(Vec::new())));
			}

			let mut data = match buffer.source() {
				gltf::buffer::Source::Bin => binary_data.map(std::borrow::Cow::into_owned).ok_or_else(|| {
					log::error!("glTF binary buffer is missing. The most likely cause is a GLB without its required BIN chunk.");
					LoadErrors::FailedToProcess
				})?,
				gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => decode_gltf_buffer_data_uri(uri)?,
				gltf::buffer::Source::Uri(uri) => {
					let buffer_url = resolve_gltf_uri(source, uri)?;
					let (bytes, ..) = asset_storage_backend
						.resolve_in(ResourceId::new(&buffer_url), allocator)
						.await
						.map_err(|_| {
							log::error!(
								"glTF external buffer could not be loaded. The most likely cause is a missing file-local URI '{buffer_url}'."
							);
							LoadErrors::AssetCouldNotBeLoaded
						})?;
					copy_gltf_buffer_bytes(&bytes)?
				}
			};

			let raw_length = data.len();
			if raw_length < buffer.length() {
				log::error!(
					"glTF buffer is shorter than declared. The most likely cause is truncated data for buffer {}: expected at least {} bytes but loaded {}.",
					buffer.index(),
					buffer.length(),
					raw_length
				);
				return Err(LoadErrors::FailedToProcess);
			}

			// Reserve once before adding the alignment bytes required by glTF buffer-view access.
			let aligned_length = aligned_gltf_buffer_length(raw_length)?;
			if data.capacity() < aligned_length {
				data.reserve_exact(aligned_length - raw_length);
			}
			data.resize(aligned_length, 0);
			Ok((buffer.index(), gltf::buffer::Data(data)))
		}
	});

	// External files are independent; cap open reads while retaining document buffer order.
	let mut buffers = utils::r#async::stream::iter(requests)
		.buffer_unordered(8)
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect::<Result<Vec<_>, _>>()?;
	buffers.sort_unstable_by_key(|(index, _)| *index);
	Ok(buffers.into_iter().map(|(_, data)| data).collect())
}

/// Decodes a glTF data URI into storage with enough capacity for final four-byte alignment.
pub(crate) fn decode_gltf_buffer_data_uri(uri: &str) -> Result<Vec<u8>, LoadErrors> {
	let data = uri.strip_prefix("data:").ok_or_else(|| {
		log::error!("glTF data buffer URI is invalid. The most likely cause is a missing data URI payload.");
		LoadErrors::FailedToProcess
	})?;
	let encoded = data.split_once(";base64,").map_or(data, |(_, encoded)| encoded);
	let decoded_capacity = encoded
		.len()
		.checked_add(3)
		.and_then(|length| length.checked_div(4))
		.and_then(|chunks| chunks.checked_mul(3))
		.ok_or_else(|| {
			log::error!("glTF data buffer is too large. The most likely cause is an overflowing data URI length.");
			LoadErrors::FailedToProcess
		})?;
	let mut decoded = vec![0; aligned_gltf_buffer_length(decoded_capacity)?];
	let written = base64::decode_config_slice(encoded, base64::STANDARD, &mut decoded).map_err(|error| {
		log::error!("glTF data buffer could not be decoded. The most likely cause is a malformed data URI: {error}.");
		LoadErrors::FailedToProcess
	})?;
	decoded.truncate(written);
	Ok(decoded)
}

/// Copies external buffer bytes once into storage already reserved for glTF alignment padding.
pub(crate) fn copy_gltf_buffer_bytes(bytes: &[u8]) -> Result<Vec<u8>, LoadErrors> {
	let mut data = Vec::with_capacity(aligned_gltf_buffer_length(bytes.len())?);
	data.extend_from_slice(bytes);
	Ok(data)
}

/// Rounds a glTF payload length up to its required four-byte buffer alignment.
pub(crate) fn aligned_gltf_buffer_length(length: usize) -> Result<usize, LoadErrors> {
	length.checked_add(3).map(|length| length & !3).ok_or_else(|| {
		log::error!("glTF buffer is too large. The most likely cause is a payload length that overflows alignment.");
		LoadErrors::FailedToProcess
	})
}

/// Finds the image addressed by a glTF resource fragment.
/// Generated fragments use `images/<index>...` so unnamed GLB images remain addressable.
pub(crate) fn image_for_gltf_fragment<'a>(gltf: &'a gltf::Gltf, fragment: &str) -> Option<gltf::Image<'a>> {
	if let Some(index) = generated_image_fragment_index(fragment) {
		return gltf.images().find(|image| image.index() == index as usize);
	}

	gltf.images().find(|image| image.name() == Some(fragment))
}

pub(crate) fn generated_image_fragment_index(fragment: &str) -> Option<u32> {
	let suffix = fragment.strip_prefix("images/")?;
	let digits = suffix
		.chars()
		.take_while(|character| character.is_ascii_digit())
		.collect::<String>();
	if digits.is_empty() {
		None
	} else {
		digits.parse().ok()
	}
}

/// Loads a glTF image from embedded buffer data, data URIs, or file-local URI references.
/// File-local references are resolved through the engine asset backend so ad-hoc textures inside `.gltf` assets do not need to be standalone engine resources.
pub(crate) async fn load_gltf_image_data(
	asset_storage_backend: &dyn asset::DynStorageBackend,
	mesh_url: ResourceId<'_>,
	image: gltf::Image<'_>,
	buffers: &[gltf::buffer::Data],
	allocator: &dyn std::alloc::Allocator,
) -> Result<gltf::image::Data, LoadErrors> {
	match image.source() {
		gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
			let image_url = resolve_gltf_uri(mesh_url, uri)?;
			let (bytes, ..) = asset_storage_backend
				.resolve_in(ResourceId::new(&image_url), allocator)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;
			decode_external_gltf_image(&bytes)
		}
		_ => gltf::image::Data::from_source(image.source(), None, buffers).map_err(|_| LoadErrors::FailedToProcess),
	}
}

pub(crate) fn resolve_gltf_uri(mesh_url: ResourceId<'_>, uri: &str) -> Result<String, LoadErrors> {
	if uri.contains("://") || uri.starts_with('/') {
		return Ok(uri.to_string());
	}

	let uri = urlencoding::decode(uri).map_err(|error| {
		log::error!("glTF file-local URI is invalid. The most likely cause is malformed percent encoding: {error}.");
		LoadErrors::FailedToProcess
	})?;
	let base = mesh_url.get_base();
	let parent = Path::new(base.as_ref()).parent();
	if let Some(parent) = parent {
		Ok(parent.join(uri.as_ref()).to_string_lossy().replace('\\', "/"))
	} else {
		Ok(uri.into_owned())
	}
}

pub(crate) fn decode_external_gltf_image(bytes: &[u8]) -> Result<gltf::image::Data, LoadErrors> {
	let image = image::load_from_memory(bytes).map_err(|_| LoadErrors::FailedToProcess)?;
	let rgba = image.to_rgba8();
	let (width, height) = rgba.dimensions();

	Ok(gltf::image::Data {
		pixels: rgba.into_raw(),
		format: gltf::image::Format::R8G8B8A8,
		width,
		height,
	})
}

/// Processes decoded glTF pixels and stores their image metadata and binary payload.
pub(crate) fn store_gltf_image(
	context: BakeContext<'_>,
	id: ResourceId<'_>,
	image: gltf::image::Data,
	semantic: Semantic,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<crate::SerializableResource, LoadErrors> {
	let format = gltf_image_format(image.format)?;
	let image_description = ImageDescription {
		format,
		extent: Extent::rectangle(image.width, image.height),
		semantic,
		gamma: gamma_from_semantic(semantic),
		generate_mipmaps: mip_backend.is_some(),
	};

	let (resource, data) = process_image_with_mip_backend_in(
		id,
		image_description,
		image.pixels.into_boxed_slice(),
		context.allocator(),
		mip_backend,
	)?;
	context.store_resource(resource, &data)
}

pub(crate) fn gltf_image_format(format: gltf::image::Format) -> Result<Formats, LoadErrors> {
	match format {
		gltf::image::Format::R8G8B8 => Ok(Formats::RGB8),
		gltf::image::Format::R8G8B8A8 => Ok(Formats::RGBA8),
		gltf::image::Format::R16G16B16 => Ok(Formats::RGB16),
		gltf::image::Format::R16G16B16A16 => Ok(Formats::RGBA16),
		_ => Err(LoadErrors::UnsupportedType),
	}
}

/// Collects unique glTF image dependencies in material-slot order.
/// The generated shader uses material texture-variable names while the runtime fills those slots with bindless descriptor indices.
pub(crate) fn collect_gltf_texture_dependencies(
	material: &BrdfMaterialDescription,
) -> Result<Vec<GltfTextureDependency>, BrdfMaterialValidationError> {
	material.validate()?;
	let mut dependencies = Vec::new();
	let BrdfNode::MetallicRoughness(surface) = material.node(material.surface)? else {
		return Ok(dependencies);
	};

	collect_texture_dependencies_from_node(material, surface.base_color, Semantic::Albedo, &mut dependencies)?;
	collect_texture_dependencies_from_node(material, surface.metallic, Semantic::Metallic, &mut dependencies)?;
	collect_texture_dependencies_from_node(material, surface.roughness, Semantic::Roughness, &mut dependencies)?;
	if let Some(normal) = surface.normal {
		collect_texture_dependencies_from_node(material, normal, Semantic::Normal, &mut dependencies)?;
	}
	if let Some(occlusion) = surface.occlusion {
		collect_texture_dependencies_from_node(material, occlusion, Semantic::AO, &mut dependencies)?;
	}
	if let Some(emission) = surface.emission {
		collect_texture_dependencies_from_node(material, emission, Semantic::Emissive, &mut dependencies)?;
	}

	Ok(dependencies)
}

pub(crate) fn collect_texture_dependencies_from_node(
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
	semantic: Semantic,
	dependencies: &mut Vec<GltfTextureDependency>,
) -> Result<(), BrdfMaterialValidationError> {
	match material.node(node)? {
		BrdfNode::Texture(texture) => push_gltf_texture_dependency(dependencies, texture.image_index, semantic),
		BrdfNode::Multiply { left, right } => {
			collect_texture_dependencies_from_node(material, *left, semantic, dependencies)?;
			collect_texture_dependencies_from_node(material, *right, semantic, dependencies)?;
		}
		BrdfNode::ExtractChannel { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, semantic, dependencies)?;
		}
		BrdfNode::NormalMap { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, Semantic::Normal, dependencies)?;
		}
		BrdfNode::Occlusion { source, .. } => {
			collect_texture_dependencies_from_node(material, *source, Semantic::AO, dependencies)?;
		}
		BrdfNode::Emission { color } => {
			collect_texture_dependencies_from_node(material, *color, Semantic::Emissive, dependencies)?;
		}
		BrdfNode::Constant(_) | BrdfNode::MetallicRoughness(_) => {}
	}

	Ok(())
}

pub(crate) fn push_gltf_texture_dependency(
	dependencies: &mut Vec<GltfTextureDependency>,
	image_index: u32,
	semantic: Semantic,
) {
	if let Some(existing) = dependencies
		.iter_mut()
		.find(|dependency| dependency.image_index == image_index)
	{
		existing.semantic = merge_texture_semantics(existing.semantic, semantic);
		return;
	}

	dependencies.push(GltfTextureDependency { image_index, semantic });
}

pub(crate) fn merge_texture_semantics(left: Semantic, right: Semantic) -> Semantic {
	if left == right {
		return left;
	}

	// Prefer color semantics when an unusual glTF reuses the same image for color and data textures.
	// This avoids accidentally sampling an albedo texture as linear data after processing.
	match (left, right) {
		(Semantic::Albedo, _) | (_, Semantic::Albedo) => Semantic::Albedo,
		(Semantic::Emissive, _) | (_, Semantic::Emissive) => Semantic::Emissive,
		(Semantic::Normal, _) | (_, Semantic::Normal) => Semantic::Normal,
		(Semantic::AO, _) | (_, Semantic::AO) => Semantic::AO,
		(Semantic::Metallic, _) | (_, Semantic::Metallic) => Semantic::Metallic,
		(Semantic::Roughness, _) | (_, Semantic::Roughness) => Semantic::Roughness,
		_ => left,
	}
}

pub(crate) async fn store_gltf_texture_dependencies(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	dependencies: &[GltfTextureDependency],
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<Vec<VariantVariableModel>, LoadErrors> {
	use utils::r#async::StreamExt as _;

	let requests = dependencies.iter().map(|dependency| async move {
		let image = gltf
			.images()
			.find(|image| image.index() == dependency.image_index as usize)
			.ok_or(LoadErrors::FailedToProcess)?;
		let id = generated_gltf_image_id(mesh_url, image.index() as u32, image.name());
		let image_ref =
			load_and_store_gltf_image(context, mesh_url, &id, image, buffers, dependency.semantic, mip_backend).await?;

		Ok(VariantVariableModel {
			name: material_texture_variable_name(dependency.image_index),
			r#type: "Texture2D".to_string(),
			value: ValueModel::Image(image_ref),
		})
	});

	// Distinct image dependencies can overlap file I/O; ordered buffering keeps material variables deterministic.
	utils::r#async::stream::iter(requests)
		.buffered(4)
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect()
}

/// Loads one glTF image dependency and stores its processed resource.
pub(crate) async fn load_and_store_gltf_image(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	id: &str,
	image: gltf::Image<'_>,
	buffers: &[gltf::buffer::Data],
	semantic: Semantic,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ReferenceModel<Image>, LoadErrors> {
	let image_data =
		load_gltf_image_data(context.asset_storage_backend(), mesh_url, image, buffers, context.allocator()).await?;
	store_gltf_image(context, ResourceId::new(id), image_data, semantic, mip_backend).map(Into::into)
}

pub(crate) fn generated_material_json(variables: &[VariantVariableModel]) -> crate::asset::JsonObject {
	let variables = variables
		.iter()
		.map(|variable| serde_json::json!({ "name": variable.name, "data_type": variable.r#type }))
		.collect::<Vec<_>>();

	serde_json::json!({ "variables": variables })
		.as_object()
		.expect("generated material JSON should be an object")
		.clone()
}

pub(crate) fn generated_gltf_image_id(mesh_url: ResourceId<'_>, image_index: u32, image_name: Option<&str>) -> String {
	let readable_name = image_name
		.map(sanitize_material_name)
		.filter(|name| !name.is_empty())
		.map(|name| format!("_{name}"))
		.unwrap_or_default();
	format!("{}#images/{image_index}{readable_name}", mesh_url.as_ref())
}
