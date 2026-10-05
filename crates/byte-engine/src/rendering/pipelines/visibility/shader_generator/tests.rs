use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value};
use ghi::AccessPolicies;
use resource_management::asset::handler::implementations::bema::ProgramGenerator;

use super::super::tests::{ssgi_projection, ssgi_ray_at};
use super::*;
use crate::rendering::shader_vm_test::{buffer, column_major, compile, run_at, texture_2d};

macro_rules! material_metadata {
	($($json:tt)*) => {
		serde_json::json!({ $($json)* })
			.as_object()
			.expect("test material metadata should be an object")
			.clone()
	};
}

/// The access declaration used when baking material-evaluation shaders.
fn material_generator() -> VisibilityShaderGenerator {
	VisibilityShaderGenerator::with_access(ScopeAccess {
		material_count: AccessPolicies::READ,
		material_offset: AccessPolicies::READ,
		material_offset_scratch: AccessPolicies::NONE,
		pixel_mapping: AccessPolicies::READ,
	})
}

/// Parses `source` as a `main`, adds `nodes`, such as helper functions and bindings, and compiles the result for the
/// VM.
fn compile_with(source: &str, nodes: Vec<Node<'static>>) -> besl::vm::ExecutableProgram {
	let mut root = besl::parse(source).expect("Failed to parse a VM test. The most likely cause is invalid BESL test syntax.");
	root.add(nodes);
	compile(
		besl::lex(root).expect("Failed to lex a VM test. The most likely cause is an unresolved portable helper operation."),
	)
}

