use std::sync::Arc;
use std::{cell::RefCell, ops::Deref, rc::Rc, sync::OnceLock};

use besl::{parser::Node, NodeReference};
use resource_management::asset::{bema_asset_handler::ProgramGenerator, JsonObject};
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use crate::rendering::common_shader_generator::CommonShaderScope;
use crate::rendering::pipelines::visibility::{
	MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
};

fn light_array_type() -> &'static str {
	static LIGHT_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	LIGHT_ARRAY_TYPE
		.get_or_init(|| format!("Light[{MAX_LIGHTS}]").into_boxed_str())
		.as_ref()
}

fn material_array_type() -> &'static str {
	static MATERIAL_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_ARRAY_TYPE
		.get_or_init(|| format!("Material[{MAX_MATERIALS}]").into_boxed_str())
		.as_ref()
}

fn material_texture_array_type() -> &'static str {
	static MATERIAL_TEXTURE_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_TEXTURE_ARRAY_TYPE
		.get_or_init(|| format!("u32[{MAX_MATERIAL_TEXTURES}]").into_boxed_str())
		.as_ref()
}

fn vertex_vec3_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec3f[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_normal_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE
		.get_or_init(|| {
			format!(
				"{}[{MAX_VERTICES}]",
				crate::rendering::pipelines::visibility::VERTEX_NORMAL_SHADER_TYPE
			)
			.into_boxed_str()
		})
		.as_ref()
}

fn vertex_uv_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE
		.get_or_init(|| {
			format!(
				"{}[{MAX_VERTICES}]",
				crate::rendering::pipelines::visibility::VERTEX_UV_SHADER_TYPE
			)
			.into_boxed_str()
		})
		.as_ref()
}

fn skinned_vertex_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("SkinnedVertex[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u16[{MAX_PRIMITIVE_TRIANGLES}]").into_boxed_str())
}

fn primitive_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u8[{}]", MAX_TRIANGLES * 3).into_boxed_str())
}

fn meshlet_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("Meshlet[{MAX_MESHLETS}]").into_boxed_str())
}

/// Parses one reusable BESL helper function from an isolated source scope.
fn parse_besl_function(source: &'static str, function_name: &str) -> besl::parser::Node<'static> {
	let mut root = besl::parse(source).unwrap_or_else(|error| {
		panic!(
			"Failed to parse `{function_name}`. The most likely cause is invalid BESL syntax in the visibility shader module: {error:?}"
		)
	});

	match root.node_mut() {
		besl::parser::Nodes::Scope { children, .. } if children.len() == 1 => children.remove(0),
		_ => panic!(
			"Invalid `{function_name}` helper scope. The most likely cause is that its BESL source defines more than one top-level element."
		),
	}
}

/// Extracts reusable statements from one portable BESL helper function.
fn parse_besl_statements(source: &'static str, function_name: &str) -> Vec<besl::parser::Node<'static>> {
	let mut function = parse_besl_function(source, function_name);
	match function.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => std::mem::take(statements),
		_ => panic!(
			"Invalid `{function_name}` helper. The most likely cause is that its BESL source no longer defines a function."
		),
	}
}

#[derive(Clone, Copy, Default)]
struct MaterialReconstructionFeatures {
	uses_uv: bool,
	uses_tangent_frame: bool,
}

fn material_reconstruction_features(node: &besl::parser::Node<'_>) -> MaterialReconstructionFeatures {
	let mut features = MaterialReconstructionFeatures::default();
	collect_material_reconstruction_features(node, &mut features);
	features
}

/// Finds material sampling operations before texture shorthand is expanded.
fn collect_material_reconstruction_features(node: &besl::parser::Node<'_>, features: &mut MaterialReconstructionFeatures) {
	match node.node() {
		besl::parser::Nodes::Function { statements, .. } => {
			for statement in statements {
				collect_material_reconstruction_features(statement, features);
			}
		}
		besl::parser::Nodes::Conditional { condition, statements } => {
			collect_material_reconstruction_features(condition, features);
			for statement in statements {
				collect_material_reconstruction_features(statement, features);
			}
		}
		besl::parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_material_reconstruction_features(initializer, features);
			collect_material_reconstruction_features(condition, features);
			collect_material_reconstruction_features(update, features);
			for statement in statements {
				collect_material_reconstruction_features(statement, features);
			}
		}
		besl::parser::Nodes::Expression(expression) => {
			collect_material_expression_features(expression, features);
		}
		_ => {}
	}
}

/// Tracks the operations whose generated helpers require UVs or a tangent frame.
fn collect_material_expression_features(
	expression: &besl::parser::Expressions<'_>,
	features: &mut MaterialReconstructionFeatures,
) {
	match expression {
		besl::parser::Expressions::Call { name, parameters } => {
			if let besl::parser::TypeName::Named(name) = name {
				if matches!(
					*name,
					"sample_material"
						| "sample_normal" | "decode_material_normal"
						| "decode_material_normal_f16"
						| "scale_normal_xy"
						| "scale_material_normal_xy_f16"
				) {
					features.uses_uv = true;
				}
				if matches!(
					*name,
					"sample_normal"
						| "decode_material_normal"
						| "decode_material_normal_f16"
						| "scale_normal_xy"
						| "scale_material_normal_xy_f16"
				) {
					features.uses_tangent_frame = true;
				}
			}
			for parameter in parameters {
				collect_material_reconstruction_features(parameter, features);
			}
		}
		besl::parser::Expressions::Expression(elements) => {
			for element in elements {
				collect_material_reconstruction_features(element, features);
			}
		}
		besl::parser::Expressions::Operator { name, left, right } => {
			if *name == "="
				&& matches!(
					left.node(),
					besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "normal"
				) && !is_default_tangent_normal(right)
			{
				features.uses_uv = true;
				features.uses_tangent_frame = true;
			}
			collect_material_reconstruction_features(left, features);
			collect_material_reconstruction_features(right, features);
		}
		besl::parser::Expressions::Accessor { left, right } => {
			collect_material_reconstruction_features(left, features);
			collect_material_reconstruction_features(right, features);
		}
		besl::parser::Expressions::Return { value } => {
			if let Some(value) = value {
				collect_material_reconstruction_features(value, features);
			}
		}
		besl::parser::Expressions::Macro { body, .. } => collect_material_reconstruction_features(body, features),
		besl::parser::Expressions::Member { name } => {
			if name == "vertex_uv" {
				features.uses_uv = true;
			}
			if matches!(name.as_ref(), "T" | "B") {
				features.uses_uv = true;
				features.uses_tangent_frame = true;
			}
		}
		besl::parser::Expressions::Literal { .. }
		| besl::parser::Expressions::VariableDeclaration { .. }
		| besl::parser::Expressions::RawCode { .. }
		| besl::parser::Expressions::Continue => {}
	}
}

/// Recognizes the canonical no-normal-map value emitted by the BRDF generators.
fn is_default_tangent_normal(node: &besl::parser::Node<'_>) -> bool {
	let besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters }) = node.node() else {
		return false;
	};
	if !matches!(name, besl::parser::TypeName::Named(name) if matches!(*name, "vec3f" | "vec3f16")) || parameters.len() != 3 {
		return false;
	}

	parameters.iter().zip([0.0_f32, 0.0, 1.0]).all(|(parameter, expected)| {
		let literal = match parameter.node() {
			besl::parser::Nodes::Expression(besl::parser::Expressions::Literal { value }) => Some(value),
			besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters })
				if matches!(name, besl::parser::TypeName::Named(name) if *name == "f16") && parameters.len() == 1 =>
			{
				match parameters[0].node() {
					besl::parser::Nodes::Expression(besl::parser::Expressions::Literal { value }) => Some(value),
					_ => None,
				}
			}
			_ => None,
		};
		literal.is_some_and(|value| value.parse::<f32>().is_ok_and(|value| value == expected))
	})
}

fn material_evaluation_prefix_statements(features: MaterialReconstructionFeatures) -> Vec<besl::parser::Node<'static>> {
	let mut statements = parse_besl_statements(MATERIAL_EVALUATION_PREFIX_SOURCE, "material_evaluation_prefix");
	if features.uses_uv {
		statements.extend(parse_besl_statements(MATERIAL_EVALUATION_UV_SOURCE, "material_evaluation_uv"));
	}
	if features.uses_tangent_frame {
		statements.extend(parse_besl_statements(
			MATERIAL_EVALUATION_TANGENT_SOURCE,
			"material_evaluation_tangent",
		));
	}
	statements.extend(parse_besl_statements(
		MATERIAL_EVALUATION_DEFAULTS_SOURCE,
		"material_evaluation_defaults",
	));
	statements
}

fn material_evaluation_suffix_statements(features: MaterialReconstructionFeatures) -> Vec<besl::parser::Node<'static>> {
	let normal_source = if features.uses_tangent_frame {
		MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE
	} else {
		MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE
	};
	let mut statements = parse_besl_statements(normal_source, "material_evaluation_normal");
	statements.extend(parse_besl_statements(
		MATERIAL_EVALUATION_SUFFIX_SOURCE,
		"material_evaluation_suffix",
	));
	statements
}

/// Narrows material properties at the authored-program boundary so every material graph uses the compact evaluation ABI.
fn narrow_material_property_assignments(node: &mut besl::parser::Node<'_>) {
	match node.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => {
			for statement in statements {
				narrow_material_property_assignments(statement);
			}
		}
		besl::parser::Nodes::Conditional { condition, statements } => {
			narrow_material_property_assignments(condition);
			for statement in statements {
				narrow_material_property_assignments(statement);
			}
		}
		besl::parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			narrow_material_property_assignments(initializer);
			narrow_material_property_assignments(condition);
			narrow_material_property_assignments(update);
			for statement in statements {
				narrow_material_property_assignments(statement);
			}
		}
		besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { name, left, right }) => {
			if *name == "=" {
				let target_type = match left.node() {
					besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) => match name.as_ref() {
						"albedo" => Some("vec4f16"),
						"normal" | "emission" => Some("vec3f16"),
						"metalness" | "roughness" | "occlusion" => Some("f16"),
						_ => None,
					},
					_ => None,
				};
				if let Some(target_type) = target_type {
					*right = Box::new(besl::parser::Node::call(target_type, vec![*right.clone()]));
				}
			}
			narrow_material_property_assignments(left);
			narrow_material_property_assignments(right);
		}
		besl::parser::Nodes::Expression(expression) => narrow_material_property_assignment_expression(expression),
		_ => {}
	}
}

/// Recurses through nested authored expressions while preserving assignments that require material narrowing.
fn narrow_material_property_assignment_expression(expression: &mut besl::parser::Expressions<'_>) {
	match expression {
		besl::parser::Expressions::Call { parameters, .. } | besl::parser::Expressions::Expression(parameters) => {
			for parameter in parameters {
				narrow_material_property_assignments(parameter);
			}
		}
		besl::parser::Expressions::Accessor { left, right } | besl::parser::Expressions::Operator { left, right, .. } => {
			narrow_material_property_assignments(left);
			narrow_material_property_assignments(right);
		}
		besl::parser::Expressions::Return { value } => {
			if let Some(value) = value {
				narrow_material_property_assignments(value);
			}
		}
		besl::parser::Expressions::Macro { body, .. } => narrow_material_property_assignments(body),
		besl::parser::Expressions::Member { .. }
		| besl::parser::Expressions::Literal { .. }
		| besl::parser::Expressions::VariableDeclaration { .. }
		| besl::parser::Expressions::RawCode { .. }
		| besl::parser::Expressions::Continue => {}
	}
}

