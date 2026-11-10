//! Turns the scene's render entities into the frame's instance records on several alley lanes.
//!
//! An instance's index is its entity's slot in [`super::scene::VisibilityScene::render_entities`], so every lane
//! writes the records of its own range of slots, in one pass, without knowing what the other lanes admit. Slots of
//! entities the frame doesn't draw keep stale records that no work item names. Each lane lists its instances per
//! raster phase in slot order, and the lists join in lane order, so the output is the same for every lane count. Work
//! that changes shared state, a skinned instance's vertices and palette or a parameter block that must be written,
//! can't run on a lane: the lane writes the rest of the record and lists the instance in [`LaneScratch::deferred`]
//! for the caller to finish in order.

use std::ops::Range;

use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::StableVec;

use super::acceleration_structures::TracedPrimitive;
use super::layout::{ActiveEvaluationMask, MAX_MATERIALS};
use super::lod::LodView;
use super::material_parameters::MaterialParameters;
use super::residency::MeshResidencies;
use super::scene::{Instance, RenderEntity};
use super::shader_data::{MESH_FLAG_DOUBLE_SIDED, ShaderMesh};
use crate::core::alley::Alley;
use crate::rendering::renderable::mesh::MeshKey;

/// The raster phases instances are listed by: opaque, masked, double-sided, double-sided masked, and transparent.
pub(super) const PHASES: usize = 5;
/// The phase of blended instances, whose evaluation slots go to the transparent mask.
pub(super) const TRANSPARENT_PHASE: usize = 4;

/// The render entities one lane takes.
///
/// On an Apple M4 one lane rebuilds about 25 instances a microsecond and a dispatch costs about a microsecond, so a
/// lane pays for itself well below this; the margin keeps lanes off small scenes, where the caller is already fast.
/// The rebuild reads each entity whole, so past a few lanes it waits on memory rather than on the cores.
const ENTITIES_PER_LANE: usize = 8192;

const EMPTY_MASK: ActiveEvaluationMask = [0; MAX_MATERIALS / u64::BITS as usize];

/// Returns the raster phase of an instance drawn with `alpha_mode`, showing both faces when `double_sided`.
///
/// Double-sided blend instances stay in the transparent phase, which still culls back faces.
pub(super) fn instance_phase(alpha_mode: &AlphaMode, double_sided: bool) -> usize {
	match alpha_mode {
		AlphaMode::Opaque if double_sided => 2,
		AlphaMode::Opaque => 0,
		AlphaMode::Mask(_) if double_sided => 3,
		AlphaMode::Mask(_) => 1,
		AlphaMode::Blend => TRANSPARENT_PHASE,
	}
}

/// Returns how many lanes rebuild `entities` render entities, at most `max_lanes`.
pub(super) fn rebuild_lanes(entities: usize, max_lanes: usize) -> usize {
	(entities / ENTITIES_PER_LANE).clamp(1, max_lanes.max(1))
}

/// The `Admission` struct is what the frame knows about an entity it draws: its phase and evaluation slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Admission {
	pub(super) phase: usize,
	pub(super) evaluation: u32,
	pub(super) double_sided: bool,
}

/// The `InstanceSources` struct holds what every lane reads while it rebuilds instances.
///
/// `admit` decides whether an entity enters the frame and how, so this module doesn't see the manager's material
/// tables. Build one per frame and hand it to [`rebuild_instances`].
pub(super) struct InstanceSources<'a, A> {
	pub(super) entities: &'a StableVec<RenderEntity>,
	pub(super) admit: A,
	pub(super) lod_view: Option<LodView>,
	pub(super) residencies: &'a MeshResidencies,
	pub(super) parameters: &'a MaterialParameters,
	pub(super) traces_rays: bool,
}

/// The `Deferred` struct names an instance a lane couldn't finish, by its entity slot, which is also its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Deferred {
	pub(super) slot: u32,
	/// The first vertex of the level the instance draws, which a newly posed instance copies its UVs from.
	pub(super) vertex_base: u32,
}

/// The `LaneScratch` struct is what one lane lists while it rebuilds, kept across frames so they reuse its room.
#[derive(Default)]
pub(super) struct LaneScratch {
	phases: [Vec<Instance>; PHASES],
	traced: Vec<TracedPrimitive>,
	/// The instances whose skinning or parameter block the caller finishes, in slot order.
	pub(super) deferred: Vec<Deferred>,
	masks: [ActiveEvaluationMask; 2],
}

