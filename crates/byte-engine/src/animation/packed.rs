use resource_management::{
	Reference,
	resources::{
		animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
};

use super::math::{hermite, nlerp_quaternion, normalize_quaternion};

const NONE: u32 = u32::MAX;
const HEADER_WORDS: usize = 8;
const TRACK_WORDS: usize = 4;
const CURVE_WORDS: usize = 4;

#[derive(Clone, Copy)]
enum CurveInterpolation {
	Step = 0,
	Linear = 1,
	CubicSpline = 2,
}

#[derive(Clone, Copy)]
struct CurveDescriptor {
	interpolation: CurveInterpolation,
	key_start: u32,
	value_start: u32,
	key_count: u32,
}

struct TrackDescriptor {
	node: u32,
	translation: Option<u32>,
	rotation: Option<u32>,
	scale: Option<u32>,
}

/// The `PackedAnimationData` struct stages one packed clip before the animation pool copies it into its arena.
#[derive(Debug)]
pub(crate) struct PackedAnimationData {
	pub(crate) skeleton: Reference<Skeleton>,
	pub(crate) data: Box<[u32]>,
}

impl PackedAnimationData {
	/// Returns the exact arena bytes needed after packing without allocating staging arrays.
	pub(crate) fn resident_bytes(animation: &Animation) -> usize {
		let curve_count = animation
			.tracks
			.iter()
			.map(|track| {
				usize::from(track.translation.is_some())
					+ usize::from(track.rotation.is_some())
					+ usize::from(track.scale.is_some())
			})
			.sum::<usize>();
		let key_words = animation.tracks.iter().fold(0usize, |total, track| {
			total
				+ track.translation.as_ref().map_or(0, vector3_curve_words)
				+ track.rotation.as_ref().map_or(0, quaternion_curve_words)
				+ track.scale.as_ref().map_or(0, vector3_curve_words)
		});
		(HEADER_WORDS + animation.tracks.len() * TRACK_WORDS + curve_count * CURVE_WORDS + key_words)
			* std::mem::size_of::<u32>()
	}

	/// Consumes a loaded resource and combines all curve descriptors, times, and values into one allocation.
	pub(crate) fn from_resource(animation: Animation) -> Self {
		let expected_bytes = Self::resident_bytes(&animation);
		let Animation {
			name: _,
			skeleton,
			duration,
			tracks,
		} = animation;
		let data = pack_data(duration, tracks);
		debug_assert_eq!(data.len() * std::mem::size_of::<u32>(), expected_bytes);
		Self { skeleton, data }
	}
}

/// The `PackedAnimation` struct provides a borrowing interface over one packed CPU animation buffer.
///
/// Resident evaluation leases create this view over their pinned arena range.
/// It contains no owned storage and is cheap to recreate for each sample.
#[derive(Clone, Copy)]
pub struct PackedAnimation<'a> {
	words: &'a [u32],
}

impl<'a> PackedAnimation<'a> {
	pub(crate) fn from_words(words: &'a [u32]) -> Self {
		Self { words }
	}

	/// Returns the clip duration encoded in the packed buffer header.
	pub fn duration(self) -> f32 {
		f32::from_bits(self.words[0])
	}

	/// Samples the clip into a complete source-skeleton local pose while reusing caller storage.
	pub fn sample_local_pose(self, skeleton: &Skeleton, time: f32, output: &mut Vec<LocalTransform>) {
		output.clear();
		output.extend(skeleton.nodes.iter().map(|node| node.rest_local));
		for track_index in 0..self.track_count() {
			let track = self.track(track_index);
			let node = track.node as usize;
			self.sample_track(track, time, &mut output[node]);
		}
	}

	/// Samples directly into a mapped target pose, avoiding a transient complete source pose.
	pub(crate) fn sample_target_local_pose(self, pose_map: &SkeletonPoseMap, time: f32, output: &mut [LocalTransform]) {
		output.copy_from_slice(pose_map.target_rest_pose());
		for track_index in 0..self.track_count() {
			let track = self.track(track_index);
			let Some(target) = pose_map.direct_target_node(track.node as usize) else {
				continue;
			};
			self.sample_track(track, time, &mut output[target]);
		}
	}

