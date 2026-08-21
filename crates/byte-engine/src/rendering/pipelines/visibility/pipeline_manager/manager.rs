use super::*;
use crate::rendering::lights::IesProfile;

/// Returns the latest retained transform for scene creation, or identity when no update has arrived.
pub(super) fn retained_renderable_transform(transforms: &HashMap<Handle, Transform>, handle: Handle) -> Transform {
	transforms.get(&handle).cloned().unwrap_or_default()
}

impl VisibilityPipelineManager {
	/// Retains a renderable's latest world transform and applies it to every registered primitive.
	pub(crate) fn update_transform(&mut self, handle: Handle, transform: &crate::gameplay::transform::Transform) {
		self.renderable_transforms.insert(handle, transform.clone());
		self.scene.update_renderable_transform(handle, transform);
		self.scene.update_light_transform(handle, transform);
	}

	/// Retains queued transforms before resource adoption and applies them to already registered scene instances.
	pub(crate) fn process_transform_updates(&mut self) {
		while let Some(message) = self.transforms_listener.read() {
			self.update_transform(*message.handle(), message.transform());
		}
	}

	/// Retains a renderable's global skeleton pose for palette generation during frame preparation.
	pub fn update_pose(&mut self, handle: Handle, global_matrices: &[math::Matrix]) {
		self.scene.write_skinned_pose(handle, global_matrices);
	}

	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		resource_manager: VisibilityPipelineResourceManagerClient,
		shader_resources: EntityHandle<ResourceManager>,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		transforms_listener: DefaultListener<crate::gameplay::transform::TransformationUpdate>,
		gtao_configuration: crate::configuration::ConfigurationPort,
		settings: VisibilityPipelineSettings,
	) -> Self {
		let environment_texture = create_fallback_environment_texture(context);
		let skinning_pass = SkinningPass::new(
			context,
			&pipeline_manager,
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
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				MATERIALS_DATA_BINDING.slot(),
				materials_data_buffer_handle.into(),
			),
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
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		resource_manager.configure_material_pipeline(MaterialPipelineConfig::new(
			vec![ghi::pipelines::PushConstantRange::new(0, 8)],
			context.create_factory(),
			pipeline_manager.clone(),
		));
		Self {
			pipeline_manager,
			materials_data,
			materials_data_buffer_handle,
			mesh_dispatch_work,
			skinning_pass,
			shader_resources,
			transforms_listener,
			skinning_palette_scratch: Vec::new(),
			skinning_dual_quaternion_palette_scratch: Vec::new(),
			skinning_palette_cache: Vec::new(),
			resource_manager,
			requested_meshes: std::collections::HashSet::new(),
			pending_renderables: Vec::new(),
			renderable_transforms: HashMap::new(),
			loaded_meshes: HashMap::new(),
			loaded_materials: HashMap::new(),
			loaded_textures: HashSet::new(),
			loaded_ies_profiles: HashMap::new(),
			incomplete_renderables: HashSet::new(),
			environment_resource_id: None,
			environment_texture,
			cone_shadow_map_pool_capacity: settings.cone_shadow_map_pool_capacity,
			point_shadow_map_pool_capacity: settings.point_shadow_map_pool_capacity,
			gtao_configuration,
			gtao_settings: Default::default(),
			scene: VisibilitySceneManager {
				render_entities: StableVec::new(),
				skinning_poses: HashMap::new(),
				render_entity_handles: HashMap::new(),
				views_data_buffer_handle,
				descriptor_set,
				meshes_data_buffer,
				light_data_buffer,
				lights: StableVec::new(),
				render_info: RenderInfo {
					opaque_instances: Vec::new(),
					masked_instances: Vec::new(),
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
		if let Some(resource_id) = ies_profile_resource_id(&light) {
			self.resource_manager.request_image(resource_id.to_owned());
		}
		self.scene.lights.push((handle, light, Transform::default()));
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
			.find(|(_, (light_handle, ..))| *light_handle == handle)
		else {
			return;
		};

		self.scene.lights.remove(handle);
	}

	/// Requests the renderable mesh resources and keeps the scene instance pending until those resources are ready.
	pub(crate) fn request_mesh(&mut self, handle: Handle, renderable: RenderableMesh) {
		// Creation messages are upserts, but the latest independently published transform must survive replacement.
		self.remove_mesh_instance(handle);

		let source = renderable.source().clone();
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
			renderable,
			mesh_key: mesh_key.clone(),
		});
		self.resolve_pending_renderables_for_mesh(&mesh_key);
	}

	/// Removes a renderable and any transform retained for asynchronous creation.
	pub(crate) fn remove_mesh(&mut self, handle: Handle) {
		self.remove_mesh_instance(handle);
		self.renderable_transforms.remove(&handle);
	}

	/// Removes pending and resident instance state so an upsert can reuse the retained transform.
	fn remove_mesh_instance(&mut self, handle: Handle) {
		self.pending_renderables
			.retain(|pending_renderable| pending_renderable.handle != handle);
		self.scene.remove_renderable(handle);
	}

	pub(crate) fn adopt_resource_completions(&mut self, frame: &mut ghi::implementation::Frame) {
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
				VisibilityResourceCompletion::MaterialReady {
					id,
					index,
					pipeline,
					alpha_mode,
					coverage,
					textures,
				} => self.adopt_material_completion(id, index, pipeline, alpha_mode, coverage, textures),
				VisibilityResourceCompletion::ImageReady {
					key,
					index,
					image,
					sampler,
					upload,
					photometry,
				} => {
					let image = frame.intern_image(image);
					let sampler = frame.intern_sampler(sampler);
					let image = ghi::BaseImageHandle::from(image);
					self.resource_manager
						.enqueue_texture_upload(key, index, image, sampler, upload, photometry);
				}
				VisibilityResourceCompletion::EnvironmentReady { id, environment } => {
					if self.environment_resource_id.as_deref() == Some(id.as_str()) {
						let upload = environment.intern(id, frame);
						self.resource_manager.enqueue_environment_upload(upload);
					}
				}
				VisibilityResourceCompletion::TextureUploadReady {
					key,
					index,
					image,
					sampler,
					photometry,
				} => {
					log::debug!("Visibility texture upload adopted: index={}", index);
					self.write_texture_descriptors(frame, index, image, sampler);
					let profile_texture = photometry.and_then(|photometry| {
						(photometry.intensity_scale_candela.is_finite() && photometry.intensity_scale_candela > 0.0).then_some(
							IesProfileTexture {
								texture_index: index,
								intensity_scale_candela: photometry.intensity_scale_candela,
							},
						)
					});
					if let Some(profile_texture) = profile_texture {
						self.loaded_ies_profiles.insert(key.into_string(), profile_texture);
					} else if self
						.scene
						.lights
						.iter()
						.any(|(_, light, _)| ies_profile_resource_id(light) == Some(key.as_str()))
					{
						warn!(
							"Visibility IES profile is invalid: {}. The most likely cause is that the image was not baked from a usable .ies file or has an invalid candela scale. See https://byte-engine.0x44491229.dev/docs/use/lighting#use-an-ies-profile",
							key
						);
					}
					if self.loaded_textures.insert(index)
						&& self
							.loaded_materials
							.values()
							.any(|material| material.texture_indices.contains(&index))
					{
						self.rebuild_material_lists();
					}
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
		pipeline_ref: crate::rendering::PipelineRef,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		textures: Vec<Option<(String, u32)>>,
	) {
		let pipeline = self.pipeline_manager.pipeline(pipeline_ref);
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
		material_data.coverage_factor = coverage.factor;
		material_data.coverage_texture_slot = coverage.texture_slot.unwrap_or(u32::MAX);
		material_data.alpha_cutoff = match alpha_mode {
			AlphaMode::Mask(cutoff) => cutoff,
			AlphaMode::Opaque | AlphaMode::Blend => 0.0,
		};

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
				pipeline_ref,
				name: id,
				alpha_mode,
				texture_indices,
			},
		);
		self.rebuild_material_lists();
	}

	/// Publishes material pipeline handles resolved by the shared compiler since the previous frame.
	pub(crate) fn refresh_material_pipelines(&mut self) {
		let mut changed = false;
		for material in self.loaded_materials.values_mut() {
			let pipeline = self.pipeline_manager.pipeline(material.pipeline_ref);
			if material.pipeline != pipeline {
				material.pipeline = pipeline;
				changed = true;
			}
		}
		if changed {
			self.rebuild_material_lists();
		}
	}

	/// Copies canonical material metadata into the current frame-local buffer before rendering.
	pub(crate) fn write_material_data(&self, frame: &mut ghi::implementation::Frame) {
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

	/// Rebuilds the active instance list from whole renderables whose material dependencies are ready.
	pub(crate) fn rebuild_active_instances(&mut self, frame: &mut ghi::implementation::Frame) {
		self.scene.render_info.clear_active_instances();
		let loaded_materials = &self.loaded_materials;
		let loaded_textures = &self.loaded_textures;
		let render_entities = &self.scene.render_entities;
		let skinning_poses = &self.scene.skinning_poses;
		let palette_scratch = &mut self.skinning_palette_scratch;
		let dual_quaternion_palette_scratch = &mut self.skinning_dual_quaternion_palette_scratch;
		let palette_cache = &mut self.skinning_palette_cache;
		let mesh_data = frame.get_mut_dynamic_buffer_slice(self.scene.meshes_data_buffer);
		// Frame caches retain capacity but never retain entity or resource pointers beyond this rebuild.
		palette_cache.clear();
		palette_scratch.clear();
		dual_quaternion_palette_scratch.clear();
		collect_incomplete_renderables(
			render_entities
				.iter()
				.map(|render_entity| (render_entity.handle, render_entity.shader_mesh.material_index)),
			|material_index| {
				loaded_materials.get(&material_index).is_some_and(|material| {
					material.pipeline.is_some()
						&& material
							.texture_indices
							.iter()
							.all(|texture_index| loaded_textures.contains(texture_index))
				})
			},
			&mut self.incomplete_renderables,
		);

		let mut active_index = 0;
		let mut skipped_missing_material = 0usize;
		let mut active_meshlets = 0u32;
		let mut deformed_vertex_count = 0usize;
		let mut pose_matrix_count = 0usize;
		let mut palette_matrix_count = 0usize;
		let mut palette_dual_quaternion_count = 0usize;
		for render_entity in render_entities.iter() {
			// A renderable enters a frame as one object. Never expose the subset whose
			// materials happened to become resident first.
			if self.incomplete_renderables.contains(&render_entity.handle) {
				skipped_missing_material += 1;
				continue;
			}
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
					let palette = match cached_skin_palette(palette_cache, render_entity.handle, binding_ptr) {
						Some(palette) => Some(palette),
						_ => {
							let matrix_candidate_end = palette_matrix_count.checked_add(skinning.binding.len()).expect(
								"Visibility skin palette count overflowed. The most likely cause is corrupted skin binding metadata.",
							);
							// Grow only to the scene's high-water mark, then reuse this palette storage on later frames.
							palette_scratch.resize(matrix_candidate_end, identity_affine_matrix4x3_columns());

							match skinning
								.binding
								.write_matrix_palette(pose, &mut palette_scratch[palette_matrix_count..matrix_candidate_end])
							{
								Ok(()) => {
									let rigid_palette_start = dual_quaternion_palette_scratch.len();
									let palette_kind = if append_dual_quaternion_palette(
										&palette_scratch[palette_matrix_count..matrix_candidate_end],
										dual_quaternion_palette_scratch,
									) {
										let rigid_palette_end = dual_quaternion_palette_scratch.len();
										if rigid_palette_end > MAX_SKINNING_MATRICES {
											panic!(
												"Visibility dual-quaternion palette limit exceeded. The most likely cause is that active rigid skins require more joint transforms than the visibility pipeline supports."
											);
										}
										palette_dual_quaternion_count = rigid_palette_end;
										palette_scratch.truncate(palette_matrix_count);
										SkinningPaletteKind::DualQuaternion
									} else {
										if matrix_candidate_end > MAX_SKINNING_MATRICES {
											panic!(
												"Visibility matrix palette limit exceeded. The most likely cause is that active non-rigid skins require more joint matrices than the visibility pipeline supports."
											);
										}
										palette_matrix_count = matrix_candidate_end;
										SkinningPaletteKind::Matrix
									};
									let palette_base = match palette_kind {
										SkinningPaletteKind::Matrix => palette_matrix_count - skinning.binding.len(),
										SkinningPaletteKind::DualQuaternion => rigid_palette_start,
									} as u32;
									palette_cache.push(SkinningPaletteCacheEntry {
										handle: render_entity.handle,
										binding: binding_ptr,
										palette_base,
										palette_kind,
									});
									Some((palette_base, palette_kind))
								}
								Err(error) => {
									palette_scratch.truncate(palette_matrix_count);
									error!("Visibility skin palette could not be written: {error}");
									None
								}
							}
						}
					};

					if let Some((palette_base, palette_kind)) = palette {
						// Output is dense per active primitive, so shared meshes never overwrite another instance's pose.
						shader_mesh.skinned_base_vertex_index =
							reserve_deformed_vertex_range(&mut deformed_vertex_count, skinning.vertex_count);
						self.scene.render_info.skinning_dispatches.push(SkinningDispatch::new(
							skinning.source_vertex_offset,
							shader_mesh.skinned_base_vertex_index,
							palette_base,
							u32::try_from(skinning.binding.len())
								.expect("Skin palette size exceeds u32. The most likely cause is a corrupted skin binding."),
							skinning.vertex_count,
							palette_kind,
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
		if palette_dual_quaternion_count > 0 {
			self.skinning_pass
				.write_dual_quaternion_palette(frame, &dual_quaternion_palette_scratch[..palette_dual_quaternion_count]);
		}

		log::debug!(
			"Visibility active primitives rebuilt: render_entities={}, active={}, skipped_missing_material={}, active_meshlets={}, opaque_primitives={}, transparent_primitives={}, skinning_dispatches={}, deformed_vertices={}, pose_matrices={}, palette_matrices={}, palette_dual_quaternions={}",
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
			palette_dual_quaternion_count,
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

			let model = retained_renderable_transform(&self.renderable_transforms, pending_renderable.handle)
				.get_matrix()
				.into();
			resolved_renderables += 1;
			for primitive in &mesh.primitives {
				added_primitives += 1;
				added_meshlets += primitive.meshlet_count;
				self.scene.add_render_entity(RenderEntity {
					handle: pending_renderable.handle,
					renderable: pending_renderable.renderable.clone(),
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

	pub(crate) fn make_shader_view_data(view: View) -> ShaderViewData {
		let view_projection = view.view_projection();

		ShaderViewData {
			view: view.view().into(),
			view_projection: view_projection.into(),
			inverse_view: math::inverse(view.view()).into(),
			fov: view.fov(),
			near: view.near(),
			far: view.far(),
		}
	}
}

/// Returns the authored IES profile for a local light.
fn ies_profile(light: &Lights) -> Option<&IesProfile> {
	match light {
		Lights::Cone(light) => light.ies_profile(),
		Lights::Point(light) => light.ies_profile(),
		Lights::Direction(_) => None,
	}
}

/// Returns the authored IES image ID for a local profile light.
fn ies_profile_resource_id(light: &Lights) -> Option<&str> {
	ies_profile(light).map(IesProfile::resource_id)
}

/// Returns the dimmer while a profile is pending, then its dimmed calibrated scale after residency.
pub(super) fn ies_intensity_scale_for_profile(
	profile: Option<&IesProfile>,
	profiles: &HashMap<String, IesProfileTexture>,
) -> f32 {
	let Some(profile) = profile else {
		return 1.0;
	};
	profiles
		.get(profile.resource_id())
		.map_or(profile.dimmer(), |texture| texture.intensity_scale_candela * profile.dimmer())
}

/// Returns one light's analytic, fallback-profile, or calibrated-profile intensity scale.
pub(super) fn ies_intensity_scale(light: &Lights, profiles: &HashMap<String, IesProfileTexture>) -> f32 {
	ies_intensity_scale_for_profile(ies_profile(light), profiles)
}

/// Resolves an authored profile to its resident texture and dimmed calibrated candela scale.
pub(super) fn resolved_ies_profile_texture_for_profile(
	profile: Option<&IesProfile>,
	profiles: &HashMap<String, IesProfileTexture>,
) -> Option<IesProfileTexture> {
	let profile = profile?;
	let mut texture = profiles.get(profile.resource_id()).copied()?;
	texture.intensity_scale_candela *= profile.dimmer();
	Some(texture)
}

/// Resolves a profile light to its resident texture and dimmed calibrated candela scale.
pub(super) fn resolved_ies_profile_texture(
	light: &Lights,
	profiles: &HashMap<String, IesProfileTexture>,
) -> Option<IesProfileTexture> {
	resolved_ies_profile_texture_for_profile(ies_profile(light), profiles)
}

/// Finds a binding already written for one renderable's frame pose, regardless of primitive ordering.
pub(super) fn cached_skin_palette(
	cache: &[SkinningPaletteCacheEntry],
	handle: Handle,
	binding: *const SkinBinding,
) -> Option<(u32, SkinningPaletteKind)> {
	cache
		.iter()
		.find(|entry| entry.handle == handle && entry.binding == binding)
		.map(|entry| (entry.palette_base, entry.palette_kind))
}

/// Reserves a non-overlapping frame-local vertex range for one active skinned primitive.
pub(super) fn reserve_deformed_vertex_range(cursor: &mut usize, vertex_count: u32) -> u32 {
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
	pub(crate) fn apply_gtao_configuration(&mut self) {
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
