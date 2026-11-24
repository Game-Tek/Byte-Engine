//! Runs the virtual shadow map's page passes and level rule in the BESL VM against the Reference's rules: receivers
//! pick coarser levels as they recede, scrolling frees the pages that entered a level's window, allocation takes free
//! pages before unneeded ones and leaves the rest waiting, and the page hierarchy ORs dirty pages up to its root.

use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, Texture, Value};

use super::super::virtual_shadows::{VIRTUAL_SHADOW_LEVELS, VIRTUAL_SHADOW_ORIGIN_BIAS, VIRTUAL_SHADOW_PAGES};
use super::{asset_program, read_u32, run_workgroup};
use crate::rendering::shader_vm_test::{array_buffer, asset_source, buffer, compile, empty_image, lane_config, slot};

const PAGES: u32 = VIRTUAL_SHADOW_PAGES;
const LEVEL_SLOTS: u32 = PAGES * PAGES;
const TABLE_ENTRIES: usize = VIRTUAL_SHADOW_LEVELS * LEVEL_SLOTS as usize;
const ORIGIN_BIAS: u32 = VIRTUAL_SHADOW_ORIGIN_BIAS as u32;
/// Matches `VIRTUAL_SHADOW_HIERARCHY_ENTRIES` and `VIRTUAL_SHADOW_LINE_WORKGROUP` in `layouts/virtual_shadows.besl`.
const HIERARCHY_ENTRIES: usize = 1365;
const LINE_WORKGROUP: u32 = 64;

const PAGE_ALLOCATED: u32 = 1 << 31;
const PAGE_REQUESTED: u32 = 1 << 30;
const PAGE_DIRTY: u32 = 1 << 28;
const PAGE_VISITED: u32 = 1 << 27;
const OWNER_ALLOCATED: u32 = 1 << 31;
const OWNER_VISITED: u32 = 1 << 29;

/// The `VirtualShadowPages` buffers and how many elements a runtime array of them holds in the fixtures.
const BUFFERS: [(&str, Option<usize>); 11] = [
	("page_table", None),
	("page_owners", None),
	("page_origins", None),
	("requests", None),
	("page_lists", None),
	("dispatches", None),
	("page_hierarchy", Some(16 * HIERARCHY_ENTRIES)),
	("page_cells", Some(4 * 256)),
	("page_frame", None),
	("caster_bounds", Some(16)),
	("caster_levels", Some(16)),
];

/// The `Pages` struct holds one fixture's `VirtualShadowPages` state, with a pool of four pages, which every pass of a
/// fixture shares.
struct Pages {
	buffers: Vec<(&'static str, Buffer)>,
	pool: Texture,
}

impl Pages {
	/// Creates empty pages for the passes in `programs`, every level placed at the biased origin with last frame's
	/// placement the same. Each buffer takes its layout from the first pass that keeps it.
	fn new(programs: &[&ExecutableProgram]) -> Self {
		let buffers = BUFFERS
			.iter()
			.filter_map(|&(name, elements)| {
				let program = programs
					.iter()
					.find(|program| program.slot(name).is_some_and(|slot| program.buffer_layout(slot).is_some()))?;
				let slot = slot(program, name);
				let buffer = match elements {
					Some(count) => array_buffer(program, slot, count),
					None => buffer(program, slot),
				};
				Some((name, buffer))
			})
			.collect();
		let mut pages = Self {
			buffers,
			pool: empty_image(256, 256),
		};
		let frame = pages.get_mut("page_frame");
		for level in 0..16 {
			frame
				.write_indexed("origin_pages", level, Value::Vec4U([ORIGIN_BIAS; 4]))
				.expect("level origin");
		}
		frame
			.write("constants", Value::Vec4F([4.0, 1.0, 0.0, 2000.0]))
			.expect("constants");
		frame.write("camera", Value::Vec4F([0.0, 0.0, 0.0, 200.0])).expect("camera");
		frame.write("pool_pages_per_row", Value::U32(2)).expect("pool row");
		frame.write("pool_pages", Value::U32(4)).expect("pool pages");
		pages
	}

	fn get(&self, name: &str) -> &Buffer {
		&self
			.buffers
			.iter()
			.find(|(buffer, _)| *buffer == name)
			.expect("fixture buffer")
			.1
	}

	fn get_mut(&mut self, name: &str) -> &mut Buffer {
		&mut self
			.buffers
			.iter_mut()
			.find(|(buffer, _)| *buffer == name)
			.expect("fixture buffer")
			.1
	}

	fn entry(&self, slot: u32) -> u32 {
		read_u32(self.get("page_table"), slot as usize)
	}

	fn set_entry(&mut self, slot: u32, value: u32) {
		// A buffer of one array member lays out as that array.
		self.get_mut("page_table")
			.write_array_element(slot as usize, Value::U32(value))
			.expect("page table entry");
	}

	fn owner(&self, physical: u32) -> u32 {
		read_u32(self.get("page_owners"), physical as usize)
	}

	fn set_owner(&mut self, physical: u32, value: u32) {
		self.get_mut("page_owners")
			.write_array_element(physical as usize, Value::U32(value))
			.expect("page owner");
	}