fn results_binding(members: Vec<Node<'static>>, slot: ResourceSlot) -> Node<'static> {
	Node::binding("results", Node::buffer(members), slot.slot(), false, true)
}

/// Declares one buffer member of type `r#type` per name, in the order given, which sets the buffer layout.
fn members(r#type: &str, names: &[&'static str]) -> Vec<Node<'static>> {
	names.iter().map(|name| Node::member(name, r#type)).collect()
}

fn read_f32(results: &besl::vm::Buffer, name: &str) -> f32 {
	results.read_f32(name).expect("VM result")
}

/// Runs `source` as `main` with the `helpers` functions it calls, `textures` bound from slot one on, and a `results`
/// buffer of `members` at slot zero, and returns the results.
///
/// Helper tests use it to execute production BESL helper functions on fixed inputs.
fn run_helper_test(
	source: &str,
	textures: &mut [(&'static str, besl::parser::BindingResource<'static>, &mut Texture)],
	helpers: &[(&'static str, &str)],
	members: Vec<Node<'static>>,
) -> besl::vm::Buffer {
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(0);
	let mut nodes = (1..)
		.zip(textures.iter())
		.map(|(slot, (name, resource, _))| Node::binding(name, resource.clone(), slot, true, false))
		.chain(helpers.iter().map(|(source, name)| parse_besl_function(source, name)))
		.collect::<Vec<_>>();
	nodes.push(results_binding(members, RESULT_SLOT));
	let executable = compile_with(source, nodes);
	let mut results = buffer(&executable, RESULT_SLOT);
	let mut descriptors = DescriptorBindings::new();
	for (slot, (_, _, texture)) in (1..).zip(textures) {
		descriptors.bind_texture(ResourceSlot::new(slot), texture);
	}
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);
	results
}

/// Executes representative octahedral seams and axes through the optimized production decoder.
#[test]
fn octahedral_decoder_preserves_normal_directions_in_the_besl_vm() {
	const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
	let executable = compile_with(
		r#"
		main: fn () -> void {
			for (let index: u32 = 0; index < 5; index = index + 1) {
				results.values[index] = normalize(decode_octahedral_normal(inputs.values[index]));
			}
		}
		"#,
		vec![
			parse_besl_function(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
			Node::binding(
				"inputs",
				Node::buffer(vec![Node::member("values", "vec2u16[5]")]),
				INPUT_SLOT.slot(),
				true,
				false,
			),
			results_binding(vec![Node::member("values", "vec3f[5]")], RESULT_SLOT),
		],
	);
	let cases = [
		([32768, 32768], [0.0, 0.0, 1.0]),
		([65535, 32768], [1.0, 0.0, 0.0]),
		([0, 32768], [-1.0, 0.0, 0.0]),
		([32768, 65535], [0.0, 1.0, 0.0]),
		([65535, 65535], [0.0, 0.0, -1.0]),
	];
	let mut inputs = buffer(&executable, INPUT_SLOT);
	let mut results = buffer(&executable, RESULT_SLOT);
	for (index, (encoded, _)) in cases.iter().enumerate() {
		inputs
			.write_array_element(index, Value::Vec2U16(*encoded))
			.expect("octahedral input");
	}
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(INPUT_SLOT, &mut inputs);
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	for (index, (encoded, expected)) in cases.iter().enumerate() {
		let Value::Vec3F(actual) = results.read_array_element(index).expect("decoded normal") else {
			panic!("Unexpected decoded-normal type.");
		};
		assert!(
			actual
				.iter()
				.zip(expected)
				.all(|(actual, expected)| (actual - expected).abs() <= 0.00005),
			"Unexpected decoded normal {actual:?} for {encoded:?}. The most likely cause is incorrect octahedral fold math."
		);
	}
}

/// Verifies the packed C0 tangent defines Type C horizontal angles without a world-axis singularity.
#[test]
fn ies_profile_uv_uses_the_uploaded_orientation_frame_in_the_besl_vm() {
	const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
	let executable = compile_with(
		r#"
		main: fn () -> void {
			for (let index: u32 = 0; index < 5; index = index + 1) {
				results.values[index] = ies_profile_uv(
					inputs.emission_directions[index],
					inputs.axes[index],
					inputs.c0_tangents[index]
				);
			}
		}
		"#,
		vec![
			parse_besl_function(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
			parse_besl_function(IES_PROFILE_UV_SOURCE, "ies_profile_uv"),
			Node::binding(
				"inputs",
				Node::buffer(vec![
					Node::member("emission_directions", "vec3f[5]"),
					Node::member("axes", "vec3f[5]"),
					Node::member("c0_tangents", "vec2u16[5]"),
				]),
				INPUT_SLOT.slot(),
				true,
				false,
			),
			results_binding(vec![Node::member("values", "vec2f[5]")], RESULT_SLOT),
		],
	);
	let cases = [
		([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [65535, 32768], [0.0, 0.5]),
		([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [65535, 32768], [0.25, 0.5]),
		([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [32768, 65535], [0.0, 0.5]),
		([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [32768, 65535], [0.25, 0.5]),
		([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [65535, 32768], [0.0, 0.5]),
	];
	let mut inputs = buffer(&executable, INPUT_SLOT);
	let mut results = buffer(&executable, RESULT_SLOT);
	for (index, (emission_direction, axis, c0_tangent, _)) in cases.iter().enumerate() {
		inputs
			.write_indexed("emission_directions", index, Value::Vec3F(*emission_direction))
			.expect("IES emission direction");
		inputs.write_indexed("axes", index, Value::Vec3F(*axis)).expect("IES axis");
		inputs
			.write_indexed("c0_tangents", index, Value::Vec2U16(*c0_tangent))
			.expect("IES C0 tangent");
	}
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(INPUT_SLOT, &mut inputs);
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	for (index, (_, _, _, expected)) in cases.iter().enumerate() {
		let Value::Vec2F(actual) = results.read_array_element(index).expect("IES UV") else {
			panic!("Unexpected IES UV type.");
		};
		// C0 lies on the duplicated horizontal seam, so packed-vector rounding may wrap a value just below zero to one.
		let horizontal_delta = (actual[0] - expected[0]).abs();
		let horizontal_delta = horizontal_delta.min(1.0 - horizontal_delta);
		assert!(
			horizontal_delta <= 0.0001 && (actual[1] - expected[1]).abs() <= 0.0001,
			"Unexpected IES UV {actual:?}. The most likely cause is incorrect C0-frame coordinate mapping."
		);
	}
}

#[test]
fn vec4f_variable_becomes_specialization() {
	let material = material_metadata! {
		"variables": [{ "name": "albedo", "data_type": "vec4f" }]
	};
	let shader_node = besl::parse("main: fn () -> void { out_color = albedo; }").expect("test shader");

	let shader = VisibilityShaderGenerator::new().transform(shader_node, &material);

	let besl::parser::Nodes::Scope { children, .. } = shader.node() else {
		panic!("Expected generated material root scope.");
	};
	let child = |name| children.iter().find(|child| child.name() == Some(name));
	let specialization = child("albedo").expect("Generated material program should declare the vec4f variable.");
	assert!(matches!(
		specialization.node(),
		besl::parser::Nodes::Specialization { r#type, .. } if *r#type == "vec4f"
	));
	let main = child("main").expect("Generated material program should contain main.");
	let besl::parser::Nodes::Function { statements, .. } = main.node() else {
		panic!("Expected generated material main function.");
	};
	assert!(statements.iter().any(|statement| {
		matches!(
			statement.node(),
			besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment
					&& matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "out_color")
					&& matches!(right.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "albedo")
		)
	}));
}

#[test]
fn material_evaluation_texture_variables_produce_valid_besl() {
	let material = material_metadata! {
		"variables": [
			{ "name": "base_color", "data_type": "Texture2D" },
			{ "name": "normal_map", "data_type": "Texture2D" }
		]
	};
	let shader_node =
		besl::parse("main: fn () -> void { albedo = sample_material(base_color); normal = sample_normal(normal_map); }")
			.expect("test shader");
	let shader = material_generator().transform(shader_node, &material);
	besl::lex(shader).expect("generated normal-mapped program should link");
}

/// Verifies the generated material evaluation program lowers to the running platform's shader language.
///
/// Linking only proves the BESL is well formed. The generated program reaches the GPU through a platform
/// backend, and defects that BESL accepts — an array local the backend must place in C position, or a local
/// that shadows a binding of the same name — surface only when the platform compiler runs.
#[cfg(target_os = "macos")]
#[compio::test]
async fn material_evaluation_lowers_to_the_platform_shader_language() {
	use resource_management::shader::ShaderGenerationSettings;
	use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

	let material = material_metadata! { "variables": [] };
	let shader_node = besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("test shader");
	let shader = material_generator().transform(shader_node, &material);
	let root = besl::lex(shader).expect("generated program should link");

	let settings = ShaderGenerationSettings::compute(utils::Extent::line(128)).name("material_evaluation".to_string());

	PlatformShaderCompiler::new()
		.generate(&settings, &root)
		.await
		.expect("generated material evaluation program should compile for the platform shader language");
}

/// Verifies cone PCF evaluates its receiver plane at each fetched shadow texel center.
#[test]
fn cone_shadow_receiver_plane_depth_gradient_executes_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let identity: mat4f = mat4f(
				vec4f(1.0, 0.0, 0.0, 0.0),
				vec4f(0.0, 1.0, 0.0, 0.0),
				vec4f(0.0, 0.0, 1.0, 0.0),
				vec4f(0.0, 0.0, 0.0, 1.0)
			);
			let surface_light_clip_position: vec4f = vec4f(0.1, 0.0 - 0.2, 0.5, 1.0);
			let surface_light_ndc_position: vec3f = vec3f(0.1, 0.0 - 0.2, 0.5);
			let receiver_plane_depth_gradient: vec2f = shadow_receiver_plane_depth_gradient(
				identity,
				surface_light_clip_position,
				surface_light_ndc_position,
				vec3f(0.2, 0.0, 0.3),
				vec3f(0.0, 0.0 - 0.4, 0.0 - 0.2)
			);
			results.gradient = receiver_plane_depth_gradient;
			results.corrected_depth = 0.5 + dot(
				receiver_plane_depth_gradient,
				vec2f(0.6, 0.8) - vec2f(0.55, 0.6)
			);
			results.degenerate = shadow_receiver_plane_depth_gradient(
				identity,
				surface_light_clip_position,
				surface_light_ndc_position,
				vec3f(0.0, 0.0, 0.0),
				vec3f(0.0, 0.0, 0.0)
			);
		}
		"#,
		&mut [],
		&[(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient")],
		vec![
			Node::member("gradient", "vec2f"),
			Node::member("corrected_depth", "f32"),
			Node::member("degenerate", "vec2f"),
		],
	);

	let Value::Vec2F(gradient) = results.read("gradient").expect("receiver-plane gradient") else {
		panic!("Unexpected receiver-plane gradient type.");
	};
	assert!(
		(gradient[0] - 3.0).abs() <= 0.00001 && (gradient[1] + 1.0).abs() <= 0.00001,
		"Unexpected cone receiver-plane gradient: {gradient:?}. The most likely cause is incorrect projected-depth derivative math."
	);
	let corrected_depth = read_f32(&results, "corrected_depth");
	assert!(
		(corrected_depth - 0.45).abs() <= 0.00001,
		"Unexpected cone receiver depth at a shadow texel center: {corrected_depth}. The most likely cause is incorrect receiver-plane tap correction."
	);
	assert_eq!(
		results.read("degenerate").expect("degenerate receiver-plane gradient"),
		Value::Vec2F([0.0, 0.0]),
		"A degenerate shadow projection must retain the base depth bias."
	);
}

/// Verifies the directional probe skips PCF only when every fine cell touching the footprint is clear.
#[test]
fn directional_shadow_depth_probe_is_conservative_in_the_besl_vm() {
	let cascade_depths = [0.2, 0.4, 0.7, 0.9];
	let mut base_depths = (0..8)
		.flat_map(|y| std::iter::repeat_n([cascade_depths[y / 2], 0.0, 0.0, 1.0], 2))
		.collect::<Vec<_>>();
	// Cascade zero contains a blocker in the neighboring 8x8 cell. A maximum gather may conservatively include
	// it even when the footprint stays in cell zero.
	base_depths[0] = [0.2, 0.0, 0.0, 1.0];
	base_depths[1] = [0.9, 0.0, 0.0, 1.0];
	let mut pyramid = texture_2d(2, 8, &base_depths);
	pyramid.add_mip(texture_2d(
		1,
		4,
		&[
			[0.9, 0.0, 0.0, 1.0],
			[0.4, 0.0, 0.0, 1.0],
			[0.7, 0.0, 0.0, 1.0],
			[0.9, 0.0, 0.0, 1.0],
		],
	));
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			results.fully_lit = 0;
			results.may_be_occluded = 0;
			results.crosses_tile_boundary = 0;
			results.adjacent_cell_may_occlude = 0;
			if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.8, 2, vec2u(16, 16))) {
				results.fully_lit = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.6, 2, vec2u(16, 16))) {
				results.may_be_occluded = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.1, 0.5), 1.0, 2, vec2u(16, 16))) {
				results.crosses_tile_boundary = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.25, 0.25), 0.8, 0, vec2u(16, 16))) {
				results.adjacent_cell_may_occlude = 1;
			}
		}
		"#,
		&mut [(
			"directional_shadow_depth_pyramid",
			Node::combined_image_sampler(),
			&mut pyramid,
		)],
		&[(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit")],
		members(
			"u32",
			&[
				"fully_lit",
				"may_be_occluded",
				"crosses_tile_boundary",
				"adjacent_cell_may_occlude",
			],
		),
	);

	for (name, expected) in [
		("fully_lit", 1),
		("may_be_occluded", 0),
		("crosses_tile_boundary", 1),
		("adjacent_cell_may_occlude", 0),
	] {
		assert_eq!(
			results.read(name).expect("directional shadow probe result"),
			Value::U32(expected),
			"Unexpected directional shadow probe result for {name}."
		);
	}
}

/// Verifies the tent shadow filter turns a hard shadow-map edge into a smooth ramp, keeps reverse-Z comparison, compares
/// a sloped receiver on its own plane so it does not shadow itself, treats texels outside the map as lit, and reaches
/// further from an edge when its taps are spaced wider.
#[test]
fn shadow_tent_filter_ramps_across_an_edge_in_the_besl_vm() {
	// Texels from column four on hold a blocker at depth 0.9, closer to the light than a receiver at 0.8 under
	// reverse-Z. The others hold 0.2, farther than it.
	let mut shadow_map = column_shadow_map(8, |x| if x >= 4 { 0.9 } else { 0.2 });
	// A surface sloped toward the light along x stores its depth at each texel center.
	let mut sloped_map = column_shadow_map(8, |x| 0.5 + 0.01 * (x as f32 + 0.5));
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let flat: vec2f = vec2f(0.0, 0.0);
			let extent: vec2u = vec2u(8, 8);
			// Texels from column four on hold a blocker. The tent reaches two tap spacings to each side of the receiver.
			results.clear = sample_directional_shadow_tent(shadow_map, vec2f(2.0, 4.0) / 8.0, 0.8, flat, u32(0), extent, 1.0);
			results.quarter_covered = sample_directional_shadow_tent(shadow_map, vec2f(3.5, 4.0) / 8.0, 0.8, flat, u32(0), extent, 1.0);
			results.on_edge = sample_directional_shadow_tent(shadow_map, vec2f(4.0, 4.0) / 8.0, 0.8, flat, u32(0), extent, 1.0);
			results.covered = sample_directional_shadow_tent(shadow_map, vec2f(6.0, 4.0) / 8.0, 0.8, flat, u32(0), extent, 1.0);
			results.wide_on_edge = sample_directional_shadow_tent(shadow_map, vec2f(4.0, 4.0) / 8.0, 0.8, flat, u32(0), extent, 2.0);
			// The receiver lies 0.0001 in front of the stored slope, which rises 0.01 per texel, 0.08 per unit of uv.
			results.sloped_receiver = sample_directional_shadow_tent(sloped_map, vec2f(3.2, 3.5) / 8.0, 0.5321, vec2f(0.08, 0.0), u32(0), extent, 1.0);
			results.bounded_on_edge = sample_shadow_tent(shadow_map, vec2f(0.5, 0.5), 0.8, flat, u32(0), vec2u(8, 8), 1.0);
			// One quarter of this footprint's weight falls past the map's right edge.
			results.bounded_past_border = sample_shadow_tent(shadow_map, vec2f(7.5 / 8.0, 0.5), 0.8, flat, u32(0), vec2u(8, 8), 1.0);
			// Two texels from the edge, a one-texel spacing stays clear, but a two-texel spacing reaches the blocker.
			results.bounded_wide_reach = sample_shadow_tent(shadow_map, vec2f(2.0 / 8.0, 0.5), 0.8, flat, u32(0), vec2u(8, 8), 2.0);
		}
		"#,
		&mut [
			("shadow_map", Node::combined_array_image_sampler(), &mut shadow_map),
			("sloped_map", Node::combined_array_image_sampler(), &mut sloped_map),
		],
		&[
			(SHADOW_TAP_SOURCE, "sample_shadow_tap"),
			(SHADOW_TENT_SOURCE, "sample_shadow_tent"),
			(DIRECTIONAL_SHADOW_TENT_SOURCE, "sample_directional_shadow_tent"),
		],
		members(
			"f32",
			&[
				"clear",
				"quarter_covered",
				"on_edge",
				"covered",
				"wide_on_edge",
				"sloped_receiver",
				"bounded_on_edge",
				"bounded_past_border",
				"bounded_wide_reach",
			],
		),
	);

	assert_f32_results(
		&results,
		&[
			("clear", 1.0),
			("quarter_covered", 0.75),
			("on_edge", 0.5),
			("covered", 0.0),
			("wide_on_edge", 0.5),
			("sloped_receiver", 1.0),
			("bounded_on_edge", 0.5),
			("bounded_past_border", 0.25),
			("bounded_wide_reach", 0.875),
		],
		"The most likely cause is incorrect tent weights or tap addressing.",
	);
}

/// Returns a square one-layer shadow map, `size` texels wide, whose texels in column `x` hold the stored depth
/// `depth(x)`.
fn column_shadow_map(size: u32, depth: impl Fn(u32) -> f32) -> Texture {
	let mut shadow_map = Texture::new_3d(size, size, 1).expect("shadow map fixture");
	for y in 0..size {
		for x in 0..size {
			shadow_map
				.write_3d([x, y, 0], [depth(x), 0.0, 0.0, 1.0])
				.expect("shadow map fixture");
		}
	}
	shadow_map
}

/// Asserts that each named `f32` result matches its expected value.
fn assert_f32_results(results: &besl::vm::Buffer, expected: &[(&str, f32)], likely_cause: &str) {
	for &(name, expected) in expected {
		let actual = read_f32(results, name);
		assert!(
			(actual - expected).abs() <= 0.00001,
			"Unexpected result for {name}: {actual}, expected {expected}. {likely_cause}"
		);
	}
}

/// Verifies the directional penumbra widens with its radius and stays sharp at contact: a receiver four texels from a
/// shadow edge is fully lit under the sharpest tent, partly shadowed under a wide one, and in between for a radius
/// between tent spacings. A receiver on the edge stays half lit at any radius.
#[test]
fn directional_shadow_penumbra_widens_with_its_radius_in_the_besl_vm() {
	// Texels from column 16 on hold a blocker in front of a receiver at 0.8.
	let mut shadow_map = column_shadow_map(32, |x| if x >= 16 { 0.9 } else { 0.2 });
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let flat: vec2f = vec2f(0.0, 0.0);
			let near_edge: vec2f = vec2f(12.0 / 32.0, 0.5);
			let extent: vec2u = vec2u(32, 32);
			results.contact = sample_directional_shadow_penumbra(shadow_map, near_edge, 0.8, flat, u32(0), extent, 0.0);
			results.between = sample_directional_shadow_penumbra(shadow_map, near_edge, 0.8, flat, u32(0), extent, 6.0);
			results.wide = sample_directional_shadow_penumbra(shadow_map, near_edge, 0.8, flat, u32(0), extent, 8.0);
			results.on_edge = sample_directional_shadow_penumbra(shadow_map, vec2f(0.5, 0.5), 0.8, flat, u32(0), extent, 8.0);
		}
		"#,
		&mut [("shadow_map", Node::combined_array_image_sampler(), &mut shadow_map)],
		&[
			(SHADOW_TAP_SOURCE, "sample_shadow_tap"),
			(SHADOW_TENT_SOURCE, "sample_shadow_tent"),
			(DIRECTIONAL_SHADOW_TENT_SOURCE, "sample_directional_shadow_tent"),
			(DIRECTIONAL_SHADOW_PENUMBRA_SOURCE, "sample_directional_shadow_penumbra"),
		],
		members("f32", &["contact", "between", "wide", "on_edge"]),
	);

	// A radius of six lies between the two- and four-texel spacings, which give 1.0 and 0.875 here.
	let between_blend = 3.0_f32.log2() - 1.0;
	assert_f32_results(
		&results,
		&[
			("contact", 1.0),
			("between", 1.0 + (0.875 - 1.0) * between_blend),
			("wide", 0.875),
			("on_edge", 0.5),
		],
		"The most likely cause is an incorrect penumbra level or blend between tent spacings.",
	);
}

