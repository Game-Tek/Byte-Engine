//! The statements wrapped around a material body, and the parser-tree transforms applied to them.

use std::borrow::Cow;

use besl::parser::{ElseBranch, Expressions, Node, Nodes};

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
pub(super) fn parse_besl_statements(source: &'static str, function_name: &str) -> Vec<Node<'static>> {
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
				| Expressions::Continue
				| Expressions::Break
				| Expressions::Discard => {}
			}
		}
		_ => {}
	}
}

/// The `MaterialReconstructionFeatures` struct records what a material reads and writes, so the prefix reconstructs
/// only the pixel attributes it needs and the suffix rotates only a normal it wrote.
#[derive(Clone, Copy, Default)]
pub(super) struct MaterialReconstructionFeatures {
	/// The material reads the vertex texture coordinates, directly or by sampling a texture.
	pub reads_uv: bool,
	/// The material reads the tangent or the bitangent.
	pub reads_tangent_frame: bool,
	/// The material writes a tangent-space normal, which the suffix turns out of the tangent frame.
	pub writes_normal: bool,
}

impl MaterialReconstructionFeatures {
	/// Reports whether the prefix rebuilds the tangent frame.
	fn needs_tangent_frame(self) -> bool {
		self.reads_tangent_frame || self.writes_normal
	}

	/// Reports whether the prefix interpolates the texture coordinates, which the tangent frame is built from too.
	fn needs_uv(self) -> bool {
		self.reads_uv || self.needs_tangent_frame()
	}
}

impl MaterialReconstructionFeatures {
	/// Returns what any of `features` needs, so one reconstruction serves every material of an uber shader.
	pub(super) fn union(features: impl IntoIterator<Item = Self>) -> Self {
		features.into_iter().fold(Self::default(), |all, features| Self {
			reads_uv: all.reads_uv || features.reads_uv,
			reads_tangent_frame: all.reads_tangent_frame || features.reads_tangent_frame,
			writes_normal: all.writes_normal || features.writes_normal,
		})
	}
}

/// Statements that pick a per-material pipeline's pixel from its slot's list and reconstruct the pixel's surface
/// before the material body runs.
pub(super) fn material_evaluation_prefix_statements(features: MaterialReconstructionFeatures) -> Vec<Node<'static>> {
	let mut statements = parse_besl_statements(MATERIAL_EVALUATION_PIXEL_LIST_SOURCE, "material_evaluation_pixel_list");
	statements.extend(material_surface_statements(features));
	statements
}

/// Statements that start an uber evaluation shader's thread: the opaque phase's workgroup window of indirect diffuse
/// texels, then the thread's own pixel of its tile and the arm key of its material, `material_arm`.
pub(super) fn material_evaluation_tile_statements(transparent: bool) -> Vec<Node<'static>> {
	let mut statements = Vec::new();
	// Only opaque surfaces read indirect diffuse light, so only the opaque shader loads the window and waits for it.
	if !transparent {
		statements.extend(material_evaluation_tile_window_statements());
	}
	statements.extend(parse_besl_statements(
		MATERIAL_EVALUATION_TILE_SOURCE,
		"material_evaluation_tile",
	));
	statements
}

/// Statements that load an opaque uber shader tile's workgroup window of indirect diffuse texels and wait for it.
pub(super) fn material_evaluation_tile_window_statements() -> Vec<Node<'static>> {
	parse_besl_statements(MATERIAL_EVALUATION_TILE_WINDOW_SOURCE, "material_evaluation_tile_window")
}

/// Statements that reconstruct the pixel's surface from the visibility buffer, the attributes `features` asks for,
/// and the material outputs' defaults. A pixel source runs first.
pub(super) fn material_surface_statements(features: MaterialReconstructionFeatures) -> Vec<Node<'static>> {
	let mut statements = parse_besl_statements(MATERIAL_EVALUATION_PREFIX_SOURCE, "material_evaluation_prefix");
	if features.needs_uv() {
		statements.extend(parse_besl_statements(MATERIAL_EVALUATION_UV_SOURCE, "material_evaluation_uv"));
	}
	if features.needs_tangent_frame() {
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

/// Statements that rebuild a ray tracing hit's surface before a closest-hit shader's material body runs.
///
/// Like [`material_evaluation_prefix_statements`], they interpolate texture coordinates and build the tangent frame
/// only for a material that reads them.
pub(super) fn material_hit_prefix_statements(features: MaterialReconstructionFeatures) -> Vec<Node<'static>> {
	let mut statements = parse_besl_statements(MATERIAL_HIT_PREFIX_SOURCE, "material_hit_prefix");
	if features.needs_uv() {
		statements.extend(parse_besl_statements(MATERIAL_HIT_UV_SOURCE, "material_hit_uv"));
	}
	if features.needs_tangent_frame() {
		statements.extend(parse_besl_statements(MATERIAL_HIT_TANGENT_SOURCE, "material_hit_tangent"));
	}
	statements.extend(parse_besl_statements(MATERIAL_HIT_DEFAULTS_SOURCE, "material_hit_defaults"));
	statements
}

