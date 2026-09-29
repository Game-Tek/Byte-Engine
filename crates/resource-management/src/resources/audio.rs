use crate::types::BitDepths;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
/// The `Audio` struct describes the interleaved PCM payload stored for runtime
/// sample playback.
pub struct Audio {
	/// Number of bits stored for each channel sample.
	pub bit_depth: BitDepths,
	/// Number of interleaved channels in each frame.
	pub channel_count: u16,
	/// Number of frames played per second.
	pub sample_rate: u32,
	/// Number of audio frames. A stereo frame contains two channel samples.
	pub sample_count: u32,
}

super::impl_direct_resource!(Audio, "Audio");
