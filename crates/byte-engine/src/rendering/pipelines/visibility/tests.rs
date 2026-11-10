//! Executes the checked-in visibility BESL assets in the BESL VM against small fixtures, and places meshes in the
//! geometry streams.

use besl::vm::{
	Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, MeshOutputs, ResourceSlot, TaskOutputs, Texture, Value,
	WorkgroupState, input_slot, output_slot,
};

use super::geometry::{GeometryCapacity, GeometryCounts, GeometryRanges, RetiredRanges};
use super::layout::ShaderMeshletData;
use super::mesh_dispatch::MeshDispatchWorkItem;
use super::render_pass::{OCCLUSION_PYRAMID_HEIGHT, OCCLUSION_PYRAMID_MIP_COUNT, OCCLUSION_PYRAMID_WIDTH, OcclusionPhase};
use super::shader_data::{MESH_FLAG_DOUBLE_SIDED, MESH_FLAG_SKINNED};
use crate::rendering::shader_vm_test::{
	IDENTITY_MATRIX, array_buffer, assert_rgba_close, buffer, column_major, compile, empty_image, input_buffer, lane_config,
	output_buffer, push_constant_buffer, rgba, run_at, slot, texture_2d,
};

const VIEWS: &str = "views";
const GTAO_VIEW: &str = "gtao_view";
const SCREEN_VIEW: &str = "screen_view";
const GTAO_PARAMETERS: &str = "gtao_parameters";
const MESH_DATA: &str = "meshes";
const MATERIAL_EVALUATIONS: &str = "material_evaluations";
const MATERIAL_COUNT: &str = "material_count_buffer";
const MATERIAL_OFFSET: &str = "material_offset_buffer";
const MATERIAL_OFFSET_SCRATCH: &str = "material_offset_scratch_buffer";
const MATERIAL_DISPATCH: &str = "material_evaluation_dispatches";
const PIXEL_MAPPING: &str = "pixel_mapping_buffer";
const INSTANCE_INDEX: &str = "instance_index_render_target";
const LIT: &str = "lit_map";
const DIFFUSE_RADIANCE_HISTORY: &str = "diffuse_radiance_map";
const MESH_DISPATCH_WORK: &str = "mesh_dispatch_work";
const OCCLUSION_PYRAMID: &str = "occlusion_pyramid";
const OCCLUSION_VISIBILITY: &str = "occlusion_visibility";
const VERTEX_POSITIONS: &str = "vertex_positions";
const VERTEX_UVS: &str = "vertex_uvs";
const VERTEX_INDICES: &str = "vertex_indices";
const PRIMITIVE_INDICES: &str = "primitive_indices";
const MESHLETS: &str = "meshlets";
const MESH_LEVELS: &str = "mesh_levels";
const FIXTURE_INSTANCE_INDEX: usize = 3;
/// The fixture instance's level record.
const FIXTURE_LEVEL_INDEX: usize = 2;
/// The length of the fixtures' runtime-sized instance and work tables.
const FIXTURE_TABLE_LENGTH: usize = 64;
const FIXTURE_MESHLET_INDEX: usize = 5;
const MESHLET_INSTANCE_BITS: u32 = 12;
/// Task payload words keep the view's offset into its batch above the meshlet index.
const VIEW_OFFSET_SHIFT: u32 = MESHLET_INSTANCE_BITS;
/// The task payload word that holds the workgroup's work, after the six views of 32 meshlets.
const PAYLOAD_WORK_WORD: usize = 192;
/// The flag of a payload's work word whose lanes each have a work item of their own.
const PAYLOAD_WORK_SHARED: u32 = 1 << 31;
/// The flag of the last payload entry of a group.
const ENTRY_ENDS_GROUP: u32 = 1 << 21;
const TASK_WORKGROUP_SIZE: u32 = 32;
const GTAO_WORKGROUP_WIDTH: u32 = 16;
const GTAO_WORKGROUP_SIZE: usize = 128;
/// The 8x4 workgroup of the GTAO and directional-shadow depth pyramids.
const PYRAMID_WORKGROUP_WIDTH: u32 = 8;
const PYRAMID_WORKGROUP_HEIGHT: u32 = 4;
const PYRAMID_WORKGROUP_SIZE: usize = 32;
/// The linear depth pyramid pass's workgroup.
const DEPTH_PYRAMID_WORKGROUP_WIDTH: u32 = 16;
const DEPTH_PYRAMID_WORKGROUP_HEIGHT: u32 = 16;
const DEPTH_PYRAMID_WORKGROUP_SIZE: usize = 256;
const PIXEL_MAPPING_WORKGROUP_WIDTH: u32 = 16;
const PIXEL_MAPPING_WORKGROUP_SIZE: usize = 256;
/// The 8x8 workgroup most visibility compute passes use, such as the material count, the occlusion pyramid, the GTAO
/// blur and upscale, the sun visibility resolve, the SSGI trace and temporal passes, and the receiver bounds.
const TILE_WORKGROUP_WIDTH: u32 = 8;
const TILE_WORKGROUP_SIZE: usize = 64;

/// Parses and links one visibility shader source that production baking consumes.
///
/// Returns the program rather than its `main`, because the program owns every function it calls.
fn asset_program(source: &str) -> besl::NodeReference {
	let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
	// Baking gives every standalone shader the common helpers, such as `source_over`.
	root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
	let program = besl::lex(root)
		.expect("Failed to link a visibility shader asset. The most likely cause is an invalid shader declaration.");
	program
		.get_main()
		.expect("Missing visibility shader main. The most likely cause is that a checked-in BESL asset is incomplete.");
	program
}

/// Reads one checked-in visibility asset's source.
macro_rules! asset_source {
	($name:literal) => {
		crate::rendering::shader_vm_test::asset_source(concat!("visibility/", $name))
	};
}

/// Compiles one checked-in visibility asset for VM execution.
macro_rules! asset {
	($name:literal) => {
		compile(asset_program(asset_source!($name)))
	};
}

/// Runs one workgroup with one lane per entry of `configs` and fresh workgroup state.
///
/// Fixtures bind their resources to `descriptors` and hand them over, so their borrows end with the run.
fn run_workgroup(program: &ExecutableProgram, descriptors: DescriptorBindings<'_>, configs: &[ExecutionConfig]) {
	let mut workgroup = WorkgroupState::new();
	// Rebinding shortens the bindings' lifetime to the local workgroup state's.
	let mut descriptors = descriptors;
	descriptors.bind_workgroup_state(&mut workgroup);
	program.run_workgroup(&mut descriptors, configs).expect(
		"Failed to run a visibility workgroup in the BESL VM. The most likely cause is a shader regression or a missing fixture binding.",
	);
}

/// Runs the whole workgroup of `N` lanes, `width` lanes wide, that holds `pixel`.
///
/// Shaders that share a tile across their workgroup need every lane to compute any one pixel.
fn run_workgroup_containing<const N: usize>(
	program: &ExecutableProgram,
	descriptors: DescriptorBindings<'_>,
	width: u32,
	pixel: [u32; 2],
) {
	let base = [pixel[0] - pixel[0] % width, pixel[1] - pixel[1] % (N as u32 / width)];
	let configs: [ExecutionConfig; N] = std::array::from_fn(|lane| {
		let lane = lane as u32;
		lane_config(lane).with_thread_id([base[0] + lane % width, base[1] + lane / width])
	});
	run_workgroup(program, descriptors, &configs);
}

fn read_u32(buffer: &besl::vm::Buffer, index: usize) -> u32 {
	match buffer.read_array_element(index).expect("VM u32 array element") {
		Value::U32(value) => value,
		value => panic!("Unexpected visibility buffer value: {value:?}."),
	}
}

fn read_vec3u(buffer: &besl::vm::Buffer, index: usize) -> [u32; 3] {
	match buffer.read_array_element(index).expect("VM vec3u array element") {
		Value::Vec3U(value) => value,
		value => panic!("Unexpected visibility dispatch value: {value:?}."),
	}
}

fn read_vec4f(texture: &Texture, coord: [u32; 2]) -> [f32; 4] {
	match texture.fetch(coord).expect("VM texel") {
		Value::Vec4F(value) => value,
		value => panic!("Unexpected texel value: {value:?}."),
	}
}

fn read_vec2u16(buffer: &besl::vm::Buffer, index: usize) -> [u16; 2] {
	match buffer.read_array_element(index).expect("VM vec2u16 array element") {
		Value::Vec2U16(value) => value,
		value => panic!("Unexpected visibility pixel mapping value: {value:?}."),
	}
}

/// Verifies the visibility fragment preserves the mesh-stage identifiers consumed by later compute passes.
#[test]
fn visibility_fragment_main_forwards_primitive_and_instance_identifiers() {
	let program = asset!("visibility-fragment.besl");
	let mut instance_input = input_buffer(&program, 0);
	let mut primitive_input = input_buffer(&program, 1);
	let mut primitive_output = output_buffer(&program, 0);
	let mut instance_output = output_buffer(&program, 1);
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

/// A column-major affine identity matrix in the BESL VM representation.
const IDENTITY_AFFINE_MATRIX: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// Returns a view-projection matrix that moves identity geometry outside the horizontal clip range.
const fn horizontally_translated_matrix(translation: f32) -> [f32; 16] {
	let mut matrix = IDENTITY_MATRIX;
	matrix[12] = translation;
	matrix
}

/// The `TaskMeshFixture` struct selects the instance and normal-cone inputs a task-culling test exercises.
#[derive(Clone, Copy)]
struct TaskMeshFixture {
	/// The instance's `ShaderMesh` flags, such as [`MESH_FLAG_SKINNED`].
	flags: u32,
	/// Gives every meshlet a narrow normal cone facing straight away from the camera at the origin. Otherwise the
	/// cone test is disabled, so the fixture isolates frustum and skinning behavior.
	back_facing: bool,
	/// The sphere of the instance's level, which task shaders test before any meshlet and decode the meshlets' spheres
	/// within. The fixture's model is the identity.
	bounding_sphere: [f32; 4],
	/// The radius the level casts shadows with, when it differs from its sphere's: one copy's, for a merged mesh.
	caster_radius: Option<f32>,
	/// Every meshlet's vertex and triangle counts. The default fills a mesh grid, so each meshlet draws alone.
	counts: [u8; 2],
	/// The meshlet each lane's own work item names, counted from the level's first, for a workgroup whose lanes
	/// have items of their own. `None` runs a workgroup with one item: the instance's first chunk of meshlets.
	lane_meshlets: Option<&'static [u32]>,
}

impl Default for TaskMeshFixture {
	fn default() -> Self {
		Self {
			flags: 0,
			back_facing: false,
			// Wide enough to reach every fixture meshlet, so the instance test passes unless a test moves it.
			bounding_sphere: [0.0, 0.0, 0.5, 8.0],
			caster_radius: None,
			counts: [64, 126],
			lane_meshlets: None,
		}
	}
}

/// Executes one exact production task workgroup at its global dispatch position over consecutive meshlets, culling
/// against `view_count` views from `view_base`.
///
/// View `i` draws with `view_projections[i]`, and only instances at least `minimum_caster_extent` normalized device
/// units across. With `occlusion`, the workgroup also culls by occlusion as the camera's passes do, and view zero is
/// [`fixture_camera`] at an aspect ratio of two.
fn run_meshlet_task_workgroup(
	view_projections: &[[f32; 16]],
	[view_base, view_count]: [u32; 2],
	minimum_caster_extent: f32,
	center_radii: &[[f32; 4]],
	mesh: TaskMeshFixture,
	workgroup_index: u32,
	occlusion: Option<CameraOcclusion<'_>>,
) -> TaskOutputs {
	let program = asset!("meshlet-task.besl");
	let meshlet_count = center_radii.len() as u32;
	assert!(
		(1..=TASK_WORKGROUP_SIZE).contains(&meshlet_count),
		"Task meshlet fixture must hold between one meshlet and one workgroup of meshlets."
	);
	let mut views = buffer(&program, slot(&program, VIEWS));
	for (view_index, view_projection) in view_projections.iter().copied().enumerate() {
		views
			.write_array_member(view_index, "view_projection", Value::Mat4F(view_projection))
			.expect("task view");
		views
			.write_array_member(view_index, "inverse_view", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX))
			.expect("task inverse view");
	}
	if occlusion.is_some() {
		let camera = fixture_camera(2.0);
		for (member, value) in [
			("view_projection", Value::Mat4F(column_major(camera.view_projection()))),
			("view", Value::Mat4x3F(bytemuck::cast(ghi::pod::Mat4x3f::from(camera.view())))),
			("near", Value::F32(camera.near())),
			("far", Value::F32(camera.far())),
		] {
			views.write_array_member(0, member, value).expect("task occlusion camera");
		}
	}
	let mut meshes = array_buffer(&program, slot(&program, MESH_DATA), FIXTURE_TABLE_LENGTH);
	write_fixture_mesh(&mut meshes, mesh.flags, [0; 2]);
	let mut mesh_levels = array_buffer(&program, slot(&program, MESH_LEVELS), FIXTURE_TABLE_LENGTH);
	write_fixture_level_caster(
		&mut mesh_levels,
		mesh.bounding_sphere,
		mesh.caster_radius.unwrap_or(mesh.bounding_sphere[3]),
		meshlet_count,
		0,
	);
	let mut meshlets = array_buffer(&program, slot(&program, MESHLETS), FIXTURE_MESHLET_INDEX + center_radii.len());
	for (meshlet_offset, center_radius) in center_radii.iter().copied().enumerate() {
		// A cutoff past one disables cone rejection. The back-facing cone points along +Z, the direction from the
		// camera at the origin to the meshlet.
		let cone_cutoff = if mesh.back_facing { 0.5 } else { 2.0 };
		let meshlet = ShaderMeshletData::new(
			0,
			0,
			mesh.counts,
			center_radius,
			mesh.bounding_sphere,
			[0.0, 0.0, 1.0],
			cone_cutoff,
		);
		write_meshlet(&mut meshlets, FIXTURE_MESHLET_INDEX + meshlet_offset, meshlet);
	}
	let mut push_constant = push_constant_buffer(&program);
	for (member, value) in [
		("work_item_base", 0),
		("view_base", view_base),
		("layer_base", 0),
		("view_count", view_count),
		(
			"occlusion_phase",
			occlusion
				.as_ref()
				.map_or(OcclusionPhase::Disabled, |occlusion| occlusion.phase) as u32,
		),
		("shared_lanes", u32::from(mesh.lane_meshlets.is_some())),
	] {
		push_constant.write(member, Value::U32(value)).expect("task push constant");
	}
	push_constant
		.write("minimum_caster_extent", Value::F32(minimum_caster_extent))
		.expect("task minimum caster extent");
	let mut mesh_dispatch_work = array_buffer(&program, slot(&program, MESH_DISPATCH_WORK), FIXTURE_TABLE_LENGTH);
	match mesh.lane_meshlets {
		None => {
			let packed_work = MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed;
			mesh_dispatch_work
				.write_array_element(workgroup_index as usize, Value::U32(packed_work))
				.expect("compact mesh dispatch work");
		}
		// Each lane's item, then empty items for the lanes without one.
		Some(lane_meshlets) => {
			for lane in 0..TASK_WORKGROUP_SIZE as usize {
				let item = lane_meshlets.get(lane).map_or(MeshDispatchWorkItem::EMPTY, |&meshlet| {
					MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, meshlet)
				});
				mesh_dispatch_work
					.write_array_element(
						(workgroup_index * TASK_WORKGROUP_SIZE) as usize + lane,
						Value::U32(item.packed),
					)
					.expect("shared mesh dispatch work");
			}
		}
	}
	// The record is keyed by the work item's position in the work buffer.
	let mut occlusion_visibility = array_buffer(&program, slot(&program, OCCLUSION_VISIBILITY), FIXTURE_TABLE_LENGTH);
	if let Some(occlusion) = &occlusion {
		occlusion_visibility
			.write_array_element(workgroup_index as usize, Value::U32(*occlusion.record))
			.expect("task occlusion record");
	}

	let mut task_outputs = TaskOutputs::new();
	let configs = (0..TASK_WORKGROUP_SIZE)
		.map(|lane| lane_config(lane).with_thread_position(workgroup_index * TASK_WORKGROUP_SIZE + lane))
		.collect::<Vec<_>>();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, VIEWS), &mut views);
	descriptors.bind_buffer(slot(&program, MESH_DATA), &mut meshes);
	descriptors.bind_buffer(slot(&program, MESH_LEVELS), &mut mesh_levels);
	descriptors.bind_buffer(slot(&program, MESHLETS), &mut meshlets);
	descriptors.bind_buffer(slot(&program, MESH_DISPATCH_WORK), &mut mesh_dispatch_work);
	let record = occlusion.map(|occlusion| {
		descriptors.bind_texture(slot(&program, OCCLUSION_PYRAMID), occlusion.pyramid);
		descriptors.bind_buffer(slot(&program, OCCLUSION_VISIBILITY), &mut occlusion_visibility);
		occlusion.record
	});
	descriptors.bind_push_constant(&mut push_constant);
	descriptors.bind_task_outputs(&mut task_outputs);
	run_workgroup(&program, descriptors, &configs);
	if let Some(record) = record {
		*record = read_u32(&occlusion_visibility, workgroup_index as usize);
	}
	task_outputs
}

/// Writes the fixture instance's record: the identity model, drawing the fixture level with `flags`, material zero, and
/// the `surface` words.
fn write_fixture_mesh(meshes: &mut Buffer, flags: u32, surface: [u32; 2]) {
	for (member, value) in [
		("model", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX)),
		("level", Value::U32(FIXTURE_LEVEL_INDEX as u32 | flags)),
		("material", Value::U32(0)),
		("surface", Value::Vec2U(surface)),
	] {
		meshes
			.write_array_member(FIXTURE_INSTANCE_INDEX, member, value)
			.expect("fixture mesh field");
	}
}

/// Writes the fixture level's record: `meshlet_count` meshlets from the fixture meshlet, inside `bounding_sphere`, over
/// vertices from `vertex_base`.
fn write_fixture_level(mesh_levels: &mut Buffer, bounding_sphere: [f32; 4], meshlet_count: u32, vertex_base: u32) {
	write_fixture_level_caster(mesh_levels, bounding_sphere, bounding_sphere[3], meshlet_count, vertex_base);
}

/// Writes the fixture level's record as [`write_fixture_level`] does, casting shadows with `caster_radius` where one
/// object casts with its whole sphere.
fn write_fixture_level_caster(
	mesh_levels: &mut Buffer,
	bounding_sphere: [f32; 4],
	caster_radius: f32,
	meshlet_count: u32,
	vertex_base: u32,
) {
	for (member, value) in [
		("bounding_sphere", Value::Vec4F(bounding_sphere)),
		("caster_radius", Value::F32(caster_radius)),
		("first_meshlet", Value::U32(FIXTURE_MESHLET_INDEX as u32)),
		("meshlet_count", Value::U32(meshlet_count)),
		("vertex_base", Value::U32(vertex_base)),
	] {
		mesh_levels
			.write_array_member(FIXTURE_LEVEL_INDEX, member, value)
			.expect("fixture level field");
	}
}

/// Writes `meshlet` at `index` of a meshlet buffer.
fn write_meshlet(meshlets: &mut Buffer, index: usize, meshlet: ShaderMeshletData) {
	for (member, value) in [
		("primitive_offset", Value::U32(meshlet.primitive_offset)),
		("triangle_offset", Value::U32(meshlet.triangle_offset)),
		("counts", Value::U32(meshlet.counts)),
		("cone", Value::U32(meshlet.cone)),
		("bounds", Value::Vec2U(meshlet.bounds)),
	] {
		meshlets
			.write_array_member(index, member, value)
			.expect("fixture meshlet field");
	}
}

/// Runs one task workgroup against the camera, view zero, and returns every payload word it emitted.
fn camera_task_payload(center_radii: &[[f32; 4]], mesh: TaskMeshFixture, workgroup_index: u32) -> Vec<Value> {
	task_payload(&run_meshlet_task_workgroup(
		&[IDENTITY_MATRIX],
		[0, 1],
		0.0,
		center_radii,
		mesh,
		workgroup_index,
		None,
	))
}

/// Reads the groups a task workgroup handed its mesh grids, in grid order: each group's entries as the lane that kept
/// the meshlet and the view's offset into the batch.
fn task_groups(outputs: &TaskOutputs) -> Vec<Vec<[u32; 2]>> {
	let word = |index: usize| match outputs.payload_value("meshlet_instances", index) {
		Some(Value::U32(word)) => *word,
		word => panic!("Unexpected task payload word: {word:?}."),
	};
	(0..outputs.mesh_output_count().expect("task mesh output count") as usize)
		.map(|group| {
			let first = (word(group) >> 22 & 255) as usize;
			let last = (first..)
				.find(|&index| word(index) & ENTRY_ENDS_GROUP != 0)
				.expect("a group ends");
			(first..=last).map(|index| [word(index) & 31, word(index) >> 5 & 7]).collect()
		})
		.collect()
}

/// Reads every meshlet a task workgroup emitted, in emission order, each as [`batched_meshlet_instance`] writes it.
fn task_payload(outputs: &TaskOutputs) -> Vec<Value> {
	task_groups(outputs)
		.into_iter()
		.flatten()
		.map(|[lane, view_offset]| batched_meshlet_instance(lane, view_offset))
		.collect()
}

/// What [`task_payload`] reads for a fixture meshlet seen by the view `view_offset` steps into its batch.
///
/// `relative_meshlet_index` counts from the first meshlet of the instance's level, which is the lane that culls it.
fn batched_meshlet_instance(relative_meshlet_index: u32, view_offset: u32) -> Value {
	Value::U32(relative_meshlet_index | (view_offset << VIEW_OFFSET_SHIFT))
}

/// Returns the payload entry of `lane`'s meshlet in the view `view_offset` steps into its batch, as the last entry of
/// a group that holds `vertices` vertices and `triangles` triangles up to and including it.
fn group_entry(lane: u32, view_offset: u32, [vertices, triangles]: [u32; 2], ends_group: bool) -> Value {
	let ends = if ends_group { ENTRY_ENDS_GROUP } else { 0 };
	Value::U32(lane | (view_offset << 5) | ((vertices - 1) << 8) | ((triangles - 1) << 14) | ends)
}

/// Returns the payload a task workgroup hands its mesh grids: `entries`, which form group zero, and `work`.
fn mesh_task_payload(entries: &[Value], work: u32) -> Vec<Value> {
	let mut payload = vec![Value::U32(0); PAYLOAD_WORK_WORD + 1];
	payload[..entries.len()].clone_from_slice(entries);
	payload[PAYLOAD_WORK_WORD] = Value::U32(work);
	payload
}

/// Verifies the task stage hands every mesh grid its workgroup's first work item in the payload's last word.
#[test]
fn task_main_hands_its_work_to_the_mesh_stage() {
	let work = |mesh| {
		let outputs = run_meshlet_task_workgroup(&[IDENTITY_MATRIX], [0, 1], 0.0, &[[0.0, 0.0, 0.5, 0.1]], mesh, 1, None);
		outputs.payload_value("meshlet_instances", PAYLOAD_WORK_WORD).cloned()
	};

	assert_eq!(work(TaskMeshFixture::default()), Some(Value::U32(1)));
	// Lanes with items of their own take 32 of them to a workgroup.
	let shared = TaskMeshFixture {
		lane_meshlets: Some(&[0]),
		..Default::default()
	};
	assert_eq!(work(shared), Some(Value::U32(TASK_WORKGROUP_SIZE | PAYLOAD_WORK_SHARED)));
}

/// Verifies small meshlets share a mesh grid while its vertices and triangles hold them, and start another when not.
#[test]
fn task_main_groups_small_meshlets_into_full_mesh_grids() {
	let groups = |counts| {
		let mesh = TaskMeshFixture {
			counts,
			..Default::default()
		};
		task_groups(&run_meshlet_task_workgroup(
			&[IDENTITY_MATRIX],
			[0, 1],
			0.0,
			&[[0.0, 0.0, 0.5, 0.1]; 5],
			mesh,
			0,
			None,
		))
	};
	let lanes = |groups: Vec<Vec<[u32; 2]>>| -> Vec<Vec<u32>> {
		groups
			.into_iter()
			.map(|group| group.into_iter().map(|[lane, _]| lane).collect())
			.collect()
	};

	// Three meshlets of 20 vertices fit the 64 a grid emits, and a fourth doesn't.
	assert_eq!(lanes(groups([20, 10])), [vec![0, 1, 2], vec![3, 4]]);
	// Triangles bound a group as well: two meshlets of 50 fit the 126 a grid emits.
	assert_eq!(lanes(groups([8, 50])), [vec![0, 1], vec![2, 3], vec![4]]);
	// A meshlet that fills a grid draws alone.
	assert_eq!(lanes(groups([64, 126])), [[0], [1], [2], [3], [4]]);
}

/// Verifies lanes with work items of their own cull the meshlets their items name, and lanes without one emit nothing.
#[test]
fn task_main_culls_the_meshlet_each_shared_lane_names() {
	let mesh = TaskMeshFixture {
		counts: [20, 10],
		// The second lane's meshlet lies outside the frustum.
		lane_meshlets: Some(&[2, 1, 0]),
		..Default::default()
	};
	let outputs = run_meshlet_task_workgroup(
		&[IDENTITY_MATRIX],
		[0, 1],
		0.0,
		&[[0.0, 0.0, 0.5, 0.1], [4.0, 0.0, 0.5, 0.1], [0.5, 0.0, 0.5, 0.1]],
		mesh,
		0,
		None,
	);

	assert_eq!(task_groups(&outputs), [[[0, 0], [2, 0]]]);
}

/// Verifies camera culling retains an intersecting meshlet and rejects one outside the frustum.
#[test]
fn task_main_emits_in_frustum_and_culls_off_frustum_meshlets() {
	let payload = |center_radius| camera_task_payload(&[center_radius], TaskMeshFixture::default(), 0);

	assert_eq!(payload([0.0, 0.0, 0.5, 0.1]), [batched_meshlet_instance(0, 0)]);
	assert_eq!(payload([4.0, 0.0, 0.5, 0.1]), []);
}

/// Verifies the task emits nothing for an instance whose bounds no view reaches, even where its meshlet bounds would
/// pass on their own.
#[test]
fn task_main_culls_instances_outside_every_view() {
	let outside = TaskMeshFixture {
		bounding_sphere: [4.0, 0.0, 0.5, 0.1],
		..Default::default()
	};
	let output = run_meshlet_task_workgroup(&[IDENTITY_MATRIX; 2], [0, 2], 0.0, &[[0.0, 0.0, 0.5, 0.1]], outside, 0, None);
	assert_eq!(task_payload(&output), []);
}

