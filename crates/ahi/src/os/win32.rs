use std::{
	cell::{Cell, RefCell},
	time::{Duration, Instant},
};

use windows::Win32::{
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
	// SAFETY: The current thread initializes COM before using any COM audio interface. A thread that reopens a lost
	// device already entered the apartment, which COM reports as `S_FALSE`, another success.
	if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
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

/// How long a device that lost its endpoint waits between attempts to open the default endpoint again.
const REOPEN_INTERVAL: Duration = Duration::from_secs(1);

/// How long a device without an endpoint sleeps when asked to wait for playback space, instead of spinning.
const SILENT_WAIT: Duration = Duration::from_millis(10);

/// The `Stream` struct is a started WASAPI render stream on one endpoint.
struct Stream {
	_device: IMMDevice,
	client: IAudioClient,
	render_client: IAudioRenderClient,
	parameters: HardwareParameters,
}

impl Stream {
	/// Opens the default render endpoint and starts a shared-mode stream as close to `requested` as it supports.
	fn open(requested: HardwareParameters) -> Result<Self, String> {
		let device = create_default_device()?;
		let (client, parameters) = create_audio_client(&device, requested)?;

		// SAFETY: The initialized audio client exposes services for the negotiated render stream.
		let render_client: IAudioRenderClient = unsafe { client.GetService() }.map_err(|_| {
			"Failed to get render client service. The audio client did not expose IAudioRenderClient.".to_string()
		})?;

		// SAFETY: The client has a negotiated shared-mode format and has not started yet.
		unsafe { client.Start() }.map_err(|_| {
			"Failed to start audio stream. The audio client could not transition to the running state.".to_string()
		})?;

		Ok(Self {
			_device: device,
			client,
			render_client,
			parameters,
		})
	}

	/// Returns how many frames the stream's buffer holds.
	fn buffer_size(&self) -> windows::core::Result<u32> {
		// SAFETY: The audio client stays initialized while the stream owns it.
		unsafe { self.client.GetBufferSize() }
	}

	/// Writes up to `frame_limit` frames through `wpf` and returns how many it submitted.
	fn play(&self, frame_limit: u32, wpf: impl WritePlayFunction) -> Result<u32, StreamError> {
		let buffer_size = self.buffer_size()?;
		// SAFETY: The audio client stays initialized and started while the stream owns it.
		let padding = unsafe { self.client.GetCurrentPadding() }?;
		// A replacement endpoint may hold more frames than the period the caller sized its mix buffers for.
		let available_space = buffer_size.saturating_sub(padding).min(frame_limit);

		if available_space == 0 {
			return Ok(0);
		}

		match self.parameters.bit_depth {
			16 => match self.parameters.channels {
				1 => {
					// SAFETY: The negotiated format stores one i16 sample in each frame.
					unsafe { self.write_render_buffer::<i16>(available_space, |buffer| wpf(Streams::Mono16Bit(buffer))) }?;
				}
				2 => {
					// SAFETY: The negotiated format stores two i16 samples in each frame.
					unsafe {
						self.write_render_buffer::<(i16, i16)>(available_space, |buffer| wpf(Streams::Stereo16Bit(buffer)))?;
					}
				}
				_ => return Err(StreamError::Format),
			},
			32 => match self.parameters.channels {
				1 => {
					// SAFETY: The negotiated format stores one f32 sample in each frame.
					unsafe { self.write_render_buffer::<f32>(available_space, |buffer| wpf(Streams::MonoFloat32(buffer))) }?;
				}
				2 => {
					// SAFETY: The negotiated format stores two f32 samples in each frame.
					unsafe {
						self.write_render_buffer::<(f32, f32)>(available_space, |buffer| wpf(Streams::StereoFloat32(buffer)))?;
					}
				}
				_ => return Err(StreamError::Format),
			},
			_ => return Err(StreamError::Format),
		}

		// SAFETY: GetBuffer succeeded for this frame count, and the writer no longer borrows the returned buffer.
		unsafe { self.render_client.ReleaseBuffer(available_space, 0) }?;

		Ok(available_space)
	}

	/// Gives the writer the current hardware buffer interpreted as the negotiated sample-frame type.
	///
	/// # Safety
	///
	/// `T` must match the sample-frame layout selected by `self.parameters`.
	unsafe fn write_render_buffer<T>(&self, frame_count: u32, writer: impl FnOnce(&mut [T])) -> windows::core::Result<()> {
		// SAFETY: The caller guarantees the frame type matches the negotiated stream, and no render buffer is currently borrowed.
		let buffer = unsafe { self.render_client.GetBuffer(frame_count) }?.cast::<T>();
		// SAFETY: WASAPI returned a writable buffer containing frame_count values of the negotiated frame type.
		let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, frame_count as usize) };
		writer(buffer);
		Ok(())
	}
}

