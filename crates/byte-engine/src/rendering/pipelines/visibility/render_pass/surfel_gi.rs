//! Surfel global illumination: surfels on the surfaces the camera sees trace rays to gather bounce light, which opaque
//! material evaluation reads in place of SSGI's on a device that traces rays.
//!
//! Build it in with `render.surfel-gi.enabled` and select it with `render.gi.technique` set to `surfels`. The
//! visibility pipeline manager picks which source lights each frame's indirect diffuse light; see
//! [`super::GiSettings`]. One [`SurfelPool`] serves every sink: the first sink
//! records its frame-wide stages, which age, count, and list the surfels in a grid around its camera, check that a share
//! of them still lie on a surface, trace and shade their rays, fold the rays' light into each surfel's estimate, and spawn
//! surfels where rays hit surfaces no surfel covers. Each sink's [`SurfelResolvePass`] then estimates its pixels' light
//! from the grid and asks for the surfels its view misses, blends that estimate into the pixels' history by how much
//! its surfels are trusted, and filters the result for material evaluation. The Reference's surfel GI rules describe
//! what the stages do.
//!
//! Surfel GI follows webgiya by Jure Triglav, which builds on "Global Illumination Based on Surfels" by Henrik Halén,
//! Andreas Brinck, Kyle Hayward, and Xiangshun Bei. Its temporal and spatial reconstruction, and the surfels its rays'
//! hits spawn, follow kajiya by Tomasz Stachowiak and Embark Studios, which is also the reference its quality and cost
//! are measured against. See `THIRD_PARTY_NOTICES.md`.

use std::ops::RangeInclusive;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::super::loader::ResidentEnvironment;
use super::super::shader_data::LightingData;
use super::depth_pyramid::{DEPTH_PYRAMID_MIP_COUNT, ScreenViewData};
use super::ray_hits::{RayHits, RayStages, RayTracing, ShadeStage};
use super::{ComputeStage, Pipelines, SinkHistory, SsgiTargets};
use crate::configuration::ConfigurationValue;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The surfel capacities a renderer accepts. A power of two in this range keeps every scan over surfels in whole
/// blocks of 1,024, and 2^24 is the most surfel slots a ray record's 24 bits name.
pub(crate) const SURFEL_CAPACITY_RANGE: RangeInclusive<u32> = 1 << 10..=1 << 24;
/// The surfel capacity of a renderer that doesn't set one.
pub(crate) const DEFAULT_SURFEL_CAPACITY: u32 = 32_768;
/// Ray budgets are whole multiples of this many rays, so every scan over rays covers whole blocks.
pub(crate) const SURFEL_RAY_BUDGET_STEP: u32 = 1_024;
/// The largest ray budget, which bounds the buffers that hold each ray of a frame.
pub(crate) const MAX_SURFEL_RAY_BUDGET: u32 = 1 << 20;
/// The ray budget of a renderer that doesn't set one.
pub(crate) const DEFAULT_SURFEL_RAY_BUDGET: u32 = 16_384;

/// The surfel GI compute pipelines, named as [`Pipelines::request`] takes them: the frame-wide grid stages, the trace,
/// the support check, the estimate update, and the allocation of the hits' spawns, in recording order, then each sink's
/// resolve, which also accumulates the pixels' history, allocation, and spatial pass. The hit sort comes from [`RayHits`], and the shade pass's ray
/// tracing pipeline from [`super::super::ray_tracing::HitShading`].
pub(in crate::rendering::pipelines::visibility) const SURFEL_GI_PIPELINES: [&str; 12] = [
	"surfel-count",
	"surfel-scan-groups",
	"surfel-scan-partials",
	"surfel-scan-add",
	"surfel-scatter",
	"surfel-trace",
	"surfel-validate",
	"surfel-integrate",
	"surfel-allocate-hits",
	"surfel-resolve",
	"surfel-allocate",
	"surfel-spatial",
];
/// The cells of the grid: 8 cascades of 32 × 32 × 32.
const SURFEL_CELL_COUNT: u32 = 262_144;
/// A cascade-zero cell's side in meters. The grid moves in whole cells of it.
const SURFEL_CELL_SIZE: f32 = 0.2;
/// The most cells the grid lists one surfel in, so the cell lists keep this many entries for each pool slot. Matches
/// `SURFEL_LISTING_LIMIT` in `layouts/surfel-gi.besl`.
const SURFEL_LISTING_LIMIT: u32 = 32;
/// The threads that walk one surfel's cells in the count pass and the scatter. Matches `SURFEL_GRID_LANES` in
/// `layouts/surfel-gi.besl`.
const SURFEL_GRID_LANES: u32 = 4;
/// The elements one workgroup of the scan covers, and its threads.
const SURFEL_SCAN_GROUP_ELEMENTS: u32 = 1_024;
const SURFEL_SCAN_WORKGROUP_SIZE: u32 = 256;
/// The threads of one trace workgroup.
const SURFEL_TRACE_WORKGROUP_SIZE: u32 = 64;
/// The most spawn requests rays' hits make in a frame. Matches `SURFEL_MAX_HIT_SPAWNS` in `layouts/surfel-gi.besl`.
const SURFEL_MAX_HIT_SPAWNS: u32 = 64;
/// The frames between two support checks of one surfel, and so the pool slots one thread of the check covers. Matches
/// `SURFEL_VALIDATE_PERIOD` in `surfel-validate.besl`.
const SURFEL_VALIDATE_PERIOD: u32 = 8;
/// Each surfel's guide cells, half floats, and its radial depth cells, four moments each.
const SURFEL_GUIDE_CELLS: u32 = 64;
const SURFEL_DEPTH_CELLS: u32 = 16;
/// The resolve and the reconstruction run at half the sink resolution, in 8 × 8 workgroups; each resolve workgroup makes
/// one spawn tile.
const SURFEL_RESOLVE_DIVISOR: u32 = 2;
const SURFEL_RESOLVE_WORKGROUP_WIDTH: u32 = 8;
/// The spawn tiles one sink's request buffer holds: one per 16 × 16 pixels of an 8K (7680 × 4320) sink. A larger sink
/// asks for surfels only in the tiles that fit.
const MAX_SPAWN_TILES: u32 = (7_680 / 16) * (4_320 / 16);
/// The chance that one resolve pixel one meter away asks for a surfel in a frame, times the resolve's pixel count.
/// The chance grows with depth, so distant surfaces, which take fewer pixels, still fill in (webgiya's 5000 / 64).
const SPAWN_RATE: f32 = 78.125;

