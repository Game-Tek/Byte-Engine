//! Executes the checked-in visibility BESL assets in the BESL VM against small fixtures.

use besl::vm::{
	DescriptorBindings, ExecutableProgram, ExecutionConfig, MeshOutputs, ResourceSlot, Sampler, SamplerReductionMode,
	TaskOutputs, Texture, Value, WorkgroupState, input_slot, output_slot,
};

use super::mesh_dispatch::MeshDispatchWorkItem;
use super::render_pass::{OCCLUSION_PYRAMID_HEIGHT, OCCLUSION_PYRAMID_MIP_COUNT, OCCLUSION_PYRAMID_WIDTH, OcclusionPhase};
use super::shader_data::MESH_FLAG_DOUBLE_SIDED;
use crate::rendering::shader_vm_test::{
	array_buffer, assert_rgba_close, buffer, compile, empty_image, rgba, run_at, texture_2d,
};

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
const OCCLUSION_PYRAMID_SLOT: ResourceSlot = ResourceSlot::new(1069);
const OCCLUSION_VISIBILITY_SLOT: ResourceSlot = ResourceSlot::new(1070);
const VERTEX_POSITIONS_SLOT: ResourceSlot = ResourceSlot::new(2);
const VERTEX_UVS_SLOT: ResourceSlot = ResourceSlot::new(5);
const SKINNED_VERTICES_SLOT: ResourceSlot = ResourceSlot::new(4);
const VERTEX_INDICES_SLOT: ResourceSlot = ResourceSlot::new(6);
const PRIMITIVE_INDICES_SLOT: ResourceSlot = ResourceSlot::new(7);
const MESHLETS_SLOT: ResourceSlot = ResourceSlot::new(8);
const FIXTURE_INSTANCE_INDEX: usize = 3;
const FIXTURE_MESHLET_INDEX: usize = 5;
const MESHLET_INSTANCE_BITS: u32 = 12;
/// Task payload words keep the view's offset into its batch above the meshlet and the 10-bit instance indices.
const VIEW_OFFSET_SHIFT: u32 = MESHLET_INSTANCE_BITS + super::layout::MAX_INSTANCES.ilog2();
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
/// The 8x8 workgroup of the passes that share a tile of their neighborhood: the contact-shadow filter and the SSGI
/// trace, temporal, and upscale passes.
const TILE_WORKGROUP_WIDTH: u32 = 8;
const TILE_WORKGROUP_SIZE: usize = 64;

/// Parses and links one checked-in BESL asset that production baking consumes.
///
/// Returns the program rather than its `main`, because the program owns every function it calls.
fn asset_program(source: &str) -> besl::NodeReference {
	let program = besl::lex(
		besl::parse(source)
			.expect("Failed to parse a visibility shader asset. The most likely cause is invalid checked-in BESL source."),
	)
	.expect("Failed to link a visibility shader asset. The most likely cause is an invalid shader declaration.");
	program
		.get_main()
		.expect("Missing visibility shader main. The most likely cause is that a checked-in BESL asset is incomplete.");
	program
}

/// Reads one checked-in visibility asset's source.
macro_rules! asset_source {
	($name:literal) => {
		include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/",
			$name
		))
	};
}

