use super::super::*;

impl Device {
	/// Binds native DX12 vertex buffer views for raster input assembly.
	pub(crate) fn bind_vertex_buffers_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_descriptors: &[BufferDescriptor],
		sequence_index: u8,
	) {
		let command_list = self.command_list(command_buffer_handle);

		let mut views = SmallVec::<[D3D12_VERTEX_BUFFER_VIEW; 8]>::new();
		for buffer_descriptor in buffer_descriptors {
			let Some(resource) = self.buffer_resource_for_sequence(buffer_descriptor.buffer, sequence_index) else {
				continue;
			};
			let buffer = self.buffer(buffer_descriptor.buffer);
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
			self.transition_tracked_buffer(&command_list, &resource, BufferBarrierState::VERTEX_BUFFER);
			views.push(D3D12_VERTEX_BUFFER_VIEW {
				BufferLocation: buffer_location,
				SizeInBytes: size_in_bytes,
				StrideInBytes: 0,
			});
		}

		if views.is_empty() {
			return;
		}
		// The bound raster pipeline gives the strides; a pipeline bound later fills them in.
		let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
		for (slot, view) in views.iter_mut().enumerate() {
			view.StrideInBytes = command_buffer.vertex_strides.get(slot).copied().unwrap_or(0);
		}
		command_buffer.vertex_buffer_views = views.iter().copied().collect();

		unsafe {
			command_list.IASetVertexBuffers(0, Some(&views));
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Binds a native DX12 index buffer view for raster input assembly.
	pub(crate) fn bind_index_buffer_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_descriptor: &BufferDescriptor,
		sequence_index: u8,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let Some(resource) = self.buffer_resource_for_sequence(buffer_descriptor.buffer, sequence_index) else {
			return;
		};
		let buffer = self.buffer(buffer_descriptor.buffer);
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
			self.transition_tracked_buffer(&command_list, &resource, BufferBarrierState::INDEX_BUFFER);
			command_list.IASetIndexBuffer(Some(&view));
		}
		self.mark_command_buffer_work(command_buffer_handle);
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
		let command_list = self.command_list(command_buffer_handle);
		unsafe {
			command_list.DrawInstanced(vertex_count, instance_count, first_vertex, first_instance);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Encodes a native DX12 non-indexed draw whose counts come from one record of a GPU buffer.
	pub(crate) fn draw_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		base_buffer_handle: BaseBufferHandle,
		entry_index: usize,
		sequence_index: u8,
	) {
		self.execute_indirect_native(
			command_buffer_handle,
			base_buffer_handle,
			sequence_index,
			D3D12_INDIRECT_ARGUMENT_TYPE_DRAW,
			crate::command_buffer::indirect_entry_range::<[u32; 4], N>(entry_index),
		);
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
		let command_list = self.command_list(command_buffer_handle);
		unsafe {
			command_list.DrawIndexedInstanced(index_count, instance_count, first_index, vertex_offset, first_instance);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Encodes a native DX12 mesh shader dispatch when a mesh pipeline is bound.
	pub(crate) fn dispatch_meshes_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
		x: u32,
		y: u32,
		z: u32,
	) {
		if !self.pipelines[pipeline_handle.index() as usize].has_mesh_shader {
			return;
		}
		// ID3D12GraphicsCommandList7 extends ID3D12GraphicsCommandList6, which declares DispatchMesh.
		let command_list = self.command_list(command_buffer_handle);
		unsafe {
			command_list.DispatchMesh(x, y, z);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Encodes a native DX12 mesh shader dispatch whose workgroup counts come from one record of a GPU buffer, when
	/// a mesh pipeline is bound.
	pub(crate) fn dispatch_meshes_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
		base_buffer_handle: BaseBufferHandle,
		entry_index: usize,
		sequence_index: u8,
	) {
		if !self.pipelines[pipeline_handle.index() as usize].has_mesh_shader {
			return;
		}
		self.execute_indirect_native(
			command_buffer_handle,
			base_buffer_handle,
			sequence_index,
			D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH_MESH,
			crate::command_buffer::indirect_entry_range::<[u32; 3], N>(entry_index),
		);
	}

	/// Binds a stored mesh and encodes a native DX12 indexed draw command.
	pub(crate) fn draw_mesh_native(&mut self, command_buffer_handle: CommandBufferHandle, mesh_handle: MeshHandle) {
		let command_list = self.command_list(command_buffer_handle);
		let Some(mesh) = self.meshes.get(mesh_handle.index() as usize) else {
			return;
		};
		let (Some(vertex_resource), Some(index_resource)) = (mesh.vertex_resource.clone(), mesh.index_resource.clone()) else {
			return;
		};
		let vertex_view = D3D12_VERTEX_BUFFER_VIEW {
			BufferLocation: unsafe { vertex_resource.GetGPUVirtualAddress() },
			SizeInBytes: mesh.vertex_bytes,
			StrideInBytes: mesh.vertex_size.min(u32::MAX as usize) as u32,
		};
		let index_view = D3D12_INDEX_BUFFER_VIEW {
			BufferLocation: unsafe { index_resource.GetGPUVirtualAddress() },
			SizeInBytes: mesh.index_bytes,
			Format: DXGI_FORMAT_R16_UINT,
		};
		unsafe {
			command_list.IASetVertexBuffers(0, Some(&[vertex_view]));
			command_list.IASetIndexBuffer(Some(&index_view));
			command_list.DrawIndexedInstanced(mesh.index_count, 1, 0, 0, 0);
		}
		// The mesh's view replaced the bound ones, so a later pipeline bind has none to restride.
		self.command_buffers[command_buffer_handle.index() as usize]
			.vertex_buffer_views
			.clear();
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Returns a stable RTV, or DSV for a depth format, over some layers of one native resource, and keeps its heap
	/// alive with the recording.
	pub(crate) fn retained_attachment_view(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
		first_layer: u32,
		layer_count: u32,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		let view = self.materialize_attachment_views(resource, format, array_layers);
		// Mapping the selection to its slot also validates the requested layers.
		let slot = Self::attachment_descriptor_slot(array_layers, first_layer, layer_count);
		let handle = self.descriptor_cpu_handle(&view, Self::attachment_heap_type(format), slot);
		self.retain_descriptor_heap(command_buffer_handle, &view);
		handle
	}

	/// Returns the descriptor heap type of an attachment's views: DSVs for a depth format, RTVs otherwise.
	fn attachment_heap_type(format: Formats) -> windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE {
		if format.is_depth() {
			D3D12_DESCRIPTOR_HEAP_TYPE_DSV
		} else {
			D3D12_DESCRIPTOR_HEAP_TYPE_RTV
		}
	}

	/// Returns the heap holding every attachment view of one image, one for each layer range, creating them on first
	/// use.
	pub(crate) fn materialize_attachment_views(
		&mut self,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
	) -> DescriptorHeap {
		let native_format = Self::dxgi_format(format).expect(
			"Unsupported DX12 attachment format. The most likely cause is that the attachment uses a format without a native RTV or DSV mapping.",
		);
		let key = AttachmentViewKey {
			resource: Self::native_resource_key(resource),
			format: native_format.0,
		};
		if let Some(view) = self.attachment_views.get(&key) {
			return view.clone();
		}

		let heap_type = Self::attachment_heap_type(format);
		let descriptor_count = Self::attachment_descriptor_count(array_layers);
		let heap = self.create_cpu_descriptor_heap(heap_type, descriptor_count);
		for slot in 0..descriptor_count {
			let (first_layer, layer_count) = Self::attachment_descriptor_layers(array_layers, slot);
			let handle = self.descriptor_cpu_handle(&heap, heap_type, slot);
			unsafe {
				if format.is_depth() {
					let descriptor = Self::depth_stencil_view_desc(format, array_layers, first_layer, layer_count);
					self.device.CreateDepthStencilView(resource, Some(&descriptor), handle);
				} else {
					let descriptor = Self::render_target_view_desc(format, array_layers, first_layer, layer_count);
					self.device.CreateRenderTargetView(resource, Some(&descriptor), handle);
				}
			}
		}
		self.attachment_views.insert(key, heap.clone());
		heap
	}

	/// Creates the attachment views of a newly created image resource that render targets or depth targets use.
	pub(crate) fn materialize_image_attachment_views(
		&mut self,
		resource: &ID3D12Resource,
		format: Formats,
		uses: Uses,
		array_layers: u32,
	) {
		if uses.intersects(Uses::RenderTarget | Uses::DepthStencil) {
			self.materialize_attachment_views(resource, format, array_layers);
		}
	}

	/// Returns the value an attachment is cleared to when its pass starts, or `None` when it loads or discards.
	fn attachment_clear(attachment: &AttachmentInformation) -> Option<ClearValue> {
		match attachment.load {
			crate::LoadOp::Clear(clear) => Some(clear),
			crate::LoadOp::Load | crate::LoadOp::Discard => None,
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
		let command_list = self.command_list(command_buffer_handle);
		// A pass that clears or discards an attachment gives an image-group member new contents.
		for attachment in attachments {
			if let (crate::ImageOrSwapchain::Image(image), false) = (attachment.target, attachment.loads()) {
				if self.image_groups.member(image).is_some() {
					self.initialize_image_contents(image, sequence_index);
				}
			}
		}

		let mut targets = SmallVec::<[RenderTargetAttachment; 8]>::new();
		for attachment in attachments {
			let format = self.attachment_format(attachment);
			let Some((image_handle, resource)) =
				self.attachment_render_target_resource(command_buffer_handle, attachment, sequence_index)
			else {
				continue;
			};
			let array_layers = image_handle.map_or(1, |image_handle| self.images[image_handle.index() as usize].array_layers);
			let (first_layer, layer_count) = attachment.layer_selection();
			targets.push(RenderTargetAttachment {
				image_handle,
				resource,
				format,
				array_layers,
				first_layer,
				layer_count,
				clear: Self::attachment_clear(attachment),
			});
		}
		if targets.is_empty() {
			return;
		}
		// Integer images clear through a UAV; every other color target clears as a float render target.
		let integer_clear = |target: &RenderTargetAttachment| {
			target
				.image_handle
				.filter(|_| matches!(target.clear, Some(ClearValue::Integer(..))) && target.format == Formats::U32)
		};

		// Plan attachment transitions before recording any clears so independent attachments share one native
		// Barrier call. Integer render targets transition through UAV for their clear.
		let mut attachment_barriers = EnhancedBarrierBatch::default();
		for target in &targets {
			let state = if target.format.is_depth() {
				TextureBarrierState::DEPTH_WRITE
			} else if integer_clear(target).is_some() {
				TextureBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW)
			} else {
				TextureBarrierState::RENDER_TARGET
			};
			match target.image_handle {
				Some(image_handle) => {
					self.transition_tracked_image_into(image_handle, &target.resource, state, true, &mut attachment_barriers);
				}
				None => {
					self.transition_swapchain_texture_into(&target.resource, state, &mut attachment_barriers);
				}
			}
		}
		attachment_barriers.submit(&command_list);

		let mut handles = SmallVec::<[D3D12_CPU_DESCRIPTOR_HANDLE; 8]>::new();
		let mut depth_handle = None;
		let mut post_clear_barriers = EnhancedBarrierBatch::default();
		for target in &targets {
			let handle = self.retained_attachment_view(
				command_buffer_handle,
				&target.resource,
				target.format,
				target.array_layers,
				target.first_layer,
				target.layer_count,
			);
			match (target.clear, integer_clear(target)) {
				(Some(clear), _) if target.format.is_depth() => unsafe {
					command_list.ClearDepthStencilView(handle, D3D12_CLEAR_FLAG_DEPTH, Self::clear_depth_value(clear), 0, None);
				},
				(Some(clear), Some(image_handle)) => {
					self.record_image_clear(command_buffer_handle, image_handle, clear, sequence_index, false);
					self.transition_tracked_image_into(
						image_handle,
						&target.resource,
						TextureBarrierState::RENDER_TARGET,
						true,
						&mut post_clear_barriers,
					);
				}
				(Some(clear), None) => unsafe {
					command_list.ClearRenderTargetView(handle, &Self::clear_color_f32(clear), None);
				},
				(None, _) => {}
			}
			if target.format.is_depth() {
				depth_handle = Some(handle);
			} else {
				handles.push(handle);
			}
		}
		post_clear_barriers.submit(&command_list);

		unsafe {
			command_list.OMSetRenderTargets(
				handles.len() as u32,
				(!handles.is_empty()).then_some(handles.as_ptr()),
				false,
				depth_handle.as_ref().map(|handle| handle as *const D3D12_CPU_DESCRIPTOR_HANDLE),
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	pub(crate) fn end_render_pass_native(&mut self, command_buffer_handle: CommandBufferHandle) {
		let command_list = self.command_list(command_buffer_handle);

		unsafe {
			command_list.OMSetRenderTargets(0, None, false, None);
		}
	}

	/// Sets native DX12 viewport and scissor state for a render pass.
	pub(crate) fn set_render_area_native(&mut self, command_buffer_handle: CommandBufferHandle, extent: Extent) {
		let command_list = self.command_list(command_buffer_handle);

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
	}

	/// Sets the native DX12 scissor rectangle inside an active render pass.
	pub(crate) fn set_scissor_native(&mut self, command_buffer_handle: CommandBufferHandle, origin: [u32; 2], extent: Extent) {
		let command_list = self.command_list(command_buffer_handle);

		let scissor = RECT {
			left: origin[0] as i32,
			top: origin[1] as i32,
			right: (origin[0] + extent.width()) as i32,
			bottom: (origin[1] + extent.height()) as i32,
		};

		unsafe {
			command_list.RSSetScissorRects(&[scissor]);
		}
	}
}
