//! DX12 device operations for GPU timing and pipeline statistics counters.

use windows::Win32::Graphics::Direct3D12::{
	D3D12_FEATURE_D3D12_OPTIONS9, D3D12_FEATURE_DATA_D3D12_OPTIONS9, D3D12_QUERY_DATA_PIPELINE_STATISTICS,
	D3D12_QUERY_DATA_PIPELINE_STATISTICS1, D3D12_QUERY_HEAP_DESC, D3D12_QUERY_HEAP_TYPE,
	D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS, D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS1, D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
	D3D12_QUERY_TYPE, D3D12_QUERY_TYPE_PIPELINE_STATISTICS, D3D12_QUERY_TYPE_PIPELINE_STATISTICS1, D3D12_QUERY_TYPE_TIMESTAMP,
	ID3D12QueryHeap,
};

use super::*;
use crate::counters::{COUNTER_SLOT_COUNT, duration_from_frequency, elapsed_ticks};

/// The byte size of one resolved timestamp in the readback buffer.
const TIMESTAMP_SIZE: usize = std::mem::size_of::<u64>();

/// The `QueryStorage` struct owns one query heap of a device, timestamps or pipeline statistics, and the readback
/// buffer that command lists resolve it into, one record per slot; see [`crate::counters::Counters`] for the slots.
///
/// The readback buffer stays mapped. The CPU reads a sequence's slots only after the fence of the frame that
/// resolved them, which is when the GPU writes become visible.
pub(super) struct QueryStorage {
	heap: ID3D12QueryHeap,
	readback: ID3D12Resource,
	mapped: *mut u8,
	query_type: D3D12_QUERY_TYPE,
	/// The byte size of one resolved record.
	record_size: usize,
}

// SAFETY: `mapped` points into the persistent mapping of `readback`, which this value owns alone; the D3D12 objects
// are free-threaded. Moving the storage to another thread shares nothing.
unsafe impl Send for QueryStorage {}

impl QueryStorage {
	/// Returns the start of the record of `slot` in the readback buffer.
	fn record(&self, slot: u32) -> *const u8 {
		// SAFETY: The readback buffer stays mapped and holds one record per slot of the heap.
		unsafe { self.mapped.add(slot as usize * self.record_size) }
	}

	/// Reads the statistics record of `slot`, or `None` when no query resolved into it, which leaves it all zero.
	///
	/// Devices that count mesh shading stages use `PIPELINE_STATISTICS1` records, which extend the older records with
	/// task and mesh counts; on others those counts read `None`.
	///
	/// # Safety
	///
	/// The frame that resolved `slot` must have completed.
	unsafe fn read_statistics(&self, slot: u32) -> Option<crate::PipelineStatistics> {
		let record = self.record(slot);
		// SAFETY: Both record types start with the `PIPELINE_STATISTICS` fields, and the slot's frame completed.
		let data = unsafe { std::ptr::read_unaligned(record.cast::<D3D12_QUERY_DATA_PIPELINE_STATISTICS>()) };
		let mesh = (self.query_type == D3D12_QUERY_TYPE_PIPELINE_STATISTICS1)
			// SAFETY: The heap holds `PIPELINE_STATISTICS1` records, as its query type says.
			.then(|| unsafe { std::ptr::read_unaligned(record.cast::<D3D12_QUERY_DATA_PIPELINE_STATISTICS1>()) });
		let statistics = crate::PipelineStatistics {
			input_vertices: data.IAVertices,
			input_primitives: data.IAPrimitives,
			vertex_invocations: data.VSInvocations,
			geometry_invocations: data.GSInvocations,
			clipper_invocations: data.CInvocations,
			clipper_primitives: data.CPrimitives,
			fragment_invocations: data.PSInvocations,
			compute_invocations: data.CSInvocations,
			task_invocations: mesh.map(|data| data.ASInvocations),
			mesh_invocations: mesh.map(|data| data.MSInvocations),
			mesh_primitives: mesh.map(|data| data.MSPrimitives),
		};
		// A record of zeros is one no query resolved into, as for a recording that was dropped. A span that really
		// counted nothing reads as unwritten too, which tells the same story.
		(statistics != crate::PipelineStatistics::default()
			&& statistics
				!= crate::PipelineStatistics {
					task_invocations: Some(0),
					mesh_invocations: Some(0),
					mesh_primitives: Some(0),
					..Default::default()
				})
		.then_some(statistics)
	}
}

impl Device {
	/// Returns the counter slots the frame on `sequence_index` has written so far.
	pub(crate) fn counter_written_slots(&self, sequence_index: u8) -> std::ops::Range<u32> {
		self.counters.written_slots(sequence_index)
	}

