//! Concise functions for authoring an [`super::AudioGraph`].

use smallbox::SmallBox;
use smallvec::SmallVec;

use super::{AudioGraph, AudioGraphTime, AudioNode, AudioNodeId, CustomAudioFunction, RandomNode, RoundRobinNode};

/// Creates a graph that plays one resource-backed sample once.
///
/// Next, pass the graph to [`round_robin`], [`random`], `loop`, [`gain`],
/// [`varispeed`], or [`custom`], or publish it through
/// [`crate::audio::graph::AudioGraphFactory`].
pub fn sample(resource_id: impl Into<String>) -> AudioGraph {
	let mut nodes = SmallVec::new();
	nodes.push(SmallBox::new(AudioNode::Sample {
		resource_id: resource_id.into(),
	}));
	AudioGraph {
		nodes,
		output: AudioNodeId(0),
	}
}

/// Selects the next input chain each time this graph is submitted for playback.
///
/// A single input is returned directly without a selector node. Inputs can
/// contain any supported source, processing, or nested selector chain. A
/// looping selected chain remains selected until that play is stopped. Keep the
/// returned graph and pass it mutably to
/// [`super::AudioGraphFactory::create`] again to select the next input.
///
/// # Panics
///
/// Panics if no inputs are provided or if the combined graph would contain
/// more than 64 nodes.
pub fn round_robin(inputs: impl IntoIterator<Item = AudioGraph>) -> AudioGraph {
	AudioGraph::selector(inputs, "round-robin", |inputs| {
		AudioNode::RoundRobin(Box::new(RoundRobinNode { inputs, next_index: 0 }))
	})
}

/// Selects a random input chain each time this graph is submitted for playback.
///
/// The same input is never selected twice in sequence. A single input is
/// returned directly without a selector node. Inputs can contain any supported
/// source, processing, or selector chain. Keep the returned graph and pass it
/// mutably to [`super::AudioGraphFactory::create`] again to make another
/// selection.
///
/// # Panics
///
/// Panics if no inputs are provided or if the combined graph would contain
/// more than 64 nodes.
pub fn random(inputs: impl IntoIterator<Item = AudioGraph>) -> AudioGraph {
	AudioGraph::selector(inputs, "random", |inputs| {
		AudioNode::Random(Box::new(RandomNode::new(inputs)))
	})
}

/// Repeats the input graph until its lifecycle handle is deleted.
///
/// If the input already ends in a loop node, this returns it directly without
/// adding another node.
///
/// Next, pass the graph to [`gain`] or publish it through
/// [`crate::audio::graph::AudioGraphFactory`].
///
/// # Panics
///
/// Panics if a non-looping input graph already contains 64 nodes.
pub fn r#loop(mut input: AudioGraph) -> AudioGraph {
	if matches!(&*input.nodes[input.output.0], AudioNode::Loop { .. }) {
		return input;
	}
	let output = input.output;
	input.push(AudioNode::Loop { input: output });
	input
}

/// Applies a linear gain to every sample produced by the input graph.
///
/// A gain of `1.0` returns the input directly without a gain node.
///
/// Next, publish the graph through
/// [`crate::audio::graph::AudioGraphFactory`].
///
/// # Panics
///
/// Panics if `gain` is negative, infinite, or not a number. For a non-unity
/// gain, also panics if the input graph already contains 64 nodes.
pub fn gain(mut input: AudioGraph, gain: f32) -> AudioGraph {
	assert!(
		gain.is_finite() && gain >= 0.0,
		"Invalid audio graph gain. The gain must be finite and non-negative."
	);
	if gain == 1.0 {
		return input;
	}
	let output = input.output;
	input.push(AudioNode::Gain { input: output, gain });
	input
}

/// Changes the input playback speed and pitch by the same rate.
///
/// A rate of `1.0` returns the input directly without a varispeed node. A rate
/// of `2.0` plays it twice as fast and one octave higher, while `0.5` plays it
/// at half speed and one octave lower. Next, publish the graph through
/// [`crate::audio::graph::AudioGraphFactory`].
///
/// # Panics
///
/// Panics if `rate` is outside `0.25..=4.0`, infinite, or not a number. For a
/// non-unity rate, also panics if the input already contains a varispeed node
/// or if the graph already contains 64 nodes.
pub fn varispeed(mut input: AudioGraph, rate: f32) -> AudioGraph {
	assert!(
		rate.is_finite() && (0.25..=4.0).contains(&rate),
		"Invalid audio graph varispeed rate. The provided rate is not finite or is outside 0.25..=4.0."
	);
	if rate == 1.0 {
		return input;
	}

	assert!(
		!input.nodes.iter().any(|node| matches!(&**node, AudioNode::Varispeed { .. })),
		"Invalid audio graph varispeed. A second varispeed node was added to the graph."
	);
	let output = input.output;
	input.push(AudioNode::Varispeed { input: output, rate });
	input
}

/// Shifts the input pitch by a frequency ratio without changing its duration.
///
/// A ratio of `1.0` returns the input directly without a pitch-shift node. The
/// real-time processor uses a 1024-sample window, which adds about 21
/// milliseconds of latency at 48 kHz. Next, publish the graph through
/// [`crate::audio::graph::AudioGraphFactory`].
///
/// # Panics
///
/// Panics if `ratio` is outside `0.5..=2.0`, infinite, or not a number. For a
/// non-unity ratio, also panics if the input already contains a pitch-shift
/// node or if the graph already contains 64 nodes.
pub fn pitch_shift(mut input: AudioGraph, ratio: f32) -> AudioGraph {
	assert!(
		ratio.is_finite() && (0.5..=2.0).contains(&ratio),
		"Invalid audio graph pitch ratio. The ratio must be finite and between 0.5 and 2.0."
	);
	if ratio == 1.0 {
		return input;
	}

	assert!(
		!input.nodes.iter().any(|node| matches!(&**node, AudioNode::PitchShift { .. })),
		"Invalid audio graph pitch shift. A graph can contain at most one pitch-shift node."
	);
	let output = input.output;
	input.push(AudioNode::PitchShift { input: output, ratio });
	input
}

/// Processes each rendered audio block with a user-provided closure.
///
/// The closure receives the block's [`AudioGraphTime`] and the graph's mutable
/// mono sample block in place. Use [`AudioGraphTime::seconds_at`] for
/// non-periodic timing. For a periodic waveform, retain its phase in the
/// closure, calculate [`AudioGraphTime::periodic_phase_step`] once per block,
/// and advance each sample with [`AudioGraphTime::advance_periodic_phase`].
/// Each playback gets an independent clone of the closure, so captured mutable
/// state is not shared between simultaneous plays.
///
/// Custom functions are treated as zero-latency processors and cannot extend a
/// graph's playback tail. Produce every output sample within the supplied
/// block.
///
/// This closure runs on the audio worker. It must return quickly and must not
/// allocate, block, lock, perform resource I/O, or panic. Next, publish the
/// graph through
/// [`crate::audio::graph::AudioGraphFactory`].
///
/// # Panics
///
/// Panics if the input graph already contains 64 nodes.
pub fn custom<F>(mut input: AudioGraph, function: F) -> AudioGraph
where
	F: FnMut(AudioGraphTime, &mut [f32]) + Clone + Send + Sync + 'static,
{
	let output = input.output;
	input.push(AudioNode::Custom(output, CustomAudioFunction::new(function)));
	input
}
