pub struct VisibilitySceneManager {
	/// Render entities registered in the scene.
	pub(crate) render_entities: StableVec<RenderEntity>,
	/// Retained global poses keyed by the renderable handle used by this scene.
	pub(crate) skinning_poses: HashMap<Handle, Vec<Matrix4Columns>>,
	/// Shared views data buffer used by every visibility sink.
	pub(crate) views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	/// Shared base descriptor set used by every visibility pass.
	pub(crate) descriptor_set: ghi::DescriptorSetHandle,
	/// Per-instance mesh data buffer holding transforms and material indices for this scene.
	pub(crate) meshes_data_buffer:
		ghi::DynamicBufferHandle<[ShaderMesh; crate::rendering::pipelines::visibility::MAX_INSTANCES]>, // Using crate::rendering::pipelines::visibility::MAX_INSTANCES to avoid hardcoding MAX_INSTANCES if not exported
	/// Buffer containing lighting data for this scene.
	pub(crate) light_data_buffer: ghi::BufferHandle<LightingData>,
	/// Lights in the scene.
	pub(crate) lights: StableVec<(Handle, Lights)>,
	/// Information about the current render.
	pub(crate) render_info: RenderInfo,
	/// Per-sink render state.
	pub(crate) sink_states: Vec<SinkState>,
}

impl VisibilitySceneManager {
	/// Retains one global transform per skeleton node for the renderable identified by `handle`.
	///
	/// Rewriting an existing pose reuses its allocation when the skeleton size is unchanged. A
	/// pose remains active until it is replaced or the corresponding renderable is removed.
	pub fn write_skinned_pose(&mut self, handle: Handle, global_matrices: &[Matrix4]) {
		let pose = self.skinning_poses.entry(handle).or_default();
		pose.clear();
		pose.extend(global_matrices.iter().map(matrix4_to_columns));
	}

	/// Removes all scene state owned by the renderable identified by `handle`.
	pub(crate) fn remove_renderable(&mut self, handle: Handle) {
		self.skinning_poses.remove(&handle);

		let render_entity_handles = self
			.render_entities
			.handled_iter()
			.filter_map(|(render_entity_handle, render_entity)| {
				(render_entity.handle == handle).then_some(render_entity_handle)
			})
			.collect::<Vec<_>>();

		for render_entity_handle in render_entity_handles {
			self.render_entities.remove(render_entity_handle);
		}
	}

	/// Uploads the current scene lights to the GPU buffer used by material evaluation.
	pub(crate) fn write_light_data(&self, frame: &mut ghi::implementation::Frame, shadow_light_index: Option<usize>) {
		let lighting_data = frame.get_mut_buffer_slice(self.light_data_buffer);
		let light_count = self.lights.len().min(MAX_LIGHTS);

		if self.lights.len() > MAX_LIGHTS {
			warn!(
				"Too many lights for the visibility pipeline. The most likely cause is that the scene contains more lights than the GPU buffer can hold."
			);
		}

		lighting_data.count = light_count as u32;

		for (index, (_, light)) in self.lights.iter().take(light_count).enumerate() {
			lighting_data.lights[index] = Self::make_light_data(light, shadow_light_index == Some(index));
		}

		frame.sync_buffer(self.light_data_buffer);
	}

	fn make_light_data(light: &Lights, casts_shadow: bool) -> LightData {
		let mut cascades = [0; 8];

		if casts_shadow {
			for (index, cascade) in cascades.iter_mut().take(SHADOW_CASCADE_COUNT).enumerate() {
				*cascade = (index + 1) as u32;
			}
		}

		match light {
			Lights::Cone(light) => LightData {
				position: light.position.into(),
				color: light.color.into(),
				direction: light.direction.into(),
				cone_cosines: [light.inner_angle.cos(), light.outer_angle.cos()],
				light_type: 1,
				cascades: [0; 8],
			},
			Lights::Direction(light) => LightData {
				position: light.direction.into(),
				color: light.color.into(),
				direction: ShaderVec3::default(),
				cone_cosines: [0.0; 2],
				light_type: 68,
				cascades,
			},
			Lights::Point(light) => LightData {
				position: light.position.into(),
				color: light.color.into(),
				direction: ShaderVec3::default(),
				cone_cosines: [0.0; 2],
				light_type: 0,
				cascades: [0; 8],
			},
		}
	}
}

