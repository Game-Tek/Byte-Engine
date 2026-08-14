use super::*;

pub struct MaterialCountPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pub(super) pipeline: crate::rendering::PipelineRef,
}

impl MaterialCountPass {
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	) -> Self {
		let material_count_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/material-count.pipeline");

		MaterialCountPass {
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	pub(super) fn prepare(&self, sink: &Sink, pipeline: ghi::PipelineHandle) -> impl RenderPassFunction + use<'_> {
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let material_count_buffer = self.material_count_buffer;

		let extent = sink.extent();

		move |c, _| {
			log::debug!(
				"Visibility material count pass executing: extent={}x{}",
				extent.width(),
				extent.height()
			);
			c.start_region(|label| label.write_str("Material Count"));

			// The offset pass reads these counts without resetting them, so clear before every dispatch.
			c.clear_buffers(&[material_count_buffer.into()]);

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_pass_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(8)));

			c.end_region();
		}
	}

	fn get_material_count_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_count_buffer.into()
	}
}

pub struct MaterialOffsetPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pub(super) material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	pub(super) material_offset_pipeline: crate::rendering::PipelineRef,
}

impl MaterialOffsetPass {
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		let material_offset_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/material-offset.pipeline");

		MaterialOffsetPass {
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			descriptor_set,
			visibility_pass_descriptor_set,
			material_offset_pipeline,
		}
	}

	pub(super) fn prepare(&self, pipeline: ghi::PipelineHandle) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let visibility_passes_descriptor_set = self.visibility_pass_descriptor_set;

		move |c, _| {
			log::debug!("Visibility material offset pass executing");
			c.start_region(|label| label.write_str("Material Offset"));

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::line(1), Extent::line(1)));
			c.end_region();
		}
	}

	fn get_material_offset_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_buffer.into()
	}

	fn get_material_offset_scratch_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_scratch_buffer.into()
	}
}

pub struct PixelMappingPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) pixel_mapping_pipeline: crate::rendering::PipelineRef,
}

impl PixelMappingPass {
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let pixel_mapping_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/pixel-mapping.pipeline");

		PixelMappingPass {
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	pub(super) fn prepare(&self, sink: &Sink, pipeline: ghi::PipelineHandle) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;

		let extent = sink.extent();

		move |c, _| {
			log::debug!(
				"Visibility pixel mapping pass executing: extent={}x{}",
				extent.width(),
				extent.height()
			);
			c.start_region(|label| label.write_str("Pixel Mapping"));

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(16)));

			c.end_region();
		}
	}
}
/// Returns whether this frame contains geometry that uses one material in the requested visibility phase.
pub(super) fn material_is_active(active_materials: &ActiveMaterialMask, material_index: u32) -> bool {
	let material_index = material_index as usize;
	active_materials
		.get(material_index / u64::BITS as usize)
		.is_some_and(|word| word & (1u64 << (material_index % u64::BITS as usize)) != 0)
}

/// The `MaterialEvaluationPass` struct owns material dispatch state shared by opaque writes and transparent composition.
pub struct MaterialEvaluationPass {
	lit: ghi::BaseImageHandle,
	ao_map: ghi::BaseImageHandle,
	/// Base descriptor set shared by material layouts.
	base_descriptor_set: ghi::DescriptorSetHandle,
	/// Visibility passes descriptor set
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// Material evaluation descriptor set
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	pub(super) fn new(
		lit: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		_directional_shadow_map: ghi::BaseImageHandle,
		_cone_shadow_map: ghi::BaseImageHandle,
		_point_shadow_map: ghi::BaseImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		MaterialEvaluationPass {
			lit,
			ao_map,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			material_evaluation_dispatches,
		}
	}

	/// Prepares one material phase with explicit overwrite or source-over behavior.
	pub(super) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		materials: &'a [(String, u32, ghi::PipelineHandle)],
		active_materials: &'a ActiveMaterialMask,
		phase: VisibilityPhase,
	) -> impl RenderPassFunction + 'a {
		let lit = self.lit;
		let ao_map = self.ao_map;
		let base_descriptor_set = self.base_descriptor_set;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;
		let visibility_descriptor_set = self.visibility_descriptor_set;
		let material_evaluation_descriptor_set = self.descriptor_set;
		let extent = sink.extent();
		let active_material_count = materials
			.iter()
			.filter(|(_, index, _)| material_is_active(active_materials, *index))
			.count();

		if phase == VisibilityPhase::Opaque {
			frame.resize_image(ao_map, extent);
		}

		move |c, t| {
			if phase == VisibilityPhase::Opaque {
				c.clear_images(&[(lit, ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0)))]);
			}
			if active_material_count == 0 {
				return;
			}
			log::debug!(
				"{} visibility material evaluation executing: extent={}x{}, materials={}",
				phase.label(),
				extent.width(),
				extent.height(),
				active_material_count,
			);

			c.start_region(|label| label.write_str("Material Evaluation"));
			c.start_region(|label| label.write_str(phase.label()));

			let mut bound_material_pipeline = None;
			for (name, index, pipeline) in materials {
				if !material_is_active(active_materials, *index) {
					continue;
				}
				c.start_region(|label| label.write_str(name));
				if bound_material_pipeline != Some(*pipeline) {
					let c = c.bind_compute_pipeline(*pipeline);
					c.bind_descriptor_sets(&[
						base_descriptor_set,
						visibility_descriptor_set,
						material_evaluation_descriptor_set,
					]);
					bound_material_pipeline = Some(*pipeline);
				}
				c.write_push_constant(0, [*index, phase.blend_flag()]);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();
			c.end_region();
		}
	}
}
