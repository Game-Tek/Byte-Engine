//! Bake-time feature extraction: samples a motion database's clips frame by frame, measures every frame's ground root
//! and features, and normalizes them, as the animation reference's Motion databases section specifies.

use std::fmt;

use resource_management::resources::{
	animation::Animation,
	motion_database::{MotionClip, MotionDatabase, MotionNode, MotionSearch},
};

use super::{GroundRoot, MAX_DIMENSIONS, bead::MotionBead, ground_at, turn_back};
use crate::animation::{
	math::{NodeLookup, animated_yaw, model_rotation, object_transform, unique_node, yaw_of_direction},
	sample_local_pose,
};

/// The `DatabaseError` enum names why a motion database's clips can't form a database, for the bake to report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DatabaseError {
	/// A clip's skeleton has no node of a name the database uses.
	NodeNotFound { clip: String, node: String },
	/// A clip's skeleton has several nodes of a name the database uses.
	DuplicateNode { clip: String, node: String },
	/// No feature has a positive weight.
	NoDimensions,
	/// The features take more dimensions than a search compares.
	TooManyDimensions(usize),
	/// No clip is long enough to hold a frame and the whole trajectory after it.
	NoPickableFrame,
}

impl fmt::Display for DatabaseError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NodeNotFound { clip, node } => write!(
				formatter,
				"Motion database node '{node}' is missing from clip '{clip}'. The most likely cause is a clip authored on another skeleton, or a misspelled node name. Name a node every clip's skeleton has, or remove the clip from `clips`."
			),
			Self::DuplicateNode { clip, node } => write!(
				formatter,
				"Motion database node '{node}' appears more than once in clip '{clip}'. The most likely cause is a skeleton with several nodes of that name. Rename the nodes in the source file, or name another node."
			),
			Self::NoDimensions => write!(
				formatter,
				"Motion database has no features. The most likely cause is that every weight is zero. Give `trajectory.position`, `trajectory.direction`, or a node of `nodes` a positive weight."
			),
			Self::TooManyDimensions(dimensions) => write!(
				formatter,
				"Motion database features take {dimensions} dimensions, more than the {MAX_DIMENSIONS} a search compares. The most likely cause is too many nodes or trajectory times. Remove nodes from `nodes` or times from `trajectory.times`."
			),
			Self::NoPickableFrame => write!(
				formatter,
				"Motion database has no frame a search can find. The most likely cause is that every clip is shorter than the last trajectory time. Add longer clips, or shorten `trajectory.times`."
			),
		}
	}
}

/// The `ClipSamples` struct holds one clip's frames as the bake measures them: the raw ground root and the features'
/// nodes in object space.
struct ClipSamples {
	ground: Vec<GroundRoot>,
	/// Each frame's feature-node object-space positions, frame after frame, one per node of the database.
	positions: Vec<[f32; 3]>,
}

/// Builds a motion database from its settings and its clips, in database order, each with its resource ID.
///
/// It samples every clip at the database's rate, smooths each clip's ground root, measures every frame's features in
/// the frame's root frame, and normalizes them over the pickable frames. Pass the result to the resource store.
pub(crate) fn build_database(settings: &MotionBead, clips: &[(String, &Animation)]) -> Result<MotionDatabase, DatabaseError> {
	let nodes: Vec<MotionNode> = settings
		.nodes
		.iter()
		.map(|(name, weights)| MotionNode {
			name: name.clone(),
			position: weights.position > 0.0,
			velocity: weights.velocity > 0.0,
		})
		.collect();
	let layout = Layout::new(settings);
	if layout.dimensions == 0 {
		return Err(DatabaseError::NoDimensions);
	}
	if layout.dimensions > MAX_DIMENSIONS {
		return Err(DatabaseError::TooManyDimensions(layout.dimensions));
	}

	let rate = settings.sample_rate;
	let horizon = settings.trajectory.times.last().copied().unwrap_or(0.0);
	let mut motion_clips = Vec::with_capacity(clips.len());
	let mut ground = Vec::new();
	let mut features = Vec::new();
	let mut pickable = Vec::new();
	for (id, animation) in clips {
		let samples = sample_clip(id, animation, settings, &nodes)?;
		let frames = samples.ground.len();
		let clip_pickable = pickable_count(animation.duration, rate, horizon).min(frames);
		let first = ground.len();
		ground.extend(samples.ground.iter().map(|root| [root.x, root.z, root.yaw]));
		features.resize(features.len() + frames * layout.dimensions, 0.0);
		for frame in 0..frames {
			let row = &mut features[(first + frame) * layout.dimensions..][..layout.dimensions];
			layout.measure(settings, &samples, frame, rate, row);
		}
		pickable.extend((0..frames).map(|frame| frame < clip_pickable));
		motion_clips.push(MotionClip {
			id: id.clone(),
			frames: frames as u32,
			pickable: clip_pickable as u32,
		});
	}
	if !pickable.iter().any(|&pickable| pickable) {
		return Err(DatabaseError::NoPickableFrame);
	}

	let (mean, scale) = layout.normalization(&features, &pickable);
	for row in features.chunks_exact_mut(layout.dimensions) {
		for ((value, mean), scale) in row.iter_mut().zip(&mean).zip(&scale) {
			*value = (*value - mean) * scale;
		}
	}

	Ok(MotionDatabase {
		clips: motion_clips,
		features,
		ground,
		search: MotionSearch {
			sample_rate: rate,
			root: settings.root.node.clone(),
			nodes,
			trajectory_times: settings.trajectory.times.clone(),
			trajectory_position: settings.trajectory.position > 0.0,
			trajectory_direction: settings.trajectory.direction > 0.0,
			mean,
			scale,
			interval: settings.search.interval,
			exclusion: settings.search.exclusion,
			blend: settings.search.blend,
			velocity_halflife: settings.halflife.velocity,
			facing_halflife: settings.halflife.facing,
		},
	})
}

