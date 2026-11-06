//! Fixed limits and record layouts shared by every visibility shader and the CPU code that feeds them.
//!
//! The descriptor sets the shaders read these records through are declared by the layers in the modules under
//! `assets/rendering/visibility/layouts`; see [`super::set_layouts`]. The layers spell out the array sizes, so the
//! guards here and in [`super::shader_generator`] fail the build when a limit changes without them.

/* Limits */

/// Meshlets one primitive instance can hold. Task shaders address an instance's meshlets with 12 bits, relative to its
/// first meshlet, so this bounds each primitive rather than the scene. See [`super::geometry::GeometryCapacity`] for the
/// scene-wide geometry limits.
pub(crate) const MAX_INSTANCE_MESHLETS: usize = resource_management::resources::mesh::MAX_LEVEL_MESHLETS;
/// Scene-wide meshlets the visibility buffer can address: it stores a meshlet index above an 8-bit triangle index.
pub(crate) const MAX_ADDRESSABLE_MESHLETS: usize = 1 << 24;
/// Resident detail levels an instance record can name: it stores the level below its 8 flag bits. See
/// [`super::geometry::GeometryCapacity::levels`].
pub(crate) const MAX_ADDRESSABLE_LEVELS: usize = 1 << 24;
/// Instances a mesh dispatch work item can address: it stores the instance above a 7-bit meshlet chunk. See
/// [`super::scene::InstanceCapacity`] for how many the pipeline holds.
pub(crate) const MAX_ADDRESSABLE_INSTANCES: usize = 1 << 25;
pub(crate) const MAX_MATERIALS: usize = 1024;
/// One bit per material evaluation slot; a set bit means a material of that slot has at least one active instance in
/// the mask's phase this frame. There are never more slots than materials. See [`super::scene::MaterialEvaluation`].
pub(crate) type ActiveEvaluationMask = [u64; MAX_MATERIALS / u64::BITS as usize];
/// Materials use a small indirection table so generated shaders can use stable per-material slots into the
/// larger scene-wide bindless texture pool.
pub(crate) const MAX_MATERIAL_TEXTURES: usize = 16;
pub(crate) const MAX_BINDLESS_TEXTURES: usize = 1024;
/// The distinct IES profiles the scene's lights can have resident at once. Lights that share a profile share its slot.
pub(crate) const MAX_IES_PROFILES: usize = 64;
/// The lightmaps that can be resident at once, normally one per loaded level.
pub(crate) const MAX_LIGHTMAPS: usize = 16;
/// Entries of the material parameter buffer, one per dynamic parameter of a loaded material or of an instance the
/// application set parameters on. See [`super::material_parameters`].
pub(crate) const MAX_MATERIAL_PARAMETERS: usize = 16384;
/// The size of the scene light table. Light bucketing keeps per-pixel cost proportional to the lights near each pixel.
pub(crate) const MAX_LIGHTS: usize = 1024;
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;

/// The specialization constant that compiles a material evaluation pipeline for the transparent phase.
///
/// The opaque and transparent phases light and write pixels differently, and a runtime flag would compile the
/// transparent path's shadow-map filtering into every opaque pipeline. The loader sets it from the variant's alpha mode.
pub(crate) const TRANSPARENT_SPECIALIZATION_ID: u32 = 0;
/// Returns the first specialization constant of the material variable at `variable_index`.
///
/// Variables follow [`TRANSPARENT_SPECIALIZATION_ID`], four constants each: one per component of the widest
/// variable type, `vec4f`.
pub(crate) const fn material_variable_specialization_id(variable_index: u32) -> u32 {
	TRANSPARENT_SPECIALIZATION_ID + 1 + variable_index * 4
}

/// Vertices and triangles one meshlet can hold.
pub(crate) const VERTEX_COUNT: u32 = 64;
pub(crate) const TRIANGLE_COUNT: u32 = 126;
/// Meshlets culled by one task workgroup.
pub(crate) const MESHLET_CULLING_TASK_GROUP_SIZE: u32 = 32;
/// Views one task workgroup culls its meshlets against, such as one point light's cube faces or all the cascades.
/// `meshlet-task.besl` and the mesh shaders size their payload for this many views of every meshlet and spell the limit
/// as a literal.
pub(crate) const MAX_TASK_VIEWS: usize = 6;

