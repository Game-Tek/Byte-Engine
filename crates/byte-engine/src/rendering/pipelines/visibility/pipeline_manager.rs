/// The `VisibilityPipelineManager` struct is the visibility buffer implementation of the world render domain.
/// It owns the per-scene rendering state and references shared GPU resources via `VisibilitySharedResources`.
pub struct VisibilityPipelineManager {
	/// Base descriptor set layout template shared across all scenes and sinks.
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	/// Visibility descriptor set layout template.
	visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	/// Material evaluation descriptor set layout template.
	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	/// Materials data buffer shared across all scenes.
	materials_data_buffer_handle: ghi::BufferHandle<[MaterialData; MAX_MATERIALS]>,
	resource_manager: VisibilityPipelineResourceManagerClient,
	requested_meshes: std::collections::HashSet<VisibilityMeshKey>,
	pending_renderables: Vec<PendingRenderableInstance>,
	loaded_meshes: HashMap<VisibilityMeshKey, MeshData>,
	loaded_materials: HashMap<u32, RenderDescription>,
	loaded_textures: HashSet<u32>,
	loaded_pipelines: HashMap<String, ghi::PipelineHandle>,
	pub(crate) scene: crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager,
}

impl VisibilityPipelineManager {
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		resource_manager: VisibilityPipelineResourceManagerClient,
	) -> Self {
		let bindings = [
			VIEWS_DATA_BINDING,
			MESH_DATA_BINDING,
			VERTEX_POSITIONS_BINDING,
			VERTEX_NORMALS_BINDING,
			VERTEX_UV_BINDING,
			VERTEX_INDICES_BINDING,
			PRIMITIVE_INDICES_BINDING,
			MESHLET_DATA_BINDING,
			TEXTURES_BINDING,
		];
		let descriptor_set_layout = context.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

		let bindings = [
			MATERIAL_COUNT_BINDING,
			MATERIAL_OFFSET_BINDING,
			MATERIAL_OFFSET_SCRATCH_BINDING,
			MATERIAL_EVALUATION_DISPATCHES_BINDING,
			MATERIAL_XY_BINDING,
			TRIANGLE_INDEX_BINDING,
			INSTANCE_ID_BINDING,
		];
		let visibility_descriptor_set_layout = context.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);

		let materials_data_buffer_handle = context.build_buffer::<[MaterialData; MAX_MATERIALS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Materials Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let bindings = [
			LIT_BINDING_TEMPLATE,
			ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			UNUSED_SET2_BINDING2_TEMPLATE,
			ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			LIGHTING_DATA_BINDING_TEMPLATE,
			MATERIALS_DATA_BINDING_TEMPLATE,
			AO_MAP_BINDING_TEMPLATE,
			SHADOW_MAP_BINDING_TEMPLATE,
			VISIBILITY_DEPTH_BINDING_TEMPLATE,
		];
		let material_evaluation_descriptor_set_layout =
			context.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);

		let views_data_buffer_handle = context.build_dynamic_buffer::<[ShaderViewData; 8]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Views Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let meshes_data_buffer = context.build_dynamic_buffer::<[ShaderMesh; MAX_INSTANCES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshes Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let descriptor_set = context.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

		let _views_data_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VIEWS_DATA_BINDING, views_data_buffer_handle.into()),
		);
		let _meshes_data_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESH_DATA_BINDING, meshes_data_buffer.into()),
		);
		let (
			vertex_positions_buffer,
			vertex_normals_buffer,
			vertex_uvs_buffer,
			vertex_indices_buffer,
			primitive_indices_buffer,
			meshlets_data_buffer,
		) = {
			(
				resource_manager.gpu_vertex_data_manager.vertex_positions_buffer,
				resource_manager.gpu_vertex_data_manager.vertex_normals_buffer,
				resource_manager.gpu_vertex_data_manager.vertex_uvs_buffer,
				resource_manager.gpu_vertex_data_manager.vertex_indices_buffer,
				resource_manager.gpu_vertex_data_manager.primitive_indices_buffer,
				resource_manager.gpu_vertex_data_manager.meshlets_data_buffer,
			)
		};
		let _vertex_positions_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_POSITIONS_BINDING, vertex_positions_buffer.into()),
		);
		let _vertex_normals_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_NORMALS_BINDING, vertex_normals_buffer.into()),
		);
		let _vertex_uv_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_UV_BINDING, vertex_uvs_buffer.into()),
		);
		let _vertex_indices_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_INDICES_BINDING, vertex_indices_buffer.into()),
		);
		let _primitive_indices_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&PRIMITIVE_INDICES_BINDING, primitive_indices_buffer.into()),
		);
		let _meshlets_data_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESHLET_DATA_BINDING, meshlets_data_buffer.into()),
		);
		let textures_binding = context.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler_array(&TEXTURES_BINDING),
		);

		let light_data_buffer = context.build_buffer::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Light Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let lighting_data = context.get_mut_buffer_slice(light_data_buffer);
		lighting_data.count = 0; // Initially, no lights

		// Material evaluation resources still vary by sink because the output images vary by sink.
		let _sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let _depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let material_evaluation_descriptor_set = context.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&material_evaluation_descriptor_set_layout,
		);

		resource_manager.configure_material_pipeline(MaterialPipelineConfig::new(
			[
				descriptor_set_layout,
				visibility_descriptor_set_layout,
				material_evaluation_descriptor_set_layout,
			],
			vec![ghi::pipelines::PushConstantRange::new(0, 4)],
			context.create_factory(),
		));

		Self {
			descriptor_set_layout,
			visibility_descriptor_set_layout,
			material_evaluation_descriptor_set_layout,
			materials_data_buffer_handle,
			resource_manager,
			requested_meshes: std::collections::HashSet::new(),
			pending_renderables: Vec::new(),
			loaded_meshes: HashMap::new(),
			loaded_materials: HashMap::new(),
			loaded_textures: HashSet::new(),
			loaded_pipelines: HashMap::new(),
			scene: VisibilitySceneManager {
				render_entities: StableVec::new(),
				views_data_buffer_handle,
				descriptor_set,
				textures_binding,
				meshes_data_buffer,
				material_evaluation_descriptor_set,
				light_data_buffer,
				lights: StableVec::new(),
				render_info: RenderInfo {
					instances: Vec::new(),
					active_instances: Vec::new(),
					opaque_materials: Vec::new(),
					transparent_materials: Vec::new(),
				},
				sink_states: Vec::new(),
			},
		}
	}

	pub(crate) fn create_light(&mut self, handle: Handle, light: Lights) {
		self.scene.lights.push((handle, light));
	}

	pub(crate) fn remove_light(&mut self, handle: Handle) {
		let Some((index, _)) = self
			.scene
			.lights
			.indexed_iter()
			.find(|(_, (light_handle, _))| *light_handle == handle)
		else {
			return;
		};

		self.scene.lights.remove(index);
	}

	/// Requests the renderable mesh resources and keeps the scene instance pending until those resources are ready.
	pub(crate) fn request_mesh(&mut self, handle: Handle, renderable: EntityHandle<dyn RenderableMesh>) {
		let source = renderable.get_mesh().clone();
		let mesh_key = VisibilityMeshKey::from_source(&source);
		if self.requested_meshes.insert(mesh_key.clone()) {
			let source_kind = match &source {
				MeshSource::Resource(_) => "resource",
				MeshSource::Generated(_) => "generated",
			};
			log::debug!("Visibility mesh requested: key={}, source={}", mesh_key, source_kind);
			self.resource_manager.request_mesh(mesh_key.clone(), source);
		}
		self.pending_renderables.push(PendingRenderableInstance {
			handle,
			entity: renderable,
			mesh_key: mesh_key.clone(),
		});
		self.resolve_pending_renderables_for_mesh(&mesh_key);
	}

	pub(crate) fn remove_mesh(&mut self, handle: Handle) {
		self.pending_renderables
			.retain(|pending_renderable| pending_renderable.handle != handle);

		let render_entity_indices = self
			.scene
			.render_entities
			.indexed_iter()
			.filter_map(|(index, render_entity)| (render_entity.handle == handle).then_some(index))
			.collect::<Vec<_>>();

		for index in render_entity_indices {
			self.scene.render_entities.remove(index);
		}
	}

	fn adopt_resource_completions(&mut self, frame: &mut ghi::implementation::Frame) {
		let completions = self.resource_manager.drain_completions();
		if !completions.is_empty() {
			log::debug!("Visibility resource completions received: count={}", completions.len());
		}
		for completion in completions {
			match completion {
				VisibilityResourceCompletion::MeshReady { key, mesh } => {
					let meshlet_count = mesh.primitives.iter().map(|primitive| primitive.meshlet_count).sum::<u32>();
					log::debug!(
						"Visibility mesh adopted: key={}, primitives={}, meshlets={}, loaded_meshes_before={}, pending_renderables={}",
						key,
						mesh.primitives.len(),
						meshlet_count,
						self.loaded_meshes.len(),
						self.pending_renderables.len(),
					);
					self.loaded_meshes.insert(key.clone(), mesh);
					self.resolve_pending_renderables_for_mesh(&key);
				}
				VisibilityResourceCompletion::PipelineReady { name, pipeline } => {
					let pipeline = frame.intern_compute_pipeline(pipeline);
					log::debug!("Visibility material pipeline adopted: name={}", name);
					self.loaded_pipelines.insert(name.clone(), pipeline);
					for material in self.loaded_materials.values_mut() {
						if material.name == name {
							material.pipeline = Some(pipeline);
						}
					}
					self.rebuild_material_lists();
				}
				VisibilityResourceCompletion::MaterialReady {
					id,
					index,
					pipeline,
					pending_pipeline,
					alpha,
					textures,
				} => self.adopt_material_completion(frame, id, index, pipeline, pending_pipeline, alpha, textures),
				VisibilityResourceCompletion::ImageReady {
					key,
					index,
					image,
					sampler,
					upload,
				} => {
					let _ = key;
					let image = frame.intern_image(image);
					let sampler = frame.intern_sampler(sampler);
					let image = ghi::BaseImageHandle::from(image);
					self.resource_manager.enqueue_texture_upload(index, image, sampler, upload);
				}
				VisibilityResourceCompletion::TextureUploadReady { index, image, sampler } => {
					log::debug!("Visibility texture upload adopted: index={}", index);
					self.write_texture_descriptors(frame, index, image, sampler);
					self.loaded_textures.insert(index);
					self.rebuild_material_lists();
				}
				VisibilityResourceCompletion::Failed { key } => {
					warn!(
						"Visibility resource failed to load: {}. The most likely cause is that the resource worker could not resolve or upload the asset.",
						key
					);
				}
			}
		}
	}

	/// Writes a loaded texture into every descriptor set that can sample bindless material textures.
	fn write_texture_descriptors(
		&self,
		frame: &mut ghi::implementation::Frame,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
	) {
		frame.write(&[ghi::descriptors::Write::combined_image_sampler_array(
			self.scene.textures_binding,
			image,
			sampler,
			ghi::Layouts::Read,
			index,
		)]);
	}

	/// Adopts material metadata and writes the material texture table into the GPU material buffer.
	fn adopt_material_completion(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		id: String,
		index: u32,
		pipeline: Option<ghi::PipelineHandle>,
		pending_pipeline: Option<PendingMaterialPipeline>,
		alpha: bool,
		textures: Vec<Option<(String, u32)>>,
	) {
		let pipeline = pipeline.or_else(|| self.loaded_pipelines.get(&id).copied());
		let materials_data = frame.get_mut_buffer_slice(self.materials_data_buffer_handle);
		let material_data = &mut materials_data[index as usize];
		material_data.textures.fill(u32::MAX);

		for (texture_index, texture) in textures.iter().enumerate() {
			if texture_index >= MAX_MATERIAL_TEXTURES {
				warn!(
					"Visibility material {} has too many texture slots. The most likely cause is that the material shader expects more textures than the visibility material data supports.",
					id
				);
				break;
			}
			material_data.textures[texture_index] = texture.as_ref().map(|(_, index)| *index).unwrap_or(u32::MAX);
		}
		frame.sync_buffer(self.materials_data_buffer_handle);

		let texture_indices = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(_, index)| *index))
			.collect::<Vec<_>>();
		log::debug!(
			"Visibility material adopted: id={}, index={}, has_pipeline={}, alpha={}, textures={}",
			id,
			index,
			pipeline.is_some(),
			alpha,
			texture_indices.len(),
		);

		self.loaded_materials.insert(
			index,
			RenderDescription {
				index,
				pipeline,
				name: id,
				alpha,
				texture_indices,
			},
		);
		self.rebuild_material_lists();
	}

	/// Rebuilds the opaque and transparent material lists consumed by the material evaluation pass.
	fn rebuild_material_lists(&mut self) {
		self.scene.render_info.opaque_materials.clear();
		self.scene.render_info.transparent_materials.clear();

		let mut missing_pipeline_count = 0usize;
		let mut missing_texture_count = 0usize;

		for material in self.loaded_materials.values() {
			let Some(pipeline) = material.pipeline else {
				missing_pipeline_count += 1;
				continue;
			};
			// Material shaders index bindless textures directly, so a material must not render until every
			// referenced texture descriptor points at an upload-completed image.
			if !material
				.texture_indices
				.iter()
				.all(|texture_index| self.loaded_textures.contains(texture_index))
			{
				missing_texture_count += 1;
				continue;
			}
			let entry = (material.name.clone(), material.index, pipeline);
			if material.alpha {
				self.scene.render_info.transparent_materials.push(entry);
			} else {
				self.scene.render_info.opaque_materials.push(entry);
			}
		}

		log::debug!(
			"Visibility material lists rebuilt: loaded={}, opaque_ready={}, transparent_ready={}, missing_pipeline={}, missing_textures={}",
			self.loaded_materials.len(),
			self.scene.render_info.opaque_materials.len(),
			self.scene.render_info.transparent_materials.len(),
			missing_pipeline_count,
			missing_texture_count,
		);
	}

	/// Rebuilds the active instance list from scene entities whose material pipeline is ready.
	fn rebuild_active_instances(&mut self, frame: &mut ghi::implementation::Frame) {
		self.scene.render_info.active_instances.clear();
		let loaded_materials = &self.loaded_materials;
		let render_entities = &self.scene.render_entities;
		let active_instances = &mut self.scene.render_info.active_instances;
		let mesh_data = frame.get_mut_dynamic_buffer_slice(self.scene.meshes_data_buffer);

		let mut active_index = 0;
		let mut skipped_missing_material = 0usize;
		let mut active_meshlets = 0u32;
		for render_entity in render_entities.iter() {
			let material_ready = loaded_materials
				.get(&render_entity.shader_mesh.material_index)
				.and_then(|material| material.pipeline)
				.is_some();
			if !material_ready {
				skipped_missing_material += 1;
				continue;
			}
			if active_index >= MAX_INSTANCES {
				panic!(
					"Visibility active instance limit exceeded. The most likely cause is that the scene contains more visible mesh primitives than the visibility pipeline supports."
				);
			}

			let mut shader_mesh = render_entity.shader_mesh;
			shader_mesh.model = render_entity.entity.transform().get_matrix().into();
			mesh_data[active_index] = shader_mesh;
			active_meshlets += shader_mesh.meshlet_count;
			active_instances.push(Instance {
				meshlet_count: shader_mesh.meshlet_count,
			});
			active_index += 1;
		}

		log::debug!(
			"Visibility active primitives rebuilt: render_entities={}, active={}, skipped_missing_material={}, active_meshlets={}",
			render_entities.len(),
			active_instances.len(),
			skipped_missing_material,
			active_meshlets,
		);
	}

	/// Resolves renderable instances whose mesh resource is now available.
	fn resolve_pending_renderables_for_mesh(&mut self, key: &VisibilityMeshKey) {
		let Some(mesh) = self.loaded_meshes.get(key).cloned() else {
			return;
		};

		let pending_before = self.pending_renderables.len();
		let render_entities_before = self.scene.render_entities.len();
		let mut resolved_renderables = 0usize;
		let mut added_primitives = 0usize;
		let mut added_meshlets = 0u32;
		let mut remaining = Vec::with_capacity(self.pending_renderables.len());
		let pending = std::mem::take(&mut self.pending_renderables);

		for pending_renderable in pending {
			if &pending_renderable.mesh_key != key {
				remaining.push(pending_renderable);
				continue;
			}

			let model = pending_renderable.entity.transform().get_matrix().into();
			resolved_renderables += 1;
			for primitive in &mesh.primitives {
				added_primitives += 1;
				added_meshlets += primitive.meshlet_count;
				self.scene.render_entities.push(RenderEntity {
					handle: pending_renderable.handle,
					entity: pending_renderable.entity.clone(),
					shader_mesh: ShaderMesh {
						model,
						material_index: primitive.material_index,
						base_vertex_index: mesh.vertex_offset + primitive.vertex_offset,
						base_primitive_index: mesh.primitive_offset + primitive.primitive_offset,
						base_triangle_index: mesh.triangle_offset + primitive.triangle_offset,
						base_meshlet_index: mesh.meshlet_offset + primitive.meshlet_offset,
						meshlet_count: primitive.meshlet_count,
					},
				});
				self.scene.render_info.instances.push(Instance {
					meshlet_count: primitive.meshlet_count,
				});
			}
		}

		self.pending_renderables = remaining;
		if resolved_renderables > 0 {
			log::debug!(
				"Visibility pending mesh resolved: key={}, resolved_renderables={}, added_primitives={}, added_meshlets={}, render_entities_before={}, render_entities_after={}, pending_before={}, pending_after={}",
				key,
				resolved_renderables,
				added_primitives,
				added_meshlets,
				render_entities_before,
				self.scene.render_entities.len(),
				pending_before,
				self.pending_renderables.len(),
			);
		}
	}

	fn make_shader_view_data(view: View) -> ShaderViewData {
		let view_projection = view.view_projection();

		ShaderViewData {
			view: view.view().into(),
			projection: view.projection().into(),
			view_projection: view_projection.into(),
			inverse_view: view.view().inverse().into(),
			inverse_projection: view.projection().inverse().into(),
			inverse_view_projection: view_projection.inverse().into(),
			fov: view.fov(),
			near: view.near(),
			far: view.far(),
		}
	}
}

