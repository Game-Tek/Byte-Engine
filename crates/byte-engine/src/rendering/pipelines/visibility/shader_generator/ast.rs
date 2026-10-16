//! Parser-tree transforms applied to an authored material `main` before linking.

use besl::parser::{ElseBranch, Expressions, Node, Nodes, TypeName};

use super::sources::*;
use crate::rendering::pipelines::visibility::VisibilityFeatures;

/// Parses one reusable BESL helper function from an isolated source scope.
pub(super) fn parse_besl_function(source: &'static str, function_name: &str) -> Node<'static> {
	let mut root = besl::parse(source).unwrap_or_else(|error| {
		panic!(
			"Failed to parse `{function_name}`. The most likely cause is invalid BESL syntax in the visibility shader module: {error:?}"
		)
	});
	match root.node_mut() {
		Nodes::Scope { children, .. } if children.len() == 1 => children.remove(0),
		_ => panic!(
			"Invalid `{function_name}` helper scope. The most likely cause is that its BESL source defines more than one top-level element."
		),
	}
}

/// Extracts the statements of one BESL helper function so they can be spliced into `main`.
fn parse_besl_statements(source: &'static str, function_name: &str) -> Vec<Node<'static>> {
	match parse_besl_function(source, function_name).node_mut() {
		Nodes::Function { statements, .. } => std::mem::take(statements),
		_ => panic!(
			"Invalid `{function_name}` helper. The most likely cause is that its BESL source no longer defines a function."
		),
	}
}

/// Visits every expression under a function body, parents before children.
fn walk_expressions<'a>(node: &mut Node<'a>, visit: &mut impl FnMut(&mut Expressions<'a>)) {
	match node.node_mut() {
		Nodes::Function { statements, .. } => statements.iter_mut().for_each(|statement| walk_expressions(statement, visit)),
		Nodes::Conditional {
			condition,
			statements,
			else_branch,
		} => {
			walk_expressions(condition, visit);
			statements
				.iter_mut()
				.chain(else_branch.as_mut().map_or(&mut [][..], ElseBranch::statements_mut))
				.for_each(|statement| walk_expressions(statement, visit));
		}
		Nodes::Match { scrutinee, arms } => {
			walk_expressions(scrutinee, visit);
			arms.iter_mut()
				.flat_map(|arm| &mut arm.statements)
				.for_each(|statement| walk_expressions(statement, visit));
		}
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			walk_expressions(initializer, visit);
			walk_expressions(condition, visit);
			walk_expressions(update, visit);
			statements.iter_mut().for_each(|statement| walk_expressions(statement, visit));
		}
		Nodes::Expression(expression) => {
			visit(expression);
			match expression {
				Expressions::Call { parameters, .. } | Expressions::Expression(parameters) => {
					parameters.iter_mut().for_each(|parameter| walk_expressions(parameter, visit));
				}
				Expressions::RecordLiteral { fields } => {
					fields.iter_mut().for_each(|field| walk_expressions(&mut field.value, visit))
				}
				Expressions::Accessor { left, right } | Expressions::Operator { left, right, .. } => {
					walk_expressions(left, visit);
					walk_expressions(right, visit);
				}
				Expressions::Unary { operand, .. } => walk_expressions(operand, visit),
				Expressions::Return { value: Some(value) } | Expressions::Yield { value } => walk_expressions(value, visit),
				Expressions::Macro { body, .. } => walk_expressions(body, visit),
				Expressions::Return { value: None }
				| Expressions::Member { .. }
				| Expressions::Literal { .. }
				| Expressions::VariableDeclaration { .. }
				| Expressions::RawCode { .. }
				| Expressions::Continue
				| Expressions::Break
				| Expressions::Discard => {}
			}
		}
		_ => {}
	}
}

fn member_name<'a>(node: &'a Node<'_>) -> Option<&'a str> {
	match node.node() {
		Nodes::Expression(Expressions::Member { name }) => Some(name.as_ref()),
		_ => None,
	}
}

/// The `MaterialReconstructionFeatures` struct records which optional pixel attributes a material needs reconstructed.
#[derive(Clone, Copy, Default)]
pub(super) struct MaterialReconstructionFeatures {
	uses_uv: bool,
	uses_tangent_frame: bool,
}

/// Finds material sampling and tangent-frame operations before texture shorthand is expanded.
pub(super) fn material_reconstruction_features(main: &mut Node<'_>) -> MaterialReconstructionFeatures {
	const UV_CALLS: [&str; 6] = [
		"sample_material",
		"sample_normal",
		"decode_material_normal",
		"decode_material_normal_f16",
		"scale_normal_xy",
		"scale_material_normal_xy_f16",
	];
	let mut features = MaterialReconstructionFeatures::default();
	walk_expressions(main, &mut |expression| match expression {
		Expressions::Call {
			name: TypeName::Named(name),
			..
		} if UV_CALLS.contains(name) => {
			features.uses_uv = true;
			features.uses_tangent_frame |= *name != "sample_material";
		}
		// Assigning anything but the canonical no-normal-map value needs the tangent frame.
		Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		} if member_name(left) == Some("normal") && !is_default_tangent_normal(right) => {
			features.uses_uv = true;
			features.uses_tangent_frame = true;
		}
		Expressions::Member { name } => match name.as_ref() {
			"vertex_uv" => features.uses_uv = true,
			"T" | "B" => {
				features.uses_uv = true;
				features.uses_tangent_frame = true;
			}
			_ => {}
		},
		_ => {}
	});
	features
}

