use std::fmt::Write as _;

pub use Generator as MSLTranspiler;

use super::{
	IntrinsicRequirements, RAY_TRACING_STAGE_INTRINSICS, ResourceAccessorKind, any_code_node, intrinsic_requirements,
	invoke_payload, resource_accessor, runtime_buffer_element,
};

/// Names the generated BESL Metal entry point persisted with compiled shader artifacts.
pub const MSL_ENTRY_POINT: &str = "besl_main";

use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, Stages, emit_statement_block, is_integer_besl_type,
	ordered_shader_nodes,
};

mod bindings;
mod emit;
mod facade;
mod generate;
mod node_emitter;
mod raster;
mod ray_tracing;
mod reserved;

pub(crate) use bindings::*;
pub(crate) use emit::*;
pub use facade::Generator;
pub(crate) use facade::*;
pub(crate) use generate::*;
pub(crate) use raster::*;
pub(crate) use ray_tracing::*;
#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

	/// Lowers the `main` of the linked `program` to minified MSL.
	fn generate(settings: &ShaderGenerationSettings, program: &mut besl::Program) -> String {
		let main = program.get_main().expect("Expected fixture main function");
		Generator::new(program)
			.minified(true)
			.generate(settings, &main)
			.expect("Expected MSL generation")
	}

	/// Links a standalone BESL source and lowers its `main` to minified MSL.
	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let builtins = besl::Builtins::new();
		let mut program =
			besl::compile_to_besl(source, besl::Program::new(&builtins)).expect("Expected fixture source to link");
		generate(settings, &mut program)
	}

	/// Verifies each compute fixture lowers to MSL that holds every expected fragment and none of the rejected ones,
	/// and compiles with the Metal toolchain.
	#[compio::test]
	async fn compute_fixtures_lower_to_msl() {
		use super::super::*;
		// Each case is a name, a source, the fragments its MSL holds, and the fragments it must not hold.
		let cases: &[(&str, &str, &[&str], &[&str])] = &[
			(
				// Packed 48-byte matrices match the CPU stride; a native float4x3 would read with a 64-byte stride.
				"affine-matrix-array",
				r#"
				Palette: struct { matrices: mat4x3f[4] }
				palette: descriptor<{ type: Palette, binding: 0, access: read_write }>;
				main: fn (input: StageInput) -> void {
					let item: u32 = input.thread_id.x;
					let affine: mat4x3f = palette.matrices[item];
					palette.matrices[item + 1] = affine;
				}
				"#,
				&[
					"device _besl_packed_float4x3* palette",
					"_besl_load_mat4x3(set0.palette[item])",
					"_besl_store_mat4x3(set0.palette[item+1],affine)",
				],
				&[],
			),
			(
				// Metal multiplies matrices only by native vectors, so packed member reads in a matrix product are unpacked.
				"packed-vector-matrix-products",
				r#"
				Body: struct { velocity: vec3f, color: vec4f }
				Frame: struct { basis: mat4x3f, projection: mat4f }
				bodies: descriptor<{ type: Body[], binding: 0, access: read_write }>;
				frame: descriptor<{ type: Frame, binding: 1, access: read }>;
				push_constant: push_constant { offset: vec3f, scale: f32, }
				main: fn (input: StageInput) -> void {
					let item: u32 = input.thread_id.x;
					let row: vec4f = bodies[item].velocity * frame.basis;
					let offset: vec4f = push_constant.offset * frame.basis;
					let projected: vec4f = frame.projection * bodies[item].color;
					bodies[item].color = projected + row + offset * push_constant.scale;
				}
				"#,
				&[
					"float3(set0.bodies[item].velocity)*_besl_load_mat4x3(",
					"float3(push_constant.offset)*_besl_load_mat4x3(",
					"set0.frame->projection*float4(set0.bodies[item].color)",
				],
				&[],
			),
			(
				"prefix-operators-and-if-values",
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
				&[
					"((~set0.bodies[item].bits)^(flag?1:2))",
					// A nested negation keeps its parentheses, so `- -x` never prints as the decrement `--x`.
					"((-(-set0.bodies[item].velocity))-(-set0.bodies[item].velocity))",
					// A literal branch beside an f16 branch is narrowed so both branches share one type.
					"((!flag)?set0.bodies[item].weight:half(1.0))",
					// Negating or selecting packed reads keeps them packed, so a matrix product unpacks the whole operand.
					"set0.frame->projection*float4(-set0.bodies[item].color)",
					"set0.frame->projection*float4(flag?set0.bodies[item].color:set0.bodies[item].color)",
				],
				&[],
			),
			(
				// Packed members keep scalar alignment, so `uv` follows `depth` at offset 4, `normal` takes 12 bytes from
				// offset 28, and `tint` follows at offset 40, as in HLSL, GLSL scalar layout, and the CPU `ghi::pod` vectors.
				"packed-vector-members",
				r#"
				Sprite: struct { depth: f32, uv: vec2f, texel: vec2u, extent: vec2u16, scale: vec2f16, normal: vec3f, tint: vec4f, mask: vec4u }
				sprites: descriptor<{ type: Sprite[], binding: 0, access: read_write }>;
				atlas: descriptor<{ type: Texture2D, binding: 1, access: read }>;
				push_constant: push_constant { offset: f32, jitter: vec2f, }
				main: fn (input: StageInput) -> void {
					let item: u32 = input.thread_id.x;
					let color: vec4f = sample(atlas, sprites[item].uv + push_constant.jitter);
					let texel: vec4f = fetch(atlas, sprites[item].texel);
					sprites[item].uv = sprites[item].uv * push_constant.offset + vec2f(color.x, texel.y);
					sprites[item].texel.x = sprites[item].texel.x + u32(sprites[item].extent.x);
					sprites[item].depth = f32(sprites[item].scale.y) + length(sprites[item].uv) + dot(sprites[item].tint, color);
					sprites[item].tint = sprites[item].tint * f32(sprites[item].mask.w);
					sprites[item].normal = normalize(cross(sprites[item].normal, vec3f(0.0, 1.0, 0.0)));
				}
				"#,
				&[
					"struct Sprite{float depth;packed_float2 uv;packed_uint2 texel;packed_ushort2 extent;packed_half2 scale;packed_float3 normal;packed_float4 tint;packed_uint4 mask;};",
					"struct PushConstant{float offset;packed_float2 jitter;};",
				],
				&[],
			),
			(
				// `packed_float3` keeps the 12-byte CPU stride; a plain `float3` would read with a 16-byte stride.
				"scalar-runtime-array",
				SCALAR_RUNTIME_ARRAY_COMPUTE,
				&[
					"const device packed_float3* positions",
					"const device ushort* indices",
					"const device uchar* corners",
					"device uint* results",
				],
				&[],
			),
			(
				// A `StorageImageArray` is one `texture2d_array` whose `read` and `write` take the selected layer.
				"layered-storage-image",
				LAYERED_STORAGE_IMAGE_COMPUTE,
				&[
					"texture2d_array<float, access::write>",
					"texture2d_array<float, access::read>",
					"set0.source.read(gid.xy,face)",
					"set0.faces.write(texel*float(extent.x),gid.xy,face)",
					"uint2(set0.faces.get_width(),set0.faces.get_height())",
				],
				&[],
			),
			(
				// The level helper is qualified, so a parameter named `level` doesn't shadow it.
				"texture-lod-level-shadowing",
				r#"
				depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
				sample_depth: fn (uv: vec2f, level: u32) -> f32 {
					return texture_lod(depth_texture, uv, f32(level)).x;
				}
				main: fn () -> void {
					sample_depth(vec2f(0.5, 0.5), 1);
				}
				"#,
				&["metal::level(float(level))"],
				&[],
			),
			(
				"gather",
				r#"
				depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
				array_depth_texture: descriptor<{ type: Texture2DArray, binding: 1, access: read }>;
				main: fn () -> void {
					let quad: vec4f = gather(depth_texture, vec2f(0.5, 0.5));
					let layer_quad: vec4f = gather(array_depth_texture, vec2f(0.5, 0.5), 1);
					quad;
					layer_quad;
				}
				"#,
				&[
					"set0.depth_texture.gather(set0.depth_texture_sampler, ",
					"set0.array_depth_texture.gather(set0.array_depth_texture_sampler, ",
					", int2(0), component::x)",
				],
				&[],
			),
			(
				// A texture parameter arrives with its sampler, through a binding and through another parameter.
				"texture-parameter-sampler",
				r#"
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
				"#,
				&[
					"float quad_maximum(texture2d_array<float> depth_map,sampler depth_map_sampler,float2 uv,uint layer)",
					"quad_maximum(depth_map,depth_map_sampler,uv,0)",
					"depth_map.gather(depth_map_sampler, uv, layer, int2(0), component::x)",
					"first_layer_maximum(set0.array_depth_texture,set0.array_depth_texture_sampler,",
				],
				&[],
			),
			(
				"downsample-gather",
				r#"
				depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
				array_depth_texture: descriptor<{ type: Texture2DArray, binding: 1, access: read }>;
				main: fn () -> void {
					let minimum: f32 = downsample_min(depth_texture, vec2f(0.5, 0.5), 0.0);
					let maximum: f32 = downsample_max(depth_texture, vec2f(0.5, 0.5), 0.0);
					let array_maximum: f32 = downsample_max(array_depth_texture, vec2f(0.5, 0.5), 1, 0.0);
					minimum;
					maximum;
					array_maximum;
				}
				"#,
				&[
					"_besl_downsample_min(set0.depth_texture, set0.depth_texture_sampler",
					"_besl_downsample_max(set0.depth_texture, set0.depth_texture_sampler",
					".gather(texture_sampler, uv, int2(0), component::x)",
					".gather(texture_sampler, uv, layer, int2(0), component::x)",
					"texture.read(a, level).x",
				],
				&[],
			),
			(
				// A fixed sampler is a constexpr sampler that takes no argument ID, so the next binding takes the ID after
				// the texture.
				"fixed-samplers",
				r#"
				nearest_texel: sampler<{ filter: nearest, mip_filter: nearest, address: repeat }>;
				tiles: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: nearest_texel }>;
				albedo: descriptor<{ type: Texture2D, binding: 1, access: read }>;
				result: descriptor<{ type: f32[], binding: 2, access: write }>;
				main: fn () -> void {
					result[0] = texture_lod(tiles, vec2f(0.5, 0.5), 1.0).x + texture_lod(albedo, vec2f(0.5, 0.5), 0.0).x;
				}
				"#,
				&[
					"constexpr sampler _besl_tiles_sampler(coord::normalized, filter::nearest, mip_filter::nearest, address::repeat);",
					"texture2d<float> tiles [[id(0)]];texture2d<float> albedo [[id(1)]];sampler albedo_sampler [[id(2)]];",
					"set0.tiles.sample(_besl_tiles_sampler,",
					"set0.albedo.sample(set0.albedo_sampler,",
				],
				&[],
			),
			(
				// No GPU older than Apple10 reduces in a sampler, so the constexpr sampler declares none, and a nearest
				// mip filter reads the level nearest the LOD, clamped to the image.
				"reducing-samplers",
				r#"
				farthest_depth: sampler<{ mip_filter: nearest, reduction: min }>;
				nearest_occluder: sampler<{ mip_filter: nearest, reduction: max }>;
				pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: farthest_depth }>;
				occluders: descriptor<{ type: Texture2D, binding: 1, access: read, sampler: nearest_occluder }>;
				result: descriptor<{ type: f32[], binding: 2, access: write }>;
				main: fn () -> void {
					result[0] = texture_lod(pyramid, vec2f(0.5, 0.5), 2.0).x;
					result[1] = downsample_max(occluders, vec2f(0.5, 0.5), 0.0);
				}
				"#,
				&[
					"constexpr sampler _besl_pyramid_sampler(coord::normalized, filter::linear, mip_filter::nearest, address::clamp_to_edge);",
					"float4(_besl_downsample_min(set0.pyramid, _besl_pyramid_sampler, float2(0.5,0.5), 2.0), 0.0, 0.0, 1.0)",
					"_besl_downsample_max(set0.occluders, _besl_occluders_sampler,",
					"uint(lod + 0.5), texture.get_num_mip_levels() - 1",
				],
				&["reduction::"],
			),
			(
				"buffer-memory-classes",
				r#"
				DispatchValues: struct { value: u32, }
				Vertices: struct { values: u32[1024], }
				Counters: struct { values: u32[1024], }
				dispatch_values: descriptor<{ type: DispatchValues, binding: 0, access: read, memory: constant }>;
				vertices: descriptor<{ type: Vertices, binding: 1, access: read, memory: device }>;
				counters: descriptor<{ type: Counters, binding: 2, access: read_write, memory: device }>;
				main: fn () -> void {
					let index: u32 = thread_id().x;
					counters.values[index] = vertices.values[index] + dispatch_values.value;
				}
				"#,
				&[
					"constant _dispatch_values* dispatch_values [[id(0)]];",
					"const device uint* vertices [[id(1)]];",
					"device uint* counters [[id(2)]];",
				],
				&[],
			),
			(
				// Metal has neither GLSL's `float4[3]` type spelling nor its array constructor, and C arrays can't be
				// copied or passed by value, so value arrays become `metal::array`. Unsigned overloads cast their
				// arguments, because C++ spells BESL's unsigned literals as `int`.
				"local-array",
				r#"
				first: fn (values: vec4f[3], count: u32) -> vec4f {
					let copy: vec4f[3] = values;
					copy[0] = copy[min(count, 2)];
					return copy[0];
				}
				main: fn () -> void {
					let positions: vec4f[3] = vec4f[3](vec4f(0.0, 0.0, 0.0, 1.0), vec4f(1.0, 0.0, 0.0, 1.0), vec4f(0.0, 1.0, 0.0, 1.0));
					positions[0] = positions[1];
					let index: u32 = clamp(max(thread_idx(), 1), 0, 2);
					first(positions, index);
				}
				"#,
				&[
					"metal::array<float4, 3> positions=metal::array<float4, 3>{",
					"float4 first(metal::array<float4, 3> values,",
					"metal::array<float4, 3> copy=values;",
					"min(uint(count),uint(2))",
				],
				&["float4[3]"],
			),
			(
				// The helper reports no bit for zero, like the BESL contract.
				"find-lsb",
				r#"
				result: descriptor<{ type: u32, binding: 0, access: read_write }>;
				main: fn () -> void {
					let bits: u32 = 40;
					result = find_lsb(bits);
				}
				"#,
				&["value == 0u ? 0xffffffffu : ctz(value)", "_besl_find_lsb(bits)"],
				&[],
			),
			(
				// The lane id is forwarded only to helpers that use it.
				"subgroup-lane-id",
				r#"
				lane: fn () -> u32 {
					return subgroup_lane_index();
				}
				ordinary: fn () -> u32 {
					return 1;
				}
				main: fn () -> void {
					let lane_index: u32 = lane();
					if (lane_index == ordinary()) {
						lane_index;
					}
				}
				"#,
				&[
					"uint lane(uint3 gid,uint thread_index,uint3 threadgroup_position,uint simd_lane_id)",
					"lane(gid,thread_index,threadgroup_position,simd_lane_id)",
					"uint ordinary()",
					"uint simd_lane_id [[thread_index_in_simdgroup]]",
				],
				&[],
			),
			("else-chain", ELSE_CHAIN, &["if(n<1){n=2;}else if(n<4){n=3;}else{n=4;}"], &[]),
			(
				"grouped-accessor-bases",
				GROUPED_ACCESSOR_BASES,
				&["float sum_y=(v+w).y;", "uint picked=(values)[i];"],
				&[],
			),
			(
				// `scale` ran before the `match`, so it is captured before the switch that sets the match's value.
				"branch-values",
				BRANCH_VALUES,
				&[
					"_besl_value_1=scale;float _besl_value_0;switch(n){",
					"float weight=(_besl_value_1*_besl_value_0);",
					"float _besl_value_2;if(n>1){float lit=(weight*weight);_besl_value_2=lit;}else{_besl_value_2=0.0;}",
				],
				&[],
			),
			(
				"match",
				MATCH_IN_LOOP,
				&[
					"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{besl_match_break_0=true;break;break;}default:{break;}}if(besl_match_break_0){break;}}",
					"switch(uint(small)){case 65535u:{n=2;break;}default:{n=3;break;}}",
				],
				&[],
			),
			(
				"short-scalar-arrays",
				SHORT_SCALAR_ARRAYS,
				&[
					"float3 scalar_f32()",
					"ushort3 scalar_u16()",
					"uint3 scalar_u32()",
					"uint3 mirror_indices(uint3 indices)",
					"float3 floats=scalar_f32();",
					"ushort3 shorts=scalar_u16();",
					"uint3 indices=mirror_indices(scalar_u32());",
				],
				&[],
			),
			(
				"atomic-images-and-push-constants",
				r#"
				Counters: struct { values: atomicu32[8], }
				counters: descriptor<{ type: Counters, binding: 0, access: read_write }>;
				index_image: descriptor<{ type: StorageImage<r32ui>, binding: 1, access: read }>;
				shared_keys: workgroup<atomicu32, 8>;
				push_constant: push_constant { base: u32, }
				main: fn () -> void {
					let coord: vec2u = thread_id();
					let index: u32 = image_load_u32(index_image, coord) + push_constant.base;
					let old: u32 = atomic_add(counters.values[index], 1);
					let claimed: u32 = atomic_compare_exchange(counters.values[index], old, 7);
					let shared_claimed: u32 = atomic_compare_exchange(shared_keys[index % 8], 4294967295, index);
					atomic_store(counters.values[index], atomic_load(counters.values[old]));
				}
				"#,
				&[
					"device atomic_uint* counters",
					"texture2d<uint, access::read> index_image",
					"constant PushConstant& push_constant [[buffer(15)]]",
					".read(coord).x",
					"atomic_fetch_add_explicit(&",
					"_besl_atomic_compare_exchange(set0.counters[index],old,7)",
					"_besl_atomic_compare_exchange(shared_keys[index%8],4294967295,index)",
					"while (!atomic_compare_exchange_weak_explicit(&value, &expected, desired",
					"atomic_load_explicit(&",
					"atomic_store_explicit(&",
				],
				&[],
			),
			(
				// Both image atomics lower to Metal texture atomics on read-write textures.
				"image-atomics",
				IMAGE_ATOMICS,
				&[
					"texture2d<uint, access::read_write> flags",
					".atomic_fetch_or(coord,4).x",
					".atomic_fetch_max(coord,previous_flags+1).x",
					".write(uint4(previous_depth),coord)",
				],
				&[],
			),
			(
				// Integer atomics use Metal's explicit relaxed operations on the right atomic types; signed atomics stay
				// signed so `atomic_min` compares as signed integers. Half-precision math keeps half types.
				"modern-half-atomics",
				r#"
				unsigned_value: workgroup<atomicu32>;
				signed_value: workgroup<atomici32>;
				main: fn () -> void {
					let signed_one: i32 = 1;
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
					let zero: f16 = f16(0.0);
					let one: f16 = f16(1.0);
					let fused: f16 = fma(one, one, one);
					let fused_vector: vec3f16 = fma(vec3f16(one, one, one), vec3f16(one, one, one), vec3f16(one, one, one));
					if (is_nan(zero / zero) || is_infinite(one / zero) || is_finite(fused) || is_normal(fused_vector.x)) {
						atomic_store(unsigned_value, 0);
					}
				}
				"#,
				&[
					"threadgroup atomic_uint unsigned_value;",
					"threadgroup atomic_int signed_value;",
					"atomic_store_explicit(&",
					"atomic_load_explicit(&",
					"atomic_exchange_explicit(&",
					"atomic_fetch_add_explicit(&",
					"atomic_fetch_sub_explicit(&",
					"atomic_fetch_min_explicit(&",
					"atomic_fetch_max_explicit(&",
					"atomic_fetch_and_explicit(&",
					"atomic_fetch_or_explicit(&",
					"atomic_fetch_xor_explicit(&",
					"_besl_atomic_compare_exchange(",
					"memory_order_relaxed",
					"half fused=fma(",
					"half3 fused_vector=fma(",
					"isnan(",
					"isinf(",
					"isfinite(",
					"isnormal(",
				],
				&[],
			),
			(
				// Intrinsics without a same-named Metal function lower to their Metal equivalents, `round_to_i32` rounds
				// half away from zero like BESL before it converts, and `pow(2, x)` is rewritten to `exp2(x)` for full and
				// half precision.
				"intrinsics",
				r#"
				main: fn () -> void {
					let toward: vec3f = vec3f(1.0, -2.0, 0.0);
					let signs: vec3f = sign(toward);
					let gap: f32 = distance(toward, vec3f(0.0, 0.0, 0.0));
					let doubled: vec2f = exp2(vec2f(1.0, 2.0));
					let angle: f32 = radians(180.0);
					let inverse: f32 = inversesqrt(4.0);
					let trigonometry: vec2f = sincos(angle);
					let fused: vec2f = fma(vec2f(2.0, 3.0), vec2f(4.0, 5.0), vec2f(1.0, 2.0));
					let rounded: vec2i = round_to_i32(vec2f(0.0 - 1.6, 2.4));
					let full: f32 = pow(2.0, 3.0);
					let half: f16 = pow(f16(2.0), f16(3.0));
					signs; gap; doubled; inverse; trigonometry; fused; rounded; full; half;
				}
				"#,
				&[
					"float3 signs=sign(toward);",
					"float gap=distance(toward,float3(0.0,0.0,0.0));",
					"float2 doubled=exp2(float2(1.0,2.0));",
					"float angle=(180.0*(PI/180.0));",
					"rsqrt(4.0)",
					"float2 trigonometry=_besl_sincos(angle);",
					"float2 fused=fma(float2(2.0,3.0),float2(4.0,5.0),float2(1.0,2.0));",
					"int2 rounded=int2(round(float2(0.0-1.6,2.4)));",
				],
				&["pow("],
			),
			(
				// Workgroup storage is a threadgroup array, which helpers receive as a pointer after the kernel values.
				"compute-workgroup",
				r#"
				scratch: workgroup<f32, 64>;
				store_scratch: fn (value: f32) -> void {
					scratch[thread_idx()] = value;
					workgroup_barrier();
				}
				main: fn () -> void {
					store_scratch(f32(thread_idx()));
					let value: f32 = scratch[thread_idx()];
					value;
				}
				"#,
				&[
					"uint thread_index [[thread_index_in_threadgroup]]",
					"threadgroup float scratch[64];",
					"threadgroup float* scratch",
					"threadgroup_barrier(mem_flags::mem_threadgroup)",
					"store_scratch(float(thread_index),gid,thread_index,threadgroup_position,scratch)",
				],
				&[],
			),
			(
				// Subgroup intrinsics map onto Metal SIMD-group operations with the lane type Metal expects.
				"compute-subgroup",
				r#"
				scratch: workgroup<u32, 1>;
				main: fn () -> void {
					let mask: vec4u = subgroup_ballot(thread_idx() < 4);
					let leader: u32 = subgroup_ballot_find_lsb(mask);
					let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
					let neighbor: f32 = subgroup_shuffle_xor_f32(f32(value), 1);
					if (subgroup_ballot_any(mask)) {
						scratch[0] = subgroup_ballot_count(subgroup_ballot_and_not(mask, subgroup_ballot(value == 0))) + u32(neighbor);
					}
				}
				"#,
				&[
					"simd_ballot(predicate)",
					"simd_broadcast(value, ushort(source_lane))",
					"simd_shuffle_xor(value, ushort(mask))",
					"_besl_subgroup_shuffle_xor_f32(float(value),1)",
					"_besl_subgroup_ballot_find_lsb(mask)",
					"_besl_subgroup_ballot_count(",
					"threadgroup uint scratch[1]",
				],
				&[],
			),
			(
				// Stored `mat4x3f` values are packed behind native matrix expressions, which multiply with Metal's operator.
				"packed-mat4x3",
				r#"
				Transform: struct { model: mat4x3f, tag: u32, }
				Transforms: struct { values: Transform[2], direct: mat4x3f[2], }
				transforms: descriptor<{ type: Transforms, binding: 0, access: read_write, memory: device }>;
				main: fn() -> void {
					let model: mat4x3f = transforms.values[0].model;
					let position: vec3f = model * vec4f(1.0, 2.0, 3.0, 1.0);
					let local: Transform = Transform(model, 7);
					local.model = transforms.direct[0];
					let local_position: vec3f = local.model * vec4f(4.0, 5.0, 6.0, 1.0);
					transforms.values[1].model = local.model;
					transforms.direct[1] = transforms.direct[0];
					position;
					local_position;
				}
				"#,
				&[
					"struct _besl_packed_float4x3 { packed_float3 columns[4]; };",
					"struct Transform{_besl_packed_float4x3 model;uint tag;};",
					"_besl_packed_float4x3 direct[2]",
					"float4x3 model=_besl_load_mat4x3(",
					"float3 position=(model*float4(1.0,2.0,3.0,1.0));",
					"float3 local_position=(_besl_load_mat4x3(local.model)*float4(4.0,5.0,6.0,1.0));",
					"Transform local=Transform{_besl_pack_mat4x3(model),7};",
					"_besl_store_mat4x3(",
				],
				&["mul("],
			),
			(
				"const-array",
				CONST_ARRAY,
				&["constant float3 WEIGHTS = float3(0.5,0.25,0.125);", "float value=WEIGHTS[1];"],
				&[],
			),
		];

		for (name, source, present, absent) in cases {
			let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
			for needle in *present {
				assert!(
					shader.contains(needle),
					"Expected {name} MSL to contain '{needle}'. MSL: '{shader}'"
				);
			}
			for needle in *absent {
				assert!(
					!shader.contains(needle),
					"Expected {name} MSL not to contain '{needle}'. MSL: '{shader}'"
				);
			}
			compile_natively(&shader, name).await;
		}
	}

	/// Verifies each vertex or fragment fixture lowers to MSL that holds every expected fragment and none of the
	/// rejected ones, and compiles with the Metal toolchain.
	#[compio::test]
	async fn raster_fixtures_lower_to_msl() {
		use super::super::*;
		// Each case is a name, whether it is a vertex shader, a source, the fragments its MSL holds, and the fragments
		// it must not hold.
		let cases: &[(&str, bool, &str, &[&str], &[&str])] = &[
			(
				"runtime-array-texture-layer",
				false,
				RUNTIME_ARRAY_FRAGMENT,
				&[
					"const device Instance* instances [[id(2)]];",
					"texture2d_array<float> sprites [[id(0)]];",
					"Instance instance=set0.instances[",
					"set0.sprites.sample(set0.sprites_sampler, ",
					", instance.sprite_id)",
				],
				&[],
			),
			(
				"descriptor-array-intrinsics",
				false,
				DESCRIPTOR_ARRAY_FRAGMENT,
				&[
					"uint2(set0.textures[set0.items[index].slot].get_width(),set0.textures[set0.items[index].slot].get_height())",
					"set0.textures[index+1].sample(set0.textures_sampler[index+1], uv, metal::level(0.0))",
					"set0.textures[set0.items[index].slot].sample(set0.textures_sampler[set0.items[index].slot], uv)",
				],
				&[],
			),
			(
				// The primitive index arrives as a `[[primitive_id]]` entry-point parameter that code reads by its
				// stage-input symbol, instead of a `FragmentInput` attribute.
				"primitive-index",
				false,
				PRIMITIVE_INDEX_FRAGMENT,
				&[
					"uint _besl_stage_primitive_index [[primitive_id]]",
					"float(_besl_stage_primitive_index)",
				],
				&[
					"primitive_index [[user(",
					"_besl_stage_primitive_index=_besl_stage_primitive_index;",
				],
			),
			// A fragment reads `input.thread_id` as the pixel it covers.
			(
				"fragment-pixel",
				false,
				FRAGMENT_PIXEL_FRAGMENT,
				&["uint2(_besl_fragment_position.xy)"],
				&[],
			),
			(
				"structural-position",
				true,
				STRUCTURAL_POSITION_VERTEX,
				&[
					"struct VertexOutput{float4 position [[position]];float2 _besl_interface_uv",
					"float4 position=float4(float(_besl_stage_vertex_index),0.0,0.0,1.0);",
					"out.position=_besl_interface_position;",
				],
				&["_besl_interface_position [[user("],
			),
			(
				// The layer is the `[[render_target_array_index]]` member of `VertexOutput` instead of a user attribute.
				"render-target-array-index",
				true,
				LAYERED_VERTEX,
				&[
					"uint render_target_array_index [[render_target_array_index]];",
					"out.render_target_array_index=_besl_interface_render_target_array_index;",
				],
				&["_besl_interface_render_target_array_index [[user("],
			),
			(
				// The invocation indices arrive as `[[vertex_id]]` and `[[instance_id]]` parameters named by their
				// stage-input symbols, so locals and helper parameters with their names compile beside them.
				"vertex-invocation-indices",
				true,
				STAGE_INPUT_NAMESAKES_VERTEX,
				&[
					"struct VertexInput{};",
					"uint _besl_stage_vertex_index [[vertex_id]],uint _besl_stage_instance_index [[instance_id]]",
					"uint vertex_index=_besl_stage_vertex_index;",
					"uint instance_index=_besl_stage_instance_index;",
				],
				&[],
			),
		];

		for (name, vertex, source, present, absent) in cases {
			let settings = if *vertex {
				ShaderGenerationSettings::vertex()
			} else {
				ShaderGenerationSettings::fragment()
			};
			let shader = lower_fixture(source, &settings);
			for needle in *present {
				assert!(
					shader.contains(needle),
					"Expected {name} MSL to contain '{needle}'. MSL: '{shader}'"
				);
			}
			for needle in *absent {
				assert!(
					!shader.contains(needle),
					"Expected {name} MSL not to contain '{needle}'. MSL: '{shader}'"
				);
			}
			compile_natively(&shader, name).await;
		}
	}

	/// Verifies a compute shader that branches on `bool` and `u32` specializations compiles with the Metal toolchain.
	#[compio::test]
	async fn specialization_branch_compiles_natively() {
		let builtins = besl::Builtins::new();
		let mut root = besl::Program::new(&builtins);
		for (name, r#type, id) in [("transparent", "bool", 0), ("fallback", "u32", 1)] {
			let r#type = root.get_child(root.root(), r#type).unwrap();
			let name = root.intern(name);
			root.add_to_root(besl::Node::specialization(name, r#type, id));
		}
		let mut program = besl::compile_to_besl(
			r#"
			Output: struct { values: u32[1] }
			output: descriptor<{ type: Output, binding: 0, access: read_write }>;

			main: fn () -> void {
				if (transparent) {
					output.values[0] = 1;
				} else {
					output.values[0] = fallback;
				}
			}
			"#,
			root,
		)
		.expect("Expected specialization fixture source to link");
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &mut program);
		assert_string_contains!(shader, "constant bool transparent [[function_constant(0)]];");
		assert_string_contains!(shader, "constant uint fallback [[function_constant(1)]];");

		compile_natively(&shader, "besl-specialization-branch").await;
	}

	#[compio::test]
	async fn sampled_binding_array_argument_is_emitted_in_resources() {
		let shader = lower_fixture(
			"textures: descriptor<{ type: Texture2D, binding: 0, access: read, count: 4 }>; main: fn () -> void { sample(textures[0], vec2f(0.0, 0.0)); }",
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "texture2d<float> textures [[id(0)]][4];");
		assert_string_contains!(shader, "sampler textures_sampler [[id(4)]][4];");
		assert_string_contains!(shader, "set0.textures[0].sample(set0.textures_sampler[0]");

		compile_natively(&shader, "besl-fixed-slot-array").await;
	}

	#[test]
	fn names_reserved_by_msl_are_prefixed_at_declaration_and_use() {
		let source = r#"
			half: struct { float3: f32, }
			thread: descriptor<{ type: half, binding: 0, access: write }>;
			constant: descriptor<{ type: Texture2D, binding: 1, access: read }>;
			scale: fn (sampler: f32) -> half {
				let device: half = half(sampler);
				return device;
			}
			main: fn () -> void {
				let kernel: vec4f = sample(constant, vec2f(0.0, 0.0));
				thread.float3 = scale(kernel.x).float3;
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::line(1)));

		assert_string_contains!(shader, "struct besl_half{float besl_float3;};");
		assert_string_contains!(shader, "struct _thread{");
		assert_string_contains!(shader, "device _thread* besl_thread [[id(0)]];");
		assert_string_contains!(shader, "texture2d<float> besl_constant [[id(1)]];");
		assert_string_contains!(shader, "sampler besl_constant_sampler [[id(2)]];");
		assert_string_contains!(shader, "besl_half scale(float besl_sampler");
		assert_string_contains!(shader, "besl_half besl_device=besl_half{besl_sampler};");
		assert_string_contains!(shader, "return besl_device;");
		assert_string_contains!(
			shader,
			"float4 besl_kernel=set0.besl_constant.sample(set0.besl_constant_sampler, float2(0.0,0.0))"
		);
		assert_string_contains!(shader, "set0.besl_thread->besl_float3=scale(besl_kernel.x");
	}

	#[compio::test]
	async fn each_used_layout_becomes_one_argument_buffer() {
		let builtins = besl::Builtins::new();
		// The shader takes a shared set, and a set it never reaches; a layer it doesn't take forms no set.
		let mut program = besl::compile_to_besl(
			"Scene: layer { views: descriptor<{ type: f32[], binding: 0, access: read }>; shadow_map: descriptor<{ type: Texture2D, binding: 1, access: read }>; meshes: descriptor<{ type: f32[], binding: 2, access: read }>; } History: layer { history: descriptor<{ type: Texture2D, binding: 0, access: read }>; } result: descriptor<{ type: f32[], binding: 0, access: write }>; scale: fn (scene: Scene) -> f32 { return scene.meshes[0]; } main: fn (scene: Scene, history: History) -> void { result[0] = scale(scene); }",
			besl::Program::new(&builtins),
		)
		.expect("Expected the layered fixture to link");
		let shader = Generator::new(&mut program)
			.minified(true)
			.generate_program(&ShaderGenerationSettings::compute(utils::Extent::line(1)))
			.expect("Expected layered MSL generation");

		// The shared set keeps its whole layout with dense argument IDs, so every shader that lists it reads one buffer.
		assert_string_contains!(
			shader,
			"struct _set0{const device float* views [[id(0)]];texture2d<float> shadow_map [[id(1)]];sampler shadow_map_sampler [[id(2)]];const device float* meshes [[id(3)]];};"
		);
		assert_string_contains!(shader, "struct _set2{device float* result [[id(0)]];};");
		assert!(
			!shader.contains("history"),
			"an unreached layout should take no argument buffer"
		);
		// The unreached history set keeps its position, so the shader's own set binds at the third index.
		assert_string_contains!(
			shader,
			"constant _set0& set0 [[buffer(16)]],constant _set2& set2 [[buffer(18)]])"
		);
		// Helpers receive every set and read each resource through its own.
		assert_string_contains!(shader, "constant _set0& set0,constant _set2& set2){return set0.meshes[0];}");
		assert_string_contains!(shader, "set2.result[0]=scale(");

		compile_natively(&shader, "besl-layout-sets").await;
	}

	#[test]
	fn a_descriptor_set_keeps_the_bindings_its_shader_does_not_reach() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			"Payload: struct { value: f32, } Unused: struct { payload: Payload, } unused: descriptor<{ type: Unused, binding: 0, access: read, memory: constant }>; Used: struct { value: u32, } used: descriptor<{ type: Used, binding: 1, access: write, memory: device }>; main: fn () -> void { used.value = 7; }",
			besl::Program::new(&builtins),
		)
		.expect("Expected full-program resource ABI fixture to link");
		let shader = Generator::new(&mut program)
			.minified(true)
			.generate_program(&ShaderGenerationSettings::compute(utils::Extent::line(1)))
			.expect("Expected full-program MSL generation");
		assert_string_contains!(shader, "struct Payload");
		// Both descriptors belong to the shader's own layout, so the set keeps both, in slot order.
		assert_string_contains!(shader, "constant _unused* unused [[id(0)]];");
		assert_string_contains!(shader, "device _used* used [[id(1)]];");
		assert_string_contains!(shader, "set0.used->value=7;");
		assert!(!shader.contains("set0.unused"));
	}

	#[test]
	fn generating_the_same_program_twice_produces_identical_output() {
		let builtins = besl::Builtins::new();
		// Fresh node graphs get fresh addresses, which used to reorder independent declarations.
		let generate = || {
			let mut program = besl::compile_to_besl(crate::resources::mips::bc7::BC7_ENCODER, besl::Program::new(&builtins))
				.expect("Expected the BC7 kernel to parse and link");
			Generator::new(&mut program)
				.generate_program(&ShaderGenerationSettings::compute(utils::Extent::square(8)))
				.expect("Expected BC7 MSL generation")
		};
		let first = generate();
		for _ in 0..4 {
			assert_eq!(first, generate());
		}
	}

	#[test]
	fn metal_argument_ids_cannot_overflow() {
		let builtins = besl::Builtins::new();
		// Each texture takes an argument ID for itself and one for its sampler, which overflows 32 bits.
		let mut root = besl::Program::new(&builtins);
		let textures_name = root.intern("textures");
		root.add_to_root(besl::Node::binding_array(
			textures_name,
			besl::BindingTypes::CombinedImageSampler {
				format: String::new(),
				sampler: None,
			},
			0,
			true,
			false,
			u32::MAX as usize,
		));
		let mut program =
			besl::compile_to_besl("main: fn () -> void { textures; }", root).expect("Expected the oversized array to link");
		assert!(
			Generator::new(&mut program)
				.generate_program(&ShaderGenerationSettings::compute(utils::Extent::line(1)))
				.is_err(),
			"Metal argument IDs must not wrap"
		);
	}

	#[test]
	fn bindings() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::bindings(&builtins),
		);
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]];");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]];");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(2)]];");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(2)]];");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
	}

	#[compio::test]
	async fn vec4f_meshlet_record_keeps_its_52_byte_stride() {
		let builtins = besl::Builtins::new();
		let mut shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::vec4f_meshlet_binding(&builtins),
		);
		assert_string_contains!(shader, "packed_float4 center_radius;packed_float4 cone_apex_cutoff;");
		shader.push_str("\nstatic_assert(sizeof(Meshlet) == 52, \"Packed Meshlet stride must match the host\");\n");

		compile_natively(&shader, "besl-vec4f-meshlet").await;
	}

	#[test]
	fn packed_u16_storage_vectors_preserve_tight_array_and_mixed_struct_layouts() {
		let builtins = besl::Builtins::new();
		let vec2_array = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::vec2u16_array_binding(&builtins),
		);
		let mixed_vec4 = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::mixed_vec4u16_binding(&builtins),
		);
		assert_string_contains!(vec2_array, "device packed_ushort2* buff");
		assert_string_contains!(mixed_vec4, "struct _buff{packed_ushort4 value;ushort tail;};");
	}

	#[compio::test]
	async fn f16_storage_vectors_use_packed_msl_types() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::mixed_f16_storage_binding(&builtins),
		);
		assert_string_contains!(
			shader,
			"struct _buff{half scalar;packed_half2 uv;packed_half3 normal;packed_half4 color;};"
		);
		assert_string_contains!(shader, "half2(uv32)");
		assert_string_contains!(shader, "float2(uv16)");
		assert_string_contains!(shader, "half(0.5)");
		assert_string_contains!(shader, "float(weight16)");
		assert_string_contains!(shader, "half literal=half(0.25);");
		assert_string_contains!(shader, "weight16*half(2.0)");
		assert_string_contains!(shader, "uv16*half(2.0)");
		assert!(!shader.contains("struct vec2f16"));

		compile_natively(&shader, "besl-f16-storage").await;
	}

	#[test]
	fn compute_bindings_use_argument_buffers_by_default() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&mut generator::tests::bindings(&builtins),
		);
		assert_string_contains!(
			shader,
			"struct _set0{device _buff* buff [[id(0)]];texture2d<float, access::write> image [[id(1)]];texture2d<float> texture [[id(2)]];sampler texture_sampler [[id(3)]];};"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint3 gid [[thread_position_in_grid]],uint thread_index [[thread_index_in_threadgroup]],uint3 threadgroup_position [[threadgroup_position_in_grid]],constant _set0& set0 [[buffer(16)]])"
		);
		assert_string_contains!(shader, "set0.buff;set0.image;set0.texture;");
	}

	#[compio::test]
	async fn anisotropic_fixed_sampler_arrays_sample_through_one_constexpr_sampler() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut super::super::link_with_sample_grad(
				&builtins,
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

		assert_string_contains!(
			shader,
			"constexpr sampler _besl_textures_sampler(coord::normalized, filter::linear, mip_filter::linear, address::repeat, max_anisotropy(16));"
		);
		assert_string_contains!(shader, "set0.textures[2].sample(_besl_textures_sampler,");

		compile_natively(&shader, "besl-anisotropic-fixed-sampler-array").await;
	}

	/// Compiles generated MSL with the Metal toolchain on macOS so a lowering that Metal rejects fails the test.
	async fn compile_natively(shader: &str, name: &str) {
		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::MetalCompiler::new()
			.compile(shader, name)
			.await
			.unwrap_or_else(|error| panic!("Expected {name} MSL to compile natively. {error}"));
		#[cfg(not(target_os = "macos"))]
		let _ = (shader, name);
	}

	#[test]
	fn subgroup_intrinsics_are_limited_to_compute_stages() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			"main: fn () -> void { let mask: vec4u = subgroup_ballot(true); mask; }",
			besl::Program::new(&builtins),
		)
		.expect("Expected subgroup stage fixture source to link");
		let main = program.get_main().expect("Expected subgroup stage fixture main function");
		assert!(
			Generator::new(&mut program)
				.generate(&ShaderGenerationSettings::vertex(), &main)
				.is_err(),
			"Subgroup intrinsics must not lower outside compute stages"
		);
	}

	#[test]
	fn mesh_stage_consumes_the_same_authored_task_payload() {
		let shader = lower_fixture(
			super::super::MESH_PAYLOAD_FIXTURE,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);
		assert_string_contains!(shader, "struct ObjectPayload{uint visible_meshlets[32];};");
		assert_string_contains!(shader, "const object_data ObjectPayload& payload [[payload]]");
		assert_string_contains!(shader, "uint meshlet_index=payload.visible_meshlets[threadgroup_position];");
		assert_string_contains!(shader, "out_mesh.set_vertex(");
		assert_string_contains!(shader, "out_mesh.set_index(");
		assert_string_contains!(shader, "out_mesh.set_primitive(0, PrimitiveOutput{");
		assert_string_contains!(shader, ".render_target_array_index = 2");
		assert_string_contains!(shader, ".instance_index = meshlet_index");
		assert_string_contains!(shader, ".primitive_index = meshlet_index");
	}

	/// Verifies the task stage maps BESL builtins, workgroup storage, barriers and the mesh dispatch onto Metal object-stage features.
	/// Several of these mistakes still compile (for example swapped thread builtins or a dropped barrier), so the lowering is asserted as well as compiled.
	#[compio::test]
	async fn generated_task_and_mesh_payload_stages_compile_with_metal() {
		let task = lower_fixture(
			super::super::TASK_PAYLOAD_FIXTURE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);
		let mesh = lower_fixture(
			super::super::MESH_PAYLOAD_FIXTURE,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);
		// The runtime reads the threadgroup size from this marker when it builds the pipeline.
		assert_string_contains!(task, "// besl-threadgroup-size:32,1,1");
		assert_string_contains!(task, "struct ObjectPayload{uint visible_meshlets[32];};");
		assert_string_contains!(task, "[[object, max_total_threadgroups_per_mesh_grid(32)]] void besl_main(");
		assert_string_contains!(task, "uint thread_position [[thread_position_in_grid]]");
		assert_string_contains!(task, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(task, "object_data ObjectPayload& payload [[payload]]");
		assert_string_contains!(task, "mesh_grid_properties mesh_grid");
		assert_string_contains!(task, "threadgroup atomic_uint visible_count;");
		assert_string_contains!(task, "threadgroup_barrier(mem_flags::mem_threadgroup)");
		assert_string_contains!(task, "payload.visible_meshlets[payload_index]");
		assert_string_contains!(task, "mesh_grid.set_threadgroups_per_grid(uint3(");

		compile_natively(&task, "besl-task-payload-fixture").await;
		compile_natively(&mesh, "besl-mesh-payload-fixture").await;
	}

	/// Verifies per-vertex mesh outputs become interpolated `VertexOutput` attributes written with the vertex position,
	/// while per-primitive outputs stay flat `PrimitiveOutput` attributes.
	#[compio::test]
	async fn vertex_mesh_outputs_interpolate_with_the_vertex_position() {
		let mesh = lower_fixture(
			r#"
			out_primitive_index: output<u32, 1, 126>;
			out_uv: vertex_output<vec2f, 2, 64>;

			main: fn () -> void {
				let index: u32 = thread_idx();
				set_mesh_output_counts(3, 1);
				set_mesh_vertex_position(index, vec4f(f32(index), 0.0, 0.0, 1.0));
				out_uv[index] = vec2f(f32(index), 1.0);
				out_primitive_index[index] = index;
				set_mesh_triangle(index, vec3u(0, 1, 2));
			}
			"#,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);
		assert_string_contains!(
			mesh,
			"struct VertexOutput{float4 position [[position]];float2 uv [[user(locn2)]];};"
		);
		assert_string_contains!(
			mesh,
			"struct PrimitiveOutput{uint primitive_index [[flat]] [[user(locn1)]];};"
		);
		assert_string_contains!(
			mesh,
			"out_mesh.set_vertex(index, VertexOutput{.position = float4(float(index),0.0,0.0,1.0), .uv = float2(float(index),1.0)})"
		);
		assert_string_contains!(
			mesh,
			"out_mesh.set_primitive(index, PrimitiveOutput{.primitive_index = index})"
		);

		compile_natively(&mesh, "besl-mesh-vertex-output-fixture").await;
	}

	/// Verifies Metal rejects a per-vertex output written apart from its vertex position, which `set_vertex` would erase.
	#[test]
	#[should_panic(expected = "Metal mesh vertex outputs must be written next to `set_mesh_vertex_position`")]
	fn vertex_mesh_output_without_its_position_is_rejected() {
		lower_fixture(
			r#"
			out_uv: vertex_output<vec2f, 2, 64>;

			main: fn () -> void {
				let index: u32 = thread_idx();
				out_uv[index] = vec2f(0.0, 1.0);
			}
			"#,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);
	}

	/// A tile shader that shares texels through workgroup storage and writes two outputs, one of them from a helper.
	const TILE_FIXTURE_SOURCE: &str = r#"
		source: descriptor<{ type: Texture2D, binding: 0, access: read }>;
		color: output<vec4f, 0>;
		glow: output<vec4f, 1>;
		shared_texels: workgroup<vec4f, 64>;

		write_glow: fn (pixel: vec2u, value: vec4f) -> void {
			write(glow, pixel, value);
		}

		main: fn (input: StageInput) -> void {
			let extent: vec2u = input.extent;
			let uv: vec2f = vec2f(f32(input.thread_id.x), f32(input.thread_id.y)) / vec2f(f32(extent.x), f32(extent.y));
			shared_texels[input.thread_idx] = texture_lod(source, uv);
			workgroup_barrier();
			let pixel: vec2u = input.thread_id * vec2u(2, 2);
			write(color, pixel, shared_texels[input.thread_idx]);
			write(color, pixel + vec2u(1, 1), shared_texels[63 - input.thread_idx]);
			write_glow(pixel, vec4f(f32(input.thread_idx), 0.0, 0.0, 1.0));
		}
	"#;

	/// Verifies a tile shader lowers to a Metal tile function: an implicit imageblock holding one `float4` per output
	/// at its color index, compute values rebuilt from the thread's place in its tile, and output writes masked to the
	/// tile and the extent, which keep the pixel's other outputs.
	#[compio::test]
	async fn generated_tile_stage_compiles_with_metal() {
		let shader = lower_fixture(
			TILE_FIXTURE_SOURCE,
			&ShaderGenerationSettings::tile(16, utils::Extent::square(8)),
		);
		assert_string_contains!(
			shader,
			"struct _besl_tile_outputs{float4 color [[color(0)]];float4 glow [[color(1)]];}"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(imageblock<_besl_tile_outputs, imageblock_layout_implicit> tile_block"
		);
		assert_string_contains!(shader, "constant uint2& output_extent [[buffer(13)]]");
		assert_string_contains!(shader, "uint3 gid=uint3(_besl_tile_group*8u+uint2(_besl_tile_thread),0u)");
		assert_string_contains!(shader, "threadgroup float4* shared_texels [[threadgroup(0)]]");
		assert_string_contains!(shader, "_besl_offset=_besl_pixel-threadgroup_position.xy*16u");
		assert_string_contains!(shader, "_besl_outputs=tile_block.read(ushort2(_besl_offset))");
		assert_string_contains!(
			shader,
			"imageblock<_besl_tile_outputs, imageblock_layout_implicit> tile_block,uint2 output_extent)"
		);

		compile_natively(&shader, "besl-tile-fixture").await;
	}

	#[test]
	fn mesh_stage_uses_mesh_entry_point_and_mesh_push_constants() {
		let builtins = besl::Builtins::new();
		let script = r#"
		push_constant: push_constant {
			instance_index: u32,
		}
		main: fn () -> void {
			set_mesh_output_counts(3, 1);
			set_mesh_vertex_position(0, vec4f(f32(push_constant.instance_index), 0.0, 0.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;
		let mut program = besl::lex(&builtins, besl::parse(script).expect("Expected mesh shader source to parse"))
			.expect("Expected mesh shader source to lex");

		let shader = generate(
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
			&mut program,
		);
		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
		assert_string_contains!(shader, "[[mesh]] void besl_main(");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, "uint threadgroup_position [[threadgroup_position_in_grid]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(
			shader,
			"metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh"
		);
	}

	#[compio::test]
	async fn compute_stage_inputs_lower_to_msl_builtins() {
		let builtins = besl::Builtins::new();
		let source = r#"
		main: fn (input: StageInput) -> void {
			let coord: vec2u = input.thread_id;
			let local: u32 = input.thread_idx;
			let workgroup: u32 = input.threadgroup_position;
			let lane: u32 = input.subgroup_lane_index;
			coord;
			local;
			workgroup;
			lane;
		}
		"#;
		let root = besl::parse(source).unwrap();
		let mut program = besl::lex(&builtins, root).unwrap();

		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(128)), &mut program);
		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
		assert_string_contains!(shader, "uint3 gid [[thread_position_in_grid]]");
		assert_string_contains!(shader, "uint simd_lane_id [[thread_index_in_simdgroup]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "uint3 threadgroup_position [[threadgroup_position_in_grid]]");

		compile_natively(&shader, "besl-compute-stage-inputs").await;
	}

	/// Verifies each specialization becomes a function constant at its declared id, with vector components after it.
	#[test]
	fn specializations_lower_to_function_constants() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::specializations(&builtins),
		);
		assert_string_contains!(shader, "constant bool enabled [[function_constant(0)]];");
		assert_string_contains!(shader, "constant uint count [[function_constant(1)]];");
		assert_string_contains!(shader, "constant float scale [[function_constant(2)]];");
		assert_string_contains!(shader, "constant float color_x [[function_constant(4)]];");
		assert_string_contains!(shader, "constant float color_y [[function_constant(5)]];");
		assert_string_contains!(shader, "constant float color_z [[function_constant(6)]];");
		assert_string_contains!(shader, "constant float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void main(){enabled;count;scale;color;}");
	}

	#[test]
	fn fragment_builtin_stage_io_lowers_to_msl_semantics() {
		let shader = lower_fixture(
			"front_facing: input<bool, 0>; depth: output<f32, 0>; stencil: output<u32, 1>; sample_mask: output<u32, 2>; main: fn () -> void { front_facing; depth; stencil; sample_mask; }",
			&ShaderGenerationSettings::fragment(),
		);
		assert_string_contains!(shader, "struct FragmentInput{};");
		assert_string_contains!(shader, "float depth [[depth(any)]];");
		assert_string_contains!(shader, "uint stencil [[stencil]];");
		assert_string_contains!(shader, "uint sample_mask [[sample_mask]];");
		assert_string_contains!(shader, "bool front_facing [[front_facing]]");
		assert!(!shader.contains("bool front_facing=front_facing;"));
	}

	#[test]
	fn vertex_shader_generates_msl_entry_point() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			r#"
			Camera: struct { view_projection: mat4f, }
			Cameras: struct { cameras: Camera[8], }
			cameras: descriptor<{ type: Cameras, binding: 0, access: read, memory: constant }>;
			in_position: input<vec3f, 0>;
			position: output<vec4f, 255>;
			out_instance_index: output<u32, 0>;
			main: fn () -> void {
				position = cameras.cameras[0].view_projection * vec4f(in_position.x, in_position.y, in_position.z, 1.0);
				out_instance_index = 0;
			}
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the vertex shader to lex");
		let shader = generate(&ShaderGenerationSettings::vertex(), &mut program);
		assert_string_contains!(shader, "struct _set0{constant Camera* cameras [[id(0)]];};");
		assert_string_contains!(shader, "struct VertexInput{float3 in_position [[attribute(0)]];};");
		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];uint out_instance_index [[flat]] [[user(locn0)]];};"
		);
		assert_string_contains!(
			shader,
			"vertex VertexOutput besl_main(VertexInput in [[stage_in]],constant _set0& set0 [[buffer(16)]])"
		);
		assert_string_contains!(
			shader,
			"position=(set0.cameras[0].view_projection*float4(in_position.x,in_position.y,in_position.z,1.0))"
		);
		assert_string_contains!(shader, "return out;");
	}

	/// Verifies raster helpers retain binding access when lowered outside the Metal entry point.
	#[test]
	fn raster_helpers_receive_argument_buffer_context() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			r#"
			Camera: struct { view_projection: mat4f, }
			Cameras: struct { cameras: Camera[1], }
			cameras: descriptor<{ type: Cameras, binding: 0, access: read, memory: constant }>;
			in_position: input<vec3f, 0>;
			position: output<vec4f, 0>;
			camera_matrix: fn () -> mat4f {
				return cameras.cameras[0].view_projection;
			}
			main: fn () -> void {
				position = camera_matrix() * vec4f(in_position.x, in_position.y, in_position.z, 1.0);
			}
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Failed to compile the raster helper fixture. The most likely cause is invalid BESL syntax.");
		let shader = generate(&ShaderGenerationSettings::vertex(), &mut program);
		assert_string_contains!(shader, "float4x4 camera_matrix(constant _set0& set0);");
		assert_string_contains!(
			shader,
			"float4x4 camera_matrix(constant _set0& set0){return set0.cameras[0].view_projection;}"
		);
		assert_string_contains!(
			shader,
			"position=(camera_matrix(set0)*float4(in_position.x,in_position.y,in_position.z,1.0));"
		);
	}

	#[test]
	fn push_constant() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::push_constant(&builtins),
		);
		assert_string_contains!(shader, "struct PushConstant{uint material_id;};");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]];");
		assert_string_contains!(shader, "void main(){push_constant;}");
	}

	#[test]
	fn matrix_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f, position: vec4f) -> vec4f {
			return projection * model * position;
		}
		"#;

		let shader = lower_fixture(script, &ShaderGenerationSettings::vertex());
		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4x4 model,float4 position){return (projection*model)*position;}"
		);
	}

	#[test]
	fn test_const_variable() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::const_variable(&builtins),
		);
		// The backend declares its own `PI`, so a BESL constant with that name must be prefixed.
		assert_string_contains!(shader, "constant float besl_PI = 3.14;");
		assert_string_contains!(shader, "void main(){besl_PI;}");
	}

	#[test]
	fn mesh_intrinsics_emit_msl_mesh_commands() {
		let builtins = besl::Builtins::new();
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;

		let root = besl::parse(script).expect("Expected mesh shader source to parse");
		let mut program = besl::lex(&builtins, root).expect("Expected mesh shader source to lex");

		let shader = generate(
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
			&mut program,
		);
		assert_string_contains!(shader, "if(thread_index==0){out_mesh.set_primitive_count(2);}");
		assert_string_contains!(
			shader,
			"out_mesh.set_vertex(0, VertexOutput{.position = float4(1.0,2.0,3.0,1.0)})"
		);
		assert_string_contains!(shader, "uint _besl_triangle_index=0;uint3 _besl_triangle=uint3(0,1,2)");
		assert_string_contains!(
			shader,
			"out_mesh.set_index(_besl_triangle_index*3+0,_besl_triangle.x);out_mesh.set_index(_besl_triangle_index*3+1,_besl_triangle.y);out_mesh.set_index(_besl_triangle_index*3+2,_besl_triangle.z)"
		);
		assert_eq!(
			shader.matches("uint3(0,1,2)").count(),
			1,
			"Mesh triangle vectors must be evaluated once: {shader}"
		);
	}

	/// Verifies raster entry points and called helpers receive source-declared push constants.
	#[compio::test]
	async fn source_declared_raster_push_constants_are_entry_point_parameters() {
		let source = r#"
			push_constant: push_constant {
				transform: mat4f,
				color: vec4f,
			}
			in_position: input<vec3f, 0>;
			transform_position: fn (position: vec3f) -> vec4f {
				return push_constant.transform * vec4f(position.x, position.y, position.z, 1.0);
			}
			main: fn () -> interface { position: vec4f, color: vec4f } {
				return {
					position: transform_position(in_position),
					color: push_constant.color,
				};
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::vertex());

		assert_string_contains!(shader, "struct PushConstant{");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");

		compile_natively(&shader, "besl-raster-push-constant").await;
	}

	/// Verifies `fetch` reads an exact texel with `read` instead of filtering through the sampler.
	#[compio::test]
	async fn fetch_intrinsic_lowers_to_msl() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&mut generator::tests::texel_fetch(&builtins),
		);
		assert_string_contains!(shader, "float4 texel=set0.texture.read(coord);");

		compile_natively(&shader, "besl-fetch").await;
	}

	/// Verifies a fragment `sample` pairs the texture with its generated sampler.
	#[compio::test]
	async fn sample_intrinsic_lowers_to_a_texture_sample_call() {
		let source = r#"
			image_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
			in_uv: input<vec2f, 0>;
			out_color_attachment: output<vec4f, 0>;
			main: fn() -> void {
				out_color_attachment = sample(image_texture, in_uv);
			}
		"#;
		let shader = lower_fixture(source, &ShaderGenerationSettings::fragment());
		assert_string_contains!(shader, "set0.image_texture.sample(set0.image_texture_sampler, in_uv)");

		compile_natively(&shader, "besl-sample-intrinsic").await;
	}

	/// Stacks [`super::super::RAY_TRACING_LAYERS`] under `source` and lowers the program to minified MSL for `settings`.
	fn lower_ray_tracing_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let builtins = besl::Builtins::new();
		let mut program = super::super::link_ray_tracing_fixture(&builtins, source);
		Generator::new(&mut program)
			.minified(true)
			.generate_program(settings)
			.expect("Expected ray tracing MSL generation")
	}

	/// Verifies a ray generation shader lowers to a kernel that runs closest hits through its shader binding table and
	/// passes them every set and the push constant its layers declare, and that the kernel compiles.
	#[compio::test]
	async fn generated_ray_generation_stage_compiles_with_metal() {
		let shader = lower_ray_tracing_fixture(
			super::super::RAY_GENERATION_FIXTURE,
			&ShaderGenerationSettings::ray_generation(),
		);

		assert_string_contains!(shader, "#include <metal_raytracing>");
		// The kernel takes the materials set though it reads none of it, because closest hits read it.
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint3 _besl_launch_index [[thread_position_in_grid]],uint3 _besl_launch_size [[threads_per_grid]],visible_function_table<_besl_closest_hit> _besl_hits [[buffer(14)]],constant PushConstant& push_constant [[buffer(15)]],constant _set0& set0 [[buffer(16)]],constant _set1& set1 [[buffer(17)]]){"
		);
		assert_string_contains!(
			shader,
			"const _besl_ray_context _besl_context={&push_constant,{&set0,&set1}};"
		);
		// The helper that calls `invoke` receives the table and the addresses closest hits read.
		assert_string_contains!(
			shader,
			"SurfaceHit shade(Hit hit,uint3 _besl_launch_index,uint3 _besl_launch_size,visible_function_table<_besl_closest_hit> _besl_hits,thread const _besl_ray_context& _besl_context,constant PushConstant& push_constant,constant _set0& set0,constant _set1& set1)"
		);
		assert_string_contains!(shader, "return _besl_invoke(_besl_hits,_besl_context,hit,SurfaceHit{");
		assert_string_contains!(shader, "_besl_trace_occlusion(set0.scene,255,");

		compile_natively(&shader, "besl-ray-generation").await;
	}

	/// Verifies a closest hit lowers to a visible function that reads its sets and push constant through the addresses
	/// the kernel passes, forwards the hit to its helpers, and compiles.
	#[compio::test]
	async fn generated_closest_hit_stage_compiles_with_metal() {
		let shader = lower_ray_tracing_fixture(super::super::CLOSEST_HIT_FIXTURE, &ShaderGenerationSettings::closest_hit());

		assert_string_contains!(
			shader,
			"[[visible]] void besl_main(thread SurfaceHit& _besl_payload,thread const Hit& _besl_hit,thread const _besl_ray_context& _besl_context){"
		);
		assert_string_contains!(
			shader,
			"constant _set1& set1=*static_cast<constant _set1*>(_besl_context.sets[1]);"
		);
		assert_string_contains!(
			shader,
			"_besl_payload=_besl_closest_hit_main(_besl_payload,_besl_hit,push_constant,set0,set1);"
		);
		// Hit values read the hit's members, and packed vectors unpack to the native vectors stage values are.
		assert_string_contains!(shader, "float2 weights=float2(_besl_hit.barycentrics);");
		assert_string_contains!(
			shader,
			"(_besl_hit.besl_distance*push_constant.scale)+float3(_besl_hit.direction).z"
		);
		assert_string_contains!(
			shader,
			"albedo_of(_besl_hit.geometry_index,_besl_hit,push_constant,set0,set1)"
		);
		assert!(
			!shader.contains("kernel "),
			"a closest hit should lower to a visible function, not a kernel"
		);

		compile_natively(&shader, "besl-closest-hit").await;
	}

	/// Verifies compute shaders run ray queries and rebuild hits, and that the result compiles.
	#[compio::test]
	async fn ray_queries_compile_in_compute_shaders() {
		let shader = lower_fixture(
			r#"
			scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
			hits: descriptor<{ type: Hit[], binding: 1, access: write }>;
			occluded: descriptor<{ type: u32[], binding: 2, access: write }>;

			main: fn (input: StageInput) -> void {
				let item: u32 = input.thread_id.x;
				let origin: vec3f = vec3f(f32(item), 0.0, 0.0);
				let hit: Hit = intersect(scene, 1, origin, vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
				hits[item] = Hit(hit.record, hit.distance, hit.custom_index, hit.geometry_index, hit.primitive_index, hit.barycentrics, hit.origin, hit.direction);
				occluded[item] = 0;
				if (trace_occlusion(scene, 255, origin, vec3f(0.0, 0.0, 1.0), 0.0, hit.distance)) {
					occluded[item] = 1;
				}
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(64)),
		);

		assert_string_contains!(
			shader,
			"struct Hit{uint record;float besl_distance;uint custom_index;uint geometry_index;uint primitive_index;packed_float2 barycentrics;packed_float3 origin;packed_float3 direction;};"
		);
		assert_string_contains!(shader, "Hit hit=_besl_intersect(set0.scene,1,origin,");
		assert_string_contains!(shader, "set0.hits[item]=Hit{hit.record,hit.besl_distance,");

		compile_natively(&shader, "besl-compute-ray-queries").await;
	}

	/// Verifies a fragment entry that returns an authored output struct returns that struct from the Metal entry point.
	#[compio::test]
	async fn fragment_explicit_output_struct_return_lowers_to_msl_entry_return() {
		let script = r#"
		FragmentOutput: struct {
			color: vec4f,
		}

		main: fn () -> FragmentOutput {
			return FragmentOutput(vec4f(1.0, 0.0, 0.0, 1.0));
		}
		"#;
		let shader = lower_fixture(script, &ShaderGenerationSettings::fragment());
		assert_string_contains!(
			shader,
			"fragment FragmentOutput besl_main(FragmentInput in [[stage_in]],float4 _besl_fragment_position [[position]])"
		);
		// Metal rejects packed vectors in stage interfaces, so the returned struct keeps native members.
		assert_string_contains!(shader, "struct FragmentOutput{float4 color;};");
		assert_string_contains!(shader, "return FragmentOutput{float4(1.0,0.0,0.0,1.0)};");

		compile_natively(&shader, "besl-explicit-fragment-output").await;
	}
}
