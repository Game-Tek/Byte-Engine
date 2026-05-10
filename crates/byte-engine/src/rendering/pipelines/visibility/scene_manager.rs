pub struct VisibilitySceneManager {
	/// Render entities registered in the scene.
	pub(crate) render_entities: Vec<RenderEntity>,
	/// Shared views data buffer used by every visibility sink.
	pub(crate) views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	/// Shared base descriptor set used by every visibility pass.
	pub(crate) descriptor_set: ghi::DescriptorSetHandle,
	/// Bindless texture binding on the shared base descriptor set.
	pub(crate) textures_binding: ghi::DescriptorSetBindingHandle,
	/// Per-instance mesh data buffer holding transforms and material indices for this scene.
	pub(crate) meshes_data_buffer:
		ghi::DynamicBufferHandle<[ShaderMesh; crate::rendering::pipelines::visibility::MAX_INSTANCES]>, // Using crate::rendering::pipelines::visibility::MAX_INSTANCES to avoid hardcoding MAX_INSTANCES if not exported
	/// Unused domain-level material evaluation descriptor set kept while material evaluation remains per sink.
	pub(crate) material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	/// Buffer containing lighting data for this scene.
	pub(crate) light_data_buffer: ghi::BufferHandle<LightingData>,
	/// Lights in the scene.
	pub(crate) lights: Vec<Lights>,
	/// Information about the current render.
	pub(crate) render_info: RenderInfo,
	/// Per-sink render state.
	pub(crate) sink_states: Vec<SinkState>,
}

impl VisibilitySceneManager {
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

		for (index, light) in self.lights.iter().take(light_count).enumerate() {
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
			Lights::Direction(light) => LightData {
				position: light.direction.into(),
				color: light.color.into(),
				light_type: 68,
				cascades,
			},
			Lights::Point(light) => LightData {
				position: light.position.into(),
				color: light.color.into(),
				light_type: 0,
				cascades: [0; 8],
			},
		}
	}
}

use ghi::BufferHandle;
use ghi::DescriptorSetBindingHandle;
use ghi::DescriptorSetHandle;
use ghi::DynamicBufferHandle;
use log::warn;
use math::mat::MatInverse as _;
use utils::hash::HashMap;

use crate::rendering::lights::Lights;
use crate::rendering::pipelines::visibility::pipeline_manager::LightData;
use crate::rendering::pipelines::visibility::pipeline_manager::LightingData;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderEntity;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderInfo;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderMesh;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderViewData;
use crate::rendering::pipelines::visibility::pipeline_manager::SinkState;
use crate::rendering::pipelines::visibility::MAX_LIGHTS;
use crate::rendering::pipelines::visibility::SHADOW_CASCADE_COUNT;
use crate::rendering::View;
