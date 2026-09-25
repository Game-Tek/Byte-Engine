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
#[derive(Clone, Copy)]
pub(crate) struct MetalResourceUse {
	pub(crate) key: MetalResourceKey,
	pub(crate) region: MetalResourceRegion,
	pub(crate) stages: mtl::MTLStages,
	pub(crate) access: crate::AccessPolicies,
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
/// Build it once per descriptor materialization with [`Self::new`], then pass it to
/// [`MetalResourceTracker::consume_descriptors`] for every command that binds the table.
#[derive(Clone, Default)]
pub(crate) struct DescriptorUses {
	/// Consolidated uses, writable ones first. Every command replans the writable ones, because each command's writes
	/// conflict with the next command's accesses.
	uses: SmallVec<[MetalResourceUse; 16]>,
	/// How many of `uses` write.
	writable: usize,
	/// The encoder scope and tracker generation right after this table's uses were last applied. While both still
	/// match, the read-only uses already have every barrier they need in that encoder.
	settled: Option<(MetalEncoderScope, u64)>,
}

impl DescriptorUses {
	/// Consolidates `uses` and orders the writable ones first.
	pub(crate) fn new(mut uses: SmallVec<[MetalResourceUse; 16]>) -> Self {
		MetalResourceTracker::consolidate_in_place(&mut uses);
		// Consolidation merged overlapping uses of a resource, so a read-only use never overlaps a writable one.
		uses.sort_by_key(|resource_use| !resource_use.access.intersects(crate::AccessPolicies::WRITE));
		let writable = uses.partition_point(|resource_use| resource_use.access.intersects(crate::AccessPolicies::WRITE));
		Self {
			uses,
			writable,
			settled: None,
		}
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
	states: HashMap<MetalResourceKey, SmallVec<[MetalResourceState; 2]>>,
	undo_states: HashMap<MetalResourceKey, Option<SmallVec<[MetalResourceState; 2]>>>,
	recording: bool,
	/// Advances whenever the history changes in a way that could give an earlier read a new hazard: a new or changed
	/// write, removed states, or a finished or abandoned recording. [`DescriptorUses`] compares it to skip reads.
	generation: u64,
	/// The hazards behind the barrier of the most recently planned command.
	#[cfg(debug_assertions)]
	hazards: SmallVec<[MetalHazard; 4]>,
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
		self.recording = true;
	}

	/// Restores queue history after a command recording is abandoned before becoming executable.
	pub(crate) fn rollback_recording(&mut self) -> bool {
		if !std::mem::take(&mut self.recording) {
			return false;
		}
		for (key, states) in self.undo_states.drain() {
			if let Some(states) = states {
				self.states.insert(key, states);
			} else {
				self.states.remove(&key);
			}
		}
		self.generation += 1;
		true
	}

	/// Plans one command's hazards against prior uses, then records its resulting resource states.
	pub(crate) fn consume(
		&mut self,
		scope: MetalEncoderScope,
		uses: impl IntoIterator<Item = MetalResourceUse>,
	) -> MetalBarrier {
		let consolidated = Self::consolidate(uses);
		self.consume_consolidated(scope, &consolidated, &[], false).0
	}

	/// Plans one draw or dispatch that binds the descriptor table `descriptors`, plus its command-specific uses.
	///
	/// When the table's uses were last applied in this same encoder and the history has not changed since, its
	/// read-only uses are skipped: the barriers encoded for that earlier command still order every later command in
	/// the encoder, and reapplying the same reads would leave the history as it is.
	pub(crate) fn consume_descriptors(
		&mut self,
		scope: MetalEncoderScope,
		descriptors: &mut DescriptorUses,
		additional_uses: impl IntoIterator<Item = MetalResourceUse>,
	) -> MetalBarrier {
		let additional_uses = Self::consolidate(additional_uses);
		let aliases_primary = additional_uses.iter().any(|additional_use| {
			descriptors
				.uses
				.iter()
				.any(|primary_use| primary_use.key == additional_use.key && primary_use.region.overlaps(additional_use.region))
		});
		let settled = !aliases_primary && descriptors.settled == Some((scope, self.generation));
		let primary_uses = if settled {
			&descriptors.uses[..descriptors.writable]
		} else {
			&descriptors.uses[..]
		};
		let (barrier, generation) = self.consume_consolidated(scope, primary_uses, &additional_uses, aliases_primary);
		descriptors.settled = generation.map(|generation| (scope, generation));
		barrier
	}

