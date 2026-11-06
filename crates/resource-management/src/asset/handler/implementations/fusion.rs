//! Fusion asset baking.

/// The most descriptor sets a fused shader binds: the four every Vulkan device binds at once.
const MAX_FUSED_SETS: usize = 4;

/// The reference page that defines `.fusion` files.
const FUSION_DOCS_PATH: &str = "reference/assets#fusions";

/// The `FusionAssetHandler` struct exists to bake `.fusion` assets, so a renderer can record a run of pixel steps as
/// one pass without anyone writing the combined shader.
///
/// A fusion names step pipelines whose shaders are pixel shaders. Its bake generates one shader that runs them in order
/// and stores it as `<fusion ID>#shader`, next to a [`Pipeline`] of kind [`PipelineKind::Fused`] at the fusion's ID.
/// Renderers find those pipelines with a query for the `kind` property `fused`.
pub struct FusionAssetHandler;

/// The `FusionSource` struct is the JSON a `.fusion` file holds.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FusionSource {
	/// Lets editors find the schema; baking ignores it.
	#[serde(rename = "$schema", default)]
	_schema: serde::de::IgnoredAny,
	name: String,
	steps: Vec<String>,
}

impl AssetHandler for FusionAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "fusion"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let fail = |message: String| {
			context.error(format_args!("{message} See {}.", crate::online_docs_url(FUSION_DOCS_PATH)));
			LoadErrors::FailedToProcess
		};
		// The fused shader's ID is a fragment of the fusion, so a request for either bakes both.
		let fusion_id = id.get_base().as_ref().to_string();
		let shader_id = format!("{fusion_id}#shader");

		let (source, _) = context.resolve(id).await?;
		let fusion: FusionSource = serde_json::from_slice(&source).map_err(|error| {
			fail(format!(
				"Fusion '{fusion_id}' could not be parsed: {error}. The most likely cause is invalid JSON or a field other than `name` and `steps`. Write the fusion as `{{\"name\": ..., \"steps\": [...]}}`."
			))
		})?;
		if !(2..=pixel_chain::MAX_STEPS).contains(&fusion.steps.len()) {
			return Err(fail(format!(
				"Fusion '{fusion_id}' lists {} steps. The most likely cause is a fusion of one step, which needs no fusion, or of more steps than a fused shader runs. List 2 to {} step pipelines.",
				fusion.steps.len(),
				pixel_chain::MAX_STEPS
			)));
		}

		// Each step pipeline bakes on its own too, so an unfused run of the step finds it.
		let pipelines = context.bake_dependencies::<Pipeline>(&fusion.steps, 8).await?;
		let mut shaders = Vec::with_capacity(pipelines.len());
		for (step, pipeline) in fusion.steps.iter().zip(pipelines) {
			let pipeline = context.solve(pipeline).await.map_err(|_| {
				fail(format!(
					"Fusion '{fusion_id}' lists '{step}', which didn't load as a pipeline. The most likely cause is a step ID that names another kind of asset. List the step's `.pipeline` asset."
				))
			})?;
			let PipelineKind::Tile { shader, .. } = &pipeline.resource().kind else {
				return Err(fail(format!(
					"Fusion '{fusion_id}' lists '{step}', which isn't a `tile` pipeline. The most likely cause is a step that records its own pass, such as a compute pipeline. List only pixel step pipelines."
				)));
			};
			shaders.push(shader.clone());
		}

		// Every step's shader must run on the tile the fused shader runs on.
		let mut sources = Vec::with_capacity(shaders.len());
		let mut stage = None;
		for shader in &shaders {
			let shader_id = ResourceId::new(shader);
			let (bytes, _) = context.resolve(shader_id).await.map_err(|_| {
				fail(format!(
					"Step shader '{shader}' of fusion '{fusion_id}' could not be read. The most likely cause is a step pipeline whose `shader` names a file that doesn't exist. Point the pipeline at the step's `.besl` file."
				))
			})?;
			let source = String::from_utf8(bytes.to_vec()).map_err(|_| {
				fail(format!(
					"Step shader '{shader}' is not UTF-8 text. The most likely cause is a binary file saved with a `.besl` extension. Save the shader as UTF-8 BESL source."
				))
			})?;
			let sidecar = context.load_sidecar(shader_id).await.map_err(|_| {
				fail(format!(
					"The settings of step shader '{shader}' could not be read. The most likely cause is an invalid `.besl.bead` file next to it. Fix the file's JSON5."
				))
			})?;
			let settings = parse_shader_settings(sidecar.as_ref())
				.map_err(|error| fail(format!("Step shader '{shader}' has invalid settings: {error}")))?;
			let Stages::Tile { tile, local_size } = settings.stage else {
				return Err(fail(format!(
					"Step shader '{shader}' of fusion '{fusion_id}' isn't a `Tile` stage shader. The most likely cause is a step pipeline whose shader runs as compute. Set the shader's `stage` to `Tile`."
				)));
			};
			if *stage.get_or_insert((tile, local_size)) != (tile, local_size) {
				return Err(fail(format!(
					"The steps of fusion '{fusion_id}' run on different tiles. The most likely cause is a step shader whose `tile` or `workgroup` differs from the first step's. Give every step's shader the same `tile` and `workgroup`."
				)));
			}
			sources.push(source);
		}
		let (tile, local_size) = stage.expect("A fusion has at least two steps");

		let steps = shaders
			.iter()
			.zip(&sources)
			.map(|(shader, source)| (ResourceId::new(shader), source.as_str()))
			.collect::<Vec<_>>();
		let (wrapper, modules) = load_pixel_chain(&context, &steps)
			.await
			.map_err(|error| fail(format!("Fusion '{fusion_id}' could not be generated: {error}")))?;
		let program = chain_program(&wrapper, &modules, context.program_transform())
			.map_err(|error| fail(format!("Fusion '{fusion_id}' could not be generated: {error}")))?;
		let settings = ShaderGenerationSettings::tile(tile, local_size).name(shader_id.clone());
		let prepared = context.prepare_shader(program, &settings).map_err(|error| {
			fail(format!(
				"Fusion '{fusion_id}' could not be linked: {error} The most likely cause is two steps whose shaders declare the same name. Rename one of them."
			))
		})?;
		let sets = prepared.interface().sets.len();
		if sets > MAX_FUSED_SETS {
			return Err(fail(format!(
				"Fusion '{fusion_id}' binds {sets} descriptor sets, more than the {MAX_FUSED_SETS} a fused shader may bind. The most likely cause is too many steps that take a layer. Split the fusion into fusions with fewer steps that take a layer."
			)));
		}
		let (shader, bytes) = context
			.compile_prepared(&shader_id, prepared)
			.await
			.map_err(|error| fail(format!("Fusion '{fusion_id}' could not be compiled: {error}")))?;
		context
			.store_resource_owned(ProcessedAsset::new(ResourceId::new(&shader_id), shader), bytes)
			.await?;

		let pipeline = Pipeline {
			name: fusion.name,
			kind: PipelineKind::Fused {
				shader: shader_id,
				steps: fusion.steps,
				// The fused shader's one push constant is its step mask.
				push_constants: vec![PushConstantRange { offset: 0, size: 4 }],
			},
		};
		context
			.store_resource(ProcessedAsset::new(ResourceId::new(&fusion_id), pipeline), &[])
			.await
			.map(|_| ())
	}
}

