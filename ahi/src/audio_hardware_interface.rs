pub trait AudioHardwareInterface {
	fn play(&self, audio_data: &[i16]);

	fn pause(&self);
}

struct ALSAAudioHardwareInterface {
	pcm: Option<alsa::pcm::PCM>,
}

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

pub fn create_ahi() -> impl AudioHardwareInterface {
	#[cfg(target_os = "linux")]
	ALSAAudioHardwareInterface::open()
}