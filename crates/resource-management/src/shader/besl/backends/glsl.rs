mod generator;
mod reserved;

pub use Generator as GLSLTranspiler;
pub use generator::Generator;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

	/// Lowers the `main` of the linked `program` to minified GLSL.
	fn generate(settings: &ShaderGenerationSettings, program: &mut besl::Program) -> String {
		let main = program.get_main().expect("Expected fixture main function");
		Generator::new(program)
			.minified(true)
			.generate(settings, &main)
			.expect("Expected GLSL generation")
	}

	/// Links a standalone BESL source and lowers its `main` to minified GLSL.
	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let builtins = besl::Builtins::new();
		let mut program =
			besl::compile_to_besl(source, besl::Program::new(&builtins)).expect("Expected fixture source to link");
		generate(settings, &mut program)
	}

	/// Verifies each compute fixture lowers to GLSL that holds every expected fragment and none of the rejected ones,
	/// and compiles with glslang.
	#[test]
	fn compute_fixtures_lower_to_glsl() {
		use super::super::*;
		let sets = "push_constant._besl_sets[_besl_set0]";
		let body = "besl_binding_bodies.bodies[item]";
		let ids = format!("besl_binding_ids[{sets}.x+0u]");
		// Each case is a name, a source, the fragments its GLSL holds, and the fragments it must not hold.
		let cases: Vec<(&str, &str, Vec<String>, &[&str])> = vec![
			(
				// Each of these intrinsics is a native GLSL overload, and `pow(2, x)` is rewritten to `exp2(x)` for full
				// and half precision.
				"vector-intrinsics",
				r#"
				main: fn () -> void {
					let toward: vec3f = vec3f(1.0, -2.0, 0.0);
					let signs: vec3f = sign(toward);
					let gap: f32 = distance(toward, vec3f(0.0, 0.0, 0.0));
					let doubled: vec2f = exp2(vec2f(1.0, 2.0));
					let full: f32 = pow(2.0, 3.0);
					let half: f16 = pow(f16(2.0), f16(3.0));
					signs; gap; doubled; full; half;
				}
				"#,
				vec![
					"vec3 signs=sign(toward);".into(),
					"float gap=distance(toward,vec3(0.0,0.0,0.0));".into(),
					"vec2 doubled=exp2(vec2(1.0,2.0));".into(),
				],
				&["pow("],
			),
			(
				// A binding keeps its own name in GLSL, which lets a local hide a global of the same name.
				"local-named-like-binding",
				r#"
				Scene: layer { albedo: descriptor<{ type: Texture2D, binding: 0, access: read }>; }
				result: descriptor<{ type: StorageImage<rgba16f>, binding: 0, access: write }>;
				main: fn (input: StageInput, scene: Scene) -> void {
					let albedo: vec4f = fetch(scene.albedo, input.thread_id);
					write(result, input.thread_id, albedo * fetch(scene.albedo, input.thread_id).w);
				}
				"#,
				vec![],
				&[],
			),
			(
				"scalar-runtime-arrays",
				SCALAR_RUNTIME_ARRAY_COMPUTE,
				vec![
					"layout(set=0,binding=0,scalar) readonly buffer _positions{vec3 positions[];}besl_binding_positions;".into(),
					"layout(set=0,binding=1,scalar) readonly buffer _indices{uint16_t indices[];}besl_binding_indices;".into(),
					"layout(set=0,binding=2,scalar) readonly buffer _corners{uint8_t corners[];}besl_binding_corners;".into(),
					"layout(set=0,binding=3,scalar) writeonly buffer _results{uint32_t results[];}besl_binding_results;".into(),
				],
				&[],
			),
			(
				// `gather` reads the texel quad, and a 16-bit unorm storage image takes the `r16` layout qualifier.
				"gather-and-unorm-images",
				r#"
				depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
				array_depth_texture: descriptor<{ type: Texture2DArray, binding: 1, access: read }>;
				cells: descriptor<{ type: StorageImage<r16>, binding: 2, access: write }>;
				main: fn () -> void {
					let quad: vec4f = gather(depth_texture, vec2f(0.5, 0.5));
					let layer_quad: vec4f = gather(array_depth_texture, vec2f(0.5, 0.5), 1);
					write(cells, vec2u(0, 0), quad + layer_quad);
				}
				"#,
				vec![
					format!("textureGather(sampler2D(besl_binding_depth_texture[{sets}.x+0u],_besl_samplers[{sets}.y+0u]),"),
					format!(
						"textureGather(sampler2DArray(besl_binding_array_depth_texture[{sets}.x+1u],_besl_samplers[{sets}.y+1u]),vec3("
					),
					"layout(descriptor_heap,descriptor_stride=64,r16) writeonly uniform image2D besl_binding_cells[]".into(),
				],
				&[],
			),
			(
				// An image the shader only reads becomes a texture without a sampler, whose loads and size read it as one,
				// alone and in an array. A written image stays a storage image, without a format when it declares none.
				"read-only-storage-images",
				r#"
				ids: descriptor<{ type: StorageImage<r32ui>, binding: 0, access: read }>;
				layers: descriptor<{ type: StorageImage, binding: 1, access: read, count: 2 }>;
				result: descriptor<{ type: StorageImage, binding: 2, access: write }>;
				main: fn () -> void {
					let coord: vec2u = vec2u(1, 2);
					guard_image_bounds(ids, coord);
					let extent: vec2u = image_size(ids);
					let id: u32 = image_load_u32(ids, coord);
					let color: vec4f = image_load(layers[id % 2], coord);
					write(result, coord, color * f32(extent.x));
				}
				"#,
				vec![
					"layout(descriptor_heap,descriptor_stride=64) uniform utexture2D besl_binding_ids[];".into(),
					"layout(descriptor_heap,descriptor_stride=64) uniform texture2D besl_binding_layers[];".into(),
					"layout(descriptor_heap,descriptor_stride=64) writeonly uniform image2D besl_binding_result[];".into(),
					format!(".x>=uint(textureSize({ids},0).x)"),
					format!("uvec2(textureSize({ids},0))"),
					format!("texelFetch({ids},ivec2(coord),0).x"),
					// The array holds two descriptors, so the image after it is the set's fourth.
					format!("texelFetch(besl_binding_layers[nonuniformEXT({sets}.x+1u+(id%2))],"),
					format!("imageStore(besl_binding_result[{sets}.x+3u],"),
				],
				&[],
			),
			(
				"reserved-names",
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
				vec![
					"buffer _buffer{float besl_half;uint32_t besl_output;}besl_binding_buffer;".into(),
					"besl_binding_buffer.besl_half=besl_float;".into(),
					"float besl_texture(float besl_input,float besl_besl_float)".into(),
					"float besl_min=min(besl_input,besl_besl_float);".into(),
					"struct besl_sampler{float besl_half;uint32_t besl_output;};".into(),
					"struct Wrapper{besl_sampler value;};".into(),
					"float besl_float=besl_texture(wrapper.value.besl_half,2.0);".into(),
				],
				&[],
			),
			(
				// A layer's descriptors and the shader's own read their own sets' base indices, each found through the
				// specialization constant of its set position, push constants start after the base indices, and a
				// fixed sampler sits at the sampler heap index its state decides: nearest, nearest, average, repeat.
				"layered-descriptors",
				r#"
				nearest_texel: sampler<{ filter: nearest, mip_filter: nearest, address: repeat }>;
				Scene: layer {
					positions: descriptor<{ type: vec2f[], binding: 0, access: read }>;
					albedo: descriptor<{ type: Texture2D, binding: 1, access: read }>;
					tiles: descriptor<{ type: Texture2D, binding: 2, access: read, sampler: nearest_texel }>;
					constants: push_constant { scale: f32 };
				}
				result: descriptor<{ type: StorageImage<rgba16f>, binding: 0, access: write }>;
				main: fn (input: StageInput, scene: Scene) -> void {
					let uv: vec2f = scene.positions[input.thread_id.x];
					write(result, input.thread_id, sample(scene.albedo, uv) * sample(scene.tiles, uv) * scene.constants.scale);
				}
				"#,
				vec![
					"layout(push_constant,scalar)uniform PushConstant{uvec2 _besl_sets[15];layout(offset=128)float scale;}push_constant;".into(),
					"layout(constant_id=1984)const uint _besl_set0=0u;".into(),
					"layout(constant_id=1985)const uint _besl_set1=1u;".into(),
					format!("texture(sampler2D(besl_binding_albedo[{sets}.x+1u],_besl_samplers[{sets}.y+0u]),uv)"),
					format!("texture(sampler2D(besl_binding_tiles[{sets}.x+2u],_besl_samplers[12u]),uv)"),
					"imageStore(besl_binding_result[push_constant._besl_sets[_besl_set1].x+0u],".into(),
				],
				&[],
			),
			(
				// A texture parameter takes the heap indices of its texture and sampler, because glslang can't pass a heap
				// alias's element to a function, and functions pass both on and read them through heap aliases of the type.
				"texture-parameter",
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
				vec![
					"layout(descriptor_heap,descriptor_stride=64) uniform texture2DArray _besl_texture2DArray[];".into(),
					"float quad_maximum(uint depth_map,uint depth_map_sampler,vec2 uv,uint32_t layer)".into(),
					"textureGather(sampler2DArray(_besl_texture2DArray[nonuniformEXT(depth_map)],_besl_samplers[nonuniformEXT(depth_map_sampler)]),vec3(uv,float(layer)))".into(),
					"return quad_maximum(depth_map,depth_map_sampler,uv,0);".into(),
					format!("first_layer_maximum({sets}.x+0u,{sets}.y+0u,vec2("),
				],
				&[],
			),
			(
				"subgroup-compute",
				SUBGROUP_COMPUTE,
				vec![
					"#extension GL_KHR_shader_subgroup_basic:require".into(),
					"#extension GL_KHR_shader_subgroup_ballot:require".into(),
					"#extension GL_KHR_shader_subgroup_shuffle:require".into(),
					"subgroupShuffleXor(float(value),1)".into(),
					"subgroupBallot(uint(gl_LocalInvocationIndex)<4)".into(),
					"subgroupBroadcast(uint(gl_LocalInvocationIndex),leader)".into(),
					"subgroupBallotFindLSB(mask)".into(),
					"subgroupBallotBitCount(remaining)".into(),
				],
				&[],
			),
			(
				// GLSL converts no literal to a narrow scalar implicitly, so a 16-bit constant constructs its value.
				"narrow-constants",
				r#"
				Result: struct { value: f32 }
				result: descriptor<{ type: Result, binding: 0, access: write }>;
				SCALE: const f16 = 0.8281;
				main: fn () -> void {
					result.value = f32(SCALE);
				}
				"#,
				vec!["float16_t(0.8281)".into()],
				&[],
			),
			("else-chain", ELSE_CHAIN, vec!["if(n<1){n=2;}else if(n<4){n=3;}else{n=4;}".into()], &[]),
			(
				"grouped-accessor-bases",
				GROUPED_ACCESSOR_BASES,
				vec!["float sum_y=(v+w).y;".into(), "uint32_t picked=(values)[i];".into()],
				&[],
			),
			(
				// `scale` ran before the `match`, so it is captured before the switch that sets the match's value.
				"branch-values",
				BRANCH_VALUES,
				vec![
					"_besl_value_1=scale;float _besl_value_0;switch(n){".into(),
					"float weight=(_besl_value_1*_besl_value_0);".into(),
					"float _besl_value_2;if(n>1){float lit=(weight*weight);_besl_value_2=lit;}else{_besl_value_2=0.0;}".into(),
				],
				&[],
			),
			(
				// The inner `break` sets both flags, so it leaves both switches and then the loop.
				"match",
				r#"
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
				"#,
				vec![
					"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{{bool besl_match_break_1=false;switch(uint(flag)){case 1u:{besl_match_break_1=true;break;break;}default:{continue;break;}}if(besl_match_break_1){besl_match_break_0=true;break;}};break;}default:{break;}}if(besl_match_break_0){break;}}".into(),
					"switch(signed){case (-2147483647-1):{n=2;break;}default:{n=3;break;}}".into(),
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
				vec![
					format!("((~{body}.bits)^(flag?1:2))"),
					// A nested negation keeps its parentheses, so `- -x` never prints as the decrement `--x`.
					format!("((-(-{body}.velocity))-(-{body}.velocity))"),
					// GLSL does not narrow float literals to float16_t, so a literal branch beside an f16 branch is cast.
					format!("((!flag)?{body}.weight:float16_t(1.0))"),
				],
				&[],
			),
			(
				"image-atomics",
				IMAGE_ATOMICS,
				vec!["imageAtomicOr(".into(), "imageAtomicMax(".into(), "uvec4(previous_depth))".into()],
				&[],
			),
			(
				// A read-only `StorageImageArray` is one `texture2DArray` and a written one an `image2DArray`, whose loads,
				// stores, and size take the selected layer.
				"layered-storage-image",
				LAYERED_STORAGE_IMAGE_COMPUTE,
				vec![
					"uniform image2DArray".into(),
					"uniform texture2DArray".into(),
					format!(
						"texelFetch(besl_binding_source[{sets}.x+0u],ivec3(ivec2(uvec2(gl_GlobalInvocationID.xy)),int(face)),0)"
					),
					format!("imageStore(besl_binding_faces[{sets}.x+1u],ivec3(ivec2(uvec2(gl_GlobalInvocationID.xy)),int(face)),"),
					format!("uvec2(imageSize(besl_binding_faces[{sets}.x+1u]).xy)"),
				],
				&[],
			),
			// GLSL's `findLSB` is signed, so the conversion makes zero yield `0xffffffff` like the BESL contract.
			("find-lsb", FIND_LSB, vec!["uint(findLSB(bits))".into()], &[]),
			(
				"atomic-compare-exchange",
				r#"
				shared_keys: workgroup<atomicu32, 8>;
				main: fn () -> void {
					let previous: u32 = atomic_compare_exchange(shared_keys[thread_idx()], 4294967295, 7);
				}
				"#,
				vec!["atomicCompSwap(shared_keys[uint(gl_LocalInvocationIndex)],4294967295,7)".into()],
				&[],
			),
			(
				"const-array",
				CONST_ARRAY,
				vec!["const vec3 WEIGHTS = vec3(0.5,0.25,0.125);".into(), "float value=WEIGHTS[1];".into()],
				&[],
			),
			(
				"short-scalar-arrays",
				SHORT_SCALAR_ARRAYS,
				vec![
					"vec3 scalar_f32()".into(),
					"u16vec3 scalar_u16()".into(),
					"uvec3 scalar_u32()".into(),
					"uvec3 mirror_indices(uvec3 indices)".into(),
					"vec3 floats=scalar_f32();".into(),
					"u16vec3 shorts=scalar_u16();".into(),
					"uvec3 indices=mirror_indices(scalar_u32());".into(),
				],
				&[],
			),
			(
				"dead-locals",
				r#"
				expensive: fn() -> f32 {
					return 42.0;
				}
				main: fn() -> void {
					let x: f32 = expensive();
					return;
				}
				"#,
				vec!["void main(){return;}".into()],
				&["expensive", "float x"],
			),
			(
				// GLSL has no atomic load or subtract, so a load adds zero and a subtract adds the negated value, and
				// signed atomics stay signed so `atomic_min` compares as signed integers.
				"modern-half-atomics",
				r#"
				Counters: struct { buffer_value: atomicu32, }
				counters: descriptor<{ type: Counters, binding: 0, access: read_write }>;
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
				"#,
				[
					"shared uint32_t unsigned_value;",
					"shared int32_t signed_value;",
					"atomicAdd(besl_binding_counters.buffer_value,0u)",
					"atomicAdd(unsigned_value,-(",
					"atomicExchange(",
					"atomicMin(",
					"atomicMax(",
					"atomicAnd(",
					"atomicOr(",
					"atomicXor(",
					"atomicCompSwap(",
					"float16_t fused=fma(",
					"f16vec3 fused_vector=fma(",
					"isnan(",
					"isinf(",
					"_besl_is_finite(",
					"_besl_is_normal(",
				]
				.map(String::from)
				.to_vec(),
				&[],
			),
		];

		for (name, source, present, absent) in &cases {
			let shader = lower_fixture(source, &ShaderGenerationSettings::compute(utils::Extent::square(8)));
			for needle in present {
				assert!(shader.contains(needle.as_str()), "Expected {name} GLSL to contain '{needle}'. GLSL: '{shader}'");
			}
			for needle in *absent {
				assert!(!shader.contains(needle), "Expected {name} GLSL not to contain '{needle}'. GLSL: '{shader}'");
			}
			compile(&shader, name);
		}
	}

	/// Verifies each vertex or fragment fixture lowers to GLSL that holds every expected fragment and none of the
	/// rejected ones, and compiles with glslang.
	#[test]
	fn raster_fixtures_lower_to_glsl() {
		use super::super::*;
		let sets = "push_constant._besl_sets[_besl_set0]";
		let element = |index: &str| {
			format!(
				"sampler2D(besl_binding_textures[nonuniformEXT({sets}.x+1u+({index}))],_besl_samplers[nonuniformEXT({sets}.y+0u+({index}))])"
			)
		};
		let slot = "besl_binding_items.items[index].slot";
		// Each case is a name, whether it is a vertex shader, a source, the fragments its GLSL holds, and the fragments
		// it must not hold.
		let cases: Vec<(&str, bool, &str, Vec<String>, &[&str])> = vec![
			(
				"runtime-array-texture-layer",
				false,
				RUNTIME_ARRAY_FRAGMENT,
				vec![
					"layout(set=0,binding=1,scalar) readonly buffer _instances{Instance instances[];}besl_binding_instances;".into(),
					"layout(descriptor_heap,descriptor_stride=64) uniform texture2DArray besl_binding_sprites[];".into(),
					"Instance instance=besl_binding_instances.instances[".into(),
					format!("texture(sampler2DArray(besl_binding_sprites[{sets}.x+0u],_besl_samplers[{sets}.y+0u]),vec3("),
					"float(instance.sprite_id)".into(),
				],
				&[],
			),
			(
				// The element's texture and sampler are both picked past the array's first, with an index that may
				// diverge.
				"descriptor-array-intrinsics",
				false,
				DESCRIPTOR_ARRAY_FRAGMENT,
				vec![
					format!("uvec2(textureSize({},0))", element(slot)),
					format!("textureLod({},uv,0.0)", element("index+1")),
					format!("texture({},uv)", element(slot)),
				],
				&[],
			),
			("fragment-pixel", false, FRAGMENT_PIXEL_FRAGMENT, vec!["uvec2(gl_FragCoord.xy)".into()], &[]),
			(
				"primitive-index",
				false,
				PRIMITIVE_INDEX_FRAGMENT,
				vec![
					"float(uint(gl_PrimitiveID))".into(),
					"layout(location=0)in vec2 _besl_interface_uv;".into(),
				],
				&["in uint32_t primitive_index"],
			),
			(
				// Inputs a mesh stage writes once per primitive are declared per primitive, which Vulkan needs to link
				// them, with the mesh shader extension that declares the qualifier.
				"primitive-fragment-inputs",
				false,
				r#"
				main: fn (pipeline_input: interface { instance_index: primitive<u32>, uv: vec2f }) -> output { color: vec4f } {
					return { color: vec4f(pipeline_input.uv.x, f32(pipeline_input.instance_index), 0.0, 1.0) };
				}
				"#,
				vec![
					"#extension GL_EXT_mesh_shader:require".into(),
					"layout(location=0)flat perprimitiveEXT in uint32_t _besl_interface_instance_index;".into(),
					"layout(location=1)in vec2 _besl_interface_uv;".into(),
				],
				&[],
			),
			(
				"structural-position",
				true,
				STRUCTURAL_POSITION_VERTEX,
				vec![
					"vec4 position=vec4(float(uint(gl_VertexIndex)),0.0,0.0,1.0);".into(),
					"gl_Position=position;".into(),
				],
				&["out vec4 _besl_interface_position"],
			),
			(
				// The layer is `gl_Layer`, an `int` that needs the viewport-layer extension in a vertex shader, and it
				// leaves the varying at location 0.
				"render-target-array-index",
				true,
				LAYERED_VERTEX,
				vec![
					"#extension GL_ARB_shader_viewport_layer_array:require".into(),
					"gl_Layer=int(layer);".into(),
					"layout(location=0)out vec2 _besl_interface_uv;".into(),
				],
				&["_besl_interface_render_target_array_index"],
			),
			(
				"stage-input-namesakes",
				true,
				STAGE_INPUT_NAMESAKES_VERTEX,
				vec!["uint(gl_VertexIndex)".into(), "uint(gl_InstanceIndex)".into()],
				&["layout(location=254)", "layout(location=255)"],
			),
		];

		for (name, vertex, source, present, absent) in &cases {
			let settings = if *vertex {
				ShaderGenerationSettings::vertex()
			} else {
				ShaderGenerationSettings::fragment()
			};
			let shader = lower_fixture(source, &settings);
			for needle in present {
				assert!(shader.contains(needle.as_str()), "Expected {name} GLSL to contain '{needle}'. GLSL: '{shader}'");
			}
			for needle in *absent {
				assert!(!shader.contains(needle), "Expected {name} GLSL not to contain '{needle}'. GLSL: '{shader}'");
			}
			compile(&shader, name);
		}
	}

	#[test]
	fn bindings() {
		let builtins = besl::Builtins::new();
		let mut program = generator::tests::bindings(&builtins);

		let shader = generate(&ShaderGenerationSettings::vertex(), &mut program);

		// Images alias the heap, and code reads them at their set's base index plus their offset in the set. Buffers are
		// variables at their set and offset, which the pipeline maps to the same heap slot.
		assert_string_contains!(shader, "#extension GL_EXT_descriptor_heap:require");
		assert_string_contains!(
			shader,
			"layout(push_constant,scalar)uniform PushConstant{uvec2 _besl_sets[15];}push_constant;"
		);
		assert_string_contains!(shader, "layout(descriptor_heap)uniform sampler _besl_samplers[];");
		assert_string_contains!(shader, "layout(constant_id=1984)const uint _besl_set0=0u;");
		assert_string_contains!(
			shader,
			"layout(set=0,binding=0,scalar) buffer _buff{float member;}besl_binding_buff;"
		);
		assert_string_contains!(
			shader,
			"layout(descriptor_heap,descriptor_stride=64,r8) writeonly uniform image2D besl_binding_image[];"
		);
		assert_string_contains!(
			shader,
			"layout(descriptor_heap,descriptor_stride=64) uniform texture2D besl_binding_texture[];"
		);
		assert!(!shader.contains("GL_EXT_shader_explicit_arithmetic_types_float16"));

		// The texture's sampler is the first sampler of the set, which the sampler heap holds after the set's base.
		assert!(shader.ends_with(
			"void main(){besl_binding_buff;besl_binding_image[push_constant._besl_sets[_besl_set0].x+1u];sampler2D(besl_binding_texture[push_constant._besl_sets[_besl_set0].x+2u],_besl_samplers[push_constant._besl_sets[_besl_set0].y+0u]);}"
		));
	}

	/// Verifies a stage that can't produce a raster builtin rejects it instead of emitting GLSL glslang refuses.
	#[test]
	fn raster_builtins_are_rejected_outside_their_stage() {
		let builtins = besl::Builtins::new();
		for (source, settings) in [
			(
				r#"main: fn (input: StageInput) -> interface { position: vec4f } {
					return { position: vec4f(f32(input.primitive_index), 0.0, 0.0, 1.0) };
				}"#,
				ShaderGenerationSettings::vertex(),
			),
			(
				r#"main: fn () -> interface { render_target_array_index: u32 } {
					let layer: u32 = 1;
					return { render_target_array_index: layer };
				}"#,
				ShaderGenerationSettings::fragment(),
			),
			(
				"main: fn (input: StageInput) -> output { color: vec4f } { let color: vec4f = vec4f(f32(input.vertex_index), 0.0, 0.0, 1.0); return { color }; }",
				ShaderGenerationSettings::fragment(),
			),
		] {
			let mut program =
				besl::compile_to_besl(source, besl::Program::new(&builtins)).expect("Expected fixture source to link");
			let main = program.get_main().expect("Expected fixture main function");
			Generator::new(&mut program)
				.minified(true)
				.generate(&settings, &main)
				.expect_err("Expected a raster builtin outside its stage to fail generation");
		}
	}

	#[test]
	fn vec4f_meshlet_record_uses_scalar_buffer_layout() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::vec4f_meshlet_binding(&builtins),
		);
		assert_string_contains!(shader, "vec4 center_radius;vec4 cone_apex_cutoff;");
		assert_string_contains!(shader, ",scalar) readonly buffer");
	}

	#[test]
	fn same_named_buffer_members_lower_to_glsl() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&mut generator::tests::same_named_buffer_member_access(&builtins),
		);
		assert_string_contains!(
			shader,
			"besl_binding_pixel_mapping.pixel_mapping[0]=besl_binding_meshes.meshes[1];"
		);
	}

	/// Verifies narrowing specializations into an f16 vector stays a runtime conversion: a vector
	/// `OpSpecConstantOp FConvert` crashes NVIDIA's Vulkan shader compiler.
	#[test]
	fn specializations_narrow_to_f16_vectors_at_run_time() {
		let builtins = besl::Builtins::new();
		let mut root = besl::Program::new(&builtins);
		let vec3f = root.get_child(root.root(), "vec3f").unwrap();
		let color_name = root.intern("color");
		root.add_to_root(besl::Node::specialization(color_name, vec3f, 1));
		let mut program = besl::compile_to_besl(
			r#"
			Result: struct { value: vec4f16 }
			result: descriptor<{ type: Result, binding: 0, access: write }>;
			main: fn () -> void {
				result.value = vec4f16(vec4f(color.x, color.y, color.z, 1.0));
			}
			"#,
			root,
		)
		.unwrap();
		let shader = generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &mut program);

		#[cfg(target_os = "linux")]
		{
			const OP_SPEC_CONSTANT_OP: u32 = 52;
			const OP_F_CONVERT: u32 = 115;
			let spirv = crate::shader::besl::backends::spirv::SpirvCompiler::new()
				.compile(&shader, "f16-specialization")
				.unwrap_or_else(|error| panic!("Expected the GLSL to compile to SPIR-V. {error}"));
			let words = spirv
				.chunks_exact(4)
				.map(|word| u32::from_le_bytes(word.try_into().unwrap()))
				.collect::<Vec<_>>();
			// Walk the instructions after the five-word header: each starts with its word count and opcode.
			let mut at = 5;
			while at < words.len() {
				let (count, opcode) = ((words[at] >> 16) as usize, words[at] & 0xffff);
				assert!(
					!(opcode == OP_SPEC_CONSTANT_OP && words[at + 3] == OP_F_CONVERT),
					"A specialization narrowed to f16 must not fold into OpSpecConstantOp FConvert."
				);
				at += count.max(1);
			}
		}
		#[cfg(not(target_os = "linux"))]
		let _ = shader;
	}

	/// Verifies each specialization becomes a constant at its declared id with a typed default.
	#[test]
	fn specializations_lower_to_constant_ids() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::specializations(&builtins),
		);
		assert_string_contains!(
			shader,
			"layout(constant_id=0)const bool enabled=false;layout(constant_id=1)const uint32_t count=0u;layout(constant_id=2)const float scale=0.0;layout(constant_id=4)const float color_x=0.0;layout(constant_id=5)const float color_y=0.0;layout(constant_id=6)const float color_z=0.0;const vec3 color=vec3(color_x,color_y,color_z);void main(){enabled;count;scale;color;}"
		);
	}

	/// Verifies 16-bit stage inputs and outputs travel as 32-bit values, which every Vulkan device passes between
	/// stages, that integers stay flat only across rasterization, and that reading an input narrows it back.
	#[test]
	fn packed_integer_vector_stage_io_travels_widened_and_flat_only_across_rasterization() {
		let builtins = besl::Builtins::new();
		let mut program = generator::tests::packed_u16_stage_io(&builtins);
		let vertex_shader = generate(&ShaderGenerationSettings::vertex(), &mut program);
		let fragment_shader = generate(&ShaderGenerationSettings::fragment(), &mut program);
		assert_string_contains!(vertex_shader, "layout(location=0)in uvec2 packed_input;");
		assert_string_contains!(vertex_shader, "layout(location=1)flat out uvec4 packed_output;");
		assert_string_contains!(fragment_shader, "layout(location=0)flat in uvec2 packed_input;");
		assert_string_contains!(fragment_shader, "layout(location=1)out uvec4 packed_output;");
		assert_string_contains!(fragment_shader, "u16vec2(packed_input)");

		compile(&vertex_shader, "besl-packed-stage-io-vertex");
		compile(&fragment_shader, "besl-packed-stage-io-fragment");
	}

	#[test]
	fn cull_unused_functions() {
		let builtins = besl::Builtins::new();
		let mut program = generator::tests::cull_unused_functions(&builtins);

		let shader = generate(&ShaderGenerationSettings::vertex(), &mut program);
		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn push_constant() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::push_constant(&builtins),
		);
		assert_string_contains!(
			shader,
			"layout(push_constant,scalar)uniform PushConstant{uvec2 _besl_sets[15];layout(offset=128)uint32_t material_id;}push_constant;void main(){push_constant;}"
		);
	}

	#[test]
	fn test_const_variable() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::vertex(),
			&mut generator::tests::const_variable(&builtins),
		);
		assert_string_contains!(shader, "const float besl_PI = 3.14;");
		assert_string_contains!(shader, "void main(){besl_PI;}");
	}

	/// Verifies per-vertex mesh outputs join the interpolated vertex struct while per-primitive outputs stay flat.
	#[test]
	fn vertex_mesh_outputs_join_the_vertex_struct() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::mesh(3, 1, utils::Extent::line(32)),
			&mut generator::tests::vertex_and_primitive_mesh_outputs(&builtins),
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

	/// Verifies a task shader runs BESL's `main` as a function of its own, then emits the mesh grid from every
	/// invocation, and keeps its payload arrays in the one payload variable GLSL allows.
	/// A grid emitted inside `main` or from one lane still compiles, so the lowering is asserted as well as compiled.
	#[test]
	fn generated_task_stage_emits_the_mesh_grid_after_main_returns() {
		let shader = lower_fixture(
			super::super::TASK_PAYLOAD_FIXTURE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);

		assert_string_contains!(shader, "#pragma shader_stage(task)\n");
		assert_string_contains!(shader, "#extension GL_EXT_mesh_shader:require");
		assert_string_contains!(shader, "layout(local_size_x=32,local_size_y=1,local_size_z=1) in;");
		assert_string_contains!(shader, "struct _BeslTaskPayload{uint32_t visible_meshlets[32];};");
		assert_string_contains!(shader, "taskPayloadSharedEXT _BeslTaskPayload _besl_task_payload;");
		assert_string_contains!(shader, "shared uint32_t visible_count;");
		assert_string_contains!(shader, "uint32_t position=uint(gl_GlobalInvocationID.x);");
		assert_string_contains!(shader, "_besl_task_payload.visible_meshlets[payload_index]=");
		// The helper sets the count, so the count is declared before every function.
		assert_string_contains!(shader, "_besl_task_mesh_count=atomicAdd(visible_count,0u)");
		assert_string_contains!(
			shader,
			"void main(){if(gl_LocalInvocationIndex==0u){_besl_task_mesh_count=0u;}barrier();_besl_task_main();barrier();EmitMeshTasksEXT(_besl_task_mesh_count,1u,1u);}"
		);

		compile(&shader, "besl-task-payload");
	}

	/// Verifies a mesh shader reads the task payload through the struct the task stage declares.
	#[test]
	fn generated_mesh_stage_reads_the_task_payload() {
		let shader = lower_fixture(
			super::super::MESH_PAYLOAD_FIXTURE,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);

		assert_string_contains!(shader, "struct _BeslTaskPayload{uint32_t visible_meshlets[32];};");
		assert_string_contains!(shader, "taskPayloadSharedEXT _BeslTaskPayload _besl_task_payload;");
		assert_string_contains!(
			shader,
			"uint32_t meshlet_index=_besl_task_payload.visible_meshlets[uint(gl_WorkGroupID.x)];"
		);
		assert_string_contains!(shader, "SetMeshOutputsEXT(3,1);");

		compile(&shader, "besl-mesh-payload");
	}

	#[test]
	fn f16_storage_types_enable_native_glsl_arithmetic() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
			&mut generator::tests::mixed_f16_storage_binding(&builtins),
		);
		assert_string_contains!(shader, "#extension GL_EXT_shader_explicit_arithmetic_types_float16:require");
		assert_string_contains!(shader, "float16_t scalar;");
		assert_string_contains!(shader, "f16vec2 uv;");
		assert_string_contains!(shader, "f16vec3 normal;");
		assert_string_contains!(shader, "f16vec4 color;");
		assert_string_contains!(shader, "_besl_f16vec2(uv32)");
		assert_string_contains!(shader, "vec2(uv16)");
		assert_string_contains!(shader, "float16_t(0.5)");
		assert_string_contains!(shader, "float(weight16)");
		assert_string_contains!(shader, "float16_t literal=float16_t(0.25);");
		assert_string_contains!(shader, "weight16*float16_t(2.0)");
		assert_string_contains!(shader, "uv16*float16_t(2.0)");
		assert!(!shader.contains("struct vec2f16"));
	}

	/// Links `source` after the ray tracing fixture declarations and lowers the program to minified GLSL for `settings`.
	fn lower_ray_tracing_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let builtins = besl::Builtins::new();
		let mut program = super::super::link_ray_tracing_fixture(&builtins, source);
		generate(settings, &mut program)
	}

	/// Verifies a ray generation shader lowers to a `raygen` shader that finds hits with ray queries, runs closest hits
	/// through the shader binding table's callable records, and compiles.
	#[test]
	fn generated_ray_generation_stage_compiles_with_glslang() {
		let shader = lower_ray_tracing_fixture(
			super::super::RAY_GENERATION_FIXTURE,
			&ShaderGenerationSettings::ray_generation(),
		);

		assert_string_contains!(shader, "#version 460 core\n#pragma shader_stage(raygen)\n");
		assert_string_contains!(shader, "#extension GL_EXT_ray_tracing:require");
		assert_string_contains!(shader, "#extension GL_EXT_ray_query:require");
		assert_string_contains!(
			shader,
			"layout(set=0,binding=0) uniform accelerationStructureEXT besl_binding_scene;"
		);
		// The callable data carries the payload and the hit, and `invoke` calls the hit's record.
		assert_string_contains!(shader, "struct _besl_call{SurfaceHit payload;Hit hit;};");
		assert_string_contains!(shader, "layout(location=0) callableDataEXT _besl_call _besl_call_data;");
		assert_string_contains!(shader, "executeCallableEXT(hit.record,0)");
		assert_string_contains!(shader, "return _besl_invoke(hit,SurfaceHit(");
		assert_string_contains!(shader, "_besl_trace_occlusion(besl_binding_scene,255,");
		assert_string_contains!(shader, "gl_LaunchIDEXT.x");
		assert_string_contains!(shader, "gl_LaunchSizeEXT.x");

		compile(&shader, "besl-ray-generation");
	}

	/// Verifies a closest hit lowers to a `callable` shader that reads the hit from the callable data, in `main` and
	/// in helpers, and returns the payload through it.
	#[test]
	fn generated_closest_hit_stage_compiles_with_glslang() {
		let shader = lower_ray_tracing_fixture(super::super::CLOSEST_HIT_FIXTURE, &ShaderGenerationSettings::closest_hit());

		assert_string_contains!(shader, "#pragma shader_stage(callable)");
		assert_string_contains!(shader, "layout(location=0) callableDataInEXT _besl_call _besl_call_data;");
		assert_string_contains!(shader, "_besl_call_data.hit.besl_distance");
		assert_string_contains!(shader, "_besl_call_data.hit.barycentrics");
		assert_string_contains!(shader, "SurfaceHit _besl_closest_hit_main(SurfaceHit payload)");
		assert_string_contains!(
			shader,
			"void main(){_besl_call_data.payload=_besl_closest_hit_main(_besl_call_data.payload);}"
		);

		compile(&shader, "besl-closest-hit");
	}

	/// Verifies a compute shader runs `intersect` and `trace_occlusion` as ray queries and builds a `Hit`.
	#[test]
	fn ray_queries_compile_in_compute_shaders() {
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

		assert_string_contains!(shader, "#version 460 core\n#pragma shader_stage(compute)\n");
		assert_string_does_not_contain!(shader, "GL_EXT_ray_tracing");
		assert_string_contains!(shader, "Hit hit=_besl_intersect(besl_binding_scene,1,origin,");
		assert_string_contains!(shader, "rebuilt.besl_distance");

		compile(&shader, "besl-ray-queries");
	}

	/// Compiles generated GLSL to SPIR-V on Linux so a lowering that glslang rejects fails the test.
	fn compile(shader: &str, name: &str) {
		#[cfg(target_os = "linux")]
		crate::shader::besl::backends::spirv::SpirvCompiler::new()
			.compile(shader, name)
			.unwrap_or_else(|error| panic!("Expected {name} GLSL to compile to SPIR-V. {error}"));
		#[cfg(not(target_os = "linux"))]
		let _ = (shader, name);
	}

	/// Verifies `fetch` reads an exact texel through `texelFetch` with signed coordinates and an explicit mip level.
	#[test]
	fn fetch_intrinsic_lowers_to_glsl() {
		let builtins = besl::Builtins::new();
		let shader = generate(
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
			&mut generator::tests::texel_fetch(&builtins),
		);
		assert_string_contains!(
			shader,
			"vec4 texel=texelFetch(sampler2D(besl_binding_texture[push_constant._besl_sets[_besl_set0].x+0u],_besl_samplers[push_constant._besl_sets[_besl_set0].y+0u]),ivec2(coord),0);"
		);

		compile(&shader, "besl-fetch");
	}
}