/// Converts a gameplay matrix into the column-major representation consumed by skin palette evaluation.
// TODO: isn't this already covered by another function?
fn matrix4_to_columns(matrix: &Matrix4) -> Matrix4Columns {
	[
		[matrix[(0, 0)], matrix[(1, 0)], matrix[(2, 0)], matrix[(3, 0)]],
		[matrix[(0, 1)], matrix[(1, 1)], matrix[(2, 1)], matrix[(3, 1)]],
		[matrix[(0, 2)], matrix[(1, 2)], matrix[(2, 2)], matrix[(3, 2)]],
		[matrix[(0, 3)], matrix[(1, 3)], matrix[(2, 3)], matrix[(3, 3)]],
	]
}

#[cfg(test)]
mod tests {
	use math::{mat::MatNew4 as _, Matrix4, Vector3};

	use super::{matrix4_to_columns, VisibilitySceneManager};
	use crate::rendering::lights::{ConeLight, Lights};
	use crate::rendering::pipelines::visibility::pipeline_manager::{LightData, LightingData, ShaderVec3};

	#[test]
	fn pose_write_conversion_preserves_matrix_majorness() {
		let matrix = Matrix4::new(
			1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
		);

		assert_eq!(
			matrix4_to_columns(&matrix),
			[
				[1.0, 5.0, 9.0, 13.0],
				[2.0, 6.0, 10.0, 14.0],
				[3.0, 7.0, 11.0, 15.0],
				[4.0, 8.0, 12.0, 16.0],
			]
		);
	}

	#[test]
	fn cone_light_data_preserves_direction_and_soft_cutoffs() {
		let light = ConeLight::new(
			Vector3::new(1.0, 2.0, 3.0),
			Vector3::new(0.0, -1.0, 0.0),
			4_500.0,
			20.0_f32.to_radians(),
			35.0_f32.to_radians(),
		);
		let light_data = VisibilitySceneManager::make_light_data(&Lights::Cone(light), false);

		assert_eq!(light_data.position, ShaderVec3::from(light.position));
		assert_eq!(light_data.color, ShaderVec3::from(light.color));
		assert_eq!(light_data.direction, ShaderVec3::from(light.direction));
		assert_eq!(light_data.cone_cosines, [light.inner_angle.cos(), light.outer_angle.cos()]);
		assert_eq!(light_data.light_type, 1);
		assert_eq!(light_data.cascades, [0; 8]);
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
		assert_eq!(std::mem::offset_of!(LightData, cascades), 60);
		assert_eq!(std::mem::offset_of!(LightingData, lights), 16);
	}
}

use ghi::BufferHandle;
use ghi::DescriptorSetHandle;
use ghi::DynamicBufferHandle;
use ghi::Frame as _;
use log::warn;
use math::{mat::MatInverse as _, Matrix4};
use resource_management::resources::skeleton::Matrix4Columns;
use utils::{hash::HashMap, StableVec};

use crate::core::factory::Handle;
use crate::rendering::lights::Lights;
use crate::rendering::pipelines::visibility::pipeline_manager::LightData;
use crate::rendering::pipelines::visibility::pipeline_manager::LightingData;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderEntity;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderInfo;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderViewData;
use crate::rendering::pipelines::visibility::pipeline_manager::SinkState;
use crate::rendering::pipelines::visibility::pipeline_manager::{ShaderMesh, ShaderVec3};
use crate::rendering::pipelines::visibility::MAX_LIGHTS;
use crate::rendering::pipelines::visibility::SHADOW_CASCADE_COUNT;
use crate::rendering::View;
