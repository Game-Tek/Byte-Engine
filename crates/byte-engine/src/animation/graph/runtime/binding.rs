//! Clip data a graph resolves against its canonical skeleton as its clips become resident, shared by every player of
//! the graph.

#![expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md")]

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
/// skeleton, so playing a state costs no pose-map construction.
///
/// A graph has one once its initial clip is resident: [`AnimationGraph::bind`] creates it with its own copy of that
/// clip's skeleton, which outlives the clip's residency in the pool, and binds each state once all of its clips are
/// resident. Players only read it. A binding outlives the pool's copy of its clips, so it assumes a resource ID names
/// the same clip for as long as the graph lives: a clip that reloads with other tracks needs a new graph.
pub(in super::super) struct GraphBinding {
	/// The canonical skeleton every clip of the graph maps onto.
	pub(super) skeleton: Skeleton,
	/// One slot per graph state, filled once the state's clips are resident.
	states: Box<[Option<StateBinding>]>,
}

impl GraphBinding {
	/// Creates a binding onto `skeleton` with none of its `state_count` states bound.
	fn new(skeleton: Skeleton, state_count: usize) -> Self {
		Self {
			skeleton,
			states: (0..state_count).map(|_| None).collect(),
		}
	}

	/// Returns a state's binding, which [`AnimationGraph::bind`] made before any player could start the state.
	pub(super) fn state(&self, state: usize) -> &StateBinding {
		self.states[state]
			.as_ref()
			.expect("a state's clips are bound before a player starts it, as soon as they are resident")
	}
}

impl AnimationGraph {
	/// Binds what of the graph `pool` makes resident: the canonical skeleton once the initial clip is resident, then
	/// every unbound state whose clips are all resident. Does nothing when the pool admitted no clip since the last
	/// call, so call it before every evaluation of the graph's players.
	///
	/// A player starts a state only once all of its clips are resident, and the pool admits nothing while players
	/// evaluate, so every state a player starts is bound. A graph plays from one pool: its admission count tells
	/// which of its clips the binding has seen.
	pub(super) fn bind(&mut self, pool: &AnimationPool) {
		if self.bound_at == pool.admissions() {
			return;
		}
		self.bound_at = pool.admissions();
		if self.binding.is_none() {
			let Ok(initial) = initial_clip(self, pool) else {
				return;
			};
			// The pool's copy goes away when the clip is evicted, so the graph keeps one of its own.
			let resident = pool
				.resident(initial)
				.expect("the initial clip was found resident above")
				.skeleton;
			let skeleton = Skeleton {
				nodes: resident.nodes.clone(),
				root_bone: resident.root_bone,
			};
			self.binding = Some(GraphBinding::new(skeleton, self.states.len()));
		}
		let binding = self.binding.as_mut().expect("the graph was bound above");
		for (slot, state) in binding.states.iter_mut().zip(&self.states) {
			if slot.is_none() && state_resident(&state.motion, pool) {
				*slot = Some(bind_clips(&state.motion, pool, &binding.skeleton));
			}
		}
	}

	/// Returns the graph's binding, which [`Self::bind`] made before any player could start.
	pub(super) fn bound(&self) -> &GraphBinding {
		self.binding
			.as_ref()
			.expect("a playing graph was bound when its initial clip became resident")
	}
}

/// Returns the resource ID of the clip whose skeleton becomes `graph`'s canonical skeleton once it is resident: the
/// initial state's first clip, or for a matched initial state its database's first clip. Returns the clip that isn't
/// resident yet instead.
pub(super) fn initial_clip<'a>(graph: &'a AnimationGraph, pool: &'a AnimationPool) -> Result<&'a str, MissingClip> {
	let state = graph.initial_state();
	let motion = &graph.state(state).motion;
	let (initial, missing) = match motion {
		StateMotion::Matched { database } => {
			let Some(resident) = pool.resident_database(database.resource_id()) else {
				return Err(MissingClip::State { state, clip: 0 });
			};
			(&*resident.info.clips[0].id, MissingClip::Database { state, clip: 0 })
		}
		_ => (motion.clips()[0].resource_id(), MissingClip::State { state, clip: 0 }),
	};
	if pool.is_resident(initial) {
		Ok(initial)
	} else {
		Err(missing)
	}
}

/// Returns whether every clip `motion` plays is resident: for a matched state, its database and each clip the database
/// names. A player starts a state on the same condition.
fn state_resident(motion: &StateMotion, pool: &AnimationPool) -> bool {
	if !motion.clips().iter().all(|clip| pool.is_resident(clip.resource_id())) {
		return false;
	}
	let StateMotion::Matched { database } = motion else {
		return true;
	};
	pool.resident_database(database.resource_id())
		.is_some_and(|resident| resident.info.clips.iter().all(|clip| pool.is_resident(&clip.id)))
}
