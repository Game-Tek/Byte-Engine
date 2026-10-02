mod analysis;
mod generator;
mod header;
mod reserved;

pub use Generator as GLSLTranspiler;
pub use analysis::Generator;

#[cfg(test)]
mod tests {
	use std::cell::RefCell;

	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

	macro_rules! assert_string_contains {
		($haystack:expr, $needle:expr) => {
			assert!(
				$haystack.contains($needle),
				"Expected string to contain '{}', but it did not. String: '{}'",
				$needle,
				$haystack
			);
		};
	}

	#[test]
	fn bindings() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// We have to split the assertions because the order of the bindings is not guaranteed.
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar) buffer _buff{float member;}buff;");
		assert_string_contains!(shader, "layout(set=0,binding=1,r8) writeonly uniform image2D image;");
		assert_string_contains!(shader, "layout(set=0,binding=2) uniform sampler2D besl_texture;");
		assert_string_contains!(shader, "void main(){buff;image;besl_texture;}");
		assert!(!shader.contains("GL_EXT_shader_explicit_arithmetic_types_float16"));

		// Assert that main is the last element in the shader string, which means that the bindings are before it.
		shader.ends_with("void main(){buff;image;besl_texture;}");
	}

	#[test]
	fn runtime_buffer_and_texture_array_layer_use_native_glsl_resources() {
		let root = besl::compile_to_besl(super::super::RUNTIME_ARRAY_FRAGMENT, None)
			.expect("Expected runtime-array fragment source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected runtime-array fragment GLSL generation");

		assert_string_contains!(
			shader,
			"layout(set=0,binding=1,scalar) readonly buffer _instances{Instance instances[];};"
		);
		assert_string_contains!(shader, "layout(set=0,binding=0) uniform sampler2DArray sprites;");
		assert_string_contains!(shader, "Instance instance=instances[");
		assert_string_contains!(shader, "texture(sprites,vec3(");
		assert_string_contains!(shader, "float(instance.sprite_id)");
		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-runtime-array-texture-layer")
			.expect("Expected runtime-array fragment GLSL to compile to SPIR-V");
	}

	#[test]
	fn scalar_runtime_arrays_use_scalar_layout_glsl_blocks() {
		let root = besl::compile_to_besl(super::super::SCALAR_RUNTIME_ARRAY_COMPUTE, None)
			.expect("Expected scalar runtime-array compute source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected scalar runtime-array GLSL generation");

		assert_string_contains!(
			shader,
			"layout(set=0,binding=0,scalar) readonly buffer _positions{vec3 positions[];};"
		);
		assert_string_contains!(
			shader,
			"layout(set=0,binding=1,scalar) readonly buffer _indices{uint16_t indices[];};"
		);
		assert_string_contains!(
			shader,
			"layout(set=0,binding=2,scalar) readonly buffer _corners{uint8_t corners[];};"
		);
		assert_string_contains!(
			shader,
			"layout(set=0,binding=3,scalar) writeonly buffer _results{uint32_t results[];};"
		);
		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-scalar-runtime-array")
			.expect("Expected scalar runtime-array GLSL to compile to SPIR-V");
	}

	#[test]
	fn descriptor_array_elements_reach_every_texture_intrinsic_in_glsl() {
		let root = besl::compile_to_besl(super::super::DESCRIPTOR_ARRAY_FRAGMENT, None)
			.expect("Expected descriptor-array fragment source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected descriptor-array fragment GLSL generation");

		assert_string_contains!(shader, "uvec2(textureSize(textures[nonuniformEXT(items[index].slot)],0))");
		assert_string_contains!(shader, "textureLod(textures[nonuniformEXT(index+1)],uv,0.0)");
		assert_string_contains!(shader, "texture(textures[nonuniformEXT(items[index].slot)],uv)");

		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-descriptor-array-intrinsics")
			.expect("Expected descriptor-array fragment GLSL to compile to SPIR-V");
	}

	#[test]
	fn structural_position_uses_gl_position_without_colliding_with_a_local() {
		let root = besl::compile_to_besl(super::super::STRUCTURAL_POSITION_VERTEX, None)
			.expect("Expected structural position source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &root.get_main().expect("Expected main"))
			.expect("Expected structural position GLSL generation");

		assert_string_contains!(shader, "vec4 position=vec4(float(uint(gl_VertexIndex)),0.0,0.0,1.0);");
		assert_string_contains!(shader, "gl_Position=position;");
		assert!(!shader.contains("out vec4 _besl_interface_position"));
	}

	#[test]
	fn names_reserved_by_glsl_are_prefixed_at_declarations_and_uses() {
		let root = besl::compile_to_besl(
			r#"
			sampler: struct { half: f32, output: u32 }
			Wrapper: struct { value: sampler }
			buffer: descriptor<{ type: sampler, binding: 0, access: read_write }>;
			texture: fn (input: f32, besl_float: f32) -> f32 {
				let min: f32 = min(input, besl_float);
				return min;
			}
			main: fn () -> void {
				let wrapper: Wrapper = Wrapper(sampler(buffer.half, buffer.output));
				let float: f32 = texture(wrapper.value.half, 2.0);
				buffer.half = float;
				buffer.output = 1;
			}
			"#,
			None,
		)
		.expect("Expected reserved-name fixture source to link");
		let main = root.get_main().expect("Expected reserved-name fixture main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected reserved-name fixture to lower to GLSL");

		assert_string_contains!(shader, "buffer _buffer{float besl_half;uint32_t besl_output;}besl_buffer;");
		assert_string_contains!(shader, "float besl_texture(float besl_input,float besl_besl_float)");
		assert_string_contains!(shader, "float besl_min=min(besl_input,besl_besl_float);");
		assert_string_contains!(shader, "struct besl_sampler{float besl_half;uint32_t besl_output;};");
		assert_string_contains!(shader, "struct Wrapper{besl_sampler value;};");
		assert_string_contains!(shader, "float besl_float=besl_texture(wrapper.value.besl_half,2.0);");
		assert_string_contains!(shader, "void main(");

		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-reserved-names")
			.expect("Expected GLSL with prefixed reserved names to compile to SPIR-V");
	}

	#[test]
	fn compute_subgroup_intrinsics_require_and_lower_to_glsl_subgroup_operations() {
		let root = besl::compile_to_besl(
			r#"
			main: fn () -> void {
				let mask: vec4u = subgroup_ballot(thread_idx() < 4);
				let leader: u32 = subgroup_ballot_find_lsb(mask);
				let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
				let remaining: vec4u = subgroup_ballot_and_not(mask, subgroup_ballot(value == 0));
				if (subgroup_ballot_any(remaining)) {
					let count: u32 = subgroup_ballot_count(remaining);
					count;
				}
			}
			"#,
			None,
		)
		.expect("Expected subgroup fixture source to link");
		let main = root.get_main().expect("Expected subgroup fixture main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(32)), &main)
			.expect("Expected subgroup fixture to lower to GLSL");
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_basic:require");
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_ballot:require");
		assert_string_contains!(shader, "subgroupBallot(uint(gl_LocalInvocationIndex)<4)");
		assert_string_contains!(shader, "subgroupBroadcast(uint(gl_LocalInvocationIndex),leader)");
		assert_string_contains!(shader, "subgroupBallotFindLSB(mask)");
		assert_string_contains!(shader, "subgroupBallotBitCount(remaining)");
	}

	#[test]
	fn source_unformatted_storage_image_descriptor_omits_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<{ type: StorageImage, binding: 5, access: write }>; main: fn () -> void { image; }",
			None,
		)
		.expect("Expected unformatted storage image descriptor to compile");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected unformatted storage image shader main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected unformatted storage image GLSL generation");
		assert_string_contains!(shader, "layout(set=0,binding=5) writeonly uniform image2D image;");
		assert!(
			!shader.contains("binding=5,"),
			"Unformatted storage image emitted a dangling GLSL format comma: {shader}"
		);
	}

	#[test]
	fn packed_vec4f_uses_native_vectors_with_scalar_buffer_layout() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::packed_vec4f_meshlet_binding(),
			)
			.expect("Expected packed_vec4f GLSL generation");
		assert_string_contains!(shader, "vec4 center_radius;vec4 cone_apex_cutoff;");
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar)");
		assert!(!shader.contains("struct packed_vec4f"));
	}

	#[test]
	fn same_named_buffer_members_lower_to_glsl() {
		let main = generator::tests::same_named_buffer_member_access();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "pixel_mapping[0]=meshes[1];");
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"layout(constant_id=0)const float color_x=1.0f;layout(constant_id=1)const float color_y=1.0f;layout(constant_id=2)const float color_z=1.0f;const vec3 color=vec3(color_x,color_y,color_z);void main(){color;}"
		);
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_flat_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected packed integer vertex GLSL generation");
		let fragment_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Expected packed integer fragment GLSL generation");
		assert_string_contains!(vertex_shader, "layout(location=0)in u16vec2 packed_input;");
		assert_string_contains!(vertex_shader, "layout(location=1)flat out u16vec4 packed_output;");
		assert_string_contains!(fragment_shader, "layout(location=0)flat in u16vec2 packed_input;");
		assert_string_contains!(fragment_shader, "layout(location=1)out u16vec4 packed_output;");
	}

	#[test]
	fn cull_unused_functions() {
		let program = generator::tests::cull_unused_functions();
		let main = program.get_main().expect("Expected main");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn vertex_invocation_indices_lower_to_vulkan_builtins_inside_helpers() {
		let root = besl::compile_to_besl(
			r#"
			invocation_sum: fn () -> u32 {
				return vertex_index + instance_index;
			}
			out_value: output<u32, 0>;
			main: fn () -> void {
				out_value = invocation_sum();
			}
			"#,
			None,
		)
		.expect("Expected implicit vertex builtins to link");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &root.get_main().expect("Expected main"))
			.expect("Expected vertex builtins to lower to GLSL");

		assert_string_contains!(shader, "uint(gl_VertexIndex)");
		assert_string_contains!(shader, "uint(gl_InstanceIndex)");
		assert!(!shader.contains("layout(location=254)"));
		assert!(!shader.contains("layout(location=255)"));
	}

	#[test]
	fn vertex_invocation_indices_are_rejected_outside_the_vertex_stage() {
		let root = besl::compile_to_besl("main: fn () -> void { vertex_index; }", None)
			.expect("Expected implicit vertex builtin to link");
		assert!(
			Generator::new()
				.generate(
					&ShaderGenerationSettings::fragment(),
					&root.get_main().expect("Expected main")
				)
				.is_err()
		);
	}

	#[test]
	fn culls_dead_locals_before_glsl_emission() {
		let root = besl::compile_to_besl(
			r#"
			expensive: fn() -> f32 {
				return 42.0;
			}
			main: fn() -> void {
				let x: f32 = expensive();
				return;
			}
		"#,
			None,
		)
		.expect("Expected dead-local BESL fixture to link");
		let main = root.get_main().expect("Expected dead-local fixture main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected dead-local GLSL generation");
		assert_string_contains!(shader, "void main(){return;}");
		assert!(
			!shader.contains("expensive"),
			"Dead helper function reached GLSL emission: {shader}"
		);
		assert!(!shader.contains("float x"), "Dead local reached GLSL emission: {shader}");
	}

	#[test]
	fn push_constant() {
		let main = generator::tests::push_constant();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"layout(push_constant)uniform PushConstant{uint32_t material_id;}push_constant;void main(){push_constant;}"
		);
	}

	#[test]
	fn test_multi_language_raw_code() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();

		{
			let mut main = main.borrow_mut();
			// Create a RawCode node with both GLSL and HLSL variants
			main.add_child(
				besl::Node::raw(
					Some("gl_Position = vec4(0)".to_string()),
					Some("output.position = float4(0, 0, 0, 1)".to_string()),
					Some("out.position = float4(0, 0, 0, 1)".to_string()),
					vec![vertex_struct],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// The GLSL transpiler should use the GLSL code.
		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
		// Should NOT contain HLSL code
		assert!(!shader.contains("float4"), "GLSL shader should not contain HLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "const float besl_PI = 3.14;");
		assert_string_contains!(shader, "void main(){besl_PI;}");
	}

	#[test]
	fn short_scalar_arrays_lower_to_glsl_vectors() {
		let script = r#"
		scalar_f32: fn () -> f32[3] {
			return f32[3](0.5, 0.25, 0.125);
		}
		scalar_u16: fn () -> u16[3] {
			return u16[3](1, 2, 3);
		}
		scalar_u32: fn () -> u32[3] {
			return u32[3](4, 5, 6);
		}
		mirror_indices: fn (indices: u32[3]) -> u32[3] {
			return indices;
		}
		main: fn () -> void {
			let floats: f32[3] = scalar_f32();
			let shorts: u16[3] = scalar_u16();
			let indices: u32[3] = mirror_indices(scalar_u32());
			let sum: f32 = floats[1] + f32(u32(shorts[1])) + f32(indices[1]);
			sum;
		}
		"#;
		let root = besl::compile_to_besl(script, None).expect("Expected scalar-array shader source to lex");
		let main = root.get_main().expect("Expected scalar-array main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected scalar arrays to lower to GLSL vectors");
		assert_string_contains!(shader, "vec3 scalar_f32()");
		assert_string_contains!(shader, "u16vec3 scalar_u16()");
		assert_string_contains!(shader, "uvec3 scalar_u32()");
		assert_string_contains!(shader, "uvec3 mirror_indices(uvec3 indices)");
		assert_string_contains!(shader, "vec3 floats=scalar_f32();");
		assert_string_contains!(shader, "u16vec3 shorts=scalar_u16();");
		assert_string_contains!(shader, "uvec3 indices=mirror_indices(scalar_u32());");
	}

	/// Verifies per-vertex mesh outputs join the interpolated vertex struct while per-primitive outputs stay flat.
	#[test]
	fn vertex_mesh_outputs_join_the_vertex_struct() {
		let root = besl::compile_to_besl(
			r#"
			out_primitive_index: output<u32, 1, 1>;
			out_uv: vertex_output<vec2f, 2, 3>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				if (lane == 0) {
					set_mesh_output_counts(3, 1);
				}
				if (lane < 3) {
					set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
					out_uv[lane] = vec2f(f32(lane), 1.0);
				}
				if (lane < 1) {
					set_mesh_triangle(0, vec3u(0, 1, 2));
					out_primitive_index[0] = lane;
				}
			}
			"#,
			None,
		)
		.expect("Expected mesh shader source to compile");
		let main = root.get_main().expect("Expected mesh shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)), &main)
			.expect("Expected mesh shader source to generate GLSL");
		assert_string_contains!(shader, "layout(location=2)out vec2 out_uv[3];");
		assert_string_contains!(shader, "layout(location=1)perprimitiveEXT out uint32_t out_primitive_index[1];");
		assert_string_contains!(shader, "out_uv[lane]=vec2(float(lane),1.0);");
	}

	#[test]
	fn mesh_intrinsics_emit_glsl_mesh_commands() {
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
			set_mesh_primitive_render_target_array_index(0, 3);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "SetMeshOutputsEXT(4,2);");
		assert_string_contains!(shader, "gl_MeshVerticesEXT[0].gl_Position = vec4(1.0,2.0,3.0,1.0);");
		assert_string_contains!(shader, "gl_PrimitiveTriangleIndicesEXT[0] = uvec3(0,1,2);");
		assert_string_contains!(shader, "gl_MeshPrimitivesEXT[0].gl_Layer = int(3);");
	}

	#[test]
	fn else_chains_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			} else if (n < 4) {
				n = 3;
			} else {
				n = 4;
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected else-chain shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "if(n<1){n=2;}else if(n<4){n=3;}else{n=4;}");

		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-else-chain")
			.expect("Expected else-chain GLSL to compile to SPIR-V");
	}

	#[test]
	fn match_lowers_to_glsl_switch() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			let flag: bool = n < 1;
			let signed: i32 = 0;
			for (let i: u32 = 0; i < 4; i = i + 1) {
				match i {
					0 => n = 1,
					1 | 2 => {
						match flag {
							true => break,
							false => continue,
						}
					}
					_ => {}
				}
			}
			match signed {
				-2147483648 => n = 2,
				_ => n = 3,
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected match shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Failed to generate shader");
		// The inner `break` sets both flags, so it leaves both switches and then the loop.
		assert_string_contains!(
			shader,
			"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{{bool besl_match_break_1=false;switch(uint(flag)){case 1u:{besl_match_break_1=true;break;break;}default:{continue;break;}}if(besl_match_break_1){besl_match_break_0=true;break;}};break;}default:{break;}}if(besl_match_break_0){break;}}"
		);
		assert_string_contains!(
			shader,
			"switch(signed){case (-2147483647-1):{n=2;break;}default:{n=3;break;}}"
		);

		#[cfg(target_os = "linux")]
		crate::shader::glsl_compile::compile(&shader, "besl-match").expect("Expected match GLSL to compile to SPIR-V");
	}

	#[test]
	fn f16_storage_types_enable_native_glsl_arithmetic() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_f16_storage_binding(),
			)
			.expect("Expected f16 GLSL generation");
		assert_string_contains!(shader, "#extension GL_EXT_shader_explicit_arithmetic_types_float16:require");
		assert_string_contains!(shader, "float16_t scalar;");
		assert_string_contains!(shader, "f16vec2 uv;");
		assert_string_contains!(shader, "f16vec3 normal;");
		assert_string_contains!(shader, "f16vec4 color;");
		assert_string_contains!(shader, "f16vec2(uv32)");
		assert_string_contains!(shader, "vec2(uv16)");
		assert_string_contains!(shader, "float16_t(0.5)");
		assert_string_contains!(shader, "float(weight16)");
		assert_string_contains!(shader, "float16_t literal=float16_t(0.25);");
		assert_string_contains!(shader, "weight16*float16_t(2.0)");
		assert_string_contains!(shader, "uv16*float16_t(2.0)");
		assert!(!shader.contains("struct vec2f16"));
	}

	/// Compiles generated GLSL to SPIR-V on Linux so a lowering that glslang rejects fails the test.
	#[cfg(target_os = "linux")]
	fn compile(shader: &str, name: &str) {
		crate::shader::glsl_compile::compile(shader, name)
			.unwrap_or_else(|error| panic!("Expected {name} GLSL to compile to SPIR-V. {error}"));
	}

	/// Verifies `pow(2, x)` is rewritten to `exp2(x)` for full and half precision.
	#[test]
	fn power_of_two_uses_exp2() {
		let root = besl::compile_to_besl(
			"main: fn () -> void { let full: f32 = pow(2.0, 3.0); let half: f16 = pow(f16(2.0), f16(3.0)); full; half; }",
			None,
		)
		.expect("Expected power source to link.");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main."),
			)
			.expect("Expected GLSL power lowering.");

		assert_eq!(shader.matches("exp2(").count(), 2);
		assert!(!shader.contains("pow("));
	}

	/// Verifies atomics that GLSL lacks are emulated with the portable operations it has, and half-precision math keeps half types.
	#[test]
	fn modern_half_and_integer_atomics_lower_to_portable_glsl() {
		let source = r#"
			Counters: struct { buffer_value: atomicu32, }
			counters: descriptor<{ type: Counters, binding: 7, access: read_write }>;
			unsigned_value: workgroup<atomicu32>;
			signed_value: workgroup<atomici32>;
			main: fn () -> void {
				let signed_one: i32 = 1;
				atomic_store(unsigned_value, 1);
				atomic_load(counters.buffer_value);
				atomic_load(unsigned_value);
				atomic_exchange(unsigned_value, 2);
				atomic_add(unsigned_value, 1);
				atomic_sub(unsigned_value, 1);
				atomic_min(unsigned_value, 1);
				atomic_max(unsigned_value, 2);
				atomic_and(unsigned_value, 3);
				atomic_or(unsigned_value, 4);
				atomic_xor(unsigned_value, 5);
				atomic_compare_exchange(unsigned_value, 1, 2);
				atomic_store(signed_value, signed_one);
				atomic_min(signed_value, signed_one);
				let zero: f16 = f16(0.0);
				let one: f16 = f16(1.0);
				let fused: f16 = fma(one, one, one);
				let fused_vector: vec3f16 = fma(vec3f16(one, one, one), vec3f16(one, one, one), vec3f16(one, one, one));
				if (is_nan(zero / zero) || is_infinite(one / zero) || is_finite(fused) || is_normal(fused_vector.x)) {
					atomic_store(unsigned_value, 0);
				}
			}
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected modern GLSL source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected modern GLSL source generation");

		assert_string_contains!(shader, "shared uint32_t unsigned_value;");
		// Signed atomics must stay signed so `atomic_min` compares as signed integers.
		assert_string_contains!(shader, "shared int32_t signed_value;");
		// GLSL has no atomic load, so a load is an add of zero.
		assert_string_contains!(shader, "atomicAdd(counters.buffer_value,0u)");
		for operation in [
			"atomicExchange(",
			"atomicAdd(",
			"atomicMin(",
			"atomicMax(",
			"atomicAnd(",
			"atomicOr(",
			"atomicXor(",
			"atomicCompSwap(",
		] {
			assert_string_contains!(shader, operation);
		}
		// GLSL has no atomic subtract, so a subtract is an add of the negated value.
		assert_string_contains!(shader, "atomicAdd(unsigned_value,-(");
		assert_string_contains!(shader, "float16_t fused=fma(");
		assert_string_contains!(shader, "f16vec3 fused_vector=fma(");
		for predicate in ["isnan(", "isinf(", "_besl_is_finite(", "_besl_is_normal("] {
			assert_string_contains!(shader, predicate);
		}

		#[cfg(target_os = "linux")]
		compile(&shader, "besl-modern-half-atomics");
	}

	/// Verifies `find_lsb` converts GLSL's signed `findLSB` result, so zero yields `0xffffffff` like the BESL contract.
	#[test]
	fn find_lsb_lowers_to_find_lsb_converted_to_unsigned() {
		let root = besl::compile_to_besl(
			r#"
			main: fn () -> void {
				let bits: u32 = 40;
				let lowest: u32 = find_lsb(bits);
				lowest;
			}
			"#,
			None,
		)
		.expect("Expected find_lsb fixture source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected find_lsb fixture main function"),
			)
			.expect("Expected find_lsb fixture to lower to GLSL");
		assert_string_contains!(shader, "uint(findLSB(bits))");

		#[cfg(target_os = "linux")]
		compile(&shader, "besl-find-lsb");
	}

	/// Verifies a storage image declares its texel format, which GLSL requires for images that are read or written.
	#[test]
	fn source_storage_image_descriptor_emits_explicit_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<{ type: StorageImage<rgba16f>, binding: 4, access: write }>; main: fn () -> void { image; }",
			None,
		)
		.expect("Expected formatted storage image descriptor to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root
					.get_main()
					.expect("Expected formatted storage image shader main function"),
			)
			.expect("Expected formatted storage image GLSL generation");
		assert_string_contains!(shader, "layout(set=0,binding=4,rgba16f) writeonly uniform image2D image;");
	}

	/// Verifies `fetch` reads an exact texel through `texelFetch` with signed coordinates and an explicit mip level.
	#[test]
	fn fetch_intrinsic_lowers_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
			texel;
		}
		"#;

		let mut root = besl::Node::root();
		root.add_child(
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				true,
				false,
			)
			.into(),
		);
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(8)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "vec4 texel=texelFetch(besl_texture,ivec2(coord),0);");

		#[cfg(target_os = "linux")]
		compile(&shader, "besl-fetch");
	}

	/// Verifies compare-exchange lowers to `atomicCompSwap` indexed by the local invocation index.
	#[test]
	fn atomic_compare_exchange_lowers_to_glsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected compare-exchange shader source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(8)),
				&root.get_main().expect("Expected compare-exchange main function"),
			)
			.expect("Expected compare-exchange source to lower to GLSL");
		assert_string_contains!(
			shader,
			"atomicCompSwap(shared_keys[uint(gl_LocalInvocationIndex)],4294967295,7)"
		);

		#[cfg(target_os = "linux")]
		compile(&shader, "besl-atomic-compare-exchange");
	}

	/// Verifies a global scalar-array constant keeps its vector spelling as a GLSL `const`.
	#[test]
	fn const_array_variable_lowers_to_glsl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
			value;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "const vec3 WEIGHTS = vec3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");

		#[cfg(target_os = "linux")]
		compile(&shader, "besl-const-array");
	}
}
