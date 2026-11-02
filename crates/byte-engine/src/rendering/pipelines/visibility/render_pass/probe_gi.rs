//! Probe GI: cascades of irradiance probes around the camera, updated every frame by rays traced from the probes, which
//! light opaque material evaluation's indirect diffuse light in place of SSGI's on a device that traces rays.
//!
//! Build it in with `render.probe-gi.enabled` and select it with `render.gi.technique` set to `probes`. One
//! [`ProbeVolume`] serves every sink: the first sink records its frame-wide stages, which schedule the probes that most
//! need light within the frame's ray budget, trace their rays, shade the hits, and blend the rays into each probe's
//! irradiance and distance maps. Each sink's [`ProbeResolvePass`] then lights its pixels from the probes around them and
//! marks those probes, so the next schedule updates them first. The Reference's probe GI rules describe what the stages
//! do.
//!
//! Probe GI follows "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields" by Zander Majercik,
//! Jean-Philippe Guertin, Derek Nowrouzezahrai, and Morgan McGuire (JCGT 2019), and the relocation, classification,
//! scrolling, and surface bias of "Scaling Probe-Based Real-Time Dynamic Global Illumination for Production" by Zander
//! Majercik, Adam Marrs, Josef Spjut, and Morgan McGuire (JCGT 2021).

use std::ops::RangeInclusive;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::super::loader::ResidentEnvironment;
use super::super::shader_data::LightingData;
use super::depth_pyramid::{DEPTH_PYRAMID_MIP_COUNT, ScreenViewData};
use super::ray_hits::{RayHits, RayStages, RayTracing, ShadeStage};
use super::{ComputeStage, Pipelines, SsgiTargets};
use crate::configuration::ConfigurationValue;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The probe GI compute pipelines, named as [`Pipelines::request`] takes them: the schedule, the trace, and the update,
/// in recording order, then each sink's resolve. The hit sort comes from [`RayHits`], and the shade pass's ray tracing
/// pipeline from [`super::super::ray_tracing::HitShading`].
pub(in crate::rendering::pipelines::visibility) const PROBE_GI_PIPELINES: [&str; 6] = [
	"probe-schedule",
	"probe-select",
	"probe-gather",
	"probe-trace",
	"probe-update",
	"probe-resolve",
];
/// The probes of one cascade along each axis, and the cascades. Match `PROBE_GRID_X`, `PROBE_GRID_Y`, `PROBE_GRID_Z`, and
/// `PROBE_CASCADES` in `layouts/probe-volume.besl`.
const PROBE_GRID: [u32; 3] = [24, 12, 24];
const PROBE_CASCADES: usize = 4;
/// Every probe slot of every cascade. Matches `PROBE_COUNT` in `layouts/probe-volume.besl`.
const PROBE_COUNT: u32 = PROBE_GRID[0] * PROBE_GRID[1] * PROBE_GRID[2] * PROBE_CASCADES as u32;
/// The rays one probe update traces. Matches `PROBE_RAYS` in `layouts/probe-gi.besl`.
pub(crate) const PROBE_RAYS: u32 = 128;
/// The atlas tiles of one probe's irradiance and distance maps, borders included. Match `PROBE_IRRADIANCE_TILE` and
/// `PROBE_DISTANCE_TILE` in `layouts/probe-volume.besl`.
const IRRADIANCE_TILE: u32 = 8;
const DISTANCE_TILE: u32 = 16;
/// The priority buckets of the schedule. Matches `PROBE_PRIORITY_BUCKETS` in `layouts/probe-gi.besl`.
const PROBE_PRIORITY_BUCKETS: usize = 64;
/// The threads of one update workgroup, which updates one probe. Matches `PROBE_UPDATE_WORKGROUP_SIZE` in
/// `probe-update.besl`.
const PROBE_UPDATE_WORKGROUP_SIZE: u32 = 256;
/// The resolve runs at half the sink resolution, in 8 × 8 workgroups.
const PROBE_RESOLVE_DIVISOR: u32 = 2;
const PROBE_RESOLVE_WORKGROUP_WIDTH: u32 = 8;

