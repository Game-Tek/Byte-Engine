//! Runs the surfel GI passes in the BESL VM on small pools: the grid, the economy, the ray grants, the support check,
//! the hit sort, the shade pass, the estimate update, the allocators, the resolve and its temporal accumulation, and
//! the spatial pass.
//!
//! Every fixture centers the grid and the camera on the origin, so the cascades follow the Reference's grid rule
//! directly.

use std::sync::Arc;

use besl::vm::{
	AccelerationStructure, BottomLevelStructure, Buffer, DescriptorBindings, ExecutableProgram, Instance, RayTracingPipeline,
	ResourceSlot, Texture, Value,
};
use half::f16;

use super::{
	SSGI_WALL_NORMAL, TILE_WORKGROUP_SIZE, TILE_WORKGROUP_WIDTH, VIEWS_SLOT, asset_program, read_u32, run_workgroup,
	run_workgroup_containing, screen_view_buffer, ssgi_projection, ssgi_ray_at,
};
use crate::rendering::shader_vm_test::{
	IDENTITY_MATRIX, array_buffer, assert_rgba_close, buffer, column_major, compile, empty_image, lane_config, rgba,
	stacked_asset, stacked_on_layers_of, texture_2d,
};

const DEPTH_PYRAMID_SLOT: ResourceSlot = ResourceSlot::new(1033);
const PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1120);
const RECORDS_SLOT: ResourceSlot = ResourceSlot::new(1121);
const STATES_SLOT: ResourceSlot = ResourceSlot::new(1122);
const FREE_SLOT: ResourceSlot = ResourceSlot::new(1123);
const POOL_SLOT: ResourceSlot = ResourceSlot::new(1124);
const CELLS_SLOT: ResourceSlot = ResourceSlot::new(1125);
const SCAN_SLOT: ResourceSlot = ResourceSlot::new(1126);
const PARTIALS_SLOT: ResourceSlot = ResourceSlot::new(1127);
const ENTRIES_SLOT: ResourceSlot = ResourceSlot::new(1128);
const RAY_WANTS_SLOT: ResourceSlot = ResourceSlot::new(1129);
const IRRADIANCE_SLOT: ResourceSlot = ResourceSlot::new(1130);
const RAYS_SLOT: ResourceSlot = ResourceSlot::new(1131);
const RAY_RESULTS_SLOT: ResourceSlot = ResourceSlot::new(1132);
const SCENE_LIGHTS_SLOT: ResourceSlot = ResourceSlot::new(1133);
const HIT_SPAWNS_SLOT: ResourceSlot = ResourceSlot::new(1134);
const SCENE_STRUCTURE_SLOT: ResourceSlot = ResourceSlot::new(1140);
// The ray hits set the trace and shade passes share with probe GI.
const HITS_SLOT: ResourceSlot = ResourceSlot::new(1145);
const HIT_KEYS_SLOT: ResourceSlot = ResourceSlot::new(1146);
const HIT_SORT_SLOT: ResourceSlot = ResourceSlot::new(1147);
const SORTED_HITS_SLOT: ResourceSlot = ResourceSlot::new(1148);
const GUIDES_SLOT: ResourceSlot = ResourceSlot::new(1138);
const DEPTHS_SLOT: ResourceSlot = ResourceSlot::new(1139);
const CLEARANCES_SLOT: ResourceSlot = ResourceSlot::new(1143);
const ENVIRONMENT_RADIANCE_SLOT: ResourceSlot = ResourceSlot::new(1144);
/// Each surfel's guide cells and radial depth cells.
const GUIDE_CELLS: usize = 64;
const DEPTH_CELLS: usize = 16;
const RESOLVE_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1150);
const SPAWNS_SLOT: ResourceSlot = ResourceSlot::new(1151);
const NORMALS_SLOT: ResourceSlot = ResourceSlot::new(1153);
// The resolve's temporal accumulation set.
const PREVIOUS_HISTORY_SLOT: ResourceSlot = ResourceSlot::new(1222);
const PREVIOUS_WEIGHTS_SLOT: ResourceSlot = ResourceSlot::new(1223);
const PREVIOUS_DEPTH_SLOT: ResourceSlot = ResourceSlot::new(1224);
const PREVIOUS_NORMALS_SLOT: ResourceSlot = ResourceSlot::new(1226);
const HISTORY_OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1227);
const WEIGHTS_OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1228);
// The spatial pass's own set.
const SPATIAL_HISTORY_SLOT: ResourceSlot = ResourceSlot::new(1230);
const SPATIAL_WEIGHTS_SLOT: ResourceSlot = ResourceSlot::new(1231);
const SPATIAL_NORMALS_SLOT: ResourceSlot = ResourceSlot::new(1232);
const OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1233);

/// The pool slots of the fixture pools. The passes read the capacity from the parameters, so a small pool works.
const CAPACITY: u32 = 128;
/// 8 cascades of 32 × 32 × 32 cells.
const CELL_COUNT: u32 = 262_144;
/// The most cells the grid lists one surfel in.
const LISTING_LIMIT: u32 = 32;
const SCAN_GROUP_ELEMENTS: u32 = 1024;
/// The workgroup of every one-dimensional surfel pass.
const WORKGROUP_SIZE: u32 = 256;
const NO_SURFEL: u32 = u32::MAX;
const TTL: u32 = 500;
const KILL: u32 = 255;
/// The fixture pools' ray budget, which covers every young surfel of [`CAPACITY`]'s worth of tests.
const RAY_BUDGET: u32 = 1024;
/// The rays a settled surfel traces each frame, and a young one.
const SETTLED_RAYS: u32 = 4;
const YOUNG_RAYS: u32 = 32;
/// A surfel that received this many rays is mature: lookups fully trust it.
const MATURE_RAYS: f32 = 128.0;
/// A surfel traces [`YOUNG_RAYS`] a frame until it received this many rays, and [`SETTLED_RAYS`] after.
const WARM_UP_RAYS: f32 = 512.0;
/// The rays the fixtures' settled surfels have received, well past [`WARM_UP_RAYS`].
const SETTLED_RECEIVED: f32 = 1000.0;
/// The threads that walk one surfel's cells in the count pass and the scatter. Matches `SURFEL_GRID_LANES` in
/// `layouts/surfel-gi.besl`.
const GRID_LANES: u32 = 4;
/// The most spawn requests rays' hits make in a frame.
const MAX_HIT_SPAWNS: usize = 64;
/// A hit's sort key keeps its rank among the hits on its material in this many low bits.
const HIT_RANK_BITS: u32 = 20;
/// A surfel's radius within 3.2 m of the camera.
const NEAR_RADIUS: f32 = 0.24;
/// Lookups weigh a surfel out to this multiple of its radius.
const LOOKUP_SCALE: f32 = 1.25;
/// The resolve fixtures' half-resolution image: one 8x8 resolve workgroup, and one spawn tile.
const RESOLVE_EXTENT: u32 = 8;
/// The distance from the camera to the wall the resolve fixtures see.
const WALL_DEPTH: f32 = 1.0;
/// The normal of the wall, which faces the camera.
const WALL_NORMAL: [f32; 3] = [0.0, 0.0, -1.0];
const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// Runs the production material reconstruction against a covered planar face at half resolution.
fn reconstruct_covered_face(normal: [f32; 3], geometric: [f32; 3], stored: [f32; 4]) -> [f32; 4] {
	use super::super::shader_generator::{
		INDIRECT_DIFFUSE_SOURCE_POSITION_SOURCE, SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE, SSGI_STORED_NORMAL_SOURCE,
		SSGI_SURFACE_WEIGHT_SOURCE, WEIGH_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE,
	};
	const RESULT: ResourceSlot = ResourceSlot::new(0);
	const VIEW: ResourceSlot = ResourceSlot::new(1048);
	let (_, layout) = crate::rendering::set_layout::layer!("visibility/layouts/material-evaluation.besl");
	let source = format!(
		r#"
		{layout}
		{SSGI_STORED_NORMAL_SOURCE}
		{SSGI_SURFACE_WEIGHT_SOURCE}
		{INDIRECT_DIFFUSE_SOURCE_POSITION_SOURCE}
		{WEIGH_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE}
		{SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE}
		ReconstructionResult: struct {{ light: vec4f }}
		results: descriptor<{{ type: ReconstructionResult, binding: 0, access: write, memory: device }}>;
		main: fn () -> void {{
			results.light = sample_screen_space_indirect_diffuse(
				vec2u(16, 16), vec2u(32, 32), vec3f(0.0, 0.0, 1.0),
				vec3f({}, {}, {}), vec3f({}, {}, {}), true
			);
		}}
		"#,
		normal[0], normal[1], normal[2], geometric[0], geometric[1], geometric[2],
	);
	let program = compile(besl::lex(besl::parse(&source).expect("reconstruction fixture")).expect("reconstruction shader"));
	let mut view = buffer(&program, VIEW);
	view.write("pixel_to_ray_mul", Value::Vec2F([0.01, -0.01]))
		.expect("view scale");
	view.write("pixel_to_ray_add", Value::Vec2F([-0.075, 0.075]))
		.expect("view offset");
	let mut depth = texture_2d(16, 16, &[[1.0, 0.0, 0.0, 1.0]; 256]);
	let mut normals = texture_2d(16, 16, &[stored; 256]);
	let mut history = texture_2d(16, 16, &[[0.3125, 0.15625, 0.0625, 1.0]; 256]);
	let mut result = buffer(&program, RESULT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEW, &mut view);
	descriptors.bind_texture(ResourceSlot::new(1060), &mut depth);
	descriptors.bind_texture(ResourceSlot::new(1047), &mut normals);
	descriptors.bind_texture(ResourceSlot::new(1056), &mut history);
	descriptors.bind_buffer(RESULT, &mut result);
	crate::rendering::shader_vm_test::run_at(&program, &mut descriptors, [0, 0]);
	drop(descriptors);
	match result.read("light").expect("reconstructed light") {
		Value::Vec4F(value) => value,
		value => panic!("Unexpected reconstructed light: {value:?}."),
	}
}

/// A covered ornamental facet must retain its GI and occlusion even when its smoothed vertex normal differs from
/// its triangle plane by more than the reconstruction's normal gate. The native lion census measured such faces.
#[test]
fn surfel_reconstruction_recovers_a_covered_facet_with_a_smoothed_normal() {
	assert_rgba_close(
		reconstruct_covered_face([0.9, 0.0, -0.4358899], WALL_NORMAL, SSGI_WALL_NORMAL),
		[0.3125, 0.15625, 0.0625, 1.0],
		0.001,
	);
}

/// Retrying the face plane must not replace a reconstruction that already has valid light and hit information.
#[test]
fn surfel_reconstruction_preserves_an_already_reconstructed_surface() {
	assert_rgba_close(
		reconstruct_covered_face(WALL_NORMAL, UP, SSGI_WALL_NORMAL),
		[0.3125, 0.15625, 0.0625, 1.0],
		0.001,
	);
}

/// A surface that no history texel matches in either normal, such as a rod or a bevel thinner than a texel, takes
/// the light and hit fraction of the nearest tap. An empty result would count every ray as missed and show the
/// unoccluded sky's reflection as a thin white line.
#[test]
fn surfel_reconstruction_takes_the_nearest_tap_for_an_unmatched_surface() {
	assert_rgba_close(
		reconstruct_covered_face(WALL_NORMAL, WALL_NORMAL, [0.0, 1.0, 0.0, 0.0]),
		[0.3125, 0.15625, 0.0625, 1.0],
		0.001,
	);
}

/// The frame-wide surfel passes, in the order a frame records them.
struct SurfelPasses {
	count: ExecutableProgram,
	scan_groups: ExecutableProgram,
	scan_partials: ExecutableProgram,
	scan_add: ExecutableProgram,
	scatter: ExecutableProgram,
	validate: ExecutableProgram,
	sort_scan: ExecutableProgram,
	sort_scatter: ExecutableProgram,
	integrate: ExecutableProgram,
	allocate_hits: ExecutableProgram,
}

impl SurfelPasses {
	fn new() -> Self {
		Self {
			count: asset!("surfel-count.besl"),
			scan_groups: asset!("surfel-scan-groups.besl"),
			scan_partials: asset!("surfel-scan-partials.besl"),
			scan_add: asset!("surfel-scan-add.besl"),
			scatter: asset!("surfel-scatter.besl"),
			validate: asset!("surfel-validate.besl"),
			sort_scan: asset!("ray-hit-sort-scan.besl"),
			sort_scatter: asset!("ray-hit-sort-scatter.besl"),
			integrate: asset!("surfel-integrate.besl"),
			allocate_hits: asset!("surfel-allocate-hits.besl"),
		}
	}

	/// Runs one frame of the grid: the economy and cell counts, the three scan passes, and the scatter.
	fn run_grid(&self, set: &mut SurfelSet) {
		let scan_threads = scan_group_count() * WORKGROUP_SIZE;
		dispatch(&self.count, set, CAPACITY * GRID_LANES);
		dispatch(&self.scan_groups, set, scan_threads);
		dispatch(&self.scan_partials, set, WORKGROUP_SIZE);
		dispatch(&self.scan_add, set, scan_threads);
		dispatch(&self.scatter, set, CAPACITY * GRID_LANES);
	}

	/// Runs the support check of frame `frame_index` against `scene`, one thread per eight pool slots.
	fn validate(&self, set: &mut SurfelSet, scene: &AccelerationStructure, frame_index: u32) {
		set.parameters
			.write("frame_index", Value::U32(frame_index))
			.expect("frame index");
		let configs: Vec<_> = (0..WORKGROUP_SIZE)
			.map(|lane| lane_config(lane).with_thread_id([lane, 0]))
			.collect();
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		descriptors.bind_acceleration_structure(SCENE_STRUCTURE_SLOT, scene);
		run_workgroup(&self.validate, descriptors, &configs);
	}
}

