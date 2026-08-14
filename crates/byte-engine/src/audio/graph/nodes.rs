//! Authored audio node definitions and selector state.

use super::*;

/// The `AudioNodeId` struct identifies one node inside an [`AudioGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioNodeId(pub(crate) usize);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AudioNode {
	Sample { resource_id: String },
	RoundRobin(Box<RoundRobinNode>),
	Random(Box<RandomNode>),
	Loop { input: AudioNodeId },
	Gain { input: AudioNodeId, gain: f32 },
	Varispeed { input: AudioNodeId, rate: f32 },
	PitchShift { input: AudioNodeId, ratio: f32 },
	Custom(AudioNodeId, CustomAudioFunction),
}

impl AudioNode {
	/// Moves every input connection by the offset assigned while graphs are
	/// merged under a selector node.
	pub(crate) fn remap_inputs(&mut self, offset: usize) {
		match self {
			Self::Sample { .. } => {}
			Self::RoundRobin(node) => {
				for input in &mut node.inputs {
					input.0 += offset;
				}
			}
			Self::Random(node) => {
				for input in &mut node.inputs {
					input.0 += offset;
				}
			}
			Self::Loop { input }
			| Self::Gain { input, .. }
			| Self::Varispeed { input, .. }
			| Self::PitchShift { input, .. } => input.0 += offset,
			Self::Custom(input, _) => input.0 += offset,
		}
	}
}

/// The `CustomAudioFunction` struct retains a closure prototype that can
/// create independent mutable state for each playback.
#[derive(Clone)]
pub(crate) struct CustomAudioFunction(CustomFunctionFactory);

impl CustomAudioFunction {
	pub(crate) fn new<F>(function: F) -> Self
	where
		F: FnMut(AudioGraphTime, &mut [f32]) + Clone + Send + Sync + 'static,
	{
		Self(Arc::new(move || Box::new(function.clone())))
	}

	pub(crate) fn create(&self) -> RuntimeCustomFunction {
		(self.0)()
	}
}

impl fmt::Debug for CustomAudioFunction {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("CustomAudioFunction")
	}
}

impl PartialEq for CustomAudioFunction {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.0, &other.0)
	}
}

/// The `RoundRobinNode` struct keeps branch connections and per-instance
/// selection state for an authored round-robin node with at least two inputs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoundRobinNode {
	pub(crate) inputs: SelectorInputs,
	pub(crate) next_index: usize,
}

/// The `RandomNode` struct keeps branch connections and per-instance
/// pseudo-random state for a non-repeating authored selector with at least two
/// inputs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RandomNode {
	pub(crate) inputs: SelectorInputs,
	pub(crate) state: u64,
	pub(crate) last_index: Option<usize>,
}

impl RandomNode {
	/// Creates a random selector with state unique to this authored node.
	pub(crate) fn new(inputs: SelectorInputs) -> Self {
		Self {
			inputs,
			state: new_random_seed(),
			last_index: None,
		}
	}

	/// Peeks at the next selection without changing authored graph state.
	pub(crate) fn selection(&self) -> RandomSelection {
		let next_state = self.state.wrapping_add(RANDOM_STATE_INCREMENT);
		let random = mix_random_bits(next_state);
		let input_count = self.inputs.len();
		let index = match self.last_index {
			Some(previous) => {
				// Draw from N - 1 slots, then skip the previous input. This
				// preserves a uniform choice without retrying or allocating.
				let slot = (random % (input_count - 1) as u64) as usize;
				if slot >= previous {
					slot + 1
				} else {
					slot
				}
			}
			None => (random % input_count as u64) as usize,
		};
		RandomSelection { index, next_state }
	}

	/// Commits the selection that was published by the graph factory.
	pub(crate) fn commit(&mut self, selection: RandomSelection) {
		debug_assert_eq!(self.selection(), selection);
		self.state = selection.next_state;
		self.last_index = Some(selection.index);
	}
}

/// The `RandomSelection` struct pairs one selected input with the generator
/// state to commit after publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RandomSelection {
	pub(crate) index: usize,
	pub(crate) next_state: u64,
}

/// Records one state transition to apply after a graph is published.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SelectorCommit {
	RoundRobin {
		node_id: AudioNodeId,
	},
	Random {
		node_id: AudioNodeId,
		selection: RandomSelection,
	},
}

/// Produces a distinct initial state without adding work to the audio thread.
fn new_random_seed() -> u64 {
	let sequence = NEXT_RANDOM_SEED.fetch_add(RANDOM_STATE_INCREMENT, Ordering::Relaxed);
	let time = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |duration| duration.as_nanos() as u64);
	mix_random_bits(sequence ^ time.rotate_left(17))
}

/// Mixes one generator state into well-distributed pseudo-random bits.
fn mix_random_bits(mut value: u64) -> u64 {
	value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
	value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
	value ^ (value >> 31)
}

/// The `NodeProperties` struct summarizes constraints present on any selectable
/// path that can reach one authored node.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NodeProperties {
	pub(crate) has_varispeed: bool,
	pub(crate) has_pitch_shift: bool,
}

impl NodeProperties {
	pub(crate) fn include(&mut self, other: Self) {
		self.has_varispeed |= other.has_varispeed;
		self.has_pitch_shift |= other.has_pitch_shift;
	}
}
