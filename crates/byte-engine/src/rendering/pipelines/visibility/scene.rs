//! Scene-visible visibility state: resident render entities, lights, poses, and the per-frame buffers they feed.

use std::sync::Arc;

use ghi::frame::Frame as _;
use log::warn;
use math::{AffineShaderMatrix, Matrix};
use resource_management::resources::skeleton::{AffineMatrix4x3Columns, SkinBinding};
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
use crate::rendering::lights::{IesProfile, Lights};
use crate::rendering::utils::affine_matrix4x3_from_matrix4;
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
		self.transparent_instances.clear();
		self.skinning_dispatches.clear();
		self.opaque_material_mask.fill(0);
		self.transparent_material_mask.fill(0);
	}

	/// Adds one active primitive to the phase selected by its authored alpha mode.
	pub(crate) fn push_active_instance(&mut self, instance: Instance, material_index: u32, alpha_mode: &AlphaMode) {
		let material_index = material_index as usize;
		assert!(
			material_index < MAX_MATERIALS,
			"Visibility material index is out of range. The most likely cause is that an active primitive references a material beyond MAX_MATERIALS."
		);
		let material_bit = 1u64 << (material_index % u64::BITS as usize);
		let material_word = material_index / u64::BITS as usize;
		let (instances, mask) = match alpha_mode {
			AlphaMode::Blend => (&mut self.transparent_instances, &mut self.transparent_material_mask),
			AlphaMode::Mask(_) => (&mut self.masked_instances, &mut self.opaque_material_mask),
			AlphaMode::Opaque => (&mut self.opaque_instances, &mut self.opaque_material_mask),
		};
		instances.push(instance);
		mask[material_word] |= material_bit;
	}

	pub(crate) fn active_instance_count(&self) -> usize {
		self.opaque_instances.len() + self.masked_instances.len() + self.transparent_instances.len()
	}
}

pub struct SinkState {
	pub(crate) id: usize,
	pub(crate) render_pass: VisibilityRenderPass,
}

/// The `VisibilityScene` struct owns everything the renderer retains between frames for one visibility world.
pub struct VisibilityScene {
	pub(crate) render_entities: StableVec<RenderEntity>,
	/// Retained global poses keyed by renderable handle.
	pub(crate) skinning_poses: HashMap<Handle, Vec<AffineMatrix4x3Columns>>,
	/// Scene-instance slots grouped by renderable handle.
	pub(crate) render_entity_handles: HashMap<Handle, SmallVec<[StableVecHandle; 1]>>,
	pub(crate) lights: StableVec<(Handle, Lights, Transform)>,
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
		let model: AffineShaderMatrix = transform.get_matrix().into();
		update_renderable_instances(&self.render_entity_handles, &mut self.render_entities, handle, |entity| {
			entity.shader_mesh.model = model;
		});
		if let Some((_, _, light_transform)) = self.lights.iter_mut().find(|(light_handle, ..)| *light_handle == handle) {
			*light_transform = transform.clone();
		}
	}

	/// Retains one global transform per skeleton node for the renderable identified by `handle`.
	///
	/// A pose remains active until it is replaced or the renderable is removed.
	pub fn write_skinned_pose(&mut self, handle: Handle, global_matrices: &[Matrix]) {
		let pose = self.skinning_poses.entry(handle).or_default();
		pose.clear();
		pose.extend(global_matrices.iter().map(|matrix| {
			assert_affine_matrix(matrix);
			affine_matrix4x3_from_matrix4(matrix)
		}));
	}

	/// Removes all scene state owned by the renderable identified by `handle`.
	pub(crate) fn remove_renderable(&mut self, handle: Handle) {
		self.skinning_poses.remove(&handle);
		if let Some(render_entity_handles) = self.render_entity_handles.remove(&handle) {
			for render_entity_handle in render_entity_handles {
				self.render_entities.remove(render_entity_handle);
			}
		}
	}

	pub(crate) fn remove_light(&mut self, handle: Handle) {
		let slot = self
			.lights
			.handled_iter()
			.find(|(_, (light_handle, ..))| *light_handle == handle)
			.map(|(slot, _)| slot);
		if let Some(slot) = slot {
			self.lights.remove(slot);
		}
	}

	/// Uploads the current scene lights to the GPU buffer used by material evaluation.
	pub(crate) fn write_lighting(
		&self,
		frame: &mut ghi::implementation::Frame,
		shadows: &ShadowLightSelection<'_>,
		mut resolve_ies_profile: impl FnMut(&Lights) -> Option<IesProfileTexture>,
	) {
		if self.lights.len() > MAX_LIGHTS {
			warn!(
				"Too many lights for the visibility pipeline. The most likely cause is that the scene contains more than {MAX_LIGHTS} lights."
			);
		}
		let lighting_data = frame.get_mut_dynamic_buffer_slice(self.lighting_buffer);
		// Rewrite the complete record so recycled frame sequences cannot retain stale counts, lights, or padding.
		*lighting_data = LightingData::default();
		for (index, (_, light, transform)) in self.lights.iter().take(MAX_LIGHTS).enumerate() {
			lighting_data.lights[index] = light_data(light, transform, shadows.shadow_for(index), resolve_ies_profile(light));
			lighting_data.count = index as u32 + 1;
		}
		frame.sync_buffer(self.lighting_buffer);
	}
}

