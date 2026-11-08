/// The `ProgramGenerator` trait turns a material's BEMX graph into the BESL program its renderer evaluates.
///
/// Each renderer owns how a material reaches its BRDF: the entry point, the bindings, and the code around the surface.
/// Install one on the material, glTF, and FBX asset handlers so every material they bake targets that renderer.
/// Generators write node values with [`bemx::besl::Lowering`].
pub trait ProgramGenerator: Send + Sync {
	/// Builds the BESL program that evaluates `graph` for a material whose parameters are `parameters`, in the order
	/// the renderer binds them, or `None` when the renderer evaluates its materials in shaders it assembles after the
	/// bake instead. The bake stores the material's [`MaterialProgram`] either way.
	///
	/// Returns the error text to report when the graph needs something the renderer cannot supply, such as a
	/// parameter `parameters` does not declare.
	fn generate<'a>(
		&self,
		graph: &'a bemx::Graph,
		parameters: &'a [MaterialParameter],
	) -> Result<Option<besl::parser::Node<'a>>, String>;

	/// Builds the closest-hit BESL program that reports `graph`'s surface to the rays that hit it, or `None` when the
	/// renderer traces no rays.
	///
	/// The bake stores the shader beside the one [`Self::generate`] builds, and the renderer's ray tracing pipelines run
	/// it for hits on surfaces of the material. Returns the error text to report, as [`Self::generate`] does.
	fn generate_hit<'a>(
		&self,
		graph: &'a bemx::Graph,
		parameters: &'a [MaterialParameter],
	) -> Result<Option<besl::parser::Node<'a>>, String> {
		let _ = (graph, parameters);
		Ok(None)
	}

	/// Returns the key of the arm that evaluates `material` in the assembled shader `id`, a `.uber` resource ID the
	/// renderer names, or `None` when that shader leaves the material out.
	///
	/// Materials with equal keys must generate equal code, because the shader holds one arm per key. The renderer
	/// computes the same key for a material at runtime to find out whether a baked shader evaluates it.
	fn assembled_key(&self, id: &str, material: &AssembledMaterial<'_>) -> Result<Option<u32>, String> {
		let _ = (id, material);
		Err(NO_ASSEMBLED_SHADERS.to_string())
	}

	/// Builds the assembled shader `id`, with one arm for each of `arms`, and the settings to compile it with.
	///
	/// The bake calls it once all materials are baked, with every material [`Self::assembled_key`] keeps.
	fn generate_assembled<'a>(
		&self,
		id: &str,
		arms: &'a [AssembledArm<'a>],
	) -> Result<(besl::parser::Node<'a>, ShaderGenerationSettings), String> {
		let _ = (id, arms);
		Err(NO_ASSEMBLED_SHADERS.to_string())
	}
}

/// The error [`ProgramGenerator`]'s assembled-shader methods report for a renderer that assembles no shaders.
const NO_ASSEMBLED_SHADERS: &str = "This renderer assembles no evaluation shaders. The most likely cause is a bake of a `.uber` resource in a project whose renderer evaluates each material with its own shader. Bake only the resources the renderer's material evaluation mode names.";

/// The `AssembledMaterial` struct is one material variant an assembled evaluation shader can evaluate: its program
/// and the values that specialize it.
///
/// The bake reads one for every variant in the store and passes them to [`ProgramGenerator::assembled_key`].
pub struct AssembledMaterial<'a> {
	/// The render model and pass the material targets.
	pub model: &'a RenderModel,
	pub graph: &'a bemx::Graph,
	/// The parameters the program binds, in binding order.
	pub parameters: &'a [MaterialParameter],
	/// See [`MaterialProgram::identity`].
	pub identity: u64,
	/// The components of every static value parameter of [`Self::parameters`], in their order.
	pub static_values: &'a [Vec<f32>],
	pub alpha_mode: AlphaMode,
}

/// The `AssembledArm` struct is one arm of an assembled evaluation shader: the code that evaluates the materials
/// whose key is `key`. Pass every arm to [`ProgramGenerator::generate_assembled`].
pub struct AssembledArm<'a> {
	/// The key, as the decimal literal the shader matches.
	pub key: &'a str,
	/// One of the materials the arm evaluates; all of them generate the same code.
	pub material: &'a AssembledMaterial<'a>,
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
	/// A constant of the given type, which [`bemx::Node::Parameter`] reads. Its value is compiled into the program.
	Value(bemx::Type),
	/// A value of the given type, which [`bemx::Node::Parameter`] reads, that the application can change while the
	/// material is in use. The program reads it from memory the renderer writes.
	Dynamic(bemx::Type),
	/// A texture, which [`bemx::Node::Image`] samples.
	Texture,
}

impl ParameterKind {
	/// Returns the kind of a `.bema` variable of `data_type` (`float`, `vec3f`, `vec4f`, or `Texture2D`), or `None`
	/// for a data type no variable can have. A texture is never dynamic.
	fn from_data_type(data_type: &str, dynamic: bool) -> Option<Self> {
		let r#type = match data_type {
			"float" => bemx::Type::Float,
			"vec3f" => bemx::Type::Vector3,
			"vec4f" => bemx::Type::Vector4,
			"Texture2D" if !dynamic => return Some(Self::Texture),
			_ => return None,
		};
		Some(if dynamic { Self::Dynamic(r#type) } else { Self::Value(r#type) })
	}

	/// Returns the `.bema` data type of this kind, which [`Self::from_data_type`] reads back.
	fn data_type(self) -> &'static str {
		match self {
			Self::Value(r#type) | Self::Dynamic(r#type) => match r#type {
				bemx::Type::Float => "float",
				bemx::Type::Vector3 => "vec3f",
				_ => "vec4f",
			},
			Self::Texture => "Texture2D",
		}
	}
}

impl MaterialProgram {
	/// Keeps `graph` and `parameters` so a renderer can generate the material's surface code after the bake.
	pub(crate) fn new(graph: &bemx::Graph, parameters: &[MaterialParameter]) -> Result<Self, String> {
		let reads_backdrop = graph.reads_backdrop();
		let graph = serde_json::to_string(graph).map_err(|error| {
			format!("Failed to store the material graph: {error}. The most likely cause is a non-finite constant in it.")
		})?;
		let parameters = parameters
			.iter()
			.map(|parameter| ProgramParameter {
				name: parameter.name.clone(),
				data_type: parameter.kind.data_type().to_string(),
				dynamic: matches!(parameter.kind, ParameterKind::Dynamic(_)),
			})
			.collect::<Vec<_>>();
		let mut identity = graph.clone().into_bytes();
		for parameter in &parameters {
			identity.extend_from_slice(parameter.name.as_bytes());
			identity.push(0);
			identity.extend_from_slice(parameter.data_type.as_bytes());
			identity.push(u8::from(parameter.dynamic));
		}
		Ok(Self {
			identity: crate::resource::compression::payload_hash(&identity),
			graph,
			parameters,
			reads_backdrop,
		})
	}

	/// Reads the graph back, checking it as [`bemx::GraphBuilder`] checks a new one.
	pub fn graph(&self) -> Result<bemx::Graph, String> {
		serde_json::from_str(&self.graph).map_err(|error| {
			format!(
				"Failed to read a stored material graph: {error}. The most likely cause is a material baked by another engine version. Bake it again with `beld bake --force`."
			)
		})
	}

	/// Returns the parameters the program binds, in the order the renderer binds them.
	pub fn parameters(&self) -> Vec<MaterialParameter> {
		self.parameters
			.iter()
			.map(|parameter| MaterialParameter {
				name: parameter.name.clone(),
				kind: ParameterKind::from_data_type(&parameter.data_type, parameter.dynamic)
					.expect("A stored program parameter was written from a valid kind."),
			})
			.collect()
	}
}

/// The `BEMAAssetHandler` struct bakes `.bema` materials, their variants, and the assembled evaluation shaders a
/// renderer names.
///
/// Materials target the asset manager's [`ProgramGenerator`].
pub struct BEMAAssetHandler;

impl BEMAAssetHandler {
	/// Bakes a material definition and the shader its graph generates.
	async fn bake_material<'a>(&self, context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
		let invalid = |problem: &str| report_invalid(context, "material", url, problem);

