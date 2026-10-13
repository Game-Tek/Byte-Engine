//! The corners of the patch where two shapes with flat sides or straight edges touch.
//!
//! GJK and EPA find the direction two shapes touch along and how deep, and one pair of points that are that far
//! apart. Where flat sides or edges meet, there is a whole patch of such pairs, and those two points are an arbitrary
//! one of them, often at a corner. A body that is pushed at one point turns about it, so a body that rests on a flat
//! side and is held up at one point rocks on it. Held up at the corners of the patch it rests on, it lies still.

use math::{OrientedBox, Point, UnitVector, Vector};
use smallvec::SmallVec;

/// The part of a shape that touches another: the corners of its core that reach furthest along the contact's normal,
/// or nearly so, which are one corner, the two ends of an edge, or the corners of a flat side.
pub(crate) type Feature<Space> = SmallVec<[Point<Space>; 8]>;

/// How far short of its shape's furthest corner a corner may be, beyond how deep the shapes are in each other, to
/// still count as part of the feature that touches.
///
/// A body rests a few millimeters into what it lies on, and not evenly, so the corners of the side it rests on are
/// not equally deep. All of them have to keep counting from step to step for the body to lie still, so a corner
/// counts while it is within a few times that depth of touching. A corner that has not reached the other shape yet
/// gets a contact with a negative depth, which lets it close that gap and no more.
pub(crate) const FEATURE_SLACK: f32 = 0.02;

/// A position in the frame of a contact: two coordinates across the normal, and the height along it.
type Framed = [f32; 3];

/// The `PatchCorner` struct holds one of the places where two shapes that touch over a patch should be held apart.
pub(crate) struct PatchCorner<Space> {
	/// The point on shape A's surface.
	pub(crate) on_a: Point<Space>,
	/// The point on shape B's surface, across from it.
	pub(crate) on_b: Point<Space>,
	/// How far the shapes are in each other there. Negative where they do not reach each other yet, by the gap
	/// between them.
	pub(crate) depth: f32,
	/// A number from zero to three that tells this corner from the patch's others, and that the corner at about the
	/// same place of the patch has again in the next step, so that what was learned about a corner can be carried
	/// over.
	pub(crate) number: u8,
}

/// Returns up to four corners of the patch where the features of two shapes touch, along `normal`, which points
/// from shape A toward shape B. Each shape is its feature made thicker by its radius.
///
/// The patch is where the two features overlap, seen along the normal: all of a flat side that lies on a larger one,
/// the shared part of two sides that overlap, the stretch of an edge that lies on a side, or the one place where two
/// edges cross. A patch with more corners than four is held at four that are far from each other, starting with the
/// deepest.
///
/// Returns none when the features do not overlap, seen along the normal, which rounding can do to features that only
/// just touch.
pub(crate) fn patch_corners<Space>(
	feature_a: &[Point<Space>],
	radius_a: f32,
	feature_b: &[Point<Space>],
	radius_b: f32,
	normal: UnitVector<Space>,
) -> SmallVec<[PatchCorner<Space>; 4]> {
	let mut held = SmallVec::new();
	let Some(&origin) = feature_a.first() else {
		return held;
	};
	let Some((across, along)) = frame_across(normal) else {
		return held;
	};
	let normal: Vector<Space> = normal.into();
	let framed = |feature: &[Point<Space>]| -> SmallVec<[Framed; 8]> {
		let mut corners: SmallVec<[Framed; 8]> = feature
			.iter()
			.map(|&corner| {
				let offset = corner - origin;
				[across.dot(offset), along.dot(offset), normal.dot(offset)]
			})
			.collect();
		// The corners of a flat side are put in sequence around it, which is by their angle around its middle.
		if corners.len() > 2 {
			let middle = middle_of(&corners);
			let turn = |corner: &Framed| turn_around(corner[0] - middle[0], corner[1] - middle[1]);
			corners.sort_unstable_by(|left, right| turn(left).total_cmp(&turn(right)));
		}
		corners
	};
	let (a, b) = (framed(feature_a), framed(feature_b));

	// The patch is the smaller feature cut down to what is over the larger one. A flat side cuts with its edges, and
	// when neither feature is a flat side, the patch is where two edges cross or run side by side, or a corner.
	let mut patch: SmallVec<[[f32; 2]; 8]> = SmallVec::new();
	match (a.len(), b.len()) {
		(0, _) | (_, 0) => {}
		(1, _) => patch.push([a[0][0], a[0][1]]),
		(_, 1) => patch.push([b[0][0], b[0][1]]),
		(2, 2) => patch.push(edges_meet([a[0], a[1]], [b[0], b[1]])),
		(_, 2) => patch.extend(clipped_segment([b[0], b[1]], &a).into_iter().flatten()),
		(2, _) => patch.extend(clipped_segment([a[0], a[1]], &b).into_iter().flatten()),
		_ => {
			patch.extend(a.iter().map(|corner| [corner[0], corner[1]]));
			clip_polygon(&mut patch, &b);
		}
	}

	// Each corner of the patch with the height of shape A's surface there and of shape B's, which are how far the
	// shapes are in each other at it. Both surfaces are worked out once, and every corner once.
	let (top, bottom) = (Surface::of(&a), Surface::of(&b));
	let patch: SmallVec<[([f32; 2], f32, f32); 8]> = patch
		.into_iter()
		.map(|corner| (corner, top.height_at(corner) + radius_a, bottom.height_at(corner) - radius_b))
		.collect();

	let chosen = spread_four(&patch);
	let numbers = quarter_numbers(chosen.iter().map(|(corner, ..)| *corner));
	held.extend(chosen.into_iter().zip(numbers).map(|((corner, top, bottom), number)| {
		let placed = |height: f32| origin + across * corner[0] + along * corner[1] + normal * height;
		PatchCorner {
			on_a: placed(top),
			on_b: placed(bottom),
			depth: top - bottom,
			number,
		}
	}));
	held
}

