//! The masked raster passes' fragment entry points, which the coverage shaders splice their arms into.
//!
//! Each reads the instance's material word and table index and the vertex UV under the names
//! `CoverageStage` lowers `opacity` against, starts `opacity` at 1, runs the arm of the material's coverage key, and
//! leaves out the fragment where `opacity` falls below the material's cutoff. The generator inserts the arms' `match`
//! right before the cutoff test.

/// The masked visibility pass's fragment, which writes the primitive and instance it keeps.
pub(crate) const COVERAGE_VISIBILITY_SOURCE: &str = r#"
coverage_visibility: fn (
	input: StageInput,
	// The mesh stage writes the instance and primitive once per primitive, and the UV per vertex.
	pipeline_input: interface {
		instance_index: primitive<u32>,
		primitive_index: primitive<u32>,
		uv: vec2f16,
	},
) -> output {
	primitive_index: u32,
	instance_id: u32,
} {
	let material_word: u32 = scene.meshes[pipeline_input.instance_index].material;
	let material_index: u32 = material_word & MESH_MATERIAL_MASK;
	let vertex_uv: vec2f = vec2f(pipeline_input.uv);
	let opacity: f32 = 1.0;
	if (opacity < scene.materials.materials[material_index].alpha_cutoff) {
		discard;
	}
	return {
		primitive_index: pipeline_input.primitive_index,
		instance_id: pipeline_input.instance_index,
	};
}
"#;

/// The masked shadow passes' fragment, which writes only depth.
pub(crate) const COVERAGE_DEPTH_SOURCE: &str = r#"
coverage_depth: fn (
	input: StageInput,
	// The mesh stage writes the instance and primitive once per primitive, and the UV per vertex.
	pipeline_input: interface {
		instance_index: primitive<u32>,
		primitive_index: primitive<u32>,
		uv: vec2f16,
	},
) -> void {
	let material_word: u32 = scene.meshes[pipeline_input.instance_index].material;
	let material_index: u32 = material_word & MESH_MATERIAL_MASK;
	let vertex_uv: vec2f = vec2f(pipeline_input.uv);
	let opacity: f32 = 1.0;
	if (opacity < scene.materials.materials[material_index].alpha_cutoff) {
		discard;
	}
}
"#;

/// How many statements of either source come before the cutoff test, which is where the arms go.
pub(crate) const COVERAGE_ARMS_POSITION: usize = 4;
