//! The acceleration structures surfel GI traces rays through: one bottom level per mesh, over its opaque primitives at
//! full detail, and a top level that places the frame's instances.
//!
//! The scene's rays hit what the Reference's ray-traced scene rules describe. Bottom levels build on the render
//! context, a frame's worth of triangles at a time, because the loader's copy queue can't build them; see the loading
//! design note. Each mesh's geometries take a run of consecutive hit records, which ray tracing pipelines map to the
//! hit shaders of the geometries' materials.
//!
//! Next, the surfel trace and shade passes bind [`SceneAccelerationStructures::descriptor_set`], which holds the top
//! level and the geometry table that leads a hit to the frame's mesh record of the primitive it hit and to that
//! primitive's triangles.

use std::collections::VecDeque;
use std::ops::Range;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::pod::Mat4x3f;
use resource_management::types::AlphaMode;
use utils::hash::HashMap;

use super::geometry::{GeometryHandles, MeshData};
use super::layout::MAX_INSTANCES;
use super::set_layouts;
use crate::core::factory::Handle;
use crate::rendering::renderable::mesh::MeshKey;

/// The triangles of bottom-level builds one frame records. Meshes past it wait for later frames, but a frame always
/// builds at least one mesh, so a mesh larger than the budget still builds.
pub(crate) const BOTTOM_LEVEL_BUILD_TRIANGLES_PER_FRAME: u32 = 262_144;
/// The bits of every instance's mask: rays that trace the scene use them all.
const SCENE_MASK: u8 = 0xFF;

/// The `TracedPrimitive` struct is one primitive the frame draws, as the acceleration structures need it.
///
/// The visibility pipeline manager lists one per admitted primitive while it rebuilds the frame's instances, and hands
/// the list to [`SceneAccelerationStructures::prepare`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct TracedPrimitive {
	pub(crate) renderable: Handle,
	pub(crate) mesh: MeshKey,
	/// The primitive's index in its mesh.
	pub(crate) primitive: u32,
	/// The frame's mesh record of the primitive, which hits on it read.
	pub(crate) record: u32,
	/// The renderable's object-to-world transform this frame.
	pub(crate) model: Mat4x3f,
}

/// The `PrimitiveGeometry` struct locates one primitive's full-detail triangles in the geometry buffers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PrimitiveGeometry {
	/// The material table slot, whose alpha mode decides whether rays hit the primitive.
	pub(crate) material: u32,
	/// The bind-pose vertices in the vertex position buffer.
	pub(crate) vertices: Range<u32>,
	/// The triangles in the flat triangle buffer, as indices relative to the first vertex. Empty for a skinned
	/// primitive, so rays pass through the shapes skinning deforms.
	pub(crate) triangles: Range<u32>,
}

impl PrimitiveGeometry {
	/// Returns where each primitive of `mesh` keeps its full-detail triangles.
	pub(crate) fn of_mesh(mesh: &MeshData) -> Vec<Self> {
		mesh.primitives
			.iter()
			.map(|primitive| {
				let vertices = mesh.vertex_offset + primitive.vertex_offset;
				let triangles = mesh.flat_triangle_offset + primitive.flat_triangle_offset;
				Self {
					material: primitive.material_index,
					vertices: vertices..vertices + primitive.vertex_count,
					triangles: triangles..triangles + primitive.flat_triangle_count,
				}
			})
			.collect()
	}
}

/// Returns the primitives of a mesh that rays hit, in geometry order: those with triangles whose material is opaque.
///
/// Masked and blended materials are left out, so rays pass through foliage and glass instead of hitting the whole of
/// each triangle. `alpha_mode` returns a material slot's alpha mode.
pub(crate) fn opaque_geometries<'a>(primitives: &[PrimitiveGeometry], alpha_mode: impl Fn(u32) -> &'a AlphaMode) -> Vec<u32> {
	(0..primitives.len() as u32)
		.filter(|&index| {
			let primitive = &primitives[index as usize];
			!primitive.triangles.is_empty() && matches!(alpha_mode(primitive.material), AlphaMode::Opaque)
		})
		.collect()
}

