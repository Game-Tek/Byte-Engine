use super::*;

/// The `VisibilityPipelineResourceManagerClient` struct connects render logic to the asynchronous visibility resource worker.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	pub(crate) gpu_vertex_data_manager: GPUVertexDataManager,
	pub(super) commands: kanal::Sender<VisibilityTransferCommand>,
	pub(super) completions: Receiver<VisibilityResourceCompletion>,
	pub(super) upload_completions: CompletionList,
	pub(super) prepared_uploads: Receiver<PreparedUpload>,
	pub(super) pending_uploads: VecDeque<PreparedUpload>,
	pub(super) submitted_uploads: VecDeque<SubmittedUploadBatch>,
	pub(super) staging_data_buffer: ghi::BaseBufferHandle,
}

/// The `VisibilityPipelineResourceManagerWorker` struct owns asynchronous visibility resource loading and preparation.
pub(crate) struct VisibilityPipelineResourceManagerWorker {
	pub(super) resource_manager: VisibilityPipelineResourceManager,
	pub(super) commands: kanal::AsyncReceiver<VisibilityTransferCommand>,
	pub(super) prepared_uploads: Sender<PreparedUpload>,
}

impl VisibilityPipelineResourceManagerClient {
	/// Sends one ordered command to the asynchronous resource worker.
	fn send(&self, command: VisibilityTransferCommand) {
		if self.commands.send(command).is_err() {
			log::error!(
				"Visibility resource command failed. The most likely cause is that the asynchronous resource task terminated."
			);
		}
	}

	/// Requests a mesh from the asynchronous resource task.
	pub(crate) fn request_mesh(&self, key: VisibilityMeshKey, source: MeshSource) {
		self.send(VisibilityTransferCommand::RequestMesh { key, source });
	}

	/// Requests one baked image for a non-material consumer such as an IES light profile.
	pub(crate) fn request_image(&self, id: String) {
		self.send(VisibilityTransferCommand::RequestImage {
			key: VisibilityTextureKey::new(id),
		});
	}

	/// Requests the baked lighting subresources stored with one environment image.
	pub(crate) fn request_environment(&self, id: String) {
		self.send(VisibilityTransferCommand::RequestEnvironment { id });
	}

	/// Configures material pipeline creation on the asynchronous resource task.
	pub(crate) fn configure_material_pipeline(&self, config: MaterialPipelineConfig) {
		self.send(VisibilityTransferCommand::ConfigureMaterialPipeline(config));
	}

	/// Drains completed resource work without blocking the render thread.
	pub(crate) fn drain_completions(&mut self) -> CompletionList {
		let mut completions = CompletionList::new();
		while let Ok(completion) = self.completions.try_recv() {
			completions.push(completion);
		}
		completions.extend(self.upload_completions.drain(..));
		completions
	}

	/// Enqueues a texture upload and reports the descriptor data once the transfer frame completes.
	pub(crate) fn enqueue_texture_upload(
		&mut self,
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	) {
		let upload = PreparedUpload::Texture {
			key,
			index,
			image,
			sampler,
			upload,
			photometry,
		};
		self.pending_uploads.push_back(upload);
	}

	/// Enqueues every image in one environment as one transfer-frame completion.
	pub(crate) fn enqueue_environment_upload(&mut self, upload: PendingEnvironmentUpload) {
		self.pending_uploads.push_back(PreparedUpload::Environment(upload));
	}
}