impl Drop for Stream {
	fn drop(&mut self) {
		// SAFETY: This client was started when the stream opened and is stopped once before its interfaces are released.
		// A lost endpoint fails the call, which leaves nothing to stop.
		let _ = unsafe { self.client.Stop() };
	}
}

/// The `StreamError` enum tells why a stream could not take a period.
enum StreamError {
	/// A WASAPI call failed, most often because the endpoint went away, such as an unplugged headset or a monitor that
	/// carried the audio going to sleep.
	Lost(windows::core::Error),
	/// The endpoint negotiated a channel count or bit depth this backend can't write.
	Format,
}

impl From<windows::core::Error> for StreamError {
	fn from(error: windows::core::Error) -> Self {
		Self::Lost(error)
	}
}

/// The `Device` struct plays through the default render endpoint and outlives it.
///
/// When there is no endpoint, at creation or because it went away, the device plays silence and opens the default endpoint again, at most once every
/// [`REOPEN_INTERVAL`], so audio returns when a device does.
pub struct Device {
	/// The running stream, or `None` while no endpoint is available.
	stream: RefCell<Option<Stream>>,
	/// The format the device asked for, which a reopened stream asks for again.
	requested: HardwareParameters,
	/// The frames one period holds, fixed when the device opened, since callers size their mix buffers by it.
	period_size: u32,
	/// When the device last tried to open an endpoint after losing one.
	last_reopen: Cell<Instant>,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Result<Self, String> {
		let opened = Stream::open(params).and_then(|stream| {
			let period_size = stream.buffer_size().map_err(|error| {
				format!("Failed to read the audio buffer size. The audio endpoint went away while it opened. {error}")
			})?;
			Ok((stream, period_size))
		});
		// Without an endpoint the device starts silent, with WASAPI's usual shared-mode period of 10 ms.
		let (stream, period_size) = match opened {
			Ok((stream, period_size)) => (Some(stream), period_size),
			Err(error) => {
				log::warn!("Audio playback starts silently. {error} Audio starts when a default audio device is available.");
				(None, (params.get_sample_rate() / 100).max(1))
			}
		};
		Ok(Device {
			stream: RefCell::new(stream),
			requested: params,
			period_size,
			last_reopen: Cell::new(Instant::now()),
		})
	}

	fn get_period_size(&self) -> usize {
		self.period_size as usize
	}

	fn wait_for_playback_space(&self) {
		if self.stream.borrow().is_none() {
			std::thread::sleep(SILENT_WAIT);
		} else {
			std::thread::yield_now();
		}
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, AudioPlayError> {
		let mut stream = self.stream.borrow_mut();
		if stream.is_none() && self.last_reopen.get().elapsed() >= REOPEN_INTERVAL {
			self.last_reopen.set(Instant::now());
			if let Ok(reopened) = Stream::open(self.requested) {
				log::info!("Audio playback resumed on the default audio device.");
				*stream = Some(reopened);
			}
		}
		let Some(current) = stream.as_ref() else {
			return Ok(0);
		};
		match current.play(self.period_size, wpf) {
			Ok(frames) => Ok(frames as usize),
			Err(StreamError::Lost(error)) => {
				log::warn!(
					"Audio playback lost its device and continues silently. The most likely cause is that the audio device was removed or disabled. Audio resumes when a default audio device is available. {error}"
				);
				*stream = None;
				self.last_reopen.set(Instant::now());
				Ok(0)
			}
			Err(StreamError::Format) => Err(AudioPlayError::UnsupportedFormat),
		}
	}
}