use super::{
	ResourceId,
	besl::{chain_program, load_pixel_chain, parse_shader_settings},
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	resources::pipeline::{Pipeline, PipelineKind, PushConstantRange},
	shader::{ShaderGenerationSettings, generator::Stages, pixel_chain},
};

#[cfg(test)]
mod tests {
	use super::FusionAssetHandler;
	use crate::{
		asset::{
			handler::implementations::{
				PipelineAssetHandler,
				besl::{BESLShaderAssetHandler, PreparedBeslShader, ShaderCompiler},
			},
			manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		resource::{
			resource_manager::ResourceManager,
			storage_backend::{MemoryStorageBackend, Query},
		},
		resources::{
			material::{Shader, ShaderArtifact},
			pipeline::{Pipeline, PipelineKind},
		},
	};

	/// The `LinkingCompiler` struct stands in for the platform toolchain and keeps the interface reflection found, so
	/// the tests depend only on chain generation, linking, and lowering.
	struct LinkingCompiler;

	impl ShaderCompiler for LinkingCompiler {
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
						interface: shader.interface().clone(),
						artifact: ShaderArtifact::Spirv,
					},
					b"compiled-shader".to_vec().into_boxed_slice(),
				))
			})
		}
	}

	const TILE_BEAD: &str = r#"{ "stage": "Tile", "tile": 16 }"#;

	const EXPOSE: &str = r#"
