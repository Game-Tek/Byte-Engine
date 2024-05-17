use ghi::AccessPolicies;

pub struct GLSLSettings {
	version: String,
}

pub struct ShaderGenerationSettings {
	glsl: GLSLSettings,
	stage: String,
}

impl ShaderGenerationSettings {
	pub fn new(stage: &str) -> ShaderGenerationSettings {
		ShaderGenerationSettings {
			glsl: GLSLSettings {
				version: "450".to_string(),
			},
			stage: stage.to_string(),
		}
	}
}
pub fn generate_uniform_block(set: u32, binding: u32, access: AccessPolicies, r#type: &str, name: &str) -> String {
	let access = if access == AccessPolicies::READ | AccessPolicies::WRITE {
		""
	} else if access == AccessPolicies::READ {
		"readonly"
	} else if access == AccessPolicies::WRITE {
		"writeonly"
	} else {
		""
	};

	format!("layout(set={set}, binding={binding}) {access} {type} {name};", set = set, binding = binding, access = access, type = r#type, name = name)
}

pub fn generate_glsl_header_block(compilation_settings: &ShaderGenerationSettings) -> String {
	let mut glsl_block = String::with_capacity(512);
	let glsl_version = &compilation_settings.glsl.version;

	glsl_block.push_str(&format!("#version {glsl_version} core\n"));

	// shader type

	let shader_stage = compilation_settings.stage.as_str();

	match shader_stage {
		"Vertex" => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
		"Fragment" => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
		"Compute" => glsl_block.push_str("#pragma shader_stage(compute)\n"),
		"Mesh" => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
		_ => glsl_block.push_str("#define BE_UNKNOWN_SHADER_TYPE\n")
	}

	// extensions

	glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
	glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
	glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
	glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
	glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
	glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
	glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");

	match shader_stage {
		"Compute" => {
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:enable\n");
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_arithmetic:enable\n");
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:enable\n");
			glsl_block.push_str("#extension GL_KHR_shader_subgroup_shuffle:enable\n");
		}
		"Mesh" => {
			glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
			// TODO: make this next lines configurable
			glsl_block.push_str("layout(location=0) perprimitiveEXT out uint out_instance_index[126]\n");
			glsl_block.push_str("layout(location=1) perprimitiveEXT out uint out_primitive_index[126]\n");
			glsl_block.push_str("layout(triangles,max_vertices=64,max_primitives=126) out\n");
			glsl_block.push_str("layout(local_size_x=128) in\n");
		}
		_ => {}
	}
	// memory layout declarations

	glsl_block.push_str("layout(row_major) uniform; layout(row_major) buffer;\n");

	glsl_block.push_str("const float PI = 3.14159265359;");

	glsl_block
}

use std::rc::Rc;

use besl::lexer;