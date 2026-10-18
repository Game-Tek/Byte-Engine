//! Packed clips laid out for the way a pose is sampled: every track of a clip at one time.
//!
//! Curves keyed at the same times share one timeline, so a pose searches each timeline's keys once instead of each
//! curve's. A timeline stores its values key by key, with every curve's value at a key side by side, so sampling a
//! pose reads two rows of adjacent words: the keys before and after the time.

use resource_management::{
	Reference,
	resources::{
		animation::{Animation, Curve, CurveComponents, NodeTrack},
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
};

use super::math::{CurveInterpolation, CurveValue, KeySpan, interpolate};

/// Marks a track channel without a curve.
const NONE: u32 = u32::MAX;
const HEADER_WORDS: usize = 6;
/// Header words: duration, frame rate or zero, track count, track table offset, timeline count, timeline table offset.
const DURATION_WORD: usize = 0;
const FRAME_RATE_WORD: usize = 1;
const TRACK_COUNT_WORD: usize = 2;
const TRACKS_WORD: usize = 3;
const TIMELINE_COUNT_WORD: usize = 4;
const TIMELINES_WORD: usize = 5;
/// A track is its node, then a `(channel, offset)` pair each for translation, rotation, and scale. A channel is
/// [`NONE`] or its timeline shifted left by two bits over its [`CurveInterpolation`]; the offset is where the curve's
/// words start within each row of that timeline.
const TRACK_WORDS: usize = 7;
/// A timeline is its key count, where its key times start, where its rows start, and how many words a row holds.
const TIMELINE_WORDS: usize = 4;
/// Timelines a pose sample places on the stack. A clip with more places each curve's time on its own.
const STACK_TIMELINES: usize = 16;

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

	/// Consumes a loaded resource and combines its tracks, timelines, and values into one allocation.
	pub(crate) fn from_resource(animation: Animation) -> Self {
		Self {
			data: pack_data(animation.duration, animation.frame_rate, animation.tracks),
			skeleton: animation.skeleton,
		}
	}
}