impl PipelineManager for VisibilityPipelineManager {
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
	) -> Option<Vec<Box<dyn RenderPassFunction + 'a>>> {
		self.adopt_resource_completions(frame);
		self.rebuild_active_instances(frame);

		let shadow_light = self
			.scene
			.lights
			.iter()
			.enumerate()
			.find_map(|(index, (_, light))| match light {
				Lights::Direction(light) => Some((index, light.direction)),
				Lights::Point(_) => None,
			});
		let shadow_light_index = if !sinks.is_empty() {
			shadow_light.map(|(index, _)| index)
		} else {
			None
		};

		if let Some(sink) = sinks.first() {
			let main_view = sink.view();
			let main_view_data = Self::make_shader_view_data(main_view);
			let views_data_buffer = frame.get_mut_dynamic_buffer_slice(self.scene.views_data_buffer_handle);

			for view_data in views_data_buffer.iter_mut() {
				*view_data = main_view_data;
			}

			if let Some((_, light_direction)) = shadow_light {
				for (cascade_index, (cascade_view, cascade_far)) in
					csm::make_csm_views(main_view, light_direction, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION)
						.into_iter()
						.zip(
							csm::make_cascade_split_ranges(main_view, SHADOW_CASCADE_COUNT)
								.into_iter()
								.map(|(_, far)| far),
						)
						.enumerate()
				{
					let mut cascade_view_data = Self::make_shader_view_data(cascade_view);
					cascade_view_data.far = cascade_far;
					views_data_buffer[cascade_index + 1] = cascade_view_data;
				}
			}

			frame.sync_buffer(self.scene.views_data_buffer_handle);
		}

		self.scene.write_light_data(frame, shadow_light_index);

		let sink_x_rp = sinks.iter().filter_map(|sink| {
			self.scene
				.sink_states
				.iter()
				.find(|sink_state| sink_state.id == sink.index())
				.map(|sink_state| (sink, &sink_state.render_pass))
		});

		let commands: Vec<Box<dyn RenderPassFunction + 'a>> = sink_x_rp
			.map(|(v, r)| {
				Box::new(r.prepare(
					frame,
					v,
					&self.scene.render_info.active_instances,
					&self.scene.render_info.opaque_materials,
					&self.scene.render_info.transparent_materials,
					shadow_light_index.is_some(),
				)) as Box<dyn RenderPassFunction + 'a>
			})
			.collect::<Vec<_>>();

		log::debug!(
			"Visibility prepare summary: sinks={}, sink_states={}, commands={}, requested_meshes={}, loaded_meshes={}, pending_renderables={}, render_entities={}, active_primitives={}, opaque_materials={}, transparent_materials={}, shadow_enabled={}",
			sinks.len(),
			self.scene.sink_states.len(),
			commands.len(),
			self.requested_meshes.len(),
			self.loaded_meshes.len(),
			self.pending_renderables.len(),
			self.scene.render_entities.len(),
			self.scene.render_info.active_instances.len(),
			self.scene.render_info.opaque_materials.len(),
			self.scene.render_info.transparent_materials.len(),
			shadow_light_index.is_some(),
		);

		Some(commands)
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		log::debug!("Visibility sink created: sink_id={}", sink_id);
		let lit_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Lit"),
		);
		let depth_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image).name("Depth"),
		);
		let primitive_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("primitive index"),
		);
		let instance_id = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("instance_id"),
		);

		let context = render_pass_builder.context();
		let visibility_passes_descriptor_set =
			context.create_descriptor_set(Some("Visibility Descriptor Set"), &self.visibility_descriptor_set_layout);
		let material_evaluation_descriptor_set = context.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&self.material_evaluation_descriptor_set_layout,
		);

		let material_count_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Count")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_xy = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material XY")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_evaluation_dispatches = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect)
				.name("Material Evaluation Dipatches")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_scratch_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset Scratch")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32)),
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let visibility_depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);

		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::image(&LIT_BINDING_TEMPLATE, ghi::BaseImageHandle::from(lit_target)),
		);
		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&LIGHTING_DATA_BINDING_TEMPLATE, self.scene.light_data_buffer.into()),
		);
		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIALS_DATA_BINDING_TEMPLATE, self.materials_data_buffer_handle.into()),
		);
		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&AO_MAP_BINDING_TEMPLATE, ao_map, sampler, ghi::Layouts::Read),
		);
		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&SHADOW_MAP_BINDING_TEMPLATE,
				shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = context.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&VISIBILITY_DEPTH_BINDING_TEMPLATE,
				ghi::BaseImageHandle::from(depth_target),
				visibility_depth_sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_COUNT_BINDING, material_count_buffer.into()),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_BINDING, material_offset_buffer.into()),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_scratch_buffer.into()),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_EVALUATION_DISPATCHES_BINDING, material_evaluation_dispatches.into()),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_XY_BINDING, material_xy.into()),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&TRIANGLE_INDEX_BINDING, ghi::BaseImageHandle::from(primitive_index)),
		);
		let _ = context.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&INSTANCE_ID_BINDING, ghi::BaseImageHandle::from(instance_id)),
		);

		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Lit", "main");

		let shader_storage = render_pass_builder.shader_storage();
		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.context(),
			shader_storage,
			self.descriptor_set_layout,
			self.visibility_descriptor_set_layout,
			self.scene.descriptor_set,
			visibility_passes_descriptor_set,
			material_evaluation_descriptor_set,
			material_count_buffer,
			ghi::BaseImageHandle::from(lit_target),
			ao_map.into(),
			shadow_map.into(),
			ghi::BaseImageHandle::from(depth_target),
			ghi::BaseImageHandle::from(primitive_index),
			ghi::BaseImageHandle::from(instance_id),
			material_xy,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);

		self.scene.sink_states.push(SinkState {
			id: sink_id,
			render_pass,
		});
	}
}