/// The scan's workgroups: every cell, every slot's wanted priority and regular rays, and the grand total, 1024 elements
/// per workgroup.
fn scan_group_count() -> u32 {
	(CELL_COUNT + CAPACITY * 2 + 1).div_ceil(SCAN_GROUP_ELEMENTS)
}

/// Runs `program` over `threads` threads in workgroups of [`WORKGROUP_SIZE`] lanes, as the frame dispatches the
/// one-dimensional surfel passes.
fn dispatch(program: &ExecutableProgram, set: &mut SurfelSet, threads: u32) {
	for group in 0..threads.div_ceil(WORKGROUP_SIZE) {
		let configs: Vec<_> = (0..WORKGROUP_SIZE)
			.map(|lane| {
				lane_config(lane)
					.with_thread_id([group * WORKGROUP_SIZE + lane, 0])
					.with_threadgroup_position(group)
			})
			.collect();
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		run_workgroup(program, descriptors, &configs);
	}
}

/// One frame-wide surfel set in VM memory, for a grid and a camera at the origin.
struct SurfelSet {
	parameters: Buffer,
	records: Buffer,
	states: Buffer,
	free: Buffer,
	pool: Buffer,
	cells: Buffer,
	scan: Buffer,
	partials: Buffer,
	entries: Buffer,
	ray_wants: Buffer,
	irradiance: Buffer,
	rays: Buffer,
	ray_results: Buffer,
	hit_keys: Buffer,
	hit_sort: Buffer,
	sorted_hits: Buffer,
	guides: Buffer,
	depths: Buffer,
	clearances: Buffer,
	hit_spawns: Buffer,
	/// The environment's specular levels, the same in every direction.
	environment_radiance: Texture,
}

impl SurfelSet {
	/// Creates an empty pool of [`CAPACITY`] slots, laid out as `passes` declare it.
	fn new(passes: &SurfelPasses) -> Self {
		let capacity = CAPACITY as usize;
		let groups = scan_group_count() as usize;
		let mut parameters = buffer(&passes.count, PARAMETERS_SLOT);
		for (member, value) in [
			("grid_origin", Value::Vec3F([0.0; 3])),
			("surfel_capacity", Value::U32(CAPACITY)),
			("camera_position", Value::Vec3F([0.0; 3])),
			("ray_budget", Value::U32(RAY_BUDGET)),
			("exposure", Value::F32(1.0)),
			("previous_exposure", Value::F32(1.0)),
			("environment_intensity", Value::F32(1.0)),
		] {
			parameters.write(member, value).expect("surfel parameters");
		}
		Self {
			parameters,
			records: array_buffer(&passes.count, RECORDS_SLOT, capacity),
			states: array_buffer(&passes.count, STATES_SLOT, capacity),
			free: array_buffer(&passes.count, FREE_SLOT, capacity),
			pool: buffer(&passes.count, POOL_SLOT),
			cells: buffer(&passes.count, CELLS_SLOT),
			// The group pass writes whole groups, so the scan holds every element of its last group.
			scan: array_buffer(&passes.count, SCAN_SLOT, groups * SCAN_GROUP_ELEMENTS as usize),
			partials: array_buffer(&passes.scan_groups, PARTIALS_SLOT, groups),
			entries: array_buffer(&passes.scatter, ENTRIES_SLOT, capacity * LISTING_LIMIT as usize),
			// Each slot's wanted priority rays, then its wanted regular rays.
			ray_wants: array_buffer(&passes.count, RAY_WANTS_SLOT, capacity * 2),
			irradiance: array_buffer(&passes.integrate, IRRADIANCE_SLOT, capacity),
			rays: array_buffer(&passes.scatter, RAYS_SLOT, RAY_BUDGET as usize),
			ray_results: array_buffer(&passes.integrate, RAY_RESULTS_SLOT, RAY_BUDGET as usize),
			hit_keys: array_buffer(&passes.sort_scatter, HIT_KEYS_SLOT, RAY_BUDGET as usize),
			hit_sort: buffer(&passes.sort_scan, HIT_SORT_SLOT),
			sorted_hits: array_buffer(&passes.sort_scatter, SORTED_HITS_SLOT, RAY_BUDGET as usize),
			guides: array_buffer(&passes.integrate, GUIDES_SLOT, capacity * GUIDE_CELLS),
			depths: array_buffer(&passes.integrate, DEPTHS_SLOT, capacity * DEPTH_CELLS),
			clearances: array_buffer(&passes.integrate, CLEARANCES_SLOT, capacity),
			hit_spawns: array_buffer(&passes.allocate_hits, HIT_SPAWNS_SLOT, MAX_HIT_SPAWNS),
			environment_radiance: texture_2d(1, 1, &[[0.0; 4]]),
		}
	}

	fn bind<'a>(&'a mut self, descriptors: &mut DescriptorBindings<'a>) {
		descriptors.bind_buffer(PARAMETERS_SLOT, &mut self.parameters);
		descriptors.bind_buffer(RECORDS_SLOT, &mut self.records);
		descriptors.bind_buffer(STATES_SLOT, &mut self.states);
		descriptors.bind_buffer(FREE_SLOT, &mut self.free);
		descriptors.bind_buffer(POOL_SLOT, &mut self.pool);
		descriptors.bind_buffer(CELLS_SLOT, &mut self.cells);
		descriptors.bind_buffer(SCAN_SLOT, &mut self.scan);
		descriptors.bind_buffer(PARTIALS_SLOT, &mut self.partials);
		descriptors.bind_buffer(ENTRIES_SLOT, &mut self.entries);
		descriptors.bind_buffer(RAY_WANTS_SLOT, &mut self.ray_wants);
		descriptors.bind_buffer(IRRADIANCE_SLOT, &mut self.irradiance);
		descriptors.bind_buffer(RAYS_SLOT, &mut self.rays);
		descriptors.bind_buffer(RAY_RESULTS_SLOT, &mut self.ray_results);
		descriptors.bind_buffer(HIT_KEYS_SLOT, &mut self.hit_keys);
		descriptors.bind_buffer(HIT_SORT_SLOT, &mut self.hit_sort);
		descriptors.bind_buffer(SORTED_HITS_SLOT, &mut self.sorted_hits);
		descriptors.bind_buffer(GUIDES_SLOT, &mut self.guides);
		descriptors.bind_buffer(DEPTHS_SLOT, &mut self.depths);
		descriptors.bind_buffer(CLEARANCES_SLOT, &mut self.clearances);
		descriptors.bind_buffer(HIT_SPAWNS_SLOT, &mut self.hit_spawns);
		descriptors.bind_texture(ENVIRONMENT_RADIANCE_SLOT, &mut self.environment_radiance);
	}

	/// Returns the guide cells of `slot`.
	fn guide(&self, slot: u32) -> Vec<f32> {
		(0..GUIDE_CELLS)
			.map(
				|cell| match self.guides.read_array_element(slot as usize * GUIDE_CELLS + cell) {
					Ok(Value::F16(value)) => value.to_f32(),
					value => panic!("Unexpected guide cell: {value:?}."),
				},
			)
			.collect()
	}

	/// Returns the radial depth cell `cell` of `slot`.
	fn depth(&self, slot: u32, cell: usize) -> [f32; 4] {
		vec4(
			self.depths
				.read_array_element(slot as usize * DEPTH_CELLS + cell)
				.expect("radial depth cell"),
		)
	}

	/// Fills every radial depth cell of `slot` as if all its rays went `distance`.
	fn set_depths(&mut self, slot: u32, distance: f32) {
		for cell in 0..DEPTH_CELLS {
			self.depths
				.write_array_element(slot as usize * DEPTH_CELLS + cell, Value::Vec4F(moments(distance)))
				.expect("radial depth cell");
		}
	}

	/// Places a newly spawned surfel in `slot` that faces `normal`, fully trusted and without light. Its estimate holds
	/// no rays yet, so it traces as a young surfel.
	fn place(&mut self, slot: u32, position: [f32; 3], normal: [f32; 3]) {
		for (member, value) in [
			("position", Value::Vec3F(position)),
			("radius", Value::F32(surfel_radius(position))),
			("normal", Value::U32(encode_normal(normal))),
			("weight", Value::F32(1.0)),
			("irradiance", Value::Vec4F16([f16::ZERO; 4])),
		] {
			self.records
				.write_array_member(slot as usize, member, value)
				.expect("surfel record");
		}
		for (member, value) in [("life", 1), ("touched", 0), ("cell", NO_SURFEL)] {
			self.set_state(slot, member, value);
		}
	}

	/// Returns the ray records the budget granted this frame, as (slot, sample) pairs in ray order.
	fn granted_rays(&self) -> Vec<(u32, u32)> {
		(0..(self.pool_count("priority_rays") + self.pool_count("regular_rays")) as usize)
			.map(|ray| {
				let record = read_u32(&self.rays, ray);
				(record & 0x00FF_FFFF, record >> 24)
			})
			.collect()
	}

	fn set_estimate(&mut self, slot: u32, member: &str, value: Value) {
		self.irradiance
			.write_array_member(slot as usize, member, value)
			.expect("surfel estimate");
	}

	fn estimate(&self, slot: u32, member: &str) -> Value {
		self.irradiance
			.read_array_member(slot as usize, member)
			.expect("surfel estimate")
	}

	fn set_record(&mut self, slot: u32, member: &str, value: Value) {
		self.records
			.write_array_member(slot as usize, member, value)
			.expect("surfel record");
	}

	fn set_state(&mut self, slot: u32, member: &str, value: u32) {
		self.states
			.write_array_member(slot as usize, member, Value::U32(value))
			.expect("surfel state");
	}

	fn state(&self, slot: u32, member: &str) -> u32 {
		match self.states.read_array_member(slot as usize, member).expect("surfel state") {
			Value::U32(value) => value,
			value => panic!("Unexpected surfel state value: {value:?}."),
		}
	}

	fn record(&self, slot: u32, member: &str) -> Value {
		self.records.read_array_member(slot as usize, member).expect("surfel record")
	}

	fn pool_count(&self, member: &str) -> u32 {
		match self.pool.read(member).expect("surfel pool count") {
			Value::U32(value) => value,
			value => panic!("Unexpected surfel pool value: {value:?}."),
		}
	}

	fn set_pool_count(&mut self, member: &str, value: u32) {
		self.pool.write(member, Value::U32(value)).expect("surfel pool count");
	}

	/// Returns the slots on the free stack, bottom first.
	fn free_slots(&self) -> Vec<u32> {
		(0..self.pool_count("free_count") as usize)
			.map(|index| read_u32(&self.free, index))
			.collect()
	}

	/// Returns the slots the grid lists in `cell`, in the order of its list.
	fn listed(&self, cell: u32) -> Vec<u32> {
		let start = read_u32(&self.scan, cell as usize);
		let end = read_u32(&self.scan, cell as usize + 1);
		(start..end).map(|entry| read_u32(&self.entries, entry as usize)).collect()
	}

	/// Makes the surfel in `slot` settled: its estimate holds [`SETTLED_RECEIVED`] rays, so it traces [`SETTLED_RAYS`]
	/// a frame.
	fn settle(&mut self, slot: u32) {
		self.set_estimate(slot, "rays", Value::F32(SETTLED_RECEIVED));
	}

	fn cell_count(&self, cell: u32) -> u32 {
		match self.cells.read_indexed("counts", cell as usize).expect("surfel cell count") {
			Value::U32(value) => value,
			value => panic!("Unexpected surfel cell count: {value:?}."),
		}
	}
}

/// Returns the radius of a surfel at `position` for a camera at the origin: the near radius, growing past 3.2 m.
fn surfel_radius(position: [f32; 3]) -> f32 {
	NEAR_RADIUS * (length(position) / 3.2).max(1.0)
}

/// Returns the grid cell that holds `position`: the finest cascade that covers it, indexed x + 32y + 32²z + 32³c.
fn cell_of(position: [f32; 3]) -> u32 {
	let cell = position.map(|coordinate| (coordinate / 0.2).floor());
	let reach = cell.iter().fold(0.0f32, |reach, cell| reach.max((cell + 0.5).abs()));
	let cascade = (reach / 16.0).log2().max(0.0).ceil().min(7.0) as u32;
	let scale = (1u32 << cascade) as f32;
	let [x, y, z] = cell.map(|cell| ((cell / scale).floor() + 16.0).clamp(0.0, 31.0) as u32);
	x + y * 32 + z * 32 * 32 + cascade * 32 * 32 * 32
}

