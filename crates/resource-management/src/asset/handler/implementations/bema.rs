/// The `ProgramGenerator` trait provides renderer-specific shader adaptation before platform compilation.
pub trait ProgramGenerator: Send + Sync {
	/// Adapts a parsed material program to the bindings and entry-point contract used by its renderer.
	fn transform<'a>(&self, node: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a>;
}

/// The `ShaderCompiler` trait isolates BEMA resource orchestration from platform shader toolchains.
trait ShaderCompiler: Send + Sync {
	fn compile<'a>(
		&'a self,
		generator: &'a dyn ProgramGenerator,
		name: &'a str,
		shader_code: &'a str,
		format: &'a str,
		domain: &'a str,
		material: &'a JsonObject,
		shader_json: &'a Value,
		stage: &'a str,
	) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), ()>>;
}

/// The `PlatformShaderCompilerAdapter` struct routes production BEMA shaders through the active platform compiler.
struct PlatformShaderCompilerAdapter;

impl ShaderCompiler for PlatformShaderCompilerAdapter {
	fn compile<'a>(
		&'a self,
		generator: &'a dyn ProgramGenerator,
		name: &'a str,
		shader_code: &'a str,
		format: &'a str,
		domain: &'a str,
		material: &'a JsonObject,
		shader_json: &'a Value,
		stage: &'a str,
	) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), ()>> {
		Box::pin(compile_shader(
			generator,
			name,
			shader_code,
			format,
			domain,
			material,
			shader_json,
			stage,
		))
	}
}

pub struct BEMAAssetHandler {
	generator: Option<Box<dyn ProgramGenerator>>,
	compiler: Box<dyn ShaderCompiler>,
}

impl Default for BEMAAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl BEMAAssetHandler {
	pub fn new() -> BEMAAssetHandler {
		BEMAAssetHandler {
			generator: None,
			compiler: Box::new(PlatformShaderCompilerAdapter),
		}
	}

	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Box::new(generator));
	}

	/// Bakes a material definition and its independently compiled shader stages.
	async fn bake_material<'a>(&self, context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
		use utils::r#async::StreamExt as _;

		let asset_object = asset.as_object().ok_or(LoadErrors::FailedToProcess)?;
		let material_domain = asset["domain"].as_str().ok_or(LoadErrors::FailedToProcess)?;
		let generator = self.generator.as_deref().ok_or(LoadErrors::FailedToProcess)?;
		let asset_shaders = asset["shaders"].as_object().ok_or(LoadErrors::FailedToProcess)?;

		// Compile independent stages together while preserving declaration order in the material model.
		let shader_requests = asset_shaders.iter().map(|(shader_type, shader_json)| {
			compile_and_store_shader(
				context,
				self.compiler.as_ref(),
				generator,
				material_domain,
				asset_object,
				shader_json,
				shader_type,
			)
		});
		let shaders = utils::r#async::stream::iter(shader_requests)
			.buffered(4)
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect::<Result<Vec<_>, _>>()?;

		let asset_variables = asset["variables"].as_array().ok_or(LoadErrors::FailedToProcess)?;
		// Texture parameters can trigger independent dependency bakes; scalar values complete immediately.
		let value_requests = asset_variables.iter().map(|variable| async move {
			let data_type = variable["data_type"].as_str().ok_or(LoadErrors::FailedToProcess)?;
			let value = variable["value"].as_str().ok_or(LoadErrors::FailedToProcess)?;
			resolve_value(context, data_type, value).await
		});
		let values = utils::r#async::stream::iter(value_requests)
			.buffered(8)
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect::<Result<Vec<_>, _>>()?;
		let parameters = asset_variables
			.iter()
			.zip(values)
			.map(|(variable, value)| ParameterModel {
				name: variable["name"].as_str().unwrap().to_string(),
				r#type: variable["data_type"].as_str().unwrap().to_string(),
				value,
			})
			.collect();

		let resource = MaterialModel {
			double_sided: false,
			alpha_mode: AlphaMode::Opaque,
			coverage: MaterialCoverage {
				factor: 1.0,
				texture_slot: None,
			},
			model: RenderModel {
				name: "Visibility".to_string(),
				pass: "MaterialEvaluation".to_string(),
			},
			shaders,
			parameters,
		};

		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}

	/// Bakes one material variant by resolving its inherited parameters.
	async fn bake_variant<'a>(context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
		use utils::r#async::StreamExt as _;

		let parent_material_url = asset["parent"].as_str().ok_or(LoadErrors::FailedToProcess)?;
		let material = context.bake_dependency(parent_material_url).await?;
		let material_repr: MaterialModel = crate::from_slice(&material.resource).map_err(|_| LoadErrors::FailedToProcess)?;
		let authored_variables = asset["variables"].as_array().ok_or(LoadErrors::FailedToProcess)?;
		let value_requests = material_repr.parameters.iter().map(|parameter| async move {
			let value = authored_variables
				.iter()
				.find(|variable| variable["name"].as_str() == Some(parameter.name.as_str()))
				.and_then(|variable| variable["value"].as_str())
				.ok_or(LoadErrors::FailedToProcess)?;
			resolve_value(context, &parameter.r#type, value).await
		});
		let values = utils::r#async::stream::iter(value_requests)
			.buffered(8)
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect::<Result<Vec<_>, _>>()?;
		let variables = material_repr
			.parameters
			.iter()
			.zip(values)
			.map(|(parameter, value)| VariantVariableModel {
				value,
				name: parameter.name.clone(),
				r#type: parameter.r#type.clone(),
			})
			.collect();
		let alpha_mode = match asset.get("transparency") {
			Some(Value::Bool(true)) => AlphaMode::Blend,
			Some(Value::String(value)) if value == "Blend" => AlphaMode::Blend,
			_ => AlphaMode::Opaque,
		};
		let resource = VariantModel {
			material,
			variables,
			alpha_mode,
		};

		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}
}

