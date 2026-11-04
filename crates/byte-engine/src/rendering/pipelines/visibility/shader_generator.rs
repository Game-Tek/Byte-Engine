//! Turns BEMX material graphs into visibility material-evaluation compute shaders, and into the closest-hit shaders
//! that report a material's surface to surfel GI's rays.
//!
//! [`visibility_shader_module`] declares the buffers, images, structs, and BESL helper functions every visibility
//! shader can reference. [`VisibilityShaderGenerator`] writes the statements that compute a graph's surface, see
//! [`material`], and wraps them with the pixel reconstruction prefix and the lighting suffix from [`sources`], or with
//! the hit reconstruction prefix and the payload suffix.

mod ast;
mod material;
mod sources;
#[cfg(test)]
mod tests;

use std::sync::LazyLock;

use besl::parser::{MatchArm, MatchPattern, Node};
use resource_management::asset::handler::implementations::bema::{
	AssembledArm, AssembledMaterial, MaterialParameter, ParameterKind, ProgramGenerator,
};
use resource_management::bemx;
use resource_management::shader::generator::ShaderGenerationSettings;
use resource_management::types::AlphaMode;
#[cfg(test)]
pub(super) use sources::{
	INDIRECT_DIFFUSE_SOURCE_POSITION_SOURCE, SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE, SSGI_STORED_NORMAL_SOURCE,
	SSGI_SURFACE_WEIGHT_SOURCE, WEIGH_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE,
};

use self::ast::*;
pub(crate) use self::ast::{MATERIAL_DEBUG_VIEW_NAMES, remove_statements_reading};
use self::sources::*;
use super::VisibilityFeatures;
use super::features::{MaterialEvaluationMode, UBER_MATERIAL_SHADERS};
use super::layout::{
	LIGHT_CLUSTER_COLUMNS, LIGHT_CLUSTER_MASK_WORD_COUNT, LIGHT_CLUSTER_ROWS, LIGHT_CLUSTER_SLICES,
	LOCAL_SHADOW_SIZE_CLASS_COUNT, MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS,
	MAX_PIXEL_MAPPING_ENTRIES, MIN_LOCAL_SHADOW_MAP_RESOLUTION, TRANSPARENT_SPECIALIZATION_ID,
	material_variable_specialization_id,
};
use crate::application::parameters::Parameters;
use crate::rendering::common_shader_generator::common_shader_module;
use crate::rendering::set_layout::{ENGINE_MODULES, module_source};

// The layer modules spell out their array sizes; these guards catch limit changes.
const _: () = assert!(
	MAX_LIGHTS == 1024
		&& LIGHT_CLUSTER_MASK_WORD_COUNT == 98304
		&& MAX_MATERIALS == 1024
		&& MAX_MATERIAL_TEXTURES == 16
		&& MAX_BINDLESS_TEXTURES == 1024
		&& MAX_PIXEL_MAPPING_ENTRIES == 8294400,
	"Update the array sizes in `scene.besl`, `visibility-buffer.besl`, `material-dispatch.besl`, and `material-evaluation.besl` when visibility limits change."
);
// `material-evaluation.besl` spells out the size-class count and the material evaluation suffix in `sources` the
// smallest class's resolution.
const _: () = assert!(
	LOCAL_SHADOW_SIZE_CLASS_COUNT == 5 && MIN_LOCAL_SHADOW_MAP_RESOLUTION == 128,
	"Update the `cone_shadow_maps` and `point_shadow_maps` counts in `material-evaluation.besl` and the point shadow resolution in the material evaluation suffix when local shadow size classes change."
);
// The material evaluation suffix in `sources` and `light-clusters.besl` spell out the cluster grid.
const _: () = assert!(
	LIGHT_CLUSTER_COLUMNS == 16 && LIGHT_CLUSTER_ROWS == 8 && LIGHT_CLUSTER_SLICES == 24,
	"Update the light cluster grid in the material evaluation suffix and light-clusters.besl when it changes."
);

/// The layers every material-evaluation shader's `main` takes, in set order, as parameter name and layer type: the
/// scene, visibility buffer, and material evaluation sets.
///
/// The render pass creates those sets from the same layers, see [`super::set_layouts`], so a generated shader and the
/// sets it is bound with cannot disagree.
const MATERIAL_EVALUATION_LAYERS: [(&str, &str); 3] = [
	("scene", "Scene"),
	("visibility_buffer", "VisibilityBuffer"),
	("material_evaluation", "MaterialEvaluation"),
];