/// The `PackedAnimation` struct provides a borrowing interface over one packed CPU animation buffer.
///
/// The animation pool creates this view over a resident clip's arena range. It contains no owned storage and is
/// cheap to recreate for each sample.
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
		f32::from_bits(self.words[DURATION_WORD])
	}

	/// Returns the frames per second the clip was authored at, if its source recorded one.
	pub fn frame_rate(self) -> Option<f32> {
		let rate = f32::from_bits(self.words[FRAME_RATE_WORD]);
		(rate > 0.0).then_some(rate)
	}

	/// Samples every track at `time` into a pose of the skeleton `targets` maps the clip onto.
	///
	/// Each node starts from its pose in [`TrackTargets`], and every channel its track animates overwrites it.
	pub(crate) fn sample_pose(self, time: f32, targets: &TrackTargets, output: &mut [LocalTransform]) {
		output.copy_from_slice(&targets.rest);
		let (tracks, timelines) = (self.tracks(), self.timelines());
		let animated = output
			.iter_mut()
			.zip(targets.tracks.iter().copied())
			.filter(|(_, track)| *track != NONE);
		if timelines.len() > STACK_TIMELINES {
			// Too many timelines to place up front: place each curve's time as it is read.
			let mut place = |channel| self.place(&timelines[timeline_of(channel)], time);
			for (local, track) in animated {
				self.sample_track(&tracks[track as usize], local, &mut place);
			}
			return;
		}
		// Place the time once per timeline; every curve on it reads the same two rows. Most clips have one timeline,
		// so only the timelines the clip has are written.
		let placed: smallvec::SmallVec<[KeySpan; STACK_TIMELINES]> =
			timelines.iter().map(|timeline| self.place(timeline, time)).collect();
		let placed = placed.as_slice();
		let mut place = |channel| placed[timeline_of(channel)];
		for (local, track) in animated {
			self.sample_track(&tracks[track as usize], local, &mut place);
		}
	}

	/// Samples the target nodes `nodes` at `time` into `output`, one transform per node, without a whole pose.
	///
	/// Use it for the few nodes root motion measures: the root motion node and its ancestors.
	pub(crate) fn sample_nodes(self, time: f32, targets: &TrackTargets, nodes: &[u32], output: &mut [LocalTransform]) {
		let (tracks, timelines) = (self.tracks(), self.timelines());
		// The nodes' curves usually share one timeline, so the latest placement serves the next curve too.
		let mut placed: Option<(usize, KeySpan)> = None;
		let mut place = |channel| {
			let timeline = timeline_of(channel);
			match placed {
				Some((placed_timeline, keys)) if placed_timeline == timeline => keys,
				_ => {
					let keys = self.place(&timelines[timeline], time);
					placed = Some((timeline, keys));
					keys
				}
			}
		};
		for (&node, local) in nodes.iter().zip(output) {
			let node = node as usize;
			*local = targets.rest[node];
			let track = targets.tracks[node];
			if track != NONE {
				self.sample_track(&tracks[track as usize], local, &mut place);
			}
		}
	}

	/// Overwrites the channels `track` animates in `local`. `place` places the sample time on a channel's timeline.
	#[inline]
	fn sample_track(
		self,
		&[_, translation, translation_offset, rotation, rotation_offset, scale, scale_offset]: &[u32; TRACK_WORDS],
		local: &mut LocalTransform,
		place: &mut impl FnMut(u32) -> KeySpan,
	) {
		if translation != NONE {
			local.translation = self.sample_channel(translation, translation_offset, place(translation));
		}
		if rotation != NONE {
			local.rotation = self.sample_channel(rotation, rotation_offset, place(rotation));
		}
		if scale != NONE {
			local.scale = self.sample_channel(scale, scale_offset, place(scale));
		}
	}

	/// Places `time` on `timeline`: the words where the rows its curves read start, and how far between them it lies.
	fn place(self, &[key_count, times, rows, row_words]: &[u32; TIMELINE_WORDS], time: f32) -> KeySpan {
		let times = &self.words[times as usize..][..key_count as usize];
		let keys = KeySpan::find(times, time, |bits| f32::from_bits(*bits));
		let row = |key: usize| rows as usize + key * row_words as usize;
		KeySpan {
			step: row(keys.step),
			lower: row(keys.lower),
			upper: row(keys.upper),
			..keys
		}
	}

	/// Interpolates one channel between the rows `rows` placed, where the channel's curve starts `offset` words into
	/// each row.
	///
	/// Cubic curves store each key as `[value, in_tangent, out_tangent]`; other curves store one value per key.
	fn sample_channel<V: CurveValue<N>, const N: usize>(self, channel: u32, offset: u32, rows: KeySpan) -> V {
		let interpolation = match channel & 0b11 {
			0 => CurveInterpolation::Step,
			1 => CurveInterpolation::Linear,
			2 => CurveInterpolation::CubicSpline,
			_ => unreachable!("packed curves are produced only by the engine encoder"),
		};
		let offset = offset as usize;
		// A key's words for this curve, from the start of the key's row.
		let read = |row: usize, word: usize| -> [f32; N] {
			let start = row + offset + word * N;
			// One range check covers the key's words.
			let words: &[u32; N] = self.words[start..start + N]
				.try_into()
				.expect("a range of N words converts to an array of N words");
			words.map(f32::from_bits)
		};
		interpolate(
			interpolation,
			rows,
			|row| V::from_key(read(row, 0)),
			|row| (read(row, 1), read(row, 2)),
		)
	}

	/// Returns each track's words.
	fn tracks(self) -> &'a [[u32; TRACK_WORDS]] {
		let start = self.words[TRACKS_WORD] as usize;
		self.words[start..][..self.words[TRACK_COUNT_WORD] as usize * TRACK_WORDS]
			.as_chunks()
			.0
	}

	/// Returns each timeline's words.
	fn timelines(self) -> &'a [[u32; TIMELINE_WORDS]] {
		let start = self.words[TIMELINES_WORD] as usize;
		self.words[start..][..self.words[TIMELINE_COUNT_WORD] as usize * TIMELINE_WORDS]
			.as_chunks()
			.0
	}
}

