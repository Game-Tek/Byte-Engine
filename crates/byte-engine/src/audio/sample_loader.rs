//! Audio sample loading facade.

mod sample_loading;
mod sample_pool;

use sample_loading::*;
pub(crate) use sample_loading::{AudioSampleLoader, AudioSampleLoaderClient};
use sample_pool::*;
pub(crate) use sample_pool::{AudioSampleLease, AudioSampleLeaseId, AUDIO_GRAPH_CAPACITY, AUDIO_SAMPLE_RELEASE_CAPACITY};
pub use sample_pool::{AudioSamplePoolConfig, DEFAULT_AUDIO_SAMPLE_POOL_BYTE_BUDGET};

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
