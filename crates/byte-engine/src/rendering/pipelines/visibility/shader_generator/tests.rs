use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value};
use ghi::AccessPolicies;
use resource_management::asset::handler::implementations::bema::ProgramGenerator;
use utils::json::JsonValueTrait;

use super::*;
use crate::rendering::shader_vm_test::{buffer, compile, run_at, texture_2d};

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

fn main_statements<'a>(program: &'a besl::parser::Node<'a>) -> &'a [besl::parser::Node<'a>] {
	let besl::parser::Nodes::Scope { children, .. } = program.node() else {
		panic!("Expected generated material root scope.");
	};
	let main = children
		.iter()
		.find(|child| child.name() == Some("main"))
		.expect("Generated material program should contain main.");
	let besl::parser::Nodes::Function { statements, .. } = main.node() else {
		panic!("Expected generated material main function.");
	};
	statements
}

/// Parses `source` as a `main`, adds `helpers` and `bindings`, and compiles the result for the VM.
fn compile_with_helpers(
	source: &str,
	helpers: &[(&'static str, &str)],
	bindings: Vec<besl::parser::Node<'static>>,
) -> besl::vm::ExecutableProgram {
	let mut root = besl::parse(source).expect("Failed to parse a VM test. The most likely cause is invalid BESL test syntax.");
	let mut nodes = helpers
		.iter()
		.map(|(source, name)| parse_besl_function(source, name))
		.collect::<Vec<_>>();
	nodes.extend(bindings);
	root.add(nodes);
	compile(
		besl::lex(root).expect("Failed to lex a VM test. The most likely cause is an unresolved portable helper operation."),
	)
}

fn results_binding(
	name: &'static str,
	members: Vec<besl::parser::Node<'static>>,
	slot: ResourceSlot,
) -> besl::parser::Node<'static> {
	besl::ParserNode::binding("results", besl::ParserNode::buffer(name, members), slot.slot(), false, true)
}

fn read_f32(results: &besl::vm::Buffer, name: &str) -> f32 {
	match results.read(name).expect("VM result") {
		Value::F32(value) => value,
		value => panic!("Unexpected VM result type for {name}: {value:?}."),
	}
}

/// Executes representative octahedral seams and axes through the optimized production decoder.
#[test]
fn octahedral_decoder_preserves_normal_directions_in_the_besl_vm() {
	const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
	let executable = compile_with_helpers(
		r#"
		main: fn () -> void {
			for (let index: u32 = 0; index < 5; index = index + 1) {
				results.values[index] = normalize(decode_octahedral_normal(inputs.values[index]));
			}
		}
		"#,
		&[(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal")],
		vec![
			besl::ParserNode::binding(
				"inputs",
				besl::ParserNode::buffer("OctahedralInputs", vec![besl::ParserNode::member("values", "vec2u16[5]")]),
				INPUT_SLOT.slot(),
				true,
				false,
			),
			results_binding(
				"OctahedralResults",
				vec![besl::ParserNode::member("values", "vec3f[5]")],
				RESULT_SLOT,
			),
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
			.write_indexed("values", index, Value::Vec2U16(*encoded))
			.expect("octahedral input");
	}
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(INPUT_SLOT, &mut inputs);
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	for (index, (encoded, expected)) in cases.iter().enumerate() {
		let Value::Vec3F(actual) = results.read_indexed("values", index).expect("decoded normal") else {
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
	let executable = compile_with_helpers(
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
		&[
			(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
			(IES_PROFILE_UV_SOURCE, "ies_profile_uv"),
		],
		vec![
			besl::ParserNode::binding(
				"inputs",
				besl::ParserNode::buffer(
					"IesProfileUvInputs",
					vec![
						besl::ParserNode::member("emission_directions", "vec3f[5]"),
						besl::ParserNode::member("axes", "vec3f[5]"),
						besl::ParserNode::member("c0_tangents", "vec2u16[5]"),
					],
				),
				INPUT_SLOT.slot(),
				true,
				false,
			),
			results_binding(
				"IesProfileUvResults",
				vec![besl::ParserNode::member("values", "vec2f[5]")],
				RESULT_SLOT,
			),
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
		let Value::Vec2F(actual) = results.read_indexed("values", index).expect("IES UV") else {
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
fn albedo_write_is_narrowed_to_vec4f16() {
	let material = material_metadata! { "variables": [] };
	let shader_node = besl::parse("main: fn () -> void { albedo = vec4f(1, 2, 3, 4); }").expect("test shader");

	let shader = VisibilityShaderGenerator::new().transform(shader_node, &material);

	let assignment = main_statements(&shader)
		.iter()
		.find(|statement| {
			matches!(
				statement.node(),
				besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { left, .. })
					if matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "albedo")
			)
		})
		.expect("Generated material program should retain the authored albedo assignment.");
	let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator {
		name: operator, right, ..
	}) = assignment.node()
	else {
		panic!("Expected generated albedo assignment.");
	};
	assert_eq!(*operator, "=");
	let besl::parser::Nodes::Expression(besl::parser::Expressions::Call {
		name: besl::parser::TypeName::Named("vec4f16"),
		parameters: narrowed_values,
	}) = right.node()
	else {
		panic!("Generated albedo assignment should narrow to vec4f16.");
	};
	let [authored_value] = narrowed_values.as_slice() else {
		panic!("Generated albedo narrowing should preserve one authored value.");
	};
	let besl::parser::Nodes::Expression(besl::parser::Expressions::Call {
		name: besl::parser::TypeName::Named("vec4f"),
		parameters: components,
	}) = authored_value.node()
	else {
		panic!("Generated albedo assignment should preserve the authored vec4f value.");
	};
	let components = components
		.iter()
		.map(|component| match component.node() {
			besl::parser::Nodes::Expression(besl::parser::Expressions::Literal { value }) => value.as_ref(),
			_ => panic!("Expected literal authored albedo component."),
		})
		.collect::<Vec<_>>();
	assert_eq!(components, ["1", "2", "3", "4"]);
	besl::lex(shader).expect("Generated albedo program should link.");
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
	let specialization = children
		.iter()
		.find(|child| child.name() == Some("albedo"))
		.expect("Generated material program should declare the vec4f variable.");
	assert!(matches!(
		specialization.node(),
		besl::parser::Nodes::Specialization { r#type, .. } if *r#type == "vec4f"
	));
	assert!(main_statements(&shader).iter().any(|statement| {
		matches!(
			statement.node(),
			besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { name, left, right })
				if *name == "="
					&& matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "out_color")
					&& matches!(right.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == "albedo")
		)
	}));
}

/// Verifies material texture variables produce valid BESL.
#[test]
fn texture_variable_transform_produces_valid_besl() {
	let material = material_metadata! {
		"variables": [{ "name": "base_color", "data_type": "Texture2D" }]
	};
	let shader_node = besl::parse("main: fn () -> void { albedo = sample_material(base_color); }").expect("test shader");
	let shader = VisibilityShaderGenerator::new().transform(shader_node, &material);
	besl::lex(shader).expect("generated texture program should link");
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

/// Verifies material evaluation with skinned geometry produces valid BESL.
#[test]
fn material_evaluation_with_skinning_produces_valid_besl() {
	let material = material_metadata! { "variables": [] };
	let shader_node = besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("test shader");
	let shader = material_generator().transform(shader_node, &material);
	besl::lex(shader).expect("generated program should link");
}

/// Verifies cone PCF evaluates its receiver plane at each fetched shadow texel center.
#[test]
fn cone_shadow_receiver_plane_depth_gradient_executes_in_the_besl_vm() {
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(0);
	let executable = compile_with_helpers(
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
		&[(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient")],
		vec![results_binding(
			"ConeShadowReceiverPlaneResults",
			vec![
				besl::ParserNode::member("gradient", "vec2f"),
				besl::ParserNode::member("corrected_depth", "f32"),
				besl::ParserNode::member("degenerate", "vec2f"),
			],
			RESULT_SLOT,
		)],
	);
	let mut results = buffer(&executable, RESULT_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	let Value::Vec2F(gradient) = results.read("gradient").expect("receiver-plane gradient") else {
		panic!("Unexpected receiver-plane gradient type.");
	};
	let Value::Vec2F(degenerate) = results.read("degenerate").expect("degenerate receiver-plane gradient") else {
		panic!("Unexpected degenerate receiver-plane gradient type.");
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
		degenerate,
		[0.0, 0.0],
		"A degenerate shadow projection must retain the base depth bias."
	);
}

/// Verifies the directional probe skips PCF only when every fine cell touching the footprint is clear.
#[test]
fn directional_shadow_depth_probe_is_conservative_in_the_besl_vm() {
	const PYRAMID_SLOT: ResourceSlot = ResourceSlot::new(0);
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
	let executable = compile_with_helpers(
		r#"
		main: fn () -> void {
			results.fully_lit = 0;
			results.may_be_occluded = 0;
			results.crosses_tile_boundary = 0;
			results.adjacent_cell_may_occlude = 0;
			if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.8, 2, vec2u(8, 8))) {
				results.fully_lit = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.5, 0.5), 0.6, 2, vec2u(8, 8))) {
				results.may_be_occluded = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.1, 0.5), 1.0, 2, vec2u(8, 8))) {
				results.crosses_tile_boundary = 1;
			}
			if (directional_shadow_area_is_fully_lit(vec2f(0.25, 0.25), 0.8, 0, vec2u(8, 8))) {
				results.adjacent_cell_may_occlude = 1;
			}
		}
		"#,
		&[],
		vec![
			besl::ParserNode::binding(
				"directional_shadow_depth_pyramid",
				besl::ParserNode::combined_image_sampler(),
				PYRAMID_SLOT.slot(),
				true,
				false,
			),
			parse_besl_function(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit"),
			results_binding(
				"DirectionalShadowProbeResults",
				vec![
					besl::ParserNode::member("fully_lit", "u32"),
					besl::ParserNode::member("may_be_occluded", "u32"),
					besl::ParserNode::member("crosses_tile_boundary", "u32"),
					besl::ParserNode::member("adjacent_cell_may_occlude", "u32"),
				],
				RESULT_SLOT,
			),
		],
	);
	let cascade_depths = [0.2, 0.4, 0.7, 0.9];
	let mut base_depths = (0..8)
		.flat_map(|y| std::iter::repeat_n([cascade_depths[y / 2], 0.0, 0.0, 1.0], 2))
		.collect::<Vec<_>>();
	// Cascade zero contains a blocker in the neighboring 4x4 cell. A maximum gather may conservatively include
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
	let mut results = buffer(&executable, RESULT_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(PYRAMID_SLOT, &mut pyramid);
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	for (name, expected) in [
		("fully_lit", 1),
		("may_be_occluded", 0),
		("crosses_tile_boundary", 1),
		("adjacent_cell_may_occlude", 0),
	] {
		let Value::U32(actual) = results.read(name).expect("directional shadow probe result") else {
			panic!("Unexpected directional shadow probe result type for {name}.");
		};
		assert_eq!(actual, expected, "Unexpected directional shadow probe result for {name}.");
	}
}

/// Verifies the interior texel-space directional fallback preserves reverse-Z shadow comparison.
#[test]
fn directional_shadow_tap_uses_texel_coordinates_in_the_besl_vm() {
	const SHADOW_SLOT: ResourceSlot = ResourceSlot::new(0);
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(1);
	let executable = compile_with_helpers(
		r#"
		main: fn () -> void {
			results.lit = sample_directional_shadow_tap(
				shadow_map, vec2f(1.0, 1.0), 0.8, vec2f16(0.0, 0.0), vec2f16(1.0, 0.0), u32(0)
			);
			results.blocked = sample_directional_shadow_tap(
				shadow_map, vec2f(2.0, 2.0), 0.8, vec2f16(0.0, 0.0), vec2f16(1.0, 0.0), u32(0)
			);
		}
		"#,
		&[],
		vec![
			besl::ParserNode::binding(
				"shadow_map",
				besl::ParserNode::combined_array_image_sampler(),
				SHADOW_SLOT.slot(),
				true,
				false,
			),
			parse_besl_function(SHADOW_POISSON_ROTATION_SOURCE, "rotate_shadow_poisson_offset"),
			parse_besl_function(DIRECTIONAL_SHADOW_TAP_SOURCE, "sample_directional_shadow_tap"),
			results_binding(
				"DirectionalShadowTapResults",
				vec![
					besl::ParserNode::member("lit", "f32"),
					besl::ParserNode::member("blocked", "f32"),
				],
				RESULT_SLOT,
			),
		],
	);
	let mut shadow_map = Texture::new_3d(4, 4, 1).expect("directional shadow fixture");
	for y in 0..4 {
		for x in 0..4 {
			shadow_map
				.write_3d([x, y, 0], [0.2, 0.0, 0.0, 1.0])
				.expect("directional shadow fixture");
		}
	}
	shadow_map
		.write_3d([2, 2, 0], [0.9, 0.0, 0.0, 1.0])
		.expect("directional shadow blocker");
	let mut results = buffer(&executable, RESULT_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(SHADOW_SLOT, &mut shadow_map);
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);

	assert_eq!(read_f32(&results, "lit"), 1.0);
	assert_eq!(read_f32(&results, "blocked"), 0.0);
}

/// Runs `source` with only buffer-free point-shadow helpers bound and returns the results buffer.
fn run_point_shadow_helper(
	source: &str,
	helpers: &[(&'static str, &str)],
	members: Vec<besl::parser::Node<'static>>,
) -> besl::vm::Buffer {
	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(0);
	let executable = compile_with_helpers(
		source,
		helpers,
		vec![results_binding("PointShadowResults", members, RESULT_SLOT)],
	);
	let mut results = buffer(&executable, RESULT_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(RESULT_SLOT, &mut results);
	run_at(&executable, &mut descriptors, [0, 0]);
	drop(descriptors);
	results
}

/// Verifies point receivers use the perspective depth stored by the selected cube face.
#[test]
fn point_shadow_receiver_depth_uses_the_dominant_cube_axis_in_the_besl_vm() {
	let results = run_point_shadow_helper(
		r#"
		main: fn () -> void {
			results.center = point_shadow_receiver_depth(vec3f(0.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
			results.off_axis = point_shadow_receiver_depth(vec3f(4.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
			results.adjacent_face = point_shadow_receiver_depth(vec3f(6.0, 0.0 - 5.0, 0.0), 0.1, 100.0);
		}
		"#,
		&[(POINT_SHADOW_RECEIVER_DEPTH_SOURCE, "point_shadow_receiver_depth")],
		vec![
			besl::ParserNode::member("center", "f32"),
			besl::ParserNode::member("off_axis", "f32"),
			besl::ParserNode::member("adjacent_face", "f32"),
		],
	);
	let center = read_f32(&results, "center");
	assert!((center - read_f32(&results, "off_axis")).abs() < 0.000001);
	assert!(read_f32(&results, "adjacent_face") < center);
}

/// Verifies offset point-shadow rays compare against the shaded receiver plane instead of a constant radius.
#[test]
fn point_shadow_taps_intersect_the_receiver_plane_in_the_besl_vm() {
	let results = run_point_shadow_helper(
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
		&[(POINT_SHADOW_RECEIVER_VECTOR_SOURCE, "point_shadow_receiver_vector")],
		vec![besl::ParserNode::member("x", "f32"), besl::ParserNode::member("y", "f32")],
	);
	assert!((read_f32(&results, "x") - 1.0).abs() < 0.000001);
	assert!((read_f32(&results, "y") + 5.0).abs() < 0.000001);
}

/// Verifies receiver-plane orientation does not change as close-camera derivatives shrink.
#[test]
fn point_shadow_receiver_plane_normal_is_camera_scale_invariant_in_the_besl_vm() {
	let results = run_point_shadow_helper(
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
		&[(
			POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE,
			"point_shadow_receiver_plane_normal",
		)],
		vec![
			besl::ParserNode::member("large", "f32"),
			besl::ParserNode::member("small", "f32"),
		],
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
	let results = run_point_shadow_helper(
		r#"
		main: fn () -> void {
			let direction: vec3f = point_shadow_texel_direction(normalize(vec3f(1.0, 0.0 - 0.25, 0.1)));
			results.y_over_x = direction.y / direction.x;
			results.z_over_x = direction.z / direction.x;
		}
		"#,
		&[(POINT_SHADOW_TEXEL_DIRECTION_SOURCE, "point_shadow_texel_direction")],
		vec![
			besl::ParserNode::member("y_over_x", "f32"),
			besl::ParserNode::member("z_over_x", "f32"),
		],
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
	let results = run_point_shadow_helper(
		r#"
		main: fn () -> void {
			results.blocker_beyond_far = point_shadow_occlusion(0.4, 0.0 - 0.01, 110.0, 0.1, 100.0);
			results.clear_beyond_far = point_shadow_occlusion(0.0, 0.0 - 0.01, 110.0, 0.1, 100.0);
			results.blocked_inside = point_shadow_occlusion(0.4, 0.2, 10.0, 0.1, 100.0);
			results.lit_inside = point_shadow_occlusion(0.1, 0.2, 10.0, 0.1, 100.0);
		}
		"#,
		&[(POINT_SHADOW_OCCLUSION_SOURCE, "point_shadow_occlusion")],
		vec![
			besl::ParserNode::member("blocker_beyond_far", "f32"),
			besl::ParserNode::member("clear_beyond_far", "f32"),
			besl::ParserNode::member("blocked_inside", "f32"),
			besl::ParserNode::member("lit_inside", "f32"),
		],
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
