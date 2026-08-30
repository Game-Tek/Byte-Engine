use super::*;
use crate::rendering::pipelines::visibility::RuntimeUnitVector;

#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct ShaderMesh {
	pub(crate) model: AffineShaderMatrix,
	pub(crate) material_index: u32,
	/// The position into the vertex components data (positions, normals, uvs, ..) buffer this instance's data starts
	/// Also, the position into the vertex indices buffer this instance's data starts
	pub(crate) base_vertex_index: u32,
	pub(crate) base_primitive_index: u32,
	pub(crate) base_triangle_index: u32,
	pub(crate) base_meshlet_index: u32,
	pub(crate) meshlet_count: u32,
	/// Base vertex in the frame-local deformation buffer, or `u32::MAX` for immutable geometry.
	pub(crate) skinned_base_vertex_index: u32,
	pub(crate) _padding: u32,
}

/// The `LightingData` struct preserves the complete CPU storage layout consumed by material evaluation.
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct LightingData {
	pub count: u32,
	pub(crate) _padding: [u32; 3],
	pub lights: [LightData; MAX_LIGHTS],
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ShaderVec3 {
	pub(crate) x: f32,
	pub(crate) y: f32,
	pub(crate) z: f32,
	pub(crate) _padding: f32,
}

impl ShaderVec3 {
	fn new(x: f32, y: f32, z: f32) -> Self {
		Self { x, y, z, _padding: 0.0 }
	}
}

impl From<(f32, f32, f32)> for ShaderVec3 {
	fn from(value: (f32, f32, f32)) -> Self {
		Self::new(value.0, value.1, value.2)
	}
}

impl From<maths_rs::Vec3f> for ShaderVec3 {
	fn from(value: maths_rs::Vec3f) -> Self {
		Self::new(value.x, value.y, value.z)
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct ShaderViewData {
	pub(crate) view: AffineShaderMatrix,
	pub(crate) view_projection: ShaderMatrix,
	pub(crate) inverse_view: AffineShaderMatrix,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
}

/// Sentinel used when a local light has no resident IES profile texture.
pub(crate) const NO_IES_PROFILE_TEXTURE: u32 = u32::MAX;

/// The `IesProfileTexture` struct carries a resident intensity map's bindless slot and calibrated candela scale.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct IesProfileTexture {
	pub(crate) texture_index: u32,
	pub(crate) intensity_scale_candela: f32,
}

/// The `LightData` struct preserves one 16-byte-aligned, scene-referred light record across shader backends.
///
/// `color` stores RGB illuminance in lux for directional lights and RGB luminous intensity in candela for local lights.
/// IES-backed local lights resolve their calibrated candela scale into `color` on the CPU, so the shader only samples
/// a normalized profile when `ies_profile_texture` is not `NO_IES_PROFILE_TEXTURE`. Their C0 tangent uses
/// the same compact octahedral encoding as other runtime unit vectors.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightData {
	pub position: ShaderVec3,
	pub color: ShaderVec3,
	pub direction: ShaderVec3,
	pub cone_cosines: [f32; 2],
	pub light_type: u32,
	pub shadow_views: [u32; 8],
	pub shadow_layer: u32,
	pub(crate) ies_profile_texture: u32,
	pub(crate) ies_c0_tangent: RuntimeUnitVector,
	pub(crate) _ies_padding: [u32; 2],
}

impl Default for LightData {
	fn default() -> Self {
		Self {
			position: ShaderVec3::default(),
			color: ShaderVec3::default(),
			direction: ShaderVec3::default(),
			cone_cosines: [0.0; 2],
			light_type: 0,
			shadow_views: [0; 8],
			shadow_layer: 0,
			ies_profile_texture: NO_IES_PROFILE_TEXTURE,
			ies_c0_tangent: [32768, 32768],
			_ies_padding: [0; 2],
		}
	}
}

/// The `MaterialData` struct retains fixed-width material texture indices for frame-local GPU uploads.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct MaterialData {
	pub(crate) textures: [u32; MAX_MATERIAL_TEXTURES],
	pub(crate) coverage_factor: f32,
	pub(crate) coverage_texture_slot: u32,
	pub(crate) alpha_cutoff: f32,
	pub(crate) _padding: u32,
}

impl Default for MaterialData {
	fn default() -> Self {
		Self {
			textures: [u32::MAX; MAX_MATERIAL_TEXTURES],
			coverage_factor: 1.0,
			coverage_texture_slot: u32::MAX,
			alpha_cutoff: 0.0,
			_padding: 0,
		}
	}
}

/// Replaces one complete canonical material record and reports whether input slots were truncated.
pub(crate) fn write_material_texture_indices(
	material_data: &mut MaterialData,
	texture_indices: impl IntoIterator<Item = Option<u32>>,
) -> bool {
	*material_data = MaterialData::default();

	for (slot, texture_index) in texture_indices.into_iter().enumerate() {
		let Some(destination) = material_data.textures.get_mut(slot) else {
			return true;
		};
		*destination = texture_index.unwrap_or(u32::MAX);
	}

	false
}

/// The `RenderEntity` struct preserves an owned renderable payload beside its resident scene instance.
pub struct RenderEntity {
	pub(crate) handle: Handle,
	/// Cached dependency-closure handle used by frame-local admission.
	pub(crate) availability: AvailabilityHandle,
	pub(crate) renderable: RenderableMesh,
	pub(crate) shader_mesh: ShaderMesh,
	pub(crate) skinning: Option<RenderSkin>,
}

