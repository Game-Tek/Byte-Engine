use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use objc2_audio_toolbox::{
	AURenderCallbackStruct, AudioComponent, AudioComponentDescription, AudioComponentFindNext, AudioComponentInstanceDispose,
	AudioComponentInstanceNew, AudioOutputUnitStart, AudioOutputUnitStop, AudioUnit, AudioUnitGetProperty, AudioUnitInitialize,
	AudioUnitRenderActionFlags, AudioUnitSetProperty, AudioUnitUninitialize, kAudioOutputUnitProperty_EnableIO,
	kAudioUnitManufacturer_Apple, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitProperty_SetRenderCallback,
	kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, kAudioUnitScope_Output, kAudioUnitSubType_DefaultOutput,
	kAudioUnitSubType_HALOutput, kAudioUnitType_Output,
};
use objc2_core_audio_types::{
	AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, kAudioFormatLinearPCM, kLinearPCMFormatFlagIsFloat,
	kLinearPCMFormatFlagIsPacked, kLinearPCMFormatFlagIsSignedInteger,
};

use crate::audio_hardware_interface::{AudioPlayError, HardwareParameters, Streams, WritePlayFunction};

const DEFAULT_PERIOD_SIZE: usize = 1024;
const RING_PERIOD_COUNT: usize = 4;
const AUDIO_UNIT_SUBTYPE_REMOTE_IO: u32 = u32::from_be_bytes(*b"rioc");
const IO_ENABLED: u32 = 1;
const IO_DISABLED: u32 = 0;

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
		let audio_unit = create_audio_unit_instance(component)?;
		if let Err(error) = configure_audio_unit(
			audio_unit,
			subtype,
			params,
			period_size,
			bytes_per_frame,
			format_flags,
			callback_state_ptr,
		) {
			dispose_audio_unit(audio_unit);
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

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, AudioPlayError> {
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
					// SAFETY: The chunk is aligned for Core Audio samples, and the frame count stays within its byte length.
					let buffer = unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<i16>(), available_frames) };
					wpf(Streams::Mono16Bit(buffer));
					available_frames * size_of::<i16>()
				}
				(16, 2) => {
					// SAFETY: The chunk is aligned for stereo samples, and the frame count stays within its byte length.
					let buffer =
						unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<(i16, i16)>(), available_frames) };
					wpf(Streams::Stereo16Bit(buffer));
					available_frames * size_of::<(i16, i16)>()
				}
				(32, 1) => {
					// SAFETY: The chunk is aligned for Core Audio samples, and the frame count stays within its byte length.
					let buffer = unsafe { std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<f32>(), available_frames) };
					wpf(Streams::MonoFloat32(buffer));
					available_frames * size_of::<f32>()
				}
				(32, 2) => {
					// SAFETY: The chunk is aligned for stereo samples, and the frame count stays within its byte length.
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
			// SAFETY: The audio unit is initialized and owned by this device until `drop`.
			let start_status = unsafe { AudioOutputUnitStart(self.audio_unit) };
			if start_status != 0 {
				self.started.store(false, Ordering::Release);
				return Err(AudioPlayError::StartFailed {
					platform_status: start_status,
				});
			}
		}

		Ok(frames)
	}

	fn pause(&self) {
		if self.started.swap(false, Ordering::AcqRel) {
			// SAFETY: The audio unit is initialized, owned by this device, and was marked as started.
			unsafe {
				let _ = AudioOutputUnitStop(self.audio_unit);
			}
		}
	}
}

// Creates and validates the Core Audio instance returned for a component.
fn create_audio_unit_instance(component: AudioComponent) -> Result<AudioUnit, String> {
	let mut audio_unit: AudioUnit = std::ptr::null_mut();
	// SAFETY: `audio_unit` is a live out-parameter for the duration of the call, and `component` came from Core Audio.
	let status = unsafe { AudioComponentInstanceNew(component, NonNull::from(&mut audio_unit)) };
	if status == 0 && !audio_unit.is_null() {
		return Ok(audio_unit);
	}

	Err(os_status_error(
		"Failed to create audio unit instance",
		status,
		"The most likely cause is that the selected output component could not be instantiated by Core Audio.",
	))
}

