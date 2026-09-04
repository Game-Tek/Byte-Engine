//! Bounded asynchronous animation loading and decoded clip residency.

use super::*;

/// The `AnimationPoolConfig` struct sets the decoded clip memory available to an [`AnimationPool`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationPoolConfig {
	byte_budget: NonZeroUsize,
}

impl AnimationPoolConfig {
	/// Creates a strict byte budget for decoded animation resources.
	pub const fn new(byte_budget: NonZeroUsize) -> Self {
		Self { byte_budget }
	}

	/// Returns the maximum estimated decoded bytes retained by the pool cache.
	pub const fn byte_budget(self) -> usize {
		self.byte_budget.get()
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnimationArenaRegion {
	offset: usize,
	word_count: usize,
}

impl AnimationArenaRegion {
	fn end(self) -> usize {
		self.offset + self.word_count
	}
}

#[derive(Debug)]
struct CachedAnimation {
	skeleton: Arc<Skeleton>,
	region: AnimationArenaRegion,
	last_used: std::cell::Cell<u64>,
	lease_count: std::cell::Cell<usize>,
}

/// Pins one resident arena region for the duration of an animation evaluation.
struct ResidentAnimationLease<'a> {
	entry: &'a CachedAnimation,
	words: &'a [u32],
}

impl ResidentAnimationLease<'_> {
	fn packed(&self) -> PackedAnimation<'_> {
		PackedAnimation::from_words(self.words)
	}

	fn skeleton(&self) -> &Skeleton {
		&self.entry.skeleton
	}

	fn shared_skeleton(&self) -> Arc<Skeleton> {
		Arc::clone(&self.entry.skeleton)
	}
}

impl Drop for ResidentAnimationLease<'_> {
	fn drop(&mut self) {
		self.entry.lease_count.set(
			self.entry
				.lease_count
				.get()
				.checked_sub(1)
				.expect("Resident animation lease count must match evaluation borrows."),
		);
	}
}

/// Tracks one clip through asynchronous loading, arena admission, residency, or failure.
enum AnimationPoolEntry {
	Loading { command: Option<AnimationLoadCommand> },
	Resident(CachedAnimation),
	Blocked { resident_bytes: usize, animation: Animation },
	Failed,
}

enum AnimationLoadCommand {
	Load { resource_id: String },
}

enum AnimationLoadCompletion {
	Ready { resource_id: String, animation: Animation },
	Failed { resource_id: String, error: String },
}

/// The `AnimationPoolRequest` enum reports whether a lease can be sampled immediately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPoolRequest {
	/// Acquire and sample the requested clip.
	Ready,
	/// Wait while the requested clip loads asynchronously.
	Loading,
	/// Retry when loading or residency capacity becomes available.
	WaitingForCapacity,
	/// Handle the load failure or call [`AnimationPool::retry`].
	Failed,
}

/// The `AnimationPoolEvent` enum reports load outcomes that require application-level handling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationPoolEvent {
	/// Handle a resource that could not be loaded.
	LoadFailed {
		/// The identifier of the resource that failed to load.
		resource_id: String,
		/// The load error reported by the resource system.
		error: String,
	},
	/// Increase the pool budget or use a smaller animation resource.
	Oversized {
		/// The identifier of the resource that exceeded the pool budget.
		resource_id: String,
		/// The estimated decoded size required by the resource.
		resident_bytes: usize,
		/// The maximum decoded size available to the pool.
		byte_budget: usize,
	},
	/// Request the resource again before its next use.
	Evicted {
		/// The identifier of the resource removed from the resident cache.
		resource_id: String,
	},
}

/// The `AnimationPool` struct owns a preallocated word arena and byte-bounded LRU clip cache.
///
/// Graph clips keep stable [`AnimationLease`] handles across eviction. During
/// evaluation, the player pins resident arena regions so admission cannot reuse
/// their words until sampling completes.
pub struct AnimationPool {
	commands: kanal::Sender<AnimationLoadCommand>,
	completions: kanal::Receiver<AnimationLoadCompletion>,
	storage: Box<[u32]>,
	free_regions: Vec<AnimationArenaRegion>,
	entries: HashMap<AnimationLease, AnimationPoolEntry>,
	events: VecDeque<AnimationPoolEvent>,
	byte_budget: usize,
	resident_bytes: usize,
	next_use: std::cell::Cell<u64>,
	commands_closed: bool,
	completions_closed: bool,
}