/// Packs a unit normal the way spawn requests carry it: two octahedral 16-bit coordinates.
fn encode_normal(normal: [f32; 3]) -> u32 {
	let length_l1 = normal.iter().map(|coordinate| coordinate.abs()).sum::<f32>();
	let mut projected = [normal[0] / length_l1, normal[1] / length_l1];
	if normal[2] < 0.0 {
		let sign = |value: f32| if value >= 0.0 { 1.0 } else { -1.0 };
		projected = [
			(1.0 - projected[1].abs()) * sign(projected[0]),
			(1.0 - projected[0].abs()) * sign(projected[1]),
		];
	}
	let quantize = |value: f32| ((value * 0.5 + 0.5).clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
	quantize(projected[0]) | (quantize(projected[1]) << 16)
}

/// Returns the first four moments of `distance`, as a radial depth cell holds them.
fn moments(distance: f32) -> [f32; 4] {
	[distance, distance.powi(2), distance.powi(3), distance.powi(4)]
}

fn length(vector: [f32; 3]) -> f32 {
	vector.iter().map(|coordinate| coordinate * coordinate).sum::<f32>().sqrt()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
	length([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[
		a[1] * b[2] - a[2] * b[1],
		a[2] * b[0] - a[0] * b[2],
		a[0] * b[1] - a[1] * b[0],
	]
}

/// Returns a disc's center and four points `radius` from it along two directions in the disc's plane.
fn disc_points(center: [f32; 3], normal: [f32; 3], radius: f32) -> [[f32; 3]; 5] {
	let axis = if normal[0].abs() < 0.9 {
		[1.0, 0.0, 0.0]
	} else {
		[0.0, 1.0, 0.0]
	};
	let first = cross(normal, axis);
	let first = first.map(|coordinate| coordinate / length(first));
	let second = cross(normal, first);
	let along = |direction: [f32; 3], scale: f32| std::array::from_fn(|axis| center[axis] + direction[axis] * scale);
	[
		center,
		along(first, radius),
		along(first, -radius),
		along(second, radius),
		along(second, -radius),
	]
}

/// Verifies the grid lists every live surfel wherever a lookup weighs it: out to its lookup radius in its plane, and off
/// its plane as far as its squished distance reaches. It lists a surfel next to a cascade boundary around its center in
/// both cascades, never lists a free slot, and lists the same surfels again the next frame, because each frame's scan
/// leaves the counts at zero.
#[test]
fn surfel_grid_lists_every_cell_a_lookup_weighs_a_surfel_in_and_leaves_counts_zero() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let surfels = [
		(0, [0.5, 0.05, 0.5], UP),
		(1, [0.6, 0.05, 0.5], UP),
		(2, [0.5, 0.05, 0.9], [0.0, 0.0, 1.0]),
		// 12.75 m out, next to the boundary between cascades 2 and 3 at 12.8 m.
		(5, [12.75, 0.05, 0.5], UP),
	];
	for (slot, position, normal) in surfels {
		set.place(slot, position, normal);
	}
	// A free slot whose stale record lies among the live ones.
	set.place(3, [0.55, 0.05, 0.5], UP);
	set.set_state(3, "life", 0);

	for frame in 0..2 {
		passes.run_grid(&mut set);
		for (slot, position, normal) in surfels {
			// Lookups this far from the center read the surfel whatever the cell boundaries do.
			let mut points = disc_points(position, normal, 0.3 * surfel_radius(position)).to_vec();
			// The near surfels reach fewer cells than the grid lists one surfel in, so every point a lookup weighs them
			// at finds them: the edge of the lookup radius in their plane, and a point above it, whose squished distance
			// is 0.2 × (1 + 2 × 0.2) = 0.28 of the lookup radius's 0.3.
			if slot != 5 {
				points.extend(disc_points(position, normal, 0.95 * LOOKUP_SCALE * surfel_radius(position)));
				points.push(std::array::from_fn(|axis| position[axis] + normal[axis] * 0.2));
			}
			for point in points {
				let listed = set.listed(cell_of(point));
				assert!(
					listed.contains(&slot),
					"Frame {frame}: the cell of {point:?} should list surfel {slot}, but lists {listed:?}."
				);
			}
		}
		assert_eq!(set.pool_count("last_alive"), surfels.len() as u32, "frame {frame}");
	}

	// The boundary surfel's lookups above read cells of both cascades.
	let (_, position, normal) = surfels[3];
	let cascades = disc_points(position, normal, 0.3 * surfel_radius(position)).map(|point| cell_of(point) / 32_768);
	assert!(cascades.contains(&2) && cascades.contains(&3), "{cascades:?}");
	for cell in 0..CELL_COUNT {
		assert_eq!(set.cell_count(cell), 0, "cell {cell} should end the frame with a zero count");
		let listed = set.listed(cell);
		let mut distinct = listed.clone();
		distinct.sort_unstable();
		distinct.dedup();
		assert_eq!(distinct.len(), listed.len(), "cell {cell} lists a surfel twice: {listed:?}");
		assert!(
			listed.iter().all(|slot| surfels.iter().any(|(live, ..)| live == slot)),
			"cell {cell} lists a free slot: {listed:?}"
		);
	}
}

/// Verifies a full pool of surfels that each reach more cells than the grid lists one surfel in, next to a cascade
/// boundary, takes at most the limit's entries per surfel, so the lists fit the entries the pool keeps and hold exactly
/// the surfels their counts promise, and that the cells it keeps are the nearest: every surfel stays listed in the
/// cells around its center, such as the one at (-2.7, -2.7, -2.8) facing +z, whose center cell the first 32 cells in
/// grid order left out.
#[test]
fn surfel_grid_keeps_the_nearest_cells_of_a_surfel_that_reaches_more_than_32() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// 16 surfels near each corner of cascade zero, where each reaches 40 to 76 cells, and half of them lie close enough
	// to cascade one to be listed in it too.
	let mut surfels = Vec::new();
	for slot in 0..CAPACITY {
		let sign = |axis: u32| if (slot / 16) >> axis & 1 == 1 { -1.0 } else { 1.0 };
		let step = |index: u32| 2.6 + 0.05 * index as f32;
		let position = [sign(0) * step(slot % 4), sign(1) * step(slot % 16 / 4), sign(2) * 2.8];
		let tilted = [sign(0) * 0.1, sign(1) * 0.05, sign(2)];
		let normal = tilted.map(|coordinate| coordinate / length(tilted));
		set.place(slot, position, normal);
		surfels.push((slot, position, normal));
	}
	assert!(
		surfels
			.iter()
			.any(|(_, position, _)| distance(*position, [-2.7, -2.7, -2.8]) < 0.0001)
	);

	passes.run_grid(&mut set);

	let mut listings = vec![0u32; CAPACITY as usize];
	for cell in 0..CELL_COUNT {
		for slot in set.listed(cell) {
			listings[slot as usize] += 1;
		}
	}
	// An entry the scatter left unwritten would list slot 0 again, past its limit.
	assert!(
		listings.iter().all(|&count| count > 0 && count <= LISTING_LIMIT),
		"Every surfel should be listed in at most {LISTING_LIMIT} cells: {listings:?}"
	);
	assert_eq!(set.pool_count("cell_entries"), listings.iter().sum::<u32>());
	for (slot, position, normal) in surfels {
		for point in disc_points(position, normal, 0.3 * surfel_radius(position)) {
			let listed = set.listed(cell_of(point));
			assert!(
				listed.contains(&slot),
				"The cell of {point:?} should list surfel {slot} at {position:?}, but lists {listed:?}."
			);
		}
	}
}

/// Verifies each cell lists the surfels near its center before the others, whichever order the scatter visits them
/// in, so a pixel or a hit that reads only a crowded list's first entries reads the surfels that weigh most there.
#[test]
fn surfel_grid_lists_the_surfels_near_each_cells_center_first() {
	let passes = SurfelPasses::new();
	// The cell from (0.4, 0, 0.4) to (0.6, 0.2, 0.6), whose center is (0.5, 0.1, 0.5), lists a surfel near its front
	// when the center lies within half the surfel's 0.3 m lookup radius.
	let cell = cell_of([0.5, 0.1, 0.5]);
	let position = |slot: u32| [0.5 + 0.004 * slot as f32, 0.1, 0.5];
	for count in [8, 70] {
		let mut set = SurfelSet::new(&passes);
		// Surfels in a row out from the center, the lowest slot nearest, so the scatter, which visits the slots in order,
		// meets the nearest first.
		for slot in 0..count {
			set.place(slot, position(slot), UP);
		}
		passes.run_grid(&mut set);

		let listed = set.listed(cell);
		assert!(listed.len() >= count.min(32) as usize, "{listed:?}");
		let near = |slot: u32| distance(position(slot), [0.5, 0.1, 0.5]) < 0.5 * LOOKUP_SCALE * NEAR_RADIUS;
		let first_far = listed.iter().position(|&slot| !near(slot)).unwrap_or(listed.len());
		assert!(
			listed[first_far..].iter().all(|&slot| !near(slot)),
			"A cell of {} surfels should list every near surfel before the others: {listed:?}",
			listed.len()
		);
		assert!(first_far > 0);
	}
}

/// Verifies a crowded cell charges its untouched surfels rent until they reach the end of their life, while a lone
/// surfel of the same age and a crowded surfel that pixels vote for live on, and that every dead surfel's slot,
/// including one a despawn vote killed, goes on the free stack.
#[test]
fn surfel_economy_kills_crowded_untouched_surfels_into_the_free_stack() {
	const CROWD: u32 = 100;
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// 40 frames from the end of their lives: a frame without votes ages a lone surfel by one, but a surfel in a cell
	// of 101 by 93.
	let life = TTL - 40;
	let voted = CROWD;
	let lone = CROWD + 1;
	let killed = CROWD + 2;
	for slot in 0..=voted {
		set.place(slot, [0.5, 0.05, 0.5], UP);
		set.set_state(slot, "life", life);
	}
	set.place(lone, [-1.5, 0.05, -1.5], UP);
	set.set_state(lone, "life", life);
	set.place(killed, [1.5, 0.05, -1.5], UP);
	set.set_state(killed, "touched", KILL);

	// The crowd pays no rent before the grid has listed it once.
	passes.run_grid(&mut set);
	assert_eq!(set.state(killed, "life"), 0);
	assert_eq!(set.free_slots(), [killed]);
	assert_eq!(set.pool_count("last_alive"), CROWD + 2);

	set.set_state(voted, "touched", 100);
	passes.run_grid(&mut set);
	for slot in 0..CROWD {
		assert_eq!(
			set.state(slot, "life"),
			0,
			"crowded surfel {slot} should have paid its life in rent"
		);
	}
	assert_ne!(set.state(voted, "life"), 0, "a vote should keep a crowded surfel alive");
	assert_eq!(set.state(lone, "life"), life + 2);
	let mut free = set.free_slots();
	free.sort_unstable();
	let mut dead: Vec<u32> = (0..CROWD).chain([killed]).collect();
	dead.sort_unstable();
	assert_eq!(free, dead);
	assert_eq!(set.pool_count("last_recycled"), CROWD);
	assert_eq!(set.pool_count("last_alive"), 2);
}

/// Verifies the budget grants a window of the wanted rays that starts where last frame's window ended, so the surfels'
/// rays take turns, and that each granted ray names its surfel and its index among the surfel's rays.
#[test]
fn surfel_scan_grants_rays_round_robin_within_the_budget() {
	const BUDGET: u32 = 40;
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.parameters.write("ray_budget", Value::U32(BUDGET)).expect("ray budget");
	// Three young surfels want 32 rays each, 96 in all.
	for slot in 0..3 {
		set.place(slot, [0.5 + slot as f32, 0.05, 0.5], UP);
	}
	let rays = |slot: u32, samples: std::ops::Range<u32>| samples.map(move |sample| (slot, sample));

	passes.run_grid(&mut set);
	assert_eq!(set.pool_count("priority_wanted"), 3 * YOUNG_RAYS);
	assert_eq!(set.pool_count("priority_rays") + set.pool_count("regular_rays"), BUDGET);
	assert_eq!(set.granted_rays(), rays(0, 0..32).chain(rays(1, 0..8)).collect::<Vec<_>>());
	// Each window starts where the last one ended, and wraps around past the last wanted ray.
	passes.run_grid(&mut set);
	assert_eq!(set.granted_rays(), rays(1, 8..32).chain(rays(2, 0..16)).collect::<Vec<_>>());
	passes.run_grid(&mut set);
	assert_eq!(set.granted_rays(), rays(2, 16..32).chain(rays(0, 0..24)).collect::<Vec<_>>());
}

/// Verifies young surfels, including one lookups already trust fully, and surfels whose light keeps changing get their
/// rays first, up to three quarters of the budget while settled surfels want the rest, and that each class's window of
/// rays starts where its last one ended, so the settled surfels' rays still take their turns.
#[test]
fn surfel_scan_grants_young_and_changing_surfels_first_and_keeps_settled_turns() {
	const BUDGET: u32 = 40;
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.parameters.write("ray_budget", Value::U32(BUDGET)).expect("ray budget");
	// Two young surfels want 32 rays each, and one whose light keeps changing 16, 80 in the priority class. Four
	// settled surfels want 4 each, 16 in the regular class.
	for slot in 0..7 {
		set.place(slot, [0.5 + slot as f32, 0.05, 0.5], UP);
		if slot >= 2 {
			set.settle(slot);
		}
	}
	set.set_estimate(2, "inconsistency", Value::F32(0.5));
	// Lookups trust the second young surfel fully, but it is still warming up.
	set.set_estimate(1, "rays", Value::F32(MATURE_RAYS));
	let rays = |slot: u32, samples: std::ops::Range<u32>| samples.map(move |sample| (slot, sample));

	passes.run_grid(&mut set);
	assert_eq!(set.pool_count("priority_wanted"), 2 * YOUNG_RAYS + 16);
	assert_eq!(set.pool_count("regular_wanted"), 4 * SETTLED_RAYS);
	// The settled surfels want 16 of the 40 rays, so the priority class takes its share of 30.
	assert_eq!(set.pool_count("priority_rays"), 30);
	assert_eq!(set.pool_count("regular_rays"), 10);
	let expected: Vec<_> = rays(0, 0..30)
		.chain(rays(3, 0..4))
		.chain(rays(4, 0..4))
		.chain(rays(5, 0..2))
		.collect();
	assert_eq!(set.granted_rays(), expected);
	// Each class's window starts where its last one ended; the regular one wraps around past its last wanted ray.
	passes.run_grid(&mut set);
	let expected: Vec<_> = rays(0, 30..32)
		.chain(rays(1, 0..28))
		.chain(rays(5, 2..4))
		.chain(rays(6, 0..4))
		.chain(rays(3, 0..4))
		.collect();
	assert_eq!(set.granted_rays(), expected);
}

/// Verifies a young surfel that a ray's hit spawned asks for its rays in the regular class, so it never takes rays from
/// the surfaces the camera sees.
#[test]
fn surfel_scan_grants_hit_spawned_surfels_regular_rays() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.place(0, [0.5, 0.05, 0.5], UP);
	set.set_state(0, "bounce", 1);

	passes.run_grid(&mut set);
	assert_eq!(set.pool_count("priority_wanted"), 0);
	assert_eq!(set.pool_count("regular_wanted"), SETTLED_RAYS);
}

