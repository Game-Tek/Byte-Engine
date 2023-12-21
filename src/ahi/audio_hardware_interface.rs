use crate::orchestrator::{EntityReturn, Entity, System};

pub trait AudioHardwareInterface {
	fn play(&self);

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
			hwp.set_rate(44800, alsa::ValueOr::Nearest).unwrap();
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
	fn play(&self) {
		let pcm = if let Some(pcm) = &self.pcm { pcm } else { return; };

		// Make a sine wave
		let mut buf = [0i16; 1024];
		for (i, a) in buf.iter_mut().enumerate() {
			*a = ((i as f32 * 2.0 * ::std::f32::consts::PI / 32.0).sin() * 8192.0) as i16
		}

		let io = pcm.io_i16().unwrap();

		// Play it back for 2 seconds.
		for _ in 0..2*44800/1024 {
			assert_eq!(io.writei(&buf[..]).unwrap(), 1024);
		}

		// // In case the buffer was larger than 2 seconds, start the stream manually.
		// if pcm.state() != alsa::pcm::State::Running { pcm.start().unwrap() };
		// // Wait for the stream to finish playback.
		// pcm.drain().unwrap();
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