/// The ray budgets a renderer accepts: whole probe updates, at least one and at most 8,192 a frame.
pub(crate) const PROBE_RAY_BUDGET_RANGE: RangeInclusive<u32> = PROBE_RAYS..=PROBE_RAYS * 8_192;
/// The ray budget of a renderer that doesn't set one: 512 probe updates a frame.
pub(crate) const DEFAULT_PROBE_RAY_BUDGET: u32 = PROBE_RAYS * 512;
/// The cascade-zero probe spacings a renderer accepts, in meters.
pub(crate) const PROBE_SPACING_RANGE: RangeInclusive<f32> = 0.05..=100.0;
/// The cascade-zero probe spacing of a renderer that doesn't set one, in meters.
pub(crate) const DEFAULT_PROBE_SPACING: f32 = 1.0;

/// The `ProbeCascade` struct places one cascade's window. It matches the struct of the same name in
/// `layouts/probe-volume.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeCascade {
	/// The grid coordinates of the window's first probe: whole numbers.
	start: ghi::pod::Vec3f,
	spacing: f32,
}

/// The `ProbeVolumeParameters` struct carries what every reader of the volume shares in a frame. It matches the struct
/// of the same name in `layouts/probe-volume.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeVolumeParameters {
	cascades: [ProbeCascade; PROBE_CASCADES],
	frame_index: u32,
	environment_intensity: f32,
	_padding: [u32; 2],
}

/// The `ProbeRecord` struct is one probe slot. It matches `ProbeRecord` in `layouts/probe-volume.besl`; only the GPU
/// reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeRecord {
	offset: ghi::pod::Vec3f,
	state: u32,
	coordinate: ghi::pod::Vec3f,
	updates: u32,
	updated_frame: u32,
	_padding: [u32; 3],
}

/// The `ProbeGiParameters` struct carries one frame's settings of the update stages. It matches the struct of the same
/// name in `layouts/probe-gi.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeGiParameters {
	rotation_x: ghi::pod::Vec3f,
	probe_budget: u32,
	rotation_y: ghi::pod::Vec3f,
	_padding0: u32,
	rotation_z: ghi::pod::Vec3f,
	_padding1: u32,
}

/// The `ProbeSchedule` struct is the frame's schedule and the probe counts the host reads back for logging. It matches
/// `ProbeSchedule` in `layouts/probe-gi.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ProbeSchedule {
	histogram: [u32; PROBE_PRIORITY_BUCKETS],
	threshold: u32,
	quota: u32,
	quota_taken: u32,
	count: u32,
	active: u32,
	idle: u32,
	buried: u32,
	unset: u32,
	/// The probes of each state the last completed schedule counted.
	pub(crate) last_active: u32,
	pub(crate) last_idle: u32,
	pub(crate) last_buried: u32,
	pub(crate) last_unset: u32,
	/// The probes the frame before the last schedule updated.
	pub(crate) last_count: u32,
	_padding: [u32; 3],
}

/// The `ProbeRayResult` struct is what one ray brings back. It matches `ProbeRayResult` in `layouts/probe-gi.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeRayResult {
	radiance: ghi::pod::Vec3f,
	distance: f32,
}

/// The `ProbeResolveParameters` struct carries one sink's resolve settings for a frame. It matches the struct of the
/// same name in `layouts/probe-resolve.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeResolveParameters {
	view_to_world: ghi::pod::Mat4f,
	exposure: f32,
	/// The pool slot the first shown debug view writes.
	debug_view_first_slot: u32,
	/// The shader's `bool` members: bit 0 shows the probes, bit 1 their light.
	debug_views: u8,
	_padding: [u8; 3],
	_padding_1: u32,
}

/// Returns the camera position of `view`: the translation of its inverse.
fn camera_position(view: View) -> [f32; 3] {
	let position = math::inverse(view.view()).get_column(3);
	[position.x, position.y, position.z]
}

