use std::{
	collections::VecDeque,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};

use ghi::context::{Context as _, ContextCreate as _};

/// The `UploadStagingArena` struct gives loader lanes exclusive regions of one persistently mapped transfer buffer.
///
/// Share this lightweight client with asynchronous loader lanes. Request a
/// [`StagingLease`] with [`Self::allocate`], load or convert directly into its
/// bytes, and hand the lease to the loader with the upload that reads it. The
/// [`Loader`](crate::rendering::loading::Loader) drops the lease once the GPU
/// finished the copy, and its region returns to [`UploadStagingWorker`] for
/// coalescing and reuse.
///
/// The arena owns raw access transferred from one GHI mapping, not the backing
/// buffer or context. Keep the GHI context and mapped buffer alive until the
/// arena, its worker, and every lease have been dropped.
pub struct UploadStagingArena {
	byte_count: usize,
	commands: kanal::AsyncSender<StagingCommand>,
	returner: kanal::Sender<StagingCommand>,
	exhausted: Arc<AtomicBool>,
}

/// The `UploadStagingWorker` struct serializes allocation and reclamation for one mapped staging arena.
///
/// The [`Loader`](crate::rendering::loading::Loader) runs the one worker of its arena on the loading thread.
/// Keeping free-region state here lets loader lanes share the arena without
/// placing synchronization inside GHI or exposing mapped pointers across the
/// public allocation API. The worker exits after every arena client and lease
/// return channel has been dropped.
pub(crate) struct UploadStagingWorker {
	/// The mapped address of arena offset zero.
	base_address: usize,
	free_bytes: utils::RangeAllocator,
	pending_allocations: VecDeque<StagingAllocationRequest>,
	commands: kanal::AsyncReceiver<StagingCommand>,
	exhausted: Arc<AtomicBool>,
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
	/// Run the returned worker on the loading thread. Record copies from the
	/// returned buffer in the same `context`.
	pub(crate) fn create(
		context: &mut ghi::implementation::Context,
		byte_count: usize,
		name: &str,
	) -> (ghi::BaseBufferHandle, Arc<Self>, UploadStagingWorker) {
		let buffer: ghi::BufferHandle<[u8]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::TransferSource)
				.name(name)
				.length(byte_count)
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
		Self::from_mapped_bytes(address, byte_count)
	}

	fn from_mapped_bytes(base_address: usize, byte_count: usize) -> (Arc<Self>, UploadStagingWorker) {
		let (commands, command_receiver) = kanal::unbounded_async();
		let exhausted = Arc::new(AtomicBool::new(false));
		(
			Arc::new(Self {
				byte_count,
				returner: commands.clone().to_sync(),
				commands,
				exhausted: exhausted.clone(),
			}),
			UploadStagingWorker {
				base_address,
				free_bytes: utils::RangeAllocator::new(byte_count, 1),
				pending_allocations: VecDeque::new(),
				commands: command_receiver,
				exhausted,
			},
		)
	}

	/// Returns whether an allocation is waiting for regions that only finished uploads can return.
	///
	/// The loader submits its collected uploads early when this is `true`, because waiting longer cannot free
	/// staging space.
	pub(crate) fn is_exhausted(&self) -> bool {
		self.exhausted.load(Ordering::Relaxed)
	}

	/// Creates the client and worker halves over a caller-owned test buffer.
	///
	/// Like [`Self::create`], the arena keeps only the buffer's address, so the caller must keep
	/// `bytes` alive until the arena, its worker, and every lease have been dropped.
	#[cfg(test)]
	pub(crate) fn new_for_test(bytes: &mut [u8]) -> (Arc<Self>, UploadStagingWorker) {
		Self::from_mapped_bytes(bytes.as_mut_ptr() as usize, bytes.len())
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
	pub(crate) async fn run(mut self) {
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
				self.exhausted.store(false, Ordering::Relaxed);
				return;
			};
			let Some(region) = self.try_take_region(request.byte_count, request.alignment) else {
				self.pending_allocations.push_front(request);
				self.exhausted.store(true, Ordering::Relaxed);
				return;
			};
			let mut region = Some(region);
			if !matches!(request.response.try_send_option(&mut region), Ok(true)) {
				self.return_region(region.expect("An undelivered staging response must retain its region."));
			}
		}
	}

	/// Leases one aligned slice of the mapped arena.
	fn try_take_region(&mut self, byte_count: usize, alignment: usize) -> Option<StagingRegion> {
		let range = self.free_bytes.take(byte_count, alignment)?;
		Some(StagingRegion {
			offset: range.start,
			address: self.base_address + range.start,
			byte_count,
		})
	}

	fn return_region(&mut self, region: StagingRegion) {
		self.free_bytes.give_back(region.offset..region.offset + region.byte_count);
	}
}

/// The `StagingLease` struct ties exclusive mapped bytes to their GPU-use lifetime.
///
/// Fill the region through [`Self::bytes_mut`], describe where each part lies
/// relative to the start of the lease, and hand the lease to the loader with the
/// upload that reads it. Do not free it manually. Dropping the lease returns the
/// region to the worker, so the loader keeps it until the copy completes.
pub struct StagingLease {
	region: Option<StagingRegion>,
	returner: kanal::Sender<StagingCommand>,
}

impl StagingLease {
	/// Returns the lease's absolute byte offset in the GPU upload buffer.
	///
	/// The loader adds the lease-relative offsets of an upload to this value when it records the copies.
	pub(crate) fn offset(&self) -> usize {
		self.region
			.as_ref()
			.expect("Live staging leases retain their mapped region.")
			.offset
	}

	/// Returns exclusive CPU access to the persistently mapped region.
	///
	/// Finish all writes before handing the lease to the loader. The
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

		let mut bytes = vec![0u8; 64];
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(&mut bytes);
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

	#[test]
	fn full_arena_reports_exhaustion_until_a_lease_returns() {
		use std::{future::Future as _, task::Poll};

		let mut bytes = vec![0u8; 64];
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(&mut bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let full = arena.allocate(64, 16).await.expect("full lease");
			assert!(!arena.is_exhausted(), "A granted request must not report exhaustion.");

			let mut context = std::task::Context::from_waker(std::task::Waker::noop());
			let mut waiting = Box::pin(arena.allocate(16, 16));
			assert!(matches!(waiting.as_mut().poll(&mut context), Poll::Pending));
			// The worker runs on this executor, so give it turns until it sees the request it cannot grant.
			for _ in 0..16 {
				if arena.is_exhausted() {
					break;
				}
				crate::core::async_runtime::yield_now().await;
			}
			assert!(
				arena.is_exhausted(),
				"A request the arena cannot grant must report exhaustion."
			);

			drop(full);
			let lease = waiting.await.expect("A returned lease must satisfy the waiting request.");
			assert!(!arena.is_exhausted(), "Exhaustion must clear once every request was granted.");
			drop(lease);
		});
	}
}