/// Compiles one checked-in visibility asset for VM execution.
macro_rules! asset {
	($name:literal) => {
		compile(asset_program(asset_source!($name)))
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

/// Runs the whole 8x8 workgroup that holds `pixel`, which shaders sharing a tile across their workgroup need to compute
/// any one pixel. The caller binds the workgroup state.
fn run_tile_workgroup_containing(program: &ExecutableProgram, descriptors: &mut DescriptorBindings<'_>, pixel: [u32; 2]) {
	let configs = tile_configs::<TILE_WORKGROUP_SIZE>(
		TILE_WORKGROUP_WIDTH,
		pixel.map(|coordinate| coordinate - coordinate % TILE_WORKGROUP_WIDTH),
	);
	program
		.run_workgroup(descriptors, &configs)
		.expect("tile workgroup execution");
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

fn read_vec2u16(buffer: &besl::vm::Buffer, index: usize) -> [u16; 2] {
	match buffer.read_array_element(index).expect("VM vec2u16 array element") {
		Value::Vec2U16(value) => value,
		value => panic!("Unexpected visibility pixel mapping value: {value:?}."),
	}
}

/// Reads the first element of a mesh-shader output array.
fn read_output_u32(buffer: &besl::vm::Buffer, member: &str) -> u32 {
	match buffer.read_indexed(member, 0).expect("VM mesh output element") {
		Value::U32(value) => value,
		value => panic!("Unexpected mesh output value: {value:?}."),
	}
}

/// Verifies both masked fragment assets parse and link through the source-owned BESL seam.
#[test]
fn masked_fragment_assets_parse_and_link_with_structural_interfaces() {
	for source in [
		asset_source!("masked-fragment.besl"),
		asset_source!("masked-depth-fragment.besl"),
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

/// A column-major identity matrix in the BESL VM representation.
const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A column-major affine identity matrix in the BESL VM representation.
const IDENTITY_AFFINE_MATRIX: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// Returns a view-projection matrix that moves identity geometry outside the horizontal clip range.
fn horizontally_translated_matrix(translation: f32) -> [f32; 16] {
	let mut matrix = IDENTITY_MATRIX;
	matrix[12] = translation;
	matrix
}

/// Packs the production task payload without allowing its meshlet and instance indices to diverge.
///
/// `relative_meshlet_index` counts from the instance's first meshlet, `base_meshlet_index`.
fn meshlet_instance(relative_meshlet_index: u32, instance_index: u32) -> u32 {
	relative_meshlet_index | (instance_index << MESHLET_INSTANCE_BITS)
}

/// The `TaskMeshFixture` struct selects the instance and normal-cone inputs a task-culling test exercises.
#[derive(Clone, Copy)]
struct TaskMeshFixture {
	skinned: bool,
	/// The instance's `ShaderMesh::flags` word.
	flags: u32,
	/// Gives every meshlet a narrow normal cone facing straight away from the camera at the origin. Otherwise the
	/// cone test is disabled, so the fixture isolates frustum and skinning behavior.
	back_facing: bool,
	/// The instance's bounding sphere, which task shaders test before any meshlet. The fixture's model is the identity.
	bounding_sphere: [f32; 4],
}

impl Default for TaskMeshFixture {
	fn default() -> Self {
		Self {
			skinned: false,
			flags: 0,
			back_facing: false,
			// Wide enough to reach every fixture meshlet, so the instance test passes unless a test moves it.
			bounding_sphere: [0.0, 0.0, 0.5, 8.0],
		}
	}
}

/// Executes one exact production task workgroup at its global dispatch position over consecutive meshlets, culling
/// against `view_count` views from `view_base`.
///
/// With `occlusion`, the workgroup also culls by occlusion as the camera's passes do, and view zero is
/// [`occlusion_fixture_camera`].
fn run_meshlet_task_workgroup(
	view_projections: &[(usize, [f32; 16])],
	[view_base, view_count]: [u32; 2],
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
	let mut views = buffer(&program, VIEWS_SLOT);
	for (view_index, view_projection) in view_projections.iter().copied() {
		views
			.write_array_member(view_index, "view_projection", Value::Mat4F(view_projection))
			.expect("task view");
		views
			.write_array_member(view_index, "inverse_view", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX))
			.expect("task inverse view");
	}
	if occlusion.is_some() {
		let camera = occlusion_fixture_camera();
		for (member, value) in [
			("view_projection", Value::Mat4F(column_major(camera.view_projection()))),
			("view", Value::Mat4x3F(bytemuck::cast(ghi::pod::Mat4x3f::from(camera.view())))),
			("near", Value::F32(camera.near())),
			("far", Value::F32(camera.far())),
		] {
			views.write_array_member(0, member, value).expect("task occlusion camera");
		}
	}
	let mut meshes = buffer(&program, MESH_DATA_SLOT);
	meshes
		.write_array_member(FIXTURE_INSTANCE_INDEX, "model", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX))
		.expect("task mesh transform");
	meshes
		.write_array_member(FIXTURE_INSTANCE_INDEX, "bounding_sphere", Value::Vec4F(mesh.bounding_sphere))
		.expect("task mesh bounds");
	for (field, value) in [
		("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
		("meshlet_count", meshlet_count),
		("skinned_base_vertex_index", if mesh.skinned { 0 } else { u32::MAX }),
		("flags", mesh.flags),
	] {
		meshes
			.write_array_member(FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
			.expect("task mesh field");
	}
	let mut meshlets = array_buffer(&program, MESHLETS_SLOT, FIXTURE_MESHLET_INDEX + center_radii.len());
	for (meshlet_offset, center_radius) in center_radii.iter().copied().enumerate() {
		let meshlet_index = FIXTURE_MESHLET_INDEX + meshlet_offset;
		meshlets
			.write_array_member(meshlet_index, "center_radius", Value::PackedVec4F(center_radius))
			.expect("task meshlet bound");
		// A cutoff above one disables cone rejection. The back-facing cone sits at the meshlet center and points
		// along +Z, the octahedral center, which is the direction from the camera at the origin to the meshlet.
		let (cone_apex_cutoff, cone_axis) = if mesh.back_facing {
			(
				[center_radius[0], center_radius[1], center_radius[2], 0.5],
				[u16::MAX / 2 + 1; 2],
			)
		} else {
			([0.0, 0.0, 0.0, 2.0], [0; 2])
		};
		meshlets
			.write_array_member(meshlet_index, "cone_apex_cutoff", Value::PackedVec4F(cone_apex_cutoff))
			.expect("task cone cutoff");
		meshlets
			.write_array_member(meshlet_index, "cone_axis", Value::Vec2U16(cone_axis))
			.expect("task cone axis");
	}
	let mut push_constant = besl::vm::Buffer::new(program.push_constant_layout().expect("task push constants").clone());
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
	] {
		push_constant.write(member, Value::U32(value)).expect("task push constant");
	}
	let mut mesh_dispatch_work = buffer(&program, MESH_DISPATCH_WORK_SLOT);
	let packed_work = MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed();
	mesh_dispatch_work
		.write_array_element(workgroup_index as usize, Value::U32(packed_work))
		.expect("compact mesh dispatch work");
	// The record is keyed by the packed work item, not by its position in the dispatch.
	let mut occlusion_visibility = buffer(&program, OCCLUSION_VISIBILITY_SLOT);
	if let Some(occlusion) = &occlusion {
		occlusion_visibility
			.write_array_element(packed_work as usize, Value::U32(*occlusion.record))
			.expect("task occlusion record");
	}

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
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut views);
	descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
	descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
	descriptors.bind_buffer(MESH_DISPATCH_WORK_SLOT, &mut mesh_dispatch_work);
	let record = occlusion.map(|occlusion| {
		descriptors.bind_texture(OCCLUSION_PYRAMID_SLOT, occlusion.pyramid);
		descriptors.bind_buffer(OCCLUSION_VISIBILITY_SLOT, &mut occlusion_visibility);
		occlusion.record
	});
	descriptors.bind_push_constant(&mut push_constant);
	descriptors.bind_task_outputs(&mut task_outputs);
	descriptors.bind_workgroup_state(&mut workgroup_state);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("production task workgroup execution");
	drop(descriptors);
	if let Some(record) = record {
		*record = read_u32(&occlusion_visibility, packed_work as usize);
	}
	task_outputs
}

/// Runs one task workgroup against the camera, view zero, and returns every payload word it emitted.
fn camera_task_payload(center_radii: &[[f32; 4]], mesh: TaskMeshFixture, workgroup_index: u32) -> Vec<Value> {
	task_payload(&run_meshlet_task_workgroup(
		&[(0, IDENTITY_MATRIX)],
		[0, 1],
		center_radii,
		mesh,
		workgroup_index,
		None,
	))
}

/// Reads every payload word a task workgroup emitted, in emission order.
fn task_payload(outputs: &TaskOutputs) -> Vec<Value> {
	(0..outputs.mesh_output_count().expect("task mesh output count") as usize)
		.map(|index| {
			outputs
				.payload_value("meshlet_instances", index)
				.expect("emitted task payload word")
				.clone()
		})
		.collect()
}

/// The payload a task emits for a fixture meshlet seen by the view `view_offset` steps into its batch.
fn batched_meshlet_instance(relative_meshlet_index: u32, view_offset: u32) -> Value {
	Value::U32(meshlet_instance(relative_meshlet_index, FIXTURE_INSTANCE_INDEX as u32) | (view_offset << VIEW_OFFSET_SHIFT))
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
	let output = run_meshlet_task_workgroup(
		&[(0, IDENTITY_MATRIX), (1, IDENTITY_MATRIX)],
		[0, 2],
		&[[0.0, 0.0, 0.5, 0.1]],
		outside,
		0,
		None,
	);
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
		skinned: true,
		bounding_sphere: [4.0, 0.0, 0.5, 0.1],
		..Default::default()
	};
	let output = run_meshlet_task_workgroup(
		&[(0, IDENTITY_MATRIX), (1, IDENTITY_MATRIX)],
		[0, 2],
		&[[4.0, 0.0, 0.5, 0.1]],
		skinned,
		0,
		None,
	);

	assert_eq!(
		task_payload(&output),
		[batched_meshlet_instance(0, 0), batched_meshlet_instance(0, 1)]
	);
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
	let mut view_projections: [(usize, [f32; 16]); 8] =
		std::array::from_fn(|view_index| (view_index, horizontally_translated_matrix(4.0)));
	view_projections[3].1 = IDENTITY_MATRIX;
	view_projections[5].1 = IDENTITY_MATRIX;
	// Views 2 through 5: only the second and fourth see the meshlet.
	let output = run_meshlet_task_workgroup(
		&view_projections,
		[2, 4],
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

/// The camera that drew the occlusion fixture pyramids: at the origin, looking down +Z.
fn occlusion_fixture_camera() -> crate::rendering::View {
	crate::rendering::View::new_perspective(
		math::Degrees::new(90.0),
		2.0,
		0.1,
		100.0,
		math::Point::origin(),
		math::UnitVector::z_axis(),
	)
}

/// Returns the reversed depth the fixture camera stores for a surface `z` units in front of it.
fn occlusion_fixture_depth(z: f32) -> f32 {
	let projection = occlusion_fixture_camera().projection();
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
		skinned: true,
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

	run_meshlet_task_workgroup(&[], [0, 1], &[[100.0, 0.0, 10.0, 0.5]], outside, 0, Some(occlusion));

	assert_eq!(record, 0);
}

/// Verifies each seeded texel keeps the farthest depth of every pixel its footprint touches, including pixels it only
/// partly covers.
#[test]
fn occlusion_pyramid_seed_keeps_the_farthest_depth_under_each_texel() {
	let program = asset!("hiz-seed.besl");
	// Five depth pixels map onto two texels, so the middle pixel straddles both.
	let texels = [0.9, 0.8, 0.3, 0.7, 0.6].map(|depth| [depth, 0.0, 0.0, 1.0]);
	let mut depth = texture_2d(5, 1, &texels);
	let mut seeded = empty_image(2, 1);

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
	descriptors.bind_image(ResourceSlot::new(1034), &mut seeded);
	run_tile_workgroup_containing(&program, &mut descriptors, [0, 0]);
	drop(descriptors);

	assert_rgba_close(rgba(&seeded, [0, 0]), [0.3, 0.0, 0.0, 1.0], 0.0);
	assert_rgba_close(rgba(&seeded, [1, 0]), [0.3, 0.0, 0.0, 1.0], 0.0);
}

/// Verifies each reduced texel keeps the farthest of its four source texels.
#[test]
fn occlusion_pyramid_reduce_keeps_the_farthest_of_four_texels() {
	let program = asset!("hiz-reduce.besl");
	let texels = [0.5, 0.4, 0.9, 0.8, 0.6, 0.7, 0.2, 0.95].map(|depth| [depth, 0.0, 0.0, 1.0]);
	let mut source = texture_2d(4, 2, &texels);
	let mut reduced = empty_image(2, 1);

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_image(ResourceSlot::new(1033), &mut source);
	descriptors.bind_image(ResourceSlot::new(1034), &mut reduced);
	run_tile_workgroup_containing(&program, &mut descriptors, [0, 0]);
	drop(descriptors);

	assert_rgba_close(rgba(&reduced, [0, 0]), [0.4, 0.0, 0.0, 1.0], 0.0);
	assert_rgba_close(rgba(&reduced, [1, 0]), [0.2, 0.0, 0.0, 1.0], 0.0);
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
}

/// Executes one production mesh main over one identity triangle meshlet and verifies its complete output contract.
fn assert_triangle_mesh_program(
	program: ExecutableProgram,
	view: MeshView,
	skinned_positions: Option<[[f32; 4]; 3]>,
	expected_clip_positions: [[f32; 4]; 3],
	expected_render_target_array_index: Option<u32>,
) {
	let mut views = buffer(&program, VIEWS_SLOT);
	let mut meshes = buffer(&program, MESH_DATA_SLOT);
	meshes
		.write_array_member(FIXTURE_INSTANCE_INDEX, "model", Value::Mat4x3F(IDENTITY_AFFINE_MATRIX))
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
			.write_array_member(FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
			.expect("mesh offset");
	}
	let mut positions = array_buffer(&program, VERTEX_POSITIONS_SLOT, 3);
	for (index, position) in [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
		positions
			.write_array_element(index, Value::Vec3F(position))
			.expect("mesh vertex");
	}
	let mut skinned_vertices = buffer(&program, SKINNED_VERTICES_SLOT);
	let mut vertex_uvs = array_buffer(&program, VERTEX_UVS_SLOT, 3);
	let mut vertex_indices = array_buffer(&program, VERTEX_INDICES_SLOT, 3);
	let mut primitive_indices = array_buffer(&program, PRIMITIVE_INDICES_SLOT, 3);
	for index in 0..3 {
		vertex_uvs
			.write_array_element(index, Value::Vec2F16([besl::vm::f16::ZERO; 2]))
			.expect("mesh UV");
		vertex_indices
			.write_array_element(index, Value::U16(index as u16))
			.expect("vertex index");
		primitive_indices
			.write_array_element(index, Value::U8(index as u8))
			.expect("triangle index");
	}
	let mut meshlets = array_buffer(&program, MESHLETS_SLOT, FIXTURE_MESHLET_INDEX + 1);
	for (field, value) in [
		("primitive_offset", 0),
		("triangle_offset", 0),
		("primitive_count", 3),
		("triangle_count", 1),
	] {
		meshlets
			.write_array_member(FIXTURE_MESHLET_INDEX, field, Value::U32(value))
			.expect("meshlet field");
	}
	if let Some(skinned_positions) = skinned_positions {
		const SKINNED_BASE_VERTEX: usize = 7;
		meshes
			.write_array_member(
				FIXTURE_INSTANCE_INDEX,
				"skinned_base_vertex_index",
				Value::U32(SKINNED_BASE_VERTEX as u32),
			)
			.expect("skinned mesh vertices");
		for (index, position) in skinned_positions.into_iter().enumerate() {
			skinned_vertices
				.write_array_member(SKINNED_BASE_VERTEX + index, "position", Value::Vec4F(position))
				.expect("skinned mesh vertex");
		}
	}
	let mut push_constant = besl::vm::Buffer::new(program.push_constant_layout().expect("mesh push constant layout").clone());
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
	let payload = batched_meshlet_instance(0, view.view_offset);

	let mut out_instance_indices = buffer(&program, output_slot(0));
	let mut out_primitive_indices = buffer(&program, output_slot(1));
	let mut out_uvs = buffer(&program, output_slot(2));
	let mut mesh_outputs = MeshOutputs::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_payload("meshlet_instances", [payload]);
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
		read_output_u32(&out_instance_indices, "out_instance_index"),
		FIXTURE_INSTANCE_INDEX as u32
	);
	assert_eq!(
		read_output_u32(&out_primitive_indices, "out_primitive_index"),
		(FIXTURE_MESHLET_INDEX as u32) << 8
	);
}

/// Verifies visibility mesh output geometry and metadata through the BESL VM.
#[test]
fn visibility_mesh_main_emits_identity_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("visibility-mesh.besl"),
		MeshView::CAMERA,
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
		MeshView::CAMERA,
		Some(skinned_positions),
		skinned_positions,
		None,
	);
}

/// Verifies shadow mesh output draws the view its payload names and its layer, offset from the batch's first view
/// and layer.
#[test]
fn shadow_mesh_main_emits_selected_view_triangle_and_metadata() {
	assert_triangle_mesh_program(
		asset!("shadow-mesh.besl"),
		MeshView {
			view_base: 5,
			layer_base: 1,
			view_offset: 2,
			view_projection: horizontally_translated_matrix(2.0),
		},
		None,
		[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		Some(3),
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

/// Runs the material-offset pass's one 256-thread workgroup over `material_counts`, and returns the buffers it writes:
/// the offsets, the scratch cursors, and the indirect dispatches.
fn run_material_offset(
	program: &ExecutableProgram,
	material_counts: &mut besl::vm::Buffer,
) -> (besl::vm::Buffer, besl::vm::Buffer, besl::vm::Buffer) {
	let mut offsets = buffer(program, MATERIAL_OFFSET_SLOT);
	let mut scratch = buffer(program, MATERIAL_OFFSET_SCRATCH_SLOT);
	let mut dispatches = buffer(program, MATERIAL_DISPATCH_SLOT);
	let mut workgroup = WorkgroupState::new();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(MATERIAL_COUNT_SLOT, material_counts);
	descriptors.bind_buffer(MATERIAL_OFFSET_SLOT, &mut offsets);
	descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut scratch);
	descriptors.bind_buffer(MATERIAL_DISPATCH_SLOT, &mut dispatches);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &tile_configs::<256>(256, [0, 0]))
		.expect("material-offset workgroup execution");
	drop(descriptors);
	(offsets, scratch, dispatches)
}

/// Verifies the offset scan gives every material the exclusive sum of the counts before it, across every thread of its
/// workgroup.
#[test]
fn material_offset_scans_every_material_count() {
	let program = asset!("material-offset.besl");
	let count = |material: usize| (material * 37 % 300) as u32;
	let mut material_counts = buffer(&program, MATERIAL_COUNT_SLOT);
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

	// Three visible instances span two materials; the fourth texel is the renderer's empty-pixel sentinel.
	let mut mesh_data = buffer(&material_count_program, MESH_DATA_SLOT);
	for (mesh_index, material_index) in [(0, 2), (1, 5), (2, 2)] {
		mesh_data
			.write_array_member(mesh_index, "material_index", Value::U32(material_index))
			.expect("VM mesh");
	}
	let mut instance_indices = Texture::new(2, 2).expect("visibility index fixture");
	for (coordinate, instance_index) in [([0, 0], 0), ([1, 0], 1), ([0, 1], u32::MAX), ([1, 1], 2)] {
		instance_indices
			.write_u32(coordinate, instance_index)
			.expect("visibility index texel");
	}

	let mut material_counts = run_material_count(&material_count_program, &mut mesh_data, &mut instance_indices);
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
	let pixel_mapping = run_pixel_mapping(
		&pixel_mapping_program,
		&mut mesh_data,
		&mut material_offset_scratch,
		&mut instance_indices,
	);
	assert_eq!(read_vec2u16(&pixel_mapping, 0), [1, 1]);
	assert_eq!(read_vec2u16(&pixel_mapping, 1), [2, 2]);
	assert_eq!(read_vec2u16(&pixel_mapping, 2), [2, 1]);
	assert_eq!(read_u32(&material_offset_scratch, 2), 2);
	assert_eq!(read_u32(&material_offset_scratch, 5), 3);
}

/// Verifies a coherent tile reuses its established local key while preserving every pixel mapping.
#[test]
fn pixel_mapping_load_fast_path_preserves_coherent_tile_mappings() {
	let program = asset!("pixel-mapping.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	mesh_data
		.write_array_member(0, "material_index", Value::U32(7))
		.expect("coherent mesh");
	let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
	let mut instance_indices = instance_texture(PIXEL_MAPPING_WORKGROUP_WIDTH, |_| 0);

	let pixel_mapping = run_pixel_mapping(&program, &mut mesh_data, &mut material_offset_scratch, &mut instance_indices);

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

/// Verifies tile-local reservations preserve mappings when distinct materials exceed the bounded histogram.
#[test]
fn pixel_mapping_tile_reservation_preserves_overflowed_materials() {
	let program = asset!("pixel-mapping.besl");
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
	for material_index in 0..33 {
		mesh_data
			.write_array_member(material_index, "material_index", Value::U32(material_index as u32))
			.expect("VM mesh");
		material_offset_scratch
			.write_array_element(material_index, Value::U32(material_index as u32))
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
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
	for material_index in 0..MATERIALS {
		mesh_data
			.write_array_member(material_index, "material_index", Value::U32(material_index as u32))
			.expect("VM mesh");
		material_offset_scratch
			.write_array_element(material_index, Value::U32((material_index * RANGE) as u32))
			.expect("material mapping offset");
	}
	// Horizontal pairs of texels share a material, so one subgroup ranks several pixels of most materials.
	let material_of = |texel: usize| (texel / 2) % MATERIALS;
	let width = PIXEL_MAPPING_WORKGROUP_WIDTH as usize;
	let mut instance_indices = instance_texture(PIXEL_MAPPING_WORKGROUP_WIDTH, |texel| material_of(texel) as u32);

	let pixel_mapping = run_pixel_mapping(&program, &mut mesh_data, &mut material_offset_scratch, &mut instance_indices);

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
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	for material_index in 0..33 {
		mesh_data
			.write_array_member(material_index, "material_index", Value::U32(material_index as u32))
			.expect("VM mesh");
	}
	let mut instance_indices = instance_texture(MATERIAL_COUNT_WORKGROUP_WIDTH, |lane| (lane % 33) as u32);

	let material_counts = run_material_count(&program, &mut mesh_data, &mut instance_indices);

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
	let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
	mesh_data
		.write_array_member(0, "material_index", Value::U32(7))
		.expect("coherent mesh");
	let mut instance_indices = instance_texture(MATERIAL_COUNT_WORKGROUP_WIDTH, |_| 0);

	let material_counts = run_material_count(&program, &mut mesh_data, &mut instance_indices);

	assert_eq!(read_u32(&material_counts, 7), MATERIAL_COUNT_WORKGROUP_SIZE as u32);
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

/// Builds a GTAO depth pyramid whose mip zero is `levels[0]` at the fixture extent.
fn gtao_depth_pyramid(width: u32, height: u32, levels: [&[[f32; 4]]; 3], extents: [(u32, u32); 2]) -> Texture {
	let mut pyramid = texture_2d(width, height, levels[0]);
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

/// Verifies one SIMD group reduces two adjacent 8x8 tiles to one max-depth cell each in every cascade.
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
				let maximum = cell_maximum(layer, x / 8, y / 8);
				let depth = if x % 8 == 2 * layer + 1 && y % 8 == 7 - 2 * layer {
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
	let mut reduced = empty_image(2, 4);
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
		for cell_x in 0..2 {
			assert_rgba_close(
				rgba(&reduced, [cell_x, layer]),
				[cell_maximum(layer, cell_x, 0), 0.0, 0.0, 1.0],
				0.00001,
			);
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

/* SSGI */

const SSGI_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1);
const SSGI_EXTENT: u32 = 32;
/// The uniform diffuse radiance of the previous frame in trace fixtures, as the trace reports it for a hit.
const SSGI_LIT_COLOR: [f32; 4] = [2.0, 1.0, 0.5, 1.0];

/// Returns the square fixture projection that [`gtao_view_data`] also encodes.
fn ssgi_projection() -> maths_rs::Mat4f {
	math::projection_matrix(math::Degrees::new(60.0), 1.0, GTAO_NEAR, GTAO_FAR)
}

/// Converts a row-major matrix to the column-major element order the BESL VM multiplies with.
fn column_major(matrix: maths_rs::Mat4f) -> [f32; 16] {
	std::array::from_fn(|index| matrix[(index % 4) * 4 + index / 4])
}

/// Creates SSGI per-frame parameters. `previous_clip` is `None` when the frame has no usable history.
fn ssgi_parameters(program: &ExecutableProgram, previous_clip: Option<maths_rs::Mat4f>, frame_index: u32) -> besl::vm::Buffer {
	let mut parameters = buffer(program, SSGI_PARAMETERS_SLOT);
	for (member, value) in [
		(
			"current_view_to_previous_clip",
			Value::Mat4F(column_major(previous_clip.unwrap_or_else(maths_rs::Mat4f::identity))),
		),
		("frame_index", Value::U32(frame_index)),
		("history_valid", Value::U32(previous_clip.is_some() as u32)),
	] {
		parameters.write(member, value).expect("SSGI parameters");
	}
	parameters
}

/// Returns the view-space ray `(x / z, y / z)` through the center of pixel `(x, y)` of a square fixture image.
fn ssgi_ray_at(x: f32, y: f32, extent: u32) -> [f32; 2] {
	let projection = ssgi_projection();
	[
		(2.0 * (x + 0.5) / extent as f32 - 1.0) / projection[0],
		(1.0 - 2.0 * (y + 0.5) / extent as f32) / projection[5],
	]
}

/// Returns the depth a ray sees in a scene with a floor one unit below the camera and, optionally, a facing wall.
fn ssgi_floor_depth(ray: [f32; 2], wall_z: Option<f32>) -> f32 {
	let floor_z = if ray[1] < 0.0 { -1.0 / ray[1] } else { f32::INFINITY };
	let depth = wall_z.map_or(floor_z, |wall_z| floor_z.min(wall_z));
	if depth <= GTAO_FAR { depth } else { 0.0 }
}

/// Renders `scene` into the half-resolution depth the trace marches, `extent` pixels square, and the full-resolution
/// diffuse radiance it reads, whose alpha holds each pixel's depth. `scene` maps a pixel ray to its depth and radiance.
fn ssgi_scene_images(extent: u32, scene: impl Fn([f32; 2]) -> (f32, [f32; 3])) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
	let image = |extent: u32| {
		(0..extent * extent)
			.map(|index| scene(ssgi_ray_at((index % extent) as f32, (index / extent) as f32, extent)))
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
	run_ssgi_trace_outputs(program, SSGI_EXTENT, &depth, &radiance, history, frame_index, &[pixel])[0].0
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
) -> Vec<([f32; 4], [f32; 4])> {
	let mut view = gtao_view_data(program, extent, extent);
	// A static camera reprojects through the unchanged projection.
	let mut parameters = ssgi_parameters(program, history.then(ssgi_projection), frame_index);
	let mut depth_pyramid = texture_2d(extent, extent, depth);
	let mut previous_lit = texture_2d(extent * 2, extent * 2, radiance);
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
		descriptors.bind_buffer(VIEWS_SLOT, &mut view);
		descriptors.bind_buffer(SSGI_PARAMETERS_SLOT, &mut parameters);
		descriptors.bind_texture(ResourceSlot::new(1033), &mut depth_pyramid);
		descriptors.bind_image(ResourceSlot::new(1034), &mut output);
		descriptors.bind_texture(ResourceSlot::new(1035), &mut previous_lit);
		descriptors.bind_image(ResourceSlot::new(1036), &mut normals);
		let mut workgroup = WorkgroupState::new();
		descriptors.bind_workgroup_state(&mut workgroup);
		run_tile_workgroup_containing(program, &mut descriptors, workgroup_base);
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
	let stored =
		|depth: &[[f32; 4]], pixel| run_ssgi_trace_outputs(&program, SSGI_EXTENT, depth, &radiance, false, 0, &[pixel])[0].1;

	assert_rgba_close(stored(&depth, [column, wall_row]), SSGI_WALL_NORMAL, 0.0001);
	depth[(wall_row * SSGI_EXTENT + column) as usize] = [0.0; 4];
	assert_rgba_close(stored(&depth, [column, wall_row]), [0.0; 4], 0.0);
}

const SSGI_TEMPORAL_EXTENT: u32 = 8;
/// The stored view-space normal of a wall that faces the camera, (0, 0, -1). View space is y-up and the camera looks
/// down positive z. The trace stores normals as an octahedral pair in RG, which folds the lower z hemisphere into the
/// corners.
const SSGI_WALL_NORMAL: [f32; 4] = [1.0, 1.0, 0.0, 0.0];
/// The stored view-space normal of the floor below the camera, (0, 1, 0).
const SSGI_FLOOR_NORMAL: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

/// The inputs of one SSGI temporal fixture. Every image is `extent` pixels square.
struct SsgiTemporalFixture {
	extent: u32,
	depth: Vec<[f32; 4]>,
	normals: Vec<[f32; 4]>,
	raw: Vec<[f32; 4]>,
	previous_depth: Vec<[f32; 4]>,
	previous_normals: Vec<[f32; 4]>,
	previous_history: [f32; 4],
	history: bool,
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
		}
	}

	fn run(&self, pixel: [u32; 2]) -> [f32; 4] {
		let program = asset!("ssgi-temporal.besl");
		let extent = self.extent;
		let texel_count = (extent * extent) as usize;
		let mut view = gtao_view_data(&program, extent, extent);
		let mut parameters = ssgi_parameters(&program, self.history.then(ssgi_projection), 0);
		let mut depth_pyramid = texture_2d(extent, extent, &self.depth);
		let mut raw = texture_2d(extent, extent, &self.raw);
		let mut output = empty_image(extent, extent);
		let mut previous_history = texture_2d(extent, extent, &vec![self.previous_history; texel_count]);
		let mut previous_depth_pyramid = texture_2d(extent, extent, &self.previous_depth);
		let mut normals = texture_2d(extent, extent, &self.normals);
		let mut previous_normals = texture_2d(extent, extent, &self.previous_normals);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(VIEWS_SLOT, &mut view);
		descriptors.bind_buffer(SSGI_PARAMETERS_SLOT, &mut parameters);
		descriptors.bind_texture(ResourceSlot::new(1033), &mut depth_pyramid);
		descriptors.bind_texture(ResourceSlot::new(1034), &mut raw);
		descriptors.bind_image(ResourceSlot::new(1035), &mut output);
		descriptors.bind_texture(ResourceSlot::new(1036), &mut previous_history);
		descriptors.bind_texture(ResourceSlot::new(1037), &mut previous_depth_pyramid);
		descriptors.bind_texture(ResourceSlot::new(1038), &mut normals);
		descriptors.bind_texture(ResourceSlot::new(1039), &mut previous_normals);
		let mut workgroup = WorkgroupState::new();
		descriptors.bind_workgroup_state(&mut workgroup);
		run_tile_workgroup_containing(&program, &mut descriptors, pixel);
		drop(descriptors);
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

/// Returns the depth and view-space normal of the wall at depth `wall_z` standing on the floor of
/// [`ssgi_floor_depth`], at pixel `(x, y)` of an `extent` square image.
fn ssgi_contact_surface(x: u32, y: u32, extent: u32, wall_z: f32) -> (f32, [f32; 4]) {
	let depth = ssgi_floor_depth(ssgi_ray_at(x as f32, y as f32, extent), Some(wall_z));
	(
		depth,
		if depth == wall_z {
			SSGI_WALL_NORMAL
		} else {
			SSGI_FLOOR_NORMAL
		},
	)
}

/// Verifies the spatial filter keeps light off a surface that touches the center's surface at the same depth, as a
/// floor meets a wall or a foot.
#[test]
fn ssgi_temporal_filter_keeps_light_off_a_touching_surface() {
	const EXTENT: u32 = 16;
	const WALL_Z: f32 = 4.0;
	let mut fixture = SsgiTemporalFixture::uniform([0.0; 4], [0.0; 4], false);
	let surfaces: Vec<_> = (0..EXTENT * EXTENT)
		.map(|index| ssgi_contact_surface(index % EXTENT, index / EXTENT, EXTENT, WALL_Z))
		.collect();
	fixture.extent = EXTENT;
	fixture.depth = surfaces.iter().map(|&(z, _)| [z, 0.0, 0.0, 1.0]).collect();
	fixture.normals = surfaces.iter().map(|&(_, normal)| normal).collect();
	fixture.previous_depth = fixture.depth.clone();
	fixture.previous_normals = fixture.normals.clone();
	// Only the wall gathered light.
	fixture.raw = surfaces
		.iter()
		.map(|&(z, _)| if z == WALL_Z { [1.0; 4] } else { [0.0; 4] })
		.collect();
	let column = EXTENT / 2;
	let floor_row = (0..EXTENT)
		.find(|&row| surfaces[(row * EXTENT + column) as usize].0 != WALL_Z)
		.expect("the wall stands on the floor");
	let floor_z = surfaces[(floor_row * EXTENT + column) as usize].0;
	assert!(
		(WALL_Z - floor_z) / WALL_Z < 0.02,
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

/// Runs the SSGI upscale over the 8x8 workgroup that holds `pixel` and returns that full-resolution pixel. Low-resolution
/// inputs are half the full extent.
fn run_ssgi_upscale(
	full_extent: u32,
	device_depth: &[[f32; 4]],
	low_resolution_depth: &[[f32; 4]],
	low_resolution_normals: &[[f32; 4]],
	radiance: &[[f32; 4]],
	pixel: [u32; 2],
) -> [f32; 4] {
	let program = asset!("ssgi-upscale.besl");
	let low_extent = full_extent / 2;
	let mut view = gtao_view_data(&program, low_extent, low_extent);
	let mut visibility_depth = texture_2d(full_extent, full_extent, device_depth);
	let mut source = texture_2d(low_extent, low_extent, radiance);
	let mut output = empty_image(full_extent, full_extent);
	let mut depth_pyramid = texture_2d(low_extent, low_extent, low_resolution_depth);
	let mut normals = texture_2d(low_extent, low_extent, low_resolution_normals);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_texture(ResourceSlot::new(1033), &mut visibility_depth);
	descriptors.bind_texture(ResourceSlot::new(1034), &mut source);
	descriptors.bind_image(ResourceSlot::new(1035), &mut output);
	descriptors.bind_texture(ResourceSlot::new(1036), &mut depth_pyramid);
	descriptors.bind_texture(ResourceSlot::new(1037), &mut normals);
	let mut workgroup = WorkgroupState::new();
	descriptors.bind_workgroup_state(&mut workgroup);
	run_tile_workgroup_containing(&program, &mut descriptors, pixel);
	drop(descriptors);
	rgba(&output, pixel)
}

/// Verifies upscaling keeps indirect light on its own side of a depth edge and leaves the background unlit.
#[test]
fn ssgi_upscale_keeps_light_on_its_own_side_of_a_depth_edge() {
	const FULL: u32 = 16;
	const LOW: u32 = FULL / 2;
	let device_depth_for = |linear_depth: f32| {
		let range = GTAO_FAR - GTAO_NEAR;
		(GTAO_NEAR * GTAO_FAR / range) / linear_depth - GTAO_NEAR / range
	};
	let device_depth: Vec<[f32; 4]> = (0..FULL * FULL)
		.map(|index| {
			let (x, y) = (index % FULL, index / FULL);
			let depth = if y == FULL - 1 {
				0.0
			} else if x < FULL / 2 {
				device_depth_for(2.0)
			} else {
				device_depth_for(10.0)
			};
			[depth, 0.0, 0.0, 1.0]
		})
		.collect();
	let low_depth: Vec<[f32; 4]> = (0..LOW * LOW)
		.map(|index| [if index % LOW < LOW / 2 { 2.0 } else { 10.0 }, 0.0, 0.0, 1.0])
		.collect();
	let radiance: Vec<[f32; 4]> = (0..LOW * LOW)
		.map(|index| {
			if index % LOW < LOW / 2 {
				[1.0, 0.0, 0.0, 1.0]
			} else {
				[0.0, 1.0, 0.0, 1.0]
			}
		})
		.collect();
	// Both walls face the camera.
	let normals = vec![SSGI_WALL_NORMAL; (LOW * LOW) as usize];

	let near = run_ssgi_upscale(FULL, &device_depth, &low_depth, &normals, &radiance, [FULL / 2 - 1, 4]);
	let far = run_ssgi_upscale(FULL, &device_depth, &low_depth, &normals, &radiance, [FULL / 2, 4]);
	let background = run_ssgi_upscale(FULL, &device_depth, &low_depth, &normals, &radiance, [3, FULL - 1]);

	assert_rgba_close(near, [1.0, 0.0, 0.0, 1.0], 0.0001);
	assert_rgba_close(far, [0.0, 1.0, 0.0, 1.0], 0.0001);
	assert_rgba_close(background, [0.0; 4], 0.0);
}

/// Verifies upscaling keeps light off a surface that touches the pixel's surface at nearly the same depth.
#[test]
fn ssgi_upscale_keeps_light_off_a_touching_surface() {
	const FULL: u32 = 32;
	const LOW: u32 = FULL / 2;
	const WALL_Z: f32 = 4.0;
	let range = GTAO_FAR - GTAO_NEAR;
	let device_depth: Vec<[f32; 4]> = (0..FULL * FULL)
		.map(|index| {
			let (z, _) = ssgi_contact_surface(index % FULL, index / FULL, FULL, WALL_Z);
			let depth = if z == 0.0 {
				0.0
			} else {
				(GTAO_NEAR * GTAO_FAR / range) / z - GTAO_NEAR / range
			};
			[depth, 0.0, 0.0, 1.0]
		})
		.collect();
	let low_surfaces: Vec<_> = (0..LOW * LOW)
		.map(|index| ssgi_contact_surface(index % LOW, index / LOW, LOW, WALL_Z))
		.collect();
	let low_depth: Vec<[f32; 4]> = low_surfaces.iter().map(|&(z, _)| [z, 0.0, 0.0, 1.0]).collect();
	let low_normals: Vec<[f32; 4]> = low_surfaces.iter().map(|&(_, normal)| normal).collect();
	// Only the wall gathered light.
	let radiance: Vec<[f32; 4]> = low_surfaces
		.iter()
		.map(|&(z, _)| if z == WALL_Z { [1.0; 4] } else { [0.0; 4] })
		.collect();
	let column = FULL / 2;
	let floor_row = (0..FULL)
		.find(|&row| ssgi_contact_surface(column, row, FULL, WALL_Z).0 != WALL_Z)
		.expect("the wall stands on the floor");

	let wall = run_ssgi_upscale(
		FULL,
		&device_depth,
		&low_depth,
		&low_normals,
		&radiance,
		[column, floor_row - 1],
	);
	let floor = run_ssgi_upscale(FULL, &device_depth, &low_depth, &low_normals, &radiance, [column, floor_row]);
	assert!(
		wall[0] > 0.99,
		"Expected the wall next to the floor to keep its light, found {wall:?}."
	);
	assert!(
		floor[0] < 0.01,
		"Expected no wall light on the floor next to it, found {floor:?}."
	);
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

	let mut hits = 0;
	for frame_index in 0..64 {
		let radiance = run_ssgi_trace(&program, Some(4.0), true, frame_index, pixel);
		if radiance[3] != 0.0 {
			assert_rgba_close(radiance, SSGI_LIT_COLOR, 0.0001);
			hits += 1;
		}
	}
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

	let mut hits = 0;
	for frame_index in 0..64 {
		let radiance = run_ssgi_trace_outputs(&program, EXTENT, &depth, &radiance, true, frame_index, &[pixel])[0].0;
		if radiance[3] != 0.0 {
			assert_rgba_close(radiance, SSGI_LIT_COLOR, 0.0001);
			hits += 1;
		}
	}
	// About a third of cosine-weighted rays point down steeply enough to land on the floor within reach.
	assert!(
		hits >= 16,
		"Expected about a third of the rays to hit the floor, found {hits} hits in 64 frames."
	);
}

/// Returns the view-space depth that a ray through `ray` sees in a scene with a floor one unit below the camera
/// and a pillar whose front face stands at depth three, and whether that surface is the pillar.
fn ssgi_pillar_scene(ray: [f32; 2]) -> (f32, bool) {
	const PILLAR_Z: f32 = 3.0;
	let pillar = (ray[0] * PILLAR_Z).abs() <= 0.25 && (-1.0..=0.5).contains(&(ray[1] * PILLAR_Z));
	if pillar {
		return (PILLAR_Z, true);
	}
	let floor_z = if ray[1] < 0.0 { -1.0 / ray[1] } else { 0.0 };
	(if floor_z <= GTAO_FAR { floor_z } else { 0.0 }, false)
}

/// Verifies floor rays that reach a pillar take its light and never read the floor seen past the pillar's edge.
///
/// A floor cannot light itself, so every hit from a floor pixel in front of the pillar must carry the pillar's red.
#[test]
fn ssgi_trace_does_not_read_the_background_past_a_silhouette() {
	let program = asset!("ssgi-trace.besl");
	let (depth, radiance) = ssgi_scene_images(SSGI_EXTENT, |ray| {
		let (z, pillar) = ssgi_pillar_scene(ray);
		(z, if pillar { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] })
	});
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
		let outputs = run_ssgi_trace_outputs(&program, SSGI_EXTENT, &depth, &radiance, true, frame_index, &pixels);
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

/// Returns the depth a pixel ray sees on a floor [`CONTACT_SHADOW_CAMERA_HEIGHT`] below the camera and, optionally,
/// a low wall facing the camera at [`CONTACT_SHADOW_WALL_Z`], or zero for the sky.
fn contact_shadow_scene_depth(ray: [f32; 2], wall: bool) -> f32 {
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

/// Returns the reversed device depth of every pixel of the floor scene, with or without the low wall.
fn contact_shadow_device_depth(wall: bool) -> Vec<[f32; 4]> {
	let extent = CONTACT_SHADOW_EXTENT;
	let range = GTAO_FAR - GTAO_NEAR;
	(0..extent * extent)
		.map(|index| {
			let z = contact_shadow_scene_depth(ssgi_ray_at((index % extent) as f32, (index / extent) as f32, extent), wall);
			let depth = if z == 0.0 {
				0.0
			} else {
				(GTAO_NEAR * GTAO_FAR / range) / z - GTAO_NEAR / range
			};
			[depth, 0.0, 0.0, 1.0]
		})
		.collect()
}

/// The ray reach the contact-shadow tests trace with. The wall test's rows are laid out for it.
const CONTACT_SHADOW_TEST_DISTANCE: f32 = 0.3;

/// Runs the contact-shadow trace at one pixel of the floor scene and returns the value it writes: one where the ray
/// toward the light is clear, falling toward zero where it is blocked.
fn run_contact_shadows(wall: bool, direction_to_light: [f32; 3], pixel: [u32; 2]) -> f32 {
	let program = asset!("contact-shadows.besl");
	let extent = CONTACT_SHADOW_EXTENT;
	let [x, y, z] = direction_to_light;
	let length = (x * x + y * y + z * z).sqrt();
	let mut view = gtao_view_data(&program, extent, extent);
	let mut parameters = buffer(&program, ResourceSlot::new(1));
	parameters
		.write("direction_to_light", Value::Vec4F([x / length, y / length, z / length, 0.0]))
		.expect("contact shadow parameters");
	parameters
		.write("max_distance", Value::F32(CONTACT_SHADOW_TEST_DISTANCE))
		.expect("contact shadow parameters");
	let mut depth = texture_2d(extent, extent, &contact_shadow_device_depth(wall));
	let mut output = empty_image(extent, extent);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_buffer(ResourceSlot::new(1), &mut parameters);
	descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
	descriptors.bind_image(ResourceSlot::new(1034), &mut output);
	run_at(&program, &mut descriptors, pixel);
	drop(descriptors);
	rgba(&output, pixel)[0]
}

/// Runs the contact-shadow filter at one pixel of the floor scene with the low wall, over a trace whose value at each
/// pixel is `trace(column, row)`, and returns the filtered value.
fn run_contact_shadow_filter(trace: impl Fn(u32, u32) -> f32, pixel: [u32; 2]) -> f32 {
	let program = asset!("contact-shadows-filter.besl");
	let extent = CONTACT_SHADOW_EXTENT;
	let trace = (0..extent * extent)
		.map(|index| [trace(index % extent, index / extent), 0.0, 0.0, 1.0])
		.collect::<Vec<_>>();
	let mut view = gtao_view_data(&program, extent, extent);
	let mut depth = texture_2d(extent, extent, &contact_shadow_device_depth(true));
	let mut trace = texture_2d(extent, extent, &trace);
	let mut output = empty_image(extent, extent);
	let mut workgroup = WorkgroupState::new();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
	descriptors.bind_texture(ResourceSlot::new(1035), &mut trace);
	descriptors.bind_image(ResourceSlot::new(1034), &mut output);
	descriptors.bind_workgroup_state(&mut workgroup);
	run_tile_workgroup_containing(&program, &mut descriptors, pixel);
	drop(descriptors);
	rgba(&output, pixel)[0]
}

/// Returns the floor's depth at a pixel row of the center column, or zero where that row does not see the floor.
fn contact_shadow_floor_z(row: u32) -> f32 {
	let ray = ssgi_ray_at((CONTACT_SHADOW_EXTENT / 2) as f32, row as f32, CONTACT_SHADOW_EXTENT);
	let z = contact_shadow_scene_depth(ray, true);
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
	let shadowed_rows = rows_at(CONTACT_SHADOW_WALL_Z - 0.09, CONTACT_SHADOW_WALL_Z);
	let fading_rows = rows_at(CONTACT_SHADOW_WALL_Z - 0.18, CONTACT_SHADOW_WALL_Z - 0.12);
	let lit_rows = rows_at(3.0, CONTACT_SHADOW_WALL_Z - 0.3);
	assert!(!shadowed_rows.is_empty() && !fading_rows.is_empty() && !lit_rows.is_empty());

	for row in shadowed_rows {
		let value = run_contact_shadows(true, direction_to_light, [column, row]);
		assert_eq!(
			value,
			0.0,
			"Expected floor row {row} at z={} to be shadowed.",
			contact_shadow_floor_z(row)
		);
	}
	for row in fading_rows {
		let value = run_contact_shadows(true, direction_to_light, [column, row]);
		assert!(
			value > 0.0 && value < 1.0,
			"Expected floor row {row} at z={} to be partly shadowed, got {value}.",
			contact_shadow_floor_z(row)
		);
	}
	for row in lit_rows {
		let value = run_contact_shadows(true, direction_to_light, [column, row]);
		assert_eq!(
			value,
			1.0,
			"Expected floor row {row} at z={} to be lit.",
			contact_shadow_floor_z(row)
		);
	}
}

/// Verifies the contact-shadow filter turns the trace's pixel dither into a smooth value on the floor, and keeps a
/// shadowed floor from darkening the top edge of the wall in front of it.
#[test]
fn contact_shadow_filter_smooths_dither_without_crossing_depth_edges() {
	let column = CONTACT_SHADOW_EXTENT / 2;
	// The floor seven units away lies behind the wall, several rows above it on screen.
	let open_floor_row = (0..CONTACT_SHADOW_EXTENT)
		.find(|&row| (6.9..7.1).contains(&contact_shadow_floor_z(row)))
		.expect("a floor row near seven units");
	let checkerboard = |x: u32, y: u32| ((x + y) % 2) as f32;
	let smoothed = [0, 1].map(|step| run_contact_shadow_filter(checkerboard, [column + step, open_floor_row]));
	assert!(
		smoothed.iter().all(|value| (0.3..0.7).contains(value)) && (smoothed[0] - smoothed[1]).abs() < 0.3,
		"Expected the filter to smooth a checkerboard of zeros and ones, got {smoothed:?}."
	);

	// The wall's top row borders the floor far behind it. Only the floor is shadowed.
	let wall_top_row = (0..CONTACT_SHADOW_EXTENT)
		.find(|&row| {
			contact_shadow_scene_depth(ssgi_ray_at(column as f32, row as f32, CONTACT_SHADOW_EXTENT), true)
				== CONTACT_SHADOW_WALL_Z
		})
		.expect("a wall row");
	let shadowed_floor = |x: u32, y: u32| {
		let on_wall =
			contact_shadow_scene_depth(ssgi_ray_at(x as f32, y as f32, CONTACT_SHADOW_EXTENT), true) == CONTACT_SHADOW_WALL_Z;
		if on_wall { 1.0 } else { 0.0 }
	};
	let wall_edge = run_contact_shadow_filter(shadowed_floor, [column, wall_top_row]);
	assert_eq!(wall_edge, 1.0, "The floor behind the wall darkened the wall's top edge.");
}

/// Verifies every visibility pass compiles with the platform shader compiler, past BESL linking.
///
/// Each entry's settings mirror the asset's `.bead` file.
#[cfg(target_os = "macos")]
#[compio::test]
async fn visibility_assets_lower_to_the_platform_shader_language() {
	use resource_management::shader::ShaderGenerationSettings as Settings;
	use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;
	use utils::Extent;

	let mesh = || Settings::mesh(64, 126, Extent::line(128));
	let tile = || Settings::compute(Extent::square(8));
	for (name, source, settings) in [
		(
			"meshlet_task",
			asset_source!("meshlet-task.besl"),
			Settings::task(Extent::line(32), 192),
		),
		("visibility_mesh", asset_source!("visibility-mesh.besl"), mesh()),
		("shadow_mesh", asset_source!("shadow-mesh.besl"), mesh()),
		(
			"visibility_fragment",
			asset_source!("visibility-fragment.besl"),
			Settings::fragment(),
		),
		("masked_fragment", asset_source!("masked-fragment.besl"), Settings::fragment()),
		(
			"masked_depth_fragment",
			asset_source!("masked-depth-fragment.besl"),
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
		("hiz_seed", asset_source!("hiz-seed.besl"), tile()),
		("hiz_reduce", asset_source!("hiz-reduce.besl"), tile()),
		("contact_shadows", asset_source!("contact-shadows.besl"), tile()),
		("contact_shadow_filter", asset_source!("contact-shadows-filter.besl"), tile()),
		("ssgi_trace", asset_source!("ssgi-trace.besl"), tile()),
		("ssgi_temporal", asset_source!("ssgi-temporal.besl"), tile()),
		("ssgi_upscale", asset_source!("ssgi-upscale.besl"), tile()),
		(
			"light_clusters",
			asset_source!("light-clusters.besl"),
			Settings::compute(Extent::line(super::layout::LIGHT_CLUSTER_MASK_WORDS as u32)),
		),
		(
			"directional_shadow_receiver_bounds",
			asset_source!("directional-shadow-receiver-bounds.besl"),
			Settings::compute(Extent::square(RECEIVER_BOUNDS_WORKGROUP_WIDTH)),
		),
		(
			"directional_shadow_cascade_fit",
			asset_source!("directional-shadow-cascade-fit.besl"),
			Settings::compute(Extent::line(4)),
		),
	] {
		let root = besl::lex(besl::parse(source).unwrap_or_else(|error| panic!("{name} should parse: {error:?}")))
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

/// Runs the light-cluster pass for one cluster and returns its first two mask words.
///
/// The camera sits at the origin and looks down +Z with a 90 degree field of view and a 0.1 m to 100 m clip range.
fn run_light_clusters(lights: &[super::shader_data::LightData], exposure: f32, cluster: u32) -> [u32; 2] {
	let program = asset!("light-clusters.besl");
	let view = crate::rendering::View::new_perspective(
		math::Degrees::new(90.0),
		1.0,
		0.1,
		100.0,
		math::Point::origin(),
		math::UnitVector::z_axis(),
	);
	let parameters = super::shader_data::LightClusterParameters::from(view);
	let mut cluster_parameters = buffer(&program, ResourceSlot::new(1));
	for (field, value) in [
		("view", Value::Mat4x3F(bytemuck::cast(parameters.view))),
		("edge_slopes", Value::Vec2F(parameters.edge_slopes)),
		("near", Value::F32(parameters.near)),
		("depth_slice_scale", Value::F32(parameters.depth_slice_scale)),
	] {
		cluster_parameters.write(field, value).expect("light cluster parameter");
	}
	let mut lighting = buffer(&program, ResourceSlot::new(0));
	lighting
		.write("light_count", Value::U32(lights.len() as u32))
		.expect("light count");
	lighting.write("exposure", Value::F32(exposure)).expect("exposure");
	let vec4 = |vector: super::shader_data::ShaderVec3| Value::Vec4F([vector.x, vector.y, vector.z, 0.0]);
	for (index, light) in lights.iter().enumerate() {
		for (field, value) in [
			("position", vec4(light.position)),
			("color", vec4(light.color)),
			("direction", vec4(light.direction)),
			("cone_cosines", Value::Vec2F(light.cone_cosines)),
			("type", Value::U32(light.light_type)),
			("reach", Value::F32(light.reach)),
		] {
			lighting
				.write_indexed_field("lights", index, field, value)
				.expect("light field");
		}
	}
	let mut masks = buffer(&program, ResourceSlot::new(1033));
	let configs: [ExecutionConfig; 32] = std::array::from_fn(|lane| {
		ExecutionConfig::new(INSTRUCTION_LIMIT)
			.with_call_depth_limit(128)
			.with_thread_idx(lane as u32)
			.with_threadgroup_position(cluster)
	});
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(ResourceSlot::new(0), &mut lighting);
	descriptors.bind_buffer(ResourceSlot::new(1), &mut cluster_parameters);
	descriptors.bind_buffer(ResourceSlot::new(1033), &mut masks);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("Failed to run the light-cluster pass in the BESL VM.");
	drop(descriptors);
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
	let mut lights = vec![
		// In front of the camera, 20 m away.
		uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, 20.0), forward),
		// Behind the camera, 20 m away.
		uploaded_light(light_cluster_fixture_light(false), Point::new(0.0, 0.0, -20.0), forward),
		uploaded_light(sun, Point::origin(), -UnitVector::y_axis()),
		// Cones 5 m behind the camera, facing away from and toward the view.
		uploaded_light(light_cluster_fixture_light(true), Point::new(0.0, 0.0, -5.0), -forward),
		uploaded_light(light_cluster_fixture_light(true), Point::new(0.0, 0.0, -5.0), forward),
	];
	// Lights without reach fill the rest of the first mask word, so the last light lands in the second word.
	lights.resize(40, super::shader_data::LightData::default());
	lights.push(uploaded_light(
		light_cluster_fixture_light(false),
		Point::new(0.0, 0.0, 20.0),
		forward,
	));

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

const RECEIVER_BOUNDS_SLOT: ResourceSlot = ResourceSlot::new(1034);
const RECEIVER_FIT_SLOT: ResourceSlot = ResourceSlot::new(1035);
const CASCADE_SIZE_STEPS_SLOT: ResourceSlot = ResourceSlot::new(1036);
const RECEIVER_BOUNDS_WORKGROUP_WIDTH: u32 = 8;
const RECEIVER_BOUNDS_WORKGROUP_SIZE: usize = 64;
/// Pixels one receiver-bounds workgroup covers on each side: eight threads of four pixels.
const RECEIVER_BOUNDS_TILE: u32 = 32;
const RECEIVER_FIT_EXTENT: u32 = 64;
const RECEIVER_FIT_WALL_Z: f32 = 20.0;
const RECEIVER_FIT_WALL_HEIGHT: f32 = 4.0;

/// A camera 1.7 m above a floor, looking a little down toward a 4 m wall 20 m ahead, with sky above the wall, under a
/// diagonal sun.
struct ReceiverFitScene {
	cascades: [crate::rendering::csm::CascadeFrame; super::layout::SHADOW_CASCADE_COUNT],
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
	let sun = math::Vector::new(0.5, -1.0, 0.3).normalized().expect("sun direction");
	let cascades = csm::make_cascade_frames(
		camera,
		sun,
		super::layout::SHADOW_CASCADE_COUNT,
		super::layout::SHADOW_MAP_RESOLUTION,
		csm::CascadeSplits::default(),
	)
	.collect::<smallvec::SmallVec<[_; 4]>>()
	.into_inner()
	.expect("four cascades");
	let extent = utils::Extent::square(RECEIVER_FIT_EXTENT);
	let screen = super::render_pass::screen_view_data(&Sink::new(camera, extent, 0), extent);
	let shader_data = super::render_pass::receiver_fit_shader_data(screen, camera, &cascades);

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
		cascades,
		shader_data,
		device_depth,
		receivers,
	}
}

fn receiver_fit_buffer(program: &ExecutableProgram, data: &super::render_pass::ReceiverFitShaderData) -> besl::vm::Buffer {
	let mut fit = buffer(program, RECEIVER_FIT_SLOT);
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
	fit
}

/// Returns views holding the CPU's frustum-fitted cascades after the camera.
fn cascade_views(program: &ExecutableProgram, scene: &ReceiverFitScene) -> besl::vm::Buffer {
	let mut views = buffer(program, VIEWS_SLOT);
	for (cascade, frame) in scene.cascades.iter().enumerate() {
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
	views
}

/// Runs the receiver-bounds pass over the whole scene and returns the bounds it found.
fn run_receiver_bounds(scene: &ReceiverFitScene) -> besl::vm::Buffer {
	let program = asset!("directional-shadow-receiver-bounds.besl");
	let mut depth = texture_2d(RECEIVER_FIT_EXTENT, RECEIVER_FIT_EXTENT, &scene.device_depth);
	let mut fit = receiver_fit_buffer(&program, &scene.shader_data);
	let mut bounds = buffer(&program, RECEIVER_BOUNDS_SLOT);
	let tiles = RECEIVER_FIT_EXTENT / RECEIVER_BOUNDS_TILE;
	for tile in 0..tiles * tiles {
		let base = [
			tile % tiles * RECEIVER_BOUNDS_WORKGROUP_WIDTH,
			tile / tiles * RECEIVER_BOUNDS_WORKGROUP_WIDTH,
		];
		let configs = tile_configs::<RECEIVER_BOUNDS_WORKGROUP_SIZE>(RECEIVER_BOUNDS_WORKGROUP_WIDTH, base);
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
		descriptors.bind_buffer(RECEIVER_BOUNDS_SLOT, &mut bounds);
		descriptors.bind_buffer(RECEIVER_FIT_SLOT, &mut fit);
		descriptors.bind_workgroup_state(&mut workgroup);
		program
			.run_workgroup(&mut descriptors, &configs)
			.expect("receiver-bounds workgroup execution");
	}
	bounds
}

/// Runs the cascade fit on `bounds` over the scene's frustum-fitted views and returns the views it wrote.
fn run_cascade_fit(
	program: &ExecutableProgram,
	scene: &ReceiverFitScene,
	bounds: &mut besl::vm::Buffer,
	size_steps: &mut besl::vm::Buffer,
) -> besl::vm::Buffer {
	let mut views = cascade_views(program, scene);
	let mut fit = receiver_fit_buffer(program, &scene.shader_data);
	let configs: [ExecutionConfig; 4] = std::array::from_fn(|lane| {
		ExecutionConfig::new(INSTRUCTION_LIMIT)
			.with_call_depth_limit(128)
			.with_thread_idx(lane as u32)
			.with_thread_id([lane as u32, 0])
	});
	let mut workgroup = WorkgroupState::new();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut views);
	descriptors.bind_buffer(RECEIVER_BOUNDS_SLOT, bounds);
	descriptors.bind_buffer(RECEIVER_FIT_SLOT, &mut fit);
	descriptors.bind_buffer(CASCADE_SIZE_STEPS_SLOT, size_steps);
	descriptors.bind_workgroup_state(&mut workgroup);
	program
		.run_workgroup(&mut descriptors, &configs)
		.expect("cascade-fit workgroup execution");
	drop(descriptors);
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
	let mut size_steps = buffer(&program, CASCADE_SIZE_STEPS_SLOT);
	let views = run_cascade_fit(&program, &scene, &mut bounds, &mut size_steps);
	let resolution = super::layout::SHADOW_MAP_RESOLUTION as f32;

	for (cascade, frame) in scene.cascades.iter().enumerate() {
		let view_projection = read_matrix(&views, 1 + cascade, "view_projection");
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
				"Cascade {cascade} has no receivers but changed. The most likely cause is that the fit rewrote an empty cascade."
			);
			continue;
		}
		assert!(
			half_extent <= frame.half_extent * 1.0001,
			"Cascade {cascade} grew from {} to {half_extent} meters. The most likely cause is that the fit is not limited by the frustum fit.",
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
				"Cascade {cascade} receiver {point:?} lies at {ndc:?}, inside the edge margin. The most likely cause is that the receiver bounds or the fit's padding are wrong."
			);
			assert!(
				(-0.0001..=1.0001 - CASTER_REACH / depth_range).contains(&ndc[2]),
				"Cascade {cascade} receiver {point:?} lies at depth {} of 0..{}. The most likely cause is that the fit's depth range misses its receivers or its caster reach.",
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
				"Cascade {cascade} puts the world origin at texel {texel}. The most likely cause is that the fit does not snap to the world-fixed texel grid."
			);
		}

		// The light's position is the center of the view's light-facing side, and the view inverts the inverse view.
		let inverse_view = read_matrix(&views, 1 + cascade, "inverse_view");
		let light_position = [inverse_view[9], inverse_view[10], inverse_view[11]];
		let light_ndc = transform_point(&view_projection, 4, light_position);
		assert!(
			light_ndc[0].abs() < 0.0001 && light_ndc[1].abs() < 0.0001 && (light_ndc[2] - 1.0).abs() < 0.0001,
			"Cascade {cascade} light position maps to {light_ndc:?}. The most likely cause is that the inverse view did not move with the view-projection."
		);
		let view = read_matrix(&views, 1 + cascade, "view");
		let light_view_position = transform_point(&view, 3, light_position);
		assert!(
			light_view_position.iter().all(|coordinate| coordinate.abs() < 0.001),
			"Cascade {cascade} view maps its own light position to {light_view_position:?}. The most likely cause is that the view did not move with the inverse view."
		);
	}

	// The camera sees the floor from about a meter away, so the first cascade drops the empty space in front of it.
	let first = fitted_half_extent(&read_matrix(&views, 1, "view_projection"));
	assert!(
		first < scene.cascades[0].half_extent * 0.8,
		"The first cascade only shrank from {} to {first} meters. The most likely cause is that the receiver bounds span the whole frustum slice.",
		scene.cascades[0].half_extent
	);
}

/// Returns bounds holding one box, centered in the frustum fit, `half_size` normalized device units wide in x and y.
fn box_bounds(program: &ExecutableProgram, half_size: f32) -> besl::vm::Buffer {
	const STEPS_PER_UNIT: f32 = 4_194_304.0;
	const OFFSET: f32 = 8_388_608.0;
	let lower = |coordinate: f32| 16_777_216 - ((coordinate * STEPS_PER_UNIT).floor() + OFFSET) as u32;
	let upper = |coordinate: f32| ((coordinate * STEPS_PER_UNIT).ceil() + OFFSET) as u32;
	let mut bounds = buffer(program, RECEIVER_BOUNDS_SLOT);
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
	let mut size_steps = buffer(&program, CASCADE_SIZE_STEPS_SLOT);
	let mut fit = |half_size: f32| {
		let views = run_cascade_fit(&program, &scene, &mut box_bounds(&program, half_size), &mut size_steps);
		fitted_half_extent(&read_matrix(&views, 1, "view_projection"))
	};

	let fitted = fit(0.5);
	assert!(
		fitted < scene.cascades[0].half_extent,
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
