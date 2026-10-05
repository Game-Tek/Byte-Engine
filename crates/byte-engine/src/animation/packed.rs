use resource_management::{
	Reference,
	resources::{
		animation::{Animation, Curve, CurveComponents, NodeTrack},
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
};

use super::math::{CurveInterpolation, CurveValue, sample_curve};

const NONE: u32 = u32::MAX;
const HEADER_WORDS: usize = 8;
/// The header word that holds the offset of the three-component value table, used by translation and scale curves.
const VECTOR3_VALUES_WORD: usize = 5;
/// The header word that holds the offset of the four-component value table, used by rotation curves.
const QUATERNION_VALUES_WORD: usize = 6;
const TRACK_WORDS: usize = 4;
const CURVE_WORDS: usize = 4;

/// The `PackedAnimationData` struct stages one packed clip before the animation pool copies it into its arena.
///
/// The load worker packs each clip, so the thread that admits it into the arena only copies its words.
#[derive(Debug)]
pub(crate) struct PackedAnimationData {
	pub(crate) skeleton: Reference<Skeleton>,
	pub(crate) data: Box<[u32]>,
}

impl PackedAnimationData {
	/// Returns the arena bytes this packed clip occupies.
	pub(crate) fn resident_bytes(&self) -> usize {
		self.data.len() * std::mem::size_of::<u32>()
	}

	/// Consumes a loaded resource and combines all curve descriptors, times, and values into one allocation.
	pub(crate) fn from_resource(animation: Animation) -> Self {
		Self {
			data: pack_data(animation.duration, animation.tracks),
			skeleton: animation.skeleton,
		}
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
		for &[node, curves @ ..] in self.tracks() {
			self.sample_track(curves, time, &mut output[node as usize]);
		}
	}

	/// Samples directly into a mapped target pose, avoiding a transient complete source pose.
	pub(crate) fn sample_target_local_pose(self, pose_map: &SkeletonPoseMap, time: f32, output: &mut [LocalTransform]) {
		output.copy_from_slice(pose_map.target_rest_pose());
		for &[node, curves @ ..] in self.tracks() {
			// Skip tracks without a target before decoding any of their curves.
			let Some(target) = pose_map.direct_target_node(node as usize) else {
				continue;
			};
			self.sample_track(curves, time, &mut output[target]);
		}
	}

	/// Applies the sampled channels of one packed track, given its translation, rotation, and scale curve indices.
	fn sample_track(self, [translation, rotation, scale]: [u32; 3], time: f32, local: &mut LocalTransform) {
		if let Some(curve) = self.optional_curve(translation) {
			local.translation = self.sample(curve, time, VECTOR3_VALUES_WORD);
		}
		if let Some(curve) = self.optional_curve(rotation) {
			local.rotation = self.sample(curve, time, QUATERNION_VALUES_WORD);
		}
		if let Some(curve) = self.optional_curve(scale) {
			local.scale = self.sample(curve, time, VECTOR3_VALUES_WORD);
		}
	}

	/// Returns each track's `[node, translation, rotation, scale]` words.
	fn tracks(self) -> &'a [[u32; TRACK_WORDS]] {
		let start = self.words[2] as usize;
		self.words[start..][..self.words[1] as usize * TRACK_WORDS].as_chunks().0
	}

	fn optional_curve(self, index: u32) -> Option<PackedCurve> {
		(index != NONE).then(|| {
			let [interpolation, key_start, value_start, key_count] =
				self.words[self.words[3] as usize..].as_chunks::<CURVE_WORDS>().0[index as usize];
			PackedCurve {
				interpolation: match interpolation {
					0 => CurveInterpolation::Step,
					1 => CurveInterpolation::Linear,
					2 => CurveInterpolation::CubicSpline,
					_ => unreachable!("packed curves are produced only by the engine encoder"),
				},
				key_start: key_start as usize,
				value_start: value_start as usize,
				key_count: key_count as usize,
			}
		})
	}

	/// Samples one packed curve whose `N`-component values start at the offset in header word `table`.
	///
	/// Cubic curves store each key as `[value, in_tangent, out_tangent]`; other curves store one value per key.
	fn sample<V: CurveValue<N>, const N: usize>(self, curve: PackedCurve, time: f32, table: usize) -> V {
		// Slice both tables once, so each key read is one bounds-checked load instead of one per word.
		let times = &self.words[self.words[4] as usize + curve.key_start..];
		let values = &self.words[self.words[table] as usize..].as_chunks::<N>().0[curve.value_start..];
		let read = |index: usize| values[index].map(f32::from_bits);
		let stride = if curve.interpolation == CurveInterpolation::CubicSpline {
			3
		} else {
			1
		};
		sample_curve(
			curve.interpolation,
			curve.key_count,
			time,
			|key| f32::from_bits(times[key]),
			|key| V::from_key(read(key * stride)),
			|key| (read(key * stride + 1), read(key * stride + 2)),
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

/// Builds transient typed arrays, then writes the retained representation into one word-aligned allocation.
fn pack_data(duration: f32, tracks: Vec<NodeTrack>) -> Box<[u32]> {
	let mut descriptors = Vec::new();
	let mut packed_tracks = Vec::with_capacity(tracks.len());
	let mut times = Vec::new();
	let mut vector3_values = Vec::new();
	let mut quaternion_values = Vec::new();

	for track in tracks {
		// Array elements evaluate left to right, so curve descriptors keep the translation, rotation, scale order.
		packed_tracks.push([
			track.node,
			track.translation.map_or(NONE, |curve| {
				pack_curve(curve, &mut descriptors, &mut times, &mut vector3_values)
			}),
			track.rotation.map_or(NONE, |curve| {
				pack_curve(curve, &mut descriptors, &mut times, &mut quaternion_values)
			}),
			track.scale.map_or(NONE, |curve| {
				pack_curve(curve, &mut descriptors, &mut times, &mut vector3_values)
			}),
		]);
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
	words.extend(packed_tracks.into_iter().flatten());
	words.extend(descriptors.into_iter().flatten());
	words.extend(times.into_iter().map(f32::to_bits));
	words.extend(vector3_values.into_iter().flatten().map(f32::to_bits));
	words.extend(quaternion_values.into_iter().flatten().map(f32::to_bits));
	words.into_boxed_slice()
}

/// Appends one curve to the packed tables and returns its descriptor index.
///
/// Values and tangents are stored as their components. Values are stored as [`CurveValue::from_components`] would
/// rebuild them, so sampling reads them back with [`CurveValue::from_key`] and normalizes each rotation only once.
/// Cubic keys are stored as consecutive `[value, in_tangent, out_tangent]` triples.
fn pack_curve<V: CurveValue<N>, T: CurveComponents<N>, const N: usize>(
	curve: Curve<V, T>,
	descriptors: &mut Vec<[u32; CURVE_WORDS]>,
	times: &mut Vec<f32>,
	values: &mut Vec<[f32; N]>,
) -> u32 {
	let index = descriptors.len() as u32;
	let value_start = values.len() as u32;
	let (interpolation, curve_times) = match curve {
		Curve::Step { times, values: keys } => {
			values.extend(keys.into_iter().map(stored_key));
			(CurveInterpolation::Step, times)
		}
		Curve::Linear { times, values: keys } => {
			values.extend(keys.into_iter().map(stored_key));
			(CurveInterpolation::Linear, times)
		}
		Curve::CubicSpline {
			times,
			values: keys,
			in_tangents,
			out_tangents,
		} => {
			for ((key, incoming), outgoing) in keys.into_iter().zip(in_tangents).zip(out_tangents) {
				values.extend([stored_key(key), incoming.components(), outgoing.components()]);
			}
			(CurveInterpolation::CubicSpline, times)
		}
	};
	descriptors.push([
		interpolation as u32,
		times.len() as u32,
		value_start,
		curve_times.len() as u32,
	]);
	times.extend(curve_times);
	index
}

/// Returns the components that sampling would rebuild from `key`, which normalizes rotations ahead of time.
fn stored_key<V: CurveValue<N>, const N: usize>(key: V) -> [f32; N] {
	V::from_components(key.components()).components()
}

#[cfg(test)]
mod tests {
	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, TranslationCurve},
			skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
		},
	};

	use super::{PackedAnimation, PackedAnimationData};
	use crate::animation::test_node;

	#[test]
	fn direct_sampling_preserves_the_last_duplicate_source_node() {
		let rest = |x| LocalTransform {
			translation: math::Vector::from_array([x, 0.0, 0.0]),
			..LocalTransform::identity()
		};
		let source = Skeleton {
			nodes: vec![
				test_node(Some("Hips"), None, rest(1.0)),
				test_node(Some("Hips"), None, rest(2.0)),
			],
		};
		let target = Skeleton {
			nodes: vec![test_node(Some("Hips"), None, LocalTransform::identity())],
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