impl AnimationPool {
	/// Creates the pool, preallocates its complete word arena, and returns its load worker.
	pub fn new(resource_manager: EntityHandle<ResourceManager>, config: AnimationPoolConfig) -> (Self, AnimationLoadWorker) {
		let (commands, command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let word_capacity = config.byte_budget() / std::mem::size_of::<u32>();
		let free_regions = (word_capacity > 0)
			.then_some(AnimationArenaRegion {
				offset: 0,
				word_count: word_capacity,
			})
			.into_iter()
			.collect();
		(
			Self {
				commands: commands.to_sync(),
				completions: completions.to_sync(),
				storage: vec![0; word_capacity].into_boxed_slice(),
				free_regions,
				entries: HashMap::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
				byte_budget: config.byte_budget(),
				resident_bytes: 0,
				next_use: std::cell::Cell::new(0),
				commands_closed: false,
				completions_closed: false,
			},
			AnimationLoadWorker {
				resource_manager,
				commands: command_receiver,
				completions: completion_sender,
			},
		)
	}

	/// Submits queued loads and adopts completed clips without blocking the caller.
	///
	/// Call this once per application tick before advancing graph players that
	/// share this pool. This keeps asynchronous queue polling independent from
	/// the number of animated skeletons.
	pub fn update(&mut self) {
		self.submit_requests();
		if self.completions_closed {
			return;
		}
		loop {
			match self.completions.try_recv_realtime() {
				Ok(Some(completion)) => self.process_completion(completion),
				Ok(None) => break,
				Err(_) => {
					self.completions_closed = true;
					break;
				}
			}
		}
	}

	/// Returns lease residency or queues an asynchronous reload after eviction.
	pub fn request(&mut self, lease: &AnimationLease) -> AnimationPoolRequest {
		let Some(entry) = self.entries.get(lease) else {
			return if self.queue_load(lease.clone()) {
				AnimationPoolRequest::Loading
			} else {
				AnimationPoolRequest::WaitingForCapacity
			};
		};
		match entry {
			AnimationPoolEntry::Resident(_) => AnimationPoolRequest::Ready,
			AnimationPoolEntry::Loading { .. } => AnimationPoolRequest::Loading,
			AnimationPoolEntry::Failed => AnimationPoolRequest::Failed,
			AnimationPoolEntry::Blocked { resident_bytes, .. } => {
				let resident_bytes = *resident_bytes;
				if !self.make_room(resident_bytes) {
					return AnimationPoolRequest::WaitingForCapacity;
				}
				let Some(AnimationPoolEntry::Blocked { animation, .. }) = self.entries.remove(lease) else {
					unreachable!("Blocked animation entry changed during synchronous admission.");
				};
				self.write_animation(lease.clone(), animation);
				AnimationPoolRequest::Ready
			}
		}
	}

	/// Pins a resident clip until the returned evaluation lease is dropped.
	fn acquire(&self, lease: &AnimationLease) -> Option<ResidentAnimationLease<'_>> {
		let AnimationPoolEntry::Resident(entry) = self.entries.get(lease)? else {
			return None;
		};
		entry.last_used.set(self.next_use());
		entry.lease_count.set(entry.lease_count.get() + 1);
		Some(ResidentAnimationLease {
			entry,
			words: &self.storage[entry.region.offset..entry.region.end()],
		})
	}

	/// Clears one recorded load failure and requests that lease again.
	pub fn retry(&mut self, lease: &AnimationLease) -> AnimationPoolRequest {
		if matches!(self.entries.get(lease), Some(AnimationPoolEntry::Failed)) {
			self.entries.remove(lease);
		}
		self.request(lease)
	}

	/// Returns bytes occupied by resident packed clip regions.
	pub const fn resident_bytes(&self) -> usize {
		self.resident_bytes
	}

	/// Returns the configured arena byte budget.
	pub const fn byte_budget(&self) -> usize {
		self.byte_budget
	}

