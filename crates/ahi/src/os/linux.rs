use std::sync::Mutex;

pub struct Device {
	pcm: Mutex<alsa::pcm::PCM>,
	parameters: HardwareParameters,
}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
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

		Device {
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
									let _ = io.writei(b);
								})))
							}
							2 => {
								bpf(Writer::Stereo16Bit(Box::new(|b| {
									let _ = io.writei(unsafe { std::mem::transmute(b) });
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
									let _ = io.writei(b);
								})))
							}
							2 => {
								bpf(Writer::StereoFloat32(Box::new(|b| {
									let _ = io.writei(unsafe { std::mem::transmute(b) });
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
