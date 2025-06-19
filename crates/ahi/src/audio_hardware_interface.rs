#[cfg(target_os = "linux")]
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use windows::Win32::{
	Foundation::S_OK,
	Media::{
		KernelStreaming::{WAVE_FORMAT_EXTENSIBLE, KSDATAFORMAT_SUBTYPE_PCM, SPEAKER_ALL, SPEAKER_FRONT_LEFT, SPEAKER_FRONT_RIGHT},
		Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
		Audio::{WAVEFORMATEXTENSIBLE as WAVEFORMATEXTENSIBLE_t, eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY},
	},
	System::Com::{CoCreateInstance, CoTaskMemFree, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,},
};

/// The `Mono16Bit` type represents a reference to a mono (single channel) audio buffer with signed 16-bit samples.
pub type Mono16Bit<'a> = &'a [i16];
/// The `Stereo16Bit` type represents a reference to a stereo (two channels) audio buffer with signed 16-bit samples.
pub type Stereo16Bit<'a> = &'a [(i16, i16)];
/// The `MonoFloat32` type represents a reference to a mono (single channel) audio buffer with 32-bit floating-point samples.
pub type MonoFloat32<'a> = &'a [f32];
/// The `StereoFloat32` type represents a reference to a stereo (two channels) audio buffer with 32-bit floating-point samples.
pub type StereoFloat32<'a> = &'a [(f32, f32)];

/// The `Mono16Bit` type represents a mutable reference to a mono (single channel) audio buffer with signed 16-bit samples.
pub type Mono16BitMut<'a> = &'a mut [i16];
/// The `Stereo16Bit` type represents a mutable reference to a stereo (two channels) audio buffer with signed 16-bit samples.
pub type Stereo16BitMut<'a> = &'a mut [(i16, i16)];
/// The `MonoFloat32` type represents a mutable reference to a mono (single channel) audio buffer with 32-bit floating-point samples.
pub type MonoFloat32Mut<'a> = &'a mut [f32];
/// The `StereoFloat32` type represents a mutable reference to a stereo (two channels) audio buffer with 32-bit floating-point samples.
pub type StereoFloat32Mut<'a> = &'a mut [(f32, f32)];

