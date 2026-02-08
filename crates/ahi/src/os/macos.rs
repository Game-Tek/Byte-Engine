use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use objc2_audio_toolbox::{
	AudioComponent, AudioComponentDescription, AudioComponentFindNext, AudioComponentInstanceDispose, AudioComponentInstanceNew, AudioOutputUnitStart, AudioOutputUnitStop, AudioUnit, AudioUnitGetProperty, AudioUnitInitialize, AudioUnitRenderActionFlags, AudioUnitSetProperty, AudioUnitUninitialize, AURenderCallbackStruct, kAudioOutputUnitProperty_EnableIO, kAudioUnitManufacturer_Apple, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitProperty_SetRenderCallback, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, kAudioUnitScope_Output, kAudioUnitSubType_DefaultOutput, kAudioUnitSubType_HALOutput, kAudioUnitType_Output,
};
use objc2_core_audio_types::{
	AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, kAudioFormatLinearPCM, kLinearPCMFormatFlagIsFloat, kLinearPCMFormatFlagIsPacked, kLinearPCMFormatFlagIsSignedInteger,
};

use crate::audio_hardware_interface::{BufferPlayFunction, HardwareParameters, WritePlayFunction, Writer};

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
		let bytes_per_period = period_size.checked_mul(bytes_per_frame).ok_or_else(|| {
			"Failed to calculate period buffer size. The most likely cause is integer overflow when deriving bytes per period.".to_string()
		})?;
		let ring_capacity = bytes_per_period.checked_mul(RING_PERIOD_COUNT).ok_or_else(|| {
			"Failed to calculate ring buffer size. The most likely cause is integer overflow when deriving total ring capacity.".to_string()
		})?;

		let callback_state = Box::new(CallbackState {
			ring: SpscByteRing::new(ring_capacity)?,
			underrun_count: AtomicUsize::new(0),
		});
		let callback_state_ptr = (&*callback_state as *const CallbackState)
			.cast_mut()
			.cast::<c_void>();

		let (component, subtype) = find_output_component()?;
		let mut audio_unit: AudioUnit = std::ptr::null_mut();

		let create_status = unsafe {
			AudioComponentInstanceNew(component, NonNull::from(&mut audio_unit))
		};
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

	fn play(
		&self,
		_wpf: impl WritePlayFunction,
		bpf: impl BufferPlayFunction,
	) -> Result<usize, ()> {
		let required_bytes = self.period_size * self.bytes_per_frame;
		if self.callback_state.ring.available_write() < required_bytes {
			return Ok(0);
		}

		let ring = &self.callback_state.ring;
		let frames = match (self.parameters.bit_depth, self.parameters.channels) {
			(16, 1) => bpf(Writer::Mono16Bit(Box::new(move |buffer| {
				let byte_count = buffer.len() * size_of::<i16>();
				let bytes = unsafe {
					std::slice::from_raw_parts(buffer.as_ptr() as *const u8, byte_count)
				};
				let written = ring.push(bytes);
				debug_assert_eq!(written, bytes.len());
			}))),
			(16, 2) => bpf(Writer::Stereo16Bit(Box::new(move |buffer| {
				let byte_count = buffer.len() * size_of::<(i16, i16)>();
				let bytes = unsafe {
					std::slice::from_raw_parts(buffer.as_ptr() as *const u8, byte_count)
				};
				let written = ring.push(bytes);
				debug_assert_eq!(written, bytes.len());
			}))),
			(32, 1) => bpf(Writer::MonoFloat32(Box::new(move |buffer| {
				let byte_count = buffer.len() * size_of::<f32>();
				let bytes = unsafe {
					std::slice::from_raw_parts(buffer.as_ptr() as *const u8, byte_count)
				};
				let written = ring.push(bytes);
				debug_assert_eq!(written, bytes.len());
			}))),
			(32, 2) => bpf(Writer::StereoFloat32(Box::new(move |buffer| {
				let byte_count = buffer.len() * size_of::<(f32, f32)>();
				let bytes = unsafe {
					std::slice::from_raw_parts(buffer.as_ptr() as *const u8, byte_count)
				};
				let written = ring.push(bytes);
				debug_assert_eq!(written, bytes.len());
			}))),
			_ => return Err(()),
		};

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

unsafe impl Send for Device {}
unsafe impl Sync for Device {}

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

		let destination = unsafe {
			std::slice::from_raw_parts_mut(buffer.mData as *mut u8, byte_count)
		};

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

fn get_audio_unit_stream_format(audio_unit: AudioUnit, scope: u32, element: u32) -> Result<AudioStreamBasicDescription, String> {
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
		return Err(
			"Invalid output stream format payload size. The most likely cause is that the audio unit returned an unexpected stream format structure size.".into()
		);
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
	for subtype in [AUDIO_UNIT_SUBTYPE_REMOTE_IO, kAudioUnitSubType_HALOutput, kAudioUnitSubType_DefaultOutput] {
		let mut description = AudioComponentDescription {
			componentType: kAudioUnitType_Output,
			componentSubType: subtype,
			componentManufacturer: kAudioUnitManufacturer_Apple,
			componentFlags: 0,
			componentFlagsMask: 0,
		};

		let component = unsafe {
			AudioComponentFindNext(std::ptr::null_mut(), NonNull::from(&mut description))
		};

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
}

impl SpscByteRing {
	// Creates a fixed-size lock-free SPSC ring buffer used by play() and the Core Audio callback.
	fn new(capacity: usize) -> Result<Self, String> {
		if capacity == 0 {
			return Err("Failed to create ring buffer. The most likely cause is that the computed buffer capacity was zero.".into());
		}

		Ok(Self {
			storage: vec![0; capacity].into_boxed_slice(),
			capacity,
			read_index: AtomicUsize::new(0),
			write_index: AtomicUsize::new(0),
		})
	}

	fn available_write(&self) -> usize {
		let read = self.read_index.load(Ordering::Acquire);
		let write = self.write_index.load(Ordering::Acquire);
		self.capacity - write.wrapping_sub(read)
	}

	// Pushes bytes into the ring buffer from the producer side.
	fn push(&self, source: &[u8]) -> usize {
		let read = self.read_index.load(Ordering::Acquire);
		let write = self.write_index.load(Ordering::Relaxed);

		let available = self.capacity - write.wrapping_sub(read);
		let to_write = source.len().min(available);

		if to_write == 0 {
			return 0;
		}

		let start = write % self.capacity;
		let first_len = to_write.min(self.capacity - start);

		unsafe {
			let destination = self.storage.as_ptr().cast_mut();
			std::ptr::copy_nonoverlapping(source.as_ptr(), destination.add(start), first_len);

			if to_write > first_len {
				std::ptr::copy_nonoverlapping(source.as_ptr().add(first_len), destination, to_write - first_len);
			}
		}

		self.write_index.store(write.wrapping_add(to_write), Ordering::Release);
		to_write
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
		to_read
	}
}

struct CallbackState {
	ring: SpscByteRing,
	underrun_count: AtomicUsize,
}