		// Every material declares its render domain, even though shader generation does not read it.
		asset["domain"]
			.as_str()
			.ok_or_else(|| invalid("it has no `domain`. The most likely cause is a material written without one."))?;
		let generator = context.program_generator()?;
		let graph = asset["graph"].as_str().ok_or_else(|| {
			invalid("it names no `graph`. The most likely cause is a material written for the removed `shaders` field; point `graph` at a `.bemx` file instead.")
		})?;
		// A root material starts from an opaque, one-sided surface.
		let (alpha_mode, double_sided) =
			surface_coverage(asset, AlphaMode::Opaque, false).map_err(|problem| invalid(&problem))?;
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
		let invalid = &invalid;
		let value_requests = sources
			.iter()
			.map(|&(data_type, value)| async move { resolve_value(context, data_type, value, invalid).await });

		// The shader and the parameter values do not depend on each other, so they bake together.
		let (program, values) = std::future::join!(
			self.compile_graph(context, url, generator, ResourceId::new(graph), &parameters),
			buffered_in_order(value_requests, 8),
		)
		.await;

		let parameters = parameters
			.into_iter()
			.zip(sources)
			.zip(values?)
			.map(|((parameter, (data_type, _)), value)| ParameterModel {
				dynamic: matches!(parameter.kind, ParameterKind::Dynamic(_)),
				name: parameter.name,
				r#type: data_type.to_string(),
				value,
			})
			.collect();
		let (shaders, program) = program?;
		backdrop_needs_blending(program.reads_backdrop, alpha_mode).map_err(|problem| invalid(&problem))?;

		let resource = MaterialModel {
			double_sided,
			alpha_mode,
			model: RenderModel {
				name: "Visibility".to_string(),
				pass: "MaterialEvaluation".to_string(),
			},
			shaders,
			parameters,
			program,
		};

		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}

	/// Bakes one material variant by resolving its inherited parameters.
	async fn bake_variant<'a>(context: BakeContext<'a>, url: ResourceId<'a>, asset: &Value) -> Result<(), LoadErrors> {
		let parent_material_url = asset["parent"].as_str().ok_or(LoadErrors::FailedToProcess)?;
		let material = context.bake_dependency(parent_material_url).await?;
		let authored_variables = asset["variables"].as_array().ok_or(LoadErrors::FailedToProcess)?;
		let invalid = |problem: &str| report_invalid(context, "material variant", url, problem);
		let value_of = |name: &str| {
			authored_variables
				.iter()
				.find(|variable| variable["name"].as_str() == Some(name))
				.map(|variable| &variable["value"])
				.filter(|value| !value.is_null())
		};
		let resource = variant_model(context, material, &value_of, asset, VariantSource::File, &invalid).await?;

		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}

	/// Bakes the assembled evaluation shader `url`, a `.uber` resource with no asset behind it, from every material
	/// variant the store holds.
	///
	/// The generator decides from `url` and each material which variants the shader evaluates. Variants are baked as
	/// dependencies, so editing one of their sources makes the shader stale. The compiled shader is stored under an ID
	/// derived from its platform source, so an unchanged material set reuses it.
	async fn bake_assembled<'a>(&self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let fail = |error: String| {
			context.error(shader_compilation_error_message(url.as_ref(), &error));
			LoadErrors::FailedToProcess
		};
		let generator = context.program_generator()?;
		let mut ids = context
			.query_resources(crate::resource::storage_backend::Query::new("Variant"))
			.await?;
		ids.sort_unstable();
		let variants = context.bake_dependencies::<VariantModel>(&ids, 8).await?;

		// Each variant's program, parameters, and static values, owned here so the materials below can borrow them.
		let mut programs = Vec::with_capacity(variants.len());
		for (id, variant) in ids.iter().zip(&variants) {
			let read_error = |error: crate::ResourceArchiveError| {
				fail(format!(
					"Failed to read variant '{id}': {error}. The most likely cause is a resource baked by another engine version. Bake again with `beld bake --force`."
				))
			};
			let variant: VariantModel = crate::from_slice(&variant.resource).map_err(read_error)?;
			let material: MaterialModel = crate::from_slice(&variant.material.resource).map_err(read_error)?;
			let program = material.program;
			let static_values = variant.static_values().map(<[f32]>::to_vec).collect::<Vec<_>>();
			programs.push((
				material.model,
				program.graph().map_err(fail)?,
				program.parameters(),
				program.identity,
				static_values,
				variant.alpha_mode,
			));
		}
		let materials = programs
			.iter()
			.map(
				|(model, graph, parameters, identity, static_values, alpha_mode)| AssembledMaterial {
					model,
					graph,
					parameters,
					identity: *identity,
					static_values,
					alpha_mode: *alpha_mode,
				},
			)
			.collect::<Vec<_>>();

		// One arm per key, in key order, so the same materials always assemble the same program. Two materials share a
		// key when they share a program and static values; any other pair that does is a hash collision.
		let mut by_key = std::collections::BTreeMap::<u32, &AssembledMaterial>::new();
		for material in &materials {
			let Some(key) = generator.assembled_key(url.as_ref(), material).map_err(fail)? else {
				continue;
			};
			let arm = by_key.entry(key).or_insert(material);
			if arm.identity != material.identity || arm.static_values != material.static_values {
				return Err(fail(format!(
					"Two different materials hash to arm key {key}. The most likely cause is a rare collision of the 32-bit key. Change one of the two materials' static values or graphs slightly, then bake again."
				)));
			}
		}
		let keys = by_key.keys().copied().collect::<Vec<_>>();
		let key_literals = keys.iter().map(u32::to_string).collect::<Vec<_>>();
		let arms = key_literals
			.iter()
			.zip(by_key.values())
			.map(|(key, material)| AssembledArm { key, material })
			.collect::<Vec<_>>();

		let (program, settings) = generator.generate_assembled(url.as_ref(), &arms).map_err(fail)?;
		let prepared = context.prepare_shader(program, &settings).map_err(fail)?;
		let compiler_identity = PlatformShaderCompiler::compiler_identity().await.map_err(fail)?;
		let shader_id = format!("{}#shaders/{:016x}", url.as_ref(), prepared.cache_key(compiler_identity));
		store_generated_shader(context, &shader_id, prepared).await?;

		let resource = AssembledShader { shader: shader_id, keys };
		context.store_primary(ProcessedAsset::new(url, resource), &[]).await
	}

	/// Reads the material's BEMX graph, generates its programs, and compiles and stores their shaders: the material's
	/// shader, when the generator builds one, then its closest-hit shader, when the generator builds one. It also
	/// returns the material's [`MaterialProgram`], which the shaders assembled at the end of the bake read.
	///
	/// The shaders are stored under the material's own ID with `#shader` and `#hit-shader` fragments, because the
	/// programs depend on the material's parameters as well as on the graph, so two materials sharing a graph may still
	/// need two shaders.
	async fn compile_graph(
		&self,
		context: BakeContext<'_>,
		material: ResourceId<'_>,
		generator: &dyn ProgramGenerator,
		path: ResourceId<'_>,
		parameters: &[MaterialParameter],
	) -> Result<(Vec<ReferenceModel<Shader>>, MaterialProgram), LoadErrors> {
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
		let hit_program = generator.generate_hit(&graph, parameters).map_err(compile_error)?;

		let hit_shader_id = format!("{}#hit-shader", material.as_ref());
		let hit_compile_error = |error: String| {
			context.error(shader_compilation_error_message(&hit_shader_id, &error));
			LoadErrors::FailedToProcess
		};
		let material_program = MaterialProgram::new(&graph, parameters).map_err(compile_error)?;
		// The two programs don't depend on each other, so they compile together.
		let shader = async {
			let Some(program) = program else {
				return Ok(None);
			};
			let (shader, bytes) = context
				.compile_shader(program, compute_material_stage(&shader_id))
				.await
				.map_err(compile_error)?;
			store_model_owned(context, &shader_id, shader, bytes).await.map(Some)
		};
		let hit_shader = async {
			let Some(program) = hit_program else {
				return Ok(None);
			};
			let (shader, bytes) = context
				.compile_shader(program, closest_hit_material_stage(&hit_shader_id))
				.await
				.map_err(hit_compile_error)?;
			store_model_owned(context, &hit_shader_id, shader, bytes).await.map(Some)
		};
		let (shader, hit_shader) = std::future::join!(shader, hit_shader).await;

		Ok((shader?.into_iter().chain(hit_shader?).collect(), material_program))
	}
}

/// The cutoff a `"Mask"` material without `alpha_cutoff` tests its `opacity` against.
const DEFAULT_ALPHA_CUTOFF: f32 = 0.5;