/// Returns up to four corners of the patch where a side of one box touches another box, when the direction the boxes
/// overlap least along is one that a side faces, as [`math::collision::obb_vs_obb_separation`] reports.
///
/// `reference` is the box with that side, `axis` the index of its axis along the direction, and `outward` the
/// direction the side faces, toward the other box, `incident`. `reference_is_a` tells which of the two is shape A of
/// the contact, whose normal points from shape A toward shape B.
///
/// This is what [`patch_corners`] finds for two boxes, worked out from what is known about a box: the side of the
/// other box that faces the reference side most is cut down to the part over the reference side, which is a
/// rectangle along two of the reference box's own axes. Corners further than [`FEATURE_SLACK`] from reaching the
/// reference side are left out, so a box that stands on an edge or a corner is held at that edge or corner only.
pub(crate) fn box_side_patch<Space>(
	reference: &OrientedBox<Space>,
	axis: usize,
	outward: UnitVector<Space>,
	incident: &OrientedBox<Space>,
	reference_is_a: bool,
) -> SmallVec<[PatchCorner<Space>; 4]> {
	let outward_vector: Vector<Space> = outward.into();
	let (side_axes, side_half) = (
		[reference.axes()[(axis + 1) % 3], reference.axes()[(axis + 2) % 3]],
		[
			reference.half_extents()[(axis + 1) % 3],
			reference.half_extents()[(axis + 2) % 3],
		],
	);
	let side_height = reference.half_extents()[axis];

	// The side of the other box that faces the reference side most squarely, and its four corners in sequence
	// around it.
	let (incident_axes, incident_half) = (incident.axes(), incident.half_extents());
	let leans: [f32; 3] = std::array::from_fn(|index| incident_axes[index].dot(outward_vector));
	let facing = (0..3)
		.max_by(|&left: &usize, &right: &usize| leans[left].abs().total_cmp(&leans[right].abs()))
		.unwrap_or(0);
	let (first, second) = ((facing + 1) % 3, (facing + 2) % 3);
	let middle = incident.center() - incident_axes[facing] * incident_half[facing].copysign(leans[facing]);
	let corners = [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]].map(|[along_first, along_second]: [f32; 2]| {
		let corner = middle
			+ incident_axes[first] * (incident_half[first] * along_first)
			+ incident_axes[second] * (incident_half[second] * along_second);
		// Where the corner is over the reference side, along the side's two axes, and how far it is into the
		// reference box through that side.
		let offset = corner - reference.center();
		[
			side_axes[0].dot(offset),
			side_axes[1].dot(offset),
			side_height - outward.dot(offset),
		]
	});

	// Cut the side down to the part over the reference side, one edge of the rectangle at a time. A corner made by a
	// cut is as deep as the place it is at on the edge that was cut. Four cuts of a side with four corners leave at
	// most eight, so the corners go back and forth between two fixed rows.
	let mut rows = [[[0.0f32; 3]; 8]; 2];
	rows[0][..4].copy_from_slice(&corners);
	let (mut count, mut row) = (4, 0);
	for (coordinate, toward) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
		let limit = side_half[coordinate];
		// A corner within the slack of an edge counts as inside it, which keeps a side that lies right on the edge.
		let slack = -1e-4 * limit.max(1e-3);
		let [first_row, second_row] = &mut rows;
		let (from, to) = if row == 0 {
			(&*first_row, second_row)
		} else {
			(&*second_row, first_row)
		};
		let mut kept = 0;
		for index in 0..count {
			let (current, next) = (from[index], from[if index + 1 == count { 0 } else { index + 1 }]);
			let (current_inside, next_inside) = (limit - toward * current[coordinate], limit - toward * next[coordinate]);
			if current_inside >= slack && kept < 8 {
				to[kept] = current;
				kept += 1;
			}
			if (current_inside >= slack) != (next_inside >= slack) && kept < 8 {
				let share = current_inside / (current_inside - next_inside);
				to[kept] = std::array::from_fn(|part| current[part] + (next[part] - current[part]) * share);
				kept += 1;
			}
		}
		(count, row) = (kept, 1 - row);
	}

	// Each corner that is near enough to count, as a place on the reference side with the two heights there: the
	// side's own, and the corner's.
	let mut patch = [([0.0f32; 2], 0.0f32, 0.0f32); 8];
	let mut near = 0;
	for &[across, along, depth] in &rows[row][..count] {
		if depth >= -FEATURE_SLACK {
			patch[near] = ([across, along], side_height, side_height - depth);
			near += 1;
		}
	}
	let chosen = spread_four(&patch[..near]);

	// The corners are numbered in the frame that the contact's normal gives, as those of every other patch are, so
	// that a corner keeps its number when the side that faces the other box changes from one box's to the other's.
	// Only which side of the patch's middle a corner is on counts, so the frame's directions can be of any length.
	let normal: Vector<Space> = if reference_is_a { outward_vector } else { -outward_vector };
	let (across, along) = unscaled_frame_across(normal);
	let placed = |[across, along]: [f32; 2], height: f32| {
		reference.center() + side_axes[0] * across + side_axes[1] * along + outward * height
	};
	let numbers = quarter_numbers(chosen.iter().map(|&(corner, top, _)| {
		let offset = placed(corner, top) - reference.center();
		[across.dot(offset), along.dot(offset)]
	}));

	chosen
		.into_iter()
		.zip(numbers)
		.map(|((corner, top, bottom), number)| {
			let (on_reference, on_incident) = (placed(corner, top), placed(corner, bottom));
			let (on_a, on_b) = if reference_is_a {
				(on_reference, on_incident)
			} else {
				(on_incident, on_reference)
			};
			PatchCorner {
				on_a,
				on_b,
				depth: top - bottom,
				number,
			}
		})
		.collect()
}