/// The render-graph name of the half-resolution light the resolve accumulates from its estimates: pre-exposed light in
/// RGB and the hit fraction in alpha. The next frame's resolve reprojects it, and the spatial pass filters it into
/// `SSGI History`.
pub(crate) const SURFEL_HISTORY_TARGET: &str = "Surfel History";
/// The render-graph name of what each `Surfel History` texel holds besides its light: the confidence it accumulated
/// in R, and the variance of its light's luminance in G.
pub(crate) const SURFEL_HISTORY_WEIGHTS_TARGET: &str = "Surfel History Weights";

/// The `SurfelTargets` struct names the render-graph images surfel GI keeps for one sink besides SSGI's, which it
/// writes in SSGI's place.
///
/// They are render targets so the renderer sizes them with the sink and they can be captured by name for debugging.
/// Create them with [`create_surfel_targets`] for a sink that surfel GI can light.
#[derive(Clone, Copy)]
pub(crate) struct SurfelTargets {
	/// Named [`SURFEL_HISTORY_TARGET`].
	pub(crate) history: ghi::DynamicImageHandle,
	/// Named [`SURFEL_HISTORY_WEIGHTS_TARGET`].
	pub(crate) weights: ghi::DynamicImageHandle,
}

/// Creates the surfel GI render-graph targets for the sink that `render_pass_builder` sets up.
///
/// Next, pass them to the visibility render pass in [`super::SinkTargets`], which hands them to the sink's
/// [`SurfelResolvePass`].
pub(crate) fn create_surfel_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> SurfelTargets {
	let image = |name, format| {
		ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image)
			.name(name)
			.device_accesses(ghi::DeviceAccesses::DeviceOnly)
	};
	SurfelTargets {
		history: render_pass_builder
			.create_history_target(image(SURFEL_HISTORY_TARGET, ghi::Formats::RGBA16F), SURFEL_RESOLVE_DIVISOR),
		weights: render_pass_builder.create_history_target(
			image(SURFEL_HISTORY_WEIGHTS_TARGET, ghi::Formats::RG16F),
			SURFEL_RESOLVE_DIVISOR,
		),
	}
}

/// The `SurfelGiParameters` struct carries one frame's settings of every surfel stage. It matches the struct of the
/// same name in `layouts/surfel-gi.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelGiParameters {
	/// The first sink's camera position, snapped to whole cascade-zero cells.
	grid_origin: ghi::pod::Vec3f,
	surfel_capacity: u32,
	camera_position: ghi::pod::Vec3f,
	ray_budget: u32,
	frame_index: u32,
	exposure: f32,
	previous_exposure: f32,
	environment_intensity: f32,
}

/// The `SurfelLight` struct carries the exposures one frame's surfel light is stored at, and the environment's scale.
/// The scene's lights reach the rays' hits through the light table [`SurfelPool::new`] binds.
///
/// The visibility pipeline manager fills it from the first sink and the scene each frame surfel GI lights, and hands it
/// to [`SurfelPool::prepare`].
#[derive(Clone, Copy)]
pub(crate) struct SurfelLight {
	/// Multiplies this frame's stored light, as the lit image's exposure does.
	pub(crate) exposure: f32,
	/// The exposure the light the pool gathered last frame was stored at.
	pub(crate) previous_exposure: f32,
	/// Scales the environment's light, as it does for material evaluation.
	pub(crate) environment_intensity: f32,
}