/// Reads a `.bema` file's `alpha_mode`, `alpha_cutoff`, and `double_sided`, returning the alpha mode and sidedness the
/// file declares. A field the file leaves out keeps the value `alpha_mode` and `double_sided` give, which are a
/// variant's parent's, or a root material's defaults.
fn surface_coverage(asset: &Value, alpha_mode: AlphaMode, double_sided: bool) -> Result<(AlphaMode, bool), String> {
	let cutoff = match asset.get("alpha_cutoff") {
		None => match alpha_mode {
			AlphaMode::Mask(cutoff) => cutoff,
			AlphaMode::Opaque | AlphaMode::Blend => DEFAULT_ALPHA_CUTOFF,
		},
		Some(cutoff) => cutoff
			.as_f64()
			.filter(|cutoff| (0.0..=1.0).contains(cutoff))
			.ok_or_else(|| {
				format!(
					"its `alpha_cutoff` {cutoff} is not a number from 0 to 1. The most likely cause is an opacity written as a percentage or a string. Write the cutoff as a number from 0 to 1, such as 0.5."
				)
			})? as f32,
	};
	let alpha_mode = match asset.get("alpha_mode") {
		None => match alpha_mode {
			AlphaMode::Mask(_) => AlphaMode::Mask(cutoff),
			other => other,
		},
		Some(mode) => match mode.as_str() {
			Some("Opaque") => AlphaMode::Opaque,
			Some("Mask") => AlphaMode::Mask(cutoff),
			Some("Blend") => AlphaMode::Blend,
			_ => {
				return Err(format!(
					"its `alpha_mode` {mode} is not `\"Opaque\"`, `\"Mask\"`, or `\"Blend\"`. The most likely cause is a misspelt mode or the removed `transparency` field's `true`. Set `alpha_mode` to one of the three."
				));
			}
		},
	};
	let double_sided = match asset.get("double_sided") {
		None => double_sided,
		Some(Value::Bool(double_sided)) => *double_sided,
		Some(value) => {
			return Err(format!(
				"its `double_sided` {value} is not `true` or `false`. The most likely cause is a quoted Boolean. Write `true` or `false` without quotes."
			));
		}
	};
	Ok((alpha_mode, double_sided))
}

/// Reads one entry of a `.bema` file's `variables`: the parameter it declares, its data type, and its value.
fn material_variable(variable: &Value) -> Result<(MaterialParameter, (&str, &Value)), String> {
	let name = variable["name"]
		.as_str()
		.ok_or("a variable has no `name`. The most likely cause is a variable written without one.")?;
	let data_type = variable["data_type"].as_str().ok_or_else(|| {
		format!("variable '{name}' has no `data_type`. The most likely cause is a variable written without one.")
	})?;
	// Values are compiled into the program unless the variable asks to change at runtime.
	let dynamic = match variable.get("type").map(Value::as_str) {
		None | Some(Some("Static")) => false,
		Some(Some("Dynamic")) => true,
		Some(_) => {
			return Err(format!(
				"variable '{name}' has a `type` other than `Static` or `Dynamic`. The most likely cause is a misspelt binding."
			));
		}
	};

	let kind = match ParameterKind::from_data_type(data_type, dynamic) {
		Some(kind) => kind,
		None if data_type == "Texture2D" => {
			return Err(format!(
				"texture variable '{name}' is `Dynamic`. The most likely cause is a `type` copied from a value variable; only `float`, `vec3f`, and `vec4f` variables can change at runtime."
			));
		}
		None => {
			return Err(format!(
				"variable '{name}' has the unsupported data type '{data_type}'. The most likely cause is a type other than `float`, `vec3f`, `vec4f`, or `Texture2D`."
			));
		}
	};

	let value = variable
		.get("value")
		.filter(|value| !value.is_null())
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
		matches!(r#type, "bema" | "uber")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		// An assembled shader has no asset: the renderer names it, and the bake gathers it from the stored materials.
		if url.get_asset_type() == "uber" {
			return self.bake_assembled(context, url).await;
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

/// Returns the compute generation settings shared by authored and generated material shaders.
fn compute_material_stage(name: &str) -> ShaderGenerationSettings {
	ShaderGenerationSettings::compute(Extent::line(128)).name(name.to_string())
}

/// Returns the closest-hit generation settings shared by authored and generated materials' hit shaders.
fn closest_hit_material_stage(name: &str) -> ShaderGenerationSettings {
	ShaderGenerationSettings::closest_hit().name(name.to_string())
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
	/// A BEAD sidecar describes the variant itself. It is stored as `id`, the ID the generated material's variant would
	/// have.
	Inline { id: String, binding: InlineBinding },
	/// The importer describes the material as a BEMX graph, which is generated.
	Generated(GeneratedMaterial),
}

impl MaterialSource {
	/// Maps what a sidecar binds a material to into its source, generating the material with `generate` when nothing
	/// binds it. `base_id` is the generated material's ID without the `.variant` suffix, which inline variants share.
	///
	/// `binding` comes from [`material_binding`]; a malformed binding is reported with `container_id` and fails.
	pub(crate) fn new(
		context: BakeContext<'_>,
		container_id: ResourceId<'_>,
		binding: Result<Option<MaterialBinding>, String>,
		base_id: String,
		generate: impl FnOnce(String) -> Result<Self, LoadErrors>,
	) -> Result<Self, LoadErrors> {
		match binding {
			Ok(Some(MaterialBinding::Asset(id))) => Ok(Self::Override(id)),
			Ok(Some(MaterialBinding::Inline(binding))) => Ok(Self::Inline {
				id: format!("{base_id}.variant"),
				binding,
			}),
			Ok(None) => generate(base_id),
			Err(problem) => {
				context.error(format_args!(
					"Invalid material binding in the sidecar settings of '{}': {problem} See {}.",
					container_id.as_ref(),
					online_docs_url(MATERIAL_BINDINGS_DOCS_PATH)
				));
				Err(LoadErrors::FailedToProcess)
			}
		}
	}
}

/// Bounds concurrent BEAD material override bakes.
const OVERRIDE_BAKE_CONCURRENCY: usize = 8;

/// Resolves the variants of a container's materials, in the order given.
///
/// glTF and FBX importers call this once per container. Overrides bake as dependencies, inline variants resolve their
/// files and bake their parents, and the other materials are generated through [`store_generated_materials`]; all three
/// run concurrently. `image_ids` holds the resource ID of each container image, indexed by the materials' `images`.
///
/// Next, reference the returned variants from the container's mesh primitives.
pub(crate) async fn resolve_container_materials(
	context: BakeContext<'_>,
	container_id: ResourceId<'_>,
	image_ids: &[String],
	materials: Vec<MaterialSource>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	/// Which of the three lists below holds each material, in material order.
	#[derive(Clone, Copy)]
	enum Origin {
		Override,
		Inline,
		Generated,
	}

	let mut origins = Vec::with_capacity(materials.len());
	let mut overrides = Vec::new();
	let mut inline = Vec::new();
	let mut generated = Vec::new();

	for material in materials {
		match material {
			MaterialSource::Override(id) => {
				origins.push(Origin::Override);
				overrides.push(id);
			}
			MaterialSource::Inline { id, binding } => {
				origins.push(Origin::Inline);
				inline.push((id, binding));
			}
			MaterialSource::Generated(material) => {
				origins.push(Origin::Generated);
				generated.push(material);
			}
		}
	}

	let inline_requests = inline
		.iter()
		.map(|(id, binding)| bake_inline_variant(context, container_id, id, binding));

	let (overridden, inline, generated) = std::future::join!(
		context.bake_dependencies::<VariantModel>(&overrides, OVERRIDE_BAKE_CONCURRENCY),
		buffered_in_order(inline_requests, OVERRIDE_BAKE_CONCURRENCY),
		store_generated_materials(context, container_id, image_ids, generated),
	)
	.await;

	let (mut overridden, mut inline, mut generated) = (overridden?.into_iter(), inline?.into_iter(), generated?.into_iter());

	// Put the variants back in material order.
	origins
		.into_iter()
		.map(|origin| match origin {
			Origin::Override => overridden.next(),
			Origin::Inline => inline.next(),
			Origin::Generated => generated.next(),
		})
		.collect::<Option<Vec<_>>>()
		.ok_or(LoadErrors::FailedToProcess)
}

/// Generates and stores the variants of a container's generated materials, in the order given.
///
/// Every image the materials sample bakes once, as its own dependency, while materials whose graphs match share one
/// compiled shader.
async fn store_generated_materials(
	context: BakeContext<'_>,
	container_id: ResourceId<'_>,
	image_ids: &[String],
	materials: Vec<GeneratedMaterial>,
) -> Result<Vec<ReferenceModel<VariantModel>>, LoadErrors> {
	if materials.is_empty() {
		return Ok(Vec::new());
	}

	let generator = context.program_generator()?;

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

	for (generated, (shaders, program)) in materials.into_iter().zip(shaders) {
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
					dynamic: false,
				}
			})
			.collect();

		variants.push(store_generated_variant(context, generated, shaders, program, variables).await?);
	}

	Ok(variants)
}