/// Verifies the blocker search returns the depth of an occluder near the receiver, ignores occluders beyond its reach
/// and in other cascades, lets an occluder entering the search move the estimate only gradually, and does not count a
/// sloped receiver as its own blocker.
#[test]
fn directional_shadow_blocker_search_finds_nearby_occluders_in_the_besl_vm() {
	// Four 64x64 cascades reduce to four stacked blocks of 8x8 max-depth cells.
	let mut cells = vec![[0.2, 0.0, 0.0, 1.0]; 8 * 32];
	cells[4 * 8 + 4] = [0.9, 0.0, 0.0, 1.0];
	let mut pyramid = texture_2d(8, 32, &cells);
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let flat: vec2f = vec2f(0.0, 0.0);
			let extent: vec2u = vec2u(64, 64);
			// Stored depth spans 100 meters, so occluders fade in over their first 0.0005 above the receiver.
			let depth_per_meter: f32 = 0.01;
			// Cell (4, 4) of cascade zero, texels 32 through 39, holds an occluder at 0.9.
			results.nearby = directional_shadow_blocker_depth(vec2f(36.0, 36.0), 0.5, flat, depth_per_meter, u32(0), extent);
			results.out_of_reach = directional_shadow_blocker_depth(vec2f(8.0, 8.0), 0.5, flat, depth_per_meter, u32(0), extent);
			results.other_cascade = directional_shadow_blocker_depth(vec2f(36.0, 36.0), 0.5, flat, depth_per_meter, u32(1), extent);
			// The occluder lies 0.00025 above this receiver, halfway through its fade, and alone at full tent weight
			// it still counts fully once its share of four units of weight exceeds one.
			results.fading_in = directional_shadow_blocker_depth(vec2f(36.0, 36.0), 0.89975, flat, depth_per_meter, u32(0), extent);
			// At the edge of the search the same occluder carries a quarter unit of weight, so the estimate moves only a
			// quarter of the way from the receiver toward it.
			results.entering = directional_shadow_blocker_depth(vec2f(24.0, 24.0), 0.5, flat, depth_per_meter, u32(0), extent);
		}
		"#,
		&mut [(
			"directional_shadow_depth_pyramid",
			Node::combined_image_sampler(),
			&mut pyramid,
		)],
		&[(DIRECTIONAL_SHADOW_BLOCKER_SOURCE, "directional_shadow_blocker_depth")],
		members("f32", &["nearby", "out_of_reach", "other_cascade", "fading_in", "entering"]),
	);

	assert_f32_results(
		&results,
		&[
			("nearby", 0.9),
			("out_of_reach", 0.0),
			("other_cascade", 0.0),
			("fading_in", 0.9),
			("entering", 0.5 + (0.9 - 0.5) * 0.25),
		],
		"The most likely cause is incorrect cell addressing or blocker comparison.",
	);

	// A receiver sloped toward the light along x: each cell's maximum is the receiver's own depth at the cell's
	// nearest-to-light texel center, 3.5 texels past the cell center.
	let sloped_cells = (0..8 * 32)
		.map(|index| {
			let cell_center_x = (index % 8) as f32 * 8.0 + 4.0;
			[0.5 + 0.01 * (cell_center_x + 3.5 - 36.0), 0.0, 0.0, 1.0]
		})
		.collect::<Vec<_>>();
	let mut sloped_pyramid = texture_2d(8, 32, &sloped_cells);
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			results.sloped_self = directional_shadow_blocker_depth(vec2f(36.0, 36.0), 0.5, vec2f(0.01, 0.0), 0.01, u32(0), vec2u(64, 64));
		}
		"#,
		&mut [(
			"directional_shadow_depth_pyramid",
			Node::combined_image_sampler(),
			&mut sloped_pyramid,
		)],
		&[(DIRECTIONAL_SHADOW_BLOCKER_SOURCE, "directional_shadow_blocker_depth")],
		vec![Node::member("sloped_self", "f32")],
	);

	assert_eq!(
		read_f32(&results, "sloped_self"),
		0.0,
		"A sloped receiver counted as its own blocker. The most likely cause is comparing cells against the receiver's center depth instead of its plane."
	);
}