/// The `SurfelIrradiance` struct is one surfel's light estimate. It matches `SurfelIrradiance` in
/// `layouts/surfel-gi.besl`; only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelIrradiance {
	mean: [f32; 4],
	short_mean: [f32; 4],
	variance: [f32; 4],
	blend: f32,
	inconsistency: f32,
	count: f32,
	guide_mass: f32,
	rays: f32,
	buried: f32,
	updated: u32,
	_padding: u32,
}

/// The `SurfelRayResult` struct is what one ray brings back. It matches `SurfelRayResult` in `layouts/surfel-gi.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelRayResult {
	radiance: ghi::pod::Vec3f,
	distance: f32,
	direction: u32,
	weight: f32,
	flags: u32,
}

/// The `SurfelRecord` struct is one surfel as lookups read it. It matches `SurfelRecord` in `layouts/surfel-gi.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelRecord {
	position: ghi::pod::Vec3f,
	radius: f32,
	normal: u32,
	weight: f32,
	irradiance: ghi::pod::Vec4f16,
}

/// The `SurfelState` struct is the lifecycle of one pool slot. It matches `SurfelState` in `layouts/surfel-gi.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelState {
	life: u32,
	touched: u32,
	cell: u32,
	listing: u32,
	supported: u32,
	bounce: u32,
}

/// The `SurfelHitSpawn` struct is one spawn request a ray's hit made. It matches `SurfelHitSpawn` in
/// `layouts/surfel-gi.besl`; only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelHitSpawn {
	position: ghi::pod::Vec3f,
	normal: u32,
}

/// The `SurfelPoolCounts` struct is the pool's allocator and its counts, which the host reads back for logging. It
/// matches `SurfelPool` in `layouts/surfel-gi.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SurfelPoolCounts {
	free_count: u32,
	high_water: u32,
	alive: u32,
	recycled: u32,
	spawned: u32,
	lost: u32,
	/// The live surfels the last completed grid counted.
	pub(crate) last_alive: u32,
	/// The surfels that grid recycled, and the surfels the frame before it spawned.
	pub(crate) last_recycled: u32,
	pub(crate) last_spawned: u32,
	/// The surfels the frame before voted out because their surface moved or they lay inside geometry.
	pub(crate) last_lost: u32,
	/// The cell entries that grid listed.
	pub(crate) cell_entries: u32,
	/// The rays the priority class wanted in that frame, where its window started, and how many the budget granted.
	pub(crate) priority_wanted: u32,
	priority_cursor: u32,
	pub(crate) priority_rays: u32,
	/// The same for the regular class.
	pub(crate) regular_wanted: u32,
	regular_cursor: u32,
	pub(crate) regular_rays: u32,
	hit_spawns: u32,
	/// The spawn requests the frame before made from rays' hits.
	pub(crate) last_hit_spawns: u32,
	bounced: u32,
	/// The live surfels hits spawned, as the last completed grid counted them.
	pub(crate) last_bounced: u32,
}

/// The `SurfelResolveParameters` struct carries one sink's resolve settings for a frame. It matches the struct of the
/// same name in `layouts/surfel-resolve.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelResolveParameters {
	view_to_world: ghi::pod::Mat4f,
	current_view_to_previous_clip: ghi::pod::Mat4f,
	current_view_to_previous_view: ghi::pod::Mat4f,
	debug_views: u32,
	spawn_tiles_per_row: u32,
	spawn_tile_count: u32,
	spawn_probability: f32,
	history_valid: u32,
	history_exposure_ratio: f32,
	_padding: [u32; 2],
}

/// The `SurfelSpawn` struct is one tile's spawn request. It matches `SurfelSpawn` in `layouts/surfel-resolve.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfelSpawn {
	position: ghi::pod::Vec3f,
	normal: u32,
	parent: u32,
	requested: u32,
}

/// Returns the scan's workgroups for a pool of `capacity` slots: every cell, every slot's wanted priority and regular
/// rays, and the grand total, 1,024 elements per workgroup.
fn scan_group_count(capacity: u32) -> u32 {
	(SURFEL_CELL_COUNT + capacity * 2 + 1).div_ceil(SURFEL_SCAN_GROUP_ELEMENTS)
}

/// Returns the camera position of `view`: the translation of its inverse.
fn camera_position(view: View) -> [f32; 3] {
	let position = math::inverse(view.view()).get_column(3);
	[position.x, position.y, position.z]
}

/// Snaps `position` down to whole cascade-zero cells, so the grid moves only by whole cells and keeps its lists
/// stable while the camera moves within one.
fn snapped_grid_origin(position: [f32; 3]) -> [f32; 3] {
	position.map(|coordinate| (coordinate / SURFEL_CELL_SIZE).floor() * SURFEL_CELL_SIZE)
}

