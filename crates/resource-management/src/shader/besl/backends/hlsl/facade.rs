use utils::hash::HashMap;

use crate::shader::generator::Stages;

/// The `Generator` struct exists to produce HLSL source for DirectX-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	pub(crate) minified: bool,
	/// The stage being generated, which decides entry-point signatures, semantics, and interpolation.
	pub(crate) stage: Stages,
	pub(crate) mesh_uses_render_target_array_index: bool,
	pub(crate) task_payloads: Vec<besl::NodeReference>,
	pub(crate) mesh_outputs: Vec<besl::NodeReference>,
	pub(crate) raster_inputs: Vec<besl::NodeReference>,
	pub(crate) raster_outputs: Vec<besl::NodeReference>,
	pub(crate) user_struct_constructors: Vec<besl::NodeReference>,
	pub(crate) packed_write_counter: u32,
	pub(crate) atomic_temporary_counter: u32,
	pub(crate) atomic_temporaries: HashMap<besl::NodeReference, String>,
	pub(crate) match_break_depth: Option<usize>,
}

/// The `HlslBufferBindingSource` struct preserves the binding metadata needed while flattening BESL buffers for HLSL.
pub(crate) struct HlslBufferBindingSource {
	pub(crate) name: String,
	pub(crate) write: bool,
	/// The `u8` or `u16` element of an array buffer, which DX12 packs into 32-bit words.
	pub(crate) narrow_element: Option<&'static str>,
}

impl Generator {
	/// Creates an HLSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			stage: Stages::Vertex,
			mesh_uses_render_target_array_index: false,
			task_payloads: Vec::new(),
			mesh_outputs: Vec::new(),
			raster_inputs: Vec::new(),
			raster_outputs: Vec::new(),
			user_struct_constructors: Vec::new(),
			packed_write_counter: 0,
			atomic_temporary_counter: 0,
			atomic_temporaries: HashMap::default(),
			match_break_depth: None,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}
}
