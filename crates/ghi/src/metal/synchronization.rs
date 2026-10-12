use objc2::runtime::ImplementedBy;
use objc2_metal::MTL4CommandEncoder as _;

use super::*;

/// The `MetalResourceKey` enum identifies one native allocation across command recordings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum MetalResourceKey {
	Buffer(BufferHandle),
	Image(ImageHandle),
	SwapchainDrawable(usize),
	/// One acceleration structure, identified by its index in the context's acceleration-structure storage.
	AccelerationStructure(usize),
	/// One image-group heap, identified by [`GroupSlot::heap_serial`]. Members are regions of it.
	GroupHeap(u64),
}

impl MetalResourceKey {
	fn drawable(texture: &ProtocolObject<dyn mtl::MTLTexture>) -> Self {
		Self::SwapchainDrawable(std::ptr::from_ref(texture).cast::<()>() as usize)
	}
}

/// The `MetalResourceRegion` enum limits hazard tracking to an accessed buffer range or texture subresource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MetalResourceRegion {
	Buffer {
		start: usize,
		end: usize,
	},
	Texture {
		mip_level: Option<u32>,
		layer: Option<u32>,
	},
	/// The whole resource, for allocations Metal does not let a command access piecewise.
	Whole,
}

impl MetalResourceRegion {
	fn overlaps(self, other: Self) -> bool {
		match (self, other) {
			(
				Self::Buffer {
					start: left_start,
					end: left_end,
				},
				Self::Buffer {
					start: right_start,
					end: right_end,
				},
			) => left_start < right_end && right_start < left_end,
			(
				Self::Texture {
					mip_level: left_mip,
					layer: left_layer,
				},
				Self::Texture {
					mip_level: right_mip,
					layer: right_layer,
				},
			) => {
				left_mip.zip(right_mip).is_none_or(|(left, right)| left == right)
					&& left_layer.zip(right_layer).is_none_or(|(left, right)| left == right)
			}
			_ => true,
		}
	}

	fn union(self, other: Self) -> Self {
		match (self, other) {
			(
				Self::Buffer {
					start: left_start,
					end: left_end,
				},
				Self::Buffer {
					start: right_start,
					end: right_end,
				},
			) => Self::Buffer {
				start: left_start.min(right_start),
				end: left_end.max(right_end),
			},
			(
				Self::Texture {
					mip_level: left_mip,
					layer: left_layer,
				},
				Self::Texture {
					mip_level: right_mip,
					layer: right_layer,
				},
			) => Self::Texture {
				mip_level: if left_mip == right_mip { left_mip } else { None },
				layer: if left_layer == right_layer { left_layer } else { None },
			},
			_ => self,
		}
	}

	fn covers(self, other: Self) -> bool {
		match (self, other) {
			(
				Self::Buffer {
					start: left_start,
					end: left_end,
				},
				Self::Buffer {
					start: right_start,
					end: right_end,
				},
			) => left_start <= right_start && left_end >= right_end,
			(
				Self::Texture {
					mip_level: left_mip,
					layer: left_layer,
				},
				Self::Texture {
					mip_level: right_mip,
					layer: right_layer,
				},
			) => (left_mip.is_none() || left_mip == right_mip) && (left_layer.is_none() || left_layer == right_layer),
			(Self::Whole, Self::Whole) => true,
			_ => false,
		}
	}
}

/// The `MetalResourceUse` struct describes one resource access by one Metal command.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct MetalResourceUse {
	pub(crate) key: MetalResourceKey,
	pub(crate) region: MetalResourceRegion,
	pub(crate) stages: mtl::MTLStages,
	pub(crate) access: crate::AccessPolicies,
	/// The image-group member this use reaches through its heap, set by [`Self::in_group_memory`].
	pub(crate) member: Option<ImageHandle>,
}

impl MetalResourceUse {
	pub(crate) fn buffer(
		handle: BufferHandle,
		offset: usize,
		size: usize,
		stages: mtl::MTLStages,
		access: crate::AccessPolicies,
	) -> Self {
		Self::new(
			MetalResourceKey::Buffer(handle),
			MetalResourceRegion::Buffer {
				start: offset,
				end: offset.saturating_add(size),
			},
			stages,
			access,
		)
	}

	pub(crate) fn image(
		handle: ImageHandle,
		mip_level: Option<u32>,
		layer: Option<u32>,
		stages: mtl::MTLStages,
		access: crate::AccessPolicies,
	) -> Self {
		Self::new(
			MetalResourceKey::Image(handle),
			MetalResourceRegion::Texture { mip_level, layer },
			stages,
			access,
		)
	}

	/// Records one access to a whole acceleration structure.
	///
	/// Metal builds and reads an acceleration structure as one opaque allocation, so its hazards are tracked
	/// without a region the way buffer ranges and texture subresources are.
	pub(crate) fn acceleration_structure(index: usize, stages: mtl::MTLStages, access: crate::AccessPolicies) -> Self {
		Self::new(
			MetalResourceKey::AccelerationStructure(index),
			MetalResourceRegion::Whole,
			stages,
			access,
		)
	}

	pub(crate) fn drawable(
		texture: &ProtocolObject<dyn mtl::MTLTexture>,
		stages: mtl::MTLStages,
		access: crate::AccessPolicies,
	) -> Self {
		Self::new(
			MetalResourceKey::drawable(texture),
			MetalResourceRegion::Texture {
				mip_level: None,
				layer: None,
			},
			stages,
			access,
		)
	}

	fn new(key: MetalResourceKey, region: MetalResourceRegion, stages: mtl::MTLStages, access: crate::AccessPolicies) -> Self {
		Self {
			key,
			region,
			stages,
			access,
			member: None,
		}
	}

	/// Tracks an image-group member by the heap bytes it occupies instead of by its image.
	///
	/// Members that share memory then conflict like overlapping ranges of one buffer, so a member that reuses another
	/// member's memory waits for that member's earlier accesses. Other uses are returned unchanged.
	pub(crate) fn in_group_memory(
		self,
		images: &crate::ResourceCollection<Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	) -> Self {
		let MetalResourceKey::Image(handle) = self.key else {
			return self;
		};
		let Some(slot) = &images.resource(handle).slot else {
			return self;
		};
		Self {
			key: MetalResourceKey::GroupHeap(slot.heap_serial),
			region: MetalResourceRegion::Buffer {
				start: slot.offset,
				end: slot.offset + slot.size,
			},
			member: Some(handle),
			..self
		}
	}

	fn merge(&mut self, other: Self) {
		self.region = self.region.union(other.region);
		self.stages |= other.stages;
		self.access |= other.access;
	}
}