/// Returns how many frames a clip of `duration` seconds has at `rate`: one per frame interval from zero up to its
/// duration.
fn frame_count(duration: f32, rate: f32) -> usize {
	// The tolerance keeps a duration that is a whole number of frames from losing its last frame to rounding.
	(duration * rate + 1.0e-4).floor() as usize + 1
}

/// Returns how many of a clip's first frames have the whole trajectory, up to `horizon` seconds, left in the clip.
fn pickable_count(duration: f32, rate: f32, horizon: f32) -> usize {
	if duration + 1.0e-6 < horizon {
		return 0;
	}
	frame_count(duration - horizon, rate)
}

/// Samples every frame of one clip, measuring its smoothed ground root and its feature nodes' object-space positions.
fn sample_clip(id: &str, animation: &Animation, settings: &MotionBead, nodes: &[MotionNode]) -> Result<ClipSamples, DatabaseError> {
	let skeleton = animation.skeleton.resource();
	let find = |name: &str| {
		unique_node(skeleton, name).map_err(|lookup| {
			let (clip, node) = (id.to_owned(), name.to_owned());
			match lookup {
				NodeLookup::Missing => DatabaseError::NodeNotFound { clip, node },
				NodeLookup::Duplicate => DatabaseError::DuplicateNode { clip, node },
			}
		})
	};
	let root = find(&settings.root.node)?;
	let feature_nodes = nodes.iter().map(|node| find(&node.name)).collect::<Result<Vec<_>, _>>()?;
	let rest_rotation = model_rotation(skeleton, |node| skeleton.nodes[node].rest_local.rotation, Some(root as u32));

	let rate = settings.sample_rate;
	let frames = frame_count(animation.duration, rate);
	let mut local = Vec::with_capacity(skeleton.nodes.len());
	let mut raw = Vec::with_capacity(frames);
	let mut positions = Vec::with_capacity(frames * feature_nodes.len());
	for frame in 0..frames {
		let time = (frame as f32 / rate).min(animation.duration);
		sample_local_pose(animation, time, &mut local);
		let root_transform = object_transform(skeleton, &local, root);
		raw.push(GroundRoot {
			x: root_transform.translation.x(),
			z: root_transform.translation.z(),
			yaw: animated_yaw(root_transform.rotation, rest_rotation),
		});
		positions.extend(
			feature_nodes
				.iter()
				.map(|&node| object_transform(skeleton, &local, node).translation.to_array()),
		);
	}

	Ok(ClipSamples {
		ground: smooth_ground(&raw, settings.root.position_smoothing * rate, settings.root.facing_smoothing * rate),
		positions,
	})
}

/// Smooths a clip's ground root over its frames: the position with a Gaussian whose standard deviation is
/// `position_sigma` frames, and the yaw's direction with one of `facing_sigma` frames.
///
/// Samples beyond the clip's ends repeat its first or last frame. A direction that averages out to nothing, like a
/// spin in place, keeps the frame's own yaw.
fn smooth_ground(raw: &[GroundRoot], position_sigma: f32, facing_sigma: f32) -> Vec<GroundRoot> {
	let values = |value: fn(&GroundRoot) -> f32| raw.iter().map(value).collect::<Vec<_>>();
	let x = gaussian(&values(|root| root.x), position_sigma);
	let z = gaussian(&values(|root| root.z), position_sigma);
	let forward_x = gaussian(&values(|root| -root.yaw.sin()), facing_sigma);
	let forward_z = gaussian(&values(|root| -root.yaw.cos()), facing_sigma);
	raw.iter()
		.enumerate()
		.map(|(frame, root)| {
			let (dx, dz) = (forward_x[frame], forward_z[frame]);
			GroundRoot {
				x: x[frame],
				z: z[frame],
				yaw: if dx * dx + dz * dz > 1.0e-12 {
					yaw_of_direction(dx, dz)
				} else {
					root.yaw
				},
			}
		})
		.collect()
}

