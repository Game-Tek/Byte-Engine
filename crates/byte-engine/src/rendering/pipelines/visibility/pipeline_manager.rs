/// The `SkinningPaletteCacheEntry` struct shares one uploaded binding palette across a renderable's primitives.
#[derive(Clone, Copy)]
struct SkinningPaletteCacheEntry {
	handle: Handle,
	binding: *const SkinBinding,
	palette_base: u32,
}

/// The `EnvironmentTexture` struct retains the image and sampler currently used for visibility reflections.
#[derive(Clone, Copy)]
struct EnvironmentTexture {
	diffuse_image: ghi::BaseImageHandle,
	specular_image: ghi::BaseImageHandle,
	sampler: ghi::SamplerHandle,
}

/// The `VisibilityPipelineManager` struct provides the visibility buffer implementation for the world render domain.
///
/// See the [visibility render-model guide](https://byte-engine.0x44491229.dev/docs/develop/design/rendering/render-models/visibility)
/// for its frame stages, resource ownership, and material evaluation path.
pub struct VisibilityPipelineManager {
	/// Canonical CPU material metadata retained across every frame sequence.
	materials_data: std::boxed::Box<[MaterialData; MAX_MATERIALS]>,
	/// Frame-local material metadata buffer shared across all scenes.
	materials_data_buffer_handle: ghi::DynamicBufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Compact single-view mesh work shared by shadow cascades and visibility phases.
	mesh_dispatch_work: crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatchWorkBuffer,
	/// Compute resources shared by every sink for frame-local mesh deformation.
	skinning_pass: SkinningPass,
	/// Application-owned baked resources used by the fixed visibility shader set.
	shader_resources: EntityHandle<ResourceManager>,
	/// Reused palette upload storage prevents per-frame matrix allocations.
	skinning_palette_scratch: Vec<AffineMatrix4x3Columns>,
	/// Reused per-instance palette lookup avoids duplicate uploads when primitive order is noncontiguous.
	skinning_palette_cache: Vec<SkinningPaletteCacheEntry>,
	resource_manager: VisibilityPipelineResourceManagerClient,
	requested_meshes: std::collections::HashSet<VisibilityMeshKey>,
	pending_renderables: Vec<PendingRenderableInstance>,
	loaded_meshes: HashMap<VisibilityMeshKey, MeshData>,
	loaded_materials: HashMap<u32, RenderDescription>,
	loaded_textures: HashSet<u32>,
	loaded_pipelines: HashMap<String, ghi::PipelineHandle>,
	/// Requested environment resource retained until its asynchronous upload completes.
	environment_resource_id: Option<String>,
	/// Texture bound to material evaluation; starts as a transparent analytical-fallback marker.
	environment_texture: EnvironmentTexture,
	gtao_configuration: crate::configuration::ConfigurationPort,
	gtao_settings: crate::rendering::pipelines::visibility::render_pass::GtaoSettings,
	pub(crate) scene: crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager,
}