/// Verifies workgroup barriers and atomics compact visible meshlets in lane order before publishing the final count.
#[test]
fn task_workgroup_compacts_mixed_meshlets_in_lane_order() {
	let payload = camera_task_payload(
		&[[0.0, 0.0, 0.5, 0.1], [4.0, 0.0, 0.5, 0.1], [0.5, 0.0, 0.5, 0.1]],
		TaskMeshFixture::default(),
		0,
	);

	assert_eq!(payload, [batched_meshlet_instance(0, 0), batched_meshlet_instance(2, 0)]);
}

/// Verifies culling reads the work item selected by the global dispatch position.
#[test]
fn task_main_selects_later_batched_workgroup() {
	let payload = camera_task_payload(&[[0.0, 0.0, 0.5, 0.1]], TaskMeshFixture::default(), 1);
	assert_eq!(payload, [batched_meshlet_instance(0, 0)]);
}

/// Verifies posed geometry reaches every view of the batch, whatever its bind-pose bounds.
#[test]
fn task_main_keeps_skinned_meshlets_in_every_view() {
	let skinned = TaskMeshFixture {
		flags: MESH_FLAG_SKINNED,
		bounding_sphere: [4.0, 0.0, 0.5, 0.1],
		..Default::default()
	};
	let output = run_meshlet_task_workgroup(&[IDENTITY_MATRIX; 2], [0, 2], 0.0, &[[4.0, 0.0, 0.5, 0.1]], skinned, 0, None);

	assert_eq!(
		task_payload(&output),
		[batched_meshlet_instance(0, 0), batched_meshlet_instance(0, 1)]
	);
}

/// Verifies an orthographic view skips an instance narrower than the minimum caster extent and keeps one at least that
/// wide, while a skinned instance reaches the view whatever its size.
#[test]
fn task_main_skips_casters_narrower_than_the_minimum_extent() {
	// The identity view-projection is orthographic at one normalized device unit per meter, so a sphere of radius r is
	// 2r across.
	let payload = |radius: f32, flags| {
		let mesh = TaskMeshFixture {
			flags,
			bounding_sphere: [0.0, 0.0, 0.5, radius],
			..Default::default()
		};
		let center_radius = [0.0, 0.0, 0.5, radius];
		task_payload(&run_meshlet_task_workgroup(
			&[IDENTITY_MATRIX],
			[0, 1],
			0.5,
			&[center_radius],
			mesh,
			0,
			None,
		))
	};

	assert_eq!(payload(0.2, 0), []);
	assert_eq!(payload(0.25, 0), [batched_meshlet_instance(0, 0)]);
	assert_eq!(payload(0.2, MESH_FLAG_SKINNED), [batched_meshlet_instance(0, 0)]);
}

/// Verifies an orthographic view measures a merged level by one of its copies: a level whose sphere is wide, because it
/// spans many copies, is skipped when a copy is narrower than the minimum caster extent.
#[test]
fn task_main_skips_a_wide_level_whose_copies_are_narrower_than_the_minimum_extent() {
	let payload = |caster_radius: f32| {
		let mesh = TaskMeshFixture {
			// Two units across, well past the half unit the view asks for.
			bounding_sphere: [0.0, 0.0, 0.5, 1.0],
			caster_radius: Some(caster_radius),
			..Default::default()
		};
		task_payload(&run_meshlet_task_workgroup(
			&[IDENTITY_MATRIX],
			[0, 1],
			0.5,
			&[[0.0, 0.0, 0.5, 0.1]],
			mesh,
			0,
			None,
		))
	};

	assert_eq!(payload(0.2), []);
	assert_eq!(payload(0.25), [batched_meshlet_instance(0, 0)]);
}

/// Verifies the task rejects a meshlet facing away from the view unless its material is double-sided.
#[test]
fn task_main_keeps_back_facing_meshlets_only_for_double_sided_meshes() {
	let visible_meshlets = |flags| {
		let mesh = TaskMeshFixture {
			flags,
			back_facing: true,
			..Default::default()
		};
		camera_task_payload(&[[0.0, 0.0, 0.5, 0.1]], mesh, 0).len()
	};

	assert_eq!(visible_meshlets(0), 0);
	assert_eq!(visible_meshlets(MESH_FLAG_DOUBLE_SIDED), 1);
}

/// Verifies a task culls each meshlet against every view of its batch and tags each emitted copy with the view that
/// kept it.
#[test]
fn task_main_emits_one_meshlet_copy_per_view_that_sees_it() {
	let mut view_projections = [horizontally_translated_matrix(4.0); 8];
	view_projections[3] = IDENTITY_MATRIX;
	view_projections[5] = IDENTITY_MATRIX;
	// Views 2 through 5: only the second and fourth see the meshlet.
	let output = run_meshlet_task_workgroup(
		&view_projections,
		[2, 4],
		0.0,
		&[[0.0, 0.0, 0.5, 0.1]],
		TaskMeshFixture::default(),
		0,
		None,
	);

	assert_eq!(
		task_payload(&output),
		[batched_meshlet_instance(0, 1), batched_meshlet_instance(0, 3)]
	);
}

/* Occlusion culling */

/// The camera of the occlusion and light-cluster fixtures: at the origin, looking down +Z, with a 90 degree field of
/// view and a 0.1 m to 100 m clip range.
fn fixture_camera(aspect_ratio: f32) -> crate::rendering::View {
	crate::rendering::View::new_perspective(
		math::Degrees::new(90.0),
		aspect_ratio,
		0.1,
		100.0,
		math::Point::origin(),
		math::UnitVector::z_axis(),
	)
}

/// Returns the reversed depth the occlusion fixture camera stores for a surface `z` units in front of it.
fn occlusion_fixture_depth(z: f32) -> f32 {
	let projection = fixture_camera(2.0).projection();
	let clip = projection * maths_rs::Vec4f::new(0.0, 0.0, z, 1.0);
	clip.z / clip.w
}

/// Builds a full production-extent occlusion pyramid whose mip zero holds `depth(x, y)`. Each later level keeps the
/// farthest of four texels, as the production build does.
fn occlusion_pyramid(depth: impl Fn(u32, u32) -> f32) -> Texture {
	let mut level: Vec<f32> = (0..OCCLUSION_PYRAMID_WIDTH * OCCLUSION_PYRAMID_HEIGHT)
		.map(|index| depth(index % OCCLUSION_PYRAMID_WIDTH, index / OCCLUSION_PYRAMID_WIDTH))
		.collect();
	let texels = |level: &[f32]| level.iter().map(|&depth| [depth, 0.0, 0.0, 1.0]).collect::<Vec<_>>();
	let mut pyramid = texture_2d(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT, &texels(&level));
	for mip in 1..OCCLUSION_PYRAMID_MIP_COUNT {
		let source_width = OCCLUSION_PYRAMID_WIDTH >> (mip - 1);
		let [width, height] = [OCCLUSION_PYRAMID_WIDTH >> mip, OCCLUSION_PYRAMID_HEIGHT >> mip];
		level = (0..width * height)
			.map(|index| {
				let [x, y] = [index % width * 2, index / width * 2];
				let at = |x: u32, y: u32| level[(y * source_width + x) as usize];
				at(x, y).min(at(x + 1, y)).min(at(x, y + 1)).min(at(x + 1, y + 1))
			})
			.collect();
		pyramid.add_mip(texture_2d(width, height, &texels(&level)));
	}
	pyramid
}

/// The `CameraOcclusion` struct selects how a task-shader fixture culls by occlusion, as one of the camera's passes.
struct CameraOcclusion<'a> {
	phase: OcclusionPhase,
	pyramid: &'a mut Texture,
	/// The fixture work item's record of unoccluded meshlets. The late phase overwrites it.
	record: &'a mut u32,
}

/// Runs the camera's task workgroup over `center_radii`, culling by occlusion in `phase`, and returns the relative
/// indices of the meshlets it emitted.
///
/// The instance bounds wrap every meshlet.
fn camera_occlusion_meshlets(
	center_radii: &[[f32; 4]],
	mesh: TaskMeshFixture,
	phase: OcclusionPhase,
	pyramid: &mut Texture,
	record: &mut u32,
) -> Vec<u32> {
	let mesh = TaskMeshFixture {
		bounding_sphere: [0.0, 0.0, 10.0, 25.0],
		..mesh
	};
	let occlusion = CameraOcclusion { phase, pyramid, record };
	task_payload(&run_meshlet_task_workgroup(
		&[],
		[0, 1],
		0.0,
		center_radii,
		mesh,
		0,
		Some(occlusion),
	))
	.into_iter()
	.map(|word| match word {
		Value::U32(word) => word & ((1 << MESHLET_INSTANCE_BITS) - 1),
		word => panic!("Unexpected task payload word: {word:?}."),
	})
	.collect()
}

/// Returns how many of one meshlet's copies a pass that only tests occlusion keeps.
fn occlusion_tested_meshlets(center_radius: [f32; 4], mesh: TaskMeshFixture, pyramid: &mut Texture) -> usize {
	camera_occlusion_meshlets(&[center_radius], mesh, OcclusionPhase::Test, pyramid, &mut 0).len()
}

/// A meshlet in front of the fixture wall, and one behind it.
const IN_FRONT_OF_WALL: [f32; 4] = [0.0, 0.0, 3.0, 0.25];
const BEHIND_WALL: [f32; 4] = [0.0, 0.0, 10.0, 0.5];

/// Returns a pyramid whose every texel holds a wall five units in front of the fixture camera.
fn wall_pyramid() -> Texture {
	occlusion_pyramid(|_, _| occlusion_fixture_depth(5.0))
}

/// Verifies a pass that tests occlusion drops geometry behind the pyramid's surfaces, and keeps geometry in front of
/// them.
#[test]
fn task_main_culls_meshlets_hidden_behind_the_occlusion_pyramid() {
	let mesh = TaskMeshFixture::default();
	let mut wall = wall_pyramid();

	assert_eq!(occlusion_tested_meshlets(BEHIND_WALL, mesh, &mut wall), 0);
	assert_eq!(occlusion_tested_meshlets(IN_FRONT_OF_WALL, mesh, &mut wall), 1);
}

/// Verifies one uncovered texel anywhere under a meshlet's screen footprint keeps it, so culling never removes geometry
/// that shows through a gap.
#[test]
fn task_main_keeps_meshlets_seen_through_a_gap_in_the_occlusion_pyramid() {
	// The meshlet covers about the center twentieth of the screen's width. Leave one texel inside it uncovered.
	let gap = [OCCLUSION_PYRAMID_WIDTH / 2 + 3, OCCLUSION_PYRAMID_HEIGHT / 2 - 5];
	let mut pyramid = occlusion_pyramid(|x, y| if [x, y] == gap { 0.0 } else { occlusion_fixture_depth(5.0) });

	assert_eq!(
		occlusion_tested_meshlets(BEHIND_WALL, TaskMeshFixture::default(), &mut pyramid),
		1
	);
}

/// Verifies geometry that reaches past the screen edge or the near plane is kept, because the pyramid holds nothing
/// there.
#[test]
fn task_main_keeps_meshlets_the_occlusion_pyramid_cannot_see_whole() {
	let mut wall = wall_pyramid();
	let mesh = TaskMeshFixture::default();
	// The fixture camera spans 90 degrees vertically at an aspect ratio of two, so x = 2z is the right screen edge.
	let at_screen_edge = [19.8, 0.0, 10.0, 0.5];
	let at_near_plane = [0.0, 0.0, 0.3, 0.25];

	assert_eq!(occlusion_tested_meshlets(at_screen_edge, mesh, &mut wall), 1);
	assert_eq!(occlusion_tested_meshlets(at_near_plane, mesh, &mut wall), 1);
}

/// Verifies posed geometry is never culled by occlusion, since its bind-pose bounds do not hold its pose.
#[test]
fn task_main_keeps_skinned_meshlets_behind_the_occlusion_pyramid() {
	let skinned = TaskMeshFixture {
		flags: MESH_FLAG_SKINNED,
		..Default::default()
	};

	assert_eq!(occlusion_tested_meshlets(BEHIND_WALL, skinned, &mut wall_pyramid()), 1);
}

/// Verifies the camera's early and late passes together draw every unoccluded meshlet exactly once, whatever the
/// previous frame recorded, and that the late pass records exactly the unoccluded meshlets.
#[test]
fn camera_passes_draw_each_unoccluded_meshlet_once_and_record_them() {
	// Meshlets zero and one stand in front of the wall, and meshlet two behind it.
	let meshlets = [IN_FRONT_OF_WALL, [0.5, 0.0, 3.0, 0.25], BEHIND_WALL];
	let mesh = TaskMeshFixture::default();
	let mut wall = wall_pyramid();

	for previous_record in 0..8u32 {
		let mut record = previous_record;
		let early = camera_occlusion_meshlets(&meshlets, mesh, OcclusionPhase::Early, &mut wall, &mut record);
		assert_eq!(
			record, previous_record,
			"The early pass must leave the record to the late pass."
		);
		let late = camera_occlusion_meshlets(&meshlets, mesh, OcclusionPhase::Late, &mut wall, &mut record);

		let expected_early: Vec<u32> = (0..3).filter(|meshlet| previous_record & (1 << meshlet) != 0).collect();
		assert_eq!(
			early, expected_early,
			"The early pass draws what the previous frame recorded."
		);
		let mut drawn = [early, late].concat();
		drawn.sort();
		assert!(
			drawn == [0, 1] || drawn == [0, 1, 2] && previous_record & 0b100 != 0,
			"Expected each unoccluded meshlet once, and the hidden one only if the early pass drew it from a stale record. previous_record={previous_record:#05b}, drawn={drawn:?}"
		);
		assert_eq!(record, 0b011);
	}
}

/// Verifies the late pass clears the record of an instance outside the view, so the next early pass skips it.
#[test]
fn late_pass_clears_the_record_of_instances_outside_the_view() {
	let outside = TaskMeshFixture {
		bounding_sphere: [100.0, 0.0, 10.0, 0.5],
		..Default::default()
	};
	let mut wall = wall_pyramid();
	let mut record = 0b1;
	let occlusion = CameraOcclusion {
		phase: OcclusionPhase::Late,
		pyramid: &mut wall,
		record: &mut record,
	};

	run_meshlet_task_workgroup(&[], [0, 1], 0.0, &[[100.0, 0.0, 10.0, 0.5]], outside, 0, Some(occlusion));

	assert_eq!(record, 0);
}

/// The occlusion pyramid base stage's 16x16 workgroup, and the tail stage's 16x8 one.
const HIZ_BASE_WORKGROUP_WIDTH: u32 = 16;
const HIZ_BASE_WORKGROUP_SIZE: usize = 256;
const HIZ_TAIL_WORKGROUP_SIZE: usize = 128;

/// Returns the 16-bit floats the occlusion pyramid's seed may store for `depth`: the nearest one at or below it, and,
/// when the nearest overall lies above, also the step below that, which the seed's scaling can land on.
fn seeded_depths(depth: f32) -> [f32; 2] {
	let nearest = half::f16::from_f32(depth);
	if nearest.to_f32() > depth {
		let below = half::f16::from_bits(nearest.to_bits() - 1);
		[below.to_f32(), half::f16::from_bits(below.to_bits() - 1).to_f32()]
	} else {
		[nearest.to_f32(); 2]
	}
}

/// Reduces a level of `width` by `height` texels to the farthest of each 2x2 block, as the production build does.
fn farthest_of_four(level: &[f32], width: u32, height: u32) -> Vec<f32> {
	(0..width / 2 * (height / 2))
		.map(|index| {
			let [x, y] = [index % (width / 2) * 2, index / (width / 2) * 2];
			let at = |x: u32, y: u32| level[(y * width + x) as usize];
			at(x, y).min(at(x + 1, y)).min(at(x, y + 1)).min(at(x + 1, y + 1))
		})
		.collect()
}

/// Reads a one-channel image back as a row-major vector.
fn channel(image: &Texture, width: u32, height: u32) -> Vec<f32> {
	(0..width * height)
		.map(|index| rgba(image, [index % width, index / width])[0])
		.collect()
}

/// Runs the occlusion pyramid base stage's one 16x16 block over a depth buffer of `width` by `height` pixels holding
/// `depth(x, y)`, and returns mips zero through four of that block.
fn run_hiz_base(width: u32, height: u32, depth: impl Fn(u32, u32) -> f32) -> [Vec<f32>; 5] {
	let program = asset!("hiz-base.besl");
	let texels = (0..width * height)
		.map(|index| [depth(index % width, index / width), 0.0, 0.0, 1.0])
		.collect::<Vec<_>>();
	let mut source = texture_2d(width, height, &texels);
	let mut levels = [16, 8, 4, 2, 1].map(|size| empty_image(size, size));

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(slot(&program, "source_depth"), &mut source);
	for (index, level) in levels.iter_mut().enumerate() {
		descriptors.bind_image(slot(&program, &format!("level_{index}")), level);
	}
	run_workgroup_containing::<HIZ_BASE_WORKGROUP_SIZE>(&program, descriptors, HIZ_BASE_WORKGROUP_WIDTH, [0, 0]);
	std::array::from_fn(|level| channel(&levels[level], 16 >> level, 16 >> level))
}

/// Verifies each seeded texel keeps the farthest depth of every pixel its footprint touches, including pixels it only
/// partly covers, stored as the 16-bit float at or just below it, and that each level above holds the farthest of
/// four texels of the one below.
#[test]
fn occlusion_pyramid_base_seeds_the_farthest_depth_and_reduces_its_block() {
	// Forty pixels map onto sixteen texels, so every other texel straddles a pixel with its neighbor.
	let depth = |x: u32, y: u32| (((x * 7 + y * 13) % 29) as f32 + 1.0) / 31.0;
	let [level_0, level_1, level_2, level_3, level_4] = run_hiz_base(40, 16, depth);

	for (index, &stored) in level_0.iter().enumerate() {
		let [x, y] = [index as u32 % 16, index as u32 / 16];
		let farthest = (x * 40 / 16..((x + 1) * 40).div_ceil(16))
			.map(|pixel| depth(pixel, y))
			.fold(1.0f32, f32::min);
		assert!(
			seeded_depths(farthest).contains(&stored),
			"Texel ({x}, {y}) holds {stored} for a farthest depth of {farthest}. The most likely cause is a wrong footprint or a rounding toward the camera."
		);
	}
	assert_eq!(level_1, farthest_of_four(&level_0, 16, 16));
	assert_eq!(level_2, farthest_of_four(&level_1, 8, 8));
	assert_eq!(level_3, farthest_of_four(&level_2, 4, 4));
	assert_eq!(level_4, farthest_of_four(&level_3, 2, 2));
}

/// Verifies the seed stores a depth that is a 16-bit float, including zero, exactly, and never rounds one toward the
/// camera, which would let the pyramid cull a surface in front of what was drawn.
#[test]
fn occlusion_pyramid_seed_never_rounds_toward_the_camera() {
	let depths = [0.0, 0.5, 0.999_511_7, 0.1, 0.7, 0.123_456_79, 0.000_061_035_156, 0.3];
	let [level_0, ..] = run_hiz_base(16, 16, |x, y| if y == 0 && x < 8 { depths[x as usize] } else { 1.0 });

	for (&depth, &stored) in depths.iter().zip(&level_0) {
		assert!(
			stored <= depth,
			"Depth {depth} was stored as {stored}, nearer than drawn. The most likely cause is that the seed rounded to the nearest 16-bit float."
		);
		assert_eq!(
			half::f16::from_f32(stored).to_f32(),
			stored,
			"Depth {depth} was stored as {stored}, which is not a 16-bit float, so the image would round it again."
		);
		// At most two steps of the 16-bit significand below the nearest 16-bit float.
		let nearest = half::f16::from_f32(depth);
		let two_below = half::f16::from_bits(nearest.to_bits().saturating_sub(2)).to_f32();
		assert!(
			stored >= two_below,
			"Depth {depth} was stored as {stored}, further than two 16-bit steps below it. The most likely cause is an over-eager rounding step."
		);
	}
}

/// Verifies the tail stage takes a 32x16 mip four to the 2x1 top, each level the farthest of four texels below it.
#[test]
fn occlusion_pyramid_tail_reduces_mip_four_to_the_top() {
	let program = asset!("hiz-tail.besl");
	let level_4: Vec<f32> = (0..32 * 16).map(|index| ((index * 11 % 37) as f32 + 1.0) / 40.0).collect();
	let texels = level_4.iter().map(|&depth| [depth, 0.0, 0.0, 1.0]).collect::<Vec<_>>();
	let mut source = texture_2d(32, 16, &texels);
	let mut levels = [[16, 8], [8, 4], [4, 2], [2, 1]].map(|[width, height]| empty_image(width, height));

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_image(slot(&program, "source_level"), &mut source);
	for (index, level) in levels.iter_mut().enumerate() {
		descriptors.bind_image(slot(&program, &format!("level_{}", index + 5)), level);
	}
	run_workgroup_containing::<HIZ_TAIL_WORKGROUP_SIZE>(&program, descriptors, HIZ_BASE_WORKGROUP_WIDTH, [0, 0]);

	let mut expected = level_4;
	for ([width, height], level) in [[32, 16], [16, 8], [8, 4], [4, 2]].into_iter().zip(&levels) {
		expected = farthest_of_four(&expected, width, height);
		assert_eq!(
			channel(level, width / 2, height / 2),
			expected,
			"Level {width}x{height} reduced wrongly."
		);
	}
}

/// The `MeshView` struct selects the view a mesh-shader test draws: the batch its push constants name and the view's
/// offset into it, which the task payload carries.
#[derive(Clone, Copy)]
struct MeshView {
	view_base: u32,
	layer_base: u32,
	view_offset: u32,
	view_projection: [f32; 16],
}

impl MeshView {
	/// The camera's view zero, drawn as a batch of one.
	const CAMERA: Self = Self {
		view_base: 0,
		layer_base: 0,
		view_offset: 0,
		view_projection: IDENTITY_MATRIX,
	};

	/// The third view of a shadow batch that starts at view five and layer one, shifted right by two.
	const SHADOW: Self = Self {
		view_base: 5,
		layer_base: 1,
		view_offset: 2,
		view_projection: horizontally_translated_matrix(2.0),
	};
}

/// The `MeshAttributes` enum names the attributes a mesh stage passes to its fragment stage, which a mesh-shader test
/// checks as part of the stage's output contract.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MeshAttributes {
	/// Nothing, for a stage no fragment stage follows, as for opaque shadow casters.
	None,
	/// The instance at location 0 and the UV at location 1, which the masked shadow passes' alpha test reads.
	Instance,
	/// The instance and the primitive at locations 0 and 1, which the visibility pass writes.
	InstanceAndPrimitive,
	/// The instance, the primitive, and the UV at locations 0, 1, and 2, which the masked visibility pass writes.
	InstancePrimitiveAndUv,
}

impl MeshAttributes {
	/// Returns whether the stage reads UVs to pass on.
	fn has_uv(self) -> bool {
		matches!(self, Self::Instance | Self::InstancePrimitiveAndUv)
	}

	/// Returns whether the stage marks each triangle with its meshlet and place in it.
	fn has_primitive(self) -> bool {
		matches!(self, Self::InstanceAndPrimitive | Self::InstancePrimitiveAndUv)
	}
}

