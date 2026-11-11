//! Turns the scene's render entities into the frame's instance records on several alley lanes.
//!
//! An instance's index is its entity's slot in [`super::scene::VisibilityScene::render_entities`], so every lane
//! writes the records of its own range of slots, in one pass, without knowing what the other lanes admit. Slots of
//! entities the frame doesn't draw keep stale records that no work item names. Each lane lists its instances per
//! raster phase in slot order, and the lists join in lane order, so the output is the same for every lane count. Work
//! that changes shared state, a skinned instance's vertices and palette or a parameter block that must be written,
//! can't run on a lane: the lane writes the rest of the record and lists the instance in [`LaneScratch::deferred`]
//! for the caller to finish in order.
//!
//! A rebuild reads the scene's entities, their loaded resources and materials, and the detail view. The visibility
//! manager rebuilds only on a frame after one of those changed, once for each frame sequence's copy of the instance
//! table, and otherwise draws the instances it has. A frame that defers an instance is never repeated.

use std::ops::Range;

use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::StableVec;

use super::acceleration_structures::TracedPrimitive;
use super::layout::{ActiveEvaluationMask, MAX_MATERIALS};
use super::lod::{Held, LodView};
use super::material_parameters::MaterialParameters;
use super::residency::{MeshResidencies, ResidencySlot};
use super::scene::{Instance, RenderEntity};
use super::shader_data::{MESH_FLAG_DOUBLE_SIDED, MESH_FLAG_HIDDEN, ShaderMesh};
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

/// Returns whether rays hit the instances of raster phase `phase`: the opaque ones. Rays pass through masked and
/// blended materials, whose primitives no bottom-level acceleration structure holds.
pub(super) fn rays_hit_phase(phase: usize) -> bool {
	matches!(phase, 0 | 2)
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

/// The `Kept` struct is what a rebuild remembers of one entity slot, so a later frame can follow the camera without
/// rebuilding: the entity's pick and how long it holds, and the phase that lists its instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Kept {
	/// The level the entity wants and the camera distances over which it keeps wanting it.
	held: Held,
	/// Where the wanted level is recorded each frame.
	residency: Option<ResidencySlot>,
	/// The phase list that holds the entity's instance, or [`PHASES`] for a slot the frame doesn't draw.
	phase: u8,
}

impl Kept {
	/// A slot without an instance: an empty one, or an entity the frame doesn't admit.
	const ABSENT: Self = Self {
		held: Held::always(None),
		residency: None,
		phase: PHASES as u8,
	};

	/// Gives up the slot's pick, so the next [`follow_instances`] picks its entity again and rewrites its record.
	/// Call it for an entity whose transform changed.
	pub(super) fn release(&mut self) {
		if self.phase as usize != PHASES {
			self.held = Held::never(self.held.wanted);
		}
	}
}

/// The `LaneScratch` struct is what one lane lists while it rebuilds, kept across frames so they reuse its room.
#[derive(Default)]
pub(super) struct LaneScratch {
	phases: [Vec<Instance>; PHASES],
	traced: Vec<TracedPrimitive>,
	/// The instances whose skinning or parameter block the caller finishes, in slot order.
	pub(super) deferred: Vec<Deferred>,
	masks: [ActiveEvaluationMask; 2],
	/// The slots whose records [`follow_instances`] changed, from the first to one past the last.
	rewritten: Range<usize>,
	/// Whether [`follow_instances`] met an entity it can't follow, so the frame has to rebuild.
	outdated: bool,
}

/// The `InstanceOutputs` struct is where [`rebuild_instances`] writes the instances and [`follow_instances`] patches
/// them.
///
/// `meshes` is the instance table, whose length is the capacity. A rebuild empties and refills the vectors, and the
/// masks gain the evaluation slots of the admitted instances. `lanes` keeps one scratch per lane across frames, and
/// `kept` what a later frame follows the camera with.
pub(super) struct InstanceOutputs<'a> {
	pub(super) meshes: &'a mut [ShaderMesh],
	/// Filled only when the sources trace rays.
	pub(super) traced: &'a mut Vec<TracedPrimitive>,
	/// The opaque, masked, double-sided, double-sided masked, and transparent instance lists. Each holds every
	/// admitted instance of its phase, in slot order, with no meshlets for one that isn't drawn from here.
	pub(super) phases: [&'a mut Vec<Instance>; PHASES],
	/// The opaque-pass and transparent evaluation masks.
	pub(super) masks: [&'a mut ActiveEvaluationMask; 2],
	pub(super) lanes: &'a mut Vec<LaneScratch>,
	pub(super) kept: &'a mut Vec<Kept>,
}

/// The `Share` struct is one lane's part of a rebuild: a range of slots and the outputs they fill.
struct Share<'a> {
	slots: Range<usize>,
	meshes: &'a mut [ShaderMesh],
	kept: &'a mut [Kept],
	scratch: &'a mut LaneScratch,
}