	/// Applies the sampled channels from one packed track to a local transform.
	fn sample_track(self, track: PackedTrack, time: f32, local: &mut LocalTransform) {
		if let Some(curve) = track.translation {
			local.translation = self.sample_vector3(curve, time);
		}
		if let Some(curve) = track.rotation {
			local.rotation = self.sample_rotation(curve, time);
		}
		if let Some(curve) = track.scale {
			local.scale = self.sample_vector3(curve, time);
		}
	}

	fn track_count(self) -> usize {
		self.words[1] as usize
	}

	fn track(self, index: usize) -> PackedTrack {
		let start = self.words[2] as usize + index * TRACK_WORDS;
		PackedTrack {
			node: self.words[start],
			translation: self.optional_curve(self.words[start + 1]),
			rotation: self.optional_curve(self.words[start + 2]),
			scale: self.optional_curve(self.words[start + 3]),
		}
	}

	fn optional_curve(self, index: u32) -> Option<PackedCurve> {
		(index != NONE).then(|| {
			let start = self.words[3] as usize + index as usize * CURVE_WORDS;
			PackedCurve {
				interpolation: match self.words[start] {
					0 => CurveInterpolation::Step,
					1 => CurveInterpolation::Linear,
					2 => CurveInterpolation::CubicSpline,
					_ => unreachable!("packed curves are produced only by the engine encoder"),
				},
				key_start: self.words[start + 1] as usize,
				value_start: self.words[start + 2] as usize,
				key_count: self.words[start + 3] as usize,
			}
		})
	}

	fn time(self, curve: PackedCurve, key: usize) -> f32 {
		f32::from_bits(self.words[self.words[4] as usize + curve.key_start + key])
	}

	fn vector3(self, index: usize) -> [f32; 3] {
		let start = self.words[5] as usize + index * 3;
		std::array::from_fn(|component| f32::from_bits(self.words[start + component]))
	}

	fn quaternion(self, index: usize) -> [f32; 4] {
		let start = self.words[6] as usize + index * 4;
		std::array::from_fn(|component| f32::from_bits(self.words[start + component]))
	}

	fn sample_vector3(self, curve: PackedCurve, time: f32) -> [f32; 3] {
		match curve.interpolation {
			CurveInterpolation::Step => self.vector3(curve.value_start + self.step_key(curve, time)),
			CurveInterpolation::Linear => {
				let (lower, upper, factor, _) = self.interpolation_segment(curve, time);
				let lower = self.vector3(curve.value_start + lower);
				let upper = self.vector3(curve.value_start + upper);
				std::array::from_fn(|component| lower[component] + (upper[component] - lower[component]) * factor)
			}
			CurveInterpolation::CubicSpline => {
				let (lower, upper, factor, span) = self.interpolation_segment(curve, time);
				hermite(
					self.vector3(curve.value_start + lower * 3),
					self.vector3(curve.value_start + lower * 3 + 2),
					self.vector3(curve.value_start + upper * 3),
					self.vector3(curve.value_start + upper * 3 + 1),
					factor,
					span,
				)
			}
		}
	}

	fn sample_rotation(self, curve: PackedCurve, time: f32) -> [f32; 4] {
		match curve.interpolation {
			CurveInterpolation::Step => self.quaternion(curve.value_start + self.step_key(curve, time)),
			CurveInterpolation::Linear => {
				let (lower, upper, factor, _) = self.interpolation_segment(curve, time);
				nlerp_quaternion(
					self.quaternion(curve.value_start + lower),
					self.quaternion(curve.value_start + upper),
					factor,
				)
			}
			CurveInterpolation::CubicSpline => {
				let (lower, upper, factor, span) = self.interpolation_segment(curve, time);
				normalize_quaternion(hermite(
					self.quaternion(curve.value_start + lower * 3),
					self.quaternion(curve.value_start + lower * 3 + 2),
					self.quaternion(curve.value_start + upper * 3),
					self.quaternion(curve.value_start + upper * 3 + 1),
					factor,
					span,
				))
			}
		}
	}

