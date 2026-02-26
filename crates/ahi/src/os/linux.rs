use std::sync::Mutex;

use crate::audio_hardware_interface::{HardwareParameters, Streams, WritePlayFunction};

pub struct Device {
	pcm: Mutex<alsa::pcm::PCM>,
	parameters: HardwareParameters,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Result<Self, String> {
		let name = std::ffi::CString::new("default").unwrap();
		let pcm = alsa::pcm::PCM::open(&name, alsa::Direction::Playback, false)
			.map_err(|e| format!("Failed to open PCM device: {}", e))?;

		let sample_size = (params.bit_depth.next_multiple_of(8) / 8) as usize;
		let channel_count = params.channels as usize;

		{
			let hwp = alsa::pcm::HwParams::any(&pcm).map_err(|e| format!("Failed to get hardware parameters: {}", e))?;
			hwp.set_channels(params.channels).map_err(|e| format!("Failed to set channels: {}", e))?;
			hwp.set_rate(params.sample_rate, alsa::ValueOr::Nearest).map_err(|e| format!("Failed to set sample rate: {}", e))?;
			hwp.set_format(match params.bit_depth {
				8 => alsa::pcm::Format::S8,
				16 => alsa::pcm::Format::S16LE,
				24 => return Err("24-bit audio is not supported by this implementation".to_string()),
				32 => alsa::pcm::Format::FloatLE,
				_ => return Err(format!("Unsupported bit depth: {}", params.bit_depth)),
			}).map_err(|e| format!("Failed to set format: {}", e))?;
			hwp.set_access(alsa::pcm::Access::RWInterleaved).map_err(|e| format!("Failed to set access type: {}", e))?;
			let effective_period_size = hwp.set_period_size_near(1024, alsa::ValueOr::Nearest).map_err(|e| format!("Failed to set period size: {}", e))?;
			let _ = hwp.set_buffer_size_near(effective_period_size * sample_size as i64 * channel_count as i64);

			pcm.hw_params(&hwp).map_err(|e| format!("Failed to apply hardware parameters: {}", e))?;
		}

		{
			let hwp = pcm.hw_params_current().map_err(|e| format!("Failed to get current hardware parameters: {}", e))?;
			let swp = pcm.sw_params_current().map_err(|e| format!("Failed to get current software parameters: {}", e))?;
			swp.set_start_threshold(hwp.get_buffer_size().map_err(|e| format!("Failed to get buffer size: {}", e))?).map_err(|e| format!("Failed to set start threshold: {}", e))?;
			pcm.sw_params(&swp).map_err(|e| format!("Failed to set software parameters: {}", e))?;
		}

		Ok(Device {
			pcm: Mutex::new(pcm),
			parameters: params,
		})
	}

	fn get_period_size(&self) -> usize {
		let pcm = &self.pcm.lock().unwrap();
		let hwp = pcm.hw_params_current().ok().unwrap();
		hwp.get_period_size().ok().unwrap() as usize
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, ()> {
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
					_ => return Err(()),
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
					_ => return Err(()),
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
