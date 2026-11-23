//! DX12 device operations for presentation.

use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Threading::WaitForSingleObjectEx;

use super::super::pacing;
use super::*;

/// How long acquisition waits for room in a swapchain's present queue before it skips the window for the frame. It
/// matches Metal's wait for a drawable, after which a window the compositor does not show stops pacing the loop.
const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Returns whether presents may tear on this system: the display, driver, and Windows version all support it.
fn supports_tearing(factory: &IDXGIFactory4) -> bool {
	let Ok(factory) = factory.cast::<IDXGIFactory5>() else {
		return false;
	};
	let mut supported = BOOL(0);
	// SAFETY: The feature writes one `BOOL`, the size passed.
	let result = unsafe {
		factory.CheckFeatureSupport(
			DXGI_FEATURE_PRESENT_ALLOW_TEARING,
			(&mut supported as *mut BOOL).cast(),
			std::mem::size_of::<BOOL>() as u32,
		)
	};
	result.is_ok() && supported.as_bool()
}

impl Swapchain {
	/// Reads which present the display showed last, and when, from the swapchain's frame statistics.
	///
	/// Returns `None` before the first present reached the display, while the statistics are disjoint (after a mode
	/// change or while the window is hidden), or when the reported time lies ahead of `now`.
	fn shown_frame(&mut self, now: std::time::Instant) -> Option<pacing::ShownFrame> {
		let mut statistics = DXGI_FRAME_STATISTICS::default();
		// SAFETY: Both calls only read the swapchain's presentation history.
		let last_present = unsafe {
			self.swapchain.GetFrameStatistics(&mut statistics).ok()?;
			self.swapchain.GetLastPresentCount().ok()?
		};
		if statistics.SyncQPCTime == 0 {
			return None;
		}
		let at = match self.shown_instant {
			Some((qpc, instant)) if qpc == statistics.SyncQPCTime => instant,
			_ => {
				let instant = pacing::qpc_to_instant(statistics.SyncQPCTime)?;
				self.shown_instant = Some((statistics.SyncQPCTime, instant));
				instant
			}
		};
		// No more presents can wait than the latency allows; a larger count means the two reads disagree.
		let pending = last_present.wrapping_sub(statistics.PresentCount).min(self.frame_latency);
		(at <= now).then_some(pacing::ShownFrame { at, pending })
	}
}

impl Device {
	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		desired_image_count: Option<u8>,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		let extent = Self::query_window_extent(window_os_handles, fallback_extent);
		let image_count = swapchain_image_count(self.frames, desired_image_count);

		let (queue_index, queue) = self
			.queues
			.iter()
			.enumerate()
			.find(|(_, queue)| queue.workloads.intersects(WorkloadTypes::RASTER))
			.or_else(|| self.queues.first().map(|queue| (0, queue)))
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");
		let queue_handle = QueueHandle::from_index(queue_index as u64);