/// Returns the window of each cascade around `camera`, whose cascade-zero probes stand `spacing` meters apart.
///
/// Each window starts at whole grid coordinates, so it moves only by whole probes, and puts the camera in its middle
/// cell: 11 probes before it and 12 after along x and z, 5 and 6 along y.
fn cascade_windows(camera: [f32; 3], spacing: f32) -> [ProbeCascade; PROBE_CASCADES] {
	std::array::from_fn(|cascade| {
		let spacing = spacing * (1u32 << cascade) as f32;
		let start =
			std::array::from_fn::<f32, 3, _>(|axis| (camera[axis] / spacing).floor() - (PROBE_GRID[axis] / 2 - 1) as f32);
		// The shaders find slots from whole grid coordinates, which stay exact in 32-bit floats.
		debug_assert!(start.iter().all(|start| start.fract() == 0.0), "{start:?}");
		ProbeCascade {
			start: start.into(),
			spacing,
		}
	})
}

/// Returns the rows of a rotation drawn from `seed`, uniform over all rotations, so each frame's rays sample new
/// directions (Shoemake, "Uniform Random Rotations", Graphics Gems III).
fn random_rotation(seed: u64) -> [[f32; 3]; 3] {
	// SplitMix64 turns the frame index into three independent uniform numbers.
	let mut state = seed;
	let mut uniform = || {
		state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = state;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^= z >> 31;
		(z >> 40) as f32 / (1u64 << 24) as f32
	};
	let (u1, u2, u3) = (uniform(), uniform(), uniform());
	let tau = std::f32::consts::TAU;
	let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
	let [x, y, z, w] = [
		a * (tau * u2).sin(),
		a * (tau * u2).cos(),
		b * (tau * u3).sin(),
		b * (tau * u3).cos(),
	];
	let rows = [
		[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
		[2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
		[2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
	];
	// A unit quaternion makes orthonormal rows, which keep the ray pattern's spacing.
	debug_assert!(
		(0..3).all(|row| (0..3).all(|other| {
			let dot: f32 = (0..3).map(|axis| rows[row][axis] * rows[other][axis]).sum();
			(dot - if row == other { 1.0 } else { 0.0 }).abs() < 1e-4
		})),
		"{rows:?}"
	);
	rows
}

/// Returns the writes that bind `environment`'s specular levels and irradiance in the probe volume set
/// `descriptor_set`.
fn environment_writes(descriptor_set: ghi::DescriptorSetHandle, environment: ResidentEnvironment) -> [ghi::DescriptorWrite; 2] {
	let set = set_layouts::PROBE_VOLUME.binder(descriptor_set);
	let ResidentEnvironment {
		diffuse_image,
		specular_image,
		sampler,
		..
	} = environment;
	[
		set.sampled("probe_environment_radiance", specular_image, sampler),
		set.sampled("probe_environment_irradiance", diffuse_image, sampler),
	]
}

/// The `ProbeVolume` struct holds the probe cascades that probe GI keeps around the first sink's camera, for every sink
/// that probe GI lights.
///
/// The visibility pipeline manager creates one when the project builds probe GI in and the device traces rays. Each
/// frame probe GI lights, call [`Self::prepare`] once, and hand the result to every sink's
/// [`super::VisibilityRenderPass::prepare`].
pub(crate) struct ProbeVolume {
	/// The volume as its readers see it: cascades, records, and atlases to sample.
	volume_set: ghi::DescriptorSetHandle,
	/// What the update stages write.
	update_set: ghi::DescriptorSetHandle,
	/// Every probe GI compute pipeline, in the order of [`PROBE_GI_PIPELINES`].
	pipelines: Pipelines<6>,
	volume_parameters: ghi::DynamicBufferHandle<ProbeVolumeParameters>,
	update_parameters: ghi::DynamicBufferHandle<ProbeGiParameters>,
	/// The schedule and its counts, in memory the host can read.
	schedule: ghi::BufferHandle<ProbeSchedule>,
	ray_budget: u32,
	spacing: f32,
}

impl ProbeVolume {
	/// Creates the cascades with every probe unset, whose updates share `ray_budget` rays a frame and whose cascade-zero
	/// probes stand `spacing` meters apart, and requests the probe GI pipelines.
	///
	/// Take both from [`super::super::VisibilityPipelineSettings`], which validated them at startup. `environment` is
	/// the bound environment; bind the next one with [`Self::environment_writes`]. `lighting_buffer` is the scene's
	/// light table, whose point and cone lights light what the probes' rays hit.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		ray_budget: u32,
		spacing: f32,
		environment: ResidentEnvironment,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
	) -> Self {
		let volume_set = set_layouts::PROBE_VOLUME.create_set(context, "Probe Volume Descriptor Set");
		let update_set = set_layouts::PROBE_GI.create_set(context, "Probe GI Descriptor Set");
		let probe_budget = ray_budget / PROBE_RAYS;
		// Every buffer starts zeroed: unset probes, an empty schedule, and zero hit counts.
		let device_buffer = |name, length: u32| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.length(length as usize)
		};
		let records: ghi::BufferHandle<[ProbeRecord]> = context.build_buffer(device_buffer("Probe Records", PROBE_COUNT));
		let seen: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Probe Seen Marks", PROBE_COUNT));
		let priorities: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Probe Priorities", PROBE_COUNT));
		let list: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Probe List", probe_budget));
		let ray_results: ghi::BufferHandle<[ProbeRayResult]> =
			context.build_buffer(device_buffer("Probe Ray Results", ray_budget));
		let schedule = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Probe Schedule")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly | ghi::DeviceAccesses::CpuRead),
		);
		// One tile per probe: the tiles of a cascade's x and y run along a row, its z along the rows, cascade after
		// cascade.
		let tiles = Extent::rectangle(PROBE_GRID[0] * PROBE_GRID[1], PROBE_GRID[2] * PROBE_CASCADES as u32);
		let mut atlas = |name, format, tile: u32| {
			context.build_image(
				ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.extent(Extent::rectangle(tiles.width() * tile, tiles.height() * tile))
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
		};
		let irradiance = atlas("Probe Irradiance", ghi::Formats::RGBA16F, IRRADIANCE_TILE);
		let distance = atlas("Probe Distance", ghi::Formats::RG16F, DISTANCE_TILE);
		let volume_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Probe Volume Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let update_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Probe GI Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let volume = set_layouts::PROBE_VOLUME.binder(volume_set);
		let update = set_layouts::PROBE_GI.binder(update_set);
		context.write(&[
			volume.buffer("probe_volume_parameters", volume_parameters),
			volume.buffer("probe_records", records),
			volume.buffer("probe_seen", seen),
			volume.texture("probe_irradiance", irradiance),
			volume.texture("probe_distance", distance),
			update.buffer("probe_gi_parameters", update_parameters),
			update.buffer("probe_priorities", priorities),
			update.buffer("probe_schedule", schedule),
			update.buffer("probe_list", list),
			update.buffer("probe_ray_results", ray_results),
			update.image("probe_irradiance_atlas", irradiance),
			update.image("probe_distance_atlas", distance),
			update.buffer("probe_scene_lights", lighting_buffer),
		]);
		context.write(&environment_writes(volume_set, environment));
		Self {
			volume_set,
			update_set,
			pipelines: Pipelines::request(pipeline_manager, PROBE_GI_PIPELINES),
			volume_parameters,
			update_parameters,
			schedule,
			ray_budget,
			spacing,
		}
	}

	/// Returns the writes that bind `environment`'s maps: the specular levels whose light rays that miss bring back, and
	/// the irradiance that lights pixels no cascade reaches.
	///
	/// The visibility pipeline manager writes them whenever it binds another environment.
	pub(crate) fn environment_writes(&self, environment: ResidentEnvironment) -> [ghi::DescriptorWrite; 2] {
		environment_writes(self.volume_set, environment)
	}

	/// Uploads this frame's probe settings and returns the frame's probe work, or `None` while a probe GI pipeline is
	/// still compiling, in which case probe GI can't light the frame.
	///
	/// `sink` is the first sink, whose camera the cascades follow. `environment_intensity` scales the environment's
	/// light, as it does for material evaluation; probe light is stored in nits, not pre-exposed, so it needs no
	/// exposure. `tracing` names the sets and the pipeline the rays trace and shade with, and `hits` where the trace
	/// keeps their hits.
	pub(crate) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		sink: &Sink,
		environment_intensity: f32,
		tracing: RayTracing,
		hits: &RayHits,
	) -> Option<ProbeFrame> {
		let [schedule, select, gather, trace, update, resolve] = self.pipelines.resolve(pipeline_manager)?;
		let sort = hits.sort(pipeline_manager, self.ray_budget)?;
		let camera = camera_position(sink.view());
		let frame_index = frame.key().frame_index();
		*frame.get_mut_dynamic_buffer_slice(self.volume_parameters) = ProbeVolumeParameters {
			cascades: cascade_windows(camera, self.spacing),
			// Only differences between frame indices matter, so wrapping is harmless.
			frame_index: frame_index as u32,
			environment_intensity,
			_padding: [0; 2],
		};
		frame.sync_buffer(self.volume_parameters);
		let [rotation_x, rotation_y, rotation_z] = random_rotation(frame_index);
		let probe_budget = self.ray_budget / PROBE_RAYS;
		*frame.get_mut_dynamic_buffer_slice(self.update_parameters) = ProbeGiParameters {
			rotation_x: rotation_x.into(),
			probe_budget,
			rotation_y: rotation_y.into(),
			_padding0: 0,
			rotation_z: rotation_z.into(),
			_padding1: 0,
		};
		frame.sync_buffer(self.update_parameters);

		let sets = [self.volume_set, self.update_set];
		let stage = |label, pipeline, threads, workgroup| ComputeStage {
			label,
			pipeline,
			descriptor_sets: sets,
			extent: Extent::line(threads),
			workgroup: Extent::line(workgroup),
		};
		// The trace and shade passes read the scene and its acceleration structures before the probe sets, as every
		// closest-hit shader's sets must be the first of the shade pass's.
		let traced_sets = [
			tracing.scene,
			tracing.ray_traced_scene,
			self.volume_set,
			self.update_set,
			hits.descriptor_set(),
		];
		Some(ProbeFrame {
			schedule: [
				stage("Probe Schedule", schedule, PROBE_COUNT, 64),
				stage("Probe Select", select, 1, 1),
				stage("Probe Gather", gather, PROBE_COUNT, 64),
			],
			rays: RayStages {
				label: "Probe Rays",
				trace: ComputeStage {
					label: "Probe Trace",
					pipeline: trace,
					descriptor_sets: traced_sets,
					extent: Extent::line(self.ray_budget),
					workgroup: Extent::line(64),
				},
				sort,
				shade: ShadeStage {
					label: "Probe Shade",
					tracing,
					descriptor_sets: traced_sets,
					rays: self.ray_budget,
				},
			},
			update: stage(
				"Probe Update",
				update,
				probe_budget * PROBE_UPDATE_WORKGROUP_SIZE,
				PROBE_UPDATE_WORKGROUP_SIZE,
			),
			volume_set: self.volume_set,
			resolve,
		})
	}

	/// Returns the schedule's counts as the GPU last left them, for logging. Frames still in flight may change them.
	pub(crate) fn schedule(&self, frame: &mut ghi::implementation::Frame) -> ProbeSchedule {
		*frame.get_mut_buffer_slice(self.schedule)
	}
}