	fn step_key(self, curve: PackedCurve, time: f32) -> usize {
		self.upper_key(curve, time).saturating_sub(1)
	}

	/// Finds the first key after `time` without materializing a typed time slice from the packed words.
	fn upper_key(self, curve: PackedCurve, time: f32) -> usize {
		let mut lower = 0;
		let mut upper = curve.key_count;
		while lower < upper {
			let middle = lower + (upper - lower) / 2;
			if self.time(curve, middle) <= time {
				lower = middle + 1;
			} else {
				upper = middle;
			}
		}
		lower
	}

	fn interpolation_segment(self, curve: PackedCurve, time: f32) -> (usize, usize, f32, f32) {
		let upper = self.upper_key(curve, time).min(curve.key_count.saturating_sub(1));
		let lower = upper.saturating_sub(1);
		let span = self.time(curve, upper) - self.time(curve, lower);
		let factor = if span > 0.0 {
			(time - self.time(curve, lower)) / span
		} else {
			0.0
		}
		.clamp(0.0, 1.0);
		(lower, upper, factor, span)
	}
}

#[derive(Clone, Copy)]
struct PackedCurve {
	interpolation: CurveInterpolation,
	key_start: usize,
	value_start: usize,
	key_count: usize,
}

struct PackedTrack {
	node: u32,
	translation: Option<PackedCurve>,
	rotation: Option<PackedCurve>,
	scale: Option<PackedCurve>,
}

/// Builds transient typed arrays, then writes the retained representation into one word-aligned allocation.
fn pack_data(duration: f32, tracks: Vec<NodeTrack>) -> Box<[u32]> {
	let mut descriptors = Vec::new();
	let mut packed_tracks = Vec::with_capacity(tracks.len());
	let mut times = Vec::new();
	let mut vector3_values = Vec::new();
	let mut quaternion_values = Vec::new();

	for track in tracks {
		let translation = track
			.translation
			.map(|curve| pack_vector3_curve(curve, &mut descriptors, &mut times, &mut vector3_values));
		let rotation = track
			.rotation
			.map(|curve| pack_quaternion_curve(curve, &mut descriptors, &mut times, &mut quaternion_values));
		let scale = track
			.scale
			.map(|curve| pack_vector3_curve(curve, &mut descriptors, &mut times, &mut vector3_values));
		packed_tracks.push(TrackDescriptor {
			node: track.node,
			translation,
			rotation,
			scale,
		});
	}

	let tracks_offset = HEADER_WORDS;
	let curves_offset = tracks_offset + packed_tracks.len() * TRACK_WORDS;
	let times_offset = curves_offset + descriptors.len() * CURVE_WORDS;
	let vector3_offset = times_offset + times.len();
	let quaternion_offset = vector3_offset + vector3_values.len() * 3;
	let total_words = quaternion_offset + quaternion_values.len() * 4;
	let mut words = Vec::with_capacity(total_words);
	words.extend([
		duration.to_bits(),
		packed_tracks.len() as u32,
		tracks_offset as u32,
		curves_offset as u32,
		times_offset as u32,
		vector3_offset as u32,
		quaternion_offset as u32,
		0,
	]);
	for track in packed_tracks {
		words.extend([
			track.node,
			track.translation.unwrap_or(NONE),
			track.rotation.unwrap_or(NONE),
			track.scale.unwrap_or(NONE),
		]);
	}
	for curve in descriptors {
		words.extend([
			curve.interpolation as u32,
			curve.key_start,
			curve.value_start,
			curve.key_count,
		]);
	}
	words.extend(times.into_iter().map(f32::to_bits));
	words.extend(vector3_values.into_iter().flatten().map(f32::to_bits));
	words.extend(quaternion_values.into_iter().flatten().map(f32::to_bits));
	words.into_boxed_slice()
}

fn vector3_curve_words(curve: &Vector3Curve) -> usize {
	match curve {
		Vector3Curve::Step { times, .. } | Vector3Curve::Linear { times, .. } => times.len() * 4,
		Vector3Curve::CubicSpline { times, .. } => times.len() * 10,
	}
}

