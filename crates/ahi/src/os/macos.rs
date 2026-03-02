use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use objc2_audio_toolbox::{
	kAudioOutputUnitProperty_EnableIO, kAudioUnitManufacturer_Apple, kAudioUnitProperty_MaximumFramesPerSlice,
	kAudioUnitProperty_SetRenderCallback, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, kAudioUnitScope_Output,
	kAudioUnitSubType_DefaultOutput, kAudioUnitSubType_HALOutput, kAudioUnitType_Output, AURenderCallbackStruct,
	AudioComponent, AudioComponentDescription, AudioComponentFindNext, AudioComponentInstanceDispose,
	AudioComponentInstanceNew, AudioOutputUnitStart, AudioOutputUnitStop, AudioUnit, AudioUnitGetProperty, AudioUnitInitialize,
	AudioUnitRenderActionFlags, AudioUnitSetProperty, AudioUnitUninitialize,
};
use objc2_core_audio_types::{
	kAudioFormatLinearPCM, kLinearPCMFormatFlagIsFloat, kLinearPCMFormatFlagIsPacked, kLinearPCMFormatFlagIsSignedInteger,
	AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
};

use crate::audio_hardware_interface::{HardwareParameters, Streams, WritePlayFunction};

const DEFAULT_PERIOD_SIZE: usize = 1024;
const RING_PERIOD_COUNT: usize = 4;
const AUDIO_UNIT_SUBTYPE_REMOTE_IO: u32 = u32::from_be_bytes(*b"rioc");

