//! Bounded asynchronous animation loading and decoded clip residency.

use super::*;

#[derive(Debug)]
struct CachedAnimation {
	skeleton: Arc<Skeleton>,
	/// The clip's word range in [`AnimationPool::storage`].
	region: std::ops::Range<usize>,
	/// The last [`AnimationPool::update`] whose step sampled the clip, which eviction picks the oldest of.
	///
	/// Players on several lanes stamp it while they read the pool, so it is atomic.
	last_used: AtomicU64,
}

/// The `ResidentAnimation` struct lends one resident clip's skeleton and packed words to a player that samples it.
///
/// It borrows the pool, so no admission or eviction, which take the pool mutably, can move the words while it lives.
pub(crate) struct ResidentAnimation<'a> {
	pub(crate) skeleton: &'a Arc<Skeleton>,
	pub(crate) packed: PackedAnimation<'a>,
}

/// Tracks one clip through asynchronous loading, arena admission, residency, or failure.
enum AnimationPoolEntry {
	/// Holds the resource ID until [`AnimationPool::update`] sends it to the load worker.
	Loading {
		resource_id: Option<String>,
	},
	Resident(CachedAnimation),
	/// Holds a packed clip until the arena has room for it.
	Blocked(PackedAnimationData),
	Failed,
}

/// A finished load: the requested resource ID and its packed clip or load error.
type AnimationLoadCompletion = (String, Result<PackedAnimationData, resource_management::RequestError>);

/// The `AnimationPoolRequest` enum reports whether a clip can be sampled immediately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPoolRequest {
	/// The clip is resident, so players sample it.
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
		error: resource_management::RequestError,
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
/// Graph clips keep their resource IDs as stable keys across eviction. Players only read the pool while they
/// evaluate, so any number of them can sample it at once, on any lanes; loads are admitted and clips evicted only
/// by the calls that take it mutably, [`Self::update`] and [`Self::request`], between evaluations.
pub struct AnimationPool {
	commands: kanal::Sender<String>,
	completions: kanal::Receiver<AnimationLoadCompletion>,
	storage: Box<[u32]>,
	free_words: utils::RangeAllocator,
	/// Players look up their clips several times per frame, so the map uses the engine's fast in-memory hasher.
	entries: utils::hash::HashMap<Box<str>, AnimationPoolEntry>,
	events: VecDeque<AnimationPoolEvent>,
	byte_budget: usize,
	resident_bytes: usize,
	/// Counts [`Self::update`] calls, the steps whose use stamps each clip's least-recently-used order.
	step: u64,
	commands_closed: bool,
	completions_closed: bool,
}

impl AnimationPool {
	/// Creates the pool, preallocates its complete word arena for `byte_budget` decoded bytes, and returns its load
	/// worker.
	///
	/// Spawn [`AnimationLoadWorker::run`] next on the async runtime that serves resource requests.
	pub fn new(resource_manager: EntityHandle<ResourceManager>, byte_budget: NonZeroUsize) -> (Self, AnimationLoadWorker) {
		let (pool, commands, completions) = Self::with_load_queues(byte_budget.get());
		(
			pool,
			AnimationLoadWorker {
				resource_manager,
				commands,
				completions,
			},
		)
	}

	/// Creates a pool whose load queues have no worker, for tests and benchmarks that admit clips directly.
	///
	/// Dropping the worker ends closes both queues, so no request ever reaches a load worker.
	pub(crate) fn detached(byte_budget: usize) -> Self {
		Self::with_load_queues(byte_budget).0
	}

