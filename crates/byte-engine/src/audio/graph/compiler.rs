//! Audio graph validation and selected-path compilation.

use super::*;

impl AudioGraph {
	/// Validates the complete authored graph and compiles its current selection
	/// without advancing selector state.
	pub(crate) fn compile(&self) -> Result<CompiledAudioGraph, String> {
		self.compile_selection().map(|(compiled, _)| compiled)
	}

	/// Resolves the current path and returns the choices that a successful
	/// factory submission must commit.
	pub(super) fn compile_selection(&self) -> Result<(CompiledAudioGraph, SelectorCommits), String> {
		if self.nodes.is_empty() || self.nodes.len() > MAX_AUDIO_GRAPH_NODES {
			return Err(format!(
				"Invalid audio graph. A graph must contain between 1 and {MAX_AUDIO_GRAPH_NODES} nodes."
			));
		}
		self.validate()?;

		let mut selector_commits = SelectorCommits::new();
		let compiled = self.compile_selected(self.output, &mut selector_commits)?;
		Ok((compiled, selector_commits))
	}

	/// Commits selection state only after the compiled play has been published.
	pub(super) fn commit_selectors(&mut self, selector_commits: &SelectorCommits) {
		for commit in selector_commits {
			match *commit {
				SelectorCommit::RoundRobin { node_id } => {
					let AudioNode::RoundRobin(node) = &mut *self.nodes[node_id.0] else {
						unreachable!("validated round-robin selection referred to another node type");
					};
					node.next_index = (node.next_index + 1) % node.inputs.len();
				}
				SelectorCommit::Random { node_id, selection } => {
					let AudioNode::Random(node) = &mut *self.nodes[node_id.0] else {
						unreachable!("validated random selection referred to another node type");
					};
					node.commit(selection);
				}
			}
		}
	}

	/// Validates every selectable path without changing selector state.
	fn validate(&self) -> Result<(), String> {
		if self.output.0 >= self.nodes.len() {
			return Err("Invalid audio graph output. The output refers to a node outside this graph.".to_string());
		}

		let mut cached = [None; MAX_AUDIO_GRAPH_NODES];
		let mut visiting = [false; MAX_AUDIO_GRAPH_NODES];
		self.validate_node(
			self.output,
			&mut cached[..self.nodes.len()],
			&mut visiting[..self.nodes.len()],
		)?;
		if cached[..self.nodes.len()].iter().any(Option::is_none) {
			return Err("Invalid audio graph. One or more authored nodes are not connected to the graph output.".to_string());
		}
		Ok(())
	}

	/// Validates one node and returns the path constraints inherited by nodes
	/// that consume it.
	fn validate_node(
		&self,
		node_id: AudioNodeId,
		cached: &mut [Option<NodeProperties>],
		visiting: &mut [bool],
	) -> Result<NodeProperties, String> {
		if node_id.0 >= self.nodes.len() {
			return Err("Invalid audio graph connection. A node refers to an input outside this graph.".to_string());
		}
		if let Some(properties) = cached[node_id.0] {
			return Ok(properties);
		}
		if visiting[node_id.0] {
			return Err("Invalid audio graph cycle. A node input eventually refers back to the same node.".to_string());
		}
		visiting[node_id.0] = true;

		let properties = match &*self.nodes[node_id.0] {
			AudioNode::Sample { resource_id } => {
				if resource_id.is_empty() {
					return Err("Invalid audio sample node. The sample resource ID is empty.".to_string());
				}
				NodeProperties::default()
			}
			AudioNode::RoundRobin(node) => {
				if node.inputs.is_empty() {
					return Err("Invalid audio round-robin node. No input chains were provided.".to_string());
				}
				let mut combined = NodeProperties::default();
				for input in &node.inputs {
					combined.include(self.validate_node(*input, cached, visiting)?);
				}
				combined
			}
			AudioNode::Random(node) => {
				if node.inputs.is_empty() {
					return Err("Invalid audio random node. No input chains were provided.".to_string());
				}
				if node.last_index.is_some_and(|index| index >= node.inputs.len()) {
					return Err("Invalid audio random node state. Its previous selection is outside its inputs.".to_string());
				}
				let mut combined = NodeProperties::default();
				for input in &node.inputs {
					combined.include(self.validate_node(*input, cached, visiting)?);
				}
				combined
			}
			AudioNode::Loop { input } => self.validate_node(*input, cached, visiting)?,
			AudioNode::Gain { input, gain } => {
				if !gain.is_finite() || *gain < 0.0 {
					return Err("Invalid audio gain node. Its gain is not finite or is negative.".to_string());
				}
				self.validate_node(*input, cached, visiting)?
			}
			AudioNode::Varispeed { input, rate } => {
				if !rate.is_finite() || !(0.25..=4.0).contains(rate) {
					return Err("Invalid audio varispeed node. Its rate is not finite or is outside 0.25..=4.0.".to_string());
				}
				let mut inherited = self.validate_node(*input, cached, visiting)?;
				if inherited.has_varispeed {
					return Err(
						"Invalid audio graph. At least one selectable path contains more than one varispeed node.".to_string(),
					);
				}
				inherited.has_varispeed = true;
				inherited
			}
			AudioNode::PitchShift { input, ratio } => {
				if !ratio.is_finite() || !(0.5..=2.0).contains(ratio) {
					return Err("Invalid audio pitch-shift node. Its ratio is not finite or is outside 0.5..=2.0.".to_string());
				}
				let mut inherited = self.validate_node(*input, cached, visiting)?;
				if inherited.has_pitch_shift {
					return Err(
						"Invalid audio graph. At least one selectable path contains more than one pitch-shift node."
							.to_string(),
					);
				}
				inherited.has_pitch_shift = true;
				inherited
			}
			AudioNode::Custom(input, _) => self.validate_node(*input, cached, visiting)?,
		};

		visiting[node_id.0] = false;
		cached[node_id.0] = Some(properties);
		Ok(properties)
	}

