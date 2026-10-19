/// The `ProgramGenerator` trait turns a material's BEMX graph into the BESL program its renderer evaluates.
///
/// Each renderer owns how a material reaches its BRDF: the entry point, the bindings, and the code around the surface.
/// Install one on the material, glTF, and FBX asset handlers so every material they bake targets that renderer.
/// Generators write node values with [`bemx::besl::Lowering`].
pub trait ProgramGenerator: Send + Sync {
	/// Builds the BESL program that evaluates `graph` for a material whose parameters are `parameters`, in the order
	/// the renderer binds them.
	///
	/// Returns the error text to report when the graph needs something the renderer cannot supply, such as a
	/// parameter `parameters` does not declare.
	fn generate<'a>(
		&self,
		graph: &'a bemx::Graph,
		parameters: &'a [MaterialParameter],
	) -> Result<besl::parser::Node<'a>, String>;
}

/// The `MaterialParameter` struct declares one value or texture a material supplies, so a generated program can bind
/// what a graph's parameter and image nodes name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterialParameter {
	pub name: String,
	pub kind: ParameterKind,
}

/// The `ParameterKind` enum tells a program generator how a material parameter is bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterKind {
	/// A constant of the given type, which [`bemx::Node::Parameter`] reads.
	Value(bemx::Type),
	/// A texture, which [`bemx::Node::Image`] samples.
	Texture,
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

	/// Bakes a material definition and the shader its graph generates.
	async fn bake_material<'a>(&self, context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
		let invalid = |problem: &str| {
			context.error(format_args!(
				"Invalid material '{}': {problem} See {}.",
				url.as_ref(),
				online_docs_url(BEMA_DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		};

		// Every material declares its render domain, even though shader generation does not read it.
		asset["domain"]
			.as_str()
			.ok_or_else(|| invalid("it has no `domain`. The most likely cause is a material written without one."))?;
		let generator = self.generator.as_deref().ok_or(LoadErrors::FailedToProcess)?;
		let graph = asset["graph"].as_str().ok_or_else(|| {
			invalid("it names no `graph`. The most likely cause is a material written for the removed `shaders` field; point `graph` at a `.bemx` file instead.")
		})?;
		let asset_variables = asset["variables"].as_array().ok_or_else(|| {
			invalid("it has no `variables` array. The most likely cause is a missing or misspelt `variables` field.")
		})?;
		// Each variable is read once: the shader binds its parameter and the material stores its value.
		let (parameters, sources): (Vec<_>, Vec<_>) = asset_variables
			.iter()
			.map(material_variable)
			.collect::<Result<Vec<_>, _>>()
			.map_err(|problem| invalid(&problem))?
			.into_iter()
			.unzip();

		// Texture parameters can trigger independent dependency bakes; scalar values complete immediately.
		let value_requests = sources
			.iter()
			.map(|&(data_type, value)| async move { resolve_value(context, data_type, value).await });

		// The shader and the parameter values do not depend on each other, so they bake together.
		let (shader, values) = std::future::join!(
			self.compile_graph(context, url, generator, ResourceId::new(graph), &parameters),
			buffered_in_order(value_requests, 8),
		)
		.await;

		let parameters = parameters
			.into_iter()
			.zip(sources)
			.zip(values?)
			.map(|((parameter, (data_type, _)), value)| ParameterModel {
				name: parameter.name,
				r#type: data_type.to_string(),
				value,
			})
			.collect();

		let resource = MaterialModel {
			double_sided: false,
			alpha_mode: AlphaMode::Opaque,
			coverage: MaterialCoverage::default(),
			model: RenderModel {
				name: "Visibility".to_string(),
				pass: "MaterialEvaluation".to_string(),
			},
			shaders: vec![shader?],
			parameters,
		};

		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}

	/// Bakes one material variant by resolving its inherited parameters.
	async fn bake_variant<'a>(context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
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
		let values = buffered_in_order(value_requests, 8).await?;
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

	/// Reads the material's BEMX graph, generates its program, and compiles and stores the shader.
	///
	/// The shader is stored under the material's own ID with a `#shader` fragment, because the program depends on the
	/// material's parameters as well as on the graph, so two materials sharing a graph may still need two shaders.
	async fn compile_graph(
		&self,
		context: BakeContext<'_>,
		material: ResourceId<'_>,
		generator: &dyn ProgramGenerator,
		path: ResourceId<'_>,
		parameters: &[MaterialParameter],
	) -> Result<ReferenceModel<Shader>, LoadErrors> {
		let (source, format) = context.resolve(path).await?;

		if format != "bemx" {
			context.error(format_args!(
				"Unknown material graph format '{format}' in '{}'. The most likely cause is a `graph` field that names a file other than a `.bemx` graph. See {}.",
				material.as_ref(),
				online_docs_url(BEMA_DOCS_PATH)
			));
			return Err(LoadErrors::FailedToProcess);
		}

		let shader_id = format!("{}#shader", material.as_ref());
		let compile_error = |error: String| {
			context.error(shader_compilation_error_message(&shader_id, &error));
			LoadErrors::FailedToProcess
		};

		let source = std::str::from_utf8(&source).map_err(|_| {
			compile_error(format!(
				"Material graph '{}' is not UTF-8 text. The most likely cause is a binary file saved with a `.bemx` extension.",
				path.as_ref()
			))
		})?;
		let graph = bemx::json::parse(source)
			.map_err(|error| compile_error(format!("Failed to read material graph '{}': {error}", path.as_ref())))?;
		let program = generator.generate(&graph, parameters).map_err(compile_error)?;
		let (shader, bytes) = self
			.compiler
			.compile(program, compute_material_stage(&shader_id))
			.await
			.map_err(compile_error)?;

		store_model_owned(context, &shader_id, shader, bytes).await
	}
}

/// Reads one entry of a `.bema` file's `variables`: the parameter it declares, its data type, and its value.
fn material_variable(variable: &Value) -> Result<(MaterialParameter, (&str, &str)), String> {
	let name = variable["name"]
		.as_str()
		.ok_or("a variable has no `name`. The most likely cause is a variable written without one.")?;
	let data_type = variable["data_type"].as_str().ok_or_else(|| {
		format!("variable '{name}' has no `data_type`. The most likely cause is a variable written without one.")
	})?;

	let kind = match data_type {
		"float" => ParameterKind::Value(bemx::Type::Float),
		"vec3f" => ParameterKind::Value(bemx::Type::Vector3),
		"vec4f" => ParameterKind::Value(bemx::Type::Vector4),
		"Texture2D" => ParameterKind::Texture,
		data_type => {
			return Err(format!(
				"variable '{name}' has the unsupported data type '{data_type}'. The most likely cause is a type other than `float`, `vec3f`, `vec4f`, or `Texture2D`."
			));
		}
	};

	let value = variable["value"]
		.as_str()
		.ok_or_else(|| format!("variable '{name}' has no `value`. The most likely cause is a variable written without one."))?;

	Ok((
		MaterialParameter {
			name: name.to_string(),
			kind,
		},
		(data_type, value),
	))
}

/// Runs `requests` with at most `limit` in flight and returns their results in input order once all have finished.
///
/// The first error in input order wins, after every request has completed.
async fn buffered_in_order<T, E>(
	requests: impl Iterator<Item = impl Future<Output = Result<T, E>>>,
	limit: usize,
) -> Result<Vec<T>, E> {
	use utils::r#async::StreamExt as _;

	utils::r#async::stream::iter(requests)
		.buffered(limit)
		.collect::<Vec<_>>()
		.await
		.into_iter()
		.collect()
}

impl AssetHandler for BEMAAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "bema"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
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

/// Returns the compute generation settings shared by authored and generated material shaders.
fn compute_material_stage(name: &str) -> ShaderGenerationSettings {
	ShaderGenerationSettings::compute(Extent::line(128)).name(name.to_string())
}

/// Bounds concurrent generated-material shader compiles so platform compiler processes do not oversubscribe the machine.
const GENERATED_SHADER_COMPILE_CONCURRENCY: usize = 4;

/// Bounds concurrent texture bakes so decoded images and compression buffers stay within a predictable memory footprint.
const TEXTURE_BAKE_CONCURRENCY: usize = 8;

/// The `GeneratedMaterial` struct describes one importer material whose shader, textures, and variant are generated.
pub(crate) struct GeneratedMaterial {
	/// The material and variant are stored as `<base_id>.material` and `<base_id>.variant`.
	pub(crate) base_id: String,
	/// The material. Its `images` index the container's image IDs.
	pub(crate) material: ImportedMaterial,
}

/// The `MaterialSource` enum tells [`resolve_container_materials`] where one glTF or FBX material's variant comes from.
pub(crate) enum MaterialSource {
	/// A BEAD sidecar names an authored variant, which bakes as a dependency.
	Override(String),
	/// The importer describes the material as a BEMX graph, which is generated.
	Generated(GeneratedMaterial),
}

/// Bounds concurrent BEAD material override bakes.
const OVERRIDE_BAKE_CONCURRENCY: usize = 8;

/// Resolves the variants of a container's materials, in the order given.
///
/// glTF and FBX importers call this once per container. Overrides bake as dependencies while the other materials are
/// generated through [`store_generated_materials`]; both run concurrently. `image_ids` holds the resource ID of each
/// container image, indexed by the materials' `images`.
///
/// Next, reference the returned variants from the container's mesh primitives.
pub(crate) async fn resolve_container_materials(
	context: BakeContext<'_>,
	generator: Option<&dyn ProgramGenerator>,
	container_id: ResourceId<'_>,
	image_ids: &[String],
	materials: Vec<MaterialSource>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	let mut is_override = Vec::with_capacity(materials.len());
	let mut overrides = Vec::new();
	let mut generated = Vec::new();

	for material in materials {
		is_override.push(matches!(material, MaterialSource::Override(_)));

		match material {
			MaterialSource::Override(id) => overrides.push(id),
			MaterialSource::Generated(material) => generated.push(material),
		}
	}

	let (overridden, generated) = std::future::join!(
		context.bake_dependencies::<VariantModel>(&overrides, OVERRIDE_BAKE_CONCURRENCY),
		store_generated_materials(context, generator, container_id, image_ids, generated),
	)
	.await;

	let (mut overridden, mut generated) = (overridden?.into_iter(), generated?.into_iter());

	// Put overridden and generated variants back in material order.
	is_override
		.into_iter()
		.map(|is_override| if is_override { overridden.next() } else { generated.next() })
		.collect::<Option<Vec<_>>>()
		.ok_or(LoadErrors::FailedToProcess)
}

/// Reads the variant a BEAD sidecar assigns to the container material `name`, from `asset.<name>.asset`.
pub(crate) fn bead_material_override(spec: Option<&serde_json::Value>, name: &str) -> Option<String> {
	spec?["asset"][name]["asset"].as_str().map(ToString::to_string)
}

/// Generates and stores the variants of a container's generated materials, in the order given.
///
/// Every image the materials sample bakes once, as its own dependency, while materials whose graphs match share one
/// compiled shader.
async fn store_generated_materials(
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

	let mut baked_images = Vec::new();

	for &image_index in materials.iter().flat_map(|generated| &generated.material.images) {
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

	let imported = materials.iter().map(|generated| &generated.material).collect::<Vec<_>>();

	// Texture bakes and shader compiles do not depend on each other, so they run together.
	let (images, shaders) = std::future::join!(
		context.bake_dependencies::<Image>(&baked_image_ids, TEXTURE_BAKE_CONCURRENCY),
		store_generated_shaders(context, generator, container_id, &imported),
	)
	.await;

	let (images, shaders) = (images?, shaders?);

	let mut variants = Vec::with_capacity(materials.len());

	for (generated, shader) in materials.into_iter().zip(shaders) {
		let variables = generated
			.material
			.images
			.iter()
			.enumerate()
			.map(|(slot, image_index)| {
				let image = baked_images
					.iter()
					.position(|baked| baked == image_index)
					.map(|position| images[position].clone())
					.expect("every sampled image was baked");

				VariantVariableModel {
					name: material_texture_parameter_name(slot),
					r#type: "Texture2D".to_string(),
					value: ValueModel::Image(image),
				}
			})
			.collect();

		variants.push(store_generated_variant(context, generated, shader, variables).await?);
	}

	Ok(variants)
}

/// Stores one generated material and its variant.
async fn store_generated_variant(
	context: BakeContext<'_>,
	GeneratedMaterial { base_id, material }: GeneratedMaterial,
	shader: ReferenceModel<Shader>,
	variables: Vec<VariantVariableModel>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let alpha_mode = material.alpha_mode;

	let model = MaterialModel {
		double_sided: material.double_sided,
		alpha_mode,
		coverage: generated_material_coverage(&material),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};

	let model = store_model::<MaterialModel>(context, &format!("{base_id}.material"), model, &[]).await?;

	let variant = VariantModel {
		material: model,
		variables,
		alpha_mode,
	};

	store_model::<VariantModel>(context, &format!("{base_id}.variant"), variant, &[]).await
}

/// Extracts a generated material's opacity into the compact masked-raster contract: a constant factor times, at most,
/// the alpha of one texture.
fn generated_material_coverage(material: &ImportedMaterial) -> MaterialCoverage {
	/// Folds the constants an opacity multiplies together and finds the texture whose alpha it reads.
	fn collect<'a>(graph: &'a bemx::Graph, node: bemx::NodeId, factor: &mut f32, texture: &mut Option<&'a str>) {
		match graph.node(node) {
			bemx::Node::Constant(bemx::Value::Float(value)) => *factor *= value,
			// A colour constant is reached through the extraction of its alpha below.
			bemx::Node::Constant(bemx::Value::Vector4(value)) => *factor *= value[3],
			bemx::Node::Image { texture: name, .. } => *texture = Some(name),
			bemx::Node::Extract { value, index: 3 } => collect(graph, *value, factor, texture),
			bemx::Node::Binary {
				operator: bemx::BinaryOperator::Multiply,
				left,
				right,
			} => {
				collect(graph, *left, factor, texture);
				collect(graph, *right, factor, texture);
			}
			_ => {}
		}
	}

	let mut coverage = MaterialCoverage::default();
	let mut texture = None;

	if let Some(opacity) = material.graph.surface().opacity {
		collect(&material.graph, opacity, &mut coverage.factor, &mut texture);
	}

	coverage.texture_slot = texture
		.and_then(material_texture_slot)
		.filter(|slot| *slot < material.images.len())
		.map(|slot| slot as u32);

	coverage
}

/// Declares the texture parameters a generated material binds: one per image, named by slot.
fn generated_material_parameters(material: &ImportedMaterial) -> Vec<MaterialParameter> {
	(0..material.images.len())
		.map(|slot| MaterialParameter {
			name: material_texture_parameter_name(slot),
			kind: ParameterKind::Texture,
		})
		.collect()
}

/// Compiles each distinct generated shader once and stores it under an ID derived from its compile inputs.
///
/// Returns one shader reference per entry in `materials`, in order.
///
/// The ID hashes the lowered platform source and the compiler identity, so a later bake of the same container reuses
/// the stored binary while a change to the graph, program generator, BESL backend, settings, or toolchain compiles
/// again.
async fn store_generated_shaders(
	context: BakeContext<'_>,
	generator: &dyn ProgramGenerator,
	container_id: ResourceId<'_>,
	materials: &[&ImportedMaterial],
) -> Result<Vec<ReferenceModel<Shader>>, LoadErrors> {
	// Every generated shader of a container shares one compiler name, so the name never depends on the key it feeds.
	let compiler_name = format!("{}#shaders", container_id.as_ref());

	let compile_error = |error: String| {
		context.error(shader_compilation_error_message(&compiler_name, &error));
		LoadErrors::FailedToProcess
	};

	let compiler_identity = PlatformShaderCompiler::compiler_identity().await.map_err(compile_error)?;

	// Generated materials evaluate in the same compute stage as authored material shaders.
	let settings = compute_material_stage(&compiler_name);

	// Graph hashes skip generating repeated graphs; cache keys also merge distinct graphs that lower to the same source.
	let mut unique_by_graph: HashMap<u64, usize> = HashMap::new();
	let mut unique_by_key: HashMap<u64, usize> = HashMap::new();
	let mut unique = Vec::new();
	let mut unique_index_per_material = Vec::with_capacity(materials.len());

	for &material in materials {
		// Only the graph and its texture count reach the program; images, names, sidedness, and alpha mode stay in
		// the material resource.
		let graph = serde_json::to_vec(&(&material.graph, material.images.len())).map_err(|_| LoadErrors::FailedToProcess)?;
		let graph_hash = crate::resource::compression::payload_hash(&graph);

		let index = match unique_by_graph.entry(graph_hash) {
			Entry::Occupied(entry) => *entry.get(),
			Entry::Vacant(entry) => {
				let parameters = generated_material_parameters(material);
				let program = generator.generate(&material.graph, &parameters).map_err(compile_error)?;
				let prepared = prepare_besl_shader(program, &settings).map_err(compile_error)?;
				let key = prepared.cache_key(compiler_identity);

				*entry.insert(*unique_by_key.entry(key).or_insert_with(|| {
					unique.push((key, prepared));
					unique.len() - 1
				}))
			}
		};

		unique_index_per_material.push(index);
	}

	let requests = unique.into_iter().map(|(key, prepared)| async move {
		let shader_id = format!("{}#shaders/{key:016x}", container_id.as_ref());

		// A stored shader under this key came from the same platform source and compiler, so compiling again cannot
		// change it. This is what keeps iterating on a mesh from waiting on the platform compiler.
		if let Some(stored) = context.reusable_resource(ResourceId::new(&shader_id)).await {
			return Ok(stored.into());
		}

		let (shader, shader_bytes) = prepared.compile(&shader_id).await.map_err(|error| {
			context.error(shader_compilation_error_message(&shader_id, &error));
			LoadErrors::FailedToProcess
		})?;

		store_model_owned::<Shader, _>(context, &shader_id, shader, shader_bytes).await
	});

	// Ordered buffering keeps the stored shader order deterministic while platform compiler processes overlap.
	let unique_shaders = buffered_in_order(requests, GENERATED_SHADER_COMPILE_CONCURRENCY).await?;

	Ok(unique_index_per_material
		.into_iter()
		.map(|index| unique_shaders[index].clone())
		.collect())
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
		"vec4f" => Ok(ValueModel::Vector4(to_color(value))),
		"vec3f" => {
			let [red, green, blue, _] = to_color(value);

			Ok(ValueModel::Vector3([red, green, blue]))
		}
		"float" => Ok(ValueModel::Scalar(0f32)),
		"Texture2D" => Ok(ValueModel::Image(context.bake_dependency(value).await?)),
		_ => Err(LoadErrors::FailedToProcess),
	}
}

const BEMA_DOCS_PATH: &str = "reference/bema";

#[cfg(test)]
pub mod tests {

	use std::sync::Arc;

	use super::{MaterialParameter, ProgramGenerator};
	use crate::asset::handler::implementations::besl::ShaderCompiler;
	use crate::bemx;
	use crate::{
		ReferenceModel,
		asset::{
			ResourceId, handler::AssetHandler, handler::implementations::bema::BEMAAssetHandler, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::{Binding, BindingKind, SetInterface, Shader, ShaderArtifact, ShaderInterface, VariantModel},
		shader::{besl::backends::msl::MSL_ENTRY_POINT, generator::ShaderGenerationSettings},
	};

	struct TestShaderCompiler;

	impl ShaderCompiler for TestShaderCompiler {
		fn compile<'a>(
			&'a self,
			_program: besl::parser::Node<'a>,
			settings: ShaderGenerationSettings,
		) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), String>> {
			Box::pin(async move {
				Ok((
					Shader {
						id: settings.name,
						stage: settings.stage.into(),
						interface: ShaderInterface {
							workgroup_size: Some((128, 0, 0)),
							sets: vec![SetInterface::with_bindings(
								"material",
								vec![Binding::new(0, BindingKind::StorageBuffer, 1, Some(4), true, false)],
							)],
						},
						artifact: ShaderArtifact::Msl {
							entry_point: MSL_ENTRY_POINT.to_string(),
						},
					},
					b"compiled-test-shader".to_vec().into_boxed_slice(),
				))
			})
		}
	}

	/// The `MinimalTestShaderGenerator` struct isolates importer tests from renderer-specific material shader contracts.
	pub struct MinimalTestShaderGenerator;

	impl ProgramGenerator for MinimalTestShaderGenerator {
		fn generate<'a>(&self, _: &'a bemx::Graph, _: &'a [MaterialParameter]) -> Result<besl::parser::Node<'a>, String> {
			Ok(besl::parser::Node::root_with_children(vec![
				besl::parser::Node::main_function(Vec::new()),
			]))
		}
	}

	#[r#async::test]
	async fn load_variant() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"graph": "load_variant.bemx",
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

		let graph = r#"{
			nodes: [
				{ name: "color", node: "parameter", parameter: "color", type: "vector4" },
				{ name: "rgb", node: "swizzle", in: "color", channels: "rgb" },
				{ name: "surface", node: "surface", base_color: "rgb" },
			],
		}"#;

		asset_storage_backend.add_file("load_variant.bemx", graph.as_bytes());

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

		asset_manager.add_asset_handler(BEMAAssetHandler {
			generator: Some(Box::new(MinimalTestShaderGenerator)),
			compiler: Box::new(TestShaderCompiler),
		});

		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("load_variant.bema")
			.await
			.expect("Failed to load material");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 3);

		let shader = resource_storage_backend
			.get_resource(ResourceId::new("load_variant_material.bema#shader"))
			.expect("Expected shader");

		assert_eq!(shader.id, "load_variant_material.bema#shader");
		assert_eq!(shader.class, "Shader");

		let shader_spirv = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("load_variant_material.bema#shader"))
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

use std::collections::hash_map::Entry;
use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use utils::Extent;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	store_model, store_model_owned,
};
use crate::asset::handler::implementations::besl::{
	PlatformShaderCompilerAdapter, ShaderCompiler, prepare_besl_shader, shader_compilation_error_message,
};
use crate::bemx::{self, import::ImportedMaterial, material_texture_parameter_name, material_texture_slot};
use crate::resources::image::Image;
use crate::{
	ProcessedAsset, ReferenceModel, asset,
	r#async::spawn_cpu_task,
	online_docs_url, resource,
	resources::material::{
		MaterialCoverage, MaterialModel, ParameterModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel,
	},
	shader::{besl::backends::platform::PlatformShaderCompiler, generator::ShaderGenerationSettings},
	types::AlphaMode,
};