	/// Preallocates the word arena and returns the pool with the worker ends of its load queues.
	fn with_load_queues(
		byte_budget: usize,
	) -> (
		Self,
		kanal::AsyncReceiver<String>,
		kanal::AsyncSender<AnimationLoadCompletion>,
	) {
		let (commands, command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let word_capacity = byte_budget / std::mem::size_of::<u32>();
		let pool = Self {
			commands: commands.to_sync(),
			completions: completions.to_sync(),
			storage: vec![0; word_capacity].into_boxed_slice(),
			free_words: utils::RangeAllocator::new(word_capacity, 1),
			entries: utils::hash::HashMap::with_capacity_and_hasher(ANIMATION_LOAD_QUEUE_CAPACITY, Default::default()),
			events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
			byte_budget,
			resident_bytes: 0,
			step: 0,
			commands_closed: false,
			completions_closed: false,
		};
		(pool, command_receiver, completion_sender)
	}

	/// Submits queued loads and adopts completed clips without blocking the caller.
	///
	/// Call this once per application tick before advancing graph players that
	/// share this pool. This keeps asynchronous queue polling independent from
	/// the number of animated skeletons.
	pub fn update(&mut self) {
		self.step += 1;
		self.submit_requests();
		while !self.completions_closed {
			match self.completions.try_recv_realtime() {
				Ok(Some(completion)) => self.process_completion(completion),
				Ok(None) => break,
				Err(_) => self.completions_closed = true,
			}
		}
	}

	/// Returns clip residency or queues an asynchronous reload after eviction.
	pub fn request(&mut self, resource_id: &str) -> AnimationPoolRequest {
		let Some(entry) = self.entries.get(resource_id) else {
			return if self.queue_load(resource_id) {
				AnimationPoolRequest::Loading
			} else {
				AnimationPoolRequest::WaitingForCapacity
			};
		};
		match entry {
			AnimationPoolEntry::Resident(_) => AnimationPoolRequest::Ready,
			AnimationPoolEntry::Loading { .. } => AnimationPoolRequest::Loading,
			AnimationPoolEntry::Failed => AnimationPoolRequest::Failed,
			AnimationPoolEntry::Blocked(packed) => {
				if !self.make_room(packed.resident_bytes()) {
					return AnimationPoolRequest::WaitingForCapacity;
				}
				let Some((resource_id, AnimationPoolEntry::Blocked(packed))) = self.entries.remove_entry(resource_id) else {
					unreachable!("Blocked animation entry changed during synchronous admission.");
				};
				self.write_animation(resource_id, packed);
				AnimationPoolRequest::Ready
			}
		}
	}

	/// Returns a resident clip to sample, and stamps it as used this step.
	///
	/// It only reads the pool, so players on several lanes call it at once.
	pub(crate) fn resident(&self, resource_id: &str) -> Option<ResidentAnimation<'_>> {
		let AnimationPoolEntry::Resident(entry) = self.entries.get(resource_id)? else {
			return None;
		};
		// Only the first use in a step writes, so lanes that share a clip don't contend for its stamp.
		if entry.last_used.load(Ordering::Relaxed) < self.step {
			entry.last_used.store(self.step, Ordering::Relaxed);
		}
		Some(ResidentAnimation {
			skeleton: &entry.skeleton,
			packed: PackedAnimation::from_words(&self.storage[entry.region.clone()]),
		})
	}

	/// Returns whether a clip is resident and can be sampled now.
	pub(crate) fn is_resident(&self, resource_id: &str) -> bool {
		matches!(self.entries.get(resource_id), Some(AnimationPoolEntry::Resident(_)))
	}