/// Makes material texture context explicit before the parser tree is linked.
fn add_material_sample_context(node: &mut besl::parser::Node<'_>, texture_slots: &[(&str, u32)]) {
	match node.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => {
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::Conditional { condition, statements } => {
			add_material_sample_context(condition, texture_slots);
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			add_material_sample_context(initializer, texture_slots);
			add_material_sample_context(condition, texture_slots);
			add_material_sample_context(update, texture_slots);
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::Expression(expression) => add_material_sample_context_to_expression(expression, texture_slots),
		_ => {}
	}
}

/// Recurses through one expression and expands the material sampling shorthand.
fn add_material_sample_context_to_expression(expression: &mut besl::parser::Expressions<'_>, texture_slots: &[(&str, u32)]) {
	match expression {
		besl::parser::Expressions::Call { name, parameters } => {
			for parameter in parameters.iter_mut() {
				add_material_sample_context(parameter, texture_slots);
			}
			let besl::parser::TypeName::Named(name) = name else {
				return;
			};
			if !matches!(*name, "sample_material" | "sample_normal") || parameters.len() != 1 {
				return;
			}

			let slot = parameters.remove(0);
			let slot = match slot.node() {
				besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) => texture_slots
					.iter()
					.find_map(|(texture_name, index)| (*texture_name == *name).then_some(*index))
					.map_or(slot, |index| Node::literal_expression(format!("{index}u"))),
				_ => slot,
			};
			let material_textures = Node::accessor(Node::member_expression("material"), Node::member_expression("textures"));
			// Index access has an expression on its right side. Preserve that shape so
			// the linked backend AST can distinguish `textures[slot]` from `.field`.
			parameters.push(Node::accessor(material_textures, Node::sentence(vec![slot])));
			parameters.push(Node::member_expression("vertex_uv"));
		}
		besl::parser::Expressions::Expression(elements) => {
			for element in elements {
				add_material_sample_context(element, texture_slots);
			}
		}
		besl::parser::Expressions::Accessor { left, right } | besl::parser::Expressions::Operator { left, right, .. } => {
			add_material_sample_context(left, texture_slots);
			add_material_sample_context(right, texture_slots);
		}
		besl::parser::Expressions::Return { value } => {
			if let Some(value) = value {
				add_material_sample_context(value, texture_slots);
			}
		}
		besl::parser::Expressions::Macro { body, .. } => add_material_sample_context(body, texture_slots),
		besl::parser::Expressions::Member { .. }
		| besl::parser::Expressions::Literal { .. }
		| besl::parser::Expressions::VariableDeclaration { .. }
		| besl::parser::Expressions::RawCode { .. }
		| besl::parser::Expressions::Continue => {}
	}
}

// These statements are spliced around the material-authored main body. Keeping
// them in BESL lets each backend derive resource access, packed loads, type names,
// and matrix multiplication from the linked AST.
const DECODE_UNORM16_VEC2_SOURCE: &str = r#"
decode_unorm16_vec2: fn (encoded: vec2u16) -> vec2f {
	return vec2f(f32(u32(encoded.x)), f32(u32(encoded.y))) / 65535.0;
}
"#;

const DECODE_OCTAHEDRAL_NORMAL_SOURCE: &str = r#"
decode_octahedral_normal: fn (encoded: vec2u16) -> vec3f {
	let octahedral: vec2f = decode_unorm16_vec2(encoded) * 2.0 - vec2f(1.0, 1.0);
	let normal_z: f32 = 1.0 - abs(octahedral.x) - abs(octahedral.y);
	let fold: f32 = max(0.0 - normal_z, 0.0);
	// `step` returns the positive direction at zero, matching the CPU encoder's fold convention.
	return vec3f(
		octahedral.x - (step(0.0, octahedral.x) * 2.0 - 1.0) * fold,
		octahedral.y - (step(0.0, octahedral.y) * 2.0 - 1.0) * fold,
		normal_z
	);
}
"#;

const MATERIAL_EVALUATION_PREFIX_SOURCE: &str = r#"
material_evaluation_prefix: fn () -> void {
	let invocation: vec2u = thread_id();
	if (invocation.x >= material_evaluation_dispatches.material_evaluation_dispatches[push_constant.material_id].w) {
		return;
	}

	let offset: u32 = material_offset.material_offset[push_constant.material_id];
	let packed_pixel_coordinates: vec2u16 = pixel_mapping.pixel_mapping[offset + invocation.x];
	let raw_pixel_coordinates: vec2u = vec2u(
		u32(packed_pixel_coordinates.x),
		u32(packed_pixel_coordinates.y)
	);
	if (raw_pixel_coordinates.x == 0 || raw_pixel_coordinates.y == 0) {
		return;
	}
	let pixel_coordinates: vec2u = raw_pixel_coordinates - vec2u(1, 1);
	let image_extent: vec2u = image_size(triangle_index);
	if (pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) {
		return;
	}

	let triangle_meshlet_indices: u32 = image_load_u32(triangle_index, pixel_coordinates);
	let instance_index: u32 = image_load_u32(instance_index_render_target, pixel_coordinates);
	let meshlet_triangle_index: u32 = triangle_meshlet_indices & 255;
	let meshlet_index: u32 = triangle_meshlet_indices >> 8;
	let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
	let mesh: Mesh = meshes.meshes[instance_index];
	let material: Material = materials.materials[push_constant.material_id];

	let primitive_index_base: u32 = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3;
	let primitive_index0: u32 = u32(primitive_indices.primitive_indices[primitive_index_base]);
	let primitive_index1: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 1]);
	let primitive_index2: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 2]);
	let vertex_index0: u32 = compute_vertex_index(mesh, meshlet, primitive_index0);
	let vertex_index1: u32 = compute_vertex_index(mesh, meshlet, primitive_index1);
	let vertex_index2: u32 = compute_vertex_index(mesh, meshlet, primitive_index2);

	let model_space_vertex_position0: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let model_space_vertex_position1: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let model_space_vertex_position2: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let vertex_normal0: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);
	let vertex_normal1: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);
	let vertex_normal2: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);

	if (mesh.skinned_base_vertex_index != 4294967295) {
		let skinned_vertex_index0: u32 = mesh.skinned_base_vertex_index + (vertex_index0 - mesh.base_vertex_index);
		let skinned_vertex_index1: u32 = mesh.skinned_base_vertex_index + (vertex_index1 - mesh.base_vertex_index);
		let skinned_vertex_index2: u32 = mesh.skinned_base_vertex_index + (vertex_index2 - mesh.base_vertex_index);
		let skinned_vertex0: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index0];
		let skinned_vertex1: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index1];
		let skinned_vertex2: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index2];
		model_space_vertex_position0 = skinned_vertex0.position;
		model_space_vertex_position1 = skinned_vertex1.position;
		model_space_vertex_position2 = skinned_vertex2.position;
		vertex_normal0 = skinned_vertex0.normal;
		vertex_normal1 = skinned_vertex1.normal;
		vertex_normal2 = skinned_vertex2.normal;
	}
	if (mesh.skinned_base_vertex_index == 4294967295) {
		let position0: vec3f = vertex_positions.positions[vertex_index0];
		let position1: vec3f = vertex_positions.positions[vertex_index1];
		let position2: vec3f = vertex_positions.positions[vertex_index2];
		let normal0: vec3f = decode_octahedral_normal(vertex_normals.normals[vertex_index0]);
		let normal1: vec3f = decode_octahedral_normal(vertex_normals.normals[vertex_index1]);
		let normal2: vec3f = decode_octahedral_normal(vertex_normals.normals[vertex_index2]);
		model_space_vertex_position0 = vec4f(position0.x, position0.y, position0.z, 1.0);
		model_space_vertex_position1 = vec4f(position1.x, position1.y, position1.z, 1.0);
		model_space_vertex_position2 = vec4f(position2.x, position2.y, position2.z, 1.0);
		vertex_normal0 = vec4f(normal0.x, normal0.y, normal0.z, 0.0);
		vertex_normal1 = vec4f(normal1.x, normal1.y, normal1.z, 0.0);
		vertex_normal2 = vec4f(normal2.x, normal2.y, normal2.z, 0.0);
	}
	let nc: vec2f = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);
	let view: View = views.views[0];
	let model: mat4x3f = mesh.model;
	let world_space_vertex_position0: vec3f = model * model_space_vertex_position0;
	let world_space_vertex_position1: vec3f = model * model_space_vertex_position1;
	let world_space_vertex_position2: vec3f = model * model_space_vertex_position2;
	let clip_space_vertex_position0: vec4f = view.view_projection * vec4f(
		world_space_vertex_position0.x,
		world_space_vertex_position0.y,
		world_space_vertex_position0.z,
		1.0
	);
	let clip_space_vertex_position1: vec4f = view.view_projection * vec4f(
		world_space_vertex_position1.x,
		world_space_vertex_position1.y,
		world_space_vertex_position1.z,
		1.0
	);
	let clip_space_vertex_position2: vec4f = view.view_projection * vec4f(
		world_space_vertex_position2.x,
		world_space_vertex_position2.y,
		world_space_vertex_position2.z,
		1.0
	);
	let world_space_vertex_normal0: vec3f = normalize(model * vertex_normal0);
	let world_space_vertex_normal1: vec3f = normalize(model * vertex_normal1);
	let world_space_vertex_normal2: vec3f = normalize(model * vertex_normal2);

	let barycentric_deriv: BarycentricDeriv = calculate_full_bary(
		clip_space_vertex_position0,
		clip_space_vertex_position1,
		clip_space_vertex_position2,
		nc,
		vec2f(f32(image_extent.x), f32(image_extent.y))
	);
	let barycenter: vec3f = barycentric_deriv.lambda;
	let derivative_x: vec3f = barycentric_deriv.ddx;
	let derivative_y: vec3f = barycentric_deriv.ddy;
	let world_space_vertex_position: vec3f = interpolate_vec3f_with_deriv(
		barycenter,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
	let world_space_vertex_normal: vec3f = normalize(interpolate_vec3f_with_deriv(
		barycenter,
		world_space_vertex_normal0,
		world_space_vertex_normal1,
		world_space_vertex_normal2
	));
	let N: vec3f = world_space_vertex_normal;
	let camera_position: vec3f = view.inverse_view * vec4f(0.0, 0.0, 0.0, 1.0);
	let V: vec3f = normalize(camera_position - world_space_vertex_position);
	let position_derivative_x: vec3f = interpolate_vec3f_with_deriv(
		derivative_x,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
	let position_derivative_y: vec3f = interpolate_vec3f_with_deriv(
		derivative_y,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
}
"#;

const MATERIAL_EVALUATION_UV_SOURCE: &str = r#"
material_evaluation_uv: fn () -> void {
	// Runtime UVs use 16-bit UNORM storage and are expanded only for materials that sample them.
	let vertex_uv0: vec2f = decode_unorm16_vec2(vertex_uvs.uvs[vertex_index0]);
	let vertex_uv1: vec2f = decode_unorm16_vec2(vertex_uvs.uvs[vertex_index1]);
	let vertex_uv2: vec2f = decode_unorm16_vec2(vertex_uvs.uvs[vertex_index2]);
	let vertex_uv: vec2f = interpolate_vec2f_with_deriv(barycenter, vertex_uv0, vertex_uv1, vertex_uv2);
}
"#;

const MATERIAL_EVALUATION_TANGENT_SOURCE: &str = r#"
material_evaluation_tangent: fn () -> void {
	let uv_derivative_x: vec2f = interpolate_vec2f_with_deriv(derivative_x, vertex_uv0, vertex_uv1, vertex_uv2);
	let uv_derivative_y: vec2f = interpolate_vec2f_with_deriv(derivative_y, vertex_uv0, vertex_uv1, vertex_uv2);
	let tangent_scale: f32 = 1.0 / (uv_derivative_x.x * uv_derivative_y.y - uv_derivative_y.x * uv_derivative_x.y);
	let T: vec3f = normalize(
		tangent_scale * (uv_derivative_y.y * position_derivative_x - uv_derivative_x.y * position_derivative_y)
	);
	let B: vec3f = normalize(
		tangent_scale * ((0.0 - uv_derivative_y.x) * position_derivative_x + uv_derivative_x.x * position_derivative_y)
	);
}
"#;

const MATERIAL_EVALUATION_DEFAULTS_SOURCE: &str = r#"
material_evaluation_defaults: fn () -> void {
	// Material inputs are normalized or artist-bounded values. Keep them compact until lighting needs f32 range.
	let albedo: vec4f16 = vec4f16(1.0, 0.0, 0.0, 1.0);
	let normal: vec3f16 = vec3f16(0.0, 0.0, 1.0);
	let metalness: f16 = 0.0;
	let roughness: f16 = 0.5;
	let occlusion: f16 = 1.0;
	let emission: vec3f16 = vec3f16(0.0, 0.0, 0.0);
}
"#;

const MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = vec3f16(normalize(f32(normal.x) * T + f32(normal.y) * B + f32(normal.z) * N));
}
"#;

const MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = vec3f16(N);
}
"#;