/// The `SurfelPool` struct holds the surfels surfel GI keeps on the surfaces the first sink's camera sees, and the grid
/// that lists them per cell, for every sink that surfel GI lights.
///
/// The visibility pipeline manager creates one when the project builds surfel GI in and the device traces rays. Each
/// frame surfel GI lights, call [`Self::prepare`] once, and hand the result to every sink's
/// [`super::VisibilityRenderPass::prepare`].
pub(crate) struct SurfelPool {
	descriptor_set: ghi::DescriptorSetHandle,
	/// Every surfel GI compute pipeline, in the order of [`SURFEL_GI_PIPELINES`].
	pipelines: Pipelines<12>,
	parameters: ghi::DynamicBufferHandle<SurfelGiParameters>,
	/// The allocator and counts, in memory the host can read.
	counts: ghi::BufferHandle<SurfelPoolCounts>,
	capacity: u32,
	ray_budget: u32,
}

/// Returns the write that binds `environment`'s specular levels in the surfel set `descriptor_set`.
fn environment_writes(descriptor_set: ghi::DescriptorSetHandle, environment: ResidentEnvironment) -> [ghi::DescriptorWrite; 1] {
	let set = set_layouts::SURFEL_GI.binder(descriptor_set);
	let ResidentEnvironment {
		specular_image, sampler, ..
	} = environment;
	[set.sampled("surfel_environment_radiance", specular_image, sampler)]
}