/// Executes one production mesh main over one triangle meshlet placed by `model` and verifies its complete output
/// contract, including that the triangle winds on screen as it does under an identity model.
fn assert_triangle_mesh_program(
	program: ExecutableProgram,
	view: MeshView,
	model: [f32; 12],
	base_vertex: u32,
	expected_clip_positions: [[f32; 4]; 3],
	expected_render_target_array_index: Option<u32>,
	attributes: MeshAttributes,
) {
	let mut views = buffer(&program, slot(&program, VIEWS));
	let mut meshes = array_buffer(&program, slot(&program, MESH_DATA), FIXTURE_TABLE_LENGTH);
	write_fixture_mesh(&mut meshes, 0, [0; 2]);
	meshes
		.write_array_member(FIXTURE_INSTANCE_INDEX, "model", Value::Mat4x3F(model))
		.expect("fixture mesh model");
	let mut mesh_levels = array_buffer(&program, slot(&program, MESH_LEVELS), FIXTURE_TABLE_LENGTH);
	write_fixture_level(&mut mesh_levels, [0.0, 0.0, 0.0, 2.0], 1, base_vertex);
	let vertex_count = base_vertex as usize + 3;
	let mut positions = array_buffer(&program, slot(&program, VERTEX_POSITIONS), vertex_count);
	for (index, position) in [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
		positions
			.write_array_element(base_vertex as usize + index, Value::Vec3F(position))
			.expect("mesh vertex");
	}
	// A stage that passes no attributes reads no UVs, so its program keeps no UV binding.
	let mut vertex_uvs = attributes
		.has_uv()
		.then(|| array_buffer(&program, slot(&program, VERTEX_UVS), vertex_count));
	let mut vertex_indices = array_buffer(&program, slot(&program, VERTEX_INDICES), 3);
	let mut primitive_indices = array_buffer(&program, slot(&program, PRIMITIVE_INDICES), 3);
	for index in 0..3 {
		if let Some(vertex_uvs) = &mut vertex_uvs {
			vertex_uvs
				.write_array_element(base_vertex as usize + index, Value::Vec2F16([besl::vm::f16::ZERO; 2]))
				.expect("mesh UV");
		}
		vertex_indices
			.write_array_element(index, Value::U16(index as u16))
			.expect("vertex index");
		primitive_indices
			.write_array_element(index, Value::U8(index as u8))
			.expect("triangle index");
	}
	let mut meshlets = array_buffer(&program, slot(&program, MESHLETS), FIXTURE_MESHLET_INDEX + 1);
	let meshlet = ShaderMeshletData::new(0, 0, [3, 1], [0.0; 4], [0.0, 0.0, 0.0, 2.0], [0.0, 0.0, 1.0], 2.0);
	write_meshlet(&mut meshlets, FIXTURE_MESHLET_INDEX, meshlet);
	let mut push_constant = push_constant_buffer(&program);
	push_constant.write("work_item_base", Value::U32(0)).expect("mesh work base");
	views
		.write_array_member(
			(view.view_base + view.view_offset) as usize,
			"view_projection",
			Value::Mat4F(view.view_projection),
		)
		.expect("selected mesh view");
	for (member, value) in [
		("view_base", view.view_base),
		("layer_base", view.layer_base),
		("view_count", view.view_offset + 1),
	] {
		push_constant.write(member, Value::U32(value)).expect("mesh view batch");
	}
	let payload = [group_entry(0, view.view_offset, [3, 1], true)];
	// The workgroup's one work item names the fixture instance's first chunk of meshlets.
	let mut mesh_dispatch_work = array_buffer(&program, slot(&program, MESH_DISPATCH_WORK), FIXTURE_TABLE_LENGTH);
	mesh_dispatch_work
		.write_array_element(
			0,
			Value::U32(MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed),
		)
		.expect("mesh dispatch work");

	// One buffer per attribute location the stage writes.
	let locations: u8 = match attributes {
		MeshAttributes::None => 0,
		MeshAttributes::Instance | MeshAttributes::InstanceAndPrimitive => 2,
		MeshAttributes::InstancePrimitiveAndUv => 3,
	};
	let mut out_attributes = (0..locations)
		.map(|location| buffer(&program, output_slot(location)))
		.collect::<Vec<_>>();
	let mut mesh_outputs = MeshOutputs::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_payload("meshlet_instances", mesh_task_payload(&payload, 0));
		descriptors.bind_buffer(slot(&program, MESH_DISPATCH_WORK), &mut mesh_dispatch_work);
		descriptors.bind_buffer(slot(&program, VIEWS), &mut views);
		descriptors.bind_buffer(slot(&program, MESH_DATA), &mut meshes);
		descriptors.bind_buffer(slot(&program, MESH_LEVELS), &mut mesh_levels);
		descriptors.bind_buffer(slot(&program, VERTEX_POSITIONS), &mut positions);
		if let Some(vertex_uvs) = &mut vertex_uvs {
			descriptors.bind_buffer(slot(&program, VERTEX_UVS), vertex_uvs);
		}
		descriptors.bind_buffer(slot(&program, VERTEX_INDICES), &mut vertex_indices);
		descriptors.bind_buffer(slot(&program, PRIMITIVE_INDICES), &mut primitive_indices);
		descriptors.bind_buffer(slot(&program, MESHLETS), &mut meshlets);
		for (location, attribute) in out_attributes.iter_mut().enumerate() {
			descriptors.bind_buffer(output_slot(location as u8), attribute);
		}
		descriptors.bind_push_constant(&mut push_constant);
		descriptors.bind_mesh_outputs(&mut mesh_outputs);
		// Mesh invocations share their capture just as lanes in one production mesh workgroup share output arrays.
		for thread_idx in 0..3 {
			program
				.run_main_with_config(&mut descriptors, &lane_config(thread_idx).with_threadgroup_position(0))
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
	// The fixture triangle winds counterclockwise as an identity model shows it. Back-face culling decides by winding, so
	// a model that changes the winding on screen would hide the other side.
	let triangle = mesh_outputs.triangle(0).expect("mesh triangle output");
	let mut sorted = triangle;
	sorted.sort_unstable();
	assert_eq!(sorted, [0, 1, 2]);
	let [a, b, c] = triangle.map(|index| mesh_outputs.vertex_position(index as usize).expect("mesh vertex output"));
	let screen_area =
		(b[0] / b[3] - a[0] / a[3]) * (c[1] / c[3] - a[1] / a[3]) - (c[0] / c[3] - a[0] / a[3]) * (b[1] / b[3] - a[1] / a[3]);
	assert!(screen_area > 0.0, "the triangle {triangle:?} winds clockwise on screen");
	if let Some(expected) = expected_render_target_array_index {
		assert_eq!(mesh_outputs.render_target_array_index(0), Some(expected));
	}
	if attributes != MeshAttributes::None {
		assert_eq!(
			out_attributes[0].read_indexed("out_instance_index", 0).expect("mesh output"),
			Value::U32(FIXTURE_INSTANCE_INDEX as u32)
		);
	}
	if attributes.has_primitive() {
		assert_eq!(
			out_attributes[1].read_indexed("out_primitive_index", 0).expect("mesh output"),
			Value::U32((FIXTURE_MESHLET_INDEX as u32) << 8)
		);
	}
}

/// Verifies visibility mesh output geometry and metadata through the BESL VM.
#[test]
fn visibility_mesh_main_emits_identity_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		MeshView::CAMERA,
		IDENTITY_AFFINE_MATRIX,
		0,
		[[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
		None,
		MeshAttributes::InstanceAndPrimitive,
	);
}

/// Verifies the masked visibility mesh stage also passes its fragment stage the UV, at a location of its own.
#[test]
fn masked_visibility_mesh_main_emits_identity_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("masked-visibility-mesh.besl"),
		MeshView::CAMERA,
		IDENTITY_AFFINE_MATRIX,
		0,
		[[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
		None,
		MeshAttributes::InstancePrimitiveAndUv,
	);
}

/// Verifies one mesh grid draws every meshlet of its group: each one's vertices after the earlier ones', its triangles
/// naming them there, and each triangle marked with its own meshlet and its place in it.
#[test]
fn visibility_mesh_main_draws_every_meshlet_of_its_group() {
	let program = asset!("masked-visibility-mesh.besl");
	let mut views = buffer(&program, slot(&program, VIEWS));
	views
		.write_array_member(0, "view_projection", Value::Mat4F(IDENTITY_MATRIX))
		.expect("mesh view");
	let mut meshes = array_buffer(&program, slot(&program, MESH_DATA), FIXTURE_TABLE_LENGTH);
	write_fixture_mesh(&mut meshes, 0, [0; 2]);
	let mut mesh_levels = array_buffer(&program, slot(&program, MESH_LEVELS), FIXTURE_TABLE_LENGTH);
	write_fixture_level(&mut mesh_levels, [0.0, 0.0, 0.0, 2.0], 2, 0);
	// Two triangle meshlets, the second beside the first, each over three vertices of its own.
	let mut positions = array_buffer(&program, slot(&program, VERTEX_POSITIONS), 6);
	let mut vertex_uvs = array_buffer(&program, slot(&program, VERTEX_UVS), 6);
	let mut vertex_indices = array_buffer(&program, slot(&program, VERTEX_INDICES), 6);
	let mut primitive_indices = array_buffer(&program, slot(&program, PRIMITIVE_INDICES), 6);
	let corners = [[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]];
	for index in 0..6 {
		let [x, y] = corners[index % 3];
		positions
			.write_array_element(index, Value::Vec3F([x + 4.0 * (index / 3) as f32, y, 0.0]))
			.expect("mesh vertex");
		vertex_uvs
			.write_array_element(index, Value::Vec2F16([besl::vm::f16::ZERO; 2]))
			.expect("mesh UV");
		vertex_indices
			.write_array_element(index, Value::U16(index as u16))
			.expect("vertex index");
		// Each triangle names the three vertices of its own meshlet.
		primitive_indices
			.write_array_element(index, Value::U8((index % 3) as u8))
			.expect("triangle index");
	}
	let mut meshlets = array_buffer(&program, slot(&program, MESHLETS), FIXTURE_MESHLET_INDEX + 2);
	for meshlet in 0..2 {
		let record = ShaderMeshletData::new(
			meshlet * 3,
			meshlet,
			[3, 1],
			[0.0; 4],
			[0.0, 0.0, 0.0, 2.0],
			[0.0, 0.0, 1.0],
			2.0,
		);
		write_meshlet(&mut meshlets, FIXTURE_MESHLET_INDEX + meshlet as usize, record);
	}
	let mut mesh_dispatch_work = array_buffer(&program, slot(&program, MESH_DISPATCH_WORK), FIXTURE_TABLE_LENGTH);
	mesh_dispatch_work
		.write_array_element(
			0,
			Value::U32(MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed),
		)
		.expect("mesh dispatch work");
	let mut push_constant = push_constant_buffer(&program);
	for member in ["work_item_base", "view_base", "layer_base"] {
		push_constant.write(member, Value::U32(0)).expect("mesh push constant");
	}
	push_constant.write("view_count", Value::U32(1)).expect("mesh view count");
	// The lanes that kept the two meshlets, as one group.
	let payload = [group_entry(0, 0, [3, 1], false), group_entry(1, 0, [6, 2], true)];

	let mut out_attributes = (0..3)
		.map(|location| buffer(&program, output_slot(location)))
		.collect::<Vec<_>>();
	let mut mesh_outputs = MeshOutputs::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_payload("meshlet_instances", mesh_task_payload(&payload, 0));
		descriptors.bind_buffer(slot(&program, MESH_DISPATCH_WORK), &mut mesh_dispatch_work);
		descriptors.bind_buffer(slot(&program, VIEWS), &mut views);
		descriptors.bind_buffer(slot(&program, MESH_DATA), &mut meshes);
		descriptors.bind_buffer(slot(&program, MESH_LEVELS), &mut mesh_levels);
		descriptors.bind_buffer(slot(&program, VERTEX_POSITIONS), &mut positions);
		descriptors.bind_buffer(slot(&program, VERTEX_UVS), &mut vertex_uvs);
		descriptors.bind_buffer(slot(&program, VERTEX_INDICES), &mut vertex_indices);
		descriptors.bind_buffer(slot(&program, PRIMITIVE_INDICES), &mut primitive_indices);
		descriptors.bind_buffer(slot(&program, MESHLETS), &mut meshlets);
		for (location, attribute) in out_attributes.iter_mut().enumerate() {
			descriptors.bind_buffer(output_slot(location as u8), attribute);
		}
		descriptors.bind_push_constant(&mut push_constant);
		descriptors.bind_mesh_outputs(&mut mesh_outputs);
		for thread_idx in 0..6 {
			program
				.run_main_with_config(&mut descriptors, &lane_config(thread_idx).with_threadgroup_position(0))
				.expect("production mesh shader execution");
		}
	}

	assert_eq!((mesh_outputs.vertex_count(), mesh_outputs.primitive_count()), (6, 2));
	for index in 0..6 {
		let [x, y] = corners[index % 3];
		assert_rgba_close(
			mesh_outputs.vertex_position(index).expect("mesh vertex output"),
			[x + 4.0 * (index / 3) as f32, y, 0.0, 1.0],
			0.00001,
		);
	}
	assert_eq!(
		[0, 1].map(|triangle| mesh_outputs.triangle(triangle).expect("mesh triangle output")),
		[[0, 1, 2], [3, 4, 5]]
	);
	for triangle in 0..2 {
		assert_eq!(
			out_attributes[0]
				.read_indexed("out_instance_index", triangle)
				.expect("mesh output"),
			Value::U32(FIXTURE_INSTANCE_INDEX as u32)
		);
		// Each triangle is the first of its own meshlet.
		assert_eq!(
			out_attributes[1]
				.read_indexed("out_primitive_index", triangle)
				.expect("mesh output"),
			Value::U32((FIXTURE_MESHLET_INDEX as u32 + triangle as u32) << 8)
		);
	}
}

/// Verifies that an instance reads its vertices from the range its record names, as a posed instance's own range is.
#[test]
fn visibility_mesh_main_reads_vertices_from_its_base_vertex() {
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		MeshView::CAMERA,
		IDENTITY_AFFINE_MATRIX,
		7,
		[[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
		None,
		MeshAttributes::InstanceAndPrimitive,
	);
}

/// Verifies shadow mesh output draws the view its payload names and its layer, offset from the batch's first view
/// and layer.
#[test]
fn shadow_mesh_main_emits_selected_view_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("shadow-mesh.besl"),
		MeshView::SHADOW,
		IDENTITY_AFFINE_MATRIX,
		0,
		[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		Some(3),
		MeshAttributes::None,
	);
}

/// Verifies masked shadow mesh output draws the same triangle and passes the instance its alpha test reads.
#[test]
fn masked_shadow_mesh_main_emits_selected_view_triangle_and_instance() {
	assert_triangle_mesh_program(
		asset!("masked-shadow-mesh.besl"),
		MeshView::SHADOW,
		IDENTITY_AFFINE_MATRIX,
		0,
		[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		Some(3),
		MeshAttributes::Instance,
	);
}

/// A model that mirrors x, as a scale of -1 along it does.
const MIRRORING_AFFINE_MATRIX: [f32; 12] = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// Verifies that a mirrored instance's triangles keep facing front in the visibility pass, so back-face culling hides
/// the same faces it hides when the instance isn't mirrored.
#[test]
fn visibility_mesh_main_keeps_a_mirrored_instance_facing_front() {
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		MeshView::CAMERA,
		MIRRORING_AFFINE_MATRIX,
		0,
		[[1.0, -1.0, 0.0, 1.0], [-1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
		None,
		MeshAttributes::InstanceAndPrimitive,
	);
}

/// Verifies that a mirrored instance's triangles keep facing front in the shadow passes.
#[test]
fn shadow_mesh_main_keeps_a_mirrored_instance_facing_front() {
	assert_triangle_mesh_program(
		asset!("shadow-mesh.besl"),
		MeshView::SHADOW,
		MIRRORING_AFFINE_MATRIX,
		0,
		[[3.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		Some(3),
		MeshAttributes::None,
	);
}

/* Material prepasses */

/// Binds the instance-index image, the mesh table, and the material evaluation slots, and runs one 8x8 material-count
/// workgroup.
fn run_material_count(
	program: &ExecutableProgram,
	tables: &mut PrepassTables,
	instance_indices: &mut Texture,
) -> besl::vm::Buffer {
	let mut material_counts = buffer(program, slot(program, MATERIAL_COUNT));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, MESH_DATA), &mut tables.meshes);
	descriptors.bind_buffer(slot(program, MATERIAL_EVALUATIONS), &mut tables.evaluations);
	descriptors.bind_buffer(slot(program, MATERIAL_COUNT), &mut material_counts);
	descriptors.bind_image(slot(program, INSTANCE_INDEX), instance_indices);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(program, descriptors, TILE_WORKGROUP_WIDTH, [0, 0]);
	material_counts
}

/// The light an older frame left in the lit target and the diffuse radiance history before pixel mapping runs.
const STALE_LIGHT: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

/// Runs one 16x16 pixel-mapping workgroup over the square instance-index image of `width` texels in visibility
/// `phase`, zero for opaque and one for transparent. Returns the mapping, the lit target, and the diffuse radiance
/// history, which started full of [`STALE_LIGHT`] at the instance image's size.
fn run_pixel_mapping(
	program: &ExecutableProgram,
	tables: &mut PrepassTables,
	material_offset_scratch: &mut besl::vm::Buffer,
	instance_indices: &mut Texture,
	width: u32,
	phase: u32,
) -> (besl::vm::Buffer, [Texture; 2]) {
	let mut pixel_mapping = buffer(program, slot(program, PIXEL_MAPPING));
	let mut push_constant = push_constant_buffer(program);
	push_constant.write("phase", Value::U32(phase)).expect("pixel mapping phase");
	let mut images = std::array::from_fn(|_| {
		let mut texture = Texture::new(width, width).expect("light fixture");
		for lane in 0..(width * width) {
			texture.write([lane % width, lane / width], STALE_LIGHT).expect("light texel");
		}
		texture
	});
	let [lit, diffuse_radiance_history] = &mut images;
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, MESH_DATA), &mut tables.meshes);
	descriptors.bind_buffer(slot(program, MATERIAL_EVALUATIONS), &mut tables.evaluations);
	descriptors.bind_buffer(slot(program, MATERIAL_OFFSET_SCRATCH), material_offset_scratch);
	descriptors.bind_buffer(slot(program, PIXEL_MAPPING), &mut pixel_mapping);
	descriptors.bind_image(slot(program, INSTANCE_INDEX), instance_indices);
	descriptors.bind_image(slot(program, LIT), lit);
	descriptors.bind_image(slot(program, DIFFUSE_RADIANCE_HISTORY), diffuse_radiance_history);
	descriptors.bind_push_constant(&mut push_constant);
	run_workgroup_containing::<PIXEL_MAPPING_WORKGROUP_SIZE>(program, descriptors, PIXEL_MAPPING_WORKGROUP_WIDTH, [0, 0]);
	(pixel_mapping, images)
}

/// The `PrepassTables` struct holds the mesh table and the material evaluation slots that a material prepass test binds.
struct PrepassTables {
	meshes: besl::vm::Buffer,
	evaluations: besl::vm::Buffer,
}

/// Creates the tables of a material prepass test, where mesh `i` uses the `i`th of `materials` and every material is
/// alone in the evaluation slot of its own index, so the prepasses' per-slot results are per-material results.
fn mesh_materials(program: &ExecutableProgram, materials: impl IntoIterator<Item = u32>) -> PrepassTables {
	mesh_materials_in_evaluations(program, materials, |material| material)
}

/// Creates the tables of a material prepass test, where mesh `i` uses the `i`th of `materials` and material `m` belongs
/// to the evaluation slot `evaluation(m)`.
fn mesh_materials_in_evaluations(
	program: &ExecutableProgram,
	materials: impl IntoIterator<Item = u32>,
	evaluation: impl Fn(u32) -> u32,
) -> PrepassTables {
	let mut meshes = array_buffer(program, slot(program, MESH_DATA), FIXTURE_TABLE_LENGTH);
	let mut evaluations = buffer(program, slot(program, MATERIAL_EVALUATIONS));
	for (mesh_index, material_index) in materials.into_iter().enumerate() {
		meshes
			.write_array_member(mesh_index, "material", Value::U32(material_index))
			.expect("VM mesh");
		evaluations
			.write_array_element(material_index as usize, Value::U32(evaluation(material_index)))
			.expect("VM material evaluation");
	}
	PrepassTables { meshes, evaluations }
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

/// Runs the material-offset pass's one 256-thread workgroup over `material_counts`, and returns the buffers it writes:
/// the offsets, the scratch cursors, and the indirect dispatches.
fn run_material_offset(
	program: &ExecutableProgram,
	material_counts: &mut besl::vm::Buffer,
) -> (besl::vm::Buffer, besl::vm::Buffer, besl::vm::Buffer) {
	let mut offsets = buffer(program, slot(program, MATERIAL_OFFSET));
	let mut scratch = buffer(program, slot(program, MATERIAL_OFFSET_SCRATCH));
	let mut dispatches = buffer(program, slot(program, MATERIAL_DISPATCH));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, MATERIAL_COUNT), material_counts);
	descriptors.bind_buffer(slot(program, MATERIAL_OFFSET), &mut offsets);
	descriptors.bind_buffer(slot(program, MATERIAL_OFFSET_SCRATCH), &mut scratch);
	descriptors.bind_buffer(slot(program, MATERIAL_DISPATCH), &mut dispatches);
	run_workgroup_containing::<256>(program, descriptors, 256, [0, 0]);
	(offsets, scratch, dispatches)
}

/// Verifies the offset scan gives every material the exclusive sum of the counts before it, across every thread of its
/// workgroup.
#[test]
fn material_offset_scans_every_material_count() {
	let program = asset!("material-offset.besl");
	let count = |material: usize| (material * 37 % 300) as u32;
	let mut material_counts = buffer(&program, slot(&program, MATERIAL_COUNT));
	for material in 0..super::layout::MAX_MATERIALS {
		material_counts
			.write_array_element(material, Value::U32(count(material)))
			.expect("material count");
	}

	let (offsets, scratch, dispatches) = run_material_offset(&program, &mut material_counts);

	let mut expected_offset = 0;
	for material in 0..super::layout::MAX_MATERIALS {
		assert_eq!(
			read_u32(&offsets, material),
			expected_offset,
			"Material {material} has the wrong offset."
		);
		assert_eq!(
			read_u32(&scratch, material),
			expected_offset,
			"Material {material} has the wrong cursor."
		);
		assert_eq!(read_vec3u(&dispatches, material), [count(material).div_ceil(128), 1, 1]);
		expected_offset += count(material);
	}
}

/// Exercises the production material prepasses as one stateful VM pipeline.
#[test]
fn visibility_material_compute_pipeline_counts_offsets_and_maps_valid_pixels() {
	let material_count_program = asset!("material-count.besl");
	let material_offset_program = asset!("material-offset.besl");
	let pixel_mapping_program = asset!("pixel-mapping.besl");

	// Three visible instances span two materials; the remaining texel is the renderer's empty-pixel sentinel.
	let mut tables = mesh_materials(&material_count_program, [2, 5, 2]);
	let mut instance_indices = instance_texture(2, |texel| [0, 1, u32::MAX, 2][texel]);

	let mut material_counts = run_material_count(&material_count_program, &mut tables, &mut instance_indices);
	assert_eq!(read_u32(&material_counts, 2), 2);
	assert_eq!(read_u32(&material_counts, 5), 1);
	assert_eq!(read_u32(&material_counts, 0), 0);

	// The offset pass converts sparse counts into exclusive offsets and one indirect dispatch tuple per material.
	let (material_offsets, mut material_offset_scratch, material_dispatches) =
		run_material_offset(&material_offset_program, &mut material_counts);
	assert_eq!(read_u32(&material_offsets, 2), 0);
	assert_eq!(read_u32(&material_offsets, 5), 2);
	assert_eq!(read_u32(&material_offsets, 6), 3);
	// The offset pass does not clear material_count; evaluation reads it directly for bounds.
	assert_eq!(read_u32(&material_counts, 2), 2);
	assert_eq!(read_u32(&material_counts, 5), 1);
	assert_eq!(read_vec3u(&material_dispatches, 0), [0, 1, 1]);
	assert_eq!(read_vec3u(&material_dispatches, 2), [1, 1, 1]);
	assert_eq!(read_vec3u(&material_dispatches, 5), [1, 1, 1]);

	// Mapping reuses the scratch offsets as atomic cursors and stores one-based coordinates for later zero-sentinel checks.
	let (pixel_mapping, _) = run_pixel_mapping(
		&pixel_mapping_program,
		&mut tables,
		&mut material_offset_scratch,
		&mut instance_indices,
		2,
		0,
	);
	assert_eq!(read_vec2u16(&pixel_mapping, 0), [1, 1]);
	assert_eq!(read_vec2u16(&pixel_mapping, 1), [2, 2]);
	assert_eq!(read_vec2u16(&pixel_mapping, 2), [2, 1]);
	assert_eq!(read_u32(&material_offset_scratch, 2), 2);
	assert_eq!(read_u32(&material_offset_scratch, 5), 3);
}

/// Verifies the prepasses give materials that share an evaluation slot one pixel count, one offset, one indirect
/// dispatch, and one pixel list, so one material-evaluation dispatch shades all of them.
#[test]
fn materials_sharing_an_evaluation_slot_share_one_pixel_list() {
	let material_count_program = asset!("material-count.besl");
	let material_offset_program = asset!("material-offset.besl");
	let pixel_mapping_program = asset!("pixel-mapping.besl");

	// Materials 4 and 9 compiled to one pipeline and share slot 0; material 6 is alone in slot 1.
	let mut tables = mesh_materials_in_evaluations(&material_count_program, [4, 6, 9], |material| match material {
		4 | 9 => 0,
		_ => 1,
	});
	// Texel 0 shows material 4, texel 1 material 6, and texels 2 and 3 material 9.
	let mut instance_indices = instance_texture(2, |texel| [0, 1, 2, 2][texel]);

	let mut counts = run_material_count(&material_count_program, &mut tables, &mut instance_indices);
	assert_eq!(read_u32(&counts, 0), 3);
	assert_eq!(read_u32(&counts, 1), 1);
	for material in [4, 6, 9] {
		assert_eq!(
			read_u32(&counts, material),
			0,
			"Material {material} was counted under its own index instead of its evaluation slot."
		);
	}

	let (offsets, mut scratch, dispatches) = run_material_offset(&material_offset_program, &mut counts);
	assert_eq!(read_u32(&offsets, 0), 0);
	assert_eq!(read_u32(&offsets, 1), 3);
	assert_eq!(read_vec3u(&dispatches, 0), [1, 1, 1]);
	assert_eq!(read_vec3u(&dispatches, 1), [1, 1, 1]);

	let (pixel_mapping, _) = run_pixel_mapping(&pixel_mapping_program, &mut tables, &mut scratch, &mut instance_indices, 2, 0);
	// Slot 0's list holds the pixels of both of its materials, in tile rank order; slot 1's holds material 6's pixel.
	let mut shared = (0..3).map(|index| read_vec2u16(&pixel_mapping, index)).collect::<Vec<_>>();
	shared.sort_unstable();
	assert_eq!(shared, [[1, 1], [1, 2], [2, 2]]);
	assert_eq!(read_vec2u16(&pixel_mapping, 3), [2, 1]);
	assert_eq!(read_u32(&scratch, 0), 3);
	assert_eq!(read_u32(&scratch, 1), 4);
}

/// Verifies a coherent tile reuses its established local key while preserving every pixel mapping.
#[test]
fn pixel_mapping_load_fast_path_preserves_coherent_tile_mappings() {
	let program = asset!("pixel-mapping.besl");
	let mut tables = mesh_materials(&program, [7]);
	let mut material_offset_scratch = buffer(&program, slot(&program, MATERIAL_OFFSET_SCRATCH));
	let mut instance_indices = instance_texture(PIXEL_MAPPING_WORKGROUP_WIDTH, |_| 0);

	let (pixel_mapping, _) = run_pixel_mapping(
		&program,
		&mut tables,
		&mut material_offset_scratch,
		&mut instance_indices,
		PIXEL_MAPPING_WORKGROUP_WIDTH,
		0,
	);

	let width = PIXEL_MAPPING_WORKGROUP_WIDTH as usize;
	let mut seen = vec![false; width * width];
	for mapping_index in 0..PIXEL_MAPPING_WORKGROUP_SIZE {
		let [x, y] = read_vec2u16(&pixel_mapping, mapping_index).map(usize::from);
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
		read_u32(&material_offset_scratch, 7),
		PIXEL_MAPPING_WORKGROUP_SIZE as u32,
		"Pixel Mapping advanced the coherent material cursor incorrectly."
	);
}

/// Verifies the opaque phase zeroes the lit target and both histories only where no surface was drawn, and the
/// transparent phase leaves every pixel as it found it.
#[test]
fn pixel_mapping_zeroes_uncovered_pixels_in_the_opaque_phase_only() {
	let program = asset!("pixel-mapping.besl");
	let mut tables = mesh_materials(&program, [3]);
	// Texel 0 is covered by instance 0; the other three hold the renderer's empty-pixel sentinel.
	let instances = [0, u32::MAX, u32::MAX, u32::MAX];

	for (phase, uncovered_light) in [(0, [0.0; 4]), (1, STALE_LIGHT)] {
		let mut material_offset_scratch = buffer(&program, slot(&program, MATERIAL_OFFSET_SCRATCH));
		let mut instance_indices = instance_texture(2, |texel| instances[texel]);

		let (_, images) = run_pixel_mapping(
			&program,
			&mut tables,
			&mut material_offset_scratch,
			&mut instance_indices,
			2,
			phase,
		);

		for image in &images {
			assert_eq!(
				read_vec4f(image, [0, 0]),
				STALE_LIGHT,
				"Phase {phase} touched the covered pixel, which material evaluation writes."
			);
			for coord in [[1, 0], [0, 1], [1, 1]] {
				assert_eq!(read_vec4f(image, coord), uncovered_light, "Phase {phase} at {coord:?}.");
			}
		}
	}
}