/// The shader stages Metal runs on its compute timeline.
///
/// Metal has no ray-tracing pipeline: a ray-generation function is a compute function that resolves hits through
/// the acceleration structure it is given, so every ray-tracing stage dispatches as compute work here.
pub(crate) const DISPATCH_STAGES: crate::Stages = crate::Stages::COMPUTE
	.union(crate::Stages::RAYGEN)
	.union(crate::Stages::CLOSEST_HIT)
	.union(crate::Stages::ANY_HIT)
	.union(crate::Stages::INTERSECTION)
	.union(crate::Stages::MISS)
	.union(crate::Stages::CALLABLE);

/// Converts GHI shader-stage visibility to the stages Metal 4 accepts in barrier commands.
pub(crate) fn to_metal_stages(stages: crate::Stages) -> mtl::MTLStages {
	[
		(crate::Stages::VERTEX | crate::Stages::INDEX, mtl::MTLStages::Vertex),
		(crate::Stages::TASK, mtl::MTLStages::Object),
		(crate::Stages::MESH, mtl::MTLStages::Mesh),
		(crate::Stages::FRAGMENT, mtl::MTLStages::Fragment),
		(DISPATCH_STAGES, mtl::MTLStages::Dispatch),
		(crate::Stages::TRANSFER, mtl::MTLStages::Blit),
		(
			crate::Stages::ACCELERATION_STRUCTURE_BUILD,
			mtl::MTLStages::AccelerationStructure,
		),
	]
	.into_iter()
	.fold(mtl::MTLStages::empty(), |metal, (source, target)| {
		if stages.intersects(source) { metal | target } else { metal }
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetalEncoderScope {
	Queue,
	Encoder(u32),
}

#[derive(Clone, Copy)]
struct MetalResourceState {
	region: MetalResourceRegion,
	stages: mtl::MTLStages,
	access: crate::AccessPolicies,
	scope: MetalEncoderScope,
	/// The newest recording that made this access, so the state can be dropped once that recording has completed.
	recording: u64,
	/// The unordered group the access was made in, or `0` outside a group. Later uses in the same group plan no
	/// hazard against it.
	group: u64,
	/// How many encoder barriers had been planned in the state's encoder when it was recorded, so a later hazard
	/// against it is already ordered by any barrier logged at this index or after.
	barriers: u32,
}

impl MetalResourceState {
	/// Returns the scope the state stands at while `recording` is the recording in progress.
	///
	/// A state keeps its encoder only within the recording that made it: encoder ids restart with every recording,
	/// and a finished recording's accesses can only be ordered by queue barriers. Reading the scope this way spares
	/// [`MetalResourceTracker::finish_recording`] a pass over every state the recording touched.
	fn scope_in(&self, recording: u64) -> MetalEncoderScope {
		if self.recording == recording {
			self.scope
		} else {
			MetalEncoderScope::Queue
		}
	}
}

/// The `ResourceHistory` struct holds the accesses the queue still has to order later commands against, for one
/// resource.
#[derive(Clone)]
struct ResourceHistory {
	states: SmallVec<[MetalResourceState; 2]>,
	/// The number of the recording [`Self::settle`] last ran for, which merged what finished recordings left.
	settled: u64,
	/// The [`MetalResourceTracker::transaction`] that last saved the states for a rollback, or `0` when none did.
	saved: u64,
}

impl ResourceHistory {
	/// Merges the states finished recordings left that are the same access at queue scope, so a resource read every
	/// frame keeps one state for all of them. Call it before the recording numbered `recording` first changes the
	/// states.
	fn settle(&mut self, recording: u64) {
		for state in &mut self.states {
			state.scope = MetalEncoderScope::Queue;
		}
		let states = &mut self.states;
		while let Some((left, right)) = (0..states.len()).find_map(|left| {
			((left + 1)..states.len())
				.find(|&right| states[left].region == states[right].region && states[left].access == states[right].access)
				.map(|right| (left, right))
		}) {
			let right = states.swap_remove(right);
			states[left].stages |= right.stages;
			states[left].recording = states[left].recording.max(right.recording);
		}
		self.settled = recording;
	}
}

/// The `PlannedEncoderBarrier` struct records one encoder barrier a command encoded, so later hazards in the same
/// encoder that it already orders plan no barrier of their own.
#[derive(Clone, Copy)]
struct PlannedEncoderBarrier {
	after: mtl::MTLStages,
	before: mtl::MTLStages,
	/// Whether the barrier made prior writes visible to later reads, which a write-after-read barrier does not.
	device_visibility: bool,
}

impl PlannedEncoderBarrier {
	/// Returns whether this barrier, encoded after `state` was recorded, orders a use of `stages` against it.
	fn covers(self, state: MetalResourceState, stages: mtl::MTLStages, needs_visibility: bool) -> bool {
		self.after.contains(state.stages) && self.before.contains(stages) && (self.device_visibility || !needs_visibility)
	}
}

/// The `MetalBarrier` struct contains the precise inter-encoder and intra-encoder dependencies for one command.
#[derive(Clone, Copy)]
pub(crate) struct MetalBarrier {
	pub(crate) queue_after: mtl::MTLStages,
	pub(crate) queue_before: mtl::MTLStages,
	pub(crate) encoder_after: mtl::MTLStages,
	pub(crate) encoder_before: mtl::MTLStages,
	queue_visibility: mtl::MTL4VisibilityOptions,
	encoder_visibility: mtl::MTL4VisibilityOptions,
}

impl Default for MetalBarrier {
	fn default() -> Self {
		Self {
			queue_after: mtl::MTLStages::empty(),
			queue_before: mtl::MTLStages::empty(),
			encoder_after: mtl::MTLStages::empty(),
			encoder_before: mtl::MTLStages::empty(),
			queue_visibility: mtl::MTL4VisibilityOptions::None,
			encoder_visibility: mtl::MTL4VisibilityOptions::None,
		}
	}
}

impl MetalBarrier {
	pub(crate) fn has_queue_dependency(self) -> bool {
		!self.queue_after.is_empty()
	}

	pub(crate) fn has_encoder_dependency(self) -> bool {
		!self.encoder_after.is_empty()
	}

	/// Encodes this dependency on a compute or render encoder.
	pub(crate) fn encode<E: objc2::Message + ?Sized>(self, encoder: &E)
	where
		dyn mtl::MTL4CommandEncoder: ImplementedBy<E>,
	{
		let encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder> = ProtocolObject::from_ref(encoder);
		if self.has_queue_dependency() {
			encoder.barrierAfterQueueStages_beforeStages_visibilityOptions(
				self.queue_after,
				self.queue_before,
				self.queue_visibility,
			);
		}
		if self.has_encoder_dependency() {
			encoder.barrierAfterEncoderStages_beforeEncoderStages_visibilityOptions(
				self.encoder_after,
				self.encoder_before,
				self.encoder_visibility,
			);
		}
	}
}

/// The `DescriptorUses` struct holds the resource uses of one bound descriptor table, so repeated draws and dispatches
/// in one encoder only replan the uses that can still need a barrier.
///
/// Build it once per descriptor materialization with [`Self::new`], then track it in each encoder that binds it with a
/// [`TrackedTable`].
#[derive(Clone)]
pub(crate) struct DescriptorUses {
	/// Consolidated uses, writable ones first and ordered by key inside each part. Every command replans the writable
	/// ones, because each command's writes conflict with the next command's accesses.
	uses: SmallVec<[MetalResourceUse; 16]>,
	/// How many of `uses` write.
	writable: usize,
}

/// The encoder scope, the tracker generation, and the number of barriers encoded in that encoder, right after a
/// descriptor table's uses were last applied.
///
/// While all three still match, the table's read-only uses already have every barrier they need in that encoder. A
/// barrier encoded since then orders the reads recorded before it, not the ones of a later command, so the table is
/// applied again after one.
pub(crate) type SettledDescriptors = Option<(MetalEncoderScope, u64, usize)>;

/// The `TrackedTable` struct is one descriptor table as an encoder tracks it: the table's uses and whether the
/// encoder still has to plan its reads.
///
/// The encoder that applies the table owns this state, so a table shared by several recordings stays immutable. Pass
/// it to [`MetalResourceTracker::consume_descriptor_tables`] for every command that binds the table.
pub(crate) struct TrackedTable {
	pub(crate) uses: std::sync::Arc<DescriptorUses>,
	pub(crate) settled: SettledDescriptors,
}

impl TrackedTable {
	/// Tracks `uses` in an encoder that has not planned them yet.
	pub(crate) fn new(uses: std::sync::Arc<DescriptorUses>) -> Self {
		Self { uses, settled: None }
	}
}

impl DescriptorUses {
	/// Returns the image-group members the table reaches.
	pub(crate) fn members(&self) -> impl Iterator<Item = ImageHandle> + '_ {
		self.uses.iter().filter_map(|resource_use| resource_use.member)
	}

	/// Consolidates `uses` and orders the writable ones first.
	pub(crate) fn new(mut uses: SmallVec<[MetalResourceUse; 16]>) -> Self {
		MetalResourceTracker::consolidate_in_place(&mut uses);
		// Consolidation merged overlapping uses of a resource, so a read-only use never overlaps a writable one. The
		// sort is stable, so both parts keep the key order consolidation left. A table that only reads, as a texture
		// table does, is already in order.
		let writes = |resource_use: &MetalResourceUse| resource_use.access.intersects(crate::AccessPolicies::WRITE);
		if !uses.is_sorted_by_key(|resource_use| !writes(resource_use)) {
			uses.sort_by_key(|resource_use| !writes(resource_use));
		}
		let writable = uses.partition_point(writes);
		debug_assert!(
			uses[..writable].is_sorted_by_key(|resource_use| resource_use.key)
				&& uses[writable..].is_sorted_by_key(|resource_use| resource_use.key),
			"Metal descriptor uses lost their key order. The most likely cause is that consolidation stopped sorting uses by key.",
		);
		Self { uses, writable }
	}

	/// Empties the table and returns the storage of its uses, for [`Self::new`] to build the table that replaces it
	/// without allocating.
	pub(crate) fn take_storage(&mut self) -> SmallVec<[MetalResourceUse; 16]> {
		self.writable = 0;
		std::mem::take(&mut self.uses)
	}

	/// Returns whether `resource_use` overlaps one of the table's uses.
	///
	/// Each part is ordered by key, so only the uses of the same resource are compared.
	fn overlaps(&self, resource_use: &MetalResourceUse) -> bool {
		let (writable, read_only) = self.uses.split_at(self.writable);
		Self::part_overlaps(writable, resource_use) || Self::part_overlaps(read_only, resource_use)
	}

	/// Returns whether `resource_use` overlaps one of `part`, which is ordered by key.
	fn part_overlaps(part: &[MetalResourceUse], resource_use: &MetalResourceUse) -> bool {
		let start = part.partition_point(|primary_use| primary_use.key < resource_use.key);
		part[start..]
			.iter()
			.take_while(|primary_use| primary_use.key == resource_use.key)
			.any(|primary_use| primary_use.region.overlaps(resource_use.region))
	}

	/// Returns whether one of this table's uses and one of `other`'s reach the same memory and at least one of them
	/// writes it.
	///
	/// Tables that conflict cannot be planned as separate tables, because one's reads would no longer be free of the
	/// command's own writes; plan their [`Self::merged`] table instead. Tables that only read the same memory do not
	/// conflict.
	pub(crate) fn conflicts_with(&self, other: &Self) -> bool {
		// The smaller table is walked and the larger one searched.
		let (walked, searched) = if self.uses.len() <= other.uses.len() {
			(self, other)
		} else {
			(other, self)
		};
		let (writable, read_only) = walked.uses.split_at(walked.writable);
		writable.iter().any(|resource_use| searched.overlaps(resource_use))
			|| read_only
				.iter()
				.any(|resource_use| Self::part_overlaps(&searched.uses[..searched.writable], resource_use))
	}

	/// Returns one table that holds the uses of every table of `tables`, consolidated.
	pub(crate) fn merged<'a>(tables: impl IntoIterator<Item = &'a Self>) -> Self {
		Self::new(tables.into_iter().flat_map(|table| table.uses.iter().copied()).collect())
	}
}

/// The `MetalHazard` struct records one earlier access that forced a barrier, so capture tools can show why it exists.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct MetalHazard {
	pub(crate) key: MetalResourceKey,
	pub(crate) region: MetalResourceRegion,
	pub(crate) previous: crate::AccessPolicies,
	pub(crate) next: crate::AccessPolicies,
}

