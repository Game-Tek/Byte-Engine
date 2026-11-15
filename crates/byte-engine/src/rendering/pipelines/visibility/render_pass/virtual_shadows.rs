//! The GPU side of virtual shadow maps: the page table, the physical pool, and the passes that keep them, for the one
//! sun that `render.directional-shadows.technique = virtual` shadows.
//!
//! [`VirtualShadowMaps`] owns the state every sink's sun visibility samples, and records each frame's page passes with
//! the sink that records the frame-wide work: it frees the pages the levels scrolled past and the pages under casters
//! that changed, marks the pages that sink's receivers need, gives them physical pages, draws casters into the dirty
//! ones, and reduces their cells. See the [design note](/docs/develop/rendering/visibility/virtual-shadow-maps).
//!
//! Under cascades the state is still created, with a one-page pool, so the sun visibility resolve binds the same sets
//! either way; nothing records.

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::super::mesh_dispatch::PhaseDispatches;
use super::super::virtual_shadows::{
	ShaderVirtualShadowFrame, VIRTUAL_SHADOW_LEVELS, VIRTUAL_SHADOW_PAGE_TEXELS, VIRTUAL_SHADOW_PAGES,
	VIRTUAL_SHADOW_RESOLUTION, VirtualShadowSettings,
};
use super::depth_pyramid::{ScreenViewData, screen_view_data};
use super::{OcclusionPhase, Pipelines, opaque_layer_pipelines, record_meshlet_dispatches};
use crate::rendering::csm::MIN_CASTER_TEXELS;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

/// Page table slots: one per page of every level.
const TABLE_ENTRIES: usize = VIRTUAL_SHADOW_LEVELS * (VIRTUAL_SHADOW_PAGES * VIRTUAL_SHADOW_PAGES) as usize;
/// Each level's hierarchy of dirty pages: 32x32, 16x16, 8x8, 4x4, 2x2, and 1x1 entries, a geometric series.
const HIERARCHY_ENTRIES: usize = (4 * (VIRTUAL_SHADOW_PAGES * VIRTUAL_SHADOW_PAGES) as usize - 1) / 3;
/// Each physical page's min and max depth cells, one per 8x8 texels.
const PAGE_CELLS: usize = (VIRTUAL_SHADOW_PAGE_TEXELS as usize / 8).pow(2);
/// Workgroup width of the line-shaped page passes.
const LINE_WORKGROUP: u32 = 64;

/// The page passes' pipelines, in the order [`VirtualShadowMaps::prepare`] records them.
const PAGE_PIPELINES: [&str; 8] = [
	"virtual-shadow-free",
	"virtual-shadow-invalidate",
	"virtual-shadow-mark",
	"virtual-shadow-find-free",
	"virtual-shadow-allocate",
	"virtual-shadow-clear",
	"virtual-shadow-hierarchy",
	"virtual-shadow-cells",
];

/// The indirect dispatch records the find-free pass writes: the allocation, the clear, and the cells. They match
/// `VIRTUAL_SHADOW_*_DISPATCH` in `layouts/virtual_shadows.besl`.
type Dispatches = [[u32; 3]; 3];
const ALLOCATE_DISPATCH: usize = 0;
const CLEAR_DISPATCH: usize = 1;
const CELLS_DISPATCH: usize = 2;

/// The `VirtualShadowMark` struct is one sink's view of its receivers for the marking: its depth and its camera.
///
/// Each sink owns one, created by [`VirtualShadowMaps::create_mark`].
pub(crate) struct VirtualShadowMark {
	descriptor_set: ghi::DescriptorSetHandle,
	view: ghi::BufferHandle<ScreenViewData>,
}

