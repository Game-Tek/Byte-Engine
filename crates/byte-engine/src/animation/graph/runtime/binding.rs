//! Clip data a graph resolves once against its canonical skeleton, shared by every player of the graph.

use std::sync::OnceLock;

use super::*;

/// The `ClipBinding` struct keeps what playing one clip of a state needs once the clip is resident: how its tracks
/// map onto the canonical skeleton, how long it lasts, and the frame rate its frame points count in.
pub(in crate::animation::graph) struct ClipBinding {
	pub(super) resource_id: Box<str>,
	pub(super) targets: TrackTargets,
	pub(super) duration: f32,
	/// The authored frames per second, which [`ClipTime::Frame`] points count in.
	pub(super) frame_rate: Option<f32>,
}

/// The `GraphBinding` struct lets every player of a graph share the clip data resolved against the graph's canonical
/// skeleton, so playing a state costs no pose-map construction after the first player enters it.
///
/// Players fill it as clips become resident: the initial clip supplies the canonical skeleton, and a state's clips
/// bind the first time any player enters the state. Each slot is written once and only read afterwards, so players
/// on several threads can share it. A binding outlives the pool's copy of its clip, so it assumes a resource ID
/// names the same clip for as long as the graph lives: a clip that reloads with other tracks needs a new graph.
pub(in crate::animation::graph) struct GraphBinding {
	skeleton: OnceLock<Arc<Skeleton>>,
	/// One slot per graph state, holding its clips' bindings in [`StateMotion::clips`] order.
	states: Box<[OnceLock<Box<[ClipBinding]>>]>,
}

impl GraphBinding {
	/// Creates an empty binding for a graph of `state_count` states.
	pub(in crate::animation::graph) fn new(state_count: usize) -> Self {
		Self {
			skeleton: OnceLock::new(),
			states: (0..state_count).map(|_| OnceLock::new()).collect(),
		}
	}

	/// Returns the canonical skeleton, taking it from `resident` the first time it's asked for.
	pub(super) fn skeleton(&self, resident: impl FnOnce() -> Arc<Skeleton>) -> &Arc<Skeleton> {
		self.skeleton.get_or_init(resident)
	}

	/// Returns the bindings of a state's clips, resolving them with `resolve` the first time the state is entered.
	///
	/// Call [`Self::skeleton`] first: `resolve` maps clips onto the skeleton it returns.
	pub(super) fn state(&self, state: usize, resolve: impl FnOnce() -> Box<[ClipBinding]>) -> &[ClipBinding] {
		self.states[state].get_or_init(resolve)
	}
}