const MATERIAL_EVALUATION_SUFFIX_SOURCE: &str = r#"
material_evaluation_suffix: fn () -> void {
	// Preserve compact material values and normalized vectors through the BRDF.
	// Positions, shadow projections, HDR radiance, and accumulation remain f32.
	let albedo_rgb: vec3f16 = vec3f16(albedo.x, albedo.y, albedo.z);
	let V_material: vec3f16 = vec3f16(V);
	let F0: vec3f16 = vec3f16(0.04, 0.04, 0.04) * (1.0 - metalness) + albedo_rgb * metalness;
	let NdotV: f16 = max(dot(normal, V_material), f16(0.0));
	let roughness_alpha: f16 = roughness * roughness;
	let roughness_alpha_squared: f16 = roughness_alpha * roughness_alpha;
	let adjusted_roughness: f16 = roughness + 1.0;
	let geometry_k: f16 = adjusted_roughness * adjusted_roughness / 8.0;
	let diffuse: vec3f = vec3f(0.0, 0.0, 0.0);
	let specular: vec3f = vec3f(0.0, 0.0, 0.0);
	let ao_factor: f16 = 1.0;
	if (push_constant.blend == 0) {
		ao_factor = f16(fetch(ao, pixel_coordinates).x);
	}
	let view_fresnel_base: f16 = clamp(f16(1.0) - NdotV, f16(0.0), f16(1.0));
	let view_fresnel_squared: f16 = view_fresnel_base * view_fresnel_base;
	let view_fresnel_factor: f16 = view_fresnel_squared * view_fresnel_squared * view_fresnel_base;
	let one_minus_fresnel_n_dot_v: vec3f16 = vec3f16(1.0, 1.0, 1.0) - (F0 + (vec3f16(1.0, 1.0, 1.0) - F0) * view_fresnel_factor);
	// These terms depend only on the shaded pixel. Evaluate them once instead of once per light.
	let geometry_view: f16 = NdotV / (NdotV * (1.0 - geometry_k) + geometry_k);
	let view_space_surface_position: vec3f = view.view * vec4f(
		world_space_vertex_position.x,
		world_space_vertex_position.y,
		world_space_vertex_position.z,
		1.0
	);
	let light_count: u32 = lighting_data.light_count;

	for (let light_index: u32 = 0; light_index < light_count; light_index = light_index + 1) {
		let light: Light = lighting_data.lights[light_index];
		let L: vec3f = vec3f(0.0, 0.0, 0.0);
		let attenuation: f32 = 1.0;
		let light_position: vec3f = vec3f(light.position.x, light.position.y, light.position.z);
		if (light.type == 68) {
			L = normalize(vec3f(0.0, 0.0, 0.0) - light_position);
		}
		if (light.type != 68) {
			let surface_to_light: vec3f = light_position - world_space_vertex_position;
			let distance_squared: f32 = dot(surface_to_light, surface_to_light);
			if (distance_squared <= 0.0) {
				continue;
			}
			L = surface_to_light * inversesqrt(distance_squared);
			attenuation = 1.0 / distance_squared;
		}

		let L_material: vec3f16 = vec3f16(L);
		let NdotL: f16 = max(dot(normal, L_material), f16(0.0));
		if (NdotL <= 0.0) {
			continue;
		}

		let occlusion_factor: f16 = 1.0;
		if (light.type == 68) {
			occlusion_factor = f16(sample_shadow(
				depth_shadow_map,
				light,
				world_space_vertex_position,
				view_space_surface_position,
				world_space_vertex_normal,
				L,
				vec3f(0.0, 0.0, 0.0),
				vec3f(0.0, 0.0, 0.0)
			));
			if (occlusion_factor == 0.0) {
				continue;
			}
			attenuation = 1.0;
		}
		if (light.type != 68) {
			if (light.type == 1) {
			let light_direction: vec3f = vec3f(light.direction.x, light.direction.y, light.direction.z);
			let cone_cosine: f16 = dot(vec3f16(normalize(light_direction)), vec3f16(0.0, 0.0, 0.0) - L_material);
			let cone_factor: f16 = f16(cone_attenuation(f32(cone_cosine), light.cone_cosines.x, light.cone_cosines.y));
			if (cone_factor <= 0.0) {
				continue;
			}
			attenuation = attenuation * f32(cone_factor);
			occlusion_factor = f16(sample_shadow(
				cone_shadow_map,
				light,
				world_space_vertex_position,
				view_space_surface_position,
				world_space_vertex_normal,
				L,
				position_derivative_x,
				position_derivative_y
			));
			if (occlusion_factor == 0.0) {
				continue;
			}
			}
		}

		let H: vec3f16 = normalize(V_material + L_material);
		let half_view_fresnel_base: f16 = clamp(f16(1.0) - max(dot(H, V_material), f16(0.0)), f16(0.0), f16(1.0));
		let half_view_fresnel_squared: f16 = half_view_fresnel_base * half_view_fresnel_base;
		let half_view_fresnel_factor: f16 = half_view_fresnel_squared * half_view_fresnel_squared * half_view_fresnel_base;
		let F: vec3f16 = F0 + (vec3f16(1.0, 1.0, 1.0) - F0) * half_view_fresnel_factor;
		let NdotH: f16 = max(dot(normal, H), f16(0.0));
		let denominator_base: f16 = NdotH * NdotH * (roughness_alpha_squared - 1.0) + 1.0;
		let NDF: f16 = roughness_alpha_squared / (3.14159265359 * denominator_base * denominator_base);
		let geometry_light: f16 = NdotL / (NdotL * (1.0 - geometry_k) + geometry_k);
		let local_specular: vec3f16 = (NDF * geometry_view * geometry_light * F) / (4.0 * NdotV * NdotL + 0.000001);
		let light_fresnel_base: f16 = clamp(f16(1.0) - NdotL, f16(0.0), f16(1.0));
		let light_fresnel_squared: f16 = light_fresnel_base * light_fresnel_base;
		let light_fresnel_factor: f16 = light_fresnel_squared * light_fresnel_squared * light_fresnel_base;
		let kD: vec3f16 = (vec3f16(1.0, 1.0, 1.0) - (F0 + (vec3f16(1.0, 1.0, 1.0) - F0) * light_fresnel_factor))
			* one_minus_fresnel_n_dot_v
			* (1.0 - metalness);
		let local_diffuse: vec3f16 = kD * albedo_rgb / 3.14159265359;
		let light_color: vec3f = vec3f(light.color.x, light.color.y, light.color.z);
		let irradiance: vec3f = light_color * (attenuation * f32(NdotL * occlusion_factor));
		diffuse = diffuse + vec3f(local_diffuse) * irradiance;
		specular = specular + vec3f(local_specular) * irradiance;
	}

	let ambient_irradiance: vec3f = sample_environment_irradiance(vec3f(normal));
	let incident: vec3f = vec3f(0.0, 0.0, 0.0) - V;
	let reflection_direction: vec3f = incident - 2.0 * dot(incident, vec3f(normal)) * vec3f(normal);
	let reflection_radiance: vec3f = sample_environment_specular(reflection_direction, f32(roughness));
	let grazing: vec3f16 = vec3f16(max(f16(1.0) - roughness, F0.x), max(f16(1.0) - roughness, F0.y), max(f16(1.0) - roughness, F0.z));
	let F_ibl: vec3f16 = F0 + (grazing - F0) * view_fresnel_factor;
	let kD_ibl: vec3f16 = (vec3f16(1.0, 1.0, 1.0) - F_ibl) * (1.0 - metalness);
	let ibl_diffuse: vec3f = vec3f(kD_ibl * albedo_rgb) * ambient_irradiance;

	let c0: vec4f16 = vec4f16(0.0 - 1.0, 0.0 - 0.0275, 0.0 - 0.572, 0.022);
	let c1: vec4f16 = vec4f16(1.0, 0.0425, 1.04, 0.0 - 0.04);
	let r: vec4f16 = roughness * c0 + c1;
	let a004: f16 = min(r.x * r.x, pow(f16(2.0), (f16(0.0) - f16(9.28)) * NdotV)) * r.x + r.y;
	let env_brdf: vec2f16 = vec2f16(0.0 - 1.04, 1.04) * a004 + vec2f16(r.z, r.w);
	let ibl_specular: vec3f = vec3f(F0 * env_brdf.x + env_brdf.y) * reflection_radiance;
	let ambient: vec3f = ibl_diffuse + ibl_specular;
	ao_factor = ao_factor * occlusion;
	let lit: vec3f = (diffuse + specular) * f32(ao_factor) + ambient * f32(ao_factor) + vec3f(emission);
	let output_color: vec4f = vec4f(lit.x, lit.y, lit.z, 1.0);
	if (push_constant.blend != 0) {
		let source_alpha: f32 = f32(clamp(albedo.w, f16(0.0), f16(1.0)));
		let destination_color: vec4f = image_load(lit_map, pixel_coordinates);
		output_color = source_over(
			vec4f(lit.x * source_alpha, lit.y * source_alpha, lit.z * source_alpha, source_alpha),
			destination_color
		);
	}
	write(lit_map, pixel_coordinates, output_color);
}
"#;

