/// The `Generator` struct exists to produce GLSL source for Vulkan-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	pub(super) minified: bool,
	pub(super) current_stage_interpolates_inputs: bool,
	pub(super) current_stage_interpolates_outputs: bool,
	pub(super) current_stage_supports_workgroup_storage: bool,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates a GLSL generator with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage_interpolates_inputs: false,
			current_stage_interpolates_outputs: false,
			current_stage_supports_workgroup_storage: false,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	/// Reports whether one reachable AST branch uses the requested intrinsic.
	fn uses_intrinsic(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
		match node.borrow().node() {
			besl::Nodes::Function { statements, .. } => statements
				.iter()
				.any(|statement| Self::uses_intrinsic(statement, intrinsic_name)),
			besl::Nodes::Conditional { condition, statements } => {
				Self::uses_intrinsic(condition, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				Self::uses_intrinsic(initializer, intrinsic_name)
					|| Self::uses_intrinsic(condition, intrinsic_name)
					|| Self::uses_intrinsic(update, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				} => {
					intrinsic.borrow().get_name() == Some(intrinsic_name)
						|| arguments
							.iter()
							.any(|argument| Self::uses_intrinsic(argument, intrinsic_name))
				}
				besl::Expressions::Operator { left, right, .. } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::FunctionCall { parameters, .. } => parameters
					.iter()
					.any(|parameter| Self::uses_intrinsic(parameter, intrinsic_name)),
				besl::Expressions::Expression { elements } => {
					elements.iter().any(|element| Self::uses_intrinsic(element, intrinsic_name))
				}
				besl::Expressions::Macro { body, .. } => Self::uses_intrinsic(body, intrinsic_name),
				besl::Expressions::Member { source, .. } => Self::uses_intrinsic(source, intrinsic_name),
				besl::Expressions::Return { value } => value
					.as_ref()
					.is_some_and(|value| Self::uses_intrinsic(value, intrinsic_name)),
				besl::Expressions::Accessor { left, right } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. }
				| besl::Expressions::Continue
				| besl::Expressions::Discard => false,
			},
			_ => false,
		}
	}

	/// Reports whether reachable code uses one of BESL's compute-only subgroup operations.
	pub(super) fn uses_subgroup_intrinsics(order: &[besl::NodeReference]) -> bool {
		const SUBGROUP_INTRINSICS: [&str; 8] = [
			"subgroup_lane_index",
			"subgroup_ballot",
			"subgroup_ballot_any",
			"subgroup_ballot_find_lsb",
			"subgroup_ballot_count",
			"subgroup_ballot_and_not",
			"subgroup_broadcast_u32",
			"subgroup_broadcast_f32",
		];
		order.iter().any(|node| {
			SUBGROUP_INTRINSICS
				.iter()
				.any(|intrinsic| Self::uses_intrinsic(node, intrinsic))
		})
	}

	/// Reports whether reachable code requires native 16-bit floating-point arithmetic.
	pub(super) fn uses_f16_types(order: &[besl::NodeReference]) -> bool {
		const F16_TYPES: [&str; 4] = ["f16", "vec2f16", "vec3f16", "vec4f16"];
		order
			.iter()
			.any(|node| matches!(node.borrow().node(), besl::Nodes::Struct { name, .. } if F16_TYPES.contains(&name.as_str())))
			|| order
				.iter()
				.any(|node| F16_TYPES.iter().any(|name| Self::uses_intrinsic(node, name)))
	}
}

use std::cell::RefCell;

use crate::shader::generator::{
	MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages, ordered_shader_nodes,
};