/// Returns two directions of length one that are square to `normal` and to each other, which with it make the frame
/// that a patch is laid out in.
fn frame_across<Space>(normal: UnitVector<Space>) -> Option<(Vector<Space>, Vector<Space>)> {
	let normal: Vector<Space> = normal.into();
	let across: Vector<Space> = unscaled_frame_across(normal).0.normalized().ok()?.into();
	Some((across, normal.cross(across)))
}

/// Returns two directions that are square to `normal` and to each other, of whatever length they come out at. The
/// first comes from the axis that the normal leans on least, so it is never close to no length.
fn unscaled_frame_across<Space>(normal: Vector<Space>) -> (Vector<Space>, Vector<Space>) {
	let parts = normal.to_array().map(f32::abs);
	let least = if parts[0] <= parts[1] && parts[0] <= parts[2] {
		0
	} else if parts[1] <= parts[2] {
		1
	} else {
		2
	};
	let mut axis = [0.0; 3];
	axis[least] = 1.0;
	let across = normal.cross(Vector::from_array(axis));
	(across, normal.cross(across))
}

/// Returns up to four corners of a patch, each given as its place and the heights of the two surfaces there. Four
/// corners hold a patch as well as all of them do, when they are far apart. The deepest comes first, and each one
/// after it is the corner furthest from those already chosen.
fn spread_four(patch: &[([f32; 2], f32, f32)]) -> SmallVec<[([f32; 2], f32, f32); 4]> {
	let mut chosen: SmallVec<[([f32; 2], f32, f32); 4]> = SmallVec::new();
	if let Some(&deepest) = patch
		.iter()
		.max_by(|left, right| (left.1 - left.2).total_cmp(&(right.1 - right.2)))
	{
		chosen.push(deepest);
	}
	while chosen.len() < 4 && chosen.len() < patch.len() {
		let apart = |corner: [f32; 2]| {
			chosen
				.iter()
				.map(|(held, ..)| (corner[0] - held[0]).powi(2) + (corner[1] - held[1]).powi(2))
				.fold(f32::INFINITY, f32::min)
		};
		let Some((furthest, distance)) = patch
			.iter()
			.map(|&held| (held, apart(held.0)))
			.max_by(|left, right| left.1.total_cmp(&right.1))
		else {
			break;
		};
		// Cutting a side can leave corners on top of each other, and a corner is only held once.
		if distance <= f32::EPSILON {
			break;
		}
		chosen.push(furthest);
	}
	chosen
}

