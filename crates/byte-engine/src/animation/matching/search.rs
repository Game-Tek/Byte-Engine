//! The runtime half of motion matching: a motion database packed into the clip pool's arena, the query a matched
//! state builds from its pose and requested motion, and the exact search over the database's frames.

use std::sync::Arc;

use resource_management::resources::motion_database::{MotionDatabase, MotionSearch};

use super::{GroundRoot, MAX_DIMENSIONS, direction, ground_at};

/// Frames in a small bounding block, which a search skips at once when the block can't hold a better frame.
const SMALL_BLOCK: usize = 16;
/// Frames in a large bounding block, which holds four small ones.
const LARGE_BLOCK: usize = 64;

/// The `DatabaseClip` struct places one clip in a resident database: its ID and where its frames are.
#[derive(Debug)]
pub(crate) struct DatabaseClip {
	pub(crate) id: Arc<str>,
	/// The clip's first frame among the database's frames.
	pub(crate) first: usize,
	pub(crate) frames: usize,
	pub(crate) pickable: usize,
}

/// The `DatabaseInfo` struct keeps the small part of a resident motion database outside the pool's arena: its clips
/// and its [`MotionSearch`]. The arena keeps the per-frame data.
#[derive(Debug)]
pub(crate) struct DatabaseInfo {
	pub(crate) clips: Box<[DatabaseClip]>,
	pub(crate) search: MotionSearch,
	pub(crate) node_dimensions: usize,
	pub(crate) frames: usize,
}

impl DatabaseInfo {
	pub(crate) fn dimensions(&self) -> usize {
		self.search.dimensions()
	}
}

/// Packs a baked database for the pool: the small part into a [`DatabaseInfo`], and every frame's features, ground
/// root, and the bounds of each block of frames into arena words.
///
/// Words hold the features, then the ground roots, then the small blocks' bounds, then the large blocks'; a block's
/// bounds are each dimension's minimum, then each one's maximum.
pub(crate) fn pack_database(database: MotionDatabase) -> (DatabaseInfo, Box<[u32]>) {
	let frames = database.frame_count();
	let MotionDatabase {
		clips,
		features,
		ground,
		search,
	} = database;
	let dimensions = search.dimensions();
	let mut words = Vec::with_capacity(packed_words(frames, dimensions));
	words.extend(features.iter().map(|value| value.to_bits()));
	words.extend(ground.iter().flatten().map(|value| value.to_bits()));
	for block in [SMALL_BLOCK, LARGE_BLOCK] {
		for rows in features.chunks(block * dimensions.max(1)) {
			let mut minimum = [f32::INFINITY; MAX_DIMENSIONS];
			let mut maximum = [f32::NEG_INFINITY; MAX_DIMENSIONS];
			for row in rows.chunks_exact(dimensions) {
				for (dimension, value) in row.iter().enumerate() {
					minimum[dimension] = minimum[dimension].min(*value);
					maximum[dimension] = maximum[dimension].max(*value);
				}
			}
			words.extend(minimum[..dimensions].iter().chain(&maximum[..dimensions]).map(|value| value.to_bits()));
		}
	}
	debug_assert_eq!(words.len(), packed_words(frames, dimensions));

	let mut first = 0;
	let clips = clips
		.into_iter()
		.map(|clip| {
			let placed = DatabaseClip {
				id: clip.id.into(),
				first,
				frames: clip.frames as usize,
				pickable: clip.pickable as usize,
			};
			first += placed.frames;
			placed
		})
		.collect();
	let info = DatabaseInfo {
		clips,
		node_dimensions: search.node_dimensions(),
		search,
		frames,
	};
	(info, words.into_boxed_slice())
}

/// Returns how many arena words a database of `frames` frames and `dimensions` dimensions packs into.
fn packed_words(frames: usize, dimensions: usize) -> usize {
	let bounds = |block: usize| frames.div_ceil(block) * dimensions * 2;
	frames * dimensions + frames * 3 + bounds(SMALL_BLOCK) + bounds(LARGE_BLOCK)
}

/// The `PackedDatabase` struct reads a resident motion database: its [`DatabaseInfo`] and its arena words.
///
/// The pool lends one for as long as players read it, so no admission or eviction moves the words meanwhile.
#[derive(Clone, Copy)]
pub(crate) struct PackedDatabase<'a> {
	pub(crate) info: &'a DatabaseInfo,
	words: &'a [u32],
}

impl<'a> PackedDatabase<'a> {
	pub(crate) fn new(info: &'a DatabaseInfo, words: &'a [u32]) -> Self {
		debug_assert_eq!(words.len(), packed_words(info.frames, info.dimensions()));
		Self { info, words }
	}