// Applies every property required before the output unit can start rendering.
fn configure_audio_unit(
	audio_unit: AudioUnit,
	subtype: u32,
	params: HardwareParameters,
	period_size: usize,
	bytes_per_frame: usize,
	format_flags: u32,
	callback_state_ptr: *mut c_void,
) -> Result<(), String> {
	configure_output_io(audio_unit, subtype)?;
	configure_stream_format(audio_unit, params, bytes_per_frame, format_flags)?;
	configure_maximum_frames(audio_unit, period_size)?;
	configure_render_callback(audio_unit, callback_state_ptr)?;

	// SAFETY: All required stream, buffer, and callback properties have been set on this live audio unit.
	let status = unsafe { AudioUnitInitialize(audio_unit) };
	if status == 0 {
		Ok(())
	} else {
		Err(os_status_error(
			"Failed to initialize audio unit",
			status,
			"The most likely cause is an unsupported or incomplete audio unit configuration.",
		))
	}
}

// Configures output-only IO for components that expose separate input and output buses.
fn configure_output_io(audio_unit: AudioUnit, subtype: u32) -> Result<(), String> {
	if subtype == kAudioUnitSubType_DefaultOutput {
		return Ok(());
	}

	set_audio_unit_property(
		audio_unit,
		kAudioOutputUnitProperty_EnableIO,
		kAudioUnitScope_Output,
		0,
		&IO_ENABLED,
		"Failed to enable output IO on audio unit",
		"The most likely cause is that the selected output unit rejected the requested IO bus configuration.",
	)?;
	set_audio_unit_property(
		audio_unit,
		kAudioOutputUnitProperty_EnableIO,
		kAudioUnitScope_Input,
		1,
		&IO_DISABLED,
		"Failed to disable input IO on audio unit",
		"The most likely cause is that the selected output unit rejected the requested input bus configuration.",
	)
}

// Sets the requested PCM format and verifies that Core Audio did not coerce it.
fn configure_stream_format(
	audio_unit: AudioUnit,
	params: HardwareParameters,
	bytes_per_frame: usize,
	format_flags: u32,
) -> Result<(), String> {
	let requested = AudioStreamBasicDescription {
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
		&requested,
		"Failed to set output stream format",
		"The most likely cause is that the selected output unit does not support the requested sample format, rate, or channel count.",
	)?;
	let actual = get_audio_unit_stream_format(audio_unit, kAudioUnitScope_Input, 0)?;
	validate_stream_format(&requested, &actual)
}

// Limits callback slices to the period size reserved by the ring buffer.
fn configure_maximum_frames(audio_unit: AudioUnit, period_size: usize) -> Result<(), String> {
	set_audio_unit_property(
		audio_unit,
		kAudioUnitProperty_MaximumFramesPerSlice,
		kAudioUnitScope_Output,
		0,
		&(period_size as u32),
		"Failed to set maximum frames per slice",
		"The most likely cause is that the selected output unit rejected the requested slice size.",
	)
}

// Registers the allocation-free callback and its stable heap-owned state pointer.
fn configure_render_callback(audio_unit: AudioUnit, callback_state_ptr: *mut c_void) -> Result<(), String> {
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
	)
}

// Releases a live audio unit after configuration failure or device shutdown.
fn dispose_audio_unit(audio_unit: AudioUnit) {
	// SAFETY: The instance is live and no callback can run after the owning device stops using it.
	let _ = unsafe { AudioUnitUninitialize(audio_unit) };
	// SAFETY: The instance is live, uninitialized, and this is its final owner-side operation.
	let _ = unsafe { AudioComponentInstanceDispose(audio_unit) };
}

