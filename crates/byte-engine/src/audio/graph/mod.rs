//! Inline-authored audio processing graphs.
//!
//! Build a graph with the functions in [`fns`], then publish it through
//! [`crate::gameplay::world::DefaultWorld::audio_graph_factory`]. The
//! default audio worker validates and compiles each graph before its sample
//! resources cross to the audio thread.

use std::fmt;

use smallbox::{SmallBox, smallbox, space::S4};
use smallvec::SmallVec;

use crate::core::{
	Entity,
	factory::{CreateMessage, Factory, Handle},
	listener::DefaultListener,
};

mod authoring;
mod compiler;
pub mod fns;
mod nodes;
mod optimization;
mod pitch_shift;
mod plan;
mod time;

const INLINE_AUDIO_NODE_CAPACITY: usize = 8;
const INLINE_SELECTOR_INPUT_CAPACITY: usize = 4;
pub(crate) const MAX_AUDIO_GRAPH_NODES: usize = 64;
const RANDOM_STATE_INCREMENT: u64 = 0x9E37_79B9_7F4A_7C15;
pub(crate) type AudioProcessors = SmallVec<[AudioProcessor; INLINE_AUDIO_NODE_CAPACITY]>;
pub(crate) type RuntimeAudioProcessors = SmallVec<[SmallBox<dyn RuntimeAudioProcessor + Send, S4>; INLINE_AUDIO_NODE_CAPACITY]>;
pub(super) type SelectorInputs = SmallVec<[AudioNodeId; INLINE_SELECTOR_INPUT_CAPACITY]>;
pub(super) type SelectorCommits = SmallVec<[SelectorCommit; MAX_AUDIO_GRAPH_NODES]>;
pub(super) type RuntimeCustomFunction = Box<dyn FnMut(AudioGraphTime, &mut [f32]) + Send>;

pub use authoring::{AudioGraph, AudioGraphFactory};
pub(crate) use nodes::{
	AudioNode, AudioNodeId, CustomAudioFunction, NodeProperties, RandomNode, RoundRobinNode, SelectorCommit,
};
pub(crate) use plan::{
	AudioGraphRenderPlan, AudioProcessContext, AudioProcessor, CompiledAudioGraph, PlaybackRate, PreparedAudioGraphRenderPlan,
	RuntimeAudioProcessor, SamplePlaybackMode,
};
pub use time::AudioGraphTime;