/// Verifies the hit sort lists every ray that hit in the order of the materials they hit, the rays on one material in
/// the order the trace counted them, leaves misses out, and leaves the counts at zero for the next frame's trace.
#[test]
fn ray_hit_sort_orders_hits_by_material() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// The material slot each ray hit, or `None` for a miss. The trace counts each hit on its material, and the count
	// before the hit is its rank.
	let hits = [Some(3), Some(1), Some(3), None, Some(0), Some(1), Some(1023)];
	let mut counts = vec![0; 1024];
	for (ray, hit) in hits.iter().enumerate() {
		let key = hit.map_or(NO_SURFEL, |material: u32| {
			let rank = counts[material as usize];
			counts[material as usize] += 1;
			(material << HIT_RANK_BITS) | rank
		});
		set.hit_keys.write_array_element(ray, Value::U32(key)).expect("hit key");
	}
	for (material, count) in counts.iter().enumerate() {
		set.hit_sort
			.write_indexed("counts", material, Value::U32(*count))
			.expect("hit count");
	}
	set.hit_sort.write("rays", Value::U32(hits.len() as u32)).expect("ray count");

	dispatch(&passes.sort_scan, &mut set, WORKGROUP_SIZE);
	dispatch(&passes.sort_scatter, &mut set, hits.len() as u32);

	assert_eq!(set.hit_sort.read("hit_count").expect("hit count"), Value::U32(6));
	let sorted: Vec<u32> = (0..6).map(|index| read_u32(&set.sorted_hits, index)).collect();
	assert_eq!(sorted, [4, 1, 5, 0, 2, 6]);
	for material in 0..1024 {
		assert_eq!(
			set.hit_sort.read_indexed("counts", material).expect("hit count"),
			Value::U32(0),
			"material {material} should end the sort with a zero count"
		);
	}
}

/// A settled surfel's four rays: two hits that bring light back and two misses.
const SETTLED_RESULTS: [([f32; 3], f32); 4] = [
	([2.0, 4.0, 6.0], 1.5),
	([0.0, 0.0, 0.0], -1.0),
	([2.0, 0.0, 2.0], 0.5),
	([0.0, 0.0, 0.0], -1.0),
];
/// The points of the equal-area square the fixture rays leave toward, one per ray, packed as the trace packs them.
const SETTLED_POINTS: [[f32; 2]; 4] = [[0.5, 0.5], [0.1, 0.3], [0.7, 0.9], [0.95, 0.2]];

/// Packs a point of the unit square into two 16-bit coordinates, as the trace does.
fn pack_point(point: [f32; 2]) -> u32 {
	let quantize = |value: f32| (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
	quantize(point[0]) | (quantize(point[1]) << 16)
}

/// Places a settled surfel in slot 0 whose long mean holds `count` rays, runs the grid, gives each of its rays the
/// result of the same index in `results`, from the cosine-weighted distribution toward [`SETTLED_POINTS`], and folds
/// them into its estimate at `exposure`.
fn integrate_settled_surfel(
	passes: &SurfelPasses,
	set: &mut SurfelSet,
	count: f32,
	exposure: f32,
	results: [([f32; 3], f32); 4],
) {
	integrate_settled_surfel_toward(passes, set, count, exposure, results, SETTLED_POINTS);
}

/// Does what [`integrate_settled_surfel`] does, with each ray leaving toward the point of the equal-area square of the
/// same index in `points`.
fn integrate_settled_surfel_toward(
	passes: &SurfelPasses,
	set: &mut SurfelSet,
	count: f32,
	exposure: f32,
	results: [([f32; 3], f32); 4],
	points: [[f32; 2]; 4],
) {
	let rays = settle_surfel(passes, set, count, SETTLED_RECEIVED, exposure);
	integrate_rays(passes, set, &rays, results, points);
}

/// Places a surfel in slot 0 that received `received` rays and whose long mean holds `count` of them, runs the grid at
/// `exposure`, and returns the rays the budget granted it as (ray, sample) pairs: [`SETTLED_RAYS`] for a mature surfel,
/// and [`YOUNG_RAYS`] for a young one. Until the grid runs again, every frame grants it the same rays.
fn settle_surfel(passes: &SurfelPasses, set: &mut SurfelSet, count: f32, received: f32, exposure: f32) -> Vec<(usize, u32)> {
	set.place(0, [0.5, 0.05, 0.5], UP);
	set.set_estimate(0, "count", Value::F32(count));
	set.set_estimate(0, "rays", Value::F32(received));
	for (member, value) in [("exposure", exposure), ("previous_exposure", exposure)] {
		set.parameters.write(member, Value::F32(value)).expect("exposure");
	}
	passes.run_grid(set);
	let granted = set.granted_rays();
	let expected = if received < WARM_UP_RAYS { YOUNG_RAYS } else { SETTLED_RAYS };
	assert_eq!(granted.len(), expected as usize);
	granted
		.into_iter()
		.enumerate()
		.map(|(ray, (slot, sample))| {
			assert_eq!(slot, 0);
			(ray, sample)
		})
		.collect()
}

/// Gives each of `rays` the result of its sample's index in `results`, from the cosine-weighted distribution toward the
/// point of the equal-area square of the same index in `points`, and folds them into slot 0's estimate. A young
/// surfel's 32 rays take the four results in turn, so their average is the four's.
fn integrate_rays(
	passes: &SurfelPasses,
	set: &mut SurfelSet,
	rays: &[(usize, u32)],
	results: [([f32; 3], f32); 4],
	points: [[f32; 2]; 4],
) {
	let drawn = results.map(|(radiance, distance)| DrawnRay {
		radiance,
		distance,
		weight: 1.0,
		guided: false,
		back_face: false,
	});
	integrate_drawn_rays(passes, set, rays, drawn, points);
}

/// What one fixture ray brings back: its radiance, how far it went (negative for a miss), its weight in the surfel's
/// light, whether the surfel's guide drew it rather than the cosine-weighted distribution, and whether it hit the back
/// of a surface.
#[derive(Clone, Copy)]
struct DrawnRay {
	radiance: [f32; 3],
	distance: f32,
	weight: f32,
	guided: bool,
	back_face: bool,
}

/// Gives each of `rays` the result of its sample's index in `drawn`, toward the point of the equal-area square of the
/// same index in `points`, and folds them into slot 0's estimate. A young surfel's 32 rays take the four in turn.
fn integrate_drawn_rays(
	passes: &SurfelPasses,
	set: &mut SurfelSet,
	rays: &[(usize, u32)],
	drawn: [DrawnRay; 4],
	points: [[f32; 2]; 4],
) {
	for &(ray, sample) in rays {
		let index = sample as usize % drawn.len();
		let drawn = drawn[index];
		let flags = u32::from(drawn.guided) | u32::from(drawn.back_face) << 1;
		for (member, value) in [
			("radiance", Value::Vec3F(drawn.radiance)),
			("distance", Value::F32(drawn.distance)),
			("direction", Value::U32(pack_point(points[index]))),
			("weight", Value::F32(drawn.weight)),
			("flags", Value::U32(flags)),
		] {
			set.ray_results.write_array_member(ray, member, value).expect("ray result");
		}
	}
	dispatch(&passes.integrate, set, CAPACITY);
}

fn vec4(value: Value) -> [f32; 4] {
	match value {
		Value::Vec4F(value) => value,
		value => panic!("Unexpected vector: {value:?}."),
	}
}

/// Verifies a new surfel's first estimate is the average of its rays: pre-exposed light, with misses bringing the
/// environment's light, and the fraction of rays that hit. Lookups trust the surfel by the share of a mature surfel's
/// rays it received: its first 32 rays earn a quarter.
#[test]
fn surfel_integrate_estimates_bounce_and_occluded_fraction() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.environment_radiance = texture_2d(1, 1, &[[0.5, 1.0, 1.5, 1.0]]);
	let rays = settle_surfel(&passes, &mut set, 0.0, 0.0, 2.0);
	integrate_rays(&passes, &mut set, &rays, SETTLED_RESULTS, SETTLED_POINTS);

	// The hits bring (2, 4, 6) and (2, 0, 2) and the two misses the environment's (0.5, 1, 1.5) each, averaged over the
	// rays at an exposure of 2.
	let expected = [2.5, 3.0, 5.5, 0.5];
	assert_rgba_close(vec4(set.estimate(0, "mean")), expected, 0.0001);
	assert_eq!(set.estimate(0, "count"), Value::F32(32.0));
	assert_eq!(set.estimate(0, "rays"), Value::F32(32.0));
	assert_eq!(set.record(0, "irradiance"), Value::Vec4F16(expected.map(f16::from_f32)));
	assert_eq!(set.record(0, "weight"), Value::F32(0.25));
}

/// Verifies a young estimate averages its rays evenly: 32 new rays on top of the 96 it holds weigh a quarter, and they
/// make the surfel mature.
#[test]
fn surfel_integrate_averages_a_young_surfels_rays_evenly() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.set_estimate(0, "mean", Value::Vec4F([1.0, 1.0, 1.0, 0.0]));
	let rays = settle_surfel(&passes, &mut set, 96.0, 96.0, 1.0);
	integrate_rays(&passes, &mut set, &rays, [([5.0, 5.0, 5.0], 1.0); 4], SETTLED_POINTS);

	assert_rgba_close(vec4(set.estimate(0, "mean")), [2.0, 2.0, 2.0, 0.25], 0.0001);
	assert_eq!(set.estimate(0, "count"), Value::F32(128.0));
	assert_eq!(set.record(0, "weight"), Value::F32(1.0));
}

/// Verifies a settled estimate counts a sample far above its recent light only up to eight deviations above its short
/// mean, so one bright frame barely moves the light lookups read.
#[test]
fn surfel_integrate_clamps_fireflies() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	for (member, value) in [
		("mean", [1.0, 1.0, 1.0, 0.5]),
		("short_mean", [1.0, 1.0, 1.0, 0.5]),
		("variance", [0.01; 4]),
	] {
		set.set_estimate(0, member, Value::Vec4F(value));
	}
	set.set_estimate(0, "blend", Value::F32(1.0));
	integrate_settled_surfel(&passes, &mut set, 400.0, 1.0, [([100.0, 100.0, 100.0], 1.0); 4]);

	// The clamped sample, 0.1 + 1.0 + 8 × 0.1 = 1.9, moves the short mean a tenth of the way.
	let short_mean = vec4(set.estimate(0, "short_mean"));
	assert_rgba_close(short_mean, [1.09, 1.09, 1.09, 0.55], 0.0001);
	let mean = vec4(set.estimate(0, "mean"));
	assert!(mean[..3].iter().all(|&channel| channel > 1.0 && channel < 1.01), "{mean:?}");
}

/// Verifies a settled estimate keeps converging on light that looks steady but is noisy: an error under a third of a
/// deviation, which the estimator's own catch-up leaves nearly alone, shrinks by more than half within 100 frames, as
/// it does in an even average of the frames the estimate holds.
#[test]
fn surfel_integrate_keeps_converging_after_it_settles() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// The light alternates between 0 and 0.2 a frame, a mean of 0.1 with a deviation of 0.1, and the estimate starts
	// 0.03 above it.
	for (member, value) in [
		("mean", [0.13, 0.13, 0.13, 1.0]),
		("short_mean", [0.1, 0.1, 0.1, 1.0]),
		("variance", [0.01, 0.01, 0.01, 0.0]),
	] {
		set.set_estimate(0, member, Value::Vec4F(value));
	}
	set.set_estimate(0, "blend", Value::F32(1.0 / 32.0));
	let rays = settle_surfel(&passes, &mut set, 160.0, SETTLED_RECEIVED, 1.0);
	for frame in 0..100 {
		let light = if frame % 2 == 0 { 0.0 } else { 0.2 };
		integrate_rays(&passes, &mut set, &rays, [([light; 3], 1.0); 4], SETTLED_POINTS);
	}

	let mean = vec4(set.estimate(0, "mean"));
	assert!(mean[..3].iter().all(|&channel| (channel - 0.1).abs() < 0.015), "{mean:?}");
}

/// Verifies the hit fraction counts each ray the cosine-weighted distribution drew once and no ray the guide drew, while
/// the light keeps every ray's weight. Weighed like the light, the first frame's hit fraction would read 0.875 where
/// every ray hits, and 0.125 where only the guided rays do.
#[test]
fn surfel_integrate_counts_only_cosine_rays_toward_the_hit_fraction() {
	let passes = SurfelPasses::new();
	// Half the rays from each distribution, weighted as a guide that holds most of its light in their cell weighs them.
	let ray = |radiance: f32, distance: f32, guided: bool| DrawnRay {
		radiance: [radiance; 3],
		distance,
		weight: if guided { 0.25 } else { 1.5 },
		guided,
		back_face: false,
	};

	let mut enclosed = SurfelSet::new(&passes);
	let rays = settle_surfel(&passes, &mut enclosed, 0.0, 0.0, 1.0);
	let drawn = [
		ray(4.0, 1.0, false),
		ray(4.0, 1.0, false),
		ray(4.0, 1.0, true),
		ray(4.0, 1.0, true),
	];
	integrate_drawn_rays(&passes, &mut enclosed, &rays, drawn, SETTLED_POINTS);
	// The light averages 4 × (1.5 + 1.5 + 0.25 + 0.25) / 4 = 3.5.
	assert_rgba_close(vec4(enclosed.estimate(0, "mean")), [3.5, 3.5, 3.5, 1.0], 0.0001);

	let mut open = SurfelSet::new(&passes);
	let rays = settle_surfel(&passes, &mut open, 0.0, 0.0, 1.0);
	let drawn = [
		ray(0.0, -1.0, false),
		ray(0.0, -1.0, false),
		ray(4.0, 1.0, true),
		ray(4.0, 1.0, true),
	];
	integrate_drawn_rays(&passes, &mut open, &rays, drawn, SETTLED_POINTS);
	assert_eq!(vec4(open.estimate(0, "mean"))[3], 0.0);
}