impl SurfelPool {
	/// Creates an empty pool of `capacity` surfels whose rays share `ray_budget` rays a frame, and requests the surfel GI
	/// pipelines.
	///
	/// Take both sizes from [`super::super::VisibilityPipelineSettings`], which validated them at startup.
	/// `environment` is the bound environment; bind the next one with [`Self::environment_writes`]. `lighting_buffer` is
	/// the scene's light table, whose lights light what the rays hit.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		capacity: u32,
		ray_budget: u32,
		environment: ResidentEnvironment,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
	) -> Self {
		let descriptor_set = set_layouts::SURFEL_GI.create_set(context, "Surfel GI Descriptor Set");
		// Every buffer starts zeroed, which is an empty pool: no live slot, an empty free stack, and zero cell counts.
		let device_buffer = |name, length: u32| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.length(length as usize)
		};
		let records: ghi::BufferHandle<[SurfelRecord]> = context.build_buffer(device_buffer("Surfel Records", capacity));
		let states: ghi::BufferHandle<[SurfelState]> = context.build_buffer(device_buffer("Surfel States", capacity));
		let free: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Surfel Free Stack", capacity));
		// Each cell's count, fill from the end, fill from the front, and spawn claim, as `SurfelCells` lays them out.
		let cells: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Surfel Cells", SURFEL_CELL_COUNT * 4));
		let scan_groups = scan_group_count(capacity);
		// The group pass writes whole groups, so the scan holds every element of its last group.
		let scan: ghi::BufferHandle<[u32]> =
			context.build_buffer(device_buffer("Surfel Scan", scan_groups * SURFEL_SCAN_GROUP_ELEMENTS));
		let partials: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Surfel Scan Partials", scan_groups));
		let entries: ghi::BufferHandle<[u32]> =
			context.build_buffer(device_buffer("Surfel Cell Entries", capacity * SURFEL_LISTING_LIMIT));
		// Each slot's wanted rays in the priority class, then in the regular class.
		let ray_wants: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Surfel Ray Wants", capacity * 2));
		let irradiance: ghi::BufferHandle<[SurfelIrradiance]> =
			context.build_buffer(device_buffer("Surfel Irradiance", capacity));
		let rays: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Surfel Rays", ray_budget));
		let ray_results: ghi::BufferHandle<[SurfelRayResult]> =
			context.build_buffer(device_buffer("Surfel Ray Results", ray_budget));
		// Every guide cell is a half float, which the host sizes as a 16-bit element.
		let guides: ghi::BufferHandle<[u16]> =
			context.build_buffer(device_buffer("Surfel Guides", capacity * SURFEL_GUIDE_CELLS));
		let depths: ghi::BufferHandle<[[f32; 4]]> =
			context.build_buffer(device_buffer("Surfel Radial Depths", capacity * SURFEL_DEPTH_CELLS));
		let clearances: ghi::BufferHandle<[f32]> = context.build_buffer(device_buffer("Surfel Clearances", capacity));
		let hit_spawns: ghi::BufferHandle<[SurfelHitSpawn]> =
			context.build_buffer(device_buffer("Surfel Hit Spawns", SURFEL_MAX_HIT_SPAWNS));
		let counts = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Surfel Pool")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly | ghi::DeviceAccesses::CpuRead),
		);
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Surfel GI Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let set = set_layouts::SURFEL_GI.binder(descriptor_set);
		context.write(&[
			set.buffer("surfel_parameters", parameters),
			set.buffer("surfel_records", records),
			set.buffer("surfel_states", states),
			set.buffer("surfel_free", free),
			set.buffer("surfel_pool", counts),
			set.buffer("surfel_cells", cells),
			set.buffer("surfel_scan", scan),
			set.buffer("surfel_scan_partials", partials),
			set.buffer("surfel_cell_entries", entries),
			set.buffer("surfel_ray_wants", ray_wants),
			set.buffer("surfel_irradiance", irradiance),
			set.buffer("surfel_rays", rays),
			set.buffer("surfel_ray_results", ray_results),
			set.buffer("surfel_guides", guides),
			set.buffer("surfel_depths", depths),
			set.buffer("surfel_clearances", clearances),
			set.buffer("surfel_scene_lights", lighting_buffer),
			set.buffer("surfel_hit_spawns", hit_spawns),
		]);
		context.write(&environment_writes(descriptor_set, environment));
		Self {
			descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, SURFEL_GI_PIPELINES),
			parameters,
			counts,
			capacity,
			ray_budget,
		}
	}

	/// Returns the write that binds `environment`'s specular levels, whose light rays that miss bring back.
	///
	/// The visibility pipeline manager writes it whenever it binds another environment.
	pub(crate) fn environment_writes(&self, environment: ResidentEnvironment) -> [ghi::DescriptorWrite; 1] {
		environment_writes(self.descriptor_set, environment)
	}

	/// Uploads this frame's surfel settings and returns the frame's surfel work, or `None` while a surfel GI pipeline
	/// is still compiling, in which case surfel GI can't light the frame.
	///
	/// `sink` is the first sink, whose camera the grid follows. `light` carries the frame's exposures, `tracing` the
	/// sets and the pipeline the rays trace and shade with, and `hits` where the trace keeps their hits.
	pub(crate) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		sink: &Sink,
		light: SurfelLight,
		tracing: RayTracing,
		hits: &RayHits,
	) -> Option<SurfelFrame> {
		let [
			count,
			scan_groups,
			scan_partials,
			scan_add,
			scatter,
			trace,
			validate,
			integrate,
			allocate_hits,
			resolve,
			allocate,
			spatial,
		] = self.pipelines.resolve(pipeline_manager)?;
		let sort = hits.sort(pipeline_manager, self.ray_budget)?;
		let camera = camera_position(sink.view());
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = SurfelGiParameters {
			grid_origin: snapped_grid_origin(camera).into(),
			surfel_capacity: self.capacity,
			camera_position: camera.into(),
			ray_budget: self.ray_budget,
			// Only the low bits seed the random draws, so wrapping the frame index is harmless.
			frame_index: frame.key().frame_index() as u32,
			exposure: light.exposure,
			previous_exposure: light.previous_exposure,
			environment_intensity: light.environment_intensity,
		};
		frame.sync_buffer(self.parameters);

		let scan_threads = scan_group_count(self.capacity) * SURFEL_SCAN_WORKGROUP_SIZE;
		let stage = |label, pipeline, threads| ComputeStage {
			label,
			pipeline,
			descriptor_sets: [self.descriptor_set],
			extent: Extent::line(threads),
			workgroup: Extent::line(SURFEL_SCAN_WORKGROUP_SIZE),
		};
		// The trace and shade passes read the scene and its acceleration structures before the surfel set, as every
		// closest-hit shader's sets must be the first of the shade pass's.
		let traced_sets = [
			tracing.scene,
			tracing.ray_traced_scene,
			self.descriptor_set,
			hits.descriptor_set(),
		];
		Some(SurfelFrame {
			grid: [
				stage("Surfel Count", count, self.capacity * SURFEL_GRID_LANES),
				stage("Surfel Scan Groups", scan_groups, scan_threads),
				stage("Surfel Scan Partials", scan_partials, SURFEL_SCAN_WORKGROUP_SIZE),
				stage("Surfel Scan Add", scan_add, scan_threads),
				stage("Surfel Scatter", scatter, self.capacity * SURFEL_GRID_LANES),
			],
			validate: ComputeStage {
				label: "Surfel Validate",
				pipeline: validate,
				descriptor_sets: [tracing.scene, tracing.ray_traced_scene, self.descriptor_set],
				// The capacity is a power of two of at least 1,024, so the period divides it.
				extent: Extent::line(self.capacity / SURFEL_VALIDATE_PERIOD),
				workgroup: Extent::line(SURFEL_SCAN_WORKGROUP_SIZE),
			},
			rays: RayStages {
				label: "Surfel Rays",
				trace: ComputeStage {
					label: "Surfel Trace",
					pipeline: trace,
					descriptor_sets: traced_sets,
					extent: Extent::line(self.ray_budget),
					workgroup: Extent::line(SURFEL_TRACE_WORKGROUP_SIZE),
				},
				sort,
				shade: ShadeStage {
					label: "Surfel Shade",
					tracing,
					descriptor_sets: traced_sets,
					rays: self.ray_budget,
				},
			},
			update: [
				stage("Surfel Integrate", integrate, self.capacity),
				stage("Surfel Allocate Hits", allocate_hits, SURFEL_MAX_HIT_SPAWNS),
			],
			descriptor_set: self.descriptor_set,
			resolve,
			allocate,
			spatial,
		})
	}

	/// Returns the pool's counts as the GPU last left them, for logging. Frames still in flight may change them.
	pub(crate) fn counts(&self, frame: &mut ghi::implementation::Frame) -> SurfelPoolCounts {
		*frame.get_mut_buffer_slice(self.counts)
	}
}