/// Returns how many of the queued builds, whose triangle counts `queued` lists in arrival order, fit in `budget`
/// triangles. The first always fits, so a mesh larger than the budget builds alone.
pub(crate) fn builds_within_budget(queued: impl IntoIterator<Item = u32>, budget: u32) -> usize {
	let mut triangles = 0u64;
	queued
		.into_iter()
		.enumerate()
		.take_while(|&(index, count)| {
			triangles += u64::from(count);
			index == 0 || triangles <= u64::from(budget)
		})
		.count()
}

/// The `BuildQueue` struct orders the bottom-level builds: meshes wait in arrival order, each frame takes the builds
/// that fit its budget, and a frame that doesn't record its builds gives them back to the front.
#[derive(Default)]
pub(crate) struct BuildQueue {
	/// Meshes waiting for their build, with their triangle counts.
	waiting: VecDeque<(MeshKey, u32)>,
	/// The builds the last frame took, until [`Self::settle`] says whether it recorded them.
	taken: Vec<(MeshKey, u32)>,
}

impl BuildQueue {
	pub(crate) fn push(&mut self, mesh: MeshKey, triangles: u32) {
		self.waiting.push_back((mesh, triangles));
	}

	/// Takes the builds that fit in `budget` triangles, in arrival order. Next, record them and call [`Self::settle`].
	pub(crate) fn take(&mut self, budget: u32) -> &[(MeshKey, u32)] {
		let count = builds_within_budget(self.waiting.iter().map(|(_, triangles)| *triangles), budget);
		self.taken.clear();
		self.taken.extend(self.waiting.drain(..count));
		&self.taken
	}

	/// Settles the taken builds: they are done when `recorded`, and otherwise go back to the front of the queue, in
	/// order, each passed to `unbuilt`.
	pub(crate) fn settle(&mut self, recorded: bool, mut unbuilt: impl FnMut(MeshKey)) {
		if recorded {
			self.taken.clear();
			return;
		}
		for (mesh, triangles) in self.taken.drain(..).rev() {
			unbuilt(mesh);
			self.waiting.push_front((mesh, triangles));
		}
	}
}

/// The `HitGeometry` struct is one geometry of a bottom level, as hits on it look it up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HitGeometry {
	/// The mesh primitive the geometry holds.
	pub(crate) primitive: u32,
	/// The geometry's first triangle in the flat triangle buffer.
	pub(crate) first_triangle: u32,
}

/// The `BottomLevel` struct is one mesh's bottom level and the hit records of its geometries.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BottomLevel<S> {
	pub(crate) structure: S,
	/// The geometries, in geometry order.
	pub(crate) geometries: Vec<HitGeometry>,
	/// The hit record of the first geometry; the others follow it.
	pub(crate) record_base: u32,
	pub(crate) built: bool,
}

/// The `PlacedInstance` struct is one renderable that the frame's top level places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlacedInstance<S> {
	pub(crate) structure: S,
	pub(crate) model: Mat4x3f,
	/// The renderable's first entry in the geometry records; its geometries follow it.
	pub(crate) custom_index: u32,
	pub(crate) record_base: u32,
}

/// Places one instance for each renderable of `primitives` whose mesh's bottom level is built or builds this frame, and
/// writes `geometry_records` so that entry `custom_index + geometry` holds the frame's mesh record of the primitive
/// that geometry holds, then the geometry's first triangle in the flat triangle buffer.
///
/// `primitives` lists every primitive the frame draws, and is sorted by renderable and primitive here. A renderable
/// enters a frame whole, so each of its geometries has a primitive in the list. Returns how many records it wrote.
pub(crate) fn place_instances<S: Copy>(
	primitives: &mut [TracedPrimitive],
	bottom_levels: &HashMap<MeshKey, BottomLevel<S>>,
	instances: &mut impl Extend<PlacedInstance<S>>,
	geometry_records: &mut [[u32; 2]],
) -> u32 {
	primitives.sort_unstable_by_key(|primitive| (primitive.renderable.id(), primitive.primitive));
	let mut written = 0u32;
	for renderable in primitives.chunk_by(|first, second| first.renderable == second.renderable) {
		let first = &renderable[0];
		let Some(bottom_level) = bottom_levels.get(&first.mesh).filter(|bottom_level| bottom_level.built) else {
			continue;
		};
		let custom_index = written;
		for (index, geometry) in bottom_level.geometries.iter().enumerate() {
			let record = renderable
				.binary_search_by_key(&geometry.primitive, |traced| traced.primitive)
				.map(|position| renderable[position].record)
				.expect("A ray-traced renderable lacks one of its mesh's primitives. The most likely cause is that the frame admitted part of a renderable, which it never does.");
			geometry_records[custom_index as usize + index] = [record, geometry.first_triangle];
		}
		written += bottom_level.geometries.len() as u32;
		instances.extend([PlacedInstance {
			structure: bottom_level.structure,
			model: first.model,
			custom_index,
			record_base: bottom_level.record_base,
		}]);
	}
	written
}

