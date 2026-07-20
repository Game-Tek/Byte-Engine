use std::{
	collections::hash_map::DefaultHasher,
	hash::Hasher as _,
	sync::{Arc, OnceLock},
};

use utils::{
	json::{JsonContainerTrait as _, JsonValueTrait as _},
	Extent,
};

use super::{
	asset_handler::{AssetHandler, BakeContext, LoadErrors},
	bema_asset_handler::ProgramGenerator,
	BEADType, ResourceId,
};
use crate::{
	r#async::spawn_cpu_task,
	resources::material::{Binding, Shader, ShaderInterface},
	shader::{
		artifact::finalize_platform_shader_artifact,
		besl::{
			backends::platform::{PlatformShaderGenerator, PlatformShaderLanguage},
			evaluation::ProgramEvaluation,
		},
		generator::ShaderGenerationSettings,
	},
	types::ShaderTypes,
	ProcessedAsset,
};

/// The `BESLShaderAssetHandler` struct exists to bake standalone BESL programs into runtime shader resources.
pub struct BESLShaderAssetHandler {
	compiler: Arc<dyn ShaderCompiler>,
	generator: Option<Arc<dyn ProgramGenerator>>,
}

impl Default for BESLShaderAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl BESLShaderAssetHandler {
	pub fn new() -> Self {
		Self {
			compiler: Arc::new(PlatformShaderCompiler),
			generator: None,
		}
	}

	/// Configures the shared program scope applied before standalone shaders are linked and lowered.
	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
	}
}

impl AssetHandler for BESLShaderAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "besl"
	}

	fn should_discover(&self, _id: ResourceId<'_>, has_sidecar: bool) -> bool {
		has_sidecar
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		if context.resource_type(id).is_some_and(|resource_type| resource_type != "besl") {
			return Err(LoadErrors::UnsupportedType);
		}

		let (source, spec, format) = context.resolve(id).await?;
		if format != "besl" {
			return Err(LoadErrors::UnsupportedType);
		}

		let source = std::str::from_utf8(&source)
			.map_err(|_| LoadErrors::FailedToProcess)?
			.to_string();
		let settings = parse_shader_settings(spec.as_ref()).map_err(|error| {
			log::error!(
				"Failed to read standalone BESL shader settings for '{}': {error}",
				id.as_ref()
			);
			LoadErrors::FailedToProcess
		})?;
		let id_string = id.as_ref().to_string();
		let source_hash = hash_shader_source(&id_string, &source, settings);
		let compiler = Arc::clone(&self.compiler);
		let generator = self.generator.clone();

		// Platform compilation may invoke native shader toolchains, so it must not block the asset executor.
		let (shader, bytes) =
			spawn_cpu_task(move || compiler.compile(&id_string, &source, settings, source_hash, generator.as_deref()))
				.await
				.map_err(|_| LoadErrors::FailedToProcess)?
				.map_err(|error| {
					log::error!("Failed to compile standalone BESL shader '{}': {error}", id.as_ref());
					LoadErrors::FailedToProcess
				})?;

		context.store_primary(ProcessedAsset::new(id, shader), &bytes)
	}
}

trait ShaderCompiler: Send + Sync {
	fn compile(
		&self,
		id: &str,
		source: &str,
		settings: BESLShaderSettings,
		source_hash: u64,
		generator: Option<&dyn ProgramGenerator>,
	) -> Result<(Shader, Box<[u8]>), String>;
}

/// The `PlatformShaderCompiler` struct keeps standalone asset baking on the platform compiler selected by resource management.
struct PlatformShaderCompiler;

impl ShaderCompiler for PlatformShaderCompiler {
	fn compile(
		&self,
		id: &str,
		source: &str,
		settings: BESLShaderSettings,
		source_hash: u64,
		generator: Option<&dyn ProgramGenerator>,
	) -> Result<(Shader, Box<[u8]>), String> {
		compile_shader(id, source, settings, source_hash, generator)
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BESLShaderSettings {
	stage: ShaderTypes,
	workgroup_size: Option<(u32, u32, u32)>,
	maximum_mesh_threadgroups: Option<u32>,
	maximum_vertices: Option<u32>,
	maximum_primitives: Option<u32>,
}

static STANDALONE_SHADER_CONTEXT: OnceLock<utils::json::Object> = OnceLock::new();

/// Returns the allocation-retaining empty material context used by standalone program generators.
fn standalone_shader_context() -> &'static utils::json::Object {
	STANDALONE_SHADER_CONTEXT.get_or_init(|| utils::json::object! { "variables": Vec::<utils::json::Value>::new() })
}

impl BESLShaderSettings {
	fn generation_settings(self, name: &str) -> ShaderGenerationSettings {
		let settings = match self.stage {
			ShaderTypes::Vertex => ShaderGenerationSettings::vertex(),
			ShaderTypes::Fragment => ShaderGenerationSettings::fragment(),
			ShaderTypes::Compute => {
				let (width, height, depth) = self.workgroup_size.expect(
					"Missing compute workgroup. The most likely cause is that validated BESL shader settings were not preserved.",
				);
				ShaderGenerationSettings::compute(Extent::new(width, height, depth))
			}
			ShaderTypes::Task => {
				let (width, height, depth) = self.workgroup_size.expect(
					"Missing task workgroup. The most likely cause is that validated BESL shader settings were not preserved.",
				);
				ShaderGenerationSettings::task(
					Extent::new(width, height, depth),
					self.maximum_mesh_threadgroups.expect(
						"Missing task mesh-threadgroup limit. The most likely cause is that validated BESL shader settings were not preserved.",
					),
				)
			}
			ShaderTypes::Mesh => {
				let (width, height, depth) = self.workgroup_size.expect(
					"Missing mesh workgroup. The most likely cause is that validated BESL shader settings were not preserved.",
				);
				ShaderGenerationSettings::mesh(
					self.maximum_vertices.expect(
						"Missing mesh vertex limit. The most likely cause is that validated BESL shader settings were not preserved.",
					),
					self.maximum_primitives.expect(
						"Missing mesh primitive limit. The most likely cause is that validated BESL shader settings were not preserved.",
					),
					Extent::new(width, height, depth),
				)
			}
			_ => unreachable!(
				"Unsupported standalone BESL shader stage. The most likely cause is invalid shader settings validation."
			),
		};

		settings.name(name.to_string())
	}
}

/// Reads the stage contract used to compile one standalone BESL source file.
fn parse_shader_settings(spec: Option<&BEADType>) -> Result<BESLShaderSettings, String> {
	let spec = spec.ok_or_else(|| {
		"Missing BESL shader settings. The most likely cause is that the source has no adjacent `.besl.bead` file.".to_string()
	})?;
	let stage = spec.get("stage").and_then(|stage| stage.as_str()).ok_or_else(|| {
		"Missing BESL shader stage. The most likely cause is that `stage` is absent or is not a string.".to_string()
	})?;

	let stage = match stage {
		"Vertex" => ShaderTypes::Vertex,
		"Fragment" => ShaderTypes::Fragment,
		"Compute" => ShaderTypes::Compute,
		"Task" => ShaderTypes::Task,
		"Mesh" => ShaderTypes::Mesh,
		stage => {
			return Err(format!(
				"Unsupported BESL shader stage '{stage}'. The most likely cause is that `stage` is not `Vertex`, `Fragment`, `Compute`, `Task`, or `Mesh`."
			));
		}
	};

	let workgroup_size = if matches!(stage, ShaderTypes::Compute | ShaderTypes::Task | ShaderTypes::Mesh) {
		Some(parse_workgroup_size(spec.get("workgroup").ok_or_else(|| {
			format!(
				"Missing {stage:?} workgroup. The most likely cause is that `workgroup` is absent from the shader's `.besl.bead` file."
			)
		})?)?)
	} else {
		None
	};

	let maximum_mesh_threadgroups = if stage == ShaderTypes::Task {
		Some(parse_positive_u32_setting(
			spec,
			"maximum_mesh_threadgroups",
			"task mesh-threadgroup limit",
		)?)
	} else {
		None
	};
	let maximum_vertices = if stage == ShaderTypes::Mesh {
		Some(parse_positive_u32_setting(spec, "maximum_vertices", "mesh vertex limit")?)
	} else {
		None
	};
	let maximum_primitives = if stage == ShaderTypes::Mesh {
		Some(parse_positive_u32_setting(
			spec,
			"maximum_primitives",
			"mesh primitive limit",
		)?)
	} else {
		None
	};

	Ok(BESLShaderSettings {
		stage,
		workgroup_size,
		maximum_mesh_threadgroups,
		maximum_vertices,
		maximum_primitives,
	})
}

fn parse_positive_u32_setting(spec: &BEADType, key: &str, description: &str) -> Result<u32, String> {
	let value = spec
		.get(key)
		.and_then(|value| value.as_u64())
		.and_then(|value| u32::try_from(value).ok())
		.ok_or_else(|| {
			format!(
				"Missing or invalid {description}. The most likely cause is that `{key}` is absent or is not a positive 32-bit integer."
			)
		})?;
	if value == 0 {
		return Err(format!(
			"Invalid zero {description}. The most likely cause is that `{key}` was not configured as at least one."
		));
	}
	Ok(value)
}

/// Validates the three positive dimensions required by a compute, task, or mesh workgroup contract.
fn parse_workgroup_size(value: &utils::json::Value) -> Result<(u32, u32, u32), String> {
	let dimensions = value.as_array().ok_or_else(|| {
		"Invalid shader workgroup. The most likely cause is that `workgroup` is not an array of three positive integers."
			.to_string()
	})?;
	if dimensions.len() != 3 {
		return Err(format!(
			"Invalid shader workgroup length {}. The most likely cause is that `workgroup` does not contain exactly three dimensions.",
			dimensions.len()
		));
	}

	let mut parsed = [0; 3];
	for (index, dimension) in dimensions.iter().enumerate() {
		let dimension = dimension
			.as_u64()
			.and_then(|dimension| u32::try_from(dimension).ok())
			.ok_or_else(|| {
				format!(
				"Invalid shader workgroup dimension {index}. The most likely cause is that the dimension is not a positive 32-bit integer."
			)
			})?;
		if dimension == 0 {
			return Err(format!(
				"Invalid zero shader workgroup dimension {index}. The most likely cause is that every workgroup dimension was not configured as at least one."
			));
		}
		parsed[index] = dimension;
	}

	Ok((parsed[0], parsed[1], parsed[2]))
}

/// Parses, links, and reflects a standalone shader before platform lowering starts.
fn prepare_shader(
	source: &str,
	workgroup_size: Option<(u32, u32, u32)>,
	generator: Option<&dyn ProgramGenerator>,
) -> Result<(besl::NodeReference, ShaderInterface), String> {
	let parsed = besl::parse(source).map_err(|error| {
		format!("Failed to parse BESL source ({error:?}). The most likely cause is invalid standalone shader syntax.")
	})?;
	let parsed = match generator {
		Some(generator) => generator.transform(parsed, standalone_shader_context()),
		None => parsed,
	};
	let program = besl::lex(parsed).map_err(|error| {
		format!("Failed to link BESL source ({error:?}). The most likely cause is an unresolved or invalid shader declaration.")
	})?;
	let main = program.get_main().ok_or_else(|| {
		"Missing BESL main function. The most likely cause is that the standalone shader does not declare `main`.".to_string()
	})?;
	let bindings = ProgramEvaluation::from_main(&main)?
		.into_bindings()
		.into_iter()
		.map(|binding| {
			Binding::named(
				binding.name,
				binding.slot,
				binding.kind,
				binding.count,
				binding.read,
				binding.write,
			)
		})
		.collect();

	Ok((
		main,
		ShaderInterface {
			workgroup_size,
			bindings,
		},
	))
}

/// Compiles one validated standalone program and persists its semantic interface alongside the platform artifact.
fn compile_shader(
	id: &str,
	source: &str,
	settings: BESLShaderSettings,
	source_hash: u64,
	generator: Option<&dyn ProgramGenerator>,
) -> Result<(Shader, Box<[u8]>), String> {
	let (main, interface) = prepare_shader(source, settings.workgroup_size, generator)?;
	let mut generator = PlatformShaderGenerator::new();
	let compiled = generator.generate(&settings.generation_settings(id), &main)?;

	// Compiled reflection is a backend contract; semantic reflection supplies the authored names retained in the resource.
	let semantic_bindings = interface
		.bindings
		.iter()
		.map(|binding| (binding.slot, binding.kind, binding.count, binding.read, binding.write))
		.collect::<Vec<_>>();
	let compiled_bindings = compiled
		.bindings()
		.iter()
		.map(|binding| (binding.slot, binding.kind, binding.count, binding.read, binding.write))
		.collect::<Vec<_>>();
	if compiled_bindings != semantic_bindings {
		return Err(
			"BESL shader reflection mismatch. The most likely cause is that the active platform compiler emitted a different resource interface than semantic evaluation."
				.to_string(),
		);
	}

	let compiled_workgroup = compiled
		.extent()
		.map(|extent| (extent.width(), extent.height(), extent.depth()));
	if compiled_workgroup != interface.workgroup_size {
		return Err(
			"BESL shader workgroup mismatch. The most likely cause is that the active platform compiler did not preserve the configured compute, task, or mesh workgroup."
				.to_string(),
		);
	}

	let language = PlatformShaderLanguage::current_platform();
	let entry_point = compiled.entry_point();
	let (artifact, bytes) =
		finalize_platform_shader_artifact(language, settings.stage, id, entry_point, compiled.into_binary())?;

	Ok((
		Shader {
			id: id.to_string(),
			stage: settings.stage,
			interface,
			artifact,
			source_hash,
		},
		bytes,
	))
}

/// Hashes every source-side input that can change a standalone shader resource.
fn hash_shader_source(id: &str, source: &str, settings: BESLShaderSettings) -> u64 {
	let mut hasher = DefaultHasher::new();
	hasher.write(b"standalone-besl-shader-v2");
	hash_text(&mut hasher, id);
	hash_text(&mut hasher, source);
	hasher.write_u8(match settings.stage {
		ShaderTypes::Vertex => 0,
		ShaderTypes::Fragment => 1,
		ShaderTypes::Compute => 2,
		ShaderTypes::Task => 3,
		ShaderTypes::Mesh => 4,
		_ => unreachable!("Unsupported standalone BESL shader stage. The most likely cause is invalid settings validation."),
	});
	match settings.workgroup_size {
		Some((width, height, depth)) => {
			hasher.write_u8(1);
			hasher.write_u32(width);
			hasher.write_u32(height);
			hasher.write_u32(depth);
		}
		None => hasher.write_u8(0),
	}
	hash_optional_u32(&mut hasher, settings.maximum_mesh_threadgroups);
	hash_optional_u32(&mut hasher, settings.maximum_vertices);
	hash_optional_u32(&mut hasher, settings.maximum_primitives);
	hasher.write_u8(match PlatformShaderLanguage::current_platform() {
		PlatformShaderLanguage::Glsl => 0,
		PlatformShaderLanguage::Hlsl => 1,
		PlatformShaderLanguage::Msl => 2,
	});
	hasher.finish()
}

fn hash_optional_u32(hasher: &mut DefaultHasher, value: Option<u32>) {
	match value {
		Some(value) => {
			hasher.write_u8(1);
			hasher.write_u32(value);
		}
		None => hasher.write_u8(0),
	}
}

fn hash_text(hasher: &mut DefaultHasher, text: &str) {
	hasher.write_u64(text.len() as u64);
	hasher.write(text.as_bytes());
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::{
		hash_shader_source, parse_shader_settings, prepare_shader, BESLShaderAssetHandler, BESLShaderSettings, ShaderCompiler,
	};
	use crate::{
		asset::{
			asset_handler::LoadErrors,
			asset_manager::{AssetManager, LoadMessages},
			bema_asset_handler::ProgramGenerator,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
			ResourceId,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::{Binding, BindingKind, Shader, ShaderArtifact, ShaderInterface, TextureView},
		shader::generator::Stages,
		types::ShaderTypes,
	};

	struct TestShaderCompiler;

	/// The `TestStandaloneGenerator` struct supplies one reusable helper to standalone shader bake tests.
	struct TestStandaloneGenerator;

	impl ProgramGenerator for TestStandaloneGenerator {
		fn transform<'a>(&self, mut node: besl::parser::Node<'a>, _: &'a utils::json::Object) -> besl::parser::Node<'a> {
			let mut helper_scope = besl::parse("shared_value: fn () -> f32 { return 0.5; }")
				.expect("Failed to parse the standalone generator fixture. The most likely cause is invalid BESL test syntax.");
			let helpers = match helper_scope.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
				_ => unreachable!(
					"Invalid standalone generator fixture root. The most likely cause is a parser contract regression."
				),
			};
			node.add(helpers);
			node
		}
	}

	impl ShaderCompiler for TestShaderCompiler {
		fn compile(
			&self,
			id: &str,
			source: &str,
			settings: BESLShaderSettings,
			source_hash: u64,
			generator: Option<&dyn crate::asset::bema_asset_handler::ProgramGenerator>,
		) -> Result<(Shader, Box<[u8]>), String> {
			assert_eq!(id, "passes/resolve.besl");
			assert!(source.contains("main"));
			assert_eq!(settings.stage, ShaderTypes::Compute);
			assert_eq!(settings.workgroup_size, Some((8, 8, 1)));
			assert_eq!(settings.maximum_mesh_threadgroups, None);
			assert_eq!(settings.maximum_vertices, None);
			assert_eq!(settings.maximum_primitives, None);
			prepare_shader(source, settings.workgroup_size, generator)?;

			Ok((
				Shader {
					id: id.to_string(),
					stage: settings.stage,
					interface: ShaderInterface {
						workgroup_size: settings.workgroup_size,
						bindings: vec![Binding::named("output", 1, BindingKind::StorageImage, 1, false, true)],
					},
					artifact: ShaderArtifact::Spirv,
					source_hash,
				},
				b"compiled-shader".to_vec().into_boxed_slice(),
			))
		}
	}

	#[test]
	fn standalone_besl_discovery_requires_an_adjacent_sidecar() {
		let asset_storage = AssetTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(BESLShaderAssetHandler {
			compiler: Arc::new(TestShaderCompiler),
			generator: None,
		});

		assert!(asset_manager.supports("passes/resolve.besl"));
		assert!(!asset_manager.should_discover("passes/resolve.besl", false));
		assert!(asset_manager.should_discover("passes/resolve.besl", true));
	}

	#[r#async::test]
	async fn explicit_bake_without_a_sidecar_still_reaches_the_besl_handler() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("passes/no-settings.besl", b"main: fn () -> void {}");
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(BESLShaderAssetHandler {
			compiler: Arc::new(TestShaderCompiler),
			generator: None,
		});

		let result = asset_manager.bake("passes/no-settings.besl", &resource_storage).await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "passes/no-settings.besl".to_string(),
				error: LoadErrors::FailedToProcess,
			})
		);
	}

	#[r#async::test]
	async fn standalone_besl_handler_bakes_source_id_metadata_and_binary_in_memory() {
		let source = "main: fn () -> void {}";
		let bead = r#"{ "stage": "Compute", "workgroup": [8, 8, 1] }"#;
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("passes/resolve.besl", source.as_bytes());
		asset_storage.add_file("passes/resolve.besl.bead", bead.as_bytes());
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(BESLShaderAssetHandler {
			compiler: Arc::new(TestShaderCompiler),
			generator: None,
		});

		asset_manager
			.bake("passes/resolve.besl", &resource_storage)
			.await
			.expect("standalone BESL source should bake through its registered handler");

		let resource = resource_storage
			.get_resource(ResourceId::new("passes/resolve.besl"))
			.expect("standalone shader should use its source asset ID");
		let shader: Shader = crate::from_slice(&resource.resource).expect("stored shader metadata should deserialize");
		let settings = BESLShaderSettings {
			stage: ShaderTypes::Compute,
			workgroup_size: Some((8, 8, 1)),
			maximum_mesh_threadgroups: None,
			maximum_vertices: None,
			maximum_primitives: None,
		};
		assert_eq!(shader.id, "passes/resolve.besl");
		assert_eq!(shader.stage, ShaderTypes::Compute);
		assert_eq!(shader.interface.workgroup_size, Some((8, 8, 1)));
		assert_eq!(shader.interface.bindings[0].name, "output");
		assert_eq!(
			shader.source_hash,
			hash_shader_source("passes/resolve.besl", source, settings)
		);
		assert_eq!(
			resource_storage
				.get_resource_data_by_name(ResourceId::new("passes/resolve.besl"))
				.as_deref(),
			Some(b"compiled-shader".as_slice())
		);
	}

	#[r#async::test]
	async fn standalone_besl_handler_applies_the_configured_program_generator() {
		let source = "main: fn () -> void { let value: f32 = shared_value(); value; }";
		let bead = r#"{ "stage": "Compute", "workgroup": [8, 8, 1] }"#;
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("passes/resolve.besl", source.as_bytes());
		asset_storage.add_file("passes/resolve.besl.bead", bead.as_bytes());
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		let mut handler = BESLShaderAssetHandler {
			compiler: Arc::new(TestShaderCompiler),
			generator: None,
		};
		handler.set_shader_generator(TestStandaloneGenerator);
		asset_manager.add_asset_handler(handler);

		asset_manager
			.bake("passes/resolve.besl", &resource_storage)
			.await
			.expect(
				"Failed to bake a generated standalone shader. The most likely cause is that the configured program generator was not forwarded to compilation.",
			);
	}

	#[test]
	fn shader_settings_require_workgroups_and_stage_specific_mesh_limits() {
		for (stage, expected) in [("Vertex", ShaderTypes::Vertex), ("Fragment", ShaderTypes::Fragment)] {
			let spec = utils::json::from_str(&format!(r#"{{ "stage": "{stage}" }}"#)).unwrap();
			assert_eq!(
				parse_shader_settings(Some(&spec)),
				Ok(BESLShaderSettings {
					stage: expected,
					workgroup_size: None,
					maximum_mesh_threadgroups: None,
					maximum_vertices: None,
					maximum_primitives: None,
				})
			);
		}

		let compute_without_workgroup = utils::json::from_str(r#"{ "stage": "Compute" }"#).unwrap();
		assert!(parse_shader_settings(Some(&compute_without_workgroup)).is_err());
		let zero_workgroup = utils::json::from_str(r#"{ "stage": "Compute", "workgroup": [8, 0, 1] }"#).unwrap();
		assert!(parse_shader_settings(Some(&zero_workgroup)).is_err());

		let task =
			utils::json::from_str(r#"{ "stage": "Task", "workgroup": [32, 1, 1], "maximum_mesh_threadgroups": 32 }"#).unwrap();
		let task = parse_shader_settings(Some(&task)).expect("task sidecar should parse");
		assert_eq!(
			task,
			BESLShaderSettings {
				stage: ShaderTypes::Task,
				workgroup_size: Some((32, 1, 1)),
				maximum_mesh_threadgroups: Some(32),
				maximum_vertices: None,
				maximum_primitives: None,
			}
		);
		assert!(matches!(
			task.generation_settings("task").stage,
			Stages::Task {
				local_size,
				maximum_mesh_threadgroups: 32,
			} if local_size == utils::Extent::new(32, 1, 1)
		));

		let mesh = utils::json::from_str(
			r#"{ "stage": "Mesh", "workgroup": [128, 1, 1], "maximum_vertices": 64, "maximum_primitives": 126 }"#,
		)
		.unwrap();
		let mesh = parse_shader_settings(Some(&mesh)).expect("mesh sidecar should parse");
		assert_eq!(
			mesh,
			BESLShaderSettings {
				stage: ShaderTypes::Mesh,
				workgroup_size: Some((128, 1, 1)),
				maximum_mesh_threadgroups: None,
				maximum_vertices: Some(64),
				maximum_primitives: Some(126),
			}
		);
		assert!(matches!(
			mesh.generation_settings("mesh").stage,
			Stages::Mesh {
				maximum_vertices: 64,
				maximum_primitives: 126,
				local_size,
			} if local_size == utils::Extent::new(128, 1, 1)
		));

		for invalid in [
			r#"{ "stage": "Task", "workgroup": [32, 1, 1] }"#,
			r#"{ "stage": "Task", "workgroup": [32, 1, 1], "maximum_mesh_threadgroups": 0 }"#,
			r#"{ "stage": "Mesh", "workgroup": [128, 1, 1], "maximum_vertices": 64 }"#,
			r#"{ "stage": "Mesh", "workgroup": [128, 1, 1], "maximum_vertices": 0, "maximum_primitives": 126 }"#,
		] {
			let invalid = utils::json::from_str(invalid).unwrap();
			assert!(parse_shader_settings(Some(&invalid)).is_err());
		}
	}

	#[test]
	fn shader_hash_covers_source_stage_workgroup_and_id() {
		let compute = BESLShaderSettings {
			stage: ShaderTypes::Compute,
			workgroup_size: Some((8, 8, 1)),
			maximum_mesh_threadgroups: None,
			maximum_vertices: None,
			maximum_primitives: None,
		};
		let changed_workgroup = BESLShaderSettings {
			workgroup_size: Some((16, 8, 1)),
			..compute
		};
		let fragment = BESLShaderSettings {
			stage: ShaderTypes::Fragment,
			workgroup_size: None,
			maximum_mesh_threadgroups: None,
			maximum_vertices: None,
			maximum_primitives: None,
		};
		let task = BESLShaderSettings {
			stage: ShaderTypes::Task,
			workgroup_size: Some((32, 1, 1)),
			maximum_mesh_threadgroups: Some(32),
			maximum_vertices: None,
			maximum_primitives: None,
		};
		let changed_task_limit = BESLShaderSettings {
			maximum_mesh_threadgroups: Some(16),
			..task
		};
		let base = hash_shader_source("shader.besl", "main: fn () -> void {}", compute);

		assert_ne!(base, hash_shader_source("other.besl", "main: fn () -> void {}", compute));
		assert_ne!(base, hash_shader_source("shader.besl", "main: fn () -> void { 1; }", compute));
		assert_ne!(
			base,
			hash_shader_source("shader.besl", "main: fn () -> void {}", changed_workgroup)
		);
		assert_ne!(base, hash_shader_source("shader.besl", "main: fn () -> void {}", fragment));
		assert_ne!(
			hash_shader_source("task.besl", "main: fn () -> void {}", task),
			hash_shader_source("task.besl", "main: fn () -> void {}", changed_task_limit)
		);
	}

	#[test]
	fn shader_interface_reflection_preserves_descriptor_names_and_shapes() {
		let source = r#"
			Data: struct { value: u32, }
			data: descriptor<Data, 2, read_write>;
			textures: descriptor<Texture2DArray, 5, read, 4>;
			main: fn () -> void {
				data;
				textures;
			}
		"#;
		let (_, interface) = prepare_shader(source, None, None).expect(
			"Failed to prepare standalone descriptor reflection. The most likely cause is invalid descriptor fixture syntax.",
		);

		assert_eq!(interface.bindings.len(), 2);
		assert_eq!(interface.bindings[0].name, "data");
		assert_eq!(interface.bindings[0].slot, 2);
		assert_eq!(interface.bindings[0].kind, BindingKind::StorageBuffer);
		assert!(interface.bindings[0].read);
		assert!(interface.bindings[0].write);
		assert_eq!(interface.bindings[1].name, "textures");
		assert_eq!(interface.bindings[1].slot, 5);
		assert_eq!(interface.bindings[1].count, 4);
		assert_eq!(
			interface.bindings[1].kind,
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture2DArray
			}
		);
	}

	#[test]
	fn shader_interface_reflection_preserves_visibility_atomic_and_integer_image_access() {
		let source = r#"
			Counters: struct { values: atomicu32[1024], }
			counters: descriptor<Counters, 1033, read_write>;
			index_image: descriptor<StorageImage<r32ui>, 1040, read>;
			main: fn () -> void {
				let coord: vec2u = thread_id();
				let index: u32 = image_load_u32(index_image, coord);
				atomic_add(counters.values[index], 1);
			}
		"#;
		let (_, interface) = prepare_shader(source, Some((32, 32, 1)), None).expect(
			"Failed to prepare atomic descriptor reflection. The most likely cause is invalid visibility fixture syntax.",
		);

		assert_eq!(interface.workgroup_size, Some((32, 32, 1)));
		assert_eq!(interface.bindings.len(), 2);
		assert_eq!(interface.bindings[0].name, "counters");
		assert_eq!(interface.bindings[0].slot, 1033);
		assert_eq!(interface.bindings[0].kind, BindingKind::StorageBuffer);
		assert!(interface.bindings[0].read);
		assert!(interface.bindings[0].write);
		assert_eq!(interface.bindings[1].name, "index_image");
		assert_eq!(interface.bindings[1].slot, 1040);
		assert_eq!(interface.bindings[1].kind, BindingKind::StorageImage);
		assert!(interface.bindings[1].read);
		assert!(!interface.bindings[1].write);
	}
}