impl VisibilityPipelineManager {
	/// Retains a renderable's global skeleton pose for palette generation during frame preparation.
	pub fn update_pose(&mut self, handle: Handle, global_matrices: &[math::Matrix4]) {
		self.scene.write_skinned_pose(handle, global_matrices);
	}

	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		resource_manager: VisibilityPipelineResourceManagerClient,
		shader_resources: EntityHandle<ResourceManager>,
		gtao_configuration: crate::configuration::ConfigurationPort,
	) -> Self {
		let environment_texture = create_fallback_environment_texture(context);
		let skinning_pass = SkinningPass::new(
			context,
			&shader_resources,
			SkinningSourceBuffers::new(
				resource_manager.gpu_vertex_data_manager.skinning_rest_positions_buffer.into(),
				resource_manager.gpu_vertex_data_manager.skinning_rest_normals_buffer.into(),
				resource_manager.gpu_vertex_data_manager.skinning_joints_buffer.into(),
				resource_manager.gpu_vertex_data_manager.skinning_weights_buffer.into(),
			),
		);
		let materials_data = vec![MaterialData::default(); MAX_MATERIALS]
			.into_boxed_slice()
			.try_into()
			.unwrap_or_else(|_| {
				unreachable!(
					"Visibility material table has an invalid length. The most likely cause is that its fixed-size initialization no longer matches MAX_MATERIALS."
				)
			});
		let materials_data_buffer_handle = context.build_dynamic_buffer::<[MaterialData; MAX_MATERIALS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Materials Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let views_data_buffer_handle = context.build_dynamic_buffer::<[ShaderViewData; SHADOW_VIEW_COUNT]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Views Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let meshes_data_buffer = context.build_dynamic_buffer::<[ShaderMesh; MAX_INSTANCES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshes Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let descriptor_set = context.create_descriptor_set(Some("Base Descriptor Set"));
		let mesh_dispatch_work =
			crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatchWorkBuffer::new(context, descriptor_set);
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
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, VIEWS_DATA_BINDING.slot(), views_data_buffer_handle.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, MESH_DATA_BINDING.slot(), meshes_data_buffer.into()),
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				VERTEX_POSITIONS_BINDING.slot(),
				vertex_positions_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(descriptor_set, VERTEX_NORMALS_BINDING.slot(), vertex_normals_buffer.into()),
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				SKINNED_VERTICES_BINDING.slot(),
				skinning_pass.skinned_vertices_buffer().into(),
			),
			ghi::DescriptorWrite::buffer(descriptor_set, VERTEX_UV_BINDING.slot(), vertex_uvs_buffer.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, VERTEX_INDICES_BINDING.slot(), vertex_indices_buffer.into()),
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				PRIMITIVE_INDICES_BINDING.slot(),
				primitive_indices_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(descriptor_set, MESHLET_DATA_BINDING.slot(), meshlets_data_buffer.into()),
		]);

		let light_data_buffer = context.build_dynamic_buffer::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Light Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

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
		resource_manager.configure_material_pipeline(MaterialPipelineConfig::new(
			vec![ghi::pipelines::PushConstantRange::new(0, 8)],
			context.create_factory(),
		));
		Self {
			materials_data,
			materials_data_buffer_handle,
			mesh_dispatch_work,
			skinning_pass,
			shader_resources,
			skinning_palette_scratch: Vec::new(),
			skinning_palette_cache: Vec::new(),
			resource_manager,
			requested_meshes: std::collections::HashSet::new(),
			pending_renderables: Vec::new(),
			loaded_meshes: HashMap::new(),
			loaded_materials: HashMap::new(),
			loaded_textures: HashSet::new(),
			loaded_pipelines: HashMap::new(),
			environment_resource_id: None,
			environment_texture,
			gtao_configuration,
			gtao_settings: Default::default(),
			scene: VisibilitySceneManager {
				render_entities: StableVec::new(),
				skinning_poses: HashMap::new(),
				views_data_buffer_handle,
				descriptor_set,
				meshes_data_buffer,
				light_data_buffer,
				lights: StableVec::new(),
				render_info: RenderInfo {
					opaque_instances: Vec::new(),
					transparent_instances: Vec::new(),
					skinning_dispatches: Vec::with_capacity(MAX_INSTANCES),
					opaque_materials: Vec::new(),
					transparent_materials: Vec::new(),
					opaque_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
					transparent_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
				},
				sink_states: Vec::new(),
			},
		}
	}

	pub(crate) fn create_light(&mut self, handle: Handle, light: Lights) {
		self.scene.lights.push((handle, light));
	}

	/// Selects an environment and requests its baked lighting resources.
	pub(crate) fn create_environment(&mut self, environment: Environment) {
		let resource_id = environment.resource_id().to_owned();
		self.environment_resource_id = Some(resource_id.clone());
		self.resource_manager.request_environment(resource_id);
	}

	pub(crate) fn remove_light(&mut self, handle: Handle) {
		let Some((handle, _)) = self
			.scene
			.lights
			.handled_iter()
			.find(|(_, (light_handle, _))| *light_handle == handle)
		else {
			return;
		};

		self.scene.lights.remove(handle);
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
		self.scene.remove_renderable(handle);
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
					alpha_mode,
					textures,
				} => self.adopt_material_completion(id, index, pipeline, pending_pipeline, alpha_mode, textures),
				VisibilityResourceCompletion::ImageReady {
					key: _,
					index,
					image,
					sampler,
					upload,
				} => {
					let image = frame.intern_image(image);
					let sampler = frame.intern_sampler(sampler);
					let image = ghi::BaseImageHandle::from(image);
					self.resource_manager.enqueue_texture_upload(index, image, sampler, upload);
				}
				VisibilityResourceCompletion::EnvironmentReady { id, environment } => {
					if self.environment_resource_id.as_deref() == Some(id.as_str()) {
						let upload = environment.intern(id, frame);
						self.resource_manager.enqueue_environment_upload(upload);
					}
				}
				VisibilityResourceCompletion::TextureUploadReady { index, image, sampler } => {
					log::debug!("Visibility texture upload adopted: index={}", index);
					self.write_texture_descriptors(frame, index, image, sampler);
					self.loaded_textures.insert(index);
					self.rebuild_material_lists();
				}
				VisibilityResourceCompletion::EnvironmentUploadReady {
					id,
					diffuse_image,
					specular_image,
					sampler,
				} => {
					if self.environment_resource_id.as_deref() == Some(id.as_str()) {
						self.environment_texture = EnvironmentTexture {
							diffuse_image,
							specular_image,
							sampler,
						};
						self.write_environment_descriptors(frame);
						log::debug!(
							"Visibility environment IBL adopted: id={}, specular_levels={}",
							id,
							IBL_SPECULAR_LEVEL_COUNT
						);
					}
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
		frame.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
			self.scene.descriptor_set,
			TEXTURES_BINDING.slot(),
			image,
			sampler,
			ghi::Layouts::Read,
			index,
		)]);
	}

	/// Writes the current environment into every sink's material-evaluation descriptor set.
	fn write_environment_descriptors(&self, frame: &mut ghi::implementation::Frame) {
		for sink_state in &self.scene.sink_states {
			let descriptor_set = sink_state.render_pass.material_evaluation_descriptor_set();
			frame.write(&[diffuse_environment_descriptor_write(descriptor_set, self.environment_texture)]);
			frame.write(&[specular_environment_descriptor_write(
				descriptor_set,
				self.environment_texture,
			)]);
		}
	}

	/// Adopts material metadata into the canonical CPU table used by every frame sequence.
	fn adopt_material_completion(
		&mut self,
		id: String,
		index: u32,
		pipeline: Option<ghi::PipelineHandle>,
		pending_pipeline: Option<PendingMaterialPipeline>,
		alpha_mode: AlphaMode,
		textures: Vec<Option<(String, u32)>>,
	) {
		let pipeline = pipeline.or_else(|| self.loaded_pipelines.get(&id).copied());
		let material_data = self.materials_data.get_mut(index as usize).unwrap_or_else(|| {
			panic!(
				"Visibility material index is out of range. The most likely cause is that the resource manager assigned more than MAX_MATERIALS material indices."
			)
		});
		if write_material_texture_indices(
			material_data,
			textures.iter().map(|texture| texture.as_ref().map(|(_, index)| *index)),
		) {
			warn!(
				"Visibility material {} has too many texture slots. The most likely cause is that the material shader expects more textures than the visibility material data supports.",
				id
			);
		}

		let texture_indices = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(_, index)| *index))
			.collect::<Vec<_>>();
		log::debug!(
			"Visibility material adopted: id={}, index={}, has_pipeline={}, alpha_mode={:?}, textures={}",
			id,
			index,
			pipeline.is_some(),
			alpha_mode,
			texture_indices.len(),
		);

		self.loaded_materials.insert(
			index,
			RenderDescription {
				index,
				pipeline,
				name: id,
				alpha_mode,
				texture_indices,
			},
		);
		self.rebuild_material_lists();
	}

	/// Copies canonical material metadata into the current frame-local buffer before rendering.
	fn write_material_data(&self, frame: &mut ghi::implementation::Frame) {
		frame
			.get_mut_dynamic_buffer_slice(self.materials_data_buffer_handle)
			.copy_from_slice(self.materials_data.as_ref());
		frame.sync_buffer(self.materials_data_buffer_handle);
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
			if is_transparent(&material.alpha_mode) {
				self.scene.render_info.transparent_materials.push(entry);
			} else {
				self.scene.render_info.opaque_materials.push(entry);
			}
		}

		// Materials with the same generated shader share one pipeline. Keep them adjacent so recording can retain
		// the native pipeline and argument-buffer binding across consecutive indirect dispatches.
		self.scene
			.render_info
			.opaque_materials
			.sort_unstable_by(|left, right| left.2.cmp(&right.2).then(left.1.cmp(&right.1)));
		self.scene
			.render_info
			.transparent_materials
			.sort_unstable_by(|left, right| left.2.cmp(&right.2).then(left.1.cmp(&right.1)));

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
		self.scene.render_info.clear_active_instances();
		let loaded_materials = &self.loaded_materials;
		let render_entities = &self.scene.render_entities;
		let skinning_poses = &self.scene.skinning_poses;
		let palette_scratch = &mut self.skinning_palette_scratch;
		let palette_cache = &mut self.skinning_palette_cache;
		let mesh_data = frame.get_mut_dynamic_buffer_slice(self.scene.meshes_data_buffer);
		// Frame caches retain capacity but never retain entity or resource pointers beyond this rebuild.
		palette_cache.clear();

		let mut active_index = 0;
		let mut skipped_missing_material = 0usize;
		let mut active_meshlets = 0u32;
		let mut deformed_vertex_count = 0usize;
		let mut pose_matrix_count = 0usize;
		let mut palette_matrix_count = 0usize;
		for render_entity in render_entities.iter() {
			let Some(material) = loaded_materials.get(&render_entity.shader_mesh.material_index) else {
				skipped_missing_material += 1;
				continue;
			};
			if material.pipeline.is_none() {
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
			shader_mesh.skinned_base_vertex_index = u32::MAX;

			if let Some(skinning) = render_entity.skinning.as_ref() {
				let skeleton_node_count = skinning.skeleton_node_count as usize;
				let pose = skinning_poses.get(&render_entity.handle);
				if let Some(pose) = pose {
					assert_eq!(
						pose.len(),
						skeleton_node_count,
						"Visibility skin pose has the wrong matrix count. The most likely cause is that the pose was written for a different skeleton."
					);
					pose_matrix_count += pose.len();
				}

				if let Some(pose) = pose.filter(|_| skinning.vertex_count > 0) {
					let binding_ptr = Arc::as_ptr(&skinning.binding);
					let palette_base = match cached_skin_palette_base(palette_cache, render_entity.handle, binding_ptr) {
						Some(palette_base) => Some(palette_base),
						_ => {
							let palette_end = palette_matrix_count.checked_add(skinning.binding.len()).expect(
								"Visibility skin palette count overflowed. The most likely cause is corrupted skin binding metadata.",
							);
							if palette_end > MAX_SKINNING_MATRICES {
								panic!(
									"Visibility skin palette limit exceeded. The most likely cause is that active animated instances require more joint matrices than the visibility pipeline supports."
								);
							}
							// Grow only to the scene's high-water mark, then reuse this palette storage on later frames.
							palette_scratch.resize(palette_end, identity_affine_matrix4x3_columns());

							let palette_base = palette_matrix_count as u32;
							match skinning
								.binding
								.write_matrix_palette(pose, &mut palette_scratch[palette_matrix_count..palette_end])
							{
								Ok(()) => {
									palette_matrix_count = palette_end;
									palette_cache.push(SkinningPaletteCacheEntry {
										handle: render_entity.handle,
										binding: binding_ptr,
										palette_base,
									});
									Some(palette_base)
								}
								Err(error) => {
									error!("Visibility skin palette could not be written: {error}");
									None
								}
							}
						}
					};

					if let Some(palette_base) = palette_base {
						// Output is dense per active primitive, so shared meshes never overwrite another instance's pose.
						shader_mesh.skinned_base_vertex_index =
							reserve_deformed_vertex_range(&mut deformed_vertex_count, skinning.vertex_count);
						self.scene.render_info.skinning_dispatches.push(SkinningDispatch::new(
							skinning.source_vertex_offset,
							shader_mesh.skinned_base_vertex_index,
							palette_base,
							skinning.vertex_count,
						));
					}
				}
			}
			mesh_data[active_index] = shader_mesh;
			active_meshlets += shader_mesh.meshlet_count;
			let instance = Instance {
				shader_mesh_index: active_index as u32,
				meshlet_count: shader_mesh.meshlet_count,
			};
			self.scene
				.render_info
				.push_active_instance(instance, shader_mesh.material_index, &material.alpha_mode);
			active_index += 1;
		}
		// The active mesh table is frame-local dynamic data; flush the current frame resource after rebuilding it.
		frame.sync_buffer(self.scene.meshes_data_buffer);
		if palette_matrix_count > 0 {
			self.skinning_pass
				.write_matrix_palette(frame, &palette_scratch[..palette_matrix_count]);
		}

		log::debug!(
			"Visibility active primitives rebuilt: render_entities={}, active={}, skipped_missing_material={}, active_meshlets={}, opaque_primitives={}, transparent_primitives={}, skinning_dispatches={}, deformed_vertices={}, pose_matrices={}, palette_matrices={}",
			render_entities.len(),
			self.scene.render_info.active_instance_count(),
			skipped_missing_material,
			active_meshlets,
			self.scene.render_info.opaque_instances.len(),
			self.scene.render_info.transparent_instances.len(),
			self.scene.render_info.skinning_dispatches.len(),
			deformed_vertex_count,
			pose_matrix_count,
			palette_matrix_count,
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
						skinned_base_vertex_index: u32::MAX,
						_padding: 0,
					},
					skinning: primitive.skin.as_ref().map(|binding| RenderSkin {
						binding: binding.clone(),
						source_vertex_offset: primitive.skinning_source_vertex_offset.expect(
							"Skinned primitive has no GPU source range. The most likely cause is that skin streams were not uploaded with the mesh resource.",
						),
						vertex_count: primitive.skinning_vertex_count,
						skeleton_node_count: mesh.skeleton_node_count,
					}),
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
			view_projection: view_projection.into(),
			inverse_view: view.view().inverse().into(),
			fov: view.fov(),
			near: view.near(),
			far: view.far(),
		}
	}
}