impl AssetHandler for BEMAAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "bema"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(dt) = context.resource_type(url)
			&& dt != "bema"
		{
			return Err(LoadErrors::UnsupportedType);
		}

		let (data, at) = context.resolve(url).await?;

		if at != "bema" {
			return Err(LoadErrors::UnsupportedType);
		}

		let asset = asset::parse_json(std::str::from_utf8(&data).map_err(|_| LoadErrors::FailedToProcess)?)
			.map_err(|_| LoadErrors::FailedToProcess)?;

		if asset.get("parent").is_none() {
			self.bake_material(context, url, &asset).await
		} else {
			Self::bake_variant(context, url, &asset).await
		}
	}
}

/// Converts a shader source into a compiled shader and binary payload.
async fn compile_shader(
	generator: &dyn ProgramGenerator,
	name: &str,
	shader_code: &str,
	format: &str,
	_domain: &str,
	material: &JsonObject,
	_shader_json: &Value,
	stage: &str,
) -> Result<(Shader, Box<[u8]>), ()> {
	let root_node = if format == "glsl" {
		// besl::parser::NodeReference::glsl(&shader_code,/*Vec::new()*/)
		panic!()
	} else if format == "besl" {
		if let Ok(e) = besl::parse(shader_code) {
			e
		} else {
			log::error!(
				"Failed to parse BESL material shader. The most likely cause is invalid BESL syntax. See {}.",
				online_docs_url(BESL_DOCS_PATH)
			);

			return Err(());
		}
	} else {
		log::error!(
			"Unknown material shader format '{format}'. The most likely cause is an unsupported format in the .bema declaration. See {}.",
			online_docs_url(BEMA_DOCS_PATH)
		);

		return Err(());
	};

	compile_shader_program(generator, name, root_node, _domain, material, stage).await
}