/// Stores one generated material, which evaluates with `shaders` and keeps `program` for the assembled shaders, and its
/// variant.
async fn store_generated_variant(
	context: BakeContext<'_>,
	GeneratedMaterial { base_id, material }: GeneratedMaterial,
	shaders: Vec<ReferenceModel<Shader>>,
	program: MaterialProgram,
	variables: Vec<VariantVariableModel>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let (alpha_mode, double_sided) = (material.alpha_mode, material.double_sided);

	let model = MaterialModel {
		double_sided,
		alpha_mode,
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders,
		parameters: Vec::new(),
		program,
	};

	let model = store_model::<MaterialModel>(context, &format!("{base_id}.material"), model, &[]).await?;

	let variant = VariantModel {
		material: model,
		variables,
		alpha_mode,
		double_sided,
	};

	store_model::<VariantModel>(context, &format!("{base_id}.variant"), variant, &[]).await
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

/// Compiles each distinct generated shader once and stores it under an ID derived from its compile inputs, with the
/// closest-hit shader beside it when the generator builds one.
///
/// Returns the shader references of each entry in `materials`, in order: its shader, if the generator builds one, then
/// its hit shader, if any, and the [`MaterialProgram`] the assembled shaders read.
///
/// The ID hashes the lowered platform source and the compiler identity, so a later bake of the same container reuses
/// the stored binary while a change to the graph, program generator, BESL backend, settings, or toolchain compiles
/// again.
async fn store_generated_shaders(
	context: BakeContext<'_>,
	generator: &dyn ProgramGenerator,
	container_id: ResourceId<'_>,
	materials: &[&ImportedMaterial],
) -> Result<Vec<(Vec<ReferenceModel<Shader>>, MaterialProgram)>, LoadErrors> {
	// Every generated shader of a container shares one compiler name, so the name never depends on the key it feeds.
	let compiler_name = format!("{}#shaders", container_id.as_ref());

	let compile_error = |error: String| {
		context.error(shader_compilation_error_message(&compiler_name, &error));
		LoadErrors::FailedToProcess
	};

	let compiler_identity = PlatformShaderCompiler::compiler_identity().await.map_err(compile_error)?;

	// Generated materials evaluate in the same compute stage as authored material shaders, and are hit the same way.
	let settings = compute_material_stage(&compiler_name);
	let hit_settings = closest_hit_material_stage(&compiler_name);

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
				// Every material keeps its program for the shaders assembled after the bake. Without an evaluation shader,
				// equal programs are what make two materials share an entry.
				let material_program = MaterialProgram::new(&material.graph, &parameters).map_err(compile_error)?;
				let (prepared, key) = match generator.generate(&material.graph, &parameters).map_err(compile_error)? {
					Some(program) => {
						let prepared = context.prepare_shader(program, &settings).map_err(compile_error)?;
						let key = prepared.cache_key(compiler_identity);
						(Some(prepared), key)
					}
					None => (None, material_program.identity),
				};
				// Graphs that lower to the same evaluation source compute the same surface, so their hit shaders match too;
				// the hit shader is generated from the same graph and parameters.
				let hit = generator
					.generate_hit(&material.graph, &parameters)
					.map_err(compile_error)?
					.map(|program| context.prepare_shader(program, &hit_settings))
					.transpose()
					.map_err(compile_error)?;

				*entry.insert(*unique_by_key.entry(key).or_insert_with(|| {
					unique.push((key, prepared, hit, material_program));
					unique.len() - 1
				}))
			}
		};

		unique_index_per_material.push(index);
	}

	let requests = unique.into_iter().map(|(key, prepared, hit, program)| async move {
		let mut shaders = Vec::with_capacity(2);
		if let Some(prepared) = prepared {
			let shader_id = format!("{}#shaders/{key:016x}", container_id.as_ref());
			shaders.push(store_generated_shader(context, &shader_id, prepared).await?);
		}
		if let Some(hit) = hit {
			let hit_key = hit.cache_key(compiler_identity);
			let hit_shader_id = format!("{}#hit-shaders/{hit_key:016x}", container_id.as_ref());
			shaders.push(store_generated_shader(context, &hit_shader_id, hit).await?);
		}
		Ok((shaders, program))
	});

	// Ordered buffering keeps the stored shader order deterministic while platform compiler processes overlap.
	let unique_shaders = buffered_in_order(requests, GENERATED_SHADER_COMPILE_CONCURRENCY).await?;

	Ok(unique_index_per_material
		.into_iter()
		.map(|index| unique_shaders[index].clone())
		.collect())
}

/// Compiles and stores one generated shader under `shader_id`, unless a stored one can be reused.
async fn store_generated_shader(
	context: BakeContext<'_>,
	shader_id: &str,
	prepared: PreparedBeslShader,
) -> Result<ReferenceModel<Shader>, LoadErrors> {
	// A stored shader under this key came from the same platform source and compiler, so compiling again cannot
	// change it. This is what keeps iterating on a mesh from waiting on the platform compiler.
	if let Some(stored) = context.reusable_resource(ResourceId::new(shader_id)).await {
		return Ok(stored.into());
	}

	let (shader, shader_bytes) = context.compile_prepared(shader_id, prepared).await.map_err(|error| {
		context.error(shader_compilation_error_message(shader_id, &error));
		LoadErrors::FailedToProcess
	})?;

	store_model_owned::<Shader, _>(context, shader_id, shader, shader_bytes).await
}

/// Fails a material of `alpha_mode` whose graph `reads_backdrop`, the scene behind its surface, unless it blends: only
/// the transparent phase has the opaque scene drawn behind its surfaces.
fn backdrop_needs_blending(reads_backdrop: bool, alpha_mode: AlphaMode) -> Result<(), String> {
	if !reads_backdrop || matches!(alpha_mode, AlphaMode::Blend) {
		return Ok(());
	}

	Err("its graph reads `scenecolor` or `sceneposition`, but its `alpha_mode` isn't `\"Blend\"`. The most likely cause is a water or glass graph given to an opaque material, or a variant that turns a blended parent opaque. Set `alpha_mode` to `\"Blend\"`.".to_string())
}

/// The `VariantSource` enum is where a variant [`variant_model`] builds is written, which sets the rules its values
/// follow.
#[derive(Clone, Copy, PartialEq)]
enum VariantSource {
	/// A `.bema` variant file, which must list every static parameter.
	File,
	/// An inline variant in a model sidecar. A static parameter without a value keeps the parent's, and a string is
	/// always an asset ID, so it is only a value for a `Texture2D` parameter.
	Inline,
}

/// Builds the variant of `material` whose values `value_of` gives by parameter name, and whose coverage fields `asset`
/// holds, as `.bema` variant files and inline sidecar variants both describe them.
///
/// A dynamic parameter without a value keeps the parent's default; a static one follows the rules of `source`.
/// Problems are reported through `invalid`.
async fn variant_model<'v>(
	context: BakeContext<'_>,
	material: ReferenceModel<MaterialModel>,
	value_of: &dyn Fn(&str) -> Option<&'v Value>,
	asset: &Value,
	source: VariantSource,
	invalid: &dyn Fn(&str) -> LoadErrors,
) -> Result<VariantModel, LoadErrors> {
	let material_repr: MaterialModel = crate::from_slice(&material.resource).map_err(|_| LoadErrors::FailedToProcess)?;
	let value_requests = material_repr.parameters.iter().map(|parameter| async move {
		let value = value_of(&parameter.name);
		if value.is_none() && (parameter.dynamic || source == VariantSource::Inline) {
			return Ok(parameter.value.clone());
		}
		if source == VariantSource::Inline && parameter.r#type != "Texture2D" && value.is_some_and(Value::is_string) {
			return Err(invalid(&format!(
				"its variable '{}' names a file, but the parent's parameter is a `{}`, not a `Texture2D`. The most likely cause is a texture bound to a value parameter, or a named color, which inline variants read as a file name. Write the value as a number or an array of numbers.",
				parameter.name, parameter.r#type
			)));
		}
		let value = value.ok_or_else(|| {
			invalid(&format!(
				"it gives no `value` for the parent's static variable '{}'. The most likely cause is a variable left out of the variant or written without a value.",
				parameter.name
			))
		})?;
		resolve_value(context, &parameter.r#type, value, invalid).await
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
			dynamic: parameter.dynamic,
		})
		.collect();
	// A variant keeps each of its material's coverage settings it leaves out.
	let (alpha_mode, double_sided) =
		surface_coverage(asset, material_repr.alpha_mode, material_repr.double_sided).map_err(|problem| invalid(&problem))?;
	backdrop_needs_blending(material_repr.program.reads_backdrop, alpha_mode).map_err(|problem| invalid(&problem))?;

	Ok(VariantModel {
		material,
		variables,
		alpha_mode,
		double_sided,
	})
}