/// Verifies directional cascade scales come from the cascade's orthographic projection: a projection whose normalized
/// device x spans 20 meters and whose stored depth spans 100 meters gives 102.4 texels per meter on a 2048-texel map and
/// 0.01 units of stored depth per meter.
#[test]
fn directional_shadow_cascade_scales_follow_the_projection_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let projection: mat4f = mat4f(
				vec4f(0.1, 0.0, 0.0, 0.0),
				vec4f(0.0, 0.1, 0.0, 0.0),
				vec4f(0.0, 0.0, 0.01, 0.0),
				vec4f(0.0, 0.0, 0.0, 1.0)
			);
			results.texels_per_meter = directional_shadow_texels_per_meter(projection, 2048.0);
			results.depth_per_meter = directional_shadow_depth_per_meter(projection);
		}
		"#,
		&mut [],
		&[
			(
				DIRECTIONAL_SHADOW_TEXELS_PER_METER_SOURCE,
				"directional_shadow_texels_per_meter",
			),
			(
				DIRECTIONAL_SHADOW_DEPTH_PER_METER_SOURCE,
				"directional_shadow_depth_per_meter",
			),
		],
		members("f32", &["texels_per_meter", "depth_per_meter"]),
	);
	let texels_per_meter = read_f32(&results, "texels_per_meter");
	let depth_per_meter = read_f32(&results, "depth_per_meter");
	assert!(
		(texels_per_meter - 102.4).abs() <= 0.001 && (depth_per_meter - 0.01).abs() <= 0.000001,
		"Unexpected cascade scales: {texels_per_meter} texels and {depth_per_meter} depth per meter. The most likely cause is reading a column instead of a row of the cascade projection."
	);
}

