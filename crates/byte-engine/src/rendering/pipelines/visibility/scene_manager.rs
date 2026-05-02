/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	/// Render entities registered in the scene.
	render_entities: Vec<RenderEntity>,
	/// Loaded mesh resources.
	meshes: Vec<ResourceStates<MeshData, ()>>,
	/// Mapping from resource ID to mesh index.
	meshes_by_resource: HashMap<String, usize>,
	/// Mapping from generated mesh hash to mesh index.
	meshes_by_generated_hash: HashMap<u64, usize>,
	/// Mesh geometry uploaded on the transfer queue and waiting for graphics-side finalization.
	pending_meshes_by_resource: HashMap<String, PendingMeshData>,
	/// Image resources used by material evaluation.
	images: HashMap<String, ResourceStates<Image, PendingImage>>,
	/// Texture manager.
	texture_manager: TextureManager,
	/// Pipeline manager.
	pipeline_manager: PipelineManager,
	/// Mapping from mesh resource ID to mesh index.
	mesh_resources: HashMap<String, u32>,
	/// Material evaluation materials.
	material_evaluation_materials: HashMap<String, ResourceStates<RenderDescription, PendingRenderDescription>>,
	/// Views data buffer.
	views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	///  Materials data buffer.
	materials_data_buffer_handle: ghi::BufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Base descriptor set layout.
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	textures_binding: ghi::DescriptorSetBindingHandle,
	/// Handle to the buffer where each instance's data is stored.
	meshes_data_buffer: ghi::DynamicBufferHandle<[ShaderMesh; MAX_INSTANCES]>,
	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	/// Buffer containing lighting data.
	light_data_buffer: ghi::BufferHandle<LightingData>,
	/// Lights in the scene.
	lights: Vec<Lights>,
	/// Information about the current render.
	render_info: RenderInfo,
	/// Per-sink render state.
	sink_states: Vec<SinkState>,
	visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	resource_manager: EntityHandle<ResourceManager>,
	gpu_vertex_data_manager: GPUVertexDataManager,
}

