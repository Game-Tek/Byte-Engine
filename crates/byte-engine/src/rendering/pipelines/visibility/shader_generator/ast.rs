use std::sync::Arc;
use std::{cell::RefCell, ops::Deref, rc::Rc, sync::OnceLock};

use besl::{parser::Node, NodeReference};

use super::sources::*;
use crate::rendering::pipelines::visibility::{
	MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
};

pub(super) fn light_array_type() -> &'static str {
	static LIGHT_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	LIGHT_ARRAY_TYPE
		.get_or_init(|| format!("Light[{MAX_LIGHTS}]").into_boxed_str())
		.as_ref()
}

pub(super) fn material_array_type() -> &'static str {
	static MATERIAL_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_ARRAY_TYPE
		.get_or_init(|| format!("Material[{MAX_MATERIALS}]").into_boxed_str())
		.as_ref()
}

pub(super) fn material_texture_array_type() -> &'static str {
	static MATERIAL_TEXTURE_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_TEXTURE_ARRAY_TYPE
		.get_or_init(|| format!("u32[{MAX_MATERIAL_TEXTURES}]").into_boxed_str())
		.as_ref()
}

pub(super) fn vertex_vec3_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec3f[{MAX_VERTICES}]").into_boxed_str())
}

pub(super) fn vertex_normal_array_type() -> &'static str {
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

pub(super) fn vertex_uv_array_type() -> &'static str {
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

pub(super) fn skinned_vertex_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("SkinnedVertex[{MAX_VERTICES}]").into_boxed_str())
}

pub(super) fn vertex_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u16[{MAX_PRIMITIVE_TRIANGLES}]").into_boxed_str())
}

pub(super) fn primitive_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u8[{}]", MAX_TRIANGLES * 3).into_boxed_str())
}

pub(super) fn meshlet_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("Meshlet[{MAX_MESHLETS}]").into_boxed_str())
}

/// Parses one reusable BESL helper function from an isolated source scope.
pub(super) fn parse_besl_function(source: &'static str, function_name: &str) -> besl::parser::Node<'static> {
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
pub(super) fn parse_besl_statements(source: &'static str, function_name: &str) -> Vec<besl::parser::Node<'static>> {
	let mut function = parse_besl_function(source, function_name);
	match function.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => std::mem::take(statements),
		_ => panic!(
			"Invalid `{function_name}` helper. The most likely cause is that its BESL source no longer defines a function."
		),
	}
}

#[derive(Clone, Copy, Default)]
pub(super) struct MaterialReconstructionFeatures {
	uses_uv: bool,
	uses_tangent_frame: bool,
}

pub(super) fn material_reconstruction_features(node: &besl::parser::Node<'_>) -> MaterialReconstructionFeatures {
	let mut features = MaterialReconstructionFeatures::default();
	collect_material_reconstruction_features(node, &mut features);
	features
}

/// Finds material sampling operations before texture shorthand is expanded.
pub(super) fn collect_material_reconstruction_features(
	node: &besl::parser::Node<'_>,
	features: &mut MaterialReconstructionFeatures,
) {
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
pub(super) fn collect_material_expression_features(
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
		| besl::parser::Expressions::Continue
		| besl::parser::Expressions::Discard => {}
	}
}

/// Recognizes the canonical no-normal-map value emitted by the BRDF generators.
pub(super) fn is_default_tangent_normal(node: &besl::parser::Node<'_>) -> bool {
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

pub(super) fn material_evaluation_prefix_statements(
	features: MaterialReconstructionFeatures,
) -> Vec<besl::parser::Node<'static>> {
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

pub(super) fn material_evaluation_suffix_statements(
	features: MaterialReconstructionFeatures,
) -> Vec<besl::parser::Node<'static>> {
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
pub(super) fn narrow_material_property_assignments(node: &mut besl::parser::Node<'_>) {
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
					**right = besl::parser::Node::call(target_type, vec![*right.clone()]);
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
pub(super) fn narrow_material_property_assignment_expression(expression: &mut besl::parser::Expressions<'_>) {
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
		| besl::parser::Expressions::Continue
		| besl::parser::Expressions::Discard => {}
	}
}

/// Makes material texture context explicit before the parser tree is linked.
pub(super) fn add_material_sample_context(node: &mut besl::parser::Node<'_>, texture_slots: &[(&str, u32)]) {
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
pub(super) fn add_material_sample_context_to_expression(
	expression: &mut besl::parser::Expressions<'_>,
	texture_slots: &[(&str, u32)],
) {
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

			// Material sampling is visibility-pipeline shorthand. Lower it here so the
			// portable BESL backends only need to know the bindless sampling operation.
			*name = match *name {
				"sample_material" => "sample_texture_2d_array_grad",
				"sample_normal" => "sample_visibility_normal",
				_ => unreachable!(),
			};

			let slot = parameters.remove(0);
			let slot = match slot.node() {
				besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) => texture_slots
					.iter()
					.find_map(|(texture_name, index)| (*texture_name == *name).then_some(*index))
					.map_or(slot, |index| Node::literal_expression(format!("{index}u"))),
				_ => slot,
			};
			let material_textures = Node::accessor(Node::member_expression("material"), Node::member_expression("textures"));
			// Pass the resource explicitly. The BESL backend must not know which
			// pipeline owns the texture array or what that resource is named.
			parameters.push(Node::member_expression("textures"));
			// Index access has an expression on its right side. Preserve that shape so
			// the linked backend AST can distinguish `textures[slot]` from `.field`.
			parameters.push(Node::accessor(material_textures, Node::sentence(vec![slot])));
			parameters.push(Node::member_expression("vertex_uv"));
			parameters.push(Node::member_expression("uv_derivative_x"));
			parameters.push(Node::member_expression("uv_derivative_y"));
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
		| besl::parser::Expressions::Continue
		| besl::parser::Expressions::Discard => {}
	}
}

// These statements are spliced around the material-authored main body. Keeping
// them in BESL lets each backend derive resource access, packed loads, type names,
// and matrix multiplication from the linked AST.