	/// Returns `frame`'s normalized feature vector, as the arena's words.
	fn features(self, frame: usize) -> &'a [u32] {
		let dimensions = self.info.dimensions();
		&self.words[frame * dimensions..][..dimensions]
	}

	/// Returns `frame`'s smoothed ground root.
	pub(crate) fn ground(self, frame: usize) -> GroundRoot {
		let at = self.info.frames * self.info.dimensions() + frame * 3;
		let value = |offset: usize| f32::from_bits(self.words[at + offset]);
		GroundRoot::from_array([value(0), value(1), value(2)])
	}

	/// Returns the ground root of clip `clip` at `time` seconds, between its frames as the reference specifies.
	pub(crate) fn ground_at(self, clip: usize, time: f32) -> GroundRoot {
		let clip = &self.info.clips[clip];
		ground_at(self.info.search.sample_rate, clip.frames, time, |frame| self.ground(clip.first + frame))
	}

	/// Returns the bounds of block `block` of `size` frames: each dimension's minimum, then each one's maximum.
	fn bounds(self, size: usize, block: usize) -> &'a [u32] {
		let dimensions = self.info.dimensions();
		let frames = self.info.frames;
		let mut at = frames * dimensions + frames * 3;
		if size == LARGE_BLOCK {
			at += frames.div_ceil(SMALL_BLOCK) * dimensions * 2;
		}
		&self.words[at + block * dimensions * 2..][..dimensions * 2]
	}
}

/// The `RequestedMotion` struct is what a matched state's query predicts the trajectory from: the velocity and facing
/// the application requests, and how the entity moved and turned in the last step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RequestedMotion {
	/// The held velocity, X and Z, in object-space units per second.
	pub(crate) velocity: [f32; 2],
	/// The held facing, a yaw about +Y in radians.
	pub(crate) facing: f32,
	/// The last step's root motion translation over its delta, in the object space after the step.
	pub(crate) start_velocity: [f32; 2],
	/// The last step's root motion yaw over its delta, in radians per second.
	pub(crate) start_turn_rate: f32,
}

/// The `TrajectorySample` struct is one predicted trajectory point of a query, in the entity's object space.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub(crate) struct TrajectorySample {
	pub(crate) time: f32,
	pub(crate) position: [f32; 2],
	pub(crate) direction: [f32; 2],
}

/// The `Query` struct is a normalized feature vector to search a database with, and the trajectory it predicts.
pub(crate) struct Query {
	pub(crate) features: [f32; MAX_DIMENSIONS],
	/// The first dimension the cost compares: the trajectory's, until the animator has produced two poses.
	pub(crate) first_dimension: usize,
	pub(crate) trajectory: [TrajectorySample; MAX_DIMENSIONS / 4],
	pub(crate) trajectory_len: usize,
}

/// Builds the query a matched state searches with.
///
/// `nodes` holds each feature node's position in the latest pose and its position in the pose before, with the last
/// step's root motion undone, and the seconds between them; without it the cost leaves out the node features. The
/// trajectory is predicted by critically damped springs from `motion`'s starting velocity and turn rate toward its
/// held velocity and facing.
pub(crate) fn build_query(info: &DatabaseInfo, nodes: Option<(&[[f32; 3]], &[[f32; 3]], f32)>, motion: RequestedMotion) -> Query {
	let search = &info.search;
	let mut query = Query {
		features: [0.0; MAX_DIMENSIONS],
		first_dimension: info.node_dimensions,
		trajectory: [TrajectorySample::default(); MAX_DIMENSIONS / 4],
		trajectory_len: search.trajectory_times.len(),
	};
	let mut values = query.features.iter_mut();
	let mut write = |components: &[f32]| {
		for component in components {
			*values.next().expect("a database has at most MAX_DIMENSIONS dimensions") = *component;
		}
	};
	match nodes {
		Some((current, previous, delta)) => {
			query.first_dimension = 0;
			for (node, (current, previous)) in search.nodes.iter().zip(current.iter().zip(previous)) {
				if node.position {
					write(current);
				}
				if node.velocity {
					write(&std::array::from_fn::<f32, 3, _>(|axis| (current[axis] - previous[axis]) / delta));
				}
			}
		}
		// Without poses the node dimensions stay at their mean, which the cost leaves out anyway.
		None => write(&search.mean[..info.node_dimensions]),
	}

	let spring = |halflife: f32| 2.0 * std::f32::consts::LN_2 / halflife;
	let (velocity_damping, facing_damping) = (spring(search.velocity_halflife), spring(search.facing_halflife));
	let difference = [
		motion.start_velocity[0] - motion.velocity[0],
		motion.start_velocity[1] - motion.velocity[1],
	];
	for (sample, &time) in query.trajectory.iter_mut().zip(search.trajectory_times.iter()) {
		let decay = (-velocity_damping * time).exp();
		let offset = (2.0 / velocity_damping) * (1.0 - decay) - time * decay;
		let decay = (-facing_damping * time).exp();
		let yaw = motion.facing + decay * (motion.start_turn_rate * time - motion.facing * (1.0 + facing_damping * time));
		*sample = TrajectorySample {
			time,
			position: [
				motion.velocity[0] * time + difference[0] * offset,
				motion.velocity[1] * time + difference[1] * offset,
			],
			direction: direction(yaw),
		};
	}
	let trajectory = &query.trajectory[..query.trajectory_len];
	if search.trajectory_position {
		for sample in trajectory {
			write(&sample.position);
		}
	}
	if search.trajectory_direction {
		for sample in trajectory {
			write(&sample.direction);
		}
	}

	for ((value, mean), scale) in query.features.iter_mut().zip(&search.mean).zip(&search.scale) {
		*value = (*value - mean) * scale;
	}
	query
}