impl VisibilityWorldRenderDomain {
	pub fn new(
		device: &mut ghi::implementation::Device,
		texture_manager: TextureManager,
		resource_manager: EntityHandle<ResourceManager>,
	) -> Self {
		// Initialize the extent to 0 to allocate memory lazily.
		let extent = Extent::square(0);

		let views_data_buffer_handle = device.build_dynamic_buffer::<[ShaderViewData; 8]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Views Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let meshes_data_buffer = device.build_dynamic_buffer::<[ShaderMesh; MAX_INSTANCES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshes Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

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

		let descriptor_set_layout = device.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

		let descriptor_set = device.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

		let views_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VIEWS_DATA_BINDING, views_data_buffer_handle.into()),
		);
		let meshes_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESH_DATA_BINDING, meshes_data_buffer.into()),
		);

		let mesh_data_manager = GPUVertexDataManager::new(device);

		let vertex_positions_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_POSITIONS_BINDING, mesh_data_manager.vertex_positions_buffer.into()),
		);
		let vertex_normals_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_NORMALS_BINDING, mesh_data_manager.vertex_normals_buffer.into()),
		);
		let vertex_uv_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_UV_BINDING, mesh_data_manager.vertex_uvs_buffer.into()),
		);
		let vertex_indices_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_INDICES_BINDING, mesh_data_manager.vertex_indices_buffer.into()),
		);
		let primitive_indices_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&PRIMITIVE_INDICES_BINDING, mesh_data_manager.primitive_indices_buffer.into()),
		);
		let meshlets_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESHLET_DATA_BINDING, mesh_data_manager.meshlets_data_buffer.into()),
		);
		let textures_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler_array(&TEXTURES_BINDING),
		);

		let bindings = [
			MATERIAL_COUNT_BINDING,
			MATERIAL_OFFSET_BINDING,
			MATERIAL_OFFSET_SCRATCH_BINDING,
			MATERIAL_EVALUATION_DISPATCHES_BINDING,
			MATERIAL_XY_BINDING,
			TRIANGLE_INDEX_BINDING,
			INSTANCE_ID_BINDING,
		];

		let visibility_descriptor_set_layout = device.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);

		let light_data_buffer = device.build_buffer::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Light Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let lighting_data = device.get_mut_buffer_slice(light_data_buffer);

		lighting_data.count = 0; // Initially, no lights

		let materials_data_buffer_handle = device.build_buffer::<[MaterialData; MAX_MATERIALS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Materials Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let bindings = [
			diffuse_binding_template,
			ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			specular_binding_template,
			ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			lighting_data_binding_template,
			materials_data_binding_template,
			ao_map_binding_template,
			shadow_map_binding_template,
			visibility_depth_binding_template,
			ibl_cubemap_binding_template,
		];

		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let material_evaluation_descriptor_set_layout =
			device.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
		let material_evaluation_descriptor_set = device.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&material_evaluation_descriptor_set_layout,
		);

		Self {
			render_entities: Vec::with_capacity(512),

			meshes: Vec::with_capacity(1024),
			meshes_by_resource: HashMap::with_capacity(1024),
			meshes_by_generated_hash: HashMap::with_capacity(128),
			pending_meshes_by_resource: HashMap::with_capacity(128),

			images: HashMap::with_capacity(1024),

			texture_manager,
			pipeline_manager: PipelineManager::new(device),
			gpu_vertex_data_manager: mesh_data_manager,

			mesh_resources: HashMap::new(),

			material_evaluation_materials: HashMap::new(),

			descriptor_set_layout,
			descriptor_set,

			visibility_descriptor_set_layout,

			textures_binding,

			views_data_buffer_handle,

			meshes_data_buffer,
			material_evaluation_descriptor_set_layout,
			material_evaluation_descriptor_set,

			light_data_buffer,
			materials_data_buffer_handle,

			lights: Vec::new(),

			render_info: RenderInfo {
				instances: Vec::with_capacity(4096),
				active_instances: Vec::with_capacity(4096),
				opaque_materials: Vec::with_capacity(MAX_MATERIALS),
				transparent_materials: Vec::with_capacity(MAX_MATERIALS),
			},

			sink_states: Vec::with_capacity(4),

			resource_manager,
		}
	}

	/// Registers a mesh instance for rendering and may load the mesh resource on the GPU if it is not already loaded.
	/// The mesh data may not be usable immediately after this call, as it is written to the GPU asynchronously.
	pub fn create_renderable_mesh_instance_and_write_mesh_data_if_not_exists<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		renderable: EntityHandle<dyn RenderableMesh>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> bool {
		let mesh_source = renderable.get_mesh();

		let Some((mesh_index, mesh)) = self.create_render_mesh_if_mesh_source_does_not_exists_and_return_mesh_object(
			c,
			staging_data_buffer,
			slice,
			mesh_source,
		) else {
			return false;
		};

		let model = renderable.transform().get_matrix().into();

		self.ensure_instance_capacity(mesh.primitives.len());

		for primitive in &mesh.primitives {
			self.render_entities.push(RenderEntity {
				entity: renderable.clone(),
				mesh_index,
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
			self.render_info.instances.push(Instance {
				meshlet_count: primitive.meshlet_count,
			});
		}

		true
	}

	/// Creates a render mesh for the given mesh source if it does not exist in the GPU, and returns the mesh object and buffer slice.
	fn create_render_mesh_if_mesh_source_does_not_exists_and_return_mesh_object<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		mesh_source: &MeshSource,
	) -> Option<(usize, MeshData)> {
		let mesh = match mesh_source {
			MeshSource::Resource(urid) => {
				if let Some(e) = self.meshes_by_resource.get(*urid) {
					// Mesh data already exists in GPU
					(*e, self.meshes[*e].get())
				} else {
					let mut resource_request: Reference<ResourceMesh> = {
						let resource_manager = &self.resource_manager;
						let Ok(resource_request) = resource_manager.request(urid) else {
							log::error!("Failed to load mesh resource {}", urid);
							return None;
						};
						resource_request
					};

					// Mesh data needs to be written to GPU
					if let Some(mesh) = self
						.gpu_vertex_data_manager
						.write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource(
							urid,
							c,
							staging_data_buffer,
							slice,
							&mut resource_request,
						) {
						let r = resource_request.resource();

						let primitives = r
							.primitives
							.iter()
							.zip(mesh.primitives)
							.map(|(rp, mp)| {
								let variant = {
									let idx = self.material_evaluation_materials.len() as u32;

									match self.material_evaluation_materials.entry(rp.material.id.clone()) {
										Entry::Occupied(v) => v.get().index(),
										Entry::Vacant(v) => {
											v.insert(ResourceStates::Pending(PendingRenderDescription { index: idx }));

											idx as u32
										}
									}
								};

								MeshPrimitive {
									material_index: variant,
									meshlet_count: mp.meshlet_count,
									meshlet_offset: mp.meshlet_offset,
									vertex_offset: mp.vertex_offset,
									primitive_offset: mp.primitive_offset,
									triangle_offset: mp.triangle_offset,
								}
							})
							.collect::<Vec<_>>();

						let mesh = MeshData {
							primitives,
							vertex_offset: mesh.vertex_offset,
							primitive_offset: mesh.primitive_offset,
							triangle_offset: mesh.triangle_offset,
							meshlet_offset: mesh.meshlet_offset,
							acceleration_structure: None,
						};

						let mesh_idx = self.meshes.len();

						self.meshes_by_resource.insert(urid.to_string(), mesh_idx); // Store render mesh idx associated to mesh resource id

						let mesh = self.meshes.push_mut(ResourceStates::Loading(c.frame_key(), mesh)).get();

						(mesh_idx, mesh)
					} else {
						return None; // We failed to load the mesh resource
					}
				}
			}
			MeshSource::Generated(generator) => {
				if let Some(e) = self.meshes_by_generated_hash.get(&generator.hash()) {
					// Mesh data already exists in GPU
					(*e, self.meshes[*e].get())
				} else {
					// Mesh data needs to be written to GPU
					if let Some(mesh) = self
						.gpu_vertex_data_manager
						.write_gpu_mesh_data_and_return_mesh_object_for_mesh_generator(
							generator.as_ref(),
							c,
							staging_data_buffer,
							slice,
						) {
						let primitives = mesh
							.primitives
							.iter()
							.map(|p| {
								let variant = {
									let idx = self.material_evaluation_materials.len() as u32;

									match self.material_evaluation_materials.entry("white_solid.bema".to_string()) {
										// TODO: remove hardcoded material
										Entry::Occupied(v) => v.get().index(),
										Entry::Vacant(v) => {
											v.insert(ResourceStates::Pending(PendingRenderDescription { index: idx }));

											idx as u32
										}
									}
								};

								MeshPrimitive {
									material_index: variant,
									meshlet_count: p.meshlet_count,
									meshlet_offset: p.meshlet_offset,
									vertex_offset: p.vertex_offset,
									primitive_offset: p.primitive_offset,
									triangle_offset: p.triangle_offset,
								}
							})
							.collect::<Vec<_>>();

						let mesh = MeshData {
							primitives,
							vertex_offset: mesh.vertex_offset,
							primitive_offset: mesh.primitive_offset,
							triangle_offset: mesh.triangle_offset,
							meshlet_offset: mesh.meshlet_offset,
							acceleration_structure: None,
						};

						let mesh_idx = self.meshes.len();

						self.meshes_by_generated_hash.insert(generator.hash(), mesh_idx); // Store render mesh idx associated to mesh generator hash

						let mesh = self.meshes.push_mut(ResourceStates::Loading(c.frame_key(), mesh)).get();

						(mesh_idx, mesh)
					} else {
						return None; // We failed to create the mesh from the generator
					}
				}
			}
		};

		Some((mesh.0, mesh.1.clone()))
	}

	fn create_material_resources<'a>(
		&'a mut self,
		resource: &mut resource_management::Reference<ResourceMaterial>,
		device: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let material_id = resource.id().to_string();
		let index = match self.material_evaluation_materials.get(&material_id) {
			Some(ResourceStates::Pending(pending)) => pending.index,
			Some(material) => return Ok(material.index()),
			None => self.material_evaluation_materials.len() as u32,
		};

		if index as usize >= MAX_MATERIALS {
			panic!(
				"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
			);
		}

		let shader_names = resource
			.resource()
			.shaders()
			.iter()
			.map(|shader| shader.id().to_string())
			.collect::<Vec<_>>();

		let parameters = &mut resource.resource_mut().parameters;

		let textures = parameters
			.iter_mut()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => Some((image.id().to_string(), self.reserve_image_resources(image.id()))),
				_ => None,
			})
			.collect::<Vec<_>>();
		let texture_dependencies = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(name, _)| name.clone()))
			.collect::<Vec<_>>();

		match resource.resource().model.name.as_str() {
			"Visibility" => match resource.resource().model.pass.as_str() {
				"MaterialEvaluation" => {
					let pipeline = self.pipeline_manager.load_material(
						&[
							self.descriptor_set_layout,
							self.visibility_descriptor_set_layout,
							self.material_evaluation_descriptor_set_layout,
						],
						&[ghi::pipelines::PushConstantRange::new(0, 4)],
						resource,
						device,
					);

					let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);
					let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;
					let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };
					material_data.textures.fill(u32::MAX);

					for (i, texture) in textures.iter().enumerate() {
						if i >= MAX_MATERIAL_TEXTURES {
							panic!(
								"Visibility material texture limit exceeded. The most likely cause is that a material references more textures than the fixed per-material indirection table supports."
							);
						}
						material_data.textures[i] = texture.as_ref().map(|(_, index)| *index).unwrap_or(0xFFFFFFFFu32) as u32;
					}

					device.sync_buffer(self.materials_data_buffer_handle);

					self.material_evaluation_materials.insert(
						material_id.clone(),
						ResourceStates::Loading(
							device.key(),
							RenderDescription {
								name: material_id,
								index,
								pipeline,
								alpha: false,
								textures: texture_dependencies,
								variant: RenderDescriptionVariants::Material { shaders: shader_names },
							},
						),
					);

					Ok(index)
				}
				_ => {
					error!("Unknown material pass: {}", resource.resource().model.pass);
					Err(())
				}
			},
			_ => {
				error!("Unknown material model");
				Err(())
			}
		}
	}

	/// Creates the needed GHI resource for the given material.
	/// Does nothing if the material has already been loaded.
	fn create_variant_resources<'s, 'a>(
		&'s mut self,
		mut resource: resource_management::Reference<ResourceVariant>,
		device: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let variant_id = resource.id().to_string();
		let index = match self.material_evaluation_materials.get(&variant_id) {
			Some(ResourceStates::Pending(pending)) => pending.index,
			Some(material) => return Ok(material.index()),
			None => self.material_evaluation_materials.len() as u32,
		};

		let specialization_constants: Vec<ghi::pipelines::SpecializationMapEntry> = resource
			.resource_mut()
			.variables
			.iter()
			.enumerate()
			.filter_map(|(i, variable)| match &variable.value {
				Value::Scalar(scalar) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "f32".to_string(), *scalar).into()
				}
				Value::Vector3(value) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec3f".to_string(), *value).into()
				}
				Value::Vector4(value) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), *value).into()
				}
				_ => None,
			})
			.collect();

		let pipeline = self.pipeline_manager.load_variant(
			&[
				self.descriptor_set_layout,
				self.visibility_descriptor_set_layout,
				self.material_evaluation_descriptor_set_layout,
			],
			&[ghi::pipelines::PushConstantRange::new(0, 4)],
			&specialization_constants,
			&mut resource,
			device,
		);

		let variant = resource.resource_mut();

		let _material_id = variant.material.id().to_string();

		self.create_material_resources(&mut variant.material, device)?;

		if index as usize >= MAX_MATERIALS {
			panic!(
				"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
			);
		}

		let textures = variant
			.variables
			.iter_mut()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => Some((image.id().to_string(), self.reserve_image_resources(image.id()))),
				_ => None,
			})
			.collect::<Vec<_>>();
		let texture_dependencies = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(name, _)| name.clone()))
			.collect::<Vec<_>>();

		let alpha = variant.alpha_mode == resource_management::types::AlphaMode::Blend;

		let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);

		let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

		let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };
		material_data.textures.fill(u32::MAX);

		for (i, texture) in textures.iter().enumerate() {
			if i >= MAX_MATERIAL_TEXTURES {
				panic!(
					"Visibility material texture limit exceeded. The most likely cause is that a material variant references more textures than the fixed per-material indirection table supports."
				);
			}
			material_data.textures[i] = texture.as_ref().map(|(_, index)| *index).unwrap_or(0xFFFFFFFFu32) as u32;
		}

		device.sync_buffer(self.materials_data_buffer_handle);

		self.material_evaluation_materials.insert(
			variant_id.clone(),
			ResourceStates::Loading(
				device.key(),
				RenderDescription {
					name: variant_id,
					index,
					pipeline,
					alpha,
					textures: texture_dependencies,
					variant: RenderDescriptionVariants::Variant {},
				},
			),
		);

		Ok(index)
	}

	pub fn create_light(&mut self, light: Lights) {
		self.lights.push(light);
	}

	pub fn transition_finished_transfer_resources(&mut self, frame_key: ghi::FrameKey) {
		self.meshes = self.meshes.drain(..).map(|mesh| mesh.frame_finished(frame_key)).collect();
		self.images = self
			.images
			.drain()
			.map(|(name, image)| (name, image.frame_finished(frame_key)))
			.collect();
	}

	pub fn transition_finished_graphics_resources(&mut self, frame_key: ghi::FrameKey) {
		self.material_evaluation_materials = self
			.material_evaluation_materials
			.drain()
			.map(|(name, material)| (name, material.frame_finished(frame_key)))
			.collect();
	}

	pub fn load_pending_material_evaluation_materials(&mut self, frame: &mut ghi::implementation::Frame) -> bool {
		let pending_materials = self
			.material_evaluation_materials
			.iter()
			.filter_map(|(name, material)| match material {
				ResourceStates::Pending(_) => Some(name.clone()),
				_ => None,
			})
			.collect::<Vec<_>>();

		let mut loaded_any = false;

		for material in pending_materials {
			let Ok(resource) = self.resource_manager.request::<ResourceVariant>(&material) else {
				log::error!("Failed to load material resource {}", material);
				continue;
			};

			loaded_any |= self.create_variant_resources(resource, frame).is_ok();
		}

		loaded_any
	}

	pub fn load_pending_material_textures(&mut self, frame: &mut ghi::implementation::Frame) -> bool {
		let pending_images = self
			.images
			.iter()
			.filter_map(|(name, image)| match image {
				ResourceStates::Pending(_) => Some(name.clone()),
				_ => None,
			})
			.collect::<Vec<_>>();

		let mut loaded_any = false;

		for image in pending_images {
			let Ok(mut resource) = self.resource_manager.request::<ResourceImage>(&image) else {
				log::error!("Failed to load image resource {}", image);
				continue;
			};

			loaded_any |= self.create_image_resources(&mut resource, frame).is_some();
		}

		loaded_any
	}

	/// Records pending texture uploads into the transfer command buffer and marks them loading for this transfer frame.
	pub fn prepare_texture_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		key: ghi::FrameKey,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> bool {
		let pending_images = self
			.images
			.iter()
			.filter_map(|(name, image)| match image {
				ResourceStates::Pending(pending) if pending.image.is_some() && pending.upload.is_some() => Some(name.clone()),
				_ => None,
			})
			.collect::<Vec<_>>();

		let mut recorded_work = false;

		for name in pending_images {
			let Some(ResourceStates::Pending(mut pending)) = self.images.remove(&name) else {
				continue;
			};
			let (Some(image), Some(upload)) = (pending.image.take(), pending.upload.take()) else {
				self.images.insert(name, ResourceStates::Pending(pending));
				continue;
			};
			const TEXTURE_UPLOAD_ALIGNMENT: usize = 256;
			if upload.data.len() > slice.remaining_aligned(TEXTURE_UPLOAD_ALIGNMENT) {
				self.images.insert(
					name,
					ResourceStates::Pending(PendingImage {
						index: image.index,
						image: Some(image),
						upload: Some(upload),
					}),
				);
				break;
			}

			let (source_offset, source_buffer) = slice.take_with_offset_aligned(upload.data.len(), TEXTURE_UPLOAD_ALIGNMENT);
			source_buffer.copy_from_slice(&upload.data);
			transfer.copy_buffer_to_images(&[ghi::BufferImageCopyDescriptor::new(
				staging_data_buffer,
				source_offset,
				upload.source_bytes_per_row,
				upload.source_bytes_per_image,
				image.image,
			)]);
			self.images.insert(name, ResourceStates::Loading(key, image));
			recorded_work = true;
		}

		recorded_work
	}

	fn reserve_image_resources(&mut self, id: &str) -> u32 {
		let index = self.images.len() as u32;

		match self.images.entry(id.to_string()) {
			Entry::Occupied(image) => image.get().index(),
			Entry::Vacant(image) => {
				if index as usize >= MAX_BINDLESS_TEXTURES {
					panic!(
						"Visibility bindless texture limit exceeded. The most likely cause is that the scene references more material images than the global descriptor array supports."
					);
				}
				image.insert(ResourceStates::Pending(PendingImage {
					index,
					image: None,
					upload: None,
				}));
				index
			}
		}
	}

	/// Creates the needed GHI resources for the given image.
	/// Does nothing if the image has already been loaded.
	fn create_image_resources(
		&mut self,
		resource: &mut resource_management::Reference<ResourceImage>,
		device: &mut ghi::implementation::Frame,
	) -> Option<u32> {
		let image_id = resource.id().to_string();
		let index = match self.images.get(&image_id) {
			Some(ResourceStates::Pending(pending)) => {
				if pending.image.is_some() {
					return Some(pending.index);
				}
				pending.index
			}
			Some(image) => return Some(image.index()),
			None => self.images.len() as u32,
		};

		let Some((_, image, sampler, upload)) = self.texture_manager.load(resource, device) else {
			return None;
		};

		let image = Image { index, image, sampler };
		self.write_image_descriptors(&image, device);

		if let Some(upload) = upload {
			self.images.insert(
				image_id,
				ResourceStates::Pending(PendingImage {
					index,
					image: Some(image),
					upload: Some(upload),
				}),
			);
		} else {
			self.images.insert(image_id, ResourceStates::Loaded(image));
		}

		Some(index)
	}

	fn write_image_descriptors(&self, image: &Image, device: &mut ghi::implementation::Frame) {
		let writes = self
			.texture_binding_handles()
			.into_iter()
			.map(|binding| {
				ghi::descriptors::Write::combined_image_sampler_array(
					binding,
					image.image,
					image.sampler,
					ghi::Layouts::Read,
					image.index,
				)
			})
			.collect::<Vec<_>>();
		device.write(&writes);
	}

	fn material_ready(&self, material: &RenderDescription) -> bool {
		material.pipeline.is_some()
			&& material
				.textures
				.iter()
				.all(|texture| self.images.get(texture).is_some_and(|image| image.is_ready()))
	}

	/// Uploads the current scene lights to the GPU buffer used by material evaluation.
	fn write_light_data(&self, frame: &mut ghi::implementation::Frame, shadow_light_index: Option<usize>) {
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

	fn texture_binding_handles(&self) -> Vec<ghi::DescriptorSetBindingHandle> {
		let mut bindings = Vec::with_capacity(self.sink_states.len() + 1);
		bindings.push(self.textures_binding);
		bindings.extend(self.sink_states.iter().map(|sink_state| sink_state.textures_binding));
		bindings
	}

	fn ensure_instance_capacity(&self, additional_instances: usize) {
		let total_instances = self.render_entities.len() + additional_instances;

		if total_instances > MAX_INSTANCES {
			panic!(
				"Visibility instance limit exceeded. The most likely cause is that the scene contains more mesh primitives than the visibility pipeline supports."
			);
		}
	}
}

