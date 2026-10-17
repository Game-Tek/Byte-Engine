//! Fixed limits and record layouts shared by every visibility shader and the CPU code that feeds them.
//!
//! The descriptor sets the shaders read these records through are declared by the layout layers in
//! `assets/rendering/visibility/layouts`; see [`super::set_layouts`]. The layers spell out the array sizes, so the
//! guards here and in [`super::shader_generator`] fail the build when a limit changes without them.

/* Limits */

/// Meshlets one primitive instance can hold. Task shaders address an instance's meshlets with 12 bits, relative to its
/// first meshlet, so this bounds each primitive rather than the scene. See [`super::geometry::GeometryCapacity`] for the
/// scene-wide geometry limits.
pub(crate) const MAX_INSTANCE_MESHLETS: usize = 1024 * 4;
/// Scene-wide meshlets the visibility buffer can address: it stores a meshlet index above an 8-bit triangle index.
pub(crate) const MAX_ADDRESSABLE_MESHLETS: usize = 1 << 24;
pub(crate) const MAX_INSTANCES: usize = 1024;
pub(crate) const MAX_MATERIALS: usize = 1024;
/// One bit per material evaluation slot; a set bit means a material of that slot has at least one active instance in
/// the mask's phase this frame. There are never more slots than materials. See [`super::scene::MaterialEvaluation`].
pub(crate) type ActiveEvaluationMask = [u64; MAX_MATERIALS / u64::BITS as usize];
/// Materials use a small indirection table so generated shaders can use stable per-material slots into the
/// larger scene-wide bindless texture pool.
pub(crate) const MAX_MATERIAL_TEXTURES: usize = 16;
pub(crate) const MAX_BINDLESS_TEXTURES: usize = 1024;
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

/// The `ShaderMeshletData` struct stores meshlet offsets and object-space culling bounds for GPU visibility passes.
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct ShaderMeshletData {
	/// Base index into the vertex-index buffer, relative to the mesh.
	pub(crate) primitive_offset: u32,
	/// Base triangle into the primitive-index buffer, relative to the mesh. Multiply by three for the raw index.
	pub(crate) triangle_offset: u32,
	/// Number of meshlet-local vertices.
	pub(crate) primitive_count: u32,
	/// Number of meshlet-local triangles.
	pub(crate) triangle_count: u32,
	/// Object-space bounding sphere encoded as xyz center and w radius.
	pub(crate) center_radius: [f32; 4],
	/// Object-space normal-cone apex encoded as xyz apex and w cutoff. A cutoff above one disables cone rejection.
	pub(crate) cone_apex_cutoff: [f32; 4],
	/// Octahedrally encoded object-space normal-cone axis.
	pub(crate) cone_axis: RuntimeVertexNormal,
}

/// The [`super::set_layouts::SCENE`] `material_evaluations` entry of a material that no evaluation dispatch shades. The
/// material prepasses skip its pixels; the shaders spell it as `4294967295`, which fails their `< 1024` slot tests.
pub(crate) const NO_EVALUATION: u32 = u32::MAX;

/// `ShaderMeshletData` must keep the packed layout the meshlet buffer is read with.
///
/// The culling shaders index this buffer by stride and read each field at a fixed offset.
///
/// These are compile-time checks so a layout change fails the build at the definition rather than
/// later in a shader that silently reads the wrong bytes.
const _: () = assert!(std::mem::align_of::<ShaderMeshletData>() == 4);
const _: () = assert!(std::mem::size_of::<ShaderMeshletData>() == 52);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, center_radius) == 16);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, cone_apex_cutoff) == 32);
const _: () = assert!(std::mem::offset_of!(ShaderMeshletData, cone_axis) == 48);