/// The `ShadowLightSelection` struct retains the bounded directional and cone shadow work for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ShadowLightSelection {
	directional: Option<(usize, Vector3)>,
	cones: [Option<(usize, ConeLight)>; MAX_CONE_SHADOWS],
	cone_count: usize,
}

/// Selects shadow casters from the same bounded light prefix uploaded to material evaluation.
fn select_shadow_lights<'a>(lights: impl Iterator<Item = &'a Lights>) -> ShadowLightSelection {
	let mut selection = ShadowLightSelection::default();

	for (index, light) in lights.take(MAX_LIGHTS).enumerate() {
		match light {
			Lights::Direction(light) if selection.directional.is_none() => {
				selection.directional = Some((index, light.direction));
			}
			Lights::Cone(light) if light.supports_shadow_mapping() => {
				if let Some(slot) = selection.cones.get_mut(selection.cone_count) {
					*slot = Some((index, *light));
				}
				selection.cone_count += 1;
			}
			Lights::Cone(_) | Lights::Direction(_) | Lights::Point(_) => {}
		}
	}

	selection
}

/// The minimum distance from a cone light covered by an automatic shadow view.
const CONE_SHADOW_NEAR_M: f32 = 0.1;
/// The smallest incident illuminance worth covering with an automatic cone shadow view.
const CONE_SHADOW_CUTOFF_LUX: f32 = 0.1;

/// Resolves the clipping range for one cone-light shadow view.
///
/// The far distance is where the light's peak illuminance reaches
/// [`CONE_SHADOW_CUTOFF_LUX`]. Manual endpoints replace their respective automatic values and
/// are clamped to retain a valid perspective projection.
fn resolve_cone_shadow_range(light: ConeLight) -> (f32, f32) {
	let peak_candela = 0.2126 * light.color.x + 0.7152 * light.color.y + 0.0722 * light.color.z;
	let automatic_far = (peak_candela / CONE_SHADOW_CUTOFF_LUX)
		.sqrt()
		.max(CONE_SHADOW_NEAR_M + CONE_SHADOW_NEAR_M);
	let near = light
		.shadow_near_override()
		.filter(|value| value.is_finite())
		.unwrap_or(CONE_SHADOW_NEAR_M)
		.max(CONE_SHADOW_NEAR_M);
	let far = light
		.shadow_far_override()
		.filter(|value| value.is_finite())
		.unwrap_or(automatic_far)
		.max(near + CONE_SHADOW_NEAR_M);

	(near, far)
}

/// Builds the perspective view used to cull and render one cone-light shadow layer.
fn make_cone_shadow_view(light: ConeLight) -> View {
	let (near, far) = resolve_cone_shadow_range(light);
	View::new_perspective(
		(light.outer_angle * 2.0).to_degrees(),
		1.0,
		near,
		far,
		light.position,
		light.direction,
	)
}

/// Builds the diffuse IBL write shared by context-time sink creation and frame-time environment adoption.
fn diffuse_environment_descriptor_write(
	descriptor_set: ghi::DescriptorSetHandle,
	environment: EnvironmentTexture,
) -> ghi::DescriptorWrite {
	ghi::DescriptorWrite::combined_image_sampler(
		descriptor_set,
		ENVIRONMENT_BINDING.slot(),
		environment.diffuse_image,
		environment.sampler,
		ghi::Layouts::Read,
	)
}