/// Gives every lane a balanced range of the first `slots` slots, with the same range of the table and of `kept`.
fn shares<'a>(
	slots: usize,
	lanes: usize,
	meshes: &'a mut [ShaderMesh],
	kept: &'a mut [Kept],
	scratches: &'a mut [LaneScratch],
) -> SmallVec<[Share<'a>; 16]> {
	let (mut meshes, mut kept) = (&mut meshes[..slots], &mut kept[..slots]);
	let mut shares = SmallVec::new();
	for (lane, scratch) in scratches.iter_mut().enumerate() {
		let range = slots * lane / lanes..slots * (lane + 1) / lanes;
		let (share_meshes, share_kept);
		(share_meshes, meshes) = std::mem::take(&mut meshes).split_at_mut(range.len());
		(share_kept, kept) = std::mem::take(&mut kept).split_at_mut(range.len());
		shares.push(Share {
			slots: range,
			meshes: share_meshes,
			kept: share_kept,
			scratch,
		});
	}
	shares
}

/// Rebuilds the instances from `sources` on up to `lanes` of `alley`'s lanes and returns how many admitted instances
/// didn't fit in the instance table.
///
/// Entities whose slot lies past the table are left out, so the outputs are the same for every lane count. Next,
/// finish the instances each of `outputs.lanes` lists in [`LaneScratch::deferred`], lane by lane, which may need to
/// write shared state. While nothing but the camera changes afterward, [`follow_instances`] keeps them current.
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
		kept,
	} = outputs;
	let slots = sources.entities.slots_len().min(meshes.len());
	scratches.resize_with(lanes, LaneScratch::default);
	kept.clear();
	kept.resize(slots, Kept::ABSENT);

	let mut shares = shares(slots, lanes, meshes, kept, scratches);
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

/// The `Followed` struct is what [`follow_instances`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Followed {
	/// The slots whose records changed, from the first to one past the last. Empty when none did.
	pub(super) records: Range<usize>,
	/// Whether the phases list other instances than before, so the frames' copies of the lists are out of date.
	/// Following never does: an instance that stops drawing keeps its place.
	pub(super) relisted: bool,
	/// Whether an entity changed in a way only [`rebuild_instances`] handles. The instances are then partly
	/// followed: rebuild them before drawing.
	pub(super) outdated: bool,
}

/// Brings rebuilt instances up to date with a camera that moved and with entities that did, when nothing else changed
/// since `outputs` were rebuilt or last followed: no other part of an entity, no resource or material, and no term of
/// the view's error scale. [`Kept::release`] the slot of each entity that moved first.
///
/// Each entity's pick says how far the camera can move before it changes, so most entities cost one distance. The
/// ones whose pick expired are picked again and their records rewritten. The lists stay as they are: the GPU counts
/// each instance's meshlets from its record. Every entity's wanted level goes to the residencies, as in a rebuild.
pub(super) fn follow_instances<A>(
	alley: &mut Alley,
	sources: &InstanceSources<'_, A>,
	lanes: usize,
	outputs: InstanceOutputs<'_>,
) -> Followed
where
	A: Fn(&RenderEntity) -> Option<Admission> + Sync,
{
	let InstanceOutputs {
		meshes,
		traced,
		lanes: scratches,
		kept,
		..
	} = outputs;
	scratches.resize_with(lanes, LaneScratch::default);
	let mut shares = shares(kept.len(), lanes, meshes, kept, scratches);
	alley.for_each_mut(&mut shares, lanes, |_, shares| {
		for share in shares {
			follow_share(share, sources);
		}
	});
	drop(shares);

	let mut followed = Followed::default();
	for scratch in scratches.iter() {
		followed.outdated |= scratch.outdated;
		if !scratch.rewritten.is_empty() {
			followed.records = if followed.records.is_empty() {
				scratch.rewritten.clone()
			} else {
				followed.records.start.min(scratch.rewritten.start)..followed.records.end.max(scratch.rewritten.end)
			};
		}
		// Rays hit a moved primitive where it is now. Few primitives move in a frame, so each is looked up.
		for moved in &scratch.traced {
			if let Some(primitive) = traced.iter_mut().find(|primitive| primitive.record == moved.record) {
				primitive.model = moved.model;
			}
		}
	}
	followed
}

