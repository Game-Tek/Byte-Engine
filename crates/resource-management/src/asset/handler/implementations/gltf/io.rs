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
					// Copy once into storage already reserved for the alignment padding.
					let mut data = Vec::with_capacity(aligned_gltf_buffer_length(bytes.len())?);
					data.extend_from_slice(&bytes);
					data
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
			data.reserve_exact(aligned_length - raw_length);
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
	match generated_image_fragment_index(fragment) {
		// `images` yields images in index order.
		Some(index) => gltf.images().nth(index as usize),
		None => gltf.images().find(|image| image.name() == Some(fragment)),
	}
}

/// Reads the image index that leads a generated `images/<index>...` fragment.
pub(crate) fn generated_image_fragment_index(fragment: &str) -> Option<u32> {
	let suffix = fragment.strip_prefix("images/")?;
	let end = suffix
		.find(|character: char| !character.is_ascii_digit())
		.unwrap_or(suffix.len());

	// An empty run of digits fails to parse.
	suffix[..end].parse().ok()
}

pub(crate) fn resolve_gltf_uri(mesh_url: ResourceId<'_>, uri: &str) -> Result<String, LoadErrors> {
	if uri.contains("://") || uri.starts_with('/') {
		return Ok(uri.to_string());
	}

	let uri = urlencoding::decode(uri).map_err(|error| {
		log::error!("glTF file-local URI is invalid. The most likely cause is malformed percent encoding: {error}.");

		LoadErrors::FailedToProcess
	})?;

	Ok(mesh_url.resolve_relative(&uri))
}

/// Maps glTF decoder layouts to the source metadata consumed by the common image processor.
pub(crate) fn gltf_image_source_layout(format: gltf::image::Format) -> Result<(SourceChannels, SourceEncoding), LoadErrors> {
	match format {
		gltf::image::Format::R8 => Ok((SourceChannels::Luminance, SourceEncoding::U8)),
		gltf::image::Format::R8G8 => Ok((SourceChannels::LuminanceAlpha, SourceEncoding::U8)),
		gltf::image::Format::R8G8B8 => Ok((SourceChannels::RGB, SourceEncoding::U8)),
		gltf::image::Format::R8G8B8A8 => Ok((SourceChannels::RGBA, SourceEncoding::U8)),
		gltf::image::Format::R16 => Ok((SourceChannels::Luminance, SourceEncoding::U16NativeEndian)),
		gltf::image::Format::R16G16 => Ok((SourceChannels::LuminanceAlpha, SourceEncoding::U16NativeEndian)),
		gltf::image::Format::R16G16B16 => Ok((SourceChannels::RGB, SourceEncoding::U16NativeEndian)),
		gltf::image::Format::R16G16B16A16 => Ok((SourceChannels::RGBA, SourceEncoding::U16NativeEndian)),
		_ => Err(LoadErrors::UnsupportedType),
	}
}

/// Merges how one material samples each image into `semantics`, which is indexed by image.
///
/// The material is validated before any texture is read, so a material that fails validation leaves `semantics`
/// unchanged. Images outside `semantics` are ignored.
pub(crate) fn merge_gltf_texture_semantics(
	material: &BrdfMaterialDescription,
	semantics: &mut [Option<Semantic>],
) -> Result<(), BrdfMaterialValidationError> {
	material.validate()?;

	let BrdfNode::MetallicRoughness(surface) = material.node(material.surface)? else {
		return Ok(());
	};

	for (node, semantic) in [
		(Some(surface.base_color), Semantic::Albedo),
		(Some(surface.metallic), Semantic::Metallic),
		(Some(surface.roughness), Semantic::Roughness),
		(surface.normal, Semantic::Normal),
		(surface.occlusion, Semantic::AO),
		(surface.emission, Semantic::Emissive),
	] {
		if let Some(node) = node {
			merge_texture_semantics_from_node(material, node, semantic, semantics)?;
		}
	}

	Ok(())
}

