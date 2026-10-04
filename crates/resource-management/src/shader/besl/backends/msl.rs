use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fmt::Write as _,
	vec::Vec,
};

pub use Generator as MSLTranspiler;

use super::{
	ResourceAccessorKind, SUBGROUP_INTRINSICS, any_code_node, is_intrinsic_call, resource_accessor, runtime_buffer_element,
};

/// Names the generated BESL Metal entry point persisted with compiled shader artifacts.
pub const MSL_ENTRY_POINT: &str = "besl_main";

use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages, emit_comma_separated_nodes,
	emit_statement_block, is_integer_besl_type, ordered_shader_nodes_in,
};

mod bindings;
mod emit;
mod facade;
mod generate;
mod node_emitter;
mod raster;
mod reserved;

pub(crate) use bindings::*;
pub(crate) use emit::*;
pub(crate) use facade::*;
pub use facade::{ComputeBindingMode, Generator};
pub(crate) use generate::*;
pub(crate) use raster::*;
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

	fn sampled_binding(name: &str, slot: u32, read: bool, write: bool) -> besl::NodeReference {
		besl::Node::binding(
			name,
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			slot,
			read,
			write,
		)
		.into()
	}

	#[compio::test]
	async fn sampled_binding_array_argument_is_emitted_in_resources() {
		let mut root = besl::Node::root();
		root.add_child(
			besl::Node::binding_array(
				"textures",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				9,
				true,
				false,
				4,
			)
			.into(),
		);
		let root = besl::compile_to_besl("main: fn () -> void { sample(textures[0], vec2f(0.0, 0.0)); }", Some(root))
			.expect("Expected sampled binding array source to link");
		let main = root.get_main().expect("Expected main");
		crate::shader::besl::evaluation::ProgramEvaluation::from_main(&main)
			.expect("Expected sampled binding array reflection");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected sampled binding array MSL generation");
		assert_string_contains!(shader, "texture2d<float> textures [[id(18)]][4];");
		assert_string_contains!(shader, "sampler textures_sampler [[id(22)]][4];");
		assert_string_contains!(shader, "resources.textures[0].sample(resources.textures_sampler[0]");

		compile_natively(&shader, "besl-fixed-slot-array").await;
	}

	#[compio::test]
	async fn runtime_buffer_and_texture_array_layer_use_native_msl_resources() {
		let root = besl::compile_to_besl(super::super::RUNTIME_ARRAY_FRAGMENT, None)
			.expect("Expected runtime-array fragment source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected runtime-array fragment MSL generation");

		assert_string_contains!(shader, "const device Instance* instances [[id(2)]];");
		assert_string_contains!(shader, "texture2d_array<float> sprites [[id(0)]];");
		assert_string_contains!(shader, "Instance instance=resources.instances[");
		assert_string_contains!(shader, "resources.sprites.sample(resources.sprites_sampler, ");
		assert_string_contains!(shader, ", instance.sprite_id)");

		compile_natively(&shader, "besl-runtime-array-texture-layer").await;
	}

	#[compio::test]
	async fn affine_matrix_arrays_use_packed_msl_storage() {
		let root = besl::compile_to_besl(
			r#"
			Palette: struct { matrices: mat4x3f[4] }
			palette: descriptor<{ type: Palette, binding: 0, access: read_write }>;
			main: fn (input: StageInput) -> void {
				let item: u32 = input.thread_id.x;
				let affine: mat4x3f = palette.matrices[item];
				palette.matrices[item + 1] = affine;
			}
			"#,
			None,
		)
		.expect("Expected affine matrix array source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected affine matrix array MSL generation");

		// Packed 48-byte matrices match the CPU stride; a native float4x3 would read with a 64-byte stride.
		assert_string_contains!(shader, "device _besl_packed_float4x3* palette");
		assert_string_contains!(shader, "_besl_load_mat4x3(resources.palette[item])");
		assert_string_contains!(shader, "_besl_store_mat4x3(resources.palette[item+1],affine)");

		compile_natively(&shader, "besl-affine-matrix-array").await;
	}

	#[compio::test]
	async fn scalar_runtime_arrays_use_packed_msl_element_pointers() {
		let root = besl::compile_to_besl(super::super::SCALAR_RUNTIME_ARRAY_COMPUTE, None)
			.expect("Expected scalar runtime-array compute source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected scalar runtime-array MSL generation");

		// `packed_float3` keeps the 12-byte CPU stride; a plain `float3` would read with a 16-byte stride.
		assert_string_contains!(shader, "const device packed_float3* positions");
		assert_string_contains!(shader, "const device ushort* indices");
		assert_string_contains!(shader, "const device uchar* corners");
		assert_string_contains!(shader, "device uint* results");

		compile_natively(&shader, "besl-scalar-runtime-array").await;
	}

	#[compio::test]
	async fn descriptor_array_elements_reach_every_texture_intrinsic_in_msl() {
		let root = besl::compile_to_besl(super::super::DESCRIPTOR_ARRAY_FRAGMENT, None)
			.expect("Expected descriptor-array fragment source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected descriptor-array fragment MSL generation");

		assert_string_contains!(
			shader,
			"uint2(resources.textures[resources.items[index].slot].get_width(),resources.textures[resources.items[index].slot].get_height())"
		);
		assert_string_contains!(
			shader,
			"resources.textures[index+1].sample(resources.textures_sampler[index+1], uv, metal::level(0.0))"
		);
		assert_string_contains!(
			shader,
			"resources.textures[resources.items[index].slot].sample(resources.textures_sampler[resources.items[index].slot], uv)"
		);

		compile_natively(&shader, "besl-descriptor-array-intrinsics").await;
	}

	#[compio::test]
	async fn structural_position_uses_metal_position_without_colliding_with_a_local() {
		let root = besl::compile_to_besl(super::super::STRUCTURAL_POSITION_VERTEX, None)
			.expect("Expected structural position source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &root.get_main().expect("Expected main"))
			.expect("Expected structural position MSL generation");

		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];float2 _besl_interface_uv"
		);
		assert_string_contains!(shader, "float4 position=float4(float(vertex_index),0.0,0.0,1.0);");
		assert_string_contains!(shader, "out.position=_besl_interface_position;");
		assert!(!shader.contains("_besl_interface_position [[user("));

		compile_natively(&shader, "besl-structural-position").await;
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
		let root = besl::compile_to_besl(source, None).expect("Expected reserved-name source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected reserved-name MSL generation");

		assert_string_contains!(shader, "struct besl_half{float besl_float3;};");
		assert_string_contains!(shader, "struct _thread{");
		assert_string_contains!(shader, "device _thread* besl_thread [[id(0)]];");
		assert_string_contains!(shader, "texture2d<float> besl_constant [[id(2)]];");
		assert_string_contains!(shader, "sampler besl_constant_sampler [[id(3)]];");
		assert_string_contains!(shader, "besl_half scale(float besl_sampler");
		assert_string_contains!(shader, "besl_half besl_device=besl_half{besl_sampler};");
		assert_string_contains!(shader, "return besl_device;");
		assert_string_contains!(
			shader,
			"float4 besl_kernel=resources.besl_constant.sample(resources.besl_constant_sampler, float2(0.0,0.0))"
		);
		assert_string_contains!(shader, "resources.besl_thread->besl_float3=scale(besl_kernel.x");
	}

	#[test]
	fn full_program_resource_abi_retains_unreachable_declared_bindings() {
		let program = besl::compile_to_besl(
			"Payload: struct { value: f32, } Unused: struct { payload: Payload, } unused: descriptor<{ type: Unused, binding: 3, access: read, memory: constant }>; Used: struct { value: u32, } used: descriptor<{ type: Used, binding: 9, access: write, memory: device }>; main: fn () -> void { used.value = 7; }",
			None,
		)
		.expect("Expected full-program resource ABI fixture to link");
		let shader = Generator::new()
			.minified(true)
			.generate_program(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &program)
			.expect("Expected full-program MSL generation");
		assert_string_contains!(shader, "struct Payload");
		assert_string_contains!(shader, "constant _unused* unused [[id(6)]];");
		assert_string_contains!(shader, "device _used* used [[id(18)]];");
		assert_string_contains!(shader, "resources.used->value=7;");
		assert!(!shader.contains("resources.unused"));
	}

	#[test]
	fn generating_the_same_program_twice_produces_identical_output() {
		// Fresh node graphs get fresh addresses, which used to reorder independent declarations.
		let generate = || {
			let program = besl::compile_to_besl(crate::resources::mips::bc7::BC7_ENCODER, None)
				.expect("Expected the BC7 kernel to parse and link");
			Generator::new()
				.generate_program(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &program)
				.expect("Expected BC7 MSL generation")
		};
		let first = generate();
		for _ in 0..4 {
			assert_eq!(first, generate());
		}
	}

	fn main_with(statements: Vec<besl::NodeReference>) -> besl::NodeReference {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		besl::Node::function("main", Vec::new(), void, statements).into()
	}

	#[test]
	fn distinct_reachable_declaration_ranges_cannot_overlap() {
		let array: besl::NodeReference = besl::Node::binding_array(
			"array",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			4,
			true,
			false,
			2,
		)
		.into();
		let main = main_with(vec![array, sampled_binding("interior", 5, true, false)]);
		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Intersecting flat slot intervals must be rejected before MSL emission"
		);
	}

	#[test]
	fn fixed_metal_argument_id_ranges_cannot_overflow() {
		let binding: besl::NodeReference = besl::Node::binding_array(
			"textures",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			true,
			false,
			u32::MAX as usize,
		)
		.into();
		let main = main_with(vec![binding]);
		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Fixed Metal argument IDs must not wrap"
		);
	}

	#[test]
	fn bindings() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]];");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]];");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(2)]];");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(2)]];");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
	}

	#[compio::test]
	async fn packed_vec4f_uses_native_msl_vectors_and_a_52_byte_record_stride() {
		let mut shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::packed_vec4f_meshlet_binding(),
			)
			.expect("Expected packed_vec4f MSL generation");
		assert_string_contains!(shader, "packed_float4 center_radius;packed_float4 cone_apex_cutoff;");
		assert!(!shader.contains("struct packed_vec4f"));
		shader.push_str("\nstatic_assert(sizeof(Meshlet) == 52, \"Packed Meshlet stride must match the host\");\n");

		compile_natively(&shader, "besl-packed-vec4f").await;
	}

	#[test]
	fn packed_u16_storage_vectors_preserve_tight_array_and_mixed_struct_layouts() {
		let vec2_array = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2u16_array_binding(),
			)
			.expect("Expected vec2u16 MSL generation");
		let mixed_vec4 = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_vec4u16_binding(),
			)
			.expect("Expected mixed vec4u16 MSL generation");
		assert_string_contains!(vec2_array, "device packed_ushort2* buff");
		assert_string_contains!(mixed_vec4, "struct _buff{packed_ushort4 value;ushort tail;};");
	}

	#[compio::test]
	async fn f16_storage_vectors_use_packed_msl_types() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_f16_storage_binding(),
			)
			.expect("Expected f16 MSL generation");
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
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"struct _resources{device _buff* buff [[id(0)]];texture2d<float, access::write> image [[id(2)]];texture2d<float> texture [[id(4)]];sampler texture_sampler [[id(5)]];};"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint2 gid [[thread_position_in_grid]],uint thread_index [[thread_index_in_threadgroup]],uint2 threadgroup_position [[threadgroup_position_in_grid]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "resources.buff;resources.image;resources.texture;");
	}

	#[compio::test]
	async fn texture_lod_qualifies_metal_level_helper() {
		let source = r#"
			depth_texture: descriptor<{ type: Texture2D, binding: 0, access: read }>;
			sample_depth: fn (uv: vec2f, level: u32) -> f32 {
				return texture_lod(depth_texture, uv, f32(level)).x;
			}
			main: fn () -> void {
				sample_depth(vec2f(0.5, 0.5), 1);
			}
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected texture LOD source to link");
		let main = root.get_main().expect("Expected texture LOD source to define main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Expected texture LOD source to lower to Metal");
		assert_string_contains!(shader, "metal::level(float(level))");

		compile_natively(&shader, "besl-texture-lod-level-shadowing").await;
	}

	#[compio::test]
	async fn conservative_downsampling_gathers_and_reduces_in_shader_code() {
		let source = r#"
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
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected conservative downsample source to link");
		let main = root
			.get_main()
			.expect("Expected conservative downsample source to define main");
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let shader = Generator::new()
			.minified(true)
			.generate(&settings, &main)
			.expect("Expected downsampling MSL");
		assert_string_contains!(
			shader,
			"_besl_downsample_min(resources.depth_texture, resources.depth_texture_sampler"
		);
		assert_string_contains!(
			shader,
			"_besl_downsample_max(resources.depth_texture, resources.depth_texture_sampler"
		);
		assert_string_contains!(shader, ".gather(texture_sampler, uv, int2(0), component::x)");
		assert_string_contains!(shader, ".gather(texture_sampler, uv, layer, int2(0), component::x)");
		assert_string_contains!(shader, "texture.read(a, level).x");

		compile_natively(&shader, "besl-downsample-gather").await;
	}

	#[compio::test]
	async fn buffer_memory_classes_select_metal_address_spaces() {
		let source = r#"
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
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected memory-class source to link");
		let main = root.get_main().expect("Expected memory-class source to define main");
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));

		let argument_buffer_shader = Generator::new()
			.minified(true)
			.generate(&settings, &main)
			.expect("Expected memory-class source to lower through Metal argument buffers");
		assert_string_contains!(
			argument_buffer_shader,
			"constant _dispatch_values* dispatch_values [[id(0)]];"
		);
		assert_string_contains!(argument_buffer_shader, "const device uint* vertices [[id(2)]];");
		assert_string_contains!(argument_buffer_shader, "device uint* counters [[id(4)]];");

		let bare_resource_shader = Generator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(&settings, &main)
			.expect("Expected memory-class source to lower through bare Metal resources");
		assert_string_contains!(
			bare_resource_shader,
			"constant _dispatch_values* dispatch_values [[buffer(0)]]"
		);
		assert_string_contains!(bare_resource_shader, "const device uint* vertices [[buffer(1)]]");
		assert_string_contains!(bare_resource_shader, "device uint* counters [[buffer(2)]]");

		compile_natively(&argument_buffer_shader, "besl-buffer-memory-classes").await;
	}

	#[test]
	fn compute_bindings_can_use_bare_resources() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "kernel void besl_main(uint2 gid [[thread_position_in_grid]],");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]]");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]]");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(2)]]");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(2)]]");
		assert_string_contains!(shader, "buff;image;texture;");
	}

	#[compio::test]
	async fn local_arrays_copy_and_pass_by_value_in_metal_syntax() {
		// Metal has neither GLSL's `float4[3]` type spelling nor its array constructor, and C arrays can't be copied or
		// passed by value, so value arrays become `metal::array`.
		let source = r#"
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
			let index: u32 = clamp(max(thread_idx(), 1), 0, 2);
			first(positions, index);
		}
		"#;

		let root = besl::compile_to_besl(source, None)
			.expect("Expected local array shader source to compile. The most likely cause is invalid BESL syntax.");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(1)), &main)
			.expect("Expected local array MSL generation");

		assert_string_contains!(shader, "metal::array<float4, 3> positions=metal::array<float4, 3>{");
		assert_string_contains!(shader, "float4 first(metal::array<float4, 3> values,");
		assert_string_contains!(shader, "metal::array<float4, 3> copy=values;");
		// Unsigned overloads cast their arguments, because C++ spells BESL's unsigned literals as `int`.
		assert_string_contains!(shader, "min(uint(count),uint(2))");
		assert!(
			!shader.contains("float4[3]"),
			"Expected no GLSL array type spelling in MSL output, got: {shader}"
		);

		compile_natively(&shader, "besl-local-array").await;
	}

	const TASK_PAYLOAD_FIXTURE_SOURCE: &str = r#"
		Meshlets: struct {
			values: u32[32],
		}
		meshlets: descriptor<{ type: Meshlets, binding: 8, access: read }>;
		visible_meshlets: task_payload<u32, 32>;
		visible_count: workgroup<atomicu32>;
		push_constant: push_constant {
			base_meshlet: u32,
		}

		dispatch_visible_meshlets: fn () -> void {
			let position: u32 = thread_position();
			let lane: u32 = thread_idx();
			if (lane == 0) {
				atomic_store(visible_count, 0);
			}
			workgroup_barrier();
			if (position < 32) {
				let payload_index: u32 = atomic_add(visible_count, 1);
				visible_meshlets[payload_index] = meshlets.values[push_constant.base_meshlet + position];
			}
			workgroup_barrier();
			if (lane == 0) {
				set_task_mesh_output_count(atomic_load(visible_count));
			}
		}

		main: fn () -> void {
			dispatch_visible_meshlets();
		}
	"#;

	const COMPUTE_WORKGROUP_FIXTURE_SOURCE: &str = r#"
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
	"#;

	const MESH_PAYLOAD_FIXTURE_SOURCE: &str = r#"
		visible_meshlets: task_payload<u32, 32>;
		out_instance_index: output<u32, 0, 126>;
		out_primitive_index: output<u32, 1, 126>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			let meshlet_index: u32 = visible_meshlets[threadgroup_position()];
			set_mesh_output_counts(3, 1);
			if (lane < 3) {
				set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
			}
			if (lane < 1) {
				set_mesh_triangle(0, vec3u(0, 1, 2));
				out_primitive_index[0] = meshlet_index;
				set_mesh_primitive_render_target_array_index(0, 2);
				out_instance_index[0] = meshlet_index;
			}
		}
	"#;

	/// Compiles generated MSL with the Metal toolchain on macOS so a lowering that Metal rejects fails the test.
	async fn compile_natively(shader: &str, name: &str) {
		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(shader, name)
			.await
			.unwrap_or_else(|error| panic!("Expected {name} MSL to compile natively. {error}"));
		#[cfg(not(target_os = "macos"))]
		let _ = (shader, name);
	}

	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let root = besl::compile_to_besl(source, None).expect("Expected stage fixture source to link");
		let main = root.get_main().expect("Expected stage fixture main function");
		Generator::new()
			.minified(true)
			.generate(settings, &main)
			.expect("Expected stage fixture to lower to MSL")
	}

	#[test]
	fn find_lsb_lowers_to_a_helper_that_reports_no_bit_for_zero() {
		let shader = lower_fixture(
			r#"
			result: descriptor<{ type: u32, binding: 0, access: read_write }>;

			main: fn () -> void {
				let bits: u32 = 40;
				result = find_lsb(bits);
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "value == 0u ? 0xffffffffu : ctz(value)");
		assert_string_contains!(shader, "_besl_find_lsb(bits)");
	}

	#[test]
	fn subgroup_lane_id_is_forwarded_only_to_helpers_that_use_it() {
		let root = besl::compile_to_besl(
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
			None,
		)
		.expect("Expected subgroup helper fixture to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(32)),
				&root.get_main().expect("Expected subgroup helper main function"),
			)
			.expect("Expected subgroup helper MSL generation");
		assert_string_contains!(
			shader,
			"uint lane(uint2 gid,uint thread_index,uint2 threadgroup_position,uint simd_lane_id)"
		);
		assert_string_contains!(shader, "lane(gid,thread_index,threadgroup_position,simd_lane_id)");
		assert_string_contains!(shader, "uint ordinary()");
		assert_string_contains!(shader, "uint simd_lane_id [[thread_index_in_simdgroup]]");
	}

	#[test]
	fn subgroup_intrinsics_are_limited_to_compute_stages() {
		let root = besl::compile_to_besl("main: fn () -> void { let mask: vec4u = subgroup_ballot(true); mask; }", None)
			.expect("Expected subgroup stage fixture source to link");
		let main = root.get_main().expect("Expected subgroup stage fixture main function");
		assert!(
			Generator::new().generate(&ShaderGenerationSettings::vertex(), &main).is_err(),
			"Subgroup intrinsics must not lower outside compute stages"
		);
	}

	#[test]
	fn mesh_stage_consumes_the_same_authored_task_payload() {
		let shader = lower_fixture(
			MESH_PAYLOAD_FIXTURE_SOURCE,
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

	#[compio::test]
	async fn mat4x3_buffer_storage_is_packed_behind_native_matrix_expressions() {
		let shader = lower_fixture(
			r#"
				Transform: struct {
					model: mat4x3f,
					tag: u32,
				}
				Transforms: struct {
					values: Transform[2],
					direct: mat4x3f[2],
				}
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
			&ShaderGenerationSettings::compute(utils::Extent::line(1)),
		);
		assert_string_contains!(shader, "struct _besl_packed_float4x3 { packed_float3 columns[4]; };");
		assert_string_contains!(shader, "struct Transform{_besl_packed_float4x3 model;uint tag;};");
		assert_string_contains!(shader, "_besl_packed_float4x3 direct[2]");
		assert_string_contains!(shader, "float4x3 model=_besl_load_mat4x3(");
		assert_string_contains!(shader, "float3 position=(model*float4(1.0,2.0,3.0,1.0));");
		assert_string_contains!(
			shader,
			"float3 local_position=(_besl_load_mat4x3(local.model)*float4(4.0,5.0,6.0,1.0));"
		);
		assert!(
			!shader.contains("mul("),
			"MSL matrix expressions must use Metal's native multiplication operator."
		);
		assert_string_contains!(shader, "Transform local=Transform{_besl_pack_mat4x3(model),7};");
		assert_string_contains!(shader, "_besl_store_mat4x3(");

		compile_natively(&shader, "besl-packed-mat4x3").await;
	}

	/// Verifies the task stage maps BESL builtins, workgroup storage, barriers and the mesh dispatch onto Metal object-stage features.
	/// Several of these mistakes still compile (for example swapped thread builtins or a dropped barrier), so the lowering is asserted as well as compiled.
	#[compio::test]
	async fn generated_task_and_mesh_payload_stages_compile_with_metal() {
		let task = lower_fixture(
			TASK_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);
		let mesh = lower_fixture(
			MESH_PAYLOAD_FIXTURE_SOURCE,
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

	/// Verifies workgroup storage stays in threadgroup memory, reaches helpers by pointer, and keeps its barrier.
	/// A dropped barrier or a per-thread copy of the storage still compiles, so the lowering is asserted as well as compiled.
	#[compio::test]
	async fn generated_compute_workgroup_stage_compiles_with_metal() {
		let shader = lower_fixture(
			COMPUTE_WORKGROUP_FIXTURE_SOURCE,
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
		);
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "threadgroup float scratch[64];");
		assert_string_contains!(shader, "threadgroup float* scratch");
		assert_string_contains!(shader, "threadgroup_barrier(mem_flags::mem_threadgroup)");
		assert_string_contains!(
			shader,
			"store_scratch(float(thread_index),gid,thread_index,threadgroup_position,scratch)"
		);

		compile_natively(&shader, "besl-compute-workgroup-fixture").await;
	}

	/// Verifies subgroup intrinsics map onto Metal SIMD-group operations with the lane type Metal expects.
	#[compio::test]
	async fn generated_compute_subgroup_stage_compiles_with_metal() {
		let shader = lower_fixture(
			r#"
			scratch: workgroup<u32, 1>;

			main: fn () -> void {
				let mask: vec4u = subgroup_ballot(thread_idx() < 4);
				let leader: u32 = subgroup_ballot_find_lsb(mask);
				let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
				if (subgroup_ballot_any(mask)) {
					scratch[0] = subgroup_ballot_count(subgroup_ballot_and_not(mask, subgroup_ballot(value == 0)));
				}
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(32)),
		);
		assert_string_contains!(shader, "simd_ballot(predicate)");
		assert_string_contains!(shader, "simd_broadcast(value, ushort(source_lane))");
		assert_string_contains!(shader, "_besl_subgroup_ballot_find_lsb(mask)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_count(");
		assert_string_contains!(shader, "threadgroup uint scratch[1]");

		compile_natively(&shader, "besl-compute-subgroup-fixture").await;
	}

	#[test]
	fn mesh_stage_uses_mesh_entry_point_and_mesh_push_constants() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let main = besl::parser::Node::function(
			"main",
			Vec::new(),
			"void",
			vec![besl::parser::Node::raw_code(
				Some("".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				&["push_constant", "VertexOutput", "PrimitiveOutput"],
				&[],
			)],
		);
		let shader = besl::parser::Node::scope("Shader", vec![push_constant, mesh_output_types, main]);
		let mut root = besl::parser::Node::root();
		root.add(vec![shader]);

		let root_node = besl::lex(root).unwrap();
		let main_node = root_node.get_main().unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");
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
		let root = besl::lex(root).unwrap();
		let main_node = root.get_main().unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
		assert_string_contains!(shader, "uint2 gid [[thread_position_in_grid]]");
		assert_string_contains!(shader, "uint simd_lane_id [[thread_index_in_simdgroup]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "uint2 threadgroup_position [[threadgroup_position_in_grid]]");

		compile_natively(&shader, "besl-compute-stage-inputs").await;
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "constant float color_x [[function_constant(0)]];");
		assert_string_contains!(shader, "constant float color_y [[function_constant(1)]];");
		assert_string_contains!(shader, "constant float color_z [[function_constant(2)]];");
		assert_string_contains!(shader, "constant float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[compio::test]
	async fn vertex_builtin_stage_inputs_lower_to_msl_semantics() {
		let root = besl::compile_to_besl(
			r#"
			invocation_sum: fn () -> u32 {
				return vertex_index + instance_index;
			}
			position: output<vec4f, 255>;
			out_value: output<u32, 0>;
			main: fn () -> void {
				position = vec4f(f32(vertex_index), 0.0, 0.0, 1.0);
				out_value = invocation_sum();
			}
			"#,
			None,
		)
		.expect("Expected implicit vertex builtins to link");
		let main = root.borrow().get_child("main").unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct VertexInput{};");
		assert_string_contains!(shader, "uint vertex_index [[vertex_id]],uint instance_index [[instance_id]]");
		assert_string_contains!(shader, "invocation_sum(vertex_index,instance_index)");
		assert!(!shader.contains("uint vertex_index=vertex_index;"));
		assert!(!shader.contains("uint instance_index=instance_index;"));

		compile_natively(&shader, "besl-vertex-invocation-indices").await;
	}

	#[test]
	fn fragment_builtin_stage_io_lowers_to_msl_semantics() {
		let mut root = besl::Node::root();
		let bool_type = root.get_child("bool").expect("Expected bool type");
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		root.add_child(besl::Node::input("front_facing", bool_type, 0).into());
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_child(besl::Node::output("depth", f32_type, 0).into());
		root.add_child(besl::Node::output("stencil", u32_type.clone(), 1).into());
		root.add_child(besl::Node::output("sample_mask", u32_type, 2).into());

		let root = besl::compile_to_besl(
			"main: fn () -> void { front_facing; depth; stencil; sample_mask; }",
			Some(root),
		)
		.unwrap();
		let main = root.borrow().get_child("main").unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct FragmentInput{};");
		assert_string_contains!(shader, "float depth [[depth(any)]];");
		assert_string_contains!(shader, "uint stencil [[stencil]];");
		assert_string_contains!(shader, "uint sample_mask [[sample_mask]];");
		assert_string_contains!(shader, "bool front_facing [[front_facing]]");
		assert!(!shader.contains("bool front_facing=front_facing;"));
	}

	#[test]
	fn raster_full_source_passthrough_uses_raw_msl_source() {
		let source = "// besl-full-source\n#include <metal_stdlib>\nvertex void besl_main() {}";
		let mut root = besl::parser::Node::root();
		let main = besl::parser::Node::main_function(vec![besl::parser::Node::raw_code(
			Some("".into()),
			None,
			Some(source.into()),
			&[],
			&[],
		)]);
		root.add(vec![besl::parser::Node::scope("Shader", vec![main])]);

		let main = besl::lex(root).unwrap().get_main().unwrap();
		let shader = Generator::new()
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_eq!(shader, "#include <metal_stdlib>\nvertex void besl_main() {}");
	}

	#[test]
	fn vertex_shader_generates_msl_entry_point() {
		let mut root = besl::parser::Node::root();
		let camera = besl::parser::Node::r#struct("Camera", vec![besl::parser::Node::member("view_projection", "mat4f")]);
		let cameras = besl::parser::Node::constant_buffer_binding(
			"cameras",
			besl::parser::Node::buffer(vec![besl::parser::Node::member("cameras", "Camera[8]")]),
			0,
			true,
			false,
		);
		let main = besl::parser::Node::main_function(vec![besl::parser::Node::raw_code(
			Some("".into()),
			None,
			Some("position = resources.cameras[0].view_projection * float4(in_position, 1.0); out_instance_index = 0u;".into()),
			&["cameras", "in_position", "out_instance_index"],
			&[],
		)]);
		root.add(vec![besl::parser::Node::scope(
			"Shader",
			vec![
				camera,
				cameras,
				besl::parser::Node::input("in_position", "vec3f", 0),
				besl::parser::Node::output("out_instance_index", "u32", 0),
				main,
			],
		)]);

		let main = besl::lex(root).unwrap().get_main().unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(shader, "struct _resources{constant Camera* cameras [[id(0)]];};");
		assert_string_contains!(shader, "struct VertexInput{float3 in_position [[attribute(0)]];};");
		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];uint out_instance_index [[flat]] [[user(locn0)]];};"
		);
		assert_string_contains!(
			shader,
			"vertex VertexOutput besl_main(VertexInput in [[stage_in]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "position = resources.cameras[0].view_projection");
		assert_string_contains!(shader, "return out;");
	}

	/// Verifies raster helpers retain binding access when lowered outside the Metal entry point.
	#[test]
	fn raster_helpers_receive_argument_buffer_context() {
		let mut root = besl::Node::root();
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let vec3f = root.get_child("vec3f").expect("Expected vec3f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		let camera =
			root.add_child(besl::Node::r#struct("Camera", vec![besl::Node::member("view_projection", mat4f).into()]).into());
		root.add_children(vec![
			besl::Node::binding_in_memory(
				"cameras",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("cameras", camera, 1)],
				},
				0,
				true,
				false,
				besl::BufferMemoryClass::Constant,
				None,
			)
			.into(),
			besl::Node::input("in_position", vec3f, 0).into(),
			besl::Node::output("position", vec4f, 0).into(),
		]);

		let program = besl::compile_to_besl(
			r#"
			camera_matrix: fn () -> mat4f {
				return cameras.cameras[0].view_projection;
			}
			main: fn () -> void {
				position = camera_matrix() * vec4f(in_position.x, in_position.y, in_position.z, 1.0);
			}
			"#,
			Some(root),
		)
		.expect("Failed to compile the raster helper fixture. The most likely cause is invalid BESL syntax.");
		let main = program.get_main().expect("Expected raster helper fixture main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate raster helper MSL. The most likely cause is missing raster resource context.");
		assert_string_contains!(shader, "float4x4 camera_matrix(constant _resources& resources);");
		assert_string_contains!(
			shader,
			"float4x4 camera_matrix(constant _resources& resources){return resources.cameras[0].view_projection;}"
		);
		assert_string_contains!(
			shader,
			"position=(camera_matrix(resources)*float4(in_position.x,in_position.y,in_position.z,1.0));"
		);
	}

	#[test]
	fn push_constant() {
		let main = generator::tests::push_constant();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
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

		let root = besl::compile_to_besl(script, None).expect("Expected matrix multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4x4 model,float4 position){return (projection*model)*position;}"
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

		// The MSL transpiler should use the explicit MSL code.
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void main(){out.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "MSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");
		// The backend declares its own `PI`, so a BESL constant with that name must be prefixed.
		assert_string_contains!(shader, "constant float besl_PI = 3.14;");
		assert_string_contains!(shader, "void main(){besl_PI;}");
	}

	#[test]
	fn mesh_intrinsics_emit_msl_mesh_commands() {
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![mesh_output_types]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");
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

	#[compio::test]
	async fn else_chains_lower_to_msl() {
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

		compile_natively(&shader, "besl-else-chain").await;
	}

	#[compio::test]
	async fn match_lowers_to_msl_switch() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			let small: u16 = u16(n);
			for (let i: u32 = 0; i < 4; i = i + 1) {
				match i {
					0 => n = 1,
					1 | 2 => break,
					_ => {}
				}
			}
			match small {
				65535 => n = 2,
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
		assert_string_contains!(
			shader,
			"{bool besl_match_break_0=false;switch(i){case 0u:{n=1;break;}case 1u:case 2u:{besl_match_break_0=true;break;break;}default:{break;}}if(besl_match_break_0){break;}}"
		);
		assert_string_contains!(shader, "switch(uint(small)){case 65535u:{n=2;break;}default:{n=3;break;}}");

		compile_natively(&shader, "besl-match").await;
	}

	#[compio::test]
	async fn short_scalar_arrays_lower_to_msl_vectors() {
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
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected scalar arrays to lower to MSL vectors");
		assert_string_contains!(shader, "float3 scalar_f32()");
		assert_string_contains!(shader, "ushort3 scalar_u16()");
		assert_string_contains!(shader, "uint3 scalar_u32()");
		assert_string_contains!(shader, "uint3 mirror_indices(uint3 indices)");
		assert_string_contains!(shader, "float3 floats=scalar_f32();");
		assert_string_contains!(shader, "ushort3 shorts=scalar_u16();");
		assert_string_contains!(shader, "uint3 indices=mirror_indices(scalar_u32());");

		compile_natively(&shader, "besl-short-scalar-arrays").await;
	}

	#[compio::test]
	async fn source_declared_atomic_images_and_push_constants_lower_to_msl() {
		let source = r#"
			Counters: struct {
				values: atomicu32[8],
			}
			counters: descriptor<{ type: Counters, binding: 2, access: read_write }>;
			index_image: descriptor<{ type: StorageImage<r32ui>, binding: 4, access: read }>;
			shared_keys: workgroup<atomicu32, 8>;
			push_constant: push_constant {
				base: u32,
			}
			main: fn () -> void {
				let coord: vec2u = thread_id();
				let index: u32 = image_load_u32(index_image, coord) + push_constant.base;
				let old: u32 = atomic_add(counters.values[index], 1);
				let claimed: u32 = atomic_compare_exchange(counters.values[index], old, 7);
				let shared_claimed: u32 = atomic_compare_exchange(shared_keys[index % 8], 4294967295, index);
				atomic_store(counters.values[index], atomic_load(counters.values[old]));
			}
		"#;

		let root = besl::compile_to_besl(source, None).expect("Expected standalone atomic source to lex");
		let main = root.get_main().expect("Expected standalone atomic source main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected standalone atomic source to lower to MSL");
		assert_string_contains!(shader, "device atomic_uint* counters");
		assert_string_contains!(shader, "texture2d<uint, access::read> index_image");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, ".read(coord).x");
		assert_string_contains!(shader, "atomic_fetch_add_explicit(&");
		assert_string_contains!(shader, "_besl_atomic_compare_exchange(resources.counters[index],old,7)");
		assert_string_contains!(shader, "_besl_atomic_compare_exchange(shared_keys[index%8],4294967295,index)");
		assert_string_contains!(
			shader,
			"while (!atomic_compare_exchange_weak_explicit(&value, &expected, desired"
		);
		assert_string_contains!(shader, "atomic_load_explicit(&");
		assert_string_contains!(shader, "atomic_store_explicit(&");

		compile_natively(&shader, "besl-atomic-compare-exchange").await;
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
		let root = besl::compile_to_besl(source, None).expect("Expected raster push-constant source to lex");
		let main = root.get_main().expect("Expected raster push-constant main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Expected raster push constants to lower to MSL");

		assert_string_contains!(shader, "struct PushConstant{");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");

		compile_natively(&shader, "besl-raster-push-constant").await;
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
			.expect("Expected MSL power lowering.");

		assert_eq!(shader.matches("exp2(").count(), 2);
		assert!(!shader.contains("pow("));
	}

	/// Verifies integer atomics use Metal's explicit relaxed operations on the right atomic types, and half-precision math keeps half types.
	#[compio::test]
	async fn modern_half_and_integer_atomics_lower_to_relaxed_msl() {
		let source = r#"
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
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected modern MSL source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected modern MSL source generation");

		assert_string_contains!(shader, "threadgroup atomic_uint unsigned_value;");
		// Signed atomics must stay signed so `atomic_min` compares as signed integers.
		assert_string_contains!(shader, "threadgroup atomic_int signed_value;");
		for operation in [
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
		] {
			assert_string_contains!(shader, operation);
		}
		assert_string_contains!(shader, "memory_order_relaxed");
		assert_string_contains!(shader, "half fused=fma(");
		assert_string_contains!(shader, "half3 fused_vector=fma(");
		for predicate in ["isnan(", "isinf(", "isfinite(", "isnormal("] {
			assert_string_contains!(shader, predicate);
		}

		compile_natively(&shader, "besl-modern-half-atomics").await;
	}

	/// Verifies BESL intrinsics without a same-named Metal function lower to their Metal equivalents.
	#[compio::test]
	async fn intrinsics_lower_to_valid_msl_names() {
		let source = r#"
		main: fn () -> void {
			let angle: f32 = radians(180.0);
			let inverse: f32 = inversesqrt(4.0);
			let trigonometry: vec2f = sincos(angle);
			let fused: vec2f = fma(vec2f(2.0, 3.0), vec2f(4.0, 5.0), vec2f(1.0, 2.0));
			let rounded: vec2i = round_to_i32(vec2f(0.0 - 1.6, 2.4));
			angle;
			inverse;
			trigonometry;
			fused;
			rounded;
		}
		"#;

		let root = besl::compile_to_besl(source, None).expect("Expected intrinsic shader source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected intrinsic MSL generation");
		assert_string_contains!(shader, "float angle=(180.0*(PI/180.0));");
		assert_string_contains!(shader, "rsqrt(4.0)");
		assert_string_contains!(shader, "float2 trigonometry=_besl_sincos(angle);");
		assert_string_contains!(shader, "float2 fused=fma(float2(2.0,3.0),float2(4.0,5.0),float2(1.0,2.0));");
		// Rounds half away from zero like BESL, then converts, instead of truncating.
		assert_string_contains!(shader, "int2 rounded=int2(round(float2(0.0-1.6,2.4)));");

		compile_natively(&shader, "besl-intrinsic-names").await;
	}

	/// Verifies `fetch` reads an exact texel with `read` instead of filtering through the sampler.
	#[compio::test]
	async fn fetch_intrinsic_lowers_to_msl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
			texel;
		}
		"#;

		let mut root = besl::Node::root();
		root.add_child(sampled_binding("texture", 0, true, false));
		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(8)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected fetch MSL generation");
		assert_string_contains!(shader, "float4 texel=resources.texture.read(coord);");

		compile_natively(&shader, "besl-fetch").await;
	}

	/// Verifies a global scalar-array constant is placed in Metal's `constant` address space with its vector spelling.
	#[compio::test]
	async fn const_array_variable_lowers_to_msl() {
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
			.expect("Expected const-array MSL generation");
		assert_string_contains!(shader, "constant float3 WEIGHTS = float3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");

		compile_natively(&shader, "besl-const-array").await;
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
		let root = besl::compile_to_besl(source, None).expect("Expected sample source to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected sample source to lower to Metal");
		assert_string_contains!(
			shader,
			"resources.image_texture.sample(resources.image_texture_sampler, in_uv)"
		);

		compile_natively(&shader, "besl-sample-intrinsic").await;
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
		let root = besl::compile_to_besl(script, None).expect("Expected explicit fragment output shader to link");
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::fragment(),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected explicit fragment output MSL generation");
		assert_string_contains!(shader, "fragment FragmentOutput besl_main(FragmentInput in [[stage_in]])");
		assert_string_contains!(shader, "return FragmentOutput{float4(1.0,0.0,0.0,1.0)};");

		compile_natively(&shader, "besl-explicit-fragment-output").await;
	}
}