/// The `SurfelFrame` struct holds one frame's surfel work: the frame-wide stages, which the first sink records, and the
/// pipelines every sink's resolve runs.
#[derive(Clone, Copy)]
pub(crate) struct SurfelFrame {
	/// The economy and cell count, the three scan passes, and the scatter, in recording order.
	pub(super) grid: [ComputeStage; 5],
	/// Traces the granted rays, sorts their hits, and lights what they hit.
	pub(super) rays: RayStages<4>,
	/// Checks that a share of the surfels still lie on a surface. It reads the acceleration structures and the radius
	/// the grid wrote.
	pub(super) validate: ComputeStage<3>,
	/// Folds the rays into every live surfel's estimate and record, which the resolve reads, then turns the spawn
	/// requests of the rays' hits into surfels.
	pub(super) update: [ComputeStage; 2],
	descriptor_set: ghi::DescriptorSetHandle,
	resolve: ghi::PipelineHandle,
	allocate: ghi::PipelineHandle,
	spatial: ghi::PipelineHandle,
}

/// The `SurfelResolvePass` struct lights one sink's opaque pixels from the surfel pool, and asks for the surfels its
/// view misses.
///
/// It writes the images SSGI writes, so material evaluation reads either source the same way. It runs inside the
/// screen-space stages, in SSGI's slots, after the first sink recorded the frame's grid.
pub(super) struct SurfelResolvePass {
	/// The screen view constants and the depth pyramid the stages rebuild surfaces from.
	screen_trace_descriptor_set: ghi::DescriptorSetHandle,
	/// The sink's own settings, spawn requests, and the resolve's images.
	descriptor_set: ghi::DescriptorSetHandle,
	/// The history the resolve reprojects and the history it writes.
	temporal_descriptor_set: ghi::DescriptorSetHandle,
	/// The images the spatial pass reads and writes.
	spatial_descriptor_set: ghi::DescriptorSetHandle,
	parameters: ghi::DynamicBufferHandle<SurfelResolveParameters>,
}