/// The `ProbeFrame` struct holds one frame's probe work: the frame-wide stages, which the first sink records, and the
/// pipeline every sink's resolve runs.
#[derive(Clone, Copy)]
pub(crate) struct ProbeFrame {
	/// Ranks every probe, finds the frame's cut, and lists the probes that made it.
	pub(super) schedule: [ComputeStage<2>; 3],
	/// Traces the listed probes' rays, sorts their hits, and lights what they hit.
	pub(super) rays: RayStages<5>,
	/// Blends each listed probe's rays into its maps, then relocates and classifies it.
	pub(super) update: ComputeStage<2>,
	volume_set: ghi::DescriptorSetHandle,
	resolve: ghi::PipelineHandle,
}

/// The `ProbeResolvePass` struct lights one sink's opaque pixels from the probe volume, and marks the probes they read.
///
/// It writes the images SSGI writes, so material evaluation reads either source the same way. It runs inside the
/// screen-space stages, in SSGI's first slot, after the first sink recorded the frame's probe updates.
pub(super) struct ProbeResolvePass {
	/// The screen view constants and the depth pyramid the resolve rebuilds surfaces from.
	screen_trace_descriptor_set: ghi::DescriptorSetHandle,
	/// The sink's own settings and output images.
	descriptor_set: ghi::DescriptorSetHandle,
	parameters: ghi::DynamicBufferHandle<ProbeResolveParameters>,
}