/// Follows the camera for the slots of `share`: records every held pick's want, and picks the expired ones again.
fn follow_share<A>(share: &mut Share<'_>, sources: &InstanceSources<'_, A>)
where
	A: Fn(&RenderEntity) -> Option<Admission>,
{
	let view = sources
		.lod_view
		.expect("Instances were followed without a camera. The most likely cause is a frame without a sink that follows a detail view instead of rebuilding.");
	let scratch = &mut *share.scratch;
	scratch.traced.clear();
	scratch.rewritten = 0..0;
	scratch.outdated = false;
	for (offset, slot) in share.slots.clone().enumerate() {
		let kept = &mut share.kept[offset];
		if kept.held.holds(&view) {
			if let (Some(wanted), Some(residency)) = (kept.held.wanted, kept.residency) {
				sources.residencies.want(residency, wanted as u8);
			}
			continue;
		}
		// The entity and its phase are what the rebuild saw unless something other than the camera changed.
		let admitted = sources
			.entities
			.get_slot(slot)
			.and_then(|entity| Some((entity, (sources.admit)(entity)?)))
			.filter(|(_, admission)| admission.phase == kept.phase as usize);
		let Some((entity, admission)) = admitted else {
			scratch.outdated = true;
			continue;
		};
		let picked = pick(entity, admission, slot, sources);
		// An instance that needs finishing in order is beyond a patch.
		scratch.outdated |= picked.deferred.is_some();
		if bytemuck::bytes_of(&share.meshes[offset]) != bytemuck::bytes_of(&picked.record) {
			if picked.traced && share.meshes[offset].model != picked.record.model {
				scratch.traced.push(TracedPrimitive {
					renderable: entity.handle,
					mesh: entity.mesh,
					primitive: entity.primitive,
					record: slot as u32,
					model: picked.record.model,
				});
			}
			share.meshes[offset] = picked.record;
			scratch.rewritten = if scratch.rewritten.is_empty() {
				slot..slot + 1
			} else {
				scratch.rewritten.start..slot + 1
			};
		}
		kept.held = picked.held;
	}
}

/// The `Picked` struct is one admitted entity's instance for the current view.
struct Picked {
	/// The instance's record, or `None` for an entity that is neither drawn nor hit by rays from here.
	record: ShaderMesh,
	/// The meshlets the instance draws: none when it isn't drawn.
	meshlets: u32,
	held: Held,
	/// Set for an instance whose skinning or parameter block the caller finishes.
	deferred: Option<Deferred>,
	/// Whether rays hit the instance.
	traced: bool,
}