fn quaternion_curve_words(curve: &QuaternionCurve) -> usize {
	match curve {
		QuaternionCurve::Step { times, .. } | QuaternionCurve::Linear { times, .. } => times.len() * 5,
		QuaternionCurve::CubicSpline { times, .. } => times.len() * 13,
	}
}

fn pack_vector3_curve(
	curve: Vector3Curve,
	descriptors: &mut Vec<CurveDescriptor>,
	times: &mut Vec<f32>,
	values: &mut Vec<[f32; 3]>,
) -> u32 {
	let (interpolation, curve_times, curve_values) = match curve {
		Vector3Curve::Step { times, values } => (CurveInterpolation::Step, times, values),
		Vector3Curve::Linear { times, values } => (CurveInterpolation::Linear, times, values),
		Vector3Curve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			let mut packed = Vec::with_capacity(values.len() * 3);
			for ((value, incoming), outgoing) in values.into_iter().zip(in_tangents).zip(out_tangents) {
				packed.extend([value, incoming, outgoing]);
			}
			(CurveInterpolation::CubicSpline, times, packed)
		}
	};
	push_curve(interpolation, curve_times, curve_values, descriptors, times, values)
}

fn pack_quaternion_curve(
	curve: QuaternionCurve,
	descriptors: &mut Vec<CurveDescriptor>,
	times: &mut Vec<f32>,
	values: &mut Vec<[f32; 4]>,
) -> u32 {
	let (interpolation, curve_times, curve_values) = match curve {
		QuaternionCurve::Step { times, values } => (CurveInterpolation::Step, times, values),
		QuaternionCurve::Linear { times, values } => (CurveInterpolation::Linear, times, values),
		QuaternionCurve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			let mut packed = Vec::with_capacity(values.len() * 3);
			for ((value, incoming), outgoing) in values.into_iter().zip(in_tangents).zip(out_tangents) {
				packed.extend([value, incoming, outgoing]);
			}
			(CurveInterpolation::CubicSpline, times, packed)
		}
	};
	push_curve(interpolation, curve_times, curve_values, descriptors, times, values)
}

fn push_curve<T>(
	interpolation: CurveInterpolation,
	curve_times: Vec<f32>,
	curve_values: Vec<T>,
	descriptors: &mut Vec<CurveDescriptor>,
	times: &mut Vec<f32>,
	values: &mut Vec<T>,
) -> u32 {
	let index = descriptors.len() as u32;
	descriptors.push(CurveDescriptor {
		interpolation,
		key_start: times.len() as u32,
		value_start: values.len() as u32,
		key_count: curve_times.len() as u32,
	});
	times.extend(curve_times);
	values.extend(curve_values);
	index
}

#[cfg(test)]
mod tests {
	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode, SkeletonPoseMap},
		},
	};

	use super::{PackedAnimation, PackedAnimationData};

	#[test]
	fn direct_sampling_preserves_the_last_duplicate_source_node() {
		let source = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform {
						translation: [1.0, 0.0, 0.0],
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform {
						translation: [2.0, 0.0, 0.0],
						..LocalTransform::identity()
					},
				},
			],
		};
		let target = Skeleton {
			nodes: vec![SkeletonNode {
				name: Some("Hips".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		};
		let animation = Animation {
			name: None,
			skeleton: Reference::in_memory("duplicate-source.skeleton", source),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(Vector3Curve::Step {
					times: vec![0.0],
					values: vec![[3.0, 0.0, 0.0]],
				}),
				rotation: None,
				scale: None,
			}],
		};
		let packed = PackedAnimationData::from_resource(animation);
		let map = SkeletonPoseMap::by_name(packed.skeleton.resource(), &target);
		let mut output = [LocalTransform::identity()];

		PackedAnimation::from_words(&packed.data).sample_target_local_pose(&map, 0.0, &mut output);

		assert_eq!(output[0].translation, [2.0, 0.0, 0.0]);
	}
}
