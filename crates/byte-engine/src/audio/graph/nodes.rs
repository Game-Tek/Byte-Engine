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
///
/// Clones keep the authored identity, so a cloned graph still compares equal
/// to its source while owning an independent closure copy. The identity lives
/// inside the box to keep this handle two words wide, which lets
/// [`AudioNode`] stay inline in its small box.
pub(crate) struct CustomAudioFunction(Box<dyn CustomFunctionPrototype>);

impl CustomAudioFunction {
	pub(crate) fn new<F>(function: F) -> Self
	where
		F: FnMut(AudioGraphTime, &mut [f32]) + Clone + Send + Sync + 'static,
	{
		Self(Box::new(IdentifiedFunction {
			// A fresh standard-library hash key gives each authored function a
			// distinct identity without an engine-owned counter.
			id: new_random_seed(),
			function,
		}))
	}

	pub(crate) fn create(&self) -> RuntimeCustomFunction {
		self.0.create()
	}
}

impl Clone for CustomAudioFunction {
	fn clone(&self) -> Self {
		Self(self.0.clone_prototype())
	}
}

/// The `CustomFunctionPrototype` trait lets a type-erased closure prototype
/// hand out independent copies, both for new playbacks and for cloned graphs.
trait CustomFunctionPrototype: Send + Sync {
	/// Returns the identity shared by every clone of one authored function.
	fn id(&self) -> u64;

	/// Creates the mutable closure state owned by one playback.
	fn create(&self) -> RuntimeCustomFunction;

	/// Copies the prototype so each graph clone owns its own closure.
	fn clone_prototype(&self) -> Box<dyn CustomFunctionPrototype>;
}

/// The `IdentifiedFunction` struct pairs an authored closure with its identity.
#[derive(Clone)]
struct IdentifiedFunction<F> {
	id: u64,
	function: F,
}

impl<F> CustomFunctionPrototype for IdentifiedFunction<F>
where
	F: FnMut(AudioGraphTime, &mut [f32]) + Clone + Send + Sync + 'static,
{
	fn id(&self) -> u64 {
		self.id
	}

	fn create(&self) -> RuntimeCustomFunction {
		Box::new(self.function.clone())
	}

	fn clone_prototype(&self) -> Box<dyn CustomFunctionPrototype> {
		Box::new(self.clone())
	}
}

impl fmt::Debug for CustomAudioFunction {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("CustomAudioFunction")
	}
}

impl PartialEq for CustomAudioFunction {
	fn eq(&self, other: &Self) -> bool {
		self.0.id() == other.0.id()
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
				if slot >= previous { slot + 1 } else { slot }
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
///
/// Each [`RandomState`](std::hash::RandomState) gets fresh keys from the
/// standard library, so every authored node starts from a different state
/// without an engine-owned seed counter.
fn new_random_seed() -> u64 {
	use std::hash::BuildHasher as _;

	mix_random_bits(std::hash::RandomState::new().hash_one(RANDOM_STATE_INCREMENT))
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