/// Verifies a frame whose rays all came from the guide moves the light and leaves the hit fraction as it was, for a
/// young estimate and for a settled one whose short mean strayed from its long mean.
#[test]
fn surfel_integrate_keeps_the_hit_fraction_when_the_guide_drew_every_ray() {
	let passes = SurfelPasses::new();
	let guided = DrawnRay {
		radiance: [5.0; 3],
		distance: 1.0,
		weight: 1.0,
		guided: true,
		back_face: false,
	};

	let mut young = SurfelSet::new(&passes);
	young.set_estimate(0, "mean", Value::Vec4F([1.0, 1.0, 1.0, 0.25]));
	let rays = settle_surfel(&passes, &mut young, 96.0, 96.0, 1.0);
	integrate_drawn_rays(&passes, &mut young, &rays, [guided; 4], SETTLED_POINTS);
	// The 32 new rays weigh a quarter of the 128: 1 + (5 - 1) / 4 = 2.
	assert_rgba_close(vec4(young.estimate(0, "mean")), [2.0, 2.0, 2.0, 0.25], 0.0001);

	let mut settled = SurfelSet::new(&passes);
	for (member, value) in [
		("mean", [1.0, 1.0, 1.0, 0.9]),
		("short_mean", [1.0, 1.0, 1.0, 0.5]),
		("variance", [0.01, 0.01, 0.01, 0.0]),
	] {
		settled.set_estimate(0, member, Value::Vec4F(value));
	}
	settled.set_estimate(0, "blend", Value::F32(1.0));
	let rays = settle_surfel(&passes, &mut settled, 400.0, SETTLED_RECEIVED, 1.0);
	integrate_drawn_rays(&passes, &mut settled, &rays, [guided; 4], SETTLED_POINTS);
	assert_eq!(vec4(settled.estimate(0, "mean"))[3], 0.9);
	assert_eq!(vec4(settled.estimate(0, "short_mean"))[3], 0.5);
	assert_eq!(vec4(settled.estimate(0, "variance"))[3], 0.0);
}

/// Verifies a surfel's guide learns where its rays found light, the sky's light where they missed, and that the sum it
/// keeps for drawing rays stays the sum of its cells.
#[test]
fn surfel_guide_stays_normalized_after_an_update() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	integrate_settled_surfel(&passes, &mut set, 160.0, 1.0, SETTLED_RESULTS);
	// The sky turns on for the second frame, so only the rays that missed then learn it.
	set.environment_radiance = texture_2d(1, 1, &[[0.5; 4]]);
	integrate_settled_surfel(&passes, &mut set, 164.0, 1.0, SETTLED_RESULTS);

	let guide = set.guide(0);
	let Value::F32(mass) = set.estimate(0, "guide_mass") else {
		panic!("Expected a guide mass.");
	};
	let sum: f32 = guide.iter().sum();
	assert!(mass > 0.0, "the hits should have taught the guide some light");
	assert!((mass - sum).abs() <= sum * 0.0001, "mass {mass} against the cells' sum {sum}");
	let cell = |point: [f32; 2]| (point[1] * 8.0) as usize * 8 + (point[0] * 8.0) as usize;
	assert!(guide[cell(SETTLED_POINTS[0])] > 0.0);
	assert!(
		guide[cell(SETTLED_POINTS[1])] > 0.0,
		"a miss should teach the guide the sky's light"
	);
	// No ray left toward the square's corner.
	assert_eq!(guide[cell([0.99, 0.99])], 0.0);
}

/// Verifies a surfel's radial depth follows a nearer hit five times faster than a farther one, counts a hit past twice
/// the surfel's radius, and a miss, as twice the radius, and fills an empty cell with its first ray as it is.
#[test]
fn surfel_radial_depth_blends_nearer_hits_faster() {
	let passes = SurfelPasses::new();
	let reach = 2.0 * surfel_radius([0.5, 0.05, 0.5]);
	// Every ray leaves toward one direction, so the four fold into one cell, one after another.
	let toward = [[0.5, 0.5]; 4];
	let blend = |start: [f32; 4], rate: f32| {
		(0..4).fold(start, |cell, _| {
			std::array::from_fn(|moment| cell[moment] + (moments(reach)[moment] - cell[moment]) * rate)
		})
	};
	// Hits 1.5 m away, past the reach, are nearer than cells at 1 m; hits 0.5 m away, also past it, are farther than
	// cells at 0.1 m.
	for (start, distance, rate) in [(1.0, 1.5, 0.05), (0.1, 0.5, 0.01)] {
		let mut set = SurfelSet::new(&passes);
		set.set_depths(0, start);
		integrate_settled_surfel_toward(&passes, &mut set, 160.0, 1.0, [([1.0; 3], distance); 4], toward);

		let changed: Vec<usize> = (0..DEPTH_CELLS)
			.filter(|&cell| set.depth(0, cell) != moments(start))
			.collect();
		assert_eq!(changed.len(), 1, "cells at {start} m changed: {changed:?}");
		assert_rgba_close(set.depth(0, changed[0]), blend(moments(start), rate), 0.0001);
	}

	let mut set = SurfelSet::new(&passes);
	integrate_settled_surfel_toward(&passes, &mut set, 160.0, 1.0, [([0.0; 3], -1.0); 4], toward);
	let filled: Vec<[f32; 4]> = (0..DEPTH_CELLS)
		.map(|cell| set.depth(0, cell))
		.filter(|depth| depth[3] != 0.0)
		.collect();
	assert_eq!(filled.len(), 1, "cells the misses filled: {filled:?}");
	assert_rgba_close(filled[0], moments(reach), 0.0001);
}

/// Verifies a surfel's clearance reaches almost to twice its radius while every ray leaves the reach without a hit, and
/// stops just short of a wall once one ray hits it: the four-moment bound sees no shadow nearer than the wall, though
/// Cantelli's inequality, from the cell's mean and spread alone, can't clear any distance there.
#[test]
fn surfel_clearance_stops_short_of_the_nearest_wall() {
	let passes = SurfelPasses::new();
	let reach = 2.0 * surfel_radius([0.5, 0.05, 0.5]);
	let clearance = |set: &SurfelSet| match set.clearances.read_array_element(0) {
		Ok(Value::F32(clearance)) => clearance,
		value => panic!("Unexpected clearance: {value:?}."),
	};

	let mut set = SurfelSet::new(&passes);
	integrate_settled_surfel(&passes, &mut set, 160.0, 1.0, [([0.0; 3], -1.0); 4]);
	let open = clearance(&set);
	assert!(
		open > 0.9 * reach && open < reach,
		"an open surfel's clearance {open} against its reach {reach}"
	);

	let wall = 0.1;
	integrate_settled_surfel(
		&passes,
		&mut set,
		160.0,
		1.0,
		[([0.0; 3], -1.0), ([0.0; 3], wall), ([0.0; 3], -1.0), ([0.0; 3], -1.0)],
	);
	let walled = clearance(&set);
	assert!(
		walled > 0.9 * wall && walled < wall,
		"the clearance {walled} should stop just short of the wall {wall} m away"
	);
}

/// Verifies the equal-area map sends the square's points to unit directions of the upper hemisphere and back, and
/// spreads them evenly in solid angle: half of a uniform grid of points lands within 60 degrees of the axis, the cap
/// that holds half of the hemisphere's solid angle.
#[test]
fn surfel_hemisphere_map_round_trips_and_keeps_solid_angle() {
	const RESULTS_SLOT: ResourceSlot = ResourceSlot::new(1290);
	const MAP_STAND_IN: &str = "
MapResults: struct {
	round_trip_error: f32,
	length_error: f32,
	below_horizon: u32,
	upper_half: u32,
}
results: descriptor<{ type: MapResults, binding: 1290, access: read_write }>;
main: fn () -> void {
	for (let row: u32 = 0; row < 64; row = row + 1) {
		for (let column: u32 = 0; column < 64; column = column + 1) {
			let point: vec2f = vec2f((f32(column) + 0.5) / 64.0, (f32(row) + 0.5) / 64.0);
			let direction: vec3f = surfel_square_to_hemisphere(point);
			let back: vec2f = surfel_hemisphere_to_square(direction);
			results.round_trip_error = max(results.round_trip_error, max(abs(back.x - point.x), abs(back.y - point.y)));
			results.length_error = max(results.length_error, abs(length(direction) - 1.0));
			if (direction.z < 0.0) {
				results.below_horizon = results.below_horizon + 1;
			}
			if (direction.z > 0.5) {
				results.upper_half = results.upper_half + 1;
			}
		}
	}
}
";
	let program = compile(
		besl::lex(stacked_on_layers_of("visibility/surfel-count.besl", MAP_STAND_IN))
			.expect("the hemisphere map stand-in should link"),
	);
	let mut results = buffer(&program, RESULTS_SLOT);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(RESULTS_SLOT, &mut results);
	run_workgroup(&program, descriptors, &[lane_config(0)]);

	let read = |member: &str| results.read(member).expect("map result");
	let Value::F32(round_trip) = read("round_trip_error") else {
		panic!()
	};
	let Value::F32(length) = read("length_error") else { panic!() };
	assert!(round_trip < 0.0001, "round trip error {round_trip}");
	assert!(length < 0.00001, "length error {length}");
	assert_eq!(read("below_horizon"), Value::U32(0));
	let Value::U32(upper) = read("upper_half") else { panic!() };
	// The grid's points sample the square's area, which the map keeps as solid angle.
	assert!(
		(upper as f32 / 4096.0 - 0.5).abs() < 0.03,
		"{upper} of 4096 points above z = 0.5"
	);
}

/// One sink whose camera sits at the grid origin and sees a wall [`WALL_DEPTH`] ahead in every resolve pixel.
struct WallSink {
	program: ExecutableProgram,
	view: Buffer,
	parameters: Buffer,
	spawns: Buffer,
	depth: Texture,
	normals: Texture,
	/// The light and the weights the resolve accumulated. Without a previous frame's history, the light is the
	/// pixel's estimate and the first weight its confidence.
	history: Texture,
	weights: Texture,
}

impl WallSink {
	fn new() -> Self {
		let program = asset!("surfel-resolve.besl");
		let mut parameters = buffer(&program, RESOLVE_PARAMETERS_SLOT);
		for (member, value) in [
			("view_to_world", Value::Mat4F(IDENTITY_MATRIX)),
			("debug_view_first_slot", Value::U32(0)),
			("spawn_tiles_per_row", Value::U32(1)),
			("spawn_tile_count", Value::U32(1)),
			// Every candidate passes the random draw, which scales with the wall's one-meter depth.
			("spawn_probability", Value::F32(1.0)),
		] {
			parameters.write(member, value).expect("surfel resolve parameters");
		}
		let texels = vec![[WALL_DEPTH, 0.0, 0.0, 1.0]; (RESOLVE_EXTENT * RESOLVE_EXTENT) as usize];
		Self {
			view: screen_view_buffer(&program, VIEWS_SLOT, RESOLVE_EXTENT, RESOLVE_EXTENT),
			spawns: array_buffer(&program, SPAWNS_SLOT, 1),
			depth: texture_2d(RESOLVE_EXTENT, RESOLVE_EXTENT, &texels),
			normals: empty_image(RESOLVE_EXTENT, RESOLVE_EXTENT),
			history: empty_image(RESOLVE_EXTENT, RESOLVE_EXTENT),
			weights: empty_image(RESOLVE_EXTENT, RESOLVE_EXTENT),
			parameters,
			program,
		}
	}

	/// Returns the point of the wall that resolve pixel `pixel` sees.
	fn wall_point(pixel: [u32; 2]) -> [f32; 3] {
		let [x, y] = ssgi_ray_at(pixel[0], pixel[1], RESOLVE_EXTENT);
		[x * WALL_DEPTH, y * WALL_DEPTH, WALL_DEPTH]
	}

	/// Runs the resolve over the whole image, which is one workgroup.
	fn resolve(&mut self, set: &mut SurfelSet) {
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		descriptors.bind_buffer(VIEWS_SLOT, &mut self.view);
		descriptors.bind_buffer(RESOLVE_PARAMETERS_SLOT, &mut self.parameters);
		descriptors.bind_buffer(SPAWNS_SLOT, &mut self.spawns);
		descriptors.bind_texture(DEPTH_PYRAMID_SLOT, &mut self.depth);
		descriptors.bind_image(NORMALS_SLOT, &mut self.normals);
		descriptors.bind_image(HISTORY_OUTPUT_SLOT, &mut self.history);
		descriptors.bind_image(WEIGHTS_OUTPUT_SLOT, &mut self.weights);
		run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&self.program, descriptors, TILE_WORKGROUP_WIDTH, [0, 0]);
	}

	fn spawn(&self, member: &str) -> Value {
		self.spawns.read_array_member(0, member).expect("surfel spawn request")
	}

	/// Returns the light the resolve accumulated at `pixel`, which is its estimate on a first frame.
	fn light(&self, pixel: [u32; 2]) -> [f32; 4] {
		rgba(&self.history, pixel)
	}

	/// Returns the confidence the resolve accumulated at `pixel`, which is its estimate's on a first frame.
	fn confidence(&self, pixel: [u32; 2]) -> f32 {
		rgba(&self.weights, pixel)[0]
	}
}

