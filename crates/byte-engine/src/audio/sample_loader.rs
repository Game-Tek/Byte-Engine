use std::{mem::size_of, num::NonZeroUsize, ptr::NonNull, sync::Arc};

use crossbeam_queue::ArrayQueue;
use resource_management::{
	resource::{resource_manager::ResourceManager, ReadTargetsMut},
	resources::audio::Audio,
	types::BitDepths,
	Reference,
};

use super::graph::{AudioGraphRenderPlan, CompiledAudioGraph, PreparedAudioGraphRenderPlan};
use crate::{
	core::async_runtime,
	core::{factory::Handle, EntityHandle},
};

/// Keep both sides bounded so the application loader and audio worker exert
/// backpressure instead of growing queues during a resource burst.
pub(crate) const AUDIO_GRAPH_CAPACITY: usize = 64;

/// Holds every active graph lease plus a full completion queue of stale leases.
pub(crate) const AUDIO_SAMPLE_RELEASE_CAPACITY: usize = AUDIO_GRAPH_CAPACITY * 2;

/// Covers every possible free interval plus one temporary returned region before coalescing.
const AUDIO_SAMPLE_FREE_REGION_CAPACITY: usize = AUDIO_GRAPH_CAPACITY + 2;

/// The decoded PCM byte budget used by the default audio setup.
pub const DEFAULT_AUDIO_SAMPLE_POOL_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// The `AudioSamplePoolConfig` struct sets the decoded PCM memory available to
/// the global audio sample pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSamplePoolConfig {
	byte_budget: NonZeroUsize,
}

impl AudioSamplePoolConfig {
	/// Creates a strict byte budget for decoded audio samples.
	pub const fn new(byte_budget: NonZeroUsize) -> Self {
		Self { byte_budget }
	}

	/// Returns the maximum decoded PCM bytes available to the sample pool.
	pub const fn byte_budget(self) -> usize {
		self.byte_budget.get()
	}
}

