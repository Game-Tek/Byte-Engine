//! Executes the checked-in visibility BESL assets in the BESL VM against small fixtures.

use besl::vm::{
	DescriptorBindings, ExecutableProgram, ExecutionConfig, MeshOutputs, ResourceSlot, Sampler, SamplerReductionMode,
	TaskOutputs, Texture, Value, WorkgroupState, input_slot, output_slot,
};

use super::mesh_dispatch::MeshDispatchWorkItem;
use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, compile, empty_image, rgba, run_at, texture_2d};

const VIEWS_SLOT: ResourceSlot = ResourceSlot::new(0);
const GTAO_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1);
const MESH_DATA_SLOT: ResourceSlot = ResourceSlot::new(1);
const MATERIAL_COUNT_SLOT: ResourceSlot = ResourceSlot::new(1033);
const MATERIAL_OFFSET_SLOT: ResourceSlot = ResourceSlot::new(1034);
const MATERIAL_OFFSET_SCRATCH_SLOT: ResourceSlot = ResourceSlot::new(1035);
const MATERIAL_DISPATCH_SLOT: ResourceSlot = ResourceSlot::new(1036);
const PIXEL_MAPPING_SLOT: ResourceSlot = ResourceSlot::new(1037);
const INSTANCE_INDEX_SLOT: ResourceSlot = ResourceSlot::new(1040);
const MESH_DISPATCH_WORK_SLOT: ResourceSlot = ResourceSlot::new(1063);
const VERTEX_POSITIONS_SLOT: ResourceSlot = ResourceSlot::new(2);
const VERTEX_UVS_SLOT: ResourceSlot = ResourceSlot::new(5);
const SKINNED_VERTICES_SLOT: ResourceSlot = ResourceSlot::new(4);
const VERTEX_INDICES_SLOT: ResourceSlot = ResourceSlot::new(6);
const PRIMITIVE_INDICES_SLOT: ResourceSlot = ResourceSlot::new(7);
const MESHLETS_SLOT: ResourceSlot = ResourceSlot::new(8);
const FIXTURE_INSTANCE_INDEX: usize = 3;
const FIXTURE_MESHLET_INDEX: usize = 5;
const MESHLET_INSTANCE_BITS: u32 = 12;
const TASK_WORKGROUP_SIZE: u32 = 32;
const INSTRUCTION_LIMIT: usize = 4_000_000;
const GTAO_WORKGROUP_WIDTH: u32 = 16;
const GTAO_WORKGROUP_HEIGHT: u32 = 8;
const GTAO_WORKGROUP_SIZE: usize = 128;
const GTAO_BLUR_WORKGROUP_WIDTH: u32 = 8;
const GTAO_BLUR_WORKGROUP_SIZE: usize = 64;
const GTAO_PYRAMID_WORKGROUP_WIDTH: u32 = 8;
const GTAO_PYRAMID_WORKGROUP_SIZE: usize = 32;
const DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_WIDTH: u32 = 8;
const DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_HEIGHT: u32 = 4;
const DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_SIZE: usize = 32;
const MATERIAL_COUNT_WORKGROUP_WIDTH: u32 = 8;
const MATERIAL_COUNT_WORKGROUP_SIZE: usize = 64;
const PIXEL_MAPPING_WORKGROUP_WIDTH: u32 = 16;
const PIXEL_MAPPING_WORKGROUP_SIZE: usize = 256;

/// Parses and links one checked-in BESL asset that production baking consumes.
fn asset_program(source: &str) -> besl::NodeReference {
	besl::lex(
		besl::parse(source)
			.expect("Failed to parse a visibility shader asset. The most likely cause is invalid checked-in BESL source."),
	)
	.expect("Failed to link a visibility shader asset. The most likely cause is an invalid shader declaration.")
	.get_main()
	.expect("Missing visibility shader main. The most likely cause is that a checked-in BESL asset is incomplete.")
}

/// Compiles one checked-in visibility asset for VM execution.
macro_rules! asset {
	($name:literal) => {
		compile(asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/",
			$name
		))))
	};
}

/// Builds one workgroup of lane configurations over a 2D tile at `base`.
fn tile_configs<const N: usize>(width: u32, base: [u32; 2]) -> [ExecutionConfig; N] {
	std::array::from_fn(|lane| {
		let lane = lane as u32;
		ExecutionConfig::new(INSTRUCTION_LIMIT)
			.with_call_depth_limit(128)
			.with_thread_idx(lane)
			.with_thread_id([base[0] + lane % width, base[1] + lane / width])
	})
}

fn read_u32(buffer: &besl::vm::Buffer, member: &str, index: usize) -> u32 {
	match buffer.read_indexed(member, index).expect("VM u32 array element") {
		Value::U32(value) => value,
		value => panic!("Unexpected visibility buffer value: {value:?}."),
	}
}

fn read_vec3u(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u32; 3] {
	match buffer.read_indexed(member, index).expect("VM vec3u array element") {
		Value::Vec3U(value) => value,
		value => panic!("Unexpected visibility dispatch value: {value:?}."),
	}
}

fn read_vec2u16(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u16; 2] {
	match buffer.read_indexed(member, index).expect("VM vec2u16 array element") {
		Value::Vec2U16(value) => value,
		value => panic!("Unexpected visibility pixel mapping value: {value:?}."),
	}
}

/// Verifies both masked fragment assets parse and link through the source-owned BESL seam.
#[test]
fn masked_fragment_assets_parse_and_link_with_structural_interfaces() {
	for source in [
		include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/masked-fragment.besl"
		)),
		include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/masked-depth-fragment.besl"
		)),
	] {
		asset_program(source);
	}
}

/// Verifies the visibility fragment preserves the mesh-stage identifiers consumed by later compute passes.
#[test]
fn visibility_fragment_main_forwards_primitive_and_instance_identifiers() {
	let program = asset!("visibility-fragment.besl");
	let layout =
		|layout: Option<&besl::vm::BufferLayout>| besl::vm::Buffer::new(layout.expect("visibility fragment interface").clone());
	let mut instance_input = layout(program.input_layout(0));
	let mut primitive_input = layout(program.input_layout(1));
	let mut primitive_output = layout(program.output_layout(0));
	let mut instance_output = layout(program.output_layout(1));
	instance_input
		.write("_besl_interface_instance_index", Value::U32(37))
		.expect("instance input");
	primitive_input
		.write("_besl_interface_primitive_index", Value::U32(0x0102_03ab))
		.expect("primitive input");

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(input_slot(0), &mut instance_input);
	descriptors.bind_buffer(input_slot(1), &mut primitive_input);
	descriptors.bind_buffer(output_slot(0), &mut primitive_output);
	descriptors.bind_buffer(output_slot(1), &mut instance_output);
	program.run_main(&mut descriptors).expect("visibility fragment execution");
	drop(descriptors);

	assert_eq!(
		primitive_output
			.read("_besl_output_primitive_index")
			.expect("primitive output"),
		Value::U32(0x0102_03ab)
	);
	assert_eq!(
		instance_output.read("_besl_output_instance_id").expect("instance output"),
		Value::U32(37)
	);
}

/// Returns a column-major identity matrix in the BESL VM representation.
fn identity_matrix() -> [f32; 16] {
	[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

/// Returns a column-major affine identity matrix in the BESL VM representation.
fn identity_affine_matrix() -> [f32; 12] {
	[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
}

/// Returns a view-projection matrix that moves identity geometry outside the horizontal clip range.
fn horizontally_translated_matrix(translation: f32) -> [f32; 16] {
	let mut matrix = identity_matrix();
	matrix[12] = translation;
	matrix
}

/// Packs the production task payload without allowing its meshlet and instance indices to diverge.
fn meshlet_instance(meshlet_index: u32, instance_index: u32) -> u32 {
	meshlet_index | (instance_index << MESHLET_INSTANCE_BITS)
}

fn fixture_meshlet_instance() -> Value {
	Value::U32(meshlet_instance(FIXTURE_MESHLET_INDEX as u32, FIXTURE_INSTANCE_INDEX as u32))
}

/// Executes one exact production task workgroup at its global dispatch position over consecutive meshlets.
fn run_meshlet_task_workgroup(
	program: &ExecutableProgram,
	view_projections: &[(usize, [f32; 16])],
	selected_view_index: Option<u32>,
	center_radii: &[[f32; 4]],
	skinned: bool,
	workgroup_index: u32,
) -> TaskOutputs {
	let meshlet_count = center_radii.len() as u32;
	assert!(
		(1..=TASK_WORKGROUP_SIZE).contains(&meshlet_count),
		"Task meshlet fixture must hold between one meshlet and one workgroup of meshlets."
	);
	let mut views = buffer(program, VIEWS_SLOT);
	for (view_index, view_projection) in view_projections.iter().copied() {
		views
			.write_indexed_field("views", view_index, "view_projection", Value::Mat4F(view_projection))
			.expect("task view");
		views
			.write_indexed_field("views", view_index, "inverse_view", Value::Mat4x3F(identity_affine_matrix()))
			.expect("task inverse view");
	}
	let mut meshes = buffer(program, MESH_DATA_SLOT);
	meshes
		.write_indexed_field(
			"meshes",
			FIXTURE_INSTANCE_INDEX,
			"model",
			Value::Mat4x3F(identity_affine_matrix()),
		)
		.expect("task mesh transform");
	for (field, value) in [
		("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
		("meshlet_count", meshlet_count),
		("skinned_base_vertex_index", if skinned { 0 } else { u32::MAX }),
	] {
		meshes
			.write_indexed_field("meshes", FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
			.expect("task mesh field");
	}
	let mut meshlets = buffer(program, MESHLETS_SLOT);
	for (meshlet_offset, center_radius) in center_radii.iter().copied().enumerate() {
		let meshlet_index = FIXTURE_MESHLET_INDEX + meshlet_offset;
		meshlets
			.write_indexed_field("meshlets", meshlet_index, "center_radius", Value::PackedVec4F(center_radius))
			.expect("task meshlet bound");
		// A cutoff above one disables cone rejection so each fixture isolates frustum and skinning behavior.
		meshlets
			.write_indexed_field(
				"meshlets",
				meshlet_index,
				"cone_apex_cutoff",
				Value::PackedVec4F([0.0, 0.0, 0.0, 2.0]),
			)
			.expect("task cone cutoff");
	}
	let mut push_constant = besl::vm::Buffer::new(program.push_constant_layout().expect("task push constants").clone());
	push_constant.write("work_item_base", Value::U32(0)).expect("task work base");
	push_constant
		.write("view_index", Value::U32(selected_view_index.unwrap_or(0)))
		.expect("task view index");
	let mut mesh_dispatch_work = buffer(program, MESH_DISPATCH_WORK_SLOT);
	let packed_work = MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed();
	mesh_dispatch_work
		.write_indexed("items", workgroup_index as usize, Value::U32(packed_work))
		.expect("compact mesh dispatch work");

	let mut task_outputs = TaskOutputs::new();
	let mut workgroup_state = WorkgroupState::new();
	let configs = (0..TASK_WORKGROUP_SIZE)
		.map(|lane| {
			ExecutionConfig::new(INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_position(workgroup_index * TASK_WORKGROUP_SIZE + lane)
		})
		.collect::<Vec<_>>();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(VIEWS_SLOT, &mut views);
		descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
		descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
		descriptors.bind_buffer(MESH_DISPATCH_WORK_SLOT, &mut mesh_dispatch_work);
		descriptors.bind_push_constant(&mut push_constant);
		descriptors.bind_task_outputs(&mut task_outputs);
		descriptors.bind_workgroup_state(&mut workgroup_state);
		program
			.run_workgroup(&mut descriptors, &configs)
			.expect("production task workgroup execution");
	}
	task_outputs
}

/// Executes one lane of an exact production task main with one meshlet.
fn run_single_meshlet_task(
	program: &ExecutableProgram,
	view_projections: &[(usize, [f32; 16])],
	selected_view_index: Option<u32>,
	center_radius: [f32; 4],
	skinned: bool,
) -> (Option<u32>, Option<Value>) {
	let outputs = run_meshlet_task_workgroup(program, view_projections, selected_view_index, &[center_radius], skinned, 0);
	(
		outputs.mesh_output_count(),
		outputs.payload_value("meshlet_instances", 0).cloned(),
	)
}

/// Verifies view-zero culling retains an intersecting meshlet and rejects one outside the frustum.
#[test]
fn visibility_task_main_emits_in_frustum_and_culls_off_frustum_meshlets() {
	let program = asset!("visibility-task.besl");
	let visible = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [0.0, 0.0, 0.5, 0.1], false);
	assert_eq!(visible, (Some(1), Some(fixture_meshlet_instance())));

	let culled = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [4.0, 0.0, 0.5, 0.1], false);
	assert_eq!(culled, (Some(0), None));
}

/// Verifies workgroup barriers and atomics compact visible meshlets in lane order before publishing the final count.
#[test]
fn visibility_task_workgroup_compacts_mixed_meshlets_in_lane_order() {
	let program = asset!("visibility-task.besl");
	let output = run_meshlet_task_workgroup(
		&program,
		&[(0, identity_matrix())],
		None,
		&[[0.0, 0.0, 0.5, 0.1], [4.0, 0.0, 0.5, 0.1], [0.5, 0.0, 0.5, 0.1]],
		false,
		0,
	);

	assert_eq!(output.mesh_output_count(), Some(2));
	assert_eq!(
		output.payload_value("meshlet_instances", 0),
		Some(&fixture_meshlet_instance())
	);
	assert_eq!(
		output.payload_value("meshlet_instances", 1),
		Some(&Value::U32(meshlet_instance(
			FIXTURE_MESHLET_INDEX as u32 + 2,
			FIXTURE_INSTANCE_INDEX as u32
		)))
	);
	assert_eq!(output.payload_value("meshlet_instances", 2), None);
}

/// Verifies visibility culling reads the work item selected by the global dispatch position.
#[test]
fn visibility_task_main_selects_later_batched_workgroup() {
	let program = asset!("visibility-task.besl");
	let output = run_meshlet_task_workgroup(&program, &[(0, identity_matrix())], None, &[[0.0, 0.0, 0.5, 0.1]], false, 1);

	assert_eq!(output.mesh_output_count(), Some(1));
	assert_eq!(
		output.payload_value("meshlet_instances", 0),
		Some(&fixture_meshlet_instance())
	);
}

/// Verifies deformed geometry reaches the mesh stage even when its static meshlet bound is outside the frustum.
#[test]
fn visibility_task_main_bypasses_static_culling_for_skinned_meshes() {
	let program = asset!("visibility-task.besl");
	let output = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [4.0, 0.0, 0.5, 0.1], true);
	assert_eq!(output, (Some(1), Some(fixture_meshlet_instance())));
}

/// Verifies shadow culling selects the cascade view named by the second push constant.
#[test]
fn shadow_task_main_uses_selected_view_index() {
	let program = asset!("shadow-task.besl");
	let mut view_projections: [(usize, [f32; 16]); 8] =
		std::array::from_fn(|view_index| (view_index, horizontally_translated_matrix(4.0)));
	view_projections[3].1 = identity_matrix();
	let output = run_single_meshlet_task(&program, &view_projections, Some(3), [0.0, 0.0, 0.5, 0.1], false);
	assert_eq!(output, (Some(1), Some(fixture_meshlet_instance())));
}

/// Verifies later object workgroups select their own compact work item from global thread positions.
#[test]
fn shadow_task_main_selects_later_batched_workgroup() {
	let program = asset!("shadow-task.besl");
	let output = run_meshlet_task_workgroup(
		&program,
		&[(3, identity_matrix())],
		Some(3),
		&[[0.0, 0.0, 0.5, 0.1]],
		false,
		1,
	);

	assert_eq!(output.mesh_output_count(), Some(1));
	assert_eq!(
		output.payload_value("meshlet_instances", 0),
		Some(&fixture_meshlet_instance())
	);
}

/// Executes one production mesh main over one identity triangle meshlet and verifies its complete output contract.
fn assert_triangle_mesh_program(
	program: ExecutableProgram,
	selected_view: Option<(usize, [f32; 16], u32)>,
	skinned_positions: Option<[[f32; 4]; 3]>,
	expected_clip_positions: [[f32; 4]; 3],
	expected_render_target_array_index: Option<u32>,
) {
	let mut views = buffer(&program, VIEWS_SLOT);
	views
		.write_indexed_field("views", 0, "view_projection", Value::Mat4F(identity_matrix()))
		.expect("mesh view");
	let mut meshes = buffer(&program, MESH_DATA_SLOT);
	meshes
		.write_indexed_field(
			"meshes",
			FIXTURE_INSTANCE_INDEX,
			"model",
			Value::Mat4x3F(identity_affine_matrix()),
		)
		.expect("mesh model matrix");
	for (field, value) in [
		("base_vertex_index", 0),
		("base_primitive_index", 0),
		("base_triangle_index", 0),
		("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
		("meshlet_count", 1),
		("skinned_base_vertex_index", u32::MAX),
	] {
		meshes
			.write_indexed_field("meshes", FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
			.expect("mesh offset");
	}
	let mut positions = buffer(&program, VERTEX_POSITIONS_SLOT);
	for (index, position) in [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
		positions
			.write_indexed("positions", index, Value::Vec3F(position))
			.expect("mesh vertex");
	}
	let mut skinned_vertices = buffer(&program, SKINNED_VERTICES_SLOT);
	let mut vertex_uvs = buffer(&program, VERTEX_UVS_SLOT);
	let mut vertex_indices = buffer(&program, VERTEX_INDICES_SLOT);
	let mut primitive_indices = buffer(&program, PRIMITIVE_INDICES_SLOT);
	for index in 0..3 {
		vertex_uvs
			.write_indexed("uvs", index, Value::Vec2F16([besl::vm::f16::ZERO; 2]))
			.expect("mesh UV");
		vertex_indices
			.write_indexed("vertex_indices", index, Value::U16(index as u16))
			.expect("vertex index");
		primitive_indices
			.write_indexed("primitive_indices", index, Value::U8(index as u8))
			.expect("triangle index");
	}
	let mut meshlets = buffer(&program, MESHLETS_SLOT);
	for (field, value) in [
		("primitive_offset", 0),
		("triangle_offset", 0),
		("primitive_count", 3),
		("triangle_count", 1),
	] {
		meshlets
			.write_indexed_field("meshlets", FIXTURE_MESHLET_INDEX, field, Value::U32(value))
			.expect("meshlet field");
	}
	if let Some(skinned_positions) = skinned_positions {
		const SKINNED_BASE_VERTEX: usize = 7;
		meshes
			.write_indexed_field(
				"meshes",
				FIXTURE_INSTANCE_INDEX,
				"skinned_base_vertex_index",
				Value::U32(SKINNED_BASE_VERTEX as u32),
			)
			.expect("skinned mesh vertices");
		for (index, position) in skinned_positions.into_iter().enumerate() {
			skinned_vertices
				.write_indexed_field("vertices", SKINNED_BASE_VERTEX + index, "position", Value::Vec4F(position))
				.expect("skinned mesh vertex");
		}
	}
	let mut push_constant = besl::vm::Buffer::new(program.push_constant_layout().expect("mesh push constant layout").clone());
	let (view_index, render_target_array_index) = match selected_view {
		Some((view_index, view_projection, render_target_array_index)) => {
			views
				.write_indexed_field("views", view_index, "view_projection", Value::Mat4F(view_projection))
				.expect("selected mesh view");
			(view_index as u32, render_target_array_index)
		}
		None => (0, 0),
	};
	push_constant.write("work_item_base", Value::U32(0)).expect("mesh work base");
	push_constant
		.write("view_index", Value::U32(view_index))
		.expect("mesh view index");
	push_constant
		.write("render_target_array_index", Value::U32(render_target_array_index))
		.expect("mesh target layer");

	let mut out_instance_indices = buffer(&program, output_slot(0));
	let mut out_primitive_indices = buffer(&program, output_slot(1));
	let mut out_uvs = buffer(&program, output_slot(2));
	let mut mesh_outputs = MeshOutputs::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_payload("meshlet_instances", [fixture_meshlet_instance()]);
		descriptors.bind_buffer(VIEWS_SLOT, &mut views);
		descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
		descriptors.bind_buffer(VERTEX_POSITIONS_SLOT, &mut positions);
		descriptors.bind_buffer(VERTEX_UVS_SLOT, &mut vertex_uvs);
		descriptors.bind_buffer(SKINNED_VERTICES_SLOT, &mut skinned_vertices);
		descriptors.bind_buffer(VERTEX_INDICES_SLOT, &mut vertex_indices);
		descriptors.bind_buffer(PRIMITIVE_INDICES_SLOT, &mut primitive_indices);
		descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
		descriptors.bind_buffer(output_slot(0), &mut out_instance_indices);
		descriptors.bind_buffer(output_slot(1), &mut out_primitive_indices);
		descriptors.bind_buffer(output_slot(2), &mut out_uvs);
		descriptors.bind_push_constant(&mut push_constant);
		descriptors.bind_mesh_outputs(&mut mesh_outputs);
		// Mesh invocations share their capture just as lanes in one production mesh workgroup share output arrays.
		for thread_idx in 0..3 {
			let config = ExecutionConfig::new(INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(thread_idx)
				.with_threadgroup_position(0);
			program
				.run_main_with_config(&mut descriptors, &config)
				.expect("production mesh shader execution");
		}
	}

	assert_eq!(mesh_outputs.vertex_count(), 3);
	assert_eq!(mesh_outputs.primitive_count(), 1);
	for (index, expected) in expected_clip_positions.into_iter().enumerate() {
		assert_rgba_close(
			mesh_outputs.vertex_position(index).expect("mesh vertex output"),
			expected,
			0.00001,
		);
	}
	assert_eq!(mesh_outputs.triangle(0), Some([0, 1, 2]));
	if let Some(expected) = expected_render_target_array_index {
		assert_eq!(mesh_outputs.render_target_array_index(0), Some(expected));
	}
	assert_eq!(
		read_u32(&out_instance_indices, "out_instance_index", 0),
		FIXTURE_INSTANCE_INDEX as u32
	);
	assert_eq!(
		read_u32(&out_primitive_indices, "out_primitive_index", 0),
		(FIXTURE_MESHLET_INDEX as u32) << 8
	);
}

/// Verifies visibility mesh output geometry and metadata through the BESL VM.
#[test]
fn visibility_mesh_main_emits_identity_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		None,
		None,
		[[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
		None,
	);
}

/// Verifies that posed instances source raster positions from their frame-local deformation range.
#[test]
fn visibility_mesh_main_reads_skinned_positions() {
	let skinned_positions = [[2.0, 3.0, 4.0, 1.0], [5.0, 6.0, 7.0, 1.0], [8.0, 9.0, 10.0, 1.0]];
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		None,
		Some(skinned_positions),
		skinned_positions,
		None,
	);
}

/// Verifies shadow mesh output keeps the selected view independent from the target texture-array layer.
#[test]
fn shadow_mesh_main_emits_selected_view_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("shadow-mesh.besl"),
		Some((7, horizontally_translated_matrix(2.0), 2)),
		None,
		[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		Some(2),
	);
}

/* Material prepasses */

/// Binds the instance-index image and mesh table and runs one 8x8 material-count workgroup.
fn run_material_count(
	program: &ExecutableProgram,
	mesh_data: &mut besl::vm::Buffer,
	instance_indices: &mut Texture,
) -> besl::vm::Buffer {
	let mut material_counts = buffer(program, MATERIAL_COUNT_SLOT);
	let mut workgroup = WorkgroupState::new();
	let configs = tile_configs::<MATERIAL_COUNT_WORKGROUP_SIZE>(MATERIAL_COUNT_WORKGROUP_WIDTH, [0, 0]);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(MESH_DATA_SLOT, mesh_data);
	descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
	descriptors.bind_image(INSTANCE_INDEX_SLOT, instance_indices);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("material-count workgroup execution");
	drop(descriptors);
	material_counts
}

/// Runs one 16x16 pixel-mapping workgroup over the instance-index image.
fn run_pixel_mapping(
	program: &ExecutableProgram,
	mesh_data: &mut besl::vm::Buffer,
	material_offset_scratch: &mut besl::vm::Buffer,
	instance_indices: &mut Texture,
) -> besl::vm::Buffer {
	let mut pixel_mapping = buffer(program, PIXEL_MAPPING_SLOT);
	let mut workgroup = WorkgroupState::new();
	let configs = tile_configs::<PIXEL_MAPPING_WORKGROUP_SIZE>(PIXEL_MAPPING_WORKGROUP_WIDTH, [0, 0]);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(MESH_DATA_SLOT, mesh_data);
	descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, material_offset_scratch);
	descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
	descriptors.bind_image(INSTANCE_INDEX_SLOT, instance_indices);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("pixel-mapping workgroup execution");
	drop(descriptors);
	pixel_mapping
}

/// Fills a square instance-index image where texel `lane` holds `instance(lane)`.
fn instance_texture(width: u32, instance: impl Fn(usize) -> u32) -> Texture {
	let mut texture = Texture::new(width, width).expect("instance index fixture");
	for lane in 0..(width * width) as usize {
		texture
			.write_u32([lane as u32 % width, lane as u32 / width], instance(lane))
			.expect("instance index texel");
	}
	texture
}

/// Exercises the production material prepasses as one stateful VM pipeline.
#[test]
fn visibility_material_compute_pipeline_counts_offsets_and_maps_valid_pixels() {
	let material_count_program = asset!("material-count.besl");
	let material_offset_program = asset!("material-offset.besl");
	let pixel_mapping_program = asset!("pixel-mapping.besl");

	// Three visible instances span two materials; the fourth texel is the renderer's empty-pixel sentinel.
	let mut mesh_data = buffer(&material_count_program, MESH_DATA_SLOT);
	for (mesh_index, material_index) in [(0, 2), (1, 5), (2, 2)] {
		mesh_data
			.write_indexed_field("meshes", mesh_index, "material_index", Value::U32(material_index))
			.expect("VM mesh");
	}
	let mut instance_indices = Texture::new(2, 2).expect("visibility index fixture");
	for (coordinate, instance_index) in [([0, 0], 0), ([1, 0], 1), ([0, 1], u32::MAX), ([1, 1], 2)] {
		instance_indices
			.write_u32(coordinate, instance_index)
			.expect("visibility index texel");
	}

	let mut material_counts = run_material_count(&material_count_program, &mut mesh_data, &mut instance_indices);
	assert_eq!(read_u32(&material_counts, "material_count", 2), 2);
	assert_eq!(read_u32(&material_counts, "material_count", 5), 1);
	assert_eq!(read_u32(&material_counts, "material_count", 0), 0);

	// The offset pass converts sparse counts into exclusive offsets and one indirect dispatch tuple per material.
	let mut material_offsets = buffer(&material_offset_program, MATERIAL_OFFSET_SLOT);
	let mut material_offset_scratch = buffer(&material_offset_program, MATERIAL_OFFSET_SCRATCH_SLOT);
	let mut material_dispatches = buffer(&material_offset_program, MATERIAL_DISPATCH_SLOT);
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
		descriptors.bind_buffer(MATERIAL_OFFSET_SLOT, &mut material_offsets);
		descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
		descriptors.bind_buffer(MATERIAL_DISPATCH_SLOT, &mut material_dispatches);
		run_at(&material_offset_program, &mut descriptors, [0, 0]);
	}
	assert_eq!(read_u32(&material_offsets, "material_offset", 2), 0);
	assert_eq!(read_u32(&material_offsets, "material_offset", 5), 2);
	assert_eq!(read_u32(&material_offsets, "material_offset", 6), 3);
	// The offset pass does not clear material_count; evaluation reads it directly for bounds.
	assert_eq!(read_u32(&material_counts, "material_count", 2), 2);
	assert_eq!(read_u32(&material_counts, "material_count", 5), 1);
	assert_eq!(
		read_vec3u(&material_dispatches, "material_evaluation_dispatches", 0),
		[0, 1, 1]
	);
	assert_eq!(
		read_vec3u(&material_dispatches, "material_evaluation_dispatches", 2),
		[1, 1, 1]
	);
	assert_eq!(
		read_vec3u(&material_dispatches, "material_evaluation_dispatches", 5),
		[1, 1, 1]
	);

	// Mapping reuses the scratch offsets as atomic cursors and stores one-based coordinates for later zero-sentinel checks.
	let pixel_mapping = run_pixel_mapping(
		&pixel_mapping_program,
		&mut mesh_data,
		&mut material_offset_scratch,
		&mut instance_indices,
	);
	assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 0), [1, 1]);
	assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 1), [2, 2]);
	assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 2), [2, 1]);
	assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 2), 2);
	assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 5), 3);
}