impl ProbeResolvePass {
	/// Wires the resolve descriptor sets of one sink.
	///
	/// `depth_pyramid` and `view_data` come from [`super::depth_pyramid::DepthPyramidPass`]. `targets` are the sink's
	/// SSGI images, which the resolve writes in SSGI's place, and `debug_views` the sink's debug target pool.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		depth_pyramid: ghi::DynamicImageHandle,
		view_data: ghi::DynamicBufferHandle<ScreenViewData>,
		targets: SsgiTargets,
		debug_views: &crate::rendering::DebugViews,
	) -> Self {
		let screen_trace_descriptor_set =
			set_layouts::SCREEN_TRACE.create_set(context, "Probe Resolve Screen Trace Descriptor Set");
		let descriptor_set = set_layouts::PROBE_RESOLVE.create_set(context, "Probe Resolve Descriptor Set");
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Probe Resolve Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let screen_trace = set_layouts::SCREEN_TRACE.binder(screen_trace_descriptor_set);
		let resolve = set_layouts::PROBE_RESOLVE.binder(descriptor_set);
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
			resolve.buffer("probe_resolve_parameters", parameters),
			resolve.image("history_output", targets.history),
			resolve.image("normal_output", targets.normals),
		]);
		context.write(&writes);
		Self {
			screen_trace_descriptor_set,
			descriptor_set,
			parameters,
		}
	}

	/// Uploads this frame's resolve settings and returns the resolve.
	///
	/// `probes` is the frame's probe work from [`ProbeVolume::prepare`], `exposure` the frame's shared exposure, and
	/// `shown` the probe views to write, from [`crate::rendering::DebugViews::shown`].
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		probes: &ProbeFrame,
		exposure: f32,
		shown: crate::rendering::ShownDebugViews,
	) -> ComputeStage<3> {
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = ProbeResolveParameters {
			view_to_world: math::inverse(sink.view().view()).into(),
			exposure,
			debug_view_first_slot: shown.first_slot,
			debug_views: shown.views,
			_padding: [0; 3],
			_padding_1: 0,
		};
		frame.sync_buffer(self.parameters);
		ComputeStage {
			label: "Probe Resolve",
			pipeline: probes.resolve,
			descriptor_sets: [self.screen_trace_descriptor_set, probes.volume_set, self.descriptor_set],
			extent: sink.extent().scaled_down(PROBE_RESOLVE_DIVISOR),
			workgroup: Extent::square(PROBE_RESOLVE_WORKGROUP_WIDTH),
		}
	}
}