pub(crate) fn merge_texture_semantics_from_node(
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
	semantic: Semantic,
	semantics: &mut [Option<Semantic>],
) -> Result<(), BrdfMaterialValidationError> {
	match material.node(node)? {
		BrdfNode::Texture(texture) => {
			if let Some(merged) = semantics.get_mut(texture.image_index as usize) {
				*merged = Some(merged.map_or(semantic, |merged| merge_texture_semantics(merged, semantic)));
			}
		}
		BrdfNode::Multiply { left, right } => {
			merge_texture_semantics_from_node(material, *left, semantic, semantics)?;

			merge_texture_semantics_from_node(material, *right, semantic, semantics)?;
		}
		BrdfNode::ExtractChannel { source, channel } => {
			// A metallic or roughness read of a channel the packing keeps lets the image bake as a two-channel map. Any
			// other read of the image outranks it in `merge_texture_semantics`, so only fully packable images pack.
			let packable = matches!(semantic, Semantic::Metallic | Semantic::Roughness)
				&& METALLIC_ROUGHNESS_PACKING.stored_channel(channel.index()).is_some()
				&& matches!(material.node(*source)?, BrdfNode::Texture(_));
			let semantic = if packable { Semantic::MetallicRoughness } else { semantic };
			merge_texture_semantics_from_node(material, *source, semantic, semantics)?;
		}
		BrdfNode::NormalMap { source, .. } => {
			merge_texture_semantics_from_node(material, *source, Semantic::Normal, semantics)?;
		}
		BrdfNode::Occlusion { source, .. } => {
			merge_texture_semantics_from_node(material, *source, Semantic::AO, semantics)?;
		}
		BrdfNode::Emission { color } => {
			merge_texture_semantics_from_node(material, *color, Semantic::Emissive, semantics)?;
		}
		BrdfNode::Constant(_) | BrdfNode::MetallicRoughness(_) => {}
	}

	Ok(())
}

/// Picks the semantic an image bakes with when materials sample it in more than one way.
///
/// For the semantics glTF materials read with, the pick is a fixed priority, so the order in which reads merge does not
/// change the result.
pub(crate) fn merge_texture_semantics(left: Semantic, right: Semantic) -> Semantic {
	// Prefer color semantics when an unusual glTF reuses the same image for color and data textures.
	// This avoids accidentally sampling an albedo texture as linear data after processing.
	match (left, right) {
		(Semantic::Albedo, _) | (_, Semantic::Albedo) => Semantic::Albedo,
		(Semantic::Emissive, _) | (_, Semantic::Emissive) => Semantic::Emissive,
		(Semantic::Normal, _) | (_, Semantic::Normal) => Semantic::Normal,
		(Semantic::AO, _) | (_, Semantic::AO) => Semantic::AO,
		(Semantic::Metallic, _) | (_, Semantic::Metallic) => Semantic::Metallic,
		(Semantic::Roughness, _) | (_, Semantic::Roughness) => Semantic::Roughness,
		// A packed map drops channels, so any other use of the image keeps all of them.
		(Semantic::MetallicRoughness, other) | (other, Semantic::MetallicRoughness) => other,
		_ => left,
	}
}

/// Returns how the glTF's materials sample each image, by image index, merged across every material that references
/// it.
///
/// Image fragments bake with these semantics and generated materials remap their channel reads with them, so a
/// texture decodes the same way whether a material or a direct request bakes it, and is sampled where it was stored.
/// An image no material samples is `None`.
pub(crate) fn gltf_image_semantics(gltf: &gltf::Gltf) -> Vec<Option<Semantic>> {
	let mut semantics = vec![None; gltf.images().count()];
	for material in gltf.materials() {
		// A material whose graph fails validation samples no texture, so its error needs no handling here.
		let _ = merge_gltf_texture_semantics(&brdf_material_from_gltf(&material), &mut semantics);
	}
	semantics
}