/// The `Playing` struct is the clip a matched state plays when it searches, which sets the search's bar and the frames
/// it excludes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Playing {
	pub(crate) clip: usize,
	pub(crate) time: f32,
}

/// The `Found` struct is the frame a search found and its cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Found {
	pub(crate) clip: usize,
	/// The frame's time in its clip.
	pub(crate) time: f32,
	pub(crate) cost: f32,
}

/// The `SearchResult` struct is what one search measured: the bar candidates had to beat, and the frame that beat it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SearchResult {
	pub(crate) bar: Option<f32>,
	pub(crate) found: Option<Found>,
}

/// Searches `database` for the candidate frame of least cost against `query`, below the playing frame's cost.
///
/// Candidates are the pickable frames of the clips `resident` accepts, except the playing clip's frames within the
/// database's exclusion time of the playing time. Of equal costs, the lowest frame wins. Blocks of frames whose bounds
/// can't beat the best cost so far are skipped, which finds the same frame as comparing every candidate.
pub(crate) fn search(
	database: PackedDatabase<'_>,
	query: &Query,
	playing: Option<Playing>,
	mut resident: impl FnMut(usize) -> bool,
) -> SearchResult {
	let info = database.info;
	let rate = info.search.sample_rate;
	let range = query.first_dimension..info.dimensions();
	let query_features = &query.features[range.clone()];
	let bar = playing.and_then(|playing| {
		let clip = &info.clips[playing.clip];
		// The nearest frame, the earlier of two equally near ones.
		let frame = ((playing.time * rate - 0.5).ceil().max(0.0) as usize).min(clip.frames - 1);
		(frame < clip.pickable).then(|| cost(&database.features(clip.first + frame)[range.clone()], query_features, f32::INFINITY))
	});

	let mut best = bar.unwrap_or(f32::INFINITY);
	let mut found = None;
	for (index, clip) in info.clips.iter().enumerate() {
		if clip.pickable == 0 || !resident(index) {
			continue;
		}
		let excluded = playing
			.filter(|playing| playing.clip == index)
			.map(|playing| (playing.time - info.search.exclusion, playing.time + info.search.exclusion));
		let end = clip.first + clip.pickable;
		let mut frame = clip.first;
		while frame < end {
			let large_end = end.min((frame / LARGE_BLOCK + 1) * LARGE_BLOCK);
			if bound_cost(database.bounds(LARGE_BLOCK, frame / LARGE_BLOCK), &range, query_features) >= best {
				frame = large_end;
				continue;
			}
			while frame < large_end {
				let small_end = large_end.min((frame / SMALL_BLOCK + 1) * SMALL_BLOCK);
				if bound_cost(database.bounds(SMALL_BLOCK, frame / SMALL_BLOCK), &range, query_features) < best {
					for candidate in frame..small_end {
						// Only the playing clip and a better frame need the candidate's time.
						let time = || (candidate - clip.first) as f32 / rate;
						if excluded.is_some_and(|(start, end)| start < time() && time() < end) {
							continue;
						}
						let cost = cost(&database.features(candidate)[range.clone()], query_features, best);
						if cost < best {
							best = cost;
							found = Some(Found {
								clip: index,
								time: time(),
								cost,
							});
						}
					}
				}
				frame = small_end;
			}
		}
	}
	SearchResult { bar, found }
}

/// Sums the squared differences between a frame's features and the query's, stopping once the sum reaches `limit`.
///
/// It checks the limit after every eight dimensions rather than every one, so the compiler can vectorize the sums; a
/// sum that stops is at or above the limit either way.
fn cost(frame: &[u32], query: &[f32], limit: f32) -> f32 {
	let mut sum = 0.0;
	for (values, queries) in frame.chunks(8).zip(query.chunks(8)) {
		for (value, query) in values.iter().zip(queries) {
			let difference = f32::from_bits(*value) - query;
			sum += difference * difference;
		}
		if sum >= limit {
			break;
		}
	}
	sum
}

/// Returns the least cost any frame within `bounds` can have against the query: the squared distance from the query
/// to the bounding box, which never exceeds a frame's own cost.
fn bound_cost(bounds: &[u32], range: &std::ops::Range<usize>, query: &[f32]) -> f32 {
	let dimensions = bounds.len() / 2;
	let (minimum, maximum) = bounds.split_at(dimensions);
	let mut sum = 0.0;
	for ((minimum, maximum), query) in minimum[range.clone()].iter().zip(&maximum[range.clone()]).zip(query) {
		let nearest = query.clamp(f32::from_bits(*minimum), f32::from_bits(*maximum));
		let difference = nearest - query;
		sum += difference * difference;
	}
	sum
}