ExposureParameters: struct { scale: vec4f, }
Exposure: layer {
	parameters: descriptor<{ type: ExposureParameters, binding: 0, access: read, memory: constant }>;
}
pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, exposure: Exposure) -> vec4f {
	return color * exposure.parameters.scale.x;
}
"#;

	const ENCODE: &str = "pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u) -> vec4f { return color * color; }";

	/// Adds a pixel step `name` with `source`, its sidecar `bead`, and its tile pipeline under `post/`.
	fn add_step(assets: &AssetTestStorageBackend, name: &str, source: &str, bead: &str) {
		// The test storage keeps every path for the whole test run.
		let path = |extension: &str| -> &'static str { format!("post/{name}.{extension}").leak() };
		assets.add_file(path("besl"), source.as_bytes());
		assets.add_file(path("besl.bead"), bead.as_bytes());
		let pipeline = format!(r#"{{"name":"{name}","kind":{{"type":"tile","shader":"post/{name}.besl"}}}}"#);
		assets.add_file(path("pipeline"), pipeline.as_bytes());
	}

	fn asset_manager(assets: AssetTestStorageBackend, resources: MemoryStorageBackend) -> AssetManager {
		let mut asset_manager = AssetManager::new(assets, resources);
		asset_manager.set_shader_compiler(LinkingCompiler);
		asset_manager.add_asset_handler(BESLShaderAssetHandler);
		asset_manager.add_asset_handler(PipelineAssetHandler);
		asset_manager.add_asset_handler(FusionAssetHandler);
		asset_manager
	}

	/// Returns a store with the exposure and encoding steps and a fusion of both at `post/display.fusion`.
	fn display_assets() -> AssetTestStorageBackend {
		let assets = AssetTestStorageBackend::new();
		add_step(&assets, "expose", EXPOSE, TILE_BEAD);
		add_step(&assets, "encode", ENCODE, TILE_BEAD);
		assets.add_file(
			"post/display.fusion",
			br#"{"name":"Display","steps":["post/expose.pipeline","post/encode.pipeline"]}"#,
		);
		assets
	}

	#[r#async::test]
	async fn a_fusion_bakes_a_fused_pipeline_that_queries_find() {
		let resources = MemoryStorageBackend::new();
		asset_manager(display_assets(), resources.clone())
			.bake("post/display.fusion")
			.await
			.expect("the fusion should bake");

		let resource_manager = ResourceManager::new(resources);
		let fused = resource_manager
			.query_ids(Query::new("Pipeline").eq("kind", "fused"))
			.await
			.expect("the store should answer the query");
		assert_eq!(fused.items, vec!["post/display.fusion".to_string()]);

		let pipeline = resource_manager
			.request::<Pipeline>("post/display.fusion")
			.await
			.expect("the fused pipeline should be stored");
		let PipelineKind::Fused {
			shader,
			steps,
			push_constants,
		} = &pipeline.resource().kind
		else {
			panic!("A fusion stored a pipeline of another kind: {:?}", pipeline.resource().kind);
		};
		assert_eq!(steps, &["post/expose.pipeline", "post/encode.pipeline"]);
		// The step mask is the fused shader's one push constant.
		assert_eq!(
			push_constants
				.iter()
				.map(|range| (range.offset, range.size))
				.collect::<Vec<_>>(),
			[(0, 4)]
		);

		let shader = resource_manager
			.request::<Shader>(shader)
			.await
			.expect("the fused shader should be stored");
		let tile = shader
			.resource()
			.interface
			.tile
			.as_ref()
			.expect("a fused shader is a tile shader");
		assert_eq!((tile.size, tile.outputs.len()), (16, 1));
		assert_eq!(tile.outputs[0].name, "result");
	}

	#[r#async::test]
	async fn requesting_the_fused_shader_bakes_the_whole_fusion() {
		let resources = MemoryStorageBackend::new();
		asset_manager(display_assets(), resources.clone())
			.bake("post/display.fusion#shader")
			.await
			.expect("the fused shader should bake");

		let resource_manager = ResourceManager::new(resources);
		resource_manager
			.request::<Pipeline>("post/display.fusion")
			.await
			.expect("baking the shader should also store the fused pipeline");
	}

	#[r#async::test]
	async fn a_pixel_step_bakes_on_its_own_with_a_generated_main() {
		let resources = MemoryStorageBackend::new();
		asset_manager(display_assets(), resources.clone())
			.bake("post/expose.besl")
			.await
			.expect("a pixel shader should bake alone");

		let shader = ResourceManager::new(resources)
			.request::<Shader>("post/expose.besl")
			.await
			.expect("the step shader should be stored");
		let tile = shader.resource().interface.tile.as_ref().expect("a step is a tile shader");
		assert_eq!(tile.outputs[0].name, "result");
	}

	#[r#async::test]
	async fn steps_on_different_tiles_do_not_fuse() {
		let assets = AssetTestStorageBackend::new();
		add_step(&assets, "expose", EXPOSE, TILE_BEAD);
		add_step(&assets, "encode", ENCODE, r#"{ "stage": "Tile", "tile": 32 }"#);
		assets.add_file(
			"post/display.fusion",
			br#"{"name":"Display","steps":["post/expose.pipeline","post/encode.pipeline"]}"#,
		);

		asset_manager(assets, MemoryStorageBackend::new())
			.bake("post/display.fusion")
			.await
			.expect_err("steps on different tiles can't share one shader");
	}

	#[r#async::test]
	async fn a_step_that_records_its_own_pass_does_not_fuse() {
		let assets = display_assets();
		assets.add_file("post/blur.besl", b"main: fn (input: StageInput) -> void {}");
		assets.add_file("post/blur.besl.bead", br#"{ "stage": "Compute", "workgroup": [8, 8, 1] }"#);
		assets.add_file(
			"post/blur.pipeline",
			br#"{"name":"Blur","kind":{"type":"compute","shader":"post/blur.besl"}}"#,
		);
		assets.add_file(
			"post/blurred.fusion",
			br#"{"name":"Blurred","steps":["post/expose.pipeline","post/blur.pipeline"]}"#,
		);

		asset_manager(assets, MemoryStorageBackend::new())
			.bake("post/blurred.fusion")
			.await
			.expect_err("a compute pipeline isn't a pixel step");
	}

	#[r#async::test]
	async fn a_fusion_of_one_step_is_rejected() {
		let assets = display_assets();
		assets.add_file("post/single.fusion", br#"{"name":"Single","steps":["post/expose.pipeline"]}"#);

		asset_manager(assets, MemoryStorageBackend::new())
			.bake("post/single.fusion")
			.await
			.expect_err("one step needs no fusion");
	}
}
