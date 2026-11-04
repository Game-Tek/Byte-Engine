//! The groups of texels that share a guide while the bake traces their paths.
//!
//! A guide learns where a texel's light comes from, and needs more paths than one texel traces to learn it, so
//! neighboring texels that face the same way pool their learning paths. Neighbors are found in world space rather than
//! in the image, since the charts of a detailed mesh are small and a block of the image holds few texels of one surface.
//! The rule is the Guide groups row of the asset formats reference's Lightmaps section.

use super::texels::LightmapTexel;

/// The least cosine between a texel's normal and its group's first normal: about 25 degrees.
const SAME_FACING: f32 = 0.9;
/// The most texels a group holds, which bounds how many texels one learning invocation traces in turn.
const MOST_TEXELS: u32 = 32;

/// The `LightmapGuideGroup` struct is one group of texels, as the trace kernel reads it: a run of
/// [`GuideGroups::texels`].
///
/// Its layout matches `LightmapGuideGroup` in `lightmaps/layouts/lightmap_bake.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightmapGuideGroup {
	pub(crate) first: u32,
	pub(crate) count: u32,
	/// The world axis, 0 through 2 for x through z, its texels' hemisphere frames turn around: the one its first
	/// texel's normal lies farthest from.
	pub(crate) axis: u32,
}

/// The `GuideGroups` struct is how a lightmap's texels share guides, which the trace kernel learns and samples by.
///
/// [`group_texels`] makes it; the bake uploads its three lists as they are.
#[derive(Debug)]
pub(crate) struct GuideGroups {
	pub(crate) groups: Vec<LightmapGuideGroup>,
	/// The texels of every group, group after group.
	pub(crate) texels: Vec<u32>,
	/// The group of each texel.
	pub(crate) texel_groups: Vec<u32>,
}

/// Groups `texels` by where they lie and which way they face: texels in one cube of world space `cell` meters wide whose
/// normals share their largest component's axis and sign share a group when they face within about 25 degrees of its
/// first texel, up to 32 texels. Each group turns its texels' frames around the world axis its first texel's normal
/// lies farthest from, which no normal of the group lies along.
pub(crate) fn group_texels(texels: &[LightmapTexel], cell: f32) -> GuideGroups {
	let cube = |texel: &LightmapTexel| {
		let [x, y, z] = texel.position.map(|coordinate| (coordinate / cell).floor() as i32);
		let normal = texel.normal;
		let axis = (0..3).max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs())).unwrap_or(0);
		(x, y, z, axis * 2 + usize::from(normal[axis] < 0.0))
	};
	let mut order: Vec<u32> = (0..texels.len() as u32).collect();
	order.sort_by_key(|&texel| cube(&texels[texel as usize]));

	let mut result = GuideGroups {
		groups: Vec::new(),
		texels: Vec::with_capacity(texels.len()),
		texel_groups: vec![0; texels.len()],
	};
	// The texels of the cube being grouped that no group holds yet.
	let mut waiting = Vec::new();
	for run in order.chunk_by(|&a, &b| cube(&texels[a as usize]) == cube(&texels[b as usize])) {
		waiting.clear();
		waiting.extend_from_slice(run);
		while let Some(&seed) = waiting.first() {
			let facing = texels[seed as usize].normal;
			let group = result.groups.len() as u32;
			let first = result.texels.len() as u32;
			waiting.retain(|&texel| {
				let normal = texels[texel as usize].normal;
				let same = result.texels.len() as u32 - first < MOST_TEXELS
					&& normal[0] * facing[0] + normal[1] * facing[1] + normal[2] * facing[2] >= SAME_FACING;
				if same {
					result.texels.push(texel);
					result.texel_groups[texel as usize] = group;
				}
				!same
			});
			let axis = (0..3).min_by(|&a, &b| facing[a].abs().total_cmp(&facing[b].abs())).unwrap_or(0) as u32;
			result.groups.push(LightmapGuideGroup {
				first,
				count: result.texels.len() as u32 - first,
				axis,
			});
		}
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	fn texel(position: [f32; 3], normal: [f32; 3]) -> LightmapTexel {
		LightmapTexel {
			position,
			pixel: 0,
			normal,
			offset: 0.0,
		}
	}

	#[test]
	fn nearby_texels_that_face_alike_share_a_group_and_every_texel_has_one() {
		let up = [0.0, 1.0, 0.0];
		let side = [1.0, 0.0, 0.0];
		// Two texels facing up in the first cube, one facing sideways there, and one facing up in the next cube.
		let texels = [texel([0.1, 0.0, 0.1], up), texel([1.2, 0.0, 0.1], up), texel([0.2, 0.0, 0.1], side), texel([0.4, 0.0, 0.3], up)];

		let grouped = group_texels(&texels, 0.5);

		assert_eq!(grouped.groups.len(), 3);
		assert_eq!(grouped.texel_groups[0], grouped.texel_groups[3], "same cube, same facing");
		assert_ne!(grouped.texel_groups[0], grouped.texel_groups[2], "same cube, another facing");
		assert_ne!(grouped.texel_groups[0], grouped.texel_groups[1], "another cube");
		let up_group = grouped.groups[grouped.texel_groups[0] as usize];
		assert_ne!(up_group.axis, 1, "an upward group's frames turn around an axis its normals don't lie along");
		let mut listed = grouped.texels.clone();
		listed.sort();
		assert_eq!(listed, [0, 1, 2, 3], "every texel lies in exactly one group");
		for (index, group) in grouped.groups.iter().enumerate() {
			let members = &grouped.texels[group.first as usize..(group.first + group.count) as usize];
			assert!(members.iter().all(|&texel| grouped.texel_groups[texel as usize] == index as u32));
		}
	}

	#[test]
	fn a_crowded_cube_splits_into_groups_of_at_most_32_texels() {
		let texels = vec![texel([0.0; 3], [0.0, 1.0, 0.0]); 70];

		let grouped = group_texels(&texels, 0.5);

		let counts: Vec<u32> = grouped.groups.iter().map(|group| group.count).collect();
		assert_eq!(counts, [32, 32, 6]);
	}
}
