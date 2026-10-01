use resource_management::{
	Reference,
	resources::{
		animation::{Animation, Curve, NodeTrack},
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
};

use super::math::{CurveComponents, CurveInterpolation, CurveValue, sample_curve};

const NONE: u32 = u32::MAX;
const HEADER_WORDS: usize = 8;
const TRACK_WORDS: usize = 4;
const CURVE_WORDS: usize = 4;

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
				+ track.translation.as_ref().map_or(0, curve_words::<3, _, _>)
				+ track.rotation.as_ref().map_or(0, curve_words::<4, _, _>)
				+ track.scale.as_ref().map_or(0, curve_words::<3, _, _>)
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
			local.translation = self.sample(curve, time, Self::vector3);
		}
		if let Some(curve) = track.rotation {
			local.rotation = self.sample(curve, time, Self::quaternion);
		}
		if let Some(curve) = track.scale {
			local.scale = self.sample(curve, time, Self::vector3);
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

	/// Samples one packed curve whose values `read` decodes from the value array.
	///
	/// Cubic curves store each key as `[value, in_tangent, out_tangent]`; other curves store one value per key.
	fn sample<V: CurveValue<N>, const N: usize>(
		self,
		curve: PackedCurve,
		time: f32,
		read: impl Fn(Self, usize) -> [f32; N],
	) -> V {
		let stride = if curve.interpolation == CurveInterpolation::CubicSpline {
			3
		} else {
			1
		};
		let value_index = |key: usize| curve.value_start + key * stride;
		sample_curve(
			curve.interpolation,
			curve.key_count,
			time,
			|key| self.time(curve, key),
			|key| V::from_components(read(self, value_index(key))),
			|key| (read(self, value_index(key) + 1), read(self, value_index(key) + 2)),
		)
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
			.map(|curve| pack_curve(curve, &mut descriptors, &mut times, &mut vector3_values));
		let rotation = track
			.rotation
			.map(|curve| pack_curve(curve, &mut descriptors, &mut times, &mut quaternion_values));
		let scale = track
			.scale
			.map(|curve| pack_curve(curve, &mut descriptors, &mut times, &mut vector3_values));
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

/// Counts the packed words one curve adds: a time plus every value of each key, where cubic keys also carry two tangents.
fn curve_words<const N: usize, V, T>(curve: &Curve<V, T>) -> usize {
	match curve {
		Curve::Step { times, .. } | Curve::Linear { times, .. } => times.len() * (1 + N),
		Curve::CubicSpline { times, .. } => times.len() * (1 + 3 * N),
	}
}

/// Appends one curve to the packed tables and returns its descriptor index.
///
/// Values and tangents are stored as their components. Cubic keys are stored as consecutive
/// `[value, in_tangent, out_tangent]` triples.
fn pack_curve<V: CurveComponents<N>, T: CurveComponents<N>, const N: usize>(
	curve: Curve<V, T>,
	descriptors: &mut Vec<CurveDescriptor>,
	times: &mut Vec<f32>,
	values: &mut Vec<[f32; N]>,
) -> u32 {
	let (interpolation, curve_times, curve_values) = match curve {
		Curve::Step { times, values } => (
			CurveInterpolation::Step,
			times,
			values.into_iter().map(CurveComponents::components).collect(),
		),
		Curve::Linear { times, values } => (
			CurveInterpolation::Linear,
			times,
			values.into_iter().map(CurveComponents::components).collect(),
		),
		Curve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			let mut packed = Vec::with_capacity(values.len() * 3);
			for ((key, incoming), outgoing) in values.into_iter().zip(in_tangents).zip(out_tangents) {
				packed.extend([key.components(), incoming.components(), outgoing.components()]);
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
			animation::{Animation, NodeTrack, TranslationCurve},
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
						translation: math::Vector::from_array([1.0, 0.0, 0.0]),
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform {
						translation: math::Vector::from_array([2.0, 0.0, 0.0]),
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
				translation: Some(TranslationCurve::Step {
					times: vec![0.0],
					values: vec![math::Vector::from_array([3.0, 0.0, 0.0])],
				}),
				rotation: None,
				scale: None,
			}],
		};
		let packed = PackedAnimationData::from_resource(animation);
		let map = SkeletonPoseMap::by_name(packed.skeleton.resource(), &target);
		let mut output = [LocalTransform::identity()];

		PackedAnimation::from_words(&packed.data).sample_target_local_pose(&map, 0.0, &mut output);

		assert_eq!(output[0].translation, math::Vector::new(2.0, 0.0, 0.0));
	}
}