// Computes the projected receiver plane so cone PCF compares each texel against the depth at that texel's center.
// Returning no correction for a degenerate projection keeps the existing bias as a safe fallback.
const SHADOW_RECEIVER_PLANE_SOURCE: &str = r#"
shadow_receiver_plane_depth_gradient: fn (
	shadow_view_projection: mat4f,
	surface_light_clip_position: vec4f,
	surface_light_ndc_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> vec2f {
	let light_clip_derivative_x: vec4f = shadow_view_projection * vec4f(
		world_space_position_derivative_x.x,
		world_space_position_derivative_x.y,
		world_space_position_derivative_x.z,
		0.0
	);
	let light_clip_derivative_y: vec4f = shadow_view_projection * vec4f(
		world_space_position_derivative_y.x,
		world_space_position_derivative_y.y,
		world_space_position_derivative_y.z,
		0.0
	);
	let light_ndc_derivative_x: vec3f = (
		vec3f(light_clip_derivative_x.x, light_clip_derivative_x.y, light_clip_derivative_x.z)
		- surface_light_ndc_position * light_clip_derivative_x.w
	) / surface_light_clip_position.w;
	let light_ndc_derivative_y: vec3f = (
		vec3f(light_clip_derivative_y.x, light_clip_derivative_y.y, light_clip_derivative_y.z)
		- surface_light_ndc_position * light_clip_derivative_y.w
	) / surface_light_clip_position.w;
	let shadow_uv_derivative_x: vec2f = vec2f(
		light_ndc_derivative_x.x * 0.5,
		0.0 - light_ndc_derivative_x.y * 0.5
	);
	let shadow_uv_derivative_y: vec2f = vec2f(
		light_ndc_derivative_y.x * 0.5,
		0.0 - light_ndc_derivative_y.y * 0.5
	);
	let shadow_uv_determinant: f32 = shadow_uv_derivative_x.x * shadow_uv_derivative_y.y
		- shadow_uv_derivative_y.x * shadow_uv_derivative_x.y;
	if (abs(shadow_uv_determinant) <= 0.0000000001) {
		return vec2f(0.0, 0.0);
	}
	return vec2f(
		(light_ndc_derivative_x.z * shadow_uv_derivative_y.y
			- light_ndc_derivative_y.z * shadow_uv_derivative_x.y) / shadow_uv_determinant,
		(shadow_uv_derivative_x.x * light_ndc_derivative_y.z
			- shadow_uv_derivative_y.x * light_ndc_derivative_x.z) / shadow_uv_determinant
	);
}
"#;

const SHADOW_TAP_SOURCE: &str = r#"
sample_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	offset: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let offset_shadow_uv: vec2f = shadow_uv + offset;
	if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) {
		return 1.0;
	}
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let maximum_texel: vec2u = shadow_map_extent - vec2u(1, 1);
	let shadow_texel: vec2u = vec2u(
		u32(clamp(offset_shadow_uv.x * f32(shadow_map_extent.x), 0.0, f32(maximum_texel.x))),
		u32(clamp(offset_shadow_uv.y * f32(shadow_map_extent.y), 0.0, f32(maximum_texel.y)))
	);
	let texel_center_uv: vec2f = (vec2f(f32(shadow_texel.x), f32(shadow_texel.y)) + vec2f(0.5, 0.5))
		/ vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let tap_surface_depth: f32 = surface_depth + dot(receiver_plane_depth_gradient, texel_center_uv - shadow_uv);
	if (tap_surface_depth < 0.0 || tap_surface_depth > 1.0) {
		return 1.0;
	}
	let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
	if (tap_surface_depth < closest_depth) {
		return 0.0;
	}
	return 1.0;
}
"#;

const ROTATED_SHADOW_TAP_SOURCE: &str = r#"
sample_rotated_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	poisson_offset: vec2f,
	rotation: vec2f,
	texel_size: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let rotated_offset: vec2f = vec2f(
		poisson_offset.x * rotation.x - poisson_offset.y * rotation.y,
		poisson_offset.x * rotation.y + poisson_offset.y * rotation.x
	) * texel_size * 1.5;
	return sample_shadow_tap(
		shadow_map,
		shadow_uv,
		surface_depth,
		receiver_plane_depth_gradient,
		rotated_offset,
		shadow_layer,
		shadow_map_extent
	);
}
"#;

// Directional shadows have one depth for the whole PCF kernel. Interior taps stay
// in texel space after one kernel-wide bounds check, avoiding eight normalize,
// bounds, clamp, receiver-plane, and denormalize sequences.
const DIRECTIONAL_SHADOW_TAP_SOURCE: &str = r#"
sample_directional_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_texel_position: vec2f,
	surface_depth: f32,
	poisson_offset: vec2f,
	rotation: vec2f,
	shadow_layer: u32
) -> f32 {
	let rotated_offset: vec2f = vec2f(
		poisson_offset.x * rotation.x - poisson_offset.y * rotation.y,
		poisson_offset.x * rotation.y + poisson_offset.y * rotation.x
	) * 1.5;
	let tap_position: vec2f = shadow_texel_position + rotated_offset;
	let shadow_texel: vec2u = vec2u(u32(tap_position.x), u32(tap_position.y));
	let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
	if (surface_depth < closest_depth) {
		return 0.0;
	}
	return 1.0;
}
"#;

// Proves one directional PCF footprint is fully lit from the 4x4 max-depth level.
// The gather covers every reduction cell touched by the rotated tap footprint.
const DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE: &str = r#"
directional_shadow_area_is_fully_lit: fn (
	shadow_uv: vec2f,
	surface_depth: f32,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> bool {
	if (shadow_uv.x <= 0.0 || shadow_uv.x >= 1.0 || shadow_uv.y <= 0.0 || shadow_uv.y >= 1.0) {
		return false;
	}

	let shadow_texel_position: vec2f = shadow_uv * vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let fine_level_extent: vec2u = texture_size(directional_shadow_depth_pyramid);
	let footprint_min: vec2f = vec2f(
		max(shadow_texel_position.x - 1.5, 0.0),
		max(shadow_texel_position.y - 1.5, 0.0)
	);
	let footprint_max: vec2f = vec2f(
		min(shadow_texel_position.x + 1.5, f32(shadow_map_extent.x - 1)),
		min(shadow_texel_position.y + 1.5, f32(shadow_map_extent.y - 1))
	);

	// One maximum-reduction gather reads the one, two, or four 4x4 cells touched by the footprint.
	let fine_first_cell: vec2u = vec2u(u32(footprint_min.x) / 4, u32(footprint_min.y) / 4);
	let fine_last_cell: vec2u = vec2u(u32(footprint_max.x) / 4, u32(footprint_max.y) / 4);
	let fine_layer_offset: u32 = shadow_layer * (shadow_map_extent.y / 4);
	let fine_probe_texel: vec2f = vec2f(
		f32(fine_first_cell.x + fine_last_cell.x) + 1.0,
		f32(fine_first_cell.y + fine_last_cell.y + fine_layer_offset + fine_layer_offset) + 1.0
	) * 0.5;
	let fine_probe_uv: vec2f = fine_probe_texel / vec2f(f32(fine_level_extent.x), f32(fine_level_extent.y));
	let fine_maximum_depth: f32 = downsample_max(
		directional_shadow_depth_pyramid,
		fine_probe_uv,
		0.0
	);
	return surface_depth >= fine_maximum_depth;
}
"#;

// Cone maps use two positive Depth16Unorm steps as a reverse-Z comparison margin after receiver-plane correction.
const SHADOW_SOURCE: &str = r#"
sample_shadow: fn (
	shadow_map: ArrayTexture2D,
	light: Light,
	world_space_position: vec3f,
	view_space_position: vec3f,
	surface_normal: vec3f,
	surface_to_light_direction: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> f32 {
	if (light.shadow_views[0] == 0) {
		return 1.0;
	}

	let shadow_view_index: u32 = light.shadow_views[0];
	let shadow_layer: u32 = light.shadow_layer;
	let bias_scale: f32 = 1.0;
	if (light.type == 68) {
		let depth_value: f32 = abs(view_space_position.z);
		// Descend only while the surface lies beyond a split. This avoids testing
		// a sentinel cascade index after every successful near-cascade match.
		let cascade_index: u32 = 0;
		if (depth_value >= views.views[light.shadow_views[0]].far) {
			cascade_index = 1;
			if (depth_value >= views.views[light.shadow_views[1]].far) {
				cascade_index = 2;
				if (depth_value >= views.views[light.shadow_views[2]].far) {
					cascade_index = 3;
				}
			}
		}
		shadow_view_index = light.shadow_views[cascade_index];
		shadow_layer = cascade_index;
		bias_scale = f32(cascade_index + 1);
	}

	let shadow_view: View = views.views[shadow_view_index];
	let surface_light_clip_position: vec4f = shadow_view.view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let surface_light_ndc_position: vec3f = vec3f(
		surface_light_clip_position.x,
		surface_light_clip_position.y,
		surface_light_clip_position.z
	) / surface_light_clip_position.w;
	let shadow_uv: vec2f = vec2f(
		surface_light_ndc_position.x * 0.5 + 0.5,
		0.5 - surface_light_ndc_position.y * 0.5
	);
	let receiver_plane_depth_gradient: vec2f = vec2f(0.0, 0.0);
	let surface_depth_bias: f32 = 2.0 / 65535.0;
	if (light.type == 68) {
		// Material evaluation passes a normalized world-space normal.
		let normal_alignment: f32 = max(dot(surface_normal, surface_to_light_direction), 0.0);
		let cascade_depth_range: f32 = max(shadow_view.far - shadow_view.near, 0.0001);
		let slope_scaled_bias: f32 = 0.0002 * bias_scale * (1.0 - normal_alignment);
		let constant_bias: f32 = 0.00002 * bias_scale;
		let cascade_range_bias: f32 = cascade_depth_range * 0.0000025;
		surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
	}
	if (light.type == 1) {
		receiver_plane_depth_gradient = shadow_receiver_plane_depth_gradient(
			shadow_view.view_projection,
			surface_light_clip_position,
			surface_light_ndc_position,
			world_space_position_derivative_x,
			world_space_position_derivative_y
		);
	}
	let surface_depth: f32 = surface_light_ndc_position.z + surface_depth_bias;
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let shadow_map_extent: vec2u = texture_size(shadow_map);
	if (light.type == 68 && directional_shadow_area_is_fully_lit(
		shadow_uv,
		surface_depth,
		shadow_layer,
		shadow_map_extent
	)) {
		return 1.0;
	}
	// Generate the PCF rotation only after the hierarchy fails to prove the area is lit.
	let rotation_noise: f32 = fract(
		sin(dot(vec2f(world_space_position.x, world_space_position.z) + world_space_position.y, vec2f(12.9898, 78.233))) * 43758.5453
	);
	let rotation_angle: f32 = rotation_noise * 6.2831853;
	let rotation: vec2f = vec2f(cos(rotation_angle), sin(rotation_angle));
	if (light.type == 68) {
		// Poisson offsets are expressed in texels. Keep the entire directional
		// fallback in texel space. One footprint check removes bounds and clamp
		// work from all eight taps for every interior shadow-map pixel.
		let shadow_texel_position: vec2f = shadow_uv
			* vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
		let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
		let footprint_is_inside: bool = shadow_texel_position.x >= 1.5
			&& shadow_texel_position.y >= 1.5
			&& shadow_texel_position.x <= shadow_map_extent_f.x - 1.5
			&& shadow_texel_position.y <= shadow_map_extent_f.y - 1.5;
		if (footprint_is_inside) {
			let directional_occlusion: f32 = 0.0;
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.0 - 0.613392, 0.617481), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.170019, 0.0 - 0.040254), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.0 - 0.299417, 0.791925), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.645680, 0.493210), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.0 - 0.651784, 0.717887), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.421003, 0.027070), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.0 - 0.817194, 0.0 - 0.271096), rotation, shadow_layer);
			directional_occlusion = directional_occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f(0.0 - 0.705374, 0.0 - 0.668203), rotation, shadow_layer);
			return directional_occlusion / 8.0;
		}
	}

	// Cone shadows and the rare directional map-edge fallback need per-tap border handling.
	let texel_size: vec2f = vec2f(1.0, 1.0) / vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let occlusion: f32 = 0.0;
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.0 - 0.613392, 0.617481), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.170019, 0.0 - 0.040254), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.0 - 0.299417, 0.791925), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.645680, 0.493210), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.0 - 0.651784, 0.717887), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.421003, 0.027070), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.0 - 0.817194, 0.0 - 0.271096), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f(0.0 - 0.705374, 0.0 - 0.668203), rotation, texel_size, shadow_layer, shadow_map_extent);
	return occlusion / 8.0;
}
"#;

