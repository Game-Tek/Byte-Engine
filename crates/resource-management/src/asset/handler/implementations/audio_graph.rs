//! Audio graph asset baking.
//!
//! Baking parses an `.audiograph` file's JSON5 node tree, checks every rule one file can break, removes the nodes
//! that change nothing, and stores the rest as a flat node list. Nested graphs stay as references: the audio worker
//! expands them when it loads the graph, so each file bakes on its own.

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	resources::audio_graph::{AudioGraph, AudioGraphNode, MAX_NODES},
};

/// The `AudioGraphAssetHandler` struct exists to bake `.audiograph` files into graphs the audio worker spawns
/// without parsing text.
pub struct AudioGraphAssetHandler;

impl AssetHandler for AudioGraphAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "audiograph"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, format) = context.resolve(id).await?;

		if format != "audiograph" {
			return Err(LoadErrors::UnsupportedType);
		}

		let graph = bake_audio_graph(id.as_ref(), &source).map_err(|cause| {
			log::error!("Audio graph '{}' could not be baked. {cause}", id.as_ref());
			LoadErrors::FailedToProcess
		})?;

		context.store_primary(ProcessedAsset::new(id, graph), &[]).await
	}
}

/// Parses an `.audiograph` source with the ID `id`, checks it, and lays it out as a baked [`AudioGraph`].
///
/// Returns the cause and fix of the first problem found, ready to follow a statement of what failed.
pub fn bake_audio_graph(id: &str, source: &[u8]) -> Result<AudioGraph, String> {
	let text = std::str::from_utf8(source)
		.map_err(|_| "The most likely cause is that the file isn't UTF-8 text. Save it as UTF-8.".to_string())?;
	let output: NodeSource = json5::from_str(text).map_err(|error| {
		format!("The most likely cause is that the file doesn't hold an audio graph node: {error}. Fix the reported field.")
	})?;

	let mut baker = Baker { id, nodes: Vec::new() };
	baker.node(&output, "the output node", Path::default())?;
	Ok(AudioGraph { nodes: baker.nodes })
}

/// The `NodeSource` struct is one node as an `.audiograph` file writes it: exactly one function field, plus `input`
/// for the functions that process one.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeSource {
	#[serde(rename = "$schema")]
	schema: Option<String>,
	sample: Option<String>,
	graph: Option<String>,
	round_robin: Option<Vec<NodeSource>>,
	random: Option<Vec<NodeSource>>,
	r#loop: Option<Box<NodeSource>>,
	gain: Option<f32>,
	varispeed: Option<f32>,
	pitch_shift: Option<f32>,
	input: Option<Box<NodeSource>>,
}

/// The `Function` enum is a [`NodeSource`] once its single function field is found.
enum Function<'a> {
	Sample(&'a str),
	Graph(&'a str),
	Select(Selector, &'a [NodeSource]),
	Loop(&'a NodeSource),
	Process(Processor, f32, &'a NodeSource),
}

/// The `Selector` enum names the functions that pick one of their entries on each spawn.
#[derive(Clone, Copy)]
enum Selector {
	RoundRobin,
	Random,
}

impl Selector {
	fn name(self) -> &'static str {
		match self {
			Selector::RoundRobin => "round_robin",
			Selector::Random => "random",
		}
	}

	fn node(self, inputs: Vec<u32>) -> AudioGraphNode {
		match self {
			Selector::RoundRobin => AudioGraphNode::RoundRobin { inputs },
			Selector::Random => AudioGraphNode::Random { inputs },
		}
	}
}

/// The `Processor` enum names the functions that change their `input` by a value, where a value of one changes nothing.
#[derive(Clone, Copy)]
enum Processor {
	Gain,
	Varispeed,
	PitchShift,
}

impl Processor {
	fn name(self) -> &'static str {
		match self {
			Processor::Gain => "gain",
			Processor::Varispeed => "varispeed",
			Processor::PitchShift => "pitch_shift",
		}
	}

	/// Returns how to fix `value` when the processor doesn't accept it.
	fn check(self, value: f32) -> Result<(), &'static str> {
		let (accepted, fix) = match self {
			Processor::Gain => (value >= 0.0, "Set a finite factor of zero or more."),
			Processor::Varispeed => ((0.25..=4.0).contains(&value), "Set a rate from 0.25 to 4.0."),
			Processor::PitchShift => ((0.5..=2.0).contains(&value), "Set a ratio from 0.5 to 2.0."),
		};
		if value.is_finite() && accepted { Ok(()) } else { Err(fix) }
	}

	fn node(self, input: u32, value: f32) -> AudioGraphNode {
		match self {
			Processor::Gain => AudioGraphNode::Gain { input, gain: value },
			Processor::Varispeed => AudioGraphNode::Varispeed { input, rate: value },
			Processor::PitchShift => AudioGraphNode::PitchShift { input, ratio: value },
		}
	}
}

impl NodeSource {
	/// Returns the node's one function, or the cause and fix when it has none, several, or a misplaced `input`.
	fn function(&self, location: &str) -> Result<Function<'_>, String> {
		// A processor without an `input` still counts as the node's function, so its own error can name it.
		let process = |processor, value: Option<f32>| {
			value.map(|value| {
				self.input
					.as_deref()
					.map(|input| Function::Process(processor, value, input))
					.ok_or(processor)
			})
		};
		let functions = [
			self.sample.as_deref().map(|id| Ok(Function::Sample(id))),
			self.graph.as_deref().map(|id| Ok(Function::Graph(id))),
			self.round_robin
				.as_deref()
				.map(|inputs| Ok(Function::Select(Selector::RoundRobin, inputs))),
			self.random
				.as_deref()
				.map(|inputs| Ok(Function::Select(Selector::Random, inputs))),
			self.r#loop.as_deref().map(|input| Ok(Function::Loop(input))),
			process(Processor::Gain, self.gain),
			process(Processor::Varispeed, self.varispeed),
			process(Processor::PitchShift, self.pitch_shift),
		];

		let mut set = functions.into_iter().flatten();
		let function = match (set.next(), set.next()) {
			(Some(function), None) => function,
			(first, _) => {
				let count = first.map_or(0, |_| 2 + set.count());
				return Err(format!(
					"The most likely cause is that {location} has {count} function fields. Give it exactly one of `sample`, `graph`, `round_robin`, `random`, `loop`, `gain`, `varispeed`, or `pitch_shift`."
				));
			}
		};
		let function = function.map_err(|processor| {
			format!(
				"The most likely cause is that {location} sets `{}` without an `input`. Add the node it processes as `input`.",
				processor.name()
			)
		})?;
		if self.input.is_some() && !matches!(function, Function::Process(..)) {
			return Err(format!(
				"The most likely cause is that {location} has an `input`, which only `gain`, `varispeed`, and `pitch_shift` take. Remove `input` or nest it under the function's own field."
			));
		}
		Ok(function)
	}
}

