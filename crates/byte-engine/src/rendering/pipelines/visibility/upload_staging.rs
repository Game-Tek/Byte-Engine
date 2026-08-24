use std::{collections::VecDeque, sync::Arc};

/// The `UploadStagingArena` struct hands out exclusive slices of one persistently mapped GPU transfer-source buffer.
///
/// Request a [`StagingLease`] with [`Self::allocate`], load or convert the resource directly into that lease, then keep
/// the lease alive until the GPU transfer frame completes.
pub(crate) struct UploadStagingArena {
	byte_count: usize,
	commands: kanal::AsyncSender<StagingCommand>,
	returner: kanal::Sender<StagingCommand>,
}

/// The `UploadStagingWorker` struct exclusively owns upload-region allocation state.
pub(crate) struct UploadStagingWorker {
	available_regions: VecDeque<StagingRegion>,
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
	/// Creates an arena that exclusively owns a transferred GHI buffer mapping.
	pub(crate) fn new(mapping: ghi::buffer::Mapping) -> (Arc<Self>, UploadStagingWorker) {
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
				available_regions: VecDeque::from([region]),
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

	/// Waits until one aligned region is available or rejects a request larger than the complete arena.
	pub(crate) async fn allocate(self: &Arc<Self>, byte_count: usize, alignment: usize) -> Option<StagingLease> {
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
		let region = self
			.available_regions
			.remove(index)
			.expect("The staging region index was selected from this queue.");
		let aligned_offset = region.offset.next_multiple_of(alignment);
		let prefix_len = aligned_offset - region.offset;
		let suffix_len = region.byte_count - prefix_len - byte_count;
		let aligned_address = region
			.address
			.checked_add(prefix_len)
			.expect("Upload staging address overflowed. The most likely cause is a corrupted mapped region.");

		if prefix_len != 0 {
			self.available_regions.push_back(StagingRegion {
				offset: region.offset,
				address: region.address,
				byte_count: prefix_len,
			});
		}
		if suffix_len != 0 {
			self.available_regions.push_back(StagingRegion {
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
		self.available_regions.push_back(region);
		self.available_regions
			.make_contiguous()
			.sort_unstable_by_key(|region| region.offset);
		let mut coalesced = VecDeque::with_capacity(self.available_regions.len());
		while let Some(region) = self.available_regions.pop_front() {
			let Some(previous) = coalesced.pop_back() else {
				coalesced.push_back(region);
				continue;
			};
			if previous.offset + previous.byte_count == region.offset {
				coalesced.push_back(join_adjacent_regions(previous, region));
			} else {
				coalesced.push_back(previous);
				coalesced.push_back(region);
			}
		}
		self.available_regions = coalesced;
	}
}

/// Reconstructs one ownership token after both adjacent exclusive regions return to the arena.
fn join_adjacent_regions(left: StagingRegion, right: StagingRegion) -> StagingRegion {
	let total_len = left.byte_count + right.byte_count;

	assert_eq!(
		left.address + left.byte_count,
		right.address,
		"Adjacent upload staging offsets must also refer to adjacent mapped memory."
	);
	StagingRegion {
		offset: left.offset,
		address: left.address,
		byte_count: total_len,
	}
}

/// The `StagingLease` struct grants exclusive CPU access to one GPU upload-buffer region until its transfer completes.
pub(crate) struct StagingLease {
	region: Option<StagingRegion>,
	returner: kanal::Sender<StagingCommand>,
}

impl StagingLease {
	/// Returns the lease's absolute byte offset in the GPU upload buffer.
	pub(crate) fn offset(&self) -> usize {
		self.region
			.as_ref()
			.expect("Live staging leases retain their mapped region.")
			.offset
	}

	/// Returns exclusive CPU access to the persistently mapped region.
	#[allow(unsafe_code)]
	pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
		let region = self.region.as_mut().expect("Live staging leases retain their mapped region.");
		// The allocation worker only creates disjoint region tokens, and a lease
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
	fn full_arena_holds_requests_until_a_completed_lease_returns_capacity() {
		use std::{future::Future as _, task::Poll};

		let bytes = Box::leak(vec![0u8; 64].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let complete = arena.allocate(64, 16).await.expect("complete arena lease");
			let mut blocked = std::pin::pin!(arena.allocate(16, 16));
			let mut context = std::task::Context::from_waker(std::task::Waker::noop());

			assert!(matches!(blocked.as_mut().poll(&mut context), Poll::Pending));

			drop(complete);
			let reused = blocked
				.await
				.expect("A returned staging lease must satisfy the blocked request.");

			assert_eq!(reused.offset(), 0);
		});
	}

	#[test]
	fn simultaneous_leases_own_disjoint_mapped_slices() {
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
		});
	}

	#[test]
	fn adjacent_returned_leases_coalesce_for_a_larger_waiting_request() {
		let bytes = Box::leak(vec![0u8; 64].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let first = arena.allocate(24, 16).await.expect("first lease");
			let second = arena.allocate(24, 16).await.expect("second lease");
			drop(first);
			drop(second);
			let complete = arena.allocate(64, 16).await.expect("coalesced lease");

			assert_eq!(complete.offset(), 0);
		});
	}

	#[test]
	fn cancelled_allocation_returns_its_granted_region_to_the_owner() {
		use std::future::Future as _;

		let bytes = Box::leak(vec![0u8; 64].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("staging test executor");
		executor.block_on(async {
			let (arena, worker) = UploadStagingArena::new_for_test(bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let complete = arena.allocate(64, 16).await.expect("complete arena lease");
			let mut cancelled = Box::pin(arena.allocate(64, 16));
			let mut context = std::task::Context::from_waker(std::task::Waker::noop());

			assert!(cancelled.as_mut().poll(&mut context).is_pending());
			drop(cancelled);
			drop(complete);

			let reused = arena
				.allocate(64, 16)
				.await
				.expect("A cancelled staging request must not leak its granted region.");
			assert_eq!(reused.offset(), 0);
		});
	}
}