/// Builds the mipmapped prefiltered environment write.
fn specular_environment_descriptor_write(
	descriptor_set: ghi::DescriptorSetHandle,
	environment: EnvironmentTexture,
) -> ghi::DescriptorWrite {
	ghi::DescriptorWrite::combined_image_sampler(
		descriptor_set,
		SPECULAR_ENVIRONMENT_BINDING.slot(),
		environment.specular_image,
		environment.sampler,
		ghi::Layouts::Read,
	)
}

const DEFAULT_ENVIRONMENT_TEXEL: [u8; 4] = [0, 0, 0, u8::MAX];

/// Creates the black environment sampled while no HDR environment is configured or its upload is pending.
fn create_fallback_environment_texture(context: &mut ghi::implementation::Context) -> EnvironmentTexture {
	let image = context.build_image(
		ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name("Visibility Environment Fallback")
			.extent(Extent::square(1))
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	// Keep alpha opaque so material evaluation samples this black environment instead of selecting
	// the analytical environment reserved for explicitly transparent environment texels.
	context
		.get_texture_slice_mut(image)
		.copy_from_slice(&DEFAULT_ENVIRONMENT_TEXEL);
	context.sync_texture(image);

	let sampler = context.build_sampler(
		ghi::sampler::Builder::new()
			.filtering_mode(ghi::FilteringModes::Linear)
			.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
			.mip_map_mode(ghi::FilteringModes::Linear)
			.addressing_mode(ghi::SamplerAddressingModes::Repeat)
			.min_lod(0.0)
			.max_lod(0.0),
	);

	EnvironmentTexture {
		diffuse_image: image.into(),
		specular_image: image.into(),
		sampler,
	}
}

/// Finds a binding already written for one renderable's frame pose, regardless of primitive ordering.
fn cached_skin_palette_base(cache: &[SkinningPaletteCacheEntry], handle: Handle, binding: *const SkinBinding) -> Option<u32> {
	cache
		.iter()
		.find(|entry| entry.handle == handle && entry.binding == binding)
		.map(|entry| entry.palette_base)
}

/// Reserves a non-overlapping frame-local vertex range for one active skinned primitive.
fn reserve_deformed_vertex_range(cursor: &mut usize, vertex_count: u32) -> u32 {
	let base = *cursor;
	let end = base
		.checked_add(vertex_count as usize)
		.expect("Visibility deformed vertex count overflowed. The most likely cause is corrupted primitive skinning metadata.");
	if end > MAX_SKINNED_VERTICES {
		panic!(
			"Visibility deformed vertex limit exceeded. The most likely cause is that active animated instances require more frame-local vertex storage than the visibility pipeline supports."
		);
	}
	*cursor = end;
	base as u32
}

impl VisibilityPipelineManager {
	/// Applies queued GTAO controls before any sink records this frame's commands.
	fn apply_gtao_configuration(&mut self) {
		while let Some(update) = self.gtao_configuration.read() {
			let Some(parameter) = update
				.parameter()
				.strip_prefix(crate::rendering::pipelines::visibility::render_pass::GTAO_CONFIGURATION_PREFIX)
			else {
				self.gtao_configuration.not_set(
					update.id(),
					"GTAO parameter was not set. The most likely cause is that the parameter is outside the `render.gtao.` namespace.",
				);
				continue;
			};

			match self.gtao_settings.with_parameter(parameter, update.value()) {
				Ok((settings, effective_value)) => {
					self.gtao_settings = settings;
					for sink_state in &mut self.scene.sink_states {
						sink_state.render_pass.set_gtao_settings(settings);
					}
					self.gtao_configuration.set(update.id(), effective_value);
				}
				Err(reason) => self.gtao_configuration.not_set(update.id(), reason),
			}
		}
	}
}

impl PipelineManager for VisibilityPipelineManager {
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>> {
		self.apply_gtao_configuration();
		self.adopt_resource_completions(frame);
		self.write_material_data(frame);
		self.rebuild_active_instances(frame);

		let mut shadow_lights = select_shadow_lights(self.scene.lights.iter().map(|(_, light)| light));
		if sinks.is_empty() {
			shadow_lights = ShadowLightSelection::default();
		}
		if shadow_lights.cone_count > MAX_CONE_SHADOWS {
			warn!(
				"Cone-light shadow budget exceeded. The most likely cause is that the scene contains more than {MAX_CONE_SHADOWS} cone lights. Extra cone lights remain lit without shadows."
			);
		}
		let [opaque_mesh_dispatch, transparent_mesh_dispatch] = self.mesh_dispatch_work.write_phases(
			frame,
			[
				self.scene.render_info.opaque_instances.as_slice(),
				self.scene.render_info.transparent_instances.as_slice(),
			],
		);

		if let Some(sink) = sinks.first() {
			let main_view = sink.view();
			let main_view_data = Self::make_shader_view_data(main_view);
			let views_data_buffer = frame.get_mut_dynamic_buffer_slice(self.scene.views_data_buffer_handle);

			for view_data in views_data_buffer.iter_mut() {
				*view_data = main_view_data;
			}

			if let Some((_, light_direction)) = shadow_lights.directional {
				for (cascade_index, (cascade_view, cascade_far)) in
					csm::make_csm_views(main_view, light_direction, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION)
						.zip(csm::make_cascade_split_ranges(main_view, SHADOW_CASCADE_COUNT).map(|(_, far)| far))
						.enumerate()
				{
					let mut cascade_view_data = Self::make_shader_view_data(cascade_view);
					cascade_view_data.far = cascade_far;
					views_data_buffer[cascade_index + 1] = cascade_view_data;
				}
			}

			for (layer, shadow_light) in shadow_lights.cones.iter().enumerate() {
				if let Some((_, light)) = shadow_light {
					views_data_buffer[CONE_SHADOW_VIEW_OFFSET + layer] =
						Self::make_shader_view_data(make_cone_shadow_view(*light));
				}
			}

			frame.sync_buffer(self.scene.views_data_buffer_handle);
		}

		let directional_shadow_light_index = shadow_lights.directional.map(|(index, _)| index);
		let cone_shadow_light_indices = shadow_lights.cones.map(|light| light.map(|(index, _)| index));
		self.scene
			.write_light_data(frame, directional_shadow_light_index, &cone_shadow_light_indices);

		let sink_x_rp = sinks.iter().filter_map(|sink| {
			self.scene
				.sink_states
				.iter()
				.find(|sink_state| sink_state.id == sink.index())
				.map(|sink_state| (sink, &sink_state.render_pass))
		});
		let skinning_pass = &self.skinning_pass;
		let skinning_dispatches = self.scene.render_info.skinning_dispatches.as_slice();

		let commands: SmallVec<[RenderPassReturn<'a>; 16]> = sink_x_rp
			.enumerate()
			.map(|(command_index, (v, r))| {
				crate::rendering::render_pass::allocate_render_command(
					frame_allocator,
					r.prepare(
						frame,
						v,
						(command_index == 0).then_some(skinning_pass),
						opaque_mesh_dispatch,
						transparent_mesh_dispatch,
						skinning_dispatches,
						&self.scene.render_info.opaque_instances,
						&self.scene.render_info.transparent_instances,
						&self.scene.render_info.opaque_materials,
						&self.scene.render_info.transparent_materials,
						&self.scene.render_info.opaque_material_mask,
						&self.scene.render_info.transparent_material_mask,
						directional_shadow_light_index.is_some(),
						cone_shadow_light_indices.iter().flatten().count(),
					),
				)
			})
			.collect::<SmallVec<[_; 16]>>();

		log::debug!(
			"Visibility prepare summary: sinks={}, sink_states={}, commands={}, requested_meshes={}, loaded_meshes={}, pending_renderables={}, render_entities={}, active_primitives={}, opaque_primitives={}, transparent_primitives={}, opaque_materials={}, transparent_materials={}, directional_shadow_enabled={}, cone_shadow_count={}",
			sinks.len(),
			self.scene.sink_states.len(),
			commands.len(),
			self.requested_meshes.len(),
			self.loaded_meshes.len(),
			self.pending_renderables.len(),
			self.scene.render_entities.len(),
			self.scene.render_info.active_instance_count(),
			self.scene.render_info.opaque_instances.len(),
			self.scene.render_info.transparent_instances.len(),
			self.scene.render_info.opaque_materials.len(),
			self.scene.render_info.transparent_materials.len(),
			directional_shadow_light_index.is_some(),
			cone_shadow_light_indices.iter().flatten().count(),
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
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Depth")
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);
		let primitive_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("primitive index"),
		);
		let instance_id = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("instance_id"),
		);

		let context = render_pass_builder.context();
		let visibility_passes_descriptor_set = context.create_descriptor_set(Some("Visibility Descriptor Set"));
		let material_evaluation_descriptor_set = context.create_descriptor_set(Some("Material Evaluation Descriptor Set"));

		let material_count_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Count")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_xy: ghi::BufferHandle<[(u16, u16); MAX_PIXEL_MAPPING_ENTRIES]> = context.build_buffer(
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
		let directional_shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(DIRECTIONAL_SHADOW_MAP_FORMAT, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Directional Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32))
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);
		let cone_shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(CONE_SHADOW_MAP_FORMAT, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Cone Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(MAX_CONE_SHADOWS as u32))
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
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
		context.write(&[
			ghi::DescriptorWrite::image(
				material_evaluation_descriptor_set,
				LIT_BINDING.slot(),
				ghi::BaseImageHandle::from(lit_target),
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHTING_DATA_BINDING.slot(),
				self.scene.light_data_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				MATERIALS_DATA_BINDING.slot(),
				self.materials_data_buffer_handle.into(),
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				AO_MAP_BINDING.slot(),
				ao_map,
				sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				SHADOW_MAP_BINDING.slot(),
				directional_shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				CONE_SHADOW_MAP_BINDING.slot(),
				cone_shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_COUNT_BINDING.slot(),
				material_count_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_OFFSET_BINDING.slot(),
				material_offset_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_OFFSET_SCRATCH_BINDING.slot(),
				material_offset_scratch_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_EVALUATION_DISPATCHES_BINDING.slot(),
				material_evaluation_dispatches.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_XY_BINDING.slot(),
				material_xy.into(),
			),
			ghi::DescriptorWrite::image(
				visibility_passes_descriptor_set,
				TRIANGLE_INDEX_BINDING.slot(),
				ghi::BaseImageHandle::from(primitive_index),
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				visibility_passes_descriptor_set,
				INSTANCE_ID_BINDING.slot(),
				ghi::BaseImageHandle::from(instance_id),
				ghi::Layouts::General,
			),
		]);
		context.write(&[diffuse_environment_descriptor_write(
			material_evaluation_descriptor_set,
			self.environment_texture,
		)]);
		context.write(&[specular_environment_descriptor_write(
			material_evaluation_descriptor_set,
			self.environment_texture,
		)]);

		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Lit", "main");

		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.context(),
			&self.shader_resources,
			self.scene.descriptor_set,
			visibility_passes_descriptor_set,
			material_evaluation_descriptor_set,
			material_count_buffer,
			ghi::BaseImageHandle::from(lit_target),
			ao_map.into(),
			directional_shadow_map.into(),
			cone_shadow_map.into(),
			ghi::BaseImageHandle::from(depth_target),
			ghi::BaseImageHandle::from(primitive_index),
			ghi::BaseImageHandle::from(instance_id),
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			self.gtao_settings,
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
	/// Base vertex in the frame-local deformation buffer, or `u32::MAX` for immutable geometry.
	skinned_base_vertex_index: u32,
	_padding: u32,
}

