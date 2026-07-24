use std::{collections::HashMap, sync::Arc};

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

#[derive(Debug)]
/// The `LoadedAudioSample` struct owns normalized, interleaved PCM that can be
/// read by the audio worker without resource I/O or format conversion.
pub(crate) struct LoadedAudioSample {
	samples: Box<[f32]>,
	channel_count: u16,
	sample_rate: u32,
	frame_count: usize,
}

impl LoadedAudioSample {
	fn decode(metadata: Audio, bytes: &[u8]) -> Result<Self, String> {
		if metadata.channel_count != 1 && metadata.channel_count != 2 {
			return Err("Unsupported audio sample channel count. The resource must contain mono or stereo PCM.".to_string());
		}
		if metadata.sample_rate == 0 {
			return Err("Invalid audio sample rate. The resource metadata reports zero hertz.".to_string());
		}
		if metadata.sample_count == 0 {
			return Err("Invalid audio sample length. The resource metadata reports zero frames.".to_string());
		}

		let bytes_per_sample = match metadata.bit_depth {
			BitDepths::Eight => 1,
			BitDepths::Sixteen => 2,
			BitDepths::TwentyFour => 3,
			BitDepths::ThirtyTwo => 4,
		};
		let frame_count = usize::try_from(metadata.sample_count)
			.map_err(|_| "Invalid audio sample length. The frame count does not fit this platform.".to_string())?;
		let scalar_count = frame_count
			.checked_mul(usize::from(metadata.channel_count))
			.ok_or_else(|| "Invalid audio sample layout. The channel sample count overflowed.".to_string())?;
		let expected_byte_count = scalar_count
			.checked_mul(bytes_per_sample)
			.ok_or_else(|| "Invalid audio sample layout. The PCM byte count overflowed.".to_string())?;

		if bytes.len() != expected_byte_count {
			return Err(format!(
				"Invalid audio sample payload. The resource contains {} bytes but its metadata requires {expected_byte_count}.",
				bytes.len()
			));
		}

		let mut samples = Vec::with_capacity(scalar_count);
		match metadata.bit_depth {
			// WAV PCM and the engine OGG baker both store 8-bit PCM as unsigned.
			BitDepths::Eight => samples.extend(bytes.iter().map(|byte| (*byte as f32 - 128.0) / 128.0)),
			BitDepths::Sixteen => {
				samples.extend(
					bytes
						.chunks_exact(2)
						.map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32_768.0),
				);
			}
			BitDepths::TwentyFour => {
				samples.extend(bytes.chunks_exact(3).map(|sample| {
					let sign = if sample[2] & 0x80 == 0 { 0 } else { 0xff };
					i32::from_le_bytes([sample[0], sample[1], sample[2], sign]) as f32 / 8_388_608.0
				}));
			}
			BitDepths::ThirtyTwo => {
				samples.extend(
					bytes.chunks_exact(4).map(|sample| {
						i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32 / 2_147_483_648.0
					}),
				);
			}
		}

