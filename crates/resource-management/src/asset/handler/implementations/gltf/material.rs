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

/// Bounds concurrent BEAD material override bakes.
const OVERRIDE_BAKE_CONCURRENCY: usize = 8;

/// Resolves glTF materials into variants, in the order given.
///
/// BEAD overrides bake as dependencies. Every other material is generated through [`store_generated_materials`].
pub(crate) async fn resolve_gltf_materials(
	context: BakeContext<'_>,
	spec: Option<&serde_json::Value>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	materials: &[gltf::Material<'_>],
	generator: Option<&dyn ProgramGenerator>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	let override_ids = materials
		.iter()
		.map(|material| material_override(spec, material))
		.collect::<Vec<_>>();

	let generated = materials
		.iter()
		.zip(&override_ids)
		.filter(|(_, override_id)| override_id.is_none())
		.map(|(material, _)| GeneratedMaterial {
			base_id: generated_material_base_id(mesh_url, material),
			brdf: brdf_material_from_gltf(material),
		})
		.collect::<Vec<_>>();

	let image_ids = gltf
		.images()
		.map(|image| generated_gltf_image_id(mesh_url, image.index() as u32, image.name()))
		.collect::<Vec<_>>();

	let overrides = override_ids.iter().flatten().cloned().collect::<Vec<_>>();

	let (overridden, generated) = std::future::join!(
		context.bake_dependencies::<VariantModel>(&overrides, OVERRIDE_BAKE_CONCURRENCY),
		store_generated_materials(context, generator, mesh_url, &image_ids, generated),
	)
	.await;

	let (mut overridden, mut generated) = (overridden?.into_iter(), generated?.into_iter());

	// Put overridden and generated variants back in material order.
	override_ids
		.iter()
		.map(|override_id| match override_id {
			Some(_) => overridden.next(),
			None => generated.next(),
		})
		.collect::<Option<Vec<_>>>()
		.ok_or(LoadErrors::FailedToProcess)
}

pub(crate) fn material_override(spec: Option<&serde_json::Value>, material: &gltf::Material<'_>) -> Option<String> {
	let material_name = material.name()?;

	let material = &spec?["asset"][material_name];

	material["asset"].as_str().map(ToString::to_string)
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
