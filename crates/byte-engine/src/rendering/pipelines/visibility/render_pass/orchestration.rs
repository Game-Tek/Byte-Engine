use super::*;

/// The `VisibilityPipelineRenderPass` struct sequences visibility-buffer work for one sink and scene frame.
pub(crate) struct VisibilityPipelineRenderPass {
	pipeline_manager: crate::rendering::PipelineManagerClient,
	shadow_pass: ShadowPass,
	visibility_pass: VisibilityPass,
	material_count_pass: MaterialCountPass,
	material_offset_pass: MaterialOffsetPass,
	pixel_mapping_pass: PixelMappingPass,
	gtao_pass: GtaoPass,
	material_evaluation_pass: MaterialEvaluationPass,
}

impl VisibilityPipelineRenderPass {
	pub(crate) fn set_gtao_settings(&mut self, settings: GtaoSettings) {
		self.gtao_pass.set_settings(settings);
	}

	/// Returns the descriptor set that carries material-evaluation-only resources.
	pub(crate) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation_pass.descriptor_set
	}

	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		lit: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		directional_shadow_depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
		point_shadow_map: ghi::BaseImageHandle,
		depth: ghi::BaseImageHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
		gtao_settings: GtaoSettings,
	) -> Self {
		let shadow_pass = ShadowPass::new(
			context,
			&pipeline_manager,
			base_descriptor_set,
			directional_shadow_map,
			directional_shadow_depth_pyramid,
			cone_shadow_map,
			point_shadow_map,
		);
		let visibility_pass = VisibilityPass::new(
			context,
			&pipeline_manager,
			base_descriptor_set,
			primitive_index,
			instance_id,
			depth,
		);
		let material_count_pass = MaterialCountPass::new(
			context,
			&pipeline_manager,
			base_descriptor_set,
			visibility_descriptor_set,
			material_count_buffer,
		);
		let material_offset_pass = MaterialOffsetPass::new(
			context,
			&pipeline_manager,
			base_descriptor_set,
			visibility_descriptor_set,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);
		let pixel_mapping_pass =
			PixelMappingPass::new(context, &pipeline_manager, base_descriptor_set, visibility_descriptor_set);
		let gtao_pass = GtaoPass::new(context, &pipeline_manager, depth, ao_map, gtao_settings);

		let material_evaluation_dispatches = material_offset_pass.material_evaluation_dispatches;

		let material_evaluation_pass = MaterialEvaluationPass::new(
			lit,
			ao_map,
			directional_shadow_map,
			cone_shadow_map,
			point_shadow_map,
			base_descriptor_set,
			visibility_descriptor_set,
			material_evaluation_descriptor_set,
			material_evaluation_dispatches,
		);

		Self {
			pipeline_manager,
			shadow_pass,
			visibility_pass,
			material_count_pass,
			material_offset_pass,
			pixel_mapping_pass,
			gtao_pass,
			material_evaluation_pass,
		}
	}

	/// Prepares one opaque visibility layer and one nearest-surface transparent layer.
	pub(crate) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		skinning_pass: Option<&'a SkinningPass>,
		opaque_mesh_dispatch: MeshDispatch,
		masked_mesh_dispatch: MeshDispatch,
		transparent_mesh_dispatch: MeshDispatch,
		skinning_dispatches: &'a [SkinningDispatch],
		opaque_instances: &'a [Instance],
		masked_instances: &'a [Instance],
		transparent_instances: &'a [Instance],
		opaque_materials: &'a [(String, u32, ghi::PipelineHandle)],
		transparent_materials: &'a [(String, u32, ghi::PipelineHandle)],
		opaque_material_mask: &'a ActiveMaterialMask,
		transparent_material_mask: &'a ActiveMaterialMask,
		directional_shadow_enabled: bool,
		cone_shadow_count: usize,
		point_shadow_count: usize,
	) -> Option<impl RenderPassFunction + 'a> {
		let skinning_pipeline = match skinning_pass {
			Some(pass) => Some(self.pipeline_manager.pipeline(pass.pipeline())?),
			None => None,
		};
		let visibility_pipeline = self.pipeline_manager.pipeline(self.visibility_pass.pipeline)?;
		let masked_visibility_pipeline = self.pipeline_manager.pipeline(self.visibility_pass.masked_pipeline)?;
		let directional_shadow_pipeline = self
			.pipeline_manager
			.pipeline(self.shadow_pass.directional_shadow_pass_pipeline)?;
		let directional_shadow_depth_pyramid_pipeline = self
			.pipeline_manager
			.pipeline(self.shadow_pass.directional_shadow_depth_pyramid_pipeline)?;
		let cone_shadow_pipeline = self.pipeline_manager.pipeline(self.shadow_pass.cone_shadow_pass_pipeline)?;
		let masked_directional_shadow_pipeline = self
			.pipeline_manager
			.pipeline(self.shadow_pass.masked_directional_shadow_pass_pipeline)?;
		let masked_cone_shadow_pipeline = self
			.pipeline_manager
			.pipeline(self.shadow_pass.masked_cone_shadow_pass_pipeline)?;
		let material_count_pipeline = self.pipeline_manager.pipeline(self.material_count_pass.pipeline)?;
		let material_offset_pipeline = self
			.pipeline_manager
			.pipeline(self.material_offset_pass.material_offset_pipeline)?;
		let pixel_mapping_pipeline = self
			.pipeline_manager
			.pipeline(self.pixel_mapping_pass.pixel_mapping_pipeline)?;
		let gtao_pipeline = self.pipeline_manager.pipeline(self.gtao_pass.gtao_pipeline)?;
		let depth_pyramid_pipeline = self.pipeline_manager.pipeline(self.gtao_pass.depth_pyramid_pipeline)?;
		let blur_pipeline_x = self.pipeline_manager.pipeline(self.gtao_pass.blur_pipeline_x)?;
		let upscale_pipeline = self.pipeline_manager.pipeline(self.gtao_pass.upscale_pipeline)?;
		// Blend materials have no alpha-aware shadow shader, so only opaque-phase primitives populate the depth map.
		let shadow_pass = self.shadow_pass.prepare(
			frame,
			opaque_instances,
			opaque_mesh_dispatch,
			masked_instances,
			masked_mesh_dispatch,
			directional_shadow_enabled,
			cone_shadow_count,
			point_shadow_count,
			directional_shadow_pipeline,
			masked_directional_shadow_pipeline,
			directional_shadow_depth_pyramid_pipeline,
			cone_shadow_pipeline,
			masked_cone_shadow_pipeline,
		);
		let visibility_pass = &self.visibility_pass;
		// The offset pass consumes and resets every counter before the optional transparent layer runs.
		let opaque_material_count_pass = self.material_count_pass.prepare(sink, material_count_pipeline);
		let transparent_material_count_pass = self.material_count_pass.prepare(sink, material_count_pipeline);
		let material_offset_pass = self.material_offset_pass.prepare(material_offset_pipeline);
		let pixel_mapping_pass = self.pixel_mapping_pass.prepare(sink, pixel_mapping_pipeline);
		let gtao_pass = self.gtao_pass.prepare(
			frame,
			sink,
			gtao_pipeline,
			depth_pyramid_pipeline,
			blur_pipeline_x,
			upscale_pipeline,
		);
		let opaque_material_evaluation_pass =
			self.material_evaluation_pass
				.prepare(frame, sink, opaque_materials, opaque_material_mask, VisibilityPhase::Opaque);
		let transparent_material_evaluation_pass = self.material_evaluation_pass.prepare(
			frame,
			sink,
			transparent_materials,
			transparent_material_mask,
			VisibilityPhase::Transparent,
		);
		let extent = sink.extent();
		let instance_count = opaque_instances.len() + masked_instances.len() + transparent_instances.len();
		let meshlet_count = opaque_instances
			.iter()
			.chain(transparent_instances)
			.map(|instance| instance.meshlet_count)
			.sum::<u32>();
		let opaque_count = opaque_materials
			.iter()
			.filter(|(_, index, _)| material_is_active(opaque_material_mask, *index))
			.count();
		let transparent_count = transparent_materials
			.iter()
			.filter(|(_, index, _)| material_is_active(transparent_material_mask, *index))
			.count();
		Some(
			move |c: &mut ghi::implementation::CommandBufferRecording, t: &[ghi::AttachmentInformation]| {
				log::debug!(
				"Visibility render model executing: primitives={}, opaque_primitives={}, transparent_primitives={}, meshlets={}, opaque_materials={}, transparent_materials={}, shadow_enabled={}",
				instance_count,
				opaque_instances.len(),
				transparent_instances.len(),
				meshlet_count,
				opaque_count,
				transparent_count,
				directional_shadow_enabled || cone_shadow_count > 0 || point_shadow_count > 0,
			);
				c.start_region(|label| label.write_str("Visibility Render Model"));

				if let (Some(skinning_pass), Some(skinning_pipeline)) = (skinning_pass, skinning_pipeline) {
					skinning_pass.record(c, skinning_dispatches, skinning_pipeline);
				}
				shadow_pass(c, t);

				// The opaque layer establishes the depth and color retained by every later transparent primitive.
				visibility_pass.record(
					c,
					extent,
					opaque_instances,
					opaque_mesh_dispatch,
					masked_instances,
					masked_mesh_dispatch,
					VisibilityPhase::Opaque,
					visibility_pipeline,
					masked_visibility_pipeline,
				);
				opaque_material_count_pass(c, t);
				material_offset_pass(c, t);
				pixel_mapping_pass(c, t);
				gtao_pass(c, t);
				opaque_material_evaluation_pass(c, t);

				// The visibility buffer represents one transparent layer. Resolve every blend primitive
				// together so normal depth testing selects the nearest surface before source-over evaluation.
				if let Some(transparent_layer) = transparent_visibility_layer(transparent_instances) {
					visibility_pass.record(
						c,
						extent,
						transparent_layer,
						transparent_mesh_dispatch,
						&[],
						MeshDispatch::default(),
						VisibilityPhase::Transparent,
						visibility_pipeline,
						visibility_pipeline,
					);
					transparent_material_count_pass(c, t);
					material_offset_pass(c, t);
					pixel_mapping_pass(c, t);
					transparent_material_evaluation_pass(c, t);
				}

				c.end_region();
			},
		)
	}
}
