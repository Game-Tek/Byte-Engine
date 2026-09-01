use std::{cell::RefCell, fmt::Write as _};

use crate::shader::generator::{
	MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
	emit_comma_separated_nodes, ordered_shader_nodes,
};

mod analysis;
mod emit;
mod facade;
mod generate;
mod node_emitter;

pub(crate) use analysis::*;
pub(crate) use emit::*;
pub use facade::Generator;
pub(crate) use facade::{HlslBufferBindingSource, HlslStage};
pub(crate) use generate::*;
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

	macro_rules! assert_string_does_not_contain {
		($haystack:expr, $needle:expr) => {
			assert!(
				!$haystack.contains($needle),
				"Expected string not to contain '{}', but it did. String: '{}'",
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
			.expect("Expected HLSL power lowering.");

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

		// The test sets read=true, write=true for buff, which makes it a RWStructuredBuffer
		// Check for structured buffer (writable buffer)
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "RWStructuredBuffer<_buff> buff : register(u0, space0);");

		// Check for RWTexture2D (image)
		assert_string_contains!(shader, "RWTexture2D<float4> image : register(u1, space0);");

		// Check for Texture2D and SamplerState (combined image sampler)
		assert_string_contains!(shader, "Texture2D<float4> texture : register(t2, space0);");
		assert_string_contains!(shader, "SamplerState texture_sampler : register(s2, space0);");

		// Check main function
		assert_string_contains!(shader, "void besl_main(){buff;image;texture;}");
	}

	#[test]
	fn compute_subgroup_intrinsics_lower_to_hlsl_wave_operations() {
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
			.expect("Expected subgroup fixture to lower to HLSL");
		assert_string_contains!(shader, "WaveActiveBallot(group_thread_index<4)");
		assert_string_contains!(shader, "WaveReadLaneAt(group_thread_index,leader)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_find_lsb(mask)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_count(remaining)");
	}

	#[test]
	fn vec4u16_uses_the_native_eight_byte_hlsl_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 HLSL generation");
		assert_string_contains!(shader, "uint16_t4 value;");
		assert_string_does_not_contain!(shader, "struct vec4u16");
	}

	#[test]
	fn packed_vec4f_uses_native_hlsl_vectors_in_nested_records() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::packed_vec4f_meshlet_binding(),
			)
			.expect("Expected packed_vec4f HLSL generation");
		assert_string_contains!(shader, "float4 center_radius;float4 cone_apex_cutoff;");
		assert_string_does_not_contain!(shader, "struct packed_vec4f");
	}

	#[test]
	fn vec2u16_array_uses_the_native_four_byte_hlsl_vector_type() {
		let main = generator::tests::vec2u16_array_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec2u16 HLSL generation");
		assert_string_contains!(shader, "RWStructuredBuffer<uint16_t2> buff : register(u0, space0);");
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<uint2> buff");
	}

	#[test]
	fn vec2f16_array_uses_the_native_four_byte_hlsl_vector_type() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2f16_array_binding(),
			)
			.expect("Expected vec2f16 HLSL generation");
		assert_string_contains!(shader, "RWStructuredBuffer<float16_t2> buff : register(u0, space0);");
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<float2> buff");
	}

	#[test]
	fn f16_storage_types_use_native_hlsl_types() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_f16_storage_binding(),
			)
			.expect("Expected f16 HLSL generation");
		assert_string_contains!(shader, "float16_t scalar;");
		assert_string_contains!(shader, "float16_t2 uv;");
		assert_string_contains!(shader, "float16_t3 normal;");
		assert_string_contains!(shader, "float16_t4 color;");
		assert_string_contains!(shader, "float16_t2(uv32)");
		assert_string_contains!(shader, "float2(uv16)");
		assert_string_contains!(shader, "float16_t(0.5)");
		assert_string_contains!(shader, "float(weight16)");
		assert_string_contains!(shader, "float16_t literal=float16_t(0.25);");
		assert_string_contains!(shader, "weight16*float16_t(2.0)");
		assert_string_contains!(shader, "uv16*float16_t(2.0)");
		assert_string_does_not_contain!(shader, "struct vec2f16");
	}

	#[test]
	fn vector_components_use_hlsl_members_and_numeric_indices_use_subscripts() {
		let root = besl::parse(
			r#"
			main: fn() -> void {
				let vector: vec4f = vec4f(1.0, 2.0, 3.0, 4.0);
				let component: f32 = vector.x;
				let indexed_component: f32 = vector[1];
				let joints: vec4u16 = vec4u16(0, 1, 2, 3);
				let joint_component: u16 = joints.x;
				let indexed_joint: u16 = joints[1];
				if (component > indexed_component) {
					return;
				}
				if (joint_component > indexed_joint) {
					return;
				}
			}
			"#,
		)
		.expect("Expected vector access shader source to parse");
		let root = besl::lex(root).expect("Expected vector access shader source to lex");
		let main = root
			.borrow()
			.get_child("main")
			.expect("Expected vector access shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vector access shader source to generate HLSL");
		assert_string_contains!(shader, "float component=vector.x;");
		assert_string_contains!(shader, "float indexed_component=vector[1];");
		assert_string_contains!(shader, "uint joint_component=joints.x;");
		assert_string_contains!(shader, "uint indexed_joint=joints[1];");
		assert_string_does_not_contain!(shader, "vector[x]");
		assert_string_does_not_contain!(shader, "joints[x]");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"vector-access-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected vector access HLSL to compile to DXIL");
	}

	#[test]
	fn user_struct_constructors_lower_to_hlsl_factories() {
		let root = besl::compile_to_besl(
			r#"
			Pair: struct {
				left: vec4f,
				right: vec4f,
			}

			main: fn () -> void {
				let pair: Pair = Pair(
					vec4f(1.0, 1.0, 1.0, 1.0),
					vec4f(2.0, 2.0, 2.0, 2.0)
				);
				pair;
			}
			"#,
			None,
		)
		.expect("Expected user struct constructor shader source to compile");
		let main = root
			.get_main()
			.expect("Expected user struct constructor shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected user struct constructor shader source to generate HLSL");
		assert_string_contains!(
			shader,
			"Pair pair=besl_construct_Pair(float4(1.0,1.0,1.0,1.0),float4(2.0,2.0,2.0,2.0));"
		);
		assert_string_does_not_contain!(shader, "Pair pair=Pair(");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"user-struct-constructor-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected user struct constructor HLSL to compile to DXIL");
	}

	#[test]
	fn affine_matrix_columns_and_mat4x3_multiplication_preserve_besl_semantics_in_dxil() {
		let mut root = besl::Node::root();
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_child(
			besl::Node::binding(
				"results",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec4f, 4)],
				},
				0,
				false,
				true,
			)
			.into(),
		);
		let root = besl::compile_to_besl(
			r#"
			extend_vec3f: fn (value: vec3f, w: f32) -> vec4f {
				return vec4f(value.x, value.y, value.z, w);
			}

			expand_affine: fn (model: mat4x3f) -> mat4f {
				return mat4f(
					extend_vec3f(model[0], 0.0),
					extend_vec3f(model[1], 0.0),
					extend_vec3f(model[2], 0.0),
					extend_vec3f(model[3], 1.0)
				);
			}

			transform_affine: fn (model: mat4x3f, position: vec4f) -> vec3f {
				return model * position;
			}

			componentwise_affine: fn (left: mat4x3f, right: mat4x3f) -> mat4x3f {
				return left * right;
			}

			main: fn () -> void {
				let model: mat4x3f = mat4x3f(
					vec3f(1.0, 0.0, 0.0),
					vec3f(0.0, 1.0, 0.0),
					vec3f(0.0, 0.0, 1.0),
					vec3f(10.0, 20.0, 30.0)
				);
				let position: vec4f = vec4f(2.0, 3.0, 4.0, 1.0);
				let compact_result: vec3f = transform_affine(model, position);
				let expanded_model: mat4f = expand_affine(model);
				let expanded_result: vec4f = expanded_model * position;
				let componentwise_result: mat4x3f = componentwise_affine(model, model);
				results.values[0] = extend_vec3f(compact_result, 1.0);
				results.values[1] = expanded_result;
				results.values[2] = expanded_model[3];
				results.values[3] = extend_vec3f(componentwise_result[3], 1.0);
			}
			"#,
			Some(root),
		)
		.expect("Expected affine-matrix shader source to compile");
		let main = root.get_main().expect("Expected affine-matrix shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected affine-matrix shader source to generate HLSL");
		assert_string_contains!(shader, "return mul(position, model);");
		assert_string_contains!(
			shader,
			"return transpose(float4x4(extend_vec3f(model[0],0.0),extend_vec3f(model[1],0.0),extend_vec3f(model[2],0.0),extend_vec3f(model[3],1.0)));"
		);
		assert_string_contains!(
			shader,
			"float4x3 model=float4x3(float3(1.0,0.0,0.0),float3(0.0,1.0,0.0),float3(0.0,0.0,1.0),float3(10.0,20.0,30.0));"
		);
		assert_string_contains!(shader, "return left*right;");
		assert_string_contains!(shader, "results[2]=transpose(expanded_model)[3];");
		assert_string_contains!(shader, "float4x3 componentwise_result=componentwise_affine(model,model);");
		assert_string_contains!(shader, "model[3]");
		assert_string_does_not_contain!(shader, "mul(model, position)");
		assert_string_does_not_contain!(shader, "mul(left, right)");
		assert_string_does_not_contain!(shader, "return float4x4(extend_vec3f(model[0]");
		assert_string_does_not_contain!(shader, "transpose(model)[3]");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"affine-matrix-semantics-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected affine-matrix HLSL to compile to DXIL");
	}

	#[test]
	fn square_matrix_columns_survive_buffer_and_expression_access_in_dxil() {
		let mut root = besl::Node::root();
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_children(vec![
			besl::Node::binding(
				"wrapped",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("matrix", mat4f.clone()).into()],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"matrices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", mat4f, 2)],
				},
				1,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"results",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", vec4f, 6)],
				},
				2,
				false,
				true,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(
			r#"
			copy_matrix_columns: fn (matrix: mat4f) -> mat4f {
				return mat4f(matrix[0], matrix[1], matrix[2], matrix[3]);
			}

			direct_constructed_column: fn (matrix: mat4f) -> vec4f {
				return mat4f(matrix[0], matrix[1], matrix[2], matrix[3])[2];
			}

			matrix_arithmetic_columns: fn (matrix: mat4f, scale: f32) -> vec4f {
				let multiplied: vec4f = (matrix * 2.0)[0];
				let added: vec4f = (matrix + scale)[1];
				let divided: vec4f = (matrix / scale)[2];
				let subtracted: vec4f = (scale - matrix)[3];
				let remainder: vec4f = (matrix % scale)[0];
				return multiplied + added + divided + subtracted + remainder;
			}

			main: fn () -> void {
				results.values[0] = wrapped.matrix[1];
				results.values[1] = matrices.values[0][2];
				results.values[2] = (wrapped.matrix + matrices.values[0])[3];
				results.values[3] = copy_matrix_columns(wrapped.matrix)[2];
				results.values[4] = direct_constructed_column(matrices.values[1]);
				results.values[5] = matrix_arithmetic_columns(wrapped.matrix, 2.0);
			}
			"#,
			Some(root),
		)
		.expect("Expected buffered matrix-column shader source to compile");
		let main = root
			.get_main()
			.expect("Expected buffered matrix-column shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected buffered matrix-column shader source to generate HLSL");
		assert_string_contains!(shader, "results[0]=transpose(wrapped[0].matrix)[1];");
		assert_string_contains!(shader, "results[1]=transpose(matrices[0])[2];");
		assert_string_contains!(shader, "results[2]=transpose(wrapped[0].matrix+matrices[0])[3];");
		assert_string_contains!(
			shader,
			"return transpose(float4x4(transpose(matrix)[0],transpose(matrix)[1],transpose(matrix)[2],transpose(matrix)[3]));"
		);
		assert_string_contains!(
			shader,
			"return transpose(transpose(float4x4(transpose(matrix)[0],transpose(matrix)[1],transpose(matrix)[2],transpose(matrix)[3])))[2];"
		);
		assert_string_contains!(shader, "results[3]=transpose(copy_matrix_columns(wrapped[0].matrix))[2];");
		assert_string_contains!(shader, "results[4]=direct_constructed_column(matrices[1]);");
		assert_string_contains!(shader, "float4 multiplied=transpose(mul(matrix, 2.0))[0];");
		assert_string_contains!(shader, "float4 added=transpose(matrix+scale)[1];");
		assert_string_contains!(shader, "float4 divided=transpose(matrix/scale)[2];");
		assert_string_contains!(shader, "float4 subtracted=transpose(scale-matrix)[3];");
		assert_string_contains!(shader, "float4 remainder=transpose(matrix%scale)[0];");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"buffered-matrix-column-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected buffered matrix-column HLSL to compile to DXIL");
	}

	#[test]
	fn task_payload_compaction_uses_groupshared_storage_and_compiles_as_dxil_amplification_shader() {
		let root = besl::compile_to_besl(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			visible_count: workgroup<atomicu32>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				if (lane == 0) {
					atomic_store(visible_count, 0);
				}
				workgroup_barrier();
				if (thread_position() < 32) {
					let payload_index: u32 = atomic_add(visible_count, 1);
					meshlet_indices[payload_index] = thread_position();
				}
				workgroup_barrier();
				if (lane == 0) {
					set_task_mesh_output_count(atomic_load(visible_count));
				}
			}
			"#,
			None,
		)
		.expect("Expected task shader source to compile");
		let main = root.get_main().expect("Expected task shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::task(utils::Extent::line(32), 32), &main)
			.expect("Expected task shader source to generate HLSL");
		assert_string_contains!(shader, "struct ObjectPayload{uint32_t meshlet_indices[32];};");
		assert_string_contains!(shader, "groupshared uint32_t visible_count;");
		assert_string_contains!(shader, "[numthreads(32, 1, 1)]");
		assert_string_contains!(shader, "groupshared ObjectPayload payload;");
		assert_string_contains!(shader, "besl_mesh_output_count = visible_count;");
		assert_string_contains!(shader, "DispatchMesh(besl_mesh_output_count, 1, 1, payload);");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"task-payload-regression",
			"besl_main",
			crate::types::ShaderTypes::Task,
		)
		.expect("Expected task HLSL to compile to amplification DXIL");
	}

	#[test]
	fn mesh_payload_and_primitive_outputs_compile_as_dxil_mesh_shader() {
		let root = besl::compile_to_besl(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			out_instance_index: output<u32, 0, 1>;
			out_primitive_index: output<u32, 1, 1>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				let meshlet_index: u32 = meshlet_indices[threadgroup_position()];
				if (lane == 0) {
					set_mesh_output_counts(3, 1);
				}
				if (lane < 3) {
					set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
				}
				if (lane < 1) {
					set_mesh_triangle(0, vec3u(0, 1, 2));
					set_mesh_primitive_render_target_array_index(0, 2);
					out_instance_index[0] = meshlet_index;
					out_primitive_index[0] = meshlet_index;
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
			.expect("Expected mesh shader source to generate HLSL");
		assert_string_contains!(shader, "struct ObjectPayload{uint32_t meshlet_indices[32];};");
		assert_string_contains!(shader, "struct VertexOutput{float4 position : SV_Position;};");
		assert_string_contains!(shader, "struct PrimitiveOutput{");
		assert_string_contains!(shader, "uint32_t render_target_array_index : SV_RenderTargetArrayIndex;");
		assert_string_contains!(shader, "nointerpolation uint32_t out_instance_index : TEXCOORD0;");
		assert_string_contains!(shader, "nointerpolation uint32_t out_primitive_index : TEXCOORD1;");
		assert_string_contains!(shader, "[outputtopology(\"triangle\")][numthreads(32, 1, 1)]");
		assert_string_contains!(shader, "in payload ObjectPayload payload");
		assert_string_contains!(shader, "SetMeshOutputCounts(3,1);");
		assert_string_contains!(shader, "besl_vertices[lane].position = float4(float(lane),0.0,0.0,1.0)");
		assert_string_contains!(shader, "besl_triangles[0] = uint3(0,1,2)");
		assert_string_contains!(shader, "besl_primitives[0].render_target_array_index = 2");
		assert_string_contains!(shader, "besl_primitives[0].out_instance_index=meshlet_index");

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"mesh-output-regression",
			"besl_main",
			crate::types::ShaderTypes::Mesh,
		)
		.expect("Expected mesh HLSL to compile to mesh DXIL");
	}

	#[test]
	fn array_texture_binding_declares_single_hlsl_template_argument() {
		let mut root =
			besl::parse("main: fn () -> void { shadow_map; }").expect("Expected array texture binding shader source to parse");
		root.add(vec![besl::parser::Node::binding(
			"shadow_map",
			besl::parser::Node::combined_array_image_sampler(),
			11,
			true,
			false,
		)]);

		let root = besl::lex(root).expect("Expected array texture binding shader source to lex");
		let main = RefCell::borrow(&root)
			.get_child("main")
			.expect("Expected array texture binding shader source to contain main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected array texture binding shader source to generate HLSL");
		assert_string_contains!(shader, "Texture2DArray<float4> shadow_map : register(t11, space0);");
		assert_string_does_not_contain!(shader, "Texture2DArray<float4><float4>");
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "static const float color_x=1.0f;");
		assert_string_contains!(shader, "static const float color_y=1.0f;");
		assert_string_contains!(shader, "static const float color_z=1.0f;");
		assert_string_contains!(shader, "static const float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void besl_main(){color;}");
		assert_string_does_not_contain!(shader, "vk::constant_id");
	}

	#[test]
	fn input() {
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void besl_main(float3 color : TEXCOORD0){color;}");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void besl_main(out float3 color : TEXCOORD0){color;}");
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_nointerpolation_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected packed integer vertex HLSL generation");
		let fragment_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Expected packed integer fragment HLSL generation");
		assert_string_contains!(vertex_shader, "uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(vertex_shader, "nointerpolation out uint16_t4 packed_output : TEXCOORD1");
		assert_string_contains!(fragment_shader, "nointerpolation uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(fragment_shader, "out uint16_t4 packed_output : SV_Target1");
		assert_string_does_not_contain!(vertex_shader, "nointerpolation uint16_t2 packed_input");
		assert_string_does_not_contain!(fragment_shader, "nointerpolation uint16_t4 packed_output");
	}

	#[test]
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void besl_main(){float3 albedo=float3(1.0,0.0,0.0);albedo;}");
	}

	#[test]
	fn fetch_intrinsic_lowers_to_hlsl() {
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
		assert_string_contains!(shader, "float4 texel=texture.Load(int3(coord, 0));");
	}

	#[test]
	fn storage_image_intrinsics_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			guard_image_bounds(image, coord);
			let texel: u32 = image_load_u32(image, coord);
			let color: vec4f = image_load(color_image, coord);
			texel;
			color;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
		let void_type = root.get_child("void").expect("Expected void type");
		let image_type = root.get_child("Texture2D").expect("Expected Texture2D type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");

		root.add_children(vec![
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r32ui".to_string(),
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"color_image",
				besl::BindingTypes::Image { format: String::new() },
				1,
				true,
				false,
			)
			.into(),
		]);
		let guard_image_bounds = root.add_child(besl::Node::intrinsic("guard_image_bounds", Vec::new(), void_type).into());
		guard_image_bounds.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type.clone(),
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type.clone(),
			})
			.into(),
		]);
		let image_load_u32 = root.add_child(besl::Node::intrinsic("image_load_u32", Vec::new(), u32_type).into());
		image_load_u32.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type.clone(),
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type.clone(),
			})
			.into(),
		]);
		let image_load = root.add_child(besl::Node::intrinsic("image_load", Vec::new(), vec4f_type).into());
		image_load.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: image_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected storage-image shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "uint2 _besl_image_size;");
		assert_string_contains!(shader, "image.GetDimensions(_besl_image_size.x, _besl_image_size.y);");
		assert_string_contains!(shader, "if (any(coord >= _besl_image_size)) { return; }");
		assert_string_contains!(shader, "uint32_t texel=image[coord];");
		assert_string_contains!(shader, "float4 color=color_image[coord];");
		assert_string_does_not_contain!(shader, "imagecoord");
		assert_string_does_not_contain!(shader, "color_imagecoord");
		assert_string_does_not_contain!(shader, "image[coord].x");
	}

	#[test]
	fn compute_image_math_and_storage_buffers_lower_to_dx12_hlsl() {
		let script = r#"
		main: fn (inverse_projection: mat4f, clip_space: vec4f) -> void {
			let coord: vec2u = thread_id();
			let extent: vec2u = image_size(output_image);
			let noise: f32 = fract(1.25);
			let projected: vec4f = inverse_projection * clip_space;
			let item_index: u32 = item_data.items[0].counter_index;
			write(output_image, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			atomic_store(counter_buffer.count[item_index], 2);
			extent;
			noise;
			projected;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
		let void_type = root.get_child("void").expect("Expected void type");
		let texture_2d_type = root.get_child("Texture2D").expect("Expected Texture2D type");
		let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
		let item =
			root.add_child(besl::Node::r#struct("Item", vec![besl::Node::member("counter_index", u32_type).into()]).into());

		root.add_children(vec![
			besl::Node::binding(
				"item_data",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", item, 8)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
				1,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"output_image",
				besl::BindingTypes::Image { format: String::new() },
				2,
				true,
				true,
			)
			.into(),
		]);

		let image_size = root.add_child(besl::Node::intrinsic("image_size", Vec::new(), vec2u_type.clone()).into());
		image_size.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d_type.clone(),
			})
			.into(),
		]);
		let write = root.add_child(besl::Node::intrinsic("write", Vec::new(), void_type.clone()).into());
		write.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: vec4f_type,
			})
			.into(),
		]);
		let atomic_store = root.add_child(besl::Node::intrinsic("atomic_store", Vec::new(), void_type).into());
		atomic_store.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: atomic_u32,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "stored".to_string(),
				r#type: root.get_child("u32").expect("Expected u32 type"),
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected compute shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "StructuredBuffer<Item> item_data : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter_buffer : register(u1, space0);");
		assert_string_contains!(shader, "uint2 extent;output_image.GetDimensions(extent.x, extent.y);");
		assert_string_contains!(shader, "float noise=frac(1.25);");
		assert_string_contains!(shader, "float4 projected=(mul(inverse_projection, clip_space));");
		assert_string_contains!(shader, "uint32_t item_index=item_data[0].counter_index;");
		assert_string_contains!(shader, "output_image[coord] = float4(1.0,1.0,1.0,1.0);");
		assert_string_contains!(shader, "counter_buffer[item_index] = 2;");
		assert_string_does_not_contain!(shader, "fract(");
		assert_string_does_not_contain!(shader, "item_data : register(u0");
		assert_string_does_not_contain!(shader, "item_data.items");
		assert_string_does_not_contain!(shader, "_besl_atomic_store");
	}

	#[test]
	fn compute_entry_attributes_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::new(32, 16, 1)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "[numthreads(32, 16, 1)]void besl_main(");
		assert_string_does_not_contain!(shader, "[numthreads(32, 16, 1)]#pragma");
	}

	#[test]
	fn buffer_member_access_lowers_to_hlsl_binding_model() {
		let script = r#"
		main: fn () -> void {
			let instance_index: u32 = meshes.meshes[0];
			counter.count[instance_index] = counter.count[instance_index] + 1;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_children(vec![
			besl::Node::binding(
				"meshes",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("meshes", u32_type.clone(), 2)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", u32_type, 2)],
				},
				1,
				false,
				true,
			)
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "StructuredBuffer<uint32_t> meshes : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter : register(u1, space0);");
		assert_string_contains!(shader, "uint32_t instance_index=meshes[0];");
		assert_string_contains!(shader, "counter[instance_index]=(counter[instance_index]+1);");
		assert_string_does_not_contain!(shader, "meshes.meshes");
		assert_string_does_not_contain!(shader, "counter.count");
		assert_string_does_not_contain!(shader, "struct _counter");
	}

	/// Verifies logical narrow indices are recovered from the packed words exposed by DX12.
	#[test]
	fn packed_narrow_buffer_elements_are_extracted_from_u32_words() {
		let script = r#"
		main: fn () -> void {
			let vertex_index: u16 = vertex_indices.vertex_indices[3];
			let primitive_index: u8 = primitive_indices.primitive_indices[5];
			vertex_index;
			primitive_index;
		}
		"#;
		let mut root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8 type");
		let u16_type = root.get_child("u16").expect("Expected u16 type");
		root.add_children(vec![
			besl::Node::binding(
				"vertex_indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("vertex_indices", u16_type, 8)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"primitive_indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("primitive_indices", u8_type, 8)],
				},
				1,
				true,
				false,
			)
			.into(),
		]);
		let main = besl::compile_to_besl(script, Some(root))
			.expect("Failed to compile packed narrow-buffer BESL. The most likely cause is invalid test source.")
			.get_main()
			.expect("Expected packed narrow-buffer main function");
		let shader = Generator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Failed to generate HLSL for packed narrow buffers. The most likely cause is unsupported buffer access.");
		assert_string_contains!(shader, "vertex_indices[(3) / 2u] >> (((3) % 2u) * 16u)) & 0xffffu");
		assert_string_contains!(shader, "primitive_indices[(5) / 4u] >> (((5) % 4u) * 8u)) & 0xffu");
	}

	/// Verifies read-write narrow buffers preserve packed neighbors when one logical element changes.
	#[test]
	fn packed_narrow_buffer_writes_use_atomic_word_updates() {
		let script = r#"
		next_index: fn () -> u32 {
			return 5;
		}

		main: fn () -> void {
			bytes.values[next_index()] = bytes.values[5];
			shorts.values[3] = shorts.values[3];
		}
		"#;
		let mut root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8 type");
		let u16_type = root.get_child("u16").expect("Expected u16 type");
		root.add_children(vec![
			besl::Node::binding(
				"bytes",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", u8_type, 8)],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"shorts",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("values", u16_type, 8)],
				},
				1,
				true,
				true,
			)
			.into(),
		]);
		let main = besl::compile_to_besl(script, Some(root))
			.expect("Failed to compile read-write narrow-buffer BESL. The most likely cause is invalid test source.")
			.get_main()
			.expect("Expected read-write narrow-buffer main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect(
				"Failed to generate HLSL for read-write narrow buffers. The most likely cause is unsupported packed assignment.",
			);
		assert_string_contains!(shader, "RWStructuredBuffer<uint> bytes : register(u0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint> shorts : register(u1, space0);");
		assert_string_contains!(shader, "bytes[(5) / 4u] >> (((5) % 4u) * 8u)) & 0xffu");
		assert_string_contains!(shader, "shorts[(3) / 2u] >> (((3) % 2u) * 16u)) & 0xffffu");
		assert_string_contains!(shader, "uint besl_packed_index_");
		assert_string_contains!(shader, "uint besl_packed_value_");
		assert_string_contains!(shader, "InterlockedAnd(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedOr(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedAnd(shorts[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedOr(shorts[besl_packed_index_");
		assert_eq!(
			shader.matches("=next_index();").count(),
			1,
			"Packed writes must evaluate their index expression exactly once."
		);
		let value_position = shader
			.find("uint besl_packed_value_")
			.expect("Expected packed value temporary");
		let clear_position = shader.find("InterlockedAnd(bytes").expect("Expected packed byte clear");
		assert!(
			value_position < clear_position,
			"Packed writes must evaluate a self-reading right-hand side before clearing its destination lane."
		);

		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(
			&shader,
			"packed-narrow-buffer-write-regression",
			"besl_main",
			crate::types::ShaderTypes::Compute,
		)
		.expect("Expected read-write narrow-buffer HLSL to compile to DXIL");
	}

	#[test]
	fn atomic_compare_exchange_lowers_to_hlsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
			atomic_compare_exchange(shared_keys[thread_idx()], 7, 9);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected compare-exchange shader source to lex");
		let main = root.get_main().expect("Expected compare-exchange main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Expected compare-exchange source to lower to HLSL");
		assert_string_contains!(
			shader,
			"uint32_t previous;InterlockedCompareExchange(shared_keys[group_thread_index], 4294967295, 7, previous);"
		);
		assert_string_contains!(
			shader,
			"{ uint _besl_atomic_previous; InterlockedCompareExchange(shared_keys[group_thread_index], 7, 9, _besl_atomic_previous); }"
		);
	}

	#[test]
	fn structured_buffer_and_cbuffer_access_lower_to_hlsl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = thread_id();
			let item_index: u32 = image_load_u32(index_image, coord);
			let counter_index: u32 = item_data.items[item_index].counter_index;
			atomic_add(counter_buffer.count[counter_index], 1);
			let previous_count: u32 = atomic_add(counter_buffer.count[counter_index], 1);
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
		let item = root
			.add_child(besl::Node::r#struct("Item", vec![besl::Node::member("counter_index", u32_type.clone()).into()]).into());

		root.add_children(vec![
			besl::Node::binding(
				"item_data",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", item, 8)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"counter_buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("count", atomic_u32.clone(), 8)],
				},
				1,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"index_image",
				besl::BindingTypes::Image {
					format: "r32ui".to_string(),
				},
				2,
				true,
				false,
			)
			.into(),
		]);

		let texture_2d = root.get_child("Texture2D").expect("Expected Texture2D type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");
		let image_load_u32 = root.add_child(besl::Node::intrinsic("image_load_u32", Vec::new(), u32_type.clone()).into());
		image_load_u32.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "image".to_string(),
				r#type: texture_2d,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "coord".to_string(),
				r#type: vec2u_type,
			})
			.into(),
		]);
		let atomic_add = root.add_child(besl::Node::intrinsic("atomic_add", Vec::new(), u32_type).into());
		atomic_add.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "value".to_string(),
				r#type: atomic_u32,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "increment".to_string(),
				r#type: root.get_child("u32").expect("Expected u32 type"),
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "[numthreads(8, 8, 1)]void besl_main(");
		assert_string_contains!(shader, "uint32_t item_index=index_image[coord];");
		assert_string_contains!(shader, "StructuredBuffer<Item> item_data : register(t0, space0);");
		assert_string_contains!(shader, "RWStructuredBuffer<uint32_t> counter_buffer : register(u1, space0);");
		assert_string_contains!(shader, "uint32_t counter_index=item_data[item_index].counter_index;");
		assert_string_contains!(shader, "InterlockedAdd(counter_buffer[counter_index], 1);");
		assert_string_contains!(
			shader,
			"uint32_t previous_count;InterlockedAdd(counter_buffer[counter_index], 1, previous_count);"
		);
		assert_string_does_not_contain!(shader, "item_data.items");
		assert_string_does_not_contain!(shader, "counter_buffer.count");
		assert_string_does_not_contain!(shader, "struct _counter_buffer");
		assert_string_does_not_contain!(shader, "index_image[coord].x");
		assert_string_does_not_contain!(shader, "_besl_atomic_add");
	}

	#[test]
	fn parameter_buffer_and_texture_lod_lower_to_dx12_hlsl() {
		let script = r#"
		main: fn () -> void {
			let uv: vec2f = vec2f(0.5, 0.5);
			let texel: vec4f = texture_lod(depth_texture, uv);
			let projected: vec4f = parameters.inverse_view_projection * texel;
			let sun: vec4f = parameters.sun_direction;
			projected;
			sun;
		}
		"#;

		let mut root = besl::Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let texture_2d = root.get_child("Texture2D").expect("Expected Texture2D type");

		root.add_children(vec![
			besl::Node::binding(
				"depth_texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"parameters",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("inverse_view_projection", mat4f).into(),
						besl::Node::member("sun_direction", vec4f.clone()).into(),
					],
				},
				2,
				true,
				false,
			)
			.into(),
		]);

		let texture_lod = root.add_child(besl::Node::intrinsic("texture_lod", Vec::new(), vec4f).into());
		texture_lod.borrow_mut().add_children(vec![
			besl::Node::new(besl::Nodes::Parameter {
				name: "texture".to_string(),
				r#type: texture_2d,
			})
			.into(),
			besl::Node::new(besl::Nodes::Parameter {
				name: "uv".to_string(),
				r#type: vec2f,
			})
			.into(),
		]);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected parameter-buffer shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"struct _parameters{float4x4 inverse_view_projection;float4 sun_direction;};"
		);
		assert_string_contains!(shader, "StructuredBuffer<_parameters> parameters : register(t2, space0);");
		assert_string_contains!(
			shader,
			"float4 texel=depth_texture.SampleLevel(depth_texture_sampler, uv, 0.0);"
		);
		assert_string_contains!(
			shader,
			"float4 projected=(mul(parameters[0].inverse_view_projection, texel));"
		);
		assert_string_contains!(shader, "float4 sun=parameters[0].sun_direction;");
		assert_string_does_not_contain!(shader, "cbuffer parameters");
		assert_string_does_not_contain!(shader, "depth_textureuv");
		assert_string_does_not_contain!(shader, "parameters.inverse_view_projection");
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
			"void used_by_used(){}void used(){used_by_used();}void besl_main(){used();}"
		);
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
			"struct Vertex{float3 position;float3 normal;};Vertex use_vertex(){}void besl_main(){use_vertex();}"
		);
	}

	#[test]
	fn push_constant() {
		let main = generator::tests::push_constant();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct PushConstant{uint32_t material_id;};");
		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
		assert_string_contains!(shader, "void besl_main(){push_constant;}");
		assert_string_does_not_contain!(shader, "vk::push_constant");
	}

	#[test]
	fn push_constants_and_flat_resources_use_space_zero() {
		let script = r#"
		main: fn () -> void {
			push_constant;
			values;
		}
		"#;

		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_children(vec![
			besl::Node::push_constant(vec![besl::Node::member("material_id", u32_type.clone()).into()]).into(),
			besl::Node::binding(
				"values",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("items", u32_type, 4)],
				},
				7,
				true,
				false,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected push-constant shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected push-constant shader source to generate HLSL");
		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
		assert_string_contains!(shader, "StructuredBuffer<uint32_t> values : register(t7, space0);");
		assert_string_does_not_contain!(shader, "vk::push_constant");
	}

	#[test]
	fn test_hlsl() {
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
				besl::Node::hlsl(
					"output.position = float4(0, 0, 0, 1)".to_string(),
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
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "output.position = float4(0, 0, 0, 1)");
	}

	#[test]
	fn test_instrinsic() {
		let main = generator::tests::intrinsic();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "void besl_main(){0 + 1.0 * 2;}");
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

		// The HLSL transpiler should use the HLSL code.
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void besl_main(){output.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "HLSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "static const float PI = 3.14;");
		assert_string_contains!(shader, "void besl_main(){PI;}");
	}

	#[test]
	fn conditional_blocks_lower_to_hlsl() {
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
	fn bitwise_operators_lower_to_hlsl() {
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
	fn comparison_and_continue_lower_to_hlsl() {
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
	fn scalar_max_and_clamp_lower_to_hlsl() {
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
	fn const_array_variable_lowers_to_hlsl() {
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
		assert_string_contains!(shader, "static const float3 WEIGHTS = float3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
		assert_string_does_not_contain!(shader, "WEIGHTS[3]");
	}

	#[test]
	fn short_scalar_arrays_lower_to_hlsl_vectors() {
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
			.expect("Expected scalar arrays to lower to HLSL vectors");
		assert_string_contains!(shader, "float3 scalar_f32()");
		assert_string_contains!(shader, "uint16_t3 scalar_u16()");
		assert_string_contains!(shader, "uint3 scalar_u32()");
		assert_string_contains!(shader, "uint3 mirror_indices(uint3 indices)");
		assert_string_contains!(shader, "float3 floats=scalar_f32();");
		assert_string_contains!(shader, "uint16_t3 shorts=scalar_u16();");
		assert_string_contains!(shader, "uint3 indices=mirror_indices(scalar_u32());");
	}

	#[test]
	fn mix_intrinsic_lowers_to_hlsl_lerp() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = mix(0.0, 1.0, 0.5);
			value;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected mix shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "float value=lerp(0.0,1.0,0.5);");
		assert_string_does_not_contain!(shader, "mix(");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_hlsl() {
		let main = generator::tests::return_value();

		let minified_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(minified_shader, "float besl_main(){return 1.0;}");

		let pretty_shader = Generator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(pretty_shader, "float besl_main() {\n\treturn 1.0;\n}\n");
	}
}

pub use Generator as HLSLTranspiler;