pub struct Device {
	audio_unit: AudioUnit,
	parameters: HardwareParameters,
	period_size: usize,
	bytes_per_frame: usize,
	started: AtomicBool,
	callback_state: Box<CallbackState>,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Result<Self, String>
	where
		Self: Sized,
	{
		if !matches!(params.channels, 1 | 2) {
			return Err("Unsupported number of channels. The most likely cause is that this backend only supports mono and stereo streams.".into());
		}

		let (bytes_per_sample, format_flags) = match params.bit_depth {
			16 => (2usize, kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked),
			32 => (4usize, kLinearPCMFormatFlagIsFloat | kLinearPCMFormatFlagIsPacked),
			_ => {
				return Err("Unsupported bit depth. The most likely cause is that this backend only supports 16-bit PCM and 32-bit float output.".into());
			}
		};

		let period_size = DEFAULT_PERIOD_SIZE;
		let bytes_per_frame = bytes_per_sample * params.channels as usize;
		let bytes_per_period = period_size
			.checked_mul(bytes_per_frame)
			.ok_or_else(|| "Failed to calculate period buffer size. The most likely cause is integer overflow when deriving bytes per period.".to_string())?;
		let ring_capacity = bytes_per_period
			.checked_mul(RING_PERIOD_COUNT)
			.ok_or_else(|| "Failed to calculate ring buffer size. The most likely cause is integer overflow when deriving total ring capacity.".to_string())?;

		let callback_state = Box::new(CallbackState {
			ring: SpscByteRing::new(ring_capacity)?,
			underrun_count: AtomicUsize::new(0),
		});
		let callback_state_ptr = (&*callback_state as *const CallbackState).cast_mut().cast::<c_void>();

		let (component, subtype) = find_output_component()?;
		let mut audio_unit: AudioUnit = std::ptr::null_mut();

		let create_status = unsafe { AudioComponentInstanceNew(component, NonNull::from(&mut audio_unit)) };
		if create_status != 0 || audio_unit.is_null() {
			return Err(os_status_error(
				"Failed to create audio unit instance",
				create_status,
				"The most likely cause is that the selected output component could not be instantiated by Core Audio.",
			));
		}

		let configuration_result = (|| -> Result<(), String> {
			if subtype != kAudioUnitSubType_DefaultOutput {
				let output_enabled: u32 = 1;
				set_audio_unit_property(
					audio_unit,
					kAudioOutputUnitProperty_EnableIO,
					kAudioUnitScope_Output,
					0,
					&output_enabled,
					"Failed to enable output IO on audio unit",
					"The most likely cause is that the selected output unit rejected the requested IO bus configuration.",
				)?;

				let input_disabled: u32 = 1;
				set_audio_unit_property(
					audio_unit,
					kAudioOutputUnitProperty_EnableIO,
					kAudioUnitScope_Input,
					1,
					&input_disabled,
					"Failed to disable input IO on audio unit",
					"The most likely cause is that the selected output unit rejected the requested input bus configuration.",
				)?;
			}

			let stream_format = AudioStreamBasicDescription {
				mSampleRate: params.sample_rate as f64,
				mFormatID: kAudioFormatLinearPCM,
				mFormatFlags: format_flags,
				mBytesPerPacket: bytes_per_frame as u32,
				mFramesPerPacket: 1,
				mBytesPerFrame: bytes_per_frame as u32,
				mChannelsPerFrame: params.channels,
				mBitsPerChannel: params.bit_depth,
				mReserved: 0,
			};
			set_audio_unit_property(
				audio_unit,
				kAudioUnitProperty_StreamFormat,
				kAudioUnitScope_Input,
				0,
				&stream_format,
				"Failed to set output stream format",
				"The most likely cause is that the selected output unit does not support the requested sample format, rate, or channel count.",
			)?;
			let actual_stream_format = get_audio_unit_stream_format(audio_unit, kAudioUnitScope_Input, 0)?;
			validate_stream_format(&stream_format, &actual_stream_format)?;

			let max_frames_per_slice = period_size as u32;
			set_audio_unit_property(
				audio_unit,
				kAudioUnitProperty_MaximumFramesPerSlice,
				kAudioUnitScope_Output,
				0,
				&max_frames_per_slice,
				"Failed to set maximum frames per slice",
				"The most likely cause is that the selected output unit rejected the requested slice size.",
			)?;

			let callback = AURenderCallbackStruct {
				inputProc: Some(output_render_callback),
				inputProcRefCon: callback_state_ptr,
			};
			set_audio_unit_property(
				audio_unit,
				kAudioUnitProperty_SetRenderCallback,
				kAudioUnitScope_Input,
				0,
				&callback,
				"Failed to register audio render callback",
				"The most likely cause is that the selected output unit does not support callback-based rendering with the current configuration.",
			)?;

			let initialize_status = unsafe { AudioUnitInitialize(audio_unit) };
			if initialize_status != 0 {
				return Err(os_status_error(
					"Failed to initialize audio unit",
					initialize_status,
					"The most likely cause is an unsupported or incomplete audio unit configuration.",
				));
			}

			Ok(())
		})();

		if let Err(error) = configuration_result {
			unsafe {
				let _ = AudioUnitUninitialize(audio_unit);
				let _ = AudioComponentInstanceDispose(audio_unit);
			}
			return Err(error);
		}

		Ok(Device {
			audio_unit,
			parameters: params,
			period_size,
			bytes_per_frame,
			started: AtomicBool::new(false),
			callback_state,
		})
	}

	fn get_period_size(&self) -> usize {
		self.period_size
	}

	fn get_underrun_count(&self) -> usize {
		self.callback_state.underrun_count.load(Ordering::Acquire)
	}

