use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

struct AudioSystem {
	host: cpal::Host,
	stream: Option<cpal::Stream>,
}

impl AudioSystem {
	fn new() -> Self {
		let host = cpal::default_host();

		let device = if let Some(e) = host.default_output_device() {
			e
		} else {
			log::error!("No audio output device found.\n Audio playback will be disabled.");
			
			return AudioSystem {
				host,
				stream: None,
			};
		};

		let mut format = device.default_output_config().unwrap();

		let stream = device.build_output_stream(&format.config(), move |_: &mut [u16], _| {}, move |_| {}, None).unwrap();

		if let Err(error) = stream.play() {
			log::error!("Failed to start audio stream: {}\n Audio playback will be disabled.", error);
		}

		AudioSystem {
			host,
			stream: Some(stream),
		}
	}
}