/// Resolves an inline sidecar variant and stores it as `id`: looks up each variable's candidate files, picks the first
/// `select` rule whose required variables resolved, and builds the variant of that rule's parent.
///
/// Every candidate it looks up becomes a bake dependency, present or missing, so adding an earlier candidate later
/// rebakes the container. Variables that don't resolve keep the parent's value.
async fn bake_inline_variant(
	context: BakeContext<'_>,
	container_id: ResourceId<'_>,
	id: &str,
	binding: &InlineBinding,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let mut resolved = Vec::with_capacity(binding.variables.len());

	for (name, value) in &binding.variables {
		let value = match value {
			BindingValue::Literal(value) => Some(value.clone()),
			BindingValue::Candidates(candidates) => {
				let mut chosen = None;
				for candidate in candidates {
					if context.source_exists(ResourceId::new(candidate)).await? {
						chosen = Some(Value::String(candidate.clone()));
						break;
					}
				}
				log::trace!("Material '{id}' variable '{name}' resolved to {chosen:?} from {candidates:?}.");
				chosen
			}
		};
		resolved.push((name.as_str(), value));
	}

	let is_resolved = |name: &str| resolved.iter().any(|(variable, value)| *variable == name && value.is_some());

	let Some(rule) = binding
		.rules
		.iter()
		.find(|rule| rule.requires.iter().all(|name| is_resolved(name)))
	else {
		context.error(format_args!(
			"No `select` rule of `asset.\"{}\"` matches material '{id}' of '{}'. The most likely cause is that every rule `requires` a variable whose files don't exist next to the model. Add a last rule without `requires`. See {}.",
			binding.key,
			container_id.as_ref(),
			online_docs_url(MATERIAL_BINDINGS_DOCS_PATH)
		));
		return Err(LoadErrors::FailedToProcess);
	};

	let material = context.bake_dependency(&rule.parent).await?;
	let value_of = |name: &str| {
		resolved
			.iter()
			.find(|(variable, _)| *variable == name)
			.and_then(|(_, value)| value.as_ref())
	};
	let invalid = |problem: &str| {
		context.error(format_args!(
			"Invalid inline variant `asset.\"{}\"` for material '{id}' of '{}': {problem} See {}.",
			binding.key,
			container_id.as_ref(),
			online_docs_url(MATERIAL_BINDINGS_DOCS_PATH)
		));
		LoadErrors::FailedToProcess
	};
	let variant = variant_model(context, material, &value_of, &rule.coverage, VariantSource::Inline, &invalid).await?;

	store_model::<VariantModel>(context, id, variant, &[]).await
}

/// Reports why a `.bema` file of the given `kind`, such as `"material"`, can't bake, and returns the load error.
fn report_invalid(context: BakeContext<'_>, kind: &str, url: ResourceId<'_>, problem: &str) -> LoadErrors {
	context.error(format_args!(
		"Invalid {kind} '{}': {problem} See {}.",
		url.as_ref(),
		online_docs_url(BEMA_DOCS_PATH)
	));
	LoadErrors::FailedToProcess
}

/// Resolves a material parameter value based on its type, reporting through `invalid` a value that isn't one of
/// `data_type`.
async fn resolve_value(
	context: BakeContext<'_>,
	data_type: &str,
	value: &Value,
	invalid: &dyn Fn(&str) -> LoadErrors,
) -> Result<ValueModel, LoadErrors> {
	let value = match data_type {
		"vec4f" => vector::<4>(value).map(ValueModel::Vector4),
		"vec3f" => vector::<3>(value).map(ValueModel::Vector3),
		"float" => value.as_f64().map(|value| ValueModel::Scalar(value as f32)).ok_or_else(|| {
			format!("the `float` value {value} is not a number. The most likely cause is a quoted number or a color name.")
		}),
		"Texture2D" => match value.as_str() {
			Some(image) => Ok(ValueModel::Image(context.bake_dependency(image).await?)),
			None => Err(format!(
				"the `Texture2D` value {value} is not an asset ID. The most likely cause is a value written for another data type."
			)),
		},
		data_type => Err(format!("the data type '{data_type}' has no values.")),
	};

	value.map_err(|problem| invalid(&problem))
}

/// Reads a `vec3f` or `vec4f` value: a named color or an array of `N` numbers.
fn vector<const N: usize>(value: &Value) -> Result<[f32; N], String> {
	if let Some(name) = value.as_str() {
		let color = match name {
			"Red" => [1f32, 0f32, 0f32, 1f32],
			"Green" => [0f32, 1f32, 0f32, 1f32],
			"Blue" => [0f32, 0f32, 1f32, 1f32],
			"Purple" => [1f32, 0f32, 1f32, 1f32],
			"White" => [1f32, 1f32, 1f32, 1f32],
			"Black" => [0f32, 0f32, 0f32, 1f32],
			_ => [1f32, 0f32, 1f32, 1f32],
		};

		return Ok(std::array::from_fn(|index| color[index]));
	}

	let components = value
		.as_array()
		.filter(|components| components.len() == N)
		.and_then(|components| components.iter().map(Value::as_f64).collect::<Option<Vec<_>>>())
		.ok_or_else(|| {
			format!(
				"the value {value} is neither a color name nor an array of {N} numbers. The most likely cause is an array with the wrong number of components."
			)
		})?;

	Ok(std::array::from_fn(|index| components[index] as f32))
}

const BEMA_DOCS_PATH: &str = "reference/bema";

#[cfg(test)]
pub mod tests {

	use std::sync::Arc;