/// Verifies directional shadows pick the first cascade, from the receiver's own, in which a distance fits a texel limit:
/// they stay in the receiver's cascade when it fits, move to a coarser one when it does not, never go past the last
/// allowed cascade, and never go back to a finer one than the receiver's.
#[test]
fn directional_shadow_fitting_cascade_picks_the_finest_that_fits_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			// Cascades span 500, 200, 70, and 15 texels per meter.
			let scales: vec4f = vec4f(500.0, 200.0, 70.0, 15.0);
			results.fits_own = directional_shadow_fitting_cascade(u32(0), u32(3), 0.01, scales, 8.0);
			results.moves_coarser = directional_shadow_fitting_cascade(u32(0), u32(3), 0.1, scales, 8.0);
			results.stops_at_last = directional_shadow_fitting_cascade(u32(0), u32(1), 0.1, scales, 8.0);
			results.keeps_receiver_cascade = directional_shadow_fitting_cascade(u32(2), u32(3), 0.001, scales, 8.0);
		}
		"#,
		&mut [],
		&[
			(DIRECTIONAL_SHADOW_CASCADE_SCALE_SOURCE, "directional_shadow_cascade_scale"),
			(
				DIRECTIONAL_SHADOW_FITTING_CASCADE_SOURCE,
				"directional_shadow_fitting_cascade",
			),
		],
		members(
			"u32",
			&["fits_own", "moves_coarser", "stops_at_last", "keeps_receiver_cascade"],
		),
	);
	for (name, expected) in [
		("fits_own", 0),
		// A tenth of a meter spans 50, 20, then 7 texels.
		("moves_coarser", 2),
		("stops_at_last", 1),
		("keeps_receiver_cascade", 2),
	] {
		assert_eq!(
			results.read(name).expect("fitting cascade result"),
			Value::U32(expected),
			"Unexpected fitting cascade for {name}. The most likely cause is comparing against the wrong cascade's scale."
		);
	}
}