/* Shadow views */

pub(crate) const SHADOW_CASCADE_COUNT: usize = 4;
/// Texels per side of each directional cascade unless the application sets `render.directional-shadows.resolution`.
pub(crate) const DEFAULT_SHADOW_MAP_RESOLUTION: u32 = 2048;
/// The most lights of each kind that can hold shadow maps at once. Every one has reserved slots in the `views` buffer;
/// within these limits, the shadow-map budget decides how many lights get maps. See
/// [`super::shadow_selection::ShadowBudget`].
pub(crate) const MAX_DIRECTIONAL_SHADOW_COUNT: usize = 4;
pub(crate) const MAX_CONE_SHADOW_COUNT: usize = 16;
pub(crate) const MAX_POINT_SHADOW_COUNT: usize = 16;
/// The shadow-map memory, in mebibytes, that lights share unless the application sets `render.shadow-maps.budget-mb`.
/// One sun's cascades take 32 MiB of it at the default resolution, which leaves 96 MiB for local lights and other
/// suns.
pub(crate) const DEFAULT_SHADOW_MAP_BUDGET_MIB: u32 = 128;
/// Cone and point shadow maps come in power-of-two size classes, so a light's map changes size only when its projected
/// size crosses a power of two. Class `c` holds maps of `MIN_LOCAL_SHADOW_MAP_RESOLUTION << c` texels per side, from
/// 128 to 2048. Each kind keeps one image per class, which material evaluation binds as a descriptor array and indexes
/// by the light's class. See [`super::shadow_selection::size_class_for_span`].
pub(crate) const LOCAL_SHADOW_SIZE_CLASS_COUNT: usize = 5;
pub(crate) const MIN_LOCAL_SHADOW_MAP_RESOLUTION: u32 = 128;

/// Returns the texels per side of the cone maps and point cube faces of size class `size_class`.
pub(crate) const fn local_shadow_map_resolution(size_class: usize) -> u32 {
	MIN_LOCAL_SHADOW_MAP_RESOLUTION << size_class
}
/// Shadow maps use 16-bit depth to halve their memory. Directional cascades project orthographically, so their depth
/// steps are even: each cascade's depth range over 65,535, a few millimeters at most, and finer than its texels.
pub(crate) const CONE_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
pub(crate) const POINT_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
pub(crate) const DIRECTIONAL_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
/// Layout of the `views` buffer: camera, then each sun's cascades, then cone layers, then point cube faces.
///
/// Sun slot `s` owns views `1 + 4s` through `4 + 4s`, and directional view `v` draws into layer `v - 1` of the
/// directional shadow map. The shadow shaders find a cascade's layer from its view this way.
pub(crate) const DIRECTIONAL_SHADOW_VIEW_OFFSET: usize = 1;
pub(crate) const CONE_SHADOW_VIEW_OFFSET: usize =
	DIRECTIONAL_SHADOW_VIEW_OFFSET + MAX_DIRECTIONAL_SHADOW_COUNT * SHADOW_CASCADE_COUNT;
pub(crate) const POINT_SHADOW_FACE_COUNT: usize = 6;
pub(crate) const POINT_SHADOW_VIEW_OFFSET: usize = CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_COUNT;
/// The shaders declare the `views` buffer with this many entries.
pub(crate) const SHADOW_VIEW_COUNT: usize = POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_COUNT * POINT_SHADOW_FACE_COUNT;
/// The camera and every sun's cascades, the views the directional shadow passes declare.
pub(crate) const DIRECTIONAL_VIEW_COUNT: usize = CONE_SHADOW_VIEW_OFFSET;
const _: () = assert!(
	SHADOW_VIEW_COUNT == 129 && DIRECTIONAL_VIEW_COUNT == 17 && MAX_TASK_VIEWS == 6 && SHADOW_CASCADE_COUNT <= MAX_TASK_VIEWS,
	"Update the `View[129]` declarations in the visibility shaders and the material shader generator, the `View[17]` declarations in the directional shadow shaders, and the task payload, when shadow view limits change."
);

/* Light clusters */

