use super::*;

/// The `VisibilityPipelineResourceManagerClient` struct connects render logic to the asynchronous visibility resource worker.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	pub(crate) gpu_vertex_data_manager: GPUVertexDataManager,
	pub(super) commands: kanal::Sender<VisibilityTransferCommand>,
	pub(super) completions: Receiver<VisibilityResourceCompletion>,
}

/// The `VisibilityPipelineResourceManagerWorker` struct owns visibility resource loading and GPU transfer.
pub(crate) struct VisibilityPipelineResourceManagerWorker {
	pub(super) resource_manager: VisibilityPipelineResourceManager,
	pub(super) gpu_vertex_data_manager: GPUVertexDataManager,
	pub(super) commands: kanal::AsyncReceiver<VisibilityTransferCommand>,
	pub(super) completions: Sender<VisibilityResourceCompletion>,
	pub(super) pending_uploads: VecDeque<PreparedUpload>,
	pub(super) submitted_uploads: VecDeque<SubmittedUploadBatch>,
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
		completions
	}

	/// Enqueues a texture upload and reports the descriptor data once the transfer frame completes.
	pub(crate) fn enqueue_texture_upload(
		&self,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
	) {
		self.send(VisibilityTransferCommand::UploadPrepared(PreparedUpload::Texture {
			index,
			image,
			sampler,
			upload,
		}));
	}

	/// Enqueues every image in one environment as one transfer-frame completion.
	pub(crate) fn enqueue_environment_upload(&self, upload: PendingEnvironmentUpload) {
		self.send(VisibilityTransferCommand::UploadPrepared(PreparedUpload::Environment(upload)));
	}
}

impl VisibilityPipelineResourceManagerWorker {
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
				let skin = match skin_index {
					Some(skin_index) => Some(
						skin_bindings
							.get(skin_index as usize)
							.expect("Visibility skin indices were validated before transfer recording.")
							.clone(),
					),
					None => None,
				};

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

	/// Handles resource requests and transfer completion until the command channel closes.
	pub(crate) async fn run(
		mut self,
		mut transfer_queue: ghi::implementation::queue::Queue,
		transfer_finished_synchronizer: ghi::SynchronizerHandle,
		transfer_command_buffer: ghi::CommandBufferHandle,
		upload_buffer: ghi::BufferHandle<[u8; ASYNC_UPLOAD_BUFFER_BYTE_COUNT]>,
	) {
		let mut started_frame_count = 0;

		loop {
			// Observe every ready preparation before opening the next transfer frame so
			// unrelated resources share the earliest batch that has room for them.
			let Some(drained_command_count) = self.drain_ready_commands(256) else {
				break;
			};

			if self.has_active_transfer_work() {
				if self
					.advance_transfer_queue(
						&mut transfer_queue,
						transfer_finished_synchronizer,
						transfer_command_buffer,
						upload_buffer,
						&mut started_frame_count,
					)
					.is_none()
				{
					break;
				}
			}

			if drained_command_count > 0 {
				crate::core::async_runtime::yield_now().await;
			} else if self.has_active_transfer_work() {
				// Submitted GPU work needs periodic queue progress even when no new
				// resource has finished CPU preparation.
				compio::time::sleep(ACTIVE_TRANSFER_POLL_INTERVAL).await;
			} else {
				let Ok(command) = self.commands.recv().await else {
					break;
				};
				if !self.handle_command(command) {
					break;
				}
			}
		}
	}