impl Drop for Device {
	fn drop(&mut self) {
		if self.audio_unit.is_null() {
			return;
		}

		if self.started.swap(false, Ordering::AcqRel) {
			// SAFETY: The audio unit is initialized, owned by this device, and was marked as started.
			unsafe {
				let _ = AudioOutputUnitStop(self.audio_unit);
			}
		}

		dispose_audio_unit(self.audio_unit);
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

	// SAFETY: Core Audio returns the stable callback state pointer registered during device configuration.
	let callback_state = unsafe { &*(ref_con.as_ptr() as *const CallbackState) };
	// SAFETY: Core Audio supplies a non-null buffer list that remains exclusively borrowed for this callback.
	let buffer_list = unsafe { &mut *io_data };

	let mut pulled_any_audio = false;
	let mut had_underrun = false;
	let buffers = buffer_list.mBuffers.as_mut_ptr();

	for index in 0..buffer_list.mNumberBuffers as usize {
		// SAFETY: `index` is bounded by the buffer count published in this `AudioBufferList`.
		let buffer_ptr = unsafe { buffers.add(index) };
		// SAFETY: Each buffer entry is visited once and remains exclusively borrowed during this iteration.
		let buffer = unsafe { &mut *buffer_ptr };
		let byte_count = buffer.mDataByteSize as usize;

		if byte_count == 0 || buffer.mData.is_null() {
			continue;
		}

		// SAFETY: Core Audio guarantees `mData` addresses at least `mDataByteSize` writable bytes for the callback.
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
		// SAFETY: Core Audio supplies a live flags value that is exclusively writable during this callback.
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
	// SAFETY: The audio unit is live, and `value` remains valid for the copied `size_of::<T>()` payload.
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

	// SAFETY: Both out-parameters remain live and writable for the duration of the Core Audio call.
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
	for subtype in output_component_subtypes() {
		let mut description = AudioComponentDescription {
			componentType: kAudioUnitType_Output,
			componentSubType: subtype,
			componentManufacturer: kAudioUnitManufacturer_Apple,
			componentFlags: 0,
			componentFlagsMask: 0,
		};

		// SAFETY: The description remains live for the call, and a null first argument starts component enumeration.
		let component = unsafe { AudioComponentFindNext(std::ptr::null_mut(), NonNull::from(&mut description)) };

		if !component.is_null() {
			return Ok((component, subtype));
		}
	}

	Err("Failed to find a Core Audio output unit. The most likely cause is that the default, HAL, and RemoteIO components are unavailable.".into())
}

fn output_component_subtypes() -> [u32; 3] {
	[
		kAudioUnitSubType_DefaultOutput,
		kAudioUnitSubType_HALOutput,
		AUDIO_UNIT_SUBTYPE_REMOTE_IO,
	]
}

struct SpscByteRing {
	storage: Box<[u128]>,
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
			// Word storage preserves the alignment required when the producer exposes typed audio sample slices.
			storage: vec![0; capacity.div_ceil(size_of::<u128>())].into_boxed_slice(),
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

		let storage = self.storage.as_ptr().cast::<u8>().cast_mut();
		// SAFETY: `start` is below the logical byte capacity backed by the word-aligned allocation.
		let destination_start = unsafe { storage.add(start) };
		// SAFETY: `contiguous` is bounded by both the logical capacity and the allocation's remaining bytes.
		let destination = unsafe { std::slice::from_raw_parts_mut(destination_start, contiguous) };
		let written = writer(destination).min(contiguous);

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

		let source = self.storage.as_ptr().cast::<u8>();
		// SAFETY: `start` is below the logical byte capacity and `first_len` stays within the allocation tail.
		let first_source = unsafe { source.add(start) };
		// SAFETY: The ring storage and caller-owned destination do not overlap, and both cover `first_len` bytes.
		unsafe { std::ptr::copy_nonoverlapping(first_source, destination.as_mut_ptr(), first_len) };

		if to_read > first_len {
			// SAFETY: The second destination starts after the initialized first segment and remains within the slice.
			let second_destination = unsafe { destination.as_mut_ptr().add(first_len) };
			// SAFETY: The wrapped source prefix and destination tail do not overlap and cover the remaining byte count.
			unsafe {
				std::ptr::copy_nonoverlapping(source, second_destination, to_read - first_len);
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
	use std::collections::VecDeque;
	use std::sync::atomic::Ordering;
	use std::sync::mpsc;
	use std::time::Duration;

	use super::{SpscByteRing, kAudioUnitSubType_DefaultOutput, output_component_subtypes};

	// Mirrors a producer operation in the reference queue used by the mixed-operation test.
	fn write_model_operation(ring: &SpscByteRing, model: &mut VecDeque<u8>, stream_value: &mut u8, rng: u32) {
		let requested_max = ((rng >> 1) as usize) % 33;
		let requested_count = ((rng >> 6) as usize) % 33;
		let expected_start = *stream_value;
		let written = ring.with_write_chunk(requested_max, |chunk| {
			for (index, byte) in chunk.iter_mut().enumerate() {
				*byte = expected_start.wrapping_add(index as u8);
			}
			requested_count
		});

		for _ in 0..written {
			model.push_back(*stream_value);
			*stream_value = stream_value.wrapping_add(1);
		}
	}

	// Compares a consumer operation with the bytes queued by the reference model.
	fn assert_model_pop(ring: &SpscByteRing, model: &mut VecDeque<u8>, rng: u32) {
		let pop_len = ((rng >> 1) as usize) % 33;
		let mut destination = vec![0u8; pop_len];
		let popped = ring.pop_into_slice(&mut destination);
		let expected: Vec<_> = model.drain(..popped).collect();

		assert_eq!(&destination[..popped], expected.as_slice());
	}

	#[test]
	fn default_output_is_the_first_macos_component_candidate() {
		assert_eq!(output_component_subtypes()[0], kAudioUnitSubType_DefaultOutput);
	}

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
				write_model_operation(&ring, &mut model, &mut stream_value, rng);
			} else {
				assert_model_pop(&ring, &mut model, rng);
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