/// Returns a number from zero to three for each of up to four corners of a patch, given as their places across the
/// contact's normal.
///
/// A corner is numbered by the quarter of the patch it is in, seen from the patch's middle, which stays the same from
/// step to step while the bodies lie still. The sequence the corners were chosen in does not, because which corner is
/// deepest changes with the slightest tilt. Two corners in one quarter take the numbers left over.
fn quarter_numbers(corners: impl Iterator<Item = [f32; 2]> + Clone) -> SmallVec<[u8; 4]> {
	let count = corners.clone().count().max(1) as f32;
	let middle = corners.clone().fold([0.0; 2], |sum, corner| {
		[sum[0] + corner[0] / count, sum[1] + corner[1] / count]
	});
	let mut taken = [false; 4];
	let mut numbers: SmallVec<[Option<u8>; 4]> = corners
		.map(|corner| {
			let quarter = usize::from(corner[0] >= middle[0]) | usize::from(corner[1] >= middle[1]) << 1;
			(!std::mem::replace(&mut taken[quarter], true)).then_some(quarter as u8)
		})
		.collect();
	for number in &mut numbers {
		if number.is_none() {
			let free = taken.iter().position(|taken| !taken).unwrap_or(0);
			taken[free] = true;
			*number = Some(free as u8);
		}
	}
	numbers.into_iter().map(|number| number.unwrap_or(0)).collect()
}

/// Returns a number that grows with the angle of the direction (`x`, `y`) as it turns once around, from just past
/// the negative x-axis all the way back to it. It sorts directions by angle without working any angle out.
fn turn_around(x: f32, y: f32) -> f32 {
	let reach = x.abs() + y.abs();
	if reach == 0.0 {
		return 0.0;
	}
	// From 0 along the positive x-axis to 2 along the negative one, by way of the positive y-axis, and the same
	// negated by way of the negative y-axis.
	(1.0 - x / reach).copysign(y)
}

/// Returns the middle of `corners`.
fn middle_of(corners: &[Framed]) -> Framed {
	let sum = corners.iter().fold([0.0; 3], |sum, corner| {
		[sum[0] + corner[0], sum[1] + corner[1], sum[2] + corner[2]]
	});
	sum.map(|part| part / corners.len() as f32)
}

/// Returns how far inside the edge from `from` to `to` of a flat side `point` is, times the edge's length. The
/// side's corners run counterclockwise, so inside is to the left.
fn inside(from: Framed, to: Framed, point: [f32; 2]) -> f32 {
	(to[0] - from[0]) * (point[1] - from[1]) - (to[1] - from[1]) * (point[0] - from[0])
}

/// Returns how far outside an edge a point may be to still count as inside it, in the units of [`inside`]. Sides of
/// the same size that lie right over each other have their corners on each other's edges.
fn edge_slack(from: Framed, to: Framed) -> f32 {
	let length_squared = (to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2);
	-1e-4 * length_squared
}

/// Returns where two edges cross, seen along the normal, or else the place halfway between their closest points.
/// Edges that run side by side are as close along a stretch, and the place is in the middle of it.
fn edges_meet(a: [Framed; 2], b: [Framed; 2]) -> [f32; 2] {
	let offset = |from: Framed, to: Framed| [to[0] - from[0], to[1] - from[1]];
	let dot = |left: [f32; 2], right: [f32; 2]| left[0] * right[0] + left[1] * right[1];
	let closest_on = |edge: [Framed; 2], position: [f32; 2]| {
		let along = offset(edge[0], edge[1]);
		let length_squared = dot(along, along);
		let share = if length_squared == 0.0 {
			0.0
		} else {
			(dot([position[0] - edge[0][0], position[1] - edge[0][1]], along) / length_squared).clamp(0.0, 1.0)
		};
		[edge[0][0] + along[0] * share, edge[0][1] + along[1] * share]
	};

	let (a_along, b_along) = (offset(a[0], a[1]), offset(b[0], b[1]));
	let (a_length, b_length) = (dot(a_along, a_along), dot(b_along, b_along));
	let between = offset(b[0], a[0]);
	let (a_offset, b_offset) = (dot(a_along, between), dot(b_along, between));
	let alignment = dot(a_along, b_along);

	// Where the lines through the edges cross, as a share of the way along `a`. The divisor is the squared area the
	// two edges span, which is zero when they are parallel or one of them has no length.
	let spanned = a_length * b_length - alignment * alignment;
	let share = if spanned > f32::EPSILON * a_length * b_length {
		((alignment * b_offset - a_offset * b_length) / spanned).clamp(0.0, 1.0)
	} else if a_length > 0.0 {
		// The stretch of `a` that lies beside `b` runs between where `b`'s two ends are nearest it.
		let beside = [-a_offset, alignment - a_offset].map(|along| (along / a_length).clamp(0.0, 1.0));
		(beside[0] + beside[1]) * 0.5
	} else {
		0.0
	};

	// The closest point of `b` to that point may be one of `b`'s ends, which in turn may be closer to another point
	// of `a`.
	let on_b = closest_on(b, [a[0][0] + a_along[0] * share, a[0][1] + a_along[1] * share]);
	let on_a = closest_on(a, on_b);
	[(on_a[0] + on_b[0]) * 0.5, (on_a[1] + on_b[1]) * 0.5]
}

