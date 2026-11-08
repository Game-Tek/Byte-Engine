//! CPU mirrors of the GPU records read by the visibility shaders.
//!
//! Every type here is `repr(C)` and matches a struct declared in [`super::shader_generator::visibility_shader_module`].
//! The layout tests at the bottom pin the offsets the shaders depend on.

use ghi::pod::{Mat4f, Mat4x3f, Vec3f};

use super::layout::{LIGHT_CLUSTER_SLICES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, RuntimeUnitVector};
use crate::rendering::View;

/// The `ShaderMesh` struct is one entry of the per-frame instance table read by culling, rasterization, and material
/// evaluation.
///
/// Its geometry lives in the [`ShaderMeshLevel`](super::layout::ShaderMeshLevel) record it names, which every instance drawing that level shares, so
/// the record holds only what differs between instances. Build one with [`Self::new`]; the frame's rebuild names the
/// level and the rest each frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderMesh {
	pub(crate) model: Mat4x3f,
	/// The [`ShaderMeshLevel`](super::layout::ShaderMeshLevel) the instance draws in bits 0-23, and flags such as [`MESH_FLAG_DOUBLE_SIDED`] above it.
	level: u32,
	/// The material table index in bits 0-15, and in bits 16-31 the first entry of the instance's dynamic material
	/// parameters in the material parameter buffer: its own block when the application set a parameter on it, else its
	/// material's. See [`super::material_parameters`].
	material: u32,
	/// A posed skinned instance's first vertex in the first word, or a lightmapped instance's
	/// [`InstanceLightmap::rectangle`].
	surface: [u32; 2],
}

impl ShaderMesh {
	/// Returns the record of an instance of the material table entry `material_index` placed by `model`.
	pub(crate) fn new(model: Mat4x3f, material_index: u32) -> Self {
		debug_assert!(
			material_index <= MESH_MATERIAL_MASK,
			"Material index {material_index} does not fit an instance record."
		);
		Self {
			model,
			level: 0,
			material: material_index,
			surface: [0; 2],
		}
	}

	pub(crate) fn material_index(&self) -> u32 {
		self.material & MESH_MATERIAL_MASK
	}

	#[cfg(test)]
	pub(crate) fn level(&self) -> u32 {
		self.level & MESH_LEVEL_MASK
	}

	#[cfg(test)]
	pub(crate) fn flags(&self) -> u32 {
		self.level & !MESH_LEVEL_MASK
	}

	/// Names the level record the instance draws and its flags, and forgets its lightmap and posed vertices.
	pub(crate) fn set_level(&mut self, level: u32, flags: u32) {
		debug_assert!(level <= MESH_LEVEL_MASK && flags & MESH_LEVEL_MASK == 0);
		self.level = level | flags;
		self.surface = [0; 2];
	}

	/// Sets the first entry of the instance's dynamic material parameters.
	pub(crate) fn set_parameter_block(&mut self, block: u32) {
		debug_assert!(
			block <= u16::MAX as u32,
			"Parameter block {block} does not fit an instance record."
		);
		self.material = self.material_index() | block << 16;
	}

	/// Lights the instance with `lightmap`, or with none for [`InstanceLightmap::NONE`].
	pub(crate) fn set_lightmap(&mut self, lightmap: InstanceLightmap) {
		self.level = self.level & !(MESH_LIGHTMAP_MASK << MESH_LIGHTMAP_SHIFT) | lightmap.slot << MESH_LIGHTMAP_SHIFT;
		self.surface = lightmap.rectangle;
	}

	/// Draws the instance from its own posed vertices, which start at `first_vertex`, instead of its level's. A posed
	/// instance moved away from what a lightmap holds, so it loses its lightmap.
	pub(crate) fn set_posed_vertices(&mut self, first_vertex: u32) {
		self.set_lightmap(InstanceLightmap::NONE);
		self.level |= MESH_FLAG_SKINNED;
		self.surface = [first_vertex, 0];
	}
}

