mod generator;
mod reserved;

pub use Generator as GLSLTranspiler;
pub use generator::Generator;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

	/// Lowers `main` to minified GLSL.
	fn generate(settings: &ShaderGenerationSettings, main: &besl::NodeReference) -> String {
		Generator::new()
			.minified(true)
			.generate(settings, main)
			.expect("Expected GLSL generation")
	}

	/// Links a standalone BESL source and lowers its `main` to minified GLSL.
	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let root = besl::compile_to_besl(source, None).expect("Expected fixture source to link");
		generate(settings, &root.get_main().expect("Expected fixture main function"))
	}

	#[test]
	fn bindings() {
		let main = generator::tests::bindings();

		let shader = generate(&ShaderGenerationSettings::vertex(), &main);

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
		let shader = lower_fixture(super::super::RUNTIME_ARRAY_FRAGMENT, &ShaderGenerationSettings::fragment());

		assert_string_contains!(
			shader,
			"layout(set=0,binding=1,scalar) readonly buffer _instances{Instance instances[];};"
		);
		assert_string_contains!(shader, "layout(set=0,binding=0) uniform sampler2DArray sprites;");
		assert_string_contains!(shader, "Instance instance=instances[");
		assert_string_contains!(shader, "texture(sprites,vec3(");
		assert_string_contains!(shader, "float(instance.sprite_id)");
		compile(&shader, "besl-runtime-array-texture-layer");
	}

	#[test]
	fn scalar_runtime_arrays_use_scalar_layout_glsl_blocks() {
		let shader = lower_fixture(
			super::super::SCALAR_RUNTIME_ARRAY_COMPUTE,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

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
		compile(&shader, "besl-scalar-runtime-array");
	}

	#[test]
	fn gather_reads_the_texel_quad_with_texture_gather_in_glsl() {
		let source = r#"
			depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
			array_depth_texture: descriptor<{ type: Texture2DArray, binding: 1, access: read }>;
			main: fn () -> void {
				let quad: vec4f = gather(depth_texture, vec2f(0.5, 0.5));
				let layer_quad: vec4f = gather(array_depth_texture, vec2f(0.5, 0.5), 1);
				quad;
				layer_quad;
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(shader, "textureGather(depth_texture,");
		assert_string_contains!(shader, "textureGather(array_depth_texture,vec3(");

		compile(&shader, "besl-gather");
	}

	/// Verifies a texture parameter needs no companion in GLSL, whose sampler types carry their own.
	#[test]
	fn texture_parameters_pass_as_combined_samplers_in_glsl() {
		let source = r#"
			array_depth_texture: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
			quad_maximum: fn (depth_map: ArrayTexture2D, uv: vec2f, layer: u32) -> f32 {
				let quad: vec4f = gather(depth_map, uv, layer);
				return max(max(quad.x, quad.y), max(quad.z, quad.w));
			}
			first_layer_maximum: fn (depth_map: ArrayTexture2D, uv: vec2f) -> f32 {
				return quad_maximum(depth_map, uv, 0);
			}
			main: fn () -> void {
				first_layer_maximum(array_depth_texture, vec2f(0.5, 0.5));
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(
			shader,
			"float quad_maximum(in sampler2DArray depth_map,vec2 uv,uint32_t layer)"
		);
		assert_string_contains!(shader, "first_layer_maximum(array_depth_texture,vec2(");

		compile(&shader, "besl-texture-parameter");
	}

	/// Verifies a 16-bit unorm storage image takes the `r16` layout qualifier.
	#[test]
	fn unorm_storage_image_takes_the_r16_layout_in_glsl() {
		let source = r#"
			cells: descriptor<{ type: StorageImage<r16>, binding: 0, access: write }>;
			main: fn () -> void {
				write(cells, vec2u(0, 0), vec4f(0.5, 0.0, 0.0, 1.0));
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(shader, "layout(set=0,binding=0,r16) writeonly uniform image2D cells");

		compile(&shader, "besl-unorm-storage-image");
	}

	#[test]
	fn descriptor_array_elements_reach_every_texture_intrinsic_in_glsl() {
		let shader = lower_fixture(super::super::DESCRIPTOR_ARRAY_FRAGMENT, &ShaderGenerationSettings::fragment());

		assert_string_contains!(shader, "uvec2(textureSize(textures[nonuniformEXT(items[index].slot)],0))");
		assert_string_contains!(shader, "textureLod(textures[nonuniformEXT(index+1)],uv,0.0)");
		assert_string_contains!(shader, "texture(textures[nonuniformEXT(items[index].slot)],uv)");

		compile(&shader, "besl-descriptor-array-intrinsics");
	}

	#[test]
	fn structural_position_uses_gl_position_without_colliding_with_a_local() {
		let shader = lower_fixture(super::super::STRUCTURAL_POSITION_VERTEX, &ShaderGenerationSettings::vertex());

		assert_string_contains!(shader, "vec4 position=vec4(float(uint(gl_VertexIndex)),0.0,0.0,1.0);");
		assert_string_contains!(shader, "gl_Position=position;");
		assert!(!shader.contains("out vec4 _besl_interface_position"));
	}

	#[test]
	fn names_reserved_by_glsl_are_prefixed_at_declarations_and_uses() {
		let shader = lower_fixture(
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
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		assert_string_contains!(shader, "buffer _buffer{float besl_half;uint32_t besl_output;}besl_buffer;");
		assert_string_contains!(shader, "float besl_texture(float besl_input,float besl_besl_float)");
		assert_string_contains!(shader, "float besl_min=min(besl_input,besl_besl_float);");
		assert_string_contains!(shader, "struct besl_sampler{float besl_half;uint32_t besl_output;};");
		assert_string_contains!(shader, "struct Wrapper{besl_sampler value;};");
		assert_string_contains!(shader, "float besl_float=besl_texture(wrapper.value.besl_half,2.0);");
		assert_string_contains!(shader, "void main(");

		compile(&shader, "besl-reserved-names");
	}

	#[test]
	fn compute_subgroup_intrinsics_require_and_lower_to_glsl_subgroup_operations() {
		let shader = lower_fixture(
			super::super::SUBGROUP_COMPUTE,
			&ShaderGenerationSettings::compute(utils::Extent::line(32)),
		);
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_basic:require");
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_ballot:require");
		assert_string_contains!(shader, "#extension GL_KHR_shader_subgroup_shuffle:require");
		assert_string_contains!(shader, "subgroupShuffleXor(float(value),1)");
		assert_string_contains!(shader, "subgroupBallot(uint(gl_LocalInvocationIndex)<4)");
		assert_string_contains!(shader, "subgroupBroadcast(uint(gl_LocalInvocationIndex),leader)");
		assert_string_contains!(shader, "subgroupBallotFindLSB(mask)");
		assert_string_contains!(shader, "subgroupBallotBitCount(remaining)");

		compile(&shader, "besl-subgroup-compute");
	}

	#[test]
	fn source_unformatted_storage_image_descriptor_omits_glsl_format() {
		let shader = lower_fixture(
			"image: descriptor<{ type: StorageImage, binding: 5, access: write }>; main: fn () -> void { image; }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "layout(set=0,binding=5) writeonly uniform image2D image;");
		assert!(
			!shader.contains("binding=5,"),
			"Unformatted storage image emitted a dangling GLSL format comma: {shader}"
		);
	}

	#[test]
	fn vec4f_meshlet_record_uses_scalar_buffer_layout() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&generator::tests::vec4f_meshlet_binding(),
		);
		assert_string_contains!(shader, "vec4 center_radius;vec4 cone_apex_cutoff;");
		assert_string_contains!(shader, "layout(set=0,binding=0,scalar)");
	}

	#[test]
	fn same_named_buffer_members_lower_to_glsl() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&generator::tests::same_named_buffer_member_access(),
		);
		assert_string_contains!(shader, "pixel_mapping[0]=meshes[1];");
	}

	#[test]
	fn specializtions() {
		let shader = generate(&ShaderGenerationSettings::vertex(), &generator::tests::specializations());
		assert_string_contains!(
			shader,
			"layout(constant_id=0)const float color_x=1.0f;layout(constant_id=1)const float color_y=1.0f;layout(constant_id=2)const float color_z=1.0f;const vec3 color=vec3(color_x,color_y,color_z);void main(){color;}"
		);
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_flat_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = generate(&ShaderGenerationSettings::vertex(), &main);
		let fragment_shader = generate(&ShaderGenerationSettings::fragment(), &main);
		assert_string_contains!(vertex_shader, "layout(location=0)in u16vec2 packed_input;");
		assert_string_contains!(vertex_shader, "layout(location=1)flat out u16vec4 packed_output;");
		assert_string_contains!(fragment_shader, "layout(location=0)flat in u16vec2 packed_input;");
		assert_string_contains!(fragment_shader, "layout(location=1)out u16vec4 packed_output;");
	}

	#[test]
	fn cull_unused_functions() {
		let program = generator::tests::cull_unused_functions();
		let main = program.get_main().expect("Expected main");

		let shader = generate(&ShaderGenerationSettings::vertex(), &main);
		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn vertex_invocation_indices_lower_to_vulkan_builtins_inside_helpers() {
		let shader = lower_fixture(super::super::VERTEX_BUILTIN_HELPER, &ShaderGenerationSettings::vertex());

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
		let shader = lower_fixture(
			r#"
			expensive: fn() -> f32 {
				return 42.0;
			}
			main: fn() -> void {
				let x: f32 = expensive();
				return;
			}
		"#,
			&ShaderGenerationSettings::vertex(),
		);
		assert_string_contains!(shader, "void main(){return;}");
		assert!(
			!shader.contains("expensive"),
			"Dead helper function reached GLSL emission: {shader}"
		);
		assert!(!shader.contains("float x"), "Dead local reached GLSL emission: {shader}");
	}

	#[test]
	fn push_constant() {
		let shader = generate(&ShaderGenerationSettings::vertex(), &generator::tests::push_constant());
		assert_string_contains!(
			shader,
			"layout(push_constant,scalar)uniform PushConstant{uint32_t material_id;}push_constant;void main(){push_constant;}"
		);
	}

	#[test]
	fn test_multi_language_raw_code() {
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&generator::tests::multi_language_raw_code(),
		);

		// The GLSL transpiler should use the GLSL code.
		assert_string_contains!(shader, "struct Vertex{vec3 position;vec3 normal;};");
		assert_string_contains!(shader, "void main(){gl_Position = vec4(0);}");
		// Should NOT contain HLSL code
		assert!(!shader.contains("float4"), "GLSL shader should not contain HLSL code");
	}

	#[test]
	fn test_const_variable() {
		let shader = generate(&ShaderGenerationSettings::vertex(), &generator::tests::const_variable());
		assert_string_contains!(shader, "const float besl_PI = 3.14;");
		assert_string_contains!(shader, "void main(){besl_PI;}");
	}

	#[test]
	fn short_scalar_arrays_lower_to_glsl_vectors() {
		let shader = lower_fixture(super::super::SHORT_SCALAR_ARRAYS, &ShaderGenerationSettings::vertex());
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
		let shader = generate(
			&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)),
			&generator::tests::vertex_and_primitive_mesh_outputs(),
		);
		assert_string_contains!(shader, "layout(location=2)out vec2 out_uv[3];");
		assert_string_contains!(
			shader,
			"layout(location=1)perprimitiveEXT out uint32_t out_primitive_index[1];"
		);
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

		let shader = lower_fixture(script, &ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)));
		assert_string_contains!(shader, "SetMeshOutputsEXT(4,2);");
		assert_string_contains!(shader, "gl_MeshVerticesEXT[0].gl_Position = vec4(1.0,2.0,3.0,1.0);");
		assert_string_contains!(shader, "gl_PrimitiveTriangleIndicesEXT[0] = uvec3(0,1,2);");
		assert_string_contains!(shader, "gl_MeshPrimitivesEXT[0].gl_Layer = int(3);");
	}

	#[test]
	fn else_chains_lower_to_glsl() {
		let shader = lower_fixture(
			super::super::ELSE_CHAIN,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "if(n<1){n=2;}else if(n<4){n=3;}else{n=4;}");

		compile(&shader, "besl-else-chain");
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

		let shader = lower_fixture(script, &ShaderGenerationSettings::compute(utils::Extent::line(1)));
		// The inner `break` sets both flags, so it leaves both switches and then the loop.
		assert_string_contains!(
			shader,
			"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{{bool besl_match_break_1=false;switch(uint(flag)){case 1u:{besl_match_break_1=true;break;break;}default:{continue;break;}}if(besl_match_break_1){besl_match_break_0=true;break;}};break;}default:{break;}}if(besl_match_break_0){break;}}"
		);
		assert_string_contains!(
			shader,
			"switch(signed){case (-2147483647-1):{n=2;break;}default:{n=3;break;}}"
		);

		compile(&shader, "besl-match");
	}

	#[test]
	fn f16_storage_types_enable_native_glsl_arithmetic() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&generator::tests::mixed_f16_storage_binding(),
		);
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
	fn prefix_operators_and_ternaries_lower_to_glsl() {
		let shader = lower_fixture(
			r#"
			Body: struct { velocity: vec3f, color: vec4f, bits: u32, weight: f16 }
			Frame: struct { projection: mat4f }
			bodies: descriptor<{ type: Body[], binding: 0, access: read_write }>;
			frame: descriptor<{ type: Frame, binding: 1, access: read }>;
			main: fn (input: StageInput) -> void {
				let item: u32 = input.thread_id.x;
				let flag: bool = bodies[item].bits > 3;
				bodies[item].bits = ~bodies[item].bits ^ (flag ? 1 : 2);
				bodies[item].velocity = - -bodies[item].velocity - -bodies[item].velocity;
				bodies[item].weight = !flag ? bodies[item].weight : 1.0;
				bodies[item].color = frame.projection * -bodies[item].color + frame.projection * (flag ? bodies[item].color : bodies[item].color);
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "((~bodies[item].bits)^(flag?1:2))");
		// A nested negation keeps its parentheses, so `- -x` never prints as the decrement `--x`.
		assert_string_contains!(shader, "((-(-bodies[item].velocity))-(-bodies[item].velocity))");
		// GLSL does not narrow float literals to float16_t, so a literal branch beside an f16 branch is cast.
		assert_string_contains!(shader, "((!flag)?bodies[item].weight:float16_t(1.0))");
		compile(&shader, "besl-prefix-operators-and-ternaries");
	}

	/// Compiles generated GLSL to SPIR-V on Linux so a lowering that glslang rejects fails the test.
	fn compile(shader: &str, name: &str) {
		#[cfg(target_os = "linux")]
		crate::shader::besl::backends::spirv::compile_glsl_to_spirv(shader, name)
			.unwrap_or_else(|error| panic!("Expected {name} GLSL to compile to SPIR-V. {error}"));
		#[cfg(not(target_os = "linux"))]
		let _ = (shader, name);
	}

	/// Verifies `pow(2, x)` is rewritten to `exp2(x)` for full and half precision.
	#[test]
	fn power_of_two_uses_exp2() {
		let shader = lower_fixture(
			"main: fn () -> void { let full: f32 = pow(2.0, 3.0); let half: f16 = pow(f16(2.0), f16(3.0)); full; half; }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

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
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::line(1)));

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

		compile(&shader, "besl-modern-half-atomics");
	}

	/// Verifies `find_lsb` converts GLSL's signed `findLSB` result, so zero yields `0xffffffff` like the BESL contract.
	#[test]
	fn find_lsb_lowers_to_find_lsb_converted_to_unsigned() {
		let shader = lower_fixture(
			super::super::FIND_LSB,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "uint(findLSB(bits))");

		compile(&shader, "besl-find-lsb");
	}

	/// Verifies a storage image declares its texel format, which GLSL requires for images that are read or written.
	#[test]
	fn source_storage_image_descriptor_emits_explicit_glsl_format() {
		let shader = lower_fixture(
			"image: descriptor<{ type: StorageImage<rgba16f>, binding: 4, access: write }>; main: fn () -> void { image; }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "layout(set=0,binding=4,rgba16f) writeonly uniform image2D image;");
	}

	/// Verifies `fetch` reads an exact texel through `texelFetch` with signed coordinates and an explicit mip level.
	#[test]
	fn fetch_intrinsic_lowers_to_glsl() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&generator::tests::texel_fetch(),
		);
		assert_string_contains!(shader, "vec4 texel=texelFetch(besl_texture,ivec2(coord),0);");

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

		let shader = lower_fixture(script, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(
			shader,
			"atomicCompSwap(shared_keys[uint(gl_LocalInvocationIndex)],4294967295,7)"
		);

		compile(&shader, "besl-atomic-compare-exchange");
	}

	/// Verifies a global scalar-array constant keeps its vector spelling as a GLSL `const`.
	#[test]
	fn const_array_variable_lowers_to_glsl() {
		let shader = lower_fixture(
			super::super::CONST_ARRAY,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "const vec3 WEIGHTS = vec3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");

		compile(&shader, "besl-const-array");
	}
}