/// The `MetalResourceTracker` struct retains region-aware access history for one Metal command queue.
#[derive(Default)]
pub(crate) struct MetalResourceTracker {
	states: HashMap<MetalResourceKey, ResourceHistory>,
	/// What each resource the recording in progress changed held before, or `None` when it held nothing, in the order
	/// the recording first changed them. [`Self::rollback_recording`] restores them newest first.
	undo_states: Vec<(MetalResourceKey, Option<ResourceHistory>)>,
	recording: bool,
	/// Counts the recordings begun, finished or not, so each one saves a resource for its rollback once. A recording
	/// that is rolled back keeps its number, see `next_recording`, but not its transaction.
	transaction: u64,
	/// Advances whenever the history changes in a way that could give an earlier read a new hazard: a new or changed
	/// write, removed states, or a finished or abandoned recording. [`DescriptorUses`] compares it to skip reads.
	generation: u64,
	/// The number the recording in progress, or the next one, publishes its accesses under.
	next_recording: u64,
	/// Every recording numbered below this has completed on the GPU.
	completed_below: u64,
	/// Completed recordings numbered above `completed_below`. Batches complete out of order, so these wait for
	/// every recording before them to complete too.
	completed_ahead: SmallVec<[u64; 8]>,
	/// The unordered group being planned, or `0` outside a group. See [`Self::begin_unordered`].
	group: u64,
	/// The group number given out last. Numbers are never reused, so a state only ever matches its own group.
	last_group: u64,
	/// The encoder barriers planned so far in the encoder `barrier_encoder`, the one the last command recorded in, in
	/// encoding order. A Metal encoder barrier orders every earlier command of its encoder before every later one,
	/// so a hazard between a state and a use that a logged barrier already orders plans nothing new. Encoders are
	/// sequential and never resumed, so only the current one's barriers matter.
	encoder_barriers: SmallVec<[PlannedEncoderBarrier; 8]>,
	barrier_encoder: u32,
	/// What [`Self::record_final`] last recorded and the history it left, see [`FinalUses`].
	last_final: Option<FinalUses>,
	/// The hazards behind the barrier of the most recently planned command.
	#[cfg(debug_assertions)]
	hazards: SmallVec<[MetalHazard; 4]>,
}