/// The `Path` struct records the processors between the output and a node, so a second one on a path is caught.
#[derive(Clone, Copy, Default)]
struct Path {
	varispeed: bool,
	pitch_shift: bool,
}

impl Path {
	/// Returns the path below `processor`, or `None` when the path already holds one and a graph allows only one.
	fn through(self, processor: Processor) -> Option<Path> {
		match processor {
			Processor::Gain => Some(self),
			Processor::Varispeed => (!self.varispeed).then_some(Path { varispeed: true, ..self }),
			Processor::PitchShift => (!self.pitch_shift).then_some(Path {
				pitch_shift: true,
				..self
			}),
		}
	}
}

/// The `Baker` struct appends a file's nodes inputs first, so each node's inputs precede it.
struct Baker<'a> {
	id: &'a str,
	nodes: Vec<AudioGraphNode>,
}

impl Baker<'_> {
	/// Appends `source` and its inputs and returns the index of the node that now stands for it.
	///
	/// A node that changes nothing, such as a gain of one or a selector with one input, isn't appended; its input
	/// stands for it instead, as the engine's authoring functions do.
	fn node(&mut self, source: &NodeSource, location: &str, path: Path) -> Result<u32, String> {
		if source.schema.is_some() && location != "the output node" {
			return Err(format!(
				"The most likely cause is that {location} sets `$schema`, which only the output node takes. Remove it."
			));
		}

		let node = match source.function(location)? {
			Function::Sample(resource_id) | Function::Graph(resource_id) if resource_id.is_empty() => {
				return Err(format!(
					"The most likely cause is that {location} names an empty resource ID. Write the asset's path under the assets directory."
				));
			}
			Function::Sample(resource_id) => AudioGraphNode::Sample {
				resource_id: resource_id.to_string(),
			},
			Function::Graph(resource_id) if resource_id == self.id => {
				return Err(format!(
					"The most likely cause is that {location} names this file, so it would contain itself. Name another `.audiograph` file."
				));
			}
			Function::Graph(resource_id) => AudioGraphNode::Graph {
				resource_id: resource_id.to_string(),
			},
			Function::Select(_, entries) if entries.is_empty() => {
				return Err(format!(
					"The most likely cause is that {location} selects from an empty array. List at least one node."
				));
			}
			Function::Select(selector, entries) => {
				let inputs = entries
					.iter()
					.enumerate()
					.map(|(index, entry)| self.node(entry, &format!("`{}` entry {index} of {location}", selector.name()), path))
					.collect::<Result<Vec<_>, _>>()?;
				if let [input] = inputs[..] {
					return Ok(input);
				}
				selector.node(inputs)
			}
			Function::Loop(input) => {
				let input = self.node(input, &format!("the `loop` of {location}"), path)?;
				if matches!(self.nodes[input as usize], AudioGraphNode::Loop { .. }) {
					return Ok(input);
				}
				AudioGraphNode::Loop { input }
			}
			Function::Process(processor, value, input) => {
				let name = processor.name();
				processor
					.check(value)
					.map_err(|fix| format!("The most likely cause is that {location} sets `{name}` to {value}. {fix}"))?;
				let input_location = format!("the `input` of {location}");
				if value == 1.0 {
					return self.node(input, &input_location, path);
				}
				let path = path.through(processor).ok_or_else(|| {
					format!(
						"The most likely cause is that {location} is a second `{name}` on one path. Combine them into one `{name}`."
					)
				})?;
				let input = self.node(input, &input_location, path)?;
				processor.node(input, value)
			}
		};

		if self.nodes.len() == MAX_NODES {
			return Err(format!(
				"The most likely cause is that the graph has more than {MAX_NODES} nodes. Remove nodes, such as selector entries, until it has at most {MAX_NODES}."
			));
		}
		self.nodes.push(node);
		Ok(self.nodes.len() as u32 - 1)
	}
}

#[cfg(test)]
mod tests;