/// The modules a material-evaluation shader brings in: the scene records, and the modules that declare
/// [`MATERIAL_EVALUATION_LAYERS`].
const MATERIAL_EVALUATION_MODULES: [&str; 4] = [
	"::byte_engine::rendering::visibility::layouts::scene_records",
	"::byte_engine::rendering::visibility::layouts::scene",
	"::byte_engine::rendering::visibility::layouts::visibility_buffer",
	"::byte_engine::rendering::visibility::layouts::material_evaluation",
];

/// The layers every material closest-hit shader's `main` takes: the scene and ray-traced scene sets, which the surfel
/// shade pass takes first so its hit shaders' sets are a prefix of its own.
pub(super) const MATERIAL_HIT_LAYERS: [(&str, &str); 2] = [("scene", "Scene"), ("ray_traced_scene", "RayTracedScene")];

/// The modules a material closest-hit shader brings in: the scene records, and the modules that declare
/// [`MATERIAL_HIT_LAYERS`].
pub(super) const MATERIAL_HIT_MODULES: [&str; 3] = [
	"::byte_engine::rendering::visibility::layouts::scene_records",
	"::byte_engine::rendering::visibility::layouts::scene",
	"::byte_engine::rendering::visibility::layouts::ray_traced_scene",
];

/// The module files of [`MATERIAL_EVALUATION_MODULES`], loaded once from the embedded engine modules.
static MATERIAL_EVALUATION_MODULE_FILES: LazyLock<Vec<besl::LoadedModule>> =
	LazyLock::new(|| load_engine_modules(&MATERIAL_EVALUATION_MODULES));

/// The module files of [`MATERIAL_HIT_MODULES`], loaded once from the embedded engine modules.
pub(super) static MATERIAL_HIT_MODULE_FILES: LazyLock<Vec<besl::LoadedModule>> =
	LazyLock::new(|| load_engine_modules(&MATERIAL_HIT_MODULES));

/// Loads `modules`, paths from the asset root, and the modules they use from [`ENGINE_MODULES`].
///
/// # Panics
///
/// Panics when a module doesn't load, which is an error in the engine's own assets.
fn load_engine_modules(modules: &[&str]) -> Vec<besl::LoadedModule> {
	let source = modules.iter().map(|module| format!("use {module}::*;")).collect::<String>();
	let failed = |error: besl::CompilationError| -> ! {
		panic!(
			"The visibility layer modules failed to load ({error:?}). The most likely cause is an engine layer module that is missing from `ENGINE_MODULES` or doesn't parse. Add the module to the table, or fix its source."
		)
	};
	besl::ModuleLoader::load(&source, |file| module_source(ENGINE_MODULES, file)).unwrap_or_else(|error| failed(error))
}