/// Statements that write a closest-hit shader's payload after its material body runs, ending with its `return`.
pub(super) fn material_hit_suffix_statements() -> Vec<Node<'static>> {
	parse_besl_statements(MATERIAL_HIT_SUFFIX_SOURCE, "material_hit_suffix")
}

/// Statements that light the material outputs after the material body runs.
///
/// The code of every feature `visibility_features` leaves out is removed, so the shader declares none of its bindings.
pub(super) fn material_evaluation_suffix_statements(
	features: MaterialReconstructionFeatures,
	visibility_features: VisibilityFeatures,
) -> Vec<Node<'static>> {
	let mut statements = material_normal_statements(features);
	statements.extend(material_lighting_statements(visibility_features));
	statements
}

/// Statements that turn the normal a material body wrote out of the tangent frame, or keep the geometric normal for
/// a material that wrote none.
pub(super) fn material_normal_statements(features: MaterialReconstructionFeatures) -> Vec<Node<'static>> {
	let normal_source = if features.writes_normal {
		MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE
	} else {
		MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE
	};
	parse_besl_statements(normal_source, "material_evaluation_normal")
}

/// Statements that light the material outputs and write the shown debug views, without the code of every feature
/// `visibility_features` leaves out. An uber shader's scope reads the indirect diffuse taps from its tile window.
pub(super) fn material_lighting_statements(visibility_features: VisibilityFeatures) -> Vec<Node<'static>> {
	let mut statements = parse_besl_statements(MATERIAL_EVALUATION_SUFFIX_SOURCE, "material_evaluation_suffix");
	statements.extend(parse_besl_statements(
		material_debug_view_source(),
		"material_evaluation_debug_views",
	));
	visibility_features.remove_left_out_code(&mut statements);
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
	// compressed with `x / (1 + x)` so daylight stays readable. The suffix sums it at this view's index.
	(
		"irradiance",
		"vec4f(exposed_irradiance.x / (1.0 + exposed_irradiance.x), exposed_irradiance.y / (1.0 + exposed_irradiance.y), exposed_irradiance.z / (1.0 + exposed_irradiance.z), 1.0)",
	),
];

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
/// A removed `let` takes the statements after it that read its local with it, so a feature's code may keep a value
/// in a local, as in `let slot: u32 = debug_view_slot(parameters.debug_views, 0u); if (slot != 0) { ... }`.
///
/// A conditional or loop that does not read it keeps its block, minus the statements inside that do. Backends emit
/// only what `main` reaches, so this also drops the bindings and helpers only the removed statements used.
pub(crate) fn remove_statements_reading<'a>(statements: &mut Vec<Node<'a>>, name: &'a str) {
	remove_statements_reading_any(statements, &mut vec![Cow::Borrowed(name)]);
}

/// Removes the statements that read any of `names`, adding the local of each removed `let` to `names` for the
/// statements after it. A nested block's removed locals go out of scope when the block ends.
fn remove_statements_reading_any<'a>(statements: &mut Vec<Node<'a>>, names: &mut Vec<Cow<'a, str>>) {
	let reads_any = |node: &mut Node<'a>, names: &[Cow<'a, str>]| {
		let mut reads = false;
		walk_expressions(node, &mut |expression| {
			reads |= matches!(expression, Expressions::Member { name } if names.contains(name));
		});
		reads
	};
	// Runs `remove` on a nested block, then forgets the locals the block declared.
	let in_scope = |names: &mut Vec<Cow<'a, str>>, statements: &mut Vec<Node<'a>>| {
		let outer = names.len();
		remove_statements_reading_any(statements, names);
		names.truncate(outer);
	};

	statements.retain_mut(|statement| match statement.node_mut() {
		Nodes::Conditional {
			condition,
			statements,
			else_branch,
		} => {
			if reads_any(condition, names) {
				return false;
			}
			in_scope(names, statements);
			*else_branch = match else_branch.take() {
				Some(ElseBranch::Block(mut statements)) => {
					in_scope(names, &mut statements);
					Some(ElseBranch::Block(statements))
				}
				Some(ElseBranch::If(conditional)) => {
					// An `else if` is one conditional: dropping it drops the whole link, as a block would be.
					let mut link = vec![*conditional];
					in_scope(names, &mut link);
					link.pop().map(|conditional| ElseBranch::If(Box::new(conditional)))
				}
				None => None,
			};
			true
		}
		Nodes::ForLoop { statements, .. } => {
			in_scope(names, statements);
			true
		}
		Nodes::Match { arms, .. } => {
			for arm in arms {
				in_scope(names, &mut arm.statements);
			}
			true
		}
		_ => {
			if !reads_any(statement, names) {
				return true;
			}
			// The statements after this one can't read a local it declared.
			walk_expressions(statement, &mut |expression| {
				if let Expressions::VariableDeclaration { name, .. } = expression {
					names.push(name.clone());
				}
			});
			false
		}
	});
}