/// Verifies tile-local reservations preserve mappings when distinct materials exceed the bounded histogram.
#[test]
fn pixel_mapping_tile_reservation_preserves_overflowed_materials() {
	let program = asset!("pixel-mapping.besl");
	let mut tables = mesh_materials(&program, 0..33);
	let mut material_offset_scratch = buffer(&program, slot(&program, MATERIAL_OFFSET_SCRATCH));
	for material_index in 0..33 {
		material_offset_scratch
			.write_array_element(material_index, Value::U32(material_index as u32))
			.expect("material mapping offset");
	}
	let mut instance_indices = instance_texture(
		PIXEL_MAPPING_WORKGROUP_WIDTH,
		|lane| if lane < 33 { lane as u32 } else { u32::MAX },
	);

	let (pixel_mapping, _) = run_pixel_mapping(
		&program,
		&mut tables,
		&mut material_offset_scratch,
		&mut instance_indices,
		PIXEL_MAPPING_WORKGROUP_WIDTH,
		0,
	);

	for material_index in 0..33 {
		let expected_coordinate = [
			(material_index % PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
			(material_index / PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
		];
		assert_eq!(
			read_vec2u16(&pixel_mapping, material_index),
			expected_coordinate,
			"Unexpected coordinate for material {material_index}. The most likely cause is a dropped tile reservation."
		);
		assert_eq!(
			read_u32(&material_offset_scratch, material_index),
			material_index as u32 + 1,
			"Unexpected cursor for material {material_index}. The most likely cause is a duplicated tile reservation."
		);
	}
}

/// Verifies every pixel of a tile maps exactly once into its own material's range when subgroups hold several pixels of
/// one material and the tile holds more materials than histogram slots.
#[test]
fn pixel_mapping_maps_shared_and_overflowed_materials_exactly_once() {
	const MATERIALS: usize = 40;
	// Each material's pixels land in their own range of this many entries, which is more than any material covers.
	const RANGE: usize = 16;
	let program = asset!("pixel-mapping.besl");
	let mut tables = mesh_materials(&program, 0..MATERIALS as u32);
	let mut material_offset_scratch = buffer(&program, slot(&program, MATERIAL_OFFSET_SCRATCH));
	for material_index in 0..MATERIALS {
		material_offset_scratch
			.write_array_element(material_index, Value::U32((material_index * RANGE) as u32))
			.expect("material mapping offset");
	}
	// Horizontal pairs of texels share a material, so one subgroup ranks several pixels of most materials.
	let material_of = |texel: usize| (texel / 2) % MATERIALS;
	let width = PIXEL_MAPPING_WORKGROUP_WIDTH as usize;
	let mut instance_indices = instance_texture(PIXEL_MAPPING_WORKGROUP_WIDTH, |texel| material_of(texel) as u32);

	let (pixel_mapping, _) = run_pixel_mapping(
		&program,
		&mut tables,
		&mut material_offset_scratch,
		&mut instance_indices,
		PIXEL_MAPPING_WORKGROUP_WIDTH,
		0,
	);

	for material_index in 0..MATERIALS {
		let mut expected = (0..width * width)
			.filter(|&texel| material_of(texel) == material_index)
			.map(|texel| [(texel % width) as u16 + 1, (texel / width) as u16 + 1])
			.collect::<Vec<_>>();
		let base = material_index * RANGE;
		assert_eq!(
			read_u32(&material_offset_scratch, material_index) as usize,
			base + expected.len(),
			"Unexpected cursor for material {material_index}. The most likely cause is a dropped or duplicated reservation."
		);
		let mut mapped = (base..base + expected.len())
			.map(|index| read_vec2u16(&pixel_mapping, index))
			.collect::<Vec<_>>();
		expected.sort_unstable();
		mapped.sort_unstable();
		assert_eq!(
			mapped, expected,
			"Material {material_index} mapped the wrong pixels. The most likely cause is two pixels taking the same rank."
		);
	}
}

/// Verifies a tile with more unique materials than histogram slots preserves every count through the overflow path.
#[test]
fn material_count_tile_histogram_preserves_overflowed_materials() {
	let program = asset!("material-count.besl");
	let mut tables = mesh_materials(&program, 0..33);
	let mut instance_indices = instance_texture(TILE_WORKGROUP_WIDTH, |lane| (lane % 33) as u32);

	let material_counts = run_material_count(&program, &mut tables, &mut instance_indices);

	for material_index in 0..33 {
		let expected = if material_index < 31 { 2 } else { 1 };
		assert_eq!(
			read_u32(&material_counts, material_index),
			expected,
			"Unexpected count for material {material_index}. The most likely cause is a dropped or duplicated tile-histogram entry."
		);
	}
}

/// Verifies subgroup aggregation retains every pixel in a coherent Material Count tile.
#[test]
fn material_count_subgroup_aggregation_counts_a_coherent_tile_once_per_partition() {
	let program = asset!("material-count.besl");
	let mut tables = mesh_materials(&program, [7]);
	let mut instance_indices = instance_texture(TILE_WORKGROUP_WIDTH, |_| 0);

	let material_counts = run_material_count(&program, &mut tables, &mut instance_indices);

	assert_eq!(read_u32(&material_counts, 7), TILE_WORKGROUP_SIZE as u32);
}

/* GTAO */

const GTAO_NEAR: f32 = 0.1;
const GTAO_FAR: f32 = 100.0;
/// The fixture camera's reversed-depth terms: device depth is `GTAO_DEPTH_NUMERATOR / z - GTAO_DEPTH_OFFSET` for a
/// surface `z` units in front of the camera.
pub(super) const GTAO_DEPTH_NUMERATOR: f32 = GTAO_NEAR * GTAO_FAR / (GTAO_FAR - GTAO_NEAR);
pub(super) const GTAO_DEPTH_OFFSET: f32 = GTAO_NEAR / (GTAO_FAR - GTAO_NEAR);

/// Creates compact camera data for one square GTAO shader fixture.
fn gtao_view_data(program: &ExecutableProgram, width: u32, height: u32) -> besl::vm::Buffer {
	screen_view_buffer(program, slot(program, GTAO_VIEW), width, height)
}

/// Creates the screen-view constants of a `width` by `height` image seen through the fixture projection, for the
/// `ScreenView` binding at `slot`.
fn screen_view_buffer(program: &ExecutableProgram, slot: ResourceSlot, width: u32, height: u32) -> besl::vm::Buffer {
	let projection = math::projection_matrix(math::Degrees::new(60.0), width as f32 / height as f32, GTAO_NEAR, GTAO_FAR);
	let projection_x = projection[0];
	let projection_y = projection[5];
	let width = width as f32;
	let height = height as f32;
	let mut view = buffer(program, slot);
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
		("depth_unproject_numerator", Value::F32(GTAO_DEPTH_NUMERATOR)),
		("depth_unproject_denominator_offset", Value::F32(GTAO_DEPTH_OFFSET)),
	] {
		view.write(member, value).expect("compact GTAO view data");
	}
	view
}

/// Reconstructs the positive fixture distance encoded by one reversed device depth.
fn gtao_fixture_linear_depth(depth: f32) -> f32 {
	if depth == 0.0 {
		return 0.0;
	}
	GTAO_DEPTH_NUMERATOR / (depth + GTAO_DEPTH_OFFSET)
}

/// Encodes a positive fixture distance as the reversed device depth that [`gtao_fixture_linear_depth`] decodes. Zero,
/// the sky, stays zero.
pub(super) fn gtao_fixture_device_depth(linear_depth: f32) -> f32 {
	if linear_depth == 0.0 {
		return 0.0;
	}
	GTAO_DEPTH_NUMERATOR / linear_depth - GTAO_DEPTH_OFFSET
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

/// Runs the GTAO workgroup containing `coordinate` over a `width` by `height` image and reads that pixel.
///
/// `levels` holds the linear depth of the pyramid's first three mips, each half as wide and high as the one before.
/// `controls` holds the radius, the samples per ray, and the radial ray count.
fn run_gtao(
	program: &ExecutableProgram,
	[width, height]: [u32; 2],
	levels: [&[[f32; 4]]; 3],
	coordinate: [u32; 2],
	(radius, samples_per_ray, radial_rays): (f32, u32, u32),
) -> [f32; 4] {
	let mut view = gtao_view_data(program, width, height);
	let mut parameters = buffer(program, slot(program, GTAO_PARAMETERS));
	for (member, value) in [
		("radius", Value::F32(radius)),
		("samples_per_ray", Value::U32(samples_per_ray)),
		("radial_rays", Value::U32(radial_rays)),
	] {
		parameters.write(member, value).expect("GTAO runtime parameters");
	}
	let mut depth_pyramid = texture_2d(width, height, levels[0]);
	for (divisor, texels) in [(2, levels[1]), (4, levels[2])] {
		depth_pyramid.add_mip(texture_2d((width / divisor).max(1), (height / divisor).max(1), texels));
	}
	let mut output = empty_image(width, height);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, GTAO_VIEW), &mut view);
	descriptors.bind_buffer(slot(program, GTAO_PARAMETERS), &mut parameters);
	descriptors.bind_texture(slot(program, "depth_pyramid"), &mut depth_pyramid);
	descriptors.bind_image(slot(program, "ao_output"), &mut output);
	run_workgroup_containing::<GTAO_WORKGROUP_SIZE>(program, descriptors, GTAO_WORKGROUP_WIDTH, coordinate);
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
	let (linear_depth_2, ..) = reduce_nearest_nonzero_depth(&linear_depth_1, width_1, height_1);
	run_gtao(
		program,
		[EXTENT, EXTENT],
		[&linear_depth, &linear_depth_1, &linear_depth_2],
		coordinate,
		(1.0, 4, 6),
	)
}

/// Executes the standard GTAO shader with one deterministic device-depth fixture and explicit runtime controls.
fn run_gtao_fixture(
	program: &ExecutableProgram,
	width: u32,
	height: u32,
	depth_texels: &[[f32; 4]],
	coordinate: [u32; 2],
	controls: (f32, u32, u32),
) -> [f32; 4] {
	let linear_depth_texels = depth_texels
		.iter()
		.map(|texel| [gtao_fixture_linear_depth(texel[0]), 0.0, 0.0, 1.0])
		.collect::<Vec<_>>();
	let empty = |divisor: u32| vec![[0.0, 0.0, 0.0, 1.0]; ((width / divisor).max(1) * (height / divisor).max(1)) as usize];
	run_gtao(
		program,
		[width, height],
		[&linear_depth_texels, &empty(2), &empty(4)],
		coordinate,
		controls,
	)
}

/// Runs the production GTAO shader with uniform coarse levels so a fixture can isolate hierarchical sampling.
fn run_gtao_hierarchical_fixture(program: &ExecutableProgram, coarse_linear_depth: f32) -> [f32; 4] {
	const EXTENT: u32 = 129;
	const CENTER: [u32; 2] = [64, 64];
	let linear_depth_texels = vec![[gtao_fixture_linear_depth(0.35), 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize];
	let coarse_1 = vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 64 * 64];
	let coarse_2 = vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 32 * 32];
	run_gtao(
		program,
		[EXTENT, EXTENT],
		[&linear_depth_texels, &coarse_1, &coarse_2],
		CENTER,
		(1.0, 4, 6),
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
	// Without a cascade fit, the pass leaves the bounds alone.
	let mut bounds = buffer(program, slot(program, RECEIVER_BOUNDS));
	let mut fit = buffer(program, slot(program, RECEIVER_FIT));
	let mut push_constant = push_constant_buffer(program);
	push_constant
		.write("fitted_suns", Value::U32(0))
		.expect("depth pyramid push constant");
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, GTAO_VIEW), &mut view);
	descriptors.bind_texture(slot(program, "source_depth"), source);
	let [reduced_1, reduced_2, reduced_3] = &mut reduced;
	descriptors.bind_image(slot(program, "reduced_depth_1"), reduced_1);
	descriptors.bind_image(slot(program, "reduced_depth_2"), reduced_2);
	descriptors.bind_image(slot(program, "reduced_depth_3"), reduced_3);
	descriptors.bind_buffer(slot(program, RECEIVER_BOUNDS), &mut bounds);
	descriptors.bind_buffer(slot(program, RECEIVER_FIT), &mut fit);
	descriptors.bind_push_constant(&mut push_constant);
	run_workgroup_containing::<DEPTH_PYRAMID_WORKGROUP_SIZE>(program, descriptors, DEPTH_PYRAMID_WORKGROUP_WIDTH, [0, 0]);
	reduced
}

/// Verifies each production depth-pyramid texel keeps the nearest nonzero linear depth in its source footprint.
#[test]
fn gtao_depth_pyramid_reduces_odd_extents_to_nearest_linear_depth() {
	let program = asset!("depth-pyramid.besl");
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
	let program = asset!("depth-pyramid.besl");
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
	// The expected levels are 8x4, 4x2, and 2x1 texels.
	for (level, expected, width) in [
		(&reduced_1, &expected_1, 8),
		(&reduced_2, &expected_2, 4),
		(&reduced_3, &expected_3, 2),
	] {
		for (index, &texel) in (0..).zip(expected) {
			assert_rgba_close(rgba(level, [index % width, index / width]), texel, 0.00001);
		}
	}
}

/// Verifies the four threads of each cell combine their 4x4 blocks into the cell's nearest and farthest depth in every
/// cascade, and that the workgroup reaching past a cascade 48 texels wide writes only the cells inside it.
#[test]
fn directional_shadow_depth_pyramid_combines_each_cells_four_blocks_in_every_cascade() {
	let program = asset!("directional-shadow-depth-pyramid.besl");
	let layer_count = 4u32;
	// Each row of cells is one and a half workgroups wide, so the second workgroup is half outside the cascade.
	let (cells_wide, cells_tall) = (6u32, 2u32);
	let cell_maximum =
		|layer: u32, cell_x: u32, cell_y: u32| 0.1 + layer as f32 * 0.15 + cell_y as f32 * 0.04 + cell_x as f32 * 0.01;
	// A cell's nearest and farthest texels sit in different 4x4 blocks, and the pair rotates through the four blocks
	// along a row, so a cell's bounds come out right only when its four threads exchange their blocks' bounds.
	let extreme_texels = [([0, 1], [6, 3]), ([5, 2], [3, 4]), ([2, 7], [4, 5]), ([7, 4], [1, 2])];
	let mut source = Texture::new_3d(cells_wide * 8, cells_tall * 8, layer_count).expect("directional shadow array fixture");
	for layer in 0..layer_count {
		for y in 0..cells_tall * 8 {
			for x in 0..cells_wide * 8 {
				let (cell_x, cell_y) = (x / 8, y / 8);
				let maximum = cell_maximum(layer, cell_x, cell_y);
				let (nearest, farthest) = extreme_texels[((cell_x + layer) % 4) as usize];
				let depth = if [x % 8, y % 8] == nearest {
					maximum
				} else if [x % 8, y % 8] == farthest {
					maximum * 0.25
				} else {
					maximum * 0.5
				};
				source
					.write_3d([x, y, layer], [depth, 0.0, 0.0, 1.0])
					.expect("directional shadow source texel");
			}
		}
	}
	let mut reduced = empty_image(cells_wide, cells_tall * layer_count);
	let mut reduced_minimum = empty_image(cells_wide, cells_tall * layer_count);
	for layer in 0..layer_count {
		for workgroup_x in [0, PYRAMID_WORKGROUP_WIDTH] {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(slot(&program, "source_depth"), &mut source);
			descriptors.bind_image(slot(&program, "reduced_depth_1"), &mut reduced);
			descriptors.bind_image(slot(&program, "reduced_minimum_1"), &mut reduced_minimum);
			run_workgroup_containing::<PYRAMID_WORKGROUP_SIZE>(
				&program,
				descriptors,
				PYRAMID_WORKGROUP_WIDTH,
				[workgroup_x, layer * PYRAMID_WORKGROUP_HEIGHT],
			);
		}
	}
	for layer in 0..layer_count {
		for cell_y in 0..cells_tall {
			for cell_x in 0..cells_wide {
				let packed_cell = [cell_x, layer * cells_tall + cell_y];
				let maximum = cell_maximum(layer, cell_x, cell_y);
				assert_rgba_close(rgba(&reduced, packed_cell), [maximum, 0.0, 0.0, 1.0], 0.00001);
				assert_rgba_close(rgba(&reduced_minimum, packed_cell), [maximum * 0.25, 0.0, 0.0, 1.0], 0.00001);
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

/// Runs the GTAO blur workgroup that holds `coordinate` and reads that output pixel.
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
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(slot(program, "linear_depth"), &mut depth);
	descriptors.bind_texture(slot(program, "ao_source"), &mut ao);
	descriptors.bind_image(slot(program, "ao_output"), &mut output);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(program, descriptors, TILE_WORKGROUP_WIDTH, coordinate);
	rgba(&output, coordinate)
}

/// Runs the production depth-aware upscale workgroup whose 2x2 output quads include `coordinate`, and reads that
/// full-resolution output pixel.
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
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(program, GTAO_VIEW), &mut view);
	descriptors.bind_texture(slot(program, "visibility_depth"), &mut device_depth);
	descriptors.bind_texture(slot(program, "ao_source"), &mut ao);
	descriptors.bind_image(slot(program, "ao_output"), &mut output);
	descriptors.bind_texture(slot(program, "low_resolution_depth"), &mut linear_depth);
	let quad = [coordinate[0] / 2, coordinate[1] / 2];
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(program, descriptors, TILE_WORKGROUP_WIDTH, quad);
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
	let directional_ao: [[f32; 4]; 25] = std::array::from_fn(|index| [if index % 5 == 2 { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0]);
	let horizontal = run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &directional_ao, [2, 2]);
	assert!(
		horizontal[0] < 0.8,
		"Expected X blur to mix neighboring columns, found {horizontal:?}"
	);
}

/// Verifies the backdrop copy keeps the opaque scene's light as it is and turns reversed device depth into linear view
/// depth, with the sky at the view's far depth, which `sceneposition` rebuilds positions from.
#[test]
fn backdrop_copies_light_and_linearizes_depth() {
	let program = asset!("backdrop.besl");
	let surface_depth = 3.0;
	let lit = [[0.25, 0.5, 2.0, 1.0], [4.0, 3.0, 1.0, 1.0]];
	let depth = [
		[gtao_fixture_device_depth(surface_depth), 0.0, 0.0, 1.0],
		[0.0, 0.0, 0.0, 1.0],
	];

	let mut view = gtao_view_data(&program, 2, 1);
	let mut source_lit = texture_2d(2, 1, &lit);
	let mut source_depth = texture_2d(2, 1, &depth);
	let mut color = empty_image(2, 1);
	let mut linear_depth = empty_image(2, 1);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, GTAO_VIEW), &mut view);
	descriptors.bind_texture(slot(&program, "source_lit"), &mut source_lit);
	descriptors.bind_texture(slot(&program, "source_depth"), &mut source_depth);
	descriptors.bind_image(slot(&program, "color"), &mut color);
	descriptors.bind_image(slot(&program, "depth"), &mut linear_depth);
	for pixel in [[0, 0], [1, 0]] {
		run_at(&program, &mut descriptors, pixel);
	}
	drop(descriptors);

	for x in 0..2 {
		assert_rgba_close(rgba(&color, [x, 0]), lit[x as usize], 0.00001);
	}
	assert!(
		(rgba(&linear_depth, [0, 0])[0] - surface_depth).abs() < 0.001,
		"Expected the surface's linear depth {surface_depth}, found {:?}",
		rgba(&linear_depth, [0, 0])
	);
	assert!(
		(rgba(&linear_depth, [1, 0])[0] - GTAO_FAR).abs() < 0.01,
		"Expected the sky at the far depth {GTAO_FAR}, found {:?}",
		rgba(&linear_depth, [1, 0])
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

/* SSGI */

const SSGI_PARAMETERS: &str = "ssgi_parameters";
const SSGI_EXTENT: u32 = 32;
/// The uniform diffuse radiance of the previous frame in trace fixtures, as the trace reports it for a hit.
const SSGI_LIT_COLOR: [f32; 4] = [2.0, 1.0, 0.5, 1.0];

/// Returns the square fixture projection that [`gtao_view_data`] also encodes. The screen-space reflection tests
/// share it.
pub(super) fn ssgi_projection() -> maths_rs::Mat4f {
	math::projection_matrix(math::Degrees::new(60.0), 1.0, GTAO_NEAR, GTAO_FAR)
}

/// Creates SSGI per-frame parameters. `previous_clip` is `None` when the frame has no usable history.
fn ssgi_parameters(program: &ExecutableProgram, previous_clip: Option<maths_rs::Mat4f>, frame_index: u32) -> besl::vm::Buffer {
	let mut parameters = buffer(program, slot(program, SSGI_PARAMETERS));
	for (member, value) in [
		(
			"current_view_to_previous_clip",
			Value::Mat4F(column_major(previous_clip.unwrap_or_else(maths_rs::Mat4f::identity))),
		),
		(
			"current_view_to_previous_view",
			Value::Mat4F(column_major(maths_rs::Mat4f::identity())),
		),
		(
			"noise_offset",
			Value::Vec2F(crate::rendering::lookup_textures::blue_noise_offset(u64::from(frame_index))),
		),
		("history_valid", Value::U32(previous_clip.is_some() as u32)),
		("history_exposure_ratio", Value::F32(1.0)),
	] {
		parameters.write(member, value).expect("SSGI parameters");
	}
	parameters
}

/// Returns the engine's blue noise as the RG8 texture the SSGI trace reads, each byte `k` as `k / 255`.
fn ssgi_blue_noise() -> Texture {
	use crate::rendering::lookup_textures::{BLUE_NOISE, BLUE_NOISE_SIDE};

	let texels: Vec<[f32; 4]> = BLUE_NOISE
		.chunks_exact(2)
		.map(|texel| [texel[0] as f32 / 255.0, texel[1] as f32 / 255.0, 0.0, 1.0])
		.collect();
	texture_2d(BLUE_NOISE_SIDE, BLUE_NOISE_SIDE, &texels)
}

/// Returns the view-space ray `(x / z, y / z)` through the center of pixel `(x, y)` of a square fixture image
/// `extent` pixels wide, seen through [`ssgi_projection`].
pub(super) fn ssgi_ray_at(x: u32, y: u32, extent: u32) -> [f32; 2] {
	let projection = ssgi_projection();
	[
		(2.0 * (x as f32 + 0.5) / extent as f32 - 1.0) / projection[0],
		(1.0 - 2.0 * (y as f32 + 0.5) / extent as f32) / projection[5],
	]
}

/// Returns the depth a ray sees in a scene with a floor one unit below the camera and, optionally, a facing wall.
pub(super) fn ssgi_floor_depth(ray: [f32; 2], wall_z: Option<f32>) -> f32 {
	let floor_z = if ray[1] < 0.0 { -1.0 / ray[1] } else { f32::INFINITY };
	let depth = wall_z.map_or(floor_z, |wall_z| floor_z.min(wall_z));
	if depth <= GTAO_FAR { depth } else { 0.0 }
}

/// Renders `scene` into the half-resolution depth the trace marches, `extent` pixels square, and the full-resolution
/// diffuse radiance it reads, whose alpha holds each pixel's depth. `scene` maps a pixel ray to its depth and radiance.
fn ssgi_scene_images(extent: u32, scene: impl Fn([f32; 2]) -> (f32, [f32; 3])) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
	let image = |extent: u32| {
		(0..extent * extent)
			.map(|index| scene(ssgi_ray_at(index % extent, index / extent, extent)))
			.collect::<Vec<_>>()
	};
	let depth = image(extent).into_iter().map(|(z, _)| [z, 0.0, 0.0, 1.0]).collect();
	let radiance = image(extent * 2).into_iter().map(|(z, [r, g, b])| [r, g, b, z]).collect();
	(depth, radiance)
}

/// Builds the floor and optional wall scene lit uniformly with [`SSGI_LIT_COLOR`], `extent` pixels square.
fn ssgi_floor_scene(extent: u32, wall_z: Option<f32>) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
	let [r, g, b, _] = SSGI_LIT_COLOR;
	ssgi_scene_images(extent, |ray| (ssgi_floor_depth(ray, wall_z), [r, g, b]))
}

/// Runs the SSGI trace at one pixel of the floor and optional wall scene and returns the raw radiance it writes.
fn run_ssgi_trace(
	program: &ExecutableProgram,
	wall_z: Option<f32>,
	history: bool,
	frame_index: u32,
	pixel: [u32; 2],
) -> [f32; 4] {
	let (depth, radiance) = ssgi_floor_scene(SSGI_EXTENT, wall_z);
	run_ssgi_trace_outputs(program, SSGI_EXTENT, &depth, &radiance, history, frame_index, &[pixel], 1.0)[0].0
}

/// Runs the SSGI trace at `extent` pixels square with a full-resolution previous radiance image, twice the trace
/// extent on each axis, and returns the raw radiance and the stored normal it writes at each of `pixels`.
///
/// The trace shares a depth tile across its workgroup, so this runs each 8x8 workgroup that holds one of `pixels` once.
fn run_ssgi_trace_outputs(
	program: &ExecutableProgram,
	extent: u32,
	depth: &[[f32; 4]],
	radiance: &[[f32; 4]],
	history: bool,
	frame_index: u32,
	pixels: &[[u32; 2]],
	history_exposure_ratio: f32,
) -> Vec<([f32; 4], [f32; 4])> {
	let mut view = screen_view_buffer(program, slot(program, SCREEN_VIEW), extent, extent);
	// A static camera reprojects through the unchanged projection.
	let mut parameters = ssgi_parameters(program, history.then(ssgi_projection), frame_index);
	parameters
		.write("history_exposure_ratio", Value::F32(history_exposure_ratio))
		.unwrap();
	let mut depth_pyramid = texture_2d(extent, extent, depth);
	let mut previous_lit = texture_2d(extent * 2, extent * 2, radiance);
	let mut blue_noise = ssgi_blue_noise();
	let mut output = empty_image(extent, extent);
	let mut normals = empty_image(extent, extent);
	let mut workgroups: Vec<[u32; 2]> = pixels
		.iter()
		.map(|pixel| pixel.map(|coordinate| coordinate - coordinate % TILE_WORKGROUP_WIDTH))
		.collect();
	workgroups.sort_unstable();
	workgroups.dedup();
	for workgroup_base in workgroups {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(program, SCREEN_VIEW), &mut view);
		descriptors.bind_buffer(slot(program, SSGI_PARAMETERS), &mut parameters);
		descriptors.bind_texture(slot(program, "depth_pyramid"), &mut depth_pyramid);
		descriptors.bind_image(slot(program, "raw_output"), &mut output);
		descriptors.bind_texture(slot(program, "previous_radiance_map"), &mut previous_lit);
		descriptors.bind_image(slot(program, "normal_output"), &mut normals);
		descriptors.bind_texture(slot(program, "blue_noise"), &mut blue_noise);
		run_workgroup_containing::<TILE_WORKGROUP_SIZE>(program, descriptors, TILE_WORKGROUP_WIDTH, workgroup_base);
	}
	pixels
		.iter()
		.map(|&pixel| (rgba(&output, pixel), rgba(&normals, pixel)))
		.collect()
}