	/// Plans and applies one command's consolidated uses.
	///
	/// Returns the barrier and, unless the uses alias, the generation right after the primary uses were applied, so the
	/// primary table's own writes do not stop its reads from being skipped by the next command.
	fn consume_consolidated(
		&mut self,
		scope: MetalEncoderScope,
		primary_uses: &[MetalResourceUse],
		additional_uses: &[MetalResourceUse],
		aliases_primary: bool,
	) -> (MetalBarrier, Option<u64>) {
		let mut barrier = MetalBarrier::default();
		#[cfg(debug_assertions)]
		self.hazards.clear();
		self.plan(scope, primary_uses, &mut barrier);
		self.plan(scope, additional_uses, &mut barrier);

		if aliases_primary {
			// The uncommon alias path may copy descriptors so overlapping uses become one atomic command state.
			let mut uses = primary_uses.iter().copied().collect::<SmallVec<[_; 16]>>();
			uses.extend_from_slice(additional_uses);
			Self::consolidate_in_place(&mut uses);
			for resource_use in uses {
				self.apply_use(scope, resource_use);
			}
			return (barrier, None);
		}
		for &resource_use in primary_uses {
			self.apply_use(scope, resource_use);
		}
		let generation = self.generation;
		for &resource_use in additional_uses {
			self.apply_use(scope, resource_use);
		}
		(barrier, Some(generation))
	}

	/// Records accesses that occurred throughout an encoder without adding an artificial trailing command.
	pub(crate) fn record_final(&mut self, scope: MetalEncoderScope, uses: impl IntoIterator<Item = MetalResourceUse>) {
		for resource_use in Self::consolidate(uses) {
			self.apply_use(scope, resource_use);
		}
	}

	/// Removes a presented drawable because CAMetalLayer will not expose this acquisition to later commands.
	pub(crate) fn forget_drawable(&mut self, texture: &ProtocolObject<dyn mtl::MTLTexture>) {
		let key = MetalResourceKey::drawable(texture);
		self.remember(key);
		if self.states.remove(&key).is_some() {
			self.generation += 1;
		}
	}

	/// Converts command-local encoder scopes into queue history and commits the recording transaction.
	pub(crate) fn finish_recording(&mut self) {
		assert!(
			std::mem::take(&mut self.recording),
			"Metal resource tracker finalization failed. The most likely cause is that resource recording was not started.",
		);
		for key in self.undo_states.keys() {
			let Some(states) = self.states.get_mut(key) else {
				continue;
			};
			for state in states.iter_mut() {
				state.scope = MetalEncoderScope::Queue;
			}
			while let Some((left, right)) = (0..states.len()).find_map(|left| {
				((left + 1)..states.len())
					.find(|&right| states[left].region == states[right].region && states[left].access == states[right].access)
					.map(|right| (left, right))
			}) {
				let right = states.swap_remove(right);
				states[left].stages |= right.stages;
			}
		}
		self.undo_states.clear();
		// Every state moved to queue scope, so hazards now need queue barriers rather than encoder barriers.
		self.generation += 1;
	}