/// The `FinalUses` struct remembers the uses [`MetalResourceTracker::record_final`] last recorded, with everything
/// their states were derived from. While none of it changed, recording the same uses again would write the same
/// states, so a render pass that records its attachment writes after every draw pays for them once.
struct FinalUses {
	scope: MetalEncoderScope,
	generation: u64,
	group: u64,
	barrier_encoder: u32,
	barriers: usize,
	uses: SmallVec<[MetalResourceUse; 8]>,
}

impl MetalResourceTracker {
	/// Returns the hazards behind the barrier of the most recently planned command.
	#[cfg(debug_assertions)]
	pub(crate) fn hazards(&self) -> &[MetalHazard] {
		&self.hazards
	}

	/// Starts a sparse transaction so abandoning a command recording can restore queue history.
	pub(crate) fn begin_recording(&mut self) {
		assert!(
			!self.recording,
			"Metal resource tracker transaction failed. The most likely cause is that queue history was reused before its previous recording finished.",
		);
		self.undo_states.clear();
		// Encoder ids restart with each recording, so the first encoder must not inherit the last one's barriers.
		self.encoder_barriers.clear();
		self.barrier_encoder = u32::MAX;
		// Encoder ids restart too, so the last recording's final uses say nothing about this one's.
		self.last_final = None;
		self.transaction += 1;
		self.recording = true;
	}

	/// Restores queue history after a command recording is abandoned before becoming executable.
	pub(crate) fn rollback_recording(&mut self) -> bool {
		if !std::mem::take(&mut self.recording) {
			return false;
		}
		// A resource removed and changed again was saved twice, so the oldest save is restored last.
		for (key, history) in self.undo_states.drain(..).rev() {
			if let Some(history) = history {
				self.states.insert(key, history);
			} else {
				self.states.remove(&key);
			}
		}
		self.generation += 1;
		true
	}

	/// Starts the unordered group of [`crate::command_buffer::CommandBufferRecording::unordered`]: until
	/// [`Self::end_unordered`], a use plans no hazard against a state another use in the group recorded.
	pub(crate) fn begin_unordered(&mut self) {
		self.last_group += 1;
		self.group = self.last_group;
	}

	/// Ends the unordered group [`Self::begin_unordered`] started.
	pub(crate) fn end_unordered(&mut self) {
		self.group = 0;
	}

	/// Plans one draw or dispatch that binds several descriptor tables, one per descriptor set, plus its
	/// command-specific uses.
	///
	/// `bound` indexes the tables of `tables` the command binds; `tables` holds every table the encoder tracks.
	///
	/// When a table's uses were last applied in this same encoder and the history has not changed since, its
	/// read-only uses are skipped: the barriers encoded for that earlier command still order every later command in
	/// the encoder, and reapplying the same reads would leave the history as it is. Each table settles on its own, so
	/// a table that several pipelines bind in one encoder has its read-only uses planned once, however many times the
	/// pipeline changes around it.
	///
	/// No two bound tables may conflict, see [`DescriptorUses::conflicts_with`]: a table's reads are skipped on the
	/// grounds that nothing the command writes reaches them. `additional_uses` must be consolidated as
	/// [`Self::consolidate_in_place`] leaves them.
	pub(crate) fn consume_descriptor_tables(
		&mut self,
		scope: MetalEncoderScope,
		tables: &mut [TrackedTable],
		bound: &[u32],
		additional_uses: &[MetalResourceUse],
	) -> MetalBarrier {
		debug_assert!(
			additional_uses.is_sorted_by_key(|resource_use| (resource_use.key, resource_use.region)),
			"Unconsolidated Metal command uses. The most likely cause is that a caller skipped consolidate_in_place.",
		);
		let aliases_primary = additional_uses.iter().any(|additional_use| {
			bound
				.iter()
				.any(|&table| tables[table as usize].uses.overlaps(additional_use))
		});
		self.begin_command_barriers(scope);
		// Every table is judged against the history as the command found it.
		let current = (!aliases_primary).then_some((scope, self.generation, self.encoder_barriers.len()));
		/// Returns the uses of `table` the command still has to plan: all of them, or only the writable ones when the
		/// table settled against the history `current` names.
		fn unsettled_uses(table: &TrackedTable, current: SettledDescriptors) -> &[MetalResourceUse] {
			if current.is_some() && table.settled == current {
				&table.uses.uses[..table.uses.writable]
			} else {
				&table.uses.uses[..]
			}
		}

		let mut barrier = MetalBarrier::default();
		#[cfg(debug_assertions)]
		self.hazards.clear();
		for &table in bound {
			self.plan(scope, unsettled_uses(&tables[table as usize], current), &mut barrier);
		}
		self.plan(scope, additional_uses, &mut barrier);
		self.log_encoder_barrier(&barrier);

		if aliases_primary {
			// The uncommon alias path may copy descriptors so overlapping uses become one atomic command state.
			let mut uses = bound
				.iter()
				.flat_map(|&table| tables[table as usize].uses.uses.iter().copied())
				.collect::<SmallVec<[_; 16]>>();
			uses.extend_from_slice(additional_uses);
			Self::consolidate_in_place(&mut uses);
			for resource_use in uses {
				self.apply_use(scope, resource_use);
			}
			for &table in bound {
				tables[table as usize].settled = None;
			}
			return barrier;
		}
		for &table in bound {
			for &resource_use in unsettled_uses(&tables[table as usize], current) {
				self.apply_use(scope, resource_use);
			}
		}
		// The tables' own writes do not stop their reads from being skipped by the next command.
		let settled = Some((scope, self.generation, self.encoder_barriers.len()));
		for &table in bound {
			tables[table as usize].settled = settled;
		}
		for &resource_use in additional_uses {
			self.apply_use(scope, resource_use);
		}
		barrier
	}