/// Masks the level index out of [`ShaderMesh::level`]; the shaders spell it `MESH_LEVEL_MASK`.
const MESH_LEVEL_MASK: u32 = (1 << 24) - 1;
/// Masks the material table index out of [`ShaderMesh::material`]; the shaders spell it `MESH_MATERIAL_MASK`.
const MESH_MATERIAL_MASK: u32 = u16::MAX as u32;
/// Where [`ShaderMesh::level`] holds one plus the instance's lightmap slot, or zero when no lightmap lights it.
const MESH_LIGHTMAP_SHIFT: u32 = 26;
const MESH_LIGHTMAP_MASK: u32 = 0b1_1111;
const _: () = assert!(
	super::layout::MAX_LIGHTMAPS < MESH_LIGHTMAP_MASK as usize
		&& super::layout::MAX_ADDRESSABLE_LEVELS == MESH_LEVEL_MASK as usize + 1,
	"Update the instance record's bit layout, and the shaders' `scene_records` layer, when the lightmap or level limits change."
);

/// [`ShaderMesh`] flag for a material with both faces visible. Task shaders keep its back-facing meshlets and material
/// evaluation reverses the normal of its back-facing pixels.
pub(crate) const MESH_FLAG_DOUBLE_SIDED: u32 = 1 << 24;
/// [`ShaderMesh`] flag for an instance skinning posed this frame. Posed meshlets can leave their bind-pose bounds, so
/// task shaders keep all of its meshlets in every view.
pub(crate) const MESH_FLAG_SKINNED: u32 = 1 << 25;

/// The `InstanceLightmap` struct says where an instance's light lies in the resident lightmaps, so material evaluation
/// can read it.
///
/// [`super::lightmaps::Lightmaps::instance`] makes one per primitive of a lit renderable, and the frame's rebuild hands
/// it to [`ShaderMesh::set_lightmap`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct InstanceLightmap {
	/// One plus the lightmap's slot in the scene set's `lightmaps`, or zero for an instance no lightmap lights.
	slot: u32,
	/// Maps the instance's lightmap UVs into the lightmap, as 0.16 fixed-point fractions of its side: scale U and V
	/// in the low and high halves of the first word, offset U and V in the second. Rectangles of a power-of-two image
	/// of up to 65,536 texels land on exact values.
	rectangle: [u32; 2],
}

impl InstanceLightmap {
	/// The entry of an instance no lightmap lights.
	pub(crate) const NONE: Self = Self {
		slot: 0,
		rectangle: [0; 2],
	};

	/// Returns the entry of an instance lit by the lightmap in slot `lightmap`, whose UVs map into it as
	/// `uv * scale_offset.xy + scale_offset.zw`.
	pub(crate) fn new(lightmap: u32, scale_offset: [f32; 4]) -> Self {
		debug_assert!((lightmap as usize) < super::layout::MAX_LIGHTMAPS);
		let fixed = |fraction: f32| (fraction.clamp(0.0, 1.0) * 65536.0).round().min(u16::MAX as f32) as u32;
		let [scale_u, scale_v, offset_u, offset_v] = scale_offset.map(fixed);
		Self {
			slot: lightmap + 1,
			rectangle: [scale_u | scale_v << 16, offset_u | offset_v << 16],
		}
	}

	/// Returns the lightmap slot, or `None` for an instance no lightmap lights.
	#[cfg(test)]
	pub(crate) fn lightmap(&self) -> Option<u32> {
		self.slot.checked_sub(1)
	}
}

/// The `ShaderViewData` struct is one entry of the `views` buffer: the camera view followed by every shadow view.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderViewData {
	pub(crate) view: Mat4x3f,
	pub(crate) view_projection: Mat4f,
	pub(crate) inverse_view: Mat4x3f,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
}

/// The `ShaderFrame` struct is the scene layer's `frame` record: what every pass of a frame shares, written once a
/// frame.
#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderFrame {
	/// The frame clock's elapsed seconds, which a material's `time` node reads.
	pub(crate) time: f32,
}