	use super::{MaterialParameter, ProgramGenerator};
	use crate::asset::handler::implementations::besl::{PreparedBeslShader, ShaderCompiler};
	use crate::bemx;
	use crate::types::AlphaMode;
	use crate::{
		ReferenceModel,
		asset::{
			ResourceId, handler::AssetHandler, handler::implementations::bema::BEMAAssetHandler, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		resource::storage_backend::MemoryStorageBackend,
		resources::material::{
			Binding, BindingKind, MaterialModel, SetInterface, Shader, ShaderArtifact, ShaderInterface, ValueModel,
			VariantModel,
		},
		shader::{besl::backends::msl::MSL_ENTRY_POINT, generator::ShaderGenerationSettings},
	};

	struct TestShaderCompiler;

	impl ShaderCompiler for TestShaderCompiler {
		fn compile<'a>(
			&'a self,
			shader: PreparedBeslShader,
			id: &'a str,
		) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), String>> {
			Box::pin(async move {
				Ok((
					Shader {
						id: id.to_string(),
						stage: shader.stage(),
						interface: ShaderInterface {
							workgroup_size: Some((128, 0, 0)),
							sets: vec![SetInterface::with_bindings(
								"material",
								vec![Binding::new(0, BindingKind::StorageBuffer, 1, Some(4), true, false)],
							)],
							ray_payload: None,
							tile: None,
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
		fn generate<'a>(
			&self,
			_: &'a bemx::Graph,
			_: &'a [MaterialParameter],
		) -> Result<Option<besl::parser::Node<'a>>, String> {
			Ok(Some(besl::parser::Node::root_with_children(vec![
				besl::parser::Node::main_function(Vec::new()),
			])))
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

		let resource_storage_backend = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.set_program_generator(MinimalTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);

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

	/// A dynamic variable keeps its flag through the material and its variants, its value is the default, and a variant
	/// that leaves it out inherits that default.
	#[r#async::test]
	async fn dynamic_variables_bake_their_defaults() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file(
			"dynamic_material.bema",
			r#"{
				"domain": "World",
				"graph": "dynamic.bemx",
				"variables": [
					{ "name": "tint", "data_type": "vec4f", "type": "Dynamic", "value": [0.5, 0.25, 1.0, 1.0] },
					{ "name": "glow", "data_type": "float", "type": "Dynamic", "value": 2 },
					{ "name": "roughness", "data_type": "float", "type": "Static", "value": 0.75 }
				]
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"dynamic.bemx",
			r#"{
				nodes: [
					{ name: "tint", node: "parameter", parameter: "tint", type: "vector4" },
					{ name: "rgb", node: "swizzle", in: "tint", channels: "rgb" },
					{ name: "surface", node: "surface", base_color: "rgb" },
				],
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"dynamic_variant.bema",
			r#"{
				"parent": "dynamic_material.bema",
				"variables": [
					{ "name": "tint", "value": "White" },
					{ "name": "roughness", "value": 0.25 }
				]
			}"#
			.as_bytes(),
		);

		let resource_storage_backend = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.set_program_generator(MinimalTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);

		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("dynamic_variant.bema")
			.await
			.expect("Failed to bake a material with dynamic variables");

		let material = resource_storage_backend
			.get_resource(ResourceId::new("dynamic_material.bema"))
			.expect("Expected material");
		let material: MaterialModel = crate::from_slice(&material.resource).expect("Expected a material model");
		let parameters = material
			.parameters
			.iter()
			.map(|parameter| (parameter.name.as_str(), parameter.dynamic, parameter.value.clone()))
			.collect::<Vec<_>>();
		assert!(matches!(
			parameters.as_slice(),
			[
				("tint", true, ValueModel::Vector4([0.5, 0.25, 1.0, 1.0])),
				("glow", true, ValueModel::Scalar(2.0)),
				("roughness", false, ValueModel::Scalar(0.75)),
			]
		));

		let variant = resource_storage_backend
			.get_resource(ResourceId::new("dynamic_variant.bema"))
			.expect("Expected variant");
		let variant: VariantModel = crate::from_slice(&variant.resource).expect("Expected a variant model");
		let variables = variant
			.variables
			.iter()
			.map(|variable| (variable.name.as_str(), variable.dynamic, variable.value.clone()))
			.collect::<Vec<_>>();
		assert!(matches!(
			variables.as_slice(),
			[
				("tint", true, ValueModel::Vector4([1.0, 1.0, 1.0, 1.0])),
				("glow", true, ValueModel::Scalar(2.0)),
				("roughness", false, ValueModel::Scalar(0.25)),
			]
		));
	}

	/// The `HitTestShaderGenerator` struct builds a hit program beside every material program, as a renderer that
	/// traces rays does.
	struct HitTestShaderGenerator;

	/// The smallest closest-hit program: it returns the payload it receives.
	const HIT_PROGRAM: &str =
		"SurfaceHit: struct { distance: f32 }\nmain: fn (payload: SurfaceHit) -> SurfaceHit { return payload; }";

	impl ProgramGenerator for HitTestShaderGenerator {
		fn generate<'a>(
			&self,
			graph: &'a bemx::Graph,
			parameters: &'a [MaterialParameter],
		) -> Result<Option<besl::parser::Node<'a>>, String> {
			MinimalTestShaderGenerator.generate(graph, parameters)
		}

		fn generate_hit<'a>(
			&self,
			_: &'a bemx::Graph,
			_: &'a [MaterialParameter],
		) -> Result<Option<besl::parser::Node<'a>>, String> {
			besl::parse(HIT_PROGRAM).map(Some).map_err(|error| format!("{error:?}"))
		}
	}