/// The `TrackTargets` struct maps a packed clip's tracks onto the nodes of a target skeleton, so sampling writes the
/// target pose directly: which track animates each node, and the pose a node starts from.
///
/// Build it once for each clip and target skeleton with [`Self::by_name`], then pass it to
/// [`PackedAnimation::sample_pose`] or [`PackedAnimation::sample_nodes`].
pub(crate) struct TrackTargets {
	/// The track that animates each target node, or [`NONE`] for a node that keeps its pose from `rest`.
	tracks: Box<[u32]>,
	/// The target rest pose, with the rest transform of the source node that maps onto each matched target node.
	rest: Box<[LocalTransform]>,
}

impl TrackTargets {
	/// Maps `clip`, authored for `source`, onto `target` by node name.
	///
	/// Target nodes without a source node keep the target's rest pose. When several source nodes map to one target
	/// node, the last one wins, both for the rest transform and for the track that animates it.
	pub(crate) fn by_name(clip: PackedAnimation<'_>, source: &Skeleton, target: &Skeleton) -> Self {
		let map = SkeletonPoseMap::by_name(source, target);
		let mut tracks = vec![NONE; target.nodes.len()].into_boxed_slice();
		for (track, words) in clip.tracks().iter().enumerate() {
			if let Some(node) = map.direct_target_node(words[0] as usize) {
				tracks[node] = track as u32;
			}
		}
		Self {
			tracks,
			rest: map.target_rest_pose().into(),
		}
	}
}

/// Returns the timeline a channel's curve uses.
fn timeline_of(channel: u32) -> usize {
	(channel >> 2) as usize
}

/// The `PackedTimeline` struct collects the curves keyed at one set of times while a clip is packed.
struct PackedTimeline {
	times: Vec<f32>,
	/// Each curve's words for every key, key after key.
	curves: Vec<Vec<u32>>,
	/// The words each key's row holds: every curve's words for one key.
	row_words: u32,
}

/// Builds the timelines and track table, then writes them into one word-aligned allocation.
fn pack_data(duration: f32, frame_rate: Option<f32>, tracks: Vec<NodeTrack>) -> Box<[u32]> {
	let mut timelines: Vec<PackedTimeline> = Vec::new();
	let mut packed_tracks = Vec::with_capacity(tracks.len());
	for track in tracks {
		// Array elements evaluate left to right, so each timeline's rows keep translation, rotation, scale order.
		let [translation, rotation, scale] = [
			track.translation.map(|curve| pack_curve(curve, &mut timelines)),
			track.rotation.map(|curve| pack_curve(curve, &mut timelines)),
			track.scale.map(|curve| pack_curve(curve, &mut timelines)),
		]
		.map(|channel| channel.unwrap_or((NONE, 0)));
		packed_tracks.push([
			track.node,
			translation.0,
			translation.1,
			rotation.0,
			rotation.1,
			scale.0,
			scale.1,
		]);
	}

	let tracks_offset = HEADER_WORDS;
	let timelines_offset = tracks_offset + packed_tracks.len() * TRACK_WORDS;
	let mut next = timelines_offset + timelines.len() * TIMELINE_WORDS;
	let mut timeline_words = Vec::with_capacity(timelines.len());
	for timeline in &timelines {
		let keys = timeline.times.len();
		let times = next;
		let rows = times + keys;
		next = rows + keys * timeline.row_words as usize;
		timeline_words.push([keys as u32, times as u32, rows as u32, timeline.row_words]);
	}

	let mut words = Vec::with_capacity(next);
	words.extend([
		duration.to_bits(),
		frame_rate.unwrap_or(0.0).to_bits(),
		packed_tracks.len() as u32,
		tracks_offset as u32,
		timelines.len() as u32,
		timelines_offset as u32,
	]);
	words.extend(packed_tracks.into_iter().flatten());
	words.extend(timeline_words.into_iter().flatten());
	for timeline in &timelines {
		words.extend(timeline.times.iter().map(|time| time.to_bits()));
		// Rows go key by key, so a sample reads every curve's words at one key from adjacent memory.
		for key in 0..timeline.times.len() {
			for curve in &timeline.curves {
				let key_words = curve.len() / timeline.times.len();
				words.extend_from_slice(&curve[key * key_words..][..key_words]);
			}
		}
	}
	debug_assert_eq!(words.len(), next, "packed clip layout must fill exactly the words it planned");
	words.into_boxed_slice()
}