/// The `LightingData` struct preserves the complete CPU storage layout consumed by material evaluation.
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct LightingData {
	pub count: u32,
	_padding: [u32; 3],
	pub lights: [LightData; MAX_LIGHTS],
}

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
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
	pub(crate) view: ShaderMatrix4x3,
	pub(crate) view_projection: ShaderMatrix4,
	pub(crate) inverse_view: ShaderMatrix4x3,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
}

/// The `LightData` struct preserves one 16-byte-aligned, scene-referred light record across shader backends.
///
/// `color` stores RGB illuminance in lux for directional lights and RGB luminous intensity in candela for local lights.
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct LightData {
	pub position: ShaderVec3,
	pub color: ShaderVec3,
	pub direction: ShaderVec3,
	pub cone_cosines: [f32; 2],
	pub light_type: u32,
	pub shadow_views: [u32; 8],
	pub shadow_layer: u32,
}

/// The `MaterialData` struct retains fixed-width material texture indices for frame-local GPU uploads.
#[repr(C)]
#[derive(Copy, Clone)]
struct MaterialData {
	textures: [u32; MAX_MATERIAL_TEXTURES],
}

impl Default for MaterialData {
	fn default() -> Self {
		Self {
			textures: [u32::MAX; MAX_MATERIAL_TEXTURES],
		}
	}
}

/// Replaces one complete canonical material record and reports whether input slots were truncated.
fn write_material_texture_indices(
	material_data: &mut MaterialData,
	texture_indices: impl IntoIterator<Item = Option<u32>>,
) -> bool {
	*material_data = MaterialData::default();

	for (slot, texture_index) in texture_indices.into_iter().enumerate() {
		let Some(destination) = material_data.textures.get_mut(slot) else {
			return true;
		};
		*destination = texture_index.unwrap_or(u32::MAX);
	}

	false
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
	pub(crate) handle: Handle,
	entity: EntityHandle<dyn RenderableMesh>,
	shader_mesh: ShaderMesh,
	skinning: Option<RenderSkin>,
}

/// The `RenderSkin` struct keeps one primitive's immutable skin source and palette mapping beside its scene instance.
struct RenderSkin {
	binding: Arc<SkinBinding>,
	source_vertex_offset: u32,
	vertex_count: u32,
	skeleton_node_count: u32,
}

/// The `PendingRenderableInstance` struct associates a scene renderable with the mesh resource it is waiting for.
struct PendingRenderableInstance {
	handle: Handle,
	entity: EntityHandle<dyn RenderableMesh>,
	mesh_key: VisibilityMeshKey,
}

/// The `RenderDescription` struct retains one material's render-thread pipeline and authored alpha contract.
struct RenderDescription {
	index: u32,
	pipeline: Option<ghi::PipelineHandle>,
	name: String,
	alpha_mode: AlphaMode,
	texture_indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The `Instance` struct identifies one dense shader mesh and the work needed to rasterize it.
pub struct Instance {
	pub shader_mesh_index: u32,
	pub meshlet_count: u32,
}

/// The `RenderInfo` struct groups frame-local visibility work by the phase that will consume it.
pub struct RenderInfo {
	opaque_instances: Vec<Instance>,
	transparent_instances: Vec<Instance>,
	skinning_dispatches: Vec<SkinningDispatch>,
	opaque_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	transparent_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	opaque_material_mask: ActiveMaterialMask,
	transparent_material_mask: ActiveMaterialMask,
}

impl RenderInfo {
	/// Clears frame-local instance work while retaining the allocations used by prior frames.
	fn clear_active_instances(&mut self) {
		self.opaque_instances.clear();
		self.transparent_instances.clear();
		self.skinning_dispatches.clear();
		self.opaque_material_mask.fill(0);
		self.transparent_material_mask.fill(0);
	}