	/// Advances one transfer frame and records all upload work already prepared by resource commands.
	fn advance_transfer_queue(
		&mut self,
		transfer_queue: &mut ghi::implementation::queue::Queue,
		transfer_finished_synchronizer: ghi::SynchronizerHandle,
		transfer_command_buffer: ghi::CommandBufferHandle,
		upload_buffer: ghi::BufferHandle<[u8; ASYNC_UPLOAD_BUFFER_BYTE_COUNT]>,
		started_frame_count: &mut u64,
	) -> Option<()> {
		let started_frame = transfer_queue.start_frame(*started_frame_count, transfer_finished_synchronizer);
		if let Some(completed_frame) = started_frame.completed_frame {
			self.signal_completed_frame(completed_frame);
		}

		// Frame acquisition can wait for an in-flight sequence. Adopt resources that
		// became ready during that wait before deciding what belongs in this batch.
		if self.drain_ready_commands(256).is_none() {
			return None;
		}

		if !self.has_pending_upload_work() {
			*started_frame_count += 1;
			return Some(());
		}

		let mut frame = started_frame.frame;
		let frame_key = frame.key();
		let mut transfer_recording = frame.create_command_buffer_recording_without_implicit_sync(transfer_command_buffer);
		let prepared_uploads = self.prepare_uploads(&mut transfer_recording, upload_buffer.into());

		if prepared_uploads.recorded_work {
			transfer_recording.execute(transfer_finished_synchronizer);
		} else {
			drop(transfer_recording);
		}

		self.track_submitted_uploads(frame_key, prepared_uploads.completions, prepared_uploads.leases);
		*started_frame_count += 1;
		Some(())
	}

	/// Adopts a bounded set of ready commands without waiting for more preparation work.
	fn drain_ready_commands(&mut self, max_commands: usize) -> Option<usize> {
		let mut count = 0usize;
		while count < max_commands {
			match self.commands.try_recv() {
				Ok(Some(command)) => {
					count += 1;
					if !self.handle_command(command) {
						return None;
					}
				}
				Ok(None) => break,
				Err(_) => return None,
			}
		}

		Some(count)
	}

	/// Publishes upload completions for transfer frames reported as complete by the queue.
	pub(crate) fn signal_completed_frame(&mut self, completed_frame: ghi::FrameKey) {
		while self
			.submitted_uploads
			.front()
			.is_some_and(|batch| batch.frame_key == completed_frame)
		{
			let Some(batch) = self.submitted_uploads.pop_front() else {
				break;
			};

			for completion in batch.completions {
				if self.completions.send(completion).is_err() {
					log::error!(
						"Visibility upload completion failed. The most likely cause is that the render thread stopped receiving worker results."
					);
				}
			}
		}
	}

	/// Tracks resources handled by a submitted transfer frame.
	pub(crate) fn track_submitted_uploads(
		&mut self,
		frame_key: ghi::FrameKey,
		completions: CompletionList,
		leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
	) {
		if completions.is_empty() {
			return;
		}

		self.submitted_uploads.push_back(SubmittedUploadBatch {
			frame_key,
			completions,
			_leases: leases,
		});
	}

	/// Records every currently fitting upload into the transfer command buffer.
	fn prepare_uploads(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> TransferUploadPrepareResult {
		self.record_uploads(transfer, staging_data_buffer)
	}

	/// Reports whether upload queues contain work that needs GPU transfer recording.
	fn has_pending_upload_work(&self) -> bool {
		!self.pending_uploads.is_empty()
	}

	/// Reports whether the queue must keep advancing submitted or pending transfers.
	fn has_active_transfer_work(&self) -> bool {
		self.has_pending_upload_work() || !self.submitted_uploads.is_empty()
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
				self.pending_uploads.push_back(upload);
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

	/// Records every ready lease and retains it until the submitted transfer frame completes.
	fn record_uploads(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> TransferUploadPrepareResult {
		let mut recorded_work = false;
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
							recorded_work = true;
						}
						Err(()) => self.resource_manager.send_completion(VisibilityResourceCompletion::Failed {
							key: VisibilityResourceKey::Mesh(key),
						}),
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
							recorded_work = true;
						}
						None => self.resource_manager.send_completion(VisibilityResourceCompletion::Failed {
							key: VisibilityResourceKey::Mesh(key),
						}),
					}
				}
				PreparedUpload::Texture {
					index,
					image,
					sampler,
					upload,
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
					completions.push(VisibilityResourceCompletion::TextureUploadReady { index, image, sampler });
					leases.push(upload.staging);
					recorded_work = true;
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
					recorded_work = true;
				}
			}
		}

		TransferUploadPrepareResult {
			recorded_work,
			completions,
			leases,
		}
	}
}
