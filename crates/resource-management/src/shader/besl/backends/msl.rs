use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fmt::Write as _,
	vec::Vec,
};

pub use Generator as MSLShaderGenerator;

use crate::shader::generator::{
	emit_comma_separated_nodes, emit_statement_block, ordered_shader_nodes_in, MatrixLayouts, NodeEmitter, ShaderFormatting,
	ShaderGenerationSettings, ShaderGenerator, Stages,
};

mod bindings;
mod emit;
mod facade;
mod generate;
mod node_emitter;
mod raster;

pub(crate) use bindings::*;
pub(crate) use emit::*;
pub(crate) use facade::*;
pub use facade::{ComputeBindingMode, DownsampleStrategy, Generator};
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

	#[test]
	fn sampled_binding_array_argument_is_emitted_in_resources() {
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

		assert_string_contains!(shader, "texture2d<float> textures [[id(0)]][4];");
		assert_string_contains!(shader, "sampler textures_sampler [[id(4)]][4];");
		assert_string_contains!(shader, "resources.textures[0].sample(resources.textures_sampler[0]");
	}

	#[test]
	fn sampled_binding_array_argument_uses_bare_compute_resources() {
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
		let shader = Generator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&root.get_main().expect("Expected main"),
			)
			.expect("Expected bare-resource sampled binding array MSL generation");

		assert_string_contains!(shader, "textures[0].sample(textures_sampler[0]");
		assert!(!shader.contains("resources.textures"));
	}

	fn main_with(statements: Vec<besl::NodeReference>) -> besl::NodeReference {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		besl::Node::function("main", Vec::new(), void, statements).into()
	}

	#[test]
	fn intrinsic_definition_only_bindings_do_not_shift_dense_argument_ids() {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		let intrinsic: besl::NodeReference = besl::Node::intrinsic(
			"instantiated_binding_fixture",
			vec![sampled_binding("definition_only", 0, true, false)],
			void.clone(),
		)
		.into();
		let call = besl::Node::expression(besl::Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: vec![sampled_binding("instantiated", 100, true, false)],
		})
		.into();
		let main: besl::NodeReference = besl::Node::function("main", Vec::new(), void, vec![call]).into();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected instantiated intrinsic binding generation");

		assert_string_contains!(shader, "texture2d<float> instantiated [[id(0)]];");
		assert_string_contains!(shader, "sampler instantiated_sampler [[id(1)]];");
		assert!(!shader.contains("definition_only"));
	}

	#[test]
	fn distinct_reachable_declarations_cannot_reuse_a_flat_slot() {
		let main = main_with(vec![
			sampled_binding("first", 4, true, false),
			sampled_binding("second", 4, false, true),
		]);

		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Distinct declarations at one flat slot must be rejected before MSL emission"
		);
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
	fn dense_metal_argument_id_ranges_cannot_overflow() {
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
			"Packed Metal argument IDs must not wrap"
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

	#[test]
	fn vec4u16_uses_the_native_msl_packed_storage_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 MSL generation");

		assert_string_contains!(shader, "struct _buff{packed_ushort4 value;};");
		assert!(!shader.contains("struct vec4u16"));
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
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-packed-vec4f")
			.await
			.expect("Expected packed_vec4f storage lowering to compile natively");
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

		assert_string_contains!(vec2_array, "struct _buff{packed_ushort2 values[2];};");
		assert_string_contains!(mixed_vec4, "struct _buff{packed_ushort4 value;ushort tail;};");
	}

	#[test]
	fn vec2f16_arrays_use_packed_msl_storage() {
		let shader = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2f16_array_binding(),
			)
			.expect("Expected vec2f16 MSL generation");

		assert_string_contains!(shader, "struct _buff{packed_half2 values[2];};");
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

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-f16-storage")
			.await
			.expect("Expected native f16 MSL source to compile");
	}

	#[test]
	fn generator_accepts_custom_allocator() {
		let main = generator::tests::bindings();

		let shader = Generator::new_in(std::alloc::System)
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader with custom allocator");

		assert_string_contains!(shader, "struct _buff{float member;};");
	}

	#[test]
	fn generate_accepts_call_scoped_allocator() {
		let main = generator::tests::bindings();
		let mut generator = Generator::new_in(std::alloc::System).minified(true);

		let shader = generator
			.generate_in(&ShaderGenerationSettings::vertex(), &main, std::alloc::System)
			.expect("Failed to generate shader with call-scoped allocator");

		assert_string_contains!(shader, "struct _buff{float member;};");
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
			"struct _resources{device _buff* buff [[id(0)]];texture2d<float, access::write> image [[id(1)]];texture2d<float> texture [[id(2)]];sampler texture_sampler [[id(3)]];};"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint2 gid [[thread_position_in_grid]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "resources.buff;resources.image;resources.texture;");
		assert!(
			!shader.contains("_besl_downsample_"),
			"Native sampler reduction must not emit unused gather fallback helpers: {shader}"
		);
	}

	#[test]
	fn unused_shader_gather_fallback_helpers_are_not_emitted() {
		let shader = Generator::new()
			.minified(true)
			.downsample_strategy(DownsampleStrategy::ShaderGather)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(8)),
				&generator::tests::bindings(),
			)
			.expect("Expected MSL without downsampling to generate");

		assert!(
			!shader.contains("_besl_downsample_"),
			"Unused shader-gather fallbacks increased generated MSL size: {shader}"
		);
	}

	#[compio::test]
	async fn texture_lod_qualifies_metal_level_helper() {
		let source = r#"
			depth_texture: descriptor<Texture2D, 0, read>;
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

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-texture-lod-level-shadowing")
			.await
			.expect("Expected qualified Metal level helper to compile when a BESL parameter is named level");
	}

	#[compio::test]
	async fn sample_intrinsic_lowers_to_a_texture_sample_call() {
		let source = r#"
			image_texture: descriptor<Texture2D, 0, read>;
			in_uv: input<vec2f, 0>;
			out_color_attachment: output<vec4f, 0>;
			main: fn() -> void {
				out_color_attachment = sample(image_texture, in_uv);
			}
		"#;
		let root = besl::compile_to_besl(source, None).expect("Expected sample source to link");
		let main = root.get_main().expect("Expected sample source to define main");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Expected sample source to lower to Metal");

		assert_string_contains!(
			shader,
			"resources.image_texture.sample(resources.image_texture_sampler, in_uv)"
		);

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-sample-intrinsic")
			.await
			.expect("Expected sample intrinsic MSL to compile");
	}

	#[compio::test]
	async fn conservative_downsampling_defaults_to_native_sampler_reduction_and_keeps_a_gather_fallback() {
		let source = r#"
			depth_texture: descriptor<Texture2D, 0, read>;
			array_depth_texture: descriptor<Texture2DArray, 1, read>;
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
		let fallback = Generator::new()
			.minified(true)
			.downsample_strategy(DownsampleStrategy::ShaderGather)
			.generate(&settings, &main)
			.expect("Expected gather fallback MSL");
		let native = Generator::new()
			.minified(true)
			.generate(&settings, &main)
			.expect("Expected native sampler-reduction MSL");

		assert_string_contains!(
			fallback,
			"_besl_downsample_min(resources.depth_texture, resources.depth_texture_sampler"
		);
		assert_string_contains!(
			fallback,
			"_besl_downsample_max(resources.depth_texture, resources.depth_texture_sampler"
		);
		assert_string_contains!(fallback, ".gather(texture_sampler, uv, int2(0), component::x)");
		assert_string_contains!(fallback, ".gather(texture_sampler, uv, layer, int2(0), component::x)");
		assert_string_contains!(fallback, "texture.read(a, level).x");
		assert_string_contains!(
			native,
			".sample(resources.depth_texture_sampler, float2(0.5,0.5), metal::level(0.0)).x"
		);
		assert_string_contains!(
			native,
			".sample(resources.array_depth_texture_sampler, float2(0.5,0.5), 1, metal::level(0.0)).x"
		);

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&fallback, "besl-downsample-gather")
			.await
			.expect("Expected gather fallback MSL to compile natively");
	}

	#[compio::test]
	async fn buffer_memory_classes_select_metal_address_spaces() {
		let source = r#"
			DispatchValues: struct { value: u32, }
			Vertices: struct { values: u32[1024], }
			Counters: struct { values: u32[1024], }
			dispatch_values: descriptor<DispatchValues, 0, read, constant>;
			vertices: descriptor<Vertices, 1, read, device>;
			counters: descriptor<Counters, 2, read_write, device>;
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
		assert_string_contains!(argument_buffer_shader, "const device _vertices* vertices [[id(1)]];");
		assert_string_contains!(argument_buffer_shader, "device _counters* counters [[id(2)]];");

		let bare_resource_shader = Generator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(&settings, &main)
			.expect("Expected memory-class source to lower through bare Metal resources");
		assert_string_contains!(
			bare_resource_shader,
			"constant _dispatch_values* dispatch_values [[buffer(0)]]"
		);
		assert_string_contains!(bare_resource_shader, "const device _vertices* vertices [[buffer(1)]]");
		assert_string_contains!(bare_resource_shader, "device _counters* counters [[buffer(2)]]");

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&argument_buffer_shader,
			"besl-buffer-memory-classes",
		)
		.await
		.expect("Expected generated memory-class MSL to compile natively");
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

	#[test]
	fn same_named_buffer_members_lower_to_msl() {
		let main = generator::tests::same_named_buffer_member_access();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "resources.pixel_mapping->pixel_mapping[0]");
		assert_string_contains!(shader, "resources.meshes->meshes[1]");
	}

	#[test]
	fn buffer_vector_arrays_use_packed_msl_types() {
		let script = r#"
		main: fn () -> void {
			let position: vec3f = positions.values[0];
			let uv: vec2f = uvs.values[0];
			position;
			uv;
		}
		"#;

		let mut root = besl::parse(script).expect("Expected packed buffer array test shader source to parse");
		root.add(vec![
			besl::parser::Node::binding(
				"positions",
				besl::parser::Node::buffer("Positions", vec![besl::parser::Node::member("values", "vec3f[8]")]),
				0,
				true,
				false,
			),
			besl::parser::Node::binding(
				"uvs",
				besl::parser::Node::buffer("Uvs", vec![besl::parser::Node::member("values", "vec2f[8]")]),
				1,
				true,
				false,
			),
		]);
		let root = besl::lex(root).expect(
			"Expected packed buffer array test shader source to lex. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct _positions{packed_float3 values[8];};");
		assert_string_contains!(shader, "struct _uvs{packed_float2 values[8];};");
	}

	#[test]
	fn non_buffer_vector_arrays_keep_standard_msl_types() {
		let script = r#"
		VertexBlock: struct {
			positions: vec3f[4],
		}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(script, None).expect(
			"Expected non-buffer vector array test shader source to compile. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		let vertex_block = RefCell::borrow(&root)
			.get_child("VertexBlock")
			.expect("Expected VertexBlock struct");

		{
			let mut main = main.borrow_mut();
			main.add_child(
				besl::Node::raw(
					Some("VertexBlock;".to_string()),
					Some("VertexBlock;".to_string()),
					Some("VertexBlock;".to_string()),
					vec![vertex_block],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexBlock{float3 positions[4];};");
		assert!(
			!shader.contains("packed_float3 positions[4]"),
			"Expected non-buffer vector arrays to keep standard MSL vector types"
		);
	}

	#[test]
	fn intrinsics_lower_to_valid_msl_names() {
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

		let root = besl::compile_to_besl(source, None).expect(
			"Expected intrinsic test shader source to compile. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float angle=(180.0*(PI/180.0));");
		assert_string_contains!(shader, "rsqrt(4.0)");
		assert_string_contains!(shader, "float2 trigonometry=_besl_sincos(angle);");
		assert_string_contains!(shader, "float2 fused=fma(float2(2.0,3.0),float2(4.0,5.0),float2(1.0,2.0));");
		assert_string_contains!(shader, "int2 rounded=int2(round(float2(0.0-1.6,2.4)));");
	}

	#[test]
	fn user_struct_constructors_lower_to_aggregate_initialization() {
		let mut root = besl::Node::root();
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_child(
			besl::Node::r#struct(
				"Pair",
				vec![
					besl::Node::member("left", vec4f.clone()).into(),
					besl::Node::member("right", vec4f).into(),
				],
			)
			.into(),
		);
		let root = besl::compile_to_besl(
			"main: fn () -> void { let pair: Pair = Pair(vec4f(1.0, 1.0, 1.0, 1.0), vec4f(2.0, 2.0, 2.0, 2.0)); pair; }",
			Some(root),
		)
		.expect("Expected user struct constructor shader to compile");
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "Pair pair=Pair{float4(1.0,1.0,1.0,1.0),float4(2.0,2.0,2.0,2.0)};");
	}

	const TASK_PAYLOAD_FIXTURE_SOURCE: &str = r#"
		Meshlets: struct {
			values: u32[32],
		}
		meshlets: descriptor<Meshlets, 8, read>;
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

	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let root = besl::compile_to_besl(source, None).expect("Expected stage fixture source to link");
		let main = root.get_main().expect("Expected stage fixture main function");
		Generator::new()
			.minified(true)
			.generate(settings, &main)
			.expect("Expected stage fixture to lower to MSL")
	}

	#[test]
	fn compute_stage_lowers_counted_workgroup_storage_through_helpers() {
		let shader = lower_fixture(
			COMPUTE_WORKGROUP_FIXTURE_SOURCE,
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
		);

		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "threadgroup float scratch[64];");
		assert_string_contains!(shader, "threadgroup float* scratch");
		assert_string_contains!(shader, "threadgroup_barrier(mem_flags::mem_threadgroup)");
		assert_string_contains!(shader, "store_scratch(float(thread_index),gid,thread_index,scratch)");
	}

	#[test]
	fn compute_subgroup_intrinsics_lower_to_metal_simdgroup_operations() {
		let shader = lower_fixture(
			r#"
			scratch: workgroup<u32, 1>;

			main: fn () -> void {
				let mask: vec4u = subgroup_ballot(thread_idx() < 4);
				let leader: u32 = subgroup_ballot_find_lsb(mask);
				let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
				let remaining: vec4u = subgroup_ballot_and_not(mask, subgroup_ballot(value == 0));
				if (subgroup_ballot_any(remaining)) {
					scratch[0] = subgroup_ballot_count(remaining);
				}
			}
			"#,
			&ShaderGenerationSettings::compute(utils::Extent::line(32)),
		);

		assert_string_contains!(shader, "simd_ballot(predicate)");
		assert_string_contains!(shader, "simd_broadcast(value, ushort(source_lane))");
		assert_string_contains!(shader, "_besl_subgroup_ballot_find_lsb(mask)");
		assert_string_contains!(shader, "_besl_subgroup_ballot_count(remaining)");
		assert_string_contains!(shader, "threadgroup uint scratch[1]");
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

		assert_string_contains!(shader, "uint lane(uint2 gid,uint simd_lane_id)");
		assert_string_contains!(shader, "lane(gid,simd_lane_id)");
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
	fn task_stage_lowers_workgroup_storage_payload_and_mesh_dispatch() {
		let shader = lower_fixture(
			TASK_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);

		assert_string_contains!(shader, "// #pragma shader_stage(object)");
		assert_string_contains!(shader, "// besl-threadgroup-size:32,1,1");
		assert_string_contains!(shader, "struct ObjectPayload{uint visible_meshlets[32];};");
		assert_string_contains!(shader, "[[object, max_total_threadgroups_per_mesh_grid(32)]] void besl_main(");
		assert_string_contains!(shader, "uint thread_position [[thread_position_in_grid]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "object_data ObjectPayload& payload [[payload]]");
		assert_string_contains!(shader, "mesh_grid_properties mesh_grid");
		assert_string_contains!(shader, "threadgroup atomic_uint visible_count;");
		assert_string_contains!(shader, "threadgroup_barrier(mem_flags::mem_threadgroup)");
		assert_string_contains!(shader, "payload.visible_meshlets[payload_index]");
		assert_string_contains!(shader, "mesh_grid.set_threadgroups_per_grid(uint3(");
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

	#[test]
	fn matrix_and_vector_index_access_uses_msl_subscripts() {
		let shader = lower_fixture(
			r#"
			main: fn() -> void {
				let matrix: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(0.0, 0.0, 0.0, 1.0)
				);
				let column: vec4f = matrix[0];
				let element: f32 = column[1];
				element;
			}
			"#,
			&ShaderGenerationSettings::vertex(),
		);

		assert_string_contains!(shader, "matrix[0]");
		assert_string_contains!(shader, "column[1]");
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
				transforms: descriptor<Transforms, 0, read_write, device>;

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

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-packed-mat4x3")
			.await
			.expect("Expected packed mat4x3 storage lowering to compile natively");
	}

	#[cfg(target_os = "macos")]
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

		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&task, "besl-task-payload-fixture")
			.await
			.expect("Expected generated task MSL to compile natively");
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&mesh, "besl-mesh-payload-fixture")
			.await
			.expect("Expected generated mesh MSL to compile natively");
	}

	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn generated_compute_workgroup_stage_compiles_with_metal() {
		let shader = lower_fixture(
			COMPUTE_WORKGROUP_FIXTURE_SOURCE,
			&ShaderGenerationSettings::compute(utils::Extent::square(8)),
		);

		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-compute-workgroup-fixture")
			.await
			.expect("Expected generated compute workgroup MSL to compile natively");
	}

	#[cfg(target_os = "macos")]
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

		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-compute-subgroup-fixture")
			.await
			.expect("Expected generated compute subgroup MSL to compile natively");
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

	#[test]
	fn compute_shaders_emit_threadgroup_metadata() {
		let source = "main: fn () -> void { let coord: vec3u = thread_id(); }";
		let root = besl::parse(source).unwrap();
		let root = besl::lex(root).unwrap();
		let main_node = root.get_main().unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
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

	#[test]
	fn input() {
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexInput{float3 color [[attribute(0)]];};");
		assert_string_contains!(shader, "vertex VertexOutput besl_main(VertexInput in [[stage_in]])");
		assert_string_contains!(shader, "float3 color=in.color;");
		assert_string_contains!(shader, "color;return out;");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];float3 color [[user(locn0)]];};"
		);
		assert_string_contains!(shader, "vertex VertexOutput besl_main(VertexInput in [[stage_in]])");
		assert_string_contains!(shader, "float3 color;color;out.color=color;return out;");
	}

	#[test]
	fn vertex_builtin_stage_inputs_lower_to_msl_semantics() {
		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_child(besl::Node::input("vertex_id", u32_type.clone(), 0).into());
		root.add_child(besl::Node::input("instance_id", u32_type, 1).into());

		let root = besl::compile_to_besl("main: fn () -> void { vertex_id; instance_id; }", Some(root)).unwrap();
		let main = root.borrow().get_child("main").unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexInput{};");
		assert_string_contains!(shader, "uint vertex_id [[vertex_id]],uint instance_id [[instance_id]]");
		assert!(!shader.contains("uint vertex_id=vertex_id;"));
		assert!(!shader.contains("uint instance_id=instance_id;"));
	}

	#[test]
	fn fragment_explicit_output_struct_return_lowers_to_msl_entry_return() {
		let script = r#"
		FragmentOutput: struct {
			color: vec4f,
		}

		main: fn () -> FragmentOutput {
			return FragmentOutput(vec4f(1.0, 0.0, 0.0, 1.0));
		}
		"#;
		let root = besl::compile_to_besl(script, None).expect("Expected explicit fragment output shader to lex");
		let main = root.borrow().get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct FragmentInput{};");
		assert_string_contains!(shader, "struct FragmentOutput{float4 color;};");
		assert_string_contains!(shader, "fragment FragmentOutput besl_main(FragmentInput in [[stage_in]])");
		assert_string_contains!(shader, "return FragmentOutput{float4(1.0,0.0,0.0,1.0)};");
	}

	#[test]
	fn fwidth_intrinsic_lowers_to_msl() {
		let program = besl::compile_to_besl("main: fn() -> void { let edge_width: f32 = fwidth(1.0); edge_width; }", None)
			.expect("Failed to compile fwidth BESL shader");
		let main = program.get_main().expect("Expected fwidth BESL shader main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate fwidth MSL shader");

		assert_string_contains!(shader, "fwidth(1.0)");
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
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){float3 albedo=float3(1.0,0.0,0.0);albedo;}");
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
			besl::parser::Node::buffer("CamerasBuffer", vec![besl::parser::Node::member("cameras", "Camera[8]")]),
			0,
			true,
			false,
		);
		let main = besl::parser::Node::main_function(vec![besl::parser::Node::raw_code(
			Some("".into()),
			None,
			Some(
				"position = resources.cameras->cameras[0].view_projection * float4(in_position, 1.0); out_instance_index = 0u;"
					.into(),
			),
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

		assert_string_contains!(shader, "struct _cameras{Camera cameras[8];};");
		assert_string_contains!(shader, "struct _resources{constant _cameras* cameras [[id(0)]];};");
		assert_string_contains!(shader, "struct VertexInput{float3 in_position [[attribute(0)]];};");
		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];uint out_instance_index [[flat]] [[user(locn0)]];};"
		);
		assert_string_contains!(
			shader,
			"vertex VertexOutput besl_main(VertexInput in [[stage_in]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "position = resources.cameras->cameras[0].view_projection");
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
			"float4x4 camera_matrix(constant _resources& resources){return resources.cameras->cameras[0].view_projection;}"
		);
		assert_string_contains!(
			shader,
			"position=(camera_matrix(resources)*float4(in_position.x,in_position.y,in_position.z,1.0));"
		);
	}

	#[test]
	fn fetch_intrinsic_lowers_to_msl() {
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

		assert_string_contains!(shader, "float4 texel=resources.texture.read(coord);");
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
	fn structure() {
		let main = generator::tests::structure();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct Vertex{float3 position;float3 normal;};Vertex use_vertex(){}void main(){use_vertex();}"
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
	fn test_msl() {
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
		assert_string_contains!(shader, "void main(){output.position = float4(0, 0, 0, 1);}");
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
	fn matrix_on_both_sides_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f) -> mat4f {
			return projection * model;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-matrix shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4x4 main(float4x4 projection,float4x4 model){return projection*model;}"
		);
	}

	#[test]
	fn matrix_and_vector_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f) -> vec4f {
			return projection * position;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-vector shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position){return projection*position;}"
		);
	}

	#[test]
	fn chained_matrix_vector_scalar_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f, scale: f32) -> vec4f {
			return projection * position * scale;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected chained multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position,float scale){return (projection*position)*scale;}"
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

		// MSL generator should use the explicit MSL code
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

		assert_string_contains!(shader, "constant float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
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

	#[test]
	fn mesh_output_assignments_lower_to_msl_primitive_outputs() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
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
		let out_instance_index = besl::parser::Node::output_array("out_instance_index", "u32", 0, 126);
		let out_primitive_index = besl::parser::Node::output_array("out_primitive_index", "u32", 1, 126);
		let script = r#"
		main: fn () -> void {
			out_instance_index[0] = 7;
			out_primitive_index[0] = 9;
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![
			push_constant,
			mesh_output_types,
			out_instance_index,
			out_primitive_index,
		]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "out_mesh.set_primitive(0, PrimitiveOutput{");
		assert_string_contains!(shader, ".instance_index = 7");
		assert_string_contains!(shader, ".primitive_index = 9");
	}

	#[test]
	fn mesh_stage_user_functions_do_not_receive_hidden_context_parameters() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let meshlets = besl::parser::Node::binding(
			"meshlets",
			besl::parser::Node::buffer("MeshletBuffer", vec![besl::parser::Node::member("count", "u32")]),
			0,
			true,
			false,
		);
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
		let mut parsed_shader = besl::parse(
			r#"
			helper: fn () -> void {
				meshlets.count;
				threadgroup_position();
				thread_idx();
				set_mesh_output_counts(3, 1);
			}

			main: fn () -> void {
				helper();
			}
			"#,
		)
		.expect("Expected mesh helper shader to parse");
		let parsed_children = match parsed_shader.node_mut() {
			besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
			_ => panic!(
				"Expected mesh helper shader to parse into a scope. The most likely cause is invalid BESL syntax in the mesh helper shader test."
			),
		};
		let mut shader = besl::parser::Node::root();
		shader.add(vec![meshlets, push_constant, mesh_output_types]);
		shader.add(parsed_children);
		let root = besl::lex(shader).expect("Expected mesh helper shader to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void helper()");
		assert_string_contains!(shader, "helper();");
		assert!(!shader.contains("void helper(constant _resources& resources"));
		assert!(!shader.contains("helper(resources,threadgroup_position,thread_index,out_mesh);"));
	}

	#[test]
	fn conditional_blocks_lower_to_msl() {
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
	fn bitwise_operators_lower_to_msl() {
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

		assert_string_contains!(shader, "uint packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_msl() {
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

		assert_string_contains!(shader, "for(uint i=0;i<=4;i=(i+1)){if(i>=2){continue;};};");
	}

	#[test]
	fn scalar_max_and_clamp_lower_to_msl() {
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
	fn const_array_variable_lowers_to_msl() {
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

		assert_string_contains!(shader, "constant float3 WEIGHTS = float3(0.5,0.25,0.125);");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
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
			let sum: f32 = floats[1] + f32(shorts[1]) + f32(indices[1]);
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

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-short-scalar-arrays")
			.await
			.expect("Expected vector-backed scalar arrays to compile as MSL");
	}

	#[compio::test]
	async fn source_declared_atomic_images_and_push_constants_lower_to_msl() {
		let source = r#"
			Counters: struct {
				values: atomicu32[8],
			}
			counters: descriptor<Counters, 2, read_write>;
			index_image: descriptor<StorageImage<r32ui>, 4, read>;
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

		assert_string_contains!(shader, "atomic_uint values[8]");
		assert_string_contains!(shader, "texture2d<uint, access::read> index_image");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, ".read(coord).x");
		assert_string_contains!(shader, "atomic_fetch_add_explicit(&");
		assert_string_contains!(
			shader,
			"_besl_atomic_compare_exchange(resources.counters->values[index],old,7)"
		);
		assert_string_contains!(shader, "_besl_atomic_compare_exchange(shared_keys[index%8],4294967295,index)");
		assert_string_contains!(
			shader,
			"while (!atomic_compare_exchange_weak_explicit(&value, &expected, desired"
		);
		assert_string_contains!(shader, "atomic_load_explicit(&");
		assert_string_contains!(shader, "atomic_store_explicit(&");

		#[cfg(target_os = "macos")]
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&shader, "besl-atomic-compare-exchange")
			.await
			.expect("Expected compare-exchange MSL to compile natively");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_msl() {
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
