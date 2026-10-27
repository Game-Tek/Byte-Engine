//! Runs the surfel GI passes in the BESL VM on small pools: the grid, the economy, the ray grants, the hit sort, the
//! estimate update, the resolve, and the allocator.
//!
//! Every fixture centers the grid and the camera on the origin, so the cascades follow the Reference's grid rule
//! directly.

use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, RayTracingPipeline, ResourceSlot, Texture, Value};
use half::f16;

use super::{
	TILE_WORKGROUP_SIZE, TILE_WORKGROUP_WIDTH, VIEWS_SLOT, asset_program, read_u32, run_workgroup, run_workgroup_containing,
	screen_view_buffer, ssgi_ray_at,
};
use crate::rendering::shader_vm_test::{
	IDENTITY_MATRIX, array_buffer, assert_rgba_close, buffer, compile, empty_image, lane_config, rgba, stacked_asset,
	texture_2d,
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
const HITS_SLOT: ResourceSlot = ResourceSlot::new(1133);
const HIT_KEYS_SLOT: ResourceSlot = ResourceSlot::new(1134);
const HIT_SORT_SLOT: ResourceSlot = ResourceSlot::new(1135);
const SORTED_HITS_SLOT: ResourceSlot = ResourceSlot::new(1136);
const ENVIRONMENT_IRRADIANCE_SLOT: ResourceSlot = ResourceSlot::new(1137);
const GUIDES_SLOT: ResourceSlot = ResourceSlot::new(1138);
const DEPTHS_SLOT: ResourceSlot = ResourceSlot::new(1139);
const CLEARANCES_SLOT: ResourceSlot = ResourceSlot::new(1143);
const ENVIRONMENT_RADIANCE_SLOT: ResourceSlot = ResourceSlot::new(1144);
/// Each surfel's guide cells and radial depth cells.
const GUIDE_CELLS: usize = 64;
const DEPTH_CELLS: usize = 16;
const RESOLVE_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1150);
const SPAWNS_SLOT: ResourceSlot = ResourceSlot::new(1151);
const HISTORY_SLOT: ResourceSlot = ResourceSlot::new(1152);
const NORMALS_SLOT: ResourceSlot = ResourceSlot::new(1153);

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

/// The frame-wide surfel passes, in the order a frame records them.
struct SurfelPasses {
	count: ExecutableProgram,
	scan_groups: ExecutableProgram,
	scan_partials: ExecutableProgram,
	scan_add: ExecutableProgram,
	scatter: ExecutableProgram,
	sort_scan: ExecutableProgram,
	sort_scatter: ExecutableProgram,
	integrate: ExecutableProgram,
}

impl SurfelPasses {
	fn new() -> Self {
		Self {
			count: asset!("surfel-count.besl"),
			scan_groups: asset!("surfel-scan-groups.besl"),
			scan_partials: asset!("surfel-scan-partials.besl"),
			scan_add: asset!("surfel-scan-add.besl"),
			scatter: asset!("surfel-scatter.besl"),
			sort_scan: asset!("surfel-sort-scan.besl"),
			sort_scatter: asset!("surfel-sort-scatter.besl"),
			integrate: asset!("surfel-integrate.besl"),
		}
	}

	/// Runs one frame of the grid: the economy and cell counts, the three scan passes, and the scatter.
	fn run_grid(&self, set: &mut SurfelSet) {
		let scan_threads = scan_group_count() * WORKGROUP_SIZE;
		dispatch(&self.count, set, CAPACITY);
		dispatch(&self.scan_groups, set, scan_threads);
		dispatch(&self.scan_partials, set, WORKGROUP_SIZE);
		dispatch(&self.scan_add, set, scan_threads);
		dispatch(&self.scatter, set, CAPACITY);
	}
}

