use super::*;

/// Lists each distinct material once, in first-use order, and returns every input material's position in that list.
pub(crate) fn unique_gltf_materials<'a>(
	materials: impl Iterator<Item = gltf::Material<'a>>,
) -> (Vec<gltf::Material<'a>>, Vec<usize>) {
	let mut unique_materials = Vec::new();
	let mut unique_material_indices = HashMap::new();
	let slots = materials
		.map(|material| {
			*unique_material_indices.entry(material.index()).or_insert_with(|| {
				unique_materials.push(material);
				unique_materials.len() - 1
			})
		})
		.collect();

	(unique_materials, slots)
}

/// Resolves glTF materials into variants, in the order given.
///
/// Maps each material to what its sidecar binds it to, an authored or inline variant, or else a generated BEMX graph,
/// then resolves them all through [`resolve_container_materials`].
pub(crate) async fn resolve_gltf_materials(
	context: BakeContext<'_>,
	spec: Option<&serde_json::Value>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	materials: &[gltf::Material<'_>],
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	let image_semantics = gltf_image_semantics(gltf);
	let sources = materials
		.iter()
		.map(|material| {
			let base_id = generated_material_base_id(mesh_url, material);
			let binding = gltf_material_binding(spec, mesh_url, material, &base_id);

			MaterialSource::new(context, mesh_url, binding, base_id.clone(), |base_id| {
				let material = generated_gltf_material(material, &image_semantics).map_err(|error| {
					log::error!("Failed to generate the glTF material '{base_id}': {error}");
					LoadErrors::FailedToProcess
				})?;
				Ok(MaterialSource::Generated(GeneratedMaterial { base_id, material }))
			})
		})
		.collect::<Result<Vec<_>, LoadErrors>>()?;

	let image_ids = gltf
		.images()
		.map(|image| generated_gltf_image_id(mesh_url, image.index() as u32, image.name()))
		.collect::<Vec<_>>();

	resolve_container_materials(context, mesh_url, &image_ids, sources).await
}

/// Builds the BEMX graph of a generated glTF material, with channel reads moved to where its packed images store them.
///
/// `image_semantics` comes from [`gltf_image_semantics`], the same table image fragments bake with.
pub(crate) fn generated_gltf_material(
	material: &gltf::Material<'_>,
	image_semantics: &[Option<Semantic>],
) -> Result<ImportedMaterial, String> {
	let mut material = material_from_gltf(material).map_err(|error| error.to_string())?;

	material
		.pack_texture_channels(|image_index| {
			image_semantics
				.get(image_index as usize)
				.copied()
				.flatten()
				.and_then(channel_packing_for_semantic)
		})
		.map_err(|error| error.to_string())?;

	Ok(material)
}

/// Finds the material that `url`, a `#materials/<name>.variant` or `.material` ID, names among the materials the
/// container's primitives use: a generated material, or an inline variant from the sidecar, which shares its ID.
///
/// Materials a sidecar binds to an authored `.bema` variant are left out, because the container stores nothing for
/// them.
pub(crate) fn generated_material_for_id<'a>(
	gltf: &'a gltf::Gltf,
	spec: Option<&serde_json::Value>,
	url: ResourceId<'_>,
) -> Option<gltf::Material<'a>> {
	let base = url.get_base();
	let source_id = ResourceId::new(base.as_ref());
	let primitives = gltf.meshes().flat_map(|mesh| mesh.primitives());
	let (materials, _) = unique_gltf_materials(primitives.map(|primitive| primitive.material()));

	materials.into_iter().find(|material| {
		let base_id = generated_material_base_id(source_id, material);
		let overridden = matches!(
			gltf_material_binding(spec, source_id, material, &base_id),
			Ok(Some(MaterialBinding::Asset(_)))
		);

		!overridden && names_generated_material(url, &base_id)
	})
}

/// Reads what the sidecar settings `spec` of the model `mesh_url` bind `material` to. `base_id` is the material's
/// [`generated_material_base_id`].
///
/// Unnamed materials have no key of their own, so only `"*"` binds them. Their `{material}` is the name their generated
/// resource has, such as `material_3`.
fn gltf_material_binding(
	spec: Option<&serde_json::Value>,
	mesh_url: ResourceId<'_>,
	material: &gltf::Material<'_>,
	base_id: &str,
) -> Result<Option<MaterialBinding>, String> {
	let scope = TemplateScope {
		model_id: mesh_url.as_ref(),
		material: material
			.name()
			.unwrap_or_else(|| base_id.rsplit('/').next().unwrap_or_default()),
	};
	material_binding(spec, material.name(), &scope)
}

pub(crate) fn generated_material_base_id(mesh_url: ResourceId<'_>, material: &gltf::Material<'_>) -> String {
	let material_name = material
		.name()
		.map(sanitize_material_name)
		.unwrap_or_else(|| match material.index() {
			Some(index) => format!("material_{index}"),
			None => "material_default".to_string(),
		});

	format!("{}#materials/{material_name}", mesh_url.as_ref())
}
