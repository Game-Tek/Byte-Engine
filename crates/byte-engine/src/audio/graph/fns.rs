//! Concise functions for authoring an [`super::AudioGraph`].

use super::AudioGraph;

/// Creates a graph that plays one resource-backed sample once.
///
/// Next, pass the graph to [`round_robin`], [`random`], `loop`, [`gain`], or
/// [`varispeed`], or publish it through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
pub fn sample(resource_id: impl Into<String>) -> AudioGraph {
	AudioGraph::sample(resource_id)
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
	AudioGraph::round_robin(inputs)
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
	AudioGraph::random(inputs)
}

/// Repeats the input graph until its lifecycle handle is deleted.
///
/// If the input already ends in a loop node, this returns it directly without
/// adding another node.
///
/// Next, pass the graph to [`gain`] or publish it through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if a non-looping input graph already contains 64 nodes.
pub fn r#loop(input: AudioGraph) -> AudioGraph {
	input.looping()
}

/// Applies a linear gain to every sample produced by the input graph.
///
/// A gain of `1.0` returns the input directly without a gain node.
///
/// Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `gain` is negative, infinite, or not a number. For a non-unity
/// gain, also panics if the input graph already contains 64 nodes.
pub fn gain(input: AudioGraph, gain: f32) -> AudioGraph {
	input.with_gain(gain)
}

/// Changes the input playback speed and pitch by the same rate.
///
/// A rate of `1.0` returns the input directly without a varispeed node. A rate
/// of `2.0` plays it twice as fast and one octave higher, while `0.5` plays it
/// at half speed and one octave lower. Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `rate` is outside `0.25..=4.0`, infinite, or not a number. For a
/// non-unity rate, also panics if the input already contains a varispeed node
/// or if the graph already contains 64 nodes.
pub fn varispeed(input: AudioGraph, rate: f32) -> AudioGraph {
	input.with_varispeed(rate)
}

/// Shifts the input pitch by a frequency ratio without changing its duration.
///
/// A ratio of `1.0` returns the input directly without a pitch-shift node. The
/// real-time processor uses a 1024-sample window, which adds about 21
/// milliseconds of latency at 48 kHz. Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `ratio` is outside `0.5..=2.0`, infinite, or not a number. For a
/// non-unity ratio, also panics if the input already contains a pitch-shift
/// node or if the graph already contains 64 nodes.
pub fn pitch_shift(input: AudioGraph, ratio: f32) -> AudioGraph {
	input.with_pitch_shift(ratio)
}