/// Recognizes the canonical no-normal-map value emitted by the BRDF generators: `vec3f(0, 0, 1)` in f32 or f16.
fn is_default_tangent_normal(node: &Node<'_>) -> bool {
	let Nodes::Expression(Expressions::Call { name, parameters }) = node.node() else {
		return false;
	};
	if !matches!(name, TypeName::Named(name) if matches!(*name, "vec3f" | "vec3f16")) || parameters.len() != 3 {
		return false;
	}
	parameters.iter().zip([0.0_f32, 0.0, 1.0]).all(|(parameter, expected)| {
		let literal = match parameter.node() {
			Nodes::Expression(Expressions::Literal { value }) => Some(value),
			Nodes::Expression(Expressions::Call { name, parameters })
				if matches!(name, TypeName::Named(name) if *name == "f16") && parameters.len() == 1 =>
			{
				match parameters[0].node() {
					Nodes::Expression(Expressions::Literal { value }) => Some(value),
					_ => None,
				}
			}
			_ => None,
		};
		literal.is_some_and(|value| value.parse::<f32>().is_ok_and(|value| value == expected))
	})
}

/// Statements that reconstruct the pixel's surface before the authored material body runs.
pub(super) fn material_evaluation_prefix_statements(features: MaterialReconstructionFeatures) -> Vec<Node<'static>> {
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

/// Statements that light the material outputs after the authored body runs.
///
/// The code of every feature `visibility_features` leaves out is removed, so the shader declares none of its bindings.
pub(super) fn material_evaluation_suffix_statements(
	features: MaterialReconstructionFeatures,
	visibility_features: VisibilityFeatures,
) -> Vec<Node<'static>> {
	let normal_source = if features.uses_tangent_frame {
		MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE
	} else {
		MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE
	};
	let mut statements = parse_besl_statements(normal_source, "material_evaluation_normal");
	let mut suffix = parse_besl_statements(MATERIAL_EVALUATION_SUFFIX_SOURCE, "material_evaluation_suffix");
	suffix.extend(parse_besl_statements(
		material_debug_view_source(),
		"material_evaluation_debug_views",
	));
	visibility_features.remove_left_out_code(&mut suffix);
	statements.extend(suffix);
	statements
}

/// The debug views material evaluation declares, in the order the shader numbers them: each view's name, as
/// `render.debug.view.<manager>.<name>` selects it, and the display-ready color it writes, as a BESL `vec4f`
/// expression over the material's variables.
pub(crate) const MATERIAL_DEBUG_VIEWS: [(&str, &str); 6] = [
	(
		"normals",
		"vec4f(f32(normal.x) * 0.5 + 0.5, f32(normal.y) * 0.5 + 0.5, f32(normal.z) * 0.5 + 0.5, 1.0)",
	),
	("albedo", "vec4f(f32(albedo.x), f32(albedo.y), f32(albedo.z), 1.0)"),
	("roughness", "vec4f(f32(roughness), f32(roughness), f32(roughness), 1.0)"),
	("metalness", "vec4f(f32(metalness), f32(metalness), f32(metalness), 1.0)"),
	("occlusion", "vec4f(f32(occlusion), f32(occlusion), f32(occlusion), 1.0)"),
	// The irradiance reaching the surface from every light and the environment, pre-exposed like the lit map and
	// compressed with `x / (1 + x)` so daylight stays readable. The suffix sums it at this index; see
	// `MATERIAL_IRRADIANCE_VIEW`.
	(
		"irradiance",
		"vec4f(exposed_irradiance.x / (1.0 + exposed_irradiance.x), exposed_irradiance.y / (1.0 + exposed_irradiance.y), exposed_irradiance.z / (1.0 + exposed_irradiance.z), 1.0)",
	),
];

/// The index of the irradiance view in [`MATERIAL_DEBUG_VIEWS`], which the material evaluation suffix gates its
/// irradiance sum on.
pub(crate) const MATERIAL_IRRADIANCE_VIEW: usize = 5;

/// The names of [`MATERIAL_DEBUG_VIEWS`], as the pipeline manager declares them.
pub(crate) const MATERIAL_DEBUG_VIEW_NAMES: [&str; MATERIAL_DEBUG_VIEWS.len()] = {
	let mut names = [""; MATERIAL_DEBUG_VIEWS.len()];
	let mut index = 0;
	while index < names.len() {
		names[index] = MATERIAL_DEBUG_VIEWS[index].0;
		index += 1;
	}
	names
};