/// Returns `model`, which shaders store as four columns, as the three rows an acceleration structure instance takes.
pub(crate) fn instance_transform(model: Mat4x3f) -> [[f32; 4]; 3] {
	let columns: [[f32; 3]; 4] = model.into();
	std::array::from_fn(|row| columns.map(|column| column[row]))
}

/// The `AccelerationStructureWork` struct is one frame's acceleration structure commands: the bottom levels it builds,
/// then the top level it places them in.
///
/// [`SceneAccelerationStructures::prepare`] makes it; the sink that records the frame-wide work records it before the
/// surfel grid.
#[derive(Clone, Copy)]
pub(crate) struct AccelerationStructureWork<'a> {
	pub(crate) builds: &'a [ghi::BottomLevelAccelerationStructureHandle],
	pub(crate) top_level: ghi::TopLevelAccelerationStructureHandle,
	pub(crate) instances: &'a [ghi::rt::Instance],
}

/// The `SceneAccelerationStructures` struct keeps the bottom level of every mesh renderables use, and the top level
/// that places the frame's instances.
///
/// The visibility pipeline manager creates it when surfel GI is built in and the device traces rays. Register each
/// mesh with [`Self::register_mesh`] as its first renderable arrives. Each frame, call [`Self::admit`] and write the
/// shader binding table from the hit records it returns; then, when surfel GI lights the frame, call
/// [`Self::prepare`], record its work with the frame-wide work, and call [`Self::commit`].
pub(crate) struct SceneAccelerationStructures {
	/// The ray-traced scene set, which binds the top level, the flat triangles, and the geometry records.
	descriptor_set: ghi::DescriptorSetHandle,
	top_level: ghi::TopLevelAccelerationStructureHandle,
	/// For each frame, entry `custom_index + geometry` of a hit names the frame's mesh record it hit and the hit
	/// geometry's first triangle.
	geometry_records: ghi::DynamicBufferHandle<[[u32; 2]; MAX_INSTANCES]>,
	vertex_positions: ghi::BufferHandle<[[f32; 3]]>,
	flat_triangle_indices: ghi::BufferHandle<[[u16; 3]]>,
	/// Every registered mesh's primitives, until its bottom level is created.
	geometries: HashMap<MeshKey, Vec<PrimitiveGeometry>>,
	bottom_levels: HashMap<MeshKey, BottomLevel<ghi::BottomLevelAccelerationStructureHandle>>,
	builds: BuildQueue,
	/// The material slot of each hit record's geometry, in record order.
	record_materials: Vec<u32>,
}

impl SceneAccelerationStructures {
	/// Creates the top level, the geometry records, and the ray-traced scene set over the flat triangle lists of
	/// `geometry`.
	///
	/// Returns `None` when `geometry` keeps no flat triangle lists, which means the device doesn't trace rays.
	pub(crate) fn new(context: &mut ghi::implementation::Context, geometry: &GeometryHandles) -> Option<Self> {
		let flat_triangle_indices = geometry.flat_triangle_indices?;
		let top_level = context.create_top_level_acceleration_structure(Some("Scene Top Level"), MAX_INSTANCES as u32);
		let geometry_records = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Ray Tracing Geometry Records")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let descriptor_set = set_layouts::RAY_TRACED_SCENE.create_set(context, "Ray-Traced Scene");
		let set = set_layouts::RAY_TRACED_SCENE.binder(descriptor_set);
		context.write(&[
			ghi::DescriptorWrite::acceleration_structure(descriptor_set, set.slot("scene_structure"), top_level),
			set.buffer("flat_triangle_indices", flat_triangle_indices),
			set.buffer("ray_tracing_geometries", geometry_records),
		]);
		Some(Self {
			descriptor_set,
			top_level,
			geometry_records,
			vertex_positions: geometry.vertex_positions,
			flat_triangle_indices,
			geometries: HashMap::default(),
			bottom_levels: HashMap::default(),
			builds: BuildQueue::default(),
			record_materials: Vec::new(),
		})
	}