/// Verifies a directional cascade holds a receiver only while the receiver is at least eight texels, the filter's reach,
/// inside the cascade's square: a 20-meter square on a 2048-texel map holds receivers up to 9.92 meters from its center.
#[test]
fn directional_shadow_cascade_holds_receivers_inside_the_filter_margin_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let projection: mat4f = mat4f(
				vec4f(0.1, 0.0, 0.0, 0.0),
				vec4f(0.0, 0.1, 0.0, 0.0),
				vec4f(0.0, 0.0, 0.01, 0.0),
				vec4f(0.0, 0.0, 0.0, 1.0)
			);
			results.inside = 0;
			if (directional_shadow_cascade_holds(projection, vec3f(9.9, 0.0 - 9.9, 3.0), 2048.0)) {
				results.inside = 1;
			}
			results.past_x = 0;
			if (directional_shadow_cascade_holds(projection, vec3f(9.95, 0.0, 3.0), 2048.0)) {
				results.past_x = 1;
			}
			results.past_y = 0;
			if (directional_shadow_cascade_holds(projection, vec3f(0.0, 0.0 - 9.95, 3.0), 2048.0)) {
				results.past_y = 1;
			}
		}
		"#,
		&mut [],
		&[(DIRECTIONAL_SHADOW_CASCADE_HOLDS_SOURCE, "directional_shadow_cascade_holds")],
		members("u32", &["inside", "past_x", "past_y"]),
	);
	for (name, expected) in [("inside", 1), ("past_x", 0), ("past_y", 0)] {
		assert_eq!(
			results.read(name).expect("cascade holds result"),
			Value::U32(expected),
			"Unexpected cascade holds result for {name}. The most likely cause is a margin other than eight texels."
		);
	}
}

/// Verifies double-sided shading keeps front-facing normals and reverses back-facing ones for either winding.
#[test]
fn facing_normal_reverses_only_back_facing_normals_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let to_camera: vec3f = vec3f(0.0, 0.0, 1.0);
			let right: vec3f = vec3f(1.0, 0.0, 0.0);
			let up: vec3f = vec3f(0.0, 1.0, 0.0);
			results.front = facing_normal(vec3f(0.0, 0.0, 1.0), to_camera, right, up).z;
			results.back = facing_normal(vec3f(0.0, 0.0, 0.0 - 1.0), to_camera, right, up).z;
			results.back_mirrored = facing_normal(vec3f(0.0, 0.0, 0.0 - 1.0), to_camera, up, right).z;
			results.grazing_front = facing_normal(normalize(vec3f(1.0, 0.0, 0.2)), to_camera, right, up).z;
		}
		"#,
		&mut [],
		&[(FACING_NORMAL_SOURCE, "facing_normal")],
		members("f32", &["front", "back", "back_mirrored", "grazing_front"]),
	);
	assert_f32_results(
		&results,
		&[
			("front", 1.0),
			("back", 1.0),
			("back_mirrored", 1.0),
			("grazing_front", 0.2 / (1.04f32).sqrt()),
		],
		"facing_normal does not orient normals by the camera side of the surface plane",
	);
}

/// Verifies point receivers use the perspective depth stored by the selected cube face.
#[test]
fn point_shadow_receiver_depth_uses_the_dominant_cube_axis_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			results.center = point_shadow_receiver_depth(vec3f(0.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
			results.off_axis = point_shadow_receiver_depth(vec3f(4.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
			results.adjacent_face = point_shadow_receiver_depth(vec3f(6.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
		}
		"#,
		&mut [],
		&[(POINT_SHADOW_RECEIVER_DEPTH_SOURCE, "point_shadow_receiver_depth")],
		members("f32", &["center", "off_axis", "adjacent_face"]),
	);
	let center = read_f32(&results, "center");
	assert!((center - read_f32(&results, "off_axis")).abs() < 0.000001);
	assert!(read_f32(&results, "adjacent_face") < center);
}

/// Verifies offset point-shadow rays compare against the shaded receiver plane instead of a constant radius.
#[test]
fn point_shadow_taps_intersect_the_receiver_plane_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let sample_direction: vec3f = normalize(vec3f(1.0, 0.0 - 5.0, 0.0));
			let receiver: vec3f = point_shadow_receiver_vector(
				sample_direction,
				vec3f(0.0, 0.0 - 5.0, 0.0),
				vec3f(0.0, 1.0, 0.0)
			);
			results.x = receiver.x;
			results.y = receiver.y;
		}
		"#,
		&mut [],
		&[(POINT_SHADOW_RECEIVER_VECTOR_SOURCE, "point_shadow_receiver_vector")],
		members("f32", &["x", "y"]),
	);
	assert!((read_f32(&results, "x") - 1.0).abs() < 0.000001);
	assert!((read_f32(&results, "y") + 5.0).abs() < 0.000001);
}

/// Verifies receiver-plane orientation does not change as close-camera derivatives shrink.
#[test]
fn point_shadow_receiver_plane_normal_is_camera_scale_invariant_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			results.large = point_shadow_receiver_plane_normal(
				vec3f(1.0, 0.0, 0.0),
				vec3f(0.0, 1.0, 0.0)
			).z;
			results.small = point_shadow_receiver_plane_normal(
				vec3f(0.0001, 0.0, 0.0),
				vec3f(0.0, 0.0001, 0.0)
			).z;
		}
		"#,
		&mut [],
		&[(
			POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE,
			"point_shadow_receiver_plane_normal",
		)],
		members("f32", &["large", "small"]),
	);
	for name in ["large", "small"] {
		assert!(
			(read_f32(&results, name) - 1.0).abs() < 0.000001,
			"Unexpected point-shadow receiver-plane scale result for {name}."
		);
	}
}