/// Convolves one value of every frame with a normalized Gaussian of `sigma` frames, cut off at three deviations, and
/// repeats the end frames beyond the clip. A zero `sigma` returns the values as they are.
fn gaussian(values: &[f32], sigma: f32) -> Vec<f32> {
	if sigma <= 0.0 {
		return values.to_vec();
	}
	let radius = (3.0 * sigma).ceil() as isize;
	let weights: Vec<f32> = (-radius..=radius)
		.map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
		.collect();
	let total: f32 = weights.iter().sum();
	let last = values.len() as isize - 1;
	(0..values.len() as isize)
		.map(|frame| {
			weights
				.iter()
				.zip(-radius..=radius)
				.map(|(weight, offset)| weight * values[(frame + offset).clamp(0, last) as usize])
				.sum::<f32>()
				/ total
		})
		.collect()
}

/// The `Layout` struct places each feature group's dimensions in a frame's feature vector: every node's position and
/// velocity in node order, then the trajectory positions, then the trajectory directions.
struct Layout {
	/// Each group's first dimension, dimension count, and weight, in vector order.
	groups: Vec<(usize, usize, f32)>,
	dimensions: usize,
}

impl Layout {
	/// Lays out the groups whose weights are positive.
	fn new(settings: &MotionBead) -> Self {
		let mut groups = Vec::new();
		let mut dimensions = 0;
		let mut push = |count: usize, weight: f32| {
			if weight > 0.0 {
				groups.push((dimensions, count, weight));
				dimensions += count;
			}
		};
		for weights in settings.nodes.values() {
			push(3, weights.position);
			push(3, weights.velocity);
		}
		let times = settings.trajectory.times.len();
		push(2 * times, settings.trajectory.position);
		push(2 * times, settings.trajectory.direction);
		Self { groups, dimensions }
	}

	/// Writes frame `frame`'s raw features into `row`, in this layout's order.
	fn measure(&self, settings: &MotionBead, samples: &ClipSamples, frame: usize, rate: f32, row: &mut [f32]) {
		let node_count = settings.nodes.len();
		let frames = samples.ground.len();
		let root = samples.ground[frame];
		let position = |frame: usize, node: usize| samples.positions[frame * node_count + node];
		let mut output = row.iter_mut();
		let mut write = |values: &[f32]| {
			for value in values {
				*output.next().expect("the layout sizes the row") = *value;
			}
		};
		for (node, weights) in settings.nodes.values().enumerate() {
			if weights.position > 0.0 {
				let [x, y, z] = position(frame, node);
				let (x, z) = root.to_root_frame(x, z);
				write(&[x, y, z]);
			}
			if weights.velocity > 0.0 {
				// The first frame has no frame before it, so it measures toward the frame after it.
				let (earlier, later) = match frame {
					0 if frames > 1 => (0, 1),
					0 => (0, 0),
					frame => (frame - 1, frame),
				};
				let ([x0, y0, z0], [x1, y1, z1]) = (position(earlier, node), position(later, node));
				let (x, z) = turn_back(root.yaw, (x1 - x0) * rate, (z1 - z0) * rate);
				write(&[x, (y1 - y0) * rate, z]);
			}
		}
		let time = frame as f32 / rate;
		let ahead = |offset: f32| ground_at(rate, frames, time + offset, |frame| samples.ground[frame]);
		if settings.trajectory.position > 0.0 {
			for &offset in &settings.trajectory.times {
				let future = ahead(offset);
				let (x, z) = root.to_root_frame(future.x, future.z);
				write(&[x, z]);
			}
		}
		if settings.trajectory.direction > 0.0 {
			for &offset in &settings.trajectory.times {
				let [x, z] = super::direction(ahead(offset).yaw - root.yaw);
				write(&[x, z]);
			}
		}
	}

	/// Returns each dimension's mean over the pickable frames, and its group's weight divided by the group's deviation:
	/// the mean of its dimensions' standard deviations, with a deviation of zero counting as one.
	fn normalization(&self, features: &[f32], pickable: &[bool]) -> (Vec<f32>, Vec<f32>) {
		let rows = || {
			features
				.chunks_exact(self.dimensions)
				.zip(pickable)
				.filter(|(_, pickable)| **pickable)
				.map(|(row, _)| row)
		};
		let count = rows().count() as f64;
		let mut mean = vec![0.0f64; self.dimensions];
		for row in rows() {
			for (sum, value) in mean.iter_mut().zip(row) {
				*sum += f64::from(*value);
			}
		}
		mean.iter_mut().for_each(|sum| *sum /= count);
		let mut variance = vec![0.0f64; self.dimensions];
		for row in rows() {
			for ((sum, value), mean) in variance.iter_mut().zip(row).zip(&mean) {
				let difference = f64::from(*value) - mean;
				*sum += difference * difference;
			}
		}
		let mut scale = vec![0.0f32; self.dimensions];
		for &(first, count_of_group, weight) in &self.groups {
			let deviation = variance[first..first + count_of_group]
				.iter()
				.map(|sum| (sum / count).sqrt())
				.sum::<f64>()
				/ count_of_group as f64;
			let deviation = if deviation > 0.0 { deviation } else { 1.0 };
			scale[first..first + count_of_group].fill((f64::from(weight) / deviation) as f32);
		}
		(mean.into_iter().map(|mean| mean as f32).collect(), scale)
	}
}