	/// Consolidates one materialized use table once so command recording can consume it by reference.
	pub(crate) fn consolidate_in_place(uses: &mut SmallVec<[MetalResourceUse; 16]>) {
		uses.retain(|resource_use| !resource_use.stages.is_empty() && !resource_use.access.is_empty());
		uses.sort_unstable_by_key(|resource_use| (resource_use.key, resource_use.region));

		while let Some((left, right)) = (0..uses.len()).find_map(|left| {
			((left + 1)..uses.len())
				.take_while(|&right| uses[right].key == uses[left].key)
				.find(|&right| uses[left].region.overlaps(uses[right].region))
				.map(|right| (left, right))
		}) {
			let right = uses.remove(right);
			uses[left].merge(right);
		}
	}

	/// Consolidates duplicate uses so one GPU command is compared only with state from earlier commands.
	fn consolidate(uses: impl IntoIterator<Item = MetalResourceUse>) -> SmallVec<[MetalResourceUse; 16]> {
		let mut consolidated = uses.into_iter().collect::<SmallVec<[_; 16]>>();
		Self::consolidate_in_place(&mut consolidated);
		consolidated
	}

	fn plan(&mut self, scope: MetalEncoderScope, uses: &[MetalResourceUse], barrier: &mut MetalBarrier) {
		for resource_use in uses {
			let Some(states) = self.states.get(&resource_use.key) else {
				continue;
			};
			for state in states.iter().filter(|state| state.region.overlaps(resource_use.region)) {
				if !Self::has_hazard(state.access, resource_use.access) {
					continue;
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
				let (after, before, visibility) = if state.scope == scope {
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

	fn remember(&mut self, key: MetalResourceKey) {
		if self.recording && !self.undo_states.contains_key(&key) {
			self.undo_states.insert(key, self.states.get(&key).cloned());
		}
	}

	fn apply_use(&mut self, scope: MetalEncoderScope, resource_use: MetalResourceUse) {
		self.remember(resource_use.key);
		let states = self.states.entry(resource_use.key).or_default();
		let has_hazard = states
			.iter()
			.filter(|state| state.region.overlaps(resource_use.region))
			.any(|state| Self::has_hazard(state.access, resource_use.access));

		if has_hazard {
			// Recording the same access again with nothing in between, such as a render pass's attachment writes after
			// each draw, replaces the only state it would remove with an identical one, so the history stays as it is.
			let mut covered = states.iter().filter(|state| resource_use.region.covers(state.region));
			if let (Some(state), None) = (covered.next(), covered.next())
				&& state.scope == scope
				&& state.region == resource_use.region
				&& state.access == resource_use.access
				&& state.stages == resource_use.stages
			{
				return;
			}
			states.retain(|state| !resource_use.region.covers(state.region));
			self.generation += 1;
		} else if let Some(state) = states
			.iter_mut()
			.find(|state| state.scope == scope && state.region == resource_use.region && state.access == resource_use.access)
		{
			// Without a hazard this use is a read, and widening a read's stages cannot give another read a hazard.
			state.stages |= resource_use.stages;
			return;
		} else if resource_use.access.intersects(crate::AccessPolicies::WRITE) {
			self.generation += 1;
		}

		states.push(MetalResourceState {
			region: resource_use.region,
			stages: resource_use.stages,
			access: resource_use.access,
			scope,
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

	fn descriptor_table(uses: &[MetalResourceUse]) -> DescriptorUses {
		DescriptorUses::new(uses.iter().copied().collect())
	}

	#[test]
	fn read_after_read_needs_no_barrier() {
		let mut tracker = MetalResourceTracker::default();
		tracker.consume(
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Blit)],
		);
		let barrier = tracker.consume(
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert!(!barrier.has_queue_dependency());
		assert!(!barrier.has_encoder_dependency());
	}

	#[test]
	fn queue_write_to_dispatch_read_uses_precise_stages() {
		let mut tracker = MetalResourceTracker::default();
		tracker.consume(
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);
		let barrier = tracker.consume(
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
		tracker.consume(
			MetalEncoderScope::Encoder(4),
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Dispatch)],
		);
		let barrier = tracker.consume(
			MetalEncoderScope::Encoder(4),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Blit)],
		);

		assert!(!barrier.has_queue_dependency());
		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
	}

	#[test]
	fn disjoint_buffer_ranges_do_not_conflict() {
		let mut tracker = MetalResourceTracker::default();
		tracker.consume(
			MetalEncoderScope::Queue,
			[MetalResourceUse::buffer(
				BufferHandle(1),
				0,
				64,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			)],
		);
		let barrier = tracker.consume(
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
		tracker.consume(scope, [attachment]);
		tracker.consume(scope, [buffer(crate::AccessPolicies::READ, mtl::MTLStages::Fragment)]);
		tracker.record_final(scope, [attachment]);
		tracker.finish_recording();

		let barrier = tracker.consume(
			MetalEncoderScope::Encoder(3),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Fragment);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.queue_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn repeated_descriptor_reads_see_each_render_attachment_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(2);
		let attachment = buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Fragment);
		let descriptor = buffer(crate::AccessPolicies::READ, mtl::MTLStages::Fragment);
		tracker.consume(scope, [attachment]);
		tracker.consume(scope, [descriptor]);
		tracker.record_final(scope, [attachment]);

		let barrier = tracker.consume(scope, [descriptor]);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Fragment);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn descriptor_read_after_an_intervening_blit_write_is_synchronized() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let mut descriptors = descriptor_table(&[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)]);
		tracker.consume_descriptors(scope, &mut descriptors, []);
		// The second command settles the reads, so the third must still notice the write in between.
		tracker.consume_descriptors(scope, &mut descriptors, []);
		tracker.consume(scope, [buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)]);
		let barrier = tracker.consume_descriptors(scope, &mut descriptors, []);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Dispatch);
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
		tracker.consume_descriptors(
			scope,
			&mut descriptors,
			[MetalResourceUse::buffer(
				BufferHandle(1),
				0,
				128,
				mtl::MTLStages::Vertex,
				crate::AccessPolicies::READ,
			)],
		);
		let barrier = tracker.consume(
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
		tracker.consume_descriptors(scope, &mut descriptors, []);
		tracker.consume_descriptors(scope, &mut descriptors, []);

		let barrier = tracker.consume(scope, [buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)]);

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
		tracker.consume_descriptors(scope, &mut descriptors, []);

		let barrier = tracker.consume_descriptors(scope, &mut descriptors, []);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Dispatch);
	}