/// The scan's workgroups: every cell, every slot's wanted rays, and the grand total, 1024 elements per workgroup.
fn scan_group_count() -> u32 {
	(CELL_COUNT + CAPACITY + 1).div_ceil(SCAN_GROUP_ELEMENTS)
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
	/// The environment's irradiance, the same in every direction.
	environment_irradiance: Texture,
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
			ray_wants: array_buffer(&passes.count, RAY_WANTS_SLOT, capacity),
			irradiance: array_buffer(&passes.integrate, IRRADIANCE_SLOT, capacity),
			rays: array_buffer(&passes.scatter, RAYS_SLOT, RAY_BUDGET as usize),
			ray_results: array_buffer(&passes.integrate, RAY_RESULTS_SLOT, RAY_BUDGET as usize),
			hit_keys: array_buffer(&passes.sort_scatter, HIT_KEYS_SLOT, RAY_BUDGET as usize),
			hit_sort: buffer(&passes.sort_scan, HIT_SORT_SLOT),
			sorted_hits: array_buffer(&passes.sort_scatter, SORTED_HITS_SLOT, RAY_BUDGET as usize),
			guides: array_buffer(&passes.integrate, GUIDES_SLOT, capacity * GUIDE_CELLS),
			depths: array_buffer(&passes.integrate, DEPTHS_SLOT, capacity * DEPTH_CELLS),
			clearances: array_buffer(&passes.integrate, CLEARANCES_SLOT, capacity),
			environment_irradiance: texture_2d(1, 1, &[[0.0; 4]]),
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
		descriptors.bind_texture(ENVIRONMENT_IRRADIANCE_SLOT, &mut self.environment_irradiance);
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

	/// Places a newly spawned surfel in `slot` that faces `normal`, fully trusted and without light.
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
		for (member, value) in [("life", 1), ("frames", 0), ("touched", 0), ("cell", NO_SURFEL)] {
			self.set_state(slot, member, value);
		}
	}

	/// Returns the ray records the budget granted this frame, as (slot, sample) pairs in ray order.
	fn granted_rays(&self) -> Vec<(u32, u32)> {
		(0..self.pool_count("rays") as usize)
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

	/// Returns the slots the grid lists in `cell`.
	fn listed(&self, cell: u32) -> Vec<u32> {
		let start = read_u32(&self.scan, cell as usize);
		let end = read_u32(&self.scan, cell as usize + 1);
		(start..end).map(|entry| read_u32(&self.entries, entry as usize)).collect()
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

/// Verifies the grid lists every live surfel wherever a lookup on its disc reads, in both cascades next to a cascade
/// boundary, never lists a free slot, and lists the same surfels again the next frame, because each frame's scan
/// leaves the counts at zero.
#[test]
fn surfel_grid_lists_every_cell_a_surfel_overlaps_and_leaves_counts_zero() {
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
			for point in disc_points(position, normal, 0.3 * surfel_radius(position)) {
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

/// Verifies a full pool of surfels that each overlap more cells than the grid lists one surfel in takes exactly the
/// limit's entries per surfel, so the lists fit the entries the pool keeps and hold exactly the surfels their counts
/// promise.
#[test]
fn surfel_grid_lists_each_surfel_in_at_most_32_cells() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// 16 surfels near each corner of cascade zero, where each overlaps 40 to 76 cells, and half of them lie close enough
	// to cascade one to be listed in it too.
	for slot in 0..CAPACITY {
		let sign = |axis: u32| if (slot / 16) >> axis & 1 == 1 { -1.0 } else { 1.0 };
		let step = |index: u32| 2.6 + 0.05 * index as f32;
		let position = [sign(0) * step(slot % 4), sign(1) * step(slot % 16 / 4), sign(2) * 2.8];
		let tilted = [sign(0) * 0.1, sign(1) * 0.05, sign(2)];
		set.place(slot, position, tilted.map(|coordinate| coordinate / length(tilted)));
	}

	passes.run_grid(&mut set);

	assert_eq!(set.pool_count("cell_entries"), CAPACITY * LISTING_LIMIT);
	let mut listings = vec![0u32; CAPACITY as usize];
	for cell in 0..CELL_COUNT {
		for slot in set.listed(cell) {
			listings[slot as usize] += 1;
		}
	}
	// An entry the scatter left unwritten would list slot 0 again, and leave another surfel short.
	assert!(
		listings.iter().all(|&count| count == LISTING_LIMIT),
		"Every surfel should be listed in exactly {LISTING_LIMIT} cells: {listings:?}"
	);
}

/// Verifies a crowded cell charges its untouched surfels rent until they reach the end of their life, while a lone
/// surfel of the same age and a crowded surfel that pixels vote for live on, and that every dead surfel's slot,
/// including one a despawn vote killed, goes on the free stack.
#[test]
fn surfel_economy_kills_crowded_untouched_surfels_into_the_free_stack() {
	const CROWD: u32 = 70;
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	// 40 frames from the end of their lives: a frame without votes ages a lone surfel by one, but a surfel in a cell
	// of 71 by 51.
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
	assert_eq!(set.pool_count("rays_wanted"), 3 * YOUNG_RAYS);
	assert_eq!(set.pool_count("rays"), BUDGET);
	assert_eq!(set.granted_rays(), rays(0, 0..32).chain(rays(1, 0..8)).collect::<Vec<_>>());
	// Each window starts where the last one ended, and wraps around past the last wanted ray.
	passes.run_grid(&mut set);
	assert_eq!(set.granted_rays(), rays(1, 8..32).chain(rays(2, 0..16)).collect::<Vec<_>>());
	passes.run_grid(&mut set);
	assert_eq!(set.granted_rays(), rays(2, 16..32).chain(rays(0, 0..24)).collect::<Vec<_>>());
}

/// Verifies the hit sort lists every ray that hit in the order of the materials they hit, the rays on one material in
/// the order the trace counted them, leaves misses out, and leaves the counts at zero for the next frame's trace.
#[test]
fn surfel_sort_orders_hits_by_hit_group() {
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
	set.set_pool_count("rays", hits.len() as u32);

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

/// Places a settled surfel in slot 0 whose estimate holds `count` frames, runs the grid, gives each of its rays the
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
	let rays = settle_surfel(passes, set, count, exposure);
	integrate_rays(passes, set, &rays, results, points);
}

/// Places a settled surfel in slot 0 whose estimate holds `count` frames, runs the grid at `exposure`, and returns the
/// rays the budget granted it as (ray, sample) pairs. Until the grid runs again, every frame grants it the same rays.
fn settle_surfel(passes: &SurfelPasses, set: &mut SurfelSet, count: f32, exposure: f32) -> Vec<(usize, u32)> {
	set.place(0, [0.5, 0.05, 0.5], UP);
	set.set_state(0, "frames", 10);
	set.set_estimate(0, "count", Value::F32(count));
	for (member, value) in [("exposure", exposure), ("previous_exposure", exposure)] {
		set.parameters.write(member, Value::F32(value)).expect("exposure");
	}
	passes.run_grid(set);
	let granted = set.granted_rays();
	assert_eq!(granted.len(), SETTLED_RAYS as usize);
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
/// point of the equal-area square of the same index in `points`, and folds them into slot 0's estimate.
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
	});
	integrate_drawn_rays(passes, set, rays, drawn, points);
}

/// What one fixture ray brings back: its radiance, how far it went (negative for a miss), its weight in the surfel's
/// light, and whether the surfel's guide drew it rather than the cosine-weighted distribution.
#[derive(Clone, Copy)]
struct DrawnRay {
	radiance: [f32; 3],
	distance: f32,
	weight: f32,
	guided: bool,
}

/// Gives each of `rays` the result of its sample's index in `drawn`, toward the point of the equal-area square of the
/// same index in `points`, and folds them into slot 0's estimate.
fn integrate_drawn_rays(
	passes: &SurfelPasses,
	set: &mut SurfelSet,
	rays: &[(usize, u32)],
	drawn: [DrawnRay; 4],
	points: [[f32; 2]; 4],
) {
	for &(ray, sample) in rays {
		let drawn = drawn[sample as usize];
		for (member, value) in [
			("radiance", Value::Vec3F(drawn.radiance)),
			("distance", Value::F32(drawn.distance)),
			("direction", Value::U32(pack_point(points[sample as usize]))),
			("weight", Value::F32(drawn.weight)),
			("guided", Value::U32(u32::from(drawn.guided))),
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

/// Verifies a surfel's first estimate is the average of its rays: pre-exposed light, with misses bringing the
/// environment's light, and the fraction of rays that hit. Lookups trust the surfel for one frame of rays out of 32.
#[test]
fn surfel_integrate_estimates_bounce_and_occluded_fraction() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.environment_radiance = texture_2d(1, 1, &[[0.5, 1.0, 1.5, 1.0]]);
	integrate_settled_surfel(&passes, &mut set, 0.0, 2.0, SETTLED_RESULTS);

	// The hits bring (2, 4, 6) and (2, 0, 2) and the two misses the environment's (0.5, 1, 1.5) each, averaged over the
	// four rays at an exposure of 2.
	let expected = [2.5, 3.0, 5.5, 0.5];
	assert_rgba_close(vec4(set.estimate(0, "mean")), expected, 0.0001);
	assert_eq!(set.estimate(0, "count"), Value::F32(1.0));
	assert_eq!(set.record(0, "irradiance"), Value::Vec4F16(expected.map(f16::from_f32)));
	assert_eq!(set.record(0, "weight"), Value::F32(1.0 / 32.0));
}

/// Verifies a young estimate averages its frames evenly: the fourth frame's average weighs a quarter.
#[test]
fn surfel_integrate_blends_young_surfels_by_count() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	set.set_estimate(0, "mean", Value::Vec4F([1.0, 1.0, 1.0, 0.0]));
	integrate_settled_surfel(&passes, &mut set, 3.0, 1.0, [([5.0, 5.0, 5.0], 1.0); 4]);

	assert_rgba_close(vec4(set.estimate(0, "mean")), [2.0, 2.0, 2.0, 0.25], 0.0001);
	assert_eq!(set.estimate(0, "count"), Value::F32(4.0));
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
	integrate_settled_surfel(&passes, &mut set, 100.0, 1.0, [([100.0, 100.0, 100.0], 1.0); 4]);

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
	let rays = settle_surfel(&passes, &mut set, 40.0, 1.0);
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
	};

	let mut enclosed = SurfelSet::new(&passes);
	let rays = settle_surfel(&passes, &mut enclosed, 0.0, 1.0);
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
	let rays = settle_surfel(&passes, &mut open, 0.0, 1.0);
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
	};

	let mut young = SurfelSet::new(&passes);
	young.set_estimate(0, "mean", Value::Vec4F([1.0, 1.0, 1.0, 0.25]));
	let rays = settle_surfel(&passes, &mut young, 3.0, 1.0);
	integrate_drawn_rays(&passes, &mut young, &rays, [guided; 4], SETTLED_POINTS);
	// The fourth frame's light weighs a quarter: 1 + (5 - 1) / 4 = 2.
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
	let rays = settle_surfel(&passes, &mut settled, 100.0, 1.0);
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
	integrate_settled_surfel(&passes, &mut set, 40.0, 1.0, SETTLED_RESULTS);
	// The sky turns on for the second frame, so only the rays that missed then learn it.
	set.environment_radiance = texture_2d(1, 1, &[[0.5; 4]]);
	integrate_settled_surfel(&passes, &mut set, 41.0, 1.0, SETTLED_RESULTS);

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
		integrate_settled_surfel_toward(&passes, &mut set, 40.0, 1.0, [([1.0; 3], distance); 4], toward);

		let changed: Vec<usize> = (0..DEPTH_CELLS)
			.filter(|&cell| set.depth(0, cell) != moments(start))
			.collect();
		assert_eq!(changed.len(), 1, "cells at {start} m changed: {changed:?}");
		assert_rgba_close(set.depth(0, changed[0]), blend(moments(start), rate), 0.0001);
	}

	let mut set = SurfelSet::new(&passes);
	integrate_settled_surfel_toward(&passes, &mut set, 40.0, 1.0, [([0.0; 3], -1.0); 4], toward);
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
	integrate_settled_surfel(&passes, &mut set, 40.0, 1.0, [([0.0; 3], -1.0); 4]);
	let open = clearance(&set);
	assert!(
		open > 0.9 * reach && open < reach,
		"an open surfel's clearance {open} against its reach {reach}"
	);

	let wall = 0.1;
	integrate_settled_surfel(
		&passes,
		&mut set,
		40.0,
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
	const RESULTS_SLOT: ResourceSlot = ResourceSlot::new(0);
	let layer = std::fs::read_to_string(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/assets/rendering/visibility/layouts/surfel-gi.besl"
	))
	.expect("surfel GI layer");
	let source: &'static str = format!(
		"{layer}