/// Cuts the flat side `subject` down to the part that is over the flat side `clip`, edge by edge.
fn clip_polygon(subject: &mut SmallVec<[[f32; 2]; 8]>, clip: &[Framed]) {
	let mut cut: SmallVec<[[f32; 2]; 8]> = SmallVec::new();
	for edge in 0..clip.len() {
		let (from, to) = (clip[edge], clip[(edge + 1) % clip.len()]);
		let slack = edge_slack(from, to);
		cut.clear();
		for index in 0..subject.len() {
			let (current, next) = (subject[index], subject[(index + 1) % subject.len()]);
			let (current_inside, next_inside) = (inside(from, to, current), inside(from, to, next));
			if current_inside >= slack {
				cut.push(current);
			}
			// The subject's edge crosses this one, and the crossing is a corner of what is left.
			if (current_inside >= slack) != (next_inside >= slack) {
				let share = current_inside / (current_inside - next_inside);
				cut.push([
					current[0] + (next[0] - current[0]) * share,
					current[1] + (next[1] - current[1]) * share,
				]);
			}
		}
		std::mem::swap(subject, &mut cut);
		if subject.is_empty() {
			return;
		}
	}
}

/// Returns the ends of the part of the edge `segment` that is over the flat side `clip`, when there is one.
fn clipped_segment(segment: [Framed; 2], clip: &[Framed]) -> Option<[[f32; 2]; 2]> {
	let ends = segment.map(|end| [end[0], end[1]]);
	// The part that is left, as shares of the way from the first end to the second.
	let (mut start, mut end) = (0.0f32, 1.0f32);
	for edge in 0..clip.len() {
		let (from, to) = (clip[edge], clip[(edge + 1) % clip.len()]);
		let slack = edge_slack(from, to);
		let (first, second) = (inside(from, to, ends[0]), inside(from, to, ends[1]));
		if first < slack && second < slack {
			return None;
		}
		if first < slack {
			start = start.max(first / (first - second));
		} else if second < slack {
			end = end.min(first / (first - second));
		}
	}
	(start <= end).then(|| {
		[start, end].map(|share| {
			[
				ends[0][0] + (ends[1][0] - ends[0][0]) * share,
				ends[0][1] + (ends[1][1] - ends[0][1]) * share,
			]
		})
	})
}

/// The `Surface` struct answers how high a feature's surface is at any position across the normal, from what is
/// worked out about the feature once.
enum Surface {
	/// A corner, which is as high wherever it is asked about.
	Corner(f32),
	/// An edge, which is as high as its point nearest the position.
	Edge([Framed; 2]),
	/// A flat side, which is as high as its plane: the height at its middle, and how much that changes for each unit
	/// across the normal.
	Side { middle: Framed, slope: [f32; 2] },
}

impl Surface {
	fn of(feature: &[Framed]) -> Self {
		match feature {
			[] => Self::Corner(0.0),
			[corner] => Self::Corner(corner[2]),
			[from, to] => Self::Edge([*from, *to]),
			corners => {
				// The plane's normal is the sum of the cross products of each pair of corners in a row, seen from the
				// middle, which is sound for any flat side, however many corners it has.
				let middle = middle_of(corners);
				let mut normal = [0.0; 3];
				for index in 0..corners.len() {
					let (from, to) = (corners[index], corners[(index + 1) % corners.len()]);
					let (u, v) = (
						[from[0] - middle[0], from[1] - middle[1], from[2] - middle[2]],
						[to[0] - middle[0], to[1] - middle[1], to[2] - middle[2]],
					);
					normal[0] += u[1] * v[2] - u[2] * v[1];
					normal[1] += u[2] * v[0] - u[0] * v[2];
					normal[2] += u[0] * v[1] - u[1] * v[0];
				}
				// A side that is seen edge on has no height to speak of at one position, and counts as level.
				let edge_on = normal[2].abs() <= f32::EPSILON * (normal[0].abs() + normal[1].abs());
				let slope = if edge_on {
					[0.0; 2]
				} else {
					[-normal[0] / normal[2], -normal[1] / normal[2]]
				};
				Self::Side { middle, slope }
			}
		}
	}