impl VisibilityPipelineResourceManagerClient {
	/// Records one fully prepared mesh without resource I/O or dependency waits.
	fn record_resource_mesh(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		mesh: &PreparedGpuMesh,
		material_indices: Vec<u32>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		if !Self::resource_mesh_metadata_is_valid(mesh, &material_indices, &primitive_skins, skin_bindings.len()) {
			return Err(());
		}
		let mesh = self
			.gpu_vertex_data_manager
			.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, staging_data_buffer, mesh)
			.ok_or(())?;
		Ok(Self::convert_resource_mesh_data(
			mesh,
			material_indices,
			primitive_skins,
			skin_bindings,
			skeleton_node_count,
		))
	}

	/// Rejects render metadata before transfer recording can consume GPU capacity.
	pub(crate) fn resource_mesh_metadata_is_valid(
		mesh: &PreparedGpuMesh,
		material_indices: &[u32],
		primitive_skins: &[Option<u32>],
		skin_binding_count: usize,
	) -> bool {
		let expected = mesh.render_primitive_count();
		if material_indices.len() != expected || primitive_skins.len() != expected {
			log::error!(
				"Visibility mesh primitive count changed before transfer. The most likely cause is inconsistent mesh metadata."
			);
			return false;
		}

		if let Some(skin_index) = primitive_skins
			.iter()
			.flatten()
			.find(|skin_index| **skin_index as usize >= skin_binding_count)
		{
			log::error!(
				"Visibility mesh skin index is invalid before transfer: {}. The most likely cause is that mesh validation was bypassed or the resource data is corrupted.",
				skin_index
			);
			return false;
		}

		true
	}

	/// Combines uploaded resource geometry with dependency slots reserved during metadata discovery.
	fn convert_resource_mesh_data(
		mesh: GpuMeshData,
		material_indices: Vec<u32>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	) -> crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
		let primitives = material_indices
			.into_iter()
			.zip(primitive_skins)
			.zip(mesh.primitives.iter())
			.map(|((material_index, skin_index), primitive)| {
				let skin = skin_index.map(|skin_index| {
					skin_bindings
						.get(skin_index as usize)
						.expect("Visibility skin indices were validated before transfer recording.")
						.clone()
				});

				crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
					skinning_source_vertex_offset: primitive.skinning_source_vertex_offset,
					skinning_vertex_count: primitive.skinning_vertex_count,
					skin,
				}
			})
			.collect();

		crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			skeleton_node_count,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		}
	}

	/// Maps generated mesh geometry to render-facing metadata using its prepared material slot.
	fn convert_generated_mesh_data(
		mesh: GpuMeshData,
		material_index: u32,
	) -> crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
		let primitives = mesh
			.primitives
			.iter()
			.map(
				|primitive| crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
					skinning_source_vertex_offset: primitive.skinning_source_vertex_offset,
					skinning_vertex_count: primitive.skinning_vertex_count,
					skin: None,
				},
			)
			.collect();

		crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			skeleton_node_count: 0,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		}
	}
}

impl VisibilityPipelineResourceManagerWorker {
	/// Handles resource requests and CPU preparation until the command channel closes.
	pub(crate) async fn run(mut self) {
		while let Ok(command) = self.commands.recv().await {
			if !self.handle_command(command) {
				break;
			}
			self.drain_ready_commands(255);
		}
	}

	/// Adopts a bounded set of ready commands without waiting for more preparation work.
	fn drain_ready_commands(&mut self, max_commands: usize) {
		let mut count = 0usize;
		while count < max_commands {
			match self.commands.try_recv() {
				Ok(Some(command)) => {
					count += 1;
					if !self.handle_command(command) {
						return;
					}
				}
				Ok(None) => break,
				Err(_) => return,
			}
		}
	}

	/// Moves one request or preparation completion into worker-owned state without waiting.
	fn handle_command(&mut self, command: VisibilityTransferCommand) -> bool {
		match command {
			VisibilityTransferCommand::RequestMesh { key, source } => {
				self.resource_manager.request_mesh_preparation(key, source);
			}
			VisibilityTransferCommand::ResourceMeshLoaded { key, resource } => {
				self.resource_manager.prepare_loaded_resource_mesh(key, resource);
			}
			VisibilityTransferCommand::GeneratedMeshLoaded { key, generator } => {
				self.resource_manager.prepare_loaded_generated_mesh(key, generator);
			}
			VisibilityTransferCommand::UploadPrepared(upload) => {
				if self.prepared_uploads.send(upload).is_err() {
					return false;
				}
			}
			VisibilityTransferCommand::MaterialPrepared {
				id,
				index,
				alpha_mode,
				coverage,
				texture_keys,
				pipeline,
			} => {
				self.resource_manager
					.adopt_prepared_material(id, index, alpha_mode, coverage, texture_keys, pipeline);
			}

			VisibilityTransferCommand::TexturePrepared { texture } => {
				self.resource_manager.adopt_prepared_texture(texture);
			}
			VisibilityTransferCommand::TextureResourceLoaded { key, index, resource } => {
				self.resource_manager.adopt_loaded_gpu_texture(key, index, resource);
			}
			VisibilityTransferCommand::RequestImage { key } => {
				self.resource_manager.request_image(key);
			}
			VisibilityTransferCommand::RequestEnvironment { id } => {
				self.resource_manager.request_environment_preparation(id);
			}
			VisibilityTransferCommand::EnvironmentPrepared { environment } => {
				self.resource_manager.adopt_prepared_environment(environment);
			}
			VisibilityTransferCommand::ConfigureMaterialPipeline(config) => {
				self.resource_manager.configure_material_pipeline(config);
			}
			VisibilityTransferCommand::PreparationFailed { key } => {
				self.resource_manager
					.send_completion(VisibilityResourceCompletion::Failed { key });
			}
			VisibilityTransferCommand::Shutdown => return false,
		}

		true
	}
}