	/// Forgets the barriers of the previous encoder when a command is the first one planned in `scope`.
	fn begin_command_barriers(&mut self, scope: MetalEncoderScope) {
		// A new encoder starts with no barriers; encoder ids also restart with each recording.
		if let MetalEncoderScope::Encoder(encoder) = scope
			&& encoder != self.barrier_encoder
		{
			self.encoder_barriers.clear();
			self.barrier_encoder = encoder;
		}
	}

	/// Logs the encoder barrier planned for a command, which orders every later command in the encoder against
	/// everything before it.
	fn log_encoder_barrier(&mut self, barrier: &MetalBarrier) {
		// The caller encodes the barrier before the command, so it orders this command's states against nothing, and
		// every later command in the encoder against everything before it.
		if barrier.has_encoder_dependency() {
			self.encoder_barriers.push(PlannedEncoderBarrier {
				after: barrier.encoder_after,
				before: barrier.encoder_before,
				device_visibility: barrier.encoder_visibility == mtl::MTL4VisibilityOptions::Device,
			});
		}
	}

	/// Plans and applies one command's uses, which must be consolidated as [`Self::consolidate_in_place`] leaves them.
	pub(crate) fn consume_consolidated(&mut self, scope: MetalEncoderScope, uses: &[MetalResourceUse]) -> MetalBarrier {
		let mut barrier = MetalBarrier::default();
		#[cfg(debug_assertions)]
		self.hazards.clear();
		self.begin_command_barriers(scope);
		self.plan(scope, uses, &mut barrier);
		self.log_encoder_barrier(&barrier);
		for &resource_use in uses {
			self.apply_use(scope, resource_use);
		}
		barrier
	}

	/// Records accesses that occurred throughout an encoder without adding an artificial trailing command.
	///
	/// `uses` must be consolidated as [`Self::consolidate_in_place`] leaves them, so a render pass consolidates its
	/// attachment writes once and records them after every draw.
	pub(crate) fn record_final(&mut self, scope: MetalEncoderScope, uses: &[MetalResourceUse]) {
		debug_assert!(
			uses.is_sorted_by_key(|resource_use| (resource_use.key, resource_use.region)),
			"Unconsolidated final Metal resource uses. The most likely cause is that a caller skipped consolidate_in_place.",
		);
		if self.last_final.as_ref().is_some_and(|last| {
			last.scope == scope
				&& last.generation == self.generation
				&& last.group == self.group
				&& last.barrier_encoder == self.barrier_encoder
				&& last.barriers == self.encoder_barriers.len()
				&& last.uses[..] == *uses
		}) {
			return;
		}
		for &resource_use in uses {
			self.apply_use(scope, resource_use);
		}
		// Applying a new write advances the generation, so the history is remembered as these uses left it.
		let last = self.last_final.get_or_insert_with(|| FinalUses {
			scope,
			generation: 0,
			group: 0,
			barrier_encoder: 0,
			barriers: 0,
			uses: SmallVec::new(),
		});
		last.scope = scope;
		last.generation = self.generation;
		last.group = self.group;
		last.barrier_encoder = self.barrier_encoder;
		last.barriers = self.encoder_barriers.len();
		last.uses.clear();
		last.uses.extend_from_slice(uses);
	}

	/// Removes a presented drawable because CAMetalLayer will not expose this acquisition to later commands.
	pub(crate) fn forget_drawable(&mut self, texture: &ProtocolObject<dyn mtl::MTLTexture>) {
		let key = MetalResourceKey::drawable(texture);
		if let Some(history) = self.states.remove(&key) {
			// A recording that already changed the states saved them then.
			if self.recording && history.saved != self.transaction {
				self.undo_states.push((key, Some(history)));
			}
			self.generation += 1;
		}
	}

	/// Records that the GPU completed the recording that [`Self::finish_recording`] numbered `recording`, and drops
	/// the accesses no later command can race.
	///
	/// Call it when the batch that submitted the recording completes. A state is dropped only once every recording up
	/// to its newest one has completed, since a merged read can also stand for an earlier recording that is still
	/// running. This keeps the history bounded while frames overlap, when the queue is never idle.
	pub(crate) fn complete_recording(&mut self, recording: u64) {
		if recording < self.completed_below {
			return;
		}
		self.completed_ahead.push(recording);
		let before = self.completed_below;
		while let Some(index) = self
			.completed_ahead
			.iter()
			.position(|&completed| completed == self.completed_below)
		{
			self.completed_ahead.swap_remove(index);
			self.completed_below += 1;
		}
		if self.completed_below == before || self.recording {
			return;
		}

		let completed_below = self.completed_below;
		let mut removed = false;
		self.states.retain(|_, history| {
			let count = history.states.len();
			history.states.retain(|state| state.recording >= completed_below);
			removed |= history.states.len() != count;
			!history.states.is_empty()
		});
		if removed {
			// Earlier reads may have been skipped against the removed states, so they must be planned again.
			self.generation += 1;
		}
	}

	/// Publishes the recording's accesses as queue history and commits the recording transaction.
	///
	/// Returns the number the recording's accesses were published under. Pass it to [`Self::complete_recording`]
	/// once the batch that submits the recording completes. A recording that is never submitted keeps its accesses,
	/// and every later one's, in the history.
	pub(crate) fn finish_recording(&mut self) -> u64 {
		assert!(
			std::mem::take(&mut self.recording),
			"Metal resource tracker finalization failed. The most likely cause is that resource recording was not started.",
		);
		self.undo_states.clear();
		// Every state of the recording now stands at queue scope, see `MetalResourceState::scope_in`, so hazards need
		// queue barriers rather than encoder barriers.
		self.generation += 1;
		let recording = self.next_recording;
		self.next_recording += 1;
		recording
	}

	/// Merges the uses of one command, or of one descriptor table, that reach the same memory, so each is compared
	/// only with state from earlier commands. The tracker reads consolidated uses by reference.
	pub(crate) fn consolidate_in_place<A: smallvec::Array<Item = MetalResourceUse>>(uses: &mut SmallVec<A>) {
		uses.retain(|resource_use| !resource_use.stages.is_empty() && !resource_use.access.is_empty());
		// Most commands name one resource or none besides their descriptor tables.
		if uses.len() < 2 {
			return;
		}
		uses.sort_unstable_by_key(|resource_use| (resource_use.key, resource_use.region));

		// The uses of one resource are adjacent, so a use is only compared with the uses already kept for its
		// resource. A table that names one resource many times, as a texture array filled with a placeholder does, is
		// consolidated in one pass.
		let mut kept = 0;
		let mut resource_start = 0;
		let mut ordered = true;
		for index in 0..uses.len() {
			let mut resource_use = uses[index];
			if kept == resource_start || uses[resource_start].key != resource_use.key {
				resource_start = kept;
			}
			// Merging widens the use, which can make it reach kept uses it did not reach before.
			while let Some(overlapped) =
				(resource_start..kept).find(|&position| uses[position].region.overlaps(resource_use.region))
			{
				let mut merged = uses[overlapped];
				merged.merge(resource_use);
				resource_use = merged;
				uses.copy_within(overlapped + 1..kept, overlapped);
				kept -= 1;
			}
			ordered &= kept == resource_start || uses[kept - 1].region <= resource_use.region;
			// Until a use merges, every use already is where it stays.
			if kept != index {
				uses[kept] = resource_use;
			}
			kept += 1;
		}
		uses.truncate(kept);
		// A merged use takes the place of the last use it absorbed, which a wider region can put out of order.
		if !ordered {
			uses.sort_unstable_by_key(|resource_use| (resource_use.key, resource_use.region));
		}
	}