	/// Adds one active primitive to its authored material phase.
	fn push_active_instance(&mut self, instance: Instance, material_index: u32, alpha_mode: &AlphaMode) {
		let material_index = material_index as usize;
		assert!(
			material_index < MAX_MATERIALS,
			"Visibility material index is out of range. The most likely cause is that an active primitive references a material beyond MAX_MATERIALS."
		);
		let material_bit = 1u64 << (material_index % u64::BITS as usize);
		let material_word = material_index / u64::BITS as usize;
		if is_transparent(alpha_mode) {
			self.transparent_instances.push(instance);
			self.transparent_material_mask[material_word] |= material_bit;
		} else {
			self.opaque_instances.push(instance);
			self.opaque_material_mask[material_word] |= material_bit;
		}
	}

	fn active_instance_count(&self) -> usize {
		self.opaque_instances.len() + self.transparent_instances.len()
	}
}

/// Returns whether an authored alpha mode requires source-over rendering after the opaque phase.
fn is_transparent(alpha_mode: &AlphaMode) -> bool {
	matches!(alpha_mode, AlphaMode::Blend)
}

pub struct SinkState {
	id: usize,
	render_pass: VisibilityPipelineRenderPass,
}

/// The `MeshData` struct retains the mesh ranges and skeleton size needed by the
/// renderer after resource loading.
#[derive(Debug, Clone)]
pub struct MeshData {
	// (material_id)
	pub(crate) primitives: Vec<MeshPrimitive>,
	/// Number of global pose matrices expected from a renderable using this mesh.
	pub(crate) skeleton_node_count: u32,
	/// Base position in the vertex buffer.
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub(crate) triangle_offset: u32,
	/// Base position in the meshlet buffer, relative to the mesh.
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
	/// Base vertex in the compact immutable skinning source buffers.
	pub(crate) skinning_source_vertex_offset: Option<u32>,
	/// Number of vertices written by this primitive's compute dispatch.
	pub(crate) skinning_vertex_count: u32,
	/// Palette mapping retained after the resource reference leaves the upload worker.
	pub(crate) skin: Option<Arc<SkinBinding>>,
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use math::{Base as _, VecN as _, Vector3, Vector4};
	use resource_management::resources::skeleton::SkinBinding;
	use resource_management::types::AlphaMode;

	use super::{
		AO_MAP_BINDING, CONE_SHADOW_CUTOFF_LUX, CONE_SHADOW_MAP_BINDING, CONE_SHADOW_NEAR_M, DEFAULT_ENVIRONMENT_TEXEL,
		ENVIRONMENT_BINDING, Instance, LIGHTING_DATA_BINDING, LIT_BINDING, LightData, LightingData, MATERIALS_DATA_BINDING,
		MaterialData, RenderInfo, SHADOW_MAP_BINDING, SPECULAR_ENVIRONMENT_BINDING, ShaderMesh, ShaderViewData,
		SkinningPaletteCacheEntry, cached_skin_palette_base, make_cone_shadow_view, reserve_deformed_vertex_range,
		resolve_cone_shadow_range, select_shadow_lights, write_material_texture_indices,
	};
	use crate::core::factory::Factory;
	use crate::rendering::lights::{ConeLight, DirectionalLight, LightColor, Lights, PhotometricIntensity, PointLight};
	use crate::rendering::pipelines::visibility::resource_manager::IBL_SPECULAR_LEVEL_COUNT;
	use crate::rendering::pipelines::visibility::{
		INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
		MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_MATERIAL_TEXTURES, MAX_MATERIALS, MESH_DATA_BINDING,
		MESH_DATA_BUFFER_STRIDE, MESH_DISPATCH_WORK_BINDING, MESHLET_DATA_BINDING, PRIMITIVE_INDICES_BINDING,
		SKINNED_VERTICES_BINDING, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING,
		VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEW_DATA_BUFFER_STRIDE, VIEWS_DATA_BINDING,
	};

	/// Creates one compact shadow-capable cone for selection and projection tests.
	fn cone(position_x: f32) -> ConeLight {
		ConeLight::new(
			Vector3::new(position_x, 2.0, 3.0),
			Vector3::unit_z(),
			LightColor::TemperatureKelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			15.0_f32.to_radians(),
			30.0_f32.to_radians(),
		)
		.expect("physical cone light")
	}

	#[test]
	fn shadow_selection_keeps_one_directional_light_and_the_first_four_cones() {
		let lights = [
			Lights::Cone(
				ConeLight::new(
					Vector3::zero(),
					Vector3::unit_z(),
					LightColor::TemperatureKelvin(4_500.0),
					PhotometricIntensity::LuminousIntensity {
						candela: 100.0,
						reference_distance_m: 1.0,
					},
					0.25,
					std::f32::consts::PI,
				)
				.expect("physical cone light"),
			),
			Lights::Cone(cone(0.0)),
			Lights::Point(
				PointLight::new(
					Vector3::zero(),
					LightColor::TemperatureKelvin(4_500.0),
					PhotometricIntensity::LuminousIntensity {
						candela: 100.0,
						reference_distance_m: 1.0,
					},
				)
				.expect("physical point light"),
			),
			Lights::Direction(
				DirectionalLight::new(
					-Vector3::unit_y(),
					LightColor::TemperatureKelvin(6_500.0),
					PhotometricIntensity::Illuminance {
						lux: 100_000.0,
						measurement_distance_m: 1.0,
					},
				)
				.expect("physical directional light"),
			),
			Lights::Cone(cone(1.0)),
			Lights::Cone(cone(2.0)),
			Lights::Cone(cone(3.0)),
			Lights::Cone(cone(4.0)),
		];

		let selection = select_shadow_lights(lights.iter());

		assert_eq!(selection.directional.map(|(index, _)| index), Some(3));
		assert_eq!(
			selection.cones.map(|light| light.map(|(index, _)| index)),
			[Some(1), Some(4), Some(5), Some(6)]
		);
		assert_eq!(selection.cone_count, 5);
	}

	#[test]
	fn cone_shadow_view_uses_the_light_projection_and_automatic_clip_range() {
		let light = cone(1.0);
		let view = make_cone_shadow_view(light);
		let point = light.position + light.direction * 10.0;
		let clip = view.view_projection() * Vector4::new(point.x, point.y, point.z, 1.0);
		let ndc = clip / clip.w;

		assert!((view.y_fov() - 60.0).abs() < 0.0001);
		assert_eq!(view.near(), CONE_SHADOW_NEAR_M);
		assert!((view.far() - (100.0 / CONE_SHADOW_CUTOFF_LUX).sqrt()).abs() < 0.0001);
		assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001);
		assert!((0.0..=1.0).contains(&ndc.z));
	}