MapResults: struct {{
	round_trip_error: f32,
	length_error: f32,
	below_horizon: u32,
	upper_half: u32,
}}
results: descriptor<{{ type: MapResults, binding: 0, access: read_write }}>;
main: fn () -> void {{
	for (let row: u32 = 0; row < 64; row = row + 1) {{
		for (let column: u32 = 0; column < 64; column = column + 1) {{
			let point: vec2f = vec2f((f32(column) + 0.5) / 64.0, (f32(row) + 0.5) / 64.0);
			let direction: vec3f = surfel_square_to_hemisphere(point);
			let back: vec2f = surfel_hemisphere_to_square(direction);
			results.round_trip_error = max(results.round_trip_error, max(abs(back.x - point.x), abs(back.y - point.y)));
			results.length_error = max(results.length_error, abs(length(direction) - 1.0));
			if (direction.z < 0.0) {{
				results.below_horizon = results.below_horizon + 1;
			}}
			if (direction.z > 0.5) {{
				results.upper_half = results.upper_half + 1;
			}}
		}}
	}}
}}
"
	)
	.leak();
	let program = compile(asset_program(source));
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
	history: Texture,
	normals: Texture,
}

impl WallSink {
	fn new() -> Self {
		let program = asset!("surfel-resolve.besl");
		let mut parameters = buffer(&program, RESOLVE_PARAMETERS_SLOT);
		for (member, value) in [
			("view_to_world", Value::Mat4F(IDENTITY_MATRIX)),
			("debug_views", Value::U32(0)),
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
			history: empty_image(RESOLVE_EXTENT, RESOLVE_EXTENT),
			normals: empty_image(RESOLVE_EXTENT, RESOLVE_EXTENT),
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
		descriptors.bind_image(HISTORY_SLOT, &mut self.history);
		descriptors.bind_image(NORMALS_SLOT, &mut self.normals);
		run_workgroup_containing::<TILE_WORKGROUP_SIZE>(&self.program, descriptors, TILE_WORKGROUP_WIDTH, [0, 0]);
	}

	fn spawn(&self, member: &str) -> Value {
		self.spawns.read_array_member(0, member).expect("surfel spawn request")
	}
}

/// Verifies each pixel blends the light of the surfels around it by their weights, and that a pixel no surfel lights,
/// such as one beside a surfel that has not faded in yet, takes the environment's light and a hit fraction of zero.
#[test]
fn surfel_resolve_blends_the_light_of_the_surfels_around_each_pixel() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let near = [0.5, 0.25, 0.125, 0.75];
	let far = [0.25, 0.5, 1.0, 0.25];
	let environment = [0.25, 0.5, 0.75, 1.0];
	set.environment_irradiance = texture_2d(1, 1, &[environment]);
	for (slot, pixel, light, weight) in [
		(0, [1, 1], near, 1.0),
		(1, [5, 1], far, 1.0),
		(2, [1, 6], [8.0, 8.0, 8.0, 1.0], 0.0),
	] {
		set.place(slot, WallSink::wall_point(pixel), WALL_NORMAL);
		set.set_record(slot, "irradiance", Value::Vec4F16(light.map(f16::from_f32)));
		set.set_record(slot, "weight", Value::F32(weight));
	}
	passes.run_grid(&mut set);
	sink.resolve(&mut set);