/// The `VirtualShadowMaps` struct holds one sun's virtual shadow map for every sink: the page table, the physical
/// pool, and the bookkeeping the page passes share.
///
/// The visibility pipeline manager owns it through [`super::ShadowMaps`]. Every sink's sun visibility binds
/// [`Self::sampling_descriptor_set`]; the sink that records the frame-wide work records [`Self::prepare`] after its
/// depth pyramid, before the screen-space passes.
pub(crate) struct VirtualShadowMaps {
	enabled: bool,
	pool_pages: u32,
	pages_descriptor_set: ghi::DescriptorSetHandle,
	/// The set every sink's sun visibility resolve reads the map through.
	pub(crate) sampling_descriptor_set: ghi::DescriptorSetHandle,
	/// Each level's hierarchy of dirty pages, which every sink's occlusion set binds for the page draw's culling.
	pub(crate) page_hierarchy: ghi::BaseBufferHandle,
	frame: ghi::BufferHandle<ShaderVirtualShadowFrame>,
	dispatches: ghi::BufferHandle<Dispatches>,
	/// The buffers that must start at zero: the page table, the owners, the request count, the page lists, and the
	/// casters' bounds. The passes keep them consistent from then on.
	zeroed: [ghi::BaseBufferHandle; 6],
	/// Set once a recording cleared them. The recording sets it, so a frame given up after preparing clears them again.
	cleared: std::sync::Arc<std::sync::atomic::AtomicBool>,
	page_pipelines: Option<Pipelines<8>>,
	draw_pipelines: Option<Pipelines<2>>,
}