		Ok(Self {
			samples: samples.into_boxed_slice(),
			channel_count: metadata.channel_count,
			sample_rate: metadata.sample_rate,
			frame_count,
		})
	}

	#[cfg(test)]
	pub(crate) fn from_normalized_samples(sample_rate: u32, channel_count: u16, samples: Box<[f32]>) -> Self {
		assert!(sample_rate > 0);
		assert!(channel_count == 1 || channel_count == 2);
		assert!(!samples.is_empty());
		assert_eq!(samples.len() % usize::from(channel_count), 0);

		Self {
			frame_count: samples.len() / usize::from(channel_count),
			samples,
			channel_count,
			sample_rate,
		}
	}

	pub(crate) fn sample_rate(&self) -> u32 {
		self.sample_rate
	}

	pub(crate) fn frame_count(&self) -> usize {
		self.frame_count
	}

	/// Returns one mono frame. Stereo resources are downmixed before
	/// interpolation so playback uses one consistent mono output timeline.
	pub(crate) fn mono_frame(&self, frame: usize) -> f32 {
		let index = frame * usize::from(self.channel_count);
		if self.channel_count == 1 {
			self.samples[index]
		} else {
			(self.samples[index] + self.samples[index + 1]) * 0.5
		}
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

/// Carries load and coalesced cache-maintenance work to the application
/// runtime through one bounded channel.
enum AudioLoaderCommand {
	Load(AudioLoadRequest),
	PruneCache,
}

/// Carries a completed load back to the audio worker without exposing the
/// borrowed resource backing used during conversion.
enum AudioLoadCompletion {
	Ready {
		handle: Handle,
		generation: u64,
		sample: Arc<LoadedAudioSample>,
		render_plan: PreparedAudioGraphRenderPlan,
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
	request: Option<AudioLoaderCommand>,
}

/// The `AudioSampleLoaderClient` struct bridges world lifecycle messages to
/// the async loader without waiting for Kanal lock or queue capacity.
pub(crate) struct AudioSampleLoaderClient {
	commands: kanal::Sender<AudioLoaderCommand>,
	completions: kanal::Receiver<AudioLoadCompletion>,
	pending: Vec<PendingAudioGraph>,
	next_generation: u64,
	cache_prune_requested: bool,
	commands_closed: bool,
	completions_closed: bool,
}

impl AudioSampleLoaderClient {
	fn new(commands: kanal::Sender<AudioLoaderCommand>, completions: kanal::Receiver<AudioLoadCompletion>) -> Self {
		Self {
			commands,
			completions,
			pending: Vec::with_capacity(AUDIO_GRAPH_CAPACITY),
			next_generation: 0,
			cache_prune_requested: false,
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
			request: Some(AudioLoaderCommand::Load(AudioLoadRequest {
				handle,
				generation,
				resource_id,
				render_plan,
			})),
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

	/// Coalesces requests to reclaim cache entries that the audio worker has
	/// stopped using.
	pub(crate) fn request_cache_prune(&mut self) {
		self.cache_prune_requested = true;
	}

	/// Submits waiting requests and adopts ready samples at a hardware-period
	/// boundary. The callback runs only for a still-live request generation.
	pub(crate) fn update(
		&mut self,
		mut create_graph: impl FnMut(Handle, Arc<LoadedAudioSample>, PreparedAudioGraphRenderPlan),
	) {
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
			let Some(_) = graph.request else {
				continue;
			};
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

		// Queue pruning after loads so a replacement graph can renew cache
		// ownership before an immediately preceding deletion is reclaimed.
		if !self.commands_closed && self.cache_prune_requested {
			let mut command = Some(AudioLoaderCommand::PruneCache);
			match self.commands.try_send_option_realtime(&mut command) {
				Ok(true) => self.cache_prune_requested = false,
				Ok(false) => {}
				Err(_) => self.commands_closed = true,
			}
		}
	}

	fn process_completion(
		&mut self,
		completion: AudioLoadCompletion,
		create_graph: &mut impl FnMut(Handle, Arc<LoadedAudioSample>, PreparedAudioGraphRenderPlan),
	) {
		let (handle, generation) = match &completion {
			AudioLoadCompletion::Ready { handle, generation, .. } | AudioLoadCompletion::Failed { handle, generation } => {
				(*handle, *generation)
			}
		};
		let Some(index) = self
			.pending
			.iter()
			.position(|graph| graph.handle == handle && graph.generation == generation)
		else {
			if matches!(completion, AudioLoadCompletion::Ready { .. }) {
				self.cache_prune_requested = true;
			}
			return;
		};
		let pending = self.pending.swap_remove(index);

		if let AudioLoadCompletion::Ready { sample, render_plan, .. } = completion {
			create_graph(handle, sample, render_plan);
		}
	}
}

/// The `AudioSampleLoader` struct owns resource references while converting
/// baked PCM into samples that can safely cross to the audio worker.
pub(crate) struct AudioSampleLoader {
	resource_manager: EntityHandle<ResourceManager>,
	commands: kanal::AsyncReceiver<AudioLoaderCommand>,
	completions: kanal::AsyncSender<AudioLoadCompletion>,
	cache: HashMap<AudioSampleCacheKey, Arc<LoadedAudioSample>>,
}

impl AudioSampleLoader {
	/// Creates the bounded real-time client and its application-runtime worker.
	pub(crate) fn new(resource_manager: EntityHandle<ResourceManager>) -> (AudioSampleLoaderClient, Self) {
		let (commands, command_receiver) = kanal::bounded_async(AUDIO_GRAPH_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(AUDIO_GRAPH_CAPACITY);

		(
			AudioSampleLoaderClient::new(commands.to_sync(), completions.to_sync()),
			Self {
				resource_manager,
				commands: command_receiver,
				completions: completion_sender,
				cache: HashMap::with_capacity(AUDIO_GRAPH_CAPACITY),
			},
		)
	}

	/// Handles resource requests until the audio worker closes its channel.
	pub(crate) async fn run(mut self) {
		while let Ok(command) = self.commands.recv().await {
			let AudioLoaderCommand::Load(request) = command else {
				self.prune_cache();
				async_runtime::yield_now().await;
				continue;
			};
			let AudioLoadRequest {
				handle,
				generation,
				resource_id,
				render_plan,
			} = request;
			let completion = match self.load(&resource_id).await {
				Ok(sample) => AudioLoadCompletion::Ready {
					handle,
					generation,
					sample,
					render_plan: render_plan.prepare(),
				},
				Err(error) => {
					log::error!(
						"Failed to load audio sample '{}'. The resource could not be prepared for playback: {}",
						resource_id,
						error
					);
					AudioLoadCompletion::Failed { handle, generation }
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
	async fn load(&mut self, resource_id: &str) -> Result<Arc<LoadedAudioSample>, String> {
		let mut reference: Reference<Audio> = self
			.resource_manager
			.request(resource_id)
			.await
			.map_err(|error| format!("Resource request failed. The resource manager reported: {error}"))?;
		let metadata = *reference.resource();
		let cache_key = AudioSampleCacheKey::new(resource_id, reference.hash(), metadata);

		// Keep a strong loader-side reference while the audio thread may own a
		// sample. This makes the loader responsible for freeing large PCM boxes.
		// Retain the requested version even when idle so repeated playback can
		// reuse it, and prune other versions that no consumer owns.
		self.cache
			.retain(|key, sample| key == &cache_key || Arc::strong_count(sample) > 1);
		if let Some(sample) = self.cache.get(&cache_key) {
			return Ok(sample.clone());
		}

		let loaded = reference
			.load(ReadTargetsMut::backing_storage())
			.await
			.map_err(|error| format!("PCM read failed. The resource reader reported: {error:?}"))?;
		let bytes = loaded
			.buffer()
			.ok_or_else(|| "PCM read failed. The resource reader returned non-contiguous data.".to_string())?;
		let sample = Arc::new(LoadedAudioSample::decode(metadata, bytes)?);

		self.cache.insert(cache_key, sample.clone());
		Ok(sample)
	}

	/// Drops cache ownership only after the audio worker has released its
	/// corresponding strong references.
	fn prune_cache(&mut self) {
		self.cache.retain(|_, sample| Arc::strong_count(sample) > 1);
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use resource_management::{resources::audio::Audio, types::BitDepths};

	use super::{AudioLoadCompletion, AudioLoaderCommand, AudioSampleCacheKey, AudioSampleLoaderClient, LoadedAudioSample};
	use crate::{
		audio::graph::{
			fns::{gain, r#loop, sample},
			AudioGraphRenderPlan, AudioProcessor, PreparedAudioGraphRenderPlan, SamplePlaybackMode,
		},
		core::{factory::Factory, listener::Listener},
	};

	fn prepared_plan(
		playback_mode: SamplePlaybackMode,
		processors: impl IntoIterator<Item = AudioProcessor>,
	) -> PreparedAudioGraphRenderPlan {
		AudioGraphRenderPlan {
			playback_mode,
			processors: processors.into_iter().collect(),
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

	#[test]
	fn decoder_normalizes_supported_little_endian_pcm_depths() {
		let eight = LoadedAudioSample::decode(metadata(BitDepths::Eight, 1, 3), &[0, 128, 255]).unwrap();
		assert_eq!(&*eight.samples, &[-1.0, 0.0, 127.0 / 128.0]);

		let mut sixteen_bytes = Vec::new();
		for sample in [i16::MIN, 0, i16::MAX] {
			sixteen_bytes.extend_from_slice(&sample.to_le_bytes());
		}
		let sixteen = LoadedAudioSample::decode(metadata(BitDepths::Sixteen, 1, 3), &sixteen_bytes).unwrap();
		assert_eq!(sixteen.samples[0], -1.0);
		assert_eq!(sixteen.samples[1], 0.0);
		assert!((sixteen.samples[2] - i16::MAX as f32 / 32_768.0).abs() < f32::EPSILON);

		let twenty_four_bytes = [0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0xff, 0xff, 0x7f];
		let twenty_four = LoadedAudioSample::decode(metadata(BitDepths::TwentyFour, 1, 3), &twenty_four_bytes).unwrap();
		assert_eq!(twenty_four.samples[0], -1.0);
		assert_eq!(twenty_four.samples[1], 0.0);
		assert!((twenty_four.samples[2] - 8_388_607.0 / 8_388_608.0).abs() < f32::EPSILON);

		let mut thirty_two_bytes = Vec::new();
		for sample in [i32::MIN, 0, i32::MAX] {
			thirty_two_bytes.extend_from_slice(&sample.to_le_bytes());
		}
		let thirty_two = LoadedAudioSample::decode(metadata(BitDepths::ThirtyTwo, 1, 3), &thirty_two_bytes).unwrap();
		assert_eq!(thirty_two.samples[0], -1.0);
		assert_eq!(thirty_two.samples[1], 0.0);
		assert!(thirty_two.samples[2] > 0.99);
	}

	#[test]
	fn decoder_validates_exact_interleaved_payload_length() {
		let error = LoadedAudioSample::decode(metadata(BitDepths::Sixteen, 2, 2), &[0; 6]).unwrap_err();
		assert!(error.contains("requires 8"));
		assert!(LoadedAudioSample::decode(metadata(BitDepths::Sixteen, 0, 2), &[]).is_err());
		assert!(LoadedAudioSample::decode(metadata(BitDepths::Sixteen, 3, 2), &[0; 12]).is_err());
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
		let sample = LoadedAudioSample::from_normalized_samples(48_000, 2, Box::from([1.0, -1.0, 0.5, 0.25]));
		assert_eq!(sample.mono_frame(0), 0.0);
		assert_eq!(sample.mono_frame(1), 0.375);
	}

	#[test]
	fn deleted_and_replaced_generations_reject_stale_completions() {
		let (commands, _command_receiver) = kanal::bounded_async(4);
		let (completion_sender, completions) = kanal::bounded_async(4);
		let completion_sender = completion_sender.to_sync();
		let mut client = AudioSampleLoaderClient::new(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let mut listener = factory.listener();
		let handle = factory.create(());
		let _ = listener.read();

		assert!(client.queue(handle, r#loop(sample("first.wav")).compile().unwrap(), 0));
		let first_generation = client.pending[0].generation;
		assert!(client.queue(handle, gain(sample("second.wav"), 0.25).compile().unwrap(), 0));
		let second_generation = client.pending[0].generation;
		assert_ne!(first_generation, second_generation);

		let sample = Arc::new(LoadedAudioSample::from_normalized_samples(48_000, 1, Box::from([0.0])));
		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation: first_generation,
				sample: sample.clone(),
				render_plan: prepared_plan(SamplePlaybackMode::Loop, []),
			})
			.unwrap();

		let mut created = Vec::new();
		client.update(|handle, _, plan| {
			let gains = plan
				.processors
				.iter()
				.filter_map(|processor| processor.gain_for_test())
				.collect::<Vec<_>>();
			created.push((handle, plan.playback_mode, gains));
		});
		assert!(created.is_empty());
		assert_eq!(client.pending.len(), 1);
		assert!(client.cache_prune_requested);

		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation: second_generation,
				sample,
				render_plan: prepared_plan(SamplePlaybackMode::Once, [AudioProcessor::Gain(0.25)]),
			})
			.unwrap();
		client.update(|handle, _, plan| {
			let gains = plan
				.processors
				.iter()
				.filter_map(|processor| processor.gain_for_test())
				.collect::<Vec<_>>();
			created.push((handle, plan.playback_mode, gains));
		});
		assert_eq!(created, [(handle, SamplePlaybackMode::Once, vec![0.25])]);
		assert!(client.pending.is_empty());
	}

	#[test]
	fn deleted_graph_rejects_a_completion_that_was_already_in_flight() {
		let (commands, _command_receiver) = kanal::bounded_async(4);
		let (completion_sender, completions) = kanal::bounded_async(4);
		let completion_sender = completion_sender.to_sync();
		let mut client = AudioSampleLoaderClient::new(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let handle = factory.create(());

		assert!(client.queue(handle, sample("deleted.wav").compile().unwrap(), 0));
		let generation = client.pending[0].generation;
		client.remove(handle);
		completion_sender
			.send(AudioLoadCompletion::Ready {
				handle,
				generation,
				sample: Arc::new(LoadedAudioSample::from_normalized_samples(48_000, 1, Box::from([0.0]))),
				render_plan: prepared_plan(SamplePlaybackMode::Once, []),
			})
			.unwrap();

		let mut created = false;
		client.update(|_, _, _| created = true);

		assert!(!created);
		assert!(client.cache_prune_requested);
	}

	#[test]
	fn cache_prune_requests_are_coalesced() {
		let (commands, command_receiver) = kanal::bounded_async(4);
		let command_receiver = command_receiver.to_sync();
		let (_completion_sender, completions) = kanal::bounded_async(4);
		let mut client = AudioSampleLoaderClient::new(commands.to_sync(), completions.to_sync());
		let mut factory = Factory::new();
		let handle = factory.create(());
		assert!(client.queue(handle, r#loop(sample("replacement.wav")).compile().unwrap(), 0));

		client.request_cache_prune();
		client.request_cache_prune();
		client.submit_requests();

		assert!(matches!(
			command_receiver.try_recv().unwrap(),
			Some(AudioLoaderCommand::Load(request)) if request.handle == handle
		));
		assert!(matches!(
			command_receiver.try_recv().unwrap(),
			Some(AudioLoaderCommand::PruneCache)
		));
		assert!(!client.cache_prune_requested);
	}
}