	fn height_at(&self, position: [f32; 2]) -> f32 {
		match self {
			Self::Corner(height) => *height,
			Self::Edge([from, to]) => {
				let along = [to[0] - from[0], to[1] - from[1]];
				let length_squared = along[0] * along[0] + along[1] * along[1];
				if length_squared == 0.0 {
					return from[2];
				}
				let share = ((position[0] - from[0]) * along[0] + (position[1] - from[1]) * along[1]) / length_squared;
				from[2] + (to[2] - from[2]) * share.clamp(0.0, 1.0)
			}
			Self::Side { middle, slope } => {
				middle[2] + slope[0] * (position[0] - middle[0]) + slope[1] * (position[1] - middle[1])
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::LocalSpace;

	fn points(corners: &[[f32; 3]]) -> Feature<LocalSpace> {
		corners.iter().map(|&corner| Point::from_array(corner)).collect()
	}

	fn up() -> UnitVector<LocalSpace> {
		UnitVector::y_axis()
	}

	/// Returns the corners of a level rectangle from (`x0`, `z0`) to (`x1`, `z1`) at height `y`.
	fn rectangle(x0: f32, z0: f32, x1: f32, z1: f32, y: f32) -> Feature<LocalSpace> {
		// Out of sequence, as a shape may list them.
		points(&[[x0, y, z0], [x1, y, z1], [x1, y, z0], [x0, y, z1]])
	}

	/// Asserts that the points on shape A of `corners` are `expected`, in any sequence.
	fn assert_on_a(corners: &[PatchCorner<LocalSpace>], expected: &[[f32; 3]]) {
		assert_eq!(
			corners.len(),
			expected.len(),
			"{:?}",
			corners.iter().map(|corner| corner.on_a).collect::<Vec<_>>()
		);
		for &point in expected {
			assert!(
				corners
					.iter()
					.any(|corner| (corner.on_a - Point::from_array(point)).length() <= 1e-5),
				"no corner is at {point:?}: {:?}",
				corners.iter().map(|corner| corner.on_a).collect::<Vec<_>>()
			);
		}
	}

	#[test]
	fn a_small_side_on_a_large_one_is_held_at_its_own_corners() {
		// The top of a floor at height 1, and the underside of a box 0.01 into it.
		let floor = rectangle(-4.0, -4.0, 4.0, 4.0, 1.0);
		let underside = rectangle(1.0, 2.0, 2.0, 3.0, 0.99);

		let corners = patch_corners(&floor, 0.0, &underside, 0.0, up());

		assert_on_a(
			&corners,
			&[[1.0, 1.0, 2.0], [2.0, 1.0, 2.0], [2.0, 1.0, 3.0], [1.0, 1.0, 3.0]],
		);
		for corner in &corners {
			assert!((corner.depth - 0.01).abs() <= 1e-6);
			assert!((corner.on_a - corner.on_b - Vector::new(0.0, 0.01, 0.0)).length() <= 1e-6);
		}

		// The other way around, from the box down to the floor, the points swap.
		let corners = patch_corners(&underside, 0.0, &floor, 0.0, -up());
		assert_on_a(
			&corners,
			&[[1.0, 0.99, 2.0], [2.0, 0.99, 2.0], [2.0, 0.99, 3.0], [1.0, 0.99, 3.0]],
		);
		assert!(corners.iter().all(|corner| (corner.on_b.y() - 1.0).abs() <= 1e-6));
	}

	#[test]
	fn sides_that_overlap_in_part_are_held_at_the_corners_of_that_part() {
		let lower = rectangle(0.0, 0.0, 1.0, 1.0, 1.0);
		let upper = rectangle(0.6, -0.5, 1.6, 0.5, 1.0);

		// They share the rectangle from (0.6, 0) to (1, 0.5).
		let corners = patch_corners(&lower, 0.0, &upper, 0.0, up());
		assert_on_a(
			&corners,
			&[[0.6, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 0.5], [0.6, 1.0, 0.5]],
		);

		// Sides of one size right over each other share all of themselves.
		let corners = patch_corners(&lower, 0.0, &lower, 0.0, up());
		assert_on_a(
			&corners,
			&[[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]],
		);
	}

	#[test]
	fn a_side_that_crosses_another_is_held_where_they_cross() {
		// A plank along x across a beam along z: no corner of either is over the other.
		let beam = rectangle(-0.5, -3.0, 0.5, 3.0, 1.0);
		let plank = rectangle(-2.0, 1.0, 4.0, 2.0, 1.0);

		let corners = patch_corners(&beam, 0.0, &plank, 0.0, up());

		assert_on_a(
			&corners,
			&[[-0.5, 1.0, 1.0], [0.5, 1.0, 1.0], [0.5, 1.0, 2.0], [-0.5, 1.0, 2.0]],
		);
	}

	#[test]
	fn a_tilted_side_is_deepest_at_its_lower_edge() {
		let floor = rectangle(-4.0, -4.0, 4.0, 4.0, 1.0);
		// The underside of a box whose edge at x = 0 is 0.02 into the floor and whose edge at x = 1 only touches it.
		let underside = points(&[[0.0, 0.98, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [0.0, 0.98, 1.0]]);

		let corners = patch_corners(&floor, 0.0, &underside, 0.0, up());

		assert_eq!(corners.len(), 4);
		for corner in &corners {
			let expected = if corner.on_a.x() < 0.5 { 0.02 } else { 0.0 };
			assert!(
				(corner.depth - expected).abs() <= 1e-6,
				"{} at {:?}",
				corner.depth,
				corner.on_a
			);
		}
		// The deepest corner comes first.
		assert!((corners[0].depth - 0.02).abs() <= 1e-6);
	}

	#[test]
	fn a_side_with_many_corners_is_held_at_four_that_are_far_apart() {
		let floor = rectangle(-4.0, -4.0, 4.0, 4.0, 1.0);
		let round: Feature<LocalSpace> = (0..12)
			.map(|corner| {
				let turn = corner as f32 / 12.0 * std::f32::consts::TAU;
				Point::new(turn.cos(), 1.0, turn.sin())
			})
			.collect();

		let corners = patch_corners(&floor, 0.0, &round, 0.0, up());

		// Four of the twelve, a quarter turn from each other.
		assert_eq!(corners.len(), 4);
		for corner in &corners {
			assert!(((corner.on_a - Point::new(0.0, 1.0, 0.0)).length() - 1.0).abs() <= 1e-5);
			let nearest = corners
				.iter()
				.map(|other| (other.on_a - corner.on_a).length())
				.filter(|distance| *distance > 0.0)
				.fold(f32::INFINITY, f32::min);
			assert!((nearest - std::f32::consts::SQRT_2).abs() <= 1e-4, "{nearest}");
		}
	}

	#[test]
	fn an_edge_on_a_side_is_held_at_the_ends_of_the_stretch_over_it() {
		let side = rectangle(0.0, 0.0, 2.0, 2.0, 1.0);
		// A capsule's segment, 0.3 thick, lying over the side's rim from x = 1 to x = 5.
		let segment = points(&[[1.0, 1.25, 0.5], [5.0, 1.25, 0.5]]);

		let corners = patch_corners(&side, 0.0, &segment, 0.3, up());

		assert_on_a(&corners, &[[1.0, 1.0, 0.5], [2.0, 1.0, 0.5]]);
		assert!(corners.iter().all(|corner| (corner.depth - 0.05).abs() <= 1e-6));
		assert!(corners.iter().all(|corner| (corner.on_b.y() - 0.95).abs() <= 1e-6));
		// Beside the side, there is no stretch over it.
		let beside = points(&[[3.0, 1.25, 0.5], [5.0, 1.25, 0.5]]);
		assert!(patch_corners(&side, 0.0, &beside, 0.3, up()).is_empty());
	}

	#[test]
	fn edges_are_held_where_they_cross_and_a_corner_where_it_is() {
		let first = points(&[[-1.0, 1.0, 0.0], [1.0, 1.0, 0.0]]);
		let second = points(&[[0.25, 0.9, -1.0], [0.25, 0.9, 1.0]]);
		let corners = patch_corners(&first, 0.0, &second, 0.0, up());
		assert_on_a(&corners, &[[0.25, 1.0, 0.0]]);
		assert!((corners[0].on_b - Point::new(0.25, 0.9, 0.0)).length() <= 1e-6 && (corners[0].depth - 0.1).abs() <= 1e-6);

		let side = rectangle(0.0, 0.0, 2.0, 2.0, 1.0);
		let corner = points(&[[0.5, 0.9, 1.5]]);
		let corners = patch_corners(&side, 0.0, &corner, 0.0, up());
		assert_on_a(&corners, &[[0.5, 1.0, 1.5]]);
		assert!((corners[0].on_b - Point::new(0.5, 0.9, 1.5)).length() <= 1e-6);
	}
	#[test]
	fn a_corner_that_has_not_reached_the_other_shape_has_a_gap() {
		let floor = rectangle(-4.0, -4.0, 4.0, 4.0, 1.0);
		// The underside of a box whose edge at x = 0 is 0.01 into the floor and whose edge at x = 1 is 0.01 above it.
		let underside = points(&[[0.0, 0.99, 0.0], [1.0, 1.01, 0.0], [1.0, 1.01, 1.0], [0.0, 0.99, 1.0]]);

		let corners = patch_corners(&floor, 0.0, &underside, 0.0, up());

		assert_eq!(corners.len(), 4);
		for corner in &corners {
			let lifted = corner.on_a.x() > 0.5;
			assert!(
				(corner.depth - if lifted { -0.01 } else { 0.01 }).abs() <= 1e-6,
				"{}",
				corner.depth
			);
			// Each point stays on its own shape's surface.
			assert!((corner.on_a.y() - 1.0).abs() <= 1e-6);
			assert!((corner.on_b.y() - if lifted { 1.01 } else { 0.99 }).abs() <= 1e-6);
		}
	}

	#[test]
	fn corners_keep_their_numbers_whichever_is_deepest() {
		let floor = rectangle(-4.0, -4.0, 4.0, 4.0, 1.0);
		// The same underside leaning one way and then the other, which changes the corner that is found first.
		let leaning = |low: f32, high: f32| points(&[[0.0, low, 0.0], [1.0, high, 0.0], [1.0, high, 1.0], [0.0, low, 1.0]]);
		let numbered = |underside: &Feature<LocalSpace>| {
			let mut corners: Vec<_> = patch_corners(&floor, 0.0, underside, 0.0, up())
				.iter()
				.map(|corner| (corner.number, corner.on_a.x().round() as i32, corner.on_a.z().round() as i32))
				.collect();
			corners.sort_unstable();
			corners
		};

		let (one_way, other_way) = (numbered(&leaning(0.99, 0.995)), numbered(&leaning(0.995, 0.99)));

		assert_eq!(one_way.iter().map(|corner| corner.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
		assert_eq!(one_way, other_way);
	}

	/// Returns a box around `center`, turned by `turn` radians about the z-axis.
	fn turned_box(center: [f32; 3], half_extents: [f32; 3], turn: f32) -> OrientedBox<LocalSpace> {
		let orientation = math::Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, turn)).unwrap();
		OrientedBox::new(Point::from_array(center), half_extents, orientation)
	}

	#[test]
	fn a_box_on_the_side_of_another_is_held_at_the_corners_of_what_they_share() {
		// A floor whose top is at height 1, and a box that is 0.01 into it and hangs over its rim at x = 4.
		let floor = turned_box([0.0, 0.0, 0.0], [4.0, 1.0, 4.0], 0.0);
		let resting = turned_box([3.5, 1.49, 2.0], [1.0, 0.5, 0.5], 0.0);

		let corners = box_side_patch(&floor, 1, up(), &resting, true);

		assert_on_a(
			&corners,
			&[[2.5, 1.0, 1.5], [4.0, 1.0, 1.5], [4.0, 1.0, 2.5], [2.5, 1.0, 2.5]],
		);
		let mut numbers: Vec<u8> = corners.iter().map(|corner| corner.number).collect();
		numbers.sort_unstable();
		assert_eq!(numbers, [0, 1, 2, 3]);
		for corner in &corners {
			assert!((corner.depth - 0.01).abs() <= 1e-5);
			assert!((corner.on_a - corner.on_b - Vector::new(0.0, 0.01, 0.0)).length() <= 1e-5);
		}

		// With the box as shape A, the side is shape B's, and the points swap.
		let swapped = box_side_patch(&floor, 1, up(), &resting, false);
		assert!(
			swapped
				.iter()
				.all(|corner| (corner.on_b.y() - 1.0).abs() <= 1e-5 && (corner.on_a.y() - 0.99).abs() <= 1e-5)
		);
	}

	#[test]
	fn a_box_that_stands_on_an_edge_is_held_at_that_edge() {
		// Turned an eighth of a turn about z, the box's lowest edge runs along z, 0.01 into the floor's top. The
		// side that faces the floor most rises steeply from it, so its two other corners are far from touching.
		let floor = turned_box([0.0, 0.0, 0.0], [4.0, 1.0, 4.0], 0.0);
		let reach = std::f32::consts::FRAC_1_SQRT_2;
		let standing = turned_box([1.0, 1.0 + reach - 0.01, 0.0], [0.5, 0.5, 0.5], std::f32::consts::FRAC_PI_4);

		let corners = box_side_patch(&floor, 1, up(), &standing, true);

		assert_on_a(&corners, &[[1.0, 1.0, -0.5], [1.0, 1.0, 0.5]]);
		assert!(corners.iter().all(|corner| (corner.depth - 0.01).abs() <= 1e-5));

		// A box that only leans a little is held at all four corners, the far ones by the gap they still have.
		let leaning = turned_box([1.0, 1.495, 0.0], [0.5, 0.5, 0.5], 0.01);
		let corners = box_side_patch(&floor, 1, up(), &leaning, true);
		assert_eq!(corners.len(), 4);
		assert!(corners.iter().any(|corner| corner.depth > 0.0) && corners.iter().any(|corner| corner.depth < 0.0));
	}
}