	#[test]
	fn cone_shadow_range_uses_manual_endpoints_and_clamps_invalid_values() {
		let light = cone(1.0).with_shadow_range(-4.0, f32::NAN);
		let (near, far) = resolve_cone_shadow_range(light);

		assert_eq!(near, CONE_SHADOW_NEAR_M);
		assert!((far - (100.0 / CONE_SHADOW_CUTOFF_LUX).sqrt()).abs() < 0.0001);

		let light = cone(1.0).with_shadow_near(50.0).with_shadow_far(20.0);
		assert_eq!(resolve_cone_shadow_range(light), (50.0, 50.1));
	}

	/// Verifies every pair of retained descriptor-set ranges stays disjoint.
	fn assert_descriptor_sets_disjoint(left: &[ghi::ShaderResourceDescriptor], right: &[ghi::ShaderResourceDescriptor]) {
		for left in left {
			let left_start = left.slot().index();
			let left_end = left_start + left.count();
			for right in right {
				let right_start = right.slot().index();
				let right_end = right_start + right.count();
				assert!(
					left_end <= right_start || right_end <= left_start,
					"Retained visibility descriptor ranges overlap. The most likely cause is assigning flat slots {left_start}..{left_end} and {right_start}..{right_end} to different descriptor sets."
				);
			}
		}
	}

	#[test]
	fn retained_visibility_descriptor_sets_use_disjoint_flat_ranges() {
		let base = [
			VIEWS_DATA_BINDING,
			MESH_DATA_BINDING,
			VERTEX_POSITIONS_BINDING,
			VERTEX_NORMALS_BINDING,
			SKINNED_VERTICES_BINDING,
			VERTEX_UV_BINDING,
			VERTEX_INDICES_BINDING,
			PRIMITIVE_INDICES_BINDING,
			MESHLET_DATA_BINDING,
			TEXTURES_BINDING,
			MESH_DISPATCH_WORK_BINDING,
		];
		let visibility = [
			MATERIAL_COUNT_BINDING,
			MATERIAL_OFFSET_BINDING,
			MATERIAL_OFFSET_SCRATCH_BINDING,
			MATERIAL_EVALUATION_DISPATCHES_BINDING,
			MATERIAL_XY_BINDING,
			TRIANGLE_INDEX_BINDING,
			INSTANCE_ID_BINDING,
		];
		let material = [
			LIT_BINDING,
			LIGHTING_DATA_BINDING,
			MATERIALS_DATA_BINDING,
			AO_MAP_BINDING,
			SHADOW_MAP_BINDING,
			CONE_SHADOW_MAP_BINDING,
			ENVIRONMENT_BINDING,
			SPECULAR_ENVIRONMENT_BINDING,
		];

		assert_descriptor_sets_disjoint(&base, &visibility);
		assert_descriptor_sets_disjoint(&base, &material);
		assert_descriptor_sets_disjoint(&visibility, &material);
	}

	#[test]
	fn environment_bindings_retain_diffuse_and_mipmapped_specular_images() {
		assert_eq!(ENVIRONMENT_BINDING.slot().index(), 1054);
		assert_eq!(ENVIRONMENT_BINDING.count(), 1);
		assert_eq!(SPECULAR_ENVIRONMENT_BINDING.slot().index(), 1055);
		assert_eq!(SPECULAR_ENVIRONMENT_BINDING.count(), 1);
	}

	#[test]
	fn default_environment_is_opaque_black() {
		assert_eq!(DEFAULT_ENVIRONMENT_TEXEL, [0, 0, 0, 255]);
	}

	#[test]
	fn lit_binding_supports_transparent_read_modify_write() {
		assert_eq!(LIT_BINDING.access(), ghi::AccessPolicies::READ_WRITE);
	}

	#[test]
	fn material_data_defaults_every_texture_slot_to_missing() {
		let material_data = MaterialData::default();

		assert!(material_data.textures.iter().all(|texture_index| *texture_index == u32::MAX));
	}

	#[test]
	fn material_texture_updates_replace_the_complete_canonical_record() {
		let mut material_data = MaterialData {
			textures: [41; MAX_MATERIAL_TEXTURES],
		};

		assert!(!write_material_texture_indices(&mut material_data, [Some(7), None, Some(11)]));
		assert_eq!(material_data.textures[..3], [7, u32::MAX, 11]);
		assert!(
			material_data.textures[3..]
				.iter()
				.all(|texture_index| *texture_index == u32::MAX)
		);

		assert!(!write_material_texture_indices(&mut material_data, [Some(3)]));
		assert_eq!(material_data.textures[0], 3);
		assert!(
			material_data.textures[1..]
				.iter()
				.all(|texture_index| *texture_index == u32::MAX)
		);
	}

	#[test]
	fn material_texture_updates_report_truncated_slots() {
		let mut material_data = MaterialData::default();

		assert!(write_material_texture_indices(
			&mut material_data,
			[Some(5); MAX_MATERIAL_TEXTURES + 1]
		));
		assert!(material_data.textures.iter().all(|texture_index| *texture_index == 5));
	}

	/// Verifies authored blend primitives are deferred while opaque and masked work stays in the first phase.
	#[test]
	fn active_instances_partition_by_authored_alpha_mode() {
		let mut render_info = RenderInfo {
			opaque_instances: Vec::new(),
			transparent_instances: Vec::new(),
			skinning_dispatches: Vec::new(),
			opaque_materials: Vec::new(),
			transparent_materials: Vec::new(),
			opaque_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
			transparent_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
		};
		let blended = Instance {
			shader_mesh_index: 3,
			meshlet_count: 1,
		};
		let opaque = Instance {
			shader_mesh_index: 5,
			meshlet_count: 2,
		};
		let masked = Instance {
			shader_mesh_index: 8,
			meshlet_count: 3,
		};

		render_info.push_active_instance(blended, 7, &AlphaMode::Blend);
		render_info.push_active_instance(opaque, 11, &AlphaMode::Opaque);
		render_info.push_active_instance(masked, 68, &AlphaMode::Mask(0.5));

		assert_eq!(render_info.opaque_instances, [opaque, masked]);
		assert_eq!(render_info.transparent_instances, [blended]);
		assert_eq!(render_info.opaque_material_mask[0], 1 << 11);
		assert_eq!(render_info.opaque_material_mask[1], 1 << 4);
		assert_eq!(render_info.transparent_material_mask[0], 1 << 7);
		assert_eq!(render_info.active_instance_count(), 3);
	}

	#[test]
	fn shader_mesh_matches_gpu_buffer_layout() {
		let (expected_size, expected_material_offset) = (80, 48);

		assert_eq!(
			std::mem::size_of::<ShaderMesh>(),
			expected_size,
			"Unexpected Visibility shader mesh size. The most likely cause is that the CPU-side mesh buffer layout drifted from the shader struct array stride."
		);
		assert_eq!(
			std::mem::size_of::<ShaderMesh>() as u32,
			MESH_DATA_BUFFER_STRIDE,
			"Unexpected Visibility shader mesh binding stride. The most likely cause is that the descriptor stride no longer matches the CPU-side mesh buffer layout."
		);
		assert_eq!(
			std::mem::align_of::<ShaderMesh>(),
			16,
			"Unexpected Visibility shader mesh alignment. The most likely cause is that the CPU-side mesh buffer no longer matches the shader struct alignment."
		);
		assert_eq!(
			std::mem::offset_of!(ShaderMesh, material_index),
			expected_material_offset,
			"Unexpected Visibility shader mesh material offset. The most likely cause is that the CPU-side mesh fields no longer match the shader struct."
		);
		assert_eq!(
			std::mem::offset_of!(ShaderMesh, skinned_base_vertex_index),
			expected_material_offset + 24,
			"Unexpected Visibility skinned vertex offset. The most likely cause is that the CPU-side mesh fields no longer match the visibility and material shader structs."
		);
	}

