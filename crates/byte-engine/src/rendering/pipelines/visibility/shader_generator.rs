//! Turns BEMX material graphs into visibility material-evaluation compute shaders.
//!
//! [`visibility_shader_scope`] declares the buffers, images, structs, and BESL helper functions every visibility
//! shader can reference. [`VisibilityShaderGenerator`] writes the statements that compute a graph's surface, see
//! [`material`], and wraps them with the pixel reconstruction prefix and the lighting suffix from [`sources`].

mod ast;
mod material;
mod sources;
#[cfg(test)]
mod tests;

use besl::parser::Node;
use resource_management::asset::handler::implementations::bema::{MaterialParameter, ParameterKind, ProgramGenerator};
use resource_management::bemx;

use self::ast::*;
pub(crate) use self::ast::{MATERIAL_DEBUG_VIEW_NAMES, remove_statements_reading};
use self::sources::*;
use super::VisibilityFeatures;
use super::layout::{
	LIGHT_CLUSTER_COLUMNS, LIGHT_CLUSTER_MASK_WORD_COUNT, LIGHT_CLUSTER_ROWS, LIGHT_CLUSTER_SLICES,
	LOCAL_SHADOW_SIZE_CLASS_COUNT, MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS,
	MAX_PIXEL_MAPPING_ENTRIES, MIN_LOCAL_SHADOW_MAP_RESOLUTION, TRANSPARENT_SPECIALIZATION_ID,
	material_variable_specialization_id,
};
use crate::application::parameters::Parameters;
use crate::rendering::common_shader_generator::common_shader_scope;
use crate::rendering::set_layout::layer;

// The layout layers spell out their array sizes; these guards catch limit changes.
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

/// The layout layers every material-evaluation shader builds on, lowest first: the scene records, then the scene,
/// visibility buffer, and material evaluation descriptor sets.
///
/// The render pass creates those sets from the same layers, see [`super::set_layouts`], so a generated shader and the
/// sets it is bound with cannot disagree.
const MATERIAL_EVALUATION_LAYERS: [(&str, &str); 4] = [
	layer!("visibility/layouts/scene-records.besl"),
	layer!("visibility/layouts/scene.besl"),
	layer!("visibility/layouts/visibility-buffer.besl"),
	layer!("visibility/layouts/material-evaluation.besl"),
];

/// The `VisibilityShaderGenerator` struct turns BEMX material graphs into visibility material-evaluation shaders.
///
/// Install it on the material, FBX, and glTF asset handlers so every baked material targets this pipeline. Build it
/// with the project's [`VisibilityFeatures`] so material shaders carry no code for a left-out feature.
#[derive(Clone)]
pub struct VisibilityShaderGenerator {
	// Parsed once at construction so each material build only clones it.
	common: Node<'static>,
	scope: Node<'static>,
	features: VisibilityFeatures,
}

impl VisibilityShaderGenerator {
	/// Creates a generator for a project whose `render` parameters, such as `app.parameters().scope("render")`,
	/// select the features it builds in; see [`VisibilityFeatures::from_parameters`].
	///
	/// Next, pass it to [`crate::application::graphics::register_default_asset_handlers`].
	pub fn new(parameters: impl Parameters) -> Self {
		Self::with_features(VisibilityFeatures::from_parameters(parameters))
	}