	#[test]
	fn descriptor_reads_in_each_encoder_wait_for_an_earlier_write() {
		let mut tracker = MetalResourceTracker::default();
		// Reading one mip level leaves the whole-image write in the history, so every encoder must wait for it.
		let image = |mip_level, stages, access| MetalResourceUse::image(ImageHandle(1), mip_level, None, stages, access);
		let mut descriptors = descriptor_table(&[image(Some(0), mtl::MTLStages::Fragment, crate::AccessPolicies::READ)]);
		tracker.consume(
			MetalEncoderScope::Queue,
			[image(None, mtl::MTLStages::Dispatch, crate::AccessPolicies::WRITE)],
		);
		tracker.consume_descriptors(MetalEncoderScope::Encoder(1), &mut descriptors, []);

		let barrier = tracker.consume_descriptors(MetalEncoderScope::Encoder(2), &mut descriptors, []);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Fragment);
	}

	#[test]
	fn abandoned_recording_restores_queue_history() {
		let mut tracker = MetalResourceTracker::default();
		tracker.consume(
			MetalEncoderScope::Queue,
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);
		tracker.begin_recording();
		tracker.consume(
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		tracker.rollback_recording();

		let barrier = tracker.consume(
			MetalEncoderScope::Encoder(2),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);

		assert_eq!(barrier.queue_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.queue_before, mtl::MTLStages::Dispatch);
	}

	#[test]
	fn write_after_read_uses_execution_only_visibility() {
		let mut tracker = MetalResourceTracker::default();
		tracker.consume(
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)],
		);
		let barrier = tracker.consume(
			MetalEncoderScope::Encoder(1),
			[buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)],
		);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::None);
	}
}