	/// Drains asynchronous load and eviction events without allocating a new event list.
	pub fn drain_events(&mut self) -> std::collections::vec_deque::Drain<'_, AnimationPoolEvent> {
		self.events.drain(..)
	}

	fn next_use(&self) -> u64 {
		let value = self.next_use.get();
		self.next_use.set(value.wrapping_add(1));
		value
	}

	fn queue_load(&mut self, lease: AnimationLease) -> bool {
		let loading_count = self
			.entries
			.values()
			.filter(|entry| matches!(entry, AnimationPoolEntry::Loading { .. }))
			.count();
		if self.commands_closed || loading_count >= ANIMATION_LOAD_QUEUE_CAPACITY {
			return false;
		}
		self.entries.insert(
			lease.clone(),
			AnimationPoolEntry::Loading {
				command: Some(AnimationLoadCommand::Load {
					resource_id: lease.resource_id().to_owned(),
				}),
			},
		);
		true
	}
	fn push_event(&mut self, event: AnimationPoolEvent) {
		if self.events.len() == ANIMATION_POOL_EVENT_CAPACITY {
			self.events.pop_front();
		}
		self.events.push_back(event);
	}

	fn submit_requests(&mut self) {
		if self.commands_closed {
			return;
		}
		for entry in self.entries.values_mut() {
			let AnimationPoolEntry::Loading { command } = entry else {
				continue;
			};
			if command.is_none() {
				continue;
			}
			match self.commands.try_send_option_realtime(command) {
				Ok(_) => {}
				Err(_) => {
					self.commands_closed = true;
					break;
				}
			}
		}
	}

	fn process_completion(&mut self, completion: AnimationLoadCompletion) {
		let (lease, completion) = match completion {
			AnimationLoadCompletion::Ready { resource_id, animation } => (AnimationLease::new(resource_id), Ok(animation)),
			AnimationLoadCompletion::Failed { resource_id, error } => (AnimationLease::new(resource_id), Err(error)),
		};
		if !matches!(self.entries.get(&lease), Some(AnimationPoolEntry::Loading { .. })) {
			return;
		}
		match completion {
			Ok(animation) => self.admit_lease(lease, animation),
			Err(error) => {
				self.entries.insert(lease.clone(), AnimationPoolEntry::Failed);
				self.push_event(AnimationPoolEvent::LoadFailed {
					resource_id: lease.resource_id().to_owned(),
					error,
				});
			}
		}
	}

	fn admit_lease(&mut self, lease: AnimationLease, animation: Animation) {
		let resident_bytes = PackedAnimationData::resident_bytes(&animation);
		if resident_bytes > self.byte_budget || resident_bytes / std::mem::size_of::<u32>() > self.storage.len() {
			self.entries.insert(lease.clone(), AnimationPoolEntry::Failed);
			self.push_event(AnimationPoolEvent::Oversized {
				resource_id: lease.resource_id().to_owned(),
				resident_bytes,
				byte_budget: self.byte_budget,
			});
			return;
		}
		if !self.make_room(resident_bytes) {
			let blocked_count = self
				.entries
				.values()
				.filter(|entry| matches!(entry, AnimationPoolEntry::Blocked { .. }))
				.count();
			if blocked_count < ANIMATION_LOAD_QUEUE_CAPACITY {
				self.entries.insert(
					lease,
					AnimationPoolEntry::Blocked {
						resident_bytes,
						animation,
					},
				);
			} else {
				// Drop this completed payload so a later request can retry after capacity frees.
				self.entries.remove(&lease);
			}
			return;
		}
		self.write_animation(lease, animation);
	}

	#[cfg(test)]
	fn admit(&mut self, resource_id: String, animation: Animation) {
		self.admit_lease(AnimationLease::new(resource_id), animation);
	}

	/// Packs a completed load only after admission owns a contiguous arena range.
	fn write_animation(&mut self, lease: AnimationLease, animation: Animation) {
		let packed = PackedAnimationData::from_resource(animation);
		let resident_bytes = packed.data.len() * std::mem::size_of::<u32>();
		let region = self
			.take_region(packed.data.len())
			.expect("Animation admission reserved one contiguous arena region.");
		self.storage[region.offset..region.end()].copy_from_slice(&packed.data);
		self.resident_bytes += resident_bytes;
		let replaced = self.entries.insert(
			lease,
			AnimationPoolEntry::Resident(CachedAnimation {
				skeleton: Arc::new(packed.skeleton.into_resource()),
				region,
				last_used: std::cell::Cell::new(self.next_use()),
				lease_count: std::cell::Cell::new(0),
			}),
		);
		debug_assert!(
			matches!(
				replaced,
				None | Some(AnimationPoolEntry::Loading { .. }) | Some(AnimationPoolEntry::Blocked { .. })
			),
			"Animation admission must not replace an unrelated entry."
		);
	}

	/// Evicts unleased LRU entries until one contiguous arena range can hold the requested words.
	fn make_room(&mut self, required_bytes: usize) -> bool {
		let required_words = required_bytes.div_ceil(std::mem::size_of::<u32>());
		if required_bytes > self.byte_budget || required_words > self.storage.len() {
			return false;
		}
		while !self.free_regions.iter().any(|region| region.word_count >= required_words) {
			let Some(lease) = self
				.entries
				.iter()
				.filter_map(|(lease, entry)| match entry {
					AnimationPoolEntry::Resident(entry) if entry.lease_count.get() == 0 => Some((lease, entry.last_used.get())),
					_ => None,
				})
				.min_by_key(|(_, last_used)| *last_used)
				.map(|(lease, _)| lease.clone())
			else {
				return false;
			};
			let Some(AnimationPoolEntry::Resident(evicted)) = self.entries.remove(&lease) else {
				unreachable!("The selected eviction candidate must remain resident.");
			};
			self.resident_bytes = self
				.resident_bytes
				.saturating_sub(evicted.region.word_count * std::mem::size_of::<u32>());
			self.return_region(evicted.region);
			self.push_event(AnimationPoolEvent::Evicted {
				resource_id: lease.resource_id().to_owned(),
			});
		}
		true
	}

	fn take_region(&mut self, word_count: usize) -> Option<AnimationArenaRegion> {
		let index = self.free_regions.iter().position(|region| region.word_count >= word_count)?;
		let available = self.free_regions[index];
		let region = AnimationArenaRegion {
			offset: available.offset,
			word_count,
		};
		if available.word_count == word_count {
			self.free_regions.swap_remove(index);
		} else {
			self.free_regions[index] = AnimationArenaRegion {
				offset: available.offset + word_count,
				word_count: available.word_count - word_count,
			};
		}
		Some(region)
	}

	/// Returns and coalesces an arena region so fragmented evictions can satisfy later clips.
	fn return_region(&mut self, region: AnimationArenaRegion) {
		let index = self.free_regions.partition_point(|free| free.offset < region.offset);
		self.free_regions.insert(index, region);
		let mut index = index.saturating_sub(1);
		while index + 1 < self.free_regions.len() {
			if self.free_regions[index].end() != self.free_regions[index + 1].offset {
				index += 1;
				continue;
			}
			let right = self.free_regions.remove(index + 1);
			self.free_regions[index].word_count += right.word_count;
		}
	}
}

