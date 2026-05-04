use ghi::BufferHandle;
use ghi::DescriptorSetBindingHandle;
use ghi::DescriptorSetHandle;
use ghi::DynamicBufferHandle;
use utils::hash::HashMap;

use crate::rendering::lights::Lights;
use crate::rendering::pipelines::visibility::pipeline_manager::LightingData;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderEntity;
use crate::rendering::pipelines::visibility::pipeline_manager::RenderInfo;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderMesh;
use crate::rendering::pipelines::visibility::pipeline_manager::ShaderViewData;
use crate::rendering::pipelines::visibility::pipeline_manager::SinkState;

pub struct VisibilitySceneManager {
	/// Render entities registered in the scene.
	pub(crate) render_entities: Vec<RenderEntity>,
	/// Legacy domain-level views data buffer (superseded by per-sink buffers in sink_states).
	pub(crate) views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	/// Legacy domain-level descriptor set (superseded by per-sink descriptor sets in sink_states).
	pub(crate) descriptor_set: ghi::DescriptorSetHandle,
	/// Bindless texture binding on the legacy domain-level descriptor set.
	pub(crate) textures_binding: ghi::DescriptorSetBindingHandle,
	/// Per-instance mesh data buffer holding transforms and material indices for this scene.
	pub(crate) meshes_data_buffer:
		ghi::DynamicBufferHandle<[ShaderMesh; crate::rendering::pipelines::visibility::MAX_INSTANCES]>, // Using crate::rendering::pipelines::visibility::MAX_INSTANCES to avoid hardcoding MAX_INSTANCES if not exported
	/// Legacy domain-level material evaluation descriptor set (superseded by per-sink sets).
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