impl VirtualShadowMaps {
	/// Creates the map's buffers and pool, and requests its pipelines when `enabled`. `settings` sizes the pool; a
	/// disabled map keeps one page, enough to bind. `instance_capacity` is how many instance records the scene holds,
	/// whose bounds the invalidation keeps.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		enabled: bool,
		settings: VirtualShadowSettings,
		instance_capacity: u32,
	) -> Self {
		let (pool_pages, pages_per_row) = if enabled {
			(settings.pool_pages(), settings.pool_pages_per_row())
		} else {
			(1, 1)
		};
		// A disabled map binds its buffers but never records, so one record each is enough.
		let records = if enabled { instance_capacity as usize } else { 1 };
		let device = |name| ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination).name(name);
		let page_table = context.build_buffer::<[u32; TABLE_ENTRIES]>(device("Virtual Shadow Page Table"));
		let page_owners = context.build_buffer::<[u32; TABLE_ENTRIES]>(device("Virtual Shadow Page Owners"));
		let page_origins = context.build_buffer::<[f32; TABLE_ENTRIES]>(device("Virtual Shadow Page Origins"));
		// The request count and total, then one slot per request.
		let requests = context.build_buffer::<[u32; 2 + TABLE_ENTRIES]>(device("Virtual Shadow Page Requests"));
		// The free and unvisited counts, then each list.
		let page_lists = context.build_buffer::<[u32; 2 + 2 * TABLE_ENTRIES]>(device("Virtual Shadow Page Lists"));
		let dispatches = context.build_buffer::<Dispatches>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::Indirect).name("Virtual Shadow Page Dispatches"),
		);
		let page_hierarchy = context
			.build_buffer::<[u32]>(device("Virtual Shadow Page Hierarchy").length(VIRTUAL_SHADOW_LEVELS * HIERARCHY_ENTRIES));
		let page_cells =
			context.build_buffer::<[[f32; 2]]>(device("Virtual Shadow Page Cells").length(pool_pages as usize * PAGE_CELLS));
		let host = |name| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame)
		};
		let frame = context.build_buffer::<ShaderVirtualShadowFrame>(host("Virtual Shadow Frame"));
		// Each instance record's bounds and level word when the pages last saw it; zero, as cleared, is no record.
		let caster_bounds = context.build_buffer::<[[f32; 4]]>(device("Virtual Shadow Caster Bounds").length(records));
		let caster_levels = context.build_buffer::<[u32]>(device("Virtual Shadow Caster Levels").length(records));
		// Depth codes in 32-bit texels, which the page draw writes through atomics.
		let pool: ghi::BaseImageHandle = context
			.build_image(
				ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::Storage | ghi::Uses::Atomic | ghi::Uses::Image)
					.name("Virtual Shadow Pool")
					.extent(Extent::square(pages_per_row * VIRTUAL_SHADOW_PAGE_TEXELS)),
			)
			.into();

		let pages_descriptor_set = set_layouts::VIRTUAL_SHADOW_PAGES.create_set(context, "Virtual Shadow Pages Descriptor Set");
		let sampling_descriptor_set =
			set_layouts::VIRTUAL_SHADOW_SAMPLING.create_set(context, "Virtual Shadow Sampling Descriptor Set");
		let pages = set_layouts::VIRTUAL_SHADOW_PAGES.binder(pages_descriptor_set);
		let sampling = set_layouts::VIRTUAL_SHADOW_SAMPLING.binder(sampling_descriptor_set);
		context.write(&[
			pages.buffer("page_table", page_table),
			pages.buffer("page_owners", page_owners),
			pages.buffer("page_origins", page_origins),
			pages.buffer("requests", requests),
			pages.buffer("page_lists", page_lists),
			pages.buffer("dispatches", dispatches),
			pages.buffer("page_hierarchy", page_hierarchy),
			pages.buffer("page_cells", page_cells),
			pages.buffer("page_frame", frame),
			pages.buffer("caster_bounds", caster_bounds),
			pages.buffer("caster_levels", caster_levels),
			pages.image("pool", pool),
			sampling.buffer("sampled_page_table", page_table),
			sampling.buffer("sampled_page_origins", page_origins),
			sampling.buffer("sampled_page_cells", page_cells),
			sampling.buffer("sampled_frame", frame),
			sampling.image("sampled_pool", pool),
		]);
		Self {
			enabled,
			pool_pages,
			pages_descriptor_set,
			sampling_descriptor_set,
			page_hierarchy: page_hierarchy.into(),
			frame,
			dispatches,
			zeroed: [
				page_table.into(),
				page_owners.into(),
				requests.into(),
				page_lists.into(),
				caster_bounds.into(),
				caster_levels.into(),
			],
			cleared: Default::default(),
			page_pipelines: enabled.then(|| Pipelines::request(pipeline_manager, PAGE_PIPELINES)),
			draw_pipelines: enabled.then(|| Pipelines::sides(pipeline_manager, "virtual-shadow")),
		}
	}

	/// Returns whether the sun samples this map instead of cascades.
	pub(crate) fn enabled(&self) -> bool {
		self.enabled
	}

	/// Creates one sink's marking state, which reads `depth`, the sink's opaque depth. Hand it to [`Self::prepare`]
	/// when that sink records the frame-wide work.
	pub(crate) fn create_mark(
		&self,
		context: &mut ghi::implementation::Context,
		depth: ghi::BaseImageHandle,
	) -> VirtualShadowMark {
		let descriptor_set = set_layouts::VIRTUAL_SHADOW_MARK.create_set(context, "Virtual Shadow Mark Descriptor Set");
		let view = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Virtual Shadow Mark View")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		let mark = set_layouts::VIRTUAL_SHADOW_MARK.binder(descriptor_set);
		context.write(&[
			mark.sampled("mark_depth", depth, point_sampler),
			mark.buffer("mark_view", view),
		]);
		VirtualShadowMark { descriptor_set, view }
	}

	/// Uploads this frame's placement and changed records, and returns the page passes and the page draw, or `None` while
	/// a pipeline is still compiling or the map is disabled.
	///
	/// `mark` and `sink` are the recording sink's marking state and sink, whose receivers get pages. `meshlet_sets` are
	/// its base set and occlusion set, which the shared task shader declares. `counter` times the whole stage. Record
	/// the result after the sink's depth pyramid and before its sun visibility resolve, which samples the map.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		data: ShaderVirtualShadowFrame,
		mark: &VirtualShadowMark,
		sink: &Sink,
		dispatches: PhaseDispatches,
		meshlet_sets: [ghi::DescriptorSetHandle; 2],
		counter: ghi::CounterHandle,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let page_pipelines = self.page_pipelines.as_ref()?.resolve(pipeline_manager)?;
		let [solid, double_sided] = self.draw_pipelines.as_ref()?.resolve(pipeline_manager)?;
		let [free, invalidate, mark_pipeline, find_free, allocate, clear, hierarchy, cells] = page_pipelines;

		let changed_records = data.changed_end.saturating_sub(data.changed_first);
		*frame.map(self.frame) = data;
		*frame.map(mark.view) = screen_view_data(sink, sink.extent());

		let cleared = self.cleared.clone();
		let zeroed = self.zeroed;
		let pages = self.pages_descriptor_set;
		let mark_set = mark.descriptor_set;
		let indirect = self.dispatches;
		let pool_pages = self.pool_pages;
		let extent = sink.extent();
		let [scene, occlusion] = meshlet_sets;
		// Masked casters draw solid: the page draw has no alpha test yet.
		let draw_pipelines = opaque_layer_pipelines([solid, double_sided], Some([solid, double_sided]));
		let minimum_caster_extent = 2.0 * MIN_CASTER_TEXELS / VIRTUAL_SHADOW_RESOLUTION as f32;

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
			};

			let direct = |c: &mut ghi::implementation::CommandBufferRecording,
			              label: &str,
			              pipeline,
			              sets: &[ghi::DescriptorSetHandle],
			              threads: Extent,
			              workgroup: Extent| {
				c.start_region(|region| region.write_str(label));
				let c = c.bind_compute_pipeline(pipeline);
				c.bind_descriptor_sets(sets);
				c.dispatch(ghi::DispatchExtent::new(threads, workgroup));
				c.end_region();
			};
			let indirect_pass = |c: &mut ghi::implementation::CommandBufferRecording, label: &str, pipeline, record| {
				c.start_region(|region| region.write_str(label));
				let c = c.bind_compute_pipeline(pipeline);
				c.bind_descriptor_sets(&[pages]);
				c.indirect_dispatch(indirect, record);
				c.end_region();
			};
			let line = Extent::line(LINE_WORKGROUP);

			c.counter(counter, |c| {
				c.start_region(|region| region.write_str("Virtual Shadow Maps"));
				if !cleared.swap(true, std::sync::atomic::Ordering::Relaxed) {
					c.clear_buffers(&zeroed);
				}
				direct(c, "Free Pages", free, &[pages], Extent::line(TABLE_ENTRIES as u32), line);
				if changed_records > 0 {
					let threads = Extent::line(changed_records);
					direct(c, "Invalidate Pages", invalidate, &[scene, pages], threads, line);
				}
				direct(
					c,
					"Mark Pages",
					mark_pipeline,
					&[scene, pages, mark_set],
					extent,
					Extent::new(8, 8, 1),
				);
				direct(c, "Find Free Pages", find_free, &[pages], Extent::line(pool_pages), line);
				indirect_pass(c, "Allocate Pages", allocate, ALLOCATE_DISPATCH);
				indirect_pass(c, "Clear Pages", clear, CLEAR_DISPATCH);
				let hierarchy_threads = Extent::line(VIRTUAL_SHADOW_LEVELS as u32 * 256);
				direct(c, "Page Hierarchy", hierarchy, &[pages], hierarchy_threads, Extent::line(256));

				// Every level draws in one pass without attachments: the fragments write the pool through atomics.
				c.start_region(|region| region.write_str("Draw Pages"));
				let r = c.start_render_pass(Extent::square(VIRTUAL_SHADOW_RESOLUTION), &[]);
				record_meshlet_dispatches(
					r,
					&[scene, occlusion, pages],
					OcclusionPhase::Pages,
					dispatches.opaque_layer.into_iter().zip(draw_pipelines),
					1,
					VIRTUAL_SHADOW_LEVELS,
					minimum_caster_extent,
				);
				r.end_render_pass();
				c.end_region();

				indirect_pass(c, "Page Cells", cells, CELLS_DISPATCH);
				c.end_region();
			});
		})
	}
}