/// Verifies each pixel blends the light of the surfels around it by their weights, normalized over how much lookups
/// trust them, so a young surfel shows its own light at once, with only the confidence its rays earned. A pixel no
/// trusted surfel lights, such as one beside a surfel that holds no rays yet, has neither light nor confidence, and
/// leaves its light to the reconstruction passes.
#[test]
fn surfel_resolve_blends_the_light_of_the_surfels_around_each_pixel() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let near = [0.5, 0.25, 0.125, 0.75];
	let far = [0.25, 0.5, 1.0, 0.25];
	let young = [1.0, 2.0, 3.0, 0.5];
	for (slot, pixel, light, weight) in [
		(0, [1, 1], near, 1.0),
		(1, [5, 1], far, 1.0),
		(2, [1, 6], [8.0, 8.0, 8.0, 1.0], 0.0),
		(3, [6, 6], young, 0.25),
	] {
		set.place(slot, WallSink::wall_point(pixel), WALL_NORMAL);
		set.set_record(slot, "irradiance", Value::Vec4F16(light.map(f16::from_f32)));
		set.set_record(slot, "weight", Value::F32(weight));
	}
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	assert_rgba_close(sink.light([1, 1]), near, 0.0001);
	assert_rgba_close(sink.light([5, 1]), far, 0.0001);
	// Halfway between the two lit surfels, both weigh the same.
	assert_rgba_close(
		sink.light([3, 1]),
		std::array::from_fn(|channel| (near[channel] + far[channel]) * 0.5),
		0.001,
	);
	// A pixel at a trusted surfel's center is fully confident.
	assert!((sink.confidence([1, 1]) - 1.0).abs() < 0.0001, "{}", sink.confidence([1, 1]));
	// The young surfel's light shows as it is, with a quarter of the confidence.
	assert_rgba_close(sink.light([6, 6]), young, 0.001);
	assert!((sink.confidence([6, 6]) - 0.25).abs() < 0.0001, "{}", sink.confidence([6, 6]));
	// The surfel without rays lights nothing, so its pixel, like one far from every surfel, has neither light nor
	// confidence.
	for pixel in [[1, 6], [7, 3]] {
		assert_rgba_close(sink.light(pixel), [0.0; 4], 0.0);
		assert_eq!(sink.confidence(pixel), 0.0, "pixel {pixel:?}");
	}
}

/// Verifies a pixel takes no light from a surfel whose radial depth puts a wall between them, and the light of a surfel
/// with a clear view, and that a pixel hidden from every surfel around it gets neither their light nor the sky, but
/// the confidence of the surfels that hide it, so the reconstruction keeps it dark.
#[test]
fn surfel_resolve_occlusion_hides_a_surfel_behind_a_wall() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let clear = [0.5, 0.25, 0.125, 0.75];
	let walled = [8.0, 8.0, 8.0, 0.25];
	for (slot, pixel, light) in [(0, [2, 2], clear), (1, [4, 2], walled), (2, [6, 6], walled)] {
		set.place(slot, WallSink::wall_point(pixel), WALL_NORMAL);
		set.set_record(slot, "irradiance", Value::Vec4F16(light.map(f16::from_f32)));
	}
	// Every direction of the walled surfels' hemispheres ends a millimeter away, so the wall's pixels around them lie
	// behind it.
	for slot in [1, 2] {
		set.set_depths(slot, 0.001);
	}
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	// Halfway between the clear surfel and a walled one, only the clear one lights the pixel, and the walled one's
	// light doesn't reach the clear one's pixel at the edge of its disc either.
	assert_rgba_close(sink.light([3, 2]), clear, 0.0001);
	assert_rgba_close(sink.light([2, 2]), clear, 0.0001);
	// The pixel beside the other walled surfel lies within its disc but only behind its wall: it is confidently dark.
	assert_rgba_close(sink.light([5, 6]), [0.0, 0.0, 0.0, 1.0], 0.000001);
	assert!(sink.confidence([5, 6]) > 0.0);
}

/// A closest hit that reports a white, single-sided surface facing the ray, standing in for a material's.
const WHITE_HIT: &str = "
main: fn (input: StageInput, payload: HitSurface) -> HitSurface {
	let hit_surface: HitSurface = payload;
	hit_surface.albedo = vec3f16(1.0, 1.0, 1.0);
	hit_surface.front_face = 1;
	hit_surface.emission = vec3f(0.0, 0.0, 0.0);
	hit_surface.normal = vec3f16(normalize(vec3f(0.0, 0.0, 0.0) - input.ray_direction));
	return hit_surface;
}
";

/// The `WhiteShade` struct runs the shade pass with a white closest hit standing in for every material's. Its light
/// table starts empty, so a hit's light is the surfels' alone until a test adds lights.
struct WhiteShade {
	pipeline: RayTracingPipeline,
	hits: Buffer,
	lights: Buffer,
}

impl WhiteShade {
	fn new() -> Self {
		let shade = compile(besl::lex(stacked_asset("visibility/surfel-shade.besl")).expect("the shade pass should link"));
		// Material hit shaders stack on the layers the shader generator gives them.
		let white_hit = besl::parse(WHITE_HIT).expect("the white hit should parse");
		let white_hit = besl::stack_layers(white_hit, super::super::shader_generator::MATERIAL_HIT_LAYERS)
			.expect("the white hit should stack on the material hit layers");
		let white_hit = compile(besl::lex(white_hit).expect("the white hit should link"));
		let hits = array_buffer(&shade, HITS_SLOT, RAY_BUDGET as usize);
		let lights = buffer(&shade, SCENE_LIGHTS_SLOT);
		let pipeline = RayTracingPipeline::new(shade, vec![white_hit]).expect("the white hit should run in the shade pass");
		Self { pipeline, hits, lights }
	}

	/// Shades the frame's hits in a scene without geometry: ray `i` leaves `rays[i]`'s origin along its direction, a unit
	/// vector, and hits a surface facing back along it its distance away.
	fn shade(&mut self, set: &mut SurfelSet, rays: &[([f32; 3], [f32; 3], f32)]) {
		self.shade_in(set, rays, &AccelerationStructure::new(Vec::new()));
	}

	/// Does what [`Self::shade`] does, with `scene` as what the hits' shadow rays test.
	fn shade_in(&mut self, set: &mut SurfelSet, rays: &[([f32; 3], [f32; 3], f32)], scene: &AccelerationStructure) {
		for (ray, (origin, direction, distance)) in rays.iter().enumerate() {
			for (member, value) in [
				("origin", Value::Vec3F(*origin)),
				("distance", Value::F32(*distance)),
				("direction", Value::Vec3F(*direction)),
				("record", Value::U32(0)),
			] {
				self.hits.write_array_member(ray, member, value).expect("surfel hit");
			}
			// The sort lists the hits in ray order.
			set.sorted_hits
				.write_array_element(ray, Value::U32(ray as u32))
				.expect("sorted hit");
		}
		set.hit_sort
			.write("hit_count", Value::U32(rays.len() as u32))
			.expect("surfel hit count");
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		descriptors.bind_buffer(HITS_SLOT, &mut self.hits);
		descriptors.bind_buffer(SCENE_LIGHTS_SLOT, &mut self.lights);
		descriptors.bind_acceleration_structure(SCENE_STRUCTURE_SLOT, scene);
		self.pipeline
			.trace_rays(&mut descriptors, &[0], [rays.len() as u32, 1, 1])
			.expect("the shade pass should shade the hits");
	}
}

/// Verifies a ray's hit takes the light of a surfel that sees it and none from a surfel whose radial depth puts a wall
/// between them, as a pixel does, so bounce light doesn't pass through walls, and that a covered hit asks for no surfel.
#[test]
fn surfel_shade_occlusion_hides_a_surfel_behind_a_wall() {
	let passes = SurfelPasses::new();
	let mut shade = WhiteShade::new();
	let mut set = SurfelSet::new(&passes);
	// A ray from the camera hits the wall straight ahead, halfway between a surfel with a clear view of the hit and one
	// whose every direction ends a millimeter away.
	let clear = [0.5, 0.25, 0.125, 0.75];
	let walled = [8.0, 8.0, 8.0, 0.25];
	for (slot, x, light) in [(0, 0.06, clear), (1, -0.06, walled)] {
		set.place(slot, [x, 0.0, WALL_DEPTH], WALL_NORMAL);
		set.set_record(slot, "irradiance", Value::Vec4F16(light.map(f16::from_f32)));
	}
	set.set_depths(1, 0.001);
	passes.run_grid(&mut set);
	shade.shade(&mut set, &[([0.0; 3], [0.0, 0.0, 1.0], WALL_DEPTH)]);

	// The white surface reflects the clear surfel's light alone; the walled one's would brighten it about tenfold.
	let Value::Vec3F(radiance) = set.ray_results.read_array_member(0, "radiance").expect("ray radiance") else {
		panic!("Expected the ray's radiance.");
	};
	assert_rgba_close([radiance[0], radiance[1], radiance[2], 0.0], [0.5, 0.25, 0.125, 0.0], 0.0001);
	assert_eq!(set.pool_count("hit_spawns"), 0);
}

/// Verifies a hit reflects the scene's point and cone lights as the light table describes them, unless geometry hides
/// them: a point light half a meter in front of a white wall, of π/4 candela, gives it irradiance π and radiance one.
#[test]
fn surfel_shade_lights_a_hit_with_the_scenes_lights_unless_something_hides_them() {
	let passes = SurfelPasses::new();
	let mut shade = WhiteShade::new();
	let mut set = SurfelSet::new(&passes);
	passes.run_grid(&mut set);
	let hit = [([0.0; 3], [0.0, 0.0, 1.0], WALL_DEPTH)];
	let quarter_pi = std::f32::consts::FRAC_PI_4;
	// Light type 0 is a point light, 1 a cone light.
	let light = |shade: &mut WhiteShade, light_type: u32, direction: [f32; 3]| {
		shade.lights.write("light_count", Value::U32(1)).expect("light count");
		for (field, value) in [
			("position", Value::Vec3F([0.0, 0.0, WALL_DEPTH - 0.5])),
			("color", Value::Vec3F([quarter_pi; 3])),
			("direction", Value::Vec3F(direction)),
			("cone_cosines", Value::Vec2F([0.9, 0.8])),
			("type", Value::U32(light_type)),
			("reach", Value::F32(10.0)),
		] {
			shade.lights.write_indexed_field("lights", 0, field, value).expect("light");
		}
	};
	let radiance = |set: &SurfelSet| match set.ray_results.read_array_member(0, "radiance") {
		Ok(Value::Vec3F(radiance)) => radiance,
		value => panic!("Unexpected ray radiance: {value:?}."),
	};

	light(&mut shade, 0, [0.0; 3]);
	shade.shade(&mut set, &hit);
	let [red, green, blue] = radiance(&set);
	assert_rgba_close([red, green, blue, 0.0], [1.0, 1.0, 1.0, 0.0], 0.0001);

	// A board between the wall and the light hides it.
	let board = |x: f32, y: f32| [x, y, WALL_DEPTH - 0.25];
	let blocker = quad_scene([board(-1.0, -1.0), board(1.0, -1.0), board(1.0, 1.0), board(-1.0, 1.0)]);
	shade.shade_in(&mut set, &hit, &blocker);
	assert_eq!(radiance(&set), [0.0; 3]);

	// A cone light aimed at the wall lights it fully, and one aimed away not at all.
	light(&mut shade, 1, [0.0, 0.0, 1.0]);
	shade.shade(&mut set, &hit);
	assert!((radiance(&set)[0] - 1.0).abs() < 0.0001, "{:?}", radiance(&set));
	light(&mut shade, 1, [0.0, 0.0, -1.0]);
	shade.shade(&mut set, &hit);
	assert_eq!(radiance(&set), [0.0; 3]);
}

/// Verifies a hit no surfel covers asks for a surfel there, once per grid cell a frame, and only when a surfel the
/// camera saw cast its ray, so later bounces never spawn surfels of their own.
#[test]
fn surfel_shade_asks_for_a_surfel_where_an_uncovered_hit_lands_once_per_cell() {
	let passes = SurfelPasses::new();
	let mut shade = WhiteShade::new();
	let mut set = SurfelSet::new(&passes);
	// A surfel a pixel spawned, and one a hit spawned.
	let seen = [0.0, 0.05, 0.0];
	let bounced = [-1.0, 0.05, 0.0];
	set.place(0, seen, UP);
	set.place(1, bounced, UP);
	set.set_state(1, "bounce", 1);
	passes.run_grid(&mut set);
	// The first two hits land in one cell of a wall no surfel covers, the third in another of its cells, and the fourth,
	// of the bounced surfel's ray, in a third.
	let hits = [
		(seen, [0.5, 0.5, 2.0]),
		(seen, [0.55, 0.45, 2.0]),
		(seen, [1.5, 0.5, 2.0]),
		(bounced, [-1.5, 0.5, 2.0]),
	];
	assert_eq!(cell_of(hits[0].1), cell_of(hits[1].1));
	let rays = hits.map(|(origin, hit)| {
		let offset: [f32; 3] = std::array::from_fn(|axis| hit[axis] - origin[axis]);
		let length = length(offset);
		(origin, offset.map(|coordinate| coordinate / length), length)
	});
	for (ray, caster) in [0, 0, 0, 1].into_iter().enumerate() {
		set.rays.write_array_element(ray, Value::U32(caster)).expect("ray record");
	}
	shade.shade(&mut set, &rays);

	assert_eq!(set.pool_count("hit_spawns"), 2);
	let requests: Vec<[f32; 3]> = (0..2)
		.map(|request| match set.hit_spawns.read_array_member(request, "position") {
			Ok(Value::Vec3F(position)) => position,
			value => panic!("Unexpected spawn request: {value:?}."),
		})
		.collect();
	let near = |a: [f32; 3], b: [f32; 3]| distance(a, b) < 0.001;
	assert!(
		requests
			.iter()
			.any(|request| near(*request, hits[0].1) || near(*request, hits[1].1))
			&& requests.iter().any(|request| near(*request, hits[2].1)),
		"Expected one request in each uncovered cell the seen surfel's rays hit, found {requests:?}."
	);
}