/// The `InstanceOutputs` struct is where [`rebuild_instances`] writes the frame's instances.
///
/// `meshes` is the frame's instance table, whose length is the capacity. The vectors are emptied and refilled; the
/// masks gain the evaluation slots of the admitted instances. `lanes` keeps one scratch per lane across frames.
pub(super) struct InstanceOutputs<'a> {
	pub(super) meshes: &'a mut [ShaderMesh],
	/// Filled only when the sources trace rays.
	pub(super) traced: &'a mut Vec<TracedPrimitive>,
	/// The opaque, masked, double-sided, double-sided masked, and transparent instance lists.
	pub(super) phases: [&'a mut Vec<Instance>; PHASES],
	/// The opaque-pass and transparent evaluation masks.
	pub(super) masks: [&'a mut ActiveEvaluationMask; 2],
	pub(super) lanes: &'a mut Vec<LaneScratch>,
}

/// The `Share` struct is one lane's part of a rebuild: a range of slots and the outputs they fill.
struct Share<'a> {
	slots: Range<usize>,
	meshes: &'a mut [ShaderMesh],
	scratch: &'a mut LaneScratch,
}

/// Rebuilds the frame's instances from `sources` on up to `lanes` of `alley`'s lanes and returns how many admitted
/// instances didn't fit in the instance table.
///
/// Entities whose slot lies past the table are left out, so the outputs are the same for every lane count. Next,
/// finish the instances each of `outputs.lanes` lists in [`LaneScratch::deferred`], lane by lane, which may need to
/// write shared state.
pub(super) fn rebuild_instances<A>(
	alley: &mut Alley,
	sources: &InstanceSources<'_, A>,
	lanes: usize,
	outputs: InstanceOutputs<'_>,
) -> usize
where
	A: Fn(&RenderEntity) -> Option<Admission> + Sync,
{
	let InstanceOutputs {
		meshes,
		traced,
		phases,
		masks,
		lanes: scratches,
	} = outputs;
	let slots = sources.entities.slots_len().min(meshes.len());
	scratches.resize_with(lanes, LaneScratch::default);

	// Every lane takes a balanced range of the slots and the same range of the table.
	let mut shares = SmallVec::<[Share; 16]>::new();
	{
		let mut meshes = &mut meshes[..slots];
		for (lane, scratch) in scratches.iter_mut().enumerate() {
			let range = slots * lane / lanes..slots * (lane + 1) / lanes;
			let share_meshes;
			(share_meshes, meshes) = std::mem::take(&mut meshes).split_at_mut(range.len());
			shares.push(Share {
				slots: range,
				meshes: share_meshes,
				scratch,
			});
		}
	}
	alley.for_each_mut(&mut shares, lanes, |_, shares| {
		for share in shares {
			write_instances(share, sources);
		}
	});
	drop(shares);

	// Join the lanes' lists in lane order, which is slot order.
	for (phase, list) in phases.into_iter().enumerate() {
		list.clear();
		for scratch in scratches.iter() {
			list.extend_from_slice(&scratch.phases[phase]);
		}
	}
	traced.clear();
	for scratch in scratches.iter() {
		traced.extend_from_slice(&scratch.traced);
	}
	let [opaque_mask, transparent_mask] = masks;
	for scratch in scratches.iter() {
		for (mask, lane_mask) in [&mut *opaque_mask, &mut *transparent_mask].into_iter().zip(&scratch.masks) {
			for (word, lane_word) in mask.iter_mut().zip(lane_mask) {
				*word |= lane_word;
			}
		}
	}

	// Only a scene with more slots than the table holds leaves instances out, so only then are the rest counted.
	(slots..sources.entities.slots_len())
		.filter_map(|slot| sources.entities.get_slot(slot))
		.filter(|entity| (sources.admit)(entity).is_some())
		.count()
}