/// Screen columns, screen rows, and exponential depth slices that split each sink's view frustum into light clusters.
pub(crate) const LIGHT_CLUSTER_COLUMNS: u32 = 16;
pub(crate) const LIGHT_CLUSTER_ROWS: u32 = 8;
pub(crate) const LIGHT_CLUSTER_SLICES: u32 = 24;
pub(crate) const LIGHT_CLUSTER_COUNT: usize = (LIGHT_CLUSTER_COLUMNS * LIGHT_CLUSTER_ROWS * LIGHT_CLUSTER_SLICES) as usize;
/// Each cluster stores one bit per light-table entry.
pub(crate) const LIGHT_CLUSTER_MASK_WORDS: usize = MAX_LIGHTS / u32::BITS as usize;
pub(crate) const LIGHT_CLUSTER_MASK_WORD_COUNT: usize = LIGHT_CLUSTER_COUNT * LIGHT_CLUSTER_MASK_WORDS;

/* Runtime vertex formats */

/// The octahedrally encoded runtime unit-vector element.
pub(crate) type RuntimeUnitVector = [u16; 2];
pub(crate) type RuntimeVertexNormal = RuntimeUnitVector;
pub(crate) const VERTEX_NORMAL_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexNormal>() as u32;
/// Half-float UVs preserve sampler wrapping coordinates outside `[0, 1]`.
pub(crate) type RuntimeVertexUv = [u16; 2];
pub(crate) const VERTEX_UV_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexUv>() as u32;
/// UNORM16 lightmap UVs: lightmap UVs never leave `[0, 1]`, and 16-bit fractions keep a texel's position within a
/// 4,096-texel lightmap where half floats would drift by half a texel.
pub(crate) type RuntimeVertexLightmapUv = [u16; 2];
pub(crate) const VERTEX_LIGHTMAP_UV_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexLightmapUv>() as u32;

/// The `ShaderMeshletData` struct is one meshlet as the visibility shaders read it: where its indices lie, and the
/// bounds task shaders cull it with.
///
/// Build one with [`Self::new`]. Its bounding sphere is stored relative to its level's sphere, so the level's
/// [`ShaderMeshLevel`] record must be at hand to decode it, as [`Self::sphere`] does.
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct ShaderMeshletData {
	/// First entry in the vertex-index buffer. Prepared meshes count it from the start of their own stream until
	/// [`super::geometry::GeometryBuffers`] places them.
	pub(crate) primitive_offset: u32,
	/// First triangle in the primitive-index buffer, counted like `primitive_offset`. Multiply by three for the raw
	/// index.
	pub(crate) triangle_offset: u32,
	/// The vertex count in bits 0-7 and the triangle count in bits 8-15.
	pub(crate) counts: u32,
	/// The object-space normal cone: the axis as three SNORM8 components in bits 0-23 and the cutoff as SNORM8 in bits
	/// 24-31. A cutoff of 127 never culls.
	pub(crate) cone: u32,
	/// The bounding sphere as UNORM16 values within the level's sphere: center x and y in the first word, center z and
	/// radius in the second, each low half first.
	pub(crate) bounds: [u32; 2],
}

impl ShaderMeshletData {
	/// Packs a meshlet whose indices start at `primitive_offset` and `triangle_offset`.
	///
	/// `sphere` is the meshlet's object-space bounding sphere, and `level_sphere` the sphere of its level, which must
	/// contain it with the margin [`level_sphere`] leaves. The stored sphere contains `sphere` after rounding. A normal
	/// cone with a `cone_cutoff` of one or more never culls; the stored cutoff widens the cone by the axis's rounding,
	/// so culling stays conservative.
	pub(crate) fn new(
		primitive_offset: u32,
		triangle_offset: u32,
		[vertex_count, triangle_count]: [u8; 2],
		sphere: [f32; 4],
		level_sphere: [f32; 4],
		cone_axis: [f32; 3],
		cone_cutoff: f32,
	) -> Self {
		let [cx, cy, cz, radius] = sphere;
		let [lx, ly, lz, level_radius] = level_sphere;
		let center = [cx, cy, cz].map(|component| component as f64);
		let quantized = [(cx, lx), (cy, ly), (cz, lz)].map(|(component, level_center)| {
			if level_radius > 0.0 {
				let unit = ((component - level_center) as f64 / level_radius as f64 * 0.5 + 0.5).clamp(0.0, 1.0);
				(unit * 65535.0).round() as u32
			} else {
				// Every center of a level without extent is the level's center, which the midpoint decodes to.
				32768
			}
		});
		let decoded = Self::decode_center(quantized, level_sphere).map(|component| component as f64);
		// The radius grows by how far rounding moved the center, plus a margin for the shader's float arithmetic.
		let moved = (0..3).map(|axis| (decoded[axis] - center[axis]).powi(2)).sum::<f64>().sqrt();
		let margin = 1e-6 * (lx.abs().max(ly.abs()).max(lz.abs()) + level_radius) as f64;
		let needed = radius as f64 + moved + margin;
		let quantized_radius = if level_radius > 0.0 {
			(needed / level_radius as f64 * 65535.0).ceil().clamp(0.0, 65535.0) as u32
		} else {
			0
		};
		Self {
			primitive_offset,
			triangle_offset,
			counts: u32::from(vertex_count) | u32::from(triangle_count) << 8,
			cone: encode_cone(cone_axis, cone_cutoff),
			bounds: [
				quantized[0] | quantized[1] << 16,
				quantized[2] | quantized_radius << 16,
			],
		}
	}