/// Verifies the spawn requests of rays' hits become surfels one bounce from the camera, without light, while the
/// surfels hits spawned stay below a quarter of the live surfels and the pool keeps a quarter of its slots free for the
/// surfaces the camera sees, and none once either share is reached.
#[test]
fn surfel_allocate_hits_spawns_bounced_surfels_while_the_pool_has_room() {
	let passes = SurfelPasses::new();
	let requests = [[0.5, 0.5, 2.0], [1.5, 0.5, 2.0], [2.5, 0.5, 2.0]];
	// A pool of 16 slots, with the slots in use, the live surfels the grid counted, and how many of them hits spawned:
	// room for all three requests; one hit-spawned surfel of four live ones; and 12 slots in use, the most the camera's
	// reserve leaves the hits.
	for (used, bounced, expected) in [(2, 0, 3), (4, 1, 0), (12, 0, 0)] {
		let mut set = SurfelSet::new(&passes);
		set.parameters
			.write("surfel_capacity", Value::U32(16))
			.expect("surfel capacity");
		for slot in 0..used {
			set.place(slot, [-1.0 - slot as f32, 0.05, 0.0], UP);
			set.set_state(slot, "life", 7);
		}
		set.set_pool_count("high_water", used);
		set.set_pool_count("last_alive", used);
		set.set_pool_count("last_bounced", bounced);
		for (request, position) in requests.iter().enumerate() {
			for (member, value) in [
				("position", Value::Vec3F(*position)),
				("normal", Value::U32(encode_normal(WALL_NORMAL))),
			] {
				set.hit_spawns
					.write_array_member(request, member, value)
					.expect("hit spawn request");
			}
		}
		set.set_pool_count("hit_spawns", requests.len() as u32);
		dispatch(&passes.allocate_hits, &mut set, MAX_HIT_SPAWNS as u32);

		let spawned: Vec<u32> = (used..16).filter(|&slot| set.state(slot, "life") == 1).collect();
		assert_eq!(spawned.len(), expected, "{used} slots in use, {bounced} bounced");
		for slot in spawned {
			assert_eq!(set.state(slot, "bounce"), 1);
			assert_eq!(set.record(slot, "irradiance"), Value::Vec4F16([f16::ZERO; 4]));
			assert_eq!(set.record(slot, "weight"), Value::F32(0.0));
			let Value::Vec3F(position) = set.record(slot, "position") else {
				panic!("Expected a surfel position.");
			};
			assert!(requests.contains(&position), "{position:?}");
		}
	}
}

/// Returns a square floor at height `y`, 20 meters on a side, as one instance of a VM scene.
fn floor_at(y: f32) -> AccelerationStructure {
	let corner = |x: f32, z: f32| [x, y, z];
	quad_scene([
		corner(-10.0, -10.0),
		corner(10.0, -10.0),
		corner(10.0, 10.0),
		corner(-10.0, 10.0),
	])
}

/// A scene of one quad, two triangles around its `corners` in order, placed once without a transform.
fn quad_scene([a, b, c, d]: [[f32; 3]; 4]) -> AccelerationStructure {
	AccelerationStructure::new(vec![Instance {
		transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
		custom_index: 0,
		mask: 0xFF,
		hit_group_offset: 0,
		bottom_level: Arc::new(BottomLevelStructure::new(vec![vec![[a, b, c], [a, c, d]]])),
	}])
}

/// Verifies a surfel whose support ray found its surface once votes itself out when the surface moves away, while a
/// surfel whose surface the rays never found, such as one on a masked surface, lives on, and that each surfel is
/// checked on its own frames.
#[test]
fn surfel_validate_votes_out_a_surfel_whose_surface_moved() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// Slot 0 lies on the floor, and slot 7 floats above it.
	set.place(0, [0.5, 0.0, 0.5], UP);
	set.place(7, [0.5, 1.0, 0.5], UP);
	let floor = floor_at(0.0);

	// Frame 1 checks neither: each slot checks on the frames where its slot plus the frame is a multiple of 8.
	passes.validate(&mut set, &floor, 1);
	assert_eq!(set.state(0, "supported"), 0);
	passes.validate(&mut set, &floor, 0);
	assert_eq!(set.state(0, "supported"), 1);
	passes.validate(&mut set, &floor, 1);
	assert_eq!(set.state(7, "supported"), 0);
	assert_eq!(set.state(7, "touched"), 0, "a surfel never supported should live on");

	// The floor drops half a meter, out of reach of the surfel that lay on it.
	passes.validate(&mut set, &floor_at(-0.5), 8);
	assert_eq!(set.state(0, "touched"), KILL);
	assert_eq!(set.pool_count("lost"), 1);
}

/// Verifies a mature surfel whose rays keep hitting the backs of surfaces, as under a box that now covers it, votes
/// itself out within a few updates, while one whose rays hit a back face now and then lives on.
#[test]
fn surfel_integrate_votes_out_a_surfel_buried_in_geometry() {
	let passes = SurfelPasses::new();
	let ray = |back_face: bool| DrawnRay {
		radiance: [0.0; 3],
		distance: 0.05,
		weight: 1.0,
		guided: false,
		back_face,
	};

	let mut buried = SurfelSet::new(&passes);
	let rays = settle_surfel(&passes, &mut buried, 400.0, SETTLED_RECEIVED, 1.0);
	for update in 0..3 {
		integrate_drawn_rays(&passes, &mut buried, &rays, [ray(true); 4], SETTLED_POINTS);
		assert_eq!(buried.state(0, "touched"), 0, "update {update}");
	}
	// Its share of back faces passes one half on the fourth update: 1 - 0.8⁴ = 0.59.
	integrate_drawn_rays(&passes, &mut buried, &rays, [ray(true); 4], SETTLED_POINTS);
	assert_eq!(buried.state(0, "touched"), KILL);
	assert_eq!(buried.pool_count("lost"), 1);

	let mut grazing = SurfelSet::new(&passes);
	let rays = settle_surfel(&passes, &mut grazing, 400.0, SETTLED_RECEIVED, 1.0);
	for _ in 0..20 {
		integrate_drawn_rays(
			&passes,
			&mut grazing,
			&rays,
			[ray(true), ray(false), ray(false), ray(false)],
			SETTLED_POINTS,
		);
	}
	assert_eq!(grazing.state(0, "touched"), 0);
}

/// Verifies a settled surfel's long mean forgets its rays with the frames since its last update: after 480 frames
/// without rays, as a full pool may leave a surfel, it holds e⁻² of them, so new light moves it about seven times as
/// far as right after an update.
#[test]
fn surfel_integrate_forgets_old_rays_with_elapsed_frames() {
	let passes = SurfelPasses::new();
	let mut moved = Vec::new();
	for elapsed in [0, 480] {
		let mut set = SurfelSet::new(&passes);
		for (member, value) in [
			("mean", [1.0, 1.0, 1.0, 1.0]),
			("short_mean", [1.0, 1.0, 1.0, 1.0]),
			("variance", [0.01; 4]),
		] {
			set.set_estimate(0, member, Value::Vec4F(value));
		}
		set.set_estimate(0, "blend", Value::F32(1.0 / 32.0));
		let rays = settle_surfel(&passes, &mut set, 400.0, SETTLED_RECEIVED, 1.0);
		set.parameters.write("frame_index", Value::U32(elapsed)).expect("frame index");
		integrate_rays(&passes, &mut set, &rays, [([1.2; 3], 1.0); 4], SETTLED_POINTS);
		assert_eq!(set.estimate(0, "updated"), Value::U32(elapsed));
		moved.push(vec4(set.estimate(0, "mean"))[0] - 1.0);
	}
	// The light moves the mean by 0.2 times 4 / 404 right after an update, and by 0.2 times 4 / (400 e⁻² + 4) later.
	assert!((moved[0] - 0.2 * 4.0 / 404.0).abs() < 0.0005, "{moved:?}");
	assert!(
		(moved[1] - 0.2 * 4.0 / (400.0 * (-2.0f32).exp() + 4.0)).abs() < 0.001,
		"{moved:?}"
	);
}

/// Verifies a tile with an uncovered area asks for a surfel at a pixel no surfel reaches, though pixels the nearby
/// surfel barely covers also qualify and come first in the tile.
#[test]
fn surfel_resolve_elects_the_least_covered_pixel_of_an_uncovered_tile() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let surfel = WallSink::wall_point([1, 1]);
	set.place(0, surfel, WALL_NORMAL);
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	assert_eq!(sink.spawn("requested"), Value::U32(1));
	let Value::Vec3F(position) = sink.spawn("position") else {
		panic!("Expected a spawn position.");
	};
	assert!(
		distance(position, surfel) > NEAR_RADIUS * LOOKUP_SCALE,
		"The spawn at {position:?} should lie beyond the reach of the surfel at {surfel:?}."
	);
	assert!(
		(position[2] - WALL_DEPTH).abs() < 0.0001,
		"The spawn at {position:?} should lie on the wall."
	);
	assert_eq!(sink.spawn("normal"), Value::U32(encode_normal(WALL_NORMAL)));
}

/// Verifies a pixel that many overlapping surfels cover votes to kill one of them, the heaviest, and only that one.
#[test]
fn surfel_resolve_kills_the_heaviest_surfel_of_an_overcrowded_pixel() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let stacked = 0..5;
	for slot in stacked.clone() {
		set.place(slot, WallSink::wall_point([3, 3]), WALL_NORMAL);
	}
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	let killed: Vec<u32> = stacked.filter(|&slot| set.state(slot, "touched") == KILL).collect();
	assert_eq!(
		killed.len(),
		1,
		"Expected one stacked surfel to be voted out, found {killed:?}."
	);
}

/// Verifies the pixels of a surfel's own cell keep it alive while it faces them, even when it lights nothing, and that
/// pixels it doesn't face don't. The kept surfel lies between the tile's two subgroups, so both vote for it.
#[test]
fn surfel_resolve_keeps_alive_the_surfels_its_cell_faces() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let (facing, sideways) = (0, 1);
	set.place(facing, WallSink::wall_point([3, 4]), WALL_NORMAL);
	set.place(sideways, WallSink::wall_point([6, 1]), [1.0, 0.0, 0.0]);
	// Without trust, neither surfel lights a pixel, so neither earns the dominance vote.
	for slot in [facing, sideways] {
		set.set_record(slot, "weight", Value::F32(0.0));
	}
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	assert_eq!(set.state(facing, "touched"), 5);
	assert_eq!(set.state(sideways, "touched"), 0);
}

/// Verifies the allocator takes slots from the free stack while it holds any, then slots no surfel has used, and
/// spawns nothing once both run out; a spawn starts with its parent's light, which lookups trust as much as 32 of its
/// own rays.
#[test]
fn surfel_allocate_spawns_from_the_free_stack_until_it_is_empty() {
	let passes = SurfelPasses::new();
	let program = asset!("surfel-allocate.besl");
	let requests = [[0.5, 0.05, 0.5], [1.5, 0.05, 0.5], [2.5, 0.05, 0.5]];
	let parent_light = [0.5, 0.25, 0.125, 1.0].map(f16::from_f32);

	// A pool of four slots: first with slots 0 and 2 live and 1 and 3 free again, then with two slots never used.
	for (high_water, live, free, expected) in [(4, [0, 2], vec![3, 1], vec![1, 3]), (2, [0, 1], vec![], vec![2, 3])] {
		let mut set = SurfelSet::new(&passes);
		set.parameters
			.write("surfel_capacity", Value::U32(4))
			.expect("surfel capacity");
		for slot in live {
			set.place(slot, [-1.0, 0.05, 0.0], UP);
			set.set_state(slot, "life", 7);
		}
		set.set_record(0, "irradiance", Value::Vec4F16(parent_light));
		for (index, slot) in free.iter().enumerate() {
			set.free.write_array_element(index, Value::U32(*slot)).expect("free slot");
		}
		set.set_pool_count("free_count", free.len() as u32);
		set.set_pool_count("high_water", high_water);

		let mut parameters = buffer(&program, RESOLVE_PARAMETERS_SLOT);
		parameters
			.write("spawn_tile_count", Value::U32(requests.len() as u32 + 1))
			.expect("spawn tile count");
		let mut spawns = array_buffer(&program, SPAWNS_SLOT, requests.len() + 1);
		for (tile, position) in requests.iter().enumerate() {
			for (member, value) in [
				("position", Value::Vec3F(*position)),
				("normal", Value::U32(encode_normal(UP))),
				("parent", Value::U32(if tile == 0 { 0 } else { NO_SURFEL })),
				("requested", Value::U32(1)),
			] {
				spawns.write_array_member(tile, member, value).expect("spawn request");
			}
		}

		let configs: Vec<_> = (0..64).map(|lane| lane_config(lane).with_thread_id([lane, 0])).collect();
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		descriptors.bind_buffer(RESOLVE_PARAMETERS_SLOT, &mut parameters);
		descriptors.bind_buffer(SPAWNS_SLOT, &mut spawns);
		run_workgroup(&program, descriptors, &configs);

		let spawned: Vec<u32> = (0..4).filter(|&slot| set.state(slot, "life") == 1).collect();
		assert_eq!(spawned, expected, "high water {high_water}");
		assert_eq!(set.pool_count("free_count"), 0);
		assert_eq!(set.pool_count("high_water"), 4);
		assert_eq!(set.pool_count("spawned"), 2);
		let mut positions = Vec::new();
		for slot in spawned {
			assert_eq!(set.state(slot, "cell"), NO_SURFEL);
			let Value::Vec3F(position) = set.record(slot, "position") else {
				panic!("Expected a surfel position.");
			};
			let (light, prior) = if position == requests[0] {
				(parent_light, 32.0)
			} else {
				([f16::ZERO; 4], 0.0)
			};
			assert_eq!(
				set.record(slot, "irradiance"),
				Value::Vec4F16(light),
				"surfel at {position:?}"
			);
			assert_eq!(set.record(slot, "weight"), Value::F32(prior / MATURE_RAYS));
			// The estimate starts from the same light, worth the prior's rays, which the surfel's own rays outweigh.
			assert_eq!(set.estimate(slot, "mean"), Value::Vec4F(light.map(f16::to_f32)));
			assert_eq!(set.estimate(slot, "count"), Value::F32(prior));
			assert_eq!(set.estimate(slot, "rays"), Value::F32(prior));
			positions.push(position);
		}
		assert!(
			positions.iter().all(|position| requests.contains(position)) && positions[0] != positions[1],
			"Expected two distinct requested positions, found {positions:?}."
		);
	}
}