	fn wait_for_playback_space(&self) {
		let required_bytes = self.bytes_per_frame;
		self.callback_state.ring.wait_for_available_write(required_bytes);
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, ()> {
		let max_bytes = self.period_size * self.bytes_per_frame;
		let bytes_per_frame = self.bytes_per_frame;
		let params = self.parameters;

		let bytes_written = self.callback_state.ring.with_write_chunk(max_bytes, |chunk| {
			let available_frames = chunk.len() / bytes_per_frame;
			if available_frames == 0 {
				return 0;
			}

			match (params.bit_depth, params.channels) {
				(16, 1) => {
					let buffer = unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<i16>(), available_frames) };
					wpf(Streams::Mono16Bit(buffer));
					available_frames * size_of::<i16>()
				}
				(16, 2) => {
					let buffer =
						unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<(i16, i16)>(), available_frames) };
					wpf(Streams::Stereo16Bit(buffer));
					available_frames * size_of::<(i16, i16)>()
				}
				(32, 1) => {
					let buffer = unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<f32>(), available_frames) };
					wpf(Streams::MonoFloat32(buffer));
					available_frames * size_of::<f32>()
				}
				(32, 2) => {
					let buffer =
						unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<(f32, f32)>(), available_frames) };
					wpf(Streams::StereoFloat32(buffer));
					available_frames * size_of::<(f32, f32)>()
				}
				_ => 0,
			}
		});

		let frames = bytes_written / self.bytes_per_frame;

		if frames == 0 {
			return Ok(0);
		}

		if self
			.started
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_ok()
		{
			let start_status = unsafe { AudioOutputUnitStart(self.audio_unit) };
			if start_status != 0 {
				self.started.store(false, Ordering::Release);
				return Err(());
			}
		}

		Ok(frames)
	}

	fn pause(&self) {
		if self.started.swap(false, Ordering::AcqRel) {
			unsafe {
				let _ = AudioOutputUnitStop(self.audio_unit);
			}
		}
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		if self.audio_unit.is_null() {
			return;
		}

		if self.started.swap(false, Ordering::AcqRel) {
			unsafe {
				let _ = AudioOutputUnitStop(self.audio_unit);
			}
		}

		unsafe {
			let _ = AudioUnitUninitialize(self.audio_unit);
			let _ = AudioComponentInstanceDispose(self.audio_unit);
		}
	}
}

unsafe extern "C-unwind" fn output_render_callback(
	ref_con: NonNull<c_void>,
	io_action_flags: NonNull<AudioUnitRenderActionFlags>,
	_in_time_stamp: NonNull<AudioTimeStamp>,
	_in_bus_number: u32,
	_in_number_frames: u32,
	io_data: *mut AudioBufferList,
) -> i32 {
	if io_data.is_null() {
		return 0;
	}

	let callback_state = unsafe { &*(ref_con.as_ptr() as *const CallbackState) };
	let buffer_list = unsafe { &mut *io_data };

	let mut pulled_any_audio = false;
	let mut had_underrun = false;
	let buffers = buffer_list.mBuffers.as_mut_ptr();

	for index in 0..buffer_list.mNumberBuffers as usize {
		let buffer = unsafe { &mut *buffers.add(index) };
		let byte_count = buffer.mDataByteSize as usize;

		if byte_count == 0 || buffer.mData.is_null() {
			continue;
		}

		let destination = unsafe { std::slice::from_raw_parts_mut(buffer.mData as *mut u8, byte_count) };

		let pulled = callback_state.ring.pop_into_slice(destination);
		if pulled > 0 {
			pulled_any_audio = true;
		}

		if pulled < byte_count {
			had_underrun = true;
			destination[pulled..].fill(0);
		}
	}

	if had_underrun {
		callback_state.underrun_count.fetch_add(1, Ordering::Relaxed);
	}

	if !pulled_any_audio {
		unsafe {
			(*io_action_flags.as_ptr()).insert(AudioUnitRenderActionFlags::UnitRenderAction_OutputIsSilence);
		}
	}

	0
}

fn os_status_error(message: &str, status: i32, cause: &str) -> String {
	format!("{message} (OSStatus {status}). {cause}")
}

fn set_audio_unit_property<T>(
	audio_unit: AudioUnit,
	property: u32,
	scope: u32,
	element: u32,
	value: &T,
	message: &str,
	cause: &str,
) -> Result<(), String> {
	let status = unsafe {
		AudioUnitSetProperty(
			audio_unit,
			property,
			scope,
			element,
			(value as *const T).cast(),
			size_of::<T>() as u32,
		)
	};

	if status == 0 {
		Ok(())
	} else {
		Err(os_status_error(message, status, cause))
	}
}

