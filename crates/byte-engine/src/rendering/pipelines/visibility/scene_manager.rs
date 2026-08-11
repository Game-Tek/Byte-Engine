pub struct VisibilitySceneManager {
	/// Render entities registered in the scene.
	pub(crate) render_entities: StableVec<RenderEntity>,
	/// Retained global poses keyed by the renderable handle used by this scene.
	pub(crate) skinning_poses: HashMap<Handle, Vec<AffineMatrix4x3Columns>>,
	/// Scene-instance slots grouped by their renderable handle.
	pub(crate) render_entity_handles: HashMap<Handle, SmallVec<[StableVecHandle; 1]>>,
	/// Shared views data buffer used by every visibility sink.
	pub(crate) views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; SHADOW_VIEW_COUNT]>,
	/// Shared base descriptor set used by every visibility pass.
	pub(crate) descriptor_set: ghi::DescriptorSetHandle,
	/// Per-instance mesh data buffer holding transforms and material indices for this scene.
	pub(crate) meshes_data_buffer:
		ghi::DynamicBufferHandle<[ShaderMesh; crate::rendering::pipelines::visibility::MAX_INSTANCES]>, // Using crate::rendering::pipelines::visibility::MAX_INSTANCES to avoid hardcoding MAX_INSTANCES if not exported
	/// Frame-local buffer containing lighting data for this scene.
	pub(crate) light_data_buffer: ghi::DynamicBufferHandle<LightingData>,
	/// Lights in the scene.
	pub(crate) lights: StableVec<(Handle, Lights)>,
	/// Information about the current render.
	pub(crate) render_info: RenderInfo,
	/// Per-sink render state.
	pub(crate) sink_states: Vec<SinkState>,
}

impl VisibilitySceneManager {
	/// Registers a renderable primitive and records its scene-instance slot.
	pub(crate) fn add_render_entity(&mut self, render_entity: RenderEntity) {
		let renderable_handle = render_entity.handle;
		let scene_handle = self.render_entities.push(render_entity);
		self.render_entity_handles
			.entry(renderable_handle)
			.or_default()
			.push(scene_handle);
	}

	/// Applies the latest transform update to every primitive owned by `handle`.
	pub(crate) fn update_renderable_transform(&mut self, handle: Handle, transform: &Transform) {
		let model = transform.get_matrix().into();
		update_renderable_instances(
			&self.render_entity_handles,
			&mut self.render_entities,
			handle,
			|render_entity| render_entity.set_model_transform(model),
		);
	}

	/// Retains one global transform per skeleton node for the renderable identified by `handle`.
	///
	/// Rewriting an existing pose reuses its allocation when the skeleton size is unchanged. A
	/// pose remains active until it is replaced or the corresponding renderable is removed.
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

	/// Uploads the current scene lights to the GPU buffer used by material evaluation.
	pub(crate) fn write_light_data(
		&self,
		frame: &mut ghi::implementation::Frame,
		directional_shadow_light_index: Option<usize>,
		cone_shadow_light_indices: &[Option<usize>; MAX_CONE_SHADOW_POOL_CAPACITY],
		point_shadow_light_indices: &[Option<usize>; MAX_POINT_SHADOW_POOL_CAPACITY],
	) {
		let lighting_data = frame.get_mut_dynamic_buffer_slice(self.light_data_buffer);
		let light_count = self.lights.len().min(MAX_LIGHTS);

		if self.lights.len() > MAX_LIGHTS {
			warn!(
				"Too many lights for the visibility pipeline. The most likely cause is that the scene contains more lights than the GPU buffer can hold."
			);
		}

		// Rewrite the complete record so recycled frame sequences cannot retain stale counts, lights, or padding.
		*lighting_data = LightingData::default();
		lighting_data.count = light_count as u32;

		for (index, (_, light)) in self.lights.iter().take(light_count).enumerate() {
			let shadow = if directional_shadow_light_index == Some(index) {
				LightShadow::Directional
			} else if let Some(layer) = cone_shadow_light_indices
				.iter()
				.position(|light_index| *light_index == Some(index))
			{
				LightShadow::Cone {
					view_index: (CONE_SHADOW_VIEW_OFFSET + layer) as u32,
					layer: layer as u32,
				}
			} else if let Some(cube_index) = point_shadow_light_indices
				.iter()
				.position(|light_index| *light_index == Some(index))
			{
				LightShadow::Point {
					view_index: (POINT_SHADOW_VIEW_OFFSET + cube_index * POINT_SHADOW_FACE_COUNT) as u32,
					cube_index: cube_index as u32,
				}
			} else {
				LightShadow::None
			};
			lighting_data.lights[index] = Self::make_light_data(light, shadow);
		}

		frame.sync_buffer(self.light_data_buffer);
	}

	fn make_light_data(light: &Lights, shadow: LightShadow) -> LightData {
		let mut shadow_views = [0; 8];

		if shadow == LightShadow::Directional {
			for (index, shadow_view) in shadow_views.iter_mut().take(SHADOW_CASCADE_COUNT).enumerate() {
				*shadow_view = (index + 1) as u32;
			}
		}

		match light {
			Lights::Cone(light) => LightData {
				position: light.position.into_maths().into(),
				color: light.color.into(),
				direction: light.direction.into_maths().into(),
				cone_cosines: [light.inner_angle.cos(), light.outer_angle.cos()],
				light_type: 1,
				shadow_views: match shadow {
					LightShadow::Cone { view_index, .. } => [view_index, 0, 0, 0, 0, 0, 0, 0],
					LightShadow::None | LightShadow::Directional | LightShadow::Point { .. } => [0; 8],
				},
				shadow_layer: match shadow {
					LightShadow::Cone { layer, .. } => layer,
					LightShadow::None | LightShadow::Directional | LightShadow::Point { .. } => 0,
				},
			},
			Lights::Direction(light) => LightData {
				position: light.direction.into_maths().into(),
				color: light.color.into(),
				direction: ShaderVec3::default(),
				cone_cosines: [0.0; 2],
				light_type: 68,
				shadow_views,
				shadow_layer: 0,
			},
			Lights::Point(light) => LightData {
				position: light.position.into_maths().into(),
				color: light.color.into(),
				direction: ShaderVec3::default(),
				cone_cosines: [0.0; 2],
				light_type: 0,
				shadow_views: match shadow {
					LightShadow::Point { view_index, .. } => [view_index, 0, 0, 0, 0, 0, 0, 0],
					LightShadow::None | LightShadow::Directional | LightShadow::Cone { .. } => [0; 8],
				},
				shadow_layer: match shadow {
					LightShadow::Point { cube_index, .. } => cube_index,
					LightShadow::None | LightShadow::Directional | LightShadow::Cone { .. } => 0,
				},
			},
		}
	}
}

/// The `LightShadow` enum identifies the shadow view assignment encoded for one GPU light record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LightShadow {
	None,
	Directional,
	Cone { view_index: u32, layer: u32 },
	Point { view_index: u32, cube_index: u32 },
}