/// The `Streams` enum represents a buffer of audio in different formats.
pub enum Streams<'a> {
	/// Represents a mono (single channel) audio buffer with signed 16-bit samples.
	Mono16Bit(Mono16BitMut<'a>),
	/// Represents a stereo (two channels) audio buffer with signed 16-bit samples.
	Stereo16Bit(Stereo16BitMut<'a>),
	/// Represents a mono (single channel) audio buffer with 32-bit floating-point samples.
	MonoFloat32(MonoFloat32Mut<'a>),
	/// Represents a stereo (two channels) audio buffer with 32-bit floating-point samples.
	StereoFloat32(StereoFloat32Mut<'a>),
}

impl Streams<'_> {
	/// The `zero` method fills the buffer with zeros.
	pub fn zero(&mut self) {
		match self {
			Self::Mono16Bit(buf) => buf.fill(0),
			Self::Stereo16Bit(buf) => buf.fill((0, 0)),
			Self::MonoFloat32(buf) => buf.fill(0.0),
			Self::StereoFloat32(buf) => buf.fill((0.0, 0.0)),
		}
	}

	pub fn frames(&self) -> usize {
		match self {
			Self::Mono16Bit(buf) => buf.len() / 2,
			Self::Stereo16Bit(buf) => buf.len() / 2 / 2,
			Self::MonoFloat32(buf) => buf.len() / 4,
			Self::StereoFloat32(buf) => buf.len() / 4 / 2,
		}
	}
}

pub trait Mono16BitBufferPlayFunction = FnOnce(Mono16Bit);
pub trait Stereo16BitBufferPlayFunction = FnOnce(Stereo16Bit);
pub trait MonoFloat32BufferPlayFunction = FnOnce(MonoFloat32);
pub trait StereoFloat32BufferPlayFunction = FnOnce(StereoFloat32);

pub enum Writer<'a> {
	Mono16Bit(Box<dyn Mono16BitBufferPlayFunction + 'a>),
	Stereo16Bit(Box<dyn Stereo16BitBufferPlayFunction + 'a>),
	MonoFloat32(Box<dyn MonoFloat32BufferPlayFunction + 'a>),
	StereoFloat32(Box<dyn StereoFloat32BufferPlayFunction + 'a>),
}

/// The `WritePlayFunction` trait represents a function object that writes audio data into a buffer.
/// This buffer is owned by the hardware and the client writes to it.
pub trait WritePlayFunction = FnOnce(Streams);

pub trait BufferPlayFunction = FnOnce(Writer) -> usize;

/// The `AudioHardwareInterface` trait provides a common interface for audio hardware.
pub trait AudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> where Self: Sized;

	fn get_period_size(&self) -> usize;

	/// Sends audio data to the hardware.
	///
	/// This function takes a `WritePlayFunction` typed function object as argument that writes client audio data into a hardware buffer.
	///
	/// Returns the number of frames played.
	fn play(&self, wpf: impl WritePlayFunction, bpf: impl BufferPlayFunction) -> Result<usize, ()>;

	/// Notifies the hardware that playback has been paused.
	/// This may be used to improve performance by reducing CPU usage.
	fn pause(&self);
}

/// The `HardwareParameters` struct represents the parameters for the audio hardware.
pub struct HardwareParameters {
	/// The sample rate of the audio hardware, in Hz.
	sample_rate: u32,
	/// The number of channels in the audio hardware.
	channels: u32,
	/// The bit depth of the audio hardware.
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

#[cfg(target_os = "linux")]
pub struct ALSAAudioHardwareInterface {
	pcm: Mutex<alsa::pcm::PCM>,
	parameters: HardwareParameters,
}

#[cfg(target_os = "linux")]
impl AudioHardwareInterface for ALSAAudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> {
		let name = std::ffi::CString::new("default").unwrap();
		let pcm = alsa::pcm::PCM::open(&name, alsa::Direction::Playback, false).ok()?;

		let sample_size = (params.bit_depth.next_multiple_of(8) / 8) as usize;
		let channel_count = params.channels as usize;

		{
			let hwp = alsa::pcm::HwParams::any(&pcm).ok()?;
			hwp.set_channels(params.channels).ok()?;
			hwp.set_rate(params.sample_rate, alsa::ValueOr::Nearest).ok()?;
			hwp.set_format(match params.bit_depth {
				8 => alsa::pcm::Format::S8,
				16 => alsa::pcm::Format::S16LE,
				24 => return None,
				32 => alsa::pcm::Format::FloatLE,
				_ => return None,
			}).ok()?;
			hwp.set_access(alsa::pcm::Access::RWInterleaved).ok()?;
			let effective_period_size = hwp.set_period_size_near(1024, alsa::ValueOr::Nearest).ok()?;
			let _ = hwp.set_buffer_size_near(effective_period_size * sample_size as i64 * channel_count as i64);

			pcm.hw_params(&hwp).ok()?;
		}

		{
			let hwp = pcm.hw_params_current().ok()?;
			let swp = pcm.sw_params_current().ok()?;
			swp.set_start_threshold(hwp.get_buffer_size().ok()?).ok()?;
			pcm.sw_params(&swp).ok()?;
		}

		ALSAAudioHardwareInterface {
			pcm: Mutex::new(pcm),
			parameters: params,
		}.into()
	}

	fn get_period_size(&self) -> usize {
		let pcm = &self.pcm.lock().unwrap();
		let hwp = pcm.hw_params_current().ok().unwrap();
		hwp.get_period_size().ok().unwrap() as usize
	}

	fn play(&self, wpf: impl WritePlayFunction, bpf: impl BufferPlayFunction) -> Result<usize, ()> {
		let pcm = &self.pcm.lock().unwrap();

		let hw_params = pcm.hw_params_current().unwrap();
		let access = hw_params.get_access().unwrap();

		match self.parameters.bit_depth {
			16 => {
				let io = pcm.io_i16().or(Err(()))?;

				let frames = match access {
					alsa::pcm::Access::MMapInterleaved => {
						io.mmap(self.get_period_size(), |b| {
							match self.parameters.channels {
								1 => {
									let frames = b.len() / 2;
									wpf(Streams::Mono16Bit(b));
									frames
								}
								2 => {
									let frames = b.len() / 4;
									wpf(Streams::Stereo16Bit(unsafe { std::mem::transmute(b) }));
									frames
								}
								_ => panic!(),
							}
						}).or(Err(()))?
					}
					alsa::pcm::Access::RWInterleaved => {
						match self.parameters.channels {
							1 => {
								bpf(Writer::Mono16Bit(Box::new(|b| {
									io.writei(b);
								})))
							}
							2 => {
								bpf(Writer::Stereo16Bit(Box::new(|b| {
									io.writei(unsafe { std::mem::transmute(b) });
								})))
							}
							_ => panic!("Unsupported channel count"),
						}
					}
					_ => panic!("Unsupported access type"),
				};

				Ok(frames)
			}
			32 => {
				let io = pcm.io_f32().or(Err(()))?;

				let frames = match access {
					alsa::pcm::Access::MMapInterleaved => {
						io.mmap(self.get_period_size(), |b| {
							match self.parameters.channels {
								1 => {
									let frames = b.len() / 4;
									wpf(Streams::MonoFloat32(b));
									frames
								}
								2 => {
									let frames = b.len() / 8;
									wpf(Streams::StereoFloat32(unsafe { std::mem::transmute(b) }));
									frames
								}
								_ => panic!(),
							}
						}).or(Err(()))?
					}
					alsa::pcm::Access::RWInterleaved => {
						match self.parameters.channels {
							1 => {
								bpf(Writer::MonoFloat32(Box::new(|b| {
									io.writei(b);
								})))
							}
							2 => {
								bpf(Writer::StereoFloat32(Box::new(|b| {
									io.writei(unsafe { std::mem::transmute(b) });
								})))
							}
							_ => panic!("Unsupported channel count"),
						}
					}
					_ => panic!("Unsupported access type"),
				};

				Ok(frames)
			}
			_ => Err(()),
		}
	}

	fn pause(&self) {
		let pcm = &self.pcm.lock().unwrap();

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
unsafe impl Send for WindowsAudioHardwareInterface {}

#[cfg(target_os = "windows")]
unsafe impl Sync for WindowsAudioHardwareInterface {}

#[cfg(target_os = "windows")]
impl AudioHardwareInterface for WindowsAudioHardwareInterface {
	fn new(params: HardwareParameters) -> Option<Self> {
		if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } != S_OK {
			panic!("Failed to initialize COM");
		}

		let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };

		let device: IMMDevice = unsafe {
			enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap()
		};

		let client: IAudioClient = unsafe {
			let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();

			let bits_per_sample = params.bit_depth;
			let samples_per_second = params.sample_rate;
			let channels = params.channels;

			let dw_channel_mask = match channels {
				1 => SPEAKER_FRONT_LEFT,
				2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
				_ => SPEAKER_ALL,
			};

			let m_format = WAVEFORMATEXTENSIBLE_t {
				Format: WAVEFORMATEX {
					nChannels: channels as _,
					nSamplesPerSec: samples_per_second,
					nBlockAlign: (channels * bits_per_sample / 8) as _,
					nAvgBytesPerSec: samples_per_second * (channels * bits_per_sample / 8),
					wBitsPerSample: bits_per_sample.next_multiple_of(8) as _,
					wFormatTag: WAVE_FORMAT_EXTENSIBLE as _,
					cbSize: 22,
				},
				Samples: windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: bits_per_sample as _ },
				dwChannelMask: dw_channel_mask,
				SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
			};

			let mut m_closest_format: *const WAVEFORMATEXTENSIBLE_t = std::ptr::null();

			if client.IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, std::mem::transmute(&m_format), Some(std::mem::transmute(&mut m_closest_format))).is_err() {
				if !m_closest_format.is_null() {
					let m_closest_format: &WAVEFORMATEXTENSIBLE_t = std::mem::transmute(m_closest_format);
					let closest_channels = m_closest_format.Format.nChannels;
					let closest_samples_per_second = m_closest_format.Format.nSamplesPerSec;
					let closest_bits_per_sample = m_closest_format.Format.wBitsPerSample;
					let closest_sub_format = m_closest_format.SubFormat;
					panic!("Demanded audio format and/or parameters are not supported by the target audio device. Closest match available is :\n\t- Channels: {}\n\t- Samples per second: {}\n\t- Bits per Sample: {}\n\t- SubFormat: {:#?}", closest_channels, closest_samples_per_second, closest_bits_per_sample, closest_sub_format);
				}

				panic!("Demanded audio format and/or parameters are not supported by the target audio device.");
			}

			if !m_closest_format.is_null() {
				let m_closest_format: &WAVEFORMATEXTENSIBLE_t = std::mem::transmute(m_closest_format);
				let closest_channels = m_closest_format.Format.nChannels;
				let closest_samples_per_second = m_closest_format.Format.nSamplesPerSec;
				let closest_bits_per_sample = m_closest_format.Format.wBitsPerSample;
				let closest_sub_format = m_closest_format.SubFormat;
				println!("Closest match available is :\n\t- Channels: {}\n\t- Samples per second: {}\n\t- Bits per Sample: {}\n\t- SubFormat: {:#?}", closest_channels, closest_samples_per_second, closest_bits_per_sample, closest_sub_format);
			}

			let _ = ((1f32 / samples_per_second as f32) * 256f32) as i64 * 1_000_0000;

			client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, std::mem::transmute(&m_format), None).unwrap();

			if !m_closest_format.is_null() {
				CoTaskMemFree(Some(m_closest_format as _));
			}

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

	fn get_period_size(&self) -> usize {
		let period_size = unsafe {
			self.client.GetBufferSize().unwrap()
		};

		period_size as usize
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, ()> {
		let buffer_size = unsafe { self.client.GetBufferSize().unwrap() };
		let padding = unsafe { self.client.GetCurrentPadding().unwrap() };

		let available_space = buffer_size - padding;

		if available_space == 0 {
			return Ok(0);
		}

		let buffer = unsafe {
			std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap() as *mut i16, available_space as usize)
		};

		wpf(buffer);

		unsafe {
			self.render_client.ReleaseBuffer(available_space as _, 0).unwrap();
		}

		Ok(available_space as usize)
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
