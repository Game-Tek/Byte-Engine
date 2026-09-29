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
	pub(super) match_break_depth: Option<usize>,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates a GLSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage_interpolates_inputs: false,
			current_stage_interpolates_outputs: false,
			current_stage_supports_workgroup_storage: false,
			match_break_depth: None,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	/// Reports whether reachable code requires native 16-bit floating-point arithmetic.
	pub(super) fn uses_f16_types(order: &[besl::NodeReference]) -> bool {
		const F16_TYPES: [&str; 4] = ["f16", "vec2f16", "vec3f16", "vec4f16"];
		order
			.iter()
			.any(|node| matches!(node.borrow().node(), besl::Nodes::Struct { name, .. } if F16_TYPES.contains(&name.as_str())))
			|| order
				.iter()
				.any(|node| F16_TYPES.iter().any(|name| super::super::uses_intrinsic(node, name)))
	}
}

use std::cell::RefCell;

use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages, ordered_shader_nodes,
};
