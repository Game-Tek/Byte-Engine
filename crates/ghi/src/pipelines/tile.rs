//! Tile pipelines: a tile function that a render pass runs once per tile of its color attachments.
//!
//! A tile function reads and writes its pass's attachments through tile memory, so they need render-target usage
//! and never shader-write usage, which keeps them losslessly compressed on GPUs that compress render targets. Check
//! [`crate::context::Context::supports_tile_functions`] before creating one, then record it with
//! [`crate::command_buffer::CommandBufferRecording::start_tile_pass`].

use crate::Formats;

/// The `Builder` struct collects the state of a tile pipeline before a backend creates its native pipeline.
///
/// The shader's threadgroup size is the number of threads that run for each tile, at most one per pixel.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) shader: crate::pipelines::ShaderParameter<'a>,
	pub(crate) outputs: &'a [Formats],
	pub(crate) tile: u32,
	pub(crate) threadgroup_memory: &'a [u32],
}

impl<'a> Builder<'a> {
	/// Describes a pipeline that runs `shader` on `tile` x `tile` pixel tiles of color attachments of `outputs`
	/// formats, in attachment order.
	///
	/// Tiles are 16 or 32 pixels wide, the tile sizes render passes take.
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		shader: crate::pipelines::ShaderParameter<'a>,
		outputs: &'a [Formats],
		tile: u32,
	) -> Self {
		assert!(
			matches!(tile, 16 | 32),
			"Invalid tile size {tile}. The most likely cause is a tile size that render passes don't take. Use 16 or 32 pixels.",
		);
		assert!(
			!outputs.is_empty() && outputs.len() <= 8,
			"Invalid tile pipeline outputs. The most likely cause is a tile function with no output or more outputs than a render pass has color attachments. Give it one to eight outputs.",
		);
		Self {
			name: None,
			push_constant_ranges,
			shader,
			outputs,
			tile,
			threadgroup_memory: &[],
		}
	}

	/// Names this pipeline for graphics debuggers.
	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	/// Reserves `threadgroup_memory[n]` bytes, a multiple of 16, for threadgroup argument `n` of the shader: the
	/// workgroup storage its tiles share.
	pub fn threadgroup_memory(mut self, threadgroup_memory: &'a [u32]) -> Self {
		assert!(
			threadgroup_memory.iter().all(|length| length % 16 == 0),
			"Invalid tile threadgroup memory {threadgroup_memory:?}. The most likely cause is a length that wasn't rounded up to Metal's 16-byte threadgroup alignment. Round every length up to a multiple of 16.",
		);
		self.threadgroup_memory = threadgroup_memory;
		self
	}
}

/// Panics because `backend` has no tile functions.
///
/// [`crate::context::Context::supports_tile_functions`] is always `false` on such a backend, so reaching this means
/// the application skipped that check.
pub(crate) fn unsupported(backend: &str) -> ! {
	panic!(
		"{backend} has no tile functions. The most likely cause is that the application did not check Context::supports_tile_functions, which is always false on {backend}. Check Context::supports_tile_functions, and run the shader as a compute shader when it is false."
	)
}
