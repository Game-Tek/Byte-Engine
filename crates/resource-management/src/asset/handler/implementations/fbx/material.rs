#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MaterialKey {
	Default,
	Material(u32),
}

/// Resolves each used FBX material exactly once, honoring sidecar material bindings before generating a fallback.
///
/// Maps each material to what its sidecar binds it to, an authored or inline variant, or else a generated BEMX graph,
/// then resolves them all through [`resolve_container_materials`]. Returns the variants in the order of `keys`, which
/// [`used_material_keys`] returns.
pub(crate) async fn resolve_fbx_materials(
	context: BakeContext<'_>,
	spec: Option<&Value>,
	url: ResourceId<'_>,
	scene: &ufbx::Scene,
	keys: &[MaterialKey],
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	let sources = keys
		.iter()
		.map(|&key| {
			let (material, name) = fbx_material_for_key(scene, key);

			MaterialSource::new(
				context,
				url,
				fbx_material_binding(spec, url, name),
				generated_fbx_material_base_id(url, key, material),
				|base_id| {
					let material = fbx_material(material).map_err(|error| {
						log::error!("Failed to generate the FBX material '{base_id}': {error}");
						LoadErrors::FailedToProcess
					})?;

					Ok(MaterialSource::Generated(GeneratedMaterial { base_id, material }))
				},
			)
		})
		.collect::<Result<Vec<_>, LoadErrors>>()?;

	// FBX materials index images by their position in the scene's texture list.
	let image_ids = scene
		.textures
		.iter()
		.map(|texture| generated_fbx_image_id(url, texture))
		.collect::<Vec<_>>();

	resolve_container_materials(context, url, &image_ids, sources).await
}

/// Finds the material that `url`, a `#materials/<index>_<name>.variant` or `.material` ID, names among the materials
/// the scene's meshes use: a generated material, or an inline variant from the sidecar, which shares its ID.
///
/// Materials a sidecar binds to an authored `.bema` variant are left out, because the container stores nothing for
/// them.
pub(crate) fn generated_material_key_for_id(
	scene: &ufbx::Scene,
	spec: Option<&Value>,
	url: ResourceId<'_>,
	allocator: &dyn Allocator,
) -> Option<MaterialKey> {
	let base = url.get_base();
	let source_id = ResourceId::new(base.as_ref());

	used_material_keys(scene, allocator).into_iter().find(|&key| {
		let (material, name) = fbx_material_for_key(scene, key);

		!matches!(
			fbx_material_binding(spec, source_id, name),
			Ok(Some(MaterialBinding::Asset(_)))
		) && names_generated_material(url, &generated_fbx_material_base_id(source_id, key, material))
	})
}

/// Reads what the sidecar settings `spec` of the model `url` bind its material `name` to.
fn fbx_material_binding(spec: Option<&Value>, url: ResourceId<'_>, name: &str) -> Result<Option<MaterialBinding>, String> {
	let scope = TemplateScope {
		model_id: url.as_ref(),
		material: name,
	};
	material_binding(spec, Some(name), &scope)
}

/// Returns the material `key` names in `scene`, if any, and the name a BEAD sidecar overrides it by. Unnamed materials
/// use the `default` override key.
fn fbx_material_for_key(scene: &ufbx::Scene, key: MaterialKey) -> (Option<&ufbx::Material>, &str) {
	let material = match key {
		MaterialKey::Default => None,
		MaterialKey::Material(index) => scene.materials.as_ref().get(index as usize).map(AsRef::as_ref),
	};
	let name = material
		.map(|material| material.element.name.as_ref())
		.filter(|name| !name.is_empty())
		.unwrap_or("default");

	(material, name)
}