#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct ShaderMesh {
	model: ShaderMatrix4x3,
	material_index: u32,
	/// The position into the vertex components data (positions, normals, uvs, ..) buffer this instance's data starts
	/// Also, the position into the vertex indices buffer this instance's data starts
	base_vertex_index: u32,
	base_primitive_index: u32,
	base_triangle_index: u32,
	base_meshlet_index: u32,
	meshlet_count: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightingData {
	pub count: u32,
	pub lights: [LightData; MAX_LIGHTS],
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Default)]
pub struct ShaderVec3 {
	x: f32,
	y: f32,
	z: f32,
	_padding: f32,
}

impl ShaderVec3 {
	fn new(x: f32, y: f32, z: f32) -> Self {
		Self { x, y, z, _padding: 0.0 }
	}
}

impl From<(f32, f32, f32)> for ShaderVec3 {
	fn from(value: (f32, f32, f32)) -> Self {
		Self::new(value.0, value.1, value.2)
	}
}

impl From<Vector3> for ShaderVec3 {
	fn from(value: Vector3) -> Self {
		Self::new(value.x, value.y, value.z)
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct ShaderViewData {
	pub(crate) view: ShaderMatrix4,
	pub(crate) projection: ShaderMatrix4,
	pub(crate) view_projection: ShaderMatrix4,
	pub(crate) inverse_view: ShaderMatrix4,
	pub(crate) inverse_projection: ShaderMatrix4,
	pub(crate) inverse_view_projection: ShaderMatrix4,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightData {
	pub position: ShaderVec3,
	pub color: ShaderVec3,
	pub light_type: u8,
	pub cascades: [u32; 8],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct MaterialData {
	textures: [u32; MAX_MATERIAL_TEXTURES],
}

#[derive(Clone)]
struct PendingMeshPrimitive {
	material_id: String,
	meshlet_count: u32,
	meshlet_offset: u32,
	vertex_offset: u32,
	primitive_offset: u32,
	triangle_offset: u32,
}

#[derive(Clone)]
struct PendingMeshData {
	vertex_offset: u32,
	primitive_offset: u32,
	triangle_offset: u32,
	meshlet_offset: u32,
	acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
	primitives: Vec<PendingMeshPrimitive>,
}

/// The `RenderEntity` struct preserves the mesh readiness dependency for a renderable instance.
pub struct RenderEntity {
	handle: Handle,
	entity: EntityHandle<dyn RenderableMesh>,
	shader_mesh: ShaderMesh,
}

/// The `PendingRenderableInstance` struct associates a scene renderable with the mesh resource it is waiting for.
struct PendingRenderableInstance {
	handle: Handle,
	entity: EntityHandle<dyn RenderableMesh>,
	mesh_key: VisibilityMeshKey,
}

struct RenderDescription {
	index: u32,
	pipeline: Option<ghi::PipelineHandle>,
	name: String,
	alpha: bool,
	texture_indices: Vec<u32>,
}

#[derive(Clone, Copy)]
pub struct Instance {
	pub meshlet_count: u32,
}

pub struct RenderInfo {
	instances: Vec<Instance>,
	active_instances: Vec<Instance>,
	opaque_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	transparent_materials: Vec<(String, u32, ghi::PipelineHandle)>,
}

pub struct SinkState {
	id: usize,
	render_pass: VisibilityPipelineRenderPass,
}

/// This structure hosts data analogous to the mesh resource's data.
/// It stores data relevant to the renderer which allows not to have to access/request the mesh resource.
#[derive(Debug, Clone)]
pub struct MeshData {
	// (material_id)
	pub(crate) primitives: Vec<MeshPrimitive>,
	/// The base position into the vertex buffer
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	pub(crate) triangle_offset: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the mesh
	pub(crate) meshlet_offset: u32,
	pub(crate) acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The index of the material used by this primitive.
	pub(crate) material_index: u32,
	/// The meshlet count.
	pub(crate) meshlet_count: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the primitive in the mesh
	pub(crate) meshlet_offset: u32,
	/// The vertex offset.
	/// The base position into the vertex buffer
	pub(crate) vertex_offset: u32,
	/// The primitive indices offset.
	/// The base position into the primitive indices buffer
	pub(crate) primitive_offset: u32,
	/// The triangle offset.
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	pub(crate) triangle_offset: u32,
}

#[cfg(test)]
mod tests {
	use super::ShaderMesh;

	#[test]
	fn shader_mesh_matches_metal_buffer_layout() {
		#[cfg(target_os = "macos")]
		let (expected_size, expected_material_offset) = (96, 64);
		#[cfg(not(target_os = "macos"))]
		let (expected_size, expected_material_offset) = (80, 48);

		assert_eq!(
			std::mem::size_of::<ShaderMesh>(),
			expected_size,
			"Unexpected Visibility shader mesh size. The most likely cause is that the CPU-side mesh buffer layout drifted from the Metal shader struct alignment."
		);
		assert_eq!(
			std::mem::align_of::<ShaderMesh>(),
			16,
			"Unexpected Visibility shader mesh alignment. The most likely cause is that the CPU-side mesh buffer no longer matches Metal's 16-byte struct alignment."
		);
		assert_eq!(
			std::mem::offset_of!(ShaderMesh, material_index),
			expected_material_offset,
			"Unexpected Visibility shader mesh material offset. The most likely cause is that the CPU-side mesh fields no longer match the shader struct."
		);
	}
}

const LIT_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const UNUSED_SET2_BINDING2_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const LIGHTING_DATA_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const MATERIALS_DATA_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(5, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const AO_MAP_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	10,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const SHADOW_MAP_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(
	11,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
	1,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const VISIBILITY_DEPTH_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	12,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashSet};
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};

use ::core::slice::SlicePattern;
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	graphics_hardware_interface,
};
use log::{error, warn};
use math::{mat::MatInverse as _, ShaderMatrix4, ShaderMatrix4x3, Vector3};
use resource_management::asset::bema_asset_handler::ProgramGenerator;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::shader::besl::backends::glsl::GLSLShaderGenerator;
use resource_management::shader::besl::backends::msl::MSLShaderGenerator;
use resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator;
use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::types::{IndexStreamTypes, IntegralTypes, ShaderTypes};
use resource_management::Reference;
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::sync::{Rc, RwLock};
use utils::{Box, Extent, StableVec, RGBA};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::core::{factory::Handle, Entity, EntityHandle};
use crate::ghi;
use crate::rendering::common_shader_generator::{CommonShaderGenerator, CommonShaderScope};
use crate::rendering::lights::{DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::GPUVertexDataManager;
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
use crate::rendering::pipelines::visibility::resource_manager::{
	MaterialPipelineConfig, PendingMaterialPipeline, VisibilityMeshKey, VisibilityPipelineResourceManagerClient,
	VisibilityResourceCompletion,
};
use crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager;
use crate::rendering::pipelines::visibility::{
	ShaderMeshletData, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING,
	MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_BINDLESS_TEXTURES, MAX_INSTANCES,
	MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
	MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION,
	TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassFunction, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::view::View;
use crate::rendering::{
	csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh, world_render_domain,
	RenderableMesh, Sink,
};
use crate::resource_management::{self};
use crate::space::Transformable as _;