/// The statements that write each shown debug view of [`MATERIAL_DEBUG_VIEWS`], generated once from the table so
/// the shader's numbering cannot drift from the declared names. Every statement reads `debug_views` in its
/// condition, so a bake without debug views drops them all.
fn material_debug_view_source() -> &'static str {
	static SOURCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
	SOURCE.get_or_init(|| {
		let mut source = String::from("material_evaluation_debug_views: fn () -> void {\n");
		for (view, (name, color)) in MATERIAL_DEBUG_VIEWS.iter().enumerate() {
			source.push_str(&format!(
				"\t// {name}\n\tif (debug_view_slot(push_constant.debug_views, {view}) != 0) {{\n\t\twrite(debug_view_targets[debug_view_slot(push_constant.debug_views, {view}) - 1], pixel_coordinates, {color});\n\t}}\n"
			));
		}
		source.push_str("}\n");
		source
	})
}

/// Removes every statement, at any depth, that reads the member `name`: a conditional whose condition reads it, with
/// its whole block, and any other statement that reads it, such as a `let` or a `write`.
///
/// A conditional or loop that does not read it keeps its block, minus the statements inside that do. Backends emit
/// only what `main` reaches, so this also drops the bindings and helpers only the removed statements used.
pub(crate) fn remove_statements_reading(statements: &mut Vec<Node<'_>>, name: &str) {
	statements.retain_mut(|statement| {
		let reads_name = |node: &mut Node<'_>| {
			let mut reads_name = false;
			walk_expressions(node, &mut |expression| {
				reads_name |= matches!(expression, Expressions::Member { name: member } if member.as_ref() == name);
			});
			reads_name
		};
		match statement.node_mut() {
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => {
				if reads_name(condition) {
					return false;
				}
				remove_statements_reading(statements, name);
				*else_branch = match else_branch.take() {
					Some(ElseBranch::Block(mut statements)) => {
						remove_statements_reading(&mut statements, name);
						Some(ElseBranch::Block(statements))
					}
					Some(ElseBranch::If(conditional)) => {
						// An `else if` is one conditional: dropping it drops the whole link, as a block would be.
						let mut link = vec![*conditional];
						remove_statements_reading(&mut link, name);
						link.pop().map(|conditional| ElseBranch::If(Box::new(conditional)))
					}
					None => None,
				};
				true
			}
			Nodes::ForLoop { statements, .. } => {
				remove_statements_reading(statements, name);
				true
			}
			Nodes::Match { arms, .. } => {
				for arm in arms {
					remove_statements_reading(&mut arm.statements, name);
				}
				true
			}
			_ => !reads_name(statement),
		}
	});
}

/// Narrows material property assignments so every material graph uses the compact evaluation ABI.
pub(super) fn narrow_material_property_assignments(main: &mut Node<'_>) {
	walk_expressions(main, &mut |expression| {
		let Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		} = expression
		else {
			return;
		};
		let target_type = match member_name(left) {
			Some("albedo") => "vec4f16",
			Some("normal" | "emission") => "vec3f16",
			Some("metalness" | "roughness" | "occlusion") => "f16",
			_ => return,
		};
		**right = Node::call(target_type, vec![*right.clone()]);
	});
}

/// Expands `sample_material(texture)` and `sample_normal(texture)` into explicit bindless gradient samples.
pub(super) fn add_material_sample_context(main: &mut Node<'_>, texture_slots: &[(&str, u32)]) {
	walk_expressions(main, &mut |expression| {
		let Expressions::Call {
			name: TypeName::Named(name),
			parameters,
		} = expression
		else {
			return;
		};
		if !matches!(*name, "sample_material" | "sample_normal") || parameters.len() != 1 {
			return;
		}
		// The backend intrinsic consumes the bindless binding directly; the normal helper captures it from the scope.
		let passes_bindless_binding = *name == "sample_material";
		*name = if passes_bindless_binding {
			"sample_texture_2d_array_grad"
		} else {
			"sample_visibility_normal"
		};

		let slot = parameters.remove(0);
		let slot = member_name(&slot)
			.and_then(|texture_name| texture_slots.iter().find(|(name, _)| *name == texture_name))
			.map_or(slot, |(_, index)| Node::literal_expression(format!("{index}u")));
		if passes_bindless_binding {
			parameters.push(Node::member_expression("textures"));
		}
		// Index access keeps an expression on its right side so the linked AST can distinguish `textures[slot]` from `.field`.
		let material_textures = Node::accessor(Node::member_expression("material"), Node::member_expression("textures"));
		parameters.push(Node::accessor(material_textures, Node::sentence(vec![slot])));
		parameters.push(Node::member_expression("vertex_uv"));
		parameters.push(Node::member_expression("uv_derivative_x"));
		parameters.push(Node::member_expression("uv_derivative_y"));
	});
}