/// Verifies rays that hit visible geometry return last frame's light there, and that some rays do hit a nearby wall.
#[test]
fn ssgi_trace_gathers_last_frame_light_from_geometry_that_rays_hit() {
	let program = asset!("ssgi-trace.besl");
	let (depth, _) = ssgi_floor_scene(SSGI_EXTENT, Some(4.0));
	// This floor pixel lies about 0.3 units in front of the wall, so rays leaning toward the wall hit it.
	let pixel = [SSGI_EXTENT / 2, 23];
	assert!(depth[(pixel[1] * SSGI_EXTENT + pixel[0]) as usize][0] < 4.0);

	let mut hits = 0;
	for frame_index in 0..32 {
		let radiance = run_ssgi_trace(&program, Some(4.0), true, frame_index, pixel);
		if radiance[3] == 0.0 {
			assert_rgba_close(radiance, [0.0; 4], 0.0);
		} else {
			assert_rgba_close(radiance, SSGI_LIT_COLOR, 0.0001);
			hits += 1;
		}
	}
	assert!(
		hits > 4 && hits < 32,
		"Expected some but not all rays to hit the wall, found {hits} hits in 32 frames."
	);
}

/// Verifies daylight-strength history keeps its bounce energy when the camera switches to a daylight exposure.
#[test]
fn ssgi_trace_preserves_daylight_radiance_across_an_exposure_change() {
	let program = asset!("ssgi-trace.besl");
	let (depth, mut radiance) = ssgi_floor_scene(SSGI_EXTENT, Some(4.0));
	let exposure = 1.0 / (1.2 * 32768.0);
	for color in &mut radiance {
		color[..3].copy_from_slice(&[20000.0, 10000.0, 5000.0]);
	}
	let pixel = [SSGI_EXTENT / 2, 23];
	let mut hits = 0;
	for frame_index in 0..8 {
		let (gathered, _) = run_ssgi_trace_outputs(
			&program,
			SSGI_EXTENT,
			&depth,
			&radiance,
			true,
			frame_index,
			&[pixel],
			exposure,
		)[0];
		if gathered[3] != 0.0 {
			assert_rgba_close(
				gathered,
				[20000.0 * exposure, 10000.0 * exposure, 5000.0 * exposure, 1.0],
				0.0001,
			);
			hits += 1;
		}
	}
	assert!(hits > 0, "The daylight fixture must gather bounce light.");
}

/// Verifies a flat floor never occludes itself, so every ray misses and the environment lights the pixel.
#[test]
fn ssgi_trace_reports_misses_on_an_unoccluded_floor() {
	let program = asset!("ssgi-trace.besl");
	for frame_index in 0..16 {
		assert_rgba_close(
			run_ssgi_trace(&program, None, true, frame_index, [SSGI_EXTENT / 2, 23]),
			[0.0; 4],
			0.0,
		);
	}
}

/// Verifies that without history the trace gathers nothing, because no radiance of this sink exists yet.
#[test]
fn ssgi_trace_reports_misses_without_history() {
	let program = asset!("ssgi-trace.besl");
	for frame_index in 0..16 {
		assert_rgba_close(
			run_ssgi_trace(&program, Some(4.0), false, frame_index, [SSGI_EXTENT / 2, 23]),
			[0.0; 4],
			0.0,
		);
	}
}

/// Verifies the trace stores the camera-facing normal it rebuilds as the octahedral pair the denoiser and upscale
/// decode, and marks a pixel without a surface with the zero pair.
#[test]
fn ssgi_trace_stores_octahedral_normals() {
	let program = asset!("ssgi-trace.besl");
	let (mut depth, radiance) = ssgi_floor_scene(SSGI_EXTENT, Some(4.0));
	let column = SSGI_EXTENT / 2;
	let wall_row = (0..SSGI_EXTENT)
		.find(|&row| depth[(row * SSGI_EXTENT + column) as usize][0] == 4.0 && row > 2)
		.expect("a wall row away from the image edge");
	let stored = |depth: &[[f32; 4]], pixel| {
		run_ssgi_trace_outputs(&program, SSGI_EXTENT, depth, &radiance, false, 0, &[pixel], 1.0)[0].1
	};

	assert_rgba_close(stored(&depth, [column, wall_row]), SSGI_WALL_NORMAL, 0.0001);
	depth[(wall_row * SSGI_EXTENT + column) as usize] = [0.0; 4];
	assert_rgba_close(stored(&depth, [column, wall_row]), [0.0; 4], 0.0);
}

const SSGI_TEMPORAL_EXTENT: u32 = 8;
/// The stored view-space normal of a wall that faces the camera, (0, 0, -1). View space is y-up and the camera looks
/// down positive z. The trace stores normals as an octahedral pair in RG, which folds the lower z hemisphere into the
/// corners.
pub(super) const SSGI_WALL_NORMAL: [f32; 4] = [1.0, 1.0, 0.0, 0.0];
/// The stored view-space normal of the floor below the camera, (0, 1, 0).
const SSGI_FLOOR_NORMAL: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

/// The `SsgiTemporalFixture` struct supplies matching current and previous surfaces for temporal-filter tests.
/// Every image is `extent` pixels square.
struct SsgiTemporalFixture {
	extent: u32,
	depth: Vec<[f32; 4]>,
	normals: Vec<[f32; 4]>,
	raw: Vec<[f32; 4]>,
	previous_depth: Vec<[f32; 4]>,
	previous_normals: Vec<[f32; 4]>,
	previous_history: [f32; 4],
	history: bool,
	history_exposure_ratio: f32,
	current_to_previous_view: maths_rs::Mat4f,
}

impl SsgiTemporalFixture {
	/// A camera-facing wall at depth five with uniform inputs and matching previous depth.
	fn uniform(raw: [f32; 4], previous_history: [f32; 4], history: bool) -> Self {
		let texel_count = (SSGI_TEMPORAL_EXTENT * SSGI_TEMPORAL_EXTENT) as usize;
		Self {
			extent: SSGI_TEMPORAL_EXTENT,
			depth: vec![[5.0, 0.0, 0.0, 1.0]; texel_count],
			normals: vec![SSGI_WALL_NORMAL; texel_count],
			raw: vec![raw; texel_count],
			previous_depth: vec![[5.0, 0.0, 0.0, 1.0]; texel_count],
			previous_normals: vec![SSGI_WALL_NORMAL; texel_count],
			previous_history,
			history,
			history_exposure_ratio: 1.0,
			current_to_previous_view: maths_rs::Mat4f::identity(),
		}
	}

	fn run(&self, pixel: [u32; 2]) -> [f32; 4] {
		let program = asset!("ssgi-temporal.besl");
		let extent = self.extent;
		let texel_count = (extent * extent) as usize;
		let mut view = screen_view_buffer(&program, slot(&program, SCREEN_VIEW), extent, extent);
		let mut parameters = ssgi_parameters(
			&program,
			self.history.then(|| ssgi_projection() * self.current_to_previous_view),
			0,
		);
		parameters
			.write(
				"current_view_to_previous_view",
				Value::Mat4F(column_major(self.current_to_previous_view)),
			)
			.unwrap();
		parameters
			.write("history_exposure_ratio", Value::F32(self.history_exposure_ratio))
			.unwrap();
		let mut depth_pyramid = texture_2d(extent, extent, &self.depth);
		let mut raw = texture_2d(extent, extent, &self.raw);
		let mut output = empty_image(extent, extent);
		let mut previous_history = texture_2d(extent, extent, &vec![self.previous_history; texel_count]);
		let mut previous_depth_pyramid = texture_2d(extent, extent, &self.previous_depth);
		let mut normals = texture_2d(extent, extent, &self.normals);
		let mut previous_normals = texture_2d(extent, extent, &self.previous_normals);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(&program, SCREEN_VIEW), &mut view);
		descriptors.bind_buffer(slot(&program, SSGI_PARAMETERS), &mut parameters);
		descriptors.bind_texture(slot(&program, "depth_pyramid"), &mut depth_pyramid);
		descriptors.bind_texture(slot(&program, "raw_radiance"), &mut raw);
		descriptors.bind_image(slot(&program, "history_output"), &mut output);
		descriptors.bind_texture(slot(&program, "previous_history"), &mut previous_history);
		descriptors.bind_texture(slot(&program, "previous_depth_pyramid"), &mut previous_depth_pyramid);
		descriptors.bind_texture(slot(&program, "surface_normals"), &mut normals);
		descriptors.bind_texture(slot(&program, "previous_surface_normals"), &mut previous_normals);
		run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&program, descriptors, TILE_WORKGROUP_WIDTH, pixel);
		rgba(&output, pixel)
	}
}

/// Verifies the temporal stage uses only this frame's rays when there is no history.
#[test]
fn ssgi_temporal_ignores_history_when_it_is_invalid() {
	let raw = [0.4, 0.2, 0.1, 0.5];
	assert_rgba_close(
		SsgiTemporalFixture::uniform(raw, [f32::NAN; 4], false).run([4, 4]),
		raw,
		0.00001,
	);
}

/// Verifies reprojected history of the same surface is blended in with a 90% weight.
#[test]
fn ssgi_temporal_accumulates_history_of_the_same_surface() {
	let raw = [1.0, 1.0, 1.0, 1.0];
	let history = [0.0, 0.5, 0.0, 0.0];
	assert_rgba_close(
		SsgiTemporalFixture::uniform(raw, history, true).run([4, 4]),
		[0.1, 0.55, 0.1, 0.1],
		0.00001,
	);
}

/// Verifies an exposure change rescales accumulated light while preserving the fraction of rays that hit.
#[test]
fn ssgi_temporal_rescales_history_rgb_without_changing_hit_fraction() {
	let mut fixture = SsgiTemporalFixture::uniform([2.0, 1.0, 0.5, 0.25], [0.5, 0.25, 0.125, 0.75], true);
	fixture.history_exposure_ratio = 4.0;
	assert_rgba_close(fixture.run([4, 4]), [2.0, 1.0, 0.5, 0.7], 0.00001);
}

/// Verifies history is rejected where the previous frame saw a different surface.
#[test]
fn ssgi_temporal_rejects_history_of_a_disoccluded_surface() {
	let raw = [0.4, 0.2, 0.1, 0.5];
	let mut fixture = SsgiTemporalFixture::uniform(raw, [f32::NAN; 4], true);
	fixture.previous_depth = vec![[8.0, 0.0, 0.0, 1.0]; fixture.previous_depth.len()];
	assert_rgba_close(fixture.run([4, 4]), raw, 0.00001);
}

/// Verifies history is rejected where the previous frame saw a surface facing another way at the same depth, as
/// where a foot meets the floor.
#[test]
fn ssgi_temporal_rejects_history_of_a_surface_facing_another_way() {
	let raw = [0.4, 0.2, 0.1, 0.5];
	let mut fixture = SsgiTemporalFixture::uniform(raw, [f32::NAN; 4], true);
	fixture.previous_normals = vec![SSGI_FLOOR_NORMAL; fixture.previous_normals.len()];
	assert_rgba_close(fixture.run([4, 4]), raw, 0.00001);
}

/// Verifies the spatial filter does not average rays from a surface at a different depth.
#[test]
fn ssgi_temporal_filter_keeps_light_on_its_own_surface() {
	let mut fixture = SsgiTemporalFixture::uniform([0.0; 4], [0.0; 4], false);
	let half = SSGI_TEMPORAL_EXTENT / 2;
	for index in 0..fixture.depth.len() {
		let near = (index as u32 % SSGI_TEMPORAL_EXTENT) < half;
		fixture.depth[index] = [if near { 2.0 } else { 10.0 }, 0.0, 0.0, 1.0];
		fixture.raw[index] = if near { [1.0; 4] } else { [0.0; 4] };
	}
	assert_rgba_close(fixture.run([half - 1, 4]), [1.0; 4], 0.0001);
	assert_rgba_close(fixture.run([half, 4]), [0.0; 4], 0.0001);
}

/// The depth of the wall that stands on the floor of [`ssgi_floor_depth`] in the contact scene.
pub(super) const SSGI_CONTACT_WALL_Z: f32 = 4.0;

/// Renders the contact scene, a wall at [`SSGI_CONTACT_WALL_Z`] standing on the floor of [`ssgi_floor_depth`], into
/// `extent` square images of its depth, its view-space normals, and its light, which only the wall gathered.
pub(super) fn ssgi_contact_images(extent: u32) -> [Vec<[f32; 4]>; 3] {
	let (mut depth, mut normals, mut light) = (Vec::new(), Vec::new(), Vec::new());
	for index in 0..extent * extent {
		let z = ssgi_floor_depth(ssgi_ray_at(index % extent, index / extent, extent), Some(SSGI_CONTACT_WALL_Z));
		let wall = z == SSGI_CONTACT_WALL_Z;
		depth.push([z, 0.0, 0.0, 1.0]);
		normals.push(if wall { SSGI_WALL_NORMAL } else { SSGI_FLOOR_NORMAL });
		light.push(if wall { [1.0; 4] } else { [0.0; 4] });
	}
	[depth, normals, light]
}

/// Verifies the spatial filter keeps light off a surface that touches the center's surface at the same depth, as a
/// floor meets a wall or a foot.
#[test]
fn ssgi_temporal_filter_keeps_light_off_a_touching_surface() {
	const EXTENT: u32 = 16;
	let mut fixture = SsgiTemporalFixture::uniform([0.0; 4], [0.0; 4], false);
	let [depth, normals, raw] = ssgi_contact_images(EXTENT);
	fixture.extent = EXTENT;
	fixture.previous_depth = depth.clone();
	fixture.previous_normals = normals.clone();
	fixture.depth = depth;
	fixture.normals = normals;
	fixture.raw = raw;
	let column = EXTENT / 2;
	let floor_row = (0..EXTENT)
		.find(|&row| fixture.depth[(row * EXTENT + column) as usize][0] != SSGI_CONTACT_WALL_Z)
		.expect("the wall stands on the floor");
	let floor_z = fixture.depth[(floor_row * EXTENT + column) as usize][0];
	assert!(
		(SSGI_CONTACT_WALL_Z - floor_z) / SSGI_CONTACT_WALL_Z < 0.02,
		"The fixture floor next to the wall must share its depth, found {floor_z}."
	);

	let floor = fixture.run([column, floor_row]);
	let wall = fixture.run([column, floor_row - 1]);
	assert!(
		floor[0] < 0.01,
		"Expected no wall light on the floor next to it, found {floor:?}."
	);
	assert!(wall[0] > 0.99, "Expected the wall to keep its light, found {wall:?}.");
}

/// Verifies rays from a wall find the floor in front of it, a surface seen at a grazing angle that one march step
/// can cross by more than the hit thickness.
#[test]
fn ssgi_trace_finds_a_grazing_floor_that_rays_cross_between_steps() {
	let program = asset!("ssgi-trace.besl");
	let (depth, _) = ssgi_floor_scene(SSGI_EXTENT, Some(4.0));
	// This wall pixel sits just above the floor, so about half of its cosine-weighted rays point down into it.
	let pixel = [SSGI_EXTENT / 2, 22];
	assert_eq!(depth[(pixel[1] * SSGI_EXTENT + pixel[0]) as usize][0], 4.0);

	let hits = (0..64)
		.map(|frame_index| run_ssgi_trace(&program, Some(4.0), true, frame_index, pixel))
		.filter(|radiance| radiance[3] != 0.0)
		.inspect(|&radiance| assert_rgba_close(radiance, SSGI_LIT_COLOR, 0.0001))
		.count();
	assert!(
		hits >= 24,
		"Expected about half the rays to hit the floor, found {hits} hits in 64 frames."
	);
}

/// Verifies rays from a surface that faces the camera reach the floor between it and the camera.
///
/// Every ray from such a surface heads toward the camera, and its projection grows without bound near the camera
/// plane. Cutting the ray to the screen-space reach must not also shrink its world-space path.
#[test]
fn ssgi_trace_rays_toward_the_camera_reach_the_floor_in_front_of_a_wall() {
	const EXTENT: u32 = 256;
	const WALL_Z: f32 = 4.0;
	let program = asset!("ssgi-trace.besl");
	let (depth, radiance) = ssgi_floor_scene(EXTENT, Some(WALL_Z));
	// This wall pixel sits half a unit above the floor, whose nearest visible part lies about 35 pixels lower.
	let pixel = [EXTENT / 2, 155];
	assert_eq!(depth[(pixel[1] * EXTENT + pixel[0]) as usize][0], WALL_Z);

	let hits = (0..64)
		.map(|frame_index| run_ssgi_trace_outputs(&program, EXTENT, &depth, &radiance, true, frame_index, &[pixel], 1.0)[0].0)
		.filter(|radiance| radiance[3] != 0.0)
		.inspect(|&radiance| assert_rgba_close(radiance, SSGI_LIT_COLOR, 0.0001))
		.count();
	// About a third of cosine-weighted rays point down steeply enough to land on the floor within reach.
	assert!(
		hits >= 16,
		"Expected about a third of the rays to hit the floor, found {hits} hits in 64 frames."
	);
}

/// Returns the view-space depth and light that a ray through `ray` sees in a scene with the green floor of
/// [`ssgi_floor_depth`] and a red pillar whose front face stands at depth three.
fn ssgi_pillar_scene(ray: [f32; 2]) -> (f32, [f32; 3]) {
	const PILLAR_Z: f32 = 3.0;
	if (ray[0] * PILLAR_Z).abs() <= 0.25 && (-1.0..=0.5).contains(&(ray[1] * PILLAR_Z)) {
		return (PILLAR_Z, [1.0, 0.0, 0.0]);
	}
	(ssgi_floor_depth(ray, None), [0.0, 1.0, 0.0])
}

/// Verifies floor rays that reach a pillar take its light and never read the floor seen past the pillar's edge.
///
/// A floor cannot light itself, so every hit from a floor pixel in front of the pillar must carry the pillar's red.
#[test]
fn ssgi_trace_does_not_read_the_background_past_a_silhouette() {
	let program = asset!("ssgi-trace.besl");
	let (depth, radiance) = ssgi_scene_images(SSGI_EXTENT, ssgi_pillar_scene);
	// Floor pixels just in front of the pillar's base and beside it, where rays that lean forward reach its face.
	let pillar_columns: Vec<u32> = (0..SSGI_EXTENT)
		.filter(|&column| depth[(20 * SSGI_EXTENT + column) as usize][0] == 3.0)
		.collect();
	let first_floor_row = (0..SSGI_EXTENT)
		.find(|&row| row > 20 && depth[(row * SSGI_EXTENT + pillar_columns[0]) as usize][0] != 3.0)
		.expect("the pillar stands on the floor");
	let pixels: Vec<[u32; 2]> = (first_floor_row..first_floor_row + 3)
		.flat_map(|row| (pillar_columns[0] - 2..=pillar_columns[pillar_columns.len() - 1] + 2).map(move |column| [column, row]))
		.collect();
	assert!(
		pixels
			.iter()
			.all(|pixel| depth[(pixel[1] * SSGI_EXTENT + pixel[0]) as usize][0] < 3.0)
	);

	let mut hits = 0;
	for frame_index in 0..32 {
		let outputs = run_ssgi_trace_outputs(&program, SSGI_EXTENT, &depth, &radiance, true, frame_index, &pixels, 1.0);
		for (&pixel, &(radiance, _)) in pixels.iter().zip(&outputs) {
			if radiance[3] != 0.0 {
				hits += 1;
				assert!(
					radiance[0] > radiance[1],
					"Floor pixel {pixel:?} read {radiance:?}, the floor behind the pillar, in frame {frame_index}."
				);
			}
		}
	}
	assert!(hits > 0, "Expected some floor rays to hit the pillar.");
}

/* Contact shadows */

const CONTACT_SHADOW_EXTENT: u32 = 128;
const CONTACT_SHADOW_CAMERA_HEIGHT: f32 = 2.0;
const CONTACT_SHADOW_WALL_Z: f32 = 4.0;
const CONTACT_SHADOW_WALL_HEIGHT: f32 = 0.25;

/// Returns the depth pixel `(x, y)` of the contact-shadow image sees on a floor [`CONTACT_SHADOW_CAMERA_HEIGHT`] below
/// the camera and, optionally, a low wall facing the camera at [`CONTACT_SHADOW_WALL_Z`], or zero for the sky.
fn contact_shadow_scene_depth([x, y]: [u32; 2], wall: bool) -> f32 {
	let ray = ssgi_ray_at(x, y, CONTACT_SHADOW_EXTENT);
	let wall_height = ray[1] * CONTACT_SHADOW_WALL_Z + CONTACT_SHADOW_CAMERA_HEIGHT;
	if wall && (0.0..=CONTACT_SHADOW_WALL_HEIGHT).contains(&wall_height) {
		return CONTACT_SHADOW_WALL_Z;
	}
	if ray[1] < 0.0 {
		-CONTACT_SHADOW_CAMERA_HEIGHT / ray[1]
	} else {
		0.0
	}
}

/// Returns the positive linear depth of every pixel of the floor scene, with or without the low wall.
fn contact_shadow_linear_depth(wall: bool) -> Vec<[f32; 4]> {
	let extent = CONTACT_SHADOW_EXTENT;
	(0..extent * extent)
		.map(|index| {
			[
				contact_shadow_scene_depth([index % extent, index / extent], wall),
				0.0,
				0.0,
				1.0,
			]
		})
		.collect()
}

/// The ray reach the contact-shadow tests trace with. The wall test's rows are laid out for it.
const CONTACT_SHADOW_TEST_DISTANCE: f32 = 0.3;

/// Runs the contact-shadow trace at one texel of the floor scene, traced at the scene's own resolution, and returns the
/// value it writes: one where the ray toward the light is clear, falling toward zero where it is blocked.
fn run_contact_shadows(wall: bool, direction_to_light: [f32; 3], pixel: [u32; 2]) -> f32 {
	run_contact_shadows_toward(wall, &[direction_to_light], pixel)[0]
}

/// Runs the contact-shadow trace at one texel of the floor scene with one sun slot per direction, and returns the texel
/// it writes, sun slot `s` in channel `s`.
fn run_contact_shadows_toward(wall: bool, directions_to_light: &[[f32; 3]], pixel: [u32; 2]) -> [f32; 4] {
	let program = asset!("contact-shadows.besl");
	let extent = CONTACT_SHADOW_EXTENT;
	let mut view = screen_view_buffer(&program, slot(&program, SCREEN_VIEW), extent, extent);
	let mut parameters = buffer(&program, slot(&program, "contact_shadow_parameters"));
	for (slot, [x, y, z]) in directions_to_light.iter().copied().enumerate() {
		let length = (x * x + y * y + z * z).sqrt();
		parameters
			.write_indexed("suns", slot, Value::Vec4F([x / length, y / length, z / length, 0.0]))
			.expect("contact shadow parameters");
	}
	parameters
		.write("max_distance", Value::F32(CONTACT_SHADOW_TEST_DISTANCE))
		.expect("contact shadow parameters");
	parameters
		.write("sun_count", Value::U32(directions_to_light.len() as u32))
		.expect("contact shadow parameters");
	let mut depth = texture_2d(extent, extent, &contact_shadow_linear_depth(wall));
	let mut output = empty_image(extent, extent);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, SCREEN_VIEW), &mut view);
	descriptors.bind_buffer(slot(&program, "contact_shadow_parameters"), &mut parameters);
	descriptors.bind_texture(slot(&program, "depth_pyramid"), &mut depth);
	descriptors.bind_image(slot(&program, "contact_shadow_output"), &mut output);
	run_at(&program, &mut descriptors, pixel);
	drop(descriptors);
	rgba(&output, pixel)
}

/// Texels per side of the sun's shadow map, and of each of its four cascades, in the sun visibility tests.
const SUN_SHADOW_MAP_EXTENT: u32 = 512;
/// Half the width, in meters, of the square the test cascade covers.
const SUN_CASCADE_HALF_EXTENT: f32 = 6.0;

/// Returns the world-space direction the test sun's light travels: down and toward the camera at 45 degrees, over the
/// low wall of the floor scene, so the wall shadows the quarter meter of floor just in front of it.
fn sun_direction() -> math::UnitVector {
	math::Vector::new(0.0, -1.0, -1.0).normalized().expect("sun direction")
}

/// Returns the test sun's cascade: an orthographic view along the light that covers the floor scene. Every cascade
/// uses it, so each receiver's depth cascade is cascade zero.
fn sun_cascade_view() -> crate::rendering::View {
	let half_extent = SUN_CASCADE_HALF_EXTENT;
	crate::rendering::View::new_orthographic(
		-half_extent,
		half_extent,
		-half_extent,
		half_extent,
		0.0,
		40.0,
		math::Point::new(0.0, 10.0, 14.0),
		sun_direction(),
	)
}

/// Renders the floor scene with the low wall into the test sun's shadow map: each texel stores the depth of the
/// surface the light meets first along its ray, the wall where the ray crosses it above the floor, else the floor, or
/// zero where the ray meets nothing. Returns the map with four cascade layers for each of `suns` sun slots, the last
/// slot's cascades drawn, and the layers' max-depth and min-depth cell pyramids as the directional shadow pyramid pass
/// builds them.
fn sun_shadow_map(suns: u32) -> (Texture, Texture, Texture) {
	let extent = SUN_SHADOW_MAP_EXTENT;
	let cascades = super::layout::SHADOW_CASCADE_COUNT as u32 * suns;
	let first_drawn_layer = cascades - super::layout::SHADOW_CASCADE_COUNT as u32;
	let view_projection = sun_cascade_view().view_projection();
	let inverse_view_projection = math::inverse(view_projection);
	let direction = sun_direction().into_maths();
	let depth_at = |x: u32, y: u32| {
		// The texel center's normalized device position on the near plane, as the receiver projection lays uv out.
		let ndc = [
			(x as f32 + 0.5) / extent as f32 * 2.0 - 1.0,
			1.0 - (y as f32 + 0.5) / extent as f32 * 2.0,
		];
		let near = inverse_view_projection * maths_rs::Vec4f::new(ndc[0], ndc[1], 1.0, 1.0);
		let origin = maths_rs::Vec3f::new(near.x / near.w, near.y / near.w, near.z / near.w);
		let floor_distance = (-CONTACT_SHADOW_CAMERA_HEIGHT - origin.y) / direction.y;
		let wall_distance = (CONTACT_SHADOW_WALL_Z - origin.z) / direction.z;
		let wall_height = origin.y + direction.y * wall_distance + CONTACT_SHADOW_CAMERA_HEIGHT;
		let distance = if wall_distance > 0.0 && (0.0..=CONTACT_SHADOW_WALL_HEIGHT).contains(&wall_height) {
			wall_distance.min(floor_distance)
		} else {
			floor_distance
		};
		if distance <= 0.0 {
			return 0.0;
		}
		let point = origin + direction * distance;
		let clip = view_projection * maths_rs::Vec4f::new(point.x, point.y, point.z, 1.0);
		clip.z / clip.w
	};
	let mut shadow_map = Texture::new_3d(extent, extent, cascades).expect("shadow map fixture");
	let cells = extent / 8;
	let mut pyramid = vec![[0.0f32, 0.0, 0.0, 1.0]; (cells * cells * cascades) as usize];
	let mut minimum_pyramid = pyramid.clone();
	// Every cascade shares the one view, so each is drawn with the same depths.
	for drawn_layer in first_drawn_layer..cascades {
		for y in 0..extent {
			for x in 0..extent {
				let depth = depth_at(x, y);
				shadow_map
					.write_3d([x, y, drawn_layer], [depth, 0.0, 0.0, 1.0])
					.expect("shadow map fixture");
				let cell = ((drawn_layer * cells + y / 8) * cells + x / 8) as usize;
				pyramid[cell][0] = pyramid[cell][0].max(depth);
				minimum_pyramid[cell][0] = if x % 8 == 0 && y % 8 == 0 {
					depth
				} else {
					minimum_pyramid[cell][0].min(depth)
				};
			}
		}
	}
	(
		shadow_map,
		texture_2d(cells, cells * cascades, &pyramid),
		texture_2d(cells, cells * cascades, &minimum_pyramid),
	)
}