/// The `ProbeGiSettings` struct holds the startup probe GI values that runtime updates can't change, so the visibility
/// pipeline manager can accept them and reject any change.
///
/// [`super::GiSettings`] selects probe GI at runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProbeGiSettings {
	/// Whether the project built probe GI into its shaders.
	built_in: bool,
	ray_budget: u32,
	spacing: f32,
}

impl ProbeGiSettings {
	/// Creates the controls of a renderer that built probe GI in when `built_in`, whose probes trace `ray_budget` rays a
	/// frame and whose cascade-zero probes stand `spacing` meters apart.
	///
	/// Take all three from [`super::super::VisibilityPipelineSettings`], which validated them at startup.
	pub(crate) fn new(built_in: bool, ray_budget: u32, spacing: f32) -> Self {
		Self {
			built_in,
			ray_budget,
			spacing,
		}
	}

	/// Applies one runtime parameter, returning the updated settings and the effective value, or leaves them unchanged.
	///
	/// Whether probe GI is built in, the ray budget, and the spacing are decided once, at startup. Their startup values,
	/// which every port receives, are accepted as they are; any other value is not set.
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
					"Probe GI enabled was not changed from `{}`. The most likely cause is that it decides once, at startup, whether probe GI is built into the shaders. Select a technique at runtime with `render.gi.technique` instead, or set `render.probe-gi.enabled` in `config.json` and restart the application.",
					self.built_in
				))
			}
			"ray-budget" => {
				if value.to_u32() == Some(self.ray_budget) {
					return Ok((self, ConfigurationValue::Integer(i64::from(self.ray_budget))));
				}
				Err(format!(
					"Probe ray budget was not changed from {}. The most likely cause is that it sizes the probe ray buffers once, when the renderer starts. Set `render.probe-gi.ray-budget` in `config.json` and restart the application.",
					self.ray_budget
				))
			}
			"spacing" => {
				if value.to_f64().map(|spacing| spacing as f32) == Some(self.spacing) {
					return Ok((self, ConfigurationValue::Float(f64::from(self.spacing))));
				}
				Err(format!(
					"Probe spacing was not changed from {} meters. The most likely cause is that it places the probe cascades once, when the renderer starts. Set `render.probe-gi.spacing` in `config.json` and restart the application.",
					self.spacing
				))
			}
			_ => Err(
				"Probe GI parameter was not set. The most likely cause is that the parameter name is unsupported. Use `enabled`, `ray-budget`, or `spacing`."
					.to_string(),
			),
		}
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
			stride(&set_layouts::PROBE_VOLUME, "probe_volume_parameters"),
			size_of::<ProbeVolumeParameters>()
		);
		assert_eq!(stride(&set_layouts::PROBE_VOLUME, "probe_records"), size_of::<ProbeRecord>());
		assert_eq!(
			stride(&set_layouts::PROBE_GI, "probe_gi_parameters"),
			size_of::<ProbeGiParameters>()
		);
		assert_eq!(stride(&set_layouts::PROBE_GI, "probe_schedule"), size_of::<ProbeSchedule>());
		assert_eq!(
			stride(&set_layouts::PROBE_GI, "probe_ray_results"),
			size_of::<ProbeRayResult>()
		);
		assert_eq!(
			stride(&set_layouts::PROBE_GI, "probe_scene_lights"),
			size_of::<LightingData>()
		);
		assert_eq!(
			stride(&set_layouts::PROBE_RESOLVE, "probe_resolve_parameters"),
			size_of::<ProbeResolveParameters>()
		);
	}

	#[test]
	fn startup_values_keep_their_settings() {
		let settings = ProbeGiSettings::new(true, DEFAULT_PROBE_RAY_BUDGET, 0.5);

		let (kept, effective) = settings
			.with_parameter("ray-budget", &ConfigurationValue::Text(DEFAULT_PROBE_RAY_BUDGET.to_string()))
			.unwrap();
		assert_eq!(kept, settings);
		assert_eq!(effective, ConfigurationValue::Integer(i64::from(DEFAULT_PROBE_RAY_BUDGET)));
		assert!(
			settings
				.with_parameter("spacing", &ConfigurationValue::Text("0.5".to_string()))
				.is_ok()
		);
		assert!(settings.with_parameter("enabled", &ConfigurationValue::Bool(true)).is_ok());
		assert!(settings.with_parameter("enabled", &ConfigurationValue::Bool(false)).is_err());

		let reason = settings
			.with_parameter("spacing", &ConfigurationValue::Float(1.0))
			.unwrap_err();
		assert!(reason.contains("`render.probe-gi.spacing` in `config.json`"), "{reason}");
		assert!(settings.with_parameter("cascades", &ConfigurationValue::Integer(2)).is_err());
	}
}
