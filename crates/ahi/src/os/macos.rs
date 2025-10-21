pub struct Device {

}

impl crate::audio_hardware_interface::AudioHardwareInterface for Device {
	fn new(params: crate::audio_hardware_interface::HardwareParameters) -> Option<Self> where Self: Sized {
		None
	}

	fn get_period_size(&self) -> usize {
		panic!()
	}

	fn play(&self, wpf: impl crate::audio_hardware_interface::WritePlayFunction, bpf: impl crate::audio_hardware_interface::BufferPlayFunction) -> Result<usize, ()> {
		Ok(0)
	}

	fn pause(&self) {

	}
}
