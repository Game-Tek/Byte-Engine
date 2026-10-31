//! Memory for the resources a context creates.
//!
//! A resource created on the Metal device gets an allocation of its own, which Metal rounds up to whole pages and
//! the kernel tracks one by one. An [`Allocator`] creates heaps instead and places resources inside them. Small
//! resources share a heap, so they share pages and creating one does not reach the kernel. A large resource gets a
//! heap to itself.
//!
//! [`::utils::RangeAllocator`] decides where a resource goes inside a shared heap. This module owns those heaps,
//! creates the resources, and finds out when a resource's memory can be used again.

use objc2::Message;
use objc2::rc::Weak;
use objc2::runtime::ImplementedBy;
use objc2_metal::MTLHeap as _;

use super::*;

/// The largest resource that shares a heap. A larger one gets a heap of exactly its size.
///
/// Metal makes a heap resident as a whole. A shared heap therefore costs the memory of every resource in it from
/// the moment a command uses one of them, while a resource in a heap of its own costs nothing until it is used and
/// gives its memory back as soon as it is gone. Small resources gain more from sharing pages than they lose to
/// that; large ones only lose. In the model viewer's Sponza scene, sharing resources of up to 8 MiB raised the
/// process footprint by 9 %, and sharing resources of up to 1 MiB by less than 1 %.
const LARGEST_SHARED_ALLOCATION: u64 = 1 << 20;

/// The size of the first heap that resources share. A context that creates little must not pay for a large heap.
const SMALLEST_HEAP_SIZE: u64 = 1 << 20;

/// The size shared heaps grow to as a context creates more resources; see [`Allocator::allocate`]. It bounds the
/// memory a context holds in a heap it has only started to fill.
const LARGEST_HEAP_SIZE: u64 = 8 << 20;

/// How many resources [`Allocator::collect`] looks at when a frame starts. It bounds what a frame pays, and still
/// reaches every resource of a large scene within a few seconds.
pub(crate) const COLLECTION_STEP: usize = 128;

/// The `Allocator` struct gives the buffers, textures, and acceleration structures of one context their memory, in
/// heaps it creates as they fill up and releases once they are empty.
///
/// Create resources with [`Self::buffer`], [`Self::texture`], and [`Self::acceleration_structure`]. Nothing frees
/// them: a resource's memory is used again once the last reference to its Metal object is gone, which the
/// allocator notices in [`Self::collect`]. Commands in flight and other contexts hold such references, so memory is
/// never reused while the GPU can still reach the resource it held.
///
/// Commands make these resources resident like any other resource: [`queue::QueueResidency`] makes the heap of a
/// resource resident in its place.
pub(crate) struct Allocator {
	device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	/// The heaps resources share. A large resource owns its heap, so the allocator keeps no record of it.
	heaps: Vec<Heap>,
	/// How many resources the heaps hold.
	occupants: usize,
	/// The heap, and the occupant in it, that [`Self::collect`] looks at next.
	cursor: (usize, usize),
	debug_labels: bool,
}

/// The `Heap` struct is one shared Metal heap with the bookkeeping of what lives in it.
struct Heap {
	heap: Retained<ProtocolObject<dyn mtl::MTLHeap>>,
	storage: mtl::MTLStorageMode,
	/// The free bytes of the heap. The heap's size fits in `usize` because Metal sizes heaps with `NSUInteger`.
	free: ::utils::RangeAllocator,
	occupants: Vec<Occupant>,
}

/// The `Occupant` struct records the bytes one resource holds in a heap, and tells when the resource is gone.
struct Occupant {
	offset: u64,
	size: u64,
	resource: Weak<ProtocolObject<dyn mtl::MTLAllocation>>,
}

impl Allocator {
	pub(crate) fn new(device: Retained<ProtocolObject<dyn mtl::MTLDevice>>, debug_labels: bool) -> Self {
		Self {
			device,
			heaps: Vec::new(),
			occupants: 0,
			cursor: (0, 0),
			debug_labels,
		}
	}

