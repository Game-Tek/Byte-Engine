//! CPU mirrors of the GPU records read by the visibility shaders.
//!
//! Every type here is `repr(C)` and matches a struct declared in [`super::shader_generator::VisibilityShaderScope`].
//! The layout tests at the bottom pin the offsets the shaders depend on.

use math::{AffineShaderMatrix, ShaderMatrix};

use super::layout::{MAX_LIGHTS, MAX_MATERIAL_TEXTURES, RuntimeUnitVector};
use crate::rendering::View;

/// The `ShaderMesh` struct is one entry of the per-frame instance table read by culling, rasterization, and material evaluation.
#[repr(C, align(16))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderMesh {
	pub(crate) model: AffineShaderMatrix,
	pub(crate) material_index: u32,
	/// Base position in the vertex attribute buffers.
	pub(crate) base_vertex_index: u32,
	/// Base position in the vertex-index buffer.
	pub(crate) base_primitive_index: u32,
	/// Base triangle in the primitive-index buffer.
	pub(crate) base_triangle_index: u32,
	pub(crate) base_meshlet_index: u32,
	pub(crate) meshlet_count: u32,
	/// Base vertex in the frame-local deformation buffer, or `u32::MAX` for immutable geometry.
	pub(crate) skinned_base_vertex_index: u32,
	pub(crate) _padding: u32,
}

/// The `ShaderViewData` struct is one entry of the `views` buffer: the camera view followed by every shadow view.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderViewData {
	pub(crate) view: AffineShaderMatrix,
	pub(crate) view_projection: ShaderMatrix,
	pub(crate) inverse_view: AffineShaderMatrix,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
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

/// The `ShaderVec3` struct pads a vector to the 16-byte stride every storage-buffer backend agrees on.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderVec3 {
	pub(crate) x: f32,
	pub(crate) y: f32,
	pub(crate) z: f32,
	pub(crate) _padding: f32,
}

impl From<(f32, f32, f32)> for ShaderVec3 {
	fn from((x, y, z): (f32, f32, f32)) -> Self {
		Self { x, y, z, _padding: 0.0 }
	}
}

impl From<maths_rs::Vec3f> for ShaderVec3 {
	fn from(value: maths_rs::Vec3f) -> Self {
		Self::from((value.x, value.y, value.z))
	}
}

impl ShaderVec3 {
	pub(crate) fn scaled(self, scale: f32) -> Self {
		Self::from((self.x * scale, self.y * scale, self.z * scale))
	}
}

/// Sentinel used when a local light has no resident IES profile texture.
pub(crate) const NO_IES_PROFILE_TEXTURE: u32 = u32::MAX;
/// Octahedral encoding of the +Z axis, used where no meaningful tangent exists.
pub(crate) const NEUTRAL_UNIT_VECTOR: RuntimeUnitVector = [32768, 32768];

/// The `IesProfileTexture` struct carries a resident intensity map's bindless slot and calibrated candela scale.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct IesProfileTexture {
	pub(crate) texture_index: u32,
	pub(crate) intensity_scale_candela: f32,
}

/// The `LightData` struct is one scene-referred light record consumed by material evaluation.
///
/// `color` stores RGB illuminance in lux for directional lights and RGB luminous intensity in candela for local
/// lights. IES-backed local lights resolve their calibrated candela scale into `color` on the CPU, so the shader
/// only samples a normalized profile when `ies_profile_texture` is not [`NO_IES_PROFILE_TEXTURE`].
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
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
			ies_c0_tangent: NEUTRAL_UNIT_VECTOR,
			_ies_padding: [0; 2],
		}
	}
}

/// The `LightingData` struct is the complete light table uploaded once per frame.
#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingData {
	pub count: u32,
	pub(crate) _padding: [u32; 3],
	pub lights: [LightData; MAX_LIGHTS],
}

/// The `MaterialData` struct is one entry of the material table: bindless texture slots plus coverage controls.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
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

impl MaterialData {
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::pipelines::visibility::layout::{MESH_DATA_BUFFER_STRIDE, VIEW_DATA_BUFFER_STRIDE};

	#[test]
	fn shader_mesh_matches_gpu_buffer_layout() {
		assert_eq!(std::mem::size_of::<ShaderMesh>(), 80);
		assert_eq!(std::mem::size_of::<ShaderMesh>() as u32, MESH_DATA_BUFFER_STRIDE);
		assert_eq!(std::mem::align_of::<ShaderMesh>(), 16);
		assert_eq!(std::mem::offset_of!(ShaderMesh, material_index), 48);
		assert_eq!(std::mem::offset_of!(ShaderMesh, skinned_base_vertex_index), 72);
	}

	#[test]
	fn shader_view_data_matches_compact_gpu_buffer_layout() {
		assert_eq!(std::mem::size_of::<ShaderViewData>(), 176);
		assert_eq!(std::mem::size_of::<ShaderViewData>() as u32, VIEW_DATA_BUFFER_STRIDE);
		assert_eq!(std::mem::offset_of!(ShaderViewData, view), 0);
		assert_eq!(std::mem::offset_of!(ShaderViewData, view_projection), 48);
		assert_eq!(std::mem::offset_of!(ShaderViewData, inverse_view), 112);
		assert_eq!(std::mem::offset_of!(ShaderViewData, fov), 160);
		assert_eq!(std::mem::offset_of!(ShaderViewData, near), 168);
		assert_eq!(std::mem::offset_of!(ShaderViewData, far), 172);
	}

	#[test]
	fn lighting_data_matches_gpu_buffer_layout() {
		assert_eq!(std::mem::size_of::<LightData>(), 112);
		assert_eq!(std::mem::align_of::<LightData>(), 16);
		assert_eq!(std::mem::offset_of!(LightData, position), 0);
		assert_eq!(std::mem::offset_of!(LightData, color), 16);
		assert_eq!(std::mem::offset_of!(LightData, direction), 32);
		assert_eq!(std::mem::offset_of!(LightData, cone_cosines), 48);
		assert_eq!(std::mem::offset_of!(LightData, light_type), 56);
		assert_eq!(std::mem::offset_of!(LightData, shadow_views), 60);
		assert_eq!(std::mem::offset_of!(LightData, shadow_layer), 92);
		assert_eq!(std::mem::offset_of!(LightData, ies_profile_texture), 96);
		assert_eq!(std::mem::offset_of!(LightData, ies_c0_tangent), 100);
		assert_eq!(std::mem::offset_of!(LightData, _ies_padding), 104);
		assert_eq!(std::mem::size_of::<LightingData>(), 1808);
		assert_eq!(std::mem::align_of::<LightingData>(), 16);
		assert_eq!(std::mem::offset_of!(LightingData, count), 0);
		assert_eq!(std::mem::offset_of!(LightingData, _padding), 4);
		assert_eq!(std::mem::offset_of!(LightingData, lights), 16);
	}

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

	#[test]
	fn material_texture_updates_report_truncated_slots() {
		let mut material_data = MaterialData::default();
		assert!(material_data.set_textures([Some(5); MAX_MATERIAL_TEXTURES + 1]));
		assert!(material_data.textures.iter().all(|index| *index == 5));
	}
}