/// Compiles a BESL shader program into a stored shader model and binary payload.
pub(crate) async fn compile_shader_program(
	generator: &dyn ProgramGenerator,
	name: &str,
	root_node: besl::parser::Node<'_>,
	_domain: &str,
	material: &JsonObject,
	stage: &str,
) -> Result<(Shader, Box<[u8]>), ()> {
	let root = generator.transform(root_node, material);

	let root_node = match besl::lex(root) {
		Ok(e) => e,
		Err(e) => {
			log::error!(
				"Failed to compile shader '{name}' for stage '{stage}': {e:#?}. See {}.",
				online_docs_url(BESL_DOCS_PATH)
			);

			return Err(());
		}
	};

	if root_node.get_main().is_none() {
		log::error!(
			"Failed to compile shader '{name}' for stage '{stage}'. The generated BESL program has no main function. See {}.",
			online_docs_url(BESL_DOCS_PATH)
		);

		return Err(());
	}

	let settings = match stage {
		"Vertex" => ShaderGenerationSettings::vertex(),
		"Fragment" => ShaderGenerationSettings::fragment(),
		"Compute" => ShaderGenerationSettings::compute(Extent::line(128)),
		_ => {
			panic!("Invalid shader stage")
		}
	}
	.name(name.to_string());

	let evaluation = ProgramEvaluation::from_program(&root_node).map_err(|error| {
		log::error!(
			"Failed to reflect shader '{name}' for stage '{stage}': {error}. See {}.",
			online_docs_url(BESL_DOCS_PATH)
		);
	})?;

	let shader_program = PlatformShaderCompiler::new()
		.generate(&settings, &root_node)
		.await
		.map_err(|error| {
			// The BESL program already linked and reflected, so the platform compiler rejected shader code the
			// engine generated. No BESL reference link belongs here: the defect is in the backend, not the source.
			log::error!(
				"Failed to compile shader '{name}' for stage '{stage}': {error}. The most likely cause is a defect in the platform shader backend. Report this with the shader name and the compiler output above."
			);
		})?;

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		_ => {
			panic!("Invalid shader stage")
		}
	};

	let interface = ShaderInterface {
		workgroup_size: shader_program.extent().map(|e| (e.width(), e.height(), e.depth())),
		bindings: evaluation
			.into_bindings()
			.into_iter()
			.map(|binding| {
				Binding::named(
					binding.name,
					binding.slot,
					binding.kind,
					binding.count,
					binding.buffer_stride,
					binding.read,
					binding.write,
				)
			})
			.collect(),
	};

	let language = PlatformShaderLanguage::current_platform();

	let entry_point = shader_program.entry_point();

	let (artifact, payload) =
		finalize_platform_shader_artifact(language, stage, name, entry_point, shader_program.into_binary()).map_err(
			|error| {
				log::error!(
					"Failed to finalize shader artifact '{name}' for stage '{stage:?}': {error}. See {}.",
					online_docs_url(BESL_DOCS_PATH)
				);
			},
		)?;

	let shader = Shader {
		id: name.to_string(),
		stage,
		interface,
		artifact,
		source_hash: 0,
	};

	Ok((shader, payload))
}

/// Bounds concurrent generated-material shader compiles so platform compiler processes do not oversubscribe the machine.
const GENERATED_SHADER_COMPILE_CONCURRENCY: usize = 4;

/// Bounds concurrent texture bakes so decoded images and compression buffers stay within a predictable memory footprint.
const TEXTURE_BAKE_CONCURRENCY: usize = 8;

/// The `GeneratedMaterial` struct describes one importer material whose shader, textures, and variant are generated.
pub(crate) struct GeneratedMaterial {
	/// The material and variant are stored as `<base_id>.material` and `<base_id>.variant`.
	pub(crate) base_id: String,
	/// The material graph. Each texture node's `image_index` indexes the container's image IDs.
	pub(crate) brdf: BrdfMaterialDescription,
}