	/// Creates a buffer of `length` bytes with the storage mode `options` selects.
	///
	/// A private buffer is zeroed. A CPU-visible buffer may hold what another resource left in its memory, so clear
	/// or fill it.
	pub(crate) fn buffer(
		&mut self,
		length: usize,
		options: mtl::MTLResourceOptions,
	) -> Retained<ProtocolObject<dyn mtl::MTLBuffer>> {
		let storage = if options.contains(mtl::MTLResourceOptions::StorageModePrivate) {
			mtl::MTLStorageMode::Private
		} else {
			mtl::MTLStorageMode::Shared
		};
		let requirements = self.device.heapBufferSizeAndAlignWithLength_options(length, options);
		// Buffers promise zeroes, which only new memory holds, and the CPU cannot clear a private buffer. Such a
		// buffer therefore never shares a heap, whose memory is reused.
		let shares = storage == mtl::MTLStorageMode::Shared;
		self.allocate("buffer", storage, shares, requirements, |heap, offset| {
			// SAFETY: The allocator placed the buffer inside the heap, at an offset aligned to the alignment Metal
			// reported for this length and these options.
			unsafe { heap.newBufferWithLength_options_offset(length, options, offset) }
		})
	}

	/// Creates the texture `descriptor` describes, with the storage mode it names. Its contents are undefined.
	pub(crate) fn texture(&mut self, descriptor: &mtl::MTLTextureDescriptor) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
		let requirements = self.device.heapTextureSizeAndAlignWithDescriptor(descriptor);
		self.allocate("texture", descriptor.storageMode(), true, requirements, |heap, offset| {
			// SAFETY: The allocator placed the texture inside the heap, at an offset aligned to the alignment Metal
			// reported for this descriptor.
			unsafe { heap.newTextureWithDescriptor_offset(descriptor, offset) }
		})
	}

	/// Creates the storage of an acceleration structure of `size` bytes. A build fills it later.
	pub(crate) fn acceleration_structure(
		&mut self,
		size: usize,
	) -> Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>> {
		// SAFETY: The size comes from the device's own sizing of an acceleration structure descriptor.
		let requirements = unsafe { self.device.heapAccelerationStructureSizeAndAlignWithSize(size) };
		let storage = mtl::MTLStorageMode::Private;
		self.allocate("acceleration structure", storage, true, requirements, |heap, offset| {
			// SAFETY: The allocator placed the structure inside the heap, at an offset aligned to the alignment Metal
			// reported for this size.
			unsafe { heap.newAccelerationStructureWithSize_offset(size, offset) }
		})
	}

	/// Finds memory that meets `requirements` in a heap of the `storage` mode and creates a resource there with
	/// `create`, which receives the heap and the resource's offset in it.
	///
	/// A resource that is large, or that does not share, gets a new heap of its size, whose memory is zeroed. Any
	/// other goes to the first shared heap with room, where it may find what an earlier resource left. When none
	/// has room, the memory of resources that are gone is collected, and if that makes no room either, a new shared
	/// heap is created: as large as the shared heaps of its storage mode together, between [`SMALLEST_HEAP_SIZE`]
	/// and [`LARGEST_HEAP_SIZE`]. Shared heaps therefore grow with the context, and what the newest one has not
	/// filled yet stays a fraction of what the context shares.
	fn allocate<T>(
		&mut self,
		kind: &str,
		storage: mtl::MTLStorageMode,
		shares: bool,
		requirements: mtl::MTLSizeAndAlign,
		create: impl FnOnce(&ProtocolObject<dyn mtl::MTLHeap>, usize) -> Option<Retained<T>>,
	) -> Retained<T>
	where
		T: Message + 'static,
		dyn mtl::MTLAllocation: ImplementedBy<T>,
	{
		let (size, alignment) = (requirements.size as u64, requirements.align as u64);
		let created = |resource: Option<Retained<T>>| {
			resource.unwrap_or_else(|| {
				panic!("Metal {kind} creation failed. The most likely cause is that the device is out of memory.")
			})
		};
		if !shares || size > LARGEST_SHARED_ALLOCATION {
			// The resource keeps its heap alive and the heap ends with it, so there is nothing to keep track of.
			return created(create(&self.create_heap(storage, size), 0));
		}

		let place = |heaps: &mut [Heap]| {
			heaps
				.iter_mut()
				.enumerate()
				.filter(|(_, heap)| heap.storage == storage)
				.find_map(|(index, heap)| {
					let range = heap.free.take(size as usize, alignment.max(1) as usize)?;
					Some((index, range.start as u64))
				})
		};
		let mut placement = place(&mut self.heaps);
		if placement.is_none() {
			self.collect(usize::MAX);
			placement = place(&mut self.heaps);
		}
		let (index, offset) = placement.unwrap_or_else(|| {
			let heaps = self.heaps.iter().filter(|heap| heap.storage == storage);
			let held: u64 = heaps.map(|heap| heap.heap.size() as u64).sum();
			let heap_size = held.clamp(SMALLEST_HEAP_SIZE, LARGEST_HEAP_SIZE);
			self.heaps.push(Heap {
				heap: self.create_heap(storage, heap_size),
				storage,
				free: ::utils::RangeAllocator::new(heap_size as usize, 1),
				occupants: Vec::new(),
			});
			place(&mut self.heaps).unwrap_or_else(|| {
				unreachable!("A new shared heap is larger than any resource that shares one, and starts aligned.")
			})
		});

		let heap = &mut self.heaps[index];
		let resource = created(create(&heap.heap, offset as usize));
		heap.occupants.push(Occupant {
			offset,
			size,
			resource: Weak::new(ProtocolObject::from_ref(&*resource)),
		});
		self.occupants += 1;

		resource
	}

	/// Creates an empty Metal heap of `size` bytes.
	fn create_heap(&self, storage: mtl::MTLStorageMode, size: u64) -> Retained<ProtocolObject<dyn mtl::MTLHeap>> {
		let descriptor = mtl::MTLHeapDescriptor::new();
		descriptor.setType(mtl::MTLHeapType::Placement);
		descriptor.setStorageMode(storage);
		// Recording tracks the hazards of every resource itself, so Metal's own tracking is not needed.
		descriptor.setHazardTrackingMode(mtl::MTLHazardTrackingMode::Untracked);
		descriptor.setSize(size as usize);
		let heap = self
			.device
			.newHeapWithDescriptor(&descriptor)
			.expect("Metal heap creation failed. The most likely cause is that the device is out of memory.");
		if cfg!(debug_assertions) && self.debug_labels {
			heap.setLabel(Some(&objc2_foundation::NSString::from_str("Context Heap")));
		}
		heap
	}

	/// Frees the memory of resources that are gone, and releases the shared heaps that held nothing else.
	///
	/// Looks at up to `limit` resources and continues where the previous call stopped, so calling it with a small
	/// limit every frame reaches every resource in time at a bounded cost.
	pub(crate) fn collect(&mut self, limit: usize) {
		let mut remaining = limit.min(self.occupants);
		while remaining > 0 {
			let (heap_index, occupant_index) = self.cursor;
			let Some(heap) = self.heaps.get_mut(heap_index) else {
				self.cursor = (0, 0);
				continue;
			};
			let Some(occupant) = heap.occupants.get(occupant_index) else {
				self.cursor = (heap_index + 1, 0);
				continue;
			};
			remaining -= 1;
			if occupant.resource.load().is_some() {
				self.cursor.1 += 1;
				continue;
			}

			// Another occupant takes this one's place in the list, so the cursor stays where it is.
			let occupant = heap.occupants.swap_remove(occupant_index);
			heap.free
				.give_back(occupant.offset as usize..(occupant.offset + occupant.size) as usize);
			self.occupants -= 1;
			if heap.occupants.is_empty() {
				// Another heap takes this one's place in the list.
				self.heaps.swap_remove(heap_index);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_allocator() -> Allocator {
		let (context, _) = crate::metal::test_context(crate::WorkloadTypes::TRANSFER);
		Allocator::new(context.device.clone(), false)
	}

	fn heap(buffer: &ProtocolObject<dyn mtl::MTLBuffer>) -> Retained<ProtocolObject<dyn mtl::MTLHeap>> {
		buffer
			.heap()
			.expect("Buffer is not in a heap. The most likely cause is that the allocator created it on the device.")
	}

	/// Returns the heap, by address, and the offset in it that hold `buffer`.
	fn placement(buffer: &ProtocolObject<dyn mtl::MTLBuffer>) -> (usize, usize) {
		(Retained::as_ptr(&heap(buffer)).cast::<()>() as usize, buffer.heapOffset())
	}

	#[test]
	fn small_resources_share_a_heap_without_sharing_memory() {
		let mut allocator = test_allocator();
		let options = mtl::MTLResourceOptions::StorageModeShared;

		let first = allocator.buffer(1000, options);
		let second = allocator.buffer(1000, options);
		allocator.collect(usize::MAX);
		let third = allocator.buffer(1000, options);

		let placements = [&first, &second, &third].map(|buffer| placement(buffer));
		assert!(placements.iter().all(|(heap, _)| *heap == placements[0].0));
		for (index, (_, offset)) in placements.iter().enumerate() {
			for (_, other) in &placements[index + 1..] {
				assert!(offset.abs_diff(*other) >= 1000);
			}
		}
	}

	#[test]
	fn memory_of_a_dropped_resource_is_reused() {
		let mut allocator = test_allocator();
		let options = mtl::MTLResourceOptions::StorageModeShared;

		let first = allocator.buffer(1000, options);
		let _second = allocator.buffer(1000, options);
		let first_placement = placement(&first);
		drop(first);
		allocator.collect(usize::MAX);
		let third = allocator.buffer(1000, options);

		assert_eq!(placement(&third), first_placement);
	}

	#[test]
	fn storage_modes_do_not_share_a_heap() {
		let mut allocator = test_allocator();
		let description = ImageDescription::new(
			&crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(Extent::square(4)),
		);

		let private = allocator.texture(&build_texture_descriptor(description));
		let shared = allocator.buffer(1000, mtl::MTLResourceOptions::StorageModeShared);

		assert_ne!(
			private.heap().map(|heap| Retained::as_ptr(&heap) as usize),
			Some(placement(&shared).0)
		);
	}

	#[test]
	fn a_large_resource_has_a_heap_that_ends_with_it() {
		let mut allocator = test_allocator();
		let options = mtl::MTLResourceOptions::StorageModeShared;

		let small = allocator.buffer(1000, options);
		let large = allocator.buffer(LARGEST_SHARED_ALLOCATION as usize + 1, options);
		let large_heap = Weak::from_retained(&heap(&large));
		assert_ne!(placement(&small).0, placement(&large).0);
		assert!(large_heap.load().is_some());

		drop(large);

		assert!(large_heap.load().is_none());
	}

	#[test]
	fn private_buffers_get_new_memory() {
		let mut allocator = test_allocator();
		let options = mtl::MTLResourceOptions::StorageModePrivate;

		let first = allocator.buffer(1000, options);
		let first_heap = Weak::from_retained(&heap(&first));
		let second = allocator.buffer(1000, options);
		assert_ne!(placement(&first).0, placement(&second).0);

		drop(first);

		assert!(first_heap.load().is_none());
	}

	#[test]
	fn a_shared_heap_is_released_once_every_resource_in_it_was_collected() {
		let mut allocator = test_allocator();
		let options = mtl::MTLResourceOptions::StorageModeShared;
		let buffers = [(); 8].map(|_| allocator.buffer(1000, options));
		let shared_heap = Weak::from_retained(&heap(&buffers[0]));

		drop(buffers);
		// Each call looks at two of the eight resources.
		for _ in 0..4 {
			assert!(shared_heap.load().is_some());
			allocator.collect(2);
		}

		assert!(shared_heap.load().is_none());
	}
}