/// Collects material identities in deterministic first-use order across FBX mesh instances.
pub(crate) fn used_material_keys<'a>(scene: &ufbx::Scene, allocator: &'a dyn Allocator) -> Vec<MaterialKey, &'a dyn Allocator> {
	let mut keys = Vec::with_capacity_in(scene.materials.len().saturating_add(1), allocator);

	let mut seen = HashSet::with_capacity(scene.materials.len().saturating_add(1));

	for (node, mesh) in fbx_mesh_instances(scene) {
		let material_node = authored_material_node(node);

		// Keep first-use ordering stable so generated fallback materials stay deterministic across reimports.
		let mut record_slot = |slot| {
			let key = material_key_for_slot(material_node, mesh, slot);

			if seen.insert(key) {
				keys.push(key);
			}
		};

		if mesh.material_parts.is_empty() {
			if (0..mesh.faces.len()).any(|index| is_visible_polygon_face(mesh, index)) {
				record_slot(0);
			}
		} else {
			for part in &mesh.material_parts {
				if part
					.face_indices
					.iter()
					.any(|&index| is_visible_polygon_face(mesh, index as usize))
				{
					record_slot(part.index as usize);
				}
			}
		}
	}

	keys
}

/// Filters point, line, and authored hole faces out of renderable polygon processing.
pub(crate) fn is_visible_polygon_face(mesh: &ufbx::Mesh, face_index: usize) -> bool {
	mesh.faces
		.get(face_index)
		.is_some_and(|face| face.num_indices >= 3 && !mesh.face_hole.get(face_index).copied().unwrap_or(false))
}

/// Resolves a material slot against the preselected authored instance before using the mesh-wide fallback binding.
pub(crate) fn material_key_for_slot(material_node: &ufbx::Node, mesh: &ufbx::Mesh, slot: usize) -> MaterialKey {
	material_node
		.materials
		.as_ref()
		.get(slot)
		.or_else(|| mesh.materials.as_ref().get(slot))
		.map(|material| MaterialKey::Material(material.element.typed_id))
		.unwrap_or(MaterialKey::Default)
}

/// Finds the authored instance node behind ufbx helper nodes so per-instance material bindings remain distinct.
pub(crate) fn authored_material_node(mut node: &ufbx::Node) -> &ufbx::Node {
	while node.is_geometry_transform_helper {
		let Some(parent) = node.parent.as_ref() else {
			break;
		};

		node = parent.as_ref();
	}

	node
}

/// Decodes a texture embedded in the FBX or resolves its file-local image through the current asset backend.
pub(crate) async fn load_fbx_texture_image(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	texture: &ufbx::Texture,
) -> Result<(Box<[u8]>, u32, u32), LoadErrors> {
	if !texture.content.is_empty() {
		return decode_fbx_texture_image(&texture.content).inspect_err(|_| {
			context.error(format_args!(
				"Embedded FBX texture '{}' could not be decoded. The most likely cause is unsupported or malformed image data.",
				texture.element.name
			));
		});
	}

	let Some(path) = fbx_texture_source_path(texture) else {
		context.error(format_args!(
			"FBX texture '{}' has no embedded image or file path. The most likely cause is an incomplete texture reference.",
			texture.element.name
		));

		return Err(LoadErrors::FailedToProcess);
	};

	let url = resolve_fbx_texture_path(mesh_url, path)?;

	let (bytes, ..) = context.resolve(ResourceId::new(&url)).await.inspect_err(|_| {
		context.error(format_args!(
			"FBX texture '{url}' could not be loaded. The most likely cause is a missing file-local image reference."
		));
	})?;

	decode_fbx_texture_image(&bytes).inspect_err(|_| {
		context.error(format_args!(
			"FBX texture '{url}' could not be decoded. The most likely cause is unsupported or malformed image data."
		));
	})
}

/// Decodes one FBX texture into the RGBA pixels expected by the image processor.
pub(crate) fn decode_fbx_texture_image(bytes: &[u8]) -> Result<(Box<[u8]>, u32, u32), LoadErrors> {
	let image = image::load_from_memory(bytes).map_err(|_| LoadErrors::FailedToProcess)?;

	let rgba = image.into_rgba8();

	let (width, height) = rgba.dimensions();

	Ok((rgba.into_raw().into_boxed_slice(), width, height))
}