/// Verifies that a project without contact shadows bakes a sun visibility resolve that never reads the trace.
#[test]
fn sun_visibility_without_contact_shadows_reaches_no_trace() {
	use resource_management::asset::handler::implementations::besl::ProgramTransform;

	use crate::rendering::common_shader_generator::CommonShaderGenerator;

	let reaches_trace = |features| {
		let source = crate::rendering::shader_vm_test::with_engine_modules(asset_source!("sun-visibility.besl"));
		let root =
			besl::lex(CommonShaderGenerator::with_features(features).transform(source)).expect("sun visibility should link");
		let main = root.get_main().expect("sun visibility should contain main");
		resource_management::shader::besl::graph::dependency_order(&main)
			.iter()
			.any(|node| node.borrow().get_name() == Some("contact_shadow_trace"))
	};

	assert!(
		reaches_trace(super::VisibilityFeatures::default()),
		"Contact shadows should read the trace."
	);
	assert!(
		!reaches_trace(super::VisibilityFeatures {
			contact_shadows: false,
			..Default::default()
		}),
		"The resolve read the trace without contact shadows. The most likely cause is a trace read in `sun-visibility.besl` outside a block that reads `contact_shadows`."
	);
}

/// The `SunVisibilityGeometry` struct holds the visibility buffer and geometry of the floor scene with the low wall: one
/// identity instance whose only meshlet holds a floor triangle and a wall triangle, and the triangle each pixel sees.
/// The resolve takes each receiver's plane and normal from them.
struct SunVisibilityGeometry {
	meshes: besl::vm::Buffer,
	meshlets: besl::vm::Buffer,
	mesh_levels: besl::vm::Buffer,
	positions: besl::vm::Buffer,
	vertex_indices: besl::vm::Buffer,
	primitive_indices: besl::vm::Buffer,
	triangle_indices: Texture,
	instance_indices: Texture,
}

impl SunVisibilityGeometry {
	/// Builds the floor scene's geometry for `program`. Triangle zero lies on the floor and triangle one on the wall.
	fn new(program: &ExecutableProgram) -> Self {
		let extent = CONTACT_SHADOW_EXTENT;
		let mut meshes = array_buffer(program, slot(program, MESH_DATA), FIXTURE_TABLE_LENGTH);
		meshes
			.write_array_member(0, "model", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX))
			.expect("mesh model matrix");
		let floor_y = -CONTACT_SHADOW_CAMERA_HEIGHT;
		let vertices = [
			[0.0, floor_y, 0.0],
			[1.0, floor_y, 0.0],
			[0.0, floor_y, 1.0],
			[0.0, 0.0, CONTACT_SHADOW_WALL_Z],
			[1.0, 0.0, CONTACT_SHADOW_WALL_Z],
			[0.0, 1.0, CONTACT_SHADOW_WALL_Z],
		];
		let mut positions = array_buffer(program, slot(program, VERTEX_POSITIONS), vertices.len());
		let mut vertex_indices = array_buffer(program, slot(program, VERTEX_INDICES), vertices.len());
		let mut primitive_indices = array_buffer(program, slot(program, PRIMITIVE_INDICES), vertices.len());
		for (index, vertex) in vertices.into_iter().enumerate() {
			positions
				.write_array_element(index, Value::Vec3F(vertex))
				.expect("scene vertex");
			vertex_indices
				.write_array_element(index, Value::U16(index as u16))
				.expect("vertex index");
			primitive_indices
				.write_array_element(index, Value::U8(index as u8))
				.expect("triangle index");
		}
		let mut meshlets = array_buffer(program, slot(program, MESHLETS), 1);
		let meshlet = ShaderMeshletData::new(0, 0, [6, 2], [0.0; 4], [0.0; 4], [0.0, 0.0, 1.0], 2.0);
		write_meshlet(&mut meshlets, 0, meshlet);
		// The instance draws level zero, whose vertices start at the first.
		let mesh_levels = array_buffer(program, slot(program, MESH_LEVELS), 1);
		let mut triangle_indices = Texture::new(extent, extent).expect("triangle index fixture");
		let instance_indices = instance_texture(extent, |_| 0);
		for y in 0..extent {
			for x in 0..extent {
				let wall = contact_shadow_scene_depth([x, y], true) == CONTACT_SHADOW_WALL_Z;
				triangle_indices
					.write_u32([x, y], u32::from(wall))
					.expect("triangle index texel");
			}
		}
		Self {
			meshes,
			meshlets,
			mesh_levels,
			positions,
			vertex_indices,
			primitive_indices,
			triangle_indices,
			instance_indices,
		}
	}

	/// Binds the geometry and the visibility buffer to the resolve's slots.
	fn bind<'a>(&'a mut self, program: &ExecutableProgram, descriptors: &mut DescriptorBindings<'a>) {
		descriptors.bind_buffer(slot(program, MESH_DATA), &mut self.meshes);
		descriptors.bind_buffer(slot(program, MESHLETS), &mut self.meshlets);
		descriptors.bind_buffer(slot(program, MESH_LEVELS), &mut self.mesh_levels);
		descriptors.bind_buffer(slot(program, VERTEX_POSITIONS), &mut self.positions);
		descriptors.bind_buffer(slot(program, VERTEX_INDICES), &mut self.vertex_indices);
		descriptors.bind_buffer(slot(program, PRIMITIVE_INDICES), &mut self.primitive_indices);
		descriptors.bind_image(slot(program, "triangle_index"), &mut self.triangle_indices);
		descriptors.bind_image(slot(program, INSTANCE_INDEX), &mut self.instance_indices);
	}
}

/// Runs the sun visibility resolve at one pixel of the floor scene with the low wall, over a half-resolution
/// contact-shadow trace whose value at each texel is `trace(column, row, depth)`, where `depth` is the texel's linear
/// depth as mip zero of the depth pyramid holds it, and returns the pixel's sun visibility. With `shadow_map`, the
/// scene's own shadow map shadows it; without, an empty map leaves the sun unshadowed. View space is world space.
fn run_sun_visibility(trace: impl Fn(u32, u32, f32) -> f32, shadow_map: bool, pixel: [u32; 2]) -> f32 {
	run_sun_visibility_in_slot(trace, shadow_map, pixel, 0)[0]
}

/// Runs the sun visibility resolve as [`run_sun_visibility`] does, with the test sun in sun slot `sun_slot` and every
/// earlier slot holding the same sun over an empty shadow map and a clear trace. Returns the pixel's texel, sun slot
/// `s` in channel `s`.
fn run_sun_visibility_in_slot(
	trace: impl Fn(u32, u32, f32) -> f32,
	shadow_map: bool,
	pixel: [u32; 2],
	sun_slot: usize,
) -> [f32; 4] {
	let program = asset!("sun-visibility.besl");
	let extent = CONTACT_SHADOW_EXTENT;
	let suns = sun_slot + 1;
	let cascades = super::layout::SHADOW_CASCADE_COUNT * suns;
	let linear_depth = contact_shadow_linear_depth(true);
	let (trace_depth, trace_extent, _) = reduce_nearest_nonzero_depth(&linear_depth, extent, extent);
	let trace = trace_depth
		.iter()
		.enumerate()
		.map(|(index, texel)| {
			let mut channels = [1.0; 4];
			channels[sun_slot] = trace(index as u32 % trace_extent, index as u32 / trace_extent, texel[0]);
			channels
		})
		.collect::<Vec<_>>();
	let device_depth = linear_depth
		.iter()
		.map(|texel| [gtao_fixture_device_depth(texel[0]), 0.0, 0.0, 1.0])
		.collect::<Vec<_>>();
	let cells = SUN_SHADOW_MAP_EXTENT / 8;
	// The blocker search gathers the maximum pyramid through a binding of its own, and the VM binds a texture once, so
	// the pyramid is built twice.
	let mut shadow_cells = if shadow_map {
		sun_shadow_map(suns as u32).1
	} else {
		empty_image(cells, cells * cascades as u32)
	};
	let (mut shadow_map, mut shadow_pyramid, mut shadow_minimum_pyramid) = if shadow_map {
		sun_shadow_map(suns as u32)
	} else {
		(
			Texture::new_3d(SUN_SHADOW_MAP_EXTENT, SUN_SHADOW_MAP_EXTENT, cascades as u32).expect("shadow map fixture"),
			empty_image(cells, cells * cascades as u32),
			empty_image(cells, cells * cascades as u32),
		)
	};
	let mut views = buffer(&program, slot(&program, VIEWS));
	for (member, value) in [
		("view", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX)),
		("inverse_view", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX)),
	] {
		views.write_array_member(0, member, value).expect("camera view");
	}
	let cascade = column_major(sun_cascade_view().view_projection());
	for view_index in 1..=cascades {
		views
			.write_array_member(view_index, "view_projection", Value::Mat4F(cascade))
			.expect("cascade view");
		views
			.write_array_member(view_index, "far", Value::F32(GTAO_FAR))
			.expect("cascade far");
	}
	let mut trace_view = screen_view_buffer(&program, slot(&program, SCREEN_VIEW), trace_extent, trace_extent);
	let screen = screen_view_buffer(&program, slot(&program, SCREEN_VIEW), extent, extent);
	let mut parameters = buffer(&program, slot(&program, "sun_visibility_parameters"));
	let [x, y, z] = [0.0, 1.0, 1.0f32].map(|component| component / 2.0f32.sqrt());
	for sun in 0..suns {
		parameters
			.write_indexed("suns", sun, Value::Vec4F([x, y, z, 0.0]))
			.expect("sun visibility parameters");
	}
	let rays = |member| match screen.read(member).expect("full-resolution rays") {
		Value::Vec2F(rays) => rays,
		value => panic!("Unexpected pixel ray value: {value:?}."),
	};
	let ([mul_x, mul_y], [add_x, add_y]) = (rays("pixel_to_ray_mul"), rays("pixel_to_ray_add"));
	for (member, value) in [
		("pixel_to_ray", Value::Vec4F([mul_x, mul_y, add_x, add_y])),
		("max_distance", Value::F32(CONTACT_SHADOW_TEST_DISTANCE)),
		("sun_count", Value::U32(suns as u32)),
		("contact_shadows", Value::U32(1)),
	] {
		parameters.write(member, value).expect("sun visibility parameters");
	}
	let mut depth = texture_2d(extent, extent, &device_depth);
	let mut trace = texture_2d(trace_extent, trace_extent, &trace);
	let mut trace_depth = texture_2d(trace_extent, trace_extent, &trace_depth);
	let mut output = empty_image(extent, extent);
	let mut geometry = SunVisibilityGeometry::new(&program);
	let mut descriptors = DescriptorBindings::new();
	geometry.bind(&program, &mut descriptors);
	descriptors.bind_buffer(slot(&program, VIEWS), &mut views);
	descriptors.bind_texture(slot(&program, "visibility_depth"), &mut depth);
	descriptors.bind_image(slot(&program, "sun_visibility_output"), &mut output);
	descriptors.bind_texture(slot(&program, "contact_shadow_trace"), &mut trace);
	descriptors.bind_texture(slot(&program, "depth_pyramid"), &mut trace_depth);
	descriptors.bind_buffer(slot(&program, SCREEN_VIEW), &mut trace_view);
	descriptors.bind_buffer(slot(&program, "sun_visibility_parameters"), &mut parameters);
	descriptors.bind_texture(slot(&program, "depth_shadow_map"), &mut shadow_map);
	descriptors.bind_texture(slot(&program, "directional_shadow_depth_pyramid"), &mut shadow_pyramid);
	descriptors.bind_texture(
		slot(&program, "directional_shadow_depth_minimum_pyramid"),
		&mut shadow_minimum_pyramid,
	);
	descriptors.bind_texture(slot(&program, "directional_shadow_depth_cells"), &mut shadow_cells);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&program, descriptors, TILE_WORKGROUP_WIDTH, pixel);
	rgba(&output, pixel)
}

/// Returns the floor's depth at a pixel row of the center column, or zero where that row does not see the floor.
fn contact_shadow_floor_z(row: u32) -> f32 {
	let z = contact_shadow_scene_depth([CONTACT_SHADOW_EXTENT / 2, row], true);
	if z == CONTACT_SHADOW_WALL_Z { 0.0 } else { z }
}

/// Verifies the floor just in front of a low wall is shadowed when the sun shines over the wall toward the camera,
/// that the shadow fades out, lightening with distance from the wall, where the wall lies near the end of the rays
/// reach, and that floor further away than the ray reaches stays lit.
#[test]
fn contact_shadows_darken_the_floor_just_in_front_of_a_low_wall() {
	// The sun is 45 degrees high behind the wall, so the wall's shadow reaches 0.25 units toward the camera. The rays
	// reach 0.3 units, about 0.21 units along the floor, and fade out over their second half, from about 0.106 units.
	let direction_to_light = [0.0, 1.0, 1.0];
	let column = CONTACT_SHADOW_EXTENT / 2;
	let rows_at = |near: f32, far: f32| {
		(0..CONTACT_SHADOW_EXTENT)
			.filter(|&row| (near..far).contains(&contact_shadow_floor_z(row)))
			.collect::<Vec<_>>()
	};
	// Each band of rows, the value every row in it must have, and how the message names that value.
	let bands: [(_, fn(f32) -> bool, _); 3] = [
		(
			rows_at(CONTACT_SHADOW_WALL_Z - 0.09, CONTACT_SHADOW_WALL_Z),
			|value| value == 0.0,
			"shadowed",
		),
		(
			rows_at(CONTACT_SHADOW_WALL_Z - 0.18, CONTACT_SHADOW_WALL_Z - 0.12),
			|value| value > 0.0 && value < 1.0,
			"partly shadowed",
		),
		(rows_at(3.0, CONTACT_SHADOW_WALL_Z - 0.3), |value| value == 1.0, "lit"),
	];
	assert!(bands.iter().all(|(rows, ..)| !rows.is_empty()));

	for (rows, expected, state) in bands {
		for row in rows {
			let value = run_contact_shadows(true, direction_to_light, [column, row]);
			assert!(
				expected(value),
				"Expected floor row {row} at z={} to be {state}, got {value}.",
				contact_shadow_floor_z(row)
			);
		}
	}
}

/// Verifies the sun visibility resolve turns the contact trace's pixel dither into a smooth value on the floor, and
/// keeps a shadowed floor from darkening the top edge of the wall in front of it. The sun's own map is empty, so it
/// shadows nothing.
#[test]
fn sun_visibility_smooths_contact_dither_without_crossing_depth_edges() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	// The floor seven units away lies behind the wall, several rows above it on screen.
	let open_floor_row = (0..CONTACT_SHADOW_EXTENT)
		.find(|&row| (6.9..7.1).contains(&contact_shadow_floor_z(row)))
		.expect("a floor row near seven units");
	let checkerboard = |x: u32, y: u32, _: f32| ((x + y) % 2) as f32;
	let smoothed = [0, 1].map(|step| run_sun_visibility(checkerboard, false, [column + step, open_floor_row]));
	assert!(
		smoothed.iter().all(|value| (0.3..0.7).contains(value)) && (smoothed[0] - smoothed[1]).abs() < 0.3,
		"Expected the filter to smooth a checkerboard of zeros and ones, got {smoothed:?}."
	);

	// The wall's top row borders the floor far behind it. Only the floor is shadowed.
	let wall_top_row = (0..CONTACT_SHADOW_EXTENT)
		.find(|&row| contact_shadow_scene_depth([column, row], true) == CONTACT_SHADOW_WALL_Z)
		.expect("a wall row");
	let shadowed_floor = |_, _, z| if z == CONTACT_SHADOW_WALL_Z { 1.0 } else { 0.0 };
	let wall_edge = run_sun_visibility(shadowed_floor, false, [column, wall_top_row]);
	assert_eq!(wall_edge, 1.0, "The floor behind the wall darkened the wall's top edge.");
}

/// Verifies the sun visibility resolve filters the shadow map: the floor just in front of the low wall, which the
/// sun's rays over the wall cannot reach, is shadowed even where the contact trace is clear, while the floor further
/// from the wall stays lit, and a clear shadow map still leaves the contact shadow to darken the floor.
#[test]
fn sun_visibility_shadows_the_floor_the_wall_hides_from_the_sun() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	let row_at = |near: f32, far: f32| {
		(0..CONTACT_SHADOW_EXTENT)
			.find(|&row| (near..far).contains(&contact_shadow_floor_z(row)))
			.expect("a floor row in the band")
	};
	// The wall shadows the quarter meter of floor before it, z from 3.75 to 4, and the filter reaches two texels, about
	// five centimeters, so rows a decimeter inside either side of the shadow's edge resolve fully.
	let shadowed_row = row_at(CONTACT_SHADOW_WALL_Z - 0.2, CONTACT_SHADOW_WALL_Z - 0.05);
	let lit_row = row_at(CONTACT_SHADOW_WALL_Z - 0.65, CONTACT_SHADOW_WALL_Z - 0.35);
	let clear = |_, _, _| 1.0;

	assert_eq!(run_sun_visibility(clear, true, [column, shadowed_row]), 0.0);
	// The tent's weights sum to one only up to rounding, so a filtered lit receiver may fall short of one by a unit in
	// the last place.
	let lit = run_sun_visibility(clear, true, [column, lit_row]);
	assert!(
		(lit - 1.0).abs() <= 0.000001,
		"Expected the floor away from the wall to be lit, got {lit}."
	);
	assert_eq!(run_sun_visibility(|_, _, _| 0.0, false, [column, lit_row]), 0.0);
}

/// Verifies a sun in a later slot resolves its own cascades, the shadow-map layers and views after the earlier suns',
/// into its own channel, while an earlier sun over an empty map stays lit.
#[test]
fn sun_visibility_resolves_each_sun_from_its_own_cascades() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	let row_at = |near: f32, far: f32| {
		(0..CONTACT_SHADOW_EXTENT)
			.find(|&row| (near..far).contains(&contact_shadow_floor_z(row)))
			.expect("a floor row in the band")
	};
	let shadowed_row = row_at(CONTACT_SHADOW_WALL_Z - 0.2, CONTACT_SHADOW_WALL_Z - 0.05);
	let lit_row = row_at(CONTACT_SHADOW_WALL_Z - 0.65, CONTACT_SHADOW_WALL_Z - 0.35);
	let clear = |_, _, _| 1.0;

	let shadowed = run_sun_visibility_in_slot(clear, true, [column, shadowed_row], 1);
	assert_eq!([shadowed[0], shadowed[1]], [1.0, 0.0]);
	// As in `sun_visibility_shadows_the_floor_the_wall_hides_from_the_sun`, a filtered lit receiver is one up to rounding.
	let lit = run_sun_visibility_in_slot(clear, true, [column, lit_row], 1)[1];
	assert!(
		(lit - 1.0).abs() <= 0.000001,
		"Expected the second sun to light the floor away from the wall, got {lit}."
	);
}

/// Verifies the contact-shadow trace marches each sun's own ray: the floor in front of the low wall is shadowed for a
/// sun behind the wall and lit for one in front of it, each in its own channel.
#[test]
fn contact_shadows_trace_each_sun_into_its_own_channel() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	let row = (0..CONTACT_SHADOW_EXTENT)
		.find(|&row| (CONTACT_SHADOW_WALL_Z - 0.05..CONTACT_SHADOW_WALL_Z).contains(&contact_shadow_floor_z(row)))
		.expect("a floor row just in front of the wall");

	let texel = run_contact_shadows_toward(true, &[[0.0, 1.0, -1.0], [0.0, 1.0, 1.0]], [column, row]);

	assert_eq!([texel[0], texel[1]], [1.0, 0.0]);
}

/// Verifies every visibility pass compiles with the platform shader compiler, past BESL linking.
///
/// Each entry's settings mirror the asset's `.bead` file.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[compio::test]
async fn visibility_assets_lower_to_the_platform_shader_language() {
	use resource_management::shader::ShaderGenerationSettings as Settings;
	use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;
	use utils::Extent;

	let mesh = || Settings::mesh(64, 126, Extent::line(128));
	let tile = || Settings::compute(Extent::square(8));
	// The surfel passes that run one thread per pool slot or scan element.
	let surfel_slots = || Settings::compute(Extent::line(256));
	for (name, source, settings) in [
		(
			"meshlet_task",
			asset_source!("meshlet-task.besl"),
			Settings::task(Extent::line(32), 192),
		),
		("visibility_mesh", asset_source!("visibility-mesh.besl"), mesh()),
		("masked_visibility_mesh", asset_source!("masked-visibility-mesh.besl"), mesh()),
		("shadow_mesh", asset_source!("shadow-mesh.besl"), mesh()),
		("masked_shadow_mesh", asset_source!("masked-shadow-mesh.besl"), mesh()),
		(
			"visibility_fragment",
			asset_source!("visibility-fragment.besl"),
			Settings::fragment(),
		),
		(
			"material_offset",
			asset_source!("material-offset.besl"),
			Settings::compute(Extent::line(256)),
		),
		(
			"pixel_mapping",
			asset_source!("pixel-mapping.besl"),
			Settings::compute(Extent::square(16)),
		),
		(
			"hiz_base",
			asset_source!("hiz-base.besl"),
			Settings::compute(Extent::square(16)),
		),
		(
			"hiz_tail",
			asset_source!("hiz-tail.besl"),
			Settings::compute(Extent::rectangle(16, 8)),
		),
		(
			"gtao_depth_pyramid",
			asset_source!("depth-pyramid.besl"),
			Settings::compute(Extent::square(16)),
		),
		("contact_shadows", asset_source!("contact-shadows.besl"), tile()),
		("sun_visibility", asset_source!("sun-visibility.besl"), tile()),
		("ssgi_trace", asset_source!("ssgi-trace.besl"), tile()),
		("ssgi_temporal", asset_source!("ssgi-temporal.besl"), tile()),
		(
			"light_clusters",
			asset_source!("light-clusters.besl"),
			Settings::compute(Extent::line(super::layout::LIGHT_CLUSTER_MASK_WORDS as u32)),
		),
		(
			"fog_light_tiles",
			asset_source!("fog-light-tiles.besl"),
			Settings::compute(Extent::line(32)),
		),
		("fog_light", asset_source!("fog-light.besl"), tile()),
		(
			"directional_shadow_cascade_fit",
			asset_source!("directional-shadow-cascade-fit.besl"),
			Settings::compute(Extent::line(4)),
		),
		("surfel_count", asset_source!("surfel-count.besl"), surfel_slots()),
		("surfel_scan_groups", asset_source!("surfel-scan-groups.besl"), surfel_slots()),
		(
			"surfel_scan_partials",
			asset_source!("surfel-scan-partials.besl"),
			surfel_slots(),
		),
		("surfel_scan_add", asset_source!("surfel-scan-add.besl"), surfel_slots()),
		("surfel_scatter", asset_source!("surfel-scatter.besl"), surfel_slots()),
		("surfel_validate", asset_source!("surfel-validate.besl"), surfel_slots()),
		(
			"surfel_trace",
			asset_source!("surfel-trace.besl"),
			Settings::compute(Extent::line(64)),
		),
		("ray_hit_sort_scan", asset_source!("ray-hit-sort-scan.besl"), surfel_slots()),
		(
			"ray_hit_sort_scatter",
			asset_source!("ray-hit-sort-scatter.besl"),
			surfel_slots(),
		),
		("surfel_integrate", asset_source!("surfel-integrate.besl"), surfel_slots()),
		(
			"surfel_allocate_hits",
			asset_source!("surfel-allocate-hits.besl"),
			Settings::compute(Extent::line(64)),
		),
		("surfel_resolve", asset_source!("surfel-resolve.besl"), tile()),
		(
			"surfel_allocate",
			asset_source!("surfel-allocate.besl"),
			Settings::compute(Extent::line(64)),
		),
		("surfel_spatial", asset_source!("surfel-spatial.besl"), tile()),
		(
			"probe_schedule",
			asset_source!("probe-schedule.besl"),
			Settings::compute(Extent::line(64)),
		),
		(
			"probe_select",
			asset_source!("probe-select.besl"),
			Settings::compute(Extent::line(1)),
		),
		(
			"probe_gather",
			asset_source!("probe-gather.besl"),
			Settings::compute(Extent::line(64)),
		),
		(
			"probe_trace",
			asset_source!("probe-trace.besl"),
			Settings::compute(Extent::line(64)),
		),
		("probe_update", asset_source!("probe-update.besl"), surfel_slots()),
		("probe_resolve", asset_source!("probe-resolve.besl"), tile()),
	] {
		let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
		// Baking gives every standalone shader the common helpers, such as `source_over`.
		root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
		let root = besl::lex(root).unwrap_or_else(|error| panic!("{name} should link: {error:?}"));
		PlatformShaderCompiler::new()
			.generate(&settings.name(name.to_string()), &root)
			.await
			.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error:?}"));
	}
}

/// Verifies the surfel and probe shade passes and the default hit lower to the running platform's shader language with
/// each layer in its own set, as the bake stacks them, so the closest hit's sets are the first of each shade pass's.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[compio::test]
async fn ray_tracing_shaders_lower_to_the_platform_shader_language() {
	use resource_management::shader::ShaderGenerationSettings as Settings;
	use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

	for (name, asset, settings) in [
		("surfel_shade", "visibility/surfel-shade.besl", Settings::ray_generation()),
		("probe_shade", "visibility/probe-shade.besl", Settings::ray_generation()),
		("default_hit", "visibility/default-hit.besl", Settings::closest_hit()),
	] {
		let root = besl::lex(crate::rendering::shader_vm_test::with_engine_modules(
			crate::rendering::shader_vm_test::asset_source(asset),
		))
		.unwrap_or_else(|error| panic!("{name} should link: {error:?}"));
		PlatformShaderCompiler::new()
			.generate(&settings.name(name.to_string()), &root)
			.await
			.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error:?}"));
	}
}

