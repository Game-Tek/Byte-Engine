//! Concise functions for authoring an [`super::AudioGraph`].

use super::AudioGraph;

/// Creates a graph that plays one resource-backed sample once.
///
/// Next, pass the graph to `loop`, [`gain`], or [`varispeed`], or publish it through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
pub fn sample(resource_id: impl Into<String>) -> AudioGraph {
	AudioGraph::sample(resource_id)
}

/// Repeats the input graph until its lifecycle handle is deleted.
///
/// Next, pass the graph to [`gain`] or publish it through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if the input graph already contains eight nodes.
pub fn r#loop(input: AudioGraph) -> AudioGraph {
	input.looping()
}

/// Applies a linear gain to every sample produced by the input graph.
///
/// Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `gain` is negative, infinite, or not a number, or if the input
/// graph already contains eight nodes.
pub fn gain(input: AudioGraph, gain: f32) -> AudioGraph {
	input.with_gain(gain)
}

/// Changes the input playback speed and pitch by the same rate.
///
/// A rate of `1.0` leaves the input unchanged. A rate of `2.0` plays it twice
/// as fast and one octave higher, while `0.5` plays it at half speed and one
/// octave lower. Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `rate` is outside `0.25..=4.0`, infinite, or not a number, if the
/// input already contains a varispeed node, or if the graph already contains
/// eight nodes.
pub fn varispeed(input: AudioGraph, rate: f32) -> AudioGraph {
	input.with_varispeed(rate)
}

/// Shifts the input pitch by a frequency ratio without changing its duration.
///
/// A ratio of `1.0` leaves the pitch unchanged. The real-time processor uses a
/// 1024-sample window, which adds about 21 milliseconds of latency at 48 kHz.
/// Next, publish the graph through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
///
/// # Panics
///
/// Panics if `ratio` is outside `0.5..=2.0`, infinite, or not a number, if the
/// input already contains a pitch-shift node, or if the graph already contains
/// eight nodes.
pub fn pitch_shift(input: AudioGraph, ratio: f32) -> AudioGraph {
	input.with_pitch_shift(ratio)
}