/// Selects the file-local path that remains usable when an FBX omits embedded texture bytes.
pub(crate) fn fbx_texture_source_path(texture: &ufbx::Texture) -> Option<&str> {
	[&texture.relative_filename, &texture.filename, &texture.absolute_filename]
		.into_iter()
		.find(|path| !path.is_empty())
		.map(|path| path.as_ref())
}

/// Resolves a FBX file-local texture path relative to its source asset while accepting Windows-authored separators.
pub(crate) fn resolve_fbx_texture_path(mesh_url: ResourceId<'_>, texture_path: &str) -> Result<String, LoadErrors> {
	if texture_path.is_empty() {
		return Err(LoadErrors::FailedToProcess);
	}

	let texture_path = texture_path.replace('\\', "/");

	let is_windows_absolute =
		texture_path.len() > 2 && texture_path.as_bytes()[1] == b':' && texture_path.as_bytes()[2] == b'/';

	if is_windows_absolute {
		return Ok(texture_path);
	}

	Ok(mesh_url.resolve_relative(&texture_path))
}

/// Builds a deterministic resource ID for one texture owned by an FBX source asset.
pub(crate) fn generated_fbx_image_id(mesh_url: ResourceId<'_>, texture: &ufbx::Texture) -> String {
	format!(
		"{}#{FBX_IMAGE_FRAGMENT_PREFIX}{}",
		mesh_url.as_ref(),
		texture.element.typed_id
	)
}

/// Prefixes the fragment of every generated FBX texture resource.
pub(crate) const FBX_IMAGE_FRAGMENT_PREFIX: &str = "images/";

/// Returns the texture index addressed by a generated FBX image fragment.
pub(crate) fn fbx_image_fragment_texture_index(fragment: &str) -> Option<usize> {
	fragment.strip_prefix(FBX_IMAGE_FRAGMENT_PREFIX)?.parse().ok()
}