/// Generates and stores the variants of a container's materials, in the order given.
///
/// glTF and FBX importers describe each material as a BRDF graph and call this once per container. Every image the
/// graphs sample bakes once, as its own dependency, while materials whose graphs match share one compiled shader.
/// `image_ids` holds the resource ID of each container image, indexed by the graphs' texture nodes.
///
/// Next, reference the returned variants from the container's mesh primitives.
pub(crate) async fn store_generated_materials(
	context: BakeContext<'_>,
	generator: Option<&dyn ProgramGenerator>,
	container_id: ResourceId<'_>,
	image_ids: &[String],
	materials: Vec<GeneratedMaterial>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	if materials.is_empty() {
		return Ok(Vec::new());
	}

	let generator = generator.ok_or_else(|| {
		context.error(
			"Material generation is unavailable. The most likely cause is that the asset handler has no shader generator.",
		);

		LoadErrors::FailedToProcess
	})?;

	// Slots follow each graph's first use of an image, so graphs that differ only in their images become identical.
	let mut materials = materials
		.into_iter()
		.map(|mut material| {
			let slots = assign_first_use_texture_slots(&mut material.brdf);
			(material, slots)
		})
		.collect::<Vec<_>>();

	let mut baked_images = Vec::new();

	for &image_index in materials.iter().flat_map(|(_, slots)| slots) {
		if !baked_images.contains(&image_index) {
			baked_images.push(image_index);
		}
	}

	let baked_image_ids = baked_images
		.iter()
		.map(|&image_index| {
			image_ids
				.get(image_index as usize)
				.cloned()
				.ok_or(LoadErrors::FailedToProcess)
		})
		.collect::<Result<Vec<_>, _>>()?;

	let brdfs = materials.iter().map(|(material, _)| &material.brdf).collect::<Vec<_>>();

	// Texture bakes and shader compiles do not depend on each other, so they run together.
	let (images, shaders) = std::future::join!(
		context.bake_dependencies::<Image>(&baked_image_ids, TEXTURE_BAKE_CONCURRENCY),
		store_generated_brdf_shaders(context, generator, container_id, &brdfs),
	)
	.await;

	let (images, shaders) = (images?, shaders?);

	let mut variants = Vec::with_capacity(materials.len());

	for ((material, slots), shader) in materials.into_iter().zip(shaders) {
		let variables = slots
			.iter()
			.enumerate()
			.map(|(slot, image_index)| {
				let image = baked_images
					.iter()
					.position(|baked| baked == image_index)
					.map(|position| images[position].clone())
					.expect("every sampled image was baked");

				VariantVariableModel {
					name: material_texture_variable_name(slot as u32),
					r#type: "Texture2D".to_string(),
					value: ValueModel::Image(image),
				}
			})
			.collect();

		variants.push(store_generated_variant(context, material, shader, variables).await?);
	}

	Ok(variants)
}

/// Renumbers a graph's texture nodes into slots in first-use order and returns the image index of each slot.
fn assign_first_use_texture_slots(brdf: &mut BrdfMaterialDescription) -> Vec<u32> {
	let mut slots = Vec::new();

	brdf.assign_texture_slots(|image_index| {
		let slot = slots.iter().position(|&used| used == image_index).unwrap_or_else(|| {
			slots.push(image_index);
			slots.len() - 1
		});

		slot as u32
	});

	slots
}

/// Stores one generated material and its variant.
async fn store_generated_variant(
	context: BakeContext<'_>,
	material: GeneratedMaterial,
	shader: ReferenceModel<Shader>,
	variables: Vec<VariantVariableModel>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let GeneratedMaterial { base_id, brdf } = material;

	let alpha_mode = AlphaMode::from(brdf.alpha_mode);

	let material = MaterialModel {
		double_sided: brdf.double_sided,
		alpha_mode: alpha_mode.clone(),
		coverage: generated_material_coverage(&brdf),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};

	let material = store_model::<MaterialModel>(context, &format!("{base_id}.material"), material, &[]).await?;

	let variant = VariantModel {
		material,
		variables,
		alpha_mode,
	};

	store_model::<VariantModel>(context, &format!("{base_id}.variant"), variant, &[]).await
}