/// Verifies point PCF compares against the center of the cube texel selected by closest sampling.
#[test]
fn point_shadow_taps_snap_to_the_selected_cube_texel_center_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			let direction: vec3f = point_shadow_texel_direction(normalize(vec3f(1.0, 0.0 - 0.25, 0.1)));
			results.y_over_x = direction.y / direction.x;
			results.z_over_x = direction.z / direction.x;
		}
		"#,
		&mut [],
		&[(POINT_SHADOW_TEXEL_DIRECTION_SOURCE, "point_shadow_texel_direction")],
		members("f32", &["y_over_x", "z_over_x"]),
	);
	for (name, expected) in [("y_over_x", -0.25097656), ("z_over_x", 0.10058594)] {
		assert!(
			(read_f32(&results, name) - expected).abs() < 0.000001,
			"Unexpected point-shadow texel-center result for {name}."
		);
	}
}

/// Verifies receivers beyond a point shadow's projection range remain unshadowed.
#[test]
fn point_shadow_occlusion_ignores_captured_depth_beyond_the_far_plane_in_the_besl_vm() {
	let results = run_helper_test(
		r#"
		main: fn () -> void {
			results.blocker_beyond_far = point_shadow_occlusion(0.4, 0.0 - 0.01, 110.0, 0.1, 100.0);
			results.clear_beyond_far = point_shadow_occlusion(0.0, 0.0 - 0.01, 110.0, 0.1, 100.0);
			results.blocked_inside = point_shadow_occlusion(0.4, 0.2, 10.0, 0.1, 100.0);
			results.lit_inside = point_shadow_occlusion(0.1, 0.2, 10.0, 0.1, 100.0);
		}
		"#,
		&mut [],
		&[(POINT_SHADOW_OCCLUSION_SOURCE, "point_shadow_occlusion")],
		members(
			"f32",
			&["blocker_beyond_far", "clear_beyond_far", "blocked_inside", "lit_inside"],
		),
	);
	for (name, expected) in [
		("blocker_beyond_far", 1.0),
		("clear_beyond_far", 1.0),
		("blocked_inside", 0.0),
		("lit_inside", 1.0),
	] {
		assert_eq!(
			read_f32(&results, name),
			expected,
			"Unexpected point-shadow occlusion result for {name}."
		);
	}
}

/* Screen-space reflections */

const REFLECTION_EXTENT: u32 = 64;
const REFLECTION_FAR: f32 = 100.0;
const REFLECTION_INPUTS_SLOT: ResourceSlot = ResourceSlot::new(0);
const REFLECTION_RESULTS_SLOT: ResourceSlot = ResourceSlot::new(1);
const REFLECTION_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1059);
const FLOOR_COLOR: [f32; 3] = [0.2, 0.2, 0.2];
const WALL_COLOR: [f32; 3] = [2.0, 1.0, 0.5];
const BAR_COLOR: [f32; 3] = [0.0, 5.0, 0.0];

/// The `ReflectionScene` struct describes a fixture seen by a camera at the origin looking down positive z, through
/// [`ssgi_projection`]: a floor one unit below the camera, an optional wall facing the camera, and an optional
/// horizontal bar floating in front.
#[derive(Clone, Copy)]
struct ReflectionScene {
	wall_z: Option<f32>,
	/// The bar sits at this depth and covers view rays whose `y / z` lies in `[-0.3, -0.2]`.
	bar_z: Option<f32>,
}

impl ReflectionScene {
	/// Returns the depth and color a view ray `(x / z, y / z)` sees, or zero depth for the sky.
	fn surface(self, ray: [f32; 2]) -> (f32, [f32; 3]) {
		let mut nearest = (f32::INFINITY, [0.0; 3]);
		if ray[1] < 0.0 {
			nearest = (-1.0 / ray[1], FLOOR_COLOR);
		}
		if let Some(wall_z) = self.wall_z
			&& wall_z < nearest.0
		{
			nearest = (wall_z, WALL_COLOR);
		}
		if let Some(bar_z) = self.bar_z
			&& (-0.3..=-0.2).contains(&ray[1])
			&& bar_z < nearest.0
		{
			nearest = (bar_z, BAR_COLOR);
		}
		if nearest.0 <= REFLECTION_FAR {
			nearest
		} else {
			(0.0, [0.0; 3])
		}
	}
}

/// Renders `scene` into the half-resolution linear depth pyramid the rays march.
fn reflection_depth_pyramid(scene: ReflectionScene) -> Texture {
	let half = REFLECTION_EXTENT / 2;
	let depth: Vec<[f32; 4]> = (0..half * half)
		.map(|index| [scene.surface(ssgi_ray_at(index % half, index / half, half)).0, 0.0, 0.0, 1.0])
		.collect();
	// Mip zero holds half-resolution depth.
	texture_2d(half, half, &depth)
}

/// Renders `scene` into a full-resolution radiance history: light multiplied by `exposure` in RGB, depth in alpha.
fn reflection_radiance_history(scene: ReflectionScene, exposure: f32) -> Texture {
	let extent = REFLECTION_EXTENT;
	let texels: Vec<[f32; 4]> = (0..extent * extent)
		.map(|index| {
			let (z, [r, g, b]) = scene.surface(ssgi_ray_at(index % extent, index / extent, extent));
			[r * exposure, g * exposure, b * exposure, z]
		})
		.collect();
	texture_2d(extent, extent, &texels)
}

/// Returns the full-resolution floor pixel row in the center column whose floor point lies closest to `z`.
fn floor_row_at(z: f32) -> u32 {
	(REFLECTION_EXTENT / 2..REFLECTION_EXTENT)
		.min_by(|&a, &b| {
			let depth = |row| -1.0 / ssgi_ray_at(REFLECTION_EXTENT / 2, row, REFLECTION_EXTENT)[1];
			(depth(a) - z).abs().total_cmp(&(depth(b) - z).abs())
		})
		.expect("floor rows")
}