	assert_rgba_close(rgba(&sink.history, [1, 1]), near, 0.0001);
	assert_rgba_close(rgba(&sink.history, [5, 1]), far, 0.0001);
	// Halfway between the two lit surfels, both weigh the same.
	assert_rgba_close(
		rgba(&sink.history, [3, 1]),
		std::array::from_fn(|channel| (near[channel] + far[channel]) * 0.5),
		0.001,
	);
	// The surfel that hasn't faded in lights nothing, so its pixel, like one far from every surfel, takes the
	// environment's light as if nothing hid the sky.
	let uncovered = [environment[0], environment[1], environment[2], 0.0];
	assert_rgba_close(rgba(&sink.history, [1, 6]), uncovered, 0.0);
	assert_rgba_close(rgba(&sink.history, [6, 4]), uncovered, 0.0);
}

/// Verifies a pixel takes no light from a surfel whose radial depth puts a wall between them, and the light of a surfel
/// with a clear view, and that a pixel hidden from every surfel around it gets neither their light nor the sky.
#[test]
fn surfel_resolve_occlusion_hides_a_surfel_behind_a_wall() {
	let passes = SurfelPasses::new();
	let mut set = SurfelSet::new(&passes);
	let mut sink = WallSink::new();
	let clear = [0.5, 0.25, 0.125, 0.75];
	let walled = [8.0, 8.0, 8.0, 0.25];
	// Pixels no surfel covers would take this light, which a hidden pixel must not.
	set.environment_irradiance = texture_2d(1, 1, &[[1.0; 4]]);
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
	assert_rgba_close(rgba(&sink.history, [3, 2]), clear, 0.0001);
	assert_rgba_close(rgba(&sink.history, [2, 2]), clear, 0.0001);
	// The pixel beside the other walled surfel lies within its disc but only behind its wall.
	assert_rgba_close(rgba(&sink.history, [5, 6]), [0.0, 0.0, 0.0, 1.0], 0.0);
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

/// Verifies a ray's hit takes the light of a surfel that sees it and none from a surfel whose radial depth puts a wall
/// between them, as a pixel does, so bounce light doesn't pass through walls.
#[test]
fn surfel_shade_occlusion_hides_a_surfel_behind_a_wall() {
	let passes = SurfelPasses::new();
	let shade = compile(besl::lex(stacked_asset("visibility/surfel-shade.besl")).expect("the shade pass should link"));
	// Material hit shaders stack on the layers the shader generator gives them.
	let white_hit = besl::parse(WHITE_HIT).expect("the white hit should parse");
	let white_hit = besl::stack_layers(white_hit, super::super::shader_generator::MATERIAL_HIT_LAYERS)
		.expect("the white hit should stack on the material hit layers");
	let white_hit = compile(besl::lex(white_hit).expect("the white hit should link"));
	let mut hits = array_buffer(&shade, HITS_SLOT, RAY_BUDGET as usize);
	let pipeline = RayTracingPipeline::new(shade, vec![white_hit]).expect("the white hit should run in the shade pass");
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
	for (member, value) in [
		("origin", Value::Vec3F([0.0; 3])),
		("distance", Value::F32(WALL_DEPTH)),
		("direction", Value::U32(encode_normal([0.0, 0.0, 1.0]))),
		("record", Value::U32(0)),
	] {
		hits.write_array_member(0, member, value).expect("surfel hit");
	}
	set.hit_sort.write("hit_count", Value::U32(1)).expect("surfel hit count");

	{
		let mut descriptors = DescriptorBindings::new();
		set.bind(&mut descriptors);
		descriptors.bind_buffer(HITS_SLOT, &mut hits);
		pipeline
			.trace_rays(&mut descriptors, &[0], [1, 1, 1])
			.expect("the shade pass should shade the hit");
	}

	// The white surface reflects the clear surfel's light alone; the walled one's would brighten it about tenfold.
	let Value::Vec3F(radiance) = set.ray_results.read_array_member(0, "radiance").expect("ray radiance") else {
		panic!("Expected the ray's radiance.");
	};
	assert_rgba_close([radiance[0], radiance[1], radiance[2], 0.0], [0.5, 0.25, 0.125, 0.0], 0.0001);
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

/// Verifies the allocator takes slots from the free stack while it holds any, then slots no surfel has used, and
/// spawns nothing once both run out; a spawn starts with its parent's light.
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

		let spawned: Vec<u32> = (0..4)
			.filter(|&slot| set.state(slot, "life") == 1 && set.state(slot, "frames") == 0)
			.collect();
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
			let light = if position == requests[0] {
				parent_light
			} else {
				[f16::ZERO; 4]
			};
			assert_eq!(
				set.record(slot, "irradiance"),
				Value::Vec4F16(light),
				"surfel at {position:?}"
			);
			assert_eq!(set.record(slot, "weight"), Value::F32(0.0));
			// The estimate starts from the same light, and the surfel's first rays replace it.
			assert_eq!(set.estimate(slot, "mean"), Value::Vec4F(light.map(f16::to_f32)));
			assert_eq!(set.estimate(slot, "count"), Value::F32(0.0));
			positions.push(position);
		}
		assert!(
			positions.iter().all(|position| requests.contains(position)) && positions[0] != positions[1],
			"Expected two distinct requested positions, found {positions:?}."
		);
	}
}