	/// Runs `program` over `threads` threads in workgroups of `width` lanes, one thread per `input.thread_id.x`.
	fn run_lines(&mut self, program: &ExecutableProgram, threads: u32, width: u32) {
		for group in 0..threads.div_ceil(width) {
			let configs: Vec<_> = (0..width)
				.map(|lane| {
					lane_config(lane)
						.with_thread_id([group * width + lane, 0])
						.with_threadgroup_position(group)
				})
				.collect();
			run_workgroup(program, self.bind(program), &configs);
		}
	}

	/// Binds the buffers `program` keeps, and the pool when it reads it.
	fn bind(&mut self, program: &ExecutableProgram) -> DescriptorBindings<'_> {
		let mut descriptors = DescriptorBindings::new();
		for (name, buffer) in &mut self.buffers {
			if let Some(slot) = program.slot(name).filter(|slot| program.buffer_layout(*slot).is_some()) {
				descriptors.bind_buffer(slot, buffer);
			}
		}
		if let Some(slot) = program.slot("pool") {
			descriptors.bind_texture(slot, &mut self.pool);
		}
		descriptors
	}
}

fn program(name: &str) -> ExecutableProgram {
	let builtins = besl::Builtins::new();
	compile(&asset_program(&builtins, &asset_source(&format!("visibility/{name}"))))
}

/// Verifies the receiver level rule the marking and the samplers share: level zero reaches 1.75 meters at the default
/// four-meter first level, each doubling of distance, or of the bias, adds a level, and the last level holds the rest.
#[test]
fn receivers_pick_coarser_levels_as_they_recede() {
	let builtins = besl::Builtins::new();
	let program = compile(&asset_program(&builtins,
		"use ::byte_engine::rendering::visibility::layouts::virtual_shadows::*;
cases: descriptor<{ type: vec2f[], binding: 0, access: read, memory: device }>;
levels: descriptor<{ type: u32[], binding: 1, access: read_write, memory: device }>;
main: fn (input: StageInput) -> void {
	let case: vec2f = cases[input.thread_idx];
	levels[input.thread_idx] = virtual_shadow_receiver_level(vec4f(4.0, case.y, 0.0, 2000.0), case.x);
}
",
	));
	// Each case is a distance in meters and a level bias, with the level it picks.
	let expected = [
		((0.5, 1.0), 0),
		((1.75, 1.0), 0),
		((1.8, 1.0), 1),
		((100.0, 1.0), 6),
		((100.0, 2.0), 7),
		((1.0e9, 1.0), VIRTUAL_SHADOW_LEVELS as u32 - 1),
	];
	let mut cases = array_buffer(&program, slot(&program, "cases"), expected.len());
	for (index, ((distance, bias), _)) in expected.iter().enumerate() {
		cases
			.write_array_element(index, Value::Vec2F([*distance, *bias]))
			.expect("receiver case");
	}
	let mut levels = array_buffer(&program, slot(&program, "levels"), expected.len());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot(&program, "cases"), &mut cases);
	descriptors.bind_buffer(slot(&program, "levels"), &mut levels);
	let configs: Vec<_> = (0..expected.len() as u32).map(lane_config).collect();
	run_workgroup(&program, descriptors, &configs);
	for (index, (case, level)) in expected.iter().enumerate() {
		assert_eq!(read_u32(&levels, index), *level, "{case:?}");
	}
}

/// Verifies that scrolling level zero one page to the right frees exactly the column of slots its new page took, and
/// that every other page keeps its depth while losing the frame's flags.
#[test]
fn scrolling_frees_only_the_pages_that_entered_the_window() {
	let free = program("virtual-shadow-free.besl");
	let mut pages = Pages::new(&[&free]);
	pages
		.get_mut("page_frame")
		.write_indexed(
			"origin_pages",
			0,
			Value::Vec4U([ORIGIN_BIAS + 1, ORIGIN_BIAS, ORIGIN_BIAS, ORIGIN_BIAS]),
		)
		.expect("scrolled origin");
	// Every level-zero slot holds a page that receivers needed last frame; the physical page is the slot.
	for slot in 0..LEVEL_SLOTS {
		pages.set_entry(slot, PAGE_ALLOCATED | PAGE_REQUESTED | PAGE_VISITED | slot);
		pages.set_owner(slot, OWNER_ALLOCATED | OWNER_VISITED | slot);
	}

	pages.run_lines(&free, TABLE_ENTRIES as u32, LINE_WORKGROUP);

	// The window's last column is grid column `bias + 32`, which lands in slot column zero.
	for slot in 0..LEVEL_SLOTS {
		if slot % PAGES == 0 {
			assert_eq!(pages.entry(slot), 0, "slot {slot} entered the window and is freed");
			assert_eq!(pages.owner(slot), 0, "slot {slot}'s physical page is freed");
		} else {
			assert_eq!(
				pages.entry(slot),
				PAGE_ALLOCATED | slot,
				"slot {slot} keeps its page without frame flags"
			);
			assert_eq!(
				pages.owner(slot),
				OWNER_ALLOCATED | slot,
				"slot {slot}'s owner is no longer visited"
			);
		}
	}
}