	/// Clears one recorded load failure and requests that clip again.
	pub fn retry(&mut self, resource_id: &str) -> AnimationPoolRequest {
		if matches!(self.entries.get(resource_id), Some(AnimationPoolEntry::Failed)) {
			self.entries.remove(resource_id);
		}
		self.request(resource_id)
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

	fn queue_load(&mut self, resource_id: &str) -> bool {
		let loading_count = self
			.entries
			.values()
			.filter(|entry| matches!(entry, AnimationPoolEntry::Loading { .. }))
			.count();
		if self.commands_closed || loading_count >= ANIMATION_LOAD_QUEUE_CAPACITY {
			return false;
		}
		self.entries.insert(
			resource_id.into(),
			AnimationPoolEntry::Loading {
				resource_id: Some(resource_id.to_owned()),
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
			// kanal panics on an empty option, so skip loads whose request already went out.
			if let AnimationPoolEntry::Loading { resource_id } = entry
				&& resource_id.is_some()
				&& self.commands.try_send_option_realtime(resource_id).is_err()
			{
				self.commands_closed = true;
				break;
			}
		}
	}

	fn process_completion(&mut self, (resource_id, completion): AnimationLoadCompletion) {
		if !matches!(
			self.entries.get(resource_id.as_str()),
			Some(AnimationPoolEntry::Loading { .. })
		) {
			return;
		}
		match completion {
			Ok(packed) => self.admit(resource_id.into(), packed),
			Err(error) => {
				self.entries.insert(resource_id.as_str().into(), AnimationPoolEntry::Failed);
				self.push_event(AnimationPoolEvent::LoadFailed { resource_id, error });
			}
		}
	}

	fn admit(&mut self, resource_id: Box<str>, packed: PackedAnimationData) {
		let resident_bytes = packed.resident_bytes();
		if resident_bytes > self.byte_budget || resident_bytes / std::mem::size_of::<u32>() > self.storage.len() {
			self.entries.insert(resource_id.clone(), AnimationPoolEntry::Failed);
			self.push_event(AnimationPoolEvent::Oversized {
				resource_id: resource_id.into(),
				resident_bytes,
				byte_budget: self.byte_budget,
			});
			return;
		}
		if !self.make_room(resident_bytes) {
			let blocked_count = self
				.entries
				.values()
				.filter(|entry| matches!(entry, AnimationPoolEntry::Blocked(_)))
				.count();
			if blocked_count < ANIMATION_LOAD_QUEUE_CAPACITY {
				self.entries.insert(resource_id, AnimationPoolEntry::Blocked(packed));
			} else {
				// Drop this completed payload so a later request can retry after capacity frees.
				self.entries.remove(&resource_id);
			}
			return;
		}
		self.write_animation(resource_id, packed);
	}

	/// Copies a packed clip into the contiguous arena range that admission made room for.
	fn write_animation(&mut self, resource_id: Box<str>, packed: PackedAnimationData) {
		let resident_bytes = packed.resident_bytes();
		let region = self
			.free_words
			.take(packed.data.len(), 1)
			.expect("Animation admission reserved one contiguous arena region.");
		self.storage[region.clone()].copy_from_slice(&packed.data);
		self.resident_bytes += resident_bytes;
		let replaced = self.entries.insert(
			resource_id,
			AnimationPoolEntry::Resident(CachedAnimation {
				skeleton: Arc::new(packed.skeleton.into_resource()),
				region,
				last_used: AtomicU64::new(self.step),
			}),
		);
		debug_assert!(
			matches!(
				replaced,
				None | Some(AnimationPoolEntry::Loading { .. }) | Some(AnimationPoolEntry::Blocked(_))
			),
			"Animation admission must not replace an unrelated entry."
		);
	}

	/// Evicts the least recently used entries until one contiguous arena range can hold the requested words.
	///
	/// It takes the pool mutably, so no player is sampling the clips it evicts.
	fn make_room(&mut self, required_bytes: usize) -> bool {
		let required_words = required_bytes.div_ceil(std::mem::size_of::<u32>());
		if required_bytes > self.byte_budget || required_words > self.storage.len() {
			return false;
		}
		while !self.free_words.fits(required_words, 1) {
			let Some(resource_id) = self
				.entries
				.iter()
				.filter_map(|(resource_id, entry)| match entry {
					AnimationPoolEntry::Resident(entry) => Some((resource_id, entry.last_used.load(Ordering::Relaxed))),
					_ => None,
				})
				.min_by_key(|(_, last_used)| *last_used)
				.map(|(resource_id, _)| resource_id.clone())
			else {
				return false;
			};
			let Some(AnimationPoolEntry::Resident(evicted)) = self.entries.remove(&resource_id) else {
				unreachable!("The selected eviction candidate must remain resident.");
			};
			self.resident_bytes = self
				.resident_bytes
				.saturating_sub(evicted.region.len() * std::mem::size_of::<u32>());
			self.free_words.give_back(evicted.region);
			self.push_event(AnimationPoolEvent::Evicted {
				resource_id: resource_id.into(),
			});
		}
		true
	}
}

/// The `AnimationLoadWorker` struct loads and packs animation resources away from synchronous pose evaluation.
pub struct AnimationLoadWorker {
	resource_manager: EntityHandle<ResourceManager>,
	commands: kanal::AsyncReceiver<String>,
	completions: kanal::AsyncSender<AnimationLoadCompletion>,
}

impl AnimationLoadWorker {
	/// Loads queued clips until the animation pool drops its command channel.
	pub async fn run(self) {
		while let Ok(resource_id) = self.commands.recv().await {
			// Animation resources keep decoded curves in metadata, so the
			// reference reader is intentionally released before pooling. Packing
			// here keeps the per-key work off the thread that calls `AnimationPool::update`.
			let completion = self
				.resource_manager
				.request::<Animation>(&resource_id)
				.await
				.map(|reference| PackedAnimationData::from_resource(reference.into_resource()));
			if self.completions.send((resource_id, completion)).await.is_err() {
				break;
			}
			async_runtime::yield_now().await;
		}
	}
}

mod binding;
mod player;

pub(super) use binding::GraphBinding;
use binding::ClipBinding;

#[doc(hidden)]
pub mod benchmarks;

pub use player::{
	AnimationGraphPlayer, AnimationGraphPlayerError, AnimationGraphPose, RootMotionRotation, RootMotionSettings,
	RootMotionTranslation,
};

#[cfg(test)]
mod tests {

	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, TranslationCurve},
			skeleton::{LocalTransform, Skeleton},
		},
	};

	use super::*;
	use crate::animation::test_node;

	fn test_skeleton() -> Skeleton {
		Skeleton {
			nodes: vec![test_node(Some("root"), None, LocalTransform::identity())],
		}
	}

	/// The frame rate test clips are authored at: a frame every 0.1 s.
	pub(super) const TEST_FRAME_RATE: f32 = 10.0;

	/// Packs a one-node clip that moves its root along x. The [`player`] tests share it and its packed size.
	pub(super) fn test_animation(name: &str, end_translation: f32) -> PackedAnimationData {
		test_motion(name, [end_translation, 0.0, 0.0], 1.0)
	}

	/// Packs a one-node clip that moves its root from the origin to `end` over `duration` seconds, authored at
	/// [`TEST_FRAME_RATE`] frames per second.
	pub(super) fn test_motion(name: &str, end: [f32; 3], duration: f32) -> PackedAnimationData {
		test_motion_from(name, [0.0; 3], end, duration)
	}

	/// Packs a one-node clip that moves its root from `start` to `end` over `duration` seconds, authored at
	/// [`TEST_FRAME_RATE`] frames per second.
	pub(super) fn test_motion_from(name: &str, start: [f32; 3], end: [f32; 3], duration: f32) -> PackedAnimationData {
		PackedAnimationData::from_resource(Animation {
			name: Some(name.into()),
			skeleton: Reference::in_memory("test.skeleton", test_skeleton()),
			duration,
			frame_rate: Some(TEST_FRAME_RATE),
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(TranslationCurve::Linear {
					times: vec![0.0, duration],
					values: vec![
						math::Vector::new(start[0], start[1], start[2]),
						math::Vector::new(end[0], end[1], end[2]),
					],
				}),
				rotation: None,
				scale: None,
			}],
		})
	}

	/// Measures the representation retained by the pool rather than the transient resource representation.
	pub(super) fn packed_test_animation_bytes(name: &str, end_translation: f32) -> usize {
		test_animation(name, end_translation).resident_bytes()
	}

	#[test]
	fn pool_evicts_the_least_recently_sampled_clip_to_admit_another() {
		let budget = packed_test_animation_bytes("idle", 1.0) * 2;
		let mut pool = AnimationPool::detached(budget);
		pool.admit("idle.animation".into(), test_animation("idle", 1.0));
		pool.admit("walk.animation".into(), test_animation("walk", 2.0));

		// The next step samples the walk only, so the idle becomes the least recently used clip.
		pool.update();
		assert!(pool.resident("walk.animation").is_some());
		pool.admit("run.animation".into(), test_animation("run", 3.0));

		assert_eq!(pool.request("walk.animation"), AnimationPoolRequest::Ready);
		assert_eq!(pool.request("run.animation"), AnimationPoolRequest::Ready);
		assert!(
			pool.drain_events()
				.any(|event| matches!(event, AnimationPoolEvent::Evicted { resource_id } if resource_id == "idle.animation"))
		);
		assert_eq!(pool.request("idle.animation"), AnimationPoolRequest::Loading);
	}

	#[test]
	fn oversized_clips_fail_once_until_the_caller_explicitly_retries_them() {
		let animation = test_animation("oversized", 1.0);
		let mut pool = AnimationPool::detached(packed_test_animation_bytes("oversized", 1.0) - 1);
		pool.admit("oversized.animation".into(), animation);

		assert!(matches!(pool.request("oversized.animation"), AnimationPoolRequest::Failed));
		assert!(pool.drain_events().any(|event| matches!(
			event,
			AnimationPoolEvent::Oversized {
				resource_id,
				..
			} if resource_id == "oversized.animation"
		)));
	}
}