impl Default for AudioSamplePoolConfig {
	fn default() -> Self {
		Self::new(
			NonZeroUsize::new(DEFAULT_AUDIO_SAMPLE_POOL_BYTE_BUDGET)
				.expect("The default audio sample pool budget is non-zero."),
		)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// The `AudioSampleCacheKey` struct distinguishes PCM payload and playback
/// metadata versions for one resource ID.
struct AudioSampleCacheKey {
	resource_id: String,
	payload_hash: u64,
	bit_depth: u8,
	channel_count: u16,
	sample_rate: u32,
	frame_count: u32,
}

impl AudioSampleCacheKey {
	fn new(resource_id: &str, payload_hash: u64, metadata: Audio) -> Self {
		let bit_depth = match metadata.bit_depth {
			BitDepths::Eight => 8,
			BitDepths::Sixteen => 16,
			BitDepths::TwentyFour => 24,
			BitDepths::ThirtyTwo => 32,
		};
		Self {
			resource_id: resource_id.to_string(),
			payload_hash,
			bit_depth,
			channel_count: metadata.channel_count,
			sample_rate: metadata.sample_rate,
			frame_count: metadata.sample_count,
		}
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The `AudioSampleLayout` struct retains the validated playback metadata for
/// one PCM arena region.
struct AudioSampleLayout {
	channel_count: u16,
	sample_rate: u32,
	frame_count: usize,
	scalar_count: usize,
}

impl AudioSampleLayout {
	/// Validates resource playback metadata before the pool reserves storage.
	fn new(metadata: Audio) -> Result<Self, String> {
		if metadata.channel_count != 1 && metadata.channel_count != 2 {
			return Err("Unsupported audio sample channel count. The resource must contain mono or stereo PCM.".to_string());
		}
		if metadata.sample_rate == 0 {
			return Err("Invalid audio sample rate. The resource metadata reports zero hertz.".to_string());
		}
		if metadata.sample_count == 0 {
			return Err("Invalid audio sample length. The resource metadata reports zero frames.".to_string());
		}

		let frame_count = usize::try_from(metadata.sample_count)
			.map_err(|_| "Invalid audio sample length. The frame count does not fit this platform.".to_string())?;
		let scalar_count = frame_count
			.checked_mul(usize::from(metadata.channel_count))
			.ok_or_else(|| "Invalid audio sample layout. The channel sample count overflowed.".to_string())?;
		Ok(Self {
			channel_count: metadata.channel_count,
			sample_rate: metadata.sample_rate,
			frame_count,
			scalar_count,
		})
	}

	/// Returns the exact normalized PCM allocation required by this metadata.
	fn decoded_byte_count(self) -> Result<usize, String> {
		self.scalar_count
			.checked_mul(size_of::<f32>())
			.ok_or_else(|| "Invalid audio sample layout. The decoded PCM byte count overflowed.".to_string())
	}
}

/// Decodes one resource directly into its reserved PCM arena region.
fn decode_into(metadata: Audio, bytes: &[u8], samples: &mut [f32]) -> Result<AudioSampleLayout, String> {
	let layout = AudioSampleLayout::new(metadata)?;
	if samples.len() != layout.scalar_count {
		return Err("Invalid audio sample reservation. The reserved PCM region has the wrong scalar count.".to_string());
	}
	let bytes_per_sample = match metadata.bit_depth {
		BitDepths::Eight => 1,
		BitDepths::Sixteen => 2,
		BitDepths::TwentyFour => 3,
		BitDepths::ThirtyTwo => 4,
	};
	let expected_byte_count = layout
		.scalar_count
		.checked_mul(bytes_per_sample)
		.ok_or_else(|| "Invalid audio sample layout. The PCM byte count overflowed.".to_string())?;

	if bytes.len() != expected_byte_count {
		return Err(format!(
			"Invalid audio sample payload. The resource contains {} bytes but its metadata requires {expected_byte_count}.",
			bytes.len()
		));
	}

	match metadata.bit_depth {
		// WAV PCM and the engine OGG baker both store 8-bit PCM as unsigned.
		BitDepths::Eight => {
			for (destination, byte) in samples.iter_mut().zip(bytes) {
				*destination = (*byte as f32 - 128.0) / 128.0;
			}
		}
		BitDepths::Sixteen => {
			for (destination, sample) in samples.iter_mut().zip(bytes.chunks_exact(2)) {
				*destination = i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32_768.0;
			}
		}
		BitDepths::TwentyFour => {
			for (destination, sample) in samples.iter_mut().zip(bytes.chunks_exact(3)) {
				let sign = if sample[2] & 0x80 == 0 { 0 } else { 0xff };
				*destination = i32::from_le_bytes([sample[0], sample[1], sample[2], sign]) as f32 / 8_388_608.0;
			}
		}
		BitDepths::ThirtyTwo => {
			for (destination, sample) in samples.iter_mut().zip(bytes.chunks_exact(4)) {
				*destination = i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32 / 2_147_483_648.0;
			}
		}
	}

	Ok(layout)
}

/// The `AudioSampleLeaseId` struct identifies one generation of a stable sample
/// pool slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AudioSampleLeaseId {
	slot: u8,
	generation: u64,
}

/// The `AudioSampleLease` struct provides lock-free read access to one stable
/// pool-owned sample allocation.
///
/// Return [`Self::into_id`] through the audio sample release queue after the graph
/// stops using this lease. The loader will then make the slot eligible for
/// eviction.
#[derive(Debug)]
pub(crate) struct AudioSampleLease {
	id: AudioSampleLeaseId,
	samples: NonNull<f32>,
	layout: AudioSampleLayout,
	#[cfg(test)]
	owned_samples: Option<Box<[f32]>>,
}

impl AudioSampleLease {
	fn new(id: AudioSampleLeaseId, samples: &[f32], layout: AudioSampleLayout) -> Self {
		debug_assert_eq!(samples.len(), layout.scalar_count);
		Self {
			id,
			samples: NonNull::new(samples.as_ptr().cast_mut()).expect("Validated audio samples are not empty."),
			layout,
			#[cfg(test)]
			owned_samples: None,
		}
	}

	pub(crate) fn into_id(self) -> AudioSampleLeaseId {
		self.id
	}

	#[cfg(test)]
	fn id(&self) -> AudioSampleLeaseId {
		self.id
	}

	#[cfg(test)]
	pub(crate) fn for_test(sample_rate: u32, channel_count: u16, samples: Box<[f32]>) -> Self {
		assert!(sample_rate > 0);
		assert!(channel_count == 1 || channel_count == 2);
		assert!(!samples.is_empty());
		assert_eq!(samples.len() % usize::from(channel_count), 0);
		let layout = AudioSampleLayout {
			channel_count,
			sample_rate,
			frame_count: samples.len() / usize::from(channel_count),
			scalar_count: samples.len(),
		};
		Self {
			id: AudioSampleLeaseId {
				slot: u8::MAX,
				generation: 0,
			},
			samples: NonNull::new(samples.as_ptr().cast_mut()).expect("Test audio samples are not empty."),
			layout,
			owned_samples: Some(samples),
		}
	}

	/// Borrows fixture-owned PCM for the external runtime benchmark.
	///
	/// The benchmark state must retain `samples` until this lease is dropped.
	pub(crate) fn for_benchmark(sample_rate: u32, channel_count: u16, samples: &[f32]) -> Self {
		assert!(sample_rate > 0);
		assert!(channel_count == 1 || channel_count == 2);
		assert!(!samples.is_empty());
		assert_eq!(samples.len() % usize::from(channel_count), 0);
		let layout = AudioSampleLayout {
			channel_count,
			sample_rate,
			frame_count: samples.len() / usize::from(channel_count),
			scalar_count: samples.len(),
		};
		Self::new(
			AudioSampleLeaseId {
				slot: u8::MAX,
				generation: 0,
			},
			samples,
			layout,
		)
	}

	pub(crate) const fn sample_rate(&self) -> u32 {
		self.layout.sample_rate
	}

	pub(crate) const fn frame_count(&self) -> usize {
		self.layout.frame_count
	}

	pub(crate) const fn channel_count(&self) -> u16 {
		self.layout.channel_count
	}

	/// Returns the complete immutable interleaved PCM region retained by this
	/// lease.
	#[allow(unsafe_code)]
	pub(crate) fn samples(&self) -> &[f32] {
		// The pool keeps this region stable until the audio worker returns the
		// lease ID. Test leases retain the same allocation in `owned_samples`.
		unsafe { std::slice::from_raw_parts(self.samples.as_ptr(), self.layout.scalar_count) }
	}

	/// Returns one mono frame from the stable arena region.
	#[allow(unsafe_code)]
	pub(crate) fn mono_frame(&self, frame: usize) -> f32 {
		let index = frame * usize::from(self.layout.channel_count);
		// The pool retains this complete region until the lease ID returns.
		let current = unsafe { *self.samples.as_ptr().add(index) };
		if self.layout.channel_count == 1 {
			current
		} else {
			let right = unsafe { *self.samples.as_ptr().add(index + 1) };
			(current + right) * 0.5
		}
	}
}

#[allow(unsafe_code)]
// The immutable allocation remains pool-owned until the audio thread returns
// this lease ID. Moving the pointer between the loader and audio thread is safe.
unsafe impl Send for AudioSampleLease {}

/// The `AudioSampleReleaseQueue` struct returns lease IDs from one audio thread
/// to one loader thread without locks or allocation.
struct AudioSampleReleaseQueue {
	queue: ArrayQueue<AudioSampleLeaseId>,
}

impl AudioSampleReleaseQueue {
	fn new() -> Self {
		Self {
			queue: ArrayQueue::new(AUDIO_SAMPLE_RELEASE_CAPACITY),
		}
	}

	/// Pushes one ID from the audio thread without waiting or contending on a lock.
	fn push(&self, id: AudioSampleLeaseId) -> bool {
		self.queue.push(id).is_ok()
	}

	/// Pops one returned ID on the loader thread.
	fn pop(&self) -> Option<AudioSampleLeaseId> {
		self.queue.pop()
	}
}

/// The `AudioSampleRegion` struct identifies one contiguous scalar range in the
/// preallocated PCM arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AudioSampleRegion {
	offset: usize,
	scalar_count: usize,
}

impl AudioSampleRegion {
	fn end(self) -> usize {
		self.offset + self.scalar_count
	}
}

/// The `CachedAudioSample` struct retains one arena range and its loader-owned
/// residency state.
struct CachedAudioSample {
	key: AudioSampleCacheKey,
	region: AudioSampleRegion,
	layout: AudioSampleLayout,
	resident_bytes: usize,
	last_used: u64,
	lease_count: usize,
}

/// The `AudioSampleSlot` struct preserves generation identity while cache
/// entries are evicted and replaced.
struct AudioSampleSlot {
	generation: u64,
	entry: Option<CachedAudioSample>,
}

/// The `AudioSamplePool` struct owns the global preallocated PCM arena and its
/// byte-bounded LRU cache.
struct AudioSamplePool {
	storage: Box<[f32]>,
	free_regions: Vec<AudioSampleRegion>,
	slots: [AudioSampleSlot; AUDIO_GRAPH_CAPACITY],
	byte_budget: usize,
	resident_bytes: usize,
	next_use: u64,
}

impl AudioSamplePool {
	fn new(config: AudioSamplePoolConfig) -> Self {
		let scalar_capacity = config.byte_budget() / size_of::<f32>();
		let mut free_regions = Vec::with_capacity(AUDIO_SAMPLE_FREE_REGION_CAPACITY);
		if scalar_capacity > 0 {
			free_regions.push(AudioSampleRegion {
				offset: 0,
				scalar_count: scalar_capacity,
			});
		}
		Self {
			storage: vec![0.0; scalar_capacity].into_boxed_slice(),
			free_regions,
			slots: std::array::from_fn(|_| AudioSampleSlot {
				generation: 0,
				entry: None,
			}),
			byte_budget: config.byte_budget(),
			resident_bytes: 0,
			next_use: 0,
		}
	}

	/// Returns a lease for an already resident sample and refreshes its LRU age.
	fn lease(&mut self, key: &AudioSampleCacheKey) -> Option<AudioSampleLease> {
		let slot_index = self
			.slots
			.iter()
			.position(|slot| slot.entry.as_ref().is_some_and(|entry| &entry.key == key))?;
		let last_used = self.next_use();
		let (slots, storage) = (&mut self.slots, &self.storage);
		let slot = &mut slots[slot_index];
		let entry = slot.entry.as_mut().expect("The matching audio sample slot is occupied.");
		entry.last_used = last_used;
		entry.lease_count += 1;
		let samples = &storage[entry.region.offset..entry.region.end()];
		Some(AudioSampleLease::new(
			AudioSampleLeaseId {
				slot: u8::try_from(slot_index).expect("Audio sample slot indices fit in u8."),
				generation: slot.generation,
			},
			samples,
			entry.layout,
		))
	}

	/// Applies every returned lease before checking cache residency or capacity.
	fn release_returned(&mut self, releases: &AudioSampleReleaseQueue) {
		while let Some(id) = releases.pop() {
			let Some(slot) = self.slots.get_mut(usize::from(id.slot)) else {
				continue;
			};
			if slot.generation != id.generation {
				continue;
			}
			let Some(entry) = &mut slot.entry else {
				continue;
			};
			entry.lease_count = entry
				.lease_count
				.checked_sub(1)
				.expect("Returned audio sample lease must belong to an active slot generation.");
		}
	}

	/// Evicts inactive LRU samples before a decode allocation is created.
	fn make_room(&mut self, required_bytes: usize) -> bool {
		let required_scalars = required_bytes / size_of::<f32>();
		if required_bytes > self.byte_budget || required_scalars > self.storage.len() {
			return false;
		}

		while !self.free_regions.iter().any(|region| region.scalar_count >= required_scalars)
			|| self.slots.iter().all(|slot| slot.entry.is_some())
		{
			let Some((slot_index, _)) = self
				.slots
				.iter()
				.enumerate()
				.filter_map(|(index, slot)| slot.entry.as_ref().map(|entry| (index, entry)))
				.filter(|(_, entry)| entry.lease_count == 0)
				.min_by_key(|(_, entry)| entry.last_used)
			else {
				return false;
			};
			let evicted = self.slots[slot_index]
				.entry
				.take()
				.expect("The selected audio sample slot is occupied.");
			self.resident_bytes = self.resident_bytes.saturating_sub(evicted.resident_bytes);
			self.return_region(evicted.region);
		}
		true
	}

	/// Decodes a resource into one free range and adopts it into a vacant slot.
	fn decode_and_insert(
		&mut self,
		key: AudioSampleCacheKey,
		metadata: Audio,
		layout: AudioSampleLayout,
		bytes: &[u8],
	) -> Result<AudioSampleLease, String> {
		let region = self
			.take_region(layout.scalar_count)
			.expect("Audio sample admission reserved one contiguous PCM region.");
		let decoded = decode_into(metadata, bytes, &mut self.storage[region.offset..region.end()]);
		let decoded_layout = match decoded {
			Ok(decoded_layout) => decoded_layout,
			Err(error) => {
				self.return_region(region);
				return Err(error);
			}
		};
		debug_assert_eq!(decoded_layout, layout);
		Ok(self.insert(key, layout, region))
	}

	/// Adopts a decoded arena region after [`Self::make_room`] reserved capacity.
	fn insert(&mut self, key: AudioSampleCacheKey, layout: AudioSampleLayout, region: AudioSampleRegion) -> AudioSampleLease {
		let resident_bytes = layout
			.decoded_byte_count()
			.expect("Validated audio sample byte count must fit this platform.");
		assert!(
			self.slots.iter().any(|slot| slot.entry.is_none())
				&& region.end() <= self.storage.len()
				&& region.scalar_count == layout.scalar_count,
			"Audio sample pool admission requires available entry and byte capacity."
		);
		let last_used = self.next_use();
		let slot_index = self
			.slots
			.iter()
			.position(|slot| slot.entry.is_none())
			.expect("Audio sample admission reserved one vacant slot.");
		let slot = &mut self.slots[slot_index];
		slot.generation = slot.generation.wrapping_add(1);
		let entry = slot.entry.insert(CachedAudioSample {
			key,
			region,
			layout,
			resident_bytes,
			last_used,
			lease_count: 1,
		});
		self.resident_bytes += resident_bytes;
		let samples = &self.storage[entry.region.offset..entry.region.end()];
		AudioSampleLease::new(
			AudioSampleLeaseId {
				slot: u8::try_from(slot_index).expect("Audio sample slot indices fit in u8."),
				generation: slot.generation,
			},
			samples,
			entry.layout,
		)
	}

	/// Removes one region from the free list and splits any unused suffix.
	fn take_region(&mut self, scalar_count: usize) -> Option<AudioSampleRegion> {
		let index = self
			.free_regions
			.iter()
			.position(|region| region.scalar_count >= scalar_count)?;
		let available = self.free_regions[index];
		let region = AudioSampleRegion {
			offset: available.offset,
			scalar_count,
		};
		if available.scalar_count == scalar_count {
			self.free_regions.swap_remove(index);
		} else {
			self.free_regions[index].offset += scalar_count;
			self.free_regions[index].scalar_count -= scalar_count;
		}
		Some(region)
	}

	/// Returns and coalesces one range without allocating free-list storage.
	fn return_region(&mut self, region: AudioSampleRegion) {
		debug_assert!(self.free_regions.len() < AUDIO_SAMPLE_FREE_REGION_CAPACITY);
		self.free_regions.push(region);
		self.free_regions.sort_unstable_by_key(|region| region.offset);
		let mut index = 1;
		while index < self.free_regions.len() {
			if self.free_regions[index - 1].end() == self.free_regions[index].offset {
				let scalar_count = self.free_regions[index].scalar_count;
				self.free_regions[index - 1].scalar_count += scalar_count;
				self.free_regions.remove(index);
			} else {
				index += 1;
			}
		}
	}

	fn next_use(&mut self) -> u64 {
		let use_stamp = self.next_use;
		self.next_use = self.next_use.wrapping_add(1);
		use_stamp
	}
}

#[derive(Debug)]
/// The `AudioLoadRequest` struct identifies one version of a graph resource
/// request sent from the audio worker.
struct AudioLoadRequest {
	handle: Handle,
	generation: u64,
	resource_id: String,
	render_plan: AudioGraphRenderPlan,
}

/// Carries a completed load back to the audio worker without exposing the
/// borrowed resource backing used during conversion.
enum AudioLoadCompletion {
	Ready {
		handle: Handle,
		generation: u64,
		sample: AudioSampleLease,
		render_plan: PreparedAudioGraphRenderPlan,
	},
	WaitingForCapacity {
		request: AudioLoadRequest,
	},
	Failed {
		handle: Handle,
		generation: u64,
	},
}

/// The `PendingAudioGraph` struct retains a render plan and an unsent request
/// while the real-time channel is busy.
struct PendingAudioGraph {
	handle: Handle,
	generation: u64,
	request: Option<AudioLoadRequest>,
	waiting_for_capacity: bool,
	submitted_release_epoch: u64,
}

/// The `AudioSampleLoaderClient` struct bridges world lifecycle messages to
/// the async loader without waiting for Kanal lock or queue capacity.
pub(crate) struct AudioSampleLoaderClient {
	commands: kanal::Sender<AudioLoadRequest>,
	completions: kanal::Receiver<AudioLoadCompletion>,
	releases: Arc<AudioSampleReleaseQueue>,
	pending_releases: Vec<AudioSampleLeaseId>,
	pending: Vec<PendingAudioGraph>,
	next_generation: u64,
	lease_release_epoch: u64,
	commands_closed: bool,
	completions_closed: bool,
}

impl AudioSampleLoaderClient {
	fn new(
		commands: kanal::Sender<AudioLoadRequest>,
		completions: kanal::Receiver<AudioLoadCompletion>,
		releases: Arc<AudioSampleReleaseQueue>,
	) -> Self {
		Self {
			commands,
			completions,
			releases,
			pending_releases: Vec::with_capacity(AUDIO_SAMPLE_RELEASE_CAPACITY),
			pending: Vec::with_capacity(AUDIO_GRAPH_CAPACITY),
			next_generation: 0,
			lease_release_epoch: 0,
			commands_closed: false,
			completions_closed: false,
		}
	}

	/// Queues one graph without allocating new audio-thread container storage.
	///
	/// `active_graph_count` lets this bridge enforce one shared limit across
	/// graphs that are loading and graphs that are already mixing.
	pub(crate) fn queue(&mut self, handle: Handle, graph: CompiledAudioGraph, active_graph_count: usize) -> bool {
		self.remove(handle);
		if self.pending.len() + active_graph_count >= AUDIO_GRAPH_CAPACITY {
			log::warn!(
				"Audio graph was not created. The audio worker already has the maximum of {} active or loading graphs.",
				AUDIO_GRAPH_CAPACITY
			);
			return false;
		}

		let generation = self.next_generation;
		self.next_generation = self.next_generation.wrapping_add(1);
		let (resource_id, render_plan) = graph.into_parts();

		self.pending.push(PendingAudioGraph {
			handle,
			generation,
			waiting_for_capacity: false,
			submitted_release_epoch: self.lease_release_epoch,
			request: Some(AudioLoadRequest {
				handle,
				generation,
				resource_id,
				render_plan,
			}),
		});
		true
	}

	/// Removes a pending graph. A completion already in flight is rejected by
	/// its handle and generation when it arrives.
	pub(crate) fn remove(&mut self, handle: Handle) {
		if let Some(index) = self.pending.iter().position(|graph| graph.handle == handle) {
			self.pending.swap_remove(index);
		}
	}

	/// Returns one lease ID without waiting for the loader thread.
	pub(crate) fn return_lease(&mut self, id: AudioSampleLeaseId) -> bool {
		if !self.releases.push(id) {
			return false;
		}
		self.mark_lease_released();
		true
	}

	fn mark_lease_released(&mut self) {
		self.lease_release_epoch = self.lease_release_epoch.wrapping_add(1);
		for graph in &mut self.pending {
			graph.waiting_for_capacity = false;
		}
	}

	/// Retains a stale completion release if the return ring is temporarily full.
	fn return_or_defer_lease(&mut self, id: AudioSampleLeaseId) {
		if self.return_lease(id) {
			return;
		}
		if self.pending_releases.len() < AUDIO_SAMPLE_RELEASE_CAPACITY {
			self.pending_releases.push(id);
		} else {
			log::error!(
				"Audio sample lease could not be returned. The most likely cause is duplicate lease release traffic exceeding the bounded return path."
			);
		}
	}

	fn flush_pending_releases(&mut self) {
		while let Some(id) = self.pending_releases.last().copied() {
			if !self.releases.push(id) {
				break;
			}
			self.pending_releases.pop();
			self.mark_lease_released();
		}
	}

	/// Submits waiting requests and adopts ready samples at a hardware-period
	/// boundary. The callback runs only for a still-live request generation.
	pub(crate) fn update(&mut self, mut create_graph: impl FnMut(Handle, AudioSampleLease, PreparedAudioGraphRenderPlan)) {
		self.flush_pending_releases();
		self.submit_requests();

		if self.completions_closed {
			return;
		}
		loop {
			match self.completions.try_recv_realtime() {
				Ok(Some(completion)) => self.process_completion(completion, &mut create_graph),
				Ok(None) => break,
				Err(_) => {
					self.completions_closed = true;
					break;
				}
			}
		}
	}

	fn submit_requests(&mut self) {
		if self.commands_closed {
			return;
		}

		for graph in &mut self.pending {
			if graph.waiting_for_capacity || graph.request.is_none() {
				continue;
			}
			graph.submitted_release_epoch = self.lease_release_epoch;
			// Kanal does not wait for its channel mutex or queue capacity here.
			// A successful send can still wake the application executor, so this
			// is a soft real-time boundary rather than a hard real-time guarantee.
			match self.commands.try_send_option_realtime(&mut graph.request) {
				Ok(true) | Ok(false) => {}
				Err(_) => {
					self.commands_closed = true;
					break;
				}
			}
		}
	}

	fn process_completion(
		&mut self,
		completion: AudioLoadCompletion,
		create_graph: &mut impl FnMut(Handle, AudioSampleLease, PreparedAudioGraphRenderPlan),
	) {
		let (handle, generation) = match &completion {
			AudioLoadCompletion::Ready { handle, generation, .. } | AudioLoadCompletion::Failed { handle, generation } => {
				(*handle, *generation)
			}
			AudioLoadCompletion::WaitingForCapacity { request } => (request.handle, request.generation),
		};
		let Some(index) = self
			.pending
			.iter()
			.position(|graph| graph.handle == handle && graph.generation == generation)
		else {
			if let AudioLoadCompletion::Ready { sample, .. } = completion {
				self.return_or_defer_lease(sample.into_id());
			}
			return;
		};

		match completion {
			AudioLoadCompletion::WaitingForCapacity { request } => {
				let graph = &mut self.pending[index];
				graph.request = Some(request);
				// A lease can be released after the loader checks capacity but before
				// this completion arrives. Retry immediately when that race occurred.
				graph.waiting_for_capacity = graph.submitted_release_epoch == self.lease_release_epoch;
			}
			AudioLoadCompletion::Ready { sample, render_plan, .. } => {
				self.pending.swap_remove(index);
				create_graph(handle, sample, render_plan);
			}
			AudioLoadCompletion::Failed { .. } => {
				self.pending.swap_remove(index);
			}
		}
	}
}

/// The `AudioSampleLoader` struct owns the global sample pool and converts baked
/// PCM into leases that can safely cross to the audio worker.
pub(crate) struct AudioSampleLoader {
	resource_manager: EntityHandle<ResourceManager>,
	commands: kanal::AsyncReceiver<AudioLoadRequest>,
	completions: kanal::AsyncSender<AudioLoadCompletion>,
	releases: Arc<AudioSampleReleaseQueue>,
	pool: AudioSamplePool,
}

enum AudioSamplePoolLoad {
	Ready(AudioSampleLease),
	WaitingForCapacity,
}

impl AudioSampleLoader {
	/// Creates the bounded real-time client and its application-runtime worker.
	pub(crate) fn new(
		resource_manager: EntityHandle<ResourceManager>,
		pool_config: AudioSamplePoolConfig,
	) -> (AudioSampleLoaderClient, Self) {
		let (commands, command_receiver) = kanal::bounded_async(AUDIO_GRAPH_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(AUDIO_GRAPH_CAPACITY);
		let releases = Arc::new(AudioSampleReleaseQueue::new());

		(
			AudioSampleLoaderClient::new(commands.to_sync(), completions.to_sync(), Arc::clone(&releases)),
			Self {
				resource_manager,
				commands: command_receiver,
				completions: completion_sender,
				releases,
				pool: AudioSamplePool::new(pool_config),
			},
		)
	}

	/// Handles resource requests until the audio worker closes its channel.
	pub(crate) async fn run(mut self) {
		while let Ok(request) = self.commands.recv().await {
			let completion = match self.load(&request.resource_id).await {
				Ok(AudioSamplePoolLoad::Ready(sample)) => AudioLoadCompletion::Ready {
					handle: request.handle,
					generation: request.generation,
					sample,
					render_plan: request.render_plan.prepare(),
				},
				Ok(AudioSamplePoolLoad::WaitingForCapacity) => AudioLoadCompletion::WaitingForCapacity { request },
				Err(error) => {
					log::error!(
						"Failed to load audio sample '{}'. The resource could not be prepared for playback: {}",
						request.resource_id,
						error
					);
					AudioLoadCompletion::Failed {
						handle: request.handle,
						generation: request.generation,
					}
				}
			};

			if self.completions.send(completion).await.is_err() {
				break;
			}
			async_runtime::yield_now().await;
		}
	}

	/// Loads and converts one resource while its borrowed backing remains local
	/// to this async task.
	async fn load(&mut self, resource_id: &str) -> Result<AudioSamplePoolLoad, String> {
		let mut reference: Reference<Audio> = self
			.resource_manager
			.request(resource_id)
			.await
			.map_err(|error| format!("Resource request failed. The resource manager reported: {error}"))?;
		let metadata = *reference.resource();
		let cache_key = AudioSampleCacheKey::new(resource_id, reference.hash(), metadata);
		self.pool.release_returned(&self.releases);

		if let Some(sample) = self.pool.lease(&cache_key) {
			return Ok(AudioSamplePoolLoad::Ready(sample));
		}
		let layout = AudioSampleLayout::new(metadata)?;
		let resident_bytes = layout.decoded_byte_count()?;
		if resident_bytes > self.pool.byte_budget {
			return Err(format!(
				"Audio sample exceeds the pool budget. The decoded sample requires {resident_bytes} bytes but the pool allows {} bytes.",
				self.pool.byte_budget
			));
		}
		if !self.pool.make_room(resident_bytes) {
			return Ok(AudioSamplePoolLoad::WaitingForCapacity);
		}

		let loaded = reference
			.load(ReadTargetsMut::backing_storage())
			.await
			.map_err(|error| format!("PCM read failed. The resource reader reported: {error:?}"))?;
		let bytes = loaded
			.buffer()
			.ok_or_else(|| "PCM read failed. The resource reader returned non-contiguous data.".to_string())?;
		let sample = self.pool.decode_and_insert(cache_key, metadata, layout, bytes)?;
		Ok(AudioSamplePoolLoad::Ready(sample))
	}
}

#[cfg(test)]
mod tests {
	use std::{mem::size_of, num::NonZeroUsize, sync::Arc};

	use resource_management::{resources::audio::Audio, types::BitDepths};

	use super::{
		decode_into, AudioLoadCompletion, AudioSampleCacheKey, AudioSampleLayout, AudioSampleLease, AudioSampleLeaseId,
		AudioSampleLoaderClient, AudioSamplePool, AudioSamplePoolConfig, AudioSampleReleaseQueue, AUDIO_GRAPH_CAPACITY,
		AUDIO_SAMPLE_RELEASE_CAPACITY,
	};
	use crate::{
		audio::graph::{
			fns::{gain, r#loop, sample},
			AudioGraphRenderPlan, AudioProcessor, PlaybackRate, PreparedAudioGraphRenderPlan, SamplePlaybackMode,
		},
		core::{factory::Factory, listener::Listener},
	};

	fn prepared_plan(
		playback_mode: SamplePlaybackMode,
		processors: impl IntoIterator<Item = AudioProcessor>,
	) -> PreparedAudioGraphRenderPlan {
		AudioGraphRenderPlan {
			playback_mode,
			playback_rate: PlaybackRate::UNITY,
			processors: processors.into_iter().collect(),
			muted: false,
			muted_drain_latency: 0,
		}
		.prepare()
	}

	fn metadata(bit_depth: BitDepths, channel_count: u16, sample_count: u32) -> Audio {
		Audio {
			bit_depth,
			channel_count,
			sample_rate: 48_000,
			sample_count,
		}
	}

	fn lease(samples: impl Into<Box<[f32]>>) -> AudioSampleLease {
		AudioSampleLease::for_test(48_000, 1, samples.into())
	}

	fn pool(byte_budget: usize) -> AudioSamplePool {
		AudioSamplePool::new(AudioSamplePoolConfig::new(
			NonZeroUsize::new(byte_budget).expect("test pool budget must be non-zero"),
		))
	}

	fn cache_key(resource_id: &str, payload_hash: u64, sample_count: u32) -> AudioSampleCacheKey {
		AudioSampleCacheKey::new(resource_id, payload_hash, metadata(BitDepths::Sixteen, 1, sample_count))
	}

	fn decode(metadata: Audio, bytes: &[u8]) -> Result<Box<[f32]>, String> {
		let layout = AudioSampleLayout::new(metadata)?;
		let mut samples = vec![0.0; layout.scalar_count].into_boxed_slice();
		decode_into(metadata, bytes, &mut samples)?;
		Ok(samples)
	}

	fn insert_normalized(
		pool: &mut AudioSamplePool,
		resource_id: &str,
		payload_hash: u64,
		samples: &[f32],
	) -> AudioSampleLease {
		let layout = AudioSampleLayout {
			channel_count: 1,
			sample_rate: 48_000,
			frame_count: samples.len(),
			scalar_count: samples.len(),
		};
		let resident_bytes = layout.decoded_byte_count().unwrap();
		assert!(pool.make_room(resident_bytes));
		let region = pool.take_region(samples.len()).expect("test arena region");
		pool.storage[region.offset..region.end()].copy_from_slice(samples);
		pool.insert(
			cache_key(resource_id, payload_hash, u32::try_from(samples.len()).unwrap()),
			layout,
			region,
		)
	}

	fn release(pool: &mut AudioSamplePool, queue: &AudioSampleReleaseQueue, lease: AudioSampleLease) {
		assert!(queue.push(lease.into_id()));
		pool.release_returned(queue);
	}

	fn loader_client(
		commands: kanal::Sender<super::AudioLoadRequest>,
		completions: kanal::Receiver<AudioLoadCompletion>,
	) -> AudioSampleLoaderClient {
		AudioSampleLoaderClient::new(commands, completions, Arc::new(AudioSampleReleaseQueue::new()))
	}

	#[test]
	fn decoder_normalizes_supported_little_endian_pcm_depths() {
		let eight = decode(metadata(BitDepths::Eight, 1, 3), &[0, 128, 255]).expect("expected test value");
		assert_eq!(&*eight, &[-1.0, 0.0, 127.0 / 128.0]);

		let mut sixteen_bytes = Vec::new();
		for sample in [i16::MIN, 0, i16::MAX] {
			sixteen_bytes.extend_from_slice(&sample.to_le_bytes());
		}
		let sixteen = decode(metadata(BitDepths::Sixteen, 1, 3), &sixteen_bytes).expect("expected test value");
		assert_eq!(sixteen[0], -1.0);
		assert_eq!(sixteen[1], 0.0);
		assert!((sixteen[2] - i16::MAX as f32 / 32_768.0).abs() < f32::EPSILON);

		let twenty_four_bytes = [0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f];
		let twenty_four = decode(metadata(BitDepths::TwentyFour, 1, 3), &twenty_four_bytes).expect("expected test value");
		assert_eq!(twenty_four[0], -1.0);
		assert_eq!(twenty_four[1], 0.0);
		assert!((twenty_four[2] - 8_388_607.0 / 8_388_608.0).abs() < f32::EPSILON);

		let mut thirty_two_bytes = Vec::new();
		for sample in [i32::MIN, 0, i32::MAX] {
			thirty_two_bytes.extend_from_slice(&sample.to_le_bytes());
		}
		let thirty_two = decode(metadata(BitDepths::ThirtyTwo, 1, 3), &thirty_two_bytes).expect("expected test value");
		assert_eq!(thirty_two[0], -1.0);
		assert_eq!(thirty_two[1], 0.0);
		assert!(thirty_two[2] > 0.99);
	}

	#[test]
	fn decoder_validates_exact_interleaved_payload_length() {
		let error = decode(metadata(BitDepths::Sixteen, 2, 2), &[0; 6]).unwrap_err();
		assert!(error.contains("requires 8"));
		assert!(decode(metadata(BitDepths::Sixteen, 0, 2), &[]).is_err());
		assert!(decode(metadata(BitDepths::Sixteen, 3, 2), &[0; 12]).is_err());
	}

	#[test]
	fn cache_key_covers_payload_hash_and_all_playback_metadata() {
		let base = metadata(BitDepths::Sixteen, 1, 2);
		let key = AudioSampleCacheKey::new("tone.wav", 7, base);

		for distinct in [
			AudioSampleCacheKey::new("other.wav", 7, base),
			AudioSampleCacheKey::new("tone.wav", 8, base),
			AudioSampleCacheKey::new("tone.wav", 7, metadata(BitDepths::Eight, 1, 2)),
			AudioSampleCacheKey::new("tone.wav", 7, metadata(BitDepths::Sixteen, 2, 2)),
			AudioSampleCacheKey::new(
				"tone.wav",
				7,
				Audio {
					sample_rate: 44_100,
					..base
				},
			),
			AudioSampleCacheKey::new("tone.wav", 7, metadata(BitDepths::Sixteen, 1, 3)),
		] {
			assert_ne!(key, distinct);
		}
	}

	#[test]
	fn stereo_frames_are_downmixed_to_mono() {
		let sample = AudioSampleLease::for_test(48_000, 2, Box::from([1.0, -1.0, 0.5, 0.25]));
		assert_eq!(sample.mono_frame(0), 0.0);
		assert_eq!(sample.mono_frame(1), 0.375);
	}

	#[test]
	fn pool_keeps_leased_samples_resident_and_evicts_them_after_release() {
		let mut pool = pool(8);
		let releases = AudioSampleReleaseQueue::new();
		let active = insert_normalized(&mut pool, "active.wav", 1, &[0.0, 1.0]);
		let second = pool
			.lease(&cache_key("active.wav", 1, 2))
			.expect("active sample should support another lease");

		assert!(!pool.make_room(4));
		assert_eq!(pool.resident_bytes, 8);
		assert_eq!(active.mono_frame(1), 1.0);
		assert_eq!(second.mono_frame(1), 1.0);

		let active_id = active.id();
		drop(active);
		assert!(!pool.make_room(4));
		assert!(releases.push(active_id));
		pool.release_returned(&releases);
		assert!(!pool.make_room(4));
		release(&mut pool, &releases, second);
		assert!(pool.make_room(4));
		let replacement = insert_normalized(&mut pool, "replacement.wav", 2, &[0.5]);

		assert_eq!(replacement.mono_frame(0), 0.5);
		assert_eq!(pool.resident_bytes, 4);
		assert!(pool.lease(&cache_key("active.wav", 1, 2)).is_none());
		assert!(pool.resident_bytes <= pool.byte_budget);
	}

	#[test]
	fn samples_occupy_disjoint_ranges_of_one_preallocated_arena() {
		let mut pool = pool(16);
		let first = insert_normalized(&mut pool, "first.wav", 1, &[1.0, 2.0]);
		let second = insert_normalized(&mut pool, "second.wav", 2, &[3.0]);
		let arena_start = pool.storage.as_ptr() as usize;
		let arena_end = arena_start + pool.storage.len() * size_of::<f32>();
		let first_pointer = first.samples.as_ptr() as usize;
		let second_pointer = second.samples.as_ptr() as usize;

		assert_eq!(pool.storage.len(), 4);
		assert!(first_pointer >= arena_start && first_pointer < arena_end);
		assert!(second_pointer >= arena_start && second_pointer < arena_end);
		assert_ne!(first_pointer, second_pointer);
		assert!(first.owned_samples.is_none());
		assert!(second.owned_samples.is_none());
		assert_eq!(&*pool.storage, &[1.0, 2.0, 3.0, 0.0]);
	}

	#[test]
	fn returned_arena_regions_coalesce_after_fragmentation() {
		let mut pool = pool(24);
		let first = pool.take_region(2).expect("first region");
		let second = pool.take_region(2).expect("second region");
		let third = pool.take_region(2).expect("third region");
		assert!(pool.free_regions.is_empty());

		pool.return_region(first);
		pool.return_region(third);
		assert_eq!(pool.free_regions, [first, third]);
		pool.return_region(second);

		assert_eq!(
			pool.free_regions,
			[super::AudioSampleRegion {
				offset: 0,
				scalar_count: 6
			}]
		);
	}

	#[test]
	fn failed_decode_returns_its_reserved_arena_region() {
		let mut pool = pool(8);
		let metadata = metadata(BitDepths::Sixteen, 1, 2);
		let layout = AudioSampleLayout::new(metadata).unwrap();
		assert!(pool.make_room(layout.decoded_byte_count().unwrap()));

		let error = pool
			.decode_and_insert(cache_key("broken.wav", 1, 2), metadata, layout, &[0; 2])
			.unwrap_err();

		assert!(error.contains("requires 4"));
		assert_eq!(pool.resident_bytes, 0);
		assert_eq!(pool.free_regions[0].scalar_count, 2);
	}

	#[test]
	fn pool_evicts_the_inactive_least_recently_used_sample() {
		let mut pool = pool(8);
		let releases = AudioSampleReleaseQueue::new();
		let old = insert_normalized(&mut pool, "old.wav", 1, &[0.0]);
		release(&mut pool, &releases, old);
		let recent = insert_normalized(&mut pool, "recent.wav", 2, &[1.0]);
		release(&mut pool, &releases, recent);
		let old = pool
			.lease(&cache_key("old.wav", 1, 1))
			.expect("old sample should be resident");
		release(&mut pool, &releases, old);

		assert!(pool.make_room(4));
		let new = insert_normalized(&mut pool, "new.wav", 3, &[2.0]);
		release(&mut pool, &releases, new);

		assert!(pool.lease(&cache_key("old.wav", 1, 1)).is_some());
		assert!(pool.lease(&cache_key("recent.wav", 2, 1)).is_none());
		assert!(pool.lease(&cache_key("new.wav", 3, 1)).is_some());
		assert_eq!(pool.resident_bytes, 8);
	}

	#[test]
	fn stale_release_id_cannot_unpin_a_reused_slot_generation() {
		let mut pool = pool(4);
		let releases = AudioSampleReleaseQueue::new();
		let first = insert_normalized(&mut pool, "first.wav", 1, &[0.0]);
		let stale_id = first.id();
		release(&mut pool, &releases, first);
		assert!(pool.make_room(4));

		let second = insert_normalized(&mut pool, "second.wav", 2, &[1.0]);
		assert_eq!(second.id().slot, stale_id.slot);
		assert_ne!(second.id().generation, stale_id.generation);
		assert!(releases.push(stale_id));
		pool.release_returned(&releases);
		assert!(!pool.make_room(4));

		release(&mut pool, &releases, second);
		assert!(pool.make_room(4));
	}

	#[test]
	fn release_queue_is_bounded_fifo_and_reuses_wrapped_slots() {
		let queue = AudioSampleReleaseQueue::new();
		for slot in 0..AUDIO_SAMPLE_RELEASE_CAPACITY {
			assert!(queue.push(AudioSampleLeaseId {
				slot: u8::try_from(slot % AUDIO_GRAPH_CAPACITY).unwrap(),
				generation: slot as u64,
			}));
		}
		assert!(!queue.push(AudioSampleLeaseId {
			slot: 0,
			generation: 999
		}));

		for generation in 0..AUDIO_SAMPLE_RELEASE_CAPACITY / 2 {
			assert_eq!(queue.pop().expect("queued release").generation, generation as u64);
		}
		for generation in AUDIO_SAMPLE_RELEASE_CAPACITY..AUDIO_SAMPLE_RELEASE_CAPACITY + 16 {
			assert!(queue.push(AudioSampleLeaseId {
				slot: 0,
				generation: generation as u64,
			}));
		}
		for generation in AUDIO_SAMPLE_RELEASE_CAPACITY / 2..AUDIO_SAMPLE_RELEASE_CAPACITY + 16 {
			assert_eq!(queue.pop().expect("queued release").generation, generation as u64);
		}
		assert!(queue.pop().is_none());
	}

	#[test]
	fn decoded_sample_size_is_checked_before_pool_admission() {
		assert_eq!(
			AudioSampleLayout::new(metadata(BitDepths::Eight, 2, 2)).and_then(AudioSampleLayout::decoded_byte_count),
			Ok(16)
		);
		let mut pool = pool(8);
		assert!(!pool.make_room(16));
		assert_eq!(pool.resident_bytes, 0);
	}

	#[test]
	fn deleted_and_replaced_generations_reject_stale_completions() {
		let (commands, _command_receiver) = kanal::bounded_async(4);
		let (completion_sender, completions) = kanal::bounded_async(4);
		let completion_sender = completion_sender.to_sync();
		let mut client = loader_client(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let mut listener = factory.listener();
		let handle = factory.create(());
		let _ = listener.read();

		assert!(client.queue(handle, r#loop(sample("first.wav")).compile().expect("expected test value"), 0));
		let first_generation = client.pending[0].generation;
		assert!(client.queue(
			handle,
			gain(sample("second.wav"), 0.25).compile().expect("expected test value"),
			0
		));
		let second_generation = client.pending[0].generation;
		assert_ne!(first_generation, second_generation);

		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation: first_generation,
				sample: lease([0.0]),
				render_plan: prepared_plan(SamplePlaybackMode::Loop, []),
			})
			.expect("expected test value");

		let mut created = Vec::new();
		client.update(|handle, _, plan| {
			created.push((handle, plan.playback_mode, plan.output_gain));
		});
		assert!(created.is_empty());
		assert_eq!(client.pending.len(), 1);
		assert_eq!(client.lease_release_epoch, 1);

		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation: second_generation,
				sample: lease([0.0]),
				render_plan: prepared_plan(SamplePlaybackMode::Once, [AudioProcessor::Gain(0.25)]),
			})
			.expect("expected test value");
		client.update(|handle, _, plan| {
			created.push((handle, plan.playback_mode, plan.output_gain));
		});
		assert_eq!(created, [(handle, SamplePlaybackMode::Once, 0.25)]);
		assert!(client.pending.is_empty());
	}

	#[test]
	fn deleted_graph_rejects_a_completion_that_was_already_in_flight() {
		let (commands, _command_receiver) = kanal::bounded_async(4);
		let (completion_sender, completions) = kanal::bounded_async(4);
		let completion_sender = completion_sender.to_sync();
		let mut client = loader_client(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let handle = factory.create(());

		assert!(client.queue(handle, sample("deleted.wav").compile().expect("expected test value"), 0));
		let generation = client.pending[0].generation;
		client.remove(handle);
		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation,
				sample: lease([0.0]),
				render_plan: prepared_plan(SamplePlaybackMode::Once, []),
			})
			.expect("expected test value");

		let mut created = false;
		client.update(|_, _, _| created = true);

		assert!(!created);
		assert_eq!(client.lease_release_epoch, 1);
	}

	#[test]
	fn capacity_wait_retries_after_a_lease_release() {
		let (commands, command_receiver) = kanal::bounded_async(4);
		let command_receiver = command_receiver.to_sync();
		let (completion_sender, completions) = kanal::bounded_async(4);
		let mut client = loader_client(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let handle = factory.create(());
		assert!(client.queue(
			handle,
			r#loop(sample("replacement.wav")).compile().expect("expected test value"),
			0
		));

		client.submit_requests();
		let Some(request) = command_receiver.try_recv().expect("expected test value") else {
			panic!("expected submitted audio load");
		};
		completion_sender
			.to_sync()
			.send(AudioLoadCompletion::WaitingForCapacity { request })
			.expect("expected test value");
		client.update(|_, _, _| panic!("capacity-blocked load must not create a graph"));
		assert!(client.pending[0].waiting_for_capacity);

		assert!(client.return_lease(lease([0.0]).id()));
		client.submit_requests();
		assert!(matches!(
			command_receiver.try_recv().expect("expected test value"),
			Some(request) if request.handle == handle
		));
		assert!(!client.pending[0].waiting_for_capacity);
	}

	#[test]
	fn lease_release_racing_with_capacity_completion_retries_without_another_release() {
		let (commands, command_receiver) = kanal::bounded_async(4);
		let command_receiver = command_receiver.to_sync();
		let (completion_sender, completions) = kanal::bounded_async(4);
		let mut client = loader_client(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let handle = factory.create(());
		assert!(client.queue(handle, sample("racing.wav").compile().expect("expected test value"), 0));
		client.submit_requests();
		let Some(request) = command_receiver.try_recv().expect("expected test value") else {
			panic!("expected submitted audio load");
		};

		assert!(client.return_lease(lease([0.0]).id()));
		completion_sender
			.to_sync()
			.send(AudioLoadCompletion::WaitingForCapacity { request })
			.expect("expected test value");
		client.update(|_, _, _| panic!("capacity-blocked load must not create a graph"));
		assert!(!client.pending[0].waiting_for_capacity);

		client.submit_requests();
		assert!(matches!(
			command_receiver.try_recv().expect("expected test value"),
			Some(request) if request.handle == handle
		));
	}
}