/// The temporal fixtures' half-resolution image: one 8 × 8 workgroup.
const TEMPORAL_EXTENT: u32 = 8;
/// The spatial fixtures' half-resolution image, wide enough for the farthest spiral taps around its center and for a
/// first fallback ring.
const SPATIAL_EXTENT: u32 = 64;
/// The depth of the camera-facing wall the temporal and spatial fixtures see.
const RECONSTRUCTION_DEPTH: f32 = 5.0;

/// What the previous frame's history holds at every pixel of the temporal fixture: its light, the confidence it
/// accumulated, its luminance's variance, and the depth the previous frame saw there.
#[derive(Clone, Copy)]
struct History {
	light: [f32; 4],
	confidence: f32,
	variance: f32,
	depth: f32,
}

/// The temporal tests' stand-in for the resolve: it accumulates each pixel's estimate and confidence as the resolve
/// does, with `surfel_temporal_accumulate`, but reads them and the pixel's normal from fixture images instead of
/// gathering them from surfels.
const TEMPORAL_STAND_IN: &str = "
temporal_estimate: descriptor<{ type: Texture2D, binding: 1290, access: read }>;
temporal_confidence: descriptor<{ type: Texture2D, binding: 1291, access: read }>;
temporal_normals: descriptor<{ type: Texture2D, binding: 1292, access: read }>;

main: fn (input: StageInput) -> void {
	let pixel: vec2u = input.thread_id;
	let z: f32 = fetch(depth_pyramid, pixel).x;
	surfel_temporal_accumulate(
		pixel,
		view_position(vec2f(f32(pixel.x), f32(pixel.y)), z),
		decode_stored_normal(fetch(temporal_normals, pixel)),
		fetch(temporal_estimate, pixel),
		fetch(temporal_confidence, pixel).x
	);
}
";
const STAND_IN_ESTIMATE_SLOT: ResourceSlot = ResourceSlot::new(1290);
const STAND_IN_CONFIDENCE_SLOT: ResourceSlot = ResourceSlot::new(1291);
const STAND_IN_NORMALS_SLOT: ResourceSlot = ResourceSlot::new(1292);

/// Runs the resolve's temporal accumulation over a camera-facing wall at [`RECONSTRUCTION_DEPTH`] whose every pixel
/// resolved `estimate` with `confidence`, over `history` reprojected by a camera that didn't move, and returns pixel
/// (4, 4)'s history and weights.
fn run_temporal(estimate: [f32; 4], confidence: f32, history: Option<History>) -> ([f32; 4], [f32; 4]) {
	let program = compile(
		besl::lex(stacked_on_layers_of("visibility/surfel-resolve.besl", TEMPORAL_STAND_IN))
			.expect("the temporal stand-in should link"),
	);
	let extent = TEMPORAL_EXTENT;
	let texels = |value: [f32; 4]| texture_2d(extent, extent, &vec![value; (extent * extent) as usize]);
	let mut view = screen_view_buffer(&program, VIEWS_SLOT, extent, extent);
	let mut parameters = buffer(&program, RESOLVE_PARAMETERS_SLOT);
	for (member, value) in [
		("current_view_to_previous_clip", Value::Mat4F(column_major(ssgi_projection()))),
		("current_view_to_previous_view", Value::Mat4F(IDENTITY_MATRIX)),
		("history_valid", Value::U32(history.is_some() as u32)),
		("history_exposure_ratio", Value::F32(1.0)),
	] {
		parameters.write(member, value).expect("surfel resolve parameters");
	}
	let previous = history.unwrap_or(History {
		light: [f32::NAN; 4],
		confidence: f32::NAN,
		variance: f32::NAN,
		depth: RECONSTRUCTION_DEPTH,
	});
	let mut depth = texels([RECONSTRUCTION_DEPTH, 0.0, 0.0, 1.0]);
	let mut raw = texels(estimate);
	let mut confidences = texels([confidence, 0.0, 0.0, 0.0]);
	let mut previous_history = texels(previous.light);
	let mut previous_weights = texels([previous.confidence, previous.variance, 0.0, 0.0]);
	let mut previous_depth = texels([previous.depth, 0.0, 0.0, 1.0]);
	let mut normals = texels(SSGI_WALL_NORMAL);
	let mut previous_normals = texels(SSGI_WALL_NORMAL);
	let mut history_output = empty_image(extent, extent);
	let mut weights_output = empty_image(extent, extent);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_buffer(RESOLVE_PARAMETERS_SLOT, &mut parameters);
	descriptors.bind_texture(DEPTH_PYRAMID_SLOT, &mut depth);
	descriptors.bind_texture(STAND_IN_ESTIMATE_SLOT, &mut raw);
	descriptors.bind_texture(STAND_IN_CONFIDENCE_SLOT, &mut confidences);
	descriptors.bind_texture(PREVIOUS_HISTORY_SLOT, &mut previous_history);
	descriptors.bind_texture(PREVIOUS_WEIGHTS_SLOT, &mut previous_weights);
	descriptors.bind_texture(PREVIOUS_DEPTH_SLOT, &mut previous_depth);
	descriptors.bind_texture(STAND_IN_NORMALS_SLOT, &mut normals);
	descriptors.bind_texture(PREVIOUS_NORMALS_SLOT, &mut previous_normals);
	descriptors.bind_image(HISTORY_OUTPUT_SLOT, &mut history_output);
	descriptors.bind_image(WEIGHTS_OUTPUT_SLOT, &mut weights_output);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&program, descriptors, TILE_WORKGROUP_WIDTH, [4, 4]);
	(rgba(&history_output, [4, 4]), rgba(&weights_output, [4, 4]))
}

/// Verifies a pixel blends its estimate into the history of its surface by the confidence behind each, keeps their
/// luminance's variance, and counts at most 16 frames of history, so its light can still follow.
#[test]
fn surfel_temporal_blends_the_estimate_into_history_by_confidence() {
	let history = |confidence: f32| History {
		light: [1.0; 4],
		confidence,
		variance: 0.01,
		depth: RECONSTRUCTION_DEPTH,
	};
	// An estimate 0.2 above the history is about one and a half deviations away: noise, not a change.
	let (light, weights) = run_temporal([1.2, 1.2, 1.2, 1.0], 1.0, Some(history(3.0)));
	assert_rgba_close(light, [1.05, 1.05, 1.05, 1.0], 0.0001);
	// The variance blends in the estimate's spread around the blend, as Welford's update does: (3 × 0.01 + 0.2 × 0.15) / 4.
	assert_rgba_close(weights, [4.0, 0.015, 0.0, 0.0], 0.0001);

	let (_, weights) = run_temporal([1.0; 4], 1.0, Some(history(40.0)));
	assert_eq!(weights[0], 17.0);
}

/// Verifies an estimate many deviations from the history replaces most of it, as when a light turns on, by the
/// confidence behind the estimate: a trusted estimate takes half the pixel's light at once, while a young surfel's, a
/// quarter trusted, barely moves it.
#[test]
fn surfel_temporal_follows_a_change_by_the_confidence_behind_it() {
	let history = History {
		light: [1.0; 4],
		confidence: 16.0,
		variance: 0.0001,
		depth: RECONSTRUCTION_DEPTH,
	};
	let (light, weights) = run_temporal([3.0, 3.0, 3.0, 1.0], 1.0, Some(history));
	assert_rgba_close(light, [2.0, 2.0, 2.0, 1.0], 0.0001);
	assert_eq!(weights[0], 2.0);
	// The history keeps 16 - 15 × 0.25 = 12.25 frames of confidence: (12.25 + 3 × 0.25) / 12.5 = 1.04.
	let (light, weights) = run_temporal([3.0, 3.0, 3.0, 1.0], 0.25, Some(history));
	assert_rgba_close(light, [1.04, 1.04, 1.04, 1.0], 0.0001);
	assert!((weights[0] - 12.5).abs() < 0.0001, "{weights:?}");
}

/// Verifies a pixel takes no history from where the previous frame saw another surface, and that a pixel neither its
/// estimate nor its history knows stays empty for the spatial pass to fill.
#[test]
fn surfel_temporal_rejects_history_of_another_surface() {
	let elsewhere = History {
		light: [8.0; 4],
		confidence: 16.0,
		variance: 0.01,
		depth: 8.0,
	};
	let estimate = [0.4, 0.2, 0.1, 0.5];
	let (light, weights) = run_temporal(estimate, 0.5, Some(elsewhere));
	assert_rgba_close(light, estimate, 0.00001);
	assert_eq!(weights[0], 0.5);

	let (light, weights) = run_temporal(estimate, 0.0, None);
	assert_eq!((light, weights), ([0.0; 4], [0.0; 4]));
}

/// Runs the spatial pass over the workgroup holding `pixel` of a [`SPATIAL_EXTENT`] square image of camera-facing walls,
/// where `texel` gives each pixel's depth, history light, and confidence, and returns the pixel's output.
fn run_spatial(texel: impl Fn([u32; 2]) -> (f32, [f32; 4], f32), pixel: [u32; 2]) -> [f32; 4] {
	let program = asset!("surfel-spatial.besl");
	let extent = SPATIAL_EXTENT;
	let image = |value: &dyn Fn((f32, [f32; 4], f32)) -> [f32; 4]| {
		let texels: Vec<[f32; 4]> = (0..extent * extent)
			.map(|index| value(texel([index % extent, index / extent])))
			.collect();
		texture_2d(extent, extent, &texels)
	};
	let mut view = screen_view_buffer(&program, VIEWS_SLOT, extent, extent);
	let mut depth = image(&|(depth, ..)| [depth, 0.0, 0.0, 1.0]);
	let mut history = image(&|(_, light, _)| light);
	let mut weights = image(&|(.., confidence)| [confidence, 0.0, 0.0, 0.0]);
	let mut normals = image(&|_| SSGI_WALL_NORMAL);
	let mut output = empty_image(extent, extent);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(VIEWS_SLOT, &mut view);
	descriptors.bind_texture(DEPTH_PYRAMID_SLOT, &mut depth);
	descriptors.bind_texture(SPATIAL_HISTORY_SLOT, &mut history);
	descriptors.bind_texture(SPATIAL_WEIGHTS_SLOT, &mut weights);
	descriptors.bind_texture(SPATIAL_NORMALS_SLOT, &mut normals);
	descriptors.bind_image(OUTPUT_SLOT, &mut output);
	run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&program, descriptors, TILE_WORKGROUP_WIDTH, pixel);
	rgba(&output, pixel)
}

/// Verifies a pixel whose history holds no confidence takes the light of its neighbors on its surface; that a pixel deep
/// in a strip no surfel lights yet, as the camera turns toward it, takes the light of the nearest surface the screen
/// knows, even at another depth; and that where nothing on the screen knows any light, a pixel takes none, with a hit
/// fraction of one, rather than the sky that walls may hide.
#[test]
fn surfel_spatial_fills_an_unknown_pixel_from_the_nearest_known_light() {
	let lit = [0.5, 0.25, 0.125, 0.75];
	let hole = [16, 16];

	let filled = run_spatial(
		|texel| {
			let confidence = if texel == hole { 0.0 } else { 1.0 };
			(RECONSTRUCTION_DEPTH, lit, confidence)
		},
		hole,
	);
	assert_rgba_close(filled, lit, 0.0001);

	// Only a band along the left edge knows its light, nearer than the strip and about 30 texels from the pixel: past
	// the reach of the spiral, within the first ring.
	let strip = [40, 32];
	let band = |texel: [u32; 2]| {
		if texel[0] < 12 {
			(RECONSTRUCTION_DEPTH * 0.5, lit, 1.0)
		} else {
			(RECONSTRUCTION_DEPTH, [0.0; 4], 0.0)
		}
	};
	assert_rgba_close(run_spatial(band, strip), lit, 0.0001);

	let unknown = run_spatial(|_| (RECONSTRUCTION_DEPTH, [0.0; 4], 0.0), hole);
	assert_rgba_close(unknown, [0.0, 0.0, 0.0, 1.0], 0.0001);
}

/// Verifies the filter keeps each surface's light apart: next to a nearer wall's edge, a confident pixel and an unknown
/// one both take their own wall's light, not the nearer wall's.
#[test]
fn surfel_spatial_keeps_light_on_its_own_surface() {
	let far = [0.5, 0.25, 0.125, 0.75];
	let near = [4.0, 4.0, 4.0, 1.0];
	let hole = [15, 16];
	let texel = |texel: [u32; 2]| {
		if texel[0] >= 16 {
			(RECONSTRUCTION_DEPTH * 0.5, near, 1.0)
		} else {
			(RECONSTRUCTION_DEPTH, far, if texel == hole { 0.0 } else { 1.0 })
		}
	};
	assert_rgba_close(run_spatial(texel, [14, 16]), far, 0.0001);
	assert_rgba_close(run_spatial(texel, hole), far, 0.01);
}
