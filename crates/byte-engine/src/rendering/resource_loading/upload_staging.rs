use std::{collections::VecDeque, sync::Arc};

use ghi::Device as _;

/// The `UploadStagingArena` struct gives loader lanes exclusive regions of one persistently mapped transfer buffer.
///
/// Share this lightweight client with asynchronous loader lanes. Request a
/// [`StagingLease`] with [`Self::allocate`], load or convert directly into its
/// bytes, and keep the lease alive through the loader transfer. When the loader
/// drops the lease after GPU completion, its region returns to
/// [`UploadStagingWorker`] for coalescing and reuse.
///
/// The arena owns raw access transferred from one GHI mapping, not the backing
/// buffer or context. Keep the GHI context and mapped buffer alive until the
/// arena, its worker, and every lease have been dropped.
pub struct UploadStagingArena {
	byte_count: usize,
	commands: kanal::AsyncSender<StagingCommand>,
	returner: kanal::Sender<StagingCommand>,
}

/// The `UploadStagingWorker` struct serializes allocation and reclamation for one mapped staging arena.
///
/// Run one worker per [`UploadStagingArena`] on an application-owned async task.
/// Keeping free-region state here lets loader lanes share the arena without
/// placing synchronization inside GHI or exposing mapped pointers across the
/// public allocation API. The worker exits after every arena client and lease
/// return channel has been dropped.
pub struct UploadStagingWorker {
	available_regions: Vec<StagingRegion>,
	pending_allocations: VecDeque<StagingAllocationRequest>,
	commands: kanal::AsyncReceiver<StagingCommand>,
}

struct StagingRegion {
	offset: usize,
	address: usize,
	byte_count: usize,
}

struct StagingAllocationRequest {
	byte_count: usize,
	alignment: usize,
	response: kanal::Sender<StagingRegion>,
}

enum StagingCommand {
	Allocate(StagingAllocationRequest),
	Return(StagingRegion),
}

impl UploadStagingArena {
	/// Creates a host-mapped GHI upload buffer and its staging client and worker.
	///
	/// Run the returned worker on an application-owned task. Pass the returned
	/// buffer and arena to the pipeline loader that records transfers.
	pub fn create<const BYTE_COUNT: usize>(
		context: &mut ghi::implementation::Context,
		name: &str,
	) -> (ghi::BaseBufferHandle, Arc<Self>, UploadStagingWorker) {
		let buffer: ghi::BufferHandle<[u8; BYTE_COUNT]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::TransferSource)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::HostOnly),
		);
		// SAFETY: The arena becomes the only CPU owner of this mapping and keeps
		// each leased region exclusive until its GPU transfer completes.
		#[allow(unsafe_code)]
		let mapping = unsafe { context.transfer_buffer_mapping(buffer) };
		let (arena, worker) = Self::new(mapping);
		(buffer.into(), arena, worker)
	}

	/// Creates the client and worker halves for one transferred GHI buffer mapping.
	///
	/// The mapping must cover the upload buffer used as the source in the
	/// renderer's loader transfer commands. Next, run
	/// [`UploadStagingWorker::run`] on an application-owned task and retain the
	/// mapped buffer handle in the renderer that records copies.
	fn new(mapping: ghi::buffer::Mapping) -> (Arc<Self>, UploadStagingWorker) {
		let (address, byte_count) = mapping.into_raw_parts();
		Self::from_region(StagingRegion {
			offset: 0,
			address,
			byte_count,
		})
	}

	fn from_region(region: StagingRegion) -> (Arc<Self>, UploadStagingWorker) {
		let byte_count = region.byte_count;
		let (commands, command_receiver) = kanal::unbounded_async();
		(
			Arc::new(Self {
				byte_count,
				returner: commands.clone().to_sync(),
				commands,
			}),
			UploadStagingWorker {
				available_regions: vec![region],
				pending_allocations: VecDeque::new(),
				commands: command_receiver,
			},
		)
	}

	#[cfg(test)]
	pub(crate) fn new_for_test(bytes: &'static mut [u8]) -> (Arc<Self>, UploadStagingWorker) {
		Self::from_region(StagingRegion {
			offset: 0,
			address: bytes.as_mut_ptr() as usize,
			byte_count: bytes.len(),
		})
	}

	/// Waits for one aligned exclusive region or rejects a request larger than the complete arena.
	///
	/// Allocation requests are served in FIFO order. A large request at the head
	/// can therefore hold smaller requests until returned regions coalesce; this
	/// favors predictable ordering over opportunistic reordering. `alignment`
	/// must be a non-zero power of two. `None` means the complete arena is too
	/// small or its worker has stopped.
	pub async fn allocate(self: &Arc<Self>, byte_count: usize, alignment: usize) -> Option<StagingLease> {
		assert!(
			alignment.is_power_of_two(),
			"Upload staging alignment must be a non-zero power of two."
		);
		if byte_count > self.byte_count {
			return None;
		}

		let (response, region) = kanal::bounded_async(1);
		self.commands
			.send(StagingCommand::Allocate(StagingAllocationRequest {
				byte_count,
				alignment,
				response: response.to_sync(),
			}))
			.await
			.ok()?;
		Some(StagingLease {
			region: Some(region.recv().await.ok()?),
			returner: self.returner.clone(),
		})
	}
}

impl UploadStagingWorker {
	/// Serves allocation and return messages until every staging client is dropped.
	///
	/// Move this future to the same application-owned runtime as loader lanes. Do
	/// not run two workers for one arena because this value is the
	/// exclusive owner of free-region state.
	pub async fn run(mut self) {
		while let Ok(command) = self.commands.recv().await {
			match command {
				StagingCommand::Allocate(request) => self.pending_allocations.push_back(request),
				StagingCommand::Return(region) => self.return_region(region),
			}
			self.satisfy_pending_allocations();
		}
	}