/// Traces the mirror reflection of the camera ray off the floor at full-resolution pixel `(column, row)`.
///
/// This frame draws `scene`, and the previous frame, from the same static camera, drew `previous_scene`. Returns
/// the helper's unexposed radiance in RGB and its confidence in alpha.
fn trace_floor_reflection(scene: ReflectionScene, previous_scene: Option<ReflectionScene>, column: u32, row: u32) -> [f32; 4] {
	const PREVIOUS_EXPOSURE: f32 = 2.0;
	let executable = compile_with(
		r#"
		main: fn () -> void {
			results.reflection = trace_screen_space_reflection(
				vec3f(inputs.position.x, inputs.position.y, inputs.position.z),
				vec3f(0.0, 1.0, 0.0),
				vec3f(inputs.direction.x, inputs.direction.y, inputs.direction.z),
				inputs.view_projection,
				inputs.extent
			);
		}
		"#,
		{
			let mut bindings = screen_space_reflection_scope();
			bindings.push(Node::binding(
				"inputs",
				Node::buffer(vec![
					Node::member("view_projection", "mat4f"),
					Node::member("position", "vec4f"),
					Node::member("direction", "vec4f"),
					Node::member("extent", "vec2u"),
				]),
				REFLECTION_INPUTS_SLOT.slot(),
				true,
				false,
			));
			bindings.push(results_binding(
				vec![Node::member("reflection", "vec4f")],
				REFLECTION_RESULTS_SLOT,
			));
			bindings
		},
	);

	// The camera sits at the origin, so world space is view space and the view-projection is the projection.
	let ray = ssgi_ray_at(column, row, REFLECTION_EXTENT);
	let z = -1.0 / ray[1];
	let position = [ray[0] * z, -1.0, z];
	let length = (position[0] * position[0] + 1.0 + z * z).sqrt();
	// The floor's normal points up, so the mirror direction flips the view ray's vertical component.
	let direction = [position[0] / length, 1.0 / length, z / length, 0.0];
	let mut inputs = buffer(&executable, REFLECTION_INPUTS_SLOT);
	for (member, value) in [
		("view_projection", Value::Mat4F(column_major(ssgi_projection()))),
		("position", Value::Vec4F([position[0], position[1], position[2], 1.0])),
		("direction", Value::Vec4F(direction)),
		("extent", Value::Vec2U([REFLECTION_EXTENT, REFLECTION_EXTENT])),
	] {
		inputs.write(member, value).expect("reflection inputs");
	}
	let mut parameters = buffer(&executable, REFLECTION_PARAMETERS_SLOT);
	for (member, value) in [
		("world_to_previous_clip", Value::Mat4F(column_major(ssgi_projection()))),
		("previous_exposure", Value::F32(PREVIOUS_EXPOSURE)),
		("history_valid", Value::U32(previous_scene.is_some() as u32)),
	] {
		parameters.write(member, value).expect("reflection parameters");
	}
	let mut depth_pyramid = reflection_depth_pyramid(scene);
	let mut previous_radiance = reflection_radiance_history(previous_scene.unwrap_or(scene), PREVIOUS_EXPOSURE);
	let mut results = buffer(&executable, REFLECTION_RESULTS_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(REFLECTION_INPUTS_SLOT, &mut inputs);
	descriptors.bind_buffer(REFLECTION_PARAMETERS_SLOT, &mut parameters);
	descriptors.bind_texture(ResourceSlot::new(1060), &mut depth_pyramid);
	descriptors.bind_texture(ResourceSlot::new(1061), &mut previous_radiance);
	descriptors.bind_buffer(REFLECTION_RESULTS_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);
	match results.read("reflection").expect("reflection result") {
		Value::Vec4F(value) => value,
		value => panic!("Unexpected reflection result type: {value:?}."),
	}
}

fn assert_reflects(reflection: [f32; 4], color: [f32; 3]) {
	for channel in 0..3 {
		assert!(
			(reflection[channel] - color[channel]).abs() < 0.0001,
			"Expected the reflection of {color:?}, found {reflection:?}."
		);
	}
	assert!(reflection[3] > 0.99, "Expected a confident reflection, found {reflection:?}.");
}

const WALL: ReflectionScene = ReflectionScene {
	wall_z: Some(4.0),
	bar_z: None,
};

/// Verifies a floor in front of a wall reflects the wall's light from last frame, at the unexposed level.
#[test]
fn reflection_rays_return_the_light_of_the_surface_they_hit() {
	for z in [1.5, 2.0, 3.0] {
		let reflection = trace_floor_reflection(WALL, Some(WALL), REFLECTION_EXTENT / 2, floor_row_at(z));
		assert_reflects(reflection, WALL_COLOR);
	}
}

/// Verifies a ray that passes behind a thin object keeps marching and reflects the surface it reaches, rather than
/// the object whose front face it passed.
#[test]
fn reflection_rays_pass_behind_thin_objects() {
	let scene = ReflectionScene {
		wall_z: Some(4.0),
		bar_z: Some(1.5),
	};
	let row = floor_row_at(2.0);
	assert_eq!(
		scene.surface(ssgi_ray_at(REFLECTION_EXTENT / 2, row, REFLECTION_EXTENT)).1,
		FLOOR_COLOR,
		"The bar must not hide the ray's origin."
	);

	assert_reflects(
		trace_floor_reflection(scene, Some(scene), REFLECTION_EXTENT / 2, row),
		WALL_COLOR,
	);
}

/// Verifies rays miss where no visible geometry lies along them, or where last frame's light is unusable, so
/// material evaluation keeps the environment.
#[test]
fn reflection_rays_miss_without_a_visible_surface_with_known_light() {
	let open_floor = ReflectionScene {
		wall_z: None,
		bar_z: None,
	};
	let row = floor_row_at(2.0);
	let column = REFLECTION_EXTENT / 2;
	for (name, scene, previous_scene) in [
		("open floor", open_floor, Some(open_floor)),
		("no history", WALL, None),
		// The wall appeared this frame, so the previous frame holds no light for it.
		("disoccluded wall", WALL, Some(open_floor)),
	] {
		let reflection = trace_floor_reflection(scene, previous_scene, column, row);
		assert_eq!(reflection[3], 0.0, "Expected the {name} ray to miss, found {reflection:?}.");
	}
}