impl From<View> for ShaderViewData {
	fn from(view: View) -> Self {
		Self {
			view: view.view().into(),
			view_projection: view.view_projection().into(),
			inverse_view: math::inverse(view.view()).into(),
			fov: view.fov(),
			near: view.near(),
			far: view.far(),
		}
	}
}

/// Sentinel used when a local light has no resident IES profile.
pub(crate) const NO_IES_PROFILE: u32 = u32::MAX;
/// Octahedral encoding of the +Z axis, used where no meaningful tangent exists.
pub(crate) const NEUTRAL_UNIT_VECTOR: RuntimeUnitVector = [32768, 32768];

/// The `IesProfileTexture` struct carries a resident intensity map's slot in the scene set's `ies_profiles` and its
/// calibrated candela scale.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct IesProfileTexture {
	pub(crate) profile_index: u32,
	pub(crate) intensity_scale_candela: f32,
}

/// The `LightData` struct is one scene-referred light record consumed by material evaluation.
///
/// `color` stores RGB illuminance in lux for directional lights and RGB luminous intensity in candela for local
/// lights. IES-backed local lights resolve their calibrated candela scale into `color` on the CPU, so the shader
/// only samples a normalized profile when `ies_profile` is not [`NO_IES_PROFILE`].
///
/// `reach` is how far a local light visibly lights at an exposure of one. The light-cluster pass scales it by the
/// square root of the frame's exposure and buckets the light into every cluster within that distance.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightData {
	pub position: Vec3f,
	pub color: Vec3f,
	pub direction: Vec3f,
	pub cone_cosines: [f32; 2],
	pub light_type: u32,
	/// A sun's four cascade views, or a cone's view or a point's first cube-face view in the first entry. Zero means
	/// the light has no shadow map.
	pub shadow_views: [u32; 4],
	/// The size class of a cone or point light's shadow map, which picks the image of that class. See
	/// [`super::layout::LOCAL_SHADOW_SIZE_CLASS_COUNT`].
	pub(crate) shadow_size_class: u32,
	pub(crate) _padding: [u32; 3],
	/// A sun's slot, a cone's layer, or a point's cube within the image of its size class.
	pub shadow_layer: u32,
	pub(crate) ies_profile: u32,
	pub(crate) ies_c0_tangent: RuntimeUnitVector,
	pub(crate) reach: f32,
	/// The tangent of a directional light's angular radius, which sizes its shadow penumbrae. Other lights leave it at
	/// zero.
	pub(crate) angular_radius_tangent: f32,
}

impl Default for LightData {
	fn default() -> Self {
		Self {
			ies_profile: NO_IES_PROFILE,
			ies_c0_tangent: NEUTRAL_UNIT_VECTOR,
			..bytemuck::Zeroable::zeroed()
		}
	}
}

/// The `LightingData` struct is the complete light table and per-frame lighting scales uploaded once per frame.
///
/// Only the first `count` lights are current; shaders never read past them.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingData {
	pub count: u32,
	/// The camera exposure as a linear factor. Material evaluation multiplies the light it writes by it, so scenes lit
	/// with real-world intensities stay within the half-float range of the scene color target.
	pub exposure: f32,
	/// The linear factor that brings the environment map to the illuminance its [`crate::rendering::Environment`] requests.
	pub environment_intensity: f32,
	pub lights: [LightData; MAX_LIGHTS],
}

/// The `LightClusterParameters` struct describes how one sink's view frustum splits into light clusters.
///
/// [`super::render_pass::LightClusterPass`] uploads it once per frame. The light-cluster pass reads it to bound each
/// cluster, and material evaluation reads it to find the cluster of the pixel it shades.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightClusterParameters {
	/// Maps world space to the view space the clusters are laid out in, where depth grows along +Z.
	pub(crate) view: Mat4x3f,
	/// The view-space X/Z and Y/Z slopes at the right and top edges of the image.
	pub(crate) edge_slopes: [f32; 2],
	pub(crate) near: f32,
	/// Depth slices per doubling of view depth. A pixel at depth `z` lies in slice `floor(log2(z / near) * scale)`.
	pub(crate) depth_slice_scale: f32,
}

