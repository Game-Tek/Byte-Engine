mod analysis;
mod generator;
mod header;

pub use analysis::Generator;
pub use Generator as GLSLShaderGenerator;

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
		assert_string_contains!(shader, "layout(set=0,binding=2) uniform sampler2D texture;");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
		assert!(!shader.contains("GL_EXT_shader_explicit_arithmetic_types_float16"));

		// Assert that main is the last element in the shader string, which means that the bindings are before it.
		shader.ends_with("void main(){buff;image;texture;}");
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
	fn source_storage_image_descriptor_emits_explicit_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<StorageImage<rgba16f>, 4, write>; main: fn () -> void { image; }",
			None,
		)
		.expect("Expected formatted storage image descriptor to compile");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected formatted storage image shader main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected formatted storage image GLSL generation");
		assert_string_contains!(shader, "layout(set=0,binding=4,rgba16f) writeonly uniform image2D image;");
	}

	#[test]
	fn source_unformatted_storage_image_descriptor_omits_glsl_format() {
		let root = besl::compile_to_besl(
			"image: descriptor<StorageImage, 5, write>; main: fn () -> void { image; }",
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
	fn vec4u16_uses_the_native_glsl_packed_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 GLSL generation");
		assert_string_contains!(shader, "u16vec4 value;");
		assert!(!shader.contains("struct vec4u16"));
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
	fn vec2f16_arrays_use_native_glsl_vector_storage() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2f16_array_binding(),
			)
			.expect("Expected vec2f16 GLSL generation");
		assert_string_contains!(shader, "f16vec2 values[2];");
		assert_string_contains!(shader, "#extension GL_EXT_shader_explicit_arithmetic_types_float16:require");
	}

	#[test]
	fn same_named_buffer_members_lower_to_glsl() {
		let main = generator::tests::same_named_buffer_member_access();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "pixel_mapping.pixel_mapping[0]=meshes.meshes[1];");
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
	fn input() {
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "layout(location=0)in vec3 color;void main(){color;}");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "layout(location=0)out vec3 color;void main(){color;}");
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
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);albedo;}");
	}

	#[test]
	fn fwidth_intrinsic_lowers_to_glsl() {
		let program = besl::compile_to_besl("main: fn() -> void { let edge_width: f32 = fwidth(1.0); edge_width; }", None)
			.expect("Failed to compile fwidth BESL shader");
		let main = program.get_main().expect("Expected fwidth BESL shader main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate fwidth GLSL shader");
		assert_string_contains!(shader, "fwidth(1.0)");
	}

	#[test]
	fn cull_unused_functions() {
		let main = generator::tests::cull_unused_functions();

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
	fn structure() {
		let main = generator::tests::structure();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"struct Vertex{vec3 position;vec3 normal;};Vertex use_vertex(){}void main(){use_vertex();}"
		);
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
	fn test_glsl() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		used: fn() -> void {}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();
		let used_function = RefCell::borrow(&root).get_child("used").unwrap();

		{
			let mut main = main.borrow_mut();
			main.add_child(
				besl::Node::glsl(
					"gl_Position = vec4(0)".to_string(),
					vec![vertex_struct, used_function],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = generator::tests::intrinsic();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void main(){0 + 1.0 * 2;}");
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

		// GLSL generator should use the GLSL code
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
		assert_string_contains!(shader, "const float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn const_array_variable_lowers_to_glsl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
			value;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "const vec3 WEIGHTS = vec3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
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
			let sum: f32 = floats[1] + f32(shorts[1]) + f32(indices[1]);
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

	#[test]
	fn atomic_compare_exchange_lowers_to_glsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected compare-exchange shader source to lex");
		let main = root.get_main().expect("Expected compare-exchange main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Expected compare-exchange source to lower to GLSL");
		assert_string_contains!(
			shader,
			"atomicCompSwap(shared_keys[uint(gl_LocalInvocationIndex)],4294967295,7)"
		);
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
	fn conditional_blocks_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected conditional shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "if(n<1){n=2;}");
	}

	#[test]
	fn bitwise_operators_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
			packed;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "uint32_t packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "for(uint32_t i=0;i<=4;i=(i+1)){if(i>=2){continue;};};");
	}

	#[test]
	fn scalar_math_intrinsics_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let a: f32 = abs(0.0 - 2.5);
			let b: f32 = sqrt(9.0);
			let c: f32 = exp(1.0);
			let d: f32 = fract(1.25);
			let e: f32 = radians(180.0);
			let f: f32 = inversesqrt(4.0);
			let g: f32 = smoothstep(0.0, 1.0, 0.5);
			let h: f32 = mix(2.0, 4.0, 0.25);
			let i: vec2f = round(vec2f(1.2, 1.8));
			a;
			b;
			c;
			d;
			e;
			f;
			g;
			h;
			i;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "abs(0.0-2.5)");
		assert_string_contains!(shader, "sqrt(9.0)");
		assert_string_contains!(shader, "exp(1.0)");
		assert_string_contains!(shader, "fract(1.25)");
		assert_string_contains!(shader, "radians(180.0)");
		assert_string_contains!(shader, "inversesqrt(4.0)");
		assert_string_contains!(shader, "smoothstep(0.0,1.0,0.5)");
		assert_string_contains!(shader, "mix(2.0,4.0,0.25)");
		assert_string_contains!(shader, "round(vec2(1.2,1.8))");
	}

	#[test]
	fn scalar_max_and_clamp_lower_to_glsl() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
			maximum;
			clamped;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "max(1.0,2.0)");
		assert_string_contains!(shader, "clamp(1.5,0.0,1.0)");
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

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "vec4 texel=texelFetch(texture,ivec2(coord),0);");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_glsl() {
		let main = generator::tests::return_value();

		let minified_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(minified_shader, "float main(){return 1.0;}");

		let pretty_shader = Generator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(pretty_shader, "float main() {\n\treturn 1.0;\n}\n");
	}
}