/// The `AnimationLoadWorker` struct resolves animation resources away from synchronous pose evaluation.
pub struct AnimationLoadWorker {
	resource_manager: EntityHandle<ResourceManager>,
	commands: kanal::AsyncReceiver<AnimationLoadCommand>,
	completions: kanal::AsyncSender<AnimationLoadCompletion>,
}

impl AnimationLoadWorker {
	/// Loads queued clips until the animation pool drops its command channel.
	pub async fn run(self) {
		while let Ok(AnimationLoadCommand::Load { resource_id }) = self.commands.recv().await {
			let completion = match self.resource_manager.request::<Animation>(&resource_id).await {
				Ok(reference) => AnimationLoadCompletion::Ready {
					resource_id,
					// Animation resources keep decoded curves in metadata, so the
					// reference reader is intentionally released before pooling.
					animation: reference.into_resource(),
				},
				Err(error) => AnimationLoadCompletion::Failed { resource_id, error },
			};
			if self.completions.send(completion).await.is_err() {
				break;
			}
			async_runtime::yield_now().await;
		}
	}
}

mod player;

#[doc(hidden)]
pub mod benchmarks;

pub use player::{
	AnimationEvaluation, AnimationGraphPlayer, AnimationGraphPlayerError, AnimationGraphPose, RootMotionRotation,
	RootMotionSettings, RootMotionTranslation,
};

