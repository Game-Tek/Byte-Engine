//! Inline-authored audio processing graphs.
//!
//! Build a graph with the functions in [`fns`], then publish it through
//! [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`]. The
//! default audio worker validates and compiles each graph before its sample
//! resources cross to the audio thread.

use std::{
	fmt,
	sync::atomic::{AtomicU64, Ordering},
	sync::Arc,
	time::{SystemTime, UNIX_EPOCH},
};

use smallbox::{smallbox, space::S4, SmallBox};
use smallvec::SmallVec;

use crate::core::{
	factory::{CreateMessage, Factory, Handle},
	listener::DefaultListener,
	Entity,
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
static NEXT_RANDOM_SEED: AtomicU64 = AtomicU64::new(0x243F_6A88_85A3_08D3);
pub(crate) type AudioProcessors = SmallVec<[AudioProcessor; INLINE_AUDIO_NODE_CAPACITY]>;
pub(crate) type RuntimeAudioProcessors = SmallVec<[SmallBox<dyn RuntimeAudioProcessor + Send, S4>; INLINE_AUDIO_NODE_CAPACITY]>;
pub(super) type SelectorInputs = SmallVec<[AudioNodeId; INLINE_SELECTOR_INPUT_CAPACITY]>;
pub(super) type SelectorCommits = SmallVec<[SelectorCommit; MAX_AUDIO_GRAPH_NODES]>;
pub(super) type RuntimeCustomFunction = Box<dyn FnMut(AudioGraphTime, &mut [f32]) + Send>;
pub(super) type CustomFunctionFactory = Arc<dyn Fn() -> RuntimeCustomFunction + Send + Sync>;

pub use authoring::{AudioGraph, AudioGraphFactory};
pub(crate) use nodes::{
	AudioNode, AudioNodeId, CustomAudioFunction, NodeProperties, RandomNode, RoundRobinNode, SelectorCommit,
};
pub(crate) use plan::{
	AudioGraphRenderPlan, AudioProcessor, CompiledAudioGraph, PlaybackRate, PreparedAudioGraphRenderPlan,
	RuntimeAudioProcessor, SamplePlaybackMode,
};
pub use time::AudioGraphTime;

#[cfg(test)]
mod tests {
	use super::{
		fns::{custom, gain, pitch_shift, r#loop, random, round_robin, sample, varispeed},
		pitch_shift::PITCH_SHIFT_LATENCY,
		AudioGraph, AudioGraphFactory, AudioGraphTime, AudioNode, AudioNodeId, AudioProcessor, CompiledAudioGraph,
		PlaybackRate, RandomNode, RoundRobinNode, SamplePlaybackMode, SelectorInputs, MAX_AUDIO_GRAPH_NODES,
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
		let mut factory = AudioGraphFactory::new();

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
	fn nested_functions_compile_to_a_looping_sample_and_gain_processor() {
		let graph = gain(r#loop(sample("audio/music.ogg")), 0.5);

		assert!(!graph.nodes.spilled());
		assert!(graph.nodes.iter().all(|node| !node.is_heap()));

		let compiled = graph.compile().expect("valid graph");

		assert_eq!(compiled.resource_id, "audio/music.ogg");
		assert_eq!(compiled.playback_mode, SamplePlaybackMode::Loop);
		assert!(!compiled.processors.spilled());
		assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
	}

	#[test]
	fn unity_gain_and_duplicate_loop_are_eliminated() {
		let gain_graph = gain(gain(sample("audio/music.ogg"), 0.5), 1.0);

		assert_eq!(gain_graph.nodes.len(), 2);
		assert_eq!(
			gain_graph
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::Gain { .. }))
				.count(),
			1
		);

		let loop_graph = r#loop(r#loop(sample("audio/music.ogg")));

		assert_eq!(loop_graph.nodes.len(), 2);
		assert_eq!(
			loop_graph
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::Loop { .. }))
				.count(),
			1
		);
		assert_eq!(
			loop_graph.compile().expect("valid graph").playback_mode,
			SamplePlaybackMode::Loop
		);
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
		let mut factory = AudioGraphFactory::new();
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
		assert_eq!(replacement.handle(), &first_handle);
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
	fn cloned_graphs_keep_independent_round_robin_cursors() {
		let mut original = round_robin([sample("audio/a.wav"), sample("audio/b.wav")]);

		assert_eq!(compile_submission(&mut original).resource_id, "audio/a.wav");
		let mut cloned = original.clone();

		assert_eq!(compile_submission(&mut original).resource_id, "audio/b.wav");
		assert_eq!(compile_submission(&mut original).resource_id, "audio/a.wav");
		assert_eq!(compile_submission(&mut cloned).resource_id, "audio/b.wav");
		assert_eq!(compile_submission(&mut cloned).resource_id, "audio/a.wav");
	}

	#[test]
	fn one_input_round_robin_is_eliminated() {
		let mut graph = round_robin([gain(sample("audio/a.wav"), 0.5)]);

		assert_eq!(graph.nodes.len(), 2);
		assert!(!graph.nodes.iter().any(|node| matches!(&**node, AudioNode::RoundRobin(_))));

		for _ in 0..3 {
			let compiled = compile_submission(&mut graph);

			assert_eq!(compiled.resource_id, "audio/a.wav");
			assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
		}
	}

	#[test]
	#[should_panic(expected = "No input chains were provided")]
	fn round_robin_rejects_empty_inputs() {
		let _ = round_robin([]);
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
	fn random_selects_complete_processing_chains() {
		let mut graph = gain(
			random([
				gain(r#loop(sample("audio/a.wav")), 0.5),
				varispeed(sample("audio/b.wav"), 1.5),
			]),
			0.25,
		);
		set_random_state(&mut graph, 0, None);

		let first = compile_submission(&mut graph);
		let second = compile_submission(&mut graph);

		assert_ne!(first.resource_id, second.resource_id);
		for compiled in [first, second] {
			match compiled.resource_id.as_str() {
				"audio/a.wav" => {

					assert_eq!(compiled.playback_mode, SamplePlaybackMode::Loop);
					assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.125)]);
				}
				"audio/b.wav" => {

					assert_eq!(compiled.playback_rate, PlaybackRate::from_rate(1.5));
					assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.25)]);
				}
				_ => panic!("random selector chose an unknown input"),
			}
		}
	}

	#[test]
	fn random_factory_submissions_commit_the_published_choice() {
		let mut graph = random([sample("audio/a.wav"), sample("audio/b.wav")]);
		set_random_state(&mut graph, 0, None);
		let expected_first = graph.compile().expect("valid graph").resource_id;

		assert_eq!(graph.compile().expect("valid graph").resource_id, expected_first);
		let mut factory = AudioGraphFactory::new();
		let mut listener = factory.listener();

		factory.create(&mut graph);
		factory.create(&mut graph);
		let first = listener.read().expect("first random selection");
		let second = listener.read().expect("second random selection");

		assert_eq!(first.data().resource_id, expected_first);
		assert_ne!(second.data().resource_id, first.data().resource_id);
	}

	#[test]
	fn nested_random_nodes_advance_only_on_the_selected_path() {
		let mut graph = round_robin([random([sample("audio/a.wav"), sample("audio/b.wav")]), sample("audio/c.wav")]);
		set_random_state(&mut graph, 0, None);

		let sequence = (0..6).map(|_| compile_submission(&mut graph).resource_id).collect::<Vec<_>>();

		assert_eq!(sequence[1], "audio/c.wav");
		assert_eq!(sequence[3], "audio/c.wav");
		assert_eq!(sequence[5], "audio/c.wav");
		assert_ne!(sequence[0], sequence[2]);
		assert_eq!(sequence[0], sequence[4]);
	}

	#[test]
	fn cloned_graphs_keep_independent_random_state() {
		let mut original = random([sample("audio/a.wav"), sample("audio/b.wav"), sample("audio/c.wav")]);
		set_random_state(&mut original, 0, None);
		compile_submission(&mut original);
		let mut cloned = original.clone();

		let cloned_next = cloned.compile().expect("valid clone").resource_id;

		assert_eq!(compile_submission(&mut original).resource_id, cloned_next);
		compile_submission(&mut original);

		assert_eq!(compile_submission(&mut cloned).resource_id, cloned_next);
	}

	#[test]
	fn one_input_random_is_eliminated() {
		let mut graph = random([gain(sample("audio/a.wav"), 0.5)]);

		assert_eq!(graph.nodes.len(), 2);
		assert!(!graph.nodes.iter().any(|node| matches!(&**node, AudioNode::Random(_))));

		for _ in 0..3 {
			let compiled = compile_submission(&mut graph);

			assert_eq!(compiled.resource_id, "audio/a.wav");
			assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
		}
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
	fn factory_optimization_reconnects_consumers_of_duplicate_loops() {
		let mut graph = sample("audio/a.wav");
		graph.push(AudioNode::Loop { input: AudioNodeId(0) });
		graph.push(AudioNode::Loop { input: AudioNodeId(1) });
		graph.push(AudioNode::Gain {
			input: AudioNodeId(2),
			gain: 0.5,
		});

		assert_eq!(graph.nodes.len(), 4);
		let mut factory = AudioGraphFactory::new();

		factory.create(&mut graph);

		assert_eq!(graph.nodes.len(), 3);
		assert_eq!(graph.output, AudioNodeId(2));
		let AudioNode::Gain { input, .. } = &*graph.nodes[graph.output.0] else {
			panic!("optimized output must remain a gain node");
		};

		assert_eq!(*input, AudioNodeId(1));
		assert_eq!(
			graph
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::Loop { .. }))
				.count(),
			1
		);
	}

	#[test]
	fn eliminated_identity_nodes_do_not_consume_the_node_limit() {
		let optimized_random = random([maximum_node_chain()]);

		assert_eq!(optimized_random.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(!optimized_random
			.nodes
			.iter()
			.any(|node| matches!(&**node, AudioNode::Random(_))));

		let optimized_round_robin = round_robin([maximum_node_chain()]);

		assert_eq!(optimized_round_robin.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(!optimized_round_robin
			.nodes
			.iter()
			.any(|node| matches!(&**node, AudioNode::RoundRobin(_))));

		let optimized_varispeed = varispeed(maximum_node_chain(), 1.0);

		assert_eq!(optimized_varispeed.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(!optimized_varispeed
			.nodes
			.iter()
			.any(|node| matches!(&**node, AudioNode::Varispeed { .. })));

		let optimized_pitch_shift = pitch_shift(maximum_node_chain(), 1.0);

		assert_eq!(optimized_pitch_shift.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(!optimized_pitch_shift
			.nodes
			.iter()
			.any(|node| matches!(&**node, AudioNode::PitchShift { .. })));

		let optimized_gain = gain(maximum_node_chain(), 1.0);

		assert_eq!(optimized_gain.nodes.len(), MAX_AUDIO_GRAPH_NODES);
		assert!(!optimized_gain
			.nodes
			.iter()
			.any(|node| matches!(&**node, AudioNode::Gain { gain: 1.0, .. })));

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
	#[should_panic(expected = "No input chains were provided")]
	fn random_rejects_empty_inputs() {
		let _ = random([]);
	}

	#[test]
	fn random_accepts_the_node_limit_and_rejects_larger_graphs() {
		let inputs = (0..63).map(|index| sample(format!("audio/{index}.wav")));
		let maximum = random(inputs);

		assert_eq!(maximum.nodes.len(), 64);
		maximum.compile().expect("maximum-size random graph should compile");

		let too_many = std::panic::catch_unwind(|| random((0..64).map(|index| sample(format!("audio/{index}.wav")))));

		assert!(too_many.is_err());
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
	fn invalid_unselected_branch_does_not_advance_random_state() {
		let mut expected = random([sample("audio/a.wav"), sample("audio/b.wav")]);
		set_random_state(&mut expected, 0, None);
		let expected_first = compile_submission(&mut expected).resource_id;

		let mut graph = random([sample("audio/a.wav"), sample("")]);
		set_random_state(&mut graph, 0, None);

		assert!(graph.compile().unwrap_err().contains("resource ID is empty"));
		let AudioNode::Sample { resource_id } = &mut *graph.nodes[1] else {
			panic!("second input must remain a sample node");
		};
		*resource_id = "audio/b.wav".to_string();

		assert_eq!(compile_submission(&mut graph).resource_id, expected_first);
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
	fn pitch_shift_compiles_in_processor_order_and_unity_is_bypassed() {
		let compiled = gain(pitch_shift(sample("audio/music.ogg"), 2.0), 0.5)
			.compile()
			.expect("valid graph");

		assert_eq!(
			&compiled.processors[..],
			&[AudioProcessor::PitchShift(2.0), AudioProcessor::Gain(0.5)]
		);

		let unity_graph = pitch_shift(pitch_shift(sample("audio/music.ogg"), 2.0), 1.0);

		assert_eq!(unity_graph.nodes.len(), 2);
		assert_eq!(
			unity_graph
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::PitchShift { .. }))
				.count(),
			1
		);
		let unity = unity_graph.compile().expect("valid graph");

		assert_eq!(&unity.processors[..], &[AudioProcessor::PitchShift(2.0)]);
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
	fn custom_function_processes_blocks_in_compiled_order() {
		let graph = gain(
			custom(sample("audio/music.ogg"), |_time, samples: &mut [f32]| {
				for sample in samples {
					*sample += 1.0;
				}
			}),
			0.5,
		);
		let (_, render_plan) = graph.compile().expect("valid graph").into_parts();
		let mut prepared = render_plan.prepare();
		let mut samples = [1.0, 2.0, 3.0];

		assert_eq!(prepared.processors.len(), 1);
		assert_eq!(prepared.output_gain, 0.5);
		prepared.processors[0].process(AudioGraphTime::new(0, 48_000), &mut samples);

		assert_eq!(samples, [2.0, 3.0, 4.0]);
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
		let mut first = first_plan.prepare();
		let mut second = second_plan.prepare();
		let mut first_samples = [0.0; 2];
		let mut second_samples = [0.0; 2];

		first.processors[0].process(AudioGraphTime::new(0, 48_000), &mut first_samples);
		first.processors[0].process(AudioGraphTime::new(2, 48_000), &mut first_samples);
		second.processors[0].process(AudioGraphTime::new(0, 48_000), &mut second_samples);

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
		let mut prepared = render_plan.prepare();
		let mut samples = [0.0; 3];

		prepared.processors[0].process(AudioGraphTime::new(2, 4), &mut samples);

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

	#[test]
	fn varispeed_compiles_as_an_exact_source_playback_rate() {
		let compiled = gain(varispeed(sample("audio/music.ogg"), 1.5), 0.5)
			.compile()
			.expect("valid graph");

		assert_eq!(
			compiled.playback_rate,
			PlaybackRate {
				numerator: 3,
				denominator: 2,
			}
		);
		assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);

		let unity_graph = varispeed(varispeed(sample("audio/music.ogg"), 1.5), 1.0);

		assert_eq!(unity_graph.nodes.len(), 2);
		assert_eq!(
			unity_graph
				.nodes
				.iter()
				.filter(|node| matches!(&***node, AudioNode::Varispeed { .. }))
				.count(),
			1
		);
		let unity = unity_graph.compile().expect("valid graph");

		assert_eq!(unity.playback_rate, PlaybackRate::from_rate(1.5));

		let unity = varispeed(sample("audio/music.ogg"), 1.0).compile().expect("valid graph");

		assert_eq!(unity.playback_rate, PlaybackRate::UNITY);
	}

	#[test]
	fn prepared_processors_keep_small_nodes_inline_and_large_state_node_local() {
		let compiled = gain(pitch_shift(gain(sample("audio/music.ogg"), 0.5), 2.0), 0.25)
			.compile()
			.expect("valid graph");
		let (_, render_plan) = compiled.into_parts();
		let prepared = render_plan.prepare();

		assert!(!prepared.processors[0].is_heap());
		assert!(prepared.processors[1].is_heap());
		assert!(!prepared.processors.spilled());
		assert_eq!(prepared.drain_latency, PITCH_SHIFT_LATENCY);
		assert_eq!(prepared.output_gain, 0.25);
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph pitch ratio")]
	fn pitch_shift_rejects_out_of_range_ratios_when_authored() {
		let _ = pitch_shift(sample("audio/music.ogg"), 2.1);
	}

	#[test]
	#[should_panic(expected = "at most one pitch-shift node")]
	fn graph_rejects_a_second_pitch_shift_when_authored() {
		let _ = pitch_shift(pitch_shift(sample("audio/music.ogg"), 0.5), 2.0);
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph varispeed rate")]
	fn varispeed_rejects_out_of_range_rates_when_authored() {
		let _ = varispeed(sample("audio/music.ogg"), 4.1);
	}

	#[test]
	#[should_panic(expected = "A second varispeed node")]
	fn graph_rejects_a_second_varispeed_when_authored() {
		let _ = varispeed(varispeed(sample("audio/music.ogg"), 0.5), 2.0);
	}

	#[test]
	fn direct_sample_compiles_as_an_unprocessed_one_shot() {
		let compiled = sample("audio/impact.wav").compile().expect("valid graph");

		assert_eq!(compiled.resource_id, "audio/impact.wav");
		assert_eq!(compiled.playback_mode, SamplePlaybackMode::Once);
		assert_eq!(compiled.playback_rate, PlaybackRate::UNITY);
		assert!(compiled.processors.is_empty());
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph gain")]
	fn gain_rejects_non_finite_values_when_the_graph_is_authored() {
		let _ = gain(sample("audio/music.ogg"), f32::NAN);
	}
}
