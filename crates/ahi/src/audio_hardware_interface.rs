#[cfg(target_os = "windows")]
use windows::{core::GUID, Win32::{Media::{Audio::WAVEFORMATEXTENSIBLE as WAVEFORMATEXTENSIBLE_t, KernelStreaming::{SPEAKER_FRONT_LEFT, SPEAKER_FRONT_RIGHT, WAVE_FORMAT_EXTENSIBLE}, Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT}, System::Com::CoTaskMemFree}};
#[cfg(target_os = "windows")]
use windows::{core::HRESULT, Win32::{Foundation::S_OK, Media::Audio::{eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX}, System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED}}};

/// The `BufferPlayFunction` trait represents a function that returns a buffer of audio data.
/// This buffer will be consumed by the hardware to play the client's audio.
pub trait BufferPlayFunction<'a> = FnOnce() -> &'a [i16];

/// The `CopyPlayFunction` trait represents a function that copies audio data into a buffer.
/// This buffer is owned by the hardware and the client writes to it.
pub trait CopyPlayFunction<'a> = FnOnce(&'a mut [i16]);

/// The `AudioHardwareInterface` trait provides a common interface for audio hardware.
pub trait AudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> where Self: Sized;

	/// Sends audio data to the hardware.
	///
	/// This function takes two functions as arguments:
	/// - `bpf`: A function that returns a buffer of client audio data.
	/// - `cpf`: A function that copies client audio data into a buffer.
	///
	/// This is done as to optimize for each implementation because some hardware requires the client to provide a buffer of audio data,
	/// while others require the client to copy audio data into a buffer owned by the hardware.
	///
	/// Only one of the two functions will be called for each call to `play`.
	/// The play function used may change if the hardware parameters change.
	fn play<'a>(&self, bpf: impl BufferPlayFunction<'a>, cpf: impl CopyPlayFunction<'a>);

	/// Notifies the hardware that playback has been paused.
	/// This may be used to improve performance by reducing CPU usage.
	fn pause(&self);
}

pub struct HardwareParameters {
	sample_rate: u32,
	channels: u32,
	bit_depth: u32,
}

impl HardwareParameters {
	/// Creates a new `HardwareParameters` instance with default values.
	///
	/// # Default Values
	/// - Sample Rate: 48000 Hz
	/// - Channels: 2
	/// - Bit Depth: 16
	pub fn new() -> Self {
		HardwareParameters {
			sample_rate: 48000,
			channels: 2,
			bit_depth: 16,
		}
	}

	pub fn sample_rate(mut self, sample_rate: u32) -> Self {
		self.sample_rate = sample_rate;
		self
	}

	pub fn channels(mut self, channels: u32) -> Self {
		self.channels = channels;
		self
	}

	pub fn bit_depth(mut self, bit_depth: u32) -> Self {
		self.bit_depth = bit_depth;
		self
	}
}

enum PlayOperation<'a, B: FnOnce(&'a [i16]), C: FnOnce() -> &'a [i16]> {
	Buffer(B),
	Copy(C),
}

#[cfg(target_os = "linux")]
pub struct ALSAAudioHardwareInterface {
	pcm: Option<alsa::pcm::PCM>,
}

#[cfg(target_os = "linux")]
impl AudioHardwareInterface for ALSAAudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> {
		let name = std::ffi::CString::new("default").unwrap();
		let pcm = alsa::pcm::PCM::open(&name, alsa::Direction::Playback, false).ok()?;

		{
			let hwp = alsa::pcm::HwParams::any(&pcm).ok()?;
			hwp.set_channels(params.channels).ok()?;
			hwp.set_rate(params.sample_rate, alsa::ValueOr::Nearest).ok()?;
			hwp.set_format(match params.bit_depth {
				8 => alsa::pcm::Format::S8,
				16 => alsa::pcm::Format::S16LE,
				24 => return None,
				32 => alsa::pcm::Format::S32LE,
				_ => return None,
			}).ok()?;
			hwp.set_access(alsa::pcm::Access::RWInterleaved).ok()?;

			pcm.hw_params(&hwp).ok()?;
		}

