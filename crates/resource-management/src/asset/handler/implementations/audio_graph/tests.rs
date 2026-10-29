use super::{AudioGraphAssetHandler, bake_audio_graph};
use crate::{
	asset::{self, manager::AssetManager},
	r#async,
	resource::{self, resource_manager::ResourceManager},
	resources::audio_graph::{AudioGraph, AudioGraphNode, MAX_NODES},
};

fn bake(source: &str) -> Result<AudioGraph, String> {
	bake_audio_graph("sfx/steps.audiograph", source.as_bytes())
}

fn sample(id: &str) -> AudioGraphNode {
	AudioGraphNode::Sample {
		resource_id: id.to_string(),
	}
}

#[test]
fn nodes_are_stored_inputs_first_with_the_output_last() {
	let graph = bake(
		r#"{
			"$schema": "../schemas/audiograph.schema.json",
			gain: 0.5,
			input: {
				round_robin: [
					{ sample: "audio/a.wav" },
					{ varispeed: 1.5, input: { sample: "audio/b.wav" } },
					{ loop: { pitch_shift: 0.75, input: { graph: "sfx/wind.audiograph" } } },
				],
			},
		}"#,
	)
	.expect("a valid graph should bake");

	assert_eq!(
		graph.nodes,
		[
			sample("audio/a.wav"),
			sample("audio/b.wav"),
			AudioGraphNode::Varispeed { input: 1, rate: 1.5 },
			AudioGraphNode::Graph {
				resource_id: "sfx/wind.audiograph".to_string()
			},
			AudioGraphNode::PitchShift { input: 3, ratio: 0.75 },
			AudioGraphNode::Loop { input: 4 },
			AudioGraphNode::RoundRobin { inputs: vec![0, 2, 5] },
			AudioGraphNode::Gain { input: 6, gain: 0.5 },
		]
	);
}

#[test]
fn nodes_that_change_nothing_are_removed() {
	let graph = bake(
		r#"{
			gain: 1,
			input: {
				random: [{
					varispeed: 1,
					input: { pitch_shift: 1, input: { loop: { loop: { sample: "audio/a.wav" } } } },
				}],
			},
		}"#,
	)
	.expect("a valid graph should bake");

	assert_eq!(graph.nodes, [sample("audio/a.wav"), AudioGraphNode::Loop { input: 0 }]);
}

#[test]
fn processors_on_separate_paths_are_accepted() {
	let graph = bake(
		r#"{
			random: [
				{ varispeed: 2, input: { sample: "audio/a.wav" } },
				{ varispeed: 0.5, input: { sample: "audio/b.wav" } },
			],
		}"#,
	);

	assert!(graph.is_ok(), "{graph:?}");
}

#[test]
fn the_node_limit_is_accepted_and_one_more_is_rejected() {
	let graph = |count: usize| {
		let entries = (0..count)
			.map(|index| format!("{{ sample: \"audio/{index}.wav\" }}"))
			.collect::<Vec<_>>();
		bake(&format!("{{ round_robin: [{}] }}", entries.join(",")))
	};

	assert_eq!(graph(MAX_NODES - 1).expect("the limit should bake").nodes.len(), MAX_NODES);
	assert!(graph(MAX_NODES).unwrap_err().contains("more than 64 nodes"));
}

#[test]
fn files_that_break_a_rule_are_rejected_with_the_cause() {
	let cases = [
		(r#"{ sample: "audio/a.wav", gain: 0.5 }"#, "2 function fields"),
		(r#"{ input: { sample: "audio/a.wav" } }"#, "0 function fields"),
		(r#"{ gain: 0.5 }"#, "without an `input`"),
		(
			r#"{ loop: { sample: "audio/a.wav" }, input: { sample: "audio/b.wav" } }"#,
			"has an `input`",
		),
		(r#"{ volume: 0.5 }"#, "unknown field"),
		(r#"{ sample: "" }"#, "empty resource ID"),
		(r#"{ round_robin: [] }"#, "empty array"),
		(r#"{ gain: -1, input: { sample: "audio/a.wav" } }"#, "finite factor"),
		(r#"{ varispeed: 8, input: { sample: "audio/a.wav" } }"#, "from 0.25 to 4.0"),
		(
			r#"{ pitch_shift: 0.25, input: { sample: "audio/a.wav" } }"#,
			"from 0.5 to 2.0",
		),
		(
			r#"{ varispeed: 2, input: { random: [{ varispeed: 0.5, input: { sample: "audio/a.wav" } }, { sample: "audio/b.wav" }] } }"#,
			"second `varispeed`",
		),
		(
			r#"{ pitch_shift: 2, input: { pitch_shift: 0.5, input: { sample: "audio/a.wav" } } }"#,
			"second `pitch_shift`",
		),
		(r#"{ loop: { "$schema": "x", sample: "audio/a.wav" } }"#, "`$schema`"),
		(r#"{ graph: "sfx/steps.audiograph" }"#, "names this file"),
	];

	for (source, cause) in cases {
		let error = bake(source).expect_err(source);

		assert!(error.contains(cause), "{source}: {error}");
	}
}

#[r#async::test]
async fn baking_stores_the_graph_under_its_file_id() {
	let assets = asset::storage_backend::tests::TestStorageBackend::new();
	let resources = resource::storage_backend::MemoryStorageBackend::new();
	assets.add_file("sfx/steps.audiograph", br#"{ loop: { sample: "audio/a.wav" } }"#);
	let mut asset_manager = AssetManager::new(assets, resources.clone());
	asset_manager.add_asset_handler(AudioGraphAssetHandler);

	asset_manager
		.bake("sfx/steps.audiograph")
		.await
		.expect("the file should bake");
	let graph = ResourceManager::new(resources)
		.request::<AudioGraph>("sfx/steps.audiograph")
		.await
		.expect("the baked graph should be requested")
		.into_resource();

	assert_eq!(graph.nodes, [sample("audio/a.wav"), AudioGraphNode::Loop { input: 0 }]);
}