impl SurfelResolvePass {
	/// Wires the resolve, allocation, temporal accumulation, and spatial descriptor sets of one sink.
	///
	/// `depth_pyramid` and `view_data` come from [`super::depth_pyramid::DepthPyramidPass`]. `ssgi` are the sink's
	/// SSGI images, which the stages write in SSGI's place, `surfels` its images from [`create_surfel_targets`], and
	/// `debug_views` its debug target pool.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		depth_pyramid: ghi::DynamicImageHandle,
		view_data: ghi::DynamicBufferHandle<ScreenViewData>,
		ssgi: SsgiTargets,
		surfels: SurfelTargets,
		debug_views: &crate::rendering::DebugViews,
	) -> Self {
		let screen_trace_descriptor_set =
			set_layouts::SCREEN_TRACE.create_set(context, "Surfel Resolve Screen Trace Descriptor Set");
		let descriptor_set = set_layouts::SURFEL_RESOLVE.create_set(context, "Surfel Resolve Descriptor Set");
		let temporal_descriptor_set = set_layouts::SURFEL_TEMPORAL.create_set(context, "Surfel Temporal Descriptor Set");
		let spatial_descriptor_set = set_layouts::SURFEL_SPATIAL.create_set(context, "Surfel Spatial Descriptor Set");
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Surfel Resolve Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let spawns: ghi::BufferHandle<[SurfelSpawn]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Surfel Spawn Requests")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.length(MAX_SPAWN_TILES as usize),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let screen_trace = set_layouts::SCREEN_TRACE.binder(screen_trace_descriptor_set);
		let resolve = set_layouts::SURFEL_RESOLVE.binder(descriptor_set);
		let temporal = set_layouts::SURFEL_TEMPORAL.binder(temporal_descriptor_set);
		let spatial = set_layouts::SURFEL_SPATIAL.binder(spatial_descriptor_set);
		let mut writes = debug_views
			.targets()
			.iter()
			.enumerate()
			.map(|(slot, image)| {
				resolve.image_element(crate::rendering::debug_views::DEBUG_VIEW_TARGETS_BINDING, *image, slot as u32)
			})
			.collect::<Vec<_>>();
		writes.extend([
			screen_trace.buffer("screen_view", view_data),
			screen_trace.sampled("depth_pyramid", depth_pyramid, point_sampler),
			resolve.buffer("surfel_resolve_parameters", parameters),
			resolve.buffer("surfel_spawns", spawns),
			resolve.image("normal_output", ssgi.normals),
			temporal.sampled_with_frame("surfel_previous_history", surfels.history, point_sampler, -1),
			temporal.sampled_with_frame("surfel_previous_weights", surfels.weights, point_sampler, -1),
			temporal.sampled_with_frame("surfel_previous_depth", depth_pyramid, point_sampler, -1),
			temporal.sampled_with_frame("surfel_previous_normals", ssgi.normals, point_sampler, -1),
			temporal.image("surfel_history_output", surfels.history),
			temporal.image("surfel_weights_output", surfels.weights),
			spatial.sampled("surfel_temporal_history", surfels.history, point_sampler),
			spatial.sampled("surfel_temporal_weights", surfels.weights, point_sampler),
			spatial.sampled("surfel_spatial_normals", ssgi.normals, point_sampler),
			spatial.image("surfel_output", ssgi.history),
		]);
		context.write(&writes);
		Self {
			screen_trace_descriptor_set,
			descriptor_set,
			temporal_descriptor_set,
			spatial_descriptor_set,
			parameters,
		}
	}

	/// Uploads this frame's resolve settings and returns the sink's three surfel dispatches.
	///
	/// `surfels` is the frame's surfel work from [`SurfelPool::prepare`]. `history` describes the previous frame's view
	/// and exposure, or is `None` when its images don't hold this sink's data; only a frame that surfel GI lit leaves
	/// surfel history. `exposure` is the shared lighting exposure uploaded for every sink in this frame, and
	/// `debug_views` the shown views, packed as [`crate::rendering::DebugViews::shown`] packs them.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		surfels: &SurfelFrame,
		history: Option<SinkHistory>,
		exposure: f32,
		debug_views: u32,
	) -> SurfelResolveStages {
		let extent = sink.extent().scaled_down(SURFEL_RESOLVE_DIVISOR);
		let tiles_per_row = extent.width().div_ceil(SURFEL_RESOLVE_WORKGROUP_WIDTH);
		let tile_count = (tiles_per_row * extent.height().div_ceil(SURFEL_RESOLVE_WORKGROUP_WIDTH)).min(MAX_SPAWN_TILES);
		let (previous_clip, previous_view, history_exposure_ratio) = history
			.map(|previous| previous.reprojection(sink.view(), exposure))
			.unwrap_or((Default::default(), Default::default(), 1.0));
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = SurfelResolveParameters {
			view_to_world: math::inverse(sink.view().view()).into(),
			current_view_to_previous_clip: previous_clip.into(),
			current_view_to_previous_view: previous_view.into(),
			debug_views,
			spawn_tiles_per_row: tiles_per_row,
			spawn_tile_count: tile_count,
			spawn_probability: SPAWN_RATE / (extent.width() * extent.height()) as f32,
			history_valid: history.is_some() as u32,
			history_exposure_ratio,
			_padding: [0; 2],
		};
		frame.sync_buffer(self.parameters);
		let screen_stage = |label, pipeline, descriptor_sets| ComputeStage {
			label,
			pipeline,
			descriptor_sets,
			extent,
			workgroup: Extent::square(SURFEL_RESOLVE_WORKGROUP_WIDTH),
		};
		SurfelResolveStages {
			resolve: screen_stage(
				"Surfel Resolve",
				surfels.resolve,
				[
					self.screen_trace_descriptor_set,
					surfels.descriptor_set,
					self.descriptor_set,
					self.temporal_descriptor_set,
				],
			),
			allocate: ComputeStage {
				label: "Surfel Allocate",
				pipeline: surfels.allocate,
				descriptor_sets: [surfels.descriptor_set, self.descriptor_set],
				extent: Extent::line(tile_count),
				workgroup: Extent::line(64),
			},
			spatial: screen_stage(
				"Surfel Spatial",
				surfels.spatial,
				[
					self.screen_trace_descriptor_set,
					surfels.descriptor_set,
					self.descriptor_set,
					self.spatial_descriptor_set,
				],
			),
		}
	}
}

/// The `SurfelResolveStages` struct holds one sink's surfel dispatches, so the visibility pass can record each among the
/// other screen-space passes' stages of the same rank.
#[derive(Clone, Copy)]
pub(super) struct SurfelResolveStages {
	/// Estimates the sink's pixels' light from the grid, blends it into their reprojected history by its confidence,
	/// and elects each tile's spawn and despawn.
	pub(super) resolve: ComputeStage<4>,
	/// Turns the spawn requests into surfels.
	pub(super) allocate: ComputeStage<2>,
	/// Filters the history into the image material evaluation reads, and fills what the history lacks.
	pub(super) spatial: ComputeStage<4>,
}

/// The `SurfelGiSettings` struct holds the startup surfel GI values that runtime updates can't change, so the
/// visibility pipeline manager can accept them and reject any change.
///
/// [`super::GiSettings`] selects surfel GI at runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfelGiSettings {
	/// Whether the project built surfel GI into its shaders.
	built_in: bool,
	surfel_capacity: u32,
	ray_budget: u32,
}

impl SurfelGiSettings {
	/// Creates the controls of a renderer that built surfel GI in when `built_in`, and whose surfel buffers hold
	/// `surfel_capacity` surfels and `ray_budget` rays.
	///
	/// Take all three from [`super::super::VisibilityPipelineSettings`], which validated them at startup.
	pub(crate) fn new(built_in: bool, surfel_capacity: u32, ray_budget: u32) -> Self {
		Self {
			built_in,
			surfel_capacity,
			ray_budget,
		}
	}

