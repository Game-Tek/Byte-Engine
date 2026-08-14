use super::analysis::Generator;
use crate::shader::generator::{MatrixLayouts, ShaderGenerationSettings, Stages};

/// Emits the GLSL version, stage, extension, and layout declarations.
pub(super) fn generate_glsl_header_block(
	generator: &Generator,
	glsl_block: &mut String,
	compilation_settings: &ShaderGenerationSettings,
	uses_subgroup_intrinsics: bool,
	uses_f16_types: bool,
) {
	let glsl_version = &compilation_settings.glsl.version;
	glsl_block.push_str(&format!("#version {glsl_version} core\n"));

	match compilation_settings.stage {
		Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
		Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
		Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
		Stages::Task { .. } => panic!(
			"GLSL task shader lowering is unsupported. The most likely cause is that a task BESL shader was sent to the deferred GLSL backend."
		),
		Stages::Mesh { .. } => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
	}

	glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
	glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
	if uses_f16_types {
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_float16:require\n");
	}
	glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
	glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
	glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
	glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
	glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");

	match compilation_settings.stage {
		Stages::Compute { .. } if uses_subgroup_intrinsics => {
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:require\n");
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:require\n");
		}
		Stages::Mesh {
			maximum_vertices,
			maximum_primitives,
			..
		} => {
			glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
			glsl_block.push_str(&format!(
				"layout(triangles,max_vertices={},max_primitives={}) out;\n",
				maximum_vertices, maximum_primitives
			));
		}
		_ => {}
	}

	match compilation_settings.stage {
		Stages::Compute { local_size } | Stages::Mesh { local_size, .. } => {
			glsl_block.push_str(&format!(
				"layout(local_size_x={},local_size_y={},local_size_z={}) in;\n",
				local_size.width().max(1),
				local_size.height().max(1),
				local_size.depth().max(1)
			));
		}
		_ => {}
	}

	match compilation_settings.matrix_layout {
		MatrixLayouts::RowMajor => glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;\n"),
		MatrixLayouts::ColumnMajor => glsl_block.push_str("layout(column_major) uniform;layout(column_major) buffer;\n"),
	}

	glsl_block.push_str("const float PI = 3.14159265359;");
	if !generator.minified {
		glsl_block.push('\n');
	}
}