/// Returns a program whose `main` takes `layers` after its own parameters, and which brings in `modules`, whose loaded
/// files are `files`.
pub(super) fn layered_program<'a>(
	mut main: Node<'a>,
	layers: &[(&'a str, &'a str)],
	modules: &[&'a str],
	files: &'a [besl::LoadedModule],
) -> Result<Node<'a>, String> {
	if let besl::parser::Nodes::Function { params, .. } = main.node_mut() {
		params.extend(layers.iter().map(|(name, r#type)| Node::parameter(name, *r#type)));
	}
	let mut children: Vec<Node<'a>> = modules.iter().map(|module| Node::r#use(module, true)).collect();
	children.push(main);
	let mut root = Node::root_with_children(children);
	besl::attach_modules(&mut root, files).map_err(|error| {
		format!(
			"Failed to attach the visibility layer modules to a generated shader ({error:?}). The most likely cause is a material parameter named like a declaration of the layer modules. Rename the parameter."
		)
	})?;
	Ok(root)
}

/// The `VisibilityShaderGenerator` struct turns BEMX material graphs into visibility material-evaluation shaders.
///
/// Install it on the material, FBX, and glTF asset handlers so every baked material targets this pipeline. Build it
/// with the project's [`VisibilityFeatures`] so material shaders carry no code for a left-out feature.
#[derive(Clone)]
pub struct VisibilityShaderGenerator {
	// Parsed once at construction so each material build only clones it.
	common: [Node<'static>; 2],
	/// The scope of the evaluation shaders the project's material evaluation mode builds. An uber shader's also
	/// declares the tile's workgroup window and reads indirect diffuse light from it.
	scope: [Node<'static>; 2],
	hit_scope: [Node<'static>; 2],
	features: VisibilityFeatures,
}

impl VisibilityShaderGenerator {
	/// Creates a generator for a project whose `render` parameters, such as `app.parameters().scope("render")`,
	/// select the features it builds in; see [`VisibilityFeatures::from_parameters`].
	///
	/// Next, pass it to [`crate::application::graphics::configure_default_baking`].
	pub fn new(parameters: impl Parameters) -> Self {
		Self::with_features(VisibilityFeatures::from_parameters(parameters))
	}

	/// Creates a generator for a project that builds in `features`. It declares read and write access to every
	/// material dispatch buffer.
	pub fn with_features(features: VisibilityFeatures) -> Self {
		Self {
			common: common_shader_module(),
			scope: visibility_shader_module(features.material_evaluation == MaterialEvaluationMode::Uber),
			hit_scope: material_hit_module(),
			features,
		}
	}

	/// Builds the uber material evaluation shader of one phase: one compute program that shades every pixel of the
	/// phase whose material has one of `arms`, in 8 x 8-pixel tiles.
	///
	/// Each thread reconstructs its pixel's surface once, runs the arm of its material's key, and lights the result
	/// with the same suffix as a per-material shader. A material's static values are literals in its arm, so the arm
	/// computes what that material's specialized pipeline computes. The opaque phase's tiles share the indirect
	/// diffuse history texels they read through workgroup memory.
	///
	/// Call it on a generator built for [`MaterialEvaluationMode::Uber`], whose scope declares the tile window. Compile it
	/// with 8 × 8-thread workgroups; the bake does so through [`ProgramGenerator::generate_assembled`].
	pub fn uber_program<'a>(&self, transparent: bool, arms: &'a [AssembledArm<'a>]) -> Result<Node<'a>, String> {
		let bodies = arms
			.iter()
			.map(|arm| {
				let (body, features) =
					material::material_body(arm.material.graph, arm.material.parameters).map_err(|error| error.to_string())?;
				Ok((arm, body, features))
			})
			.collect::<Result<Vec<_>, String>>()?;
		let features = MaterialReconstructionFeatures::union(bodies.iter().map(|(_, _, features)| *features));
		let arm_patterns = || {
			arms.iter()
				.map(|arm| MatchPattern::Literal {
					value: arm.key,
					negative: false,
				})
				.collect::<Vec<_>>()
		};

		// The phase is a literal, so each phase's shader compiles only its own path of the shared sources.
		let mut statements = vec![Node::let_assignment(
			"transparent",
			"bool",
			Node::literal_expression(if transparent { "true" } else { "false" }),
		)];
		statements.extend(material_evaluation_tile_statements(transparent));
		if transparent {
			// The instance target still holds the opaque layer where no blend surface was drawn. Those pixels return before
			// reconstructing a surface the transparent phase does not shade.
			let mut membership = Vec::new();
			if !arms.is_empty() {
				membership.push(MatchArm {
					patterns: arm_patterns(),
					statements: Vec::new(),
				});
			}
			membership.push(MatchArm {
				patterns: vec![MatchPattern::Wildcard],
				statements: vec![Node::return_void()],
			});
			statements.push(Node::r#match(Node::member_expression("material_arm"), membership));
		}
		// Texture coordinates and the tangent frame are reconstructed once, before the arms, for every arm that reads them.
		statements.extend(material_surface_statements(features));

		let mut material_arms = Vec::with_capacity(bodies.len() + 1);
		for (arm, body, features) in bodies {
			let mut arm_statements = static_value_declarations(arm)?;
			arm_statements.extend(body);
			arm_statements.extend(material_normal_statements(features));
			material_arms.push(MatchArm {
				patterns: vec![MatchPattern::Literal {
					value: arm.key,
					negative: false,
				}],
				statements: arm_statements,
			});
		}
		// The runtime draws a material only once the published shader holds its arm, so no pixel reaches this one.
		material_arms.push(MatchArm {
			patterns: vec![MatchPattern::Wildcard],
			statements: vec![Node::return_void()],
		});
		statements.push(Node::r#match(Node::member_expression("material_arm"), material_arms));
		statements.extend(material_lighting_statements(self.features));

		let main = Node::function("main", vec![Node::parameter("input", "StageInput")], "void", statements);
		let mut root = layered_program(
			main,
			&MATERIAL_EVALUATION_LAYERS,
			&MATERIAL_EVALUATION_MODULES,
			&MATERIAL_EVALUATION_MODULE_FILES,
		)?;
		root.add(self.common.iter().chain(&self.scope).cloned().collect());

		Ok(root)
	}
}

/// The workgroup of uber material evaluation shaders: one 8 x 8-pixel tile of the sink.
pub(crate) const UBER_WORKGROUP: utils::Extent = utils::Extent::square(8);

/// Returns whether `id` names the transparent phase's uber shader rather than the opaque phase's; see
/// [`UBER_MATERIAL_SHADERS`].
fn uber_phase_is_transparent(id: &str) -> Result<bool, String> {
	match UBER_MATERIAL_SHADERS.iter().position(|shader| *shader == id) {
		Some(phase) => Ok(phase == 1),
		None => Err(format!(
			"'{id}' names no uber material evaluation shader. The most likely cause is a bake request for a `.uber` resource the visibility renderer does not use. Bake {UBER_MATERIAL_SHADERS:?} instead."
		)),
	}
}

/// Returns the uber shader arm key of a material: the hash of its program's `identity`, the components of its static
/// values, and its phase.
///
/// Materials with equal keys run the same code, so they share an arm. The bake writes the key into the uber shader,
/// and the runtime into the material's entry of the evaluation table, so both must call this function. The key is
/// never `u32::MAX`, which marks a material no evaluation shades.
pub fn uber_arm_key(identity: u64, static_values: impl IntoIterator<Item = f32>, transparent: bool) -> u32 {
	use resource_management::shader::besl::evaluation::{FNV1A_OFFSET_BASIS, fnv1a};

	// FNV-1a gives the same key on every platform and in every build.
	let hash = static_values
		.into_iter()
		.fold(fnv1a(FNV1A_OFFSET_BASIS, &identity.to_le_bytes()), |hash, value| {
			fnv1a(hash, &value.to_bits().to_le_bytes())
		});
	let hash = fnv1a(hash, &[u8::from(transparent)]);
	let key = (hash ^ (hash >> 32)) as u32;
	key.min(u32::MAX - 1)
}

/// Returns the `let` statements that give an uber arm's static parameters the values of its materials.
fn static_value_declarations<'a>(arm: &AssembledArm<'a>) -> Result<Vec<Node<'a>>, String> {
	let mismatch = || {
		format!(
			"Uber arm {} has {} static values for its program's static parameters. The most likely cause is a material variant baked against another version of its material. Bake the materials again with `beld bake --force`.",
			arm.key,
			arm.material.static_values.len()
		)
	};
	let mut values = arm.material.static_values.iter();
	let mut declarations = Vec::new();
	for parameter in arm.material.parameters {
		let ParameterKind::Value(r#type) = parameter.kind else {
			continue;
		};
		let components = values.next().ok_or_else(mismatch)?;
		if components.len() != r#type.width() {
			return Err(mismatch());
		}
		// A literal folds a value that isn't finite to zero, where a specialization constant would keep it.
		if let Some(value) = components.iter().find(|value| !value.is_finite()) {
			return Err(format!(
				"A material's static value is {value}. The most likely cause is a variable value divided by zero. Give the variable a finite value."
			));
		}
		let value = bemx::besl::construct(components.iter().map(|value| bemx::besl::literal(*value)).collect());
		declarations.push(Node::let_assignment(
			parameter.name.clone(),
			bemx::besl::type_name(r#type),
			value,
		));
	}
	if values.next().is_some() {
		return Err(mismatch());
	}
	Ok(declarations)
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn generate<'a>(&self, graph: &'a bemx::Graph, parameters: &'a [MaterialParameter]) -> Result<Option<Node<'a>>, String> {
		// The uber mode bakes no shader per material; the bake keeps the material's program for the uber shaders.
		if self.features.material_evaluation == MaterialEvaluationMode::Uber {
			return Ok(None);
		}
		let (body, features) = material::material_body(graph, parameters).map_err(|error| error.to_string())?;

		let mut statements = material_evaluation_prefix_statements(features);
		statements.extend(body);
		statements.extend(material_evaluation_suffix_statements(features, self.features));

		let mut main = Node::main_function(statements);
		if let besl::parser::Nodes::Function { params, .. } = main.node_mut() {
			// The reconstruction prefix reads compute builtins through the structural entry input.
			params.push(Node::parameter("input", "StageInput"));
		}

		// The layers `main` takes declare the descriptor sets, and their modules the records the sets hold.
		let mut root = layered_program(
			main,
			&MATERIAL_EVALUATION_LAYERS,
			&MATERIAL_EVALUATION_MODULES,
			&MATERIAL_EVALUATION_MODULE_FILES,
		)?;

		// The suffix branches on `transparent`; each phase's pipeline compiles only its own path. Static value parameters
		// are specialization constants, dynamic ones are read from `material_parameters`, and texture parameters reach
		// the program as slots of `material.textures`.
		let mut declarations = vec![Node::specialization("transparent", "bool", TRANSPARENT_SPECIALIZATION_ID)];
		for (index, parameter) in (0u32..).zip(parameters) {
			if let ParameterKind::Value(r#type) = parameter.kind {
				declarations.push(Node::specialization(
					&parameter.name,
					bemx::besl::type_name(r#type),
					material_variable_specialization_id(index),
				));
			}
		}

		root.add(declarations);
		root.add(self.common.iter().chain(&self.scope).cloned().collect());

		Ok(Some(root))
	}

	/// Keeps a material in the uber shader of its phase, under the key `uber_arm_key` gives it.
	fn assembled_key(&self, id: &str, material: &AssembledMaterial<'_>) -> Result<Option<u32>, String> {
		let transparent = uber_phase_is_transparent(id)?;
		// Each phase's shader evaluates the material evaluation pass's materials of its own alpha mode.
		if material.model.name != "Visibility"
			|| material.model.pass != "MaterialEvaluation"
			|| transparent != matches!(material.alpha_mode, AlphaMode::Blend)
		{
			return Ok(None);
		}
		Ok(Some(uber_arm_key(
			material.identity,
			material.static_values.iter().flatten().copied(),
			transparent,
		)))
	}

	/// Builds the uber material evaluation shader of the asset's phase; see [`VisibilityShaderGenerator::uber_program`].
	fn generate_assembled<'a>(
		&self,
		id: &str,
		arms: &'a [AssembledArm<'a>],
	) -> Result<(Node<'a>, ShaderGenerationSettings), String> {
		let transparent = uber_phase_is_transparent(id)?;
		let name = if transparent {
			"transparent_uber_material_evaluation"
		} else {
			"opaque_uber_material_evaluation"
		};
		Ok((
			self.uber_program(transparent, arms)?,
			ShaderGenerationSettings::compute(UBER_WORKGROUP).name(name.to_string()),
		))
	}

	/// Builds the closest-hit shader that reports `graph`'s surface to the rays that hit it, when surfel GI or probe GI is
	/// built in.
	///
	/// Its `main` takes the hit as its stage input and the `HitSurface` payload, and returns the payload with the
	/// surface's diffuse albedo, emission, and normal. Static parameters are the same specialization constants as in
	/// material evaluation, so a material's hit group specializes the shader with its variant's values.
	fn generate_hit<'a>(
		&self,
		graph: &'a bemx::Graph,
		parameters: &'a [MaterialParameter],
	) -> Result<Option<Node<'a>>, String> {
		if !self.features.generates_hit_shaders() {
			return Ok(None);
		}
		let (body, features) = material::material_hit_body(graph, parameters).map_err(|error| error.to_string())?;

		let mut statements = material_hit_prefix_statements(features);
		statements.extend(body);
		statements.extend(material_hit_suffix_statements());
		let main = Node::function(
			"main",
			vec![
				Node::parameter("input", "StageInput"),
				Node::parameter("payload", "HitSurface"),
			],
			"HitSurface",
			statements,
		);

		let mut root = layered_program(main, &MATERIAL_HIT_LAYERS, &MATERIAL_HIT_MODULES, &MATERIAL_HIT_MODULE_FILES)?;
		let declarations = (0u32..)
			.zip(parameters)
			.filter_map(|(index, parameter)| match parameter.kind {
				ParameterKind::Value(r#type) => Some(Node::specialization(
					&parameter.name,
					bemx::besl::type_name(r#type),
					material_variable_specialization_id(index),
				)),
				_ => None,
			})
			.collect();
		root.add(declarations);
		root.add(self.hit_scope.to_vec());

		Ok(Some(root))
	}
}

/// Returns the intrinsic that samples a material texture with explicit derivatives, which both material evaluation and
/// closest-hit shaders read textures through.
fn sample_texture_intrinsic<'a>() -> Node<'a> {
	Node::intrinsic_with_parameters(
		"sample_texture_2d_array_grad",
		vec![
			Node::parameter("texture_array", "Texture2D"),
			Node::parameter("texture_index", "u32"),
			Node::parameter("uv", "vec2f"),
			Node::parameter("uv_derivative_x", "vec2f"),
			Node::parameter("uv_derivative_y", "vec2f"),
		],
		Node::sentence(vec![Node::member_expression("texture_array")]),
		"vec4f",
	)
}

/// Builds the `VisibilityHit` module of the helpers a material's closest-hit shader calls, which read only the scene
/// sets, followed by the `use` that lets the shader call them by name.
fn material_hit_module<'a>() -> [Node<'a>; 2] {
	let mut children = vec![sample_texture_intrinsic()];
	children.extend(
		[
			(U16_TO_U32_SOURCE, "u16_to_u32"),
			(DECODE_F16_VEC2_SOURCE, "decode_f16_vec2"),
			(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
		]
		.map(|(source, name)| parse_besl_function(source, name)),
	);
	[Node::scope("VisibilityHit", children), Node::r#use("VisibilityHit", true)]
}

/// Builds the `Visibility` module that gives material programs the helpers of the visibility lighting contract,
/// followed by the `use` that lets them call the helpers by name.
///
/// The helpers take the layers of [`MATERIAL_EVALUATION_LAYERS`] they read as parameters, and the records they read
/// come from those layers' modules, which [`VisibilityShaderGenerator`] attaches to every material's program. An uber shader's scope, `tile`, also declares the
/// tile's workgroup window of indirect diffuse texels and the helpers that read it.
pub(super) fn visibility_shader_module<'a>(tile: bool) -> [Node<'a>; 2] {
	let structs = vec![
		Node::r#struct(
			"TriangleInterpolation",
			vec![
				Node::member("origin", "vec2f"),
				Node::member("inverse_w", "vec3f"),
				Node::member("raw_ddx", "vec3f"),
				Node::member("raw_ddy", "vec3f"),
			],
		),
		// `evaluation_index` selects the pixel list this dispatch shades; every pixel reads its own material. `gtao` is
		// nonzero when GTAO wrote its image this frame. `indirect_diffuse` names the source that wrote the indirect
		// diffuse images, as `IndirectDiffuseSource::push_constant` numbers it, or is zero when none did. `ssgi` is
		// nonzero when SSGI lit the frame, so its next frame's rays need this frame's diffuse light.
		// `debug_view_first_slot` is the pool slot of the first shown debug view, and one `bool` per
		// `MATERIAL_DEBUG_VIEWS` entry says whether to write it. The phase is the `transparent` specialization instead,
		// so each phase's pipeline compiles only its own path. `MaterialEvaluationPushConstant` mirrors it.
		Node::push_constant(
			[
				Node::member("evaluation_index", "u32"),
				Node::member("gtao", "u32"),
				Node::member("ssgi", "u32"),
				Node::member("indirect_diffuse", "u32"),
				Node::member("debug_view_first_slot", "u32"),
			]
			.into_iter()
			.chain(MATERIAL_DEBUG_VIEW_MEMBERS.iter().map(|member| Node::member(member, "bool")))
			.collect(),
		),
	];

	// Texture operations that differ by API stay typed intrinsics; every other helper is authored once in BESL.
	let sample_texture = sample_texture_intrinsic();
	// Helpers are listed in dependency order: each one only calls helpers declared above it.
	let helpers = [
		(U16_TO_U32_SOURCE, "u16_to_u32"),
		(DECODE_F16_VEC2_SOURCE, "decode_f16_vec2"),
		(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
		(CONE_ATTENUATION_SOURCE, "cone_attenuation"),
		(COMPUTE_VERTEX_INDICES_SOURCE, "compute_vertex_indices"),
		(COMPUTE_TRIANGLE_INTERPOLATION_SOURCE, "compute_triangle_interpolation"),
		(FACING_NORMAL_SOURCE, "facing_normal"),
		(IES_PROFILE_UV_SOURCE, "ies_profile_uv"),
		(IES_PROFILE_SAMPLE_SOURCE, "sample_ies_profile"),
		(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient"),
		(SHADOW_POISSON_ROTATION_SOURCE, "rotate_shadow_poisson_offset"),
		(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit"),
		(DIRECTIONAL_SHADOW_BLOCKER_SOURCE, "directional_shadow_blocker_depth"),
		(DIRECTIONAL_SHADOW_PENUMBRA_SOURCE, "sample_directional_shadow_penumbra"),
		(
			DIRECTIONAL_SHADOW_TEXELS_PER_METER_SOURCE,
			"directional_shadow_texels_per_meter",
		),
		(
			DIRECTIONAL_SHADOW_DEPTH_PER_METER_SOURCE,
			"directional_shadow_depth_per_meter",
		),
		(DIRECTIONAL_SHADOW_CASCADE_VIEW_SOURCE, "directional_shadow_cascade_view"),
		(DIRECTIONAL_SHADOW_CASCADE_SCALE_SOURCE, "directional_shadow_cascade_scale"),
		(
			DIRECTIONAL_SHADOW_FITTING_CASCADE_SOURCE,
			"directional_shadow_fitting_cascade",
		),
		(DIRECTIONAL_SHADOW_CASCADE_HOLDS_SOURCE, "directional_shadow_cascade_holds"),
		(DIRECTIONAL_SHADOW_RECEIVER_SOURCE, "directional_shadow_receiver"),
		(
			DIRECTIONAL_SHADOW_RECEIVER_GRADIENT_SOURCE,
			"directional_shadow_receiver_gradient",
		),
		(
			DIRECTIONAL_SHADOW_OCCLUDER_DISTANCE_SOURCE,
			"directional_shadow_occluder_distance",
		),
		(DIRECTIONAL_SHADOW_CASCADE_SOURCE, "sample_directional_shadow_cascade"),
		(SHADOW_ROTATION_SOURCE, "compute_shadow_rotation"),
		(CONE_SHADOW_TENT_TEXEL_SOURCE, "cone_shadow_tent_texel"),
		(CONE_SHADOW_ROW_DEPTHS_SOURCE, "cone_shadow_row_depths"),
		(CONE_SHADOW_TENT_SOURCE, "sample_cone_shadow_tent"),
		(CONE_SHADOW_SOURCE, "sample_cone_shadow"),
		(DIRECTIONAL_SHADOW_SOURCE, "sample_directional_shadow"),
		(POINT_SHADOW_RECEIVER_DEPTH_SOURCE, "point_shadow_receiver_depth"),
		(POINT_SHADOW_OCCLUSION_SOURCE, "point_shadow_occlusion"),
		(POINT_SHADOW_RECEIVER_VECTOR_SOURCE, "point_shadow_receiver_vector"),
		(
			POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE,
			"point_shadow_receiver_plane_normal",
		),
		(POINT_SHADOW_TEXEL_DIRECTION_SOURCE, "point_shadow_texel_direction"),
		(POINT_SHADOW_TAP_SOURCE, "sample_point_shadow_tap"),
		(POINT_SHADOW_FACE_NORMAL_SOURCE, "point_shadow_face_normal"),
		(POINT_SHADOW_FACE_TAP_SOURCE, "sample_point_shadow_face_tap"),
		(POINT_SHADOW_POISSON_OFFSET_SOURCE, "point_shadow_poisson_offset"),
		(POINT_SHADOW_SOURCE, "sample_point_shadow"),
		(ENVIRONMENT_IRRADIANCE_SOURCE, "sample_environment_irradiance"),
		(ENVIRONMENT_SPECULAR_SOURCE, "sample_environment_specular"),
	];

	let mut children = structs;
	children.extend(screen_space_reflection_scope());
	children.extend(screen_space_indirect_diffuse_scope(tile));
	children.push(sample_texture);
	children.extend(helpers.into_iter().map(|(source, name)| parse_besl_function(source, name)));
	[Node::scope("Visibility", children), Node::r#use("Visibility", true)]
}

/// Declares the screen-space indirect diffuse helpers from [`sources`].
///
/// Opaque material evaluation reconstructs each pixel's indirect diffuse light from the half-resolution SSGI history
/// with them, so SSGI writes no full-resolution image. They read `ssgi_normals`, `ssgi_view`, `ssgi_history`, and
/// `depth_pyramid` of the material evaluation set.
///
/// An uber shader's scope, `tile`, declares the tile's workgroup window of history texels instead, and a
/// `sample_screen_space_indirect_diffuse` that reads its taps from the window, so the lighting suffix is the same.
fn screen_space_indirect_diffuse_scope(tile: bool) -> Vec<Node<'static>> {
	// The tile's 6 x 6 window of history texels, at the precision the taps read them.
	let window = std::num::NonZeroUsize::new(36);
	let window = tile.then(|| {
		[
			Node::workgroup("tile_indirect_depths", "f32", window),
			Node::workgroup("tile_indirect_normals", "vec2f", window),
			Node::workgroup("tile_indirect_lights", "vec4f", window),
		]
	});
	// Helpers follow the bindings they read, in dependency order.
	let mut helpers = vec![
		(SSGI_STORED_NORMAL_SOURCE, "ssgi_stored_normal"),
		(SSGI_SURFACE_WEIGHT_SOURCE, "ssgi_surface_weight"),
		(INDIRECT_DIFFUSE_SOURCE_POSITION_SOURCE, "indirect_diffuse_source_position"),
		(
			WEIGH_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE,
			"weigh_screen_space_indirect_diffuse",
		),
	];
	if tile {
		helpers.extend([
			(INDIRECT_DIFFUSE_WINDOW_ORIGIN_SOURCE, "indirect_diffuse_window_origin"),
			(SAMPLE_TILE_INDIRECT_DIFFUSE_SOURCE, "sample_screen_space_indirect_diffuse"),
		]);
	} else {
		helpers.push((
			SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE,
			"sample_screen_space_indirect_diffuse",
		));
	}
	window
		.into_iter()
		.flatten()
		.chain(helpers.into_iter().map(|(source, name)| parse_besl_function(source, name)))
		.collect()
}

/// Declares the screen-space reflection helpers from [`sources`], with the ray struct they share.
///
/// Material evaluation traces its reflection rays with them through `depth_pyramid`, and reads the light at their hits
/// from the previous frame's lit image. They read `reflection_parameters`, `previous_lit`, and `previous_depth` of the
/// material evaluation set.
fn screen_space_reflection_scope() -> Vec<Node<'static>> {
	let mut nodes = vec![
		// One ray's screen-space path, in normalized image coordinates. Screen position and 1/z are both linear along a
		// projected line.
		Node::r#struct(
			"ReflectionRay",
			vec![
				Node::member("start_uv", "vec2f"),
				Node::member("end_uv", "vec2f"),
				Node::member("inverse_start_z", "f32"),
				Node::member("inverse_end_z", "f32"),
			],
		),
	];
	// Helpers follow the bindings they read, in dependency order.
	nodes.extend(
		[
			(REFLECTION_RAY_IS_BEHIND_SOURCE, "reflection_ray_is_behind"),
			(REFLECTION_WORLD_FRACTION_SOURCE, "reflection_world_fraction"),
			(REFLECTION_HISTORY_DEPTH_ERROR_SOURCE, "reflection_history_depth_error"),
			(REFLECTION_HISTORY_RADIANCE_SOURCE, "reflection_history_radiance"),
			(TRACE_SCREEN_SPACE_REFLECTION_SOURCE, "trace_screen_space_reflection"),
		]
		.map(|(source, name)| parse_besl_function(source, name)),
	);
	nodes
}