	pub(crate) fn vertex_count(&self) -> u32 {
		self.counts & 0xFF
	}

	pub(crate) fn triangle_count(&self) -> u32 {
		(self.counts >> 8) & 0xFF
	}

	/// Returns the object-space bounding sphere the record stores within `level_sphere`, decoded as task shaders
	/// decode it.
	#[cfg(test)]
	pub(crate) fn sphere(&self, level_sphere: [f32; 4]) -> [f32; 4] {
		let [first, second] = self.bounds;
		let [x, y, z] = Self::decode_center([first & 0xFFFF, first >> 16, second & 0xFFFF], level_sphere);
		[x, y, z, (second >> 16) as f32 / 65535.0 * level_sphere[3]]
	}

	fn decode_center(quantized: [u32; 3], [lx, ly, lz, level_radius]: [f32; 4]) -> [f32; 3] {
		let decode = |value: u32, center: f32| center + ((value as f32 / 65535.0) * 2.0 - 1.0) * level_radius;
		[decode(quantized[0], lx), decode(quantized[1], ly), decode(quantized[2], lz)]
	}
}

/// Packs a normal cone as SNORM8 axis components and cutoff, widening the cutoff by the rounding of each axis component
/// so a cone test never culls a meshlet the exact cone keeps. A cutoff of one or more never culls.
fn encode_cone(axis: [f32; 3], cutoff: f32) -> u32 {
	let snorm8 = |value: f32| (value.clamp(-1.0, 1.0) * 127.0).round() as i8;
	let quantized_axis = axis.map(snorm8);
	let error: f32 = (0..3).map(|index| (quantized_axis[index] as f32 / 127.0 - axis[index]).abs()).sum();
	let quantized_cutoff = if cutoff >= 1.0 || !cutoff.is_finite() {
		127
	} else {
		((cutoff + error) * 127.0 + 1.0).clamp(-127.0, 127.0) as i8
	};
	u32::from_le_bytes([
		quantized_axis[0] as u8,
		quantized_axis[1] as u8,
		quantized_axis[2] as u8,
		quantized_cutoff as u8,
	])
}

/// Returns `enclosing`, a sphere around every meshlet sphere of a level, grown by the margin [`ShaderMeshletData::new`]
/// needs to round them conservatively within it.
///
/// The margin covers the distance rounding can move a center, at most √3 / 65,535 of the radius.
pub(crate) fn level_sphere(enclosing: [f32; 4]) -> [f32; 4] {
	let [x, y, z, radius] = enclosing;
	[x, y, z, radius * (1.0 + 4.0 / 65535.0)]
}

/// The `ShaderMeshLevel` struct is one resident detail level of a primitive, which every instance that draws it names.
///
/// [`super::geometry::GeometryBuffers`] writes one for each level it places, and [`super::lod::MeshLod::level`] names
/// it, so switching an instance's level rewrites one index of its [`super::shader_data::ShaderMesh`].
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct ShaderMeshLevel {
	/// Object-space sphere around the level's meshlets, as xyz center and w radius, made with [`level_sphere`]. Task
	/// shaders cull whole instances with it and decode the level's meshlet spheres within it.
	pub(crate) bounding_sphere: [f32; 4],
	pub(crate) first_meshlet: u32,
	pub(crate) meshlet_count: u32,
	/// The level's first vertex in the vertex attribute buffers.
	pub(crate) vertex_base: u32,
	pub(crate) _padding: u32,
}

