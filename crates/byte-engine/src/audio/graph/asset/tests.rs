use std::collections::HashMap;

use resource_management::{
	asset::handler::implementations::audio_graph::bake_audio_graph, resources::audio_graph::AudioGraph as BakedAudioGraph,
};

use super::super::{AudioGraph, AudioProcessor, PlaybackRate};

/// Bakes each `(id, source)` file the way the asset handler stores it.
fn bake(files: &[(&str, &str)]) -> HashMap<String, BakedAudioGraph> {
	files
		.iter()
		.map(|(id, source)| {
			let graph = bake_audio_graph(id, source.as_bytes()).unwrap_or_else(|error| panic!("{id} should bake: {error}"));
			(id.to_string(), graph)
		})
		.collect()
}

fn expand(files: &HashMap<String, BakedAudioGraph>, id: &str) -> Result<AudioGraph, String> {
	AudioGraph::expand(id, |id| files.get(id))
}

/// Spawns the graph `count` times and returns the sample each spawn plays.
fn spawns(graph: &mut AudioGraph, count: usize) -> Vec<String> {
	(0..count)
		.map(|_| graph.select().expect("a valid graph should select").resource_id)
		.collect()
}

/// A round robin over `count` samples, which bakes to `count + 1` nodes.
fn round_robin_of(count: usize) -> String {
	let entries = (0..count)
		.map(|index| format!("{{ sample: \"audio/{index}.wav\" }}"))
		.collect::<Vec<_>>();
	format!("{{ round_robin: [{}] }}", entries.join(","))
}

#[test]
fn a_nested_graph_plays_as_if_written_in_place() {
	let files = bake(&[
		(
			"sfx/parent.audiograph",
			r#"{ gain: 0.5, input: { graph: "sfx/child.audiograph" } }"#,
		),
		(
			"sfx/child.audiograph",
			r#"{ varispeed: 2, input: { sample: "audio/a.wav" } }"#,
		),
	]);
	let mut graph = expand(&files, "sfx/parent.audiograph").expect("the graph should expand");

	let compiled = graph.select().expect("a valid graph should select");

	assert_eq!(compiled.resource_id, "audio/a.wav");
	assert_eq!(compiled.playback_rate, PlaybackRate::from_rate(2.0));
	assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
}

#[test]
fn selectors_advance_on_every_spawn_and_only_on_the_selected_path() {
	let files = bake(&[
		(
			"sfx/parent.audiograph",
			r#"{ round_robin: [{ graph: "sfx/steps.audiograph" }, { sample: "audio/c.wav" }] }"#,
		),
		(
			"sfx/steps.audiograph",
			r#"{ round_robin: [{ sample: "audio/a.wav" }, { sample: "audio/b.wav" }] }"#,
		),
	]);
	let mut graph = expand(&files, "sfx/parent.audiograph").expect("the graph should expand");

	assert_eq!(
		spawns(&mut graph, 5),
		["audio/a.wav", "audio/c.wav", "audio/b.wav", "audio/c.wav", "audio/a.wav"]
	);
}

#[test]
fn each_expansion_of_a_nested_graph_has_its_own_selectors() {
	let files = bake(&[
		(
			"sfx/parent.audiograph",
			r#"{ round_robin: [{ graph: "sfx/steps.audiograph" }, { graph: "sfx/steps.audiograph" }] }"#,
		),
		(
			"sfx/steps.audiograph",
			r#"{ round_robin: [{ sample: "audio/a.wav" }, { sample: "audio/b.wav" }] }"#,
		),
	]);
	let mut graph = expand(&files, "sfx/parent.audiograph").expect("the graph should expand");

	assert_eq!(
		spawns(&mut graph, 4),
		["audio/a.wav", "audio/a.wav", "audio/b.wav", "audio/b.wav"]
	);
}

#[test]
fn a_chain_of_nested_graphs_that_leads_back_is_rejected() {
	let files = bake(&[
		("sfx/a.audiograph", r#"{ gain: 0.5, input: { graph: "sfx/b.audiograph" } }"#),
		("sfx/b.audiograph", r#"{ loop: { graph: "sfx/a.audiograph" } }"#),
	]);

	let error = expand(&files, "sfx/a.audiograph").unwrap_err();

	assert!(
		error.contains("'sfx/a.audiograph' names 'sfx/b.audiograph' names 'sfx/a.audiograph'"),
		"{error}"
	);
}

#[test]
fn the_node_limit_applies_after_expansion() {
	// Two copies of the child, one sample, and the round robin over them.
	let parent =
		r#"{ round_robin: [{ graph: "sfx/child.audiograph" }, { graph: "sfx/child.audiograph" }, { sample: "audio/x.wav" }] }"#;
	let with_child = |samples: usize| {
		let child = round_robin_of(samples);
		let files = bake(&[("sfx/parent.audiograph", parent), ("sfx/child.audiograph", &child)]);
		expand(&files, "sfx/parent.audiograph")
	};

	assert_eq!(with_child(30).expect("64 nodes should expand").nodes.len(), 64);
	assert!(with_child(31).unwrap_err().contains("more than 64 nodes"));
}

#[test]
fn looping_a_nested_graph_that_already_loops_adds_no_node() {
	let child = format!("{{ loop: {} }}", round_robin_of(62));
	let files = bake(&[
		("sfx/parent.audiograph", r#"{ loop: { graph: "sfx/child.audiograph" } }"#),
		("sfx/child.audiograph", &child),
	]);

	assert!(expand(&files, "sfx/parent.audiograph").is_ok());
}

#[test]
fn processors_repeated_across_nested_graphs_are_rejected_when_selected() {
	let files = bake(&[
		(
			"sfx/parent.audiograph",
			r#"{ varispeed: 2, input: { graph: "sfx/child.audiograph" } }"#,
		),
		(
			"sfx/child.audiograph",
			r#"{ varispeed: 0.5, input: { sample: "audio/a.wav" } }"#,
		),
	]);
	let mut graph = expand(&files, "sfx/parent.audiograph").expect("the graph should expand");

	assert!(graph.select().unwrap_err().contains("more than one varispeed"));
}