	fn plan(&mut self, scope: MetalEncoderScope, uses: &[MetalResourceUse], barrier: &mut MetalBarrier) {
		for resource_use in uses {
			let Some(history) = self.states.get(&resource_use.key) else {
				continue;
			};
			for state in history
				.states
				.iter()
				.filter(|state| state.region.overlaps(resource_use.region))
			{
				let state_scope = state.scope_in(self.next_recording);
				// The unordered group's caller promised its commands touch disjoint memory, so a state the group
				// recorded is no hazard for its later commands.
				let in_group = self.group != 0 && state.group == self.group;
				if in_group || !Self::has_hazard(state.access, resource_use.access) {
					continue;
				}
				// A barrier encoded in this encoder after the state was recorded already orders the use against it,
				// when it spans both stages and, for a read of a write, made the write visible.
				if state_scope == scope && scope != MetalEncoderScope::Queue {
					let needs_visibility = state.access.intersects(crate::AccessPolicies::WRITE)
						&& resource_use.access.intersects(crate::AccessPolicies::READ);
					if self.encoder_barriers[state.barriers as usize..]
						.iter()
						.any(|barrier| barrier.covers(*state, resource_use.stages, needs_visibility))
					{
						continue;
					}
				}
				#[cfg(debug_assertions)]
				{
					let hazard = MetalHazard {
						key: resource_use.key,
						region: resource_use.region,
						previous: state.access,
						next: resource_use.access,
					};
					if !self.hazards.contains(&hazard) {
						self.hazards.push(hazard);
					}
				}
				let (after, before, visibility) = if state_scope == scope {
					(
						&mut barrier.encoder_after,
						&mut barrier.encoder_before,
						&mut barrier.encoder_visibility,
					)
				} else {
					(
						&mut barrier.queue_after,
						&mut barrier.queue_before,
						&mut barrier.queue_visibility,
					)
				};
				*after |= state.stages;
				*before |= resource_use.stages;
				if state.access.intersects(crate::AccessPolicies::WRITE)
					&& resource_use.access.intersects(crate::AccessPolicies::READ)
				{
					*visibility = mtl::MTL4VisibilityOptions::Device;
				}
			}
		}
	}

	fn apply_use(&mut self, scope: MetalEncoderScope, resource_use: MetalResourceUse) {
		// Only the current encoder's barriers are logged; a state in any other scope sees none.
		let barriers = match scope {
			MetalEncoderScope::Encoder(encoder) if encoder == self.barrier_encoder => self.encoder_barriers.len() as u32,
			_ => 0,
		};
		let recording = self.next_recording;
		// The first change a recording makes to a resource saves what it held, for a rollback, and merges what
		// finished recordings left.
		let saved = if self.recording { self.transaction } else { 0 };
		let history = match self.states.entry(resource_use.key) {
			std::collections::hash_map::Entry::Occupied(entry) => {
				let history = entry.into_mut();
				if self.recording && history.saved != saved {
					self.undo_states.push((resource_use.key, Some(history.clone())));
					history.saved = saved;
				}
				if history.settled != recording {
					history.settle(recording);
				}
				history
			}
			std::collections::hash_map::Entry::Vacant(entry) => {
				if self.recording {
					self.undo_states.push((resource_use.key, None));
				}
				entry.insert(ResourceHistory {
					states: SmallVec::new(),
					settled: recording,
					saved,
				})
			}
		};
		let states = &mut history.states;
		let has_hazard = states
			.iter()
			.filter(|state| state.region.overlaps(resource_use.region))
			.any(|state| Self::has_hazard(state.access, resource_use.access));

		if has_hazard {
			// Recording the same access again with nothing in between, such as a render pass's attachment writes after
			// each draw, replaces the only state it would remove with an identical one, so the history stays as it is.
			let mut covered = states.iter_mut().filter(|state| resource_use.region.covers(state.region));
			if let (Some(state), None) = (covered.next(), covered.next())
				&& state.scope_in(recording) == scope
				&& state.region == resource_use.region
				&& state.access == resource_use.access
				&& state.stages == resource_use.stages
			{
				// The state now also stands for this recording, so it stays until this recording completes.
				state.recording = recording;
				// This use was already planned against the state, so the group's later uses may skip it.
				state.group = self.group;
				// It now stands for the newest command too, which only barriers encoded after that one order.
				state.barriers = barriers;
				return;
			}
			states.retain(|state| !resource_use.region.covers(state.region));
			self.generation += 1;
		} else if let Some(state) = states.iter_mut().find(|state| {
			state.scope_in(recording) == scope && state.region == resource_use.region && state.access == resource_use.access
		}) {
			// Without a hazard this use is a read, and widening a read's stages cannot give another read a hazard.
			state.stages |= resource_use.stages;
			state.recording = recording;
			state.barriers = barriers;
			return;
		} else if resource_use.access.intersects(crate::AccessPolicies::WRITE) {
			self.generation += 1;
		}

		states.push(MetalResourceState {
			region: resource_use.region,
			stages: resource_use.stages,
			access: resource_use.access,
			scope,
			recording,
			group: self.group,
			barriers,
		});
	}