	/// Compiles only the branch selected for this submission and records the
	/// selectors that must advance after compilation succeeds.
	fn compile_selected(
		&self,
		node_id: AudioNodeId,
		selector_commits: &mut SelectorCommits,
	) -> Result<CompiledAudioGraph, String> {
		let node = self
			.nodes
			.get(node_id.0)
			.ok_or_else(|| "Invalid audio graph connection. A selected node is outside this graph.".to_string())?;

		match &**node {
			AudioNode::Sample { resource_id } => Ok(CompiledAudioGraph {
				resource_id: resource_id.clone(),
				playback_mode: SamplePlaybackMode::Once,
				playback_rate: PlaybackRate::UNITY,
				processors: SmallVec::new(),
				muted: false,
				muted_drain_latency: 0,
			}),
			AudioNode::RoundRobin(node) => {
				let selected = node.inputs[node.next_index % node.inputs.len()];
				selector_commits.push(SelectorCommit::RoundRobin { node_id });
				self.compile_selected(selected, selector_commits)
			}
			AudioNode::Random(node) => {
				let selection = node.selection();
				selector_commits.push(SelectorCommit::Random { node_id, selection });
				self.compile_selected(node.inputs[selection.index], selector_commits)
			}
			AudioNode::Loop { input } => {
				let mut compiled = self.compile_selected(*input, selector_commits)?;
				compiled.playback_mode = SamplePlaybackMode::Loop;
				Ok(compiled)
			}
			AudioNode::Gain { input, gain } => {
				let mut compiled = self.compile_selected(*input, selector_commits)?;
				if *gain == 0.0 {
					// A mute still needs its source timeline and selector state,
					// but no processor below or above it can affect the output.
					compiled.muted = true;
					compiled.muted_drain_latency +=
						compiled.processors.iter().map(|processor| processor.latency()).sum::<usize>();
					compiled.processors.clear();
				} else if !compiled.muted {
					// Adjacent gains are one linear operation. Keep separate
					// processors if their product cannot remain a finite gain.
					let fused = if let Some(AudioProcessor::Gain(input_gain)) = compiled.processors.last_mut() {
						let combined_gain = *input_gain * *gain;
						if combined_gain.is_finite() {
							*input_gain = combined_gain;
							true
						} else {
							false
						}
					} else {
						false
					};
					if !fused {
						compiled.processors.push(AudioProcessor::Gain(*gain));
					}
				}
				Ok(compiled)
			}
			AudioNode::Varispeed { input, rate } => {
				let mut compiled = self.compile_selected(*input, selector_commits)?;
				compiled.playback_rate = PlaybackRate::from_rate(*rate);
				Ok(compiled)
			}
			AudioNode::PitchShift { input, ratio } => {
				let mut compiled = self.compile_selected(*input, selector_commits)?;
				if *ratio != 1.0 {
					let processor = AudioProcessor::PitchShift(*ratio);
					if compiled.muted {
						compiled.muted_drain_latency += processor.latency();
					} else {
						compiled.processors.push(processor);
					}
				}
				Ok(compiled)
			}
			AudioNode::Custom(input, function) => {
				let mut compiled = self.compile_selected(*input, selector_commits)?;
				if !compiled.muted {
					compiled.processors.push(AudioProcessor::Custom(function.clone()));
				}
				Ok(compiled)
			}
		}
	}
}