/// Adds one curve to the timeline keyed at its times, creating that timeline if no earlier curve used them, and
/// returns the track channel and row offset that read it back.
///
/// Values are stored as [`CurveValue::from_components`] would rebuild them, so sampling reads them back with
/// [`CurveValue::from_key`] and normalizes each rotation only once. Cubic keys are stored as consecutive
/// `[value, in_tangent, out_tangent]` triples.
fn pack_curve<V: CurveValue<N>, T: CurveComponents<N>, const N: usize>(
	curve: Curve<V, T>,
	timelines: &mut Vec<PackedTimeline>,
) -> (u32, u32) {
	let mut words = Vec::new();
	let (interpolation, times) = match curve {
		Curve::Step { times, values } => {
			words.extend(values.into_iter().flat_map(stored_key).map(f32::to_bits));
			(CurveInterpolation::Step, times)
		}
		Curve::Linear { times, values } => {
			words.extend(values.into_iter().flat_map(stored_key).map(f32::to_bits));
			(CurveInterpolation::Linear, times)
		}
		Curve::CubicSpline {
			times,
			values,
			in_tangents,
			out_tangents,
		} => {
			for ((key, incoming), outgoing) in values.into_iter().zip(in_tangents).zip(out_tangents) {
				words.extend(
					[stored_key(key), incoming.components(), outgoing.components()]
						.into_iter()
						.flatten()
						.map(f32::to_bits),
				);
			}
			(CurveInterpolation::CubicSpline, times)
		}
	};
	// Curves share a timeline only when their key times match exactly, so sharing never changes a sample.
	let same_times = |timeline: &PackedTimeline| {
		timeline.times.len() == times.len() && timeline.times.iter().zip(&times).all(|(a, b)| a.to_bits() == b.to_bits())
	};
	let index = match timelines.iter().position(same_times) {
		Some(index) => index,
		None => {
			timelines.push(PackedTimeline {
				times,
				curves: Vec::new(),
				row_words: 0,
			});
			timelines.len() - 1
		}
	};
	let timeline = &mut timelines[index];
	let offset = timeline.row_words;
	timeline.row_words += (words.len() / timeline.times.len()) as u32;
	timeline.curves.push(words);
	(((index as u32) << 2) | interpolation as u32, offset)
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
			animation::{Animation, NodeTrack, RotationCurve, ScaleCurve, TranslationCurve},
			skeleton::{LocalTransform, Skeleton},
		},
	};

	use super::{PackedAnimation, PackedAnimationData, STACK_TIMELINES, TrackTargets};
	use crate::animation::{sample_local_pose, test_node};

	/// Unit rotations whose components normalize without rounding, so packed and resource keys hold the same bits.
	const ROTATIONS: [[f32; 4]; 4] = [
		[0.0, 0.0, 0.0, 1.0],
		[0.5, 0.5, 0.5, 0.5],
		[0.0, 0.0, 1.0, 0.0],
		[0.5, -0.5, 0.5, 0.5],
	];

	/// Builds a chain skeleton whose nodes rest at distinct places, so a node a clip doesn't animate shows its rest.
	fn chain_skeleton(node_count: usize) -> Skeleton {
		let nodes = (0..node_count)
			.map(|node| {
				let rest = LocalTransform {
					translation: math::Vector::new(node as f32, 0.5, 0.0),
					..LocalTransform::identity()
				};
				test_node(Some(&format!("node-{node}")), node.checked_sub(1).map(|parent| parent as u32), rest)
			})
			.collect();
		Skeleton { nodes }
	}

	/// Builds tracks that cover every interpolation: some curves share their key times, one node has no track, and
	/// `own_timelines` more nodes each have a translation keyed at times of its own.
	fn mixed_tracks(own_timelines: usize) -> Vec<NodeTrack> {
		let shared = vec![0.0, 0.25, 0.5, 1.0];
		let rotation = |key: usize| math::Orientation::try_from_array(ROTATIONS[key % ROTATIONS.len()]).expect("unit rotation");
		let translation = |key: usize, node: usize| math::Vector::new(key as f32, node as f32 * 0.5, -(key as f32));
		let mut tracks = vec![
			NodeTrack {
				node: 0,
				translation: Some(TranslationCurve::Linear {
					times: shared.clone(),
					values: (0..4).map(|key| translation(key, 0)).collect(),
				}),
				rotation: Some(RotationCurve::CubicSpline {
					times: shared.clone(),
					values: (0..4).map(rotation).collect(),
					in_tangents: (0..4).map(|key| [0.25 * key as f32, 0.0, -0.5, 0.125]).collect(),
					out_tangents: (0..4).map(|key| [0.0, 0.5 * key as f32, 0.25, -0.125]).collect(),
				}),
				scale: Some(ScaleCurve::Step {
					times: vec![0.0, 1.0],
					values: vec![math::Scale::new(1.0, 2.0, 3.0), math::Scale::new(0.5, 0.5, 0.5)],
				}),
			},
			NodeTrack {
				node: 1,
				translation: None,
				rotation: Some(RotationCurve::Linear {
					times: shared.clone(),
					values: (0..4).map(rotation).collect(),
				}),
				scale: None,
			},
			// Node 2 has no track, so it keeps its rest pose.
			NodeTrack {
				node: 3,
				translation: Some(TranslationCurve::Step {
					times: shared,
					values: (0..4).map(|key| translation(key, 3)).collect(),
				}),
				rotation: None,
				scale: None,
			},
		];
		tracks.extend((0..own_timelines).map(|timeline| {
			let node = 4 + timeline;
			// Every node's middle key sits at a time no other curve uses.
			let times = vec![0.0, 0.3 + 0.01 * timeline as f32, 1.0];
			NodeTrack {
				node: node as u32,
				translation: Some(TranslationCurve::Linear {
					times,
					values: (0..3).map(|key| translation(key, node)).collect(),
				}),
				rotation: None,
				scale: None,
			}
		}));
		tracks
	}

	#[test]
	fn packed_poses_match_the_resource_curves_they_were_packed_from() {
		// One clip whose timelines all fit the stack, and one with more, which places each curve's time on its own.
		for own_timelines in [0, STACK_TIMELINES + 4] {
			let node_count = 4 + own_timelines;
			let skeleton = chain_skeleton(node_count);
			let animation = |tracks| Animation {
				name: None,
				skeleton: Reference::in_memory("mixed.skeleton", chain_skeleton(node_count)),
				duration: 1.0,
				frame_rate: None,
				tracks,
			};
			let resource = animation(mixed_tracks(own_timelines));
			let packed = PackedAnimationData::from_resource(animation(mixed_tracks(own_timelines)));
			let clip = PackedAnimation::from_words(&packed.data);
			let targets = TrackTargets::by_name(clip, &skeleton, &skeleton);
			let mut expected = Vec::new();
			let mut pose = vec![LocalTransform::identity(); node_count];
			let mut nodes = [LocalTransform::identity(); 3];

			for time in [-0.5, 0.0, 0.13, 0.25, 0.4, 0.77, 1.0, 1.5] {
				sample_local_pose(&resource, time, &mut expected);
				clip.sample_pose(time, &targets, &mut pose);
				clip.sample_nodes(time, &targets, &[3, 0, 2], &mut nodes);

				assert_eq!(pose, expected, "pose at {time} s with {own_timelines} own timelines");
				assert_eq!(nodes, [expected[3], expected[0], expected[2]], "nodes at {time} s");
			}
		}
	}

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
			frame_rate: None,
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
		let clip = PackedAnimation::from_words(&packed.data);
		let targets = TrackTargets::by_name(clip, packed.skeleton.resource(), &target);
		let mut output = [LocalTransform::identity()];

		clip.sample_pose(0.0, &targets, &mut output);

		assert_eq!(output[0].translation, math::Vector::new(2.0, 0.0, 0.0));
	}
}