/// Extracts the base-color alpha expression of a slot-numbered graph into the compact masked-raster contract.
fn generated_material_coverage(material: &BrdfMaterialDescription) -> MaterialCoverage {
	fn collect(material: &BrdfMaterialDescription, node: BrdfNodeId, factor: &mut f32, slot: &mut Option<u32>) {
		match material.node(node) {
			Ok(BrdfNode::Constant(BrdfValue::Vector4(value))) => *factor *= value[3],
			Ok(BrdfNode::Texture(texture)) => *slot = Some(texture.image_index),
			Ok(BrdfNode::Multiply { left, right }) => {
				collect(material, *left, factor, slot);
				collect(material, *right, factor, slot);
			}
			_ => {}
		}
	}

	let mut coverage = MaterialCoverage {
		factor: 1.0,
		texture_slot: None,
	};

	if let Ok(BrdfNode::MetallicRoughness(surface)) = material.node(material.surface) {
		collect(material, surface.base_color, &mut coverage.factor, &mut coverage.texture_slot);
	}

	coverage
}

/// Compiles each distinct generated BRDF graph once and stores it under an ID derived from the graph.
///
/// Texture nodes must already use slot indices. Returns one shader reference per entry in `materials`, in order.
async fn store_generated_brdf_shaders(
	context: BakeContext<'_>,
	generator: &dyn ProgramGenerator,
	container_id: ResourceId<'_>,
	materials: &[&BrdfMaterialDescription],
) -> Result<Vec<ReferenceModel<Shader>>, LoadErrors> {
	use utils::r#async::StreamExt as _;

	// Only the node graph reaches the program; names, sidedness, and alpha mode stay in the material resource.
	let mut unique_by_hash: HashMap<u64, usize> = HashMap::new();
	let mut unique = Vec::new();
	let mut unique_index_per_material = Vec::with_capacity(materials.len());

	for &material in materials {
		let key = serde_json::to_vec(&(&material.nodes, material.surface)).map_err(|_| LoadErrors::FailedToProcess)?;
		let hash = crate::resource::compression::payload_hash(&key);
		let index = *unique_by_hash.entry(hash).or_insert_with(|| {
			unique.push((hash, material));
			unique.len() - 1
		});
		unique_index_per_material.push(index);
	}

	let requests = unique.into_iter().map(|(hash, material)| async move {
		let shader_id = format!("{}#shaders/{hash:016x}", container_id.as_ref());

		let program = generate_textured_brdf_program(material).map_err(|_| LoadErrors::FailedToProcess)?;
		let material_json = generated_brdf_material_json(material);

		let (shader, shader_bytes) = compile_shader_program(generator, &shader_id, program, "World", &material_json, "Compute")
			.await
			.map_err(|_| {
				context.error(format_args!(
					"Failed to compile generated material shader '{shader_id}'. The most likely cause is an invalid generated shader or unavailable platform compiler."
				));
				LoadErrors::FailedToProcess
			})?;

		store_model_owned::<Shader, _>(context, &shader_id, shader, shader_bytes).await
	});

	// Ordered buffering keeps the stored shader order deterministic while platform compiler processes overlap.
	let unique_shaders = utils::r#async::stream::iter(requests)
		.buffered(GENERATED_SHADER_COMPILE_CONCURRENCY)
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect::<Result<Vec<_>, _>>()?;

	Ok(unique_index_per_material
		.into_iter()
		.map(|index| unique_shaders[index].clone())
		.collect())
}

/// Declares one `Texture2D` material variable per texture slot used by a generated BRDF graph.
fn generated_brdf_material_json(material: &BrdfMaterialDescription) -> JsonObject {
	let slot_count = material
		.nodes
		.iter()
		.filter_map(|node| match node {
			BrdfNode::Texture(texture) => Some(texture.image_index + 1),
			_ => None,
		})
		.max()
		.unwrap_or(0);

	let variables = (0..slot_count)
		.map(|slot| serde_json::json!({ "name": material_texture_variable_name(slot), "data_type": "Texture2D" }))
		.collect::<Vec<_>>();

	serde_json::json!({ "variables": variables })
		.as_object()
		.expect("generated material JSON should be an object")
		.clone()
}