/// Converts an affine gameplay matrix into the compact column-major skin-palette representation.
fn affine_matrix4x3_from_matrix4(matrix: &Matrix) -> AffineMatrix4x3Columns {
	[
		[matrix[(0, 0)], matrix[(1, 0)], matrix[(2, 0)]],
		[matrix[(0, 1)], matrix[(1, 1)], matrix[(2, 1)]],
		[matrix[(0, 2)], matrix[(1, 2)], matrix[(2, 2)]],
		[matrix[(0, 3)], matrix[(1, 3)], matrix[(2, 3)]],
	]
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
	let Some(scene_handles) = render_entity_handles.get(&handle) else {
		return;
	};

	for scene_handle in scene_handles {
		if let Some(render_entity) = render_entities.get_mut(*scene_handle) {
			update(render_entity);
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Matrix, Point, UnitVector};
	use maths_rs::{mat::MatNew4 as _, Vec3f};
	use smallvec::SmallVec;
	use utils::{hash::HashMap, StableVec};

	use super::{
		affine_matrix4x3_from_matrix4, assert_affine_matrix, update_renderable_instances, LightShadow, VisibilitySceneManager,
	};
	use crate::core::factory::Factory;
	use crate::rendering::lights::{ConeLight, DirectionalLight, LightColor, Lights, PhotometricIntensity, PointLight};
	use crate::rendering::pipelines::visibility::pipeline_manager::{LightData, LightingData, ShaderVec3};

	#[test]
	fn pose_write_conversion_preserves_matrix_majorness() {
		let matrix = Matrix::new(
			1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 0.0, 0.0, 0.0, 1.0,
		);

		assert_eq!(
			affine_matrix4x3_from_matrix4(&matrix),
			[[1.0, 5.0, 9.0], [2.0, 6.0, 10.0], [3.0, 7.0, 11.0], [4.0, 8.0, 12.0]]
		);
	}

	#[test]
	#[should_panic(expected = "Skinned pose matrix is projective")]
	fn compact_pose_rejects_a_projective_matrix() {
		assert_affine_matrix(&Matrix::new(
			1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		));
	}

	#[test]
	fn transform_updates_write_each_registered_primitive_without_retaining_transform_state() {
		let mut renderable_factory = Factory::new();
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
	fn cone_light_data_preserves_direction_and_soft_cutoffs() {
		let light = ConeLight::new(
			Point::new(1.0, 2.0, 3.0),
			-UnitVector::y_axis(),
			crate::rendering::lights::LightColor::Kelvin(4_500.0),
			crate::rendering::lights::PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			20.0_f32.to_radians(),
			35.0_f32.to_radians(),
		)
		.expect("physical cone light");
		let light_data =
			VisibilitySceneManager::make_light_data(&Lights::Cone(light), LightShadow::Cone { view_index: 6, layer: 1 });

		assert_eq!(light_data.position, ShaderVec3::from(light.position.into_maths()));
		assert_eq!(light_data.color, ShaderVec3::from(light.color));
		assert_eq!(light_data.direction, ShaderVec3::from(light.direction.into_maths()));
		assert_eq!(light_data.cone_cosines, [light.inner_angle.cos(), light.outer_angle.cos()]);
		assert_eq!(light_data.light_type, 1);
		assert_eq!(light_data.shadow_views, [6, 0, 0, 0, 0, 0, 0, 0]);
		assert_eq!(light_data.shadow_layer, 1);
	}

	#[test]
	fn light_data_uploads_directional_lux_and_local_candela_without_unit_tags() {
		let white = LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0));
		let directional = DirectionalLight::new(
			-UnitVector::y_axis(),
			white,
			PhotometricIntensity::Illuminance {
				lux: 80_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light");
		let point = PointLight::new(
			Point::new(1.0, 2.0, 3.0),
			white,
			PhotometricIntensity::Illuminance {
				lux: 25.0,
				measurement_distance_m: 2.0,
			},
		)
		.expect("physical point light");

		let directional_data = VisibilitySceneManager::make_light_data(&Lights::Direction(directional), LightShadow::None);
		let point_data = VisibilitySceneManager::make_light_data(&Lights::Point(point), LightShadow::None);

		assert_eq!(directional_data.color, ShaderVec3::from((80_000.0, 80_000.0, 80_000.0)));
		assert_eq!(point_data.color, ShaderVec3::from((100.0, 100.0, 100.0)));
		assert_eq!(directional_data.light_type, 68);
		assert_eq!(point_data.light_type, 0);
	}

	#[test]
	fn point_light_data_packs_cube_index_and_first_face_view() {
		let point = PointLight::new(
			Point::new(1.0, 2.0, 3.0),
			LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
		)
		.expect("physical point light");
		let light_data = VisibilitySceneManager::make_light_data(
			&Lights::Point(point),
			LightShadow::Point {
				view_index: 23,
				cube_index: 2,
			},
		);

		assert_eq!(light_data.shadow_views, [23, 0, 0, 0, 0, 0, 0, 0]);
		assert_eq!(light_data.shadow_layer, 2);
	}

	#[test]
	fn light_data_layout_matches_the_shader_light_record() {
		assert_eq!(std::mem::align_of::<LightData>(), 16);
		assert_eq!(std::mem::size_of::<LightData>(), 96);
		assert_eq!(std::mem::offset_of!(LightData, position), 0);
		assert_eq!(std::mem::offset_of!(LightData, color), 16);
		assert_eq!(std::mem::offset_of!(LightData, direction), 32);
		assert_eq!(std::mem::offset_of!(LightData, cone_cosines), 48);
		assert_eq!(std::mem::offset_of!(LightData, light_type), 56);
		assert_eq!(std::mem::offset_of!(LightData, shadow_views), 60);
		assert_eq!(std::mem::offset_of!(LightData, shadow_layer), 92);
		assert_eq!(std::mem::offset_of!(LightingData, lights), 16);
	}
}

use ghi::DescriptorSetHandle;
use ghi::DynamicBufferHandle;
use ghi::Frame as _;
use log::warn;
use math::Matrix;
use resource_management::resources::skeleton::AffineMatrix4x3Columns;
use smallvec::SmallVec;
use utils::{hash::HashMap, StableVec, StableVecHandle};

use crate::core::factory::Handle;
use crate::gameplay::transform::Transform;
use crate::rendering::lights::Lights;
use crate::rendering::pipelines::visibility::pipeline_manager::LightData;
use crate::rendering::pipelines::visibility::pipeline_manager::LightingData;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderEntity;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderInfo;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderViewData;
use crate::rendering::pipelines::visibility::pipeline_manager::SinkState;
use crate::rendering::pipelines::visibility::pipeline_manager::{ShaderMesh, ShaderVec3};
use crate::rendering::pipelines::visibility::CONE_SHADOW_VIEW_OFFSET;
use crate::rendering::pipelines::visibility::MAX_CONE_SHADOW_POOL_CAPACITY;
use crate::rendering::pipelines::visibility::MAX_LIGHTS;
use crate::rendering::pipelines::visibility::MAX_POINT_SHADOW_POOL_CAPACITY;
use crate::rendering::pipelines::visibility::POINT_SHADOW_FACE_COUNT;
use crate::rendering::pipelines::visibility::POINT_SHADOW_VIEW_OFFSET;
use crate::rendering::pipelines::visibility::SHADOW_CASCADE_COUNT;
use crate::rendering::pipelines::visibility::SHADOW_VIEW_COUNT;
use crate::rendering::View;