		let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI factory. The most likely cause is that the DXGI runtime is unavailable.");
		});

		// Tearing lets a variable-refresh display show each frame as soon as it is ready. Only immediate presentation
		// asks for it, and only where the display and driver support it.
		let allows_tearing = matches!(presentation_mode, PresentationModes::Inmediate) && supports_tearing(&factory);
		// The waitable object moves the wait for queue room from `Present` to acquisition. See `latency_waitable`.
		let flags = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT
			| if allows_tearing {
				DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
			} else {
				DXGI_SWAP_CHAIN_FLAG(0)
			};

		let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
			Width: extent.width(),
			Height: extent.height(),
			Format: DXGI_FORMAT_B8G8R8A8_UNORM,
			Stereo: false.into(),
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
			BufferCount: image_count as u32,
			Scaling: DXGI_SCALING_STRETCH,
			SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
			AlphaMode: DXGI_ALPHA_MODE_IGNORE,
			Flags: flags.0 as u32,
		};

		let swapchain = unsafe { factory.CreateSwapChainForHwnd(&queue.queue, window_os_handles.hwnd, &swapchain_desc, None, None) }.unwrap_or_else(|error| {
			panic!("Failed to create a DXGI swapchain ({error}). The most likely cause is that the window handle is invalid, the device does not support the swapchain format, or the process runs without an interactive desktop, as a service or SSH session does. Create the window from a process in an interactive session.");
		});

		let swapchain: IDXGISwapChain3 = swapchain.cast().unwrap_or_else(|_| {
			panic!(
				"Failed to upgrade the DXGI swapchain. The most likely cause is that the DXGI runtime does not support IDXGISwapChain3."
			);
		});

		let _ = unsafe { factory.MakeWindowAssociation(window_os_handles.hwnd, DXGI_MWA_NO_ALT_ENTER) };

		// SAFETY: The swapchain was created with the waitable flag, which this call requires. The returned handle is
		// owned by the caller and closed when `Owned` drops.
		let latency_waitable = unsafe { windows::core::Owned::new(swapchain.GetFrameLatencyWaitableObject()) };

		self.swapchains.push(Swapchain {
			handles: window::Handles {
				hinstance: window_os_handles.hinstance,
				hwnd: window_os_handles.hwnd,
			},
			swapchain,
			extent,
			image_count,
			desired_image_count,
			next_image_index: 0,
			present_mode: presentation_mode,
			image: None,
			proxy_uses: Uses::empty(),
			proxy_resource_uses: Uses::empty(),
			backbuffers: std::array::from_fn(|_| None),
			acquired_image_indices: [0; 8],
			acquired_sequences: [false; 8],
			queue_handle,
			present_interval: None,
			next_present_slot: None,
			flags,
			allows_tearing,
			latency_waitable,
			// No valid latency is 0, so the first acquisition sets it.
			frame_latency: 0,
			sync_interval: 1,
			refresh: None,
			shown_instant: None,
			last_target: None,
		});

		SwapchainHandle::from_index((self.swapchains.len() - 1) as u64)
	}

	/// Creates a factory that compiles shaders and pipelines for this context on another thread. See
	/// [`crate::dx12::factory::Factory`].
	pub fn create_factory(&mut self) -> Option<crate::dx12::factory::Factory> {
		Some(crate::dx12::factory::Factory {
			context: Device::from_native_parts(
				self.device.clone(),
				self.device_configuration.clone(),
				self.dxc_compiler.clone(),
				self.settings,
				self.info_queue.clone(),
				self.debug_log_function,
				Vec::new(),
				FACTORY_RESOURCE_HEAP_CAPACITY,
			),
		})
	}

	/// Returns one logical proxy whose dynamic storage resolves to the active frame sequence.
	pub fn get_swapchain_image(&mut self, swapchain_handle: SwapchainHandle, uses: Uses) -> (ImageHandle, Formats) {
		let swapchain = &self.swapchains[swapchain_handle.index() as usize];
		let requested_uses = swapchain.proxy_uses | uses;
		// A storage proxy is copied into the native backbuffer before Present. Other proxy uses should not
		// require optional typed-UAV support or unrelated render/copy capabilities from the BGRA format.
		let presentation_copy_uses = if requested_uses.intersects(Uses::Storage) {
			Uses::BlitSource
		} else {
			Uses::empty()
		};
		// Every proxy is a copy source, which needs no resource flag, so reading back a frame never replaces the proxy
		// it was rendered into.
		let resource_uses =
			swapchain.proxy_resource_uses | requested_uses | Uses::Image | Uses::TransferSource | presentation_copy_uses;
		let needs_new_proxy = swapchain.image.is_none() || !swapchain.proxy_resource_uses.contains(requested_uses);
		let extent = swapchain.extent;

		if needs_new_proxy {
			let image = self.build_image(
				crate::image::Builder::new(Formats::BGRAu8, resource_uses)
					.extent(extent)
					.copies(crate::Copies::PerFrame),
			);
			let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
			// One logical image with a copy per frame already resolves to distinct native resources for each frame sequence.
			swapchain.image = Some(image);
			swapchain.proxy_resource_uses = resource_uses;
			// Descriptors of the swapchain now name the new proxy's resources.
			self.mark_stale_descriptors(|descriptor, _| descriptor == WriteData::Swapchain(swapchain_handle));
		}
		let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
		swapchain.proxy_uses = requested_uses;
		(
			swapchain.image.expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub(crate) fn get_swapchain_image_for_sequence(
		&mut self,
		swapchain_handle: SwapchainHandle,
		uses: Uses,
		sequence_index: u8,
	) -> (ImageHandle, Formats) {
		let image = self.get_swapchain_image(swapchain_handle, uses);
		debug_assert!(sequence_index < self.frames);
		image
	}

	/// Waits for a recorded texture copy and returns its bytes.
	pub fn get_image_data(
		&mut self,
		texture_copy_handle: TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		self.wait_for_texture_copy_readback(texture_copy_handle);
		self.refresh_readback_texture_copies(None);
		// Keep native storage alive while submitted work is unresolved; only completed or failed mappings are consumed.
		if self.texture_readbacks.submitted(texture_copy_handle)?.0.resource.is_some() {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		let readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		if readback.mapping_failed {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		Ok(readback.data)
	}

	pub(crate) fn wait_for_texture_copy_readback(&mut self, texture_copy_handle: TextureCopyHandle) {
		let Some((synchronizer, value)) = self
			.texture_readbacks
			.get(texture_copy_handle)
			.filter(|readback| readback.resource.is_some())
			.and_then(|readback| readback.completion)
		else {
			return;
		};
		self.wait_for_private_synchronizer_value(synchronizer, value);
	}

	pub(crate) fn create_synchronizer_internal(&mut self, signaled: bool) -> crate::synchronizer::SynchronizerHandle {
		let handle = crate::synchronizer::SynchronizerHandle::from_index(self.synchronizers.len() as u64);
		let initial_value = if signaled { 1 } else { 0 };
		let fence = unsafe { self.device.CreateFence(initial_value, D3D12_FENCE_FLAGS(0)) }
			.expect("Failed to create a D3D12 fence. The most likely cause is that the device does not support fences.");
		self.synchronizers.push(Synchronizer {
			next: None,
			fence,
			value: initial_value,
			last_signal_queue: None,
		});
		handle
	}

	pub fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let master = SynchronizerHandle::from_index(self.synchronizers.len() as u64);
		let mut previous: Option<crate::synchronizer::SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.index() as usize].next = Some(handle);
			}
			previous = Some(handle);
		}
		self.synchronizer_masters.push(master);
		master
	}

	pub fn start_frame<'a>(&'a mut self, index: u64, synchronizer_handle: SynchronizerHandle) -> super::super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		assert!(
			!self.has_untracked_submission(),
			"DX12 frame reuse is unavailable after an untracked native submission. The most likely cause is that presentation or its terminal fence signal failed."
		);
		let previous = self.last_frame_synchronizers[frame_key.sequence_index as usize];
		if let Some(previous) = previous {
			self.wait_for_synchronizer_sequence(previous, frame_key.sequence_index);
		}
		if previous != Some(synchronizer_handle) {
			self.wait_for_synchronizer_sequence(synchronizer_handle, frame_key.sequence_index);
		}
		self.last_frame_synchronizers[frame_key.sequence_index as usize] = Some(synchronizer_handle);
		// The sequence's previous frame has completed, so its resolved timestamps are final.
		self.resolve_counters(frame_key.sequence_index);
		self.process_tasks(frame_key.sequence_index);
		self.reset_acceleration_structure_scratch(frame_key.sequence_index);
		self.advance_acceleration_structures();
		super::super::Frame::new(self, frame_key, synchronizer_handle)
	}

	/// Acquires the backbuffer that `frame` will present before the frame is started.
	///
	/// The sequence's fences are waited first so `ResizeBuffers` and backbuffer reuse see released resources,
	/// matching the guarantee `start_frame` gives for in-frame acquisition. Fences are monotonic, so the later
	/// `start_frame` wait returns immediately.
	pub fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		if let Some(previous) = self.last_frame_synchronizers[sequence_index as usize] {
			self.wait_for_synchronizer_sequence(previous, sequence_index);
		}
		self.wait_for_synchronizer_sequence(frame.synchronizer, sequence_index);
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	/// Acquires the next backbuffer of `swapchain_handle` and records it as owned by `sequence_index`.
	///
	/// Waits until the swapchain's present queue has room, so the tick starts when the display takes a frame, then
	/// picks the sync interval for the frame's present and reads when frames reach the display. Returns `None` when
	/// the queue stays full for [`ACQUIRE_TIMEOUT`], as happens when the compositor stops showing the window.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let frames = self.frames;
		let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
		assert!(
			swapchain.acquired_sequences.iter().all(|acquired| !acquired),
			"DX12 swapchain already has an acquired image. The most likely cause is that an earlier present key was not submitted."
		);
		// The swapchain starts without a latency, and frames in flight or the buffer count may change between
		// acquisitions, so the latency is set before the wait it governs.
		let frame_latency = pacing::frame_latency(frames, swapchain.image_count);
		if frame_latency != swapchain.frame_latency {
			// SAFETY: The swapchain was created with the waitable flag, which this call requires.
			unsafe { swapchain.swapchain.SetMaximumFrameLatency(frame_latency) }.unwrap_or_else(|_| {
				panic!("Failed to set the DXGI swapchain frame latency. The most likely cause is that the swapchain was created without a waitable object.")
			});
			swapchain.frame_latency = frame_latency;
		}
		// SAFETY: The handle stays open for the swapchain's lifetime. An alertable wait lets queued I/O completions run.
		let wait = unsafe { WaitForSingleObjectEx(*swapchain.latency_waitable, ACQUIRE_TIMEOUT.as_millis() as u32, true) };
		if wait != WAIT_OBJECT_0 {
			return None;
		}

		// A sync interval holds a frame-rate cap on the display's own refreshes. Only caps that are not a whole number
		// of refreshes, and presents that do not wait for a refresh, are held back by sleeping instead.
		let refresh = swapchain.refresh_interval();
		let (sync_interval, sleep_interval) = match swapchain.present_mode {
			PresentationModes::FIFO => match swapchain
				.present_interval
				.zip(refresh)
				.and_then(|(interval, refresh)| pacing::sync_interval_for(interval, refresh))
			{
				Some(sync_interval) => (sync_interval, None),
				None => (1, swapchain.present_interval),
			},
			PresentationModes::Mailbox | PresentationModes::Inmediate => (0, swapchain.present_interval),
		};
		swapchain.sync_interval = sync_interval;
		crate::swapchain::pace_present(&mut swapchain.next_present_slot, sleep_interval);

		let now = std::time::Instant::now();
		let shown = swapchain.shown_frame(now);
		// A frame lands on a predictable refresh only while presents wait for refreshes and nothing sleeps between them.
		let target_present_time = match (shown, refresh) {
			(Some(shown), Some(refresh)) if sync_interval > 0 && sleep_interval.is_none() => Some(pacing::predict_target(
				shown,
				sync_interval,
				refresh,
				now,
				swapchain.last_target,
			)),
			_ => None,
		};
		swapchain.last_target = target_present_time;
		let present_time = shown.map(|shown| shown.at);

		// ResizeBuffers invalidates every old backbuffer token, so ownership must be checked before extent maintenance.
		let extent = self.swapchain_extent(swapchain_handle, sequence_index);
		let image_index = self.next_swapchain_image_index(swapchain_handle);
		let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
		swapchain.acquired_image_indices[sequence_index as usize] = image_index;
		swapchain.acquired_sequences[sequence_index as usize] = true;
		Some(crate::frame::SwapchainAcquisition {
			present_key: PresentKey {
				image_index,
				sequence_index,
				swapchain: swapchain_handle,
			},
			extent,
			present_time,
			target_present_time,
		})
	}

	/// Replaces every copy of an array buffer with zero-initialized copies of `length` elements; see
	/// [`crate::context::Context::resize_buffer`].
	///
	/// The retired copies stay alive until every frame recorded before the resize completes.
	pub fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>, length: usize) {
		let size = T::layout(Some(length)).size();
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let buffer = self
			.buffer(buffer_handle)
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another context. Use the buffer only with the context that built it.");
		let (host_access, uses, name, count) = (
			buffer.host_access,
			buffer.uses,
			buffer.name.clone(),
			buffer.copies.count(self.frames),
		);
		// D3D12 zeroes committed resources, so the new copies start zero-initialized with no clear.
		let mut storages = (0..count)
			.map(|_| self.create_buffer_memory(size, host_access, uses, name.as_deref()))
			.collect::<Vec<_>>();
		let buffer = self.buffer_mut(buffer_handle).expect("The buffer was just read.");
		let first = storages.remove(0);
		let retired = std::iter::once(std::mem::replace(&mut buffer.memory, first))
			.chain(std::mem::replace(&mut buffer.frame_copies, storages))
			.collect::<SmallVec<[BufferMemory; crate::MAX_FRAMES_IN_FLIGHT]>>();
		buffer.size = size;

		let mut retired_state_keys = SmallVec::<[usize; crate::MAX_FRAMES_IN_FLIGHT]>::new();
		for storage in retired {
			// Dropping the storage only unmaps its CPU view; the GPU may still use the resource, which retires below.
			if let Some(resource) = storage.resource.as_ref() {
				retired_state_keys.push(Self::native_resource_key(resource));
				self.retire_resource(resource);
			}
		}
		self.invalidate_clear_uav_descriptors_for_resources(&retired_state_keys);
		for key in retired_state_keys {
			self.buffer_states.remove(&key);
		}
		self.mark_stale_descriptors(
			|descriptor, _| matches!(descriptor, WriteData::Buffer { handle } if handle == buffer_handle),
		);
	}

	/// Keeps `resource` alive until the next frame of every sequence starts, which waits for every frame recorded so
	/// far, since any of them may use it.
	pub(crate) fn retire_resource(&mut self, resource: &ID3D12Resource) {
		for sequence_index in 0..self.frames {
			self.defer_task(sequence_index, DeferredTask::RetireResource(resource.clone()));
		}
	}

	/// Establishes and waits for one terminal fence on every distinct native queue.
	pub(crate) fn wait_for_all_queues_idle(&mut self) -> windows::core::Result<()> {
		assert!(
			self.active_command_buffer.is_none()
				&& self.command_buffers.iter().all(|command_buffer| {
					!command_buffer.frames_any(|lifecycle| {
						matches!(
							lifecycle,
							CommandBufferLifecycle::Recording | CommandBufferLifecycle::Scheduled
						)
					})
				}),
			"DX12 queue-idle wait started during command recording. The most likely cause is that a resource topology change was requested before its execution finished collecting command buffers."
		);
		let mut terminal_fences = SmallVec::<[(usize, ID3D12Fence); 4]>::new();
		for queue in &self.queues {
			let identity = queue.queue.as_raw() as usize;
			if terminal_fences.iter().any(|(queued, _)| *queued == identity) {
				continue;
			}
			let fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAGS(0)) }?;
			unsafe { queue.queue.Signal(&fence, 1) }?;
			terminal_fences.push((identity, fence));
		}
		for (_, fence) in terminal_fences {
			if let Err(event_error) = unsafe { fence.SetEventOnCompletion(1, HANDLE::default()) } {
				// Event registration may fail under memory pressure. The queue signal is already ordered, so polling
				// its value is a safe, allocation-free fallback for this exceptional topology/shutdown path.
				loop {
					let completed = unsafe { fence.GetCompletedValue() };
					if completed == u64::MAX {
						return Err(unsafe { self.device.GetDeviceRemovedReason() }.err().unwrap_or(event_error));
					}
					if completed >= 1 {
						break;
					}
					std::thread::yield_now();
				}
			} else if unsafe { fence.GetCompletedValue() } == u64::MAX {
				return unsafe { self.device.GetDeviceRemovedReason() };
			}
		}
		Ok(())
	}

	/// Queues an unresolved fence wait so tests can prove that another frame sequence does not wait on it from the CPU.
	#[cfg(test)]
	pub(crate) fn block_queue_until_test_fence(&self, queue_handle: QueueHandle) -> windows::core::Result<ID3D12Fence> {
		let queue = self
			.queues
			.get(queue_handle.index() as usize)
			.expect("Invalid DX12 queue handle. The most likely cause is that the test used a queue from another device.");
		let fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAGS(0)) }?;
		unsafe { queue.queue.Wait(&fence, 1) }?;
		Ok(fence)
	}

	/// Resets completed command lists and releases engine-owned references before a resource topology change.
	pub(crate) fn prepare_for_topology_change(&mut self) -> windows::core::Result<()> {
		assert!(
			self.active_command_buffer.is_none()
				&& self.command_buffers.iter().all(|command_buffer| {
					!command_buffer.frames_any(|lifecycle| {
						!matches!(lifecycle, CommandBufferLifecycle::Idle | CommandBufferLifecycle::Submitted)
					})
				}),
			"DX12 topology is unavailable while command recording or an untracked native submission is active. The most likely cause is that a resize was requested before its execution completed reliably."
		);
		for command_buffer in &mut self.command_buffers {
			for frame in &mut command_buffer.frames {
				if let (Some(allocator), Some(command_list)) = (frame.allocator.as_ref(), frame.command_list.as_ref()) {
					if frame.is_open {
						if let Err(error) = unsafe { command_list.Close() } {
							// D3D12 permanently invalidates a command list whose Close call fails.
							frame.lifecycle = CommandBufferLifecycle::Poisoned;
							return Err(error);
						}
						frame.is_open = false;
					}
					unsafe { allocator.Reset() }?;
					unsafe { command_list.Reset(allocator, None) }?;
					if let Err(error) = unsafe { command_list.Close() } {
						// A failed Close permanently retires the list; do not publish it as reusable after topology maintenance.
						frame.lifecycle = CommandBufferLifecycle::Poisoned;
						return Err(error);
					}
				}
				frame.is_open = false;
				frame.lifecycle = CommandBufferLifecycle::Idle;
				frame.clear_recording_state();
				frame.release_clear_descriptors(&mut self.shader_visible_heaps);
			}
			command_buffer.active_sequence_index = 0;
		}
		self.untracked_present_work = false;
		Ok(())
	}

	/// Returns every fence allocated for a logical synchronizer, including dormant frame sequences.
	pub(crate) fn all_synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		crate::synchronizer::SynchronizerHandle::from_index(synchronizer_handle.index()).get_all(&self.synchronizers)
	}

	/// Returns only the fence chain visible to the current frames-in-flight topology.
	pub(crate) fn synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		let mut handles = self.all_synchronizer_handles(synchronizer_handle);
		handles.truncate(self.frames as usize);
		handles
	}

	pub(crate) fn synchronizer_for_sequence(
		&self,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) -> Option<crate::synchronizer::SynchronizerHandle> {
		let handles = self.synchronizer_handles(synchronizer_handle);
		handles.get(sequence_index as usize).or(handles.last()).copied()
	}

	pub(crate) fn wait_for_private_synchronizer(&self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.index() as usize) else {
			return;
		};
		self.wait_for_private_synchronizer_value(synchronizer_handle, synchronizer.value);
	}

	/// Reports whether a fence reached `value`, and panics when the device was removed.
	///
	/// A removed device makes every fence report `u64::MAX`, which would read as completed work. Readbacks would then
	/// return undefined bytes as valid data and retirement would free resources the GPU never released.
	pub(crate) fn fence_reached(device: &ID3D12Device10, fence: &ID3D12Fence, value: u64) -> bool {
		// SAFETY: The caller owns the live fence queried here.
		let completed = unsafe { fence.GetCompletedValue() };
		if completed == u64::MAX {
			let report = super::device_removed_report(device);
			panic!(
				"The DX12 device was removed while waiting for GPU work. The most likely cause is a GPU hang, a page fault, or a driver crash. Run with validation enabled to find the command that faulted.
{report}"
			);
		}
		completed >= value
	}

	/// Blocks without polling until one concrete fence reaches the captured submission value.
	pub(crate) fn wait_for_private_synchronizer_value(
		&self,
		synchronizer_handle: crate::synchronizer::SynchronizerHandle,
		value: u64,
	) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.index() as usize) else {
			return;
		};
		if Self::fence_reached(&self.device, &synchronizer.fence, value) {
			return;
		}
		// A null event asks D3D12 to block this thread until completion without a busy-yield loop.
		unsafe { synchronizer.fence.SetEventOnCompletion(value, HANDLE::default()) }.expect(
			"Failed to wait for a DX12 fence. The most likely cause is that the fence was invalid or the device was removed.",
		);
	}

	pub(crate) fn wait_for_synchronizer(&mut self, synchronizer_handle: SynchronizerHandle) {
		for handle in self.synchronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
		self.refresh_readback_texture_copies(None);
	}

	pub(crate) fn wait_for_synchronizer_sequence(&mut self, synchronizer_handle: SynchronizerHandle, sequence_index: u8) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.wait_for_private_synchronizer(handle);
		self.refresh_readback_texture_copies(Some(sequence_index));
	}

	pub(crate) fn synchronizer_value(&self, synchronizer_handle: SynchronizerHandle) -> Option<u64> {
		self.synchronizers
			.get(synchronizer_handle.index() as usize)
			.map(|synchronizer| synchronizer.value)
	}

	/// Validates acquired swapchain ownership and uniqueness before command recording or queue presentation mutates state.
	pub(crate) fn validate_present_keys(&self, queue_handle: QueueHandle, sequence_index: u8, present_keys: &[PresentKey]) {
		for (index, &present_key) in present_keys.iter().enumerate() {
			assert!(
				!present_keys[..index].contains(&present_key),
				"Duplicate DX12 present key. The most likely cause is that one execution returned the same acquired image more than once."
			);
			assert_eq!(
				present_key.sequence_index, sequence_index,
				"Invalid DX12 present sequence. The most likely cause is that a present key was reused by a different frame."
			);
			let swapchain = self.swapchains.get(present_key.swapchain.index() as usize).expect(
				"Invalid DX12 swapchain handle. The most likely cause is that the present key came from another device.",
			);
			assert_eq!(
				swapchain.queue_handle, queue_handle,
				"Invalid DX12 presentation queue. The most likely cause is that the swapchain was created for a different queue."
			);
			assert!(
				present_key.image_index < swapchain.image_count,
				"Invalid DX12 swapchain image index. The most likely cause is that the present key predates a swapchain resize."
			);
			let sequence = usize::from(sequence_index);
			assert!(
				swapchain.acquired_sequences[sequence],
				"DX12 swapchain image was not acquired. The most likely cause is that a present key was reused after presentation."
			);
			assert_eq!(
				swapchain.acquired_image_indices[sequence], present_key.image_index,
				"DX12 swapchain image no longer matches the acquired image. The most likely cause is that a stale present key was reused."
			);
		}
	}

	/// Checks that recorded proxy preparation and the final presentation set describe the same storage-present work.
	pub(crate) fn validate_present_preparation(&self, prepared: &[PresentKey], presented: &[PresentKey]) {
		for &present_key in prepared {
			assert!(
				presented.contains(&present_key),
				"Missing DX12 presentation request. The most likely cause is that a prepared swapchain image was omitted from the execution's returned present keys."
			);
		}
		for &present_key in presented {
			let uses_storage_proxy = self
				.swapchains
				.get(present_key.swapchain.index() as usize)
				.is_some_and(|swapchain| swapchain.proxy_uses.intersects(Uses::Storage));
			assert!(
				!uses_storage_proxy || prepared.contains(&present_key),
				"Missing DX12 storage presentation preparation. The most likely cause is that record_with_present_keys omitted a storage-backed swapchain image."
			);
		}
	}

	/// Rejects new work until a global idle boundary recovers any presentation without a terminal fence.
	pub(crate) fn validate_queue_submission_state(&self) {
		assert!(
			!self.has_untracked_submission(),
			"DX12 queue submission is unavailable after untracked native work. The most likely cause is that presentation, command-list closure, or its terminal fence signal failed."
		);
	}

	/// Returns whether native work may still run without a terminal fence that tracks its completion.
	fn has_untracked_submission(&self) -> bool {
		self.untracked_present_work
			|| self
				.command_buffers
				.iter()
				.any(|command_buffer| command_buffer.frames_any(|lifecycle| lifecycle == CommandBufferLifecycle::Poisoned))
	}

	/// Publishes that a successful terminal signal now tracks every presentation in the execution.
	pub(crate) fn complete_present_submission(&mut self, presented: bool) {
		if presented {
			self.untracked_present_work = false;
		}
	}

	pub(crate) fn begin_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle, sequence_index: u8) {
		self.validate_queue_submission_state();
		let command_buffer = self
			.command_buffers
			.get_mut(command_buffer_handle.index() as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		command_buffer.activate_sequence(sequence_index);
		let (lifecycle, last_submission) = (command_buffer.lifecycle, command_buffer.last_submission);
		assert!(
			matches!(
				lifecycle,
				CommandBufferLifecycle::Idle | CommandBufferLifecycle::Recording | CommandBufferLifecycle::Submitted
			),
			"DX12 command buffer cannot be reset. The most likely cause is that it is scheduled or its native submission has no reliable completion fence."
		);
		if let Some((synchronizer_handle, previous_sequence_index)) = last_submission {
			self.wait_for_synchronizer_sequence(synchronizer_handle, previous_sequence_index);
		}
		self.discard_command_buffer_recording(command_buffer_handle);

		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.index() as usize) else {
			return;
		};
		// One deref selects the active frame, so its fields can be borrowed independently below.
		let frame: &mut CommandBufferFrame = command_buffer;
		let (Some(allocator), Some(command_list)) = (frame.allocator.as_ref(), frame.command_list.as_ref()) else {
			return;
		};

		if frame.is_open {
			if unsafe { command_list.Close() }.is_err() {
				// Close failure permanently invalidates this native list. Poison the device-facing handle so it is never reset.
				frame.lifecycle = CommandBufferLifecycle::Poisoned;
				panic!(
					"Failed to close an abandoned DX12 command list. The most likely cause is that earlier command recording was invalid."
				);
			}
			frame.is_open = false;
		}
		unsafe { allocator.Reset() }.expect(
			"Failed to reset a DX12 command allocator. The most likely cause is that its previous GPU submission is still running.",
		);
		unsafe { command_list.Reset(allocator, None) }.expect(
			"Failed to reset a DX12 command list. The most likely cause is that the list was not closed or its allocator is invalid.",
		);
		let command_list = command_list.clone();
		// Reset removes recorded references before fence-complete transient resources and heaps are released.
		frame.clear_recording_state();
		frame.sequence_index = sequence_index;
		frame.rewind_clear_descriptors(&mut self.shader_visible_heaps);
		frame.is_open = true;
		frame.lifecycle = CommandBufferLifecycle::Recording;
		self.bind_shader_visible_heaps(&command_list);
		self.begin_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Associates a frame recording with the logical fence that must complete its sequence.
	pub(crate) fn set_command_buffer_frame_synchronizer(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		synchronizer_handle: SynchronizerHandle,
	) {
		let command_buffer = self
			.command_buffers
			.get_mut(command_buffer_handle.index() as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		command_buffer.frame_synchronizer = Some(synchronizer_handle);
	}

	/// Returns the native list that a command buffer records into, or `None` when the handle or its list is missing.
	pub(crate) fn command_list(&self, command_buffer_handle: CommandBufferHandle) -> Option<&ID3D12GraphicsCommandList7> {
		self.command_buffers
			.get(command_buffer_handle.index() as usize)?
			.command_list
			.as_ref()
	}

	/// Marks a command buffer as containing GPU-visible work that must be submitted.
	pub(crate) fn mark_command_buffer_work(&mut self, command_buffer_handle: CommandBufferHandle) {
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.index() as usize) {
			command_buffer.recorded_work = true;
		}
	}
}