	/// Applies one runtime parameter, returning the updated settings and the effective value, or leaves them unchanged.
	///
	/// Whether surfel GI is built in, the surfel capacity, and the ray budget are decided once, at startup. Their
	/// startup values, which every port receives, are accepted as they are; any other value is not set.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"enabled" => {
				if value.to_bool() == Some(self.built_in) {
					return Ok((self, ConfigurationValue::Bool(self.built_in)));
				}
				Err(format!(
					"Surfel GI enabled was not changed from `{}`. The most likely cause is that it decides once, at startup, whether surfel GI is built into the shaders. Select a technique at runtime with `render.gi.technique` instead, or set `render.surfel-gi.enabled` in `config.json` and restart the application.",
					self.built_in
				))
			}
			"surfel-capacity" => self.with_startup_size(value, self.surfel_capacity, "Surfel capacity", parameter),
			"ray-budget" => self.with_startup_size(value, self.ray_budget, "Surfel ray budget", parameter),
			_ => Err(
				"Surfel GI parameter was not set. The most likely cause is that the parameter name is unsupported. Use `enabled`, `surfel-capacity`, or `ray-budget`."
					.to_string(),
			),
		}
	}

	/// Accepts `value` for the startup size `size` only when it doesn't change it.
	fn with_startup_size(
		self,
		value: &ConfigurationValue,
		size: u32,
		label: &str,
		parameter: &str,
	) -> Result<(Self, ConfigurationValue), String> {
		if value.to_u32() == Some(size) {
			return Ok((self, ConfigurationValue::Integer(i64::from(size))));
		}
		Err(format!(
			"{label} was not changed from {size}. The most likely cause is that it sizes surfel buffers once, when the renderer starts. Set `render.surfel-gi.{parameter}` in `config.json` and restart the application."
		))
	}
}

#[cfg(test)]
mod tests {
	use std::mem::size_of;

	use super::*;
	use crate::rendering::SetLayout;

	#[test]
	fn host_records_match_the_strides_their_layout_layers_reflect() {
		let stride = |layout: &SetLayout, name: &str| {
			layout
				.interface()
				.binding(name)
				.and_then(|binding| binding.buffer_stride)
				.unwrap_or_else(|| panic!("Expected the layout to declare the buffer `{name}`")) as usize
		};

		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_parameters"),
			size_of::<SurfelGiParameters>()
		);
		assert_eq!(stride(&set_layouts::SURFEL_GI, "surfel_records"), size_of::<SurfelRecord>());
		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_irradiance"),
			size_of::<SurfelIrradiance>()
		);
		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_ray_results"),
			size_of::<SurfelRayResult>()
		);
		assert_eq!(stride(&set_layouts::SURFEL_GI, "surfel_states"), size_of::<SurfelState>());
		assert_eq!(stride(&set_layouts::SURFEL_GI, "surfel_pool"), size_of::<SurfelPoolCounts>());
		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_cells"),
			SURFEL_CELL_COUNT as usize * 4 * size_of::<u32>()
		);
		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_hit_spawns"),
			size_of::<SurfelHitSpawn>()
		);
		assert_eq!(
			stride(&set_layouts::SURFEL_GI, "surfel_scene_lights"),
			size_of::<LightingData>()
		);
		assert_eq!(
			stride(&set_layouts::SURFEL_RESOLVE, "surfel_resolve_parameters"),
			size_of::<SurfelResolveParameters>()
		);
		assert_eq!(
			stride(&set_layouts::SURFEL_RESOLVE, "surfel_spawns"),
			size_of::<SurfelSpawn>()
		);
	}

	#[test]
	fn startup_sizes_keep_their_startup_values() {
		let settings = SurfelGiSettings::new(true, 65_536, 8_192);

		// Startup parameters reach the port as text, and keep the settings as they are.
		let (kept, effective) = settings
			.with_parameter("surfel-capacity", &ConfigurationValue::Text("65536".to_string()))
			.unwrap();
		assert_eq!(kept, settings);
		assert_eq!(effective, ConfigurationValue::Integer(65_536));
		assert!(
			settings
				.with_parameter("ray-budget", &ConfigurationValue::Integer(8_192))
				.is_ok()
		);
		assert!(
			settings
				.with_parameter("enabled", &ConfigurationValue::Text("true".to_string()))
				.is_ok()
		);
		assert!(settings.with_parameter("enabled", &ConfigurationValue::Bool(false)).is_err());

		for (parameter, value) in [("surfel-capacity", 32_768), ("ray-budget", 16_384)] {
			let reason = settings
				.with_parameter(parameter, &ConfigurationValue::Integer(value))
				.unwrap_err();
			assert!(
				reason.contains(&format!("`render.surfel-gi.{parameter}` in `config.json`")),
				"{reason}"
			);
		}
	}
}