impl SceneManager for VisibilityWorldRenderDomain {
	fn before_prepare(&mut self, frame: &mut ghi::implementation::Frame, _sinks: &[Sink]) {
		for (name, pipeline) in self.pipeline_manager.poll(frame, MAX_PIPELINE_ADOPTIONS_PER_FRAME) {
			if let Some(material) = self.material_evaluation_materials.get_mut(&name) {
				match material {
					ResourceStates::Pending(_) => {}
					ResourceStates::Loading(_, material) | ResourceStates::Loaded(material) => {
						material.pipeline = Some(pipeline)
					}
				}
			}
		}

		let meshes_data_buffer = frame.get_mut_dynamic_buffer_slice(self.meshes_data_buffer);
		let mut ready_materials = [false; MAX_MATERIALS];

		for material in self.material_evaluation_materials.values() {
			if let Some(material) = material.get_loaded() {
				ready_materials[material.index as usize] = self.material_ready(material);
			}
		}

		if self.render_entities.len() > MAX_INSTANCES {
			panic!(
				"Visibility instance limit exceeded. The most likely cause is that the scene contains more mesh primitives than the visibility pipeline supports."
			);
		}

		self.render_info.active_instances.clear();

		for (render_entity, instance) in self.render_entities.iter().zip(self.render_info.instances.iter()) {
			if !self.meshes[render_entity.mesh_index].is_ready() {
				continue;
			}

			if !ready_materials[render_entity.shader_mesh.material_index as usize] {
				continue;
			}

			let active_index = self.render_info.active_instances.len();
			meshes_data_buffer[active_index] = ShaderMesh {
				model: render_entity.entity.transform().get_matrix().into(),
				..render_entity.shader_mesh
			};
			self.render_info.active_instances.push(*instance);
		}

		self.render_info.opaque_materials = self
			.material_evaluation_materials
			.values()
			.filter_map(|v| v.get_loaded())
			.filter(|v| self.material_ready(v))
			.filter(|v| v.alpha == false)
			.filter_map(|v| v.pipeline.map(|pipeline| (v.name.clone(), v.index, pipeline)))
			.collect::<Vec<_>>();
		self.render_info.transparent_materials = self
			.material_evaluation_materials
			.values()
			.filter_map(|v| v.get_loaded())
			.filter(|v| self.material_ready(v))
			.filter(|v| v.alpha == true)
			.filter_map(|v| v.pipeline.map(|pipeline| (v.name.clone(), v.index, pipeline)))
			.collect::<Vec<_>>();
	}

	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sinks: &[Sink]) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		let shadow_light = self.lights.iter().enumerate().find_map(|(index, light)| match light {
			Lights::Direction(light) => Some((index, light.direction)),
			Lights::Point(_) => None,
		});
		let shadow_light_index = if sinks.is_empty() == false {
			shadow_light.map(|(index, _)| index)
		} else {
			None
		};

		for sink in sinks {
			let Some(sink_state) = self.sink_states.iter().find(|sink_state| sink_state.id == sink.index()) else {
				continue;
			};

			let main_view = sink.view();
			let main_view_data = Self::make_shader_view_data(main_view);
			let views_data_buffer = frame.get_mut_dynamic_buffer_slice(sink_state.views_data_buffer_handle);

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
		}

		self.write_light_data(frame, shadow_light_index);

		let sink_x_rp = sinks.iter().filter_map(|sink| {
			self.sink_states
				.iter()
				.find(|sink_state| sink_state.id == sink.index())
				.map(|sink_state| (sink, &sink_state.render_pass))
		});

		let commands: Vec<Box<dyn RenderPassFunction>> = sink_x_rp
			.map(|(v, r)| {
				Box::new(r.prepare(
					frame,
					v,
					&self.render_info.active_instances,
					&self.render_info.opaque_materials,
					&self.render_info.transparent_materials,
					shadow_light_index.is_some(),
				)) as Box<dyn RenderPassFunction>
			})
			.collect::<Vec<_>>();

		Some(commands)
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let diffuse_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Diffuse"),
		);
		let specular_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Specular"),
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

		let device = render_pass_builder.device();
		let views_data_buffer_handle = device.build_dynamic_buffer::<[ShaderViewData; 8]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Views Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let base_descriptor_set = device.create_descriptor_set(Some("Base Descriptor Set"), &self.descriptor_set_layout);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(&VIEWS_DATA_BINDING, views_data_buffer_handle.into()),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(&MESH_DATA_BINDING, self.meshes_data_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(
				&VERTEX_POSITIONS_BINDING,
				self.gpu_vertex_data_manager.vertex_positions_buffer.into(),
			),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(
				&VERTEX_NORMALS_BINDING,
				self.gpu_vertex_data_manager.vertex_normals_buffer.into(),
			),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_UV_BINDING, self.gpu_vertex_data_manager.vertex_uvs_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(
				&VERTEX_INDICES_BINDING,
				self.gpu_vertex_data_manager.vertex_indices_buffer.into(),
			),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(
				&PRIMITIVE_INDICES_BINDING,
				self.gpu_vertex_data_manager.primitive_indices_buffer.into(),
			),
		);
		let _ = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::buffer(
				&MESHLET_DATA_BINDING,
				self.gpu_vertex_data_manager.meshlets_data_buffer.into(),
			),
		);
		let textures_binding = device.create_descriptor_binding(
			base_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler_array(&TEXTURES_BINDING),
		);
		let texture_writes = self
			.images
			.values()
			.filter_map(|image| match image {
				ResourceStates::Pending(pending) => pending.image.as_ref(),
				ResourceStates::Loading(_, image) | ResourceStates::Loaded(image) => Some(image),
			})
			.map(|image| {
				ghi::descriptors::Write::combined_image_sampler_array(
					textures_binding,
					image.image,
					image.sampler,
					ghi::Layouts::Read,
					image.index,
				)
			})
			.collect::<Vec<_>>();
		if texture_writes.is_empty() == false {
			device.write(&texture_writes);
		}

		let visibility_passes_descriptor_set =
			device.create_descriptor_set(Some("Visibility Descriptor Set"), &self.visibility_descriptor_set_layout);
		let material_evaluation_descriptor_set = device.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&self.material_evaluation_descriptor_set_layout,
		);

		let material_count_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Count")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_xy = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material XY")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_evaluation_dispatches = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect)
				.name("Material Evaluation Dipatches")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_scratch_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset Scratch")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let ao_map = device.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let shadow_map = device.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32)),
		);
		let ibl_cubemap = device.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("IBL Cubemap")
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.extent(Extent::square(1))
				.array_layers(NonZeroU32::new(6)),
		);
		device.write_texture(ibl_cubemap, |bytes| bytes.fill(255));
		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let visibility_depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);

		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::image(&diffuse_binding_template, ghi::BaseImageHandle::from(diffuse_target)),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::image(&specular_binding_template, ghi::BaseImageHandle::from(specular_target)),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&lighting_data_binding_template, self.light_data_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&materials_data_binding_template, self.materials_data_buffer_handle.into()),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&ao_map_binding_template,
				ao_map,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&shadow_map_binding_template,
				shadow_map,
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&visibility_depth_binding_template,
				ghi::BaseImageHandle::from(depth_target),
				visibility_depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&ibl_cubemap_binding_template,
				ibl_cubemap,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);

		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_COUNT_BINDING, material_count_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_BINDING, material_offset_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_scratch_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_EVALUATION_DISPATCHES_BINDING, material_evaluation_dispatches.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_XY_BINDING, material_xy.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&TRIANGLE_INDEX_BINDING, ghi::BaseImageHandle::from(primitive_index)),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&INSTANCE_ID_BINDING, ghi::BaseImageHandle::from(instance_id)),
		);

		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Diffuse", "main");

		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.device(),
			self.descriptor_set_layout,
			self.visibility_descriptor_set_layout,
			base_descriptor_set,
			visibility_passes_descriptor_set,
			material_evaluation_descriptor_set,
			material_count_buffer,
			ghi::BaseImageHandle::from(diffuse_target),
			ghi::BaseImageHandle::from(specular_target),
			ao_map.into(),
			shadow_map.into(),
			ibl_cubemap.into(),
			ghi::BaseImageHandle::from(depth_target),
			ghi::BaseImageHandle::from(primitive_index),
			ghi::BaseImageHandle::from(instance_id),
			material_xy,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);

		self.sink_states.push(SinkState {
			id: sink_id,
			views_data_buffer_handle,
			textures_binding,
			render_pass,
		});
	}
}