/// Picks the detail level `entity` draws from the sources' view, records the level it wants with the residencies, and
/// returns its instance. `slot` is the entity's slot and `admission` what the frame admits it with.
fn pick<A>(entity: &RenderEntity, admission: Admission, slot: usize, sources: &InstanceSources<'_, A>) -> Picked {
	assert!(
		(admission.evaluation as usize) < MAX_MATERIALS,
		"Visibility evaluation slot is out of range. The most likely cause is that an active primitive's material is not in any material evaluation list."
	);

	let mut shader_mesh = entity.shader_mesh;
	let block = sources.parameters.current_block(entity.handle, shader_mesh.material_index());
	shader_mesh.set_parameter_block(block.unwrap_or(0));
	// A primitive that takes over from others of its mesh, such as a billboard, shows only past their range.
	let takes_over = entity.lods.takes_over();
	let held = match sources.lod_view {
		Some(view) => {
			let held = view.held(&entity.lods, &shader_mesh.model, entity.bounding_sphere);
			// A merged mesh builds the level it's asked for, so one wholly out of view asks for none. It still
			// draws the level it holds, which other views may see, until that level's room is needed.
			let merged = matches!(entity.mesh, MeshKey::Merged(_));
			let unseen = merged && !view.sees(&shader_mesh.model, entity.bounding_sphere);
			if let (Some(wanted), Some(slot)) = (held.wanted, entity.residency.filter(|_| !unseen)) {
				sources.residencies.want(slot, wanted as u8);
			}
			// What a merged mesh asks for follows what the camera sees, not only how far it is, so its pick is
			// made again every frame.
			if merged { Held::never(held.wanted) } else { held }
		}
		None => Held::always((!takes_over).then_some(0)),
	};
	let wanted = held.wanted;
	// Rays hit a primitive that others replace, at its coarsest level, wherever it is and never the ones that
	// replace it, so an object is hit once whichever of them is drawn. Such a primitive keeps its record while
	// it's hidden. Only opaque primitives are listed: rays pass through the rest, so listing them would make the
	// acceleration structures sort and look up instances they never place.
	let traced = sources.traces_rays && !takes_over && rays_hit_phase(admission.phase);
	// Naming another level's record is all a level switch takes. Every view of the frame, shadows included, then
	// draws this level. A merged mesh holds no level until the one it wants is built, so its instance waits.
	let lod = (wanted.is_some() || traced)
		.then(|| entity.lods.drawn(wanted.unwrap_or(usize::MAX)))
		.flatten();
	let Some(lod) = lod else {
		// The instance stays listed, so its record says that it draws nothing.
		shader_mesh.set_level(0, MESH_FLAG_HIDDEN);
		return Picked {
			record: shader_mesh,
			meshlets: 0,
			held,
			deferred: None,
			traced: false,
		};
	};
	// Sidedness comes from the currently loaded material, so a reloaded material takes effect next frame.
	let sides = if admission.double_sided { MESH_FLAG_DOUBLE_SIDED } else { 0 };
	// The GPU counts the meshlets of every listed instance's level, so one that only rays hit is marked hidden.
	let hidden = if wanted.is_some() { 0 } else { MESH_FLAG_HIDDEN };
	shader_mesh.set_level(lod.level, sides | hidden);
	// A posed skinned instance gives this up when the caller finishes it.
	shader_mesh.set_lightmap(entity.lightmap);
	Picked {
		record: shader_mesh,
		meshlets: if wanted.is_some() { lod.meshlet_count } else { 0 },
		held,
		deferred: (entity.skinning.is_some() || block.is_none()).then_some(Deferred {
			slot: slot as u32,
			vertex_base: lod.vertex_base,
		}),
		traced,
	}
}