/// Converts ufbx's normalized PBR values and texture maps into a BEMX metallic-roughness graph.
///
/// Every texture multiplies the value of the input it maps, the way glTF factors scale their textures, so FBX and glTF
/// materials build the same graph shapes. The input that samples a texture also decides how the texture bakes; see
/// [`fbx_image_semantics`].
pub(crate) fn fbx_material(material: Option<&ufbx::Material>) -> Result<ImportedMaterial, bemx::GraphError> {
	let mut graph = ImportedGraphBuilder::new();

	let Some(material) = material else {
		let base_color = graph.builder.constant(bemx::Value::Vector4([1.0; 4]));
		let (rgb, opacity) = graph.color_and_opacity(base_color)?;
		let metalness = graph.builder.constant(bemx::Value::Float(0.0));
		let roughness = graph.builder.constant(bemx::Value::Float(1.0));
		let emission = graph.builder.constant(bemx::Value::Vector3([0.0; 3]));

		return graph.finish(
			None,
			bemx::Surface {
				base_color: Some(rgb),
				opacity: Some(opacity),
				metalness: Some(metalness),
				roughness: Some(roughness),
				emission: Some(emission),
				..bemx::Surface::default()
			},
			false,
			AlphaMode::Opaque,
		);
	};

	let base_factor = material_map_scalar(&material.pbr.base_factor, 1.0).clamp(0.0, 1.0);

	let mut base_color = material_map_vec(
		&material.pbr.base_color,
		material_map_vec(&material.fbx.diffuse_color, [1.0; 4]),
	);

	for component in &mut base_color[..3] {
		*component = finite_material_product(component.clamp(0.0, 1.0), base_factor, 1.0);
	}

	base_color[3] = finite_material_product(base_color[3].clamp(0.0, 1.0), material_opacity(material), 1.0);

	let emission_factor = material_map_scalar(&material.pbr.emission_factor, 1.0).max(0.0);

	let emission = multiply_vec3(
		material_map_vec(
			&material.pbr.emission_color,
			material_map_vec(&material.fbx.emission_color, [0.0; 3]),
		),
		[emission_factor; 3],
	);

	// Prefer each normalized PBR map, then its legacy FBX fallback.
	let base_color_texture =
		material_map_texture(&material.pbr.base_color).or_else(|| material_map_texture(&material.fbx.diffuse_color));
	let emission_texture =
		material_map_texture(&material.pbr.emission_color).or_else(|| material_map_texture(&material.fbx.emission_color));
	let normal_texture =
		material_map_texture(&material.pbr.normal_map).or_else(|| material_map_texture(&material.fbx.normal_map));
	let opacity_texture = material_map_texture(&material.pbr.opacity);
	let occlusion_texture = material_map_texture(&material.pbr.ambient_occlusion);

	// Lambert and Phong materials map shininess, a glossiness-like value, into ufbx's roughness, so only PBR shading
	// models read roughness and metalness maps.
	let is_pbr = material.features.pbr.enabled;
	let metalness_texture = material_map_texture(&material.pbr.metalness).filter(|_| is_pbr);
	let roughness_texture = material_map_texture(&material.pbr.roughness).filter(|_| is_pbr);

	let base_color_node = graph.factor_times_image(bemx::Value::Vector4(base_color), base_color_texture.map(texture_image))?;
	let (rgb, mut opacity) = graph.color_and_opacity(base_color_node)?;

	if let Some(texture) = opacity_texture {
		let sample = graph.image(texture_image(texture), bemx::Type::Vector4)?;
		let coverage = graph.builder.add(bemx::Node::Extract { value: sample, index: 0 })?;

		opacity = graph.builder.add(bemx::Node::Binary {
			operator: bemx::BinaryOperator::Multiply,
			left: opacity,
			right: coverage,
		})?;
	}

	let metalness = red_channel_times_factor(
		&mut graph,
		material_map_scalar(&material.pbr.metalness, 0.0).clamp(0.0, 1.0),
		metalness_texture,
	)?;
	let roughness = red_channel_times_factor(
		&mut graph,
		material_map_scalar(&material.pbr.roughness, 1.0).clamp(0.0, 1.0),
		roughness_texture,
	)?;
	let normal = normal_texture
		.map(|texture| graph.normal_map(texture_image(texture), 1.0))
		.transpose()?;
	let occlusion = occlusion_texture
		.map(|texture| {
			let sample = graph.image(texture_image(texture), bemx::Type::Vector4)?;

			graph.builder.add(bemx::Node::Extract { value: sample, index: 0 })
		})
		.transpose()?;
	let emission = graph.factor_times_image(bemx::Value::Vector3(emission), emission_texture.map(texture_image))?;

	// An opacity map is a cutout: FBX has no alpha mode, and sorting blended geometry per texel is not possible.
	let alpha_mode = if opacity_texture.is_some() {
		AlphaMode::Mask(0.5)
	} else if base_color[3] < 0.999 {
		AlphaMode::Blend
	} else {
		AlphaMode::Opaque
	};

	graph.finish(
		non_empty_name(&material.element.name),
		bemx::Surface {
			base_color: Some(rgb),
			opacity: Some(opacity),
			metalness: Some(metalness),
			roughness: Some(roughness),
			normal,
			emission: Some(emission),
			occlusion,
			transmission: None,
			translucency: None,
		},
		material.features.double_sided.enabled,
		alpha_mode,
	)
}

/// Adds a scalar factor, multiplied by the red channel of `texture` when the material has one, which is where FBX
/// grayscale maps store their value.
fn red_channel_times_factor(
	graph: &mut ImportedGraphBuilder,
	factor: f32,
	texture: Option<&ufbx::Texture>,
) -> Result<bemx::NodeId, bemx::GraphError> {
	let sample = texture
		.map(|texture| graph.image(texture_image(texture), bemx::Type::Vector4))
		.transpose()?;

	graph.factor_times_channel(factor, sample, 0)
}

/// Returns the container image an FBX texture bakes as: its position in the scene's texture list.
fn texture_image(texture: &ufbx::Texture) -> u32 {
	texture.element.typed_id
}

