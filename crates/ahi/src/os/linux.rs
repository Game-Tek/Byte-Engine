use crate::audio_hardware_interface::{HardwareParameters, Streams, WritePlayFunction};

pub struct Device {
	pcm: alsa::pcm::PCM,
	parameters: HardwareParameters,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: HardwareParameters) -> Result<Self, String> {
		let name = std::ffi::CString::new("default").unwrap();
		let pcm = alsa::pcm::PCM::open(&name, alsa::Direction::Playback, false)
			.map_err(|e| format!("Failed to open PCM device: {}", e))?;

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
			hwp.set_access(alsa::pcm::Access::MMapInterleaved).map_err(|e| format!("Failed to set access type: {}", e))?;
			let effective_period_size = hwp.set_period_size_near(1024, alsa::ValueOr::Nearest).map_err(|e| format!("Failed to set period size: {}", e))?;
			let _ = hwp.set_buffer_size_near(effective_period_size * 2);

			pcm.hw_params(&hwp).map_err(|e| format!("Failed to apply hardware parameters: {}", e))?;
		}

		{
			let hwp = pcm.hw_params_current().map_err(|e| format!("Failed to get current hardware parameters: {}", e))?;
			let swp = pcm.sw_params_current().map_err(|e| format!("Failed to get current software parameters: {}", e))?;
			let period_size = hwp.get_period_size().map_err(|e| format!("Failed to get period size: {}", e))?;
			swp.set_start_threshold(period_size).map_err(|e| format!("Failed to set start threshold: {}", e))?;
			swp.set_avail_min(period_size).map_err(|e| format!("Failed to set minimum available frames: {}", e))?;
			pcm.sw_params(&swp).map_err(|e| format!("Failed to set software parameters: {}", e))?;
		}

		Ok(Device {
			pcm,
			parameters: params,
		})
	}

	fn get_period_size(&self) -> usize {
		let pcm = &self.pcm;
		let hwp = pcm.hw_params_current().ok().unwrap();
		hwp.get_period_size().ok().unwrap() as usize
	}

	fn play(&self, wpf: impl WritePlayFunction) -> Result<usize, ()> {
		let pcm = &self.pcm;

		let available_frames = match pcm.avail_update() {
			Ok(frames) if frames > 0 => frames as usize,
			Ok(_) => return Ok(0),
			Err(error) => {
				return if pcm.try_recover(error, true).is_ok() {
					Ok(0)
				} else {
					Err(())
				};
			}
		};

		let hw_params = pcm.hw_params_current().unwrap();
		let access = hw_params.get_access().unwrap();
		let period_size = self.get_period_size().min(available_frames);

		if period_size == 0 {
			return Ok(0);
		}

		match self.parameters.bit_depth {
			16 => {
				let io = pcm.io_i16().or(Err(()))?;

				let frames = match access {
					alsa::pcm::Access::MMapInterleaved => {
						let mmap_result = io.mmap(period_size, |b| {
							match self.parameters.channels {
								1 => {
									let frames = b.len();
									wpf(Streams::Mono16Bit(b));
									frames
								}
								2 => {
									if b.len() % 2 != 0 {
										return 0;
									}

									let frames = b.len() / 2;
									let buffer = unsafe {
										std::slice::from_raw_parts_mut(b.as_mut_ptr() as *mut (i16, i16), frames)
									};

									wpf(Streams::Stereo16Bit(buffer));
									frames
								}
								_ => panic!(),
							}
						});

						match mmap_result {
							Ok(frames) => frames,
							Err(error) => {
								return if pcm.try_recover(error, true).is_ok() {
									Ok(0)
								} else {
									Err(())
								};
							}
						}
					}
					_ => return Err(()),
				};

				if frames > 0 && pcm.state() == alsa::pcm::State::Prepared {
					pcm.start().or(Err(()))?;
				}

				Ok(frames)
			}
			32 => {
				let io = pcm.io_f32().or(Err(()))?;

				let frames = match access {
					alsa::pcm::Access::MMapInterleaved => {
						let mmap_result = io.mmap(period_size, |b| {
							match self.parameters.channels {
								1 => {
									let frames = b.len();
									wpf(Streams::MonoFloat32(b));
									frames
								}
								2 => {
									if b.len() % 2 != 0 {
										return 0;
									}

									let frames = b.len() / 2;
									let buffer = unsafe {
										std::slice::from_raw_parts_mut(b.as_mut_ptr() as *mut (f32, f32), frames)
									};

									wpf(Streams::StereoFloat32(buffer));
									frames
								}
								_ => panic!(),
							}
						});

						match mmap_result {
							Ok(frames) => frames,
							Err(error) => {
								return if pcm.try_recover(error, true).is_ok() {
									Ok(0)
								} else {
									Err(())
								};
							}
						}
					}
					_ => return Err(()),
				};

				if frames > 0 && pcm.state() == alsa::pcm::State::Prepared {
					pcm.start().or(Err(()))?;
				}

				Ok(frames)
			}
			_ => Err(()),
		}
	}

	fn pause(&self) {
		let pcm = &self.pcm;

		pcm.pause(true).unwrap();
	}
}
