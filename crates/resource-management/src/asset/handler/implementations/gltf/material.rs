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
/// Maps each material to a BEAD override or a generated BEMX graph, then resolves them all through
/// [`resolve_container_materials`], which bakes overrides as dependencies and generates the rest.
pub(crate) async fn resolve_gltf_materials(
	context: BakeContext<'_>,
	spec: Option<&serde_json::Value>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	materials: &[gltf::Material<'_>],
	generator: Option<&dyn ProgramGenerator>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	let image_semantics = gltf_image_semantics(gltf);
	let sources = materials
		.iter()
		// Unnamed materials have no BEAD override key, so they are always generated.
		.map(
			|material| match material.name().and_then(|name| bead_material_override(spec, name)) {
				Some(override_id) => Ok(MaterialSource::Override(override_id)),
				None => {
					let base_id = generated_material_base_id(mesh_url, material);
					let material = generated_gltf_material(material, &image_semantics).map_err(|error| {
						log::error!("Failed to generate the glTF material '{base_id}': {error}");
						LoadErrors::FailedToProcess
					})?;
					Ok(MaterialSource::Generated(GeneratedMaterial { base_id, material }))
				}
			},
		)
		.collect::<Result<Vec<_>, LoadErrors>>()?;

	let image_ids = gltf
		.images()
		.map(|image| generated_gltf_image_id(mesh_url, image.index() as u32, image.name()))
		.collect::<Vec<_>>();

	resolve_container_materials(context, generator, mesh_url, &image_ids, sources).await
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