/// The [`super::set_layouts::SCENE`] `material_evaluations` entry of a material that no evaluation dispatch shades. The
/// material prepasses skip its pixels; the shaders spell it as `4294967295`, which fails their `< 1024` slot tests.
pub(crate) const NO_EVALUATION: u32 = u32::MAX;

/// `ShaderMeshletData` and `ShaderMeshLevel` must keep the packed layouts their buffers are read with.
///
/// The culling shaders index these buffers by stride and read each field at a fixed offset.
///
/// These are compile-time checks so a layout change fails the build at the definition rather than
/// later in a shader that silently reads the wrong bytes.
const _: () = assert!(std::mem::align_of::<ShaderMeshletData>() == 4);
const _: () = assert!(std::mem::size_of::<ShaderMeshletData>() == 24);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, counts) == 8);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, cone) == 12);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, bounds) == 16);
const _: () = assert!(std::mem::size_of::<ShaderMeshLevel>() == 32);
const _: () = assert!(std::mem::offset_of!(ShaderMeshLevel, first_meshlet) == 16);
const _: () = assert!(std::mem::offset_of!(ShaderMeshLevel, vertex_base) == 24);

#[cfg(test)]
mod tests {
	use super::*;

	fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
		let distance = (0..3).map(|axis| (outer[axis] - inner[axis]).powi(2)).sum::<f32>().sqrt();
		distance + inner[3] <= outer[3]
	}

	/// Verifies a stored meshlet sphere still contains the meshlet after rounding, wherever it lies in its level.
	#[test]
	fn stored_meshlet_spheres_contain_their_meshlets() {
		let spheres = [
			[0.3, -0.2, 0.1, 0.25],
			[9.9, 0.0, 0.0, 0.1],
			[-3.0, 4.0, 5.0, 1.0],
			[1234.5, -987.6, 42.0, 3.0],
			[0.0; 4],
		];
		for level in [[0.0, 0.0, 0.0, 10.0], [1000.0, -1000.0, 40.0, 500.0]] {
			let level = level_sphere(level);
			for sphere in spheres.into_iter().filter(|&sphere| contains(level, sphere)) {
				let stored = ShaderMeshletData::new(0, 0, [0; 2], sphere, level, [0.0, 0.0, 1.0], 2.0).sphere(level);
				assert!(contains(stored, sphere), "{stored:?} leaves out {sphere:?} in {level:?}");
				assert!(stored[3] <= sphere[3] + level[3] / 1000.0, "{stored:?} grew too much for {sphere:?}");
			}
		}
		// A level without extent holds only meshlets at its center, which decode exactly.
		let point = [2.0, 3.0, 4.0, 0.0];
		assert_eq!(ShaderMeshletData::new(0, 0, [0; 2], point, point, [0.0, 0.0, 1.0], 2.0).sphere(point), point);
	}

	/// Verifies the stored cone culls no more than the exact cone: its cutoff, read back, is at least the exact cutoff
	/// plus the axis's rounding.
	#[test]
	fn stored_cones_are_never_narrower_than_their_meshlets_cones() {
		let decode = |cone: u32| cone.to_le_bytes().map(|byte| byte as i8 as f32 / 127.0);
		for (axis, cutoff) in [([0.0, 0.0, 1.0], 0.5), ([0.6, 0.0, 0.8], 0.9), ([0.577, -0.577, 0.577], -0.2)] {
			let [x, y, z, stored_cutoff] = decode(encode_cone(axis, cutoff));
			let error = (x - axis[0]).abs() + (y - axis[1]).abs() + (z - axis[2]).abs();
			assert!(stored_cutoff >= cutoff + error, "{stored_cutoff} is narrower than {cutoff} + {error}");
		}
		// A cone that keeps every view stays that way.
		assert_eq!(decode(encode_cone([0.0, 0.0, 1.0], 1.0))[3], 1.0);
		assert_eq!(decode(encode_cone([0.0, 0.0, 1.0], 2.0))[3], 1.0);
	}

}
