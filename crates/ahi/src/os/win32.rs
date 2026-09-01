use windows::Win32::{
	Foundation::S_OK,
	Media::{
		Audio::{
			AUDCLNT_SHAREMODE_SHARED, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
			WAVEFORMATEX, WAVEFORMATEXTENSIBLE as WAVEFORMATEXTENSIBLE_t, eConsole, eRender,
		},
		KernelStreaming::{
			KSDATAFORMAT_SUBTYPE_PCM, SPEAKER_ALL, SPEAKER_FRONT_LEFT, SPEAKER_FRONT_RIGHT, WAVE_FORMAT_EXTENSIBLE,
		},
	},
	System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree},
};

use crate::audio_hardware_interface::{AudioPlayError, HardwareParameters, Streams, WritePlayFunction};

/// Keeps the optional WASAPI closest-match allocation alive while the audio client is initialized.
struct ClosestFormat(*mut WAVEFORMATEX);

impl ClosestFormat {
	fn new() -> Self {
		Self(std::ptr::null_mut())
	}

	fn output_pointer(&mut self) -> *mut *mut WAVEFORMATEX {
		std::ptr::from_mut(&mut self.0)
	}

	fn get(&self) -> Option<&WAVEFORMATEX> {
		if self.0.is_null() {
			None
		} else {
			// SAFETY: WASAPI returned this non-null pointer through IsFormatSupported and keeps it valid until CoTaskMemFree.
			Some(unsafe { &*self.0 })
		}
	}
}

impl Drop for ClosestFormat {
	fn drop(&mut self) {
		if !self.0.is_null() {
			// SAFETY: IsFormatSupported allocated this pointer with the COM task allocator, and this wrapper owns it.
			unsafe { CoTaskMemFree(Some(self.0 as _)) };
		}
	}
}

/// Creates the default render endpoint after initializing COM for the calling thread.
fn create_default_device() -> Result<IMMDevice, String> {
	// SAFETY: The current thread initializes COM before using any COM audio interface.
	if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } != S_OK {
		return Err("Failed to initialize COM. The calling thread could not enter the multithreaded apartment.".to_string());
	}

	// SAFETY: COM is initialized on this thread, and MMDeviceEnumerator is an in-process COM class.
	let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|_| {
		"Failed to create device enumerator. The COM class for MMDeviceEnumerator could not be instantiated.".to_string()
	})?;

	// SAFETY: The enumerator is a live COM interface owned by this thread.
	unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.map_err(|_| {
		"Failed to get default audio endpoint. The system has no default render device or it is unavailable.".to_string()
	})
}

/// Builds the extensible PCM format WASAPI uses to negotiate the requested hardware parameters.
fn requested_format(params: HardwareParameters) -> WAVEFORMATEXTENSIBLE_t {
	let bits_per_sample = params.bit_depth;
	let channels = params.channels;
	let channel_mask = match channels {
		1 => SPEAKER_FRONT_LEFT,
		2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
		_ => SPEAKER_ALL,
	};

	WAVEFORMATEXTENSIBLE_t {
		Format: WAVEFORMATEX {
			nChannels: channels as _,
			nSamplesPerSec: params.sample_rate,
			nBlockAlign: (channels * bits_per_sample / 8) as _,
			nAvgBytesPerSec: params.sample_rate * (channels * bits_per_sample / 8),
			wBitsPerSample: bits_per_sample.next_multiple_of(8) as _,
			wFormatTag: WAVE_FORMAT_EXTENSIBLE as _,
			cbSize: 22,
		},
		Samples: windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE_0 {
			wValidBitsPerSample: bits_per_sample as _,
		},
		dwChannelMask: channel_mask,
		SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
	}
}

/// Negotiates a supported stream format and initializes the endpoint's audio client.
fn create_audio_client(
	device: &IMMDevice,
	requested_parameters: HardwareParameters,
) -> Result<(IAudioClient, HardwareParameters), String> {
	// SAFETY: The endpoint is a live COM interface, and Windows infers the requested IAudioClient interface from this type.
	let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|_| {
		"Failed to activate audio client. The audio endpoint could not provide an IAudioClient interface.".to_string()
	})?;
	let requested_format = requested_format(requested_parameters);
	let mut closest_format = ClosestFormat::new();

	// SAFETY: Both pointers are valid for the call, and ClosestFormat owns any COM allocation returned through the output pointer.
	let support = unsafe {
		client.IsFormatSupported(
			AUDCLNT_SHAREMODE_SHARED,
			std::ptr::from_ref(&requested_format.Format),
			Some(closest_format.output_pointer()),
		)
	};
	if support.is_err() {
		let closest = closest_format.get().map_or_else(String::new, |format| {
			let channels = format.nChannels;
			let samples_per_second = format.nSamplesPerSec;
			let bits_per_sample = format.wBitsPerSample;
			format!(
				" Closest match: {} channels, {} samples per second, and {} bits per sample.",
				channels, samples_per_second, bits_per_sample
			)
		});
		return Err(format!(
			"Unsupported audio format. The target audio device does not support the requested parameters.{closest}"
		));
	}

	let selected_format = closest_format.get().unwrap_or(&requested_format.Format);
	// SAFETY: The selected format is either the live requested value or WASAPI's live closest-match allocation.
	unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, std::ptr::from_ref(selected_format), None) }.map_err(
		|_| "Failed to initialize audio client. The device rejected the requested stream format or parameters.".to_string(),
	)?;

	let parameters = HardwareParameters {
		sample_rate: selected_format.nSamplesPerSec,
		channels: u32::from(selected_format.nChannels),
		bit_depth: u32::from(selected_format.wBitsPerSample),
	};
	Ok((client, parameters))
}

