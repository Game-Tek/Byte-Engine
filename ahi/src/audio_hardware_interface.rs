use windows::{core::HRESULT, Win32::{Foundation::S_OK, Media::Audio::{eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX}, System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED}}};

pub trait AudioHardwareInterface {
	fn play(&self, audio_data: &[i16]);

	fn pause(&self);
}

#[cfg(target_os = "linux")]
struct ALSAAudioHardwareInterface {
	pcm: Option<alsa::pcm::PCM>,
}

#[cfg(target_os = "linux")]
impl ALSAAudioHardwareInterface {
	fn open() -> Self {
		let name = std::ffi::CString::new("default").unwrap();
		let pcm = alsa::pcm::PCM::open(&name, alsa::Direction::Playback, false).unwrap();

		{
			let hwp = alsa::pcm::HwParams::any(&pcm).unwrap();
			hwp.set_channels(1).unwrap();
			hwp.set_rate(48000, alsa::ValueOr::Nearest).unwrap();
			hwp.set_format(alsa::pcm::Format::s16()).unwrap();
			hwp.set_access(alsa::pcm::Access::RWInterleaved).unwrap();

			pcm.hw_params(&hwp).unwrap();
		}

		{
			let hwp = pcm.hw_params_current().unwrap();
			let swp = pcm.sw_params_current().unwrap();
			swp.set_start_threshold(hwp.get_buffer_size().unwrap()).unwrap();
			pcm.sw_params(&swp).unwrap();
		}

		ALSAAudioHardwareInterface {
			pcm: Some(pcm),
		}
	}
}

#[cfg(target_os = "linux")]
impl AudioHardwareInterface for ALSAAudioHardwareInterface {
	fn play(&self, audio_data: &[i16]) {
		let pcm = if let Some(pcm) = &self.pcm { pcm } else { return; };

		let io = pcm.io_i16().unwrap();

		io.writei(&audio_data[..]).unwrap();
	}

	fn pause(&self) {
		let pcm = if let Some(pcm) = &self.pcm { pcm } else { return; };

		pcm.pause(true).unwrap();
	}
}

#[cfg(target_os = "windows")]
struct WindowsAudioHardwareInterface {
	device: IMMDevice,
	client: IAudioClient,
	render_client: IAudioRenderClient,
}

#[cfg(target_os = "windows")]
impl AudioHardwareInterface for WindowsAudioHardwareInterface {
	fn play(&self, audio_data: &[i16]) {
		let buffer_size = unsafe { self.client.GetBufferSize().unwrap() };
		let padding = unsafe { self.client.GetCurrentPadding().unwrap() };

		let available_space = buffer_size - padding;

		let buffer = unsafe {
			std::slice::from_raw_parts_mut(self.render_client.GetBuffer(available_space).unwrap(), available_space as usize)
		};

		let audio_data = unsafe {
			std::slice::from_raw_parts(audio_data.as_ptr() as *const u8, audio_data.len() * std::mem::size_of::<i16>())
		};

		buffer.copy_from_slice(&audio_data[..buffer.len()])
	}

	fn pause(&self) {
	}
}

#[cfg(target_os = "windows")]
impl WindowsAudioHardwareInterface {
	fn open() -> Self {
		if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } != S_OK {
			panic!("Failed to initialize COM");
		}

		let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };

		let device = unsafe {
			enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap()
		};

		let client: IAudioClient = unsafe {
			let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
			let format = WAVEFORMATEX {
				cbSize: 22,
				nChannels: 2,
				nSamplesPerSec: 48000,
				nBlockAlign: 4,
				nAvgBytesPerSec: 48000 * 2 * 2,
				..Default::default()
			};
			client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, &format, None).unwrap();
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
		}
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

pub fn create_ahi() -> impl AudioHardwareInterface {
	#[cfg(target_os = "linux")] {
		ALSAAudioHardwareInterface::open()
	}

	#[cfg(target_os = "windows")] {
		WindowsAudioHardwareInterface::open()
	}
}