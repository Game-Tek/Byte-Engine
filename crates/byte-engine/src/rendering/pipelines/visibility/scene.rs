//! Scene-visible visibility state: resident render entities, lights, poses, and the per-frame buffers they feed.

use std::sync::Arc;

use ghi::frame::Frame as _;
use math::Matrix;
use resource_management::resources::skeleton::SkinBinding;
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::hash::HashMap;
use utils::{AvailabilityHandle, StableVec, StableVecHandle};

use super::geometry::encode_octahedral_unit_vector;
use super::layout::{ActiveMaterialMask, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, SHADOW_CASCADE_COUNT, SHADOW_VIEW_COUNT};
use super::render_pass::VisibilityRenderPass;
use super::shader_data::{
	IesProfileTexture, LightData, LightingData, NEUTRAL_UNIT_VECTOR, NO_IES_PROFILE_TEXTURE, ShaderMesh, ShaderVec3,
	ShaderViewData,
};
use super::shadow_selection::{LightShadow, ShadowLightSelection};
use super::skinning::SkinningDispatch;
use crate::core::factory::Handle;
use crate::gameplay::transform::Transform;
use crate::rendering::lights::Lights;
use crate::space::{Orientable as _, Positionable as _};

/// The `Instance` struct identifies one dense shader mesh and the work needed to rasterize it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
	pub shader_mesh_index: u32,
	pub meshlet_count: u32,
}

/// The `RenderEntity` struct is one resident primitive of a renderable, ready to become a frame instance.
pub struct RenderEntity {
	pub(crate) handle: Handle,
	/// Dependency-closure handle checked during frame-local admission.
	pub(crate) availability: AvailabilityHandle,
	pub(crate) shader_mesh: ShaderMesh,
	pub(crate) skinning: Option<RenderSkin>,
}

/// The `RenderSkin` struct keeps one primitive's immutable skin source and palette mapping beside its scene instance.
pub(crate) struct RenderSkin {
	pub(crate) binding: Arc<SkinBinding>,
	pub(crate) source_vertex_offset: u32,
	pub(crate) vertex_count: u32,
	pub(crate) skeleton_node_count: u32,
}

/// One material ready for evaluation: its debug name, table slot, and compiled pipeline.
pub(crate) type MaterialEntry = (String, u32, ghi::PipelineHandle);

/// The `RenderInfo` struct groups frame-local visibility work by the phase that consumes it.
#[derive(Default)]
pub struct RenderInfo {
	pub(crate) opaque_instances: Vec<Instance>,
	pub(crate) masked_instances: Vec<Instance>,
	/// Opaque instances whose material shows both faces, drawn without back-face culling.
	pub(crate) double_sided_instances: Vec<Instance>,
	/// Masked instances whose material shows both faces. They are kept apart from the opaque ones so only they pay for
	/// the alpha test, which also stops the GPU from discarding hidden surfaces early.
	pub(crate) double_sided_masked_instances: Vec<Instance>,
	pub(crate) transparent_instances: Vec<Instance>,
	pub(crate) skinning_dispatches: Vec<SkinningDispatch>,
	pub(crate) opaque_materials: Vec<MaterialEntry>,
	pub(crate) transparent_materials: Vec<MaterialEntry>,
	pub(crate) opaque_material_mask: ActiveMaterialMask,
	pub(crate) transparent_material_mask: ActiveMaterialMask,
}

impl RenderInfo {
	/// Clears frame-local instance work while retaining the allocations used by prior frames.
	pub(crate) fn clear_active_instances(&mut self) {
		self.opaque_instances.clear();
		self.masked_instances.clear();
		self.double_sided_instances.clear();
		self.double_sided_masked_instances.clear();
		self.transparent_instances.clear();
		self.skinning_dispatches.clear();
		self.opaque_material_mask.fill(0);
		self.transparent_material_mask.fill(0);
	}

	/// Adds one active primitive to the phase selected by its authored alpha mode and sidedness.
	///
	/// Double-sided blend primitives stay in the transparent phase, which still culls back faces.
	pub(crate) fn push_active_instance(
		&mut self,
		instance: Instance,
		material_index: u32,
		alpha_mode: &AlphaMode,
		double_sided: bool,
	) {
		let material_index = material_index as usize;
		assert!(
			material_index < MAX_MATERIALS,
			"Visibility material index is out of range. The most likely cause is that an active primitive references a material beyond MAX_MATERIALS."
		);
		let material_bit = 1u64 << (material_index % u64::BITS as usize);
		let material_word = material_index / u64::BITS as usize;
		let (instances, mask) = match alpha_mode {
			AlphaMode::Blend => (&mut self.transparent_instances, &mut self.transparent_material_mask),
			AlphaMode::Mask(_) if double_sided => (&mut self.double_sided_masked_instances, &mut self.opaque_material_mask),
			AlphaMode::Mask(_) => (&mut self.masked_instances, &mut self.opaque_material_mask),
			AlphaMode::Opaque if double_sided => (&mut self.double_sided_instances, &mut self.opaque_material_mask),
			AlphaMode::Opaque => (&mut self.opaque_instances, &mut self.opaque_material_mask),
		};
		instances.push(instance);
		mask[material_word] |= material_bit;
	}

	pub(crate) fn active_instance_count(&self) -> usize {
		self.opaque_instances.len()
			+ self.masked_instances.len()
			+ self.double_sided_instances.len()
			+ self.double_sided_masked_instances.len()
			+ self.transparent_instances.len()
	}
}

pub struct SinkState {
	pub(crate) id: usize,
	pub(crate) render_pass: VisibilityRenderPass,
	/// Fills sky pixels of the lit target before transparent surfaces composite over them.
	pub(crate) background: Option<crate::rendering::render_pass::SceneBackground>,
}

/// The `VisibilityScene` struct owns everything the renderer retains between frames for one visibility world.
pub struct VisibilityScene {
	pub(crate) render_entities: StableVec<RenderEntity>,
	/// Retained global poses keyed by renderable handle.
	pub(crate) skinning_poses: HashMap<Handle, Vec<math::AffineMatrix>>,
	/// Scene-instance slots grouped by renderable handle.
	pub(crate) render_entity_handles: HashMap<Handle, SmallVec<[StableVecHandle; 1]>>,
	pub(crate) lights: StableVec<(Handle, Lights, Transform)>,
	/// Light slots grouped by light handle.
	pub(crate) light_handles: HashMap<Handle, SmallVec<[StableVecHandle; 1]>>,
	/// Shared base descriptor set bound by every visibility pass.
	pub(crate) descriptor_set: ghi::DescriptorSetHandle,
	pub(crate) views_buffer: ghi::DynamicBufferHandle<[ShaderViewData; SHADOW_VIEW_COUNT]>,
	pub(crate) meshes_buffer: ghi::DynamicBufferHandle<[ShaderMesh; MAX_INSTANCES]>,
	pub(crate) lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
	pub(crate) render_info: RenderInfo,
	pub(crate) sink_states: Vec<SinkState>,
}

impl VisibilityScene {
	/// Registers a renderable primitive and records its scene-instance slot.
	pub(crate) fn add_render_entity(&mut self, render_entity: RenderEntity) {
		let renderable_handle = render_entity.handle;
		let scene_handle = self.render_entities.push(render_entity);
		self.render_entity_handles
			.entry(renderable_handle)
			.or_default()
			.push(scene_handle);
	}

	/// Applies the latest transform update to every primitive and light owned by `handle`.
	pub(crate) fn update_transform(&mut self, handle: Handle, transform: &Transform) {
		let model: ghi::pod::Mat4x3f = transform.get_matrix().into();
		update_renderable_instances(&self.render_entity_handles, &mut self.render_entities, handle, |entity| {
			entity.shader_mesh.model = model;
		});
		update_renderable_instances(&self.light_handles, &mut self.lights, handle, |(_, _, light_transform)| {
			*light_transform = transform.clone();
		});
	}

	/// Registers a light and records its slot.
	pub(crate) fn add_light(&mut self, handle: Handle, light: Lights) {
		let slot = self.lights.push((handle, light, Transform::default()));
		self.light_handles.entry(handle).or_default().push(slot);
	}

	/// Retains one global transform per skeleton node for the renderable identified by `handle`.
	///
	/// A pose remains active until it is replaced or the renderable is removed.
	pub fn write_skinned_pose(&mut self, handle: Handle, global_matrices: &[Matrix]) {
		let pose = self.skinning_poses.entry(handle).or_default();
		pose.clear();
		pose.extend(global_matrices.iter().map(|matrix| {
			assert_affine_matrix(matrix);
			math::AffineMatrix::from_matrix(*matrix)
		}));
	}

	/// Removes all scene state owned by the renderable identified by `handle`.
	pub(crate) fn remove_renderable(&mut self, handle: Handle) {
		self.skinning_poses.remove(&handle);
		for render_entity_handle in self.render_entity_handles.remove(&handle).into_iter().flatten() {
			self.render_entities.remove(render_entity_handle);
		}
	}

	/// Removes every light registered for `handle`.
	pub(crate) fn remove_light(&mut self, handle: Handle) {
		for slot in self.light_handles.remove(&handle).into_iter().flatten() {
			self.lights.remove(slot);
		}
	}

	/// Uploads the current scene lights and the frame's lighting scales to the GPU buffer used by material evaluation.
	///
	/// `exposure` is the camera's linear exposure and `environment_intensity` calibrates the environment map; see
	/// [`LightingData`]. `ies_profiles` holds each light's IES intensity scale and resident profile by light index.
	pub(crate) fn write_lighting(
		&self,
		frame: &mut ghi::implementation::Frame,
		shadows: &ShadowLightSelection<'_>,
		exposure: f32,
		environment_intensity: f32,
		ies_profiles: &[(f32, Option<IesProfileTexture>)],
	) {
		let lighting_data = frame.get_mut_dynamic_buffer_slice(self.lighting_buffer);
		// Rewrite the header and every current light, so a recycled frame sequence cannot retain a stale count or
		// light. Entries past the count are never read, so they are left as they are.
		lighting_data.count = 0;
		lighting_data.exposure = exposure;
		lighting_data.environment_intensity = environment_intensity;
		lighting_data._padding = 0;
		for (index, (_, light, transform)) in self.lights.iter().take(MAX_LIGHTS).enumerate() {
			lighting_data.lights[index] = light_data(light, transform, shadows.shadow_for(index), ies_profiles[index].1);
			lighting_data.count = index as u32 + 1;
		}
		frame.sync_buffer(self.lighting_buffer);
	}
}

/// The exposed illuminance below which a local light stops lighting a cluster.
///
/// A white diffuse surface lit by this much reflects about `1 / (1024 π)` of display white, which stays near the
/// smallest step an 8-bit display shows. Lights keep their inverse-square falloff, so leaving a light out past its
/// reach does not visibly change the image.
pub(crate) const LIGHT_REACH_THRESHOLD_LUX: f32 = 1.0 / 1024.0;

/// Builds one GPU light record from a scene light, its retained transform, and its shadow assignment.
pub(super) fn light_data(
	light: &Lights,
	transform: &Transform,
	shadow: LightShadow,
	ies_texture: Option<IesProfileTexture>,
) -> LightData {
	let (shadow_views, shadow_layer) = match shadow {
		LightShadow::None => ([0; 8], 0),
		LightShadow::Directional => (
			std::array::from_fn(|cascade| (cascade < SHADOW_CASCADE_COUNT) as u32 * (cascade as u32 + 1)),
			0,
		),
		LightShadow::Cone { view_index, layer } => ([view_index, 0, 0, 0, 0, 0, 0, 0], layer),
		LightShadow::Point { view_index, cube_index } => ([view_index, 0, 0, 0, 0, 0, 0, 0], cube_index),
	};
	let position = transform.position().into_maths();
	let orientation = transform.orientation();
	let direction = math::direction_from_orientation(orientation).into_maths();
	let (profile, cone_cosines, light_type, color) = match light {
		Lights::Direction(light) => {
			return LightData {
				position: direction.into(),
				color: light.color.into(),
				light_type: 68,
				shadow_views,
				angular_radius_tangent: light.angular_radius.value().tan(),
				..LightData::default()
			};
		}
		Lights::Cone(light) => (
			light.emission.ies_profile(),
			[light.inner_angle.cos(), light.outer_angle.cos()],
			1,
			ShaderVec3::from(light.emission.color),
		),
		Lights::Point(light) => (
			light.emission.ies_profile(),
			[0.0; 2],
			0,
			ShaderVec3::from(light.emission.color),
		),
	};
	let (color, ies_profile_texture, ies_c0_tangent) = match (profile, ies_texture) {
		(None, _) => (color, NO_IES_PROFILE_TEXTURE, NEUTRAL_UNIT_VECTOR),
		// Dimmed fallback until the profile texture is resident.
		(Some(profile), None) => (color.scaled(profile.dimmer()), NO_IES_PROFILE_TEXTURE, NEUTRAL_UNIT_VECTOR),
		(Some(_), Some(texture)) => {
			let tangent = orientation
				.rotate_vector(math::UnitVector::<math::WorldSpace>::x_axis().into_vector())
				.into_maths();
			(
				color.scaled(texture.intensity_scale_candela),
				texture.texture_index,
				encode_octahedral_unit_vector((tangent.x, tangent.y, tangent.z)),
			)
		}
	};
	LightData {
		position: position.into(),
		color,
		direction: direction.into(),
		cone_cosines,
		light_type,
		shadow_views,
		shadow_layer,
		ies_profile_texture,
		ies_c0_tangent,
		reach: light_reach(color),
		angular_radius_tangent: 0.0,
	}
}

/// Returns how far a local light with peak RGB intensity `color`, in candela, lights at an exposure of one.
///
/// The brightest channel sets the reach, so a saturated light keeps its full reach. A light with unusable
/// intensity gets no reach and lights nothing.
fn light_reach(color: ShaderVec3) -> f32 {
	let reach = (color.x.max(color.y).max(color.z) / LIGHT_REACH_THRESHOLD_LUX).sqrt();
	if reach.is_finite() { reach } else { 0.0 }
}

/// Rejects projective pose data before the compact representation would discard it.
fn assert_affine_matrix(matrix: &Matrix) {
	const AFFINE_EPSILON: f32 = 0.00001;
	assert!(
		matrix[(3, 0)].abs() <= AFFINE_EPSILON
			&& matrix[(3, 1)].abs() <= AFFINE_EPSILON
			&& matrix[(3, 2)].abs() <= AFFINE_EPSILON
			&& (matrix[(3, 3)] - 1.0).abs() <= AFFINE_EPSILON,
		"Skinned pose matrix is projective. The most likely cause is sending a view or projection matrix instead of an affine skeleton pose."
	);
}

/// Applies one update to every live scene slot registered for one handle.
fn update_renderable_instances<T>(
	render_entity_handles: &HashMap<Handle, SmallVec<[StableVecHandle; 1]>>,
	render_entities: &mut StableVec<T>,
	handle: Handle,
	mut update: impl FnMut(&mut T),
) {
	for scene_handle in render_entity_handles.get(&handle).into_iter().flatten() {
		if let Some(render_entity) = render_entities.get_mut(*scene_handle) {
			update(render_entity);
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, UnitVector, WorldSpace};
	use maths_rs::Vec3f;

	use super::*;
	use crate::rendering::lights::{DirectionalLight, LightColor, PhotometricIntensity, PointLight};

	#[test]
	fn ies_light_data_rotates_the_c0_tangent_with_the_retained_transform() {
		let orientation = Orientation::try_from_axis_angle(
			UnitVector::<WorldSpace>::y_axis(),
			math::Radians::new(std::f32::consts::FRAC_PI_2),
		)
		.expect("finite IES orientation");
		let transform = Transform::from_rotation(orientation);
		let light = PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.25, "lights/office.ies")
			.expect("physical IES point light");
		let resident = light_data(
			&Lights::Point(light),
			&transform,
			LightShadow::None,
			Some(IesProfileTexture {
				texture_index: 37,
				intensity_scale_candela: 45.0,
			}),
		);
		let tangent = orientation.rotate_vector(UnitVector::<WorldSpace>::x_axis().into_vector());
		let encoded_tangent = encode_octahedral_unit_vector((tangent.x(), tangent.y(), tangent.z()));

		assert_eq!(resident.color, ShaderVec3::from((45.0, 45.0, 45.0)));
		assert_eq!(resident.ies_profile_texture, 37);
		assert_eq!(resident.ies_c0_tangent, encoded_tangent);
	}

	#[test]
	fn a_light_reaches_where_its_brightest_channel_falls_to_the_threshold() {
		let reach = light_reach(ShaderVec3::from((100.0, 400.0, 25.0)));

		assert!((400.0 / (reach * reach) - LIGHT_REACH_THRESHOLD_LUX).abs() < 1.0e-9);
		assert_eq!(light_reach(ShaderVec3::default()), 0.0);
		assert_eq!(light_reach(ShaderVec3::from((f32::NAN, f32::NAN, f32::NAN))), 0.0);
	}

	#[test]
	fn light_data_uploads_directional_lux_and_local_candela_without_unit_tags() {
		let white = LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0));
		let directional = DirectionalLight::new(
			white,
			PhotometricIntensity::Illuminance {
				lux: 80_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light");
		let point = PointLight::new(
			white,
			PhotometricIntensity::Illuminance {
				lux: 25.0,
				measurement_distance_m: 2.0,
			},
		)
		.expect("physical point light");
		let transform = Transform::default();
		let directional_data = light_data(&Lights::Direction(directional), &transform, LightShadow::Directional, None);
		let point_data = light_data(&Lights::Point(point), &transform, LightShadow::None, None);

		assert_eq!(directional_data.color, ShaderVec3::from((80_000.0, 80_000.0, 80_000.0)));
		assert_eq!(directional_data.shadow_views, [1, 2, 3, 4, 0, 0, 0, 0]);
		assert_eq!(point_data.color, ShaderVec3::from((100.0, 100.0, 100.0)));
		assert_eq!(directional_data.light_type, 68);
		assert_eq!(point_data.light_type, 0);
	}
}