/// Builds one GPU light record from a scene light, its retained transform, and its shadow assignment.
fn light_data(light: &Lights, transform: &Transform, shadow: LightShadow, ies_texture: Option<IesProfileTexture>) -> LightData {
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
				..LightData::default()
			};
		}
		Lights::Cone(light) => (
			light.ies_profile(),
			[light.inner_angle.cos(), light.outer_angle.cos()],
			1,
			ShaderVec3::from(light.color),
		),
		Lights::Point(light) => (light.ies_profile(), [0.0; 2], 0, ShaderVec3::from(light.color)),
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
		_ies_padding: [0; 2],
	}
}

/// Returns the authored IES profile of a local light.
pub(crate) fn ies_profile(light: &Lights) -> Option<&IesProfile> {
	match light {
		Lights::Cone(light) => light.ies_profile(),
		Lights::Point(light) => light.ies_profile(),
		Lights::Direction(_) => None,
	}
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

/// Applies one update to every live scene instance registered for one renderable.
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
	use math::{Matrix, Orientation, Point, UnitVector, WorldSpace};
	use maths_rs::{Vec3f, mat::MatNew4 as _};

	use super::*;
	use crate::core::factory::Factory;
	use crate::rendering::lights::{ConeLight, DirectionalLight, LightColor, PhotometricIntensity, PointLight};

	#[test]
	#[should_panic(expected = "Skinned pose matrix is projective")]
	fn compact_pose_rejects_a_projective_matrix() {
		assert_affine_matrix(&Matrix::new(
			1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		));
	}

	#[test]
	fn transform_updates_write_each_registered_primitive() {
		let renderable_factory = Factory::new();
		let handle = renderable_factory.create(());
		let other_handle = renderable_factory.create(());
		let mut render_entities = StableVec::new();
		let first_primitive = render_entities.push(0u32);
		let second_primitive = render_entities.push(0u32);
		let other_primitive = render_entities.push(0u32);
		let mut render_entity_handles = HashMap::default();
		render_entity_handles.insert(handle, SmallVec::from_slice(&[first_primitive, second_primitive]));
		render_entity_handles.insert(other_handle, SmallVec::from_slice(&[other_primitive]));

		update_renderable_instances(&render_entity_handles, &mut render_entities, handle, |primitive| {
			*primitive += 1
		});

		assert_eq!(render_entities[first_primitive], 1);
		assert_eq!(render_entities[second_primitive], 1);
		assert_eq!(render_entities[other_primitive], 0);
	}

	#[test]
	fn light_data_uses_the_retained_transform_for_spatial_fields() {
		let orientation = Orientation::try_from_axis_angle(
			UnitVector::<WorldSpace>::y_axis(),
			math::Radians::new(std::f32::consts::FRAC_PI_2),
		)
		.expect("finite light orientation");
		let transform = Transform::new(Point::new(1.0, 2.0, 3.0), math::Scale::identity(), orientation);
		let light = ConeLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			math::Degrees::new(20.0).to_radians(),
			math::Degrees::new(35.0).to_radians(),
		)
		.expect("physical cone light");
		let data = light_data(
			&Lights::Cone(light.clone()),
			&transform,
			LightShadow::Cone { view_index: 6, layer: 1 },
			None,
		);

		assert_eq!(data.position, ShaderVec3::from(transform.get_position().into_maths()));
		assert_eq!(
			data.direction,
			ShaderVec3::from(math::direction_from_orientation(orientation).into_maths())
		);
		assert_eq!(data.cone_cosines, [light.inner_angle.cos(), light.outer_angle.cos()]);
		assert_eq!(data.shadow_views, [6, 0, 0, 0, 0, 0, 0, 0]);
		assert_eq!(data.shadow_layer, 1);
	}

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
