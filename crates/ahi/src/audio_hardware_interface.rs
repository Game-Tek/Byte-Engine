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
	fn new(params: HardwareParameters) -> Result<Self, String> where Self: Sized;

	fn get_period_size(&self) -> usize;

	/// Returns the number of hardware callback cycles that encountered at least one buffer underrun.
	///
	/// Backends that cannot provide this metric should return `0`.
	fn get_underrun_count(&self) -> usize {
		0
	}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HardwareParameters {
	/// The sample rate of the audio hardware, in Hz.
	pub(crate) sample_rate: u32,
	/// The number of channels in the audio hardware.
	pub(crate) channels: u32,
	/// The bit depth of the audio hardware.
	pub(crate) bit_depth: u32,
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

	pub fn get_sample_rate(&self) -> u32 {
		self.sample_rate
	}

	pub fn get_channels(&self) -> u32 {
		self.channels
	}

	pub fn get_bit_depth(&self) -> u32 {
		self.bit_depth
	}
}

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