impl From<View> for LightClusterParameters {
	fn from(view: View) -> Self {
		let projection = view.projection();
		Self {
			view: view.view().into(),
			edge_slopes: [1.0 / projection[0], 1.0 / projection[5]],
			near: view.near(),
			// Slices split the whole clip range, so the last slice ends at the far plane.
			depth_slice_scale: LIGHT_CLUSTER_SLICES as f32 / (view.far() / view.near()).log2(),
		}
	}
}

/// The `MaterialData` struct is one entry of the material table: bindless texture slots, the material's arm in the
/// coverage shaders, and the cutoff the masked passes test its opacity against.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MaterialData {
	pub(crate) textures: [u32; MAX_MATERIAL_TEXTURES],
	/// The material's key in the coverage shaders, or `u32::MAX` for a material they don't test.
	pub(crate) coverage_arm: u32,
	pub(crate) alpha_cutoff: f32,
	pub(crate) _padding: [u32; 2],
}

impl Default for MaterialData {
	fn default() -> Self {
		Self {
			textures: [u32::MAX; MAX_MATERIAL_TEXTURES],
			coverage_arm: u32::MAX,
			..bytemuck::Zeroable::zeroed()
		}
	}
}

impl MaterialData {
	/// Creates the record of a material whose variables take the bindless texture slots `textures`, and whose arm in the
	/// coverage shaders is `coverage_arm`, and reports whether it named more textures than a record holds.
	///
	/// The visibility manager records each material it loads with it, and the lightmap bake each material its rays hit.
	pub(crate) fn describe(
		textures: &[Option<u32>],
		coverage_arm: Option<u32>,
		alpha_mode: resource_management::types::AlphaMode,
	) -> (Self, bool) {
		let mut record = Self::default();
		let overflow = record.set_textures(textures.iter().copied());
		record.coverage_arm = coverage_arm.unwrap_or(u32::MAX);
		record.alpha_cutoff = match alpha_mode {
			resource_management::types::AlphaMode::Mask(cutoff) => cutoff,
			resource_management::types::AlphaMode::Opaque | resource_management::types::AlphaMode::Blend => 0.0,
		};
		(record, overflow)
	}

	/// Replaces the texture slots of this record and reports whether the input had more slots than fit.
	pub(crate) fn set_textures(&mut self, texture_indices: impl IntoIterator<Item = Option<u32>>) -> bool {
		self.textures = [u32::MAX; MAX_MATERIAL_TEXTURES];
		for (slot, texture_index) in texture_indices.into_iter().enumerate() {
			let Some(destination) = self.textures.get_mut(slot) else {
				return true;
			};
			*destination = texture_index.unwrap_or(u32::MAX);
		}
		false
	}
}

/// The `ReflectionShaderParameters` struct gives one sink's reflection rays what they need to read last frame's light.
///
/// [`super::render_pass::ScreenSpaceReflections`] uploads it once per frame for material evaluation.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ReflectionShaderParameters {
	/// Maps a world-space position to the previous frame's clip space, to find where the previous camera saw a hit.
	pub(crate) world_to_previous_clip: Mat4f,
	/// The exposure the previous frame's lit image was multiplied by.
	pub(crate) previous_exposure: f32,
	/// Nonzero when the previous frame's lit image and depth hold this sink's light.
	pub(crate) history_valid: u32,
	/// Turns the previous frame's device depth `d` into view depth as `numerator / (d + denominator_offset)`.
	pub(crate) previous_depth_unproject_numerator: f32,
	pub(crate) previous_depth_unproject_denominator_offset: f32,
}