/// Returns how the scene's materials sample each texture, by texture index, merged across every material that
/// references it.
///
/// Texture fragments bake with these semantics, so a texture decodes the same way whether a material or a direct
/// request bakes it. A texture no material samples is `None`.
pub(crate) fn fbx_image_semantics(scene: &ufbx::Scene) -> Vec<Option<Semantic>> {
	let mut semantics = vec![None; scene.textures.len()];

	for material in &scene.materials {
		// A material that fails to convert is reported when it is generated, so here it simply samples nothing.
		if let Ok(material) = fbx_material(Some(material)) {
			material.merge_image_semantics(&mut semantics);
		}
	}

	semantics
}

/// Returns a material texture only when ufbx reports its source map as enabled.
pub(crate) fn material_map_texture(map: &ufbx::MaterialMap) -> Option<&ufbx::Texture> {
	map.texture_enabled.then(|| map.texture.as_ref().map(AsRef::as_ref)).flatten()
}

/// Resolves explicit opacity before deriving alpha from FBX transparency fields.
pub(crate) fn material_opacity(material: &ufbx::Material) -> f32 {
	if material.pbr.opacity.has_value {
		return material_map_scalar(&material.pbr.opacity, 1.0).clamp(0.0, 1.0);
	}

	// Read the authored FBX `Opacity` property when ufbx does not normalize it into the PBR opacity map.
	if let Some(property) = material.element.props.find_prop("Opacity")
		&& matches!(property.type_, ufbx::PropType::Number | ufbx::PropType::Integer)
	{
		return finite_material_component(property.value_vec4.x, 1.0).clamp(0.0, 1.0);
	}

	let transparency = if material.pbr.transmission_factor.has_value {
		material_map_scalar(&material.pbr.transmission_factor, 0.0)
	} else {
		material_map_scalar(&material.fbx.transparency_factor, 0.0)
	};

	(1.0 - transparency).clamp(0.0, 1.0)
}

/// Reads the scalar x component used by ufbx material factor maps.
pub(crate) fn material_map_scalar(map: &ufbx::MaterialMap, default: f32) -> f32 {
	material_map_vec(map, [default])[0]
}

/// Reads the first `N` components of a ufbx material color with finite fallbacks per component.
pub(crate) fn material_map_vec<const N: usize>(map: &ufbx::MaterialMap, default: [f32; N]) -> [f32; N] {
	if !map.has_value {
		return default;
	}

	let value = [map.value_vec4.x, map.value_vec4.y, map.value_vec4.z, map.value_vec4.w];

	std::array::from_fn(|index| finite_material_component(value[index], default[index]))
}

/// Converts a material component without allowing f64 values that overflow the engine's f32 representation.
pub(crate) fn finite_material_component(value: f64, default: f32) -> f32 {
	finite_f32(value, "material component").unwrap_or(default)
}

/// Multiplies non-negative material colors while replacing overflow with a safe fallback.
pub(crate) fn multiply_vec3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|index| finite_material_product(left[index].max(0.0), right[index].max(0.0), 0.0))
}

/// Computes a material factor product at f64 precision before checking that it fits in f32.
pub(crate) fn finite_material_product(left: f32, right: f32, default: f32) -> f32 {
	finite_material_component(left as f64 * right as f64, default)
}

/// Builds a deterministic, collision-resistant resource prefix for a generated FBX material chain.
pub(crate) fn generated_fbx_material_base_id(
	mesh_url: ResourceId<'_>,
	key: MaterialKey,
	material: Option<&ufbx::Material>,
) -> String {
	let index = match key {
		MaterialKey::Default => "default".to_string(),
		MaterialKey::Material(index) => index.to_string(),
	};

	let name = material
		.and_then(|material| non_empty_name(&material.element.name))
		.map(|name| sanitize_material_name(&name))
		.unwrap_or_else(|| "material".to_string());

	format!("{}#materials/{index}_{name}", mesh_url.as_ref())
}

use super::*;