	/// Returns the statistics slots the frame on `sequence_index` has opened so far.
	pub(crate) fn statistics_written_slots(&self, sequence_index: u8) -> std::ops::Range<u32> {
		self.counters.statistics_written_slots(sequence_index)
	}

	/// Starts or ends `counter` in the command list: a timestamp for a timing counter, a query for a statistics one.
	pub(crate) fn record_counter(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sequence_index: u8,
		counter: crate::CounterHandle,
		start: bool,
	) {
		match self.counters.kind(counter) {
			crate::CounterKind::Timing => self.record_counter_timestamp(command_buffer_handle, sequence_index, counter, start),
			crate::CounterKind::PipelineStatistics => {
				self.record_counter_statistics(command_buffer_handle, sequence_index, counter, start)
			}
		}
	}

	/// Begins or ends the statistics query of `counter`, creating the statistics storage on first use.
	fn record_counter_statistics(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sequence_index: u8,
		counter: crate::CounterHandle,
		start: bool,
	) {
		let slot = if start {
			self.counters.begin_statistics(sequence_index, counter)
		} else {
			self.counters.end_statistics(sequence_index, counter)
		};
		let Some(slot) = slot else {
			return;
		};
		if start {
			self.open_statistics.push((counter, command_buffer_handle));
		} else {
			let position = self.open_statistics.iter().position(|&(open, _)| open == counter);
			let began_in = position.map(|position| self.open_statistics.swap_remove(position).1);
			assert_eq!(
				began_in,
				Some(command_buffer_handle),
				"Statistics counter {} ended in another recording than the one it started in. The most likely cause is that start_counter and end_counter were called on different recordings. Start and end a statistics counter in one recording.",
				counter.index(),
			);
		}
		let storage = self.ensure_statistics_storage();
		let (heap, query_type) = (storage.heap.clone(), storage.query_type);
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The heap outlives the list, and the slot comes from the counters, which stay below the heap's count.
		unsafe {
			if start {
				command_list.BeginQuery(&heap, query_type, slot);
			} else {
				command_list.EndQuery(&heap, query_type, slot);
			}
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Copies the statistics in `slots` out of the opaque query heap into the readback buffer, as the list's last work.
	///
	/// # Panics
	///
	/// Panics when a statistics counter this recording started is still open.
	pub(crate) fn resolve_statistics_slots(&mut self, command_buffer_handle: CommandBufferHandle, slots: std::ops::Range<u32>) {
		if let Some(&(counter, _)) = self
			.open_statistics
			.iter()
			.find(|&&(_, began_in)| began_in == command_buffer_handle)
		{
			panic!(
				"Statistics counter {} is still open when its recording ends. The most likely cause is a missing end_counter. End a statistics counter in the recording that started it.",
				counter.index()
			);
		}
		self.resolve_query_slots(command_buffer_handle, true, slots);
	}

	/// Copies the records in `slots` out of the opaque statistics or timestamp query heap into its readback buffer, as
	/// the list's last work. Does nothing when no counter of that kind ever wrote.
	fn resolve_query_slots(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		statistics: bool,
		slots: std::ops::Range<u32>,
	) {
		let storage = if statistics {
			&self.statistics_storage
		} else {
			&self.counter_storage
		};
		let Some(storage) = storage.as_ref().filter(|_| !slots.is_empty()) else {
			return;
		};
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The heap and the readback buffer outlive the list, and the slot range lies inside both.
		unsafe {
			command_list.ResolveQueryData(
				&storage.heap,
				storage.query_type,
				slots.start,
				slots.len() as u32,
				&storage.readback,
				u64::from(slots.start) * storage.record_size as u64,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Records the start or end timestamp of `counter` into the command list, creating the counter storage on first use.
	fn record_counter_timestamp(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sequence_index: u8,
		counter: crate::CounterHandle,
		start: bool,
	) {
		let slot = if start {
			self.counters.start(sequence_index, counter)
		} else {
			self.counters.end(sequence_index, counter)
		};
		// Disabled counters write nothing, so the storage is never created.
		let Some(slot) = slot else {
			return;
		};
		// Every recording of one frame submits to the frame's queue, so its frequency converts the frame's slots.
		let queue_handle = self.command_buffers[command_buffer_handle.index() as usize].queue_handle;
		self.counter_frequencies[sequence_index as usize] = self.queues[queue_handle.index() as usize].timestamp_frequency;
		let heap = self.ensure_counter_storage().heap.clone();
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The heap outlives the list, and the slot comes from the counters, which stay below the heap's count.
		unsafe { command_list.EndQuery(&heap, D3D12_QUERY_TYPE_TIMESTAMP, slot) };
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Copies the timestamps in `slots` out of the opaque query heap into the readback buffer, as the list's last work.
	pub(crate) fn resolve_counter_slots(&mut self, command_buffer_handle: CommandBufferHandle, slots: std::ops::Range<u32>) {
		self.resolve_query_slots(command_buffer_handle, false, slots);
	}

	/// Publishes the durations the completed frame on `sequence_index` measured.
	///
	/// Call it after the sequence's fence was waited. A slot a dropped recording allocated but never resolved reads
	/// zero and counts as unwritten.
	pub(crate) fn resolve_counters(&mut self, sequence_index: u8) {
		match self.statistics_storage.as_ref() {
			// SAFETY: The caller waited for the sequence's fence, so the frame that resolved these slots completed.
			Some(storage) => self
				.counters
				.resolve_statistics(sequence_index, |slot| unsafe { storage.read_statistics(slot) }),
			None => self.counters.resolve_statistics(sequence_index, |_| None),
		}
		let frequency = self.counter_frequencies[sequence_index as usize];
		let elapsed = |start, end| duration_from_frequency(elapsed_ticks(start, end, u64::BITS), frequency);
		let storage = self.counter_storage.as_ref();
		self.counters.resolve(
			sequence_index,
			|slot| {
				// SAFETY: The readback buffer holds one timestamp per slot of the heap; the frame that resolved this slot
				// completed before this call.
				let ticks = unsafe { std::ptr::read_unaligned(storage?.record(slot).cast::<u64>()) };
				(ticks != 0).then_some(ticks)
			},
			elapsed,
		);
	}

	/// Creates a query heap of `COUNTER_SLOT_COUNT` slots and the mapped readback buffer it resolves into.
	fn create_query_storage(
		&self,
		heap_type: D3D12_QUERY_HEAP_TYPE,
		query_type: D3D12_QUERY_TYPE,
		record_size: usize,
	) -> QueryStorage {
		let description = D3D12_QUERY_HEAP_DESC {
			Type: heap_type,
			Count: COUNTER_SLOT_COUNT,
			NodeMask: 0,
		};
		let mut heap: Option<ID3D12QueryHeap> = None;
		// SAFETY: The description is complete and the output slot is a valid interface pointer location.
		unsafe { self.device.CreateQueryHeap(&description, &mut heap) }.expect(
			"DX12 query heap creation failed. The most likely cause is that the device is out of memory. Use fewer counters.",
		);
		let (readback, mapped, _) =
			self.create_buffer_resource(COUNTER_SLOT_COUNT as usize * record_size, crate::HostAccess::Read);
		QueryStorage {
			heap: heap.expect("A successful CreateQueryHeap returns the heap."),
			readback: readback.expect("Query readback buffers are never empty."),
			mapped,
			query_type,
			record_size,
		}
	}

	/// Returns the statistics storage, creating it on the first call. Its heap holds `PIPELINE_STATISTICS1` records
	/// when the device counts mesh shading stages.
	fn ensure_statistics_storage(&mut self) -> &QueryStorage {
		if self.statistics_storage.is_none() {
			let mesh = Self::feature_support(
				&self.device,
				D3D12_FEATURE_D3D12_OPTIONS9,
				D3D12_FEATURE_DATA_D3D12_OPTIONS9::default(),
			)
			.is_some_and(|data| data.MeshShaderPipelineStatsSupported.as_bool());
			let storage = if mesh {
				self.create_query_storage(
					D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS1,
					D3D12_QUERY_TYPE_PIPELINE_STATISTICS1,
					size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS1>(),
				)
			} else {
				self.create_query_storage(
					D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS,
					D3D12_QUERY_TYPE_PIPELINE_STATISTICS,
					size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS>(),
				)
			};
			self.statistics_storage = Some(storage);
		}
		self.statistics_storage
			.as_ref()
			.expect("The statistics storage was created above.")
	}

	/// Returns the timestamp storage, creating it on the first call.
	fn ensure_counter_storage(&mut self) -> &QueryStorage {
		if self.counter_storage.is_none() {
			let storage =
				self.create_query_storage(D3D12_QUERY_HEAP_TYPE_TIMESTAMP, D3D12_QUERY_TYPE_TIMESTAMP, TIMESTAMP_SIZE);
			self.counter_storage = Some(storage);
		}
		self.counter_storage.as_ref().expect("The counter storage was created above.")
	}
}