/// The shader-facing structs must keep the exact layouts their GPU buffers are read with.
///
/// The visibility shaders index each buffer by a fixed stride and read fields at fixed offsets. The stride
/// constants in [`layout`](crate::rendering::pipelines::visibility::layout) take their values from these types.
///
/// These are compile-time checks so a layout change fails the build at the definition rather than later
/// in a shader that silently reads the wrong bytes.
const _: () = assert!(std::mem::size_of::<ShaderMesh>() == 64);
const _: () = assert!(std::mem::offset_of!(ShaderMesh, level) == 48);
const _: () = assert!(std::mem::offset_of!(ShaderMesh, material) == 52);
const _: () = assert!(std::mem::offset_of!(ShaderMesh, surface) == 56);

const _: () = assert!(std::mem::size_of::<ShaderViewData>() == 176);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, view) == 0);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, view_projection) == 48);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, inverse_view) == 112);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, fov) == 160);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, near) == 168);
const _: () = assert!(std::mem::offset_of!(ShaderViewData, far) == 172);

const _: () = assert!(std::mem::size_of::<LightData>() == 100);
const _: () = assert!(std::mem::offset_of!(LightData, position) == 0);
const _: () = assert!(std::mem::offset_of!(LightData, color) == 12);
const _: () = assert!(std::mem::offset_of!(LightData, direction) == 24);
const _: () = assert!(std::mem::offset_of!(LightData, cone_cosines) == 36);
const _: () = assert!(std::mem::offset_of!(LightData, light_type) == 44);
const _: () = assert!(std::mem::offset_of!(LightData, shadow_views) == 48);
const _: () = assert!(std::mem::offset_of!(LightData, shadow_size_class) == 64);
const _: () = assert!(std::mem::offset_of!(LightData, shadow_layer) == 80);
const _: () = assert!(std::mem::offset_of!(LightData, ies_profile) == 84);
const _: () = assert!(std::mem::offset_of!(LightData, ies_c0_tangent) == 88);
const _: () = assert!(std::mem::offset_of!(LightData, reach) == 92);
const _: () = assert!(std::mem::offset_of!(LightData, angular_radius_tangent) == 96);

const _: () = assert!(std::mem::size_of::<LightingData>() == 12 + 100 * MAX_LIGHTS);
const _: () = assert!(std::mem::offset_of!(LightingData, count) == 0);
const _: () = assert!(std::mem::offset_of!(LightingData, exposure) == 4);
const _: () = assert!(std::mem::offset_of!(LightingData, environment_intensity) == 8);
const _: () = assert!(std::mem::offset_of!(LightingData, lights) == 12);

const _: () = assert!(std::mem::size_of::<LightClusterParameters>() == 64);
const _: () = assert!(std::mem::offset_of!(LightClusterParameters, edge_slopes) == 48);
const _: () = assert!(std::mem::offset_of!(LightClusterParameters, near) == 56);
const _: () = assert!(std::mem::offset_of!(LightClusterParameters, depth_slice_scale) == 60);

const _: () = assert!(std::mem::size_of::<ReflectionShaderParameters>() == 80);
const _: () = assert!(std::mem::offset_of!(ReflectionShaderParameters, previous_exposure) == 64);
const _: () = assert!(std::mem::offset_of!(ReflectionShaderParameters, history_valid) == 68);
const _: () = assert!(std::mem::offset_of!(ReflectionShaderParameters, previous_depth_unproject_numerator) == 72);
const _: () = assert!(std::mem::offset_of!(ReflectionShaderParameters, previous_depth_unproject_denominator_offset) == 76);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn material_texture_updates_replace_the_complete_record() {
		let mut material_data = MaterialData::default();
		assert!(material_data.textures.iter().all(|texture_index| *texture_index == u32::MAX));
		material_data.textures.fill(41);

		assert!(!material_data.set_textures([Some(7), None, Some(11)]));
		assert_eq!(material_data.textures[..3], [7, u32::MAX, 11]);
		assert!(material_data.textures[3..].iter().all(|index| *index == u32::MAX));

		assert!(!material_data.set_textures([Some(3)]));
		assert_eq!(material_data.textures[0], 3);
		assert!(material_data.textures[1..].iter().all(|index| *index == u32::MAX));
	}
}
