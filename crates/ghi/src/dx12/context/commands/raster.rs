use super::super::*;

impl Device {
	/// Binds native DX12 vertex buffer views for raster input assembly.
	pub(crate) fn bind_vertex_buffers_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_descriptors: &[BufferDescriptor],
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		let mut views = SmallVec::<[D3D12_VERTEX_BUFFER_VIEW; 8]>::new();
		for buffer_descriptor in buffer_descriptors {
			let Some(resource) = self.buffer_resource_for_sequence(buffer_descriptor.buffer, sequence_index) else {
				continue;
			};
			let Some(buffer) = self.buffer(buffer_descriptor.buffer) else {
				continue;
			};
			assert!(
				buffer_descriptor.offset <= buffer.size,
				"DX12 vertex buffer offset exceeds the buffer. The most likely cause is that BufferDescriptor::offset was built from stale mesh metadata. offset={}, buffer_size={}",
				buffer_descriptor.offset,
				buffer.size,
			);
			let remaining = buffer.size - buffer_descriptor.offset;
			let size_in_bytes = u32::try_from(remaining).expect(
				"DX12 vertex buffer view exceeds four GiB. The most likely cause is that one buffer view spans more bytes than D3D12_VERTEX_BUFFER_VIEW can represent.",
			);
			let buffer_location = unsafe { resource.GetGPUVirtualAddress() }
				.checked_add(buffer_descriptor.offset as u64)
				.expect(
					"DX12 vertex buffer address overflowed. The most likely cause is an invalid native resource address or offset.",
				);
			self.transition_tracked_buffer(
				&command_list,
				buffer_descriptor.buffer,
				&resource,
				BufferBarrierState::VERTEX_BUFFER,
			);
			views.push(D3D12_VERTEX_BUFFER_VIEW {
				BufferLocation: buffer_location,
				SizeInBytes: size_in_bytes,
				StrideInBytes: 0,
			});
		}

		if views.is_empty() {
			return;
		}

		unsafe {
			command_list.IASetVertexBuffers(0, Some(&views));
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.vertex_buffer_bind_count += 1;
	}

