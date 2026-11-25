//! DX12 device operations for GPU timing and pipeline statistics counters.

use windows::Win32::Graphics::Direct3D12::{
	D3D12_FEATURE_D3D12_OPTIONS9, D3D12_FEATURE_DATA_D3D12_OPTIONS9, D3D12_QUERY_DATA_PIPELINE_STATISTICS,
	D3D12_QUERY_DATA_PIPELINE_STATISTICS1, D3D12_QUERY_HEAP_DESC, D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS,
	D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS1, D3D12_QUERY_HEAP_TYPE_TIMESTAMP, D3D12_QUERY_TYPE,
	D3D12_QUERY_TYPE_PIPELINE_STATISTICS, D3D12_QUERY_TYPE_PIPELINE_STATISTICS1, D3D12_QUERY_TYPE_TIMESTAMP, ID3D12QueryHeap,
};

use super::*;
use crate::counters::{COUNTER_SLOT_COUNT, duration_from_frequency, elapsed_ticks};

/// The byte size of one resolved timestamp in the readback buffer.
const TIMESTAMP_SIZE: usize = std::mem::size_of::<u64>();

/// The `CounterStorage` struct owns the timestamp query heap of a device and the readback buffer that command lists
/// resolve it into; see [`crate::counters::Counters`] for the slots they hold.
///
/// The readback buffer stays mapped. The CPU reads a sequence's slots only after the fence of the frame that
/// resolved them, which is when the GPU writes become visible.
pub(super) struct CounterStorage {
	heap: ID3D12QueryHeap,
	readback: ID3D12Resource,
	mapped: *mut u8,
}

// SAFETY: `mapped` points into the persistent mapping of `readback`, which this value owns alone; the D3D12 objects
// are free-threaded. Moving the storage to another thread shares nothing.
unsafe impl Send for CounterStorage {}

/// The `StatisticsStorage` struct owns the pipeline statistics query heap of a device and the readback buffer command
/// lists resolve it into, one record per statistics slot.
///
/// Devices that count mesh shading stages use `PIPELINE_STATISTICS1` records, which add task and mesh counts; others
/// use the older records, and those counts read `None`.
pub(super) struct StatisticsStorage {
	heap: ID3D12QueryHeap,
	readback: ID3D12Resource,
	mapped: *mut u8,
	/// Whether the heap holds `PIPELINE_STATISTICS1` records.
	mesh: bool,
}

// SAFETY: As for `CounterStorage`, `mapped` points into the persistent mapping of `readback`, which this value owns.
unsafe impl Send for StatisticsStorage {}

impl StatisticsStorage {
	fn query_type(&self) -> D3D12_QUERY_TYPE {
		if self.mesh {
			D3D12_QUERY_TYPE_PIPELINE_STATISTICS1
		} else {
			D3D12_QUERY_TYPE_PIPELINE_STATISTICS
		}
	}

	fn record_size(&self) -> usize {
		if self.mesh {
			size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS1>()
		} else {
			size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS>()
		}
	}

