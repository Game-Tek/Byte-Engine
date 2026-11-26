//! The masked raster passes' fragment entry points, which the coverage shaders splice their arms into.
//!
//! Each reads the instance's material word, which the mesh stage passes, its table index, and the vertex UV under the names
//! `CoverageStage` lowers `opacity` against, starts `opacity` at 1, runs the arm of the material's `coverage_arm`, and
//! leaves out the fragment where `opacity` falls below the material's cutoff. The generator inserts the arms' `match`
//! right before the cutoff test. The visibility pass dithers the cutoff in a jittered frame, as the masked cutoff in the
//! rendering reference specifies; the shadow passes keep it fixed.

/// The masked visibility pass's fragment, which writes the primitive and instance it keeps.
pub(crate) const COVERAGE_VISIBILITY_SOURCE: &str = r#"
coverage_visibility: fn (
	input: StageInput,
	// The mesh stage writes the instance, its material word, and the UV once per vertex, and the primitive once per
	// primitive.
	pipeline_input: interface {
		instance_index: u32,
		material_word: u32,
		primitive_index: primitive<u32>,
		uv: vec2f16,
	},
) -> output {
	primitive_index: u32,
	instance_index: u32,
} {
	let material_word: u32 = pipeline_input.material_word;
	let material_index: u32 = material_word & MESH_MATERIAL_MASK;
	let coverage_arm: u32 = scene.materials.materials[material_index].coverage_arm;
	let vertex_uv: vec2f = vec2f(pipeline_input.uv);
	let opacity: f32 = 1.0;
	// In a jittered frame the threshold is the pixel's blue noise, so fragments near the cutoff are dithered; see
	// `masked_coverage`.
	let threshold: f32 = 0.5;
	let noise_slice: u32 = scene.frame.noise_slice;
	if (noise_slice != 4294967295) {
		let pixel: vec2u = input.thread_id;
		threshold = fetch(scene.blue_noise, vec2u(pixel.x % 64, pixel.y % 64 + 64 * noise_slice)).x;
	}
	if (!masked_coverage(opacity, scene.materials.materials[material_index].alpha_cutoff, fwidth(opacity), threshold)) {
		discard;
	}
	return {
		primitive_index: pipeline_input.primitive_index,
		instance_index: pipeline_input.instance_index,
	};
}
"#;

/// The masked shadow passes' fragment, which writes only depth.
pub(crate) const COVERAGE_DEPTH_SOURCE: &str = r#"
coverage_depth: fn (
	input: StageInput,
	// The mesh stage writes the instance's material word and the UV once per vertex.
	pipeline_input: interface {
		material_word: u32,
		uv: vec2f16,
	},
) -> void {
	let material_word: u32 = pipeline_input.material_word;
	let material_index: u32 = material_word & MESH_MATERIAL_MASK;
	let coverage_arm: u32 = scene.materials.materials[material_index].coverage_arm;
	let vertex_uv: vec2f = vec2f(pipeline_input.uv);
	let opacity: f32 = 1.0;
	if (opacity < scene.materials.materials[material_index].alpha_cutoff) {
		discard;
	}
}
"#;

/// How many statements of either source come before the cutoff test, which is where the arms go.
pub(crate) const COVERAGE_ARMS_POSITION: usize = 5;