#[repr(C, align(16))]
#[derive(Copy, Clone)]
struct ShaderMesh {
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
struct ShaderVec3 {
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
	position: ShaderVec3,
	color: ShaderVec3,
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
struct RenderEntity {
	entity: EntityHandle<dyn RenderableMesh>,
	mesh_index: usize,
	shader_mesh: ShaderMesh,
}

enum MeshState {
	Build { mesh_handle: String },
	Update {},
}

struct RayTracing {
	top_level_acceleration_structure: ghi::TopLevelAccelerationStructureHandle,
	descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline: ghi::PipelineHandle,

	ray_gen_sbt_buffer: ghi::BaseBufferHandle,
	miss_sbt_buffer: ghi::BaseBufferHandle,
	hit_sbt_buffer: ghi::BaseBufferHandle,

	shadow_map_resolution: Extent,
	shadow_map: ghi::BaseImageHandle,

	instances_buffer: ghi::BaseBufferHandle,
	scratch_buffer: ghi::BaseBufferHandle,

	pending_meshes: Vec<MeshState>,
}

enum RenderDescriptionVariants {
	Material { shaders: Vec<String> },
	Variant {},
}

struct RenderDescription {
	index: u32,
	pipeline: Option<ghi::PipelineHandle>,
	name: String,
	alpha: bool,
	textures: Vec<String>,
	variant: RenderDescriptionVariants,
}

/// The `PendingRenderDescription` struct preserves a material slot before its render resources exist.
struct PendingRenderDescription {
	index: u32,
}

impl ResourceStates<RenderDescription, PendingRenderDescription> {
	fn index(&self) -> u32 {
		match self {
			ResourceStates::Pending(pending) => pending.index,
			ResourceStates::Loading(_, material) | ResourceStates::Loaded(material) => material.index,
		}
	}

