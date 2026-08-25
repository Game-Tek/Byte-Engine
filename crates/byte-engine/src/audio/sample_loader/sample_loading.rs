//! Async sample loading and audio-thread completion delivery.

use std::sync::Arc;

use resource_management::{
	Reference,
	resource::{ReadTargetsMut, resource_manager::ResourceManager},
	resources::audio::Audio,
};

use super::*;
use crate::{
	audio::graph::{AudioGraphRenderPlan, CompiledAudioGraph, PreparedAudioGraphRenderPlan},
	core::{EntityHandle, async_runtime, factory::Handle},
};

#[derive(Debug)]
/// The `AudioLoadRequest` struct identifies one version of a graph resource
/// request sent from the audio worker.
pub(super) struct AudioLoadRequest {
	pub(super) handle: Handle,
	pub(super) generation: u64,
	pub(super) resource_id: String,
	pub(super) render_plan: AudioGraphRenderPlan,
}

/// Carries a completed load back to the audio worker without exposing the
/// borrowed resource backing used during conversion.
pub(super) enum AudioLoadCompletion {
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
pub(super) struct PendingAudioGraph {
	pub(super) handle: Handle,
	pub(super) generation: u64,
	request: Option<AudioLoadRequest>,
	pub(super) waiting_for_capacity: bool,
	pub(super) submitted_release_epoch: u64,
}

/// The `AudioSampleLoaderClient` struct bridges world lifecycle messages to
/// the async loader without waiting for Kanal lock or queue capacity.
pub(crate) struct AudioSampleLoaderClient {
	commands: kanal::Sender<AudioLoadRequest>,
	completions: kanal::Receiver<AudioLoadCompletion>,
	releases: Arc<AudioSampleReleaseQueue>,
	pending_releases: Vec<AudioSampleLeaseId>,
	pub(super) pending: Vec<PendingAudioGraph>,
	next_generation: u64,
	pub(super) lease_release_epoch: u64,
	commands_closed: bool,
	completions_closed: bool,
}

impl AudioSampleLoaderClient {
	pub(super) fn new(
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

	pub(super) fn submit_requests(&mut self) {
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
