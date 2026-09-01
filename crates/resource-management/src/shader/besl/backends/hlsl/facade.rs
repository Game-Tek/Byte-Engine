use std::{cell::RefCell, collections::HashMap, fmt::Write as _};

use super::*;
use crate::shader::generator::{
	MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
	emit_comma_separated_nodes, ordered_shader_nodes,
};

/// The `Generator` struct exists to produce HLSL source for DirectX-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	pub(crate) minified: bool,
	pub(crate) current_stage: HlslStage,
	pub(crate) current_stage_interpolates_inputs: bool,
	pub(crate) current_stage_interpolates_outputs: bool,
	pub(crate) current_local_size: Option<utils::Extent>,
	pub(crate) current_mesh_maximum_vertices: u32,
	pub(crate) current_mesh_maximum_primitives: u32,
	pub(crate) mesh_uses_render_target_array_index: bool,
	pub(crate) task_payloads: Vec<besl::NodeReference>,
	pub(crate) mesh_outputs: Vec<besl::NodeReference>,
	pub(crate) raster_inputs: Vec<besl::NodeReference>,
	pub(crate) raster_outputs: Vec<besl::NodeReference>,
	pub(crate) user_struct_constructors: Vec<besl::NodeReference>,
	pub(crate) packed_write_counter: u32,
	pub(crate) atomic_temporary_counter: u32,
	pub(crate) atomic_temporaries: HashMap<besl::NodeReference, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HlslStage {
	Vertex,
	Fragment,
	Compute,
	Task,
	Mesh,
}

/// The `HlslBufferBindingSource` struct preserves the binding metadata needed while flattening BESL buffers for HLSL.
pub(crate) struct HlslBufferBindingSource {
	pub(crate) name: String,
	pub(crate) write: bool,
	pub(crate) flattened_member: Option<String>,
	pub(crate) flattened_element_type: Option<String>,
}

impl ShaderGenerator for Generator {}

impl Generator {
	/// Creates an HLSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			current_stage: HlslStage::Vertex,
			current_stage_interpolates_inputs: false,
			current_stage_interpolates_outputs: false,
			current_local_size: None,
			current_mesh_maximum_vertices: 0,
			current_mesh_maximum_primitives: 0,
			mesh_uses_render_target_array_index: false,
			task_payloads: Vec::new(),
			mesh_outputs: Vec::new(),
			raster_inputs: Vec::new(),
			raster_outputs: Vec::new(),
			user_struct_constructors: Vec::new(),
			packed_write_counter: 0,
			atomic_temporary_counter: 0,
			atomic_temporaries: HashMap::new(),
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}