fn get_audio_unit_stream_format(
	audio_unit: AudioUnit,
	scope: u32,
	element: u32,
) -> Result<AudioStreamBasicDescription, String> {
	let mut stream_format = AudioStreamBasicDescription {
		mSampleRate: 0.0,
		mFormatID: 0,
		mFormatFlags: 0,
		mBytesPerPacket: 0,
		mFramesPerPacket: 0,
		mBytesPerFrame: 0,
		mChannelsPerFrame: 0,
		mBitsPerChannel: 0,
		mReserved: 0,
	};
	let mut data_size = size_of::<AudioStreamBasicDescription>() as u32;

	let status = unsafe {
		AudioUnitGetProperty(
			audio_unit,
			kAudioUnitProperty_StreamFormat,
			scope,
			element,
			NonNull::from(&mut stream_format).cast(),
			NonNull::from(&mut data_size),
		)
	};

	if status != 0 {
		return Err(os_status_error(
			"Failed to read output stream format",
			status,
			"The most likely cause is that the selected output unit does not expose the stream format on the requested scope and element.",
		));
	}

	if data_size as usize != size_of::<AudioStreamBasicDescription>() {
		return Err("Invalid output stream format payload size. The most likely cause is that the audio unit returned an unexpected stream format structure size.".into());
	}

	Ok(stream_format)
}

fn validate_stream_format(requested: &AudioStreamBasicDescription, actual: &AudioStreamBasicDescription) -> Result<(), String> {
	let sample_rate_matches = (requested.mSampleRate - actual.mSampleRate).abs() <= f64::EPSILON;
	let matches = sample_rate_matches
		&& requested.mFormatID == actual.mFormatID
		&& requested.mFormatFlags == actual.mFormatFlags
		&& requested.mBytesPerPacket == actual.mBytesPerPacket
		&& requested.mFramesPerPacket == actual.mFramesPerPacket
		&& requested.mBytesPerFrame == actual.mBytesPerFrame
		&& requested.mChannelsPerFrame == actual.mChannelsPerFrame
		&& requested.mBitsPerChannel == actual.mBitsPerChannel;

	if matches {
		Ok(())
	} else {
		Err(format!(
			"Output stream format mismatch. The most likely cause is that the selected output unit coerced the requested format to a hardware-supported format. Requested: {:?}. Actual: {:?}.",
			requested, actual
		))
	}
}

fn find_output_component() -> Result<(AudioComponent, u32), String> {
	for subtype in [
		AUDIO_UNIT_SUBTYPE_REMOTE_IO,
		kAudioUnitSubType_HALOutput,
		kAudioUnitSubType_DefaultOutput,
	] {
		let mut description = AudioComponentDescription {
			componentType: kAudioUnitType_Output,
			componentSubType: subtype,
			componentManufacturer: kAudioUnitManufacturer_Apple,
			componentFlags: 0,
			componentFlagsMask: 0,
		};

		let component = unsafe { AudioComponentFindNext(std::ptr::null_mut(), NonNull::from(&mut description)) };

		if !component.is_null() {
			return Ok((component, subtype));
		}
	}

	Err("Failed to find a Core Audio output unit. The most likely cause is that neither RemoteIO nor HAL/default output units are available.".into())
}

struct SpscByteRing {
	storage: Box<[u8]>,
	capacity: usize,
	read_index: AtomicUsize,
	write_index: AtomicUsize,
	space_available_mutex: Mutex<()>,
	space_available_condvar: Condvar,
}

impl SpscByteRing {
	// Creates a fixed-size lock-free SPSC ring buffer used by play() and the Core Audio callback.
	fn new(capacity: usize) -> Result<Self, String> {
		if capacity == 0 {
			return Err(
				"Failed to create ring buffer. The most likely cause is that the computed buffer capacity was zero.".into(),
			);
		}

		Ok(Self {
			storage: vec![0; capacity].into_boxed_slice(),
			capacity,
			read_index: AtomicUsize::new(0),
			write_index: AtomicUsize::new(0),
			space_available_mutex: Mutex::new(()),
			space_available_condvar: Condvar::new(),
		})
	}

	fn available_write(&self) -> usize {
		let read = self.read_index.load(Ordering::Acquire);
		let write = self.write_index.load(Ordering::Acquire);
		self.capacity - write.wrapping_sub(read)
	}