	fn get_loaded(&self) -> Option<&RenderDescription> {
		match self {
			ResourceStates::Loaded(material) => Some(material),
			_ => None,
		}
	}
}

#[derive(Clone, Copy)]
pub struct Instance {
	pub meshlet_count: u32,
}

struct RenderInfo {
	instances: Vec<Instance>,
	active_instances: Vec<Instance>,
	opaque_materials: Vec<(String, u32, ghi::PipelineHandle)>,
	transparent_materials: Vec<(String, u32, ghi::PipelineHandle)>,
}

struct SinkState {
	id: usize,
	views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	textures_binding: ghi::DescriptorSetBindingHandle,
	render_pass: VisibilityPipelineRenderPass,
}

/// This structure hosts data analogous to the image resource's data.
struct Image {
	/// This is the index of the image in the descriptor set.
	index: u32,
	image: ghi::BaseImageHandle,
	sampler: ghi::SamplerHandle,
}

/// The `PendingImage` struct preserves a texture slot before its render resources exist.
struct PendingImage {
	index: u32,
	image: Option<Image>,
	upload: Option<TextureUpload>,
}

impl ResourceStates<Image, PendingImage> {
	fn index(&self) -> u32 {
		match self {
			ResourceStates::Pending(pending) => pending.index,
			ResourceStates::Loading(_, image) | ResourceStates::Loaded(image) => image.index,
		}
	}