	/// A generator that builds hit programs gives each material a closest-hit shader after its compute shader, stored
	/// under the material's ID with a `#hit-shader` fragment.
	#[r#async::test]
	async fn materials_store_a_hit_shader_when_the_generator_builds_one() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"hit_material.bema",
			r#"{ "domain": "World", "graph": "hit.bemx", "variables": [] }"#.as_bytes(),
		);
		asset_storage_backend.add_file("hit.bemx", r#"{ nodes: [{ name: "surface", node: "surface" }] }"#.as_bytes());
		let resource_storage_backend = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.set_program_generator(HitTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);

		let _: ReferenceModel<MaterialModel> = asset_manager
			.bake_if_not_exists("hit_material.bema")
			.await
			.expect("Failed to bake a material with a hit shader");

		let material = resource_storage_backend
			.get_resource(ResourceId::new("hit_material.bema"))
			.expect("Expected material");
		let material: MaterialModel = crate::from_slice(&material.resource).expect("Expected a material model");
		let shaders = material
			.shaders
			.iter()
			.map(|shader| shader.id().as_ref().to_string())
			.collect::<Vec<_>>();
		assert_eq!(shaders, ["hit_material.bema#shader", "hit_material.bema#hit-shader"]);
		let hit_shader = resource_storage_backend
			.get_resource(ResourceId::new("hit_material.bema#hit-shader"))
			.expect("Expected the hit shader");
		let hit_shader: Shader = crate::from_slice(&hit_shader.resource).expect("Expected a shader model");
		assert_eq!(hit_shader.stage, crate::types::ShaderTypes::ClosestHit);
	}

	/// The `RecordingInterceptor` struct collects the bake products whose names `wants` accepts.
	struct RecordingInterceptor {
		wants: fn(&str) -> bool,
		products: std::sync::Mutex<Vec<(String, String)>>,
	}

	impl crate::asset::intercept::BakeInterceptor for RecordingInterceptor {
		fn wants(&self, name: &str) -> bool {
			(self.wants)(name)
		}

		fn receive(&self, name: String, content: Vec<u8>) {
			let content = String::from_utf8(content).expect("BESL products are UTF-8 text.");
			self.products.lock().unwrap().push((name, content));
		}
	}

	/// Bakes `hit_material.bema` with a generator that builds hit programs, and returns the products the bake emitted
	/// whose names `wants` accepts, sorted by name.
	async fn intercepted_material_products(wants: fn(&str) -> bool) -> Vec<(String, String)> {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"hit_material.bema",
			r#"{ "domain": "World", "graph": "hit.bemx", "variables": [] }"#.as_bytes(),
		);
		asset_storage_backend.add_file("hit.bemx", r#"{ nodes: [{ name: "surface", node: "surface" }] }"#.as_bytes());
		let interceptor = std::sync::Arc::new(RecordingInterceptor {
			wants,
			products: std::sync::Mutex::new(Vec::new()),
		});
		let mut asset_manager = AssetManager::new(asset_storage_backend, MemoryStorageBackend::new());
		asset_manager.set_program_generator(HitTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.set_interceptor(interceptor.clone());
		asset_manager.add_asset_handler(BEMAAssetHandler);

		let _: ReferenceModel<MaterialModel> = asset_manager
			.bake_if_not_exists("hit_material.bema")
			.await
			.expect("Failed to bake an intercepted material");

		let mut products = std::mem::take(&mut *interceptor.products.lock().unwrap());
		products.sort();
		products
	}

	/// An intercepted bake emits the BESL source of each shader it compiles, named after the shader with `.besl`, as
	/// the program generator built it.
	#[r#async::test]
	async fn an_intercepted_bake_emits_the_besl_of_every_shader_it_compiles() {
		let products = intercepted_material_products(|name| name.ends_with(".besl")).await;

		let hit = besl::write(&besl::parse(HIT_PROGRAM).unwrap());
		assert_eq!(
			products,
			[
				("hit_material.bema#hit-shader.besl".to_string(), hit),
				(
					"hit_material.bema#shader.besl".to_string(),
					"main: fn () -> void {\n}\n".to_string()
				),
			]
		);
	}

	/// An intercepted bake emits the platform source each shader lowered to, named after the shader with the
	/// platform language's extension, such as `.msl`.
	#[r#async::test]
	async fn an_intercepted_bake_emits_the_platform_source_of_every_shader_it_compiles() {
		let extension = crate::shader::besl::backends::platform::PlatformShaderLanguage::current_platform().extension();
		let products = intercepted_material_products(|name| {
			name.rsplit_once('.').map(|(_, extension)| extension)
				== Some(crate::shader::besl::backends::platform::PlatformShaderLanguage::current_platform().extension())
		})
		.await;

		let names = products.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
		assert_eq!(
			names,
			[
				format!("hit_material.bema#hit-shader.{extension}"),
				format!("hit_material.bema#shader.{extension}")
			]
		);
		assert!(products.iter().all(|(_, source)| !source.trim().is_empty()));
	}

	/// An interceptor receives only the products it wants.
	#[r#async::test]
	async fn an_interceptor_receives_only_the_products_it_wants() {
		assert_eq!(intercepted_material_products(|_| false).await, Vec::new());
	}

	/// A texture can't change at runtime, so a material that declares one dynamic fails to bake.
	#[r#async::test]
	async fn a_dynamic_texture_fails_to_bake() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"dynamic_texture.bema",
			r#"{
				"domain": "World",
				"graph": "dynamic_texture.bemx",
				"variables": [
					{ "name": "albedo", "data_type": "Texture2D", "type": "Dynamic", "value": "albedo.png" }
				]
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"dynamic_texture.bemx",
			r#"{ nodes: [{ name: "surface", node: "surface" }] }"#.as_bytes(),
		);
		let mut asset_manager = AssetManager::new(asset_storage_backend, MemoryStorageBackend::new());
		asset_manager.set_program_generator(MinimalTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);

		let baked: Result<ReferenceModel<MaterialModel>, _> = asset_manager.bake_if_not_exists("dynamic_texture.bema").await;

		assert!(baked.is_err(), "A dynamic texture variable should fail the bake.");
	}

	/// The `AssemblingTestGenerator` struct assembles one shader from every material in the store, keyed by each
	/// material's program and static values, or by one key for all of them when `colliding` is set.
	struct AssemblingTestGenerator {
		colliding: bool,
	}

	impl ProgramGenerator for AssemblingTestGenerator {
		fn generate<'a>(
			&self,
			_: &'a bemx::Graph,
			_: &'a [MaterialParameter],
		) -> Result<Option<besl::parser::Node<'a>>, String> {
			Ok(None)
		}

		fn assembled_key(&self, _: &str, material: &super::AssembledMaterial<'_>) -> Result<Option<u32>, String> {
			if self.colliding {
				return Ok(Some(7));
			}
			let statics = material.static_values.iter().flatten();
			Ok(Some(statics.fold(material.identity as u32, |key, value| {
				key.rotate_left(5) ^ value.to_bits()
			})))
		}

		fn generate_assembled<'a>(
			&self,
			_: &str,
			_: &'a [super::AssembledArm<'a>],
		) -> Result<(besl::parser::Node<'a>, ShaderGenerationSettings), String> {
			Ok((
				besl::parser::Node::root_with_children(vec![besl::parser::Node::main_function(Vec::new())]),
				ShaderGenerationSettings::compute(utils::Extent::square(8)).name("assembled".to_string()),
			))
		}
	}

	/// Returns an asset manager over a material with one static color, and a variant of it per entry of `colors`,
	/// stored as `variant_<index>.bema`.
	fn assembling_asset_manager(colors: &[&str], colliding: bool) -> (AssetManager, MemoryStorageBackend) {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"assembled_material.bema",
			r#"{
				"domain": "World",
				"graph": "assembled.bemx",
				"variables": [{ "name": "color", "data_type": "vec4f", "type": "Static", "value": "White" }]
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"assembled.bemx",
			r#"{
				nodes: [
					{ name: "color", node: "parameter", parameter: "color", type: "vector4" },
					{ name: "rgb", node: "swizzle", in: "color", channels: "rgb" },
					{ name: "surface", node: "surface", base_color: "rgb" },
				],
			}"#
			.as_bytes(),
		);
		for (index, color) in colors.iter().enumerate() {
			// The test storage keeps file names for as long as the test runs.
			asset_storage_backend.add_file(
				format!("variant_{index}.bema").leak(),
				format!(r#"{{ "parent": "assembled_material.bema", "variables": [{{ "name": "color", "value": {color} }}] }}"#)
					.as_bytes(),
			);
		}
		let resource_storage_backend = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.set_program_generator(AssemblingTestGenerator { colliding });
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);
		(asset_manager, resource_storage_backend)
	}

	/// Bakes the variants `asset_manager` holds, from `variant_0.bema` to `variant_<count - 1>.bema`.
	async fn bake_variants(asset_manager: &AssetManager, count: usize) {
		for index in 0..count {
			let _: ReferenceModel<VariantModel> = asset_manager
				.bake_if_not_exists(&format!("variant_{index}.bema"))
				.await
				.expect("Failed to bake a variant of the assembled material");
		}
	}

	/// Returns the arm keys of the assembled shader `shaders.uber` in `resource_storage_backend`.
	fn assembled_keys(resource_storage_backend: &MemoryStorageBackend) -> Vec<u32> {
		let assembled = resource_storage_backend
			.get_resource(ResourceId::new("shaders.uber"))
			.expect("Expected the assembled shader");
		assert_eq!(assembled.class, "AssembledShader");
		crate::from_slice::<super::AssembledShader>(&assembled.resource)
			.expect("Expected an assembled shader model")
			.keys
	}

	/// An assembled shader holds one arm for every distinct material in the store, with no asset behind it, and a
	/// material baked later joins it on the next bake.
	#[r#async::test]
	async fn assembled_shaders_hold_one_arm_per_distinct_material() {
		let colors = ["\"White\"", "[0.5, 0.25, 1.0, 1.0]", "\"White\"", "[0.0, 1.0, 0.0, 1.0]"];
		let (asset_manager, resource_storage_backend) = assembling_asset_manager(&colors, false);

		bake_variants(&asset_manager, 3).await;
		asset_manager
			.bake("shaders.uber")
			.await
			.expect("Failed to bake the assembled shader");
		let keys = assembled_keys(&resource_storage_backend);
		assert_eq!(
			keys.len(),
			2,
			"two variants share their static values, so they share an arm: {keys:?}"
		);
		assert!(keys.is_sorted(), "keys: {keys:?}");

		bake_variants(&asset_manager, 4).await;
		asset_manager
			.bake("shaders.uber")
			.await
			.expect("Failed to bake the assembled shader again");
		let grown = assembled_keys(&resource_storage_backend);
		assert_eq!(grown.len(), 3, "keys: {grown:?}");
		assert!(keys.iter().all(|key| grown.contains(key)), "keys: {keys:?}, grown: {grown:?}");
	}

	/// A renderer that assembles its evaluation shaders keys their arms by the stored variant's static values, and finds
	/// a loaded material's arm by the solved variant's, so both list the same values in the same order.
	#[r#async::test]
	async fn solved_variants_keep_their_static_values() {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"statics_material.bema",
			r#"{
				"domain": "World",
				"graph": "statics.bemx",
				"variables": [
					{ "name": "tint", "data_type": "vec4f", "type": "Dynamic", "value": [0.5, 0.25, 1.0, 1.0] },
					{ "name": "roughness", "data_type": "float", "type": "Static", "value": 0.75 },
					{ "name": "color", "data_type": "vec4f", "type": "Static", "value": "White" }
				]
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"statics.bemx",
			r#"{
				nodes: [
					{ name: "color", node: "parameter", parameter: "color", type: "vector4" },
					{ name: "rgb", node: "swizzle", in: "color", channels: "rgb" },
					{ name: "surface", node: "surface", base_color: "rgb" },
				],
			}"#
			.as_bytes(),
		);
		asset_storage_backend.add_file(
			"statics_variant.bema",
			r#"{
				"parent": "statics_material.bema",
				"variables": [{ "name": "roughness", "value": 0.25 }, { "name": "color", "value": [0.0, 0.5, 1.0, 1.0] }]
			}"#
			.as_bytes(),
		);
		let resource_storage_backend = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.set_program_generator(AssemblingTestGenerator { colliding: false });
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);
		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("statics_variant.bema")
			.await
			.expect("Failed to bake the variant");

		let stored = resource_storage_backend
			.get_resource(ResourceId::new("statics_variant.bema"))
			.expect("Expected the variant");
		let stored: VariantModel = crate::from_slice(&stored.resource).expect("Expected a variant model");
		let solved: crate::Reference<crate::resources::material::Variant> =
			crate::resource::resource_manager::ResourceManager::new(resource_storage_backend)
				.request("statics_variant.bema")
				.await
				.expect("Failed to request the variant");

		let stored = stored.static_values().collect::<Vec<_>>();
		assert_eq!(stored, [&[0.25][..], &[0.0, 0.5, 1.0, 1.0][..]]);
		assert_eq!(solved.resource().static_values().collect::<Vec<_>>(), stored);
	}

	/// The graph of [`coverage_asset_manager`]'s material: an `opacity` read from its `strength` parameter.
	const COVERAGE_GRAPH: &str = r#"{
		nodes: [
			{ name: "strength", node: "parameter", parameter: "strength", type: "float" },
			{ name: "surface", node: "surface", opacity: "strength" },
		],
	}"#;

	/// Returns an asset manager over a foliage material whose `opacity` reads its `strength` parameter, declared with
	/// `material_fields`, and over each of `variants`, a name and the fields after its `parent` and `variables`.
	fn coverage_asset_manager(material_fields: &str, variants: &[(&str, &str)]) -> (AssetManager, MemoryStorageBackend) {
		coverage_asset_manager_with_graph(COVERAGE_GRAPH, material_fields, variants)
	}

	/// Returns [`coverage_asset_manager`]'s asset manager with `graph` as the material's graph.
	fn coverage_asset_manager_with_graph(
		graph: &'static str,
		material_fields: &str,
		variants: &[(&str, &str)],
	) -> (AssetManager, MemoryStorageBackend) {
		let asset_storage_backend = AssetTestStorageBackend::new();
		asset_storage_backend.add_file(
			"foliage.bema",
			format!(
				r#"{{
					"domain": "World",
					"graph": "foliage.bemx",
					{material_fields}
					"variables": [{{ "name": "strength", "data_type": "float", "type": "Static", "value": 1.0 }}]
				}}"#
			)
			.leak()
			.as_bytes(),
		);
		asset_storage_backend.add_file("foliage.bemx", graph.as_bytes());
		for (name, fields) in variants {
			asset_storage_backend.add_file(
				format!("{name}.bema").leak(),
				format!(r#"{{ "parent": "foliage.bema", "variables": [{{ "name": "strength", "value": 0.5 }}] {fields} }}"#)
					.leak()
					.as_bytes(),
			);
		}
		let resource_storage_backend = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());
		asset_manager.set_program_generator(MinimalTestShaderGenerator);
		asset_manager.set_shader_compiler(TestShaderCompiler);
		asset_manager.add_asset_handler(BEMAAssetHandler);
		(asset_manager, resource_storage_backend)
	}

	/// Reads the stored variant `id` from `resource_storage_backend`.
	fn stored_variant(resource_storage_backend: &MemoryStorageBackend, id: &str) -> VariantModel {
		let stored = resource_storage_backend
			.get_resource(ResourceId::new(id))
			.expect("Expected the variant");
		crate::from_slice(&stored.resource).expect("Expected a variant model")
	}

	/// A material declares its alpha mode, cutoff, and sidedness, and a variant keeps each one it leaves out and
	/// replaces each one it sets.
	#[r#async::test]
	async fn variants_inherit_and_override_alpha_mode_and_sidedness() {
		let (asset_manager, resource_storage_backend) = coverage_asset_manager(
			r#""alpha_mode": "Mask", "alpha_cutoff": 0.3, "double_sided": true,"#,
			&[
				("inherits", ""),
				("sharper", r#", "alpha_cutoff": 0.7"#),
				("solid", r#", "alpha_mode": "Opaque", "double_sided": false"#),
			],
		);

		let material: ReferenceModel<MaterialModel> = asset_manager
			.bake_if_not_exists("foliage.bema")
			.await
			.expect("Failed to bake the material");
		let material: MaterialModel = crate::from_slice(
			&resource_storage_backend
				.get_resource(material.id())
				.expect("Expected the material")
				.resource,
		)
		.expect("Expected a material model");
		assert_eq!((material.alpha_mode, material.double_sided), (AlphaMode::Mask(0.3), true));

		for (name, alpha_mode, double_sided) in [
			("inherits", AlphaMode::Mask(0.3), true),
			("sharper", AlphaMode::Mask(0.7), true),
			("solid", AlphaMode::Opaque, false),
		] {
			let id = format!("{name}.bema");
			let _: ReferenceModel<VariantModel> = asset_manager
				.bake_if_not_exists(&id)
				.await
				.unwrap_or_else(|error| panic!("Failed to bake variant {name}: {error:?}"));
			let variant = stored_variant(&resource_storage_backend, &id);
			assert_eq!(
				(variant.alpha_mode, variant.double_sided),
				(alpha_mode, double_sided),
				"variant {name}"
			);
		}
	}

	/// A root material without the fields is opaque and one-sided, and a masked one without a cutoff tests against 0.5.
	#[r#async::test]
	async fn materials_default_to_opaque_one_sided_and_a_half_cutoff() {
		let (asset_manager, resource_storage_backend) = coverage_asset_manager("", &[("masked", r#", "alpha_mode": "Mask""#)]);

		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("masked.bema")
			.await
			.expect("Failed to bake the variant");
		let material = stored_variant(&resource_storage_backend, "masked.bema").material;
		let material: MaterialModel = crate::from_slice(
			&resource_storage_backend
				.get_resource(material.id())
				.expect("Expected the material")
				.resource,
		)
		.expect("Expected a material model");

		assert_eq!((material.alpha_mode, material.double_sided), (AlphaMode::Opaque, false));
		assert_eq!(
			stored_variant(&resource_storage_backend, "masked.bema").alpha_mode,
			AlphaMode::Mask(0.5)
		);
	}

	/// An alpha mode, cutoff, or sidedness outside what `.bema` accepts fails the bake.
	#[r#async::test]
	async fn invalid_alpha_mode_cutoff_or_sidedness_fails_to_bake() {
		for fields in [
			r#""alpha_mode": "Cutout","#,
			r#""alpha_mode": "Mask", "alpha_cutoff": 1.5,"#,
			r#""alpha_cutoff": "0.5","#,
			r#""double_sided": "true","#,
		] {
			let (asset_manager, _) = coverage_asset_manager(fields, &[]);
			let baked: Result<ReferenceModel<MaterialModel>, _> = asset_manager.bake_if_not_exists("foliage.bema").await;
			assert!(baked.is_err(), "fields {fields} should fail the bake");
		}
	}

	/// Only a blended surface has the opaque scene drawn behind it, so a material that reads the backdrop bakes only
	/// while it blends, whether its own `alpha_mode` or a variant's decides that.
	#[r#async::test]
	async fn reading_the_backdrop_needs_a_blended_material() {
		const BACKDROP_GRAPH: &str = r#"{
			nodes: [
				{ name: "strength", node: "parameter", parameter: "strength", type: "float" },
				{ name: "behind", node: "scenecolor" },
				{ name: "tinted", node: "multiply", in1: "behind", in2: "strength" },
				{ name: "surface", node: "surface", transmission: "tinted" },
			],
		}"#;

		let (asset_manager, _) = coverage_asset_manager_with_graph(
			BACKDROP_GRAPH,
			r#""alpha_mode": "Blend","#,
			&[("clear", ""), ("solid", r#", "alpha_mode": "Opaque""#)],
		);
		let _: ReferenceModel<VariantModel> = asset_manager
			.bake_if_not_exists("clear.bema")
			.await
			.expect("A variant that keeps its parent's blending should bake");
		let solid: Result<ReferenceModel<VariantModel>, _> = asset_manager.bake_if_not_exists("solid.bema").await;
		assert!(solid.is_err(), "A variant that turns a blended parent opaque should fail the bake");

		for fields in ["", r#""alpha_mode": "Mask","#] {
			let (asset_manager, _) = coverage_asset_manager_with_graph(BACKDROP_GRAPH, fields, &[]);
			let baked: Result<ReferenceModel<MaterialModel>, _> = asset_manager.bake_if_not_exists("foliage.bema").await;
			assert!(baked.is_err(), "fields {fields:?} should fail the bake");
		}
	}

	/// Two different materials that hash to one key would run each other's code, so the bake fails instead.
	#[r#async::test]
	async fn assembled_shaders_reject_colliding_keys() {
		let colors = ["\"White\"", "[0.5, 0.25, 1.0, 1.0]"];
		let (asset_manager, _) = assembling_asset_manager(&colors, true);

		bake_variants(&asset_manager, colors.len()).await;

		assert!(asset_manager.bake("shaders.uber").await.is_err());
	}
}

use std::collections::hash_map::Entry;
use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use utils::Extent;

mod binding;

use binding::{BindingValue, InlineBinding, MATERIAL_BINDINGS_DOCS_PATH};
pub(crate) use binding::{MaterialBinding, TemplateScope, binds_variant_asset, material_binding};

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	store_model, store_model_owned,
};
use crate::asset::handler::implementations::besl::{PreparedBeslShader, shader_compilation_error_message};
use crate::bemx::{self, import::ImportedMaterial, material_texture_parameter_name, material_texture_slot};
use crate::resources::image::Image;
use crate::{
	ProcessedAsset, ReferenceModel, asset,
	r#async::spawn_cpu_task,
	online_docs_url, resource,
	resources::material::{
		AssembledShader, MaterialModel, MaterialProgram, ParameterModel, ProgramParameter, RenderModel, Shader, ValueModel,
		VariantModel, VariantVariableModel,
	},
	shader::{besl::backends::platform::PlatformShaderCompiler, generator::ShaderGenerationSettings},
	types::AlphaMode,
};