	// Exposes a contiguous writable chunk to the producer and commits written bytes.
	fn with_write_chunk(&self, max_bytes: usize, writer: impl FnOnce(&mut [u8]) -> usize) -> usize {
		let read = self.read_index.load(Ordering::Acquire);
		let write = self.write_index.load(Ordering::Relaxed);

		let available = self.capacity - write.wrapping_sub(read);
		if available == 0 || max_bytes == 0 {
			return 0;
		}

		let start = write % self.capacity;
		let contiguous = available.min(self.capacity - start).min(max_bytes);

		let written = unsafe {
			let destination = std::slice::from_raw_parts_mut(self.storage.as_ptr().cast_mut().add(start), contiguous);
			writer(destination).min(contiguous)
		};

		if written == 0 {
			return 0;
		}

		self.write_index.store(write.wrapping_add(written), Ordering::Release);
		written
	}

	// Blocks until the ring has enough capacity for a write of the requested size.
	fn wait_for_available_write(&self, required_bytes: usize) {
		let required_bytes = required_bytes.max(1).min(self.capacity);
		let mut lock = self.space_available_mutex.lock().unwrap();

		while self.available_write() < required_bytes {
			let waited = self
				.space_available_condvar
				.wait_timeout(lock, std::time::Duration::from_millis(2))
				.unwrap();
			lock = waited.0;
		}
	}

	// Pops bytes from the ring buffer into the consumer destination slice.
	fn pop_into_slice(&self, destination: &mut [u8]) -> usize {
		let read = self.read_index.load(Ordering::Relaxed);
		let write = self.write_index.load(Ordering::Acquire);

		let available = write.wrapping_sub(read);
		let to_read = destination.len().min(available);

		if to_read == 0 {
			return 0;
		}

		let start = read % self.capacity;
		let first_len = to_read.min(self.capacity - start);

		unsafe {
			let source = self.storage.as_ptr();
			std::ptr::copy_nonoverlapping(source.add(start), destination.as_mut_ptr(), first_len);

			if to_read > first_len {
				std::ptr::copy_nonoverlapping(source, destination.as_mut_ptr().add(first_len), to_read - first_len);
			}
		}

		self.read_index.store(read.wrapping_add(to_read), Ordering::Release);
		self.space_available_condvar.notify_one();
		to_read
	}
}

struct CallbackState {
	ring: SpscByteRing,
	underrun_count: AtomicUsize,
}

#[cfg(test)]
mod tests {
	use super::SpscByteRing;
	use std::collections::VecDeque;
	use std::sync::atomic::Ordering;
	use std::sync::mpsc;
	use std::time::Duration;

	#[test]
	fn ring_rejects_zero_capacity() {
		assert!(SpscByteRing::new(0).is_err());
	}

	#[test]
	fn ring_starts_empty_with_full_write_capacity() {
		let ring = SpscByteRing::new(8).unwrap();

		assert_eq!(ring.available_write(), 8);
		assert_eq!(ring.read_index.load(Ordering::Acquire), 0);
		assert_eq!(ring.write_index.load(Ordering::Acquire), 0);
	}

	#[test]
	fn with_write_chunk_clamps_to_writer_return_and_slice_size() {
		let ring = SpscByteRing::new(8).unwrap();
		let written = ring.with_write_chunk(8, |chunk| {
			chunk.fill(0xAB);
			chunk.len() + 4
		});

		assert_eq!(written, 8);
		assert_eq!(ring.available_write(), 0);

		let mut popped = [0u8; 8];
		let read = ring.pop_into_slice(&mut popped);
		assert_eq!(read, 8);
		assert_eq!(popped, [0xAB; 8]);
	}

	#[test]
	fn with_write_chunk_respects_contiguous_region_before_wrap() {
		let ring = SpscByteRing::new(8).unwrap();

		let first = ring.with_write_chunk(6, |chunk| {
			assert_eq!(chunk.len(), 6);
			chunk.copy_from_slice(&[1, 2, 3, 4, 5, 6]);
			chunk.len()
		});
		assert_eq!(first, 6);

		let mut dropped = [0u8; 4];
		assert_eq!(ring.pop_into_slice(&mut dropped), 4);

		let second = ring.with_write_chunk(6, |chunk| {
			assert_eq!(chunk.len(), 2);
			chunk.copy_from_slice(&[7, 8]);
			chunk.len()
		});
		assert_eq!(second, 2);

		let third = ring.with_write_chunk(6, |chunk| {
			assert_eq!(chunk.len(), 4);
			chunk.copy_from_slice(&[9, 10, 11, 12]);
			chunk.len()
		});
		assert_eq!(third, 4);
	}