	/// Returns the ray-traced scene set, which the passes that trace or shade the frame's rays bind.
	pub(crate) fn descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.descriptor_set
	}

	/// Remembers where `mesh` keeps its triangles, unless it is already known. Its bottom level is created once a frame
	/// draws one of its renderables, when its materials' alpha modes are known.
	pub(crate) fn register_mesh(&mut self, key: MeshKey, mesh: &MeshData) {
		if !self.bottom_levels.contains_key(&key) {
			self.geometries.entry(key).or_insert_with(|| PrimitiveGeometry::of_mesh(mesh));
		}
	}

	/// Creates the bottom levels of the meshes this frame draws for the first time, which gives their geometries hit
	/// records, and returns the material slot of every hit record, in record order.
	///
	/// `primitives` lists every primitive the frame draws; `alpha_mode` returns a material slot's alpha mode. Next,
	/// write the shader binding table from the returned records, so it holds every record an instance of this frame
	/// reports, then call [`Self::prepare`].
	pub(crate) fn admit<'m>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		primitives: &[TracedPrimitive],
		alpha_mode: impl Fn(u32) -> &'m AlphaMode,
	) -> &[u32] {
		for primitive in primitives {
			if let Some(geometries) = self.geometries.remove(&primitive.mesh) {
				self.create_bottom_level(frame, primitive.mesh, &geometries, &alpha_mode);
			}
		}
		&self.record_materials
	}

	/// Picks the builds that fit this frame's budget, places the frame's instances, and uploads the geometry records.
	///
	/// `primitives` lists every primitive the frame draws, whose meshes [`Self::admit`] admitted. `frame_allocator`
	/// holds the returned work until the frame is recorded. Next, record the work with the frame-wide work and call
	/// [`Self::commit`].
	pub(crate) fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		primitives: &mut [TracedPrimitive],
		frame_allocator: &'a bumpalo::Bump,
	) -> AccelerationStructureWork<'a> {
		let taken = self.builds.take(BOTTOM_LEVEL_BUILD_TRIANGLES_PER_FRAME);
		if !taken.is_empty() {
			log::debug!(
				"Building {} bottom-level acceleration structures of {} triangles this frame.",
				taken.len(),
				taken.iter().map(|(_, triangles)| u64::from(*triangles)).sum::<u64>()
			);
		}
		let mut builds = bumpalo::collections::Vec::with_capacity_in(taken.len(), frame_allocator);
		for (key, _) in taken {
			let bottom_level = self.bottom_levels.get_mut(key).expect("a queued mesh has a bottom level");
			// The top level built after these builds in the same recording may place them.
			bottom_level.built = true;
			builds.push(bottom_level.structure);
		}

		let mut placed = bumpalo::collections::Vec::with_capacity_in(MAX_INSTANCES, frame_allocator);
		let records = frame.get_mut_dynamic_buffer_slice(self.geometry_records);
		place_instances(primitives, &self.bottom_levels, &mut placed, records);
		frame.sync_buffer(self.geometry_records);
		let instances = bumpalo::collections::Vec::from_iter_in(
			placed.iter().map(|instance| ghi::rt::Instance {
				transform: instance_transform(instance.model),
				custom_index: instance.custom_index,
				mask: SCENE_MASK,
				hit_group_offset: instance.record_base,
				bottom_level: instance.structure,
			}),
			frame_allocator,
		);
		AccelerationStructureWork {
			builds: builds.into_bump_slice(),
			top_level: self.top_level,
			instances: instances.into_bump_slice(),
		}
	}

	/// Settles the builds the last [`Self::prepare`] picked: they are built when `recorded`, the frame recorded its
	/// frame-wide work; otherwise they wait at the front of the queue for the next frame.
	pub(crate) fn commit(&mut self, recorded: bool) {
		let bottom_levels = &mut self.bottom_levels;
		self.builds.settle(recorded, |key| {
			if let Some(bottom_level) = bottom_levels.get_mut(&key) {
				bottom_level.built = false;
			}
		});
	}

	/// Creates the bottom level of `key` over its opaque primitives and queues its build. A mesh without one creates
	/// none, and rays never hit it.
	fn create_bottom_level<'m>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		key: MeshKey,
		geometries: &[PrimitiveGeometry],
		alpha_mode: &impl Fn(u32) -> &'m AlphaMode,
	) {
		let primitives = opaque_geometries(geometries, alpha_mode);
		if primitives.is_empty() {
			return;
		}
		let triangle_geometries = primitives
			.iter()
			.map(|&index| {
				let geometry = &geometries[index as usize];
				let vertex_size = std::mem::size_of::<[f32; 3]>();
				ghi::rt::TriangleGeometry {
					positions: ghi::BufferStridedRange::new(
						self.vertex_positions.into(),
						geometry.vertices.start as usize * vertex_size,
						vertex_size,
						geometry.vertices.len() * vertex_size,
					),
					indices: ghi::BufferDescriptor::from(ghi::BaseBufferHandle::from(self.flat_triangle_indices))
						.offset(geometry.triangles.start as usize * std::mem::size_of::<[u16; 3]>()),
					index_format: ghi::DataTypes::U16,
					triangle_count: geometry.triangles.len() as u32,
				}
			})
			.collect::<Vec<_>>();
		let triangles = triangle_geometries.iter().map(|geometry| geometry.triangle_count).sum();
		let structure = frame.create_bottom_level_acceleration_structure(Some(&format!("{key}")), &triangle_geometries);
		let record_base = self.record_materials.len() as u32;
		self.record_materials
			.extend(primitives.iter().map(|&primitive| geometries[primitive as usize].material));
		let hit_geometries = primitives
			.iter()
			.map(|&primitive| HitGeometry {
				primitive,
				first_triangle: geometries[primitive as usize].triangles.start,
			})
			.collect();
		self.bottom_levels.insert(
			key,
			BottomLevel {
				structure,
				geometries: hit_geometries,
				record_base,
				built: false,
			},
		);
		self.builds.push(key, triangles);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn primitive(material: u32, triangles: Range<u32>) -> PrimitiveGeometry {
		PrimitiveGeometry {
			material,
			vertices: 0..3,
			triangles,
		}
	}

	fn traced(renderable: u32, mesh: &'static str, primitive: u32, record: u32) -> TracedPrimitive {
		TracedPrimitive {
			renderable: Handle::from_id(renderable),
			mesh: MeshKey::Resource(mesh),
			primitive,
			record,
			model: Mat4x3f::from([
				[1.0, 0.0, 0.0],
				[0.0, 1.0, 0.0],
				[0.0, 0.0, 1.0],
				[renderable as f32, 0.0, 0.0],
			]),
		}
	}

	/// A bottom level whose geometries hold `primitives`, each primitive's triangles starting at 100 times its index.
	fn bottom_level(structure: u32, primitives: Vec<u32>, record_base: u32, built: bool) -> BottomLevel<u32> {
		BottomLevel {
			structure,
			geometries: primitives
				.into_iter()
				.map(|primitive| HitGeometry {
					primitive,
					first_triangle: primitive * 100,
				})
				.collect(),
			record_base,
			built,
		}
	}

	#[test]
	fn bottom_levels_leave_out_masked_and_blended_primitives() {
		let alpha_modes = [AlphaMode::Opaque, AlphaMode::Mask(0.5), AlphaMode::Blend];
		let primitives = [
			primitive(0, 0..4),
			primitive(1, 4..8),
			primitive(2, 8..9),
			primitive(0, 9..12),
			// A skinned primitive keeps no triangles.
			primitive(0, 12..12),
		];

		assert_eq!(
			opaque_geometries(&primitives, |material| &alpha_modes[material as usize]),
			[0, 3]
		);
	}

	#[test]
	fn bottom_levels_build_within_the_triangle_budget_in_arrival_order() {
		assert_eq!(builds_within_budget([100, 200, 300], 300), 2);
		assert_eq!(builds_within_budget([100, 200, 300], 600), 3);
		// A mesh larger than the budget builds alone, and the next waits for another frame.
		assert_eq!(builds_within_budget([1_000, 10], 300), 1);
		assert_eq!(builds_within_budget([], 300), 0);
	}

	#[test]
	fn uncommitted_builds_are_retried_next_frame() {
		let [first, second, third] = ["first", "second", "third"].map(MeshKey::Resource);
		let mut queue = BuildQueue::default();
		for mesh in [first, second, third] {
			queue.push(mesh, 100);
		}
		let keys = |taken: &[(MeshKey, u32)]| taken.iter().map(|(mesh, _)| *mesh).collect::<Vec<_>>();

		assert_eq!(keys(queue.take(200)), [first, second]);
		let mut unbuilt = Vec::new();
		queue.settle(false, |mesh| unbuilt.push(mesh));
		assert_eq!(unbuilt, [second, first]);
		// The frame that didn't record its builds leaves them first in line.
		assert_eq!(keys(queue.take(200)), [first, second]);
		queue.settle(true, |_| panic!("A recorded build stays built."));
		assert_eq!(keys(queue.take(200)), [third]);
	}

	#[test]
	fn ray_traced_instances_leave_out_renderables_without_a_built_bottom_level() {
		let bottom_levels = HashMap::from_iter([
			(MeshKey::Resource("built"), bottom_level(7, vec![0], 0, true)),
			(MeshKey::Resource("unbuilt"), bottom_level(8, vec![0], 1, false)),
		]);
		let mut primitives = [
			traced(1, "built", 0, 10),
			traced(2, "unbuilt", 0, 11),
			// A mesh without opaque, unskinned primitives has no bottom level.
			traced(3, "skinned", 0, 12),
			traced(4, "built", 0, 13),
		];
		let mut instances = Vec::new();
		let mut records = [[u32::MAX; 2]; 8];

		assert_eq!(
			place_instances(&mut primitives, &bottom_levels, &mut instances, &mut records),
			2
		);
		assert_eq!(
			instances,
			[1, 4].map(|renderable| PlacedInstance {
				structure: 7,
				model: traced(renderable, "built", 0, 0).model,
				custom_index: if renderable == 1 { 0 } else { 1 },
				record_base: 0,
			})
		);
		assert_eq!(records[..2], [[10, 0], [13, 0]]);
	}

	#[test]
	fn instance_transforms_are_the_rows_of_the_model() {
		let model = Mat4x3f::from([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0], [10.0, 11.0, 12.0]]);

		// The fourth column, the translation, ends each row.
		assert_eq!(
			instance_transform(model),
			[[1.0, 4.0, 7.0, 10.0], [2.0, 5.0, 8.0, 11.0], [3.0, 6.0, 9.0, 12.0]]
		);
	}

	#[test]
	fn geometry_table_maps_each_geometry_to_its_frame_record() {
		// The mesh's middle primitive is masked, so its two geometries hold primitives 0 and 2.
		let bottom_levels = HashMap::from_iter([(MeshKey::Resource("hall"), bottom_level(3, vec![0, 2], 5, true))]);
		// The frame lists primitives in any order, and two renderables share the mesh.
		let mut primitives = [
			traced(2, "hall", 2, 21),
			traced(1, "hall", 1, 11),
			traced(2, "hall", 0, 20),
			traced(1, "hall", 2, 12),
			traced(1, "hall", 0, 10),
			traced(2, "hall", 1, 22),
		];
		let mut instances = Vec::new();
		let mut records = [[u32::MAX; 2]; 8];

		assert_eq!(
			place_instances(&mut primitives, &bottom_levels, &mut instances, &mut records),
			4
		);
		let placed: Vec<_> = instances
			.iter()
			.map(|instance| (instance.custom_index, instance.record_base))
			.collect();
		assert_eq!(placed, [(0, 5), (2, 5)]);
		assert_eq!(records[..4], [[10, 0], [12, 200], [20, 0], [21, 200]]);
	}
}