/// Verifies an open floor never shadows itself, even under a low sun whose rays barely leave it.
#[test]
fn contact_shadows_leave_an_open_floor_lit() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	for direction_to_light in [[0.0, 1.0, 0.0], [0.0, 0.1, 1.0], [0.0, 0.1, -1.0], [1.0, 0.1, 0.0]] {
		for row in (CONTACT_SHADOW_EXTENT / 2 + 4..CONTACT_SHADOW_EXTENT).step_by(5) {
			let value = run_contact_shadows(false, direction_to_light, [column, row]);
			assert_eq!(
				value, 1.0,
				"Expected open floor row {row} to be lit toward {direction_to_light:?}."
			);
		}
	}
}

/// Builds the light record the pipeline uploads for `light` at `position`, facing `direction`.
fn uploaded_light(
	light: crate::rendering::lights::Lights,
	position: math::Point,
	direction: math::UnitVector,
) -> super::shader_data::LightData {
	let transform = crate::gameplay::Transform::from_position(position).rotation(math::orientation_from_direction(direction));
	super::scene::light_data(&light, &transform, super::shadow_selection::LightShadow::None, None)
}

/// Returns a white local light of 100 cd, which reaches 10 m at an exposure of 1/1024.
fn light_cluster_fixture_light(cone: bool) -> crate::rendering::lights::Lights {
	use crate::rendering::lights::{ConeLight, LightColor, Lights, PhotometricIntensity, PointLight};

	let color = LightColor::LinearSrgb(maths_rs::Vec3f::new(1.0, 1.0, 1.0));
	let intensity = PhotometricIntensity::LuminousIntensity {
		candela: 100.0,
		reference_distance_m: 1.0,
	};
	if cone {
		Lights::Cone(
			ConeLight::new(
				color,
				intensity,
				math::Degrees::new(15.0).to_radians(),
				math::Degrees::new(30.0).to_radians(),
			)
			.expect("physical cone light"),
		)
	} else {
		Lights::Point(PointLight::new(color, intensity).expect("physical point light"))
	}
}

/// Runs the light-cluster pass for one cluster, seen by [`fixture_camera`] at an aspect ratio of one, and returns its
/// first two mask words.
fn run_light_clusters(lights: &[super::shader_data::LightData], exposure: f32, cluster: u32) -> [u32; 2] {
	let program = asset!("light-clusters.besl");
	let parameters = super::shader_data::LightClusterParameters::from(fixture_camera(1.0));
	let mut cluster_parameters = buffer(&program, slot(&program, "light_cluster_parameters"));
	for (field, value) in [
		("view", Value::Mat4x3F(bytemuck::cast(parameters.view))),
		("edge_slopes", Value::Vec2F(parameters.edge_slopes)),
		("near", Value::F32(parameters.near)),
		("depth_slice_scale", Value::F32(parameters.depth_slice_scale)),
	] {
		cluster_parameters.write(field, value).expect("light cluster parameter");
	}
	let mut lighting = buffer(&program, slot(&program, "lighting_data"));
	lighting
		.write("light_count", Value::U32(lights.len() as u32))
		.expect("light count");
	lighting.write("exposure", Value::F32(exposure)).expect("exposure");
	for (index, light) in lights.iter().enumerate() {
		for (field, value) in [
			("position", Value::Vec3F(light.position.into())),
			("color", Value::Vec3F(light.color.into())),
			("direction", Value::Vec3F(light.direction.into())),
			("cone_cosines", Value::Vec2F(light.cone_cosines)),
			("type", Value::U32(light.light_type)),
			("reach", Value::F32(light.reach)),
		] {
			lighting
				.write_indexed_field("lights", index, field, value)
				.expect("light field");
		}
	}
	let mut masks = buffer(&program, slot(&program, "light_cluster_masks"));
	let configs: [ExecutionConfig; 32] =
		std::array::from_fn(|lane| lane_config(lane as u32).with_threadgroup_position(cluster));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, "lighting_data"), &mut lighting);
	descriptors.bind_buffer(slot(&program, "light_cluster_parameters"), &mut cluster_parameters);
	descriptors.bind_buffer(slot(&program, "light_cluster_masks"), &mut masks);
	run_workgroup(&program, descriptors, &configs);
	let base = cluster as usize * super::layout::LIGHT_CLUSTER_MASK_WORDS;
	[read_u32(&masks, base), read_u32(&masks, base + 1)]
}

/// Returns the index of the cluster at a column, row, and depth slice.
fn light_cluster_index(column: u32, row: u32, slice: u32) -> u32 {
	use super::layout::{LIGHT_CLUSTER_COLUMNS, LIGHT_CLUSTER_ROWS};

	(slice * LIGHT_CLUSTER_ROWS + row) * LIGHT_CLUSTER_COLUMNS + column
}

/// Verifies each cluster holds exactly the lights whose reach touches it, with one bit per light-table entry.
#[test]
fn light_clusters_hold_the_lights_whose_reach_touches_them() {
	use math::{Point, UnitVector};

	use crate::rendering::lights::{DirectionalLight, LightColor, Lights, PhotometricIntensity};

	let forward = UnitVector::z_axis();
	let sun = Lights::Direction(
		DirectionalLight::new(
			LightColor::LinearSrgb(maths_rs::Vec3f::new(1.0, 1.0, 1.0)),
			PhotometricIntensity::Illuminance {
				lux: 100_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light"),
	);
	// In front of the camera, 20 m away.
	let in_front = uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, 20.0), forward);
	let mut lights = vec![
		in_front,
		// Behind the camera, 20 m away.
		uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, -20.0), forward),
		uploaded_light(sun, Point::origin(), -UnitVector::y_axis()),
		// Cones 5 m behind the camera, facing away from and toward the view.
		uploaded_light(light_cluster_fixture_light(true), Point::new(0.0, 0.0, -5.0), -forward),
		uploaded_light(light_cluster_fixture_light(true), Point::new(0.0, 0.0, -5.0), forward),
	];
	// Lights without reach fill the rest of the first mask word, so the last light, in front again, lands in the second
	// word.
	lights.resize(40, super::shader_data::LightData::default());
	lights.push(in_front);

	// Slice 18 spans about 17 m to 21 m of view depth, and slice 8 spans 1 m to 1.33 m. Column 8 and row 4 sit just
	// right of and below the center of the image.
	let far_cluster = light_cluster_index(8, 4, 18);
	let near_cluster = light_cluster_index(8, 4, 8);
	// At this exposure each light reaches 10 m.
	let dim = 1.0 / 1024.0;

	assert_eq!(run_light_clusters(&lights, dim, far_cluster), [0b101, 1 << 8]);
	assert_eq!(run_light_clusters(&lights, dim, near_cluster), [0b10100, 0]);
	// At an exposure of one each light reaches 320 m, so only the cone facing away stays out of the view.
	assert_eq!(run_light_clusters(&lights, 1.0, far_cluster), [0b10111, 1 << 8]);
}

/* Directional shadow cascade fit */

const RECEIVER_BOUNDS: &str = "receiver_bounds";
const RECEIVER_FIT: &str = "receiver_fit";
const CASCADE_SIZE_STEPS: &str = "cascade_size_steps";
const DRAWN_CASCADE_VIEWS: &str = "drawn_views";
/// Pixels one depth pyramid workgroup covers on each side: sixteen threads of two pixels.
const RECEIVER_BOUNDS_TILE: u32 = 32;
const RECEIVER_FIT_EXTENT: u32 = 64;
const RECEIVER_FIT_WALL_Z: f32 = 20.0;
const RECEIVER_FIT_WALL_HEIGHT: f32 = 4.0;

/// A camera 1.7 m above a floor, looking a little down toward a 4 m wall 20 m ahead, with sky above the wall, under two
/// diagonal suns.
struct ReceiverFitScene {
	/// Each sun's frustum-fitted cascades, by sun slot.
	suns: Vec<[crate::rendering::csm::CascadeFrame; super::layout::SHADOW_CASCADE_COUNT]>,
	shader_data: super::render_pass::ReceiverFitShaderData,
	/// Each pixel's reversed device depth, zero for the sky.
	device_depth: Vec<[f32; 4]>,
	/// Each receiver's cascade and world-space position.
	receivers: Vec<(usize, [f32; 3])>,
}

fn receiver_fit_scene() -> ReceiverFitScene {
	use crate::rendering::{Sink, View, csm};

	let camera = View::new_perspective(
		math::Degrees::new(75.0),
		1.0,
		0.1,
		1000.0,
		math::Point::new(0.0, 1.7, 0.0),
		math::Vector::new(0.0, -0.3, 1.0).normalized().expect("camera direction"),
	);
	let suns = [math::Vector::new(0.5, -1.0, 0.3), math::Vector::new(-0.4, -1.0, -0.6)]
		.map(|direction| {
			csm::make_cascade_frames(
				camera,
				direction.normalized().expect("sun direction"),
				super::layout::SHADOW_CASCADE_COUNT,
				super::layout::DEFAULT_SHADOW_MAP_RESOLUTION,
				csm::CascadeSplits::default(),
			)
			.collect::<smallvec::SmallVec<[_; 4]>>()
			.into_inner()
			.expect("four cascades")
		})
		.to_vec();
	let cascades = &suns[0];
	let extent = utils::Extent::square(RECEIVER_FIT_EXTENT);
	let screen = super::render_pass::screen_view_data(&Sink::new(camera, extent, 0), extent);
	// Every cascade draws this frame.
	let every_cascade = (1 << (super::layout::SHADOW_CASCADE_COUNT * suns.len())) - 1;
	let shader_data = super::render_pass::receiver_fit_shader_data(
		screen,
		camera,
		&suns,
		super::layout::DEFAULT_SHADOW_MAP_RESOLUTION,
		every_cascade,
		0,
	);

	let camera_to_world = math::inverse(camera.view());
	let position = camera_to_world * maths_rs::Vec4f::new(0.0, 0.0, 0.0, 1.0);
	let mut device_depth = Vec::new();
	let mut receivers = Vec::new();
	for y in 0..RECEIVER_FIT_EXTENT {
		for x in 0..RECEIVER_FIT_EXTENT {
			let ray_x = x as f32 * screen.pixel_to_ray_mul[0] + screen.pixel_to_ray_add[0];
			let ray_y = y as f32 * screen.pixel_to_ray_mul[1] + screen.pixel_to_ray_add[1];
			// One unit of view-space depth along the pixel's ray, in world space.
			let step = camera_to_world * maths_rs::Vec4f::new(ray_x, ray_y, 1.0, 0.0);
			let floor = (step.y < 0.0).then(|| -position.y / step.y);
			let wall = (step.z > 0.0)
				.then(|| (RECEIVER_FIT_WALL_Z - position.z) / step.z)
				.filter(|distance| (0.0..=RECEIVER_FIT_WALL_HEIGHT).contains(&(position.y + distance * step.y)));
			let distance = [floor, wall].into_iter().flatten().reduce(f32::min);
			let depth = distance.map_or(0.0, |distance| {
				screen.depth_unproject_numerator / distance - screen.depth_unproject_denominator_offset
			});
			device_depth.push([depth, 0.0, 0.0, 1.0]);
			if let Some(distance) = distance
				&& let Some(cascade) = cascades.iter().position(|frame| distance < frame.slice_far)
			{
				let point = position + step * distance;
				receivers.push((cascade, [point.x, point.y, point.z]));
			}
		}
	}
	ReceiverFitScene {
		suns,
		shader_data,
		device_depth,
		receivers,
	}
}

fn receiver_fit_buffer(
	program: &ExecutableProgram,
	slot: ResourceSlot,
	data: &super::render_pass::ReceiverFitShaderData,
) -> besl::vm::Buffer {
	let mut fit = buffer(program, slot);
	for (index, row) in data.view_to_cascade_rows.iter().enumerate() {
		fit.write_indexed("view_to_cascade_rows", index, Value::Vec4F(*row))
			.expect("receiver fit rows");
	}
	for (index, frame) in data.cascade_frames.iter().enumerate() {
		fit.write_indexed("cascade_frames", index, Value::Vec4F(*frame))
			.expect("receiver fit frames");
	}
	for (member, value) in [
		("split_far", data.split_far),
		("pixel_to_ray", data.pixel_to_ray),
		("depth_unproject", data.depth_unproject),
		("fit_constants", data.fit_constants),
	] {
		fit.write(member, Value::Vec4F(value)).expect("receiver fit constants");
	}
	fit.write("cascade_masks", Value::Vec4U(data.cascade_masks))
		.expect("receiver fit cascade masks");
	fit
}

/// Runs the depth pyramid pass with the cascade fit enabled over the whole scene and returns the receiver bounds it
/// found.
fn run_receiver_bounds(scene: &ReceiverFitScene) -> besl::vm::Buffer {
	let program = asset!("depth-pyramid.besl");
	let half_extent = RECEIVER_FIT_EXTENT / 2;
	let mut depth = texture_2d(RECEIVER_FIT_EXTENT, RECEIVER_FIT_EXTENT, &scene.device_depth);
	let mut view = gtao_view_data(&program, half_extent, half_extent);
	let mut reduced = [
		empty_image(half_extent, half_extent),
		empty_image(half_extent / 2, half_extent / 2),
		empty_image(half_extent / 4, half_extent / 4),
	];
	let mut fit = receiver_fit_buffer(&program, slot(&program, RECEIVER_FIT), &scene.shader_data);
	let mut bounds = buffer(&program, slot(&program, RECEIVER_BOUNDS));
	let mut push_constant = push_constant_buffer(&program);
	push_constant
		.write("fitted_suns", Value::U32(scene.suns.len() as u32))
		.expect("depth pyramid push constant");
	let tiles = RECEIVER_FIT_EXTENT / RECEIVER_BOUNDS_TILE;
	for tile in 0..tiles * tiles {
		let base = [
			tile % tiles * DEPTH_PYRAMID_WORKGROUP_WIDTH,
			tile / tiles * DEPTH_PYRAMID_WORKGROUP_HEIGHT,
		];
		let [reduced_1, reduced_2, reduced_3] = &mut reduced;
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(&program, GTAO_VIEW), &mut view);
		descriptors.bind_texture(slot(&program, "source_depth"), &mut depth);
		descriptors.bind_image(slot(&program, "reduced_depth_1"), reduced_1);
		descriptors.bind_image(slot(&program, "reduced_depth_2"), reduced_2);
		descriptors.bind_image(slot(&program, "reduced_depth_3"), reduced_3);
		descriptors.bind_buffer(slot(&program, RECEIVER_BOUNDS), &mut bounds);
		descriptors.bind_buffer(slot(&program, RECEIVER_FIT), &mut fit);
		descriptors.bind_push_constant(&mut push_constant);
		run_workgroup_containing::<DEPTH_PYRAMID_WORKGROUP_SIZE>(&program, descriptors, DEPTH_PYRAMID_WORKGROUP_WIDTH, base);
	}
	bounds
}

/// Runs the cascade fit on `bounds` over views holding every sun's frustum-fitted cascades after the camera, with the
/// cascade masks of `shader_data`, and returns the views it wrote. `drawn_views` holds each cascade's last drawn view
/// across runs.
fn run_cascade_fit(
	program: &ExecutableProgram,
	scene: &ReceiverFitScene,
	shader_data: &super::render_pass::ReceiverFitShaderData,
	bounds: &mut besl::vm::Buffer,
	size_steps: &mut besl::vm::Buffer,
	drawn_views: &mut besl::vm::Buffer,
) -> besl::vm::Buffer {
	let mut views = buffer(program, slot(program, VIEWS));
	for (cascade, frame) in scene.suns.iter().flatten().enumerate() {
		let view = frame.view.view();
		for (field, value) in [
			("view", Value::Mat4x3F(bytemuck::cast(ghi::pod::Mat4x3f::from(view)))),
			("view_projection", Value::Mat4F(column_major(frame.view.view_projection()))),
			(
				"inverse_view",
				Value::Mat4x3F(bytemuck::cast(ghi::pod::Mat4x3f::from(math::inverse(view)))),
			),
			("far", Value::F32(frame.slice_far)),
		] {
			views.write_array_member(1 + cascade, field, value).expect("cascade view");
		}
	}
	let mut fit = receiver_fit_buffer(program, slot(program, RECEIVER_FIT), shader_data);
	// One workgroup fits each sun, one lane per cascade.
	for sun in 0..scene.suns.len() as u32 {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(program, VIEWS), &mut views);
		descriptors.bind_buffer(slot(program, RECEIVER_BOUNDS), bounds);
		descriptors.bind_buffer(slot(program, RECEIVER_FIT), &mut fit);
		descriptors.bind_buffer(slot(program, CASCADE_SIZE_STEPS), size_steps);
		descriptors.bind_buffer(slot(program, DRAWN_CASCADE_VIEWS), drawn_views);
		run_workgroup_containing::<4>(program, descriptors, 4, [4 * sun, 0]);
	}
	views
}

fn read_matrix(views: &besl::vm::Buffer, view_index: usize, field: &str) -> Vec<f32> {
	match views.read_array_member(view_index, field).expect("fitted view") {
		Value::Mat4F(matrix) => matrix.to_vec(),
		Value::Mat4x3F(matrix) => matrix.to_vec(),
		value => panic!("Unexpected view matrix value: {value:?}."),
	}
}

/// Applies a column-major matrix with `rows` rows to a point.
fn transform_point(matrix: &[f32], rows: usize, point: [f32; 3]) -> Vec<f32> {
	(0..rows)
		.map(|row| (0..3).map(|column| matrix[column * rows + row] * point[column]).sum::<f32>() + matrix[3 * rows + row])
		.collect()
}

/// Returns the half width, in meters, of the square an orthographic view-projection covers.
fn fitted_half_extent(view_projection: &[f32]) -> f32 {
	1.0 / (view_projection[0].powi(2) + view_projection[4].powi(2) + view_projection[8].powi(2)).sqrt()
}

/// Verifies the GPU shrinks each cascade to the surfaces its camera sees, keeps them inside the edge margin, keeps the
/// texel grid fixed in the world, keeps its view matrices consistent, and leaves a cascade without receivers as the CPU
/// fitted it.
#[test]
fn cascades_fit_the_receivers_the_camera_sees_in_the_besl_vm() {
	use crate::rendering::csm::{CASTER_REACH, EDGE_TEXELS};

	let scene = receiver_fit_scene();
	let mut bounds = run_receiver_bounds(&scene);
	let program = asset!("directional-shadow-cascade-fit.besl");
	let mut size_steps = buffer(&program, slot(&program, CASCADE_SIZE_STEPS));
	let mut drawn_views = buffer(&program, slot(&program, DRAWN_CASCADE_VIEWS));
	let views = run_cascade_fit(
		&program,
		&scene,
		&scene.shader_data,
		&mut bounds,
		&mut size_steps,
		&mut drawn_views,
	);
	let resolution = super::layout::DEFAULT_SHADOW_MAP_RESOLUTION as f32;

	// The fit resets the bounds it read, so the next frame's depth pyramid pass merges into empty boxes.
	for bound in 0..24 * scene.suns.len() {
		assert_eq!(
			bounds.read_array_element(bound).expect("receiver bound"),
			Value::U32(0),
			"Bound {bound} was not reset after the fit. The most likely cause is that the fit skipped the store."
		);
	}

	// Every sun fits its own cascades, which follow the earlier suns' in the views; a pixel's cascade depends only on
	// its distance from the camera, so every sun's cascades hold the same receivers.
	for (fitted_box, frame) in scene.suns.iter().flatten().enumerate() {
		let cascade = fitted_box % super::layout::SHADOW_CASCADE_COUNT;
		let view_projection = read_matrix(&views, 1 + fitted_box, "view_projection");
		let half_extent = fitted_half_extent(&view_projection);
		let receivers = scene
			.receivers
			.iter()
			.filter(|(receiver_cascade, _)| *receiver_cascade == cascade)
			.map(|(_, point)| *point)
			.collect::<Vec<_>>();
		if receivers.is_empty() {
			assert_eq!(
				view_projection,
				column_major(frame.view.view_projection()),
				"Fitted cascade {fitted_box} has no receivers but changed. The most likely cause is that the fit rewrote an empty cascade."
			);
			continue;
		}
		assert!(
			half_extent <= frame.half_extent * 1.0001,
			"Fitted cascade {fitted_box} grew from {} to {half_extent} meters. The most likely cause is that the fit is not limited by the frustum fit.",
			frame.half_extent
		);

		// Receivers stay inside the edge margin, less the texel the grid snap may take, and inside the depth range past
		// the caster reach.
		let depth_range = 1.0 / view_projection[10].abs();
		let edge = 1.0 - 2.0 * (EDGE_TEXELS - 1.0) / resolution;
		for point in &receivers {
			let ndc = transform_point(&view_projection, 4, *point);
			assert!(
				ndc[0].abs() <= edge && ndc[1].abs() <= edge,
				"Fitted cascade {fitted_box} receiver {point:?} lies at {ndc:?}, inside the edge margin. The most likely cause is that the receiver bounds or the fit's padding are wrong."
			);
			assert!(
				(-0.0001..=1.0001 - CASTER_REACH / depth_range).contains(&ndc[2]),
				"Fitted cascade {fitted_box} receiver {point:?} lies at depth {} of 0..{}. The most likely cause is that the fit's depth range misses its receivers or its caster reach.",
				ndc[2],
				1.0 - CASTER_REACH / depth_range
			);
		}

		// The world origin lands on a texel corner.
		let origin = transform_point(&view_projection, 4, [0.0, 0.0, 0.0]);
		for coordinate in [origin[0], origin[1]] {
			let texel = (coordinate * 0.5 + 0.5) * resolution;
			assert!(
				(texel - texel.round()).abs() < 0.02,
				"Fitted cascade {fitted_box} puts the world origin at texel {texel}. The most likely cause is that the fit does not snap to the world-fixed texel grid."
			);
		}

		// The light's position is the center of the view's light-facing side, and the view inverts the inverse view.
		let inverse_view = read_matrix(&views, 1 + fitted_box, "inverse_view");
		let light_position = [inverse_view[9], inverse_view[10], inverse_view[11]];
		let light_ndc = transform_point(&view_projection, 4, light_position);
		assert!(
			light_ndc[0].abs() < 0.0001 && light_ndc[1].abs() < 0.0001 && (light_ndc[2] - 1.0).abs() < 0.0001,
			"Fitted cascade {fitted_box} light position maps to {light_ndc:?}. The most likely cause is that the inverse view did not move with the view-projection."
		);
		let view = read_matrix(&views, 1 + fitted_box, "view");
		let light_view_position = transform_point(&view, 3, light_position);
		assert!(
			light_view_position.iter().all(|coordinate| coordinate.abs() < 0.001),
			"Fitted cascade {fitted_box} view maps its own light position to {light_view_position:?}. The most likely cause is that the view did not move with the inverse view."
		);
	}

	// The camera sees the floor from about a meter away, so the first cascade drops the empty space in front of it.
	let first = fitted_half_extent(&read_matrix(&views, 1, "view_projection"));
	assert!(
		first < scene.suns[0][0].half_extent * 0.8,
		"The first cascade only shrank from {} to {first} meters. The most likely cause is that the receiver bounds span the whole frustum slice.",
		scene.suns[0][0].half_extent
	);
}

/// Verifies a cascade that doesn't draw keeps the view it was last drawn with instead of this frame's fit, and that a
/// cascade that holds no drawn map gets a view that no receiver falls in, while both drop their receiver bounds.
#[test]
fn cascades_that_do_not_draw_keep_their_drawn_view_in_the_besl_vm() {
	let scene = receiver_fit_scene();
	let program = asset!("directional-shadow-cascade-fit.besl");
	let mut size_steps = buffer(&program, slot(&program, CASCADE_SIZE_STEPS));
	let mut drawn_views = buffer(&program, slot(&program, DRAWN_CASCADE_VIEWS));

	// The first frame draws every cascade from the receivers the camera sees.
	let drawn = run_cascade_fit(
		&program,
		&scene,
		&scene.shader_data,
		&mut run_receiver_bounds(&scene),
		&mut size_steps,
		&mut drawn_views,
	);
	// The second draws only the first sun's first cascade, from a box that fits it differently. The first sun's second
	// cascade holds its map from the first frame, and the third holds none.
	let masks = super::render_pass::ReceiverFitShaderData {
		cascade_masks: [0b0001, 0b0011, 0, 0],
		..scene.shader_data
	};
	let mut bounds = box_bounds(&program, 0.25);
	for first in [6, 12] {
		bounds.write_array_element(first, Value::U32(1)).expect("receiver bounds");
	}
	let views = run_cascade_fit(&program, &scene, &masks, &mut bounds, &mut size_steps, &mut drawn_views);

	let view_projection = |views: &besl::vm::Buffer, view| read_matrix(views, view, "view_projection");
	assert_ne!(
		view_projection(&views, 1),
		view_projection(&drawn, 1),
		"The cascade that draws should fit this frame's receivers."
	);
	for field in ["view", "view_projection", "inverse_view"] {
		assert_eq!(
			read_matrix(&views, 2, field),
			read_matrix(&drawn, 2, field),
			"The cascade that doesn't draw changed its {field}. The most likely cause is that the fit did not restore its drawn view."
		);
	}
	let outside = transform_point(&view_projection(&views, 3), 4, [1.0, 2.0, 3.0]);
	assert_eq!(
		outside,
		[2.0, 0.0, 0.0, 1.0],
		"A cascade without a drawn map should put every receiver outside it."
	);
	for bound in 6..18 {
		assert_eq!(
			bounds.read_array_element(bound).expect("receiver bound"),
			Value::U32(0),
			"Bound {bound} of a cascade that doesn't draw was not dropped."
		);
	}
}

/// Returns bounds holding one box, centered in the frustum fit, `half_size` normalized device units wide in x and y.
fn box_bounds(program: &ExecutableProgram, half_size: f32) -> besl::vm::Buffer {
	const STEPS_PER_UNIT: f32 = 4_194_304.0;
	const OFFSET: f32 = 8_388_608.0;
	let lower = |coordinate: f32| 16_777_216 - ((coordinate * STEPS_PER_UNIT).floor() + OFFSET) as u32;
	let upper = |coordinate: f32| ((coordinate * STEPS_PER_UNIT).ceil() + OFFSET) as u32;
	let mut bounds = buffer(program, slot(program, RECEIVER_BOUNDS));
	for (index, code) in [
		lower(-half_size),
		lower(-half_size),
		lower(0.4),
		upper(half_size),
		upper(half_size),
		upper(0.5),
	]
	.into_iter()
	.enumerate()
	{
		bounds.write_array_element(index, Value::U32(code)).expect("receiver bounds");
	}
	bounds
}