	/// Binds a native DX12 index buffer view for raster input assembly.
	pub(crate) fn bind_index_buffer_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_descriptor: &BufferDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(resource) = self.buffer_resource_for_sequence(buffer_descriptor.buffer, sequence_index) else {
			return;
		};
		let Some(buffer) = self.buffer(buffer_descriptor.buffer) else {
			return;
		};
		let (format, index_element_size) = match buffer_descriptor.index_type {
			Some(DataTypes::U16) => (DXGI_FORMAT_R16_UINT, std::mem::size_of::<u16>()),
			Some(DataTypes::U32) => (DXGI_FORMAT_R32_UINT, std::mem::size_of::<u32>()),
			Some(_) => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
			None => panic!(
				"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32)."
			),
		};
		assert!(
			buffer_descriptor.offset <= buffer.size,
			"DX12 index buffer offset exceeds the buffer. The most likely cause is that BufferDescriptor::offset was built from stale mesh metadata. offset={}, buffer_size={}",
			buffer_descriptor.offset,
			buffer.size,
		);
		assert!(
			buffer_descriptor.offset.is_multiple_of(index_element_size),
			"DX12 index buffer offset is misaligned. The most likely cause is that BufferDescriptor::offset is not a multiple of the selected index element size.",
		);
		let remaining = buffer.size - buffer_descriptor.offset;
		let size_in_bytes = u32::try_from(remaining).expect(
			"DX12 index buffer view exceeds four GiB. The most likely cause is that one buffer view spans more bytes than D3D12_INDEX_BUFFER_VIEW can represent.",
		);
		let buffer_location = unsafe { resource.GetGPUVirtualAddress() }
			.checked_add(buffer_descriptor.offset as u64)
			.expect(
				"DX12 index buffer address overflowed. The most likely cause is an invalid native resource address or offset.",
			);
		assert!(
			buffer_location.is_multiple_of(index_element_size as u64),
			"DX12 index buffer address is misaligned. The most likely cause is an invalid native buffer allocation or index offset.",
		);
		let view = D3D12_INDEX_BUFFER_VIEW {
			BufferLocation: buffer_location,
			SizeInBytes: size_in_bytes,
			Format: format,
		};

		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				buffer_descriptor.buffer,
				&resource,
				BufferBarrierState::INDEX_BUFFER,
			);
			command_list.IASetIndexBuffer(Some(&view));
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.index_buffer_bind_count += 1;
	}

	/// Encodes a native DX12 non-indexed draw command.
	pub(crate) fn draw_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		vertex_count: u32,
		instance_count: u32,
		first_vertex: u32,
		first_instance: u32,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		unsafe {
			command_list.DrawInstanced(vertex_count, instance_count, first_vertex, first_instance);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.draw_encode_count += 1;
	}

	/// Encodes a native DX12 indexed draw command.
	pub(crate) fn draw_indexed_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		unsafe {
			command_list.DrawIndexedInstanced(index_count, instance_count, first_index, vertex_offset, first_instance);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.draw_indexed_encode_count += 1;
	}

	/// Encodes a native DX12 mesh shader dispatch when a mesh pipeline is bound.
	pub(crate) fn dispatch_meshes_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		x: u32,
		y: u32,
		z: u32,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::Raster) || pipeline.pipeline_state.is_none() || !pipeline.has_mesh_shader {
			return;
		}
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
			.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList6>().ok())
		else {
			return;
		};

		unsafe {
			command_list.DispatchMesh(x, y, z);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.mesh_dispatch_encode_count += 1;
	}

	/// Binds a stored mesh and encodes a native DX12 indexed draw command.
	pub(crate) fn draw_mesh_native(&mut self, command_buffer_handle: CommandBufferHandle, mesh_handle: MeshHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(mesh) = self.meshes.get(mesh_handle.0 as usize) else {
			return;
		};
		let (Some(vertex_resource), Some(index_resource)) = (mesh.vertex_resource.clone(), mesh.index_resource.clone()) else {
			return;
		};
		let vertex_view = D3D12_VERTEX_BUFFER_VIEW {
			BufferLocation: unsafe { vertex_resource.GetGPUVirtualAddress() },
			SizeInBytes: mesh.vertices.len().min(u32::MAX as usize) as u32,
			StrideInBytes: mesh.vertex_size.min(u32::MAX as usize) as u32,
		};
		let index_view = D3D12_INDEX_BUFFER_VIEW {
			BufferLocation: unsafe { index_resource.GetGPUVirtualAddress() },
			SizeInBytes: mesh.indices.len().min(u32::MAX as usize) as u32,
			Format: DXGI_FORMAT_R16_UINT,
		};
		unsafe {
			command_list.IASetVertexBuffers(0, Some(&[vertex_view]));
			command_list.IASetIndexBuffer(Some(&index_view));
			command_list.DrawIndexedInstanced(mesh.index_count, 1, 0, 0, 0);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.vertex_buffer_bind_count += 1;
		self.index_buffer_bind_count += 1;
		self.draw_indexed_encode_count += 1;
	}

	/// Returns a stable RTV descriptor for one native resource view, creating it on first use.
	pub(crate) fn retained_render_target_view(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
		layer_count: u32,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		self.materialize_render_target_views(resource, format, array_layers);
		Self::validate_attachment_layers(array_layers, layer, layer_count);
		let key = AttachmentViewKey {
			resource: Self::native_resource_key(resource),
			format: Self::dxgi_format(format)
				.expect(
					"Unsupported DX12 render-target format. The most likely cause is that the attachment uses a format without a native RTV mapping.",
				)
				.0,
		};
		let view = self
			.render_target_views
			.get(&key)
			.expect(
				"Missing retained DX12 render-target view. The most likely cause is that attachment view creation did not populate its cache.",
			)
			.heap
			.clone();
		let slot = Self::attachment_descriptor_slot(array_layers, layer, layer_count);
		let handle = self.descriptor_cpu_handle(&view, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, slot);
		self.retain_descriptor_heap(command_buffer_handle, &view);
		handle
	}

	/// Returns a stable DSV descriptor for one native resource view, creating it on first use.
	pub(crate) fn retained_depth_stencil_view(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
		layer_count: u32,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		self.materialize_depth_stencil_views(resource, format, array_layers);
		Self::validate_attachment_layers(array_layers, layer, layer_count);
		let key = AttachmentViewKey {
			resource: Self::native_resource_key(resource),
			format: Self::dxgi_format(format)
				.expect(
					"Unsupported DX12 depth-stencil format. The most likely cause is that the attachment uses a format without a native DSV mapping.",
				)
				.0,
		};
		let view = self
			.depth_stencil_views
			.get(&key)
			.expect(
				"Missing retained DX12 depth-stencil view. The most likely cause is that attachment view creation did not populate its cache.",
			)
			.heap
			.clone();
		let slot = Self::attachment_descriptor_slot(array_layers, layer, layer_count);
		let handle = self.descriptor_cpu_handle(&view, D3D12_DESCRIPTOR_HEAP_TYPE_DSV, slot);
		self.retain_descriptor_heap(command_buffer_handle, &view);
		handle
	}

	/// Materializes every RTV descriptor for one image in a single retained heap.
	pub(crate) fn materialize_render_target_views(&mut self, resource: &ID3D12Resource, format: Formats, array_layers: u32) {
		let native_format = Self::dxgi_format(format).expect(
			"Unsupported DX12 render-target format. The most likely cause is that the attachment uses a format without a native RTV mapping.",
		);
		let key = AttachmentViewKey {
			resource: Self::native_resource_key(resource),
			format: native_format.0,
		};
		if self.render_target_views.contains_key(&key) {
			return;
		}

		let descriptor_count = Self::attachment_descriptor_count(array_layers);
		let heap =
			self.create_attachment_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_RTV, descriptor_count, "render-target view");
		for slot in 0..descriptor_count {
			let (layer, layer_count) = Self::attachment_descriptor_layers(array_layers, slot);
			let descriptor = Self::render_target_view_desc(format, array_layers, layer, layer_count);
			let handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, slot);
			unsafe {
				self.device.CreateRenderTargetView(resource, Some(&descriptor), handle);
			}
		}
		self.render_target_views.insert(key, CpuDescriptorView { heap });
		self.render_target_view_allocation_count += 1;
	}

	/// Materializes every DSV descriptor for one image in a single retained heap.
	pub(crate) fn materialize_depth_stencil_views(&mut self, resource: &ID3D12Resource, format: Formats, array_layers: u32) {
		let native_format = Self::dxgi_format(format).expect(
			"Unsupported DX12 depth-stencil format. The most likely cause is that the attachment uses a format without a native DSV mapping.",
		);
		let key = AttachmentViewKey {
			resource: Self::native_resource_key(resource),
			format: native_format.0,
		};
		if self.depth_stencil_views.contains_key(&key) {
			return;
		}

		let descriptor_count = Self::attachment_descriptor_count(array_layers);
		let heap =
			self.create_attachment_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_DSV, descriptor_count, "depth-stencil view");
		for slot in 0..descriptor_count {
			let (layer, layer_count) = Self::attachment_descriptor_layers(array_layers, slot);
			let descriptor = Self::depth_stencil_view_desc(format, array_layers, layer, layer_count);
			let handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_DSV, slot);
			unsafe {
				self.device.CreateDepthStencilView(resource, Some(&descriptor), handle);
			}
		}
		self.depth_stencil_views.insert(key, CpuDescriptorView { heap });
		self.depth_stencil_view_allocation_count += 1;
	}

	/// Materializes attachment descriptors alongside a newly created image resource.
	pub(crate) fn materialize_image_attachment_views(
		&mut self,
		resource: &ID3D12Resource,
		format: Formats,
		uses: Uses,
		array_layers: u32,
	) {
		if uses.intersects(Uses::RenderTarget) {
			self.materialize_render_target_views(resource, format, array_layers);
		}
		if uses.intersects(Uses::DepthStencil) {
			self.materialize_depth_stencil_views(resource, format, array_layers);
		}
	}

	/// Creates one CPU-only descriptor heap for a retained attachment view.
	pub(crate) fn create_attachment_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
		purpose: &str,
	) -> DescriptorHeap {
		let descriptor = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: Default::default(),
			NodeMask: 0,
		};
		match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&descriptor) } {
			Ok(native) => DescriptorHeap {
				cpu_start: unsafe { native.GetCPUDescriptorHandleForHeapStart() },
				gpu_start: None,
				native,
			},
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				let message = format!(
					"Failed to create a DX12 {purpose} descriptor heap: {error:?}. The most likely cause is descriptor heap resource exhaustion or device removal. Descriptor count: {descriptor_count}. Device removed reason: {removed_reason:?}"
				);
				self.log_dx12_error(&message);
				panic!("{message}");
			}
		}
	}

	/// Binds native DX12 render target views for color attachments in a render pass.
	pub(crate) fn bind_render_targets_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		attachments: &[AttachmentInformation],
		sequence_index: u8,
	) {
		AttachmentInformation::render_pass_layer_count(attachments);
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		let mut target_resources = SmallVec::<[RenderTargetAttachment; 8]>::new();
		let mut depth_resource = None;
		for attachment in attachments {
			let format = self.attachment_format(attachment);
			if format.is_depth() {
				let image_handle = self.attachment_image_handle(attachment, sequence_index);
				let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
					continue;
				};
				let Some(image) = self.images.get(image_handle.0 as usize) else {
					continue;
				};
				let layer_count = attachment.layer_count.map_or(1, std::num::NonZeroU32::get);
				Self::validate_attachment_layers(image.array_layers, attachment.layer, layer_count);
				depth_resource = Some((
					image_handle,
					resource,
					image.format,
					image.array_layers,
					attachment.layer,
					layer_count,
					attachment.load,
					attachment.clear,
				));
				continue;
			}
			let Some((image_handle, resource, swapchain_backbuffer)) =
				self.attachment_render_target_resource(command_buffer_handle, attachment, sequence_index)
			else {
				continue;
			};
			let array_layers = image_handle
				.and_then(|image_handle| self.images.get(image_handle.0 as usize))
				.map(|image| image.array_layers)
				.unwrap_or(1);
			let layer_count = attachment.layer_count.map_or(1, std::num::NonZeroU32::get);
			Self::validate_attachment_layers(array_layers, attachment.layer, layer_count);
			target_resources.push(RenderTargetAttachment {
				image_handle,
				resource,
				format,
				array_layers,
				layer: attachment.layer,
				layer_count,
				load: attachment.load,
				clear: attachment.clear,
				swapchain_backbuffer,
			});
		}

		if target_resources.is_empty() && depth_resource.is_none() {
			return;
		}

		// Plan attachment transitions before recording any clears so independent attachments share
		// one native Barrier call. Integer render targets transition through UAV in their clear.
		let mut attachment_barriers = EnhancedBarrierBatch::default();
		for target in &target_resources {
			let state = if !target.load && matches!(target.clear, ClearValue::Integer(..)) && target.format == Formats::U32 {
				TextureBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW)
			} else {
				TextureBarrierState::RENDER_TARGET
			};
			if let Some(image_handle) = target.image_handle {
				self.transition_tracked_image_into(image_handle, &target.resource, state, &mut attachment_barriers);
			} else {
				self.transition_swapchain_texture_into(
					&target.resource,
					TextureBarrierState::RENDER_TARGET,
					&mut attachment_barriers,
				);
			}
		}
		if let Some((image_handle, resource, ..)) = &depth_resource {
			self.transition_tracked_image_into(
				*image_handle,
				resource,
				TextureBarrierState::DEPTH_WRITE,
				&mut attachment_barriers,
			);
		}
		Self::submit_resource_barriers(&command_list, &attachment_barriers);

		let mut handles = SmallVec::<[D3D12_CPU_DESCRIPTOR_HANDLE; 8]>::new();
		let mut integer_clear_targets = SmallVec::<[(crate::BaseImageHandle, ID3D12Resource); 8]>::new();
		if !target_resources.is_empty() {
			for target in target_resources {
				let RenderTargetAttachment {
					image_handle,
					resource,
					format,
					array_layers,
					layer,
					layer_count,
					load,
					clear,
					swapchain_backbuffer,
				} = target;
				let handle = self.retained_render_target_view(
					command_buffer_handle,
					&resource,
					format,
					array_layers,
					layer,
					layer_count,
				);
				if swapchain_backbuffer {
					self.swapchain_backbuffer_bind_count += 1;
				}
				if !load {
					if matches!(clear, ClearValue::Integer(..)) && format == Formats::U32 {
						if let Some(image_handle) = image_handle {
							self.record_image_clear_with_final_state(
								command_buffer_handle,
								crate::ImageHandle(image_handle),
								clear,
								sequence_index,
								None,
								false,
							);
							integer_clear_targets.push((image_handle, resource.clone()));
						} else {
							let color = Self::clear_color_f32(clear);
							unsafe {
								command_list.ClearRenderTargetView(handle, &color, None);
							}
						}
					} else {
						let color = Self::clear_color_f32(clear);
						unsafe {
							command_list.ClearRenderTargetView(handle, &color, None);
						}
					}
					self.mark_command_buffer_work(command_buffer_handle);
					self.render_target_clear_count += 1;
				}
				handles.push(handle);
			}

			self.render_target_bind_count += 1;
		}

		let mut post_clear_barriers = EnhancedBarrierBatch::default();
		for (image_handle, resource) in integer_clear_targets {
			self.transition_tracked_image_into(
				image_handle,
				&resource,
				TextureBarrierState::RENDER_TARGET,
				&mut post_clear_barriers,
			);
		}
		Self::submit_resource_barriers(&command_list, &post_clear_barriers);

		let mut depth_handle = None;
		if let Some((_, resource, format, array_layers, layer, layer_count, load, clear)) = depth_resource {
			let handle =
				self.retained_depth_stencil_view(command_buffer_handle, &resource, format, array_layers, layer, layer_count);
			if !load {
				let depth = Self::clear_depth_value(clear);
				unsafe {
					command_list.ClearDepthStencilView(handle, D3D12_CLEAR_FLAG_DEPTH, depth, 0, None);
				}
				self.mark_command_buffer_work(command_buffer_handle);
				self.depth_stencil_clear_count += 1;
			}
			depth_handle = Some(handle);
			self.depth_stencil_bind_count += 1;
		}

		let depth_handle_pointer = depth_handle
			.as_ref()
			.map(|handle| handle as *const D3D12_CPU_DESCRIPTOR_HANDLE);
		unsafe {
			command_list.OMSetRenderTargets(
				handles.len() as u32,
				(!handles.is_empty()).then_some(handles.as_ptr()),
				false,
				depth_handle_pointer,
			);
		}
		if !handles.is_empty() || depth_handle.is_some() {
			self.mark_command_buffer_work(command_buffer_handle);
		}
	}

	pub(crate) fn end_render_pass_native(&mut self, command_buffer_handle: CommandBufferHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		unsafe {
			command_list.OMSetRenderTargets(0, None, false, None);
		}
		self.render_pass_end_count += 1;
	}

	/// Sets native DX12 viewport and scissor state for a render pass.
	pub(crate) fn set_render_area_native(&mut self, command_buffer_handle: CommandBufferHandle, extent: Extent) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		let viewport = D3D12_VIEWPORT {
			TopLeftX: 0.0,
			TopLeftY: 0.0,
			Width: extent.width() as f32,
			Height: extent.height() as f32,
			MinDepth: 0.0,
			MaxDepth: 1.0,
		};
		let scissor = RECT {
			left: 0,
			top: 0,
			right: extent.width() as i32,
			bottom: extent.height() as i32,
		};

		unsafe {
			command_list.RSSetViewports(&[viewport]);
			command_list.RSSetScissorRects(&[scissor]);
		}
		self.viewport_set_count += 1;
		self.scissor_set_count += 1;
	}
}