/// Writes the record of every instance the slots of `share` admit, and what a later frame follows them with.
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
		let picked = pick(entity, admission, slot, sources);
		share.meshes[offset] = picked.record;
		if picked.traced {
			scratch.traced.push(TracedPrimitive {
				renderable: entity.handle,
				mesh: entity.mesh,
				primitive: entity.primitive,
				record: slot as u32,
				model: picked.record.model,
			});
		}
		scratch.deferred.extend(picked.deferred);
		// An instance that isn't drawn from here keeps its place in the list, so following the camera only changes
		// records. Its material's evaluation stays active, and finds no pixels.
		scratch.phases[admission.phase].push(Instance {
			shader_mesh_index: slot as u32,
			meshlet_count: picked.meshlets,
		});
		let mask = &mut scratch.masks[usize::from(admission.phase == TRANSPARENT_PHASE)];
		mask[admission.evaluation as usize / u64::BITS as usize] |= 1 << (admission.evaluation % u64::BITS);
		share.kept[offset] = Kept {
			held: picked.held,
			residency: entity.residency,
			phase: admission.phase as u8,
		};
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
		/// Each listed instance's slot, with its record's material, level, and flags, and its transform.
		phases: [Vec<(u32, [u32; 3], ghi::pod::Mat4x3f)>; PHASES],
		masks: [ActiveEvaluationMask; 2],
		/// The record of each primitive listed for the acceleration structures, and where rays hit it.
		traced: Vec<(u32, ghi::pod::Mat4x3f)>,
		left_out: usize,
	}

	/// The `Rig` struct holds one scene's instances between rebuilds, as the visibility manager does.
	struct Rig {
		alley: Alley,
		lanes: usize,
		residencies: MeshResidencies,
		meshes: Vec<ShaderMesh>,
		traced: Vec<TracedPrimitive>,
		phases: [Vec<Instance>; PHASES],
		masks: [ActiveEvaluationMask; 2],
		scratches: Vec<LaneScratch>,
		kept: Vec<Kept>,
		left_out: usize,
	}

	impl Rig {
		fn new(lanes: usize, capacity: usize, residencies: MeshResidencies) -> Self {
			Self {
				alley: Alley::with_parallelism(lanes),
				lanes,
				residencies,
				meshes: vec![bytemuck::Zeroable::zeroed(); capacity],
				traced: Vec::new(),
				phases: Default::default(),
				masks: [EMPTY_MASK; 2],
				scratches: Vec::new(),
				kept: Vec::new(),
				left_out: 0,
			}
		}

		/// Rebuilds the instances of `entities` for `view`, or follows the camera to it with `follow`, and returns
		/// what following changed.
		fn update(&mut self, entities: &StableVec<RenderEntity>, view: Option<LodView>, follow: bool) -> Followed {
			let parameters = MaterialParameters::new();
			let sources = InstanceSources {
				entities,
				admit,
				lod_view: view,
				residencies: &self.residencies,
				parameters: &parameters,
				traces_rays: true,
			};
			let [opaque, masked, double_sided, double_sided_masked, transparent] = &mut self.phases;
			let [opaque_mask, transparent_mask] = &mut self.masks;
			let outputs = InstanceOutputs {
				meshes: &mut self.meshes,
				traced: &mut self.traced,
				phases: [opaque, masked, double_sided, double_sided_masked, transparent],
				masks: [opaque_mask, transparent_mask],
				lanes: &mut self.scratches,
				kept: &mut self.kept,
			};
			if follow {
				follow_instances(&mut self.alley, &sources, self.lanes, outputs)
			} else {
				self.left_out = rebuild_instances(&mut self.alley, &sources, self.lanes, outputs);
				Followed::default()
			}
		}

		/// Returns everything the frame reads.
		fn rebuilt(&self) -> Rebuilt {
			Rebuilt {
				phases: self.phases.clone().map(|list| {
					list.into_iter()
						.map(|instance| {
							let mesh: &ShaderMesh = &self.meshes[instance.shader_mesh_index as usize];
							let record = [mesh.material_index(), mesh.level(), mesh.flags()];
							(instance.shader_mesh_index, record, mesh.model)
						})
						.collect()
				}),
				masks: self.masks,
				traced: self
					.traced
					.iter()
					.map(|primitive| (primitive.record, primitive.model))
					.collect(),
				left_out: self.left_out,
			}
		}

		/// Closes the frame and returns the detail levels it asks to load, by primitive. The loads end without
		/// bringing anything, so the next frame asks again for what it wants.
		fn loads(&mut self) -> Vec<(u32, u8)> {
			let mut loads = Vec::new();
			let mut meshes = Vec::new();
			self.residencies.end_frame(|key, primitives| {
				loads.extend(primitives.iter().map(|levels| (levels.primitive, levels.finest)));
				meshes.push(key);
				true
			});
			for key in meshes {
				self.residencies.load_finished(key, Vec::new());
			}
			loads
		}
	}

	/// Rebuilds `entities` on `lanes` lanes into an instance table of `capacity` records.
	fn rebuild(entities: &StableVec<RenderEntity>, lanes: usize, capacity: usize) -> Rebuilt {
		let mut rig = Rig::new(lanes, capacity, MeshResidencies::default());
		rig.update(entities, None, false);
		rig.rebuilt()
	}

	/// The specification: an entity in a slot below the capacity that the frame admits is the instance of that index,
	/// and each phase lists its instances in slot order.
	fn expected(entities: &StableVec<RenderEntity>, capacity: usize) -> Rebuilt {
		let mut rebuilt = Rebuilt {
			phases: Default::default(),
			masks: [EMPTY_MASK; 2],
			traced: Vec::new(),
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
				slot as u32,
				[entity.shader_mesh.material_index(), lod.level, flags],
				entity.shader_mesh.model,
			));
			let mask = &mut rebuilt.masks[usize::from(admission.phase == TRANSPARENT_PHASE)];
			mask[admission.evaluation as usize / 64] |= 1 << (admission.evaluation % 64);
			// Rays hit the opaque phases only: solid and double-sided.
			if admission.phase == 0 || admission.phase == 2 {
				rebuilt.traced.push((slot as u32, entity.shader_mesh.model));
			}
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
	/// A row of plants along +Z, each a plant primitive with three levels that a billboard primitive replaces, all
	/// instances of one mesh whose finer levels load as they are wanted.
	fn plants(count: u32, residencies: &MeshResidencies) -> StableVec<RenderEntity> {
		let mut graph = utils::AvailabilityGraph::<u32>::new();
		let mut entities = StableVec::new();
		let level = |level: u32, meshlet_count: u32, error: f32| MeshLod {
			level,
			meshlet_count,
			error,
			resident: true,
			..MeshLod::default()
		};
		let plant = MeshLods::new(&[level(0, 18, 0.0), level(1, 9, 0.02), level(2, 6, 0.05)]).replaced_at(Some(0.2));
		let billboard = MeshLods::new(&[level(3, 1, 0.2)]);
		for index in 0..count {
			let model = [
				[1.0, 0.0, 0.0],
				[0.0, 1.0, 0.0],
				[0.0, 0.0, 1.0],
				[0.0, 0.0, 3.0 + index as f32 * 0.5],
			];
			for (primitive, lods) in [plant, billboard].into_iter().enumerate() {
				entities.push(RenderEntity {
					handle: Handle::from_id(index + 1),
					mesh: MeshKey::Generated(1),
					primitive: primitive as u32,
					availability: graph.get_or_insert(index, true),
					// Material one is masked, as plants are.
					shader_mesh: ShaderMesh::new(model.into(), 1),
					bounding_sphere: [0.0, 0.0, 0.0, 0.5],
					lods,
					residency: residencies.slot(MeshKey::Generated(1), primitive as u32),
					skinning: None,
					lightmap: InstanceLightmap::NONE,
				});
			}
		}
		entities
	}

	#[test]
	fn following_the_camera_leaves_what_a_rebuild_from_there_would() {
		let residencies = || {
			let mut residencies = MeshResidencies::default();
			// The plant holds its coarsest level, and the billboard its only one.
			residencies.add_mesh(MeshKey::Generated(1), [2, 0]);
			residencies
		};
		let entities = plants(400, &residencies());
		// A pixel covers a hundredth of a unit at each unit of distance.
		let view = LodView::for_test(0.01);
		let eyes = [0.0, 0.25, 4.0, 40.0, 41.0, 120.0, 300.0, -50.0, -49.5];

		for lanes in [1, 4] {
			let mut followed = Rig::new(lanes, entities.slots_len(), residencies());
			followed.update(&entities, Some(view), false);
			followed.loads();
			let mut changes = 0;
			for z in eyes {
				let moved = view.moved_to(maths_rs::Vec3f::new(0.0, 0.0, z));
				let change = followed.update(&entities, Some(moved), true);
				assert!(!change.outdated);
				changes += usize::from(!change.records.is_empty());

				let mut rebuilt = Rig::new(lanes, entities.slots_len(), residencies());
				rebuilt.update(&entities, Some(moved), false);
				assert_eq!(followed.rebuilt(), rebuilt.rebuilt(), "{lanes} lanes, camera at {z}");
				// Both frames ask for the same detail.
				assert_eq!(followed.loads(), rebuilt.loads(), "{lanes} lanes, camera at {z}");

				// A camera that stays put changes nothing.
				assert_eq!(followed.update(&entities, Some(moved), true), Followed::default());
				followed.loads();
			}
			assert!(changes > 4, "the camera should cross detail switches: {changes}");
		}
	}

	#[test]
	fn following_a_moved_entity_leaves_what_a_rebuild_would() {
		let residencies = || {
			let mut residencies = MeshResidencies::default();
			residencies.add_mesh(MeshKey::Generated(1), [2, 0]);
			residencies
		};
		let mut entities = plants(60, &residencies());
		// Opaque plants, which rays hit.
		for (_, entity) in entities.indexed_iter_mut() {
			entity.shader_mesh = ShaderMesh::new(entity.shader_mesh.model, 0);
		}
		let view = LodView::for_test(0.01);

		for lanes in [1, 4] {
			let mut followed = Rig::new(lanes, entities.slots_len(), residencies());
			followed.update(&entities, Some(view), false);
			followed.loads();
			// Carry the nearest plant, both of its primitives, far enough for its billboard to take over.
			let far = [
				[1.0, 0.0, 0.0],
				[0.0, 1.0, 0.0],
				[0.0, 0.0, 1.0],
				[2.0, 0.0, 90.0 + lanes as f32],
			];
			let mut moved = Vec::new();
			for (slot, entity) in entities.indexed_iter_mut().take(2) {
				entity.shader_mesh.model = far.into();
				moved.push(slot);
			}
			for &slot in &moved {
				followed.kept[slot].release();
			}

			let change = followed.update(&entities, Some(view), true);

			assert_eq!(change.records, moved[0]..moved[1] + 1, "{lanes} lanes");
			assert!(!change.outdated && !change.relisted);
			let mut rebuilt = Rig::new(lanes, entities.slots_len(), residencies());
			rebuilt.update(&entities, Some(view), false);
			assert_eq!(followed.rebuilt(), rebuilt.rebuilt(), "{lanes} lanes");
			assert_eq!(followed.loads(), rebuilt.loads(), "{lanes} lanes");
		}
	}
}