	#[test]
	fn shader_view_data_matches_compact_gpu_buffer_layout() {
		let (expected_size, expected_view_projection_offset, expected_inverse_view_offset, expected_fov_offset) =
			(176, 48, 112, 160);

		assert_eq!(
			std::mem::size_of::<ShaderViewData>(),
			expected_size,
			"Unexpected compact visibility view size. The most likely cause is that the CPU view record drifted from its shader storage layout."
		);
		assert_eq!(std::mem::size_of::<ShaderViewData>() as u32, VIEW_DATA_BUFFER_STRIDE);
		assert_eq!(std::mem::offset_of!(ShaderViewData, view), 0);
		assert_eq!(
			std::mem::offset_of!(ShaderViewData, view_projection),
			expected_view_projection_offset
		);
		assert_eq!(
			std::mem::offset_of!(ShaderViewData, inverse_view),
			expected_inverse_view_offset
		);
		assert_eq!(std::mem::offset_of!(ShaderViewData, fov), expected_fov_offset);
		assert_eq!(std::mem::offset_of!(ShaderViewData, near), expected_fov_offset + 8);
		assert_eq!(std::mem::offset_of!(ShaderViewData, far), expected_fov_offset + 12);
	}

	/// Ensures instances that share immutable source vertices cannot overwrite each other's deformation output.
	#[test]
	fn active_skin_instances_receive_non_overlapping_vertex_ranges() {
		let mut cursor = 0;
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 3), 0);
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 3), 3);
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 5), 6);
		assert_eq!(cursor, 11);
	}

	/// Ensures interleaved handles keep their palettes instance-local.
	#[test]
	fn noncontiguous_primitives_reuse_their_frame_skinning_palette() {
		let mut factory = Factory::new();
		let first_handle = factory.create(());
		let second_handle = factory.create(());
		let first_binding = Arc::new(SkinBinding { entries: Vec::new() });
		let second_binding = Arc::new(SkinBinding { entries: Vec::new() });
		let palette_cache = vec![
			SkinningPaletteCacheEntry {
				handle: first_handle,
				binding: Arc::as_ptr(&first_binding),
				palette_base: 7,
			},
			SkinningPaletteCacheEntry {
				handle: first_handle,
				binding: Arc::as_ptr(&second_binding),
				palette_base: 11,
			},
			SkinningPaletteCacheEntry {
				handle: second_handle,
				binding: Arc::as_ptr(&first_binding),
				palette_base: 17,
			},
		];

		assert_eq!(
			cached_skin_palette_base(&palette_cache, first_handle, Arc::as_ptr(&first_binding)),
			Some(7)
		);
		assert_eq!(
			cached_skin_palette_base(&palette_cache, first_handle, Arc::as_ptr(&second_binding)),
			Some(11)
		);
		assert_eq!(
			cached_skin_palette_base(&palette_cache, second_handle, Arc::as_ptr(&first_binding)),
			Some(17)
		);
	}

	#[test]
	fn lighting_data_matches_gpu_buffer_layout() {
		assert_eq!(
			std::mem::size_of::<LightData>(),
			96,
			"Unexpected visibility LightData size. The most likely cause is that the CPU light buffer layout drifted from the generated shader struct."
		);
		assert_eq!(
			std::mem::align_of::<LightData>(),
			16,
			"Unexpected visibility LightData alignment. The most likely cause is that ShaderVec3 padding changed."
		);
		assert_eq!(std::mem::offset_of!(LightData, position), 0);
		assert_eq!(std::mem::offset_of!(LightData, color), 16);
		assert_eq!(std::mem::offset_of!(LightData, direction), 32);
		assert_eq!(std::mem::offset_of!(LightData, cone_cosines), 48);
		assert_eq!(std::mem::offset_of!(LightData, light_type), 56);
		assert_eq!(std::mem::offset_of!(LightData, shadow_views), 60);
		assert_eq!(std::mem::offset_of!(LightData, shadow_layer), 92);

		assert_eq!(
			std::mem::size_of::<LightingData>(),
			1552,
			"Unexpected visibility LightingData size. The most likely cause is that the CPU lighting buffer no longer matches the shader struct array stride."
		);
		assert_eq!(std::mem::align_of::<LightingData>(), 16);
		assert_eq!(std::mem::offset_of!(LightingData, count), 0);
		assert_eq!(std::mem::offset_of!(LightingData, _padding), 4);
		assert_eq!(std::mem::offset_of!(LightingData, lights), 16);
	}
}

const LIT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1041),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ_WRITE,
);
const LIGHTING_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1045),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(1552);
const MATERIALS_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1046),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(64);
const AO_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1051),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const SHADOW_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1052),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const CONE_SHADOW_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1064),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const ENVIRONMENT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1054),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const SPECULAR_ENVIRONMENT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1055),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashSet, hash_map::Entry};
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use ::core::slice::SlicePattern;
use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use log::{error, warn};
use math::{ShaderMatrix4, ShaderMatrix4x3, Vector3, mat::MatInverse as _};
use resource_management::Reference;
use resource_management::asset::bema_asset_handler::ProgramGenerator;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::resources::skeleton::{AffineMatrix4x3Columns, SkinBinding, identity_affine_matrix4x3_columns};
use resource_management::shader::besl::backends::glsl::GLSLShaderGenerator;
use resource_management::shader::besl::backends::msl::MSLShaderGenerator;
use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::types::{AlphaMode, IndexStreamTypes, IntegralTypes, ShaderTypes};
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::sync::{Rc, RwLock};
use utils::{Box, Extent, RGBA, StableVec};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::core::{Entity, EntityHandle, factory::Handle};
use crate::ghi;
use crate::rendering::lights::{ConeLight, DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::GPUVertexDataManager;
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
use crate::rendering::pipelines::visibility::resource_manager::{
	IBL_SPECULAR_LEVEL_COUNT, MaterialPipelineConfig, PendingMaterialPipeline, VisibilityMeshKey,
	VisibilityPipelineResourceManagerClient, VisibilityResourceCompletion,
};
use crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager;
use crate::rendering::pipelines::visibility::skinning::{
	MAX_SKINNED_VERTICES, MAX_SKINNING_MATRICES, SkinningDispatch, SkinningPass, SkinningSourceBuffers,
};
use crate::rendering::pipelines::visibility::{
	ActiveMaterialMask, CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_VIEW_OFFSET, DIRECTIONAL_SHADOW_MAP_FORMAT, INSTANCE_ID_BINDING,
	MATERIAL_COUNT_BINDING,
	MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING,
	MAX_BINDLESS_TEXTURES, MAX_CONE_SHADOWS, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS, MAX_MESHLETS,
	MAX_PIXEL_MAPPING_ENTRIES, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESH_DATA_BINDING, MESHLET_DATA_BINDING,
	PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, SHADOW_VIEW_COUNT, SKINNED_VERTICES_BINDING,
	ShaderMeshletData, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING,
	VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::view::View;
use crate::rendering::{
	Environment, RenderableMesh, Sink, csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor,
	mesh, world_render_domain,
};
use crate::resource_management::{self};
use crate::space::Transformable as _;
