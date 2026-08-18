use objc2_metal::MTL4CommandEncoder as _;

use super::*;

/// The `MetalResourceKey` enum identifies one native allocation across command recordings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum MetalResourceKey {
	Buffer(BufferHandle),
	Image(ImageHandle),
	SwapchainDrawable(usize),
}

impl MetalResourceKey {
	fn drawable(texture: &ProtocolObject<dyn mtl::MTLTexture>) -> Self {
		Self::SwapchainDrawable(std::ptr::from_ref(texture).cast::<()>() as usize)
	}
}

/// The `MetalResourceRegion` enum limits hazard tracking to an accessed buffer range or texture subresource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MetalResourceRegion {
	Buffer { start: usize, end: usize },
	Texture { mip_level: Option<u32>, layer: Option<u32> },
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
				end: offset.checked_add(size).unwrap_or(usize::MAX),
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

/// Converts GHI shader-stage visibility to the stages Metal 4 accepts in barrier commands.
pub(crate) fn to_metal_stages(stages: crate::Stages) -> mtl::MTLStages {
	[
		(crate::Stages::VERTEX | crate::Stages::INDEX, mtl::MTLStages::Vertex),
		(crate::Stages::TASK, mtl::MTLStages::Object),
		(crate::Stages::MESH, mtl::MTLStages::Mesh),
		(crate::Stages::FRAGMENT, mtl::MTLStages::Fragment),
		(
			crate::Stages::COMPUTE
				| crate::Stages::RAYGEN
				| crate::Stages::CLOSEST_HIT
				| crate::Stages::ANY_HIT
				| crate::Stages::INTERSECTION
				| crate::Stages::MISS
				| crate::Stages::CALLABLE,
			mtl::MTLStages::Dispatch,
		),
		(crate::Stages::TRANSFER, mtl::MTLStages::Blit),
		(
			crate::Stages::ACCELERATION_STRUCTURE_BUILD,
			mtl::MTLStages::AccelerationStructure,
		),
	]
	.into_iter()
	.fold(mtl::MTLStages::empty(), |metal, (source, target)| {
		if stages.intersects(source) {
			metal | target
		} else {
			metal
		}
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

	pub(crate) fn encode_compute(self, encoder: &ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>) {
		self.encode(ProtocolObject::from_ref(encoder));
	}

	pub(crate) fn encode_render(self, encoder: &ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>) {
		self.encode(ProtocolObject::from_ref(encoder));
	}

	/// Encodes this dependency through the stage-independent Metal 4 encoder protocol.
	fn encode(self, encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder>) {
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

/// The `MetalResourceTracker` struct retains region-aware access history for one Metal command queue.
#[derive(Default)]
pub(crate) struct MetalResourceTracker {
	states: HashMap<MetalResourceKey, SmallVec<[MetalResourceState; 2]>>,
	undo_states: HashMap<MetalResourceKey, Option<SmallVec<[MetalResourceState; 2]>>>,
	recording: bool,
}

impl MetalResourceTracker {
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
		true
	}

	/// Plans one command's hazards against prior uses, then records its resulting resource states.
	pub(crate) fn consume(
		&mut self,
		scope: MetalEncoderScope,
		uses: impl IntoIterator<Item = MetalResourceUse>,
	) -> MetalBarrier {
		let consolidated = Self::consolidate(uses);
		self.consume_preconsolidated(scope, &consolidated, [])
	}

	/// Plans one command from an immutable use table plus its small command-specific use list.
	pub(crate) fn consume_preconsolidated(
		&mut self,
		scope: MetalEncoderScope,
		primary_uses: &[MetalResourceUse],
		additional_uses: impl IntoIterator<Item = MetalResourceUse>,
	) -> MetalBarrier {
		let additional_uses = Self::consolidate(additional_uses);
		let mut barrier = MetalBarrier::default();
		self.plan(scope, primary_uses, &mut barrier);
		self.plan(scope, &additional_uses, &mut barrier);

		let aliases_primary = additional_uses.iter().any(|additional_use| {
			primary_uses
				.iter()
				.any(|primary_use| primary_use.key == additional_use.key && primary_use.region.overlaps(additional_use.region))
		});
		if aliases_primary {
			// The uncommon alias path may copy descriptors so overlapping uses become one atomic command state.
			let mut uses = primary_uses.iter().copied().collect::<SmallVec<[_; 16]>>();
			uses.extend(additional_uses);
			Self::consolidate_in_place(&mut uses);
			for resource_use in uses {
				self.apply_use(scope, resource_use);
			}
		} else {
			for resource_use in primary_uses.iter().copied().chain(additional_uses) {
				self.apply_use(scope, resource_use);
			}
		}
		barrier
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
		self.states.remove(&key);
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

	fn plan(&self, scope: MetalEncoderScope, uses: &[MetalResourceUse], barrier: &mut MetalBarrier) {
		for resource_use in uses {
			let Some(states) = self.states.get(&resource_use.key) else {
				continue;
			};
			for state in states.iter().filter(|state| state.region.overlaps(resource_use.region)) {
				if !Self::has_hazard(state.access, resource_use.access) {
					continue;
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
			states.retain(|state| !resource_use.region.covers(state.region));
		} else if let Some(state) = states
			.iter_mut()
			.find(|state| state.scope == scope && state.region == resource_use.region && state.access == resource_use.access)
		{
			state.stages |= resource_use.stages;
			return;
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
		let descriptors = [buffer(crate::AccessPolicies::READ, mtl::MTLStages::Dispatch)];
		tracker.consume_preconsolidated(scope, &descriptors, []);
		tracker.consume(scope, [buffer(crate::AccessPolicies::WRITE, mtl::MTLStages::Blit)]);
		let barrier = tracker.consume_preconsolidated(scope, &descriptors, []);

		assert_eq!(barrier.encoder_after, mtl::MTLStages::Blit);
		assert_eq!(barrier.encoder_before, mtl::MTLStages::Dispatch);
		assert_eq!(barrier.encoder_visibility, mtl::MTL4VisibilityOptions::Device);
	}

	#[test]
	fn overlapping_uses_in_one_command_preserve_the_write() {
		let mut tracker = MetalResourceTracker::default();
		let scope = MetalEncoderScope::Encoder(1);
		let descriptors = [MetalResourceUse::buffer(
			BufferHandle(1),
			0,
			64,
			mtl::MTLStages::Fragment,
			crate::AccessPolicies::WRITE,
		)];
		tracker.consume_preconsolidated(
			scope,
			&descriptors,
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