impl VisibilityPipelineResourceManagerClient {
	/// Releases completed upload leases and adopts preparation results without blocking the render thread.
	pub(crate) fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool {
		if let Some(completed_frame) = completed_frame {
			while self
				.submitted_uploads
				.front()
				.is_some_and(|batch| batch.frame_key == completed_frame)
			{
				let batch = self
					.submitted_uploads
					.pop_front()
					.expect("The completed visibility upload batch was checked before removal.");
				self.upload_completions.extend(batch.completions);
				// Dropping the batch after GPU completion returns every staging lease.
			}
		}
		while let Ok(upload) = self.prepared_uploads.try_recv() {
			self.pending_uploads.push_back(upload);
		}
		!self.pending_uploads.is_empty()
	}

	/// Records every ready upload and retains its staging lease until this graphics frame completes.
	pub(crate) fn record_frame_uploads(
		&mut self,
		frame_key: ghi::FrameKey,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
	) {
		let result = self.record_uploads(transfer, self.staging_data_buffer);
		if !result.completions.is_empty() {
			self.submitted_uploads.push_back(SubmittedUploadBatch {
				frame_key,
				completions: result.completions,
				_leases: result.leases,
			});
		}
	}

	/// Records every ready lease and retains it until the submitted transfer frame completes.
	fn record_uploads(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> TransferUploadPrepareResult {
		let mut completions = CompletionList::new();
		let mut leases = SmallVec::new();
		while let Some(upload) = self.pending_uploads.pop_front() {
			match upload {
				PreparedUpload::ResourceMesh {
					key,
					mesh: prepared_mesh,
					material_indices,
					primitive_skins,
					skin_bindings,
					skeleton_node_count,
				} => {
					match self.record_resource_mesh(
						transfer,
						staging_data_buffer,
						&prepared_mesh,
						material_indices,
						primitive_skins,
						skin_bindings,
						skeleton_node_count,
					) {
						Ok(mesh) => {
							let meshlet_count = mesh.primitives.iter().map(|primitive| primitive.meshlet_count).sum::<u32>();
							log::debug!(
								"Visibility mesh created: key={}, source={}, primitives={}, meshlets={}, vertex_offset={}, primitive_offset={}, triangle_offset={}, meshlet_offset={}",
								key,
								"resource",
								mesh.primitives.len(),
								meshlet_count,
								mesh.vertex_offset,
								mesh.primitive_offset,
								mesh.triangle_offset,
								mesh.meshlet_offset,
							);
							completions.push(VisibilityResourceCompletion::MeshReady { key, mesh });
							leases.push(prepared_mesh.into_staging());
						}
						Err(()) => {
							self.upload_completions.push(VisibilityResourceCompletion::Failed {
								key: VisibilityResourceKey::Mesh(key),
							});
						}
					}
				}
				PreparedUpload::GeneratedMesh {
					key,
					mesh: prepared_mesh,
					material_index,
				} => {
					let result = self
						.gpu_vertex_data_manager
						.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, staging_data_buffer, &prepared_mesh)
						.map(|mesh| Self::convert_generated_mesh_data(mesh, material_index));
					match result {
						Some(mesh) => {
							completions.push(VisibilityResourceCompletion::MeshReady { key, mesh });
							leases.push(prepared_mesh.into_staging());
						}
						None => {
							self.upload_completions.push(VisibilityResourceCompletion::Failed {
								key: VisibilityResourceKey::Mesh(key),
							});
						}
					}
				}
				PreparedUpload::Texture {
					key,
					index,
					image,
					sampler,
					upload,
					photometry,
				} => {
					let copies = upload
						.layouts
						.iter()
						.enumerate()
						.map(|(level, layout)| {
							staged_texture_copy(staging_data_buffer, upload.staging.offset(), image, layout, level as u32)
						})
						.collect::<SmallVec<[ghi::BufferImageCopyDescriptor; 16]>>();
					transfer.copy_buffer_to_images(&copies);
					completions.push(VisibilityResourceCompletion::TextureUploadReady {
						key,
						index,
						image,
						sampler,
						photometry,
					});
					leases.push(upload.staging);
				}
				PreparedUpload::Environment(upload) => {
					let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
					copies.push(staged_texture_copy(
						staging_data_buffer,
						upload.staging.offset(),
						upload.diffuse_image,
						&upload.diffuse_upload,
						0,
					));
					for (mip_level, mip) in upload.specular_uploads.iter().enumerate() {
						copies.push(staged_texture_copy(
							staging_data_buffer,
							upload.staging.offset(),
							upload.specular_image,
							mip,
							mip_level as u32,
						));
					}
					transfer.copy_buffer_to_images(&copies);
					completions.push(VisibilityResourceCompletion::EnvironmentUploadReady {
						id: upload.id,
						diffuse_image: upload.diffuse_image,
						specular_image: upload.specular_image,
						sampler: upload.sampler,
					});
					leases.push(upload.staging);
				}
			}
		}

		TransferUploadPrepareResult { completions, leases }
	}
}