	/// Creates a generator for a project that builds in `features`. It declares read and write access to every
	/// material dispatch buffer.
	pub fn with_features(features: VisibilityFeatures) -> Self {
		Self {
			common: common_shader_scope(),
			scope: visibility_shader_scope(),
			features,
		}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn generate<'a>(&self, graph: &'a bemx::Graph, parameters: &'a [MaterialParameter]) -> Result<Node<'a>, String> {
		let (body, features) = material::material_body(graph, parameters).map_err(|error| error.to_string())?;

		let mut statements = material_evaluation_prefix_statements(features);
		statements.extend(body);
		statements.extend(material_evaluation_suffix_statements(features, self.features));

		let mut main = Node::main_function(statements);
		if let besl::parser::Nodes::Function { params, .. } = main.node_mut() {
			// The reconstruction prefix reads compute builtins through the structural entry input.
			params.push(Node::parameter("input", "StageInput"));
		}

		// The layers declare the descriptor sets and the records they hold, ahead of everything that reads them.
		let mut root = besl::stack_layers(Node::root_with_children(vec![main]), MATERIAL_EVALUATION_LAYERS)
			.map_err(|error| {
				format!(
					"Failed to stack a material on the visibility layout layers ({error:?}). The most likely cause is a material parameter named like a declaration of the layers."
				)
			})?;

		// The suffix branches on `transparent`; each phase's pipeline compiles only its own path. Value parameters are
		// specialization constants, while texture parameters reach the program as slots of `material.textures`.
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
		root.add(vec![self.common.clone(), self.scope.clone()]);

		Ok(root)
	}
}

/// Builds the scope that gives material programs the helpers of the visibility lighting contract.
///
/// The descriptors and records the helpers read come from [`MATERIAL_EVALUATION_LAYERS`], which
/// [`VisibilityShaderGenerator`] stacks under every material.
pub(super) fn visibility_shader_scope<'a>() -> Node<'a> {
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
		// `evaluation_index` selects the pixel list this dispatch shades; every pixel reads its own material. `gtao` and
		// `ssgi` are nonzero when those passes wrote their images this frame. `debug_views` packs the debug views to
		// write, as `DebugViews::shown` does. The phase is the `transparent` specialization instead, so each phase's
		// pipeline compiles only its own path.
		Node::push_constant(vec![
			Node::member("evaluation_index", "u32"),
			Node::member("gtao", "u32"),
			Node::member("ssgi", "u32"),
			Node::member("debug_views", "u32"),
		]),
	];

	// Texture operations that differ by API stay typed intrinsics; every other helper is authored once in BESL.
	let sample_texture = Node::intrinsic_with_parameters(
		"sample_texture_2d_array_grad",
		vec![
			Node::parameter("texture_array", "Texture2D"),
			Node::parameter("texture_index", "u32"),
			Node::parameter("uv", "vec2f"),
			Node::parameter("uv_derivative_x", "vec2f"),
			Node::parameter("uv_derivative_y", "vec2f"),
		],
		Node::sentence(vec![Node::member_expression("textures")]),
		"vec4f",
	);
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
		(SHADOW_TAP_SOURCE, "sample_shadow_tap"),
		(SHADOW_POISSON_ROTATION_SOURCE, "rotate_shadow_poisson_offset"),
		(SHADOW_TENT_SOURCE, "sample_shadow_tent"),
		(DIRECTIONAL_SHADOW_TENT_SOURCE, "sample_directional_shadow_tent"),
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
	children.extend(screen_space_indirect_diffuse_scope());
	children.push(sample_texture);
	children.extend(helpers.into_iter().map(|(source, name)| parse_besl_function(source, name)));
	Node::scope("Visibility", children)
}

/// Declares the screen-space indirect diffuse helpers from [`sources`].
///
/// Opaque material evaluation reconstructs each pixel's indirect diffuse light from the half-resolution SSGI history
/// with them, so SSGI writes no full-resolution image. They read `ssgi_normals`, `ssgi_view`, `ssgi_history`, and
/// `depth_pyramid` of the material evaluation set.
fn screen_space_indirect_diffuse_scope() -> Vec<Node<'static>> {
	let mut nodes = Vec::new();
	// Helpers follow the bindings they read, in dependency order.
	nodes.extend(
		[
			(SSGI_STORED_NORMAL_SOURCE, "ssgi_stored_normal"),
			(SSGI_SURFACE_WEIGHT_SOURCE, "ssgi_surface_weight"),
			(
				SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE,
				"sample_screen_space_indirect_diffuse",
			),
		]
		.map(|(source, name)| parse_besl_function(source, name)),
	);
	nodes
}

/// Declares the screen-space reflection helpers from [`sources`], with the ray struct they share.
///
/// Material evaluation traces its reflection rays with them through `depth_pyramid`, and writes this frame's radiance
/// history for the next frame's rays. They read `reflection_parameters` and `previous_radiance` of the material
/// evaluation set.
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
			(REFLECTION_HISTORY_RADIANCE_SOURCE, "reflection_history_radiance"),
			(TRACE_SCREEN_SPACE_REFLECTION_SOURCE, "trace_screen_space_reflection"),
		]
		.map(|(source, name)| parse_besl_function(source, name)),
	);
	nodes
}