	fn has_hazard(previous: crate::AccessPolicies, next: crate::AccessPolicies) -> bool {
		(previous | next).intersects(crate::AccessPolicies::WRITE)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn buffer(access: crate::AccessPolicies, stages: mtl::MTLStages) -> MetalResourceUse {
		MetalResourceUse::buffer(BufferHandle(1), 0, 64, stages, access)
	}

	/// Plans one command that binds no descriptor table, consolidating its uses as a recording does.
	fn consume(
		tracker: &mut MetalResourceTracker,
		scope: MetalEncoderScope,
		uses: impl IntoIterator<Item = MetalResourceUse>,
	) -> MetalBarrier {
		let mut uses = uses.into_iter().collect::<SmallVec<[_; 16]>>();
		MetalResourceTracker::consolidate_in_place(&mut uses);
		tracker.consume_consolidated(scope, &uses)
	}

	/// Plans one command that binds the descriptor table `table`, plus its consolidated command-specific uses.
	fn consume_descriptors(
		tracker: &mut MetalResourceTracker,
		scope: MetalEncoderScope,
		table: &mut TrackedTable,
		additional_uses: &[MetalResourceUse],
	) -> MetalBarrier {
		tracker.consume_descriptor_tables(scope, std::slice::from_mut(table), &[0], additional_uses)
	}

	fn descriptor_table(uses: &[MetalResourceUse]) -> TrackedTable {
		TrackedTable::new(std::sync::Arc::new(DescriptorUses::new(uses.iter().copied().collect())))
	}

	#[test]
	fn unordered_group_orders_only_against_work_outside_it() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let write = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch);
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);

		tracker.begin_unordered();
		let first = consume(&mut tracker, scope, [write]);
		let second = consume(&mut tracker, scope, [write]);
		tracker.end_unordered();
		let read = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Blit)],
		);
		// A later group is a different group, so its first write waits for the read.
		tracker.begin_unordered();
		let later = consume(&mut tracker, scope, [write]);
		tracker.end_unordered();

		assert_eq!(
			(first.encoder_after, first.encoder_before),
			(mtl::MTLStages::Blit, mtl::MTLStages::Dispatch)
		);
		assert!(!second.has_encoder_dependency() && !second.has_queue_dependency());
		assert_eq!(
			(read.encoder_after, read.encoder_before),
			(mtl::MTLStages::Dispatch, mtl::MTLStages::Blit)
		);
		assert_eq!(read.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
		assert_eq!(
			(later.encoder_after, later.encoder_before),
			(mtl::MTLStages::Blit, mtl::MTLStages::Dispatch)
		);
	}

	#[test]
	fn unordered_group_skips_descriptor_table_writes_between_its_dispatches() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[
			buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
			MetalResourceUse::buffer(BufferHandle(2), 0, 64, mtl::MTLStages::Dispatch, crate::AccessPolicies::READ),
		]);
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);

		tracker.begin_unordered();
		let first = consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		let second = consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		tracker.end_unordered();

		assert_eq!(first.encoder_after, mtl::MTLStages::Blit);
		assert!(!second.has_encoder_dependency() && !second.has_queue_dependency());
	}

	#[test]
	fn read_after_read_needs_no_barrier() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Blit)],
		);
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert!(!barrier.has_queue_dependency());
		assert!(!barrier.has_encoder_dependency());
	}

	#[test]
	fn queue_write_to_dispatch_read_uses_precise_stages() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Dispatch);
		assert!(!barrier.has_encoder_dependency());
	}

	#[test]
	fn same_encoder_write_to_read_uses_encoder_barrier() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Encoder(4),
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(4),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Blit)],
		);

		assert!(!barrier.has_queue_dependency());
		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
	}

	fn second_buffer(access: crate::AccessPolicies, stages: mtl::MTLStages) -> MetalResourceUse {
		MetalResourceUse::buffer(BufferHandle(2), 0, 64, stages, access)
	}

	#[test]
	fn encoded_barrier_covers_a_later_read_of_an_earlier_write() {
		//! Tests that a read of one write, ordered by the barrier another read of a write planned after both writes,
		//! plans no barrier of its own, while a write recorded after that barrier still gets one.

		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		consume(
			&mut tracker,
			scope,
			[
				buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
				second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
			],
		);
		let first_read = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		let covered_read = consume(
			&mut tracker,
			scope,
			[second_buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		let later_write = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(first_read.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(first_read.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
		assert!(!covered_read.has_encoder_dependency() && !covered_read.has_queue_dependency());
		// The first read came after the barrier, so the write after it needs a new one.
		assert_eq!(later_write.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(later_write.encoder_before, mtl::MTLStages::Dispatch);
	}

	#[test]
	fn covering_barrier_must_span_the_hazard_stages() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		consume(
			&mut tracker,
			scope,
			[
				buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
				second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit),
			],
		);
		let dispatch_read = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		let blit_read = consume(
			&mut tracker,
			scope,
			[second_buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(dispatch_read.encoder_after, mtl::MTLStages::Dispatch);
		// The planned barrier waits on dispatch work only, so a read of blit work needs its own.
		assert_eq!(blit_read.encoder_after, mtl::MTLStages::Blit);
		assert_eq!(blit_read.encoder_before, mtl::MTLStages::Dispatch);
	}

	#[test]
	fn execution_only_barrier_does_not_cover_a_later_read_after_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		consume(
			&mut tracker,
			scope,
			[
				buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch),
				second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
			],
		);
		let write_after_read = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);
		let read_after_write = consume(
			&mut tracker,
			scope,
			[second_buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(write_after_read.encoder_visibility, mtl::MTL4VisibilityOptions::None);
		// Ordering alone does not make the write visible, so the read plans a barrier that does.
		assert_eq!(read_after_write.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(read_after_write.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn encoded_barriers_do_not_carry_into_the_next_recording() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(0);
		tracker.begin_recording();
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		tracker.finish_recording();

		tracker.begin_recording();
		consume(
			&mut tracker,
			scope,
			[second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);
		let read = consume(
			&mut tracker,
			scope,
			[second_buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		// The earlier recording's encoder 0 is a different encoder from this one's.
		assert_eq!(read.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(read.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn disjoint_buffer_ranges_do_not_conflict() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Queue,
			[MetalResourceUse::buffer(
				BufferHandle(1),
				0,
				64,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			)],
		);
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[MetalResourceUse::buffer(
				BufferHandle(1),
				128,
				64,
				mtl::MTLStages::Dispatch,
				crate::AccessPolicies::READ,
			)],
		);

		assert!(!barrier.has_queue_dependency());
		assert!(!barrier.has_encoder_dependency());
	}

	#[test]
	fn final_render_write_survives_an_aliased_descriptor_read() {
		let mut tracker = MetalResourceTracker::default();
		tracker.begin_recording();
		let scope = MetalEncoderScope::Encoder(2);
		let attachment = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Fragment);
		consume(&mut tracker, scope, [attachment]);
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Fragment)],
		);
		tracker.record_final(scope, &[attachment]);
		tracker.finish_recording();

		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(3),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Fragment);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.queue_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn table_read_after_a_barrier_is_not_ordered_by_it() {
		//! Tests that a descriptor table read again after another hazard's barrier still makes a later write of the
		//! memory it reads wait, because that barrier was encoded before the newer read.

		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)]);
		let other = second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch);
		consume(&mut tracker, scope, [other]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		// Writing the other buffer again needs a barrier, which changes nothing else in the history.
		let other_write = consume(&mut tracker, scope, [other]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		let write = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(
			(other_write.encoder_after, other_write.encoder_before),
			(mtl::MTLStages::Dispatch, mtl::MTLStages::Dispatch)
		);
		assert_eq!(
			(write.encoder_after, write.encoder_before),
			(mtl::MTLStages::Dispatch, mtl::MTLStages::Dispatch)
		);
	}

	#[test]
	fn attachment_write_recorded_after_a_barrier_is_not_ordered_by_it() {
		//! Tests that a render-target write recorded again after another hazard's barrier still makes a later write
		//! of the same memory wait, because that barrier was encoded before the newer write.

		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let attachment = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Fragment);
		let other = second_buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Fragment);
		consume(&mut tracker, scope, [other]);
		tracker.record_final(scope, &[attachment]);
		// Writing the other buffer again needs a barrier, which changes nothing else in the history.
		let other_write = consume(&mut tracker, scope, [other]);
		tracker.record_final(scope, &[attachment]);
		let attachment_write = consume(&mut tracker, scope, [attachment]);

		assert_eq!(
			(other_write.encoder_after, other_write.encoder_before),
			(mtl::MTLStages::Fragment, mtl::MTLStages::Fragment)
		);
		assert_eq!(
			(attachment_write.encoder_after, attachment_write.encoder_before),
			(mtl::MTLStages::Fragment, mtl::MTLStages::Fragment)
		);
	}

	#[test]
	fn repeated_descriptor_reads_see_each_render_attachment_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(2);
		let attachment = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Fragment);
		let descriptor = buffer(crate::AccessPolicies::READ, mtl::MTLStages::Fragment);
		consume(&mut tracker, scope, [attachment]);
		consume(&mut tracker, scope, [descriptor]);
		tracker.record_final(scope, &[attachment]);

		let barrier = consume(&mut tracker, scope, [descriptor]);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn descriptor_read_after_an_intervening_blit_write_is_synchronized() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		// The second command settles the reads, so the third must still notice the write in between.
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);
		let barrier = consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn a_use_that_bridges_two_others_makes_them_one_use() {
		//! Tests that a write overlapping two disjoint reads of one buffer in the same command leaves one write of
		//! the whole range, so a later read of either end waits for every stage of that command.

		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let range = |start: usize, end: usize, stages, access| {
			MetalResourceUse::buffer(BufferHandle(1), start, end - start, stages, access)
		};
		consume(
			&mut tracker,
			scope,
			[
				range(0, 8, mtl::MTLStages::Vertex, crate::AccessPolicies::READ),
				range(16, 24, mtl::MTLStages::Fragment, crate::AccessPolicies::READ),
				range(4, 20, mtl::MTLStages::Dispatch, crate::AccessPolicies::WRITE),
			],
		);
		let barrier = consume(
			&mut tracker,
			scope,
			[range(20, 24, mtl::MTLStages::Blit, crate::AccessPolicies::READ)],
		);

		assert_eq!(
			barrier.encoder_after,
			mtl::MTLStages::Vertex | mtl::MTLStages::Fragment | mtl::MTLStages::Dispatch
		);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn overlapping_uses_in_one_command_preserve_the_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[MetalResourceUse::buffer(
			BufferHandle(1),
			0,
			64,
			mtl::MTLStages::Fragment,
			crate::AccessPolicies::WRITE,
		)]);
		consume_descriptors(
			&mut tracker,
			scope,
			&mut descriptors,
			&[MetalResourceUse::buffer(
				BufferHandle(1),
				0,
				128,
				mtl::MTLStages::Vertex,
				crate::AccessPolicies::READ,
			)],
		);
		let barrier = consume(
			&mut tracker,
			scope,
			[MetalResourceUse::buffer(
				BufferHandle(1),
				0,
				128,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::READ,
			)],
		);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Vertex | mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn repeated_descriptor_reads_still_order_a_later_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Fragment)]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);

		let barrier = consume(
			&mut tracker,
			scope,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
	}

	#[test]
	fn repeated_descriptor_writes_order_each_command() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[
			buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch),
			MetalResourceUse::buffer(BufferHandle(2), 0, 64, mtl::MTLStages::Dispatch, crate::AccessPolicies::READ),
		]);
		consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);

		let barrier = consume_descriptors(&mut tracker, scope, &mut descriptors, &[]);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Dispatch);
	}

	#[test]
	fn descriptor_reads_in_each_encoder_wait_for_an_earlier_write() {
		let mut tracker = MetalResourceTracker::default();
		// Reading one mip level leaves the whole-image write in the history, so every encoder must wait for it.
		let image = |mip_level, stages, access| MetalResourceUse::image(ImageHandle(1), mip_level, None, stages, access);
		let mut descriptors = descriptor_table(&[image(Some(0), mtl::MTLStages::Fragment, crate::AccessPolicies::READ)]);
		consume(
			&mut tracker,
			MetalEncoderScope::Queue,
			[image(None, mtl::MTLStages::Dispatch, crate::AccessPolicies::WRITE)],
		);
		consume_descriptors(&mut tracker, MetalEncoderScope::Encoder(1), &mut descriptors, &[]);

		let barrier = consume_descriptors(&mut tracker, MetalEncoderScope::Encoder(2), &mut descriptors, &[]);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Fragment);
	}

	#[test]
	fn abandoned_recording_restores_queue_history() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);
		tracker.begin_recording();
		consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		tracker.rollback_recording();

		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(2),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Dispatch);
	}

	/// Records one finished recording that makes `resource_use` and returns the number it was published under.
	fn finished_recording(tracker: &mut MetalResourceTracker, resource_use: MetalResourceUse) -> u64 {
		tracker.begin_recording();
		consume(tracker, MetalEncoderScope::Encoder(0), [resource_use]);
		tracker.finish_recording()
	}

	#[test]
	fn completed_recording_write_needs_no_barrier() {
		let mut tracker = MetalResourceTracker::default();
		let write = finished_recording(&mut tracker, buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit));
		tracker.complete_recording(write);

		tracker.begin_recording();
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(0),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert!(!barrier.has_queue_dependency());
		assert!(!barrier.has_encoder_dependency());
	}

	#[test]
	fn reads_stay_ordered_until_every_earlier_recording_completes() {
		//! Tests that a later recording completing first does not drop a read an earlier, still running recording made.

		let mut tracker = MetalResourceTracker::default();
		let read = buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch);
		let earlier = finished_recording(&mut tracker, read);
		let later = finished_recording(&mut tracker, read);
		let write = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit);

		tracker.complete_recording(later);
		tracker.begin_recording();
		let barrier = consume(&mut tracker, MetalEncoderScope::Encoder(0), [write]);
		tracker.rollback_recording();
		assert_eq!(barrier.queue_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Blit);

		tracker.complete_recording(earlier);
		tracker.begin_recording();
		let barrier = consume(&mut tracker, MetalEncoderScope::Encoder(0), [write]);
		assert!(!barrier.has_queue_dependency());
	}

	#[test]
	fn write_after_read_uses_execution_only_visibility() {
		let mut tracker = MetalResourceTracker::default();
		consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		let barrier = consume(
			&mut tracker,
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::None);
	}
}