/// Compiles a shader definition and stores the resulting resource and binary payload.
async fn compile_and_store_shader(
	context: BakeContext<'_>,
	compiler: &dyn ShaderCompiler,
	generator: &dyn ProgramGenerator,
	domain: &str,
	material: &JsonObject,
	shader_json: &Value,
	stage: &str,
) -> Result<ReferenceModel<Shader>, LoadErrors> {
	let path = shader_json.as_str().ok_or(LoadErrors::FailedToProcess)?;

	let path = ResourceId::new(path);

	let (arlp, format) = context.resolve(path).await?;

	let shader_code = std::str::from_utf8(&arlp)
		.map_err(|_| LoadErrors::FailedToProcess)?
		.to_string();

	let material = material.clone();

	let domain = domain.to_string();

	let stage = stage.to_string();

	let format = format.to_string();

	let name = path.get_base().as_ref().to_string();

	let shader_json = shader_json.clone();

	let (shader, result_shader_bytes) = compiler
		.compile(
			generator,
			&name,
			&shader_code,
			&format,
			&domain,
			&material,
			&shader_json,
			&stage,
		)
		.await
		.map_err(|_| LoadErrors::FailedToProcess)?;

	context
		.store_generated_owned(ProcessedAsset::new(path, shader), result_shader_bytes)
		.await
		.map(Into::into)
}

/// Resolves a material parameter value based on its type.
async fn resolve_value(context: BakeContext<'_>, data_type: &str, value: &str) -> Result<ValueModel, LoadErrors> {
	let to_color = |name: &str| match name {
		"Red" => [1f32, 0f32, 0f32, 1f32],
		"Green" => [0f32, 1f32, 0f32, 1f32],
		"Blue" => [0f32, 0f32, 1f32, 1f32],
		"Purple" => [1f32, 0f32, 1f32, 1f32],
		"White" => [1f32, 1f32, 1f32, 1f32],
		"Black" => [0f32, 0f32, 0f32, 1f32],
		_ => [1f32, 0f32, 1f32, 1f32],
	};

	match data_type {
		"vec4f" => {
			let value = to_color(value);

			Ok(ValueModel::Vector4([value[0], value[1], value[2], value[3]]))
		}
		"vec3f" => {
			let value = to_color(value);

			Ok(ValueModel::Vector3([value[0], value[1], value[2]]))
		}
		"float" => Ok(ValueModel::Scalar(0f32)),
		"Texture2D" => {
			let image = context.bake_dependency(value).await?;

			Ok(ValueModel::Image(image))
		}
		_ => Err(LoadErrors::FailedToProcess),
	}
}

const BEMA_DOCS_PATH: &str = "develop/resource-management/bema";

const BESL_DOCS_PATH: &str = "reference/besl";

#[cfg(test)]
pub mod tests {

	use std::sync::Arc;

	use serde_json::Value;