	fn get_loaded(&self) -> Option<&Image> {
		match self {
			ResourceStates::Loaded(image) => Some(image),
			_ => None,
		}
	}
}

use crate::ghi;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::GPUVertexDataManager;

pub enum ResourceStates<T, P> {
	Pending(P),
	Loading(ghi::FrameKey, T),
	Loaded(T),
}

impl<T, P> ResourceStates<T, P> {
	pub fn is_ready(&self) -> bool {
		match self {
			ResourceStates::Loaded(_) => true,
			_ => false,
		}
	}

	pub fn get(&self) -> &T {
		match self {
			ResourceStates::Loading(_, v) => v,
			ResourceStates::Loaded(v) => v,
			_ => panic!(),
		}
	}

	pub fn get_mut(&mut self) -> &mut T {
		match self {
			ResourceStates::Loading(_, v) => v,
			ResourceStates::Loaded(v) => v,
			_ => panic!(),
		}
	}

	pub fn frame_finished(self, frame_key: ghi::FrameKey) -> Self {
		match self {
			ResourceStates::Loading(loading_frame_key, v) => {
				if loading_frame_key == frame_key {
					ResourceStates::Loaded(v)
				} else {
					ResourceStates::Loading(loading_frame_key, v)
				}
			}
			_ => self,
		}
	}
}

/// This structure hosts data analogous to the mesh resource's data.
/// It stores data relevant to the renderer which allows not to have to access/request the mesh resource.
#[derive(Debug, Clone)]
pub struct MeshData {
	// (material_id)
	primitives: Vec<MeshPrimitive>,
	/// The base position into the vertex buffer
	vertex_offset: u32,
	primitive_offset: u32,
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	triangle_offset: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the mesh
	meshlet_offset: u32,
	acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The index of the material used by this primitive.
	material_index: u32,
	/// The meshlet count.
	meshlet_count: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the primitive in the mesh
	meshlet_offset: u32,
	/// The vertex offset.
	/// The base position into the vertex buffer
	vertex_offset: u32,
	/// The primitive indices offset.
	/// The base position into the primitive indices buffer
	primitive_offset: u32,
	/// The triangle offset.
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	triangle_offset: u32,
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

const diffuse_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const specular_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const lighting_data_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const materials_data_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(5, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const ao_map_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	10,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const shadow_map_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(
	11,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
	1,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const visibility_depth_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	12,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const ibl_cubemap_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	13,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);

const MAX_PIPELINE_ADOPTIONS_PER_FRAME: usize = 8;

use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};

use ::core::slice::SlicePattern;
use ghi::device::{Device as _, DeviceCreate as _};
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
use resource_management::glsl_shader_generator::GLSLShaderGenerator;
use resource_management::msl_shader_generator::MSLShaderGenerator;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::Variant as ResourceVariant;
use resource_management::resources::material::{Material as ResourceMaterial, Parameter, Shader, Value, VariantVariable};
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::shader_generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::spirv_shader_generator::SPIRVShaderGenerator;
use resource_management::types::{IndexStreamTypes, IntegralTypes, ShaderTypes};
use resource_management::{glsl, Reference};
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::sync::{Rc, RwLock};
use utils::{Box, Extent, RGBA};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::core::{Entity, EntityHandle};
use crate::rendering::common_shader_generator::{CommonShaderGenerator, CommonShaderScope};
use crate::rendering::lights::{DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
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
use crate::rendering::scene_manager::SceneManager;
use crate::rendering::texture_manager::{TextureManager, TextureUpload};
use crate::rendering::view::View;
use crate::rendering::{
	csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh, world_render_domain,
	RenderableMesh, Sink,
};
use crate::resource_management::{self};
use crate::space::Transformable as _;