/// Builds a pool backed by a detached load queue, for tests that cannot supply a real
/// [`ResourceManager`] handle to [`AnimationPool::new`].
///
/// Shared with the [`player`] tests so both exercise one arena setup.
#[cfg(test)]
pub(crate) fn test_pool(byte_budget: usize) -> AnimationPool {
	let (commands, _command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
	let (_completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
	let word_capacity = byte_budget / std::mem::size_of::<u32>();
	AnimationPool {
		commands: commands.to_sync(),
		completions: completions.to_sync(),
		storage: vec![0; word_capacity].into_boxed_slice(),
		free_regions: vec![AnimationArenaRegion {
			offset: 0,
			word_count: word_capacity,
		}],
		entries: HashMap::new(),
		events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
		byte_budget,
		resident_bytes: 0,
		next_use: std::cell::Cell::new(0),
		commands_closed: false,
		completions_closed: false,
	}
}

#[cfg(test)]
mod tests {
	use std::collections::{HashMap, VecDeque};

	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode},
		},
	};

	use super::*;
	use crate::MediaTime;

	fn test_skeleton() -> Skeleton {
		Skeleton {
			nodes: vec![SkeletonNode {
				name: Some("root".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		}
	}

	fn test_animation(name: &str, end_translation: f32) -> Animation {
		Animation {
			name: Some(name.into()),
			skeleton: Reference::in_memory("test.skeleton", test_skeleton()),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0; 3], [end_translation, 0.0, 0.0]],
				}),
				rotation: None,
				scale: None,
			}],
		}
	}

	/// Measures the representation retained by the pool rather than the transient resource representation.
	fn packed_test_animation_bytes(name: &str, end_translation: f32) -> usize {
		PackedAnimationData::resident_bytes(&test_animation(name, end_translation))
	}

	#[test]
	fn pool_evicts_lru_entries_and_evaluation_leases_pin_arena_regions() {
		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = packed_test_animation_bytes("idle", 1.0).max(packed_test_animation_bytes("walk", 2.0));
		let mut first_pool = test_pool(budget);

		first_pool.admit("idle.animation".into(), idle);
		first_pool.admit("walk.animation".into(), walk);

		assert!(matches!(
			first_pool.entries.get(&AnimationLease::new("walk.animation")),
			Some(AnimationPoolEntry::Resident(_))
		));
		assert!(!first_pool.entries.contains_key(&AnimationLease::new("idle.animation")));
		assert!(
			first_pool
				.drain_events()
				.any(|event| matches!(event, AnimationPoolEvent::Evicted { resource_id } if resource_id == "idle.animation"))
		);
		let evicted_idle = AnimationLease::new("idle.animation");

		assert_eq!(first_pool.request(&evicted_idle), AnimationPoolRequest::Loading);

		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = packed_test_animation_bytes("idle", 1.0).max(packed_test_animation_bytes("walk", 2.0));
		let mut pool = test_pool(budget);
		pool.admit("idle.animation".into(), idle);
		let idle_lease = AnimationLease::new("idle.animation");
		let walk_lease = AnimationLease::new("walk.animation");

		assert_eq!(pool.request(&idle_lease), AnimationPoolRequest::Ready);
		let pinned = pool.acquire(&idle_lease).expect("expected cached idle animation");

		assert_eq!(pinned.entry.lease_count.get(), 1);
		drop(pinned);

		assert_eq!(
			match pool.entries.get(&idle_lease) {
				Some(AnimationPoolEntry::Resident(entry)) => entry.lease_count.get(),
				_ => panic!("idle animation should remain resident"),
			},
			0
		);

		pool.admit("walk.animation".into(), walk);

		assert!(!pool.entries.contains_key(&idle_lease));
		assert_eq!(pool.request(&walk_lease), AnimationPoolRequest::Ready);
	}

	#[test]
	fn pool_entries_follow_lru_eviction() {
		let clip_bytes = packed_test_animation_bytes("first", 1.0);
		let mut pool = test_pool(clip_bytes * 2);
		let first = AnimationLease::new("first.animation");
		let second = AnimationLease::new("second.animation");
		let third = AnimationLease::new("third.animation");
		pool.admit("first.animation".into(), test_animation("first", 1.0));
		pool.admit("second.animation".into(), test_animation("second", 1.0));
		pool.admit("third.animation".into(), test_animation("third", 1.0));

		assert!(matches!(pool.entries.get(&second), Some(AnimationPoolEntry::Resident(_))));
		assert_eq!(pool.request(&first), AnimationPoolRequest::Loading);
		assert_eq!(pool.request(&second), AnimationPoolRequest::Ready);
		assert_eq!(pool.request(&third), AnimationPoolRequest::Ready);
		assert!(pool.acquire(&second).is_some());
	}

	#[test]
	fn oversized_clips_fail_once_until_the_caller_explicitly_retries_them() {
		let animation = test_animation("oversized", 1.0);
		let mut pool = test_pool(packed_test_animation_bytes("oversized", 1.0) - 1);
		pool.admit("oversized.animation".into(), animation);

		assert!(matches!(
			pool.request(&AnimationLease::new("oversized.animation")),
			AnimationPoolRequest::Failed
		));
		assert!(pool.drain_events().any(|event| matches!(
			event,
			AnimationPoolEvent::Oversized {
				resource_id,
				..
			} if resource_id == "oversized.animation"
		)));
	}
}
