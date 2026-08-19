//! PCM sample allocation, cache residency, and leases.

use std::{mem::size_of, num::NonZeroUsize, ptr::NonNull, sync::Arc};

use crossbeam_queue::ArrayQueue;
use resource_management::{
	resource::{resource_manager::ResourceManager, ReadTargetsMut},
	resources::audio::Audio,
	types::BitDepths,
	Reference,
};

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
	pub(super) byte_budget: NonZeroUsize,
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
pub(super) struct AudioSampleCacheKey {
	resource_id: String,
	payload_hash: u64,
	bit_depth: u8,
	pub(super) channel_count: u16,
	pub(super) sample_rate: u32,
	pub(super) frame_count: u32,
}

impl AudioSampleCacheKey {
	pub(super) fn new(resource_id: &str, payload_hash: u64, metadata: Audio) -> Self {
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
pub(super) struct AudioSampleLayout {
	pub(super) channel_count: u16,
	pub(super) sample_rate: u32,
	pub(super) frame_count: usize,
	pub(super) scalar_count: usize,
}

impl AudioSampleLayout {
	/// Validates resource playback metadata before the pool reserves storage.
	pub(super) fn new(metadata: Audio) -> Result<Self, String> {
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
	pub(super) fn decoded_byte_count(self) -> Result<usize, String> {
		self.scalar_count
			.checked_mul(size_of::<f32>())
			.ok_or_else(|| "Invalid audio sample layout. The decoded PCM byte count overflowed.".to_string())
	}
}

/// Decodes one resource directly into its reserved PCM arena region.
pub(super) fn decode_into(metadata: Audio, bytes: &[u8], samples: &mut [f32]) -> Result<AudioSampleLayout, String> {
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
	pub(super) slot: u8,
	pub(super) generation: u64,
}

/// The `AudioSampleLease` struct provides lock-free read access to one stable
/// pool-owned sample allocation.
///
/// Return [`Self::into_id`] through the audio sample release queue after the graph
/// stops using this lease. The loader will then make the slot eligible for
/// eviction.
#[derive(Debug)]
pub(crate) struct AudioSampleLease {
	pub(super) id: AudioSampleLeaseId,
	pub(super) samples: NonNull<f32>,
	pub(super) layout: AudioSampleLayout,
	#[cfg(test)]
	pub(super) owned_samples: Option<Box<[f32]>>,
}

impl AudioSampleLease {
	pub(super) fn new(id: AudioSampleLeaseId, samples: &[f32], layout: AudioSampleLayout) -> Self {
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
	pub(super) fn id(&self) -> AudioSampleLeaseId {
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
pub(super) struct AudioSampleReleaseQueue {
	queue: ArrayQueue<AudioSampleLeaseId>,
}

impl AudioSampleReleaseQueue {
	pub(super) fn new() -> Self {
		Self {
			queue: ArrayQueue::new(AUDIO_SAMPLE_RELEASE_CAPACITY),
		}
	}

	/// Pushes one ID from the audio thread without waiting or contending on a lock.
	pub(super) fn push(&self, id: AudioSampleLeaseId) -> bool {
		self.queue.push(id).is_ok()
	}

	/// Pops one returned ID on the loader thread.
	pub(super) fn pop(&self) -> Option<AudioSampleLeaseId> {
		self.queue.pop()
	}
}

/// The `AudioSampleRegion` struct identifies one contiguous scalar range in the
/// preallocated PCM arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AudioSampleRegion {
	pub(super) offset: usize,
	pub(super) scalar_count: usize,
}

impl AudioSampleRegion {
	pub(super) fn end(self) -> usize {
		self.offset + self.scalar_count
	}
}

/// The `CachedAudioSample` struct retains one arena range and its loader-owned
/// residency state.
pub(super) struct CachedAudioSample {
	pub(super) key: AudioSampleCacheKey,
	pub(super) region: AudioSampleRegion,
	pub(super) layout: AudioSampleLayout,
	pub(super) resident_bytes: usize,
	pub(super) last_used: u64,
	pub(super) lease_count: usize,
}

/// The `AudioSampleSlot` struct preserves generation identity while cache
/// entries are evicted and replaced.
pub(super) struct AudioSampleSlot {
	pub(super) generation: u64,
	pub(super) entry: Option<CachedAudioSample>,
}

/// The `AudioSamplePool` struct owns the global preallocated PCM arena and its
/// byte-bounded LRU cache.
pub(super) struct AudioSamplePool {
	pub(super) storage: Box<[f32]>,
	pub(super) free_regions: Vec<AudioSampleRegion>,
	pub(super) slots: [AudioSampleSlot; AUDIO_GRAPH_CAPACITY],
	pub(super) byte_budget: usize,
	pub(super) resident_bytes: usize,
	pub(super) next_use: u64,
}

impl AudioSamplePool {
	pub(super) fn new(config: AudioSamplePoolConfig) -> Self {
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
	pub(super) fn lease(&mut self, key: &AudioSampleCacheKey) -> Option<AudioSampleLease> {
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
	pub(super) fn release_returned(&mut self, releases: &AudioSampleReleaseQueue) {
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
	pub(super) fn make_room(&mut self, required_bytes: usize) -> bool {
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
	pub(super) fn decode_and_insert(
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
	pub(super) fn insert(
		&mut self,
		key: AudioSampleCacheKey,
		layout: AudioSampleLayout,
		region: AudioSampleRegion,
	) -> AudioSampleLease {
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
	pub(super) fn take_region(&mut self, scalar_count: usize) -> Option<AudioSampleRegion> {
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
	pub(super) fn return_region(&mut self, region: AudioSampleRegion) {
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

	pub(super) fn next_use(&mut self) -> u64 {
		let use_stamp = self.next_use;
		self.next_use = self.next_use.wrapping_add(1);
		use_stamp
	}
}
