//! Lowers BESL task and mesh stages to `GL_EXT_mesh_shader`.
//!
//! Metal lets one invocation set a task workgroup's mesh grid and keep running. GLSL instead requires every invocation
//! to call `EmitMeshTasksEXT` exactly once, in uniform control flow, and ends the invocation there. The task stage
//! therefore runs BESL's `main` as a function of its own and emits the grid after it returns, from the count BESL's
//! `set_task_mesh_output_count` stored in workgroup memory. GLSL also allows one task payload variable, so every BESL
//! `task_payload` array becomes a member of one payload struct that both stages declare alike.

use std::fmt::Write as _;

use super::Generator;
use crate::shader::generator::{NodeEmitter, ShaderFormatting};

/// The function that holds BESL's task `main`, which the task entry point calls before it emits the mesh grid.
pub(crate) const TASK_MAIN: &str = "_besl_task_main";
/// The workgroup variable `set_task_mesh_output_count` writes and the task entry point emits the mesh grid from.
pub(crate) const TASK_MESH_COUNT: &str = "_besl_task_mesh_count";
/// The one task payload variable, whose members are BESL's `task_payload` arrays.
pub(crate) const TASK_PAYLOAD: &str = "_besl_task_payload";
/// The type of [`TASK_PAYLOAD`].
pub(crate) const TASK_PAYLOAD_TYPE: &str = "_BeslTaskPayload";

impl Generator {
	/// Declares the payload struct and its one `taskPayloadSharedEXT` variable, with a member per `task_payload`
	/// array in the order the program declares them, so the task and mesh stages agree on its layout.
	pub(super) fn emit_task_payload(&self, string: &mut String) {
		self.emit_task_payload_struct(string, TASK_PAYLOAD_TYPE, &self.task_payloads);
		let _ = write!(
			string,
			"taskPayloadSharedEXT {TASK_PAYLOAD_TYPE} {TASK_PAYLOAD};{}",
			ShaderFormatting::new(self.minified).break_str()
		);
	}

	/// Emits BESL's task `main` as [`TASK_MAIN`] and the entry point that runs it and then emits the mesh grid.
	///
	/// Invocation zero clears the count first, so a workgroup that never sets it emits no mesh workgroups. The
	/// barriers order that clear and the count BESL sets against the reads around them.
	pub(super) fn emit_task_entry_point(&mut self, string: &mut String, main: &besl::NodeReference) -> Result<(), ()> {
		self.emit_main_as(string, main, TASK_MAIN, 0)?;
		let _ = write!(
			string,
			"void main(){{if(gl_LocalInvocationIndex==0u){{{TASK_MESH_COUNT}=0u;}}barrier();{TASK_MAIN}();barrier();EmitMeshTasksEXT({TASK_MESH_COUNT},1u,1u);}}{}",
			ShaderFormatting::new(self.minified).break_str()
		);
		Ok(())
	}
}