/// Writes the record of every instance the slots of `share` admit.
///
/// Each instance draws the detail level the sources' view picks for it, and its wanted level goes to the residencies.
fn write_instances<A>(share: &mut Share<'_>, sources: &InstanceSources<'_, A>)
where
	A: Fn(&RenderEntity) -> Option<Admission>,
{
	let scratch = &mut *share.scratch;
	for list in &mut scratch.phases {
		list.clear();
	}
	scratch.traced.clear();
	scratch.deferred.clear();
	scratch.masks = [EMPTY_MASK; 2];
	for (offset, slot) in share.slots.clone().enumerate() {
		let admitted = sources
			.entities
			.get_slot(slot)
			.and_then(|entity| Some((entity, (sources.admit)(entity)?)));
		let Some((entity, admission)) = admitted else {
			continue;
		};
		assert!(
			(admission.evaluation as usize) < MAX_MATERIALS,
			"Visibility evaluation slot is out of range. The most likely cause is that an active primitive's material is not in any material evaluation list."
		);

		let mut shader_mesh = entity.shader_mesh;
		let block = sources.parameters.current_block(entity.handle, shader_mesh.material_index());
		shader_mesh.set_parameter_block(block.unwrap_or(0));
		// Naming another level's record is all a level switch takes. Every view of the frame, shadows included, then
		// draws this level.
		// A primitive that takes over from others of its mesh, such as a billboard, shows only past their range.
		let takes_over = entity.lods.takes_over();
		let wanted = match sources.lod_view {
			Some(view) => {
				let wanted = view.select(&entity.lods, &shader_mesh.model, entity.bounding_sphere);
				// A merged mesh builds the level it's asked for, so one wholly out of view asks for none. It still
				// draws the level it holds, which other views may see, until that level's room is needed.
				let unseen =
					matches!(entity.mesh, MeshKey::Merged(_)) && !view.sees(&shader_mesh.model, entity.bounding_sphere);
				if let (Some(wanted), Some(slot)) = (wanted, entity.residency.filter(|_| !unseen)) {
					sources.residencies.want(slot, wanted as u8);
				}
				wanted
			}
			None => (!takes_over).then_some(0),
		};
		// Rays hit a primitive that others replace, at its coarsest level, wherever it is and never the ones that
		// replace it, so an object is hit once whichever of them is drawn. Such a primitive keeps its record while
		// it's hidden.
		let traced = sources.traces_rays && !takes_over;
		if wanted.is_none() && !traced {
			continue;
		}
		// A merged mesh holds no level until the one it wants is built, so its instance waits out the frame.
		let Some(lod) = entity.lods.drawn(wanted.unwrap_or(usize::MAX)) else {
			continue;
		};
		// Sidedness comes from the currently loaded material, so a reloaded material takes effect next frame.
		shader_mesh.set_level(lod.level, if admission.double_sided { MESH_FLAG_DOUBLE_SIDED } else { 0 });
		// A posed skinned instance gives this up when the caller finishes it.
		shader_mesh.set_lightmap(entity.lightmap);
		if entity.skinning.is_some() || block.is_none() {
			scratch.deferred.push(Deferred {
				slot: slot as u32,
				vertex_base: lod.vertex_base,
			});
		}

		share.meshes[offset] = shader_mesh;
		if traced {
			scratch.traced.push(TracedPrimitive {
				renderable: entity.handle,
				mesh: entity.mesh,
				primitive: entity.primitive,
				record: slot as u32,
				model: shader_mesh.model,
			});
		}
		if wanted.is_none() {
			continue;
		}
		scratch.phases[admission.phase].push(Instance {
			shader_mesh_index: slot as u32,
			meshlet_count: lod.meshlet_count,
		});
		let mask = &mut scratch.masks[usize::from(admission.phase == TRANSPARENT_PHASE)];
		mask[admission.evaluation as usize / u64::BITS as usize] |= 1 << (admission.evaluation % u64::BITS);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::factory::Handle;
	use crate::rendering::pipelines::visibility::lod::{MeshLod, MeshLods};
	use crate::rendering::pipelines::visibility::shader_data::InstanceLightmap;
	use crate::rendering::renderable::mesh::MeshKey;

	/// The material slots of the fixture scene: one per phase, then one whose entities never enter the frame.
	const MATERIALS: u32 = PHASES as u32 + 1;

	/// A scene of `count` entities whose material slot cycles through [`MATERIALS`], with every seventh slot removed.
	fn scene(count: u32) -> StableVec<RenderEntity> {
		let mut graph = utils::AvailabilityGraph::<u32>::new();
		let mut entities = StableVec::new();
		let mut removed = Vec::new();
		for index in 0..count {
			let handle = entities.push(RenderEntity {
				handle: Handle::from_id(index + 1),
				mesh: MeshKey::Generated(u64::from(index % 3)),
				primitive: 0,
				availability: graph.get_or_insert(index, true),
				shader_mesh: ShaderMesh::new(bytemuck::Zeroable::zeroed(), index % MATERIALS),
				bounding_sphere: [0.0; 4],
				lods: MeshLods::new(&[MeshLod {
					level: index,
					meshlet_count: 1 + index % 40,
					resident: true,
					..MeshLod::default()
				}]),
				residency: None,
				skinning: None,
				lightmap: InstanceLightmap::NONE,
			});
			if index % 7 == 3 {
				removed.push(handle);
			}
		}
		for handle in removed {
			entities.remove(handle);
		}
		entities
	}

	/// Admits an entity unless its material is the last slot; the slot picks the phase and the evaluation.
	fn admit(entity: &RenderEntity) -> Option<Admission> {
		let material = entity.shader_mesh.material_index();
		(material < PHASES as u32).then_some(Admission {
			phase: material as usize,
			evaluation: material * 3,
			double_sided: material >= 2,
		})
	}

	/// The `Rebuilt` struct is everything one rebuild wrote that the frame reads: the records the phase lists name.
	#[derive(Debug, PartialEq)]
	struct Rebuilt {
		phases: [Vec<(Instance, [u32; 3])>; PHASES],
		masks: [ActiveEvaluationMask; 2],
		left_out: usize,
	}

	/// Rebuilds `entities` on `lanes` lanes into an instance table of `capacity` records.
	fn rebuild(entities: &StableVec<RenderEntity>, lanes: usize, capacity: usize) -> Rebuilt {
		let mut alley = Alley::with_parallelism(lanes);
		let residencies = MeshResidencies::default();
		let parameters = MaterialParameters::new();
		let sources = InstanceSources {
			entities,
			admit,
			lod_view: None,
			residencies: &residencies,
			parameters: &parameters,
			traces_rays: false,
		};
		let mut meshes = vec![bytemuck::Zeroable::zeroed(); capacity];
		let (mut traced, mut scratches) = (Vec::new(), Vec::new());
		let mut phases: [Vec<Instance>; PHASES] = Default::default();
		let mut masks = [EMPTY_MASK; 2];
		let [opaque, masked, double_sided, double_sided_masked, transparent] = &mut phases;
		let [opaque_mask, transparent_mask] = &mut masks;
		let left_out = rebuild_instances(
			&mut alley,
			&sources,
			lanes,
			InstanceOutputs {
				meshes: &mut meshes,
				traced: &mut traced,
				phases: [opaque, masked, double_sided, double_sided_masked, transparent],
				masks: [opaque_mask, transparent_mask],
				lanes: &mut scratches,
			},
		);
		Rebuilt {
			phases: phases.map(|list| {
				list.into_iter()
					.map(|instance| {
						let mesh: &ShaderMesh = &meshes[instance.shader_mesh_index as usize];
						(instance, [mesh.material_index(), mesh.level(), mesh.flags()])
					})
					.collect()
			}),
			masks,
			left_out,
		}
	}

	/// The specification: an entity in a slot below the capacity that the frame admits is the instance of that index,
	/// and each phase lists its instances in slot order.
	fn expected(entities: &StableVec<RenderEntity>, capacity: usize) -> Rebuilt {
		let mut rebuilt = Rebuilt {
			phases: Default::default(),
			masks: [EMPTY_MASK; 2],
			left_out: 0,
		};
		for (slot, entity) in entities.indexed_iter() {
			let Some(admission) = admit(entity) else {
				continue;
			};
			if slot >= capacity {
				rebuilt.left_out += 1;
				continue;
			}
			let flags = if admission.double_sided { MESH_FLAG_DOUBLE_SIDED } else { 0 };
			let Some(lod) = entity.lods.drawn(0) else {
				continue;
			};
			rebuilt.phases[admission.phase].push((
				Instance {
					shader_mesh_index: slot as u32,
					meshlet_count: lod.meshlet_count,
				},
				[entity.shader_mesh.material_index(), lod.level, flags],
			));
			let mask = &mut rebuilt.masks[usize::from(admission.phase == TRANSPARENT_PHASE)];
			mask[admission.evaluation as usize / 64] |= 1 << (admission.evaluation % 64);
		}
		rebuilt
	}

	#[test]
	fn every_lane_count_rebuilds_the_instances_in_slot_order() {
		let entities = scene(40_000);
		let capacity = 50_000;

		for lanes in [1, 2, 4, 8] {
			assert_eq!(
				rebuild(&entities, lanes, capacity),
				expected(&entities, capacity),
				"{lanes} lanes"
			);
		}
	}

	#[test]
	fn slots_past_the_table_are_left_out() {
		let entities = scene(40_000);
		let capacity = 25_001;

		for lanes in [1, 2, 4, 8] {
			let rebuilt = rebuild(&entities, lanes, capacity);
			assert!(rebuilt.left_out > 0, "{lanes} lanes");
			assert_eq!(rebuilt, expected(&entities, capacity), "{lanes} lanes");
		}
	}
}