	use super::ProgramGenerator;
	use crate::asset::JsonObject;
	use crate::{
		ReferenceModel,
		asset::{
			ResourceId, handler::AssetHandler, handler::implementations::bema::BEMAAssetHandler, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::VariantModel,
	};

	struct TestShaderCompiler;

	impl super::ShaderCompiler for TestShaderCompiler {
		fn compile<'a>(
			&'a self,
			_generator: &'a dyn ProgramGenerator,
			name: &'a str,
			_shader_code: &'a str,
			format: &'a str,
			_domain: &'a str,
			_material: &'a JsonObject,
			_shader_json: &'a Value,
			stage: &'a str,
		) -> crate::r#async::BoxedFuture<'a, Result<(crate::resources::material::Shader, Box<[u8]>), ()>> {
			Box::pin(async move {
				assert_eq!(format, "besl");

				let stage = match stage {
					"Vertex" => crate::types::ShaderTypes::Vertex,
					"Fragment" => crate::types::ShaderTypes::Fragment,
					"Compute" => crate::types::ShaderTypes::Compute,
					_ => return Err(()),
				};

				Ok((
					crate::resources::material::Shader {
						id: name.to_string(),
						stage,
						interface: crate::resources::material::ShaderInterface {
							workgroup_size: Some((128, 0, 0)),
							bindings: vec![crate::resources::material::Binding::new(
								0,
								crate::resources::material::BindingKind::StorageBuffer,
								1,
								Some(4),
								true,
								false,
							)],
						},
						artifact: crate::resources::material::ShaderArtifact::Msl {
							entry_point: crate::shader::besl::backends::msl::MSL_ENTRY_POINT.to_string(),
						},
						source_hash: 42,
					},
					b"compiled-test-shader".to_vec().into_boxed_slice(),
				))
			})
		}
	}

	/// The `RootTestShaderGenerator` struct supplies the complete test renderer contract used by BEMA integration tests.
	pub struct RootTestShaderGenerator {}

	/// The `MinimalTestShaderGenerator` struct isolates importer tests from renderer-specific material shader contracts.
	pub struct MinimalTestShaderGenerator;

	impl ProgramGenerator for MinimalTestShaderGenerator {
		fn transform<'a>(&self, _: besl::parser::Node<'a>, _: &'a JsonObject) -> besl::parser::Node<'a> {
			besl::parser::Node::root_with_children(vec![besl::parser::Node::main_function(Vec::new())])
		}
	}

	impl RootTestShaderGenerator {
		pub fn new() -> RootTestShaderGenerator {
			RootTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for RootTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
			let material_struct = besl::parser::Node::buffer("Material", vec![besl::parser::Node::member("color", "vec4f")]);

			let sample_function =
				besl::parser::Node::function("sample_", vec![besl::parser::Node::member("t", "u32")], "void", vec![]);

			let mid_test_shader_generator = MidTestShaderGenerator::new();

			root.add(vec![material_struct, sample_function]);

			mid_test_shader_generator.transform(root, material)
		}
	}

	pub struct MidTestShaderGenerator {}

	impl MidTestShaderGenerator {
		pub fn new() -> MidTestShaderGenerator {
			MidTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for MidTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
			let binding = besl::parser::Node::binding(
				"materials",
				besl::parser::Node::buffer("Materials", vec![besl::parser::Node::member("materials", "Material[16]")]),
				0,
				true,
				false,
			);

			let leaf_test_shader_generator = LeafTestShaderGenerator::new();

			root.add(vec![binding]);

			leaf_test_shader_generator.transform(root, material)
		}
	}

	struct LeafTestShaderGenerator {}

	impl LeafTestShaderGenerator {
		pub fn new() -> LeafTestShaderGenerator {
			LeafTestShaderGenerator {}
		}
	}

	impl ProgramGenerator for LeafTestShaderGenerator {
		fn transform<'a>(&self, mut root: besl::parser::Node<'a>, _: &JsonObject) -> besl::parser::Node<'a> {
			let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("material_index", "u32")]);

			let main = besl::parser::Node::function(
				"main",
				vec![],
				"void",
				vec![besl::parser::Node::glsl(
					"push_constant;\nmaterials;\nsample_(0);\n",
					&["push_constant", "materials", "sample_"],
					&[],
				)],
			);

			root.add(vec![push_constant, main]);

			root
		}
	}

	#[r#async::test]
	async fn load_material() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			// Authored material files accept the JSON5 conveniences used by hand-written assets.
			domain: 'World',
				type: 'Surface',
				shaders: {
					Compute: 'load_material_fragment.besl',
				},
			variables: [
				{
					name: 'color',
					data_type: 'vec4f',
					type: 'Static',
					value: 'Purple',
				},
			],
		}"#;

		asset_storage_backend.add_file("load_material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("load_material_fragment.besl", shader_file.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		let mut asset_handler = BEMAAssetHandler::new();

		asset_handler.compiler = Box::new(TestShaderCompiler);

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		asset_manager.add_asset_handler(asset_handler);

		asset_manager
			.bake("load_material.bema")
			.await
			.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 2);

		let shader = resource_storage_backend
			.get_resource(ResourceId::new("load_material_fragment.besl"))
			.expect("Expected shader");

		assert_eq!(shader.id, "load_material_fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("load_material_fragment.besl"))
			.expect("Expected shader data");

		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert_eq!(shader_spirv, "compiled-test-shader");

		let shader_model: crate::resources::material::Shader = crate::from_slice(&shader.resource).unwrap();

		assert_eq!(shader_model.id, "load_material_fragment.besl");
		assert!(matches!(shader_model.stage, crate::types::ShaderTypes::Compute));
		assert_eq!(shader_model.interface.workgroup_size, Some((128, 0, 0)));
		assert_eq!(shader_model.interface.bindings.len(), 1);
		assert_eq!(shader_model.source_hash, 42);
		assert!(matches!(
			shader_model.artifact,
			crate::resources::material::ShaderArtifact::Msl { ref entry_point }
				if entry_point == crate::shader::besl::backends::msl::MSL_ENTRY_POINT
		));

		let material = resource_storage_backend
			.get_resource(ResourceId::new("load_material.bema"))
			.expect("Expected material");

		assert_eq!(material.id, "load_material.bema");
		assert_eq!(material.class, "Material");
	}

	#[r#async::test]
	async fn load_variant() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			"domain": "World",
				"type": "Surface",
				"shaders": {
					"Compute": "load_variant_fragment.besl"
				},
			"variables": [
				{
					"name": "color",
					"data_type": "vec4f",
					"type": "Static",
					"value": "Purple"
				}
			]
		}"#;

		asset_storage_backend.add_file("load_variant_material.bema", material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			materials;
		}";

		asset_storage_backend.add_file("load_variant_fragment.besl", shader_file.as_bytes());

		let variant_json = r#"{
			"parent": "load_variant_material.bema",
			"variables": [
				{
					"name": "color",
					"value": "White"
				}
			]
		}"#;

		asset_storage_backend.add_file("load_variant.bema", variant_json.as_bytes());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		let mut asset_handler = BEMAAssetHandler::new();

		asset_handler.compiler = Box::new(TestShaderCompiler);

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		asset_manager.add_asset_handler(asset_handler);

		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("load_variant.bema")
			.await
			.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = resource_storage_backend
			.get_resource(ResourceId::new("load_variant_fragment.besl"))
			.expect("Expected shader");

		assert_eq!(shader.id, "load_variant_fragment.besl");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("load_variant_fragment.besl"))
			.expect("Expected shader data");

		let shader_spirv = String::from_utf8_lossy(&shader_spirv);

		assert!(!shader_spirv.is_empty());

		let material = resource_storage_backend
			.get_resource(ResourceId::new("load_variant_material.bema"))
			.expect("Expected material");

		assert_eq!(material.id, "load_variant_material.bema");
		assert_eq!(material.class, "Material");

		let variant = resource_storage_backend
			.get_resource(ResourceId::new("load_variant.bema"))
			.expect("Expected variant");

		assert_eq!(variant.id, "load_variant.bema");
		assert_eq!(variant.class, "Variant");
	}
}

use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use utils::Extent;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	store_model, store_model_owned,
};
use crate::pbr::{
	BrdfMaterialDescription, BrdfNode, BrdfNodeId, BrdfValue, generate_textured_brdf_program, material_texture_variable_name,
};
use crate::resources::image::Image;
use crate::shader::{
	artifact::finalize_platform_shader_artifact,
	besl::{
		backends::platform::{PlatformShaderCompiler, PlatformShaderLanguage},
		evaluation::ProgramEvaluation,
	},
};
use crate::{
	ProcessedAsset, ReferenceModel,
	asset::{self, JsonObject},
	r#async::spawn_cpu_task,
	online_docs_url, resource,
	resources::material::{
		Binding, MaterialCoverage, MaterialModel, ParameterModel, RenderModel, Shader, ShaderInterface, ValueModel,
		VariantModel, VariantVariableModel,
	},
	shader::generator::ShaderGenerationSettings,
	types::{AlphaMode, ShaderTypes},
};