		{
			let hwp = pcm.hw_params_current().ok()?;
			let swp = pcm.sw_params_current().ok()?;
			swp.set_start_threshold(hwp.get_buffer_size().ok()?).ok()?;
			pcm.sw_params(&swp).ok()?;
		}

		ALSAAudioHardwareInterface {
			pcm: Some(pcm),
		}.into()
	}

	fn play<'a>(&self, bpf: impl BufferPlayFunction<'a>, _: impl CopyPlayFunction<'a>) {
		let pcm = if let Some(pcm) = &self.pcm { pcm } else { return; };

		let io = pcm.io_i16().unwrap();

		let client_buffer = bpf();

		io.writei(client_buffer).unwrap();
	}

	fn pause(&self) {
		let pcm = if let Some(pcm) = &self.pcm { pcm } else { return; };

		pcm.pause(true).unwrap();
	}
}

#[cfg(target_os = "windows")]
pub struct WindowsAudioHardwareInterface {
	device: IMMDevice,
	client: IAudioClient,
	render_client: IAudioRenderClient,
}

#[cfg(target_os = "windows")]
impl AudioHardwareInterface for WindowsAudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> {
		if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } != S_OK {
			panic!("Failed to initialize COM");
		}

		let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };

		let device = unsafe {
			enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap()
		};

		let client: IAudioClient = unsafe {
			let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();

			let bits_per_sample = 32;
			let samples_per_second = 48000;
			let channels = 2;

			let m_format = WAVEFORMATEXTENSIBLE_t {
				Format: WAVEFORMATEX {
					nChannels: channels as _,
					nSamplesPerSec: samples_per_second,
					nBlockAlign: (channels * bits_per_sample / 8) as _,
					nAvgBytesPerSec: samples_per_second * (channels * bits_per_sample / 8),
					wBitsPerSample: bits_per_sample as _,
					wFormatTag: WAVE_FORMAT_EXTENSIBLE as _,
					cbSize: 22,
				},
				Samples: windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: bits_per_sample as _ },
				dwChannelMask: SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
				SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
			};

			client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, std::mem::transmute(&m_format), None).unwrap();

			client
		};

		let render_client: IAudioRenderClient = unsafe {
			client.GetService().unwrap()
		};

		unsafe {
			client.Start().unwrap();
		}

		WindowsAudioHardwareInterface {
			device,
			client,
			render_client,
		}.into()
	}

	fn play(&self, audio_data: &[i16]) {
		let buffer_size = unsafe { self.client.GetBufferSize().unwrap() };
		let padding = unsafe { self.client.GetCurrentPadding().unwrap() };

		let available_space = buffer_size - padding;

		if available_space == 0 {
			return;
		}

		let buffer = unsafe {
			std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap(), available_space as usize)
		};

		let audio_data = unsafe {
			std::slice::from_raw_parts(audio_data.as_ptr() as *const u8, audio_data.len() * std::mem::size_of::<i16>())
		};

		let write_len = std::cmp::min(buffer.len(), audio_data.len());

		buffer.copy_from_slice(&audio_data[..write_len]);

		unsafe {
			self.render_client.ReleaseBuffer(write_len as _, 0).unwrap();
		}
	}

	fn pause(&self) {
	}
}

#[cfg(target_os = "windows")]
impl Drop for WindowsAudioHardwareInterface {
	fn drop(&mut self) {
		unsafe {
			self.client.Stop().unwrap();
		}
	}
}

#[cfg(target_os = "linux")]
pub type AudioDevice = ALSAAudioHardwareInterface;

#[cfg(target_os = "windows")]
pub type AudioDevice = WindowsAudioHardwareInterface;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_ahi_hardware_parameters() {
		let params = HardwareParameters::new();
		assert_eq!(params.sample_rate, 48000);
		assert_eq!(params.channels, 2);
		assert_eq!(params.bit_depth, 16);
	}
}
