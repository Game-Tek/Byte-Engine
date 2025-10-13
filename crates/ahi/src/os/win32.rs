use windows::Win32::{
	Foundation::S_OK,
	Media::{
		KernelStreaming::{WAVE_FORMAT_EXTENSIBLE, KSDATAFORMAT_SUBTYPE_PCM, SPEAKER_ALL, SPEAKER_FRONT_LEFT, SPEAKER_FRONT_RIGHT},
		Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
		Audio::{WAVEFORMATEXTENSIBLE as WAVEFORMATEXTENSIBLE_t, eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY},
	},
	System::Com::{CoCreateInstance, CoTaskMemFree, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,},
};

pub struct Device {
	device: IMMDevice,
	client: IAudioClient,
	render_client: IAudioRenderClient,
	parameters: HardwareParameters,
}

unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Option<Self> {
		if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } != S_OK {
			panic!("Failed to initialize COM");
		}

		let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };

		let device: IMMDevice = unsafe {
			enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap()
		};

		let (client, params) = unsafe {
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

			let m_format = if !m_closest_format.is_null() {
				let m_closest_format: &WAVEFORMATEXTENSIBLE_t = std::mem::transmute(m_closest_format);
				let closest_channels = m_closest_format.Format.nChannels;
				let closest_samples_per_second = m_closest_format.Format.nSamplesPerSec;
				let closest_bits_per_sample = m_closest_format.Format.wBitsPerSample;
				let closest_sub_format = m_closest_format.SubFormat;
				println!("Closest match available is :\n\t- Channels: {}\n\t- Samples per second: {}\n\t- Bits per Sample: {}\n\t- SubFormat: {:#?}", closest_channels, closest_samples_per_second, closest_bits_per_sample, closest_sub_format);

				m_closest_format
			} else {
				&m_format
			};

			client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, std::mem::transmute(m_format), None).unwrap();

			let bit_depth = m_format.Format.wBitsPerSample as u32;
			let sample_rate = m_format.Format.nSamplesPerSec as u32;
			let channels = m_format.Format.nChannels as u32;

			let params = HardwareParameters {
				sample_rate,
				channels,
				bit_depth,
			};

			if !m_closest_format.is_null() {
				CoTaskMemFree(Some(m_closest_format as _));
			}

			(client, params)
		};

		let render_client: IAudioRenderClient = unsafe {
			client.GetService().unwrap()
		};

		unsafe {
			client.Start().unwrap();
		}

		Device {
			device,
			client,
			render_client,
			parameters: params,
		}.into()
	}

	fn get_period_size(&self) -> usize {
		let period_size = unsafe {
			self.client.GetBufferSize().unwrap()
		};

		period_size as usize
	}

	fn play(&self, wpf: impl WritePlayFunction, _: impl BufferPlayFunction) -> Result<usize, ()> {
		let buffer_size = unsafe { self.client.GetBufferSize().unwrap() };
		let padding = unsafe { self.client.GetCurrentPadding().unwrap() };

		let available_space = buffer_size - padding;

		if available_space == 0 {
			return Ok(0);
		}

		match self.parameters.bit_depth {
			16 => {
				match self.parameters.channels {
					1 => {
						let buffer = unsafe {
							std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap() as *mut i16, available_space as usize)
						};

						wpf(Streams::Mono16Bit(buffer))
					}
					2 => {
						let buffer = unsafe {
							std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap() as *mut (i16, i16), available_space as usize)
						};

						wpf(Streams::Stereo16Bit(buffer))
					}
					_ => panic!()
				}
			}
			32 => {
				match self.parameters.channels {
					1 => {
						let buffer = unsafe {
							std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap() as *mut f32, available_space as usize)
						};

						wpf(Streams::MonoFloat32(buffer))
					}
					2 => {
						let buffer = unsafe {
							std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap() as *mut (f32, f32), available_space as usize)
						};

						wpf(Streams::StereoFloat32(buffer))
					}
					_ => panic!()
				}
			}
			_ => panic!()
		}

		unsafe {
			self.render_client.ReleaseBuffer(available_space as _, 0).unwrap();
		}

		Ok(available_space as usize)
	}

	fn pause(&self) {
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		unsafe {
			self.client.Stop().unwrap();
		}
	}
}
