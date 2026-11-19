//! The reads that answer screen scene queries: which instance the visibility buffer holds at a queried pixel.
//!
//! The transparent layer clears the visibility buffer before it draws, so one read after the frame would miss the
//! opaque scene wherever no blended surface is. [`ScreenQueryPass`] reads after the opaque layer, then again after the
//! transparent layer, where only a blended surface replaces the opaque instance. The CPU maps the instances it reads
//! back to entities; see the pipeline manager's screen queries.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::{ComputeStage, Pipelines};
use crate::rendering::PipelineManagerClient;
use crate::rendering::pipelines::visibility::set_layouts;

/// The most queries one sink reads in a frame, as `layouts/screen_query.besl` spells `SCREEN_QUERY_CAPACITY`.
pub(crate) const SCREEN_QUERY_CAPACITY: usize = 16;

/// The instance a read finds where no surface was drawn or the point is outside the sink, as
/// `layouts/screen_query.besl` spells `NO_INSTANCE`.
pub(crate) const NO_INSTANCE: u32 = u32::MAX;

/// The `ScreenQueryPass` struct reads one sink's visibility buffer at the pixels the frame's screen scene queries name,
/// so the CPU can answer them once the frame completes.
///
/// Create it with the sink's instance identifier target. Each frame with queries, call [`Self::prepare`] and record
/// the returned stages around the transparent layer; two frames later, read what they found with [`Self::read`].
pub(crate) struct ScreenQueryPass {
	descriptor_sets: [ghi::DescriptorSetHandle; 1],
	/// The opaque read, then the blended read.
	pipelines: Pipelines<2>,
	/// The queried pixels, one copy per frame in flight, written by the CPU when the frame takes them.
	points: ghi::BufferHandle<[[u32; 2]; SCREEN_QUERY_CAPACITY]>,
	/// What the reads found, which the frame copies into its copy of `readback` once both reads ran.
	instances: ghi::BufferHandle<[u32; SCREEN_QUERY_CAPACITY]>,
	readback: ghi::BufferHandle<[u32; SCREEN_QUERY_CAPACITY]>,
}

/// The `ScreenQueryStages` struct is one frame's recording of a sink's screen query reads.
///
/// Record [`Self::opaque`] after the opaque layer, [`Self::blended`] after the transparent layer when the frame draws
/// one, then [`Self::copy`] last.
#[derive(Clone, Copy)]
pub(crate) struct ScreenQueryStages {
	pub(super) opaque: ComputeStage,
	pub(super) blended: ComputeStage,
	pub(super) copy: ghi::BufferCopyDescriptor,
}

impl ScreenQueryPass {
	/// Binds `instance_id`, the sink's visibility buffer of instance identifiers, and requests the read pipelines.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		instance_id: ghi::BaseImageHandle,
	) -> Self {
		let points = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Screen Query Points")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let instances = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferSource).name("Screen Query Instances"),
		);
		// Each frame copies the instances here, so the CPU reads a completed frame's copy while later frames run.
		let readback = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::TransferDestination)
				.name("Screen Query Readback")
				.host_access(ghi::HostAccess::Read)
				.copies(ghi::Copies::PerFrame),
		);
		let descriptor_set = set_layouts::SCREEN_QUERY.create_set(context, "Screen Query Descriptor Set");
		let set = set_layouts::SCREEN_QUERY.binder(descriptor_set);
		context.write(&[
			set.buffer("points", points),
			set.buffer("instances", instances),
			set.image("instance_index_render_target", instance_id),
		]);

		Self {
			descriptor_sets: [descriptor_set],
			pipelines: Pipelines::request(pipeline_manager, ["screen-query", "screen-query-blended"]),
			points,
			instances,
			readback,
		}
	}

	/// Writes `points`, at most [`SCREEN_QUERY_CAPACITY`] of them, into this frame's copy and returns the stages that
	/// read them, or `None` without points or while a pipeline is still compiling.
	pub(crate) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		points: &[[u32; 2]],
	) -> Option<ScreenQueryStages> {
		debug_assert!(
			points.len() <= SCREEN_QUERY_CAPACITY,
			"A sink reads at most {SCREEN_QUERY_CAPACITY} queries a frame."
		);
		if points.is_empty() {
			return None;
		}
		let [opaque, blended] = self.pipelines.resolve(pipeline_manager)?;
		// Unused entries name a point outside every sink, so their reads find no instance.
		let mut written = [[u32::MAX; 2]; SCREEN_QUERY_CAPACITY];
		written[..points.len()].copy_from_slice(points);
		*frame.map(self.points) = written;

		let stage = |label, pipeline| ComputeStage {
			label,
			pipeline,
			descriptor_sets: self.descriptor_sets,
			extent: Extent::new(SCREEN_QUERY_CAPACITY as u32, 1, 1),
			workgroup: Extent::new(SCREEN_QUERY_CAPACITY as u32, 1, 1),
		};
		Some(ScreenQueryStages {
			opaque: stage("Screen Query", opaque),
			blended: stage("Screen Query Blended", blended),
			copy: ghi::BufferCopyDescriptor::new(
				self.instances.into(),
				0,
				self.readback.into(),
				0,
				size_of::<[u32; SCREEN_QUERY_CAPACITY]>(),
			),
		})
	}

	/// Returns the instances the frame that completed when `frame` started read for this sink, in the order its points
	/// were given, or `None` before any frame of this sequence completed.
	///
	/// Only the entries of points that frame was given hold its reads; the caller keeps how many there were.
	pub(crate) fn read<'a>(&self, frame: &'a ghi::implementation::Frame) -> Option<&'a [u32; SCREEN_QUERY_CAPACITY]> {
		frame.read(self.readback)
	}
}