/// Verifies a cascade grows as soon as its receivers need it but shrinks only once they fit two size steps smaller, so
/// receivers hovering at a step do not switch its size every frame.
#[test]
fn cascade_fit_shrinks_only_by_two_size_steps_in_the_besl_vm() {
	let scene = receiver_fit_scene();
	let program = asset!("directional-shadow-cascade-fit.besl");
	let mut size_steps = buffer(&program, slot(&program, CASCADE_SIZE_STEPS));
	let mut drawn_views = buffer(&program, slot(&program, DRAWN_CASCADE_VIEWS));
	let mut fit = |half_size: f32| {
		let mut bounds = box_bounds(&program, half_size);
		let views = run_cascade_fit(
			&program,
			&scene,
			&scene.shader_data,
			&mut bounds,
			&mut size_steps,
			&mut drawn_views,
		);
		fitted_half_extent(&read_matrix(&views, 1, "view_projection"))
	};

	let fitted = fit(0.5);
	assert!(
		fitted < scene.suns[0][0].half_extent,
		"The box should shrink the first cascade."
	);
	assert_eq!(fit(0.5 * 0.95), fitted, "A box one step smaller should keep the size.");
	let shrunk = fit(0.5 * 0.75);
	assert!(
		shrunk < fitted * 0.85,
		"A box three steps smaller should shrink the cascade, found {shrunk} of {fitted}."
	);
	assert_eq!(fit(0.5), fitted, "The original box should grow the cascade back at once.");
}

/// Camera rotation must retain history for a static floor even when its view-space normals differ substantially.
#[test]
fn ssgi_temporal_retains_static_surface_history_after_camera_rotation() {
	let extent = SSGI_TEMPORAL_EXTENT;
	let (sine, cosine) = 60.0f32.to_radians().sin_cos();
	let mut rotation = maths_rs::Mat4f::identity();
	rotation[0] = cosine;
	rotation[1] = -sine;
	rotation[4] = sine;
	rotation[5] = cosine;
	let texel_count = (extent * extent) as usize;
	let mut fixture = SsgiTemporalFixture::uniform([0.1, 0.1, 0.1, 0.5], [2.0, 2.0, 2.0, 0.5], true);
	fixture.current_to_previous_view = rotation;
	fixture.normals.fill(SSGI_FLOOR_NORMAL);
	let normal_length = sine.abs() + cosine.abs();
	fixture
		.previous_normals
		.fill([-sine / normal_length, cosine / normal_length, 0.0, 0.0]);
	// Intersect both cameras' rays with the same floor, one unit below the camera.
	for index in 0..texel_count {
		let ray = ssgi_ray_at(index as u32 % extent, index as u32 / extent, extent);
		let previous_y = -sine * ray[0] + cosine * ray[1];
		fixture.depth[index][0] = if ray[1] < 0.0 { -1.0 / ray[1] } else { 0.0 };
		fixture.previous_depth[index][0] = if previous_y < 0.0 { -1.0 / previous_y } else { 0.0 };
	}
	assert_rgba_close(fixture.run([4, 6]), [1.81, 1.81, 1.81, 0.5], 0.00001);
}

/// A nearest surface at the last row or column of an odd source must reach the reduced image.
#[test]
fn gtao_depth_pyramid_includes_last_row_and_column_at_odd_extents() {
	let program = asset!("depth-pyramid.besl");
	for edge in [0, 1] {
		let texels: Vec<_> = (0..81)
			.map(|index| {
				let coordinate = [index % 9, index / 9];
				[if coordinate[edge] == 8 { 0.9 } else { 0.1 }, 0.0, 0.0, 1.0]
			})
			.collect();
		let mut source = texture_2d(9, 9, &texels);
		let reduced = run_gtao_depth_pyramid(&program, &mut source, 9, 9);
		assert_rgba_close(
			rgba(&reduced[0], [3, 3]),
			[gtao_fixture_linear_depth(0.9), 0.0, 0.0, 1.0],
			0.00001,
		);
	}
}

/// A pixel's hemisphere samples over 64 frames, from the trace's blue noise and its per-frame offset, must cover every
/// quadrant of the sample square, so its rays take distinct azimuth and elevation combinations.
#[test]
fn ssgi_animated_blue_noise_covers_two_dimensional_hemisphere_samples() {
	use crate::rendering::lookup_textures::{BLUE_NOISE, BLUE_NOISE_SIDE, blue_noise_offset};

	// The thread index carries the noise value the trace computes for one pixel and frame, in steps of 2^-20.
	const VALUE_STEPS: f32 = 1048576.0;
	let source = asset_source!("ssgi-trace.besl").split("main: fn").next().unwrap();
	let program = compile(asset_program(&format!(
		"{source}
probe_output: descriptor<{{ type: StorageImage<rgba16f>, binding: 0, access: write }}>;
main: fn (input: StageInput) -> void {{
	let sample: vec2f = hilbert_square(f32(input.thread_idx) / {VALUE_STEPS:?});
	write(probe_output, input.thread_id, vec4f(sample.x, sample.y, 0.0, 1.0));
}}
"
	)));
	let mut output = empty_image(32, 32);
	for pixel in [[16, 16], [16, 22], [8, 24]] {
		let mut quadrant_counts = [0; 4];
		let mut moments = [0.0; 3];
		let texel = (pixel[1] % BLUE_NOISE_SIDE * BLUE_NOISE_SIDE + pixel[0] % BLUE_NOISE_SIDE) as usize;
		for frame in 0..64 {
			// The trace's rescaled direction channel plus this frame's offset, modulo one.
			let value = ((BLUE_NOISE[texel * 2] as f32 + 0.5) / 256.0 + blue_noise_offset(frame)[0]).fract();
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_image(slot(&program, "probe_output"), &mut output);
			program
				.run_main_with_config(
					&mut descriptors,
					&lane_config((value * VALUE_STEPS) as u32).with_thread_id(pixel),
				)
				.unwrap();
			drop(descriptors);
			let sample = rgba(&output, pixel);
			quadrant_counts[(sample[0] >= 0.5) as usize + 2 * (sample[1] >= 0.5) as usize] += 1;
			moments[0] += sample[0] / 64.0;
			moments[1] += sample[1] / 64.0;
			moments[2] += sample[0] * sample[1] / 64.0;
		}
		assert!(
			quadrant_counts.iter().all(|count| *count >= 8),
			"{pixel:?}: {quadrant_counts:?}"
		);
		assert!(
			(moments[0] - 0.5).abs() < 0.1 && (moments[1] - 0.5).abs() < 0.1,
			"{moments:?}"
		);
		assert!((moments[2] - 0.25).abs() < 0.06, "{moments:?}");
	}
}

mod surfel_gi;

/// A capacity with room for exactly two [`geometry_fixture_mesh`] meshes in every stream.
fn two_mesh_geometry_capacity() -> GeometryCapacity {
	GeometryCapacity {
		vertices: 200,
		vertex_indices: 400,
		triangles: 600,
		meshlets: 8,
		levels: 4,
		skinning_vertices: 100,
	}
}

/// A mesh that uses half of [`two_mesh_geometry_capacity`] in every stream.
fn geometry_fixture_mesh() -> GeometryCounts {
	GeometryCounts {
		vertices: 100,
		primitive_indices: 200,
		triangles: 300,
		meshlets: 4,
		levels: 2,
		skinning_vertices: 50,
		flat_triangles: 300,
	}
}

#[test]
fn released_mesh_ranges_are_reused_once_no_frame_in_flight_reads_them() {
	let ranges = std::sync::Mutex::new(GeometryRanges::new(two_mesh_geometry_capacity()));
	let take = |counts| ranges.lock().unwrap().take(&counts);
	let first = take(geometry_fixture_mesh()).expect("the first mesh fits");
	let second = take(geometry_fixture_mesh()).expect("the second mesh fits");
	assert!(take(geometry_fixture_mesh()).is_err());

	// Frame 10 is the first that no longer draws the first mesh; earlier frames may still be on the GPU.
	let mut retired = RetiredRanges::default();
	retired.retire(first.clone(), 10);
	let completed_frame = 10 + ghi::MAX_FRAMES_IN_FLIGHT as u64;
	retired.give_back_completed(completed_frame - 1, &ranges);
	assert!(take(geometry_fixture_mesh()).is_err());
	retired.give_back_completed(completed_frame, &ranges);

	let third = take(geometry_fixture_mesh()).expect("the third mesh fits where the first was");
	assert_eq!(third, first);
	assert_ne!(third, second);
}

#[test]
fn a_mesh_one_stream_cannot_hold_takes_nothing_from_the_others() {
	let ranges = std::sync::Mutex::new(GeometryRanges::new(two_mesh_geometry_capacity()));
	let take = |counts| ranges.lock().unwrap().take(&counts);
	let too_many_meshlets = GeometryCounts {
		meshlets: 9,
		..geometry_fixture_mesh()
	};
	assert!(take(too_many_meshlets).is_err());

	// Two meshes still fill every stream, so the rejected mesh left no range behind.
	assert!(take(geometry_fixture_mesh()).is_ok());
	assert!(take(geometry_fixture_mesh()).is_ok());
}

/* Fog light */

const FOG_LIGHT_LIGHTING: &str = "lighting_data";
const FOG_LIGHT_PARAMETERS: &str = "parameters";
const FOG_LIGHT_TILE_MASKS: &str = "tile_masks";
const FOG_LIGHT_DEPTH: &str = "depth";
const FOG_LIGHT_BLUE_NOISE: &str = "blue_noise";
const FOG_LIGHT_DIRECTIONAL_SHADOW: &str = "directional_shadow_map";
const FOG_LIGHT_OUTPUT: &str = "scattered_light";
/// The tile grid's columns, as `fog_light.besl` declares it.
const FOG_LIGHT_TILE_COLUMNS: u32 = 32;

/// Writes `parameters` into the fog light parameter buffer of `program`.
fn fog_light_parameters(
	program: &ExecutableProgram,
	parameters: &super::render_pass::FogLightShaderParameters,
) -> besl::vm::Buffer {
	let mut buffer = buffer(program, slot(program, FOG_LIGHT_PARAMETERS));
	for (field, value) in [
		(
			"pixel_to_camera_offset",
			Value::Mat4F(bytemuck::cast(parameters.pixel_to_camera_offset)),
		),
		("view", Value::Mat4x3F(bytemuck::cast(parameters.view))),
		("camera_position", Value::Vec4F(parameters.camera_position)),
		("camera_forward", Value::Vec4F(parameters.camera_forward)),
		("phase", Value::Vec4F(parameters.phase)),
		("albedo", Value::Vec4F(parameters.albedo)),
		("edge_slopes", Value::Vec2F(parameters.edge_slopes)),
		("samples", Value::U32(parameters.samples)),
	] {
		buffer.write(field, value).expect("fog light parameter");
	}
	for index in 0..2 {
		let layer = parameters.layers[index];
		for (field, value) in [
			("shape", layer.shape),
			("bounds_min", layer.bounds_min),
			("bounds_max", layer.bounds_max),
		] {
			buffer
				.write_indexed_field("layers", index, field, Value::Vec4F(value))
				.expect("fog light layer");
		}
		let volume = parameters.volumes[index];
		for (field, value) in [("min", volume.min), ("max", volume.max)] {
			buffer
				.write_indexed_field("volumes", index, field, Value::Vec4F(value))
				.expect("fog light volume");
		}
	}
	buffer
}

/// Uploads `lights` into the fog light passes' light table at `exposure`.
fn fog_light_table(program: &ExecutableProgram, lights: &[super::shader_data::LightData], exposure: f32) -> besl::vm::Buffer {
	let mut lighting = buffer(program, slot(program, FOG_LIGHT_LIGHTING));
	lighting
		.write("light_count", Value::U32(lights.len() as u32))
		.expect("light count");
	lighting.write("exposure", Value::F32(exposure)).expect("exposure");
	for (index, light) in lights.iter().enumerate() {
		for (field, value) in [
			("position", Value::Vec3F(light.position.into())),
			("color", Value::Vec3F(light.color.into())),
			("direction", Value::Vec3F(light.direction.into())),
			("cone_cosines", Value::Vec2F(light.cone_cosines)),
			("type", Value::U32(light.light_type)),
			("shadow_view_0", Value::U32(light.shadow_views[0])),
			("shadow_size_class", Value::U32(light.shadow_size_class)),
			("shadow_layer", Value::U32(light.shadow_layer)),
			("reach", Value::F32(light.reach)),
		] {
			lighting
				.write_indexed_field("lights", index, field, value)
				.expect("light field");
		}
	}
	lighting
}

/// Runs the fog light tile pass over `lights` for the tile at `column` and `row` of `sink`, and returns its first mask
/// word.
fn run_fog_light_tiles(
	fog: &crate::rendering::ExponentialHeightFog,
	sink: &crate::rendering::Sink,
	lights: &[super::shader_data::LightData],
	exposure: f32,
	[column, row]: [u32; 2],
) -> u32 {
	let program = asset!("fog-light-tiles.besl");
	let mut parameters = fog_light_parameters(&program, &super::render_pass::fog_light_shader_parameters(fog, sink));
	let mut lighting = fog_light_table(&program, lights, exposure);
	let mut masks = buffer(&program, slot(&program, FOG_LIGHT_TILE_MASKS));
	let tile = row * FOG_LIGHT_TILE_COLUMNS + column;
	let configs: [ExecutionConfig; 32] = std::array::from_fn(|lane| lane_config(lane as u32).with_threadgroup_position(tile));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_LIGHTING), &mut lighting);
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_PARAMETERS), &mut parameters);
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_TILE_MASKS), &mut masks);
	run_workgroup(&program, descriptors, &configs);
	read_u32(&masks, tile as usize * 32)
}

/// Returns a camera at the origin that looks along +Z through a 2x2 sink, with a field of view narrow enough that
/// every pixel looks along the axis.
fn fog_light_sink() -> crate::rendering::Sink {
	let view = crate::rendering::View::new_perspective(
		math::Degrees::new(0.5),
		1.0,
		0.1,
		100.0,
		math::Point::origin(),
		math::UnitVector::z_axis(),
	);
	crate::rendering::Sink::new(view, utils::Extent::square(2), 0)
}

/// Returns the reverse-Z depth [`fog_light_sink`] stores for a surface `distance` meters ahead.
fn fog_light_depth(distance: f32) -> f32 {
	let sink = fog_light_sink();
	let clip = sink.view().projection() * maths_rs::Vec4f::new(0.0, 0.0, distance, 1.0);
	clip.z / clip.w
}

/// The `FogLightShadows` struct holds what a march fixture's shadowed directional light reads: the views of its four
/// cascades from view 1, and the depth every texel of its shadow map stores.
struct FogLightShadows {
	view_projection: [f32; 16],
	stored_depth: f32,
}

/// Marches the single pixel of [`fog_light_sink`]'s half-resolution image through `fog` toward a surface at reverse-Z
/// `depth`, lit by `lights`, every one of which the pixel's tile holds. Returns the scattered light and the distance
/// channel.
fn run_fog_light(
	fog: &crate::rendering::ExponentialHeightFog,
	lights: &[super::shader_data::LightData],
	depth: f32,
	shadows: Option<FogLightShadows>,
) -> [f32; 4] {
	let program = asset!("fog-light.besl");
	let sink = fog_light_sink();
	let mut parameters = fog_light_parameters(&program, &super::render_pass::fog_light_shader_parameters(fog, &sink));
	let mut lighting = fog_light_table(&program, lights, 1.0);
	// The sink's one march pixel lies in the first tile, which holds every light. The tile pass's own tests cover which
	// lights a tile holds.
	let mut masks = buffer(&program, slot(&program, FOG_LIGHT_TILE_MASKS));
	masks
		.write_array_element(0, Value::U32((1u32 << lights.len()) - 1))
		.expect("fog light tile mask");
	let mut depth_texture = texture_2d(2, 2, &[[depth, 0.0, 0.0, 1.0]; 4]);
	// The middle of the noise's range places every sample in the middle of its step.
	let mut blue_noise = texture_2d(1, 1, &[[0.5, 0.5, 0.0, 1.0]]);
	let mut views = buffer(&program, slot(&program, VIEWS));
	let mut shadow_map = Texture::new_3d(4, 4, 4).expect("shadow map");
	if let Some(shadows) = &shadows {
		for cascade in 0..4 {
			views
				.write_array_member(1 + cascade, "view_projection", Value::Mat4F(shadows.view_projection))
				.expect("cascade view");
			views
				.write_array_member(1 + cascade, "far", Value::F32(1000.0))
				.expect("cascade far");
			for texel in 0..16 {
				shadow_map
					.write_3d([texel % 4, texel / 4, cascade as u32], [shadows.stored_depth, 0.0, 0.0, 1.0])
					.expect("shadow texel");
			}
		}
	}
	let mut output = empty_image(1, 1);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, VIEWS), &mut views);
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_LIGHTING), &mut lighting);
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_PARAMETERS), &mut parameters);
	descriptors.bind_buffer(slot(&program, FOG_LIGHT_TILE_MASKS), &mut masks);
	descriptors.bind_texture(slot(&program, FOG_LIGHT_DEPTH), &mut depth_texture);
	descriptors.bind_texture(slot(&program, FOG_LIGHT_BLUE_NOISE), &mut blue_noise);
	descriptors.bind_texture(slot(&program, FOG_LIGHT_DIRECTIONAL_SHADOW), &mut shadow_map);
	descriptors.bind_image(slot(&program, FOG_LIGHT_OUTPUT), &mut output);
	run_at(&program, &mut descriptors, [0, 0]);
	drop(descriptors);
	rgba(&output, [0, 0])
}

/// Returns a sun of `lux` lux whose light travels along `travel`, shadowed from view 1 when `shadowed`.
fn fog_light_sun(lux: f32, travel: math::UnitVector, shadowed: bool) -> super::shader_data::LightData {
	use crate::rendering::lights::{DirectionalLight, LightColor, Lights, PhotometricIntensity};

	let sun = Lights::Direction(
		DirectionalLight::new(
			LightColor::LinearSrgb(maths_rs::Vec3f::new(1.0, 1.0, 1.0)),
			PhotometricIntensity::Illuminance {
				lux,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light"),
	);
	let shadow = if shadowed {
		super::shadow_selection::LightShadow::Directional { slot: 0 }
	} else {
		super::shadow_selection::LightShadow::None
	};
	let transform =
		crate::gameplay::Transform::from_position(math::Point::origin()).rotation(math::orientation_from_direction(travel));
	super::scene::light_data(&sun, &transform, shadow, None)
}

/// Returns uniform fog of `density` per meter, lit by nothing but the lights, that scatters with anisotropy
/// `anisotropy` and takes `samples` light samples.
fn uniform_fog(density: f32, anisotropy: f32, samples: u32) -> crate::rendering::ExponentialHeightFog {
	use crate::rendering::{ExponentialHeightFog, FogLayer};

	ExponentialHeightFog::new(FogLayer::new(density).with_height_falloff(0.0))
		.with_anisotropy(anisotropy)
		.with_light_samples(samples)
}

/// Returns the Henyey-Greenstein phase function with anisotropy `g` at a scattering angle whose cosine is `cosine`.
fn henyey_greenstein(g: f32, cosine: f32) -> f32 {
	(1.0 - g * g) / (4.0 * std::f32::consts::PI * (1.0 + g * g - 2.0 * g * cosine).powf(1.5))
}

/// Integrates, along the view axis from the camera to `distance`, the light that `intensity` candela at `light`
/// scatter toward the camera in uniform fog of `density`, weighting each point by `falloff` of the direction from the
/// light to it. A reference for the march's estimate.
fn reference_local_fog_light(
	light: [f32; 3],
	intensity: f32,
	density: f32,
	anisotropy: f32,
	distance: f32,
	falloff: impl Fn([f32; 3]) -> f32,
) -> f32 {
	const STEPS: usize = 200_000;
	let step = distance / STEPS as f32;
	(0..STEPS)
		.map(|index| {
			let t = (index as f32 + 0.5) * step;
			let to_light = [light[0], light[1], light[2] - t];
			let squared = to_light.iter().map(|axis| axis * axis).sum::<f32>();
			let length = squared.sqrt();
			let light_to_fog = to_light.map(|axis| -axis / length);
			density * (-density * t).exp() * intensity / squared
				* henyey_greenstein(anisotropy, to_light[2] / length)
				* falloff(light_to_fog)
				* step
		})
		.sum()
}

/// Verifies a tile holds directional lights, and point lights whose reach overlaps both the tile's view and the fog,
/// with one bit per light-table entry.
#[test]
fn fog_light_tiles_hold_the_lights_that_reach_the_fog() {
	use math::{Point, UnitVector};

	use crate::rendering::{ExponentialHeightFog, FogLayer};

	let sink = crate::rendering::Sink::new(fixture_camera(1.0), utils::Extent::square(64), 0);
	let forward = UnitVector::z_axis();
	let lights = [
		// 20 m ahead, in the fog, on the edge of the tile right of the image's center.
		uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, 20.0), forward),
		// Behind the camera, 20 m away.
		uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, -20.0), forward),
		fog_light_sun(100_000.0, -UnitVector::y_axis(), false),
	];
	// At this exposure each point light reaches 10 m.
	let dim = 1.0 / 1024.0;
	let center_tile = [16, 9];
	let layer = FogLayer::new(0.1).with_height_falloff(1.0);
	// The density falls to 0.0001 ln(1000) ≈ 6.9 m above the base height.
	let around_the_light = ExponentialHeightFog::new(layer);
	let far_below_the_light = ExponentialHeightFog::new(layer.with_base_height(-20.0));
	let beside_the_light = ExponentialHeightFog::new(
		FogLayer::new(0.1).with_bounds(math::AABB::new(Point::new(20.0, -1.0, 15.0), Point::new(30.0, 1.0, 25.0))),
	);

	assert_eq!(
		run_fog_light_tiles(&around_the_light, &sink, &lights, dim, center_tile),
		0b101
	);
	assert_eq!(
		run_fog_light_tiles(&far_below_the_light, &sink, &lights, dim, center_tile),
		0b100
	);
	assert_eq!(
		run_fog_light_tiles(&beside_the_light, &sink, &lights, dim, center_tile),
		0b100
	);
	// A tile at the image's left edge looks away from the light ahead.
	assert_eq!(run_fog_light_tiles(&around_the_light, &sink, &lights, dim, [0, 9]), 0b100);
}

/// Verifies an unshadowed sun lights the fog by the closed form, without samples: its illuminance times the phase
/// function times the fog's opacity, with the marched distance in alpha.
#[test]
fn fog_light_of_an_unshadowed_sun_is_the_closed_form() {
	let density = 0.1;
	let lux = 10.0;
	// Looking straight up the sun's light, which travels toward the camera.
	let sun = fog_light_sun(lux, -math::UnitVector::z_axis(), false);

	let light = run_fog_light(&uniform_fog(density, 0.5, 1), &[sun], fog_light_depth(5.0), None);

	let expected = lux * henyey_greenstein(0.5, 1.0) * (1.0 - (-density * 5.0f32).exp());
	assert_rgba_close(light, [expected, expected, expected, (1.0f32 + 5.0).log2()], expected * 1e-3);
}

/// Verifies a shadowed sun lights the fog only where its shadow map leaves it lit: fully with no occluder, and not at
/// all with one above every texel.
#[test]
fn fog_light_of_a_shadowed_sun_follows_its_shadow_map() {
	let density = 0.1;
	let sun = fog_light_sun(10.0, -math::UnitVector::z_axis(), true);
	// An orthographic cascade, in columns, that holds the view axis's first 10 m: x across it, z down it, and every point
	// at a depth of 0.5.
	let view_projection = [
		0.1, 0.0, 0.0, 0.0, //
		0.0, 0.0, 0.0, 0.0, //
		0.0, 0.1, 0.0, 0.0, //
		0.0, -0.5, 0.5, 1.0,
	];
	let shadows = |stored_depth| {
		Some(FogLightShadows {
			view_projection,
			stored_depth,
		})
	};
	let fog = uniform_fog(density, 0.0, 8);

	let lit = run_fog_light(&fog, &[sun], fog_light_depth(5.0), shadows(0.0));
	let shadowed = run_fog_light(&fog, &[sun], fog_light_depth(5.0), shadows(1.0));

	let expected = 10.0 * henyey_greenstein(0.0, 1.0) * (1.0 - (-density * 5.0f32).exp());
	assert_rgba_close(lit, [expected, expected, expected, lit[3]], expected * 1e-3);
	assert_rgba_close(shadowed, [0.0, 0.0, 0.0, shadowed[3]], 1e-6);
}

/// Verifies the march's estimate of a point light's light in the fog matches the scattering integral.
#[test]
fn fog_light_of_a_point_light_matches_the_scattering_integral() {
	use math::{Point, UnitVector};

	let density = 0.05;
	let anisotropy = 0.3;
	let light = uploaded_light(
		light_cluster_fixture_light(false),
		Point::new(1.0, 0.0, 4.0),
		UnitVector::z_axis(),
	);

	let marched = run_fog_light(&uniform_fog(density, anisotropy, 64), &[light], fog_light_depth(8.0), None);

	let expected = reference_local_fog_light([1.0, 0.0, 4.0], 100.0, density, anisotropy, 8.0, |_| 1.0);
	assert!(
		(marched[0] - expected).abs() <= expected * 0.02,
		"The march's point light estimate is off. The most likely cause is a wrong equi-angular weight: marched {marched:?}, expected {expected}"
	);
}

/// Verifies a cone light lights only the fog inside its cone: a cone above the view axis pointing down at it matches
/// the scattering integral with its angular falloff, and one pointing up lights nothing.
#[test]
fn fog_light_of_a_cone_light_stays_inside_its_cone() {
	use math::{Point, UnitVector};

	let density = 0.05;
	let apex = Point::new(0.0, 2.0, 4.0);
	let fog = uniform_fog(density, 0.0, 64);
	let down = uploaded_light(light_cluster_fixture_light(true), apex, -UnitVector::y_axis());
	let up = uploaded_light(light_cluster_fixture_light(true), apex, UnitVector::y_axis());

	let marched = run_fog_light(&fog, &[down], fog_light_depth(8.0), None);
	let away = run_fog_light(&fog, &[up], fog_light_depth(8.0), None);

	// The fixture cone's inner and outer half angles are 15° and 30°.
	let (inner, outer) = (15f32.to_radians().cos(), 30f32.to_radians().cos());
	let expected = reference_local_fog_light([0.0, 2.0, 4.0], 100.0, density, 0.0, 8.0, |direction| {
		((-direction[1] - outer) / (inner - outer)).clamp(0.0, 1.0)
	});
	assert!(
		(marched[0] - expected).abs() <= expected * 0.03,
		"The march's cone light estimate is off. The most likely cause is a wrong cone clip: marched {marched:?}, expected {expected}"
	);
	assert_rgba_close(away, [0.0, 0.0, 0.0, away[3]], 1e-6);
}