#[cfg(test)]
mod tests {
	use super::{
		AudioGraph, AudioGraphFactory, AudioGraphTime, AudioNode, AudioNodeId, AudioProcessContext, AudioProcessor,
		CompiledAudioGraph, MAX_AUDIO_GRAPH_NODES, PlaybackRate, RandomNode, RoundRobinNode, SamplePlaybackMode,
		SelectorInputs,
		fns::{custom, gain, r#loop, pitch_shift, random, round_robin, sample, varispeed},
		pitch_shift::PITCH_SHIFT_LATENCY,
	};
	use crate::core::listener::Listener;

	fn compile_submission(graph: &mut AudioGraph) -> CompiledAudioGraph {
		let (compiled, selector_commits) = graph.compile_selection().expect("valid graph");
		graph.commit_selectors(&selector_commits);
		compiled
	}

	/// Fixes a random node's authored state so behavior tests are reproducible.
	fn set_random_state(graph: &mut AudioGraph, state: u64, last_index: Option<usize>) {
		let node = graph
			.nodes
			.iter_mut()
			.find_map(|node| match &mut **node {
				AudioNode::Random(node) => Some(node),
				_ => None,
			})
			.expect("graph must contain a random node");
		node.state = state;
		node.last_index = last_index;
	}

	/// Verifies that factory optimization reconnects an identity node's consumer.
	fn assert_factory_eliminates_identity_node(identity: AudioNode) {
		let mut graph = sample("audio/a.wav");
		graph.push(identity);
		let identity = graph.output;
		graph.push(AudioNode::Gain {
			input: identity,
			gain: 0.5,
		});

		assert_eq!(graph.nodes.len(), 3);
		let factory = AudioGraphFactory::new();

		factory.create(&mut graph);

		assert_eq!(graph.nodes.len(), 2);
		assert_eq!(graph.output, AudioNodeId(1));
		let AudioNode::Gain { input, .. } = &*graph.nodes[graph.output.0] else {
			panic!("optimized output must remain a gain node");
		};

		assert_eq!(*input, AudioNodeId(0));
		assert!(!graph.nodes.iter().any(|node| {
			matches!(&**node, AudioNode::RoundRobin(node) if node.inputs.len() == 1)
				|| matches!(&**node, AudioNode::Random(node) if node.inputs.len() == 1)
				|| matches!(&**node, AudioNode::Gain { gain: 1.0, .. })
				|| matches!(&**node, AudioNode::Varispeed { rate: 1.0, .. })
				|| matches!(&**node, AudioNode::PitchShift { ratio: 1.0, .. })
		}));
	}

	/// Builds the largest graph accepted by the authoring API.
	fn maximum_node_chain() -> AudioGraph {
		let mut input = sample("audio/a.wav");
		for _ in 1..MAX_AUDIO_GRAPH_NODES {
			input = gain(input, 0.5);
		}
		input
	}

	/// Builds the largest graph whose output is already looping.
	fn maximum_looping_chain() -> AudioGraph {
		let mut input = sample("audio/a.wav");
		for _ in 1..MAX_AUDIO_GRAPH_NODES - 1 {
			input = gain(input, 0.5);
		}
		r#loop(input)
	}

	#[test]
	fn factory_submissions_cycle_through_complete_input_chains() {
		let mut graph = gain(
			round_robin([
				gain(r#loop(pitch_shift(sample("audio/a.wav"), 0.5)), 0.5),
				varispeed(sample("audio/b.wav"), 1.5),
				pitch_shift(sample("audio/c.wav"), 2.0),
			]),
			0.25,
		);
		let factory = AudioGraphFactory::new();
		let mut listener = factory.listener();

		let first_handle = factory.create(&mut graph);
		let _second_handle = factory.create(&mut graph);
		let _third_handle = factory.create(&mut graph);
		let _fourth_handle = factory.create(&mut graph);
		factory.derive(first_handle, &mut graph);
		let first = listener.read().expect("first selection");
		let second = listener.read().expect("second selection");
		let third = listener.read().expect("third selection");
		let fourth = listener.read().expect("wrapped selection");
		let replacement = listener.read().expect("derived selection");

		assert_eq!(first.data().resource_id, "audio/a.wav");
		assert_eq!(first.data().playback_mode, SamplePlaybackMode::Loop);
		assert_eq!(
			&first.data().processors[..],
			&[AudioProcessor::PitchShift(0.5), AudioProcessor::Gain(0.125)]
		);
		assert_eq!(second.data().resource_id, "audio/b.wav");
		assert_eq!(second.data().playback_rate.numerator, 3);
		assert_eq!(second.data().playback_rate.denominator, 2);
		assert_eq!(&second.data().processors[..], &[AudioProcessor::Gain(0.25)]);
		assert_eq!(third.data().resource_id, "audio/c.wav");
		assert_eq!(
			&third.data().processors[..],
			&[AudioProcessor::PitchShift(2.0), AudioProcessor::Gain(0.25)]
		);
		assert_eq!(fourth.data().resource_id, "audio/a.wav");
		assert_eq!(replacement.handle(), first_handle);
		assert_eq!(replacement.data().resource_id, "audio/b.wav");
	}

	#[test]
	fn nested_round_robins_advance_only_on_the_selected_path() {
		let mut graph = round_robin([
			round_robin([sample("audio/a.wav"), sample("audio/b.wav")]),
			sample("audio/c.wav"),
		]);

		let sequence = (0..6).map(|_| compile_submission(&mut graph).resource_id).collect::<Vec<_>>();

		assert_eq!(
			sequence,
			[
				"audio/a.wav",
				"audio/c.wav",
				"audio/b.wav",
				"audio/c.wav",
				"audio/a.wav",
				"audio/c.wav"
			]
		);
	}

	#[test]
	fn round_robin_accepts_the_node_limit_and_rejects_larger_graphs() {
		let inputs = (0..63).map(|index| sample(format!("audio/{index}.wav")));
		let maximum = round_robin(inputs);

		assert_eq!(maximum.nodes.len(), 64);
		assert_eq!(maximum.compile().expect("expected test value").resource_id, "audio/0.wav");

		let too_many = std::panic::catch_unwind(|| round_robin((0..64).map(|index| sample(format!("audio/{index}.wav")))));

		assert!(too_many.is_err());
	}

	#[test]
	fn random_selects_all_inputs_without_consecutive_repeats() {
		let mut graph = random([sample("audio/a.wav"), sample("audio/b.wav"), sample("audio/c.wav")]);
		set_random_state(&mut graph, 0, None);
		let mut previous = None;
		let mut seen = [false; 3];

		for _ in 0..96 {
			let resource_id = compile_submission(&mut graph).resource_id;

			assert_ne!(previous.as_deref(), Some(resource_id.as_str()));
			match resource_id.as_str() {
				"audio/a.wav" => seen[0] = true,
				"audio/b.wav" => seen[1] = true,
				"audio/c.wav" => seen[2] = true,
				_ => panic!("random selector chose an unknown input"),
			}
			previous = Some(resource_id);
		}

		assert!(seen.into_iter().all(|was_selected| was_selected));
	}

	#[test]
	fn factory_optimization_reconnects_consumers_of_identity_nodes() {
		let mut random_inputs = SelectorInputs::new();
		random_inputs.push(AudioNodeId(0));
		assert_factory_eliminates_identity_node(AudioNode::Random(Box::new(RandomNode {
			inputs: random_inputs,
			state: 0,
			last_index: None,
		})));

		let mut round_robin_inputs = SelectorInputs::new();
		round_robin_inputs.push(AudioNodeId(0));
		assert_factory_eliminates_identity_node(AudioNode::RoundRobin(Box::new(RoundRobinNode {
			inputs: round_robin_inputs,
			next_index: 0,
		})));
		assert_factory_eliminates_identity_node(AudioNode::Gain {
			input: AudioNodeId(0),
			gain: 1.0,
		});
		assert_factory_eliminates_identity_node(AudioNode::Varispeed {
			input: AudioNodeId(0),
			rate: 1.0,
		});
		assert_factory_eliminates_identity_node(AudioNode::PitchShift {
			input: AudioNodeId(0),
			ratio: 1.0,
		});
	}

	#[test]
	fn eliminated_identity_nodes_do_not_consume_the_node_limit() {
		let optimized_random = random([maximum_node_chain()]);

		assert_eq!(optimized_random.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(
			!optimized_random
				.nodes
				.iter()
				.any(|node| matches!(&**node, AudioNode::Random(_)))
		);

		let optimized_round_robin = round_robin([maximum_node_chain()]);

		assert_eq!(optimized_round_robin.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(
			!optimized_round_robin
				.nodes
				.iter()
				.any(|node| matches!(&**node, AudioNode::RoundRobin(_)))
		);

		let optimized_varispeed = varispeed(maximum_node_chain(), 1.0);

		assert_eq!(optimized_varispeed.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(
			!optimized_varispeed
				.nodes
				.iter()
				.any(|node| matches!(&**node, AudioNode::Varispeed { .. }))
		);

		let optimized_pitch_shift = pitch_shift(maximum_node_chain(), 1.0);

		assert_eq!(optimized_pitch_shift.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(
			!optimized_pitch_shift
				.nodes
				.iter()
				.any(|node| matches!(&**node, AudioNode::PitchShift { .. }))
		);

		let optimized_gain = gain(maximum_node_chain(), 1.0);

		assert_eq!(optimized_gain.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(
			!optimized_gain
				.nodes
				.iter()
				.any(|node| matches!(&**node, AudioNode::Gain { gain: 1.0, .. }))
		);

		let optimized_loop = r#loop(maximum_looping_chain());

		assert_eq!(optimized_loop.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert_eq!(
			optimized_loop
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::Loop { .. }))
				.count(),
			1
		);
	}

	#[test]
	fn invalid_unselected_branch_does_not_advance_the_cursor() {
		let mut graph = round_robin([sample("audio/a.wav"), sample("")]);

		assert!(graph.compile().unwrap_err().contains("resource ID is empty"));

		let AudioNode::Sample { resource_id } = &mut *graph.nodes[1] else {
			panic!("second input must remain a sample node");
		};
		*resource_id = "audio/b.wav".to_string();

		assert_eq!(compile_submission(&mut graph).resource_id, "audio/a.wav");
		assert_eq!(compile_submission(&mut graph).resource_id, "audio/b.wav");
	}

	#[test]
	fn compiler_rejects_cycles_and_disconnected_selector_inputs() {
		let mut cyclic = gain(sample("audio/a.wav"), 0.5);
		let AudioNode::Gain { input, .. } = &mut *cyclic.nodes[cyclic.output.0] else {
			panic!("graph output must be a gain node");
		};
		*input = cyclic.output;

		assert!(cyclic.compile().unwrap_err().contains("cycle"));

		let mut disconnected = round_robin([sample("audio/a.wav"), sample("audio/b.wav")]);
		let AudioNode::RoundRobin(node) = &mut *disconnected.nodes[disconnected.output.0] else {
			panic!("graph output must be a round-robin node");
		};
		node.inputs.pop();

		assert!(disconnected.compile().unwrap_err().contains("not connected"));

		let mut disconnected = random([sample("audio/a.wav"), sample("audio/b.wav")]);
		let AudioNode::Random(node) = &mut *disconnected.nodes[disconnected.output.0] else {
			panic!("graph output must be a random node");
		};
		node.inputs.pop();

		assert!(disconnected.compile().unwrap_err().contains("not connected"));

		let mut outside = gain(sample("audio/a.wav"), 0.5);
		let AudioNode::Gain { input, .. } = &mut *outside.nodes[outside.output.0] else {
			panic!("graph output must be a gain node");
		};
		*input = AudioNodeId(usize::MAX);

		assert!(outside.compile().unwrap_err().contains("outside this graph"));
	}

	#[test]
	fn consecutive_gains_compile_to_one_processor() {
		let compiled = gain(gain(sample("audio/music.ogg"), 0.5), 0.25)
			.compile()
			.expect("valid graph");

		assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.125)]);

		let separated = gain(pitch_shift(gain(sample("audio/music.ogg"), 0.5), 2.0), 0.25)
			.compile()
			.expect("valid graph");

		assert_eq!(
			&separated.processors[..],
			&[
				AudioProcessor::Gain(0.5),
				AudioProcessor::PitchShift(2.0),
				AudioProcessor::Gain(0.25),
			]
		);
	}

	#[test]
	fn each_custom_function_playback_gets_independent_closure_state() {
		let graph = custom(sample("audio/music.ogg"), {
			let mut invocation = 0.0;
			move |_time, samples: &mut [f32]| {
				invocation += 1.0;
				samples.fill(invocation);
			}
		});

		assert_eq!(graph, graph.clone());

		let (_, first_plan) = graph.compile().expect("valid graph").into_parts();
		let (_, second_plan) = graph.compile().expect("valid graph").into_parts();
		let mut context = AudioProcessContext::new();
		let mut first = first_plan.prepare();
		let mut second = second_plan.prepare();
		let mut first_samples = [0.0; 2];
		let mut second_samples = [0.0; 2];

		first.processors[0].process(&mut context, AudioGraphTime::new(0, 48_000), &mut first_samples);
		first.processors[0].process(&mut context, AudioGraphTime::new(2, 48_000), &mut first_samples);
		second.processors[0].process(&mut context, AudioGraphTime::new(0, 48_000), &mut second_samples);

		assert_eq!(first_samples, [2.0; 2]);
		assert_eq!(second_samples, [1.0; 2]);
	}

	#[test]
	fn custom_function_receives_sample_accurate_block_time() {
		let graph = custom(sample("audio/music.ogg"), |time, samples: &mut [f32]| {
			for (offset, sample) in samples.iter_mut().enumerate() {
				*sample = time.seconds_at(offset) as f32;
			}
		});
		let (_, render_plan) = graph.compile().expect("valid graph").into_parts();
		let mut context = AudioProcessContext::new();
		let mut prepared = render_plan.prepare();
		let mut samples = [0.0; 3];

		prepared.processors[0].process(&mut context, AudioGraphTime::new(2, 4), &mut samples);

		assert_eq!(samples, [0.5, 0.75, 1.0]);
	}

	#[test]
	fn zero_gain_compiles_to_a_muted_timeline_without_processors() {
		let compiled = pitch_shift(gain(varispeed(sample("audio/music.ogg"), 1.5), 0.0), 2.0)
			.compile()
			.expect("valid graph");

		assert!(compiled.muted);
		assert!(compiled.processors.is_empty());
		assert_eq!(compiled.playback_rate, PlaybackRate::from_rate(1.5));
		assert_eq!(compiled.muted_drain_latency, PITCH_SHIFT_LATENCY);

		let compiled = gain(pitch_shift(sample("audio/music.ogg"), 2.0), 0.0)
			.compile()
			.expect("valid graph");

		assert!(compiled.muted);
		assert!(compiled.processors.is_empty());
		assert_eq!(compiled.muted_drain_latency, PITCH_SHIFT_LATENCY);
	}
}
