use std::{cell::RefCell, fmt::Write as _};

use super::{
	IntrinsicRequirements, ResourceAccessorKind, any_code_node, intrinsic_requirements, resource_accessor,
	resource_reference_kind, runtime_buffer_element,
};
use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, Stages, emit_statement_block, is_integer_besl_type,
	ordered_shader_nodes,
};

mod analysis;
mod byte_windows;
mod emit;
mod facade;
mod generate;
mod node_emitter;
mod ray_tracing;
mod reserved;

pub(crate) use analysis::*;
pub(crate) use emit::*;
pub use facade::Generator;
pub(crate) use facade::HlslBufferBindingSource;
pub(crate) use generate::*;

/// Returns the narrow array element DX12 packs into shared 32-bit words, so declarations, accesses, and reflected
/// strides agree. Other element types are stored natively.
pub(crate) fn hlsl_narrow_element(element_type: &str) -> Option<&'static str> {
	match element_type {
		"u8" => Some("u8"),
		"u16" => Some("u16"),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		shader::generator::{self, ShaderGenerationSettings},
		types::ShaderTypes,
	};

	/// Builds one intrinsic parameter for tests that declare their own intrinsics.
	fn parameter(name: &str, r#type: besl::NodeReference) -> besl::NodeReference {
		besl::Node::new(besl::Nodes::Parameter {
			name: name.to_string(),
			r#type,
		})
		.into()
	}

	/// Lowers `main` to minified HLSL.
	fn generate(settings: &ShaderGenerationSettings, main: &besl::NodeReference) -> String {
		Generator::new()
			.minified(true)
			.generate(settings, main)
			.expect("Expected HLSL generation")
	}

	/// Links a standalone BESL source and lowers its `main` to minified HLSL.
	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let root = besl::compile_to_besl(source, None).expect("Expected fixture source to link");
		generate(settings, &root.get_main().expect("Expected fixture main function"))
	}

	#[test]
	fn prefix_operators_and_if_values_lower_to_hlsl() {
		let shader = lower_fixture(
			r#"
			Body: struct { velocity: vec3f, color: vec4f, bits: u32, weight: f16 }
			Frame: struct { projection: mat4f }
			bodies: descriptor<{ type: Body[], binding: 0, access: read_write }>;
			frame: descriptor<{ type: Frame, binding: 1, access: read }>;
			main: fn (input: StageInput) -> void {
				let item: u32 = input.thread_id.x;
				let flag: bool = bodies[item].bits > 3;
				bodies[item].bits = ~bodies[item].bits ^ (if (flag) { 1 } else { 2 });
				bodies[item].velocity = - -bodies[item].velocity - -bodies[item].velocity;
				bodies[item].weight = if (!flag) { bodies[item].weight } else { 1.0 };
				bodies[item].color = frame.projection * -bodies[item].color + frame.projection * (if (flag) { bodies[item].color } else { bodies[item].color });
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "((~bodies[item].bits)^(flag?1:2))");
		assert_string_contains!(shader, "((-(-bodies[item].velocity))-(-bodies[item].velocity))");
		assert_string_contains!(shader, "((!flag)?bodies[item].weight:float16_t(1.0))");
		assert_string_contains!(shader, "mul(frame[0].projection, -bodies[item].color)");
	}

	#[test]
	fn gather_reads_the_texel_quad_with_gather_in_hlsl() {
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
		assert_string_contains!(shader, "depth_texture.Gather(depth_texture_sampler, ");
		assert_string_contains!(shader, "array_depth_texture.Gather(array_depth_texture_sampler, float3(");
	}

	/// Verifies a texture parameter arrives with its sampler, through a binding and through another parameter.
	#[test]
	fn texture_parameters_carry_their_sampler_in_hlsl() {
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
			"float quad_maximum(Texture2DArray<float4> depth_map,SamplerState depth_map_sampler,float2 uv,uint32_t layer)"
		);
		assert_string_contains!(shader, "quad_maximum(depth_map,depth_map_sampler,uv,0)");
		assert_string_contains!(shader, "depth_map.Gather(depth_map_sampler, float3(");
		assert_string_contains!(shader, "first_layer_maximum(array_depth_texture,array_depth_texture_sampler,");
	}

	/// Verifies a 16-bit unorm storage image declares its unorm element type, which typed UAV stores need.
	#[test]
	fn unorm_storage_image_declares_unorm_float_in_hlsl() {
		let source = r#"
			cells: descriptor<{ type: StorageImage<r16>, binding: 0, access: write }>;
			main: fn () -> void {
				write(cells, vec2u(0, 0), vec4f(0.5, 0.0, 0.0, 1.0));
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(shader, "RWTexture2D<unorm float4> cells");
	}

	#[test]
	fn modern_half_and_integer_atomics_lower_to_shader_model_6_9_hlsl() {
		let source = r#"
			unsigned_value: workgroup<atomicu32>;
			signed_value: workgroup<atomici32>;
			main: fn () -> void {
				let signed_one: i32 = 1;
				let signed_two: i32 = 2;
				atomic_store(unsigned_value, 1);
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
				atomic_max(signed_value, signed_two);
				let narrowed: u16 = u16(7);
				let zero: f16 = f16(0.0);
				let one: f16 = f16(1.0);
				let fused: f16 = fma(one, one, one);
				let fused_vector: vec3f16 = fma(vec3f16(one, one, one), vec3f16(one, one, one), vec3f16(one, one, one));
				if (is_nan(zero / zero) || is_infinite(one / zero) || is_finite(fused) || is_normal(fused_vector.x)) {
					atomic_store(unsigned_value, u32(narrowed));
				}
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::line(1)));

		assert_string_contains!(shader, "// Shader Model 6.9");
		assert_string_contains!(shader, "groupshared uint32_t unsigned_value;");
		assert_string_contains!(shader, "groupshared int32_t signed_value;");
		for operation in [
			"InterlockedExchange(",
			"InterlockedAdd(",
			"InterlockedMin(",
			"InterlockedMax(",
			"InterlockedAnd(",
			"InterlockedOr(",
			"InterlockedXor(",
			"InterlockedCompareExchange(",
		] {
			assert_string_contains!(shader, operation);
		}
		assert_string_contains!(shader, "InterlockedAdd(unsigned_value,-(");
		assert_string_contains!(shader, "uint16_t narrowed=uint16_t(7);");
		assert_string_contains!(shader, "float16_t fused=_besl_fma_f16(");
		assert_string_contains!(shader, "float16_t3 fused_vector=_besl_fma_f16(");
		assert_string_contains!(shader, "precise float residual =");
		assert_string_does_not_contain!(shader, "double");
		assert_string_does_not_contain!(shader, "mad(");
		for predicate in ["isnan(", "isinf(", "isfinite(", "isnormal("] {
			assert_string_contains!(shader, predicate);
		}

		compile_dxil(&shader, "modern-half-atomic-regression", ShaderTypes::Compute);
	}

	/// Atomics run exactly where BESL evaluates them: an `else if` condition only when that branch is reached, the
	/// right side of `&&` only when the left is true, and a `for` condition and update on every iteration.
	#[test]
	fn atomic_calls_run_where_besl_evaluates_them() {
		let source = r#"
			counter: workgroup<atomicu32>;
			main: fn () -> void {
				let n: u32 = 0;
				if (n == 1) { n = 2; } else if (atomic_load(counter) == 0) { n = 3; }
				if (thread_idx() == 0 && atomic_add(counter, 1) == 0) { n = 4; }
				for (atomic_store(counter, 0); atomic_load(counter) < 4; atomic_store(counter, n)) { n = n + 1; }
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::line(1)));

		assert_string_contains!(shader, "}else if(_besl_atomic_0()==0){n=3;}");
		assert_string_contains!(shader, "&&(_besl_atomic_1(1)==0)");
		assert_string_contains!(shader, "for(_besl_atomic_2(0);_besl_atomic_3()<4;_besl_atomic_4(n))");
		assert_string_contains!(
			shader,
			"void _besl_atomic_2(uint32_t _besl_p0){uint32_t _besl_replaced;InterlockedExchange(counter,_besl_p0,_besl_replaced);}"
		);
		assert_string_contains!(
			shader,
			"uint32_t _besl_atomic_1(uint32_t _besl_p0){uint32_t _besl_previous;InterlockedAdd(counter,_besl_p0,_besl_previous);return _besl_previous;}"
		);

		compile_dxil(&shader, "besl-atomic-evaluation-order", ShaderTypes::Compute);
	}

	#[test]
	fn signed_atomic_sub_uses_wrapping_unsigned_bit_arithmetic() {
		let source = r#"
			signed_value: workgroup<atomici32>;
			main: fn () -> void {
				let minimum: i32 = 0 - 2147483647 - 1;
				let previous: i32 = atomic_sub(signed_value, minimum);
				previous;
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::line(1)));

		assert_string_contains!(
			shader,
			"InterlockedAdd(signed_value,asint(0u-asuint(_besl_p0)),_besl_previous);"
		);
		assert_string_contains!(shader, "int32_t previous=_besl_atomic_0(minimum);");
		assert_string_does_not_contain!(shader, "InterlockedAdd(signed_value,-(");

		compile_dxil(&shader, "signed-atomic-sub-wrapping-regression", ShaderTypes::Compute);
	}

	#[test]
	fn bindings() {
		let main = generator::tests::bindings();

		let shader = generate(&ShaderGenerationSettings::vertex(), &main);

		// The test sets read=true, write=true for buff, which makes it a RWStructuredBuffer
		// Check for structured buffer (writable buffer)
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };");
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<_buff> buff = ResourceDescriptorHeap[_besl_sets[0].x];"
		);

		// Check for RWTexture2D (image)
		assert_string_contains!(
			shader,
			"static RWTexture2D<float4> image = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);

		// Check for Texture2D and SamplerState (combined image sampler)
		// `texture` is an HLSL keyword, so the binding and its sampler use the escaped name.
		assert_string_contains!(
			shader,
			"static Texture2D<float4> besl_texture = ResourceDescriptorHeap[_besl_sets[0].x+2u];"
		);
		// The texture is the set's first texture with an application-bound sampler, so it takes the first sampler.
		assert_string_contains!(
			shader,
			"static SamplerState besl_texture_sampler = SamplerDescriptorHeap[_besl_sets[0].y];"
		);

		// Check main function
		assert_string_contains!(shader, "void besl_main(){buff;image;besl_texture;}");
	}

	#[test]
	fn user_names_that_collide_with_hlsl_reserved_words_are_prefixed() {
		let script = r#"
		half: struct {
			float3: vec3f,
		}

		Wrapper: struct {
			value: half,
		}

		lerp: fn (sampler: f32) -> f32 {
			return sampler;
		}

		texture: descriptor<{ type: Texture2D, binding: 1, access: read }>;

		main: fn () -> void {
			let float4: half = half(vec3f(1.0, 2.0, 3.0));
			let wrapper: Wrapper = Wrapper(float4);
			let mul: f32 = lerp(wrapper.value.float3.x);
			let color: vec4f = sample(texture, vec2f(0.0, 0.0));
			Buffer.min = mul;
			color;
		}
		"#;

		let mut root = besl::Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		root.add_children(vec![
			besl::Node::binding(
				"Buffer",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("min", f32_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected reserved-name shader source to compile");
		let main = root.get_main().expect("Expected reserved-name shader source to contain main");

		let shader = generate(&ShaderGenerationSettings::fragment(), &main);

		assert_string_contains!(shader, "struct besl_half{float3 besl_float3;};");
		assert_string_contains!(shader, "struct Wrapper{besl_half value;};");
		assert_string_contains!(shader, "besl_half besl_construct_half(float3 besl_argument_float3)");
		assert_string_contains!(shader, "besl_value.besl_float3=besl_argument_float3;");
		assert_string_contains!(shader, "float besl_lerp(float besl_sampler){return besl_sampler;}");
		assert_string_contains!(shader, "struct _Buffer{float besl_min;};");
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<_Buffer> besl_Buffer = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static Texture2D<float4> besl_texture = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(
			shader,
			"static SamplerState besl_texture_sampler = SamplerDescriptorHeap[_besl_sets[0].y];"
		);
		assert_string_contains!(shader, "besl_half besl_float4=besl_construct_half(float3(1.0,2.0,3.0));");
		assert_string_contains!(shader, "float besl_mul=besl_lerp(wrapper.value.besl_float3.x);");
		assert_string_contains!(shader, "besl_texture.Sample(besl_texture_sampler, float2(0.0,0.0))");
		assert_string_contains!(shader, "besl_Buffer[0].besl_min=besl_mul;");
		assert_string_contains!(shader, "void besl_main(");
	}

	#[test]
	fn each_set_reads_its_descriptors_from_its_own_root_constant_bases() {
		// The shader takes a shared set, and a set it never reaches; a layer it doesn't take forms no set.
		let shader = lower_fixture(
			"Scene: layer { views: descriptor<{ type: f32[], binding: 0, access: read }>; shadow_map: descriptor<{ type: Texture2D, binding: 1, access: read }>; meshes: descriptor<{ type: f32[], binding: 2, access: read }>; } History: layer { history: descriptor<{ type: Texture2D, binding: 0, access: read }>; } result: descriptor<{ type: f32[], binding: 0, access: write }>; scale: fn (scene: Scene) -> f32 { return scene.meshes[0]; } main: fn (scene: Scene, history: History) -> void { result[0] = scale(scene); }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		// The unreached history set keeps position 1, so the shader's own set is the third and the bases cover two vectors.
		assert_string_contains!(shader, "cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[2]; };");
		// `meshes` follows two descriptors of its set, whichever of them the shader reads.
		assert_string_contains!(
			shader,
			"static StructuredBuffer<float> meshes = ResourceDescriptorHeap[_besl_sets[0].x+2u];"
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<float> result = ResourceDescriptorHeap[_besl_sets[1].x];"
		);
		assert_string_does_not_contain!(shader, "views");
		assert_string_does_not_contain!(shader, "history");

		compile_dxil(&shader, "besl-layout-sets", ShaderTypes::Compute);
	}

	#[test]
	fn fixed_samplers_read_the_sampler_heap_index_their_state_decides() {
		let shader = lower_fixture(
			r#"
			nearest_texel: sampler<{ filter: nearest, mip_filter: nearest, address: repeat }>;
			tiles: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: nearest_texel }>;
			albedo: descriptor<{ type: Texture2D, binding: 1, access: read }>;
			result: descriptor<{ type: f32[], binding: 2, access: write }>;
			main: fn () -> void {
				result[0] = texture_lod(tiles, vec2f(0.5, 0.5), 1.0).x + texture_lod(albedo, vec2f(0.5, 0.5), 0.0).x;
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		// Nearest filters, average reduction, and repeat addressing: 0 + 2 × 0 + 4 × 0 + 12 × 1.
		assert_string_contains!(shader, "static SamplerState tiles_sampler = SamplerDescriptorHeap[12u];");
		// The fixed sampler takes none of the set's samplers, so `albedo` takes the first.
		assert_string_contains!(
			shader,
			"static Texture2D<float4> albedo = ResourceDescriptorHeap[_besl_sets[0].x+1u];static SamplerState albedo_sampler = SamplerDescriptorHeap[_besl_sets[0].y];"
		);

		compile_dxil(&shader, "besl-fixed-samplers", ShaderTypes::Compute);
	}

	#[test]
	fn anisotropic_fixed_sampler_arrays_read_one_sampler_after_the_isotropic_ones() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&super::super::link_with_sample_grad(
				r#"
				grazing: sampler<{ address: repeat, anisotropy: 16 }>;
				result: descriptor<{ type: f32[], binding: 0, access: write }>;
				textures: descriptor<{ type: Texture2D, binding: 1, access: read, count: 4, sampler: grazing }>;
				main: fn () -> void {
					result[0] = sample_texture_2d_array_grad(textures, 2, vec2f(0.5, 0.5), vec2f(0.0, 0.0), vec2f(0.0, 0.0)).x;
				}
				"#,
			),
		);

		// Anisotropic samplers follow the 36 isotropic ones: 36 + 3 × (16x is level 4 - 1) + repeat 1.
		assert_string_contains!(
			shader,
			"struct _besl_array_textures_sampler { SamplerState operator[](uint i) { return SamplerDescriptorHeap[46u]; } };"
		);

		compile_dxil(&shader, "besl-anisotropic-fixed-sampler-array", ShaderTypes::Compute);
	}

	#[test]
	fn runtime_buffer_and_texture_array_layer_use_native_hlsl_resources() {
		let shader = lower_fixture(super::super::RUNTIME_ARRAY_FRAGMENT, &ShaderGenerationSettings::fragment());

		assert_string_contains!(
			shader,
			"static StructuredBuffer<Instance> instances = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(
			shader,
			"static Texture2DArray<float4> sprites = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(shader, "Instance instance=instances[");
		assert_string_contains!(shader, "sprites.Sample(sprites_sampler, float3(");
		assert_string_contains!(shader, "float(instance.sprite_id)");
	}

	#[test]
	fn scalar_runtime_arrays_pack_narrow_hlsl_elements_into_words() {
		let shader = lower_fixture(
			super::super::SCALAR_RUNTIME_ARRAY_COMPUTE,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		assert_string_contains!(
			shader,
			"static StructuredBuffer<float3> positions = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static StructuredBuffer<uint> indices = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(
			shader,
			"static StructuredBuffer<uint> corners = ResourceDescriptorHeap[_besl_sets[0].x+2u];"
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint32_t> results = ResourceDescriptorHeap[_besl_sets[0].x+3u];"
		);
		// Narrow elements are recovered from the 32-bit word that holds them.
		assert_string_contains!(shader, "((indices[(item) / 2u] >> (((item) % 2u) * 16u)) & 0xffffu)");
		assert_string_contains!(shader, "((corners[(item) / 4u] >> (((item) % 4u) * 8u)) & 0xffu)");
	}

	#[test]
	fn size_queries_are_values_anywhere_an_expression_is() {
		let shader = lower_fixture(
			r#"
			Shadows: layer {
				cascades: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
			}
			depth: descriptor<{ type: Texture2D, binding: 0, access: read }>;
			result: descriptor<{ type: StorageImage<rgba16f>, binding: 1, access: write }>;
			texel_size: fn (extent: vec2u) -> f32 {
				return 1.0 / f32(extent.x);
			}
			main: fn (shadows: Shadows) -> void {
				let layered: vec2u = texture_size(shadows.cascades);
				let spacing: f32 = texel_size(texture_size(depth)) + f32(image_size(result).y);
				let last: vec2u = texture_size(depth) - vec2u(1, 1);
				write(result, last, vec4f(spacing, f32(layered.x), 0.0, 1.0));
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
		);

		assert_string_contains!(shader, "template<typename T> uint2 _besl_array_texture_size(T texture)");
		assert_string_contains!(shader, "uint2 layered=_besl_array_texture_size(cascades);");
		assert_string_contains!(shader, "texel_size(_besl_texture_size(depth))");
		assert_string_contains!(shader, "_besl_texture_size(result).y");
		assert_string_contains!(shader, "(_besl_texture_size(depth)-uint2(1,1))");

		compile_dxil(&shader, "size-query-expressions", ShaderTypes::Compute);
	}

	#[test]
	fn descriptor_array_elements_reach_every_texture_intrinsic_in_hlsl() {
		let shader = lower_fixture(super::super::DESCRIPTOR_ARRAY_FRAGMENT, &ShaderGenerationSettings::fragment());

		assert_string_contains!(
			shader,
			"uint2 size=_besl_texture_size(textures[NonUniformResourceIndex(items[index].slot)]);"
		);
		assert_string_contains!(
			shader,
			"textures[NonUniformResourceIndex(index+1)].SampleLevel(textures_sampler[NonUniformResourceIndex(index+1)], uv, 0.0)"
		);
		assert_string_contains!(
			shader,
			"textures[NonUniformResourceIndex(items[index].slot)].Sample(textures_sampler[NonUniformResourceIndex(items[index].slot)], uv)"
		);
		assert_string_contains!(shader, "textures[NonUniformResourceIndex(index)].Load(int3(");
		assert_string_contains!(
			shader,
			"textures[NonUniformResourceIndex(index)].Gather(textures_sampler[NonUniformResourceIndex(index)], uv)"
		);
	}

	#[test]
	fn descriptor_array_sample_grad_indexes_the_matching_sampler_and_compiles() {
		let mut root = besl::parse(
			"textures: descriptor<{ type: Texture2D, binding: 0, access: read, count: 4 }>; main: fn () -> void { let color: vec4f = sample_texture_2d_array_grad(textures, 2, vec2f(0.0, 0.0), vec2f(0.0, 0.0), vec2f(0.0, 0.0)); color; }",
		)
		.expect("Expected descriptor-array gradient sample source to parse");
		root.add(vec![besl::parser::Node::intrinsic_with_parameters(
			"sample_texture_2d_array_grad",
			vec![
				besl::parser::Node::parameter("texture_array", "Texture2D"),
				besl::parser::Node::parameter("texture_index", "u32"),
				besl::parser::Node::parameter("uv", "vec2f"),
				besl::parser::Node::parameter("uv_derivative_x", "vec2f"),
				besl::parser::Node::parameter("uv_derivative_y", "vec2f"),
			],
			besl::parser::Node::sentence(vec![besl::parser::Node::member_expression("textures")]),
			"vec4f",
		)]);
		let root = besl::lex(root).expect("Expected descriptor-array gradient sample source to link");
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&root.get_main().expect("Expected main"),
		);

		// An array reads element `i` at the set's base plus its placement plus `i`, as a native array would index.
		assert_string_contains!(
			shader,
			"struct _besl_array_textures { Texture2D<float4> operator[](uint i) { return ResourceDescriptorHeap[NonUniformResourceIndex(_besl_sets[0].x+i)]; } };static _besl_array_textures textures;"
		);
		assert_string_contains!(
			shader,
			"struct _besl_array_textures_sampler { SamplerState operator[](uint i) { return SamplerDescriptorHeap[NonUniformResourceIndex(_besl_sets[0].y+i)]; } };static _besl_array_textures_sampler textures_sampler;"
		);
		assert_string_contains!(
			shader,
			"textures[NonUniformResourceIndex(2)].SampleGrad(textures_sampler[NonUniformResourceIndex(2)],float2(0.0,0.0),float2(0.0,0.0),float2(0.0,0.0))"
		);

		compile_dxil(&shader, "descriptor-array-sample-grad-regression", ShaderTypes::Compute);
	}

	#[test]
	fn structural_position_uses_sv_position_without_colliding_with_a_local() {
		let shader = lower_fixture(super::super::STRUCTURAL_POSITION_VERTEX, &ShaderGenerationSettings::vertex());

		assert_string_contains!(shader, "out float4 _besl_interface_position : SV_Position");
		assert_string_contains!(shader, "float4 position=float4(float(vertex_index),0.0,0.0,1.0);");
		assert_string_contains!(shader, "_besl_interface_position=position;");
		assert!(!shader.contains("_besl_interface_position : TEXCOORD"));
	}

	#[test]
	fn vec4f_meshlet_record_uses_native_hlsl_vectors() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&generator::tests::vec4f_meshlet_binding(),
		);
		assert_string_contains!(shader, "float4 center_radius;float4 cone_apex_cutoff;");
	}

	#[test]
	fn vec2u16_array_uses_the_native_four_byte_hlsl_vector_type() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&generator::tests::vec2u16_array_binding(),
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint16_t2> buff = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<uint2> buff");
	}

	#[test]
	fn f16_storage_types_use_native_hlsl_types() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&generator::tests::mixed_f16_storage_binding(),
		);
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
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
		assert_string_contains!(shader, "float component=besl_vector.x;");
		assert_string_contains!(shader, "float indexed_component=besl_vector[1];");
		assert_string_contains!(shader, "uint16_t joint_component=joints.x;");
		assert_string_contains!(shader, "uint16_t indexed_joint=joints[1];");
		assert_string_does_not_contain!(shader, "vector[x]");
		assert_string_does_not_contain!(shader, "joints[x]");

		compile_dxil(&shader, "vector-access-regression", ShaderTypes::Compute);
	}

	#[test]
	fn user_struct_constructors_lower_to_hlsl_factories() {
		let shader = lower_fixture(
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
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(
			shader,
			"Pair pair=besl_construct_Pair(float4(1.0,1.0,1.0,1.0),float4(2.0,2.0,2.0,2.0));"
		);
		assert_string_does_not_contain!(shader, "Pair pair=Pair(");

		compile_dxil(&shader, "user-struct-constructor-regression", ShaderTypes::Compute);
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
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
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

		compile_dxil(&shader, "affine-matrix-semantics-regression", ShaderTypes::Compute);
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
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
		assert_string_contains!(shader, "results[0]=transpose(wrapped[0].besl_matrix)[1];");
		assert_string_contains!(shader, "results[1]=transpose(matrices[0])[2];");
		assert_string_contains!(shader, "results[2]=transpose(wrapped[0].besl_matrix+matrices[0])[3];");
		assert_string_contains!(
			shader,
			"return transpose(float4x4(transpose(besl_matrix)[0],transpose(besl_matrix)[1],transpose(besl_matrix)[2],transpose(besl_matrix)[3]));"
		);
		assert_string_contains!(
			shader,
			"return transpose(transpose(float4x4(transpose(besl_matrix)[0],transpose(besl_matrix)[1],transpose(besl_matrix)[2],transpose(besl_matrix)[3])))[2];"
		);
		assert_string_contains!(
			shader,
			"results[3]=transpose(copy_matrix_columns(wrapped[0].besl_matrix))[2];"
		);
		assert_string_contains!(shader, "results[4]=direct_constructed_column(matrices[1]);");
		assert_string_contains!(shader, "float4 multiplied=transpose(mul(besl_matrix, 2.0))[0];");
		assert_string_contains!(shader, "float4 added=transpose(besl_matrix+scale)[1];");
		assert_string_contains!(shader, "float4 divided=transpose(besl_matrix/scale)[2];");
		assert_string_contains!(shader, "float4 subtracted=transpose(scale-besl_matrix)[3];");
		assert_string_contains!(shader, "float4 remainder=transpose(besl_matrix%scale)[0];");

		compile_dxil(&shader, "buffered-matrix-column-regression", ShaderTypes::Compute);
	}

	#[test]
	fn task_payload_compaction_uses_groupshared_storage_and_compiles_as_dxil_amplification_shader() {
		let shader = lower_fixture(
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
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);
		assert_string_contains!(shader, "struct ObjectPayload{uint32_t meshlet_indices[32];};");
		assert_string_contains!(shader, "groupshared uint32_t visible_count;");
		assert_string_contains!(shader, "[numthreads(32, 1, 1)]");
		assert_string_contains!(shader, "groupshared ObjectPayload payload;");
		assert_string_contains!(shader, "InterlockedOr(visible_count,0,_besl_previous)");
		assert_string_contains!(shader, "besl_mesh_output_count = _besl_atomic_");
		assert_string_contains!(shader, "DispatchMesh(besl_mesh_output_count, 1, 1, payload);");

		compile_dxil(&shader, "task-payload-regression", ShaderTypes::Task);
	}

	#[test]
	fn task_shaders_that_return_early_still_dispatch_once_from_the_entry_point() {
		let shader = lower_fixture(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			visible_count: workgroup<atomicu32>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				if (thread_position() >= 64) {
					if (lane == 0) {
						set_task_mesh_output_count(0);
					}
					return;
				}
				if (lane == 0) {
					atomic_store(visible_count, 1);
					meshlet_indices[0] = lane;
					set_task_mesh_output_count(atomic_load(visible_count));
				}
			}
			"#,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);
		// The early return leaves only the body helper, so the entry point's one dispatch runs on every path.
		assert_string_contains!(
			shader,
			"void besl_task_body(uint3 dispatch_thread_id,uint3 group_thread_id,uint3 group_id,uint group_thread_index){"
		);
		assert_string_contains!(
			shader,
			"besl_task_body(dispatch_thread_id,group_thread_id,group_id,group_thread_index);GroupMemoryBarrierWithGroupSync();DispatchMesh(besl_mesh_output_count, 1, 1, payload);"
		);

		compile_dxil(&shader, "task-early-return", ShaderTypes::Task);
	}

	/// Verifies per-vertex mesh outputs join the interpolated vertex struct while per-primitive outputs stay flat.
	#[test]
	fn vertex_mesh_outputs_join_the_vertex_struct() {
		let shader = generate(
			&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)),
			&generator::tests::vertex_and_primitive_mesh_outputs(),
		);
		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position : SV_Position;float2 out_uv : TEXCOORD2;};"
		);
		assert_string_contains!(
			shader,
			"struct PrimitiveOutput{nointerpolation uint32_t out_primitive_index : TEXCOORD1;};"
		);
		assert_string_contains!(shader, "besl_vertices[lane].out_uv=float2(float(lane),1.0)");
		assert_string_contains!(shader, "besl_primitives[0].out_primitive_index=lane");
	}

	#[test]
	fn mesh_payload_and_primitive_outputs_compile_as_dxil_mesh_shader() {
		let shader = lower_fixture(
			r#"
			meshlet_indices: task_payload<u32, 32>;
			out_instance_index: output<u32, 0, 1>;
			out_primitive_index: output<u32, 1, 1>;

			main: fn () -> void {
				let lane: u32 = thread_idx();
				let meshlet_index: u32 = meshlet_indices[threadgroup_position()];
				set_mesh_output_counts(3, 1);
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
			&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)),
		);
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

		compile_dxil(&shader, "mesh-output-regression", ShaderTypes::Mesh);
	}

	#[test]
	fn array_texture_binding_declares_single_hlsl_template_argument() {
		let mut root =
			besl::parse("main: fn () -> void { shadow_map; }").expect("Expected array texture binding shader source to parse");
		root.add(vec![besl::parser::Node::binding(
			"shadow_map",
			besl::parser::Node::combined_array_image_sampler(),
			0,
			true,
			false,
		)]);

		let root = besl::lex(root).expect("Expected array texture binding shader source to lex");
		let main = root
			.get_main()
			.expect("Expected array texture binding shader source to contain main");
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
		assert_string_contains!(
			shader,
			"static Texture2DArray<float4> shadow_map = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_does_not_contain!(shader, "Texture2DArray<float4><float4>");
	}

	#[test]
	fn vertex_invocation_indices_lower_to_system_semantics_and_reach_helpers() {
		let shader = lower_fixture(super::super::VERTEX_BUILTIN_HELPER, &ShaderGenerationSettings::vertex());

		assert_string_contains!(shader, "vertex_index : SV_VertexID");
		assert_string_contains!(shader, "instance_index : SV_InstanceID");
		assert_string_contains!(shader, "uint32_t invocation_sum(");
		assert_string_contains!(shader, "out_value=invocation_sum(");
		assert_eq!(shader.matches("vertex_index").count(), 4);
		assert_eq!(shader.matches("instance_index").count(), 4);
		assert_string_does_not_contain!(shader, "vertex_index : TEXCOORD");
		assert_string_does_not_contain!(shader, "instance_index : TEXCOORD");
	}

	/// Verifies a pixel shader declares the position first and its varyings by location, as the vertex shader writes
	/// them, so D3D12 links every varying to the register the vertex shader wrote it to.
	#[test]
	fn pixel_inputs_line_up_with_the_vertex_outputs_they_read() {
		let vertex = lower_fixture(
			r#"
			main: fn (input: StageInput) -> interface { uv: vec2f, position: vec4f, normal: vec3f } {
				let position: vec4f = vec4f(f32(input.vertex_index), 0.0, 0.0, 1.0);
				let uv: vec2f = vec2f(0.0, 0.0);
				let normal: vec3f = vec3f(0.0, 0.0, 1.0);
				return { uv, position, normal };
			}
			"#,
			&ShaderGenerationSettings::vertex(),
		);
		let pixel = lower_fixture(
			r#"
			main: fn (input: interface { uv: vec2f, normal: vec3f }) -> output { color: vec4f } {
				let color: vec4f = vec4f(input.uv.x, input.normal.z, 0.0, 1.0);
				return { color };
			}
			"#,
			&ShaderGenerationSettings::fragment(),
		);

		// Both stages declare the position first, then the varyings by location.
		for shader in [&vertex, &pixel] {
			let position = shader
				.find(": SV_Position")
				.expect("Expected each stage to declare the position");
			let first = shader.find(": TEXCOORD0").expect("Expected each stage to declare location 0");
			let second = shader.find(": TEXCOORD1").expect("Expected each stage to declare location 1");
			assert!(position < first && first < second, "{shader}");
		}
		assert_string_contains!(pixel, "(float4 _besl_fragment_position : SV_Position,");

		compile_dxil(&vertex, "linked-vertex", ShaderTypes::Vertex);
		compile_dxil(&pixel, "linked-pixel", ShaderTypes::Fragment);
	}

	#[test]
	fn packed_integer_vector_stage_io_uses_nointerpolation_only_across_rasterization() {
		let main = generator::tests::packed_u16_stage_io();
		let vertex_shader = generate(&ShaderGenerationSettings::vertex(), &main);
		let fragment_shader = generate(&ShaderGenerationSettings::fragment(), &main);
		assert_string_contains!(vertex_shader, "uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(vertex_shader, "nointerpolation out uint16_t4 packed_output : TEXCOORD1");
		assert_string_contains!(fragment_shader, "nointerpolation uint16_t2 packed_input : TEXCOORD0");
		assert_string_contains!(fragment_shader, "out uint16_t4 packed_output : SV_Target1");
		assert_string_does_not_contain!(vertex_shader, "nointerpolation uint16_t2 packed_input");
		assert_string_does_not_contain!(fragment_shader, "nointerpolation uint16_t4 packed_output");
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
					format: Some("r32ui".to_string()),
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding("color_image", besl::BindingTypes::Image { format: None }, 1, true, false).into(),
		]);
		for (name, return_type) in [
			("guard_image_bounds", void_type),
			("image_load_u32", u32_type),
			("image_load", vec4f_type),
		] {
			let parameters = vec![parameter("image", image_type.clone()), parameter("coord", vec2u_type.clone())];
			root.add_child(besl::Node::intrinsic(name, parameters, return_type).into());
		}

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected storage-image shader source to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main);
		// A written image is a UAV, and one the shader only reads is an SRV, which binds as a sampled image.
		assert_string_contains!(
			shader,
			"static RWTexture2D<uint> image = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static Texture2D<float4> color_image = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(shader, "uint2 _besl_image_size;");
		assert_string_contains!(shader, "image.GetDimensions(_besl_image_size.x, _besl_image_size.y);");
		assert_string_contains!(shader, "if (any(coord >= _besl_image_size)) { return; }");
		assert_string_contains!(shader, "uint32_t texel=image[coord];");
		assert_string_contains!(shader, "float4 color=color_image[coord];");
		assert_string_does_not_contain!(shader, "imagecoord");
		assert_string_does_not_contain!(shader, "color_imagecoord");
		assert_string_does_not_contain!(shader, "image[coord].x");
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
		let program = besl::compile_to_besl(script, Some(root))
			.expect("Failed to compile read-write narrow-buffer BESL. The most likely cause is invalid test source.");
		let main = program.get_main().expect("Expected read-write narrow-buffer main function");
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint> bytes = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint> shorts = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(shader, "bytes[(5) / 4u] >> (((5) % 4u) * 8u)) & 0xffu");
		assert_string_contains!(shader, "shorts[(3) / 2u] >> (((3) % 2u) * 16u)) & 0xffffu");
		assert_string_contains!(shader, "uint besl_packed_index_");
		assert_string_contains!(shader, "uint besl_packed_value_");
		assert_string_contains!(shader, "InterlockedOr(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedCompareExchange(bytes[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedOr(shorts[besl_packed_index_");
		assert_string_contains!(shader, "InterlockedCompareExchange(shorts[besl_packed_index_");
		assert_string_contains!(shader, "for(;;){uint besl_packed_desired_");
		assert_eq!(
			shader.matches("=next_index();").count(),
			1,
			"Packed writes must evaluate their index expression exactly once."
		);
		let value_position = shader
			.find("uint besl_packed_value_")
			.expect("Expected packed value temporary");
		let read_position = shader.find("InterlockedOr(bytes").expect("Expected packed byte read");
		assert!(
			value_position < read_position,
			"Packed writes must evaluate a self-reading right-hand side before reading and replacing its destination word."
		);

		compile_dxil(&shader, "packed-narrow-buffer-write-regression", ShaderTypes::Compute);
	}

	/// Verifies `u8` members, and the `bool` members packed into them, live in 2-byte windows at their scalar offsets.
	#[test]
	fn byte_members_lower_to_two_byte_windows() {
		let shader = lower_fixture(
			r#"
			Record: struct { index: u32, ready: bool, visible: bool, mode: u8, layer: u16 }
			records: descriptor<{ type: Record[], binding: 0, access: read_write }>;
			push_constant: push_constant { count: u32, show: bool, level: u8 }
			main: fn () -> void {
				let local: Record = Record(1, true, false, u8(3), u16(4));
				records[0].mode = local.mode;
				records[1].ready = push_constant.show;
				records[2].mode = push_constant.level;
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		// `ready` and `visible` pack into byte 4 and `mode` is byte 5, so one window at byte 4 holds both.
		assert_string_contains!(shader, "uint16_t _besl_bytes2;");
		assert_string_does_not_contain!(shader, "uint mode;");
		assert_string_contains!(
			shader,
			"besl_value._besl_bytes2=uint16_t(((besl_argument__besl_bits1&0xFFu)<<0u)|((besl_argument_mode&0xFFu)<<8u))"
		);
		// Reads shift their byte out of the window, and writes keep the window's other byte.
		assert_string_contains!(shader, "((uint(local._besl_bytes2)>>8u)&0xFFu)");
		assert_string_contains!(
			shader,
			"records[2]._besl_bytes2=uint16_t((uint(records[2]._besl_bytes2)&255u)|((uint("
		);
		assert_string_contains!(
			shader,
			"records[1]._besl_bytes2=uint16_t((uint(records[1]._besl_bytes2)&65280u)|((uint("
		);
		// The push constant's bytes share the window after `count`.
		assert_string_contains!(shader, "((uint(push_constant._besl_bytes2)>>8u)&0xFFu)");

		compile_dxil(&shader, "byte-member-windows", ShaderTypes::Compute);
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
					format: Some("r32ui".to_string()),
				},
				2,
				true,
				false,
			)
			.into(),
		]);

		let texture_2d = root.get_child("Texture2D").expect("Expected Texture2D type");
		let vec2u_type = root.get_child("vec2u").expect("Expected vec2u type");
		root.add_child(
			besl::Node::intrinsic(
				"image_load_u32",
				vec![parameter("image", texture_2d), parameter("coord", vec2u_type)],
				u32_type.clone(),
			)
			.into(),
		);
		root.add_child(
			besl::Node::intrinsic(
				"atomic_add",
				vec![
					parameter("value", atomic_u32),
					parameter("increment", root.get_child("u32").expect("Expected u32 type")),
				],
				u32_type,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected buffer shader source to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main);
		assert_string_contains!(shader, "[numthreads(8, 8, 1)]void besl_main(");
		assert_string_contains!(shader, "uint32_t item_index=index_image[coord];");
		assert_string_contains!(
			shader,
			"static StructuredBuffer<Item> item_data = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint32_t> counter_buffer = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(shader, "uint32_t counter_index=item_data[item_index].counter_index;");
		assert_string_contains!(shader, "InterlockedAdd(counter_buffer[_besl_p0],_besl_p1,_besl_previous);");
		assert_string_contains!(shader, "_besl_atomic_0(counter_index,1);");
		assert_string_contains!(shader, "uint32_t previous_count=_besl_atomic_1(counter_index,1);");
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
				besl::BindingTypes::CombinedImageSampler {
					format: String::new(),
					sampler: None,
				},
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
				1,
				true,
				false,
			)
			.into(),
		]);

		root.add_child(
			besl::Node::intrinsic(
				"texture_lod",
				vec![parameter("texture", texture_2d), parameter("uv", vec2f)],
				vec4f,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected parameter-buffer shader source to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main);
		assert_string_contains!(
			shader,
			"struct _parameters{float4x4 inverse_view_projection;float4 sun_direction;};"
		);
		assert_string_contains!(
			shader,
			"static StructuredBuffer<_parameters> parameters = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
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
	fn push_constants_take_b0_and_set_bases_take_b1() {
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
				0,
				true,
				false,
			)
			.into(),
		]);
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected push-constant shader source to lex");
		let main = root.get_main().expect("Expected main function");
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main);
		assert_string_contains!(shader, "ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
		assert_string_contains!(shader, "cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };");
		assert_string_contains!(
			shader,
			"static StructuredBuffer<uint32_t> values = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_does_not_contain!(shader, "vk::push_constant");
	}

	#[test]
	fn test_const_variable() {
		let shader = generate(&ShaderGenerationSettings::vertex(), &generator::tests::const_variable());
		// The backend declares its own `PI`, so the user constant takes the escaped name.
		assert_string_contains!(shader, "static const float besl_PI = 3.14;");
		assert_string_contains!(shader, "void besl_main(){besl_PI;}");
	}

	#[test]
	fn else_chains_lower_to_hlsl() {
		let shader = lower_fixture(
			super::super::ELSE_CHAIN,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "if(n<1){n=2;}else if(n<4){n=3;}else{n=4;}");

		compile_dxil(&shader, "besl-else-chain", ShaderTypes::Compute);
	}

	#[test]
	fn grouped_accessor_bases_keep_their_parentheses_in_hlsl() {
		let shader = lower_fixture(
			super::super::GROUPED_ACCESSOR_BASES,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "float sum_y=(v+w).y;");
		assert_string_contains!(shader, "uint32_t picked=(values)[i];");

		compile_dxil(&shader, "besl-grouped-accessor-bases", ShaderTypes::Compute);
	}

	#[test]
	fn if_and_match_values_with_statements_lower_to_hlsl() {
		let shader = lower_fixture(
			super::super::BRANCH_VALUES,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		// `scale` ran before the `match`, so it is captured before the switch that sets the match's value.
		assert_string_contains!(shader, "_besl_value_1=scale;float _besl_value_0;switch(n){");
		assert_string_contains!(shader, "float weight=(_besl_value_1*_besl_value_0);");
		assert_string_contains!(
			shader,
			"float _besl_value_2;if(n>1){float lit=(weight*weight);_besl_value_2=lit;}else{_besl_value_2=0.0;}"
		);

		compile_dxil(&shader, "besl-branch-values", ShaderTypes::Compute);
	}

	#[test]
	fn match_lowers_to_hlsl_switch() {
		let shader = lower_fixture(
			super::super::MATCH_IN_LOOP,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(
			shader,
			"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{besl_match_break_0=true;break;break;}default:{break;}}if(besl_match_break_0){break;}}"
		);
		assert_string_contains!(shader, "switch(uint(small)){case 65535u:{n=2;break;}default:{n=3;break;}}");

		compile_dxil(&shader, "besl-match", ShaderTypes::Compute);
	}

	#[test]
	fn short_scalar_arrays_lower_to_hlsl_vectors() {
		let shader = lower_fixture(super::super::SHORT_SCALAR_ARRAYS, &ShaderGenerationSettings::vertex());
		assert_string_contains!(shader, "float3 scalar_f32()");
		assert_string_contains!(shader, "uint16_t3 scalar_u16()");
		assert_string_contains!(shader, "uint3 scalar_u32()");
		assert_string_contains!(shader, "uint3 mirror_indices(uint3 indices)");
		assert_string_contains!(shader, "float3 floats=scalar_f32();");
		assert_string_contains!(shader, "uint16_t3 shorts=scalar_u16();");
		assert_string_contains!(shader, "uint3 indices=mirror_indices(scalar_u32());");
	}

	/// Verifies value arrays declare their count after the name, including parameters, and initialize from braces.
	#[test]
	fn local_arrays_declare_in_c_position_and_initialize_from_braces() {
		let shader = lower_fixture(
			r#"
			first: fn (values: vec4f[3], count: u32) -> vec4f {
				let copy: vec4f[3] = values;
				copy[0] = copy[min(count, 2)];
				return copy[0];
			}
			main: fn () -> void {
				let positions: vec4f[3] = vec4f[3](
					vec4f(0.0, 0.0, 0.0, 1.0),
					vec4f(1.0, 0.0, 0.0, 1.0),
					vec4f(0.0, 1.0, 0.0, 1.0)
				);
				positions[0] = positions[1];
				first(positions, thread_idx());
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		assert_string_contains!(shader, "float4 positions[3]={float4(");
		assert_string_contains!(shader, "float4 first(float4 values[3],");
		assert_string_contains!(shader, "float4 copy[3]=values;");
		assert_string_does_not_contain!(shader, "float4[3]");

		compile_dxil(&shader, "local arrays", ShaderTypes::Compute);
	}

	/// Verifies an array built inside an expression, here a function argument and an assignment, comes from a
	/// constructor function, since HLSL can't build an array there.
	#[test]
	fn array_constructors_inside_expressions_call_a_constructor_function() {
		let shader = lower_fixture(
			r#"
			sum: fn (values: u32[5]) -> u32 {
				return values[0] + values[4];
			}
			main: fn () -> void {
				let total: u32 = sum(u32[5](1, 2, 3, 4, 5));
				let values: u32[5] = u32[5](0, 0, 0, 0, 0);
				values = u32[5](total, 1, 2, 3, 4);
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		assert_string_contains!(shader, "struct _besl_array_u32_5{uint32_t v[5];};");
		assert_eq!(shader.matches("struct _besl_array_u32_5").count(), 1, "{shader}");
		assert_string_contains!(shader, "uint32_t total=sum(_besl_array_u32_5_make(1,2,3,4,5).v);");
		assert_string_contains!(shader, "uint32_t values[5]={0,0,0,0,0};");
		assert_string_contains!(shader, "values=_besl_array_u32_5_make(total,1,2,3,4).v;");

		compile_dxil(&shader, "besl-array-constructors", ShaderTypes::Compute);
	}

	/// Links `source` after the ray tracing fixture declarations and lowers the program to minified HLSL for `settings`.
	fn lower_ray_tracing_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let program = super::super::link_ray_tracing_fixture(source);
		generate(settings, &program.get_main().expect("Expected the fixture's main function"))
	}

	/// Verifies a ray generation shader lowers to a `raygeneration` shader that finds hits with inline ray queries,
	/// runs closest hits through the shader binding table's callable records, and compiles.
	#[test]
	fn ray_generation_stage_lowers_to_a_dxr_raygeneration_shader() {
		let shader = lower_ray_tracing_fixture(
			super::super::RAY_GENERATION_FIXTURE,
			&ShaderGenerationSettings::ray_generation(),
		);

		assert_string_contains!(shader, "// #pragma shader_stage(raygeneration)");
		assert_string_contains!(
			shader,
			"static RaytracingAccelerationStructure scene = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		// The call struct carries the payload and the hit, and `invoke` calls the hit's record.
		assert_string_contains!(shader, "struct _besl_call{_besl_call_payload payload;Hit hit;};");
		assert_string_contains!(
			shader,
			"data.payload=_besl_widen_payload(value);data.hit=hit;CallShader(hit.record,data);return _besl_narrow_payload(data.payload);"
		);
		assert_string_contains!(shader, "RayQuery<RAY_FLAG_FORCE_OPAQUE> query;");
		assert_string_contains!(
			shader,
			"query.CommittedInstanceContributionToHitGroupIndex()+query.CommittedGeometryIndex()"
		);
		assert_string_contains!(shader, "return _besl_invoke(hit,");
		assert_string_contains!(shader, "_besl_trace_occlusion(scene,255,");
		assert_string_contains!(shader, "DispatchRaysIndex().x");
		assert_string_contains!(shader, "DispatchRaysDimensions().x");
		assert_string_contains!(shader, "[shader(\"raygeneration\")]void besl_main(){");

		compile_dxil(&shader, "besl-ray-generation", ShaderTypes::RayGen);
	}

	/// Verifies a closest hit lowers to a `callable` shader that reads the hit from its copy of the call, in `main`
	/// and in helpers, and returns the payload through the call.
	#[test]
	fn closest_hit_stage_lowers_to_a_dxr_callable_shader() {
		let shader = lower_ray_tracing_fixture(super::super::CLOSEST_HIT_FIXTURE, &ShaderGenerationSettings::closest_hit());

		assert_string_contains!(shader, "// #pragma shader_stage(callable)");
		assert_string_contains!(shader, "static _besl_call _besl_call_data;");
		assert_string_contains!(shader, "_besl_call_data.hit.besl_distance");
		assert_string_contains!(shader, "_besl_call_data.hit.barycentrics");
		assert_string_contains!(shader, "SurfaceHit _besl_closest_hit_main(SurfaceHit besl_payload)");
		assert_string_contains!(
			shader,
			"[shader(\"callable\")]void besl_main(inout _besl_call data){_besl_call_data=data;_besl_call_data.payload=_besl_widen_payload(_besl_closest_hit_main(_besl_narrow_payload(_besl_call_data.payload)));data=_besl_call_data;}"
		);
		// The prelude declares `Hit` once.
		assert_eq!(shader.matches("struct Hit{").count(), 1, "{shader}");

		compile_dxil(&shader, "besl-closest-hit", ShaderTypes::ClosestHit);
	}

	/// The payload a ray generation shader and a closest hit exchange through `CallShader` carries its 16-bit members
	/// widened to 32 bits, since DXR returns call data with 16-bit members wrong, and both shaders convert at the call.
	#[test]
	fn call_data_carries_16_bit_payload_members_widened() {
		let source = r#"
			Shaded: struct { albedo: vec3f16, front_face: u32, emission: vec3f, roughness: f16, }
			Scene: layer {
				scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
				shaded: descriptor<{ type: Shaded[], binding: 1, access: write }>;
			}
		"#;
		let ray_generation = besl::compile_to_besl(
			&format!(
				"{source}{}",
				r#"
				main: fn (input: StageInput, scene: Scene) -> void {
					let hit: Hit = intersect(scene.scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
					scene.shaded[input.launch_index.x] = invoke(hit, Shaded(vec3f16(0.0, 0.0, 0.0), 0, vec3f(0.0, 0.0, 0.0), f16(1.0)));
				}
				"#
			),
			None,
		)
		.expect("Expected the ray generation shader to link");
		let closest_hit = besl::compile_to_besl(
			&format!(
				"{source}{}",
				r#"
				main: fn (input: StageInput, payload: Shaded, scene: Scene) -> Shaded {
					let shaded: Shaded = payload;
					shaded.albedo = vec3f16(input.hit_barycentrics.x, 0.5, 0.25);
					shaded.front_face = 1;
					shaded.roughness = f16(0.5);
					return shaded;
				}
				"#
			),
			None,
		)
		.expect("Expected the closest hit to link");

		for (program, settings, name, stage) in [
			(
				ray_generation,
				ShaderGenerationSettings::ray_generation(),
				"besl-16-bit-call-ray-generation",
				ShaderTypes::RayGen,
			),
			(
				closest_hit,
				ShaderGenerationSettings::closest_hit(),
				"besl-16-bit-call-closest-hit",
				ShaderTypes::ClosestHit,
			),
		] {
			let shader = generate(&settings, &program.get_main().expect("Expected the shader's main function"));

			assert_string_contains!(
				shader,
				"struct _besl_call_payload{float3 albedo;uint32_t front_face;float3 emission;float roughness;};"
			);
			assert_string_contains!(shader, "struct _besl_call{_besl_call_payload payload;Hit hit;};");
			assert_string_contains!(shader, "value.albedo=(float16_t3)wire.albedo;");
			assert_string_contains!(shader, "value.roughness=(float16_t)wire.roughness;");

			compile_dxil(&shader, name, stage);
		}
	}

	/// Verifies a compute shader runs `intersect` and `trace_occlusion` as inline ray queries and builds a `Hit`.
	#[test]
	fn ray_queries_lower_to_inline_ray_queries_in_compute_shaders() {
		let shader = lower_ray_tracing_fixture(
			r#"
			main: fn (input: StageInput, scene: Scene) -> void {
				let origin: vec3f = vec3f(f32(input.thread_id.x), 0.0, 0.0);
				let hit: Hit = intersect(scene.scene, 1, origin, vec3f(0.0, 0.0, 1.0), 0.0, 10.0);
				let rebuilt: Hit = Hit(hit.record, hit.distance, 0, 0, 0, vec2f(0.0, 0.0), origin, vec3f(0.0, 0.0, 1.0));
				let surface: SurfaceHit = SurfaceHit(vec3f(rebuilt.barycentrics.x, 0.0, 0.0), rebuilt.distance);
				if (trace_occlusion(scene.scene, 255, origin, vec3f(0.0, 0.0, 1.0), 0.0, 10.0)) {
					surface.distance = 0.0;
				}
				scene.surfaces[input.thread_id.x] = surface;
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(64)),
		);

		assert_string_contains!(shader, "// #pragma shader_stage(compute)");
		assert_string_does_not_contain!(shader, "[shader(");
		assert_string_contains!(shader, "Hit hit=_besl_intersect(scene,1,origin,");
		assert_string_contains!(shader, "rebuilt.besl_distance");

		compile_dxil(&shader, "besl-ray-queries", ShaderTypes::Compute);
	}

	/// Compiles generated HLSL with DXC on Windows so a lowering that DXC rejects fails the test.
	fn compile_dxil(shader: &str, name: &str, stage: ShaderTypes) {
		#[cfg(target_os = "windows")]
		crate::shader::hlsl_shader_compiler::compile_hlsl_source_to_dxil(shader, name, "besl_main", stage)
			.unwrap_or_else(|error| panic!("Expected {name} HLSL to compile to DXIL. {error}"));
		#[cfg(not(target_os = "windows"))]
		let _ = (shader, name, stage);
	}

	/// Verifies `pow(2, x)` is rewritten to `exp2(x)` for full and half precision.
	#[test]
	fn power_of_two_uses_exp2() {
		let shader = lower_fixture(
			"main: fn () -> void { let full: f32 = pow(2.0, 3.0); let half: f16 = pow(f16(2.0), f16(3.0)); full; half; }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);

		assert_eq!(shader.matches("exp2(").count(), 2);
		assert_string_does_not_contain!(shader, "pow(");
	}

	/// Verifies subgroup intrinsics map onto HLSL wave operations over the group thread index.
	#[test]
	fn compute_subgroup_intrinsics_lower_to_hlsl_wave_operations() {
		let shader = lower_fixture(
			super::super::SUBGROUP_COMPUTE,
			&ShaderGenerationSettings::compute(utils::Extent::line(32)),
		);
		assert_string_contains!(shader, "WaveActiveBallot(group_thread_index<4)");
		assert_string_contains!(shader, "WaveReadLaneAt(group_thread_index,leader)");
		assert_string_contains!(shader, "WaveReadLaneAt(value, WaveGetLaneIndex() ^ mask)");
		assert_string_contains!(shader, "_besl_subgroup_shuffle_xor_f32(float(value),1)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_find_lsb(mask)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_count(remaining)");

		compile_dxil(&shader, "besl-subgroup-waves", ShaderTypes::Compute);
	}

	/// Verifies `find_lsb` lowers to `firstbitlow`, which returns `0xffffffff` for zero like the BESL contract and the other backends.
	#[test]
	fn find_lsb_lowers_to_firstbitlow() {
		let shader = lower_fixture(
			super::super::FIND_LSB,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "firstbitlow(bits)");

		compile_dxil(&shader, "besl-find-lsb", ShaderTypes::Compute);
	}

	/// Verifies specialization constants read the `SPEC_CONSTANT_<id>` defines the DX12 backend passes, with a typed
	/// default, because DX12 has no Vulkan-style specialization constants.
	#[test]
	fn specializations_lower_to_overridable_defines() {
		let shader = generate(&ShaderGenerationSettings::vertex(), &generator::tests::specializations());
		assert_string_contains!(
			shader,
			"#ifndef SPEC_CONSTANT_0\n#define SPEC_CONSTANT_0 false\n#endif\nstatic const bool enabled=SPEC_CONSTANT_0;"
		);
		assert_string_contains!(
			shader,
			"#define SPEC_CONSTANT_1 0u\n#endif\nstatic const uint32_t count=SPEC_CONSTANT_1;"
		);
		assert_string_contains!(
			shader,
			"#define SPEC_CONSTANT_2 0.0\n#endif\nstatic const float scale=SPEC_CONSTANT_2;"
		);
		assert_string_contains!(shader, "static const float color_x=SPEC_CONSTANT_4;");
		assert_string_contains!(shader, "static const float color_z=SPEC_CONSTANT_6;");
		assert_string_contains!(shader, "static const float3 color=float3(color_x,color_y,color_z);");
		assert_string_does_not_contain!(shader, "vk::constant_id");
	}

	/// Verifies `fetch` reads an exact texel through `Load` with an explicit mip level instead of sampling.
	#[test]
	fn fetch_intrinsic_lowers_to_hlsl() {
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&generator::tests::texel_fetch(),
		);
		assert_string_contains!(shader, "float4 texel=besl_texture.Load(int3(coord, 0));");

		compile_dxil(&shader, "besl-fetch", ShaderTypes::Compute);
	}

	/// Verifies compute builtins, image access, storage buffers and atomic stores use their DX12 HLSL forms.
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
			besl::Node::binding("output_image", besl::BindingTypes::Image { format: None }, 2, true, true).into(),
		]);

		root.add_child(
			besl::Node::intrinsic(
				"image_size",
				vec![parameter("image", texture_2d_type.clone())],
				vec2u_type.clone(),
			)
			.into(),
		);
		root.add_child(
			besl::Node::intrinsic(
				"write",
				vec![
					parameter("image", texture_2d_type),
					parameter("coord", vec2u_type),
					parameter("value", vec4f_type),
				],
				void_type.clone(),
			)
			.into(),
		);
		root.add_child(
			besl::Node::intrinsic(
				"atomic_store",
				vec![
					parameter("value", atomic_u32),
					parameter("stored", root.get_child("u32").expect("Expected u32 type")),
				],
				void_type,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected compute shader source to link");
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&root.get_main().expect("Expected main"),
		);
		// Read-only buffers bind as SRVs (`t`), writable ones as UAVs (`u`), and single-member buffers flatten to their element.
		assert_string_contains!(
			shader,
			"static StructuredBuffer<Item> item_data = ResourceDescriptorHeap[_besl_sets[0].x];"
		);
		assert_string_contains!(
			shader,
			"static RWStructuredBuffer<uint32_t> counter_buffer = ResourceDescriptorHeap[_besl_sets[0].x+1u];"
		);
		assert_string_contains!(shader, "uint2 extent=_besl_texture_size(output_image);");
		assert_string_contains!(shader, "float noise=frac(1.25);");
		// HLSL `*` is component-wise, so BESL matrix-vector products must use `mul`.
		assert_string_contains!(shader, "float4 projected=(mul(inverse_projection, clip_space));");
		assert_string_contains!(shader, "uint32_t item_index=item_data[0].counter_index;");
		assert_string_contains!(shader, "output_image[coord] = float4(1.0,1.0,1.0,1.0);");
		// HLSL has no atomic store, so the store is an exchange whose replaced value is discarded.
		assert_string_contains!(
			shader,
			"InterlockedExchange(counter_buffer[_besl_p0],_besl_p1,_besl_replaced)"
		);
		assert_string_contains!(shader, "_besl_atomic_0(item_index,2);");
		assert_string_does_not_contain!(shader, "fract(");
		assert_string_does_not_contain!(shader, "RWStructuredBuffer<Item> item_data");
	}

	/// Verifies compare-exchange returns the previous value through HLSL's out parameter, with one helper per call.
	#[test]
	fn atomic_compare_exchange_lowers_to_hlsl() {
		let script = r#"
		shared_keys: workgroup<atomicu32, 8>;

		main: fn () -> void {
			let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
			atomic_compare_exchange(shared_keys[thread_idx()], 7, 9);
		}
		"#;

		let shader = lower_fixture(script, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
		assert_string_contains!(
			shader,
			"uint32_t _besl_atomic_0(uint32_t _besl_p0,uint32_t _besl_p1,uint32_t _besl_p2){uint32_t _besl_previous;InterlockedCompareExchange(shared_keys[_besl_p0],_besl_p1,_besl_p2,_besl_previous);return _besl_previous;}"
		);
		assert_string_contains!(shader, "uint32_t previous=_besl_atomic_0(group_thread_index,4294967295,7);");
		assert_string_contains!(shader, "_besl_atomic_1(group_thread_index,7,9);");

		compile_dxil(&shader, "besl-atomic-compare-exchange", ShaderTypes::Compute);
	}

	/// Verifies a global scalar-array constant is `static const`, since a plain HLSL global would become a constant-buffer uniform.
	#[test]
	fn const_array_variable_lowers_to_hlsl() {
		let shader = lower_fixture(
			super::super::CONST_ARRAY,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "static const float3 WEIGHTS = float3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");

		compile_dxil(&shader, "besl-const-array", ShaderTypes::Compute);
	}

	/// Verifies BESL `mix` lowers to HLSL `lerp`, since HLSL has no `mix`.
	#[test]
	fn mix_intrinsic_lowers_to_hlsl_lerp() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = mix(0.0, 1.0, 0.5);
			value;
		}
		"#;

		let shader = lower_fixture(script, &ShaderGenerationSettings::compute(utils::Extent::line(1)));
		assert_string_contains!(shader, "float value=lerp(0.0,1.0,0.5);");
		assert_string_does_not_contain!(shader, "mix(");

		compile_dxil(&shader, "besl-mix-lerp", ShaderTypes::Compute);
	}
}

pub use Generator as HLSLTranspiler;
