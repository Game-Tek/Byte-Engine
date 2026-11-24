//! Clip data a graph resolves once against its canonical skeleton, shared by every player of the graph.

use std::sync::OnceLock;

use super::*;

/// The `ClipBinding` struct keeps what playing one clip of a state needs once the clip is resident: how its tracks
/// map onto the canonical skeleton, how long it lasts, and the frame rate its frame points count in.
pub(in super::super) struct ClipBinding {
	pub(super) resource_id: Arc<str>,
	pub(super) targets: TrackTargets,
	pub(super) duration: f32,
	/// The authored frames per second, which [`ClipTime::Frame`] points count in.
	pub(super) frame_rate: Option<f32>,
	/// Where a matched state's clip keeps its ground roots, which place its pose and measure its root motion; `None`
	/// for the clips of every other state.
	pub(super) ground: Option<GroundBinding>,
}

/// The `GroundBinding` struct names the motion database a matched state's clip belongs to and the clip's place in it,
/// so sampling the clip places its pose relative to its ground root.
pub(in super::super) struct GroundBinding {
	pub(super) database: Arc<str>,
	pub(super) clip: usize,
}

/// The `GroundPlacement` struct holds the nodes that place a matched state's pose on its ground root, resolved once
/// against the canonical skeleton: the database's root node and its ancestors.
pub(in super::super) struct GroundPlacement {
	pub(super) root: u32,
	/// The root node's ancestors, nearest first, each with its rest transform.
	pub(super) ancestors: Box<[(u32, LocalTransform)]>,
	/// The object-space transform of the root node's parent with every ancestor at rest.
	pub(super) parent_rest: LocalTransform,
}

/// The `StateBinding` struct keeps one state's clips resolved against the canonical skeleton, and a matched state's
/// database nodes.
pub(in super::super) struct StateBinding {
	/// The clips in [`StateMotion::clips`] order, or in its motion database's order for a matched state.
	pub(super) clips: Box<[ClipBinding]>,
	/// The canonical node of each of a matched state's database feature nodes, in the database's node order, or why
	/// they don't resolve; `None` for every other state.
	pub(super) nodes: Option<Result<Box<[u32]>, AnimationGraphPlayerError>>,
	/// The nodes that place a matched state's poses on their ground roots, which every clip of its database shares;
	/// `None` for every other state, and for a matched state whose database root node doesn't resolve, which rejects
	/// the player before it posts a pose.
	pub(super) placement: Option<GroundPlacement>,
}

/// The `GraphBinding` struct lets every player of a graph share the clip data resolved against the graph's canonical
/// skeleton, so playing a state costs no pose-map construction after the first player enters it.
///
/// Players fill it as clips become resident: the initial clip supplies the canonical skeleton, which the binding keeps
/// its own copy of so that it outlives the clip's residency in the pool, and a state's clips
/// bind the first time any player enters the state. Each slot is written once and only read afterwards, so players
/// on several threads can share it. A binding outlives the pool's copy of its clip, so it assumes a resource ID
/// names the same clip for as long as the graph lives: a clip that reloads with other tracks needs a new graph.
pub(in super::super) struct GraphBinding {
	skeleton: OnceLock<Skeleton>,
	/// One slot per graph state.
	states: Box<[OnceLock<StateBinding>]>,
}

impl GraphBinding {
	/// Creates an empty binding for a graph of `state_count` states.
	pub(in super::super) fn new(state_count: usize) -> Self {
		Self {
			skeleton: OnceLock::new(),
			states: (0..state_count).map(|_| OnceLock::new()).collect(),
		}
	}

	/// Returns the canonical skeleton, copying it from the skeleton `resident` returns the first time it's asked for.
	pub(super) fn skeleton<'a>(&self, resident: impl FnOnce() -> &'a Skeleton) -> &Skeleton {
		self.skeleton.get_or_init(|| {
			// The pool's copy goes away when the clip is evicted, so the graph keeps one of its own.
			let resident = resident();
			Skeleton {
				nodes: resident.nodes.clone(),
				root_bone: resident.root_bone,
			}
		})
	}

	/// Returns the canonical skeleton that [`Self::skeleton`] took when the graph's first player started.
	pub(super) fn canonical(&self) -> &Skeleton {
		self.skeleton
			.get()
			.expect("a playing graph took its canonical skeleton when its first player started")
	}

	/// Returns a state's binding, resolving it with `resolve` the first time the state is entered.
	///
	/// Call [`Self::skeleton`] first: `resolve` maps clips onto the skeleton it returns.
	pub(super) fn state(&self, state: usize, resolve: impl FnOnce() -> StateBinding) -> &StateBinding {
		self.states[state].get_or_init(resolve)
	}

	/// Returns a state's binding, which [`Self::state`] resolved when a player first started the state.
	pub(super) fn bound(&self, state: usize) -> &StateBinding {
		self.states[state]
			.get()
			.expect("a playing state's clips were bound when a player started it")
	}
}