#[allow(dead_code)]
const ENVIRONMENT_LAT_LONG_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (normalized_direction: vec3f) -> vec3f {
	// Material evaluation normalizes the shading normal before environment lighting.
	let environment_uv: vec2f = vec2f(
		atan2(normalized_direction.z, normalized_direction.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(normalized_direction.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let environment_extent: vec2u = texture_size(environment_irradiance);
	let environment_half_texel: f32 = 0.5 / f32(environment_extent.y);
	environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
	let environment_sample: vec4f = texture_lod(environment_irradiance, environment_uv);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

#[allow(dead_code)]
const ENVIRONMENT_LAT_LONG_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (normalized_direction: vec3f, roughness: f32) -> vec3f {
	// Reflecting a normalized view vector around a normalized shading normal preserves length.
	let environment_uv: vec2f = vec2f(
		atan2(normalized_direction.z, normalized_direction.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(normalized_direction.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let upper_level: u32 = u32(floor(specular_level)) + 1;
	if (upper_level > 7) {
		upper_level = 7;
	}
	let base_extent: vec2u = texture_size(environment_specular);
	let upper_level_scale: f32 = pow(2.0, f32(upper_level));
	let upper_half_texel: f32 = 0.5 * upper_level_scale / f32(base_extent.y);
	environment_uv.y = clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel);
	let environment_sample: vec4f = texture_lod(environment_specular, environment_uv, specular_level);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

const ENVIRONMENT_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (normalized_direction: vec3f) -> vec3f {
	let environment_sample: vec4f = texture_lod(environment_irradiance, normalized_direction, 0.0);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

const ENVIRONMENT_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (normalized_direction: vec3f, roughness: f32) -> vec3f {
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let environment_sample: vec4f = texture_lod(environment_specular, normalized_direction, specular_level);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

/// The `VisibilityShaderScope` struct provides material programs with the shared visibility data and lighting contract.
pub struct VisibilityShaderScope {}

/// The `VisibilityShaderGenerator` struct adapts portable material programs for visibility-buffer evaluation.
pub struct VisibilityShaderGenerator {
	scope: besl::parser::Node<'static>,
}

impl VisibilityShaderGenerator {
	pub fn new(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> Self {
		Self {
			scope: VisibilityShaderScope::new_with_params(
				material_count_read,
				material_count_write,
				material_offset_read,
				material_offset_write,
				material_offset_scratch_read,
				material_offset_scratch_write,
				pixel_mapping_read,
				pixel_mapping_write,
			),
		}
	}
}

impl VisibilityShaderScope {
	pub fn new<'a>() -> besl::parser::Node<'a> {
		Self::new_with_params(true, true, true, true, true, true, true, true)
	}

	pub fn new_with_params<'a>(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> besl::parser::Node<'a> {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct(
			"Mesh",
			vec![
				Node::member("model", "mat4x3f"),
				Node::member("material_index", "u32"),
				Node::member("base_vertex_index", "u32"),
				Node::member("base_primitive_index", "u32"),
				Node::member("base_triangle_index", "u32"),
				Node::member("base_meshlet_index", "u32"),
				Node::member("meshlet_count", "u32"),
				Node::member("skinned_base_vertex_index", "u32"),
				Node::member("padding0", "u32"),
			],
		);
		let skinned_vertex_struct = Node::r#struct(
			"SkinnedVertex",
			vec![Node::member("position", "vec4f"), Node::member("normal", "vec4f")],
		);
		let view_struct = Node::r#struct(
			"View",
			vec![
				Node::member("view", "mat4x3f"),
				Node::member("view_projection", "mat4f"),
				Node::member("inverse_view", "mat4x3f"),
				Node::member("fov", "vec2f"),
				Node::member("near", "f32"),
				Node::member("far", "f32"),
			],
		);
		let meshlet_struct = Node::r#struct(
			"Meshlet",
			vec![
				Node::member("primitive_offset", "u32"),
				Node::member("triangle_offset", "u32"),
				Node::member("primitive_count", "u32"),
				Node::member("triangle_count", "u32"),
				Node::member("center_radius", "packed_vec4f"),
				Node::member("cone_apex_cutoff", "packed_vec4f"),
				Node::member("cone_axis", "vec2u16"),
			],
		);
		let light_struct = Node::r#struct(
			"Light",
			vec![
				// Use explicit 16-byte vector fields so every storage-buffer backend shares the CPU layout.
				Node::member("position", "vec4f"),
				Node::member("color", "vec4f"),
				Node::member("direction", "vec4f"),
				Node::member("cone_cosines", "vec2f"),
				Node::member("type", "u32"),
				Node::member("shadow_views", "u32[8]"),
				Node::member("shadow_layer", "u32"),
			],
		);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", material_texture_array_type())]);

		let views_binding = Node::constant_buffer_binding(
			"views",
			Node::buffer("ViewsBuffer", vec![Node::member("views", "View[9]")]),
			0,
			true,
			false,
		);
		let meshes = Node::device_buffer_binding(
			"meshes",
			Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[1024]")]),
			1,
			true,
			false,
		);
		let positions = Node::device_buffer_binding(
			"vertex_positions",
			Node::buffer("Positions", vec![Node::member("positions", vertex_vec3_array_type())]),
			2,
			true,
			false,
		);
		let normals = Node::device_buffer_binding(
			"vertex_normals",
			Node::buffer("Normals", vec![Node::member("normals", vertex_normal_array_type())]),
			3,
			true,
			false,
		);
		let skinned_vertices = Node::device_buffer_binding(
			"skinned_vertices",
			Node::buffer("SkinnedVertices", vec![Node::member("vertices", skinned_vertex_array_type())]),
			4,
			true,
			false,
		);
		let uvs = Node::device_buffer_binding(
			"vertex_uvs",
			Node::buffer("UVs", vec![Node::member("uvs", vertex_uv_array_type())]),
			5,
			true,
			false,
		);
		let vertex_indices = Node::device_buffer_binding(
			"vertex_indices",
			Node::buffer(
				"VertexIndices",
				vec![Node::member("vertex_indices", vertex_index_array_type())],
			),
			6,
			true,
			false,
		);
		let primitive_indices = Node::device_buffer_binding(
			"primitive_indices",
			Node::buffer(
				"PrimitiveIndices",
				vec![Node::member("primitive_indices", primitive_index_array_type())],
			),
			7,
			true,
			false,
		);
		let meshlets = Node::device_buffer_binding(
			"meshlets",
			Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", meshlet_array_type())]),
			8,
			true,
			false,
		);
		let textures = Node::binding_array(
			"textures",
			Node::combined_image_sampler(),
			9,
			true,
			false,
			MAX_BINDLESS_TEXTURES as u32,
		);

		let material_count = Node::device_buffer_binding(
			"material_count",
			Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[1024]")]),
			1033,
			material_count_read,
			material_count_write,
		); // TODO: somehow set read/write properties per shader
		let material_offset = Node::device_buffer_binding(
			"material_offset",
			Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[1024]")]),
			1034,
			material_offset_read,
			material_offset_write,
		);
		let material_offset_scratch = Node::device_buffer_binding(
			"material_offset_scratch",
			Node::buffer(
				"MaterialOffsetScratch",
				vec![Node::member("material_offset_scratch", "u32[1024]")],
			),
			1035,
			material_offset_scratch_read,
			material_offset_scratch_write,
		);
		let material_evaluation_dispatches = Node::device_buffer_binding(
			"material_evaluation_dispatches",
			Node::buffer(
				"MaterialEvaluationDispatches",
				vec![Node::member("material_evaluation_dispatches", "vec4u[1024]")],
			),
			1036,
			material_offset_read,
			material_offset_write,
		);
		let pixel_mapping = Node::device_buffer_binding(
			"pixel_mapping",
			Node::buffer(
				"PixelMapping",
				vec![Node::member(
					"pixel_mapping",
					&format!("vec2u16[{MAX_PIXEL_MAPPING_ENTRIES}]"),
				)],
			),
			1037,
			pixel_mapping_read,
			pixel_mapping_write,
		);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1039, true, false);
		let instance_index = Node::binding("instance_index_render_target", Node::image("r32ui"), 1040, true, false);

		let compute_vertex_index = {
			let mut root = besl::parse(
				r#"
				compute_vertex_index: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> u32 {
					let relative_index: u16 = vertex_indices.vertex_indices[
						mesh.base_primitive_index + meshlet.primitive_offset + primitive_index
					];
					return mesh.base_vertex_index + u16_to_u32(relative_index);
				}
				"#,
			)
			.expect("Expected compute_vertex_index source to parse");

			match root.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => children.remove(0),
				_ => panic!(
					"Expected compute_vertex_index source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};
		let u16_to_u32 = parse_besl_function("u16_to_u32: fn (value: u16) -> u32 { return u32(value); }", "u16_to_u32");
		let decode_unorm16_vec2 = parse_besl_function(DECODE_UNORM16_VEC2_SOURCE, "decode_unorm16_vec2");
		let decode_octahedral_normal = parse_besl_function(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal");
		let cone_attenuation = parse_besl_function(
			"cone_attenuation: fn (cosine: f32, inner_cosine: f32, outer_cosine: f32) -> f32 { return clamp((cosine - outer_cosine) / (inner_cosine - outer_cosine), 0.0, 1.0); }",
			"cone_attenuation",
		);
		let set2_binding0 = Node::binding("lit_map", Node::image("rgba16"), 1041, true, true);
		let set2_binding4 = Node::constant_buffer_binding(
			"lighting_data",
			Node::buffer(
				"LightingBuffer",
				vec![
					Node::member("light_count", "u32"),
					// Keep the light array at the CPU record's 16-byte boundary on scalar-layout backends.
					Node::member("_light_count_padding", "u32[3]"),
					Node::member("lights", light_array_type()),
				],
			),
			1045,
			true,
			false,
		);
		let set2_binding5 = Node::device_buffer_binding(
			"materials",
			Node::buffer("MaterialBuffer", vec![Node::member("materials", material_array_type())]),
			1046,
			true,
			false,
		);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 1051, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_array_image_sampler(), 1052, true, false);
		let directional_shadow_depth_pyramid = Node::binding(
			"directional_shadow_depth_pyramid",
			Node::combined_image_sampler(),
			1053,
			true,
			false,
		);
		let cone_shadow_map = Node::binding("cone_shadow_map", Node::combined_array_image_sampler(), 1064, true, false);
		let environment_irradiance = Node::binding(
			"environment_irradiance",
			Node::combined_cube_image_sampler(),
			1054,
			true,
			false,
		);
		let environment_specular =
			Node::binding("environment_specular", Node::combined_cube_image_sampler(), 1055, true, false);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32"), Node::member("blend", "u32")]);

		let sample_function = Node::intrinsic_with_parameters(
			"sample_material",
			vec![Node::parameter("texture_index", "u32"), Node::parameter("uv", "vec2f")],
			Node::sentence(vec![Node::member_expression("textures")]),
			"vec4f",
		);

		let sample_normal_function = Node::intrinsic_with_parameters(
			"sample_normal",
			vec![Node::parameter("texture_index", "u32"), Node::parameter("uv", "vec2f")],
			Node::sentence(vec![
				Node::member_expression("textures"),
				Node::member_expression("unit_vector_from_xy"),
			]),
			"vec3f",
		);
		// Lighting helpers are authored once. Texture operations that differ by API remain typed intrinsics below.
		let shadow_receiver_plane_depth_gradient =
			parse_besl_function(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient");
		let sample_shadow_tap = parse_besl_function(SHADOW_TAP_SOURCE, "sample_shadow_tap");
		let sample_rotated_shadow_tap = parse_besl_function(ROTATED_SHADOW_TAP_SOURCE, "sample_rotated_shadow_tap");
		let sample_directional_shadow_tap = parse_besl_function(DIRECTIONAL_SHADOW_TAP_SOURCE, "sample_directional_shadow_tap");
		let directional_shadow_area_is_fully_lit =
			parse_besl_function(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit");
		let sample_shadow = parse_besl_function(SHADOW_SOURCE, "sample_shadow");
		let sample_environment_irradiance = parse_besl_function(ENVIRONMENT_IRRADIANCE_SOURCE, "sample_environment_irradiance");
		let sample_environment_specular = parse_besl_function(ENVIRONMENT_SPECULAR_SOURCE, "sample_environment_specular");

		Node::scope(
			"Visibility",
			vec![
				view_struct,
				views_binding,
				mesh_struct,
				skinned_vertex_struct,
				meshlet_struct,
				light_struct,
				material_struct,
				directional_shadow_depth_pyramid,
				shadow_receiver_plane_depth_gradient,
				sample_shadow_tap,
				sample_rotated_shadow_tap,
				sample_directional_shadow_tap,
				directional_shadow_area_is_fully_lit,
				sample_shadow,
				meshes,
				positions,
				normals,
				skinned_vertices,
				uvs,
				vertex_indices,
				primitive_indices,
				meshlets,
				textures,
				material_count,
				material_offset,
				material_offset_scratch,
				material_evaluation_dispatches,
				pixel_mapping,
				triangle_index,
				instance_index,
				u16_to_u32,
				decode_unorm16_vec2,
				decode_octahedral_normal,
				cone_attenuation,
				compute_vertex_index,
				set2_binding0,
				set2_binding4,
				set2_binding5,
				set2_binding10,
				set2_binding11,
				cone_shadow_map,
				environment_irradiance,
				environment_specular,
				push_constant,
				sample_function,
				sample_normal_function,
				sample_environment_irradiance,
				sample_environment_specular,
			],
		)
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
		let mut extra: Vec<Node<'a>> = Vec::new();
		let mut texture_slots = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].as_array().unwrap().iter() {
			let name = variable["name"].as_str().unwrap();
			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4f" => {
					let x = besl::parser::Node::specialization(name, data_type);
					extra.push(x);
				}
				"Texture2D" => {
					texture_slots.push((name, texture_count));
					let slot = format!("{texture_count}u");
					let slot_node = besl::parser::Node::literal_expression(slot);
					let x = besl::parser::Node::constant(name, "u32", slot_node);
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let m = root.get_mut("main").unwrap();
		let reconstruction_features = material_reconstruction_features(m);
		add_material_sample_context(m, &texture_slots);
		narrow_material_property_assignments(m);

		if let besl::parser::Nodes::Function { statements, .. } = m.node_mut() {
			statements.splice(0..0, material_evaluation_prefix_statements(reconstruction_features));
			statements.extend(material_evaluation_suffix_statements(reconstruction_features));
		}

		root.add(extra);
		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value};
	use resource_management::asset::{bema_asset_handler::ProgramGenerator, JsonObject};
	use resource_management::pbr::{
		generate_textured_brdf_program, BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture,
		BrdfValue,
	};
	use resource_management::shader::besl::backends::{
		glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator,
	};
	use resource_management::shader::generator::ShaderGenerationSettings;
	use utils::json::{self, JsonContainerTrait, JsonValueTrait};

	use crate::rendering::shader_vm_test::{buffer, compile, run_at, texture_2d};

	fn parser_expression_contains_raw_code(expression: &besl::parser::Expressions<'_>) -> bool {
		match expression {
			besl::parser::Expressions::RawCode { .. } => true,
			besl::parser::Expressions::Expression(elements)
			| besl::parser::Expressions::Call {
				parameters: elements, ..
			} => elements.iter().any(parser_node_contains_raw_code),
			besl::parser::Expressions::Accessor { left, right } | besl::parser::Expressions::Operator { left, right, .. } => {
				parser_node_contains_raw_code(left) || parser_node_contains_raw_code(right)
			}
			besl::parser::Expressions::Macro { body, .. } => parser_node_contains_raw_code(body),
			besl::parser::Expressions::Return { value } => value.as_deref().is_some_and(parser_node_contains_raw_code),
			besl::parser::Expressions::Member { .. }
			| besl::parser::Expressions::Literal { .. }
			| besl::parser::Expressions::VariableDeclaration { .. }
			| besl::parser::Expressions::Continue => false,
		}
	}

	/// Walks generated parser nodes so raw code cannot hide in shared lighting helpers.
	fn parser_node_contains_raw_code(node: &besl::parser::Node<'_>) -> bool {
		match node.node() {
			besl::parser::Nodes::RawCode { .. } => true,
			besl::parser::Nodes::Scope { children, .. } => children.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Function { statements, .. } => statements.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Conditional { condition, statements } => {
				parser_node_contains_raw_code(condition) || statements.iter().any(parser_node_contains_raw_code)
			}
			besl::parser::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				parser_node_contains_raw_code(initializer)
					|| parser_node_contains_raw_code(condition)
					|| parser_node_contains_raw_code(update)
					|| statements.iter().any(parser_node_contains_raw_code)
			}
			besl::parser::Nodes::Intrinsic { elements, .. } => elements.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Expression(expression) => parser_expression_contains_raw_code(expression),
			_ => false,
		}
	}

	use crate::besl;

	macro_rules! material_metadata {
		($($json:tt)*) => {
			serde_json::json!({ $($json)* })
				.as_object()
				.expect("test material metadata should be an object")
				.clone()
		};
	}

	/// Generates the production Metal material shader for source-shape regressions.
	fn material_msl(shader_source: &str, material: &JsonObject) -> String {
		let shader_node = besl::parse(shader_source).expect("Test material source should parse.");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, material))
			.expect("Material evaluation should produce valid BESL.");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to emit the Metal material pass. The most likely cause is an invalid visibility shader contract.")
	}

	/// Guards the algebraic decoder shape so compact normals do not reintroduce transcendental work per shaded pixel.
	#[test]
	fn octahedral_decoder_uses_abs_and_step_and_defers_normalization() {
		assert!(!super::DECODE_OCTAHEDRAL_NORMAL_SOURCE.contains("sqrt("));
		assert!(!super::DECODE_OCTAHEDRAL_NORMAL_SOURCE.contains("normalize("));
		assert!(super::DECODE_OCTAHEDRAL_NORMAL_SOURCE.contains("abs("));
		assert!(super::DECODE_OCTAHEDRAL_NORMAL_SOURCE.contains("step("));

		for source in [
			include_str!(concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/assets/rendering/visibility/visibility-task.besl"
			)),
			include_str!(concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/assets/rendering/visibility/shadow-task.besl"
			)),
		] {
			assert!(!source.contains("sqrt(octahedral"));
			assert!(source.contains("step(0.0, octahedral.x)"));
		}
	}

	/// Guards fixed fifth powers against returning to the general-purpose power intrinsic.
	#[test]
	fn material_fresnel_uses_multiplication_for_fifth_powers() {
		let source = super::MATERIAL_EVALUATION_SUFFIX_SOURCE;
		assert!(!source.contains("f16(5.0)"));
		for base in ["view_fresnel", "half_view_fresnel", "light_fresnel"] {
			assert!(source.contains(&format!("{base}_squared * {base}_squared * {base}_base")));
		}
	}

	/// Executes representative octahedral seams and axes through the optimized production decoder.
	#[test]
	fn octahedral_decoder_preserves_normal_directions_in_the_besl_vm() {
		const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
		const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
		let source = r#"
			main: fn () -> void {
				for (let index: u32 = 0; index < 5; index = index + 1) {
					results.values[index] = normalize(decode_octahedral_normal(inputs.values[index]));
				}
			}
		"#;
		let mut root = besl::parse(source)
			.expect("Failed to parse the octahedral decoder VM test. The most likely cause is invalid BESL test syntax.");
		root.add(vec![
			super::parse_besl_function(super::DECODE_UNORM16_VEC2_SOURCE, "decode_unorm16_vec2"),
			super::parse_besl_function(super::DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
			besl::ParserNode::binding(
				"inputs",
				besl::ParserNode::buffer("OctahedralInputs", vec![besl::ParserNode::member("values", "vec2u16[5]")]),
				INPUT_SLOT.slot(),
				true,
				false,
			),
			besl::ParserNode::binding(
				"results",
				besl::ParserNode::buffer("OctahedralResults", vec![besl::ParserNode::member("values", "vec3f[5]")]),
				RESULT_SLOT.slot(),
				false,
				true,
			),
		]);
		let executable = compile(besl::lex(root).expect(
			"Failed to lex the octahedral decoder VM test. The most likely cause is an unresolved portable decoder operation.",
		));
		let mut inputs = buffer(&executable, INPUT_SLOT);
		let mut results = buffer(&executable, RESULT_SLOT);
		let cases = [
			([32768, 32768], [0.0, 0.0, 1.0]),
			([65535, 32768], [1.0, 0.0, 0.0]),
			([0, 32768], [-1.0, 0.0, 0.0]),
			([32768, 65535], [0.0, 1.0, 0.0]),
			([65535, 65535], [0.0, 0.0, -1.0]),
		];
		for (index, (encoded, _)) in cases.iter().enumerate() {
			inputs
				.write_indexed("values", index, Value::Vec2U16(*encoded))
				.expect("Failed to initialize an octahedral input. The most likely cause is a drifted packed-vector layout.");
		}
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(INPUT_SLOT, &mut inputs);
		descriptors.bind_buffer(RESULT_SLOT, &mut results);
		run_at(&executable, &mut descriptors, [0, 0]);

		for (index, (_, expected)) in cases.iter().enumerate() {
			let Value::Vec3F(actual) = results
				.read_indexed("values", index)
				.expect("Missing decoded normal. The most likely cause is a VM output-layout regression.")
			else {
				panic!("Unexpected decoded-normal type. The most likely cause is a VM packed-vector regression.");
			};
			assert!(
				actual
					.iter()
					.zip(expected)
					.all(|(actual, expected)| (actual - expected).abs() <= 0.00005),
				"Unexpected decoded normal {actual:?} for {encoded:?}. The most likely cause is incorrect octahedral fold math.",
				encoded = cases[index].0,
			);
		}
	}

	/// Guards the branch order that prevents skinned pixels from loading and decoding static attributes first.
	#[test]
	fn skinned_material_path_selects_geometry_before_static_attribute_loads() {
		let material = material_metadata! { "variables": [] };
		let source = material_msl("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }", &material);
		let selection = source
			.find("mesh.skinned_base_vertex_index")
			.expect("Generated material shader should select static or skinned geometry.");
		let static_position_load = source
			.find("vertex_positions->positions[vertex_index0]")
			.expect("Generated material shader should retain the static geometry path.");
		let static_normal_load = source
			.find("vertex_normals->normals[vertex_index0]")
			.expect("Generated material shader should retain the static normal path.");
		assert!(selection < static_position_load);
		assert!(selection < static_normal_load);
	}

	/// Verifies generated material reconstruction includes only the UV and tangent work required by the material body.
	#[test]
	fn material_reconstruction_specializes_for_texture_and_normal_usage() {
		let constant_material = material_metadata! { "variables": [] };
		let constant = material_msl(
			"main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }",
			&constant_material,
		);
		assert!(!constant.contains("vertex_uvs->uvs"));
		assert!(!constant.contains("tangent_scale"));
		assert!(!constant.contains("normal.x * T"));

		let textured_material = material_metadata! {
			"variables": [{ "name": "base_color", "data_type": "Texture2D" }]
		};
		let textured = material_msl(
			"main: fn () -> void { albedo = sample_material(base_color); }",
			&textured_material,
		);
		assert!(textured.contains("vertex_uvs->uvs"));
		assert!(!textured.contains("tangent_scale"));
		assert!(!textured.contains("normal.x * T"));

		let normal_material = material_metadata! {
			"variables": [{ "name": "normal_map", "data_type": "Texture2D" }]
		};
		let normal = material_msl(
			"main: fn () -> void { normal = sample_normal(normal_map); }",
			&normal_material,
		);
		assert!(normal.contains("vertex_uvs->uvs"));
		assert!(normal.contains("tangent_scale"));
		assert!(normal.contains("float(normal.x) * T"));

		let procedural = material_msl("main: fn () -> void { normal = vec3f(1.0, 0.0, 0.0); }", &constant_material);
		assert!(procedural.contains("vertex_uvs->uvs"));
		assert!(procedural.contains("tangent_scale"));
	}

	#[test]
	fn write_to_albedo() {
		let material = material_metadata! {
			"variables": []
		};

		let shader_source = "main: fn () -> void { albedo = vec4f(1, 2, 3, 4); }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		let _node = besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn vec4f_variable() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "albedo",
					"data_type": "vec4f",
					"value": "Purple"
				}
			]
		};

		let shader_source = "main: fn () -> void { out_color = albedo; }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		println!("{:#?}", shader);
	}

	/// Verifies material texture variables produce valid BESL.
	#[test]
	fn texture_variable_transform_produces_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn material_evaluation_texture_variables_produce_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				},
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); normal = sample_normal(normal_map); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies HLSL transforms tangent-space normals with the same basis convention as GLSL and MSL.
	#[test]
	fn material_evaluation_hlsl_combines_tangent_basis_vectors() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_node =
			besl::parse("main: fn () -> void { normal = sample_normal(normal_map); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let source = HLSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect(
				"Failed to emit the HLSL material pass. The most likely cause is an invalid tangent-basis shader contract.",
			);
		assert!(
			source.contains("normal = float16_t3(normalize(")
				&& source.contains("float(normal.x) * T")
				&& source.contains("float(normal.y) * B")
				&& source.contains("float(normal.z) * N"),
			"HLSL did not combine the tangent basis explicitly. The most likely cause is that the material pass reintroduced a row-versus-column matrix assumption."
		);
		assert!(
			!source.contains("mul(TBN, normal)"),
			"HLSL multiplied a row-constructed tangent basis as a column basis. The most likely cause is that the material pass reintroduced the faceted-normal transform."
		);
	}

	/// Verifies cone PCF evaluates its receiver plane at each fetched shadow texel center.
	#[test]
	fn cone_shadow_receiver_plane_depth_gradient_executes_in_the_besl_vm() {
		const RESULT_SLOT: ResourceSlot = ResourceSlot::new(0);
		let source = r#"
			main: fn () -> void {
				let identity: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(0.0, 0.0, 0.0, 1.0)
				);
				let surface_light_clip_position: vec4f = vec4f(0.1, 0.0 - 0.2, 0.5, 1.0);
				let surface_light_ndc_position: vec3f = vec3f(0.1, 0.0 - 0.2, 0.5);
				let receiver_plane_depth_gradient: vec2f = shadow_receiver_plane_depth_gradient(
					identity,
					surface_light_clip_position,
					surface_light_ndc_position,
					vec3f(0.2, 0.0, 0.3),
					vec3f(0.0, 0.0 - 0.4, 0.0 - 0.2)
				);
				results.gradient = receiver_plane_depth_gradient;
				results.corrected_depth = 0.5 + dot(
					receiver_plane_depth_gradient,
					vec2f(0.6, 0.8) - vec2f(0.55, 0.6)
				);
				results.degenerate = shadow_receiver_plane_depth_gradient(
					identity,
					surface_light_clip_position,
					surface_light_ndc_position,
					vec3f(0.0, 0.0, 0.0),
					vec3f(0.0, 0.0, 0.0)
				);
			}
		"#;
		let mut root = besl::parse(source).expect(
			"Failed to parse the cone-shadow receiver-plane VM test. The most likely cause is invalid BESL test syntax.",
		);
		root.add(vec![
			super::parse_besl_function(super::SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient"),
			besl::ParserNode::binding(
				"results",
				besl::ParserNode::buffer(
					"ConeShadowReceiverPlaneResults",
					vec![
						besl::ParserNode::member("gradient", "vec2f"),
						besl::ParserNode::member("corrected_depth", "f32"),
						besl::ParserNode::member("degenerate", "vec2f"),
					],
				),
				RESULT_SLOT.slot(),
				false,
				true,
			),
		]);
		let program = besl::lex(root).expect(
			"Failed to lex the cone-shadow receiver-plane VM test. The most likely cause is an unresolved portable shadow helper.",
		);
		let executable = compile(program);
		let mut results = buffer(&executable, RESULT_SLOT);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(RESULT_SLOT, &mut results);
		run_at(&executable, &mut descriptors, [0, 0]);

		let Value::Vec2F(gradient) = results.read("gradient").expect("Missing receiver-plane gradient.") else {
			panic!("Unexpected receiver-plane gradient type. The most likely cause is a VM output-layout regression.");
		};
		let Value::F32(corrected_depth) = results.read("corrected_depth").expect("Missing corrected receiver depth.") else {
			panic!("Unexpected corrected receiver-depth type. The most likely cause is a VM output-layout regression.");
		};
		let Value::Vec2F(degenerate) = results
			.read("degenerate")
			.expect("Missing degenerate receiver-plane gradient.")
		else {
			panic!(
				"Unexpected degenerate receiver-plane gradient type. The most likely cause is a VM output-layout regression."
			);
		};

		assert!(
			(gradient[0] - 3.0).abs() <= 0.00001 && (gradient[1] + 1.0).abs() <= 0.00001,
			"Unexpected cone receiver-plane gradient: {gradient:?}. The most likely cause is incorrect projected-depth derivative math."
		);
		assert!(
			(corrected_depth - 0.45).abs() <= 0.00001,
			"Unexpected cone receiver depth at a shadow texel center: {corrected_depth}. The most likely cause is incorrect receiver-plane tap correction."
		);
		assert_eq!(
			degenerate,
			[0.0, 0.0],
			"A degenerate shadow projection must retain the base depth bias. The most likely cause is a missing receiver-plane fallback."
		);
	}

	/// Verifies the directional probe skips PCF only when every fine cell touching the footprint is clear.
	#[test]
	fn directional_shadow_depth_probe_is_conservative_in_the_besl_vm() {
		const PYRAMID_SLOT: ResourceSlot = ResourceSlot::new(0);
		const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
		let source = r#"
			main: fn () -> void {
				results.fully_lit = 0;
				results.may_be_occluded = 0;
				results.crosses_tile_boundary = 0;
				results.adjacent_cell_may_occlude = 0;
				if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.8, 2, vec2u(8, 8))) {
					results.fully_lit = 1;
				}
				if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.6, 2, vec2u(8, 8))) {
					results.may_be_occluded = 1;
				}
				if (directional_shadow_area_is_fully_lit(vec2f(0.1, 0.5), 1.0, 2, vec2u(8, 8))) {
					results.crosses_tile_boundary = 1;
				}
				if (directional_shadow_area_is_fully_lit(vec2f(0.25, 0.25), 0.8, 0, vec2u(8, 8))) {
					results.adjacent_cell_may_occlude = 1;
				}
			}
		"#;
		let mut root = besl::parse(source)
			.expect("Failed to parse the directional-shadow probe VM test. The most likely cause is invalid BESL test syntax.");
		root.add(vec![
			besl::ParserNode::binding(
				"directional_shadow_depth_pyramid",
				besl::ParserNode::combined_image_sampler(),
				PYRAMID_SLOT.slot(),
				true,
				false,
			),
			super::parse_besl_function(
				super::DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE,
				"directional_shadow_area_is_fully_lit",
			),
			besl::ParserNode::binding(
				"results",
				besl::ParserNode::buffer(
					"DirectionalShadowProbeResults",
					vec![
						besl::ParserNode::member("fully_lit", "u32"),
						besl::ParserNode::member("may_be_occluded", "u32"),
						besl::ParserNode::member("crosses_tile_boundary", "u32"),
						besl::ParserNode::member("adjacent_cell_may_occlude", "u32"),
					],
				),
				RESULT_SLOT.slot(),
				false,
				true,
			),
		]);
		let executable = compile(besl::lex(root).expect(
			"Failed to lex the directional-shadow probe VM test. The most likely cause is an unresolved portable texture operation.",
		));

		let cascade_depths = [0.2, 0.4, 0.7, 0.9];
		let mut base_depths = (0..8)
			.flat_map(|y| std::iter::repeat_n([cascade_depths[y / 2], 0.0, 0.0, 1.0], 2))
			.collect::<Vec<_>>();
		// Cascade zero contains a blocker in the neighboring 4x4 cell. A maximum
		// gather may conservatively include it even when the footprint stays in cell zero.
		base_depths[0] = [0.2, 0.0, 0.0, 1.0];
		base_depths[1] = [0.9, 0.0, 0.0, 1.0];
		let mut pyramid = texture_2d(2, 8, &base_depths);
		pyramid.add_mip(texture_2d(
			1,
			4,
			&[
				[0.9, 0.0, 0.0, 1.0],
				[0.4, 0.0, 0.0, 1.0],
				[0.7, 0.0, 0.0, 1.0],
				[0.9, 0.0, 0.0, 1.0],
			],
		));
		let mut results = buffer(&executable, RESULT_SLOT);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(PYRAMID_SLOT, &mut pyramid);
		descriptors.bind_buffer(RESULT_SLOT, &mut results);
		run_at(&executable, &mut descriptors, [0, 0]);

		for (name, expected) in [
			("fully_lit", 1),
			("may_be_occluded", 0),
			("crosses_tile_boundary", 1),
			("adjacent_cell_may_occlude", 0),
		] {
			let Value::U32(actual) = results.read(name).expect("directional shadow probe result") else {
				panic!("Unexpected directional shadow probe result type for {name}.");
			};
			assert_eq!(actual, expected, "Unexpected directional shadow probe result for {name}.");
		}
	}

	/// Verifies the interior texel-space directional fallback preserves reverse-Z shadow comparison.
	#[test]
	fn directional_shadow_tap_uses_texel_coordinates_in_the_besl_vm() {
		const SHADOW_SLOT: ResourceSlot = ResourceSlot::new(0);
		const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
		let source = r#"
			main: fn () -> void {
				results.lit = sample_directional_shadow_tap(
					shadow_map, vec2f(1.0, 1.0), 0.8, vec2f(0.0, 0.0), vec2f(1.0, 0.0), u32(0)
				);
				results.blocked = sample_directional_shadow_tap(
					shadow_map, vec2f(2.0, 2.0), 0.8, vec2f(0.0, 0.0), vec2f(1.0, 0.0), u32(0)
				);
			}
		"#;
		let mut root = besl::parse(source)
			.expect("Failed to parse the directional-shadow tap VM test. The most likely cause is invalid BESL test syntax.");
		root.add(vec![
			besl::ParserNode::binding(
				"shadow_map",
				besl::ParserNode::combined_array_image_sampler(),
				SHADOW_SLOT.slot(),
				true,
				false,
			),
			super::parse_besl_function(super::DIRECTIONAL_SHADOW_TAP_SOURCE, "sample_directional_shadow_tap"),
			besl::ParserNode::binding(
				"results",
				besl::ParserNode::buffer(
					"DirectionalShadowTapResults",
					vec![
						besl::ParserNode::member("lit", "f32"),
						besl::ParserNode::member("blocked", "f32"),
					],
				),
				RESULT_SLOT.slot(),
				false,
				true,
			),
		]);
		let executable = compile(besl::lex(root).expect(
			"Failed to lex the directional-shadow tap VM test. The most likely cause is an unresolved portable texture operation.",
		));
		let mut shadow_map = Texture::new_3d(4, 4, 1)
			.expect("Failed to create the directional shadow fixture. The most likely cause is an invalid extent.");
		for y in 0..4 {
			for x in 0..4 {
				shadow_map
					.write_3d([x, y, 0], [0.2, 0.0, 0.0, 1.0])
					.expect("Failed to initialize the directional shadow fixture.");
			}
		}
		shadow_map
			.write_3d([2, 2, 0], [0.9, 0.0, 0.0, 1.0])
			.expect("Failed to initialize the directional shadow blocker.");
		let mut results = buffer(&executable, RESULT_SLOT);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(SHADOW_SLOT, &mut shadow_map);
		descriptors.bind_buffer(RESULT_SLOT, &mut results);
		run_at(&executable, &mut descriptors, [0, 0]);

		for (name, expected) in [("lit", 1.0), ("blocked", 0.0)] {
			let Value::F32(actual) = results.read(name).expect("directional shadow tap result") else {
				panic!("Unexpected directional shadow tap result type for {name}.");
			};
			assert_eq!(actual, expected, "Unexpected directional shadow tap result for {name}.");
		}
	}

	/// Verifies material evaluation keeps per-pixel and per-light terms out of the repeated PCF tap path.
	#[test]
	fn material_evaluation_hoists_shared_terms_and_uses_direct_ao_reads() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the GLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the HLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the MSL material pass. The most likely cause is an invalid visibility shader contract.");
		assert!(glsl.contains("texelFetch(ao, ivec2(pixel_coordinates),0).x"));
		assert!(hlsl.contains("ao.Load(int3(pixel_coordinates, 0)).x"));
		assert!(msl.contains("resources.ao.read(pixel_coordinates).x"));
		assert!(glsl.contains("texelFetch(shadow_map, ivec3(ivec2(shadow_texel),int(shadow_layer)),0).x"));
		assert!(hlsl.contains("shadow_map.Load(int4(shadow_texel, int(shadow_layer), 0)).x"));
		assert!(hlsl.contains("environment_specular.SampleLevel(environment_specular_sampler, environment_uv, specular_level)"));
		assert!(glsl.contains("textureLod(environment_specular, environment_uv, specular_level)"));
		assert!(msl.contains(
			"resources.environment_specular.sample(resources.environment_specular_sampler, environment_uv, metal::level(specular_level))"
		));
		assert!(msl.contains("float3 world_space_vertex_position0"));
		assert!(!msl.contains("world_space_vertex_positions[3]"));
		assert!(!msl.contains("primitive_indices[3]"));
		assert!(msl.contains("half geometry_view"));
		assert!(msl.contains("half geometry_light"));
		assert!(msl.contains("half NdotH"));
		assert!(msl.contains("half denominator_base"));
		assert!(msl.contains("View shadow_view = resources.views->views[shadow_view_index];"));
		assert!(msl.contains(
			"float sample_shadow_tap(texture2d_array<float> shadow_map, float2 shadow_uv, float surface_depth, float2 receiver_plane_depth_gradient, float2 offset, uint shadow_layer, uint2 shadow_map_extent)"
		));
		assert!(msl.contains("float2 shadow_receiver_plane_depth_gradient("));
		assert!(msl.contains("float2 offset_shadow_uv"));
		assert!(msl.contains("float2 texel_center_uv"));
		assert!(msl.contains("float tap_surface_depth"));
		assert!(msl.contains("shadow_map.read(shadow_texel, shadow_layer).x"));

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-mipmapped-environment",
		)
		.expect(
			"Failed to compile the mipmapped-environment MSL material pass. The most likely cause is invalid explicit-LOD Metal source.",
		);
	}

	/// Verifies material evaluation with skinned geometry produces valid BESL.
	#[test]
	fn material_evaluation_with_skinning_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies material evaluation samples the bound environment without a procedural fallback.
	#[test]
	fn material_evaluation_with_environment_ibl_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect("expected material evaluation main");
		let source = MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("expected valid Metal material evaluation source");
		assert!(!source.contains("sample_analytical_reflection"));
		assert!(!source.contains("environment_sample.a"));
		assert!(!source.contains("lower_sample.a"));
		assert!(source.contains("texturecube<float> environment_irradiance"));
		assert!(source.contains("texturecube<float> environment_specular"));
		assert!(!source.contains("atan2("));
		assert!(!source.contains("asin("));
	}

	#[test]
	fn material_evaluation_emits_cone_attenuation_for_every_backend() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));

		let glsl = GLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the GLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let hlsl = HLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the HLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let msl = MSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the MSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);

		for source in [&glsl, &hlsl, &msl] {
			assert!(source.contains("cone_cosines"));
			assert!(source.contains("cone_attenuation"));
			assert!(source.contains("light.type == 1"));
			assert!(source.contains("_light_count_padding"));
			assert!(source.contains("shadow_layer"));
			assert!(source.contains("cone_shadow_map"));
		}
		assert!(glsl.contains("vec4 position"));
		assert!(glsl.contains("uint32_t type"));
		assert!(hlsl.contains("float4 position"));
		assert!(hlsl.contains("uint32_t type"));
		assert!(msl.contains("float4 position"));
		assert!(msl.contains("uint type"));
		assert!(
			!msl.contains("mul("),
			"Metal material evaluation must use the native multiplication operator."
		);

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-cone-light-material",
		)
		.expect("Failed to compile the MSL cone-light material pass. The most likely cause is invalid generated Metal source.");
	}

	/// Verifies the generated material pass stays in BESL so backend lowering owns storage and matrix syntax.
	#[test]
	fn material_evaluation_contains_no_backend_raw_code() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);

		assert!(
			!parser_node_contains_raw_code(&shader),
			"Material evaluation and its shared lighting helpers must remain portable BESL instead of embedding backend source."
		);
	}

	/// Verifies native material evaluation emits one bindless sample for a texture shared by several BRDF roles.
	#[test]
	fn generated_material_evaluation_reuses_shared_texture_sample() {
		let mut builder = BrdfMaterialBuilder::new();
		let texture = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Blue);
		let roughness = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Green);
		let normal = builder.add(BrdfNode::NormalMap {
			source: texture,
			scale: 0.5,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: texture,
			strength: 0.75,
		});
		let emission = builder.add(BrdfNode::Emission { color: texture });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color: texture,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let material_program = generate_textured_brdf_program(&material).expect(
			"Failed to generate the shared-texture material program. The most likely cause is an invalid BRDF material graph.",
		);
		let material_metadata = material_metadata! {
			"variables": [{
				"name": "material_texture_3",
				"data_type": "Texture2D"
			}]
		};

		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(material_program, &material_metadata);
		let program = besl::lex(shader).expect(
			"Failed to link the shared-texture material evaluation pass. The most likely cause is a drifted visibility shader contract.",
		);
		let main = program.get_main().expect(
			"Missing shared-texture material entry point. The most likely cause is that material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the shared-texture GLSL material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let hlsl = HLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the shared-texture HLSL material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let msl = MSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the shared-texture MSL material pass. The most likely cause is an invalid visibility shader contract.",
		);
		assert_eq!(
			glsl.match_indices("texture(textures[nonuniformEXT(").count(),
			1,
			"The generated GLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			hlsl.match_indices("].SampleLevel(textures_sampler,").count(),
			1,
			"The generated HLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			msl.match_indices("].sample(resources.textures_sampler[").count(),
			1,
			"The generated material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert!(
			msl.contains("half4 material_texture_sample_0"),
			"The generated material did not retain its reusable texel local. The most likely cause is that texture-sample lowering stopped emitting the cache binding."
		);
		assert!(
			msl.contains("half4 albedo")
				&& msl.contains("half3 normal")
				&& msl.contains("half metalness")
				&& msl.contains("half roughness")
				&& msl.contains("half occlusion")
				&& msl.contains("half3 emission"),
			"Material inputs did not remain half precision. The most likely cause is a material-evaluation type regression."
		);
		assert!(
			msl.contains("half3 albedo_rgb")
				&& msl.contains("half3 V_material")
				&& msl.contains("half3 F0")
				&& msl.contains("half NdotV")
				&& msl.contains("half3 L_material")
				&& msl.contains("half3 local_diffuse")
				&& msl.contains("half3 local_specular")
				&& !msl.contains("surface_albedo"),
			"The BRDF did not retain compact material values and normalized vectors. The most likely cause is a material-evaluation precision regression."
		);
		assert_eq!(
			msl.match_indices("decode_material_normal_f16(material_texture_sample_0)")
				.count(),
			1,
			"The scaled normal map decoded the shared texel more than once. The most likely cause is that normal scaling bypassed the reusable helper."
		);

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-shared-material-texture-sample",
		)
		.expect(
			"Failed to compile the shared-texture MSL material pass. The most likely cause is invalid generated Metal source.",
		);
	}
}