/// Verifies that requests take free pages first, then pages no receiver needed this frame, freeing the slot that held
/// one, and fail once both run out.
#[test]
fn allocation_takes_free_pages_then_unneeded_ones_then_fails() {
	let find_free = program("virtual-shadow-find-free.besl");
	let allocate = program("virtual-shadow-allocate.besl");
	let mut pages = Pages::new(&[&find_free, &allocate]);
	// Physical pages 0 and 3 are free, 1 holds a page needed this frame, and 2 one nobody needed.
	pages.set_owner(1, OWNER_ALLOCATED | OWNER_VISITED | 100);
	pages.set_entry(100, PAGE_ALLOCATED | PAGE_VISITED | 1);
	pages.set_owner(2, OWNER_ALLOCATED | 200);
	pages.set_entry(200, PAGE_ALLOCATED | 2);
	let requested = [10, 11, 12, 13];
	for (index, slot) in requested.into_iter().enumerate() {
		pages.set_entry(slot, PAGE_REQUESTED | PAGE_VISITED);
		pages
			.get_mut("requests")
			.write_indexed("slots", index, Value::U32(slot))
			.expect("request slot");
	}
	pages
		.get_mut("requests")
		.write("count", Value::U32(requested.len() as u32))
		.expect("request count");

	pages.run_lines(&find_free, 4, LINE_WORKGROUP);
	pages.run_lines(&allocate, 4, LINE_WORKGROUP);

	let physical = |pages: &Pages, slot| pages.entry(slot) & 0xFFFF;
	let mut free_taken = [physical(&pages, 10), physical(&pages, 11)];
	free_taken.sort_unstable();
	assert_eq!(free_taken, [0, 3], "the first requests take the free pages");
	for slot in [10, 11, 12] {
		let entry = pages.entry(slot);
		assert_eq!(entry & PAGE_ALLOCATED, PAGE_ALLOCATED, "slot {slot} got a page");
		assert_eq!(
			pages.owner(physical(&pages, slot)) & 0xFFFF,
			slot,
			"slot {slot}'s page names it"
		);
	}
	assert_eq!(physical(&pages, 12), 2, "the next request takes the page nobody needed");
	assert_eq!(pages.entry(200), 0, "the taken page leaves the slot that held it");
	assert_eq!(
		pages.entry(100),
		PAGE_ALLOCATED | PAGE_VISITED | 1,
		"a needed page is never taken"
	);
	assert_eq!(pages.entry(13) & PAGE_ALLOCATED, 0, "the last request finds no page");
	// The per-request passes run one workgroup of the allocation's width, and one clear workgroup per request.
	assert_eq!(read_u32(pages.get("dispatches"), 0), 1);
	assert_eq!(read_u32(pages.get("dispatches"), 3), 4);
}

/// Verifies that the page hierarchy marks the entry over a dirty page at every mip, in the level's own page
/// coordinates, and nothing else.
#[test]
fn the_page_hierarchy_ors_dirty_pages_up_to_its_root() {
	let hierarchy = program("virtual-shadow-hierarchy.besl");
	let mut pages = Pages::new(&[&hierarchy]);
	// Level one's window starts three pages right of its grid's slot zero, so page (5, 7) lives in slot column 8.
	pages
		.get_mut("page_frame")
		.write_indexed(
			"origin_pages",
			1,
			Value::Vec4U([ORIGIN_BIAS + 3, ORIGIN_BIAS, ORIGIN_BIAS + 3, ORIGIN_BIAS]),
		)
		.expect("level origin");
	pages.set_entry(LEVEL_SLOTS + 7 * PAGES + 8, PAGE_ALLOCATED | PAGE_DIRTY);
	// An allocated clean page and a dirty page without memory mark nothing.
	pages.set_entry(LEVEL_SLOTS + 20 * PAGES + 20, PAGE_ALLOCATED);
	pages.set_entry(LEVEL_SLOTS + 25 * PAGES + 25, PAGE_DIRTY);

	for level in 0..16 {
		let configs: Vec<_> = (0..256)
			.map(|lane| lane_config(lane).with_threadgroup_position(level))
			.collect();
		run_workgroup(&hierarchy, pages.bind(&hierarchy), &configs);
	}

	let base = HIERARCHY_ENTRIES;
	let offsets = [0, 1024, 1280, 1344, 1360, 1364];
	for (mip, offset) in offsets.into_iter().enumerate() {
		let side = 32 >> mip;
		let (x, y) = (5 >> mip, 7 >> mip);
		for entry in 0..side * side {
			let expected = u32::from(entry == y * side + x);
			assert_eq!(
				read_u32(pages.get("page_hierarchy"), base + offset + entry),
				expected,
				"mip {mip} entry {entry}"
			);
		}
	}
	// Level zero has no dirty page.
	assert!((0..HIERARCHY_ENTRIES).all(|entry| read_u32(pages.get("page_hierarchy"), entry) == 0));
}
