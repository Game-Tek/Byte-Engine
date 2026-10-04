use super::*;

pub(crate) fn unique_gltf_materials<'a>(primitives: &[gltf::Primitive<'a>]) -> (Vec<gltf::Material<'a>>, Vec<usize>) {
	let mut unique_materials = Vec::new();

	let mut unique_material_indices = HashMap::new();

	let mut material_indices_per_primitive = Vec::with_capacity(primitives.len());

	for primitive in primitives {
		let material = primitive.material();

		let key = material.index();

		let material_index = if let Some(index) = unique_material_indices.get(&key) {
			*index
		} else {
			let index = unique_materials.len();

			unique_materials.push(material);

			unique_material_indices.insert(key, index);

			index
		};

		material_indices_per_primitive.push(material_index);
	}

	(unique_materials, material_indices_per_primitive)
}

/// Resolves glTF materials into variants, in the order given.
///
/// Maps each material to a BEAD override or a generated BRDF graph, then resolves them all through
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
		.map(|material| match material_override(spec, material) {
			Some(override_id) => Ok(MaterialSource::Override(override_id)),
			None => {
				let base_id = generated_material_base_id(mesh_url, material);
				let brdf = generated_gltf_brdf(material, &image_semantics).map_err(|error| {
					log::error!(
						"Failed to generate the glTF material '{base_id}': {error:?}. The most likely cause is a texture read of a channel its packed image does not store."
					);
					LoadErrors::FailedToProcess
				})?;
				Ok(MaterialSource::Generated(GeneratedMaterial { base_id, brdf }))
			}
		})
		.collect::<Result<Vec<_>, LoadErrors>>()?;

	let image_ids = gltf
		.images()
		.map(|image| generated_gltf_image_id(mesh_url, image.index() as u32, image.name()))
		.collect::<Vec<_>>();

	resolve_container_materials(context, generator, mesh_url, &image_ids, sources).await
}

/// Builds the BRDF graph of a generated glTF material, with channel reads moved to where its packed images store them.
///
/// `image_semantics` comes from [`gltf_image_semantics`], the same table image fragments bake with.
pub(crate) fn generated_gltf_brdf(
	material: &gltf::Material<'_>,
	image_semantics: &[Option<Semantic>],
) -> Result<BrdfMaterialDescription, BrdfMaterialValidationError> {
	let mut brdf = brdf_material_from_gltf(material);
	brdf.pack_texture_channels(|image_index| {
		image_semantics
			.get(image_index as usize)
			.copied()
			.flatten()
			.and_then(channel_packing_for_semantic)
	})?;
	Ok(brdf)
}

/// Reads the BEAD override for a named glTF material. Unnamed materials are always generated.
pub(crate) fn material_override(spec: Option<&serde_json::Value>, material: &gltf::Material<'_>) -> Option<String> {
	bead_material_override(spec, material.name()?)
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

/// The `GltfTextureDependency` struct records a glTF image required by a generated material variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GltfTextureDependency {
	pub(crate) image_index: u32,
	pub(crate) semantic: Semantic,
}
