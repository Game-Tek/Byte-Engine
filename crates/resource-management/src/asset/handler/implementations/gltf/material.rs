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

pub(crate) async fn material_for_gltf_primitive(
	context: BakeContext<'_>,
	spec: Option<&serde_json::Value>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	material: gltf::Material<'_>,
	generator: Option<Arc<dyn ProgramGenerator>>,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	if let Some(override_asset) = material_override(spec, &material) {
		return context.bake_dependency::<VariantModel>(&override_asset).await;
	}

	generate_gltf_material_variant(context, mesh_url, gltf, buffers, material, generator, mip_backend).await
}

pub(crate) async fn generate_gltf_material_variant(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	gltf: &gltf::Gltf,
	buffers: &[gltf::buffer::Data],
	material: gltf::Material<'_>,
	generator: Option<Arc<dyn ProgramGenerator>>,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let generator = generator.ok_or(LoadErrors::FailedToProcess)?;

	let brdf = brdf_material_from_gltf(&material);

	let alpha_mode = AlphaMode::from(brdf.alpha_mode);

	let texture_dependencies = collect_gltf_texture_dependencies(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;

	let texture_variables =
		store_gltf_texture_dependencies(context, mesh_url, gltf, buffers, &texture_dependencies, mip_backend).await?;

	let program = generate_textured_brdf_program(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;

	let base_id = generated_material_base_id(mesh_url, &material);

	let shader_id = format!("{base_id}.shader");

	let material_id = format!("{base_id}.material");

	let variant_id = format!("{base_id}.variant");

	let shader_name = shader_id.clone();

	let material_json = generated_material_json(&texture_variables);

	let (shader, shader_bytes) =
		compile_shader_program(generator.as_ref(), &shader_name, program, "World", &material_json, "Compute")
			.await
			.map_err(|_| LoadErrors::FailedToProcess)?;

	let shader = store_model_owned::<Shader, _>(context, &shader_id, shader, shader_bytes).await?;

	let material = MaterialModel {
		double_sided: brdf.double_sided,
		alpha_mode: alpha_mode.clone(),
		coverage: material_coverage(&brdf, &texture_dependencies),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};

	let material = store_model::<MaterialModel>(context, &material_id, material, &[]).await?;

	let variant = VariantModel {
		material,
		variables: texture_variables,
		alpha_mode,
	};

	store_model::<VariantModel>(context, &variant_id, variant, &[]).await
}

/// Extracts the glTF base-color alpha expression into the compact masked-raster contract.
pub(crate) fn material_coverage(
	material: &BrdfMaterialDescription,
	dependencies: &[GltfTextureDependency],
) -> MaterialCoverage {
	let Ok(BrdfNode::MetallicRoughness(surface)) = material.node(material.surface) else {
		return MaterialCoverage {
			factor: 1.0,
			texture_slot: None,
		};
	};

	let mut factor = 1.0;

	let mut image_index = None;

	collect_base_color_coverage(material, surface.base_color, &mut factor, &mut image_index);

	let texture_slot = image_index.and_then(|image_index| {
		dependencies
			.iter()
			.position(|dependency| dependency.image_index == image_index)
			.map(|slot| slot as u32)
	});

	MaterialCoverage { factor, texture_slot }
}

pub(crate) fn collect_base_color_coverage(
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
	factor: &mut f32,
	image_index: &mut Option<u32>,
) {
	match material.node(node) {
		Ok(BrdfNode::Constant(BrdfValue::Vector4(value))) => *factor *= value[3],
		Ok(BrdfNode::Texture(texture)) => *image_index = Some(texture.image_index),
		Ok(BrdfNode::Multiply { left, right }) => {
			collect_base_color_coverage(material, *left, factor, image_index);

			collect_base_color_coverage(material, *right, factor, image_index);
		}
		_ => {}
	}
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