	/// Reads the record of `slot`, or `None` when no query resolved into it, which leaves it all zero.
	///
	/// # Safety
	///
	/// The frame that resolved `slot` must have completed.
	unsafe fn read(&self, slot: u32) -> Option<crate::PipelineStatistics> {
		// SAFETY: The readback buffer stays mapped and holds one record per slot of the heap.
		let record = unsafe { self.mapped.add(slot as usize * self.record_size()) };
		let statistics = if self.mesh {
			// SAFETY: The heap holds `PIPELINE_STATISTICS1` records, as `mesh` says.
			let data = unsafe { std::ptr::read_unaligned(record.cast::<D3D12_QUERY_DATA_PIPELINE_STATISTICS1>()) };
			crate::PipelineStatistics {
				input_vertices: data.IAVertices,
				input_primitives: data.IAPrimitives,
				vertex_invocations: data.VSInvocations,
				geometry_invocations: data.GSInvocations,
				clipper_invocations: data.CInvocations,
				clipper_primitives: data.CPrimitives,
				fragment_invocations: data.PSInvocations,
				compute_invocations: data.CSInvocations,
				task_invocations: Some(data.ASInvocations),
				mesh_invocations: Some(data.MSInvocations),
				mesh_primitives: Some(data.MSPrimitives),
			}
		} else {
			// SAFETY: The heap holds `PIPELINE_STATISTICS` records, as `mesh` says.
			let data = unsafe { std::ptr::read_unaligned(record.cast::<D3D12_QUERY_DATA_PIPELINE_STATISTICS>()) };
			crate::PipelineStatistics {
				input_vertices: data.IAVertices,
				input_primitives: data.IAPrimitives,
				vertex_invocations: data.VSInvocations,
				geometry_invocations: data.GSInvocations,
				clipper_invocations: data.CInvocations,
				clipper_primitives: data.CPrimitives,
				fragment_invocations: data.PSInvocations,
				compute_invocations: data.CSInvocations,
				task_invocations: None,
				mesh_invocations: None,
				mesh_primitives: None,
			}
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
		let (heap, query_type) = (storage.heap.clone(), storage.query_type());
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
		if slots.is_empty() {
			return;
		}
		let Some(storage) = self.statistics_storage.as_ref() else {
			return;
		};
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The heap and the readback buffer outlive the list, and the slot range lies inside both.
		unsafe {
			command_list.ResolveQueryData(
				&storage.heap,
				storage.query_type(),
				slots.start,
				slots.len() as u32,
				&storage.readback,
				u64::from(slots.start) * storage.record_size() as u64,
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
		if slots.is_empty() {
			return;
		}
		let Some(storage) = self.counter_storage.as_ref() else {
			return;
		};
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The heap and the readback buffer outlive the list, and the slot range lies inside both.
		unsafe {
			command_list.ResolveQueryData(
				&storage.heap,
				D3D12_QUERY_TYPE_TIMESTAMP,
				slots.start,
				slots.len() as u32,
				&storage.readback,
				u64::from(slots.start) * TIMESTAMP_SIZE as u64,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Publishes the durations the completed frame on `sequence_index` measured.
	///
	/// Call it after the sequence's fence was waited. A slot a dropped recording allocated but never resolved reads
	/// zero and counts as unwritten.
	pub(crate) fn resolve_counters(&mut self, sequence_index: u8) {
		match self.statistics_storage.as_ref().filter(|storage| !storage.mapped.is_null()) {
			// SAFETY: The caller waited for the sequence's fence, so the frame that resolved these slots completed.
			Some(storage) => self
				.counters
				.resolve_statistics(sequence_index, |slot| unsafe { storage.read(slot) }),
			None => self.counters.resolve_statistics(sequence_index, |_| None),
		}
		let slots = self.counters.written_slots(sequence_index);
		let frequency = self.counter_frequencies[sequence_index as usize];
		let elapsed = |start, end| duration_from_frequency(elapsed_ticks(start, end, u64::BITS), frequency);
		let mapped = self
			.counter_storage
			.as_ref()
			.filter(|_| !slots.is_empty())
			.map(|storage| storage.mapped)
			.filter(|mapped| !mapped.is_null());
		let Some(mapped) = mapped else {
			self.counters.resolve(sequence_index, |_| None, elapsed);
			return;
		};
		self.counters.resolve(
			sequence_index,
			|slot| {
				// SAFETY: The readback buffer stays mapped and holds one timestamp per slot of the heap; the frame
				// that resolved this slot completed before this call.
				let ticks = unsafe { std::ptr::read_unaligned(mapped.add(slot as usize * TIMESTAMP_SIZE).cast::<u64>()) };
				(ticks != 0).then_some(ticks)
			},
			elapsed,
		);
	}

	/// Returns the statistics storage, creating the query heap and its readback buffer on the first call.
	///
	/// The heap holds `PIPELINE_STATISTICS1` records when the device counts mesh shading stages.
	fn ensure_statistics_storage(&mut self) -> &StatisticsStorage {
		if self.statistics_storage.is_none() {
			let mesh = Self::feature_support(
				&self.device,
				D3D12_FEATURE_D3D12_OPTIONS9,
				D3D12_FEATURE_DATA_D3D12_OPTIONS9::default(),
			)
			.is_some_and(|data| data.MeshShaderPipelineStatsSupported.as_bool());
			let description = D3D12_QUERY_HEAP_DESC {
				Type: if mesh {
					D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS1
				} else {
					D3D12_QUERY_HEAP_TYPE_PIPELINE_STATISTICS
				},
				Count: COUNTER_SLOT_COUNT,
				NodeMask: 0,
			};
			let mut heap: Option<ID3D12QueryHeap> = None;
			// SAFETY: The description is complete and the output slot is a valid interface pointer location.
			unsafe { self.device.CreateQueryHeap(&description, &mut heap) }.expect(
				"DX12 statistics query heap creation failed. The most likely cause is that the device is out of memory.",
			);
			let heap = heap.expect(
				"DX12 statistics query heap creation returned no heap. The most likely cause is that the device was removed.",
			);
			let record_size = if mesh {
				size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS1>()
			} else {
				size_of::<D3D12_QUERY_DATA_PIPELINE_STATISTICS>()
			};
			let (readback, mapped, _) =
				self.create_buffer_resource(COUNTER_SLOT_COUNT as usize * record_size, crate::HostAccess::Read);
			let readback = readback.expect(
				"DX12 statistics readback buffer creation failed. The most likely cause is that the device is out of memory.",
			);
			self.statistics_storage = Some(StatisticsStorage {
				heap,
				readback,
				mapped,
				mesh,
			});
		}
		self.statistics_storage
			.as_ref()
			.expect("The statistics storage was created above.")
	}

	/// Returns the counter storage, creating the query heap and its readback buffer on the first call.
	fn ensure_counter_storage(&mut self) -> &CounterStorage {
		if self.counter_storage.is_none() {
			let description = D3D12_QUERY_HEAP_DESC {
				Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
				Count: COUNTER_SLOT_COUNT,
				NodeMask: 0,
			};
			let mut heap: Option<ID3D12QueryHeap> = None;
			// SAFETY: The description is complete and the output slot is a valid interface pointer location.
			unsafe { self.device.CreateQueryHeap(&description, &mut heap) }
				.expect("DX12 counter query heap creation failed. The most likely cause is that the device is out of memory.");
			let heap = heap.expect(
				"DX12 counter query heap creation returned no heap. The most likely cause is that the device was removed.",
			);
			let (readback, mapped, _) =
				self.create_buffer_resource(COUNTER_SLOT_COUNT as usize * TIMESTAMP_SIZE, crate::HostAccess::Read);
			let readback = readback.expect(
				"DX12 counter readback buffer creation failed. The most likely cause is that the device is out of memory.",
			);
			self.counter_storage = Some(CounterStorage { heap, readback, mapped });
		}
		self.counter_storage.as_ref().expect("The counter storage was created above.")
	}
}
