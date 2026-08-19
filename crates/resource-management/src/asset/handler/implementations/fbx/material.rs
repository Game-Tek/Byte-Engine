#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]

pub(crate) enum MaterialKey {
	Default,
	Material(u32),
}

/// The `ResolvedFbxMaterials` struct keeps the resource references used while imported material parts are assembled.
pub(crate) struct ResolvedFbxMaterials {
	pub(crate) materials: HashMap<MaterialKey, ReferenceModel<VariantModel>>,
}

impl ResolvedFbxMaterials {
	pub(crate) fn get(&self, key: MaterialKey) -> Result<&ReferenceModel<VariantModel>, FbxImportError> {
		self.materials.get(&key).ok_or(FbxImportError::MissingMaterial)
	}
}

/// Resolves each used FBX material exactly once, honoring `.fbx.bead` overrides before generating a solid fallback.
pub(crate) async fn resolve_fbx_materials(
	context: BakeContext<'_>,
	spec: Option<&Value>,
	url: ResourceId<'_>,
	scene: &ufbx::Scene,
	generator: Option<Arc<dyn ProgramGenerator>>,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ResolvedFbxMaterials, LoadErrors> {
	let allocator = context.allocator();

	let keys = used_material_keys(scene, allocator);

	let mut materials = HashMap::with_capacity(keys.len());

	for key in keys {
		let material = match key {
			MaterialKey::Default => None,
			MaterialKey::Material(index) => scene.materials.as_ref().get(index as usize).map(AsRef::as_ref),
		};

		let resolved = if let Some(override_id) = fbx_material_override(spec, material) {
			context.bake_dependency::<VariantModel>(&override_id).await?
		} else {
			generate_fbx_material(context, url, key, material, generator.clone(), mip_backend).await?
		};

		materials.insert(key, resolved);
	}

	Ok(ResolvedFbxMaterials { materials })
}

/// Collects material identities in deterministic first-use order across FBX mesh instances.
pub(crate) fn used_material_keys<'a>(scene: &ufbx::Scene, allocator: &'a dyn Allocator) -> Vec<MaterialKey, &'a dyn Allocator> {
	let mut keys = Vec::with_capacity_in(scene.materials.len().saturating_add(1), allocator);

	let mut seen = HashSet::with_capacity(scene.materials.len().saturating_add(1));

	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};

		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}

		let material_node = authored_material_node(node);

		// Keep first-use ordering stable so generated fallback materials stay deterministic across reimports.
		let mut record_slot = |slot| {
			let key = material_key_for_slot(material_node, mesh, slot);

			if seen.insert(key) {
				keys.push(key);
			}
		};

		if mesh.material_parts.is_empty() {
			if mesh
				.faces
				.iter()
				.enumerate()
				.any(|(index, _)| is_visible_polygon_face(mesh, index))
			{
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

/// Reads an optional `.fbx.bead` material override by authored material name or the `default` key.
pub(crate) fn fbx_material_override(spec: Option<&Value>, material: Option<&ufbx::Material>) -> Option<String> {
	let key = material
		.map(|material| material.element.name.as_ref())
		.filter(|name| !name.is_empty())
		.unwrap_or("default");

	let material = &spec?["asset"][&key];

	material["asset"].as_str().map(ToString::to_string)
}

/// Generates an FBX material and stores its shader, material, texture, and variant resource chain.
pub(crate) async fn generate_fbx_material(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	key: MaterialKey,
	material: Option<&ufbx::Material>,
	generator: Option<Arc<dyn ProgramGenerator>>,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let generator = generator.ok_or_else(|| {
		context.error(
			"FBX material generation is unavailable. The most likely cause is that the FBX asset handler has no shader generator."
		);

		LoadErrors::FailedToProcess
	})?;

	let brdf = fbx_brdf_material(material);

	let alpha_mode = AlphaMode::from(brdf.alpha_mode);

	let texture_variables = store_fbx_texture_variables(context, mesh_url, material, mip_backend).await?;

	let program = generate_textured_brdf_program(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;

	let base_id = generated_fbx_material_base_id(mesh_url, key, material);

	let shader_id = format!("{base_id}.shader");

	let material_id = format!("{base_id}.material");

	let variant_id = format!("{base_id}.variant");

	let shader_name = shader_id.clone();

	let material_json = generated_fbx_material_json(&texture_variables);

	let (shader, shader_bytes) =
		compile_shader_program(generator.as_ref(), &shader_name, program, "World", &material_json, "Compute")
			.await
		.map_err(|_| {
			context.error(format_args!(
				"Failed to compile generated FBX material shader '{shader_id}'. The most likely cause is an invalid generated shader or unavailable platform compiler."
			));
			LoadErrors::FailedToProcess
		})?;

	let shader = store_model::<Shader>(context, &shader_id, shader, &shader_bytes)?;

	let material = MaterialModel {
		double_sided: brdf.double_sided,
		alpha_mode: alpha_mode.clone(),
		coverage: fbx_material_coverage(&brdf),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};

	let material = store_model::<MaterialModel>(context, &material_id, material, &[])?;

	let variant = VariantModel {
		material,
		variables: texture_variables,
		alpha_mode,
	};

	store_model::<VariantModel>(context, &variant_id, variant, &[])
}

pub(crate) fn fbx_material_coverage(material: &crate::pbr::BrdfMaterialDescription) -> MaterialCoverage {
	let BrdfNode::MetallicRoughness(surface) = material.node(material.surface).expect("validated FBX material surface") else {
		return MaterialCoverage {
			factor: 1.0,
			texture_slot: None,
		};
	};

	let factor = base_color_alpha_factor(material, surface.base_color);

	let texture_slot = material
		.nodes
		.iter()
		.any(|node| matches!(node, BrdfNode::Texture(_)))
		.then_some(0);

	MaterialCoverage { factor, texture_slot }
}

pub(crate) fn base_color_alpha_factor(material: &crate::pbr::BrdfMaterialDescription, node: crate::pbr::BrdfNodeId) -> f32 {
	match material.node(node).expect("validated base-color node") {
		BrdfNode::Constant(BrdfValue::Vector4(value)) => value[3],
		BrdfNode::Multiply { left, right } => {
			base_color_alpha_factor(material, *left) * base_color_alpha_factor(material, *right)
		}
		BrdfNode::Texture(_) => 1.0,
		_ => 1.0,
	}
}

/// Stores the diffuse texture selected by the FBX BRDF graph as a generated material variable.
pub(crate) async fn store_fbx_texture_variables(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	material: Option<&ufbx::Material>,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<Vec<VariantVariableModel>, LoadErrors> {
	let Some(texture) = material.and_then(fbx_base_color_texture) else {
		return Ok(Vec::new());
	};

	let image_id = generated_fbx_image_id(mesh_url, texture);

	let image = load_and_store_fbx_texture(context, mesh_url, &image_id, texture, mip_backend).await?;

	Ok(vec![VariantVariableModel {
		name: material_texture_variable_name(texture.element.typed_id),
		r#type: "Texture2D".to_string(),
		value: ValueModel::Image(image),
	}])
}

/// Loads an embedded or file-local FBX texture, processes its RGBA pixels, and stores its image resource.
pub(crate) async fn load_and_store_fbx_texture(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	id: &str,
	texture: &ufbx::Texture,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ReferenceModel<Image>, LoadErrors> {
	let (pixels, width, height) = load_fbx_texture_image(context, mesh_url, texture).await?;

	let description = ImageDescription {
		format: Formats::RGBA8,
		extent: Extent::rectangle(width, height),
		semantic: Semantic::Albedo,
		gamma: gamma_from_semantic(Semantic::Albedo),
		generate_mipmaps: mip_backend.is_some(),
	};

	let (resource, data) =
		process_image_with_mip_backend_in(ResourceId::new(id), description, pixels, context.allocator(), mip_backend)?;

	context.store_resource(resource, &data).map(Into::into)
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

	let rgba = image.to_rgba8();

	let (width, height) = rgba.dimensions();

	Ok((rgba.into_raw().into_boxed_slice(), width, height))
}

/// Selects the file-local path that remains usable when an FBX omits embedded texture bytes.
pub(crate) fn fbx_texture_source_path(texture: &ufbx::Texture) -> Option<&str> {
	if !texture.relative_filename.is_empty() {
		Some(texture.relative_filename.as_ref())
	} else if !texture.filename.is_empty() {
		Some(texture.filename.as_ref())
	} else if !texture.absolute_filename.is_empty() {
		Some(texture.absolute_filename.as_ref())
	} else {
		None
	}
}

/// Resolves a FBX file-local texture path relative to its source asset while accepting Windows-authored separators.
pub(crate) fn resolve_fbx_texture_path(mesh_url: ResourceId<'_>, texture_path: &str) -> Result<String, LoadErrors> {
	if texture_path.is_empty() {
		return Err(LoadErrors::FailedToProcess);
	}

	let texture_path = texture_path.replace('\\', "/");

	let is_windows_absolute =
		texture_path.len() > 2 && texture_path.as_bytes()[1] == b':' && texture_path.as_bytes()[2] == b'/';

	if texture_path.contains("://") || texture_path.starts_with('/') || is_windows_absolute {
		return Ok(texture_path);
	}

	let base = mesh_url.get_base();

	let parent = Path::new(base.as_ref()).parent();

	if let Some(parent) = parent {
		Ok(parent.join(texture_path).to_string_lossy().replace('\\', "/"))
	} else {
		Ok(texture_path)
	}
}

/// Produces the material declarations used while compiling a generated FBX material shader.
pub(crate) fn generated_fbx_material_json(variables: &[VariantVariableModel]) -> crate::asset::JsonObject {
	let variables = variables
		.iter()
		.map(|variable| json!({ "name": variable.name, "data_type": variable.r#type }))
		.collect::<Vec<_>>();

	json!({ "variables": variables })
		.as_object()
		.expect("generated FBX material JSON should be an object")
		.clone()
}

/// Builds a deterministic resource ID for one texture owned by an FBX source asset.
pub(crate) fn generated_fbx_image_id(mesh_url: ResourceId<'_>, texture: &ufbx::Texture) -> String {
	format!("{}#images/{}", mesh_url.as_ref(), texture.element.typed_id)
}

/// Converts ufbx's normalized PBR values into the engine's metallic-roughness graph.
pub(crate) fn fbx_brdf_material(material: Option<&ufbx::Material>) -> crate::pbr::BrdfMaterialDescription {
	let mut builder = BrdfMaterialBuilder::new();

	let (name, base_color, base_color_texture, metallic, roughness, emission, double_sided) = if let Some(material) = material {
		let base_factor = material_map_scalar(&material.pbr.base_factor, 1.0).clamp(0.0, 1.0);

		let mut base_color = material_map_vec4(
			&material.pbr.base_color,
			material_map_vec4(&material.fbx.diffuse_color, [1.0; 4]),
		);

		for component in &mut base_color[..3] {
			*component = finite_material_product(component.clamp(0.0, 1.0), base_factor, 1.0);
		}

		base_color[3] = finite_material_product(base_color[3].clamp(0.0, 1.0), material_opacity(material), 1.0);

		let emission_factor = material_map_scalar(&material.pbr.emission_factor, 1.0).max(0.0);

		let emission = multiply_vec3(
			material_map_vec3(
				&material.pbr.emission_color,
				material_map_vec3(&material.fbx.emission_color, [0.0; 3]),
			),
			[emission_factor; 3],
		);

		(
			non_empty_name(&material.element.name),
			base_color,
			fbx_base_color_texture(material),
			material_map_scalar(&material.pbr.metalness, 0.0).clamp(0.0, 1.0),
			material_map_scalar(&material.pbr.roughness, 1.0).clamp(0.0, 1.0),
			emission,
			material.features.double_sided.enabled,
		)
	} else {
		(None, [1.0; 4], None, 0.0, 1.0, [0.0; 3], false)
	};

	let base_color_node = builder.constant(BrdfValue::Vector4(base_color));

	let base_color_node = if let Some(texture) = base_color_texture {
		let texture = builder.texture(BrdfTexture {
			image_index: texture.element.typed_id,
			texcoord_channel: 0,
		});

		builder.multiply(base_color_node, texture)
	} else {
		base_color_node
	};

	let metallic_node = builder.constant(BrdfValue::Scalar(metallic));

	let roughness_node = builder.constant(BrdfValue::Scalar(roughness));

	let emission_color = builder.constant(BrdfValue::Vector3(emission));

	let emission_node = builder.add(BrdfNode::Emission { color: emission_color });

	let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
		base_color: base_color_node,
		metallic: metallic_node,
		roughness: roughness_node,
		normal: None,
		occlusion: None,
		emission: Some(emission_node),
	}));

	let alpha_mode = if base_color[3] < 0.999 {
		BrdfAlphaMode::Blend
	} else {
		BrdfAlphaMode::Opaque
	};

	builder.finish(name, surface, double_sided, alpha_mode)
}

/// Selects the base-color texture from normalized PBR maps or their legacy FBX diffuse fallback.
pub(crate) fn fbx_base_color_texture(material: &ufbx::Material) -> Option<&ufbx::Texture> {
	material_map_texture(&material.pbr.base_color).or_else(|| material_map_texture(&material.fbx.diffuse_color))
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

	if let Some(opacity) = explicit_fbx_opacity(material) {
		return opacity;
	}

	let transparency = if material.pbr.transmission_factor.has_value {
		material_map_scalar(&material.pbr.transmission_factor, 0.0)
	} else {
		material_map_scalar(&material.fbx.transparency_factor, 0.0)
	};

	(1.0 - transparency).clamp(0.0, 1.0)
}

/// Reads the authored FBX `Opacity` property when ufbx does not normalize it into the PBR opacity map.
pub(crate) fn explicit_fbx_opacity(material: &ufbx::Material) -> Option<f32> {
	let property = material.element.props.find_prop("Opacity")?;

	match property.type_ {
		ufbx::PropType::Number | ufbx::PropType::Integer => {
			Some(finite_material_component(property.value_vec4.x, 1.0).clamp(0.0, 1.0))
		}
		_ => None,
	}
}

/// Reads the scalar x component used by ufbx material factor maps.
pub(crate) fn material_map_scalar(map: &ufbx::MaterialMap, default: f32) -> f32 {
	if map.has_value {
		finite_material_component(map.value_vec4.x, default)
	} else {
		default
	}
}

/// Reads a three-component ufbx material color with finite fallbacks per component.
pub(crate) fn material_map_vec3(map: &ufbx::MaterialMap, default: [f32; 3]) -> [f32; 3] {
	if map.has_value {
		[
			finite_material_component(map.value_vec4.x, default[0]),
			finite_material_component(map.value_vec4.y, default[1]),
			finite_material_component(map.value_vec4.z, default[2]),
		]
	} else {
		default
	}
}

/// Reads a four-component ufbx material color with finite fallbacks per component.
pub(crate) fn material_map_vec4(map: &ufbx::MaterialMap, default: [f32; 4]) -> [f32; 4] {
	if map.has_value {
		[
			finite_material_component(map.value_vec4.x, default[0]),
			finite_material_component(map.value_vec4.y, default[1]),
			finite_material_component(map.value_vec4.z, default[2]),
			finite_material_component(map.value_vec4.w, default[3]),
		]
	} else {
		default
	}
}

/// Converts a material component without allowing f64 values that overflow the engine's f32 representation.
pub(crate) fn finite_material_component(value: f64, default: f32) -> f32 {
	if value.is_finite() && value >= f32::MIN as f64 && value <= f32::MAX as f64 {
		value as f32
	} else {
		default
	}
}

/// Multiplies non-negative material colors while replacing overflow with a safe fallback.
pub(crate) fn multiply_vec3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		finite_material_product(left[0].max(0.0), right[0].max(0.0), 0.0),
		finite_material_product(left[1].max(0.0), right[1].max(0.0), 0.0),
		finite_material_product(left[2].max(0.0), right[2].max(0.0), 0.0),
	]
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
