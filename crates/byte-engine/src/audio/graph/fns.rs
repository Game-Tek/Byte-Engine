//! Concise functions for authoring an [`super::AudioGraph`].

use super::AudioGraph;

/// Creates a graph that plays one resource-backed sample once.
///
/// Next, pass the graph to `loop` or [`gain`], or publish it through
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