/// Verifies a coherent tile reuses its established local key while preserving every pixel mapping.
#[test]
fn pixel_mapping_load_fast_path_preserves_coherent_tile_mappings() {
	let program = asset!("pixel-mapping.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	mesh_data
		.write_indexed_field("meshes", 0, "material_index", Value::U32(7))
		.expect("coherent mesh");
	let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
	let mut instance_indices = instance_texture(PIXEL_MAPPING_WORKGROUP_WIDTH, |_| 0);

	let pixel_mapping = run_pixel_mapping(&program, &mut mesh_data, &mut material_offset_scratch, &mut instance_indices);

	let width = PIXEL_MAPPING_WORKGROUP_WIDTH as usize;
	let mut seen = vec![false; width * width];
	for mapping_index in 0..PIXEL_MAPPING_WORKGROUP_SIZE {
		let [x, y] = read_vec2u16(&pixel_mapping, "pixel_mapping", mapping_index).map(usize::from);
		assert!(
			(1..=width).contains(&x) && (1..=width).contains(&y),
			"Pixel Mapping returned an invalid coherent-tile coordinate. The most likely cause is that the fast path reused a local rank."
		);
		let slot = &mut seen[(y - 1) * width + (x - 1)];
		assert!(!*slot, "Pixel Mapping duplicated a coherent-tile coordinate.");
		*slot = true;
	}
	assert!(
		seen.into_iter().all(|coordinate| coordinate),
		"Pixel Mapping omitted a coherent-tile coordinate."
	);
	assert_eq!(
		read_u32(&material_offset_scratch, "material_offset_scratch", 7),
		PIXEL_MAPPING_WORKGROUP_SIZE as u32,
		"Pixel Mapping advanced the coherent material cursor incorrectly."
	);
}

/// Verifies tile-local reservations preserve mappings when distinct materials exceed the bounded histogram.
#[test]
fn pixel_mapping_tile_reservation_preserves_overflowed_materials() {
	let program = asset!("pixel-mapping.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
	for material_index in 0..33 {
		mesh_data
			.write_indexed_field("meshes", material_index, "material_index", Value::U32(material_index as u32))
			.expect("VM mesh");
		material_offset_scratch
			.write_indexed("material_offset_scratch", material_index, Value::U32(material_index as u32))
			.expect("material mapping offset");
	}
	let mut instance_indices = instance_texture(
		PIXEL_MAPPING_WORKGROUP_WIDTH,
		|lane| if lane < 33 { lane as u32 } else { u32::MAX },
	);

	let pixel_mapping = run_pixel_mapping(&program, &mut mesh_data, &mut material_offset_scratch, &mut instance_indices);

	for material_index in 0..33 {
		let expected_coordinate = [
			(material_index % PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
			(material_index / PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
		];
		assert_eq!(
			read_vec2u16(&pixel_mapping, "pixel_mapping", material_index),
			expected_coordinate,
			"Unexpected coordinate for material {material_index}. The most likely cause is a dropped tile reservation."
		);
		assert_eq!(
			read_u32(&material_offset_scratch, "material_offset_scratch", material_index),
			material_index as u32 + 1,
			"Unexpected cursor for material {material_index}. The most likely cause is a duplicated tile reservation."
		);
	}
}

/// Verifies a tile with more unique materials than histogram slots preserves every count through the overflow path.
#[test]
fn material_count_tile_histogram_preserves_overflowed_materials() {
	let program = asset!("material-count.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	for material_index in 0..33 {
		mesh_data
			.write_indexed_field("meshes", material_index, "material_index", Value::U32(material_index as u32))
			.expect("VM mesh");
	}
	let mut instance_indices = instance_texture(MATERIAL_COUNT_WORKGROUP_WIDTH, |lane| (lane % 33) as u32);

	let material_counts = run_material_count(&program, &mut mesh_data, &mut instance_indices);

	for material_index in 0..33 {
		let expected = if material_index < 31 { 2 } else { 1 };
		assert_eq!(
			read_u32(&material_counts, "material_count", material_index),
			expected,
			"Unexpected count for material {material_index}. The most likely cause is a dropped or duplicated tile-histogram entry."
		);
	}
}

/// Verifies subgroup aggregation retains every pixel in a coherent Material Count tile.
#[test]
fn material_count_subgroup_aggregation_counts_a_coherent_tile_once_per_partition() {
	let program = asset!("material-count.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	mesh_data
		.write_indexed_field("meshes", 0, "material_index", Value::U32(7))
		.expect("coherent mesh");
	let mut instance_indices = instance_texture(MATERIAL_COUNT_WORKGROUP_WIDTH, |_| 0);

	let material_counts = run_material_count(&program, &mut mesh_data, &mut instance_indices);

	assert_eq!(
		read_u32(&material_counts, "material_count", 7),
		MATERIAL_COUNT_WORKGROUP_SIZE as u32
	);
}

/* GTAO */

const GTAO_NEAR: f32 = 0.1;
const GTAO_FAR: f32 = 100.0;

/// Creates compact camera data for one square GTAO shader fixture.
fn gtao_view_data(program: &ExecutableProgram, width: u32, height: u32) -> besl::vm::Buffer {
	let projection = math::projection_matrix(math::Degrees::new(60.0), width as f32 / height as f32, GTAO_NEAR, GTAO_FAR);
	let projection_x = projection[0];
	let projection_y = projection[5];
	let width = width as f32;
	let height = height as f32;
	let mut view = buffer(program, VIEWS_SLOT);
	for (member, value) in [
		(
			"pixel_to_ray_mul",
			Value::Vec2F([2.0 / (width * projection_x), -2.0 / (height * projection_y)]),
		),
		(
			"pixel_to_ray_add",
			Value::Vec2F([(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y]),
		),
		("projection_pixels_y", Value::F32(height * projection_y * 0.5)),
		("view_z_sign", Value::F32(1.0)),
		(
			"depth_unproject_numerator",
			Value::F32(GTAO_NEAR * GTAO_FAR / (GTAO_FAR - GTAO_NEAR)),
		),
		(
			"depth_unproject_denominator_offset",
			Value::F32(GTAO_NEAR / (GTAO_FAR - GTAO_NEAR)),
		),
	] {
		view.write(member, value).expect("compact GTAO view data");
	}
	view
}

/// Creates GTAO runtime controls.
fn gtao_parameters_data(program: &ExecutableProgram, radius: f32, samples_per_ray: u32, radial_rays: u32) -> besl::vm::Buffer {
	let mut parameters = buffer(program, GTAO_PARAMETERS_SLOT);
	for (member, value) in [
		("radius", Value::F32(radius)),
		("samples_per_ray", Value::U32(samples_per_ray)),
		("radial_rays", Value::U32(radial_rays)),
	] {
		parameters.write(member, value).expect("GTAO runtime parameters");
	}
	parameters
}

/// Reconstructs the positive fixture distance encoded by one reversed device depth.
fn gtao_fixture_linear_depth(depth: f32) -> f32 {
	if depth == 0.0 {
		return 0.0;
	}
	let range = GTAO_FAR - GTAO_NEAR;
	(GTAO_NEAR * GTAO_FAR / range) / (depth + GTAO_NEAR / range)
}

/// Reduces one positive-linear-depth image while ignoring zero-valued background texels.
fn reduce_nearest_nonzero_depth(source: &[[f32; 4]], width: u32, height: u32) -> (Vec<[f32; 4]>, u32, u32) {
	let reduced_width = width.div_ceil(2).max(1);
	let reduced_height = height.div_ceil(2).max(1);
	let mut reduced = vec![[0.0, 0.0, 0.0, 1.0]; (reduced_width * reduced_height) as usize];
	for y in 0..reduced_height {
		for x in 0..reduced_width {
			let mut nearest = 0.0f32;
			for (offset_x, offset_y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
				let source_x = (x * 2 + offset_x).min(width - 1);
				let source_y = (y * 2 + offset_y).min(height - 1);
				let depth = source[(source_y * width + source_x) as usize][0];
				if depth != 0.0 && (nearest == 0.0 || depth < nearest) {
					nearest = depth;
				}
			}
			reduced[(y * reduced_width + x) as usize][0] = nearest;
		}
	}
	(reduced, reduced_width, reduced_height)
}

/// Builds a GTAO depth pyramid whose mip zero is a placeholder at twice the fixture extent.
fn gtao_depth_pyramid(width: u32, height: u32, levels: [&[[f32; 4]]; 3], extents: [(u32, u32); 2]) -> Texture {
	let mut pyramid = texture_2d(
		width * 2,
		height * 2,
		&vec![[0.0, 0.0, 0.0, 1.0]; (width * 2 * height * 2) as usize],
	);
	pyramid.add_mip(texture_2d(width, height, levels[0]));
	pyramid.add_mip(texture_2d(extents[0].0, extents[0].1, levels[1]));
	pyramid.add_mip(texture_2d(extents[1].0, extents[1].1, levels[2]));
	pyramid
}

/// Runs one GTAO workgroup containing `coordinate` and reads that pixel.
fn run_gtao_workgroup(
	program: &ExecutableProgram,
	view: &mut besl::vm::Buffer,
	parameters: &mut besl::vm::Buffer,
	depth_pyramid: &mut Texture,
	extent: [u32; 2],
	coordinate: [u32; 2],
) -> [f32; 4] {
	let mut output = empty_image(extent[0], extent[1]);
	let base = [
		coordinate[0] / GTAO_WORKGROUP_WIDTH * GTAO_WORKGROUP_WIDTH,
		coordinate[1] / GTAO_WORKGROUP_HEIGHT * GTAO_WORKGROUP_HEIGHT,
	];
	let configs = tile_configs::<GTAO_WORKGROUP_SIZE>(GTAO_WORKGROUP_WIDTH, base);
	let mut workgroup = WorkgroupState::new();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, view);
	descriptors.bind_buffer(GTAO_PARAMETERS_SLOT, parameters);
	descriptors.bind_texture(ResourceSlot::new(1033), depth_pyramid);
	descriptors.bind_image(ResourceSlot::new(1034), &mut output);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("GTAO workgroup execution");
	drop(descriptors);
	rgba(&output, coordinate)
}

/// Executes GTAO over a flat floor whose pixel footprint crosses the former absolute normal cutoff.
fn run_gtao_floor_fixture(program: &ExecutableProgram, camera_height: f32, coordinate: [u32; 2]) -> [f32; 4] {
	const EXTENT: u32 = 64;
	let projection = math::projection_matrix(math::Degrees::new(60.0), 1.0, GTAO_NEAR, GTAO_FAR);
	let ray_mul_y = -2.0 / (EXTENT as f32 * projection[5]);
	let ray_add_y = (1.0 - 1.0 / EXTENT as f32) / projection[5];
	let mut linear_depth = vec![[0.0, 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize];
	for y in 0..EXTENT {
		let ray_y = y as f32 * ray_mul_y + ray_add_y;
		if ray_y >= 0.0 {
			continue;
		}
		let depth = (0.0 - camera_height) / ray_y;
		if !(GTAO_NEAR..=GTAO_FAR).contains(&depth) {
			continue;
		}
		for x in 0..EXTENT {
			linear_depth[(y * EXTENT + x) as usize][0] = depth;
		}
	}
	let (linear_depth_1, width_1, height_1) = reduce_nearest_nonzero_depth(&linear_depth, EXTENT, EXTENT);
	let (linear_depth_2, width_2, height_2) = reduce_nearest_nonzero_depth(&linear_depth_1, width_1, height_1);
	let mut view = gtao_view_data(program, EXTENT, EXTENT);
	let mut parameters = gtao_parameters_data(program, 1.0, 4, 6);
	let mut depth_pyramid = gtao_depth_pyramid(
		EXTENT,
		EXTENT,
		[&linear_depth, &linear_depth_1, &linear_depth_2],
		[(width_1, height_1), (width_2, height_2)],
	);
	run_gtao_workgroup(
		program,
		&mut view,
		&mut parameters,
		&mut depth_pyramid,
		[EXTENT, EXTENT],
		coordinate,
	)
}

/// Executes the standard GTAO shader with one deterministic device-depth fixture and explicit runtime controls.
fn run_gtao_fixture(
	program: &ExecutableProgram,
	width: u32,
	height: u32,
	depth_texels: &[[f32; 4]],
	coordinate: [u32; 2],
	(radius, samples_per_ray, radial_rays): (f32, u32, u32),
) -> [f32; 4] {
	let mut view = gtao_view_data(program, width, height);
	let mut parameters = gtao_parameters_data(program, radius, samples_per_ray, radial_rays);
	let linear_depth_texels = depth_texels
		.iter()
		.map(|texel| [gtao_fixture_linear_depth(texel[0]), 0.0, 0.0, 1.0])
		.collect::<Vec<_>>();
	let extent_1 = ((width / 2).max(1), (height / 2).max(1));
	let extent_2 = ((width / 4).max(1), (height / 4).max(1));
	let empty_1 = vec![[0.0, 0.0, 0.0, 1.0]; (extent_1.0 * extent_1.1) as usize];
	let empty_2 = vec![[0.0, 0.0, 0.0, 1.0]; (extent_2.0 * extent_2.1) as usize];
	let mut depth_pyramid = gtao_depth_pyramid(
		width,
		height,
		[&linear_depth_texels, &empty_1, &empty_2],
		[extent_1, extent_2],
	);
	run_gtao_workgroup(
		program,
		&mut view,
		&mut parameters,
		&mut depth_pyramid,
		[width, height],
		coordinate,
	)
}

/// Runs the production GTAO shader with uniform coarse levels so a fixture can isolate hierarchical sampling.
fn run_gtao_hierarchical_fixture(program: &ExecutableProgram, coarse_linear_depth: f32) -> [f32; 4] {
	const EXTENT: u32 = 129;
	const CENTER: [u32; 2] = [64, 64];
	let linear_depth_texels = vec![[gtao_fixture_linear_depth(0.35), 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize];
	let coarse_1 = vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 64 * 64];
	let coarse_2 = vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 32 * 32];
	let mut view = gtao_view_data(program, EXTENT, EXTENT);
	let mut parameters = gtao_parameters_data(program, 1.0, 4, 6);
	let mut depth_pyramid = gtao_depth_pyramid(
		EXTENT,
		EXTENT,
		[&linear_depth_texels, &coarse_1, &coarse_2],
		[(EXTENT / 2, EXTENT / 2), (EXTENT / 4, EXTENT / 4)],
	);
	run_gtao_workgroup(
		program,
		&mut view,
		&mut parameters,
		&mut depth_pyramid,
		[EXTENT, EXTENT],
		CENTER,
	)
}

/// Runs the fused GTAO depth pyramid over `source` and returns the three reduced levels.
fn run_gtao_depth_pyramid(program: &ExecutableProgram, source: &mut Texture, width: u32, height: u32) -> [Texture; 3] {
	let mut reduced = [
		empty_image((width / 2).max(1), (height / 2).max(1)),
		empty_image((width / 4).max(1), (height / 4).max(1)),
		empty_image((width / 8).max(1), (height / 8).max(1)),
	];
	let mut view = gtao_view_data(program, width, height);
	let mut workgroup = WorkgroupState::new();
	let configs = tile_configs::<GTAO_PYRAMID_WORKGROUP_SIZE>(GTAO_PYRAMID_WORKGROUP_WIDTH, [0, 0]);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_texture_with_sampler(ResourceSlot::new(1033), source, Sampler::new(SamplerReductionMode::Max));
	let [reduced_1, reduced_2, reduced_3] = &mut reduced;
	descriptors.bind_image(ResourceSlot::new(1034), reduced_1);
	descriptors.bind_image(ResourceSlot::new(1035), reduced_2);
	descriptors.bind_image(ResourceSlot::new(1036), reduced_3);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("fused GTAO depth pyramid execution");
	drop(descriptors);
	reduced
}

/// Verifies each production depth-pyramid texel keeps the nearest nonzero linear depth in its source footprint.
#[test]
fn gtao_depth_pyramid_reduces_odd_extents_to_nearest_linear_depth() {
	let program = asset!("gtao-depth-pyramid.besl");
	let texels = [0.0, 0.2, 0.3, 0.4, 0.9, 0.5, 0.6, 0.7, 0.8].map(|depth| [depth, 0.0, 0.0, 1.0]);
	let mut source = texture_2d(3, 3, &texels);

	let reduced = run_gtao_depth_pyramid(&program, &mut source, 3, 3);

	let nearest = [gtao_fixture_linear_depth(0.9), 0.0, 0.0, 1.0];
	for level in &reduced {
		assert_rgba_close(rgba(level, [0, 0]), nearest, 0.00001);
	}
}

/// Verifies one SIMD group keeps the two adjacent source tiles independent through every emitted level.
#[test]
fn gtao_depth_pyramid_reduces_two_tiles_without_cross_tile_leakage() {
	let program = asset!("gtao-depth-pyramid.besl");
	let mut source_texels = Vec::with_capacity(16 * 8);
	for y in 0..8u32 {
		for x in 0..16u32 {
			let block = (y / 2) * 8 + x / 2;
			let maximum = if block == 11 { 0.0 } else { 0.1 + block as f32 * 0.02 };
			let maximum_corner = [block % 2, (block / 2) % 2];
			let depth = if [x % 2, y % 2] == maximum_corner {
				maximum
			} else {
				maximum * 0.25
			};
			source_texels.push([depth, 0.0, 0.0, 1.0]);
		}
	}
	let mut source = texture_2d(16, 8, &source_texels);

	let [reduced_1, reduced_2, reduced_3] = run_gtao_depth_pyramid(&program, &mut source, 16, 8);

	let expected_1: Vec<[f32; 4]> = (0..32u32)
		.map(|block| {
			let depth = if block == 11 {
				0.0
			} else {
				gtao_fixture_linear_depth(0.1 + block as f32 * 0.02)
			};
			[depth, 0.0, 0.0, 1.0]
		})
		.collect();
	let (expected_2, ..) = reduce_nearest_nonzero_depth(&expected_1, 8, 4);
	let (expected_3, ..) = reduce_nearest_nonzero_depth(&expected_2, 4, 2);
	for y in 0..4 {
		for x in 0..8 {
			assert_rgba_close(rgba(&reduced_1, [x, y]), expected_1[(y * 8 + x) as usize], 0.00001);
		}
	}
	for y in 0..2 {
		for x in 0..4 {
			assert_rgba_close(rgba(&reduced_2, [x, y]), expected_2[(y * 4 + x) as usize], 0.00001);
		}
	}
	for x in 0..2 {
		assert_rgba_close(rgba(&reduced_3, [x, 0]), expected_3[x as usize], 0.00001);
	}
}

/// Verifies one SIMD group emits the retained 4x4 level for two adjacent tiles in every cascade.
#[test]
fn directional_shadow_depth_pyramid_reduces_every_cascade_in_one_dispatch_shape() {
	let program = asset!("directional-shadow-depth-pyramid.besl");
	let layer_count = 4u32;
	let cell_maximum =
		|layer: u32, cell_x: u32, cell_y: u32| 0.1 + layer as f32 * 0.15 + cell_y as f32 * 0.04 + cell_x as f32 * 0.01;
	let mut source = Texture::new_3d(16, 8, layer_count).expect("directional shadow array fixture");
	for layer in 0..layer_count {
		for y in 0..8 {
			for x in 0..16 {
				let maximum = cell_maximum(layer, x / 4, y / 4);
				let depth = if x % 4 == layer && y % 4 == 3 - layer {
					maximum
				} else {
					maximum * 0.5
				};
				source
					.write_3d([x, y, layer], [depth, 0.0, 0.0, 1.0])
					.expect("directional shadow source texel");
			}
		}
	}
	let mut reduced = empty_image(4, 8);
	for layer in 0..layer_count {
		let configs = tile_configs::<DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_SIZE>(
			DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_WIDTH,
			[0, layer * DIRECTIONAL_SHADOW_PYRAMID_WORKGROUP_HEIGHT],
		);
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(1033), &mut source);
		descriptors.bind_image(ResourceSlot::new(1034), &mut reduced);
		descriptors.bind_workgroup_state(&mut workgroup);
		program
			.run_workgroup(&mut descriptors, &configs)
			.expect("fused directional shadow pyramid execution");
	}
	for layer in 0..layer_count {
		for cell_y in 0..2 {
			for cell_x in 0..4 {
				assert_rgba_close(
					rgba(&reduced, [cell_x, layer * 2 + cell_y]),
					[cell_maximum(layer, cell_x, cell_y), 0.0, 0.0, 1.0],
					0.00001,
				);
			}
		}
	}
}

/// Verifies distant GTAO steps consume conservative hierarchy levels instead of always fetching full-resolution depth.
#[test]
fn gtao_uses_depth_pyramid_for_distant_samples() {
	let program = asset!("gtao.besl");
	let empty_coarse_depth = run_gtao_hierarchical_fixture(&program, 0.0);
	let occupied_coarse_depth = run_gtao_hierarchical_fixture(&program, gtao_fixture_linear_depth(0.4));
	assert!(
		occupied_coarse_depth[0] < empty_coarse_depth[0],
		"Expected populated coarse depth to increase distant occlusion, found empty={empty_coarse_depth:?} and occupied={occupied_coarse_depth:?}. The most likely cause is that GTAO stopped selecting hierarchy levels."
	);
}

/// Verifies the standard GTAO shader's background contract and recessed-foreground response.
#[test]
fn gtao_writes_white_for_background_and_expected_recessed_foreground_ao() {
	let program = asset!("gtao.besl");
	let background = run_gtao_fixture(&program, 1, 1, &[[0.0, 0.0, 0.0, 1.0]], [0, 0], (1.0, 6, 8));
	assert_rgba_close(background, [1.0, 1.0, 1.0, 1.0], 0.00001);

	// A recessed center surrounded by nearer depth exercises reconstruction, normal estimation, and the
	// adaptive bounded AO integral.
	let mut foreground_depth = [[0.75, 0.0, 0.0, 1.0]; 25];
	foreground_depth[12] = [0.35, 0.0, 0.0, 1.0];
	let foreground = run_gtao_fixture(&program, 5, 5, &foreground_depth, [2, 2], (1.0, 6, 8));
	assert_rgba_close(foreground, [0.8315444, 0.8315444, 0.8315444, 1.0], 0.00001);

	let disabled = run_gtao_fixture(&program, 5, 5, &foreground_depth, [2, 2], (0.0, 1, 2));
	assert_rgba_close(disabled, [1.0, 1.0, 1.0, 1.0], 0.00001);
}

/// Verifies flat-floor normals remain valid when their world-space finite differences become very small.
#[test]
fn gtao_floor_has_no_scale_dependent_normal_seam() {
	let program = asset!("gtao.besl");
	let larger_floor = run_gtao_floor_fixture(&program, 0.1, [32, 63]);
	let scaled_floor = run_gtao_floor_fixture(&program, 0.06, [32, 63]);
	assert!(
		(larger_floor[0] - scaled_floor[0]).abs() < 0.0005,
		"Expected geometrically identical floors to preserve AO across world scales, got large={} and scaled={}. The most likely cause is a scale-dependent normal fallback.",
		larger_floor[0],
		scaled_floor[0]
	);
}

/// Runs one complete GTAO blur workgroup and reads the selected output pixel.
fn run_gtao_blur_fixture(
	program: &ExecutableProgram,
	width: u32,
	height: u32,
	depth_texels: &[[f32; 4]],
	ao_texels: &[[f32; 4]],
	coordinate: [u32; 2],
) -> [f32; 4] {
	let mut depth = texture_2d(width, height, depth_texels);
	let mut ao = texture_2d(width, height, ao_texels);
	let mut output = empty_image(width, height);
	let mut workgroup = WorkgroupState::new();
	let configs = tile_configs::<GTAO_BLUR_WORKGROUP_SIZE>(GTAO_BLUR_WORKGROUP_WIDTH, [0, 0]);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
	descriptors.bind_texture(ResourceSlot::new(1034), &mut ao);
	descriptors.bind_image(ResourceSlot::new(1035), &mut output);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("GTAO blur workgroup execution");
	drop(descriptors);
	rgba(&output, coordinate)
}

/// Runs the production depth-aware upscale workgroup and reads one full-resolution output pixel.
fn run_gtao_upscale_fixture(
	program: &ExecutableProgram,
	full_extent: [u32; 2],
	device_depth_texels: &[[f32; 4]],
	low_extent: [u32; 2],
	linear_depth_texels: &[[f32; 4]],
	ao_texels: &[[f32; 4]],
	coordinate: [u32; 2],
) -> [f32; 4] {
	let mut view = gtao_view_data(program, low_extent[0], low_extent[1]);
	let mut device_depth = texture_2d(full_extent[0], full_extent[1], device_depth_texels);
	let mut linear_depth = texture_2d(low_extent[0], low_extent[1], linear_depth_texels);
	let mut ao = texture_2d(low_extent[0], low_extent[1], ao_texels);
	let mut output = empty_image(full_extent[0], full_extent[1]);
	let mut workgroup = WorkgroupState::new();
	let base = [
		coordinate[0] / GTAO_BLUR_WORKGROUP_WIDTH * GTAO_BLUR_WORKGROUP_WIDTH,
		coordinate[1] / GTAO_BLUR_WORKGROUP_WIDTH * GTAO_BLUR_WORKGROUP_WIDTH,
	];
	let configs = tile_configs::<GTAO_BLUR_WORKGROUP_SIZE>(GTAO_BLUR_WORKGROUP_WIDTH, base);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_texture(ResourceSlot::new(1033), &mut device_depth);
	descriptors.bind_texture(ResourceSlot::new(1034), &mut ao);
	descriptors.bind_image(ResourceSlot::new(1035), &mut output);
	descriptors.bind_texture(ResourceSlot::new(1036), &mut linear_depth);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("GTAO upscale workgroup execution");
	drop(descriptors);
	rgba(&output, coordinate)
}

/// Verifies the half-resolution horizontal denoiser preserves uniform AO and smooths its axis.
#[test]
fn gtao_half_resolution_blur_preserves_uniform_ao_and_smooths_horizontally() {
	let blur_x = asset!("gtao-blur-x.besl");
	let depth = [[0.5, 0.0, 0.0, 1.0]; 25];
	let uniform_ao = [[0.37, 0.0, 0.0, 1.0]; 25];
	assert_rgba_close(
		run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &uniform_ao, [2, 2]),
		[0.37, 0.0, 0.0, 1.0],
		0.00001,
	);

	// Horizontal variation must be reduced before the final reconstruction stage.
	let directional_ao: [[f32; 4]; 25] = std::array::from_fn(|index| {
		if index % 5 == 2 {
			[1.0, 0.0, 0.0, 1.0]
		} else {
			[0.0, 0.0, 0.0, 1.0]
		}
	});
	let horizontal = run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &directional_ao, [2, 2]);
	assert!(
		horizontal[0] < 0.8,
		"Expected X blur to mix neighboring columns, found {horizontal:?}"
	);
}

/// Verifies full-resolution reconstruction preserves uniform input and rejects AO across depth discontinuities.
#[test]
fn gtao_upscale_is_depth_aware_and_preserves_uniform_ao() {
	let upscale = asset!("gtao-upscale.besl");
	let uniform_device_depth = vec![[0.5, 0.0, 0.0, 1.0]; 35];
	let uniform_linear_depth = vec![[gtao_fixture_linear_depth(0.5), 0.0, 0.0, 1.0]; 12];
	let uniform_ao = vec![[0.37, 0.0, 0.0, 1.0]; 12];
	assert_rgba_close(
		run_gtao_upscale_fixture(
			&upscale,
			[7, 5],
			&uniform_device_depth,
			[4, 3],
			&uniform_linear_depth,
			&uniform_ao,
			[6, 4],
		),
		[0.37, 0.0, 0.0, 1.0],
		0.00001,
	);

	let full_extent = [8, 8];
	let low_extent = [4, 4];
	let device_depth: [[f32; 4]; 64] = std::array::from_fn(|index| [if index % 8 < 4 { 0.7 } else { 0.3 }, 0.0, 0.0, 1.0]);
	let linear_depth: [[f32; 4]; 16] = std::array::from_fn(|index| {
		[
			gtao_fixture_linear_depth(if index % 4 < 2 { 0.7 } else { 0.3 }),
			0.0,
			0.0,
			1.0,
		]
	});
	let ao: [[f32; 4]; 16] = std::array::from_fn(|index| [if index % 4 < 2 { 0.2 } else { 0.8 }, 0.0, 0.0, 1.0]);
	let left = run_gtao_upscale_fixture(&upscale, full_extent, &device_depth, low_extent, &linear_depth, &ao, [3, 3]);
	let right = run_gtao_upscale_fixture(&upscale, full_extent, &device_depth, low_extent, &linear_depth, &ao, [4, 3]);
	assert!(
		left[0] < 0.3 && right[0] > 0.7,
		"Expected reconstruction to preserve the AO edge, found left={left:?} and right={right:?}. The most likely cause is missing low-resolution depth rejection."
	);
}