/// Identifies visibility objects and resources in the shared availability graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityAvailability {
	Renderable(Handle),
	Material(u32),
	Texture(u32),
}

impl RenderEntity {
	/// Replaces the model matrix used by this primitive's visibility instance.
	pub(crate) fn set_model_transform(&mut self, model: AffineShaderMatrix) {
		self.shader_mesh.model = model;
	}
}

/// The `RenderSkin` struct keeps one primitive's immutable skin source and palette mapping beside its scene instance.
pub(crate) struct RenderSkin {
	pub(crate) binding: Arc<SkinBinding>,
	pub(crate) source_vertex_offset: u32,
	pub(crate) vertex_count: u32,
	pub(crate) skeleton_node_count: u32,
}

/// The `PendingRenderableInstance` struct associates an owned renderable payload with the mesh resource it is waiting for.
pub(crate) struct PendingRenderableInstance {
	pub(crate) handle: Handle,
	pub(crate) renderable: RenderableMesh,
	pub(crate) mesh_key: VisibilityMeshKey,
}

/// The `RenderDescription` struct retains one material's render-thread pipeline and authored alpha contract.
pub(crate) struct RenderDescription {
	pub(crate) index: u32,
	pub(crate) pipeline: Option<ghi::PipelineHandle>,
	pub(crate) pipeline_ref: crate::rendering::PipelineRef,
	pub(crate) name: String,
	pub(crate) alpha_mode: AlphaMode,
	pub(crate) texture_indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The `Instance` struct identifies one dense shader mesh and the work needed to rasterize it.
pub struct Instance {
	pub shader_mesh_index: u32,
	pub meshlet_count: u32,
}

/// The `RenderInfo` struct groups frame-local visibility work by the phase that will consume it.
pub struct RenderInfo {
	pub(crate) opaque_instances: Vec<Instance>,
	pub(crate) masked_instances: Vec<Instance>,
	pub(crate) transparent_instances: Vec<Instance>,
	pub(crate) skinning_dispatches: Vec<SkinningDispatch>,
	pub(crate) opaque_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	pub(crate) transparent_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	pub(crate) opaque_material_mask: ActiveMaterialMask,
	pub(crate) transparent_material_mask: ActiveMaterialMask,
}

impl RenderInfo {
	/// Clears frame-local instance work while retaining the allocations used by prior frames.
	pub(crate) fn clear_active_instances(&mut self) {
		self.opaque_instances.clear();
		self.masked_instances.clear();
		self.transparent_instances.clear();
		self.skinning_dispatches.clear();
		self.opaque_material_mask.fill(0);
		self.transparent_material_mask.fill(0);
	}

	/// Adds one active primitive to its authored material phase.
	pub(crate) fn push_active_instance(&mut self, instance: Instance, material_index: u32, alpha_mode: &AlphaMode) {
		let material_index = material_index as usize;

		assert!(
			material_index < MAX_MATERIALS,
			"Visibility material index is out of range. The most likely cause is that an active primitive references a material beyond MAX_MATERIALS."
		);
		let material_bit = 1u64 << (material_index % u64::BITS as usize);
		let material_word = material_index / u64::BITS as usize;
		if is_transparent(alpha_mode) {
			self.transparent_instances.push(instance);
			self.transparent_material_mask[material_word] |= material_bit;
		} else if matches!(alpha_mode, AlphaMode::Mask(_)) {
			self.masked_instances.push(instance);
			self.opaque_material_mask[material_word] |= material_bit;
		} else {
			self.opaque_instances.push(instance);
			self.opaque_material_mask[material_word] |= material_bit;
		}
	}

	pub(crate) fn active_instance_count(&self) -> usize {
		self.opaque_instances.len() + self.masked_instances.len() + self.transparent_instances.len()
	}
}

/// Returns whether an authored alpha mode requires source-over rendering after the opaque phase.
pub(crate) fn is_transparent(alpha_mode: &AlphaMode) -> bool {
	matches!(alpha_mode, AlphaMode::Blend)
}

pub struct SinkState {
	pub(crate) id: usize,
	pub(crate) render_pass: VisibilityPipelineRenderPass,
}

/// The `MeshData` struct retains the mesh ranges and skeleton size needed by the
/// renderer after resource loading.
#[derive(Debug, Clone)]
pub struct MeshData {
	// (material_id)
	pub(crate) primitives: Vec<MeshPrimitive>,
	/// Number of global pose matrices expected from a renderable using this mesh.
	pub(crate) skeleton_node_count: u32,
	/// Base position in the vertex buffer.
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub(crate) triangle_offset: u32,
	/// Base position in the meshlet buffer, relative to the mesh.
	pub(crate) meshlet_offset: u32,
	pub(crate) acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The index of the material used by this primitive.
	pub(crate) material_index: u32,
	/// The meshlet count.
	pub(crate) meshlet_count: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the primitive in the mesh
	pub(crate) meshlet_offset: u32,
	/// The vertex offset.
	/// The base position into the vertex buffer
	pub(crate) vertex_offset: u32,
	/// The primitive indices offset.
	/// The base position into the primitive indices buffer
	pub(crate) primitive_offset: u32,
	/// The triangle offset.
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	pub(crate) triangle_offset: u32,
	/// Base vertex in the compact immutable skinning source buffers.
	pub(crate) skinning_source_vertex_offset: Option<u32>,
	/// Number of vertices written by this primitive's compute dispatch.
	pub(crate) skinning_vertex_count: u32,
	/// Palette mapping retained after the resource reference leaves the upload worker.
	pub(crate) skin: Option<Arc<SkinBinding>>,
}
