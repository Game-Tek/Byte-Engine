//! DX12 device operations for GPU timing counters.

use windows::Win32::Graphics::Direct3D12::{
	D3D12_QUERY_HEAP_DESC, D3D12_QUERY_HEAP_TYPE_TIMESTAMP, D3D12_QUERY_TYPE_TIMESTAMP, ID3D12QueryHeap,
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

impl Device {
	/// Returns the counter slots the frame on `sequence_index` has written so far.
	pub(crate) fn counter_written_slots(&self, sequence_index: u8) -> std::ops::Range<u32> {
		self.counters.written_slots(sequence_index)
	}

	/// Records the start or end timestamp of `counter` into the command list, creating the counter storage on first use.
	pub(crate) fn record_counter_timestamp(
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
		// Every recording of one frame submits to the frame's queue, so its frequency converts the frame's slots.
		let queue_handle = self.command_buffers[command_buffer_handle.0 as usize].queue_handle;
		self.counter_frequencies[sequence_index as usize] = self.queues[queue_handle.0 as usize].timestamp_frequency;
		let heap = self.ensure_counter_storage().heap.clone();
		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.clone() else {
			return;
		};
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
		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.clone() else {
			return;
		};
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
				self.create_buffer_resource(COUNTER_SLOT_COUNT as usize * TIMESTAMP_SIZE, crate::DeviceAccesses::CpuRead);
			let readback = readback.expect(
				"DX12 counter readback buffer creation failed. The most likely cause is that the device is out of memory.",
			);
			self.counter_storage = Some(CounterStorage { heap, readback, mapped });
		}
		self.counter_storage.as_ref().expect("The counter storage was created above.")
	}
}