pub struct Device {
	_device: IMMDevice,
	client: IAudioClient,
	render_client: IAudioRenderClient,
	parameters: HardwareParameters,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Result<Self, String> {
		let device = create_default_device()?;
		let (client, params) = create_audio_client(&device, params)?;

		// SAFETY: The initialized audio client exposes services for the negotiated render stream.
		let render_client: IAudioRenderClient = unsafe { client.GetService() }.map_err(|_| {
			"Failed to get render client service. The audio client did not expose IAudioRenderClient.".to_string()
		})?;

		// SAFETY: The client has a negotiated shared-mode format and has not started yet.
		unsafe { client.Start() }.map_err(|_| {
			"Failed to start audio stream. The audio client could not transition to the running state.".to_string()
		})?;

		Ok(Device {
			_device: device,
			client,
			render_client,
			parameters: params,
		})
	}

	fn get_period_size(&self) -> usize {
		// SAFETY: The audio client remains initialized for the lifetime of this device.
		let period_size = unsafe { self.client.GetBufferSize().unwrap() };

		period_size as usize
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, AudioPlayError> {
		// SAFETY: The audio client remains initialized and running while Device is alive.
		let buffer_size = unsafe { self.client.GetBufferSize().unwrap() };
		// SAFETY: The audio client remains initialized and running while Device is alive.
		let padding = unsafe { self.client.GetCurrentPadding().unwrap() };

		let available_space = buffer_size - padding;

		if available_space == 0 {
			return Ok(0);
		}

		match self.parameters.bit_depth {
			16 => match self.parameters.channels {
				1 => {
					// SAFETY: The negotiated format stores one i16 sample in each frame.
					unsafe { self.write_render_buffer::<i16>(available_space, |buffer| wpf(Streams::Mono16Bit(buffer))) };
				}
				2 => {
					// SAFETY: The negotiated format stores two i16 samples in each frame.
					unsafe {
						self.write_render_buffer::<(i16, i16)>(available_space, |buffer| wpf(Streams::Stereo16Bit(buffer)));
					}
				}
				_ => panic!(),
			},
			32 => match self.parameters.channels {
				1 => {
					// SAFETY: The negotiated format stores one f32 sample in each frame.
					unsafe { self.write_render_buffer::<f32>(available_space, |buffer| wpf(Streams::MonoFloat32(buffer))) };
				}
				2 => {
					// SAFETY: The negotiated format stores two f32 samples in each frame.
					unsafe {
						self.write_render_buffer::<(f32, f32)>(available_space, |buffer| wpf(Streams::StereoFloat32(buffer)));
					}
				}
				_ => panic!(),
			},
			_ => panic!(),
		}

		// SAFETY: GetBuffer succeeded for this frame count, and the writer no longer borrows the returned buffer.
		unsafe { self.render_client.ReleaseBuffer(available_space, 0) }.unwrap();

		Ok(available_space as usize)
	}

	fn pause(&self) {}
}

impl Device {
	/// Gives the writer the current hardware buffer interpreted as the negotiated sample-frame type.
	///
	/// # Safety
	///
	/// `T` must match the sample-frame layout selected by `self.parameters`.
	unsafe fn write_render_buffer<T>(&self, frame_count: u32, writer: impl FnOnce(&mut [T])) {
		// SAFETY: The caller guarantees the frame type matches the negotiated stream, and no render buffer is currently borrowed.
		let buffer = unsafe { self.render_client.GetBuffer(frame_count) }.unwrap().cast::<T>();
		// SAFETY: WASAPI returned a writable buffer containing frame_count values of the negotiated frame type.
		let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, frame_count as usize) };
		writer(buffer);
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		// SAFETY: This client was started during construction and is stopped exactly once before its interfaces are released.
		unsafe { self.client.Stop() }.unwrap();
	}
}