	#[test]
	fn pop_returns_zero_when_empty() {
		let ring = SpscByteRing::new(8).unwrap();
		let mut destination = [0u8; 8];
		assert_eq!(ring.pop_into_slice(&mut destination), 0);
	}

	#[test]
	fn ring_preserves_fifo_order_across_wraparound() {
		let ring = SpscByteRing::new(8).unwrap();

		assert_eq!(
			ring.with_write_chunk(6, |chunk| {
				chunk.copy_from_slice(&[1, 2, 3, 4, 5, 6]);
				6
			}),
			6
		);

		let mut first_pop = [0u8; 5];
		assert_eq!(ring.pop_into_slice(&mut first_pop), 5);
		assert_eq!(first_pop, [1, 2, 3, 4, 5]);

		assert_eq!(
			ring.with_write_chunk(7, |chunk| {
				assert_eq!(chunk.len(), 2);
				chunk.copy_from_slice(&[7, 8]);
				chunk.len()
			}),
			2
		);

		assert_eq!(
			ring.with_write_chunk(7, |chunk| {
				assert_eq!(chunk.len(), 5);
				chunk.copy_from_slice(&[9, 10, 11, 12, 13]);
				chunk.len()
			}),
			5
		);

		let mut second_pop = [0u8; 8];
		assert_eq!(ring.pop_into_slice(&mut second_pop), 8);
		assert_eq!(second_pop, [6, 7, 8, 9, 10, 11, 12, 13]);
	}

	#[test]
	fn wait_for_available_write_blocks_until_space_is_freed() {
		let ring = std::sync::Arc::new(SpscByteRing::new(4).unwrap());
		assert_eq!(
			ring.with_write_chunk(4, |chunk| {
				chunk.copy_from_slice(&[1, 2, 3, 4]);
				4
			}),
			4
		);

		let (sender, receiver) = mpsc::channel();
		let waiting_ring = ring.clone();

		let waiter = std::thread::spawn(move || {
			waiting_ring.wait_for_available_write(0);
			sender.send(()).unwrap();
		});

		assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());

		let mut destination = [0u8; 1];
		assert_eq!(ring.pop_into_slice(&mut destination), 1);
		assert_eq!(destination, [1]);

		receiver.recv_timeout(Duration::from_millis(500)).unwrap();
		waiter.join().unwrap();
	}

	#[test]
	fn ring_invariants_hold_during_mixed_operations() {
		let ring = SpscByteRing::new(16).unwrap();
		let mut model = VecDeque::<u8>::new();
		let mut stream_value = 0u8;
		let mut rng = 0xC0FFEEu32;

		for _ in 0..1000 {
			rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
			let operation = rng & 1;

			if operation == 0 {
				let requested_max = ((rng >> 1) as usize) % 33;
				let requested_count = ((rng >> 6) as usize) % 33;

				let expected_start = stream_value;
				let written = ring.with_write_chunk(requested_max, |chunk| {
					for (index, byte) in chunk.iter_mut().enumerate() {
						*byte = expected_start.wrapping_add(index as u8);
					}
					requested_count
				});

				for _ in 0..written {
					model.push_back(stream_value);
					stream_value = stream_value.wrapping_add(1);
				}
			} else {
				let pop_len = ((rng >> 1) as usize) % 33;
				let mut destination = vec![0u8; pop_len];
				let popped = ring.pop_into_slice(&mut destination);

				let mut expected = Vec::with_capacity(popped);
				for _ in 0..popped {
					expected.push(model.pop_front().unwrap());
				}

				assert_eq!(&destination[..popped], expected.as_slice());
			}

			let write_index = ring.write_index.load(Ordering::Acquire);
			let read_index = ring.read_index.load(Ordering::Acquire);
			let occupancy = write_index.wrapping_sub(read_index);
			let available_write = ring.available_write();

			assert!(occupancy <= ring.capacity);
			assert!(available_write <= ring.capacity);
			assert_eq!(occupancy, model.len());
			assert_eq!(available_write, ring.capacity - model.len());
		}
	}
}