/// Loads one glTF image, reading only the bytes of the buffer view that holds it.
///
/// A texture baked on its own must not copy the whole binary chunk, which would repeat the copy for every texture
/// in the container.
pub(crate) async fn load_gltf_fragment_image(
	context: BakeContext<'_>,
	source_id: ResourceId<'_>,
	image: gltf::Image<'_>,
	binary_blob: Option<&[u8]>,
) -> Result<gltf::image::Data, LoadErrors> {
	let (view, mime_type) = match image.source() {
		gltf::image::Source::View { view, mime_type } => (view, mime_type),
		// File-local references resolve through the engine asset backend, so ad-hoc textures inside `.gltf` assets do
		// not need to be standalone engine resources.
		gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
			let image_url = resolve_gltf_uri(source_id, uri)?;
			let (bytes, ..) = context
				.resolve(ResourceId::new(&image_url))
				.await
				.map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?;
			let image = image::load_from_memory(&bytes)
				.map_err(|_| LoadErrors::FailedToProcess)?
				.into_rgba8();
			let (width, height) = image.dimensions();
			return Ok(gltf::image::Data {
				pixels: image.into_raw(),
				format: gltf::image::Format::R8G8B8A8,
				width,
				height,
			});
		}
		// Only data URIs reach this arm.
		source => return gltf::image::Data::from_source(source, None, &[]).map_err(|_| LoadErrors::FailedToProcess),
	};

	let range = view.offset()..view.offset().checked_add(view.length()).ok_or(LoadErrors::FailedToProcess)?;

	let missing_view = || {
		log::error!(
			"glTF image {} is outside its buffer view. The most likely cause is a truncated or malformed buffer.",
			image.index()
		);

		LoadErrors::FailedToProcess
	};

	match view.buffer().source() {
		gltf::buffer::Source::Bin => {
			let blob = binary_blob.ok_or_else(|| {
				log::error!("glTF binary buffer is missing. The most likely cause is a GLB without its required BIN chunk.");

				LoadErrors::FailedToProcess
			})?;

			decode_gltf_view_image(blob.get(range).ok_or_else(missing_view)?, mime_type)
		}
		gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
			let data = decode_gltf_buffer_data_uri(uri)?;

			decode_gltf_view_image(data.get(range).ok_or_else(missing_view)?, mime_type)
		}
		gltf::buffer::Source::Uri(uri) => {
			let buffer_url = resolve_gltf_uri(source_id, uri)?;

			let (bytes, ..) = context.resolve(ResourceId::new(&buffer_url)).await.map_err(|_| {
				log::error!(
					"glTF external buffer could not be loaded. The most likely cause is a missing file-local URI '{buffer_url}'."
				);

				LoadErrors::AssetCouldNotBeLoaded
			})?;

			decode_gltf_view_image(bytes.get(range).ok_or_else(missing_view)?, mime_type)
		}
	}
}

/// Decodes an image stored in a glTF buffer view, keeping the channel layout the `gltf` importer would report.
pub(crate) fn decode_gltf_view_image(encoded: &[u8], mime_type: &str) -> Result<gltf::image::Data, LoadErrors> {
	let format = match mime_type {
		"image/png" => image::ImageFormat::Png,
		"image/jpeg" => image::ImageFormat::Jpeg,
		_ => {
			log::error!(
				"glTF image uses unsupported MIME type '{mime_type}'. The most likely cause is an image encoding other than PNG or JPEG."
			);

			return Err(LoadErrors::UnsupportedType);
		}
	};

	let decoded = image::load_from_memory_with_format(encoded, format).map_err(|_| LoadErrors::FailedToProcess)?;

	let (width, height) = (decoded.width(), decoded.height());

	let format = match &decoded {
		image::DynamicImage::ImageLuma8(_) => gltf::image::Format::R8,
		image::DynamicImage::ImageLumaA8(_) => gltf::image::Format::R8G8,
		image::DynamicImage::ImageRgb8(_) => gltf::image::Format::R8G8B8,
		image::DynamicImage::ImageRgba8(_) => gltf::image::Format::R8G8B8A8,
		image::DynamicImage::ImageLuma16(_) => gltf::image::Format::R16,
		image::DynamicImage::ImageLumaA16(_) => gltf::image::Format::R16G16,
		image::DynamicImage::ImageRgb16(_) => gltf::image::Format::R16G16B16,
		image::DynamicImage::ImageRgba16(_) => gltf::image::Format::R16G16B16A16,
		_ => return Err(LoadErrors::UnsupportedType),
	};

	Ok(gltf::image::Data {
		pixels: decoded.into_bytes(),
		format,
		width,
		height,
	})
}

pub(crate) fn generated_gltf_image_id(mesh_url: ResourceId<'_>, image_index: u32, image_name: Option<&str>) -> String {
	let readable_name = image_name
		.map(|name| format!("_{}", sanitize_material_name(name)))
		.unwrap_or_default();

	format!("{}#images/{image_index}{readable_name}", mesh_url.as_ref())
}