	/// Grants pending requests in FIFO order while the head request fits.
	fn satisfy_pending_allocations(&mut self) {
		loop {
			let Some(request) = self.pending_allocations.pop_front() else {
				return;
			};
			let Some(region) = self.try_take_region(request.byte_count, request.alignment) else {
				self.pending_allocations.push_front(request);
				return;
			};
			let mut region = Some(region);
			if !matches!(request.response.try_send_option(&mut region), Ok(true)) {
				self.return_region(region.expect("An undelivered staging response must retain its region."));
			}
		}
	}

	/// Splits one available mapped slice for an exclusive lease.
	fn try_take_region(&mut self, byte_count: usize, alignment: usize) -> Option<StagingRegion> {
		let index = self.available_regions.iter().position(|region| {
			let aligned_offset = region.offset.next_multiple_of(alignment);
			aligned_offset
				.checked_add(byte_count)
				.is_some_and(|end| end <= region.offset + region.byte_count)
		})?;
		let region = self.available_regions.remove(index);
		let aligned_offset = region.offset.next_multiple_of(alignment);
		let prefix_len = aligned_offset - region.offset;
		let suffix_len = region.byte_count - prefix_len - byte_count;
		let aligned_address = region
			.address
			.checked_add(prefix_len)
			.expect("Upload staging address overflowed. The most likely cause is a corrupted mapped region.");

		if prefix_len != 0 {
			self.available_regions.push(StagingRegion {
				offset: region.offset,
				address: region.address,
				byte_count: prefix_len,
			});
		}
		if suffix_len != 0 {
			self.available_regions.push(StagingRegion {
				offset: aligned_offset + byte_count,
				address: aligned_address + byte_count,
				byte_count: suffix_len,
			});
		}

		Some(StagingRegion {
			offset: aligned_offset,
			address: aligned_address,
			byte_count,
		})
	}

	fn return_region(&mut self, region: StagingRegion) {
		self.available_regions.push(region);
		self.available_regions.sort_unstable_by_key(|region| region.offset);
		self.available_regions.dedup_by(|right, left| {
			if left.offset + left.byte_count != right.offset {
				return false;
			}
			// Reconstruct one ownership token after both adjacent exclusive regions return to the arena.
			assert_eq!(
				left.address + left.byte_count,
				right.address,
				"Adjacent staging offsets must map adjacent memory."
			);
			left.byte_count += right.byte_count;
			true
		});
	}
}

/// The `StagingLease` struct ties exclusive mapped bytes to their GPU-use lifetime.
///
/// Fill the region through [`Self::bytes_mut`], use [`Self::offset`] when
/// recording a copy from the arena's backing buffer, and move the lease into
/// the prepared upload. Do not free it manually. Dropping the lease returns the
/// region to the worker, so the loader must keep it until the transfer completes.
pub struct StagingLease {
	region: Option<StagingRegion>,
	returner: kanal::Sender<StagingCommand>,
}

impl StagingLease {
	/// Returns the lease's absolute byte offset in the GPU upload buffer.
	///
	/// Add renderer-specific subrange offsets to this value when building copy
	/// descriptors.
	pub fn offset(&self) -> usize {
		self.region
			.as_ref()
			.expect("Live staging leases retain their mapped region.")
			.offset
	}

	/// Returns exclusive CPU access to the persistently mapped region.
	///
	/// Finish all writes before recording the loader transfer. The
	/// exclusive borrow prevents concurrent safe access through this lease.
	#[allow(unsafe_code)]
	pub fn bytes_mut(&mut self) -> &mut [u8] {
		let region = self.region.as_mut().expect("Live staging leases retain their mapped region.");
		// SAFETY: The allocation worker only creates disjoint region tokens, and a lease
		// provides mutable access through one exclusive `&mut self` at a time.
		unsafe { std::slice::from_raw_parts_mut(region.address as *mut u8, region.byte_count) }
	}
}

impl Drop for StagingLease {
	fn drop(&mut self) {
		if let Some(region) = self.region.take() {
			let _ = self.returner.send(StagingCommand::Return(region));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn leases_remain_disjoint_and_return_capacity_after_completion_or_cancellation() {
		use std::{future::Future as _, task::Poll};

		let bytes = Box::leak(vec![0u8; 64].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let mut first = arena.allocate(24, 16).await.expect("first lease");
			let mut second = arena.allocate(24, 16).await.expect("second lease");
			assert_eq!(first.offset(), 0);
			assert_eq!(second.offset(), 32);
			first.bytes_mut().fill(3);
			second.bytes_mut().fill(7);
			assert!(first.bytes_mut().iter().all(|byte| *byte == 3));
			assert!(second.bytes_mut().iter().all(|byte| *byte == 7));
			drop(first);
			drop(second);
			let complete = arena.allocate(64, 16).await.expect("coalesced lease");
			assert_eq!(complete.offset(), 0);
			let mut context = std::task::Context::from_waker(std::task::Waker::noop());
			let mut blocked = Box::pin(arena.allocate(64, 16));
			assert!(matches!(blocked.as_mut().poll(&mut context), Poll::Pending));
			drop(complete);
			let reused = blocked.await.expect("A returned lease must satisfy the blocked request.");
			let mut cancelled = Box::pin(arena.allocate(64, 16));
			assert!(matches!(cancelled.as_mut().poll(&mut context), Poll::Pending));
			drop(cancelled);
			drop(reused);
			let reused = arena
				.allocate(64, 16)
				.await
				.expect("A cancelled staging request must not leak its granted region.");
			assert_eq!(reused.offset(), 0);
		});
	}
}
