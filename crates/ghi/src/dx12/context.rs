/// The `Device` struct exists to own DX12 GPU resources for the shared GHI device API.
pub struct Device {
	device: ID3D12Device,
	settings: Features,
	native_16_bit_shader_ops_supported: bool,
	info_queue: Option<ID3D12InfoQueue>,
	debug_log_function: fn(&str),
	debug_log_count: AtomicU64,
	debugger: RenderDebugger,
	pub(crate) frames: u8,

	queues: Vec<StoredQueue>,
	command_buffers: Vec<CommandBuffer>,
	buffers: Vec<Buffer>,
	dynamic_buffers: Vec<Buffer>,
	images: Vec<Image>,
	samplers: Vec<Sampler>,
	descriptor_sets: Vec<DescriptorSet>,
	descriptor_materializations: HashMap<DescriptorMaterializationKey, DescriptorMaterialization>,
	pipeline_layouts: Vec<PipelineLayout>,
	pipeline_root_signatures: Vec<Option<ID3D12RootSignature>>,
	pipeline_root_tables: Vec<Vec<RootDescriptorTable>>,
	pipeline_root_constants: Vec<Vec<RootConstantRange>>,
	pipeline_layout_indices: HashMap<PipelineLayout, PipelineLayoutHandle>,
	pub(crate) pipelines: Vec<Pipeline>,
	indirect_dispatch_signature: Option<ID3D12CommandSignature>,
	shaders: Vec<Shader>,
	meshes: Vec<Mesh>,
	pub(crate) swapchains: Vec<Swapchain>,
	synchronizers: Vec<Synchronizer>,
	top_level_acceleration_structures: Vec<AccelerationStructure>,
	bottom_level_acceleration_structures: Vec<AccelerationStructure>,
	texture_copies: Vec<Vec<u8>>,
	allocations: Vec<Allocation>,
	texture_readbacks: Vec<TextureReadback>,
	gpu_uploaded_images: HashSet<crate::BaseImageHandle>,
	pending_texture_syncs: Vec<(crate::BaseImageHandle, u8)>,
	present_transitions: HashMap<CommandBufferHandle, Vec<ID3D12Resource>>,
	render_target_views: HashMap<AttachmentViewKey, CpuDescriptorView>,
	depth_stencil_views: HashMap<AttachmentViewKey, CpuDescriptorView>,
	buffer_states: HashMap<usize, D3D12_RESOURCE_STATES>,
	image_states: HashMap<usize, D3D12_RESOURCE_STATES>,
	render_target_view_allocation_count: usize,
	depth_stencil_view_allocation_count: usize,
	texture_copy_count: usize,
	buffer_copy_count: usize,
	buffer_clear_count: usize,
	native_command_list_execute_count: usize,
	empty_command_list_skip_count: usize,
	root_signature_bind_count: usize,
	descriptor_heap_bind_count: usize,
	descriptor_table_bind_count: usize,
	#[cfg(test)]
	descriptor_table_bind_records: Vec<DescriptorTableBindRecord>,
	push_constant_write_count: usize,
	#[cfg(test)]
	push_constant_write_records: Vec<PushConstantWriteRecord>,
	descriptor_write_count: usize,
	image_srv_descriptor_write_count: usize,
	image_uav_descriptor_write_count: usize,
	acceleration_structure_descriptor_write_count: usize,
	#[cfg(test)]
	sampler_descriptor_write_records: Vec<SamplerDescriptorWriteRecord>,
	pipeline_state_bind_count: usize,
	compute_pipeline_state_create_attempt_count: usize,
	graphics_pipeline_state_create_attempt_count: usize,
	graphics_pipeline_state_last_error: Option<i32>,
	hlsl_specialization_compile_count: usize,
	ray_tracing_state_object_create_attempt_count: usize,
	compute_dispatch_encode_count: usize,
	indirect_dispatch_encode_count: usize,
	trace_rays_record_count: usize,
	mesh_dispatch_encode_count: usize,
	vertex_buffer_bind_count: usize,
	index_buffer_bind_count: usize,
	draw_encode_count: usize,
	draw_indexed_encode_count: usize,
	render_target_bind_count: usize,
	render_target_clear_count: usize,
	render_pass_end_count: usize,
	depth_stencil_bind_count: usize,
	depth_stencil_clear_count: usize,
	viewport_set_count: usize,
	scissor_set_count: usize,
	primitive_topology_set_count: usize,
	swapchain_backbuffer_bind_count: usize,
	swapchain_present_transition_count: usize,
	uav_barrier_count: usize,
	acceleration_structure_resource_count: usize,
	native_acceleration_structure_resource_count: usize,
	acceleration_structure_instance_write_count: usize,
	shader_binding_table_write_count: usize,
	top_level_acceleration_structure_build_record_count: usize,
	bottom_level_acceleration_structure_build_record_count: usize,
	native_top_level_acceleration_structure_build_encode_count: usize,
	native_bottom_level_acceleration_structure_build_encode_count: usize,
	texture_readback_resolve_count: usize,
	debug_region_begin_count: Cell<usize>,
	debug_region_end_count: Cell<usize>,
}

impl Device {
	const NATIVE_16_BIT_SHADER_OPS_UNAVAILABLE: &str = "DX12 native 16-bit shader types are unavailable. The most likely cause is a GPU or driver that does not report Native16BitShaderOpsSupported.";

	/// Creates a DX12 device and initializes command queues for the requested queue types.
	pub fn new(settings: Features, queues: &mut [(QueueSelection, &mut Option<QueueHandle>)]) -> Result<Self, &'static str> {
		let adapter: Option<&IUnknown> = None;
		let mut device: Option<ID3D12Device> = None;
		unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_12_0, &mut device) }
			.or_else(|_| unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_11_0, &mut device) })
			.map_err(|_| "Failed to create a D3D12 device. The most likely cause is that the GPU or driver does not support the required feature level.")?;
		let device = device.ok_or(
			"Failed to acquire a D3D12 device. The most likely cause is that the D3D12CreateDevice call returned no device instance.",
		)?;
		let info_queue = if settings.validation {
			device.cast::<ID3D12InfoQueue>().ok()
		} else {
			None
		};
		let debug_log_function = settings.debug_log_function.unwrap_or(|message| {
			println!("{}", message);
		});

		let mut queue_storage = Vec::with_capacity(queues.len());

		for (selection, handle) in queues.iter_mut() {
			let queue_type = select_d3d12_command_list_type(selection.r#type)?;

			let desc = D3D12_COMMAND_QUEUE_DESC {
				Type: queue_type,
				Priority: 0,
				Flags: D3D12_COMMAND_QUEUE_FLAGS(0),
				NodeMask: 0,
			};

			let queue = unsafe { device.CreateCommandQueue(&desc) }
				.map_err(|_| "Failed to create a D3D12 command queue. The most likely cause is that the device does not support the requested queue type.")?;

			let index = queue_storage.len() as u64;
			queue_storage.push(StoredQueue { queue, queue_type });
			**handle = Some(QueueHandle(index));
		}

		Ok(Self::from_native_parts(
			device,
			settings,
			info_queue,
			debug_log_function,
			queue_storage,
		))
	}

	/// Creates an empty DX12 context over an already-selected native device and queues.
	fn from_native_parts(
		device: ID3D12Device,
		settings: Features,
		info_queue: Option<ID3D12InfoQueue>,
		debug_log_function: fn(&str),
		queues: Vec<StoredQueue>,
	) -> Self {
		let native_16_bit_shader_ops_supported = Self::query_native_16_bit_shader_ops_support(&device);
		Self {
			device,
			settings,
			native_16_bit_shader_ops_supported,
			info_queue,
			debug_log_function,
			debug_log_count: AtomicU64::new(0),
			debugger: RenderDebugger::new(),
			frames: 2,

			queues,
			command_buffers: Vec::new(),
			buffers: Vec::new(),
			dynamic_buffers: Vec::new(),
			images: Vec::new(),
			samplers: Vec::new(),
			descriptor_sets: Vec::new(),
			descriptor_materializations: HashMap::default(),
			pipeline_layouts: Vec::new(),
			pipeline_root_signatures: Vec::new(),
			pipeline_root_tables: Vec::new(),
			pipeline_root_constants: Vec::new(),
			pipeline_layout_indices: HashMap::default(),
			pipelines: Vec::new(),
			indirect_dispatch_signature: None,
			shaders: Vec::new(),
			meshes: Vec::new(),
			swapchains: Vec::new(),
			synchronizers: Vec::new(),
			top_level_acceleration_structures: Vec::new(),
			bottom_level_acceleration_structures: Vec::new(),
			texture_copies: Vec::new(),
			allocations: Vec::new(),
			texture_readbacks: Vec::new(),
			gpu_uploaded_images: HashSet::default(),
			pending_texture_syncs: Vec::new(),
			present_transitions: HashMap::default(),
			render_target_views: HashMap::default(),
			depth_stencil_views: HashMap::default(),
			buffer_states: HashMap::default(),
			image_states: HashMap::default(),
			render_target_view_allocation_count: 0,
			depth_stencil_view_allocation_count: 0,
			texture_copy_count: 0,
			buffer_copy_count: 0,
			buffer_clear_count: 0,
			native_command_list_execute_count: 0,
			empty_command_list_skip_count: 0,
			root_signature_bind_count: 0,
			descriptor_heap_bind_count: 0,
			descriptor_table_bind_count: 0,
			#[cfg(test)]
			descriptor_table_bind_records: Vec::new(),
			push_constant_write_count: 0,
			#[cfg(test)]
			push_constant_write_records: Vec::new(),
			descriptor_write_count: 0,
			image_srv_descriptor_write_count: 0,
			image_uav_descriptor_write_count: 0,
			acceleration_structure_descriptor_write_count: 0,
			#[cfg(test)]
			sampler_descriptor_write_records: Vec::new(),
			pipeline_state_bind_count: 0,
			compute_pipeline_state_create_attempt_count: 0,
			graphics_pipeline_state_create_attempt_count: 0,
			graphics_pipeline_state_last_error: None,
			hlsl_specialization_compile_count: 0,
			ray_tracing_state_object_create_attempt_count: 0,
			compute_dispatch_encode_count: 0,
			indirect_dispatch_encode_count: 0,
			trace_rays_record_count: 0,
			mesh_dispatch_encode_count: 0,
			vertex_buffer_bind_count: 0,
			index_buffer_bind_count: 0,
			draw_encode_count: 0,
			draw_indexed_encode_count: 0,
			render_target_bind_count: 0,
			render_target_clear_count: 0,
			render_pass_end_count: 0,
			depth_stencil_bind_count: 0,
			depth_stencil_clear_count: 0,
			viewport_set_count: 0,
			scissor_set_count: 0,
			primitive_topology_set_count: 0,
			swapchain_backbuffer_bind_count: 0,
			swapchain_present_transition_count: 0,
			uav_barrier_count: 0,
			acceleration_structure_resource_count: 0,
			native_acceleration_structure_resource_count: 0,
			acceleration_structure_instance_write_count: 0,
			shader_binding_table_write_count: 0,
			top_level_acceleration_structure_build_record_count: 0,
			bottom_level_acceleration_structure_build_record_count: 0,
			native_top_level_acceleration_structure_build_encode_count: 0,
			native_bottom_level_acceleration_structure_build_encode_count: 0,
			texture_readback_resolve_count: 0,
			debug_region_begin_count: Cell::new(0),
			debug_region_end_count: Cell::new(0),
		}
	}

	#[cfg(any(debug_assertions, test))]
	pub fn has_errors(&self) -> bool {
		self.drain_debug_messages();
		self.debug_log_count.load(Ordering::Relaxed) > 0
	}

	fn log_debug_message(&self, message: impl AsRef<str>) {
		(self.debug_log_function)(message.as_ref());
	}

	fn log_dx12_error(&self, message: impl AsRef<str>) {
		self.log_debug_message(message);
		self.debug_log_count.fetch_add(10, Ordering::Relaxed);
		self.drain_debug_messages();
	}

	fn drain_debug_messages(&self) {
		let Some(info_queue) = &self.info_queue else {
			return;
		};

		let count = unsafe { info_queue.GetNumStoredMessages() };
		for index in 0..count {
			let mut message_byte_len = 0;
			if unsafe { info_queue.GetMessage(index, None, &mut message_byte_len) }.is_err() || message_byte_len == 0 {
				continue;
			}

			let mut message_bytes = vec![0u8; message_byte_len];
			let message = message_bytes.as_mut_ptr().cast::<D3D12_MESSAGE>();
			if unsafe { info_queue.GetMessage(index, Some(message), &mut message_byte_len) }.is_err() {
				continue;
			}

			let message = unsafe { &*message };
			let description = if message.pDescription.is_null() || message.DescriptionByteLength == 0 {
				""
			} else {
				let bytes = unsafe {
					std::slice::from_raw_parts(message.pDescription, message.DescriptionByteLength.saturating_sub(1))
				};
				std::str::from_utf8(bytes).unwrap_or("<non-utf8 D3D12 debug message>")
			};
			self.log_debug_message(format!(
				"DX12 {:?} {:?} #{}: {}",
				message.Severity, message.Category, message.ID.0, description
			));
			if matches!(
				message.Severity,
				D3D12_MESSAGE_SEVERITY_CORRUPTION | D3D12_MESSAGE_SEVERITY_ERROR
			) {
				self.debug_log_count.fetch_add(10, Ordering::Relaxed);
			}
		}

		unsafe { info_queue.ClearStoredMessages() };
	}

	#[cfg(test)]
	pub(crate) fn add_debug_message_for_test(&self, message: &str) {
		let Some(info_queue) = &self.info_queue else {
			return;
		};
		let Ok(message) = std::ffi::CString::new(message) else {
			return;
		};
		if unsafe { info_queue.AddApplicationMessage(D3D12_MESSAGE_SEVERITY_ERROR, PCSTR(message.as_ptr().cast())) }.is_ok() {
			self.drain_debug_messages();
		}
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		self.frames = frames.max(1);
		self.pending_texture_syncs
			.retain(|(_, sequence_index)| *sequence_index < self.frames);
		let image_count = self.frames.max(2);
		let resizes_swapchains = self.swapchains.iter().any(|swapchain| {
			swapchain.image_count != image_count && swapchain.extent.width() > 0 && swapchain.extent.height() > 0
		});
		if resizes_swapchains {
			self.invalidate_attachment_views();
		}

		for swapchain in &mut self.swapchains {
			if swapchain.image_count != image_count && swapchain.extent.width() > 0 && swapchain.extent.height() > 0 {
				// DXGI requires every application-owned backbuffer reference to be released before ResizeBuffers.
				swapchain.backbuffers = std::array::from_fn(|_| None);
				let result = unsafe {
					swapchain.swapchain.ResizeBuffers(
						image_count as u32,
						swapchain.extent.width(),
						swapchain.extent.height(),
						DXGI_FORMAT_B8G8R8A8_UNORM,
						DXGI_SWAP_CHAIN_FLAG(0),
					)
				};

				if result.is_err() {
					panic!(
						"Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed."
					);
				}
				swapchain.backbuffers = std::array::from_fn(|_| None);
			}

			swapchain.image_count = image_count;
			swapchain.next_image_index %= image_count;
		}

		let mut retired_image_state_keys = SmallVec::<[usize; 8]>::new();
		for image in &mut self.images {
			let Some(frame_data) = image.frame_data.as_mut() else {
				continue;
			};
			let data = image.data.clone().unwrap_or_default();
			frame_data.resize(self.frames as usize, data);
			if let Some(frame_resources) = image.frame_resources.as_mut() {
				retired_image_state_keys.extend(
					frame_resources
						.iter()
						.skip(self.frames as usize)
						.flatten()
						.map(Self::native_resource_key),
				);
				frame_resources.resize(self.frames as usize, None);
			}
		}
		self.invalidate_attachment_views_for_resources(&retired_image_state_keys);
		for &key in &retired_image_state_keys {
			self.image_states.remove(&key);
		}

		let mut retired_buffer_state_keys = SmallVec::<[usize; 8]>::new();
		for buffer in &mut self.dynamic_buffers {
			if let Some(frame_resources) = buffer.frame_resources.as_mut() {
				retired_buffer_state_keys.extend(
					frame_resources
						.iter()
						.skip(self.frames as usize)
						.flatten()
						.filter_map(|frame| frame.resource.as_ref())
						.map(Self::native_resource_key),
				);
				frame_resources.resize_with(self.frames as usize, || None);
			}
		}
		for key in retired_buffer_state_keys {
			self.buffer_states.remove(&key);
		}
		self.invalidate_descriptor_materializations();
	}

	pub fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: Uses,
		_resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle {
		self.allocations.push(Allocation { data: vec![0u8; size] });
		AllocationHandle((self.allocations.len() - 1) as u64)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle {
		let vertex_size = vertex_layout.iter().map(|element| element.format.size()).sum::<usize>();
		let (vertex_resource, vertex_pointer, _) =
			self.create_buffer_resource(vertices.len(), DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead);
		let (index_resource, index_pointer, _) =
			self.create_buffer_resource(indices.len(), DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead);
		if !vertex_pointer.is_null() {
			unsafe {
				std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_pointer, vertices.len());
			}
		}
		if !index_pointer.is_null() {
			unsafe {
				std::ptr::copy_nonoverlapping(indices.as_ptr(), index_pointer, indices.len());
			}
		}

		self.meshes.push(Mesh {
			vertex_count,
			index_count,
			vertices: vertices.to_vec(),
			indices: indices.to_vec(),
			vertex_size,
			vertex_resource,
			index_resource,
		});
		MeshHandle((self.meshes.len() - 1) as u64)
	}

	pub fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()> {
		let (spirv, dxil, hlsl) = match shader_source_type {
			Sources::SPIRV(bytes) => (Some(bytes.to_vec()), None, None),
			Sources::DXIL(bytes) => (None, Some(bytes.to_vec()), None),
			Sources::HLSL { source, entry_point } => (
				None,
				Some(self.compile_hlsl(name, source, entry_point, stage, &[])?),
				Some(HlslSource {
					name: name.map(str::to_string),
					source: source.to_string(),
					entry_point: entry_point.to_string(),
				}),
			),
			Sources::MTL { .. } | Sources::MTLB { .. } => return Err(()),
		};

		let mut resources = shader_resource_descriptors.into_iter().collect::<Vec<_>>();
		if let Some(hlsl) = hlsl.as_ref() {
			Self::apply_hlsl_structured_buffer_strides(&mut resources, &hlsl.source);
		}

		self.shaders.push(Shader {
			stage,
			spirv,
			dxil,
			hlsl,
			resources,
		});

		// DX12 consumes native bytecode for PSO creation, while SPIR-V is retained as portable metadata.
		Ok(ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn compile_hlsl(
		&self,
		name: Option<&str>,
		source: &str,
		entry_point: &str,
		stage: ShaderTypes,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Result<Vec<u8>, ()> {
		if let Some(target) = Self::dxc_target(stage, Self::hlsl_uses_native_16_bit_types(source)) {
			return self.compile_hlsl_with_dxc(name, source, entry_point, target, specialization_map);
		}
		let target = match stage {
			ShaderTypes::Vertex => "vs_5_0",
			ShaderTypes::Fragment => "ps_5_0",
			ShaderTypes::Compute => "cs_5_0",
			_ => return Err(()),
		};
		let entry_point = std::ffi::CString::new(entry_point).map_err(|_| ())?;
		let target = std::ffi::CString::new(target).map_err(|_| ())?;
		let (macro_names, macro_values) = Self::hlsl_specialization_macro_storage(specialization_map)?;
		let mut macros = macro_names
			.iter()
			.zip(macro_values.iter())
			.map(|(name, value)| D3D_SHADER_MACRO {
				Name: PCSTR(name.as_ptr().cast()),
				Definition: PCSTR(value.as_ptr().cast()),
			})
			.collect::<Vec<_>>();
		if !macros.is_empty() {
			macros.push(D3D_SHADER_MACRO {
				Name: PCSTR::null(),
				Definition: PCSTR::null(),
			});
		}
		let mut shader = None;
		let mut errors = None;
		unsafe {
			D3DCompile(
				source.as_ptr().cast(),
				source.len(),
				PCSTR::null(),
				(!macros.is_empty()).then_some(macros.as_ptr()),
				None::<&ID3DInclude>,
				PCSTR(entry_point.as_ptr().cast()),
				PCSTR(target.as_ptr().cast()),
				0,
				0,
				&mut shader,
				Some(&mut errors),
			)
			.map_err(|error| {
				self.log_hlsl_compile_error(
					source,
					entry_point.to_str().unwrap_or("<invalid-entry-point>"),
					target.to_str().unwrap_or("<invalid-target>"),
					&format!("{error:?}"),
				);
			})?;
		}
		let Some(shader) = shader else {
			self.log_hlsl_compile_error(
				source,
				entry_point.to_str().unwrap_or("<invalid-entry-point>"),
				target.to_str().unwrap_or("<invalid-target>"),
				"D3DCompile returned no shader bytecode.",
			);
			return Err(());
		};
		let bytecode = unsafe { std::slice::from_raw_parts(shader.GetBufferPointer().cast::<u8>(), shader.GetBufferSize()) };
		Ok(bytecode.to_vec())
	}

	/// Selects a DXC profile when the shader stage or native-width source requires DXIL compilation.
	fn dxc_target(stage: ShaderTypes, native_16_bit_types: bool) -> Option<&'static str> {
		match (stage, native_16_bit_types) {
			// Native 16-bit scalar and vector storage requires Shader Model 6.2 or newer.
			(ShaderTypes::Vertex, true) => Some("vs_6_2"),
			(ShaderTypes::Fragment, true) => Some("ps_6_2"),
			(ShaderTypes::Compute, true) => Some("cs_6_2"),
			// HLSL sources can use SM6 resource-object syntax, so DX12 compiles native source through DXC.
			(ShaderTypes::Vertex, false) => Some("vs_6_0"),
			(ShaderTypes::Fragment, false) => Some("ps_6_0"),
			(ShaderTypes::Compute, false) => Some("cs_6_0"),
			(ShaderTypes::Task, _) => Some("as_6_5"),
			(ShaderTypes::Mesh, _) => Some("ms_6_5"),
			(
				ShaderTypes::RayGen
				| ShaderTypes::Miss
				| ShaderTypes::ClosestHit
				| ShaderTypes::AnyHit
				| ShaderTypes::Intersection,
				_,
			) => Some("lib_6_3"),
			_ => None,
		}
	}

	/// Reports whether HLSL source uses an explicit native-width 16-bit scalar or vector type.
	fn hlsl_uses_native_16_bit_types(source: &str) -> bool {
		source
			.split(|character: char| character != '_' && !character.is_ascii_alphanumeric())
			.any(|token| {
				["uint16_t", "int16_t", "float16_t"].iter().any(|&native_type| {
					let Some(suffix) = token.strip_prefix(native_type) else {
						return false;
					};

					// Match only native scalar, vector, or matrix spellings instead of similarly prefixed identifiers.
					matches!(suffix.as_bytes(), [] | [b'1'..=b'4'] | [b'1'..=b'4', b'x', b'1'..=b'4'])
				})
			})
	}

	/// Selects the minimum DXC target that can represent native 16-bit source types.
	pub(crate) fn dxc_target_for_source<'a>(target: &'a str, source: &str) -> &'a str {
		if !Self::hlsl_uses_native_16_bit_types(source) {
			return target;
		}

		// Native 16-bit types require Shader Model 6.2, including explicit DXC recompiles for mesh fragment shaders.
		match target {
			"vs_6_0" | "vs_6_1" => "vs_6_2",
			"ps_6_0" | "ps_6_1" => "ps_6_2",
			"cs_6_0" | "cs_6_1" => "cs_6_2",
			"lib_6_0" | "lib_6_1" => "lib_6_2",
			_ => target,
		}
	}

	/// Returns the user-facing failure when native 16-bit source exceeds the device capability.
	pub(crate) fn native_16_bit_support_error(source: &str, supported: bool) -> Option<&'static str> {
		(Self::hlsl_uses_native_16_bit_types(source) && !supported).then_some(Self::NATIVE_16_BIT_SHADER_OPS_UNAVAILABLE)
	}

	fn compile_hlsl_with_dxc(
		&self,
		name: Option<&str>,
		source: &str,
		entry_point: &str,
		target: &str,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Result<Vec<u8>, ()> {
		let target = Self::dxc_target_for_source(target, source);
		if let Some(error) = Self::native_16_bit_support_error(source, self.native_16_bit_shader_ops_supported) {
			self.log_dx12_error(error);
			return Err(());
		}
		let compiler = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }.map_err(|error| {
			self.log_hlsl_compile_error(
				source,
				entry_point,
				target,
				&format!("Failed to create DXC compiler: {error:?}"),
			);
		})?;
		let source_buffer = DxcBuffer {
			Ptr: source.as_ptr().cast(),
			Size: source.len(),
			Encoding: DXC_CP_UTF8.0,
		};
		let mut argument_storage = Vec::with_capacity(10 + specialization_map.len() * 2);
		let debug_artifacts_enabled = self.hlsl_debug_artifacts_enabled();
		let dxil_cache_path = (!debug_artifacts_enabled)
			.then(|| Self::hlsl_dxil_cache_path(source, entry_point, target, specialization_map))
			.flatten();
		if let Some(cache_path) = &dxil_cache_path {
			if let Ok(bytecode) = std::fs::read(cache_path) {
				return Ok(bytecode);
			}
		}
		if debug_artifacts_enabled {
			let debug_source_path = Self::shader_debug_hlsl_path(name, entry_point, target)
				.map(|path| path.to_string_lossy().into_owned())
				.unwrap_or_else(|| {
					format!(
						"{}.{}.{}.hlsl",
						Self::sanitize_shader_debug_name(name.unwrap_or("shader")),
						Self::sanitize_shader_debug_name(entry_point),
						Self::sanitize_shader_debug_name(target)
					)
				});
			argument_storage.push(Self::wide_argument(&debug_source_path));
		}
		argument_storage.push(Self::wide_argument("-E"));
		argument_storage.push(Self::wide_argument(entry_point));
		argument_storage.push(Self::wide_argument("-T"));
		argument_storage.push(Self::wide_argument(target));
		if Self::hlsl_uses_native_16_bit_types(source) {
			// DXC only exposes native-width 16-bit arithmetic and storage types when this option is explicit.
			argument_storage.push(Self::wide_argument("-enable-16bit-types"));
		}
		if debug_artifacts_enabled {
			argument_storage.push(Self::wide_argument("-Zi"));
			argument_storage.push(Self::wide_argument("-Qembed_debug"));
		}
		let (macro_names, macro_values) = Self::hlsl_specialization_macro_storage(specialization_map)?;
		for (name, value) in macro_names.iter().zip(macro_values.iter()) {
			let name = name.to_str().map_err(|_| ())?;
			let value = value.to_str().map_err(|_| ())?;
			argument_storage.push(Self::wide_argument("-D"));
			argument_storage.push(Self::wide_argument(&format!("{name}={value}")));
		}
		let arguments = argument_storage
			.iter()
			.map(|argument| PCWSTR(argument.as_ptr()))
			.collect::<Vec<_>>();
		let result = unsafe {
			compiler.Compile::<Option<&IDxcIncludeHandler>, IDxcResult>(&source_buffer, Some(arguments.as_slice()), None)
		}
		.map_err(|error| {
			self.log_hlsl_compile_error(source, entry_point, target, &format!("DXC compile call failed: {error:?}"));
		})?;
		let status = unsafe { result.GetStatus() }.map_err(|error| {
			self.log_hlsl_compile_error(source, entry_point, target, &format!("Failed to read DXC status: {error:?}"));
		})?;
		if status.is_err() {
			self.log_hlsl_compile_error(source, entry_point, target, &Self::dxc_error_output(&result));
			return Err(());
		}
		let mut object = None;
		unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_OBJECT, std::ptr::null_mut(), &mut object) }.map_err(|error| {
			self.log_hlsl_compile_error(
				source,
				entry_point,
				target,
				&format!("Failed to read DXC object output: {error:?}"),
			);
		})?;
		let Some(object) = object else {
			self.log_hlsl_compile_error(source, entry_point, target, "DXC returned no object bytecode.");
			return Err(());
		};
		if debug_artifacts_enabled {
			self.write_shader_debug_files(name, entry_point, target, source, &result);
		}
		let bytecode = unsafe { std::slice::from_raw_parts(object.GetBufferPointer().cast::<u8>(), object.GetBufferSize()) };
		let bytecode = bytecode.to_vec();
		if let Some(cache_path) = &dxil_cache_path {
			Self::write_hlsl_dxil_cache(cache_path, bytecode.as_slice());
		}
		Ok(bytecode)
	}

	fn hlsl_debug_artifacts_enabled(&self) -> bool {
		// Shader PDBs are valuable when the DX12 debug layer is active, but they make normal startup pay filesystem and
		// embedded-debug compilation costs for every generated shader.
		self.settings.validation || self.settings.gpu_validation
	}

	fn hlsl_dxil_cache_path(
		source: &str,
		entry_point: &str,
		target: &str,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Option<std::path::PathBuf> {
		let mut hash = Self::fnv64(b"byte-engine-dx12-dxil-cache-v1");
		Self::fnv64_update_text(&mut hash, source);
		Self::fnv64_update_text(&mut hash, entry_point);
		Self::fnv64_update_text(&mut hash, target);
		for entry in specialization_map {
			Self::fnv64_update_text(&mut hash, entry.get_type().as_str());
			Self::fnv64_update(&mut hash, &entry.get_constant_id().to_le_bytes());
			Self::fnv64_update(&mut hash, entry.get_data());
		}

		let mut path = std::env::current_exe().ok()?;
		path.pop();
		path.push("shader-dxil-cache");
		path.push(format!("{hash:016x}.dxil"));
		Some(path)
	}

	fn write_hlsl_dxil_cache(path: &std::path::Path, bytecode: &[u8]) {
		let Some(directory) = path.parent() else {
			return;
		};
		if std::fs::create_dir_all(directory).is_err() {
			return;
		}
		// Best-effort cache writes keep shader compilation correctness independent of filesystem availability.
		let _ = std::fs::write(path, bytecode);
	}

	fn fnv64(bytes: &[u8]) -> u64 {
		let mut hash = 0xcbf29ce484222325;
		Self::fnv64_update(&mut hash, bytes);
		hash
	}

	fn fnv64_update_text(hash: &mut u64, text: &str) {
		Self::fnv64_update(hash, &(text.len() as u64).to_le_bytes());
		Self::fnv64_update(hash, text.as_bytes());
	}

	fn fnv64_update(hash: &mut u64, bytes: &[u8]) {
		for byte in bytes {
			*hash ^= u64::from(*byte);
			*hash = hash.wrapping_mul(0x100000001b3);
		}
	}

	fn write_shader_debug_files(&self, name: Option<&str>, entry_point: &str, target: &str, source: &str, result: &IDxcResult) {
		let Some(hlsl_path) = Self::shader_debug_hlsl_path(name, entry_point, target) else {
			return;
		};
		let Some(directory) = hlsl_path.parent() else {
			return;
		};
		if let Err(error) = std::fs::create_dir_all(directory) {
			self.log_dx12_error(format!(
				"Failed to create DX12 shader debug directory '{}': {error}",
				directory.display()
			));
			return;
		}
		if let Err(error) = std::fs::write(&hlsl_path, source) {
			self.log_dx12_error(format!(
				"Failed to write DX12 shader debug source '{}': {error}",
				hlsl_path.display()
			));
		}

		let mut pdb = None;
		let mut pdb_name = None;
		if unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_PDB, &mut pdb_name, &mut pdb) }.is_err() {
			return;
		}
		let Some(pdb) = pdb else {
			return;
		};
		let pdb_path = hlsl_path.with_extension("pdb");
		let bytes = unsafe { std::slice::from_raw_parts(pdb.GetBufferPointer().cast::<u8>(), pdb.GetBufferSize()) };
		if let Err(error) = std::fs::write(&pdb_path, bytes) {
			self.log_dx12_error(format!("Failed to write DX12 shader PDB '{}': {error}", pdb_path.display()));
		}
	}

	fn shader_debug_hlsl_path(name: Option<&str>, entry_point: &str, target: &str) -> Option<std::path::PathBuf> {
		let mut directory = std::env::current_exe().ok()?;
		directory.pop();
		directory.push("shader-pdbs");
		directory.push(format!(
			"{}.{}.{}.hlsl",
			Self::sanitize_shader_debug_name(name.unwrap_or("shader")),
			Self::sanitize_shader_debug_name(entry_point),
			Self::sanitize_shader_debug_name(target)
		));
		Some(directory)
	}

	fn sanitize_shader_debug_name(name: &str) -> String {
		let sanitized = name
			.chars()
			.map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
			.collect::<String>();
		if sanitized.is_empty() {
			"shader".to_string()
		} else {
			sanitized
		}
	}

	fn dxc_error_output(result: &IDxcResult) -> String {
		let mut errors = None;
		if unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_ERRORS, std::ptr::null_mut(), &mut errors) }.is_err() {
			return "DXC compilation failed and error output could not be read.".to_string();
		}

		let Some(errors) = errors else {
			return "DXC compilation failed with no error output.".to_string();
		};

		let bytes = unsafe { std::slice::from_raw_parts(errors.GetBufferPointer().cast::<u8>(), errors.GetBufferSize()) };
		let message = String::from_utf8_lossy(bytes).trim().to_string();
		if message.is_empty() {
			"DXC compilation failed with empty error output.".to_string()
		} else {
			message
		}
	}

	fn log_hlsl_compile_error(&self, source: &str, entry_point: &str, target: &str, reason: &str) {
		self.log_dx12_error(format!(
			"Failed to compile DX12 HLSL shader. Entry point: {entry_point}. Target: {target}. Reason: {reason}\n--- HLSL source ---\n{source}\n--- End HLSL source ---"
		));
	}

	fn wide_argument(argument: &str) -> Vec<u16> {
		argument.encode_utf16().chain(std::iter::once(0)).collect()
	}

	fn hlsl_specialization_macro_storage(
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Result<(Vec<std::ffi::CString>, Vec<std::ffi::CString>), ()> {
		let mut names = Vec::new();
		let mut values = Vec::new();
		for entry in specialization_map {
			match entry.get_type().as_str() {
				"bool" => Self::push_hlsl_bool_specialization_macro(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
				)?,
				"i32" => Self::push_hlsl_i32_specialization_macro(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
				)?,
				"u32" => Self::push_hlsl_u32_specialization_macro(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
				)?,
				"f32" => Self::push_hlsl_f32_specialization_macro(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
				)?,
				"vec2f" => Self::push_hlsl_specialization_macro_vector(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
					2,
				)?,
				"vec3f" => Self::push_hlsl_specialization_macro_vector(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
					3,
				)?,
				"vec4f" => Self::push_hlsl_specialization_macro_vector(
					&mut names,
					&mut values,
					entry.get_constant_id(),
					entry.get_data(),
					4,
				)?,
				_ => return Err(()),
			}
		}
		Ok((names, values))
	}

	fn push_hlsl_bool_specialization_macro(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		data: &[u8],
	) -> Result<(), ()> {
		if data.len() != 1 {
			return Err(());
		}
		let value = if data[0] == 0 { "false" } else { "true" };
		Self::push_hlsl_specialization_macro_text(names, values, constant_id, value)
	}

	fn push_hlsl_i32_specialization_macro(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		data: &[u8],
	) -> Result<(), ()> {
		if data.len() != 4 {
			return Err(());
		}
		let value = i32::from_ne_bytes(data.try_into().map_err(|_| ())?);
		Self::push_hlsl_specialization_macro_text(names, values, constant_id, &value.to_string())
	}

	fn push_hlsl_u32_specialization_macro(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		data: &[u8],
	) -> Result<(), ()> {
		if data.len() != 4 {
			return Err(());
		}
		let value = u32::from_ne_bytes(data.try_into().map_err(|_| ())?);
		Self::push_hlsl_specialization_macro_text(names, values, constant_id, &format!("{value}u"))
	}

	fn push_hlsl_f32_specialization_macro(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		data: &[u8],
	) -> Result<(), ()> {
		if data.len() != 4 {
			return Err(());
		}
		let value = f32::from_ne_bytes(data.try_into().map_err(|_| ())?);
		Self::push_hlsl_specialization_macro_text(names, values, constant_id, &format!("{value:?}"))
	}

	fn push_hlsl_specialization_macro_text(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		value: &str,
	) -> Result<(), ()> {
		names.push(std::ffi::CString::new(format!("SPEC_CONSTANT_{constant_id}")).map_err(|_| ())?);
		values.push(std::ffi::CString::new(value).map_err(|_| ())?);
		Ok(())
	}

	fn push_hlsl_specialization_macro_vector(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		data: &[u8],
		components: u32,
	) -> Result<(), ()> {
		if data.len() != components as usize * 4 {
			return Err(());
		}
		for component in 0..components {
			let start = component as usize * 4;
			Self::push_hlsl_f32_specialization_macro(names, values, constant_id + component, &data[start..start + 4])?;
		}
		Ok(())
	}

	/// Creates one retained logical descriptor set per in-flight frame.
	pub fn create_descriptor_set(&mut self, _name: Option<&str>) -> DescriptorSetHandle {
		let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let frame_handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(DescriptorSet {
				next: None,
				version: 0,
				descriptors: HashMap::default(),
			});

			if let Some(previous) = previous {
				self.descriptor_sets[previous.0 as usize].next = Some(crate::descriptors::DescriptorSetHandle(frame_handle.0));
			}
			previous = Some(frame_handle);
		}

		handle
	}

	/// Initializes a pipeline-defined descriptor table so sparse arrays have valid native entries.
	fn initialize_descriptor_heap_defaults(
		&self,
		layout: &PipelineLayout,
		sampler_heap: bool,
		heap: &ID3D12DescriptorHeap,
		base_offset: u32,
	) {
		let heap_type = if sampler_heap {
			D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
		} else {
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
		};
		for resource in &layout.resources {
			let offset = if sampler_heap {
				resource.sampler_offset
			} else {
				resource.cbv_srv_uav_offset
			};
			let Some(offset) = offset else {
				continue;
			};
			for array_element in 0..resource.descriptor.count() {
				let cpu_handle = self.descriptor_cpu_handle(heap, heap_type, base_offset + offset + array_element);
				if sampler_heap {
					self.write_default_sampler_descriptor(cpu_handle);
				} else {
					self.write_null_cbv_srv_uav_descriptor(resource.descriptor, cpu_handle);
				}
			}
		}
	}

	/// Writes a null CBV, SRV, or UAV that matches one pipeline resource representation.
	fn write_null_cbv_srv_uav_descriptor(&self, descriptor: ShaderResourceDescriptor, cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE) {
		match descriptor.kind() {
			ResourceKind::UniformBuffer => unsafe {
				self.device.CreateConstantBufferView(None, cpu_handle);
			},
			ResourceKind::StorageBuffer => unsafe {
				if descriptor.access().intersects(crate::AccessPolicies::WRITE) {
					self.device.CreateUnorderedAccessView(
						None::<&ID3D12Resource>,
						None::<&ID3D12Resource>,
						Some(&Self::null_buffer_uav_desc(descriptor.buffer_element_stride())),
						cpu_handle,
					);
				} else {
					self.device.CreateShaderResourceView(
						None::<&ID3D12Resource>,
						Some(&Self::null_buffer_srv_desc(descriptor.buffer_element_stride())),
						cpu_handle,
					);
				}
			},
			ResourceKind::StorageImage => unsafe {
				self.device.CreateUnorderedAccessView(
					None::<&ID3D12Resource>,
					None::<&ID3D12Resource>,
					Some(&Self::null_texture_uav_desc(descriptor.texture_view())),
					cpu_handle,
				);
			},
			ResourceKind::AccelerationStructure => unsafe {
				self.device.CreateShaderResourceView(
					None::<&ID3D12Resource>,
					Some(&Self::null_acceleration_structure_srv_desc()),
					cpu_handle,
				);
			},
			ResourceKind::SampledImage | ResourceKind::CombinedImageSampler | ResourceKind::InputAttachment => unsafe {
				self.device.CreateShaderResourceView(
					None::<&ID3D12Resource>,
					Some(&Self::null_texture_srv_desc(descriptor.texture_view())),
					cpu_handle,
				);
			},
			ResourceKind::Sampler => {}
		}
	}

	/// Writes the default sampler used by unbound sampler slots.
	fn write_default_sampler_descriptor(&self, cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE) {
		let desc = D3D12_SAMPLER_DESC {
			Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
			AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			MipLODBias: 0.0,
			MaxAnisotropy: 1,
			ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
			BorderColor: [0.0, 0.0, 0.0, 0.0],
			MinLOD: 0.0,
			MaxLOD: 0.0,
		};
		unsafe {
			self.device.CreateSampler(&desc, cpu_handle);
		}
	}

	fn null_buffer_uav_desc(stride: u32) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_UAV {
					FirstElement: 0,
					NumElements: 1,
					StructureByteStride: stride.max(1),
					CounterOffsetInBytes: 0,
					Flags: D3D12_BUFFER_UAV_FLAG_NONE,
				},
			},
		}
	}

	fn raw_buffer_clear_uav_desc(size: usize) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: DXGI_FORMAT_R32_TYPELESS,
			ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_UAV {
					FirstElement: 0,
					NumElements: (size / std::mem::size_of::<u32>()).max(1) as u32,
					StructureByteStride: 0,
					CounterOffsetInBytes: 0,
					Flags: D3D12_BUFFER_UAV_FLAG_RAW,
				},
			},
		}
	}

	fn null_buffer_srv_desc(stride: u32) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_SRV {
					FirstElement: 0,
					NumElements: 1,
					StructureByteStride: stride.max(1),
					Flags: D3D12_BUFFER_SRV_FLAG_NONE,
				},
			},
		}
	}

	/// Applies inferred HLSL structured-buffer strides without overriding explicit metadata.
	fn apply_hlsl_structured_buffer_strides(resources: &mut [ShaderResourceDescriptor], hlsl: &str) {
		let strides = Self::hlsl_structured_buffer_strides(hlsl);
		for resource in resources {
			if !matches!(resource.kind(), ResourceKind::UniformBuffer | ResourceKind::StorageBuffer)
				|| resource.buffer_element_stride() != 4
			{
				continue;
			}
			let Some(stride) = strides.get(&(0, resource.slot().index())).copied() else {
				continue;
			};
			if stride != 0 {
				*resource = resource.buffer_stride(stride);
			}
		}
	}

	/// Extracts structured-buffer element strides from HLSL register declarations.
	pub(crate) fn hlsl_structured_buffer_strides(source: &str) -> HashMap<(u32, u32), u32> {
		let struct_sizes = Self::hlsl_struct_sizes(source);
		let mut strides = HashMap::default();
		let bytes = source.as_bytes();
		let mut index = 0;

		while let Some(relative) = source[index..].find("StructuredBuffer<") {
			let start = index + relative;
			let type_start = start + "StructuredBuffer<".len();
			let Some(type_end_relative) = source[type_start..].find('>') else {
				break;
			};
			let type_end = type_start + type_end_relative;
			let element_type = source[type_start..type_end].trim();
			let Some(stride) = Self::hlsl_type_size(element_type, &struct_sizes) else {
				index = type_end + 1;
				continue;
			};

			let Some(register_relative) = source[type_end..].find("register(") else {
				break;
			};
			let register_start = type_end + register_relative + "register(".len();
			let Some(register_end_relative) = source[register_start..].find(')') else {
				break;
			};
			let register_end = register_start + register_end_relative;
			let register = &source[register_start..register_end];
			if let Some((binding, space)) = Self::hlsl_register_binding(register) {
				strides.insert((space, binding), stride);
			}

			index = register_end + usize::from(register_end < bytes.len());
		}

		strides
	}

	/// Computes byte sizes for HLSL struct declarations used as structured-buffer element types.
	fn hlsl_struct_sizes(source: &str) -> HashMap<String, u32> {
		let mut struct_sizes = HashMap::default();
		let mut index = 0;

		while let Some(relative) = source[index..].find("struct ") {
			let struct_start = index + relative + "struct ".len();
			let name_start = Self::skip_hlsl_whitespace(source, struct_start);
			let name_end = Self::hlsl_identifier_end(source, name_start);
			if name_end == name_start {
				index = struct_start;
				continue;
			}

			let name = source[name_start..name_end].to_string();
			let Some(open_relative) = source[name_end..].find('{') else {
				break;
			};
			let body_start = name_end + open_relative + 1;
			let Some(body_end) = Self::matching_hlsl_brace(source, body_start - 1) else {
				break;
			};

			if let Some(size) = Self::hlsl_struct_body_size(&source[body_start..body_end], &struct_sizes) {
				struct_sizes.insert(name, size);
			}
			index = body_end + 1;
		}

		struct_sizes
	}

	/// Computes a structured-buffer struct body size from field declarations.
	fn hlsl_struct_body_size(body: &str, struct_sizes: &HashMap<String, u32>) -> Option<u32> {
		let mut size = 0u32;
		for statement in body.split(';') {
			let statement = statement.trim();
			if statement.is_empty() || statement.contains('(') {
				continue;
			}
			let mut parts = statement.split_whitespace();
			let Some(field_type) = parts.next() else {
				continue;
			};
			let Some(field_name) = parts.next() else {
				continue;
			};
			let array_count = Self::hlsl_array_count(field_name).unwrap_or(1);
			size = size.checked_add(Self::hlsl_type_size(field_type, struct_sizes)?.checked_mul(array_count)?)?;
		}
		Some(size)
	}

	/// Returns the byte size of a scalar, vector, matrix, or previously parsed struct type.
	fn hlsl_type_size(r#type: &str, struct_sizes: &HashMap<String, u32>) -> Option<u32> {
		if let Some(size) = struct_sizes.get(r#type) {
			return Some(*size);
		}

		let (base, suffix) = Self::hlsl_type_base_and_suffix(r#type);
		let scalar_size = match base {
			"bool" | "float" | "int" | "uint" | "uint32_t" | "int32_t" => 4,
			"half" | "float16_t" | "uint16_t" | "int16_t" => 2,
			"double" => 8,
			_ => return None,
		};

		if suffix.is_empty() {
			return Some(scalar_size);
		}

		if let Some((rows, columns)) = suffix.split_once('x') {
			let rows = rows.parse::<u32>().ok()?;
			let columns = columns.parse::<u32>().ok()?;
			return scalar_size.checked_mul(rows)?.checked_mul(columns);
		}

		let lanes = suffix.parse::<u32>().ok()?;
		scalar_size.checked_mul(lanes)
	}

	/// Splits an HLSL scalar/vector/matrix type into its scalar base and numeric suffix.
	fn hlsl_type_base_and_suffix(r#type: &str) -> (&str, &str) {
		for base in ["uint32_t", "int32_t", "float16_t", "uint16_t", "int16_t"] {
			if let Some(suffix) = r#type.strip_prefix(base) {
				return (base, suffix);
			}
		}

		let split = r#type
			.find(|character: char| character.is_ascii_digit())
			.unwrap_or(r#type.len());
		(&r#type[..split], &r#type[split..])
	}

	/// Parses a fixed array count from an HLSL field name.
	fn hlsl_array_count(field_name: &str) -> Option<u32> {
		let open = field_name.find('[')?;
		let close = field_name[open + 1..].find(']')? + open + 1;
		field_name[open + 1..close].trim().parse().ok()
	}

	/// Parses a register declaration into a descriptor binding and set index.
	fn hlsl_register_binding(register: &str) -> Option<(u32, u32)> {
		let mut parts = register.split(',').map(str::trim);
		let binding = parts
			.next()
			.and_then(|register| register.strip_prefix('t').or_else(|| register.strip_prefix('u')))?
			.parse()
			.ok()?;
		let space = parts
			.next()
			.and_then(|space| space.strip_prefix("space"))
			.and_then(|space| space.parse().ok())
			.unwrap_or(0);
		Some((binding, space))
	}

	/// Advances an HLSL source index past ASCII whitespace.
	fn skip_hlsl_whitespace(source: &str, mut index: usize) -> usize {
		while source.as_bytes().get(index).is_some_and(u8::is_ascii_whitespace) {
			index += 1;
		}
		index
	}

	/// Finds the end of an HLSL identifier starting at the provided byte index.
	fn hlsl_identifier_end(source: &str, mut index: usize) -> usize {
		while source
			.as_bytes()
			.get(index)
			.is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
		{
			index += 1;
		}
		index
	}

	/// Finds the matching closing brace for an HLSL block.
	fn matching_hlsl_brace(source: &str, open_brace: usize) -> Option<usize> {
		let mut depth = 0u32;
		for (offset, byte) in source.as_bytes().iter().enumerate().skip(open_brace) {
			match *byte {
				b'{' => depth = depth.saturating_add(1),
				b'}' => {
					depth = depth.checked_sub(1)?;
					if depth == 0 {
						return Some(offset);
					}
				}
				_ => {}
			}
		}
		None
	}

	fn null_texture_uav_desc(texture_view_type: TextureViewTypes) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		match texture_view_type {
			TextureViewTypes::Texture2DArray => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2DARRAY,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
						MipSlice: 0,
						FirstArraySlice: 0,
						ArraySize: 1,
						PlaneSlice: 0,
					},
				},
			},
			TextureViewTypes::Texture3D => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_UAV {
						MipSlice: 0,
						FirstWSlice: 0,
						WSize: 1,
					},
				},
			},
			TextureViewTypes::Texture2D => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				},
			},
		}
	}

	fn texture_uav_desc(format: DXGI_FORMAT, array_layers: u32) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		let array_layers = array_layers.max(1);
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: format,
			ViewDimension: if array_layers > 1 {
				D3D12_UAV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_UAV_DIMENSION_TEXTURE2D
			},
			Anonymous: if array_layers > 1 {
				D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
						MipSlice: 0,
						FirstArraySlice: 0,
						ArraySize: array_layers,
						PlaneSlice: 0,
					},
				}
			} else {
				D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				}
			},
		}
	}

	/// Resolves the native array slice range for a shader image view.
	fn descriptor_array_range(array_layers: u32, layer: Option<u32>) -> (u32, u32) {
		let array_layers = array_layers.max(1);
		if let Some(layer) = layer {
			assert!(
				layer < array_layers,
				"Invalid DX12 image descriptor layer. The most likely cause is that the selected layer exceeds the image array size."
			);
			(layer, 1)
		} else {
			(0, array_layers)
		}
	}

	/// Creates a UAV whose native dimension matches the shader resource declaration.
	fn descriptor_texture_uav_desc(
		format: DXGI_FORMAT,
		texture_view_type: TextureViewTypes,
		array_layers: u32,
		layer: Option<u32>,
	) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		assert!(
			layer.is_none() || texture_view_type == TextureViewTypes::Texture2DArray,
			"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
		);
		if texture_view_type == TextureViewTypes::Texture3D {
			panic!(
				"Unsupported DX12 Texture3D descriptor view. The most likely cause is that the image was allocated by the current 2D-only image path."
			);
		}
		if texture_view_type == TextureViewTypes::Texture2D && layer.is_none() {
			assert!(
				array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata or a selected layer."
			);
			return D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				},
			};
		}

		// DX12 represents a selected array layer as a one-slice Texture2DArray view.
		let (first_array_slice, array_size) = Self::descriptor_array_range(array_layers, layer);
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: format,
			ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2DARRAY,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
					MipSlice: 0,
					FirstArraySlice: first_array_slice,
					ArraySize: array_size,
					PlaneSlice: 0,
				},
			},
		}
	}

	fn null_texture_srv_desc(texture_view_type: TextureViewTypes) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		match texture_view_type {
			TextureViewTypes::Texture2DArray => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						FirstArraySlice: 0,
						ArraySize: 1,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::Texture3D => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE3D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::Texture2D => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
		}
	}

	/// Creates an SRV whose native dimension matches the shader resource declaration.
	fn descriptor_texture_srv_desc(
		format: DXGI_FORMAT,
		texture_view_type: TextureViewTypes,
		array_layers: u32,
		layer: Option<u32>,
	) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		assert!(
			layer.is_none() || texture_view_type == TextureViewTypes::Texture2DArray,
			"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
		);
		if texture_view_type == TextureViewTypes::Texture3D {
			panic!(
				"Unsupported DX12 Texture3D descriptor view. The most likely cause is that the image was allocated by the current 2D-only image path."
			);
		}
		if texture_view_type == TextureViewTypes::Texture2D && layer.is_none() {
			assert!(
				array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata or a selected layer."
			);
			return D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			};
		}

		// DX12 represents a selected array layer as a one-slice Texture2DArray view.
		let (first_array_slice, array_size) = Self::descriptor_array_range(array_layers, layer);
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: format,
			ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
					MostDetailedMip: 0,
					MipLevels: 1,
					FirstArraySlice: first_array_slice,
					ArraySize: array_size,
					PlaneSlice: 0,
					ResourceMinLODClamp: 0.0,
				},
			},
		}
	}

	fn null_acceleration_structure_srv_desc() -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV { Location: 0 },
			},
		}
	}

	fn descriptor_cpu_handle(
		&self,
		heap: &ID3D12DescriptorHeap,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		slot: u32,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		let mut handle = unsafe { heap.GetCPUDescriptorHandleForHeapStart() };
		let stride = unsafe { self.device.GetDescriptorHandleIncrementSize(heap_type) } as usize;
		handle.ptr = handle.ptr.saturating_add(slot as usize * stride);
		handle
	}

	fn descriptor_gpu_handle(
		&self,
		heap: &ID3D12DescriptorHeap,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		slot: u32,
	) -> D3D12_GPU_DESCRIPTOR_HANDLE {
		let mut handle = unsafe { heap.GetGPUDescriptorHandleForHeapStart() };
		let stride = unsafe { self.device.GetDescriptorHandleIncrementSize(heap_type) } as u64;
		handle.ptr = handle.ptr.saturating_add(slot as u64 * stride);
		handle
	}

	/// Creates a shader-visible heap for retained tables or transient GPU descriptor operations.
	fn create_shader_visible_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
	) -> Option<ID3D12DescriptorHeap> {
		let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
			NodeMask: 0,
		};
		match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) } {
			Ok(heap) => Some(heap),
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				let message = format!(
					"Failed to create a shader-visible DX12 descriptor heap. The most likely cause is descriptor heap exhaustion or device removal. Heap type: {:?}. Descriptor count: {descriptor_count}. Error: {error:?}. Device removed reason: {removed_reason:?}",
					heap_type
				);
				self.log_dx12_error(&message);
				panic!("{message}");
			}
		}
	}

	fn create_transient_cpu_descriptor_heap(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
	) -> Option<ID3D12DescriptorHeap> {
		let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: Default::default(),
			NodeMask: 0,
		};
		let heap = match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) } {
			Ok(heap) => heap,
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				let message = format!(
					"Failed to create a transient CPU DX12 descriptor heap: {error:?}. The most likely cause is descriptor heap exhaustion or device removal. Heap type: {:?}. Descriptor count: {descriptor_count}. Device removed reason: {removed_reason:?}",
					heap_type
				);
				self.log_dx12_error(&message);
				panic!("{message}");
			}
		};
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.retained_descriptor_heaps.push(heap.clone());
		}
		Some(heap)
	}

	fn reserve_staged_descriptor_range(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sampler_heap: bool,
		descriptor_count: u32,
	) -> Option<(ID3D12DescriptorHeap, u32)> {
		if descriptor_count == 0 {
			return None;
		}

		let heap_type = if sampler_heap {
			D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
		} else {
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
		};
		let command_buffer_index = command_buffer_handle.0 as usize;
		let (current_capacity, current_used) = {
			let command_buffer = self.command_buffers.get(command_buffer_index)?;
			let arena = if sampler_heap {
				command_buffer.sampler_staging_heap.as_ref()
			} else {
				command_buffer.cbv_srv_uav_staging_heap.as_ref()
			};
			arena.map(|arena| (arena.capacity, arena.used)).unwrap_or((0, 0))
		};
		let required = current_used.saturating_add(descriptor_count);

		if required > current_capacity {
			let capacity = required.max(current_capacity.saturating_mul(2)).max(256);
			let heap = self.create_shader_visible_descriptor_heap(heap_type, capacity)?;
			let command_buffer = self.command_buffers.get_mut(command_buffer_index)?;
			let target_arena = if sampler_heap {
				&mut command_buffer.sampler_staging_heap
			} else {
				&mut command_buffer.cbv_srv_uav_staging_heap
			};
			if let Some(previous) = target_arena.replace(DescriptorHeapArena { heap, capacity, used: 0 }) {
				if previous.used > 0 {
					command_buffer.retained_descriptor_heaps.push(previous.heap);
				}
			}
		}

		let command_buffer = self.command_buffers.get_mut(command_buffer_index)?;
		let arena = if sampler_heap {
			command_buffer.sampler_staging_heap.as_mut()
		} else {
			command_buffer.cbv_srv_uav_staging_heap.as_mut()
		}?;
		let offset = arena.used;
		arena.used = arena.used.saturating_add(descriptor_count);
		Some((arena.heap.clone(), offset))
	}

	/// Binds the command buffer's active staged descriptor heaps after transient descriptor writes.
	fn bind_active_staged_descriptor_heaps(&mut self, command_buffer_handle: CommandBufferHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(command_buffer) = self.command_buffers.get(command_buffer_handle.0 as usize) else {
			return;
		};

		let mut heaps = [None, None];
		let mut heap_count = 0usize;
		if let Some(arena) = command_buffer
			.cbv_srv_uav_staging_heap
			.as_ref()
			.filter(|arena| arena.used > 0)
		{
			heaps[heap_count] = Some(arena.heap.clone());
			heap_count += 1;
		}
		if let Some(arena) = command_buffer.sampler_staging_heap.as_ref().filter(|arena| arena.used > 0) {
			heaps[heap_count] = Some(arena.heap.clone());
			heap_count += 1;
		}
		if heap_count == 0 {
			return;
		}

		unsafe {
			command_list.SetDescriptorHeaps(&heaps[..heap_count]);
		}
		self.descriptor_heap_bind_count += 1;
	}

	/// Returns the immutable shader-visible heaps for one frame-resolved retained set union.
	///
	/// The flat binding model derives native offsets from the pipeline layout, so the first bind creates the heaps.
	/// Later binds reuse them until a retained write changes one of the participating sets.
	fn materialize_descriptor_heaps(
		&mut self,
		layout_handle: PipelineLayoutHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> Option<DescriptorMaterialization> {
		let descriptor_sets = sets
			.iter()
			.map(|&root_set_handle| {
				self.descriptor_set_for_sequence(root_set_handle, sequence_index)
					.unwrap_or(root_set_handle)
			})
			.collect::<SmallVec<[_; 8]>>();
		let versions = descriptor_sets
			.iter()
			.map(|set_handle| {
				self.descriptor_sets
					.get(set_handle.0 as usize)
					.map(|set| set.version)
					.unwrap_or(0)
			})
			.collect::<SmallVec<[_; 8]>>();
		let key = DescriptorMaterializationKey {
			layout: layout_handle,
			descriptor_sets,
			sequence_index,
		};

		if let Some(materialization) = self.descriptor_materializations.get(&key) {
			if materialization.versions == versions {
				return Some(materialization.clone());
			}
		}

		let layout = self.pipeline_layouts.get(layout_handle.0 as usize)?.clone();
		let cbv_srv_uav_heap = (layout.cbv_srv_uav_descriptor_count != 0)
			.then(|| {
				self.create_shader_visible_descriptor_heap(
					D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
					layout.cbv_srv_uav_descriptor_count,
				)
			})
			.flatten();
		let sampler_heap = (layout.sampler_descriptor_count != 0)
			.then(|| {
				self.create_shader_visible_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, layout.sampler_descriptor_count)
			})
			.flatten();

		if layout.cbv_srv_uav_descriptor_count != 0 && cbv_srv_uav_heap.is_none() {
			return None;
		}
		if layout.sampler_descriptor_count != 0 && sampler_heap.is_none() {
			return None;
		}
		if let Some(heap) = cbv_srv_uav_heap.as_ref() {
			self.initialize_descriptor_heap_defaults(&layout, false, heap, 0);
		}
		if let Some(heap) = sampler_heap.as_ref() {
			self.initialize_descriptor_heap_defaults(&layout, true, heap, 0);
		}

		let mut writes = SmallVec::<[(PipelineResource, u32, RetainedDescriptor); 32]>::new();
		for resource in &layout.resources {
			for set_handle in &key.descriptor_sets {
				let Some(descriptors) = self
					.descriptor_sets
					.get(set_handle.0 as usize)
					.and_then(|set| set.descriptors.get(&resource.descriptor.slot()))
				else {
					continue;
				};
				for (&array_element, &descriptor) in descriptors {
					writes.push((*resource, array_element, descriptor));
				}
			}
		}

		for (resource, array_element, descriptor) in writes {
			if let Some(heap) = cbv_srv_uav_heap.as_ref() {
				self.write_native_descriptor_for_heap(resource, descriptor, array_element, sequence_index, false, heap, 0);
			}
			if let Some(heap) = sampler_heap.as_ref() {
				self.write_native_descriptor_for_heap(resource, descriptor, array_element, sequence_index, true, heap, 0);
			}
		}

		let materialization = DescriptorMaterialization {
			versions,
			cbv_srv_uav_heap,
			sampler_heap,
		};
		self.descriptor_materializations.insert(key, materialization.clone());
		Some(materialization)
	}

	/// Retains each bound heap until this command buffer's submitted work has completed.
	fn retain_descriptor_materialization(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		materialization: &DescriptorMaterialization,
	) {
		for heap in [
			materialization.cbv_srv_uav_heap.as_ref(),
			materialization.sampler_heap.as_ref(),
		]
		.into_iter()
		.flatten()
		{
			self.retain_descriptor_heap(command_buffer_handle, heap);
		}
	}

	/// Retains a descriptor heap until the command buffer's previous submission has completed.
	fn retain_descriptor_heap(&mut self, command_buffer_handle: CommandBufferHandle, heap: &ID3D12DescriptorHeap) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		let identity = heap.as_raw();
		if command_buffer
			.retained_descriptor_heaps
			.iter()
			.any(|retained| retained.as_raw() == identity)
		{
			return;
		}
		command_buffer.retained_descriptor_heaps.push(heap.clone());
	}

	/// Retains a temporary GPU resource until the command buffer's previous submission has completed.
	fn retain_command_buffer_resource(&mut self, command_buffer_handle: CommandBufferHandle, resource: ID3D12Resource) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		command_buffer.retained_resources.push(resource);
	}

	/// Retains an upload resource and tracks its live command-buffer-scoped allocation.
	fn retain_command_buffer_upload_resource(&mut self, command_buffer_handle: CommandBufferHandle, resource: ID3D12Resource) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		command_buffer.retained_resources.push(resource);
		command_buffer.retained_upload_resource_count += 1;
	}

	/// Drops cached native snapshots after a resource replacement changes descriptor-visible addresses.
	fn invalidate_descriptor_materializations(&mut self) {
		self.descriptor_materializations.clear();
	}

	/// Drops attachment views whose native resources were replaced.
	fn invalidate_attachment_views_for_resources(&mut self, resources: &[usize]) {
		if resources.is_empty() {
			return;
		}
		self.render_target_views.retain(|key, _| !resources.contains(&key.resource));
		self.depth_stencil_views.retain(|key, _| !resources.contains(&key.resource));
	}

	/// Drops every retained attachment view after swapchain-wide resource replacement.
	fn invalidate_attachment_views(&mut self) {
		self.render_target_views.clear();
		self.depth_stencil_views.clear();
	}

	fn descriptor_range_type(descriptor: ShaderResourceDescriptor, sampler_heap: bool) -> Option<D3D12_DESCRIPTOR_RANGE_TYPE> {
		match descriptor.kind() {
			ResourceKind::UniformBuffer if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_CBV),
			ResourceKind::StorageBuffer if !sampler_heap && descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_UAV)
			}
			ResourceKind::StorageBuffer if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_SRV),
			ResourceKind::StorageImage if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_UAV),
			ResourceKind::SampledImage
			| ResourceKind::InputAttachment
			| ResourceKind::AccelerationStructure
			| ResourceKind::CombinedImageSampler
				if !sampler_heap =>
			{
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SRV)
			}
			ResourceKind::Sampler | ResourceKind::CombinedImageSampler if sampler_heap => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER)
			}
			_ => None,
		}
	}

	fn resource_range_end(descriptor: ShaderResourceDescriptor) -> u32 {
		descriptor
			.slot()
			.index()
			.checked_add(descriptor.count())
			.expect("DX12 shader resource range overflowed. The most likely cause is an invalid flat slot or resource count.")
	}

	fn resource_representations_match(left: ShaderResourceDescriptor, right: ShaderResourceDescriptor) -> bool {
		left.slot() == right.slot()
			&& left.kind() == right.kind()
			&& left.count() == right.count()
			&& left.texture_view() == right.texture_view()
			&& left.buffer_element_stride() == right.buffer_element_stride()
	}

	fn resource_ranges_overlap(left: ShaderResourceDescriptor, right: ShaderResourceDescriptor) -> bool {
		left.slot().index() < Self::resource_range_end(right) && right.slot().index() < Self::resource_range_end(left)
	}

	/// Merges shader resource declarations and assigns dense native heap offsets.
	fn build_pipeline_resources(&self, shaders: &[pipelines::ShaderParameter]) -> Vec<PipelineResource> {
		let mut descriptors = shaders
			.iter()
			.flat_map(|parameter| self.shaders[parameter.handle.0 as usize].resources.iter().copied())
			.collect::<Vec<_>>();
		descriptors.sort_by_key(|descriptor| descriptor.slot());

		let mut merged = Vec::<ShaderResourceDescriptor>::with_capacity(descriptors.len());
		for descriptor in descriptors {
			if let Some(previous) = merged.last_mut() {
				if previous.slot() == descriptor.slot() {
					assert!(
						Self::resource_representations_match(*previous, descriptor),
						"Conflicting DX12 shader resources. The most likely cause is that shader stages declared the same flat slot with incompatible representations.",
					);
					assert!(
						Self::descriptor_range_type(*previous, false) == Self::descriptor_range_type(descriptor, false),
						"Conflicting DX12 storage access. The most likely cause is that shader stages map the same flat slot to different SRV and UAV register classes.",
					);
					*previous = ShaderResourceDescriptor::new(
						previous.slot(),
						previous.kind(),
						previous.count(),
						previous.access() | descriptor.access(),
					)
					.texture_view_type(previous.texture_view())
					.buffer_stride(previous.buffer_element_stride());
					continue;
				}
				assert!(
					!Self::resource_ranges_overlap(*previous, descriptor),
					"Overlapping DX12 shader resources. The most likely cause is that shader resource arrays reserve intersecting flat slot ranges.",
				);
			}
			merged.push(descriptor);
		}

		let mut cbv_srv_uav_offset = 0u32;
		let mut sampler_offset = 0u32;
		merged
			.into_iter()
			.map(|descriptor| {
				let cbv_offset = Self::descriptor_range_type(descriptor, false).map(|_| {
					let offset = cbv_srv_uav_offset;
					cbv_srv_uav_offset = cbv_srv_uav_offset.checked_add(descriptor.count()).expect(
						"DX12 CBV/SRV/UAV descriptor count overflowed. The most likely cause is an invalid shader resource count.",
					);
					offset
				});
				let native_sampler_offset = Self::descriptor_range_type(descriptor, true).map(|_| {
					let offset = sampler_offset;
					sampler_offset = sampler_offset.checked_add(descriptor.count()).expect(
						"DX12 sampler descriptor count overflowed. The most likely cause is an invalid shader resource count.",
					);
					offset
				});
				PipelineResource {
					descriptor,
					cbv_srv_uav_offset: cbv_offset,
					sampler_offset: native_sampler_offset,
				}
			})
			.collect()
	}

	/// Creates a compact root signature with one resource table, one sampler table, and one push-constant block.
	fn create_root_signature(
		&self,
		layout: &PipelineLayout,
	) -> (Option<ID3D12RootSignature>, Vec<RootDescriptorTable>, Vec<RootConstantRange>) {
		let mut resource_ranges = Vec::new();
		let mut sampler_ranges = Vec::new();
		for resource in &layout.resources {
			if let (Some(range_type), Some(offset)) = (
				Self::descriptor_range_type(resource.descriptor, false),
				resource.cbv_srv_uav_offset,
			) {
				resource_ranges.push(D3D12_DESCRIPTOR_RANGE {
					RangeType: range_type,
					NumDescriptors: resource.descriptor.count(),
					BaseShaderRegister: resource.descriptor.slot().index(),
					RegisterSpace: 0,
					OffsetInDescriptorsFromTableStart: offset,
				});
			}
			if let (Some(range_type), Some(offset)) = (
				Self::descriptor_range_type(resource.descriptor, true),
				resource.sampler_offset,
			) {
				sampler_ranges.push(D3D12_DESCRIPTOR_RANGE {
					RangeType: range_type,
					NumDescriptors: resource.descriptor.count(),
					BaseShaderRegister: resource.descriptor.slot().index(),
					RegisterSpace: 0,
					OffsetInDescriptorsFromTableStart: offset,
				});
			}
		}

		let mut parameters = Vec::with_capacity(3);
		let mut tables = Vec::with_capacity(2);
		if !resource_ranges.is_empty() {
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
				Anonymous: D3D12_ROOT_PARAMETER_0 {
					DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
						NumDescriptorRanges: resource_ranges.len() as u32,
						pDescriptorRanges: resource_ranges.as_ptr(),
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			tables.push(RootDescriptorTable {
				root_parameter_index,
				sampler_heap: false,
			});
		}
		if !sampler_ranges.is_empty() {
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
				Anonymous: D3D12_ROOT_PARAMETER_0 {
					DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
						NumDescriptorRanges: sampler_ranges.len() as u32,
						pDescriptorRanges: sampler_ranges.as_ptr(),
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			tables.push(RootDescriptorTable {
				root_parameter_index,
				sampler_heap: true,
			});
		}

		let mut constants = Vec::new();
		let push_constant_size = layout
			.push_constant_ranges
			.iter()
			.map(|range| range.offset.saturating_add(range.size))
			.max()
			.unwrap_or(0);
		let push_constant_dword_count = push_constant_size.div_ceil(4);
		assert!(
			push_constant_dword_count.saturating_add(tables.len() as u32) <= 64,
			"DX12 root signature exceeds 64 DWORDs. The most likely cause is that push constants leave insufficient space for the descriptor tables."
		);
		if push_constant_size != 0 {
			assert!(
				layout.resources.iter().all(|resource| {
					resource.descriptor.kind() != ResourceKind::UniformBuffer || resource.descriptor.slot().index() != 0
				}),
				"Conflicting DX12 root register. The most likely cause is that push constants and a uniform buffer both use b0, space0.",
			);
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
				Anonymous: D3D12_ROOT_PARAMETER_0 {
					Constants: D3D12_ROOT_CONSTANTS {
						ShaderRegister: 0,
						RegisterSpace: 0,
						Num32BitValues: push_constant_dword_count,
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			constants.extend(layout.push_constant_ranges.iter().map(|range| RootConstantRange {
				root_parameter_index,
				offset: range.offset,
				size: range.size,
			}));
		}

		let desc = D3D12_ROOT_SIGNATURE_DESC {
			NumParameters: parameters.len() as u32,
			pParameters: if parameters.is_empty() {
				std::ptr::null()
			} else {
				parameters.as_ptr()
			},
			NumStaticSamplers: 0,
			pStaticSamplers: std::ptr::null(),
			Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
		};
		let mut blob = None;
		let mut error_blob = None;
		if unsafe { D3D12SerializeRootSignature(&desc, D3D_ROOT_SIGNATURE_VERSION_1_0, &mut blob, Some(&mut error_blob)) }
			.is_err()
		{
			if let Some(error_blob) = error_blob {
				let message = unsafe {
					std::slice::from_raw_parts(error_blob.GetBufferPointer().cast::<u8>(), error_blob.GetBufferSize())
				};
				self.log_dx12_error(format!(
					"Failed to serialize DX12 root signature: {}",
					String::from_utf8_lossy(message)
				));
			}
			return (None, tables, constants);
		}
		let Some(blob) = blob else {
			return (None, tables, constants);
		};
		let bytes = unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) };
		let root_signature = unsafe { self.device.CreateRootSignature(0, bytes) };
		if let Err(error) = &root_signature {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 root signature with {} parameters and {} descriptor tables: {error:?}; device removed reason: {removed_reason:?}",
				parameters.len(),
				tables.len(),
			));
		}
		(root_signature.ok(), tables, constants)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		push_constant_ranges: &[PushConstantRange],
	) -> PipelineLayoutHandle {
		let resources = self.build_pipeline_resources(shaders);
		let layout = PipelineLayout {
			cbv_srv_uav_descriptor_count: resources
				.iter()
				.filter_map(|resource| resource.cbv_srv_uav_offset.map(|offset| offset + resource.descriptor.count()))
				.max()
				.unwrap_or(0),
			sampler_descriptor_count: resources
				.iter()
				.filter_map(|resource| resource.sampler_offset.map(|offset| offset + resource.descriptor.count()))
				.max()
				.unwrap_or(0),
			resources,
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&layout) {
			return *handle;
		}

		self.pipeline_layouts.push(layout.clone());
		let handle = PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
		let (root_signature, root_tables, root_constants) = self.create_root_signature(&layout);
		self.pipeline_root_signatures.push(root_signature);
		self.pipeline_root_tables.push(root_tables);
		self.pipeline_root_constants.push(root_constants);
		self.pipeline_layout_indices.insert(layout, handle);
		handle
	}

	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let pipeline_state = self.create_graphics_pipeline_state(layout, &builder);
		let shaders = builder.shaders.iter().map(|s| *s.handle).collect();
		let has_mesh_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh));
		self.pipelines.push(Pipeline {
			layout,
			shaders,
			kind: PipelineKind::Raster,
			pipeline_state,
			ray_tracing_state_object: None,
			ray_tracing_shader_identifiers: HashMap::default(),
			has_mesh_shader,
		});

		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	fn create_graphics_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		builder: &pipelines::raster::Builder,
	) -> Option<ID3D12PipelineState> {
		if builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh)) {
			return self.create_mesh_pipeline_state(layout, builder);
		}

		let root_signature = self
			.pipeline_root_signatures
			.get(layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())?;
		let vertex_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Vertex)?;
		let fragment_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Fragment)?;
		if vertex_shader.is_empty() || fragment_shader.is_empty() {
			return None;
		}

		let semantic_names = builder
			.vertex_elements
			.iter()
			.map(|element| std::ffi::CString::new(element.name).ok())
			.collect::<Option<Vec<_>>>()?;
		let mut input_elements = Vec::with_capacity(builder.vertex_elements.len());
		let mut byte_offsets_by_slot = HashMap::<u32, u32>::default();
		for (index, element) in builder.vertex_elements.iter().enumerate() {
			let offset = byte_offsets_by_slot.entry(element.binding).or_insert(0);
			input_elements.push(D3D12_INPUT_ELEMENT_DESC {
				SemanticName: PCSTR(semantic_names[index].as_ptr().cast()),
				SemanticIndex: 0,
				Format: Self::vertex_format(element.format)?,
				InputSlot: element.binding,
				AlignedByteOffset: *offset,
				InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
				InstanceDataStepRate: 0,
			});
			*offset += element.format.size() as u32;
		}

		let mut render_targets = [D3D12_RENDER_TARGET_BLEND_DESC::default(); 8];
		let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
		let mut render_target_count = 0usize;
		let mut depth_stencil_format = DXGI_FORMAT_UNKNOWN;
		for attachment in builder.render_targets.iter() {
			if attachment.format == Formats::Depth32 {
				depth_stencil_format = DXGI_FORMAT_D32_FLOAT;
				continue;
			}
			if render_target_count >= rtv_formats.len() {
				break;
			}
			render_targets[render_target_count] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[render_target_count] = Self::dxgi_format(attachment.format)?;
			render_target_count += 1;
		}
		let has_depth_attachment = depth_stencil_format != DXGI_FORMAT_UNKNOWN;

		self.graphics_pipeline_state_create_attempt_count += 1;
		let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			VS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: vertex_shader.as_ptr().cast(),
				BytecodeLength: vertex_shader.len(),
			},
			PS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: fragment_shader.as_ptr().cast(),
				BytecodeLength: fragment_shader.len(),
			},
			DS: D3D12_SHADER_BYTECODE::default(),
			HS: D3D12_SHADER_BYTECODE::default(),
			GS: D3D12_SHADER_BYTECODE::default(),
			StreamOutput: Default::default(),
			BlendState: D3D12_BLEND_DESC {
				AlphaToCoverageEnable: BOOL(0),
				IndependentBlendEnable: BOOL((render_target_count > 1) as i32),
				RenderTarget: render_targets,
			},
			SampleMask: u32::MAX,
			RasterizerState: D3D12_RASTERIZER_DESC {
				FillMode: D3D12_FILL_MODE_SOLID,
				CullMode: Self::cull_mode(builder.cull_mode),
				FrontCounterClockwise: match builder.face_winding {
					pipelines::raster::FaceWinding::Clockwise => BOOL(0),
					pipelines::raster::FaceWinding::CounterClockwise => BOOL(1),
				},
				DepthBias: 0,
				DepthBiasClamp: 0.0,
				SlopeScaledDepthBias: 0.0,
				DepthClipEnable: BOOL(1),
				MultisampleEnable: BOOL(0),
				AntialiasedLineEnable: BOOL(0),
				ForcedSampleCount: 0,
				ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
			},
			DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
				DepthEnable: BOOL(has_depth_attachment as i32),
				DepthWriteMask: if has_depth_attachment && builder.depth_write {
					D3D12_DEPTH_WRITE_MASK_ALL
				} else {
					D3D12_DEPTH_WRITE_MASK_ZERO
				},
				DepthFunc: if has_depth_attachment {
					D3D12_COMPARISON_FUNC_GREATER_EQUAL
				} else {
					D3D12_COMPARISON_FUNC_ALWAYS
				},
				StencilEnable: BOOL(0),
				StencilReadMask: 0xff,
				StencilWriteMask: 0xff,
				FrontFace: Self::disabled_stencil_op_desc(),
				BackFace: Self::disabled_stencil_op_desc(),
			},
			InputLayout: D3D12_INPUT_LAYOUT_DESC {
				pInputElementDescs: if input_elements.is_empty() {
					std::ptr::null()
				} else {
					input_elements.as_ptr()
				},
				NumElements: input_elements.len() as u32,
			},
			IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
			PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
			NumRenderTargets: render_target_count as u32,
			RTVFormats: rtv_formats,
			DSVFormat: depth_stencil_format,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		match unsafe { self.device.CreateGraphicsPipelineState::<ID3D12PipelineState>(&desc) } {
			Ok(pipeline_state) => {
				self.graphics_pipeline_state_last_error = None;
				Some(pipeline_state)
			}
			Err(error) => {
				self.graphics_pipeline_state_last_error = Some(error.code().0);
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 graphics pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				));
				None
			}
		}
	}

	fn create_mesh_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		builder: &pipelines::raster::Builder,
	) -> Option<ID3D12PipelineState> {
		if !self.supports_native_mesh_shaders() {
			self.log_debug_message(
				"Skipping DX12 mesh pipeline creation because native mesh shaders are not supported by this device.",
			);
			return None;
		}

		let root_signature = self
			.pipeline_root_signatures
			.get(layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())?;
		let has_task_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Task));
		let task_shader = if has_task_shader {
			self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Task)?
		} else {
			Vec::new()
		};
		let mesh_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Mesh)?;
		let has_fragment_shader = builder
			.shaders
			.iter()
			.any(|shader| matches!(shader.stage, ShaderTypes::Fragment));
		let fragment_shader = if has_fragment_shader {
			self.shader_dxil_for_stage_with_dxc_target(builder.shaders.as_ref(), ShaderTypes::Fragment, "ps_6_0")?
		} else {
			Vec::new()
		};
		if (has_task_shader && task_shader.is_empty())
			|| mesh_shader.is_empty()
			|| (has_fragment_shader && fragment_shader.is_empty())
		{
			return None;
		}

		let mut render_targets = [Self::render_target_blend_desc(pipelines::raster::BlendMode::None); 8];
		let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
		let mut render_target_count = 0usize;
		let mut depth_stencil_format = DXGI_FORMAT_UNKNOWN;
		for attachment in builder.render_targets.iter() {
			if attachment.format == Formats::Depth32 {
				depth_stencil_format = DXGI_FORMAT_D32_FLOAT;
				continue;
			}
			if render_target_count >= rtv_formats.len() {
				break;
			}
			render_targets[render_target_count] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[render_target_count] = Self::dxgi_format(attachment.format)?;
			render_target_count += 1;
		}
		let has_depth_attachment = depth_stencil_format != DXGI_FORMAT_UNKNOWN;

		self.graphics_pipeline_state_create_attempt_count += 1;
		let mut stream = MeshPipelineStateStream {
			root_signature: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE,
				value: Some(root_signature),
			},
			amplification_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: if task_shader.is_empty() {
						std::ptr::null()
					} else {
						task_shader.as_ptr().cast()
					},
					BytecodeLength: task_shader.len(),
				},
			},
			mesh_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: mesh_shader.as_ptr().cast(),
					BytecodeLength: mesh_shader.len(),
				},
			},
			pixel_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: if fragment_shader.is_empty() {
						std::ptr::null()
					} else {
						fragment_shader.as_ptr().cast()
					},
					BytecodeLength: fragment_shader.len(),
				},
			},
			blend: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
				value: D3D12_BLEND_DESC {
					AlphaToCoverageEnable: BOOL(0),
					IndependentBlendEnable: BOOL((render_target_count > 1) as i32),
					RenderTarget: render_targets,
				},
			},
			sample_mask: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK,
				value: u32::MAX,
			},
			rasterizer: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER,
				value: D3D12_RASTERIZER_DESC {
					FillMode: D3D12_FILL_MODE_SOLID,
					CullMode: Self::cull_mode(builder.cull_mode),
					FrontCounterClockwise: match builder.face_winding {
						pipelines::raster::FaceWinding::Clockwise => BOOL(0),
						pipelines::raster::FaceWinding::CounterClockwise => BOOL(1),
					},
					DepthBias: 0,
					DepthBiasClamp: 0.0,
					SlopeScaledDepthBias: 0.0,
					DepthClipEnable: BOOL(1),
					MultisampleEnable: BOOL(0),
					AntialiasedLineEnable: BOOL(0),
					ForcedSampleCount: 0,
					ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
				},
			},
			depth_stencil: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL,
				value: D3D12_DEPTH_STENCIL_DESC {
					DepthEnable: BOOL(has_depth_attachment as i32),
					DepthWriteMask: if has_depth_attachment && builder.depth_write {
						D3D12_DEPTH_WRITE_MASK_ALL
					} else {
						D3D12_DEPTH_WRITE_MASK_ZERO
					},
					DepthFunc: if has_depth_attachment {
						D3D12_COMPARISON_FUNC_GREATER_EQUAL
					} else {
						D3D12_COMPARISON_FUNC_ALWAYS
					},
					StencilEnable: BOOL(0),
					StencilReadMask: 0xff,
					StencilWriteMask: 0xff,
					FrontFace: Self::disabled_stencil_op_desc(),
					BackFace: Self::disabled_stencil_op_desc(),
				},
			},
			depth_stencil_format: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
				value: depth_stencil_format,
			},
			render_targets: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
				value: D3D12_RT_FORMAT_ARRAY {
					RTFormats: rtv_formats,
					NumRenderTargets: render_target_count as u32,
				},
			},
			sample_desc: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
				value: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			},
			node_mask: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK,
				value: 0,
			},
			flags: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS,
				value: D3D12_PIPELINE_STATE_FLAG_NONE,
			},
		};
		let desc = D3D12_PIPELINE_STATE_STREAM_DESC {
			SizeInBytes: std::mem::size_of::<MeshPipelineStateStream>(),
			pPipelineStateSubobjectStream: (&mut stream as *mut MeshPipelineStateStream).cast(),
		};
		let device = self.device.cast::<ID3D12Device2>().ok()?;

		match unsafe { device.CreatePipelineState::<ID3D12PipelineState>(&desc) } {
			Ok(pipeline_state) => {
				self.graphics_pipeline_state_last_error = None;
				Some(pipeline_state)
			}
			Err(error) => {
				self.graphics_pipeline_state_last_error = Some(error.code().0);
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 mesh pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				));
				None
			}
		}
	}

	fn shader_dxil_for_stage(&mut self, shaders: &[pipelines::ShaderParameter], stage: ShaderTypes) -> Option<Vec<u8>> {
		self.shader_dxil_for_stage_impl(shaders, stage, None)
	}

	fn shader_dxil_for_stage_with_dxc_target(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		stage: ShaderTypes,
		target: &str,
	) -> Option<Vec<u8>> {
		self.shader_dxil_for_stage_impl(shaders, stage, Some(target))
	}

	fn shader_dxil_for_stage_impl(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		stage: ShaderTypes,
		dxc_target: Option<&str>,
	) -> Option<Vec<u8>> {
		let parameter = shaders.iter().find(|parameter| {
			matches!(
				(parameter.stage, stage),
				(ShaderTypes::Vertex, ShaderTypes::Vertex)
					| (ShaderTypes::Fragment, ShaderTypes::Fragment)
					| (ShaderTypes::Task, ShaderTypes::Task)
					| (ShaderTypes::Mesh, ShaderTypes::Mesh)
			)
		})?;
		let shader = self.shaders.get(parameter.handle.0 as usize)?;
		if let Some(target) = dxc_target {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl_with_dxc(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						target,
						parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() && !parameter.specialization_map.is_empty() {
					self.hlsl_specialization_compile_count += 1;
				}
				return dxil;
			}
		} else if !parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						stage,
						parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() {
					self.hlsl_specialization_compile_count += 1;
				}
				return dxil;
			}
		}
		shader.dxil.clone()
	}

	fn vertex_format(data_type: DataTypes) -> Option<DXGI_FORMAT> {
		match data_type {
			DataTypes::Float => Some(DXGI_FORMAT_R32_FLOAT),
			DataTypes::Float2 => Some(DXGI_FORMAT_R32G32_FLOAT),
			DataTypes::Float3 => Some(DXGI_FORMAT_R32G32B32_FLOAT),
			DataTypes::Float4 => Some(DXGI_FORMAT_R32G32B32A32_FLOAT),
			DataTypes::Int => Some(DXGI_FORMAT_R32_SINT),
			DataTypes::Int2 => Some(DXGI_FORMAT_R32G32_SINT),
			DataTypes::Int3 => Some(DXGI_FORMAT_R32G32B32_SINT),
			DataTypes::Int4 => Some(DXGI_FORMAT_R32G32B32A32_SINT),
			DataTypes::UInt | DataTypes::U32 => Some(DXGI_FORMAT_R32_UINT),
			DataTypes::UInt2 => Some(DXGI_FORMAT_R32G32_UINT),
			DataTypes::UInt3 => Some(DXGI_FORMAT_R32G32B32_UINT),
			DataTypes::UInt4 => Some(DXGI_FORMAT_R32G32B32A32_UINT),
			DataTypes::U8 | DataTypes::U16 => None,
		}
	}

	fn cull_mode(cull_mode: pipelines::raster::CullMode) -> windows::Win32::Graphics::Direct3D12::D3D12_CULL_MODE {
		match cull_mode {
			pipelines::raster::CullMode::None => D3D12_CULL_MODE_NONE,
			pipelines::raster::CullMode::Front => D3D12_CULL_MODE_FRONT,
			pipelines::raster::CullMode::Back => D3D12_CULL_MODE_BACK,
		}
	}

	fn render_target_blend_desc(blend: pipelines::raster::BlendMode) -> D3D12_RENDER_TARGET_BLEND_DESC {
		let blend_enable = matches!(blend, pipelines::raster::BlendMode::Alpha);
		D3D12_RENDER_TARGET_BLEND_DESC {
			BlendEnable: BOOL(blend_enable as i32),
			LogicOpEnable: BOOL(0),
			SrcBlend: if blend_enable {
				D3D12_BLEND_SRC_ALPHA
			} else {
				D3D12_BLEND_ONE
			},
			DestBlend: if blend_enable {
				D3D12_BLEND_INV_SRC_ALPHA
			} else {
				D3D12_BLEND_ZERO
			},
			BlendOp: D3D12_BLEND_OP_ADD,
			SrcBlendAlpha: D3D12_BLEND_ONE,
			DestBlendAlpha: if blend_enable {
				D3D12_BLEND_INV_SRC_ALPHA
			} else {
				D3D12_BLEND_ZERO
			},
			BlendOpAlpha: D3D12_BLEND_OP_ADD,
			LogicOp: D3D12_LOGIC_OP_NOOP,
			RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
		}
	}

	fn disabled_stencil_op_desc() -> D3D12_DEPTH_STENCILOP_DESC {
		D3D12_DEPTH_STENCILOP_DESC {
			StencilFailOp: D3D12_STENCIL_OP_KEEP,
			StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
			StencilPassOp: D3D12_STENCIL_OP_KEEP,
			StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
		}
	}

	pub fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(std::slice::from_ref(&builder.shader), builder.push_constant_ranges);
		let shader_parameter = builder.shader;
		let pipeline_state = self.create_compute_pipeline_state(layout, shader_parameter);
		self.pipelines.push(Pipeline {
			layout,
			shaders: vec![*shader_parameter.handle],
			kind: PipelineKind::Compute,
			pipeline_state,
			ray_tracing_state_object: None,
			ray_tracing_shader_identifiers: HashMap::default(),
			has_mesh_shader: false,
		});
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	fn create_compute_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		shader_parameter: pipelines::ShaderParameter,
	) -> Option<ID3D12PipelineState> {
		let root_signature = self
			.pipeline_root_signatures
			.get(layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())?;
		let shader = self.shaders.get(shader_parameter.handle.0 as usize)?;
		let dxil = if !shader_parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						shader_parameter.stage,
						shader_parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() {
					self.hlsl_specialization_compile_count += 1;
				}
				dxil
			} else {
				shader.dxil.clone()
			}
		} else {
			shader.dxil.clone()
		}?;
		if dxil.is_empty() {
			return None;
		}
		self.compute_pipeline_state_create_attempt_count += 1;
		let desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			CS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: dxil.as_ptr().cast(),
				BytecodeLength: dxil.len(),
			},
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		let pipeline_state = unsafe { self.device.CreateComputePipelineState::<ID3D12PipelineState>(&desc) };
		if let Err(error) = &pipeline_state {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 compute pipeline state: {error:?}; device removed reason: {removed_reason:?}"
			));
		}
		pipeline_state.ok()
	}

	pub fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let shaders = builder.shaders;
		let (ray_tracing_state_object, ray_tracing_shader_identifiers) = self.create_ray_tracing_state_object(layout, &shaders);
		self.pipelines.push(Pipeline {
			layout,
			shaders: shaders.iter().map(|s| *s.handle).collect(),
			kind: PipelineKind::RayTracing,
			pipeline_state: None,
			ray_tracing_state_object,
			ray_tracing_shader_identifiers,
			has_mesh_shader: false,
		});

		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	fn create_ray_tracing_state_object(
		&mut self,
		layout: PipelineLayoutHandle,
		shaders: &[pipelines::ShaderParameter],
	) -> (
		Option<ID3D12StateObject>,
		HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]>,
	) {
		if !shaders.iter().any(|shader| {
			self.shaders
				.get(shader.handle.0 as usize)
				.and_then(|shader| shader.dxil.as_ref())
				.is_some_and(|dxil| !dxil.is_empty())
		}) {
			return (None, HashMap::default());
		}
		let Ok(device) = self.device.cast::<ID3D12Device5>() else {
			return (None, HashMap::default());
		};
		let Some(root_signature) = self
			.pipeline_root_signatures
			.get(layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())
		else {
			return (None, HashMap::default());
		};
		self.ray_tracing_state_object_create_attempt_count += 1;

		let mut export_names = Vec::with_capacity(shaders.len());
		let mut source_export_names = Vec::with_capacity(shaders.len());
		let mut exports = Vec::with_capacity(shaders.len());
		let mut libraries = Vec::with_capacity(shaders.len());
		let mut hit_group_names = Vec::with_capacity(shaders.len());
		let mut hit_groups = Vec::with_capacity(shaders.len());
		let mut identifier_exports = Vec::with_capacity(shaders.len());
		let mut subobjects = Vec::with_capacity(shaders.len() * 2 + 4);

		for shader_parameter in shaders {
			let Some(shader) = self.shaders.get(shader_parameter.handle.0 as usize) else {
				continue;
			};
			let Some(dxil) = shader.dxil.as_ref() else {
				continue;
			};
			if dxil.is_empty() {
				continue;
			}
			let export_name = wide_null(&format!("ghi_shader_{}", shader_parameter.handle.0));
			export_names.push(export_name);
			let export_name = PCWSTR(export_names.last().expect("Export name was just pushed.").as_ptr());
			let source_export_name = wide_null(
				shader
					.hlsl
					.as_ref()
					.map(|source| source.entry_point.as_str())
					.unwrap_or("main"),
			);
			source_export_names.push(source_export_name);
			let source_export_name = PCWSTR(
				source_export_names
					.last()
					.expect("Source export name was just pushed.")
					.as_ptr(),
			);
			let mut identifier_export = export_name;
			exports.push(D3D12_EXPORT_DESC {
				Name: export_name,
				ExportToRename: source_export_name,
				Flags: D3D12_EXPORT_FLAG_NONE,
			});
			let export = exports.last().expect("Export descriptor was just pushed.");
			libraries.push(D3D12_DXIL_LIBRARY_DESC {
				DXILLibrary: D3D12_SHADER_BYTECODE {
					pShaderBytecode: dxil.as_ptr().cast(),
					BytecodeLength: dxil.len(),
				},
				NumExports: 1,
				pExports: export,
			});
			let library = libraries.last().expect("DXIL library descriptor was just pushed.");
			subobjects.push(D3D12_STATE_SUBOBJECT {
				Type: D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
				pDesc: (library as *const D3D12_DXIL_LIBRARY_DESC).cast(),
			});

			match shader_parameter.stage {
				ShaderTypes::ClosestHit | ShaderTypes::AnyHit | ShaderTypes::Intersection => {
					let is_any_hit = matches!(shader_parameter.stage, ShaderTypes::AnyHit);
					let is_closest_hit = matches!(shader_parameter.stage, ShaderTypes::ClosestHit);
					let is_intersection = matches!(shader_parameter.stage, ShaderTypes::Intersection);
					let hit_group_name = wide_null(&format!("ghi_hit_group_{}", shader_parameter.handle.0));
					hit_group_names.push(hit_group_name);
					let hit_group_name = PCWSTR(hit_group_names.last().expect("Hit group name was just pushed.").as_ptr());
					identifier_export = hit_group_name;
					hit_groups.push(D3D12_HIT_GROUP_DESC {
						HitGroupExport: hit_group_name,
						Type: if is_intersection {
							D3D12_HIT_GROUP_TYPE_PROCEDURAL_PRIMITIVE
						} else {
							D3D12_HIT_GROUP_TYPE_TRIANGLES
						},
						AnyHitShaderImport: if is_any_hit { export_name } else { PCWSTR::null() },
						ClosestHitShaderImport: if is_closest_hit { export_name } else { PCWSTR::null() },
						IntersectionShaderImport: if is_intersection { export_name } else { PCWSTR::null() },
					});
					let hit_group = hit_groups.last().expect("Hit group descriptor was just pushed.");
					subobjects.push(D3D12_STATE_SUBOBJECT {
						Type: D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP,
						pDesc: (hit_group as *const D3D12_HIT_GROUP_DESC).cast(),
					});
				}
				_ => {}
			}
			identifier_exports.push((*shader_parameter.handle, identifier_export));
		}

		if subobjects.is_empty() {
			return (None, HashMap::default());
		}
		let global_root_signature = D3D12_GLOBAL_ROOT_SIGNATURE {
			pGlobalRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
			pDesc: (&global_root_signature as *const D3D12_GLOBAL_ROOT_SIGNATURE).cast(),
		});
		let shader_config = D3D12_RAYTRACING_SHADER_CONFIG {
			MaxPayloadSizeInBytes: 32,
			MaxAttributeSizeInBytes: 32,
		};
		let shader_config_subobject_index = subobjects.len();
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
			pDesc: (&shader_config as *const D3D12_RAYTRACING_SHADER_CONFIG).cast(),
		});
		let shader_config_exports = identifier_exports.iter().map(|(_, export)| *export).collect::<Vec<_>>();
		let shader_config_association = D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION {
			pSubobjectToAssociate: (&subobjects[shader_config_subobject_index] as *const D3D12_STATE_SUBOBJECT).cast(),
			NumExports: shader_config_exports.len() as u32,
			pExports: shader_config_exports.as_ptr(),
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_SUBOBJECT_TO_EXPORTS_ASSOCIATION,
			pDesc: (&shader_config_association as *const D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION).cast(),
		});
		let pipeline_config = D3D12_RAYTRACING_PIPELINE_CONFIG {
			MaxTraceRecursionDepth: 1,
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
			pDesc: (&pipeline_config as *const D3D12_RAYTRACING_PIPELINE_CONFIG).cast(),
		});
		let desc = D3D12_STATE_OBJECT_DESC {
			Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
			NumSubobjects: subobjects.len() as u32,
			pSubobjects: subobjects.as_ptr(),
		};
		let state_object = match unsafe { device.CreateStateObject::<ID3D12StateObject>(&desc) } {
			Ok(state_object) => state_object,
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 ray tracing state object: {error:?}. The most likely cause is an invalid DXIL library, hit group export, or ray tracing root signature. Device removed reason: {removed_reason:?}"
				));
				return (None, HashMap::default());
			}
		};
		let identifiers = Self::ray_tracing_shader_identifiers(&state_object, &identifier_exports);
		(Some(state_object), identifiers)
	}

	fn ray_tracing_shader_identifiers(
		state_object: &ID3D12StateObject,
		exports: &[(ShaderHandle, PCWSTR)],
	) -> HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]> {
		let Ok(properties) = state_object.cast::<ID3D12StateObjectProperties>() else {
			return HashMap::default();
		};
		let mut identifiers = HashMap::default();
		for &(shader_handle, export_name) in exports {
			let identifier = unsafe { properties.GetShaderIdentifier(export_name) };
			if identifier.is_null() {
				continue;
			}
			let mut bytes = [0u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize];
			unsafe {
				std::ptr::copy_nonoverlapping(identifier.cast::<u8>(), bytes.as_mut_ptr(), bytes.len());
			}
			identifiers.insert(shader_handle, bytes);
		}
		identifiers
	}

	/// Creates a command buffer and initializes a matching command allocator and list.
	pub fn create_command_buffer(&mut self, _name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle {
		let queue = &self.queues[queue_handle.0 as usize];
		let allocator = unsafe { self.device.CreateCommandAllocator(queue.queue_type) }.ok();
		let command_list: Option<ID3D12GraphicsCommandList> = if let Some(allocator) = allocator.as_ref() {
			unsafe { self.device.CreateCommandList(0, queue.queue_type, allocator, None) }.ok()
		} else {
			None
		};
		if let Some(command_list) = command_list.as_ref() {
			let _ = unsafe { command_list.Close() };
		}

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			allocator,
			command_list,
			retained_descriptor_heaps: Vec::new(),
			retained_resources: Vec::new(),
			retained_upload_resource_count: 0,
			cbv_srv_uav_staging_heap: None,
			sampler_staging_heap: None,
			is_open: false,
			recorded_work: false,
			sequence_index: 0,
			last_submission: None,
		});

		CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.begin_command_buffer(command_buffer_handle, 0);
		super::CommandBufferRecording::new(self, command_buffer_handle, None)
	}

	pub fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Static,
		);
		BufferHandle(BaseBufferHandle(handle), std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T> {
		let handle = self.create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			BufferStorage::Dynamic,
		);
		DynamicBufferHandle(BaseBufferHandle(handle), std::marker::PhantomData)
	}

	pub fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));
		crate::DynamicImageHandle(handle.0)
	}

	pub fn get_buffer_address(&self, _buffer_handle: BaseBufferHandle) -> u64 {
		self.buffer(_buffer_handle)
			.and_then(|buffer| buffer.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	fn buffer_address_for_sequence(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) -> u64 {
		self.buffer_resource_for_sequence(buffer_handle, sequence_index)
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		let buffer = self
			.buffer(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		unsafe { &*(buffer.data as *const T) }
	}

	pub fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: BufferHandle<T>) -> &'a mut T {
		let buffer = self
			.buffer(buffer_handle.into())
			.expect("Missing DX12 buffer. The most likely cause is that the buffer handle came from another device.");
		unsafe { &mut *(buffer.data as *mut T) }
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: ImageHandle) -> &'static mut [u8] {
		self.texture_slice_mut_static(texture_handle.0)
	}

	pub(crate) fn texture_slice_mut_static(&self, texture_handle: crate::BaseImageHandle) -> &'static mut [u8] {
		self.texture_slice_mut_for_sequence(texture_handle, 0)
	}

	pub(crate) fn texture_slice_mut_for_sequence(
		&self,
		texture_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> &'static mut [u8] {
		let image = &self.images[texture_handle.0 as usize];
		let data = if let Some(frame_data) = image.frame_data.as_ref() {
			frame_data.get(sequence_index as usize).or_else(|| frame_data.first())
		} else {
			image.data.as_ref()
		};
		let Some(data) = data else { return &mut [] };
		unsafe { std::slice::from_raw_parts_mut(data.as_ptr() as *mut u8, data.len()) }
	}

	pub fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		// Writes into CPU-side staging storage when available.
		let Some(image) = self.images.get_mut(texture_handle.0 .0 as usize) else {
			return;
		};

		let Some(staging) = image.data.as_mut() else {
			return;
		};

		f(staging);
	}

	pub(crate) fn queue_texture_sync_for_sequence(&mut self, image_handle: crate::BaseImageHandle, sequence_index: u8) {
		if !self
			.pending_texture_syncs
			.iter()
			.any(|&(pending_image, pending_sequence)| pending_image == image_handle && pending_sequence == sequence_index)
		{
			self.pending_texture_syncs.push((image_handle, sequence_index));
		}
	}

	pub fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		let size = utils::texture_copy_size(builder.format, builder.extent);
		let data = size.map(|bytes| vec![0u8; bytes]);
		let array_layers = builder.array_layers.map(|layers| layers.get()).unwrap_or(1);
		let frame_data = if builder.use_case == UseCases::DYNAMIC {
			data.as_ref().map(|data| vec![data.clone(); self.frames as usize])
		} else {
			None
		};
		let resource = if builder.use_case == UseCases::DYNAMIC {
			None
		} else {
			self.create_image_resource(builder.extent, builder.format, builder.resource_uses, array_layers, None)
		};
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, builder.format, builder.resource_uses, array_layers);
		}
		let frame_resources = if builder.use_case == UseCases::DYNAMIC {
			let mut resources = vec![None; self.frames as usize];
			if let Some(first_resource) = resource.clone() {
				if let Some(slot) = resources.first_mut() {
					*slot = Some(first_resource);
				}
			}
			Some(resources)
		} else {
			None
		};

		self.images.push(Image {
			extent: builder.extent,
			format: builder.format,
			uses: builder.resource_uses,
			access: builder.device_accesses,
			array_layers,
			resource,
			data,
			frame_data,
			frame_resources,
			optimized_clear_value: None,
		});

		ImageHandle(crate::BaseImageHandle((self.images.len() - 1) as u64))
	}

	pub(crate) fn image_resource_state(&self, image: ImageHandle) -> Option<(Extent, bool)> {
		self.images
			.get(image.0 .0 as usize)
			.map(|image| (image.extent, image.resource.is_some()))
	}

	pub(crate) fn image_frame_resource_state(&self, image: ImageHandle, sequence_index: u8) -> Option<bool> {
		self.images.get(image.0 .0 as usize).map(|image| {
			image
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.is_some()
		})
	}

	pub(crate) fn tracked_image_resource_state(&self, image: ImageHandle) -> Option<D3D12_RESOURCE_STATES> {
		self.tracked_image_resource_state_for_sequence(image, 0)
	}

	pub(crate) fn tracked_image_resource_state_for_sequence(
		&self,
		image: ImageHandle,
		sequence_index: u8,
	) -> Option<D3D12_RESOURCE_STATES> {
		let image = self.images.get(image.0 .0 as usize)?;
		let resource = if let Some(resources) = image.frame_resources.as_ref() {
			resources.get(sequence_index as usize)?.as_ref()?
		} else {
			image.resource.as_ref()?
		};
		self.image_states.get(&Self::native_resource_key(resource)).copied()
	}

	#[cfg(test)]
	pub(crate) fn pending_texture_sync_count(&self) -> usize {
		self.pending_texture_syncs.len()
	}

	/// Returns the native texture for a frame, creating deferred dynamic image resources on first use.
	fn ensure_image_resource_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let (extent, format, uses, array_layers, optimized_clear_value, dynamic) = {
			let image = self.images.get(image_handle.0 as usize)?;
			(
				image.extent,
				image.format,
				image.uses,
				image.array_layers,
				image.optimized_clear_value,
				image.frame_resources.is_some(),
			)
		};
		if !dynamic {
			return self
				.images
				.get(image_handle.0 as usize)
				.and_then(|image| image.resource.clone());
		}

		let frame_index = sequence_index as usize;
		let needs_resource = self
			.images
			.get(image_handle.0 as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone)
			.is_none();

		if needs_resource {
			let resource = self.create_image_resource(extent, format, uses, array_layers, optimized_clear_value);
			if let Some(resource) = resource.as_ref() {
				self.materialize_image_attachment_views(resource, format, uses, array_layers);
			}
			let image = self.images.get_mut(image_handle.0 as usize)?;
			if let Some(resources) = image.frame_resources.as_mut() {
				if resources.len() <= frame_index {
					resources.resize(frame_index + 1, None);
				}
				resources[frame_index] = resource.clone();
			}
		}

		self.images
			.get(image_handle.0 as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone)
	}

	fn image_resource_for_sequence(&self, image_handle: crate::BaseImageHandle, sequence_index: u8) -> Option<ID3D12Resource> {
		let image = self.images.get(image_handle.0 as usize)?;
		if let Some(resources) = image.frame_resources.as_ref() {
			return resources
				.get(sequence_index as usize)
				.and_then(Clone::clone)
				.or_else(|| resources.first().and_then(Clone::clone));
		}
		image.resource.clone()
	}

	/// Stores the optimized clear value used when a deferred DX12 image resource is created.
	fn set_image_optimized_clear_value(&mut self, image_handle: crate::BaseImageHandle, clear: ClearValue) {
		let Some(image) = self.images.get_mut(image_handle.0 as usize) else {
			return;
		};
		let flags = Self::image_resource_flags(image.format, image.uses);
		image.optimized_clear_value = Self::optimized_image_clear_value(image.format, flags, clear);
	}

	pub(crate) fn buffer_resource_state(
		&self,
		buffer: BaseBufferHandle,
	) -> Option<(DeviceAccesses, BufferHeapKind, bool, bool)> {
		self.buffer(buffer).map(|buffer| {
			(
				buffer.access,
				buffer.heap_kind,
				buffer.resource.is_some(),
				!buffer.mapped.is_null(),
			)
		})
	}

	pub(crate) fn buffer_frame_resource_state(&self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<bool> {
		self.buffer(buffer).map(|buffer| {
			if sequence_index == 0 {
				return buffer.resource.is_some();
			}
			buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.and_then(|resource| resource.resource.as_ref())
				.is_some()
		})
	}

	#[cfg(test)]
	pub(crate) fn buffer_native_size_for_sequence(&mut self, buffer: BaseBufferHandle, sequence_index: u8) -> Option<u64> {
		let resource = self.buffer_resource_for_sequence(buffer, sequence_index)?;
		Some(unsafe { resource.GetDesc() }.Width)
	}

	pub(crate) fn upload_resource_count(&self) -> usize {
		self.command_buffers
			.iter()
			.map(|command_buffer| command_buffer.retained_upload_resource_count)
			.sum()
	}

	pub(crate) fn readback_resource_count(&self) -> usize {
		self.texture_readbacks.len()
	}

	#[cfg(test)]
	pub(crate) fn render_target_view_count(&self) -> usize {
		self.render_target_views.len()
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_count(&self) -> usize {
		self.depth_stencil_views.len()
	}

	#[cfg(test)]
	pub(crate) fn render_target_view_allocation_count(&self) -> usize {
		self.render_target_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_allocation_count(&self) -> usize {
		self.depth_stencil_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_descriptor_count(&self) -> u32 {
		self.depth_stencil_views
			.values()
			.map(|view| unsafe { view.heap.GetDesc() }.NumDescriptors)
			.sum()
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_array_range(array_layers: u32, layer: Option<u32>) -> Option<(u32, u32)> {
		let descriptor = Self::depth_stencil_view_desc(Formats::Depth32, array_layers, layer);
		if descriptor.ViewDimension != D3D12_DSV_DIMENSION_TEXTURE2DARRAY {
			return None;
		}
		let array = unsafe { descriptor.Anonymous.Texture2DArray };
		Some((array.FirstArraySlice, array.ArraySize))
	}

	pub(crate) fn texture_readback_resolve_count(&self) -> usize {
		self.texture_readback_resolve_count
	}

	pub(crate) fn debug_region_begin_count(&self) -> usize {
		self.debug_region_begin_count.get()
	}

	pub(crate) fn debug_region_end_count(&self) -> usize {
		self.debug_region_end_count.get()
	}

	pub(crate) fn texture_copy_count(&self) -> usize {
		self.texture_copy_count
	}

	pub(crate) fn buffer_copy_count(&self) -> usize {
		self.buffer_copy_count
	}

	pub(crate) fn buffer_clear_count(&self) -> usize {
		self.buffer_clear_count
	}

	pub(crate) fn native_command_list_execute_count(&self) -> usize {
		self.native_command_list_execute_count
	}

	pub(crate) fn empty_command_list_skip_count(&self) -> usize {
		self.empty_command_list_skip_count
	}

	pub(crate) fn buffer_is_in_common_state(&self, buffer: BaseBufferHandle) -> Option<bool> {
		self.buffer(buffer)
			.and_then(|buffer_data| buffer_data.resource.as_ref())
			.map(|resource| {
				self.buffer_states
					.get(&Self::native_resource_key(resource))
					.copied()
					.unwrap_or(D3D12_RESOURCE_STATE_COMMON)
					== D3D12_RESOURCE_STATE_COMMON
			})
	}

	pub(crate) fn buffer_bytes(&self, buffer: BaseBufferHandle, size: usize) -> Option<Vec<u8>> {
		let buffer_data = self.buffer(buffer)?;
		if size > buffer_data.size {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(buffer_data.data, size).to_vec() })
	}

	pub(crate) fn buffer_bytes_for_sequence(
		&self,
		buffer: BaseBufferHandle,
		size: usize,
		sequence_index: u8,
	) -> Option<Vec<u8>> {
		let (data, buffer_size) = self.buffer_storage_parts_for_sequence(buffer, sequence_index)?;
		if size > buffer_size {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(data, size).to_vec() })
	}

	/// Returns bytes currently visible through a host-mapped DX12 buffer resource.
	#[cfg(test)]
	pub(crate) fn buffer_mapped_bytes_for_sequence(
		&mut self,
		buffer: BaseBufferHandle,
		size: usize,
		sequence_index: u8,
	) -> Option<Vec<u8>> {
		self.ensure_buffer_frame_storage(buffer, sequence_index);
		let buffer_data = self.buffer(buffer)?;
		if size > buffer_data.size {
			return None;
		}
		let mapped = if sequence_index == 0 {
			buffer_data.mapped
		} else {
			buffer_data
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.map(|resource| resource.mapped)
				.unwrap_or(buffer_data.mapped)
		};
		if mapped.is_null() {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(mapped, size).to_vec() })
	}

	pub(crate) fn image_is_in_common_state(&self, image: ImageHandle) -> Option<bool> {
		self.images
			.get(image.0 .0 as usize)
			.and_then(|image_data| image_data.resource.as_ref())
			.map(|resource| {
				self.image_states
					.get(&Self::native_resource_key(resource))
					.copied()
					.unwrap_or(D3D12_RESOURCE_STATE_COMMON)
					== D3D12_RESOURCE_STATE_COMMON
			})
	}

	/// Returns whether any retained native materialization contains this logical set.
	pub(crate) fn descriptor_set_has_native_heaps(&self, descriptor_set: DescriptorSetHandle) -> Option<(bool, bool)> {
		self.descriptor_sets.get(descriptor_set.0 as usize)?;
		let frame_sets = self.collect_descriptor_set_handles(descriptor_set);
		let mut cbv_srv_uav = false;
		let mut sampler = false;
		for (key, materialization) in &self.descriptor_materializations {
			if !key.descriptor_sets.iter().any(|set| frame_sets.contains(set)) {
				continue;
			}
			cbv_srv_uav |= materialization.cbv_srv_uav_heap.is_some();
			sampler |= materialization.sampler_heap.is_some();
		}
		Some((cbv_srv_uav, sampler))
	}

	/// Returns the number of cached frame-resolved native descriptor snapshots.
	#[cfg(test)]
	pub(crate) fn descriptor_materialization_count(&self) -> usize {
		self.descriptor_materializations.len()
	}

	#[cfg(test)]
	pub(crate) fn pipeline_descriptor_counts(&self, pipeline: PipelineHandle) -> Option<(u32, u32)> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		let layout = self.pipeline_layouts.get(pipeline.layout.0 as usize)?;
		Some((layout.cbv_srv_uav_descriptor_count, layout.sampler_descriptor_count))
	}

	#[cfg(test)]
	pub(crate) fn pipeline_descriptor_slot(
		&self,
		pipeline: PipelineHandle,
		slot: ResourceSlot,
		array_element: u32,
		sampler_heap: bool,
	) -> Option<u32> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		let layout = self.pipeline_layouts.get(pipeline.layout.0 as usize)?;
		let resource = layout.resources.iter().find(|resource| resource.descriptor.slot() == slot)?;
		if array_element >= resource.descriptor.count() {
			return None;
		}
		let offset = if sampler_heap {
			resource.sampler_offset
		} else {
			resource.cbv_srv_uav_offset
		}?;
		Some(offset + array_element)
	}

	#[cfg(test)]
	pub(crate) fn pipeline_resource_descriptor(
		&self,
		pipeline: PipelineHandle,
		slot: ResourceSlot,
	) -> Option<ShaderResourceDescriptor> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		self.pipeline_layouts[pipeline.layout.0 as usize]
			.resources
			.iter()
			.find(|resource| resource.descriptor.slot() == slot)
			.map(|resource| resource.descriptor)
	}

	pub(crate) fn pipeline_layout_has_root_signature(&self, pipeline_layout: PipelineLayoutHandle) -> Option<bool> {
		self.pipeline_root_signatures
			.get(pipeline_layout.0 as usize)
			.map(|root_signature| root_signature.is_some())
	}

	pub(crate) fn root_signature_bind_count(&self) -> usize {
		self.root_signature_bind_count
	}

	pub(crate) fn descriptor_heap_bind_count(&self) -> usize {
		self.descriptor_heap_bind_count
	}

	pub(crate) fn descriptor_table_bind_count(&self) -> usize {
		self.descriptor_table_bind_count
	}

	#[cfg(test)]
	pub(crate) fn descriptor_table_bind_records(&self) -> &[DescriptorTableBindRecord] {
		&self.descriptor_table_bind_records
	}

	pub(crate) fn push_constant_write_count(&self) -> usize {
		self.push_constant_write_count
	}

	#[cfg(test)]
	pub(crate) fn push_constant_write_records(&self) -> &[PushConstantWriteRecord] {
		&self.push_constant_write_records
	}

	pub(crate) fn descriptor_write_count(&self) -> usize {
		self.descriptor_write_count
	}

	pub(crate) fn image_srv_descriptor_write_count(&self) -> usize {
		self.image_srv_descriptor_write_count
	}

	pub(crate) fn image_uav_descriptor_write_count(&self) -> usize {
		self.image_uav_descriptor_write_count
	}

	pub(crate) fn acceleration_structure_descriptor_write_count(&self) -> usize {
		self.acceleration_structure_descriptor_write_count
	}

	#[cfg(test)]
	pub(crate) fn sampler_descriptor_write_records(&self) -> &[SamplerDescriptorWriteRecord] {
		&self.sampler_descriptor_write_records
	}

	pub(crate) fn pipeline_has_native_state(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.pipeline_state.is_some())
	}

	pub(crate) fn pipeline_state_bind_count(&self) -> usize {
		self.pipeline_state_bind_count
	}

	pub(crate) fn compute_pipeline_state_create_attempt_count(&self) -> usize {
		self.compute_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_create_attempt_count(&self) -> usize {
		self.graphics_pipeline_state_create_attempt_count
	}

	pub(crate) fn graphics_pipeline_state_last_error(&self) -> Option<i32> {
		self.graphics_pipeline_state_last_error
	}

	pub(crate) fn hlsl_specialization_compile_count(&self) -> usize {
		self.hlsl_specialization_compile_count
	}

	pub(crate) fn ray_tracing_state_object_create_attempt_count(&self) -> usize {
		self.ray_tracing_state_object_create_attempt_count
	}

	pub(crate) fn pipeline_has_ray_tracing_state_object(&self, pipeline: PipelineHandle) -> Option<bool> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.ray_tracing_state_object.is_some())
	}

	pub(crate) fn ray_tracing_shader_identifier_count(&self, pipeline: PipelineHandle) -> Option<usize> {
		self.pipelines
			.get(pipeline.0 as usize)
			.map(|pipeline| pipeline.ray_tracing_shader_identifiers.len())
	}

	/// Queries native 16-bit shader support once so pipeline compilation can use a stable capability.
	fn query_native_16_bit_shader_ops_support(device: &ID3D12Device) -> bool {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS4::default();
		let result = unsafe {
			device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS4,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS4).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS4>() as u32,
			)
		};
		result.is_ok() && options.Native16BitShaderOpsSupported.as_bool()
	}

	/// Reports the cached native 16-bit shader capability for backend policy decisions.
	pub(crate) fn supports_native_16_bit_shader_ops(&self) -> bool {
		self.native_16_bit_shader_ops_supported
	}

	pub(crate) fn supports_native_ray_tracing(&self) -> bool {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
		let result = unsafe {
			self.device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS5,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS5).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS5>() as u32,
			)
		};
		result.is_ok() && options.RaytracingTier != D3D12_RAYTRACING_TIER_NOT_SUPPORTED
	}

	pub(crate) fn supports_native_mesh_shaders(&self) -> bool {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS7::default();
		let result = unsafe {
			self.device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS7,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS7).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS7>() as u32,
			)
		};
		result.is_ok() && options.MeshShaderTier != D3D12_MESH_SHADER_TIER_NOT_SUPPORTED
	}

	pub(crate) fn compute_dispatch_encode_count(&self) -> usize {
		self.compute_dispatch_encode_count
	}

	pub(crate) fn indirect_dispatch_encode_count(&self) -> usize {
		self.indirect_dispatch_encode_count
	}

	pub(crate) fn trace_rays_record_count(&self) -> usize {
		self.trace_rays_record_count
	}

	pub(crate) fn mesh_dispatch_encode_count(&self) -> usize {
		self.mesh_dispatch_encode_count
	}

	pub(crate) fn vertex_buffer_bind_count(&self) -> usize {
		self.vertex_buffer_bind_count
	}

	pub(crate) fn index_buffer_bind_count(&self) -> usize {
		self.index_buffer_bind_count
	}

	pub(crate) fn draw_encode_count(&self) -> usize {
		self.draw_encode_count
	}

	pub(crate) fn draw_indexed_encode_count(&self) -> usize {
		self.draw_indexed_encode_count
	}

	pub(crate) fn render_target_bind_count(&self) -> usize {
		self.render_target_bind_count
	}

	pub(crate) fn render_target_clear_count(&self) -> usize {
		self.render_target_clear_count
	}

	pub(crate) fn render_pass_end_count(&self) -> usize {
		self.render_pass_end_count
	}

	pub(crate) fn depth_stencil_bind_count(&self) -> usize {
		self.depth_stencil_bind_count
	}

	pub(crate) fn depth_stencil_clear_count(&self) -> usize {
		self.depth_stencil_clear_count
	}

	pub(crate) fn viewport_set_count(&self) -> usize {
		self.viewport_set_count
	}

	pub(crate) fn scissor_set_count(&self) -> usize {
		self.scissor_set_count
	}

	pub(crate) fn primitive_topology_set_count(&self) -> usize {
		self.primitive_topology_set_count
	}

	pub(crate) fn swapchain_backbuffer_bind_count(&self) -> usize {
		self.swapchain_backbuffer_bind_count
	}

	pub(crate) fn swapchain_present_transition_count(&self) -> usize {
		self.swapchain_present_transition_count
	}

	pub(crate) fn uav_barrier_count(&self) -> usize {
		self.uav_barrier_count
	}

	pub(crate) fn acceleration_structure_resource_count(&self) -> usize {
		self.acceleration_structure_resource_count
	}

	pub(crate) fn native_acceleration_structure_resource_count(&self) -> usize {
		self.native_acceleration_structure_resource_count
	}

	pub(crate) fn acceleration_structure_instance_write_count(&self) -> usize {
		self.acceleration_structure_instance_write_count
	}

	pub(crate) fn shader_binding_table_write_count(&self) -> usize {
		self.shader_binding_table_write_count
	}

	pub(crate) fn top_level_acceleration_structure_build_record_count(&self) -> usize {
		self.top_level_acceleration_structure_build_record_count
	}

	pub(crate) fn bottom_level_acceleration_structure_build_record_count(&self) -> usize {
		self.bottom_level_acceleration_structure_build_record_count
	}

	pub(crate) fn native_top_level_acceleration_structure_build_encode_count(&self) -> usize {
		self.native_top_level_acceleration_structure_build_encode_count
	}

	pub(crate) fn native_bottom_level_acceleration_structure_build_encode_count(&self) -> usize {
		self.native_bottom_level_acceleration_structure_build_encode_count
	}

	pub(crate) fn acceleration_structure_size(&self, handle: TopLevelAccelerationStructureHandle) -> Option<usize> {
		self.top_level_acceleration_structures
			.get(handle.0 as usize)
			.map(|acceleration_structure| acceleration_structure.size)
	}

	pub(crate) fn acceleration_structure_gpu_address(&self, handle: TopLevelAccelerationStructureHandle) -> Option<u64> {
		self.top_level_acceleration_structures
			.get(handle.0 as usize)
			.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
	}

	pub(crate) fn bottom_level_acceleration_structure_size(
		&self,
		handle: BottomLevelAccelerationStructureHandle,
	) -> Option<usize> {
		self.bottom_level_acceleration_structures
			.get(handle.0 as usize)
			.map(|acceleration_structure| acceleration_structure.size)
	}

	pub(crate) fn bottom_level_acceleration_structure_gpu_address(
		&self,
		handle: BottomLevelAccelerationStructureHandle,
	) -> Option<u64> {
		self.bottom_level_acceleration_structures
			.get(handle.0 as usize)
			.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
	}

	pub fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
		// Stores sampler parameters without creating a DX12 descriptor.
		self.samplers.push(Sampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		});
		SamplerHandle((self.samplers.len() - 1) as u64)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		_name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>();
		let handle = self.create_buffer_with_layout(
			Layout::from_size_align(size, 16).unwrap(),
			Uses::Storage,
			DeviceAccesses::HostToDevice,
			BufferStorage::Static,
		);
		BaseBufferHandle(handle)
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		let size = self.top_level_acceleration_structure_size(max_instance_count);
		let (resource, native_resource) = self.create_acceleration_structure_resource(size);
		if resource.is_some() {
			self.acceleration_structure_resource_count += 1;
		}
		if native_resource {
			self.native_acceleration_structure_resource_count += 1;
		}
		self.top_level_acceleration_structures.push(AccelerationStructure {
			resource,
			size,
			native_resource,
		});
		TopLevelAccelerationStructureHandle((self.top_level_acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		let size = self.bottom_level_acceleration_structure_allocation_size(description);
		let (resource, native_resource) = self.create_acceleration_structure_resource(size);
		if resource.is_some() {
			self.acceleration_structure_resource_count += 1;
		}
		if native_resource {
			self.native_acceleration_structure_resource_count += 1;
		}
		self.bottom_level_acceleration_structures.push(AccelerationStructure {
			resource,
			size,
			native_resource,
		});
		BottomLevelAccelerationStructureHandle((self.bottom_level_acceleration_structures.len() - 1) as u64)
	}

	fn create_acceleration_structure_resource(&mut self, size: usize) -> (Option<ID3D12Resource>, bool) {
		if size == 0 {
			return (None, false);
		}

		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC {
			Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
			Alignment: 0,
			Width: size.max(1) as u64,
			Height: 1,
			DepthOrArraySize: 1,
			MipLevels: 1,
			Format: DXGI_FORMAT_UNKNOWN,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
			// DX12 acceleration structures are built through UAV writes, so the backing buffer must allow UAV access.
			Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
		};

		let mut resource: Option<ID3D12Resource> = None;
		let result = unsafe {
			self.device.CreateCommittedResource(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE,
				None,
				&mut resource,
			)
		};
		if result.is_ok() {
			return (resource, true);
		}

		let (resource, ..) = self.create_buffer_resource(size, DeviceAccesses::DeviceOnly);
		(resource, false)
	}

	fn top_level_acceleration_structure_size(&self, max_instance_count: u32) -> usize {
		let fallback = Self::align_up(max_instance_count as usize * 128, 256).max(256);
		self.ray_tracing_prebuild_result_size(D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: max_instance_count,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			// Prebuild checks whether GPUVA fields are null, so use a dummy non-null address before real buffers exist.
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 { InstanceDescs: 1 },
		})
		.unwrap_or(fallback)
	}

	fn bottom_level_acceleration_structure_allocation_size(&self, description: &BottomLevelAccelerationStructure) -> usize {
		let fallback = Self::bottom_level_acceleration_structure_estimated_size(description);
		let Some(geometry) = Self::bottom_level_geometry_desc_for_prebuild(description) else {
			return fallback;
		};
		self.ray_tracing_prebuild_result_size(D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: 1,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
				pGeometryDescs: &geometry,
			},
		})
		.unwrap_or(fallback)
	}

	fn ray_tracing_prebuild_result_size(&self, inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS) -> Option<usize> {
		let Ok(device) = self.device.cast::<ID3D12Device5>() else {
			return None;
		};
		let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
		unsafe {
			device.GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
		}
		(info.ResultDataMaxSizeInBytes > 0).then(|| Self::align_up(info.ResultDataMaxSizeInBytes as usize, 256).max(256))
	}

	fn bottom_level_geometry_desc_for_prebuild(
		description: &BottomLevelAccelerationStructure,
	) -> Option<D3D12_RAYTRACING_GEOMETRY_DESC> {
		match description.description {
			crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				vertex_position_encoding,
				triangle_count,
				index_format,
			} => {
				let vertex_format = match vertex_position_encoding {
					crate::Encodings::FloatingPoint => DXGI_FORMAT_R32G32B32_FLOAT,
					_ => return None,
				};
				let index_format = match index_format {
					DataTypes::U16 => DXGI_FORMAT_R16_UINT,
					DataTypes::U32 => DXGI_FORMAT_R32_UINT,
					_ => return None,
				};
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
							Transform3x4: 0,
							IndexFormat: index_format,
							VertexFormat: vertex_format,
							IndexCount: triangle_count.saturating_mul(3),
							VertexCount: vertex_count,
							// Prebuild does not read GPU memory but may check whether addresses are null.
							IndexBuffer: 1,
							VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: 1,
								StrideInBytes: std::mem::size_of::<[f32; 3]>() as u64,
							},
						},
					},
				})
			}
			crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
							AABBCount: transform_count as u64,
							AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								// Prebuild does not read GPU memory but may check whether addresses are null.
								StartAddress: 1,
								StrideInBytes: std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB>(
								) as u64,
							},
						},
					},
				})
			}
		}
	}

	fn bottom_level_acceleration_structure_estimated_size(description: &BottomLevelAccelerationStructure) -> usize {
		let size = match description.description {
			crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				triangle_count,
				..
			} => vertex_count as usize * 32 + triangle_count as usize * 64,
			crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => transform_count as usize * 128,
		};
		Self::align_up(size, 256).max(256)
	}

	/// Applies retained flat-slot descriptor writes to every frame-local set.
	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		for write in descriptor_set_writes {
			let set_handles = self.collect_descriptor_set_handles(DescriptorSetHandle(write.descriptor_set.0));
			for set_handle in set_handles {
				let retained = RetainedDescriptor {
					descriptor: write.descriptor,
					frame_offset: write.frame_offset.unwrap_or(0),
				};
				let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
				let previous = descriptor_set
					.descriptors
					.entry(write.slot)
					.or_default()
					.insert(write.array_element, retained);
				if previous != Some(retained) {
					descriptor_set.version = descriptor_set.version.wrapping_add(1);
				}
				self.materialize_descriptor_base_image_resource(set_handle, write.descriptor);
			}
		}
	}

	pub fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
		let Some(buffer) = self.buffer(instances_buffer_handle) else {
			return;
		};
		let descriptor_size = std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>();
		let offset = instance_index.saturating_mul(descriptor_size);
		if offset + descriptor_size > buffer.size {
			return;
		}
		let Some(bottom_level) = self
			.bottom_level_acceleration_structures
			.get(acceleration_structure.0 as usize)
		else {
			return;
		};
		let address = bottom_level
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0);
		let instance = D3D12_RAYTRACING_INSTANCE_DESC {
			Transform: [
				transform[0][0],
				transform[0][1],
				transform[0][2],
				transform[0][3],
				transform[1][0],
				transform[1][1],
				transform[1][2],
				transform[1][3],
				transform[2][0],
				transform[2][1],
				transform[2][2],
				transform[2][3],
			],
			_bitfield1: ((mask as u32) << 24) | (custom_index as u32 & 0x00ff_ffff),
			_bitfield2: ((D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0 as u32) << 24)
				| (sbt_record_offset as u32 & 0x00ff_ffff),
			AccelerationStructure: address,
		};
		unsafe {
			std::ptr::copy_nonoverlapping(
				(&instance as *const D3D12_RAYTRACING_INSTANCE_DESC).cast::<u8>(),
				buffer.data.add(offset),
				descriptor_size,
			);
		}
		Self::sync_buffer_storage(buffer);
		self.acceleration_structure_instance_write_count += 1;
	}

	pub fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	) {
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::RayTracing) || !pipeline.shaders.contains(&shader_handle) {
			return;
		}
		let Some(buffer) = self.buffer(sbt_buffer_handle) else {
			return;
		};
		let identifier = if let Some(identifier) = pipeline.ray_tracing_shader_identifiers.get(&shader_handle) {
			*identifier
		} else {
			if pipeline.ray_tracing_state_object.is_some() {
				self.log_dx12_error(format!(
					"Missing DX12 ray tracing shader identifier. The most likely cause is that the shader handle {} was not exported by the ray tracing state object.",
					shader_handle.0
				));
			}
			Self::placeholder_shader_identifier(pipeline_handle, shader_handle)
		};
		let end = sbt_record_offset.saturating_add(identifier.len());
		if end > buffer.size {
			return;
		}
		unsafe {
			std::ptr::copy_nonoverlapping(identifier.as_ptr(), buffer.data.add(sbt_record_offset), identifier.len());
		}
		Self::sync_buffer_storage(buffer);
		self.shader_binding_table_write_count += 1;
	}

	fn placeholder_shader_identifier(pipeline_handle: PipelineHandle, shader_handle: ShaderHandle) -> [u8; 32] {
		let mut identifier = [0u8; 32];
		identifier[0..8].copy_from_slice(b"DX12SBT\0");
		identifier[8..16].copy_from_slice(&pipeline_handle.0.to_le_bytes());
		identifier[16..24].copy_from_slice(&shader_handle.0.to_le_bytes());
		identifier
	}

	pub(crate) fn record_top_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::TopLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(acceleration_structure) = self
			.top_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if acceleration_structure.resource.is_none() {
			return;
		}
		let Some(scratch_resource) = self.buffer_resource_for_sequence(build.scratch_buffer.buffer, sequence_index) else {
			return;
		};

		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				build.scratch_buffer.buffer,
				&scratch_resource,
				D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		match build.description {
			crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, .. } => {
				if let Some(instance_resource) = self.buffer_resource_for_sequence(instances_buffer, sequence_index) {
					unsafe {
						self.transition_tracked_buffer(
							&command_list,
							instances_buffer,
							&instance_resource,
							D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
						);
					}
					self.mark_command_buffer_work(command_buffer_handle);
				}
			}
		}
		self.encode_top_level_acceleration_structure_build(command_buffer_handle, &command_list, build, sequence_index);
		self.top_level_acceleration_structure_build_record_count += 1;
	}

	pub(crate) fn record_bottom_level_acceleration_structure_builds(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
		sequence_index: u8,
	) {
		for build in builds {
			if self
				.bottom_level_acceleration_structures
				.get(build.acceleration_structure.0 as usize)
				.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
				.is_none()
			{
				continue;
			}
			if !self.prepare_bottom_level_build_inputs(command_buffer_handle, build, sequence_index) {
				continue;
			}
			self.encode_bottom_level_acceleration_structure_build(command_buffer_handle, build, sequence_index);
			self.bottom_level_acceleration_structure_build_record_count += 1;
		}
	}

	fn encode_top_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		build: &crate::rt::TopLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list4) = command_list.cast::<ID3D12GraphicsCommandList4>().ok() else {
			return;
		};
		let Some(acceleration_structure) = self
			.top_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if !acceleration_structure.native_resource {
			return;
		}
		let Some(destination_resource) = acceleration_structure.resource.clone() else {
			return;
		};
		let destination = unsafe { destination_resource.GetGPUVirtualAddress() };
		let scratch =
			self.buffer_address_for_sequence(build.scratch_buffer.buffer, sequence_index) + build.scratch_buffer.offset as u64;
		if destination == 0 || scratch == 0 {
			return;
		}
		let crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
			instances_buffer,
			instance_count,
		} = build.description;
		let Some(instances_resource) = self.acceleration_structure_build_input_resource(
			command_buffer_handle,
			command_list,
			instances_buffer,
			sequence_index,
		) else {
			return;
		};
		let instances = unsafe { instances_resource.GetGPUVirtualAddress() };
		if instances == 0 {
			return;
		}
		let desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
			DestAccelerationStructureData: destination,
			Inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
				Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
				Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
				NumDescs: instance_count,
				DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
				Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
					InstanceDescs: instances,
				},
			},
			SourceAccelerationStructureData: 0,
			ScratchAccelerationStructureData: scratch,
		};
		unsafe {
			command_list4.BuildRaytracingAccelerationStructure(&desc, None);
			// DXR builds write through UAVs. The barrier makes the built TLAS visible to DispatchRays.
			Self::unordered_access_barrier_all(command_list);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.uav_barrier_count += 1;
		self.native_top_level_acceleration_structure_build_encode_count += 1;
	}

	fn encode_bottom_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(command_list4) = command_list.cast::<ID3D12GraphicsCommandList4>().ok() else {
			return;
		};
		let Some(acceleration_structure) = self
			.bottom_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if !acceleration_structure.native_resource {
			return;
		}
		let Some(destination_resource) = acceleration_structure.resource.clone() else {
			return;
		};
		let destination = unsafe { destination_resource.GetGPUVirtualAddress() };
		let scratch =
			self.buffer_address_for_sequence(build.scratch_buffer.buffer, sequence_index) + build.scratch_buffer.offset as u64;
		let Some(geometry) =
			self.bottom_level_geometry_desc(command_buffer_handle, &command_list, &build.description, sequence_index)
		else {
			return;
		};
		if destination == 0 || scratch == 0 {
			return;
		}
		let desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
			DestAccelerationStructureData: destination,
			Inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
				Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
				Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
				NumDescs: 1,
				DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
				Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
					pGeometryDescs: &geometry,
				},
			},
			SourceAccelerationStructureData: 0,
			ScratchAccelerationStructureData: scratch,
		};
		unsafe {
			command_list4.BuildRaytracingAccelerationStructure(&desc, None);
			// DXR builds write through UAVs. The barrier makes the built BLAS visible to later TLAS builds.
			Self::unordered_access_barrier_all(&command_list);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.uav_barrier_count += 1;
		self.native_bottom_level_acceleration_structure_build_encode_count += 1;
	}

	fn bottom_level_geometry_desc(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		description: &crate::rt::BottomLevelAccelerationStructureBuildDescriptions,
		sequence_index: u8,
	) -> Option<D3D12_RAYTRACING_GEOMETRY_DESC> {
		match description {
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
				vertex_buffer,
				vertex_count,
				vertex_position_encoding,
				index_buffer,
				triangle_count,
				index_format,
			} => {
				let vertex_format = match vertex_position_encoding {
					crate::Encodings::FloatingPoint => DXGI_FORMAT_R32G32B32_FLOAT,
					_ => return None,
				};
				let index_format = match index_format {
					DataTypes::U16 => DXGI_FORMAT_R16_UINT,
					DataTypes::U32 => DXGI_FORMAT_R32_UINT,
					_ => return None,
				};
				let vertex_resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					vertex_buffer.buffer_offset.buffer,
					sequence_index,
				)?;
				let index_resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					index_buffer.buffer_offset.buffer,
					sequence_index,
				)?;
				let vertex_address =
					unsafe { vertex_resource.GetGPUVirtualAddress() } + vertex_buffer.buffer_offset.offset as u64;
				let index_address = unsafe { index_resource.GetGPUVirtualAddress() } + index_buffer.buffer_offset.offset as u64;
				if vertex_address == 0 || index_address == 0 {
					return None;
				}
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
							Transform3x4: 0,
							IndexFormat: index_format,
							VertexFormat: vertex_format,
							IndexCount: triangle_count.saturating_mul(3),
							VertexCount: *vertex_count,
							IndexBuffer: index_address,
							VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: vertex_address,
								StrideInBytes: vertex_buffer.stride as u64,
							},
						},
					},
				})
			}
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB {
				aabb_buffer,
				transform_count,
				..
			} => {
				let resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					*aabb_buffer,
					sequence_index,
				)?;
				let address = unsafe { resource.GetGPUVirtualAddress() };
				if address == 0 {
					return None;
				}
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
							AABBCount: *transform_count as u64,
							AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: address,
								StrideInBytes: std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB>(
								) as u64,
							},
						},
					},
				})
			}
		}
	}

	fn acceleration_structure_build_input_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		self.sync_buffer_for_sequence(buffer_handle, sequence_index);
		let source = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let heap_kind = self.buffer_heap_kind_for_sequence(buffer_handle, sequence_index)?;
		if heap_kind == BufferHeapKind::Default {
			unsafe {
				self.transition_tracked_buffer(
					command_list,
					buffer_handle,
					&source,
					D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
				);
			}
			return Some(source);
		}

		let size = self.buffer(buffer_handle)?.size;
		let (Some(staged), ..) = self.create_buffer_resource(size, DeviceAccesses::DeviceOnly) else {
			return Some(source);
		};
		unsafe {
			self.transition_tracked_buffer(command_list, buffer_handle, &staged, D3D12_RESOURCE_STATE_COPY_DEST);
			command_list.CopyBufferRegion(&staged, 0, &source, 0, size as u64);
			self.transition_tracked_buffer(
				command_list,
				buffer_handle,
				&staged,
				D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.buffer_copy_count += 1;
		self.retain_command_buffer_upload_resource(command_buffer_handle, staged.clone());
		Some(staged)
	}

	fn prepare_bottom_level_build_inputs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
		sequence_index: u8,
	) -> bool {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return false;
		};
		let Some(scratch_resource) = self.buffer_resource_for_sequence(build.scratch_buffer.buffer, sequence_index) else {
			return false;
		};
		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				build.scratch_buffer.buffer,
				&scratch_resource,
				D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);

		let mut transition_input = |buffer_handle: BaseBufferHandle| {
			let Some(resource) = self.buffer_resource_for_sequence(buffer_handle, sequence_index) else {
				return false;
			};
			unsafe {
				self.transition_tracked_buffer(
					&command_list,
					buffer_handle,
					&resource,
					D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
				);
			}
			true
		};

		let inputs_ready = match &build.description {
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
				vertex_buffer,
				index_buffer,
				..
			} => transition_input(vertex_buffer.buffer_offset.buffer) && transition_input(index_buffer.buffer_offset.buffer),
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB {
				aabb_buffer,
				transform_buffer,
				..
			} => transition_input(*aabb_buffer) && transition_input(*transform_buffer),
		};
		if inputs_ready {
			self.mark_command_buffer_work(command_buffer_handle);
		}
		inputs_ready
	}

	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		let extent = Self::query_window_extent(window_os_handles, fallback_extent);
		let image_count = self.frames.max(2);

		let queue = self
			.queues
			.iter()
			.find(|queue| queue.queue_type == D3D12_COMMAND_LIST_TYPE_DIRECT)
			.or_else(|| self.queues.first())
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");

		let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI factory. The most likely cause is that the DXGI runtime is unavailable.");
		});

		let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
			Width: extent.width(),
			Height: extent.height(),
			Format: DXGI_FORMAT_B8G8R8A8_UNORM,
			Stereo: false.into(),
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
			BufferCount: image_count as u32,
			Scaling: DXGI_SCALING_STRETCH,
			SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
			AlphaMode: DXGI_ALPHA_MODE_IGNORE,
			Flags: 0,
		};

		let swapchain = unsafe { factory.CreateSwapChainForHwnd(&queue.queue, window_os_handles.hwnd, &swapchain_desc, None, None) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI swapchain. The most likely cause is that the window handle is invalid or the device does not support the swapchain format.");
		});

		let swapchain: IDXGISwapChain3 = swapchain.cast().unwrap_or_else(|_| {
			panic!(
				"Failed to upgrade the DXGI swapchain. The most likely cause is that the DXGI runtime does not support IDXGISwapChain3."
			);
		});

		let _ = unsafe { factory.MakeWindowAssociation(window_os_handles.hwnd, DXGI_MWA_NO_ALT_ENTER) };

		self.swapchains.push(Swapchain {
			handles: window::Handles {
				hinstance: window_os_handles.hinstance,
				hwnd: window_os_handles.hwnd,
			},
			swapchain,
			extent,
			image_count,
			next_image_index: 0,
			present_mode: presentation_mode,
			images: std::array::from_fn(|_| None),
			proxy_uses: std::array::from_fn(|_| Uses::empty()),
			backbuffers: std::array::from_fn(|_| None),
			acquired_image_indices: [0; 8],
		});

		SwapchainHandle((self.swapchains.len() - 1) as u64)
	}

	pub fn create_factory(&mut self) -> Option<crate::dx12::factory::Factory> {
		Some(crate::dx12::factory::Factory::default())
	}

	pub fn get_swapchain_image(&mut self, swapchain_handle: SwapchainHandle, uses: Uses) -> (ImageHandle, Formats) {
		let needs_new_proxy = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			swapchain.images[0].is_none() || !swapchain.proxy_uses[0].contains(uses)
		};

		if needs_new_proxy {
			let extent = self.swapchains[swapchain_handle.0 as usize].extent;
			let mut images = [None; 8];
			for image_index in 0..8 {
				let image = self.build_image(
					crate::image::Builder::new(Formats::BGRAu8, uses | Uses::BlitSource)
						.extent(extent)
						.device_accesses(DeviceAccesses::DeviceOnly)
						.use_case(crate::UseCases::DYNAMIC),
				);
				images[image_index] = Some(image);
			}
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			swapchain.images = images;
			swapchain.proxy_uses = [uses; 8];
		}
		if needs_new_proxy {
			self.invalidate_descriptor_materializations();
		}

		(
			self.swapchains[swapchain_handle.0 as usize].images[0].expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub(crate) fn get_swapchain_image_for_sequence(
		&mut self,
		swapchain_handle: SwapchainHandle,
		uses: Uses,
		sequence_index: u8,
	) -> (ImageHandle, Formats) {
		self.get_swapchain_image(swapchain_handle, uses);
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		let image_index = sequence_index as usize;
		(
			swapchain.images[image_index].or(swapchain.images[0]).expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
		self.texture_copies
			.get(texture_copy_handle.0 as usize)
			.map(|v| v.as_slice())
			.unwrap_or(&[])
	}

	fn wait_for_texture_copy_readback(&mut self, texture_copy_handle: TextureCopyHandle) {
		let Some(sequence_index) = self
			.texture_readbacks
			.iter()
			.find(|readback| readback.texture_copy == texture_copy_handle && !readback.resolved)
			.map(|readback| readback.sequence_index)
		else {
			return;
		};
		let synchronizers = self
			.command_buffers
			.iter()
			.filter_map(|command_buffer| match command_buffer.last_submission {
				Some((synchronizer, submitted_sequence)) if submitted_sequence == sequence_index => Some(synchronizer),
				_ => None,
			})
			.collect::<SmallVec<[_; 4]>>();
		for synchronizer in synchronizers {
			self.wait_for_synchronizer_sequence(synchronizer, sequence_index);
		}
	}

	fn create_synchronizer_internal(&mut self, signaled: bool) -> crate::synchronizer::SynchronizerHandle {
		let handle = crate::synchronizer::SynchronizerHandle(self.synchronizers.len() as u64);
		let initial_value = if signaled { 1 } else { 0 };
		let fence = unsafe { self.device.CreateFence(initial_value, D3D12_FENCE_FLAGS(0)) }
			.expect("Failed to create a D3D12 fence. The most likely cause is that the device does not support fences.");
		self.synchronizers.push(Synchronizer {
			next: None,
			fence,
			value: initial_value,
		});
		handle
	}

	pub fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let master = SynchronizerHandle(self.synchronizers.len() as u64);
		let mut previous: Option<crate::synchronizer::SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.0 as usize].next = Some(handle);
			}
			previous = Some(handle);
		}
		master
	}

	pub fn start_frame<'a>(&'a mut self, index: u32, _synchronizer_handle: SynchronizerHandle) -> super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % self.frames as u32) as u8,
		};
		self.wait_for_synchronizer_sequence(_synchronizer_handle, frame_key.sequence_index);
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		// Resizes CPU-side buffer storage while discarding previous per-frame contents.
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let (current_size, current_layout, current_data, current_access, current_uses, retired_state_keys) = {
			let buffer = self.buffer(buffer_handle).expect(
				"Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.",
			);
			let mut retired_state_keys = SmallVec::<[usize; 4]>::new();
			retired_state_keys.extend(buffer.resource.as_ref().map(Self::native_resource_key));
			if let Some(frame_resources) = buffer.frame_resources.as_ref() {
				retired_state_keys.extend(
					frame_resources
						.iter()
						.flatten()
						.filter_map(|frame| frame.resource.as_ref())
						.map(Self::native_resource_key),
				);
			}
			(
				buffer.size,
				buffer.layout,
				buffer.data,
				buffer.access,
				buffer.uses,
				retired_state_keys,
			)
		};

		if current_size >= size {
			return;
		}

		let layout = Layout::from_size_align(size, current_layout.align()).unwrap();
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to resize buffer storage. The most likely cause is that the system is out of memory.");
		}

		if current_layout.size() != 0 && !current_data.is_null() {
			unsafe {
				alloc::dealloc(current_data, current_layout);
			}
		}

		let frame_count = self.frames as usize;
		let resource_size = Self::buffer_resource_size(size, current_uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, current_access);
		for key in retired_state_keys {
			self.buffer_states.remove(&key);
		}
		let buffer = self
			.buffer_mut(buffer_handle)
			.expect("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		buffer.data = data;
		buffer.layout = layout;
		buffer.size = size;
		buffer.resource = resource;
		buffer.mapped = mapped;
		buffer.heap_kind = heap_kind;
		if let Some(frame_resources) = buffer.frame_resources.as_mut() {
			frame_resources.clear();
			frame_resources.resize_with(frame_count, || None);
		}
		self.invalidate_descriptor_materializations();
	}

	pub fn start_frame_capture(&mut self) {
		self.debugger.start_frame_capture();
	}

	pub fn end_frame_capture(&mut self) {
		self.debugger.end_frame_capture();
	}

	pub fn wait(&self) {
		for index in 0..self.synchronizers.len() {
			self.wait_for_private_synchronizer(crate::synchronizer::SynchronizerHandle(index as u64));
		}
	}

	fn synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		crate::synchronizer::SynchronizerHandle(synchronizer_handle.0).get_all(&self.synchronizers)
	}

	fn synchronizer_for_sequence(
		&self,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) -> Option<crate::synchronizer::SynchronizerHandle> {
		let handles = self.synchronizer_handles(synchronizer_handle);
		handles
			.get(sequence_index as usize)
			.copied()
			.or_else(|| handles.last().copied())
	}

	fn wait_for_private_synchronizer(&self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
			return;
		};
		while unsafe { synchronizer.fence.GetCompletedValue() } < synchronizer.value {
			std::thread::yield_now();
		}
	}

	pub(crate) fn wait_for_synchronizer(&mut self, synchronizer_handle: SynchronizerHandle) {
		for handle in self.synchronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
		self.refresh_readback_texture_copies(None);
	}

	pub(crate) fn wait_for_synchronizer_sequence(&mut self, synchronizer_handle: SynchronizerHandle, sequence_index: u8) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.wait_for_private_synchronizer(handle);
		self.refresh_readback_texture_copies(Some(sequence_index));
	}

	pub(crate) fn synchronizer_value(&self, synchronizer_handle: SynchronizerHandle) -> Option<u64> {
		self.synchronizers
			.get(synchronizer_handle.0 as usize)
			.map(|synchronizer| synchronizer.value)
	}

	pub(crate) fn begin_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle, sequence_index: u8) {
		if let Some((synchronizer_handle, previous_sequence_index)) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.last_submission)
		{
			self.wait_for_synchronizer_sequence(synchronizer_handle, previous_sequence_index);
		}

		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		let (Some(allocator), Some(command_list)) = (command_buffer.allocator.as_ref(), command_buffer.command_list.as_ref())
		else {
			return;
		};

		if command_buffer.is_open {
			let _ = unsafe { command_list.Close() };
			command_buffer.is_open = false;
		}
		command_buffer.recorded_work = false;
		command_buffer.sequence_index = sequence_index;
		command_buffer.last_submission = None;
		let _ = unsafe { allocator.Reset() };
		let _ = unsafe { command_list.Reset(allocator, None) };
		// Reset removes recorded references before fence-complete transient resources and heaps are released.
		command_buffer.retained_descriptor_heaps.clear();
		command_buffer.retained_resources.clear();
		command_buffer.retained_upload_resource_count = 0;
		if let Some(arena) = command_buffer.cbv_srv_uav_staging_heap.as_mut() {
			arena.used = 0;
		}
		if let Some(arena) = command_buffer.sampler_staging_heap.as_mut() {
			arena.used = 0;
		}
		command_buffer.is_open = true;
		// Resetting an unsubmitted command list discards its copies, so its pending readbacks have no future completion.
		self.texture_readbacks
			.retain(|readback| readback.command_buffer_handle != command_buffer_handle);
	}

	/// Marks a command buffer as containing GPU-visible work that must be submitted.
	fn mark_command_buffer_work(&mut self, command_buffer_handle: CommandBufferHandle) {
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.recorded_work = true;
		}
	}

	pub(crate) fn bind_pipeline_root_signature(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let Some(root_signature) = self
			.pipeline_root_signatures
			.get(pipeline.layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())
		else {
			return;
		};

		unsafe {
			match pipeline.kind {
				PipelineKind::Compute | PipelineKind::RayTracing => command_list.SetComputeRootSignature(&root_signature),
				PipelineKind::Raster => command_list.SetGraphicsRootSignature(&root_signature),
			}
		}
		self.root_signature_bind_count += 1;
	}

	pub(crate) fn bind_pipeline_state(&mut self, command_buffer_handle: CommandBufferHandle, pipeline_handle: PipelineHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline_state) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.and_then(|pipeline| pipeline.pipeline_state.clone())
		else {
			return;
		};

		unsafe {
			command_list.SetPipelineState(&pipeline_state);
		}
		self.pipeline_state_bind_count += 1;
	}

	pub(crate) fn bind_pipeline_native_state(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
	) {
		self.bind_pipeline_root_signature(command_buffer_handle, pipeline_handle);
		self.bind_pipeline_state(command_buffer_handle, pipeline_handle);
		self.bind_ray_tracing_state_object(command_buffer_handle, pipeline_handle);
		self.bind_primitive_topology(command_buffer_handle, pipeline_handle);
	}

	fn bind_ray_tracing_state_object(&mut self, command_buffer_handle: CommandBufferHandle, pipeline_handle: PipelineHandle) {
		let Some(state_object) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.and_then(|pipeline| pipeline.ray_tracing_state_object.clone())
		else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
			.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList4>().ok())
		else {
			return;
		};
		unsafe {
			command_list.SetPipelineState1(&state_object);
		}
		self.pipeline_state_bind_count += 1;
	}

	fn bind_primitive_topology(&mut self, command_buffer_handle: CommandBufferHandle, pipeline_handle: PipelineHandle) {
		let Some(Pipeline {
			kind: PipelineKind::Raster,
			..
		}) = self.pipelines.get(pipeline_handle.0 as usize)
		else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		unsafe {
			command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
		}
		self.primitive_topology_set_count += 1;
	}

	pub(crate) fn dispatch_compute_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		dispatch: DispatchExtent,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::Compute) || pipeline.pipeline_state.is_none() {
			return;
		}
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let extent = dispatch.get_extent();
		unsafe {
			command_list.Dispatch(extent.width(), extent.height(), extent.depth());
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.compute_dispatch_encode_count += 1;
	}

	/// Encodes a native DX12 indirect compute dispatch command.
	pub(crate) fn dispatch_compute_indirect_native<const N: usize>(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handle: BufferHandle<[[u32; 4]; N]>,
		entry_index: usize,
	) {
		let base_buffer_handle: BaseBufferHandle = buffer_handle.into();
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(buffer) = self.buffer(base_buffer_handle) else {
			return;
		};
		let Some(resource) = buffer.resource.clone() else {
			return;
		};
		let Some(command_signature) = self.indirect_dispatch_command_signature() else {
			return;
		};
		let argument_offset = (entry_index * std::mem::size_of::<[u32; 4]>()) as u64;

		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				base_buffer_handle,
				&resource,
				D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
			);
			command_list.ExecuteIndirect(&command_signature, 1, &resource, argument_offset, None, 0);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.indirect_dispatch_encode_count += 1;
	}

	fn indirect_dispatch_command_signature(&mut self) -> Option<ID3D12CommandSignature> {
		if let Some(command_signature) = self.indirect_dispatch_signature.clone() {
			return Some(command_signature);
		}

		let argument = D3D12_INDIRECT_ARGUMENT_DESC {
			Type: D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
			Anonymous: D3D12_INDIRECT_ARGUMENT_DESC_0::default(),
		};
		let description = D3D12_COMMAND_SIGNATURE_DESC {
			ByteStride: std::mem::size_of::<[u32; 4]>() as u32,
			NumArgumentDescs: 1,
			pArgumentDescs: &argument,
			NodeMask: 0,
		};
		let mut command_signature: Option<ID3D12CommandSignature> = None;
		unsafe {
			self.device
				.CreateCommandSignature(&description, None, &mut command_signature)
				.ok()?;
		}
		let command_signature = command_signature?;
		self.indirect_dispatch_signature = Some(command_signature.clone());
		Some(command_signature)
	}

	/// Records DX12 ray dispatch metadata from GHI shader binding table ranges.
	pub(crate) fn trace_rays_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		binding_tables: crate::rt::BindingTables,
		x: u32,
		y: u32,
		z: u32,
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::RayTracing) {
			return;
		}
		let state_object = pipeline.ray_tracing_state_object.clone();
		if self.command_buffers.get(command_buffer_handle.0 as usize).is_none() {
			return;
		}
		let Some(raygen) = self.ray_generation_shader_record(binding_tables.raygen, sequence_index) else {
			return;
		};
		let Some(miss) = self.shader_table_range(binding_tables.miss, sequence_index) else {
			return;
		};
		let Some(hit) = self.shader_table_range(binding_tables.hit, sequence_index) else {
			return;
		};
		let callable = if let Some(callable) = binding_tables.callable {
			let Some(callable) = self.shader_table_range(callable, sequence_index) else {
				return;
			};
			callable
		} else {
			D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::default()
		};

		let _desc = D3D12_DISPATCH_RAYS_DESC {
			RayGenerationShaderRecord: raygen,
			MissShaderTable: miss,
			HitGroupTable: hit,
			CallableShaderTable: callable,
			Width: x,
			Height: y,
			Depth: z,
		};
		if state_object.is_some() {
			if let Some(command_list) = self
				.command_buffers
				.get(command_buffer_handle.0 as usize)
				.and_then(|command_buffer| command_buffer.command_list.clone())
				.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList4>().ok())
			{
				unsafe {
					command_list.DispatchRays(&_desc);
				}
				self.mark_command_buffer_work(command_buffer_handle);
			}
		}
		self.trace_rays_record_count += 1;
	}

	fn ray_generation_shader_record(
		&mut self,
		range: BufferStridedRange,
		sequence_index: u8,
	) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE> {
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
			StartAddress: self.shader_table_address(&range, sequence_index)?,
			SizeInBytes: range.size as u64,
		})
	}

	fn shader_table_range(
		&mut self,
		range: BufferStridedRange,
		sequence_index: u8,
	) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE> {
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
			StartAddress: self.shader_table_address(&range, sequence_index)?,
			SizeInBytes: range.size as u64,
			StrideInBytes: range.stride as u64,
		})
	}

	fn shader_table_address(&mut self, range: &BufferStridedRange, sequence_index: u8) -> Option<u64> {
		let address = self.buffer_address_for_sequence(range.buffer_offset.buffer, sequence_index);
		if address == 0 {
			return None;
		}
		Some(address + range.buffer_offset.offset as u64)
	}

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

		let mut views = Vec::with_capacity(buffer_descriptors.len());
		for buffer_descriptor in buffer_descriptors {
			let Some(resource) = self.buffer_resource_for_sequence(buffer_descriptor.buffer, sequence_index) else {
				continue;
			};
			let Some(buffer) = self.buffer(buffer_descriptor.buffer) else {
				continue;
			};
			let size_in_bytes = buffer.size.saturating_sub(buffer_descriptor.offset).min(u32::MAX as usize) as u32;
			unsafe {
				self.transition_tracked_buffer(
					&command_list,
					buffer_descriptor.buffer,
					&resource,
					D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
				);
			}
			views.push(D3D12_VERTEX_BUFFER_VIEW {
				BufferLocation: unsafe { resource.GetGPUVirtualAddress() } + buffer_descriptor.offset as u64,
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
		let format = match buffer_descriptor.index_type {
			Some(DataTypes::U16) => DXGI_FORMAT_R16_UINT,
			Some(DataTypes::U32) => DXGI_FORMAT_R32_UINT,
			Some(_) => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
			None => panic!(
				"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32)."
			),
		};
		let view = D3D12_INDEX_BUFFER_VIEW {
			BufferLocation: unsafe { resource.GetGPUVirtualAddress() } + buffer_descriptor.offset as u64,
			SizeInBytes: buffer.size.saturating_sub(buffer_descriptor.offset).min(u32::MAX as usize) as u32,
			Format: format,
		};

		unsafe {
			self.transition_tracked_buffer(
				&command_list,
				buffer_descriptor.buffer,
				&resource,
				D3D12_RESOURCE_STATE_INDEX_BUFFER,
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
	fn retained_render_target_view(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		self.materialize_render_target_views(resource, format, array_layers);
		Self::validate_attachment_layer(array_layers, layer);
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
		let slot = Self::attachment_descriptor_slot(array_layers, layer);
		let handle = self.descriptor_cpu_handle(&view, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, slot);
		self.retain_descriptor_heap(command_buffer_handle, &view);
		handle
	}

	/// Returns a stable DSV descriptor for one native resource view, creating it on first use.
	fn retained_depth_stencil_view(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		self.materialize_depth_stencil_views(resource, format, array_layers);
		Self::validate_attachment_layer(array_layers, layer);
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
		let slot = Self::attachment_descriptor_slot(array_layers, layer);
		let handle = self.descriptor_cpu_handle(&view, D3D12_DESCRIPTOR_HEAP_TYPE_DSV, slot);
		self.retain_descriptor_heap(command_buffer_handle, &view);
		handle
	}

	/// Materializes every RTV descriptor for one image in a single retained heap.
	fn materialize_render_target_views(&mut self, resource: &ID3D12Resource, format: Formats, array_layers: u32) {
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
			let layer = Self::attachment_descriptor_layer(array_layers, slot);
			let descriptor = Self::render_target_view_desc(format, array_layers, layer);
			let handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, slot);
			unsafe {
				self.device.CreateRenderTargetView(resource, Some(&descriptor), handle);
			}
		}
		self.render_target_views.insert(key, CpuDescriptorView { heap });
		self.render_target_view_allocation_count += 1;
	}

	/// Materializes every DSV descriptor for one image in a single retained heap.
	fn materialize_depth_stencil_views(&mut self, resource: &ID3D12Resource, format: Formats, array_layers: u32) {
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
			let layer = Self::attachment_descriptor_layer(array_layers, slot);
			let descriptor = Self::depth_stencil_view_desc(format, array_layers, layer);
			let handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_DSV, slot);
			unsafe {
				self.device.CreateDepthStencilView(resource, Some(&descriptor), handle);
			}
		}
		self.depth_stencil_views.insert(key, CpuDescriptorView { heap });
		self.depth_stencil_view_allocation_count += 1;
	}

	/// Materializes attachment descriptors alongside a newly created image resource.
	fn materialize_image_attachment_views(
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
	fn create_attachment_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
		purpose: &str,
	) -> ID3D12DescriptorHeap {
		let descriptor = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: Default::default(),
			NodeMask: 0,
		};
		match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&descriptor) } {
			Ok(heap) => heap,
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
			if format == Formats::Depth32 {
				let image_handle = self.attachment_image_handle(attachment, sequence_index);
				self.set_image_optimized_clear_value(image_handle, attachment.clear);
				let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
					continue;
				};
				let Some(image) = self.images.get(image_handle.0 as usize) else {
					continue;
				};
				depth_resource = Some((
					image_handle,
					resource,
					image.format,
					image.array_layers,
					attachment.layer,
					attachment.load,
					attachment.clear,
				));
				continue;
			}
			if let ImageOrSwapchain::Image(image_handle) = attachment.target {
				self.set_image_optimized_clear_value(image_handle, attachment.clear);
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
			target_resources.push(RenderTargetAttachment {
				image_handle,
				resource,
				format,
				array_layers,
				layer: attachment.layer,
				load: attachment.load,
				clear: attachment.clear,
				swapchain_backbuffer,
			});
		}

		if target_resources.is_empty() && depth_resource.is_none() {
			return;
		}

		// Plan attachment transitions before recording any clears so independent attachments share
		// one native ResourceBarrier call. Integer render targets transition through UAV in their clear.
		let mut attachment_barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		for target in &target_resources {
			let state = if !target.load && matches!(target.clear, ClearValue::Integer(..)) && target.format == Formats::U32 {
				D3D12_RESOURCE_STATE_UNORDERED_ACCESS
			} else {
				D3D12_RESOURCE_STATE_RENDER_TARGET
			};
			unsafe {
				if let Some(image_handle) = target.image_handle {
					self.transition_tracked_image_into(image_handle, &target.resource, state, &mut attachment_barriers);
				} else {
					attachment_barriers.push(Self::transition_resource_barrier(
						&target.resource,
						D3D12_RESOURCE_STATE_PRESENT,
						D3D12_RESOURCE_STATE_RENDER_TARGET,
					));
				}
			}
		}
		if let Some((image_handle, resource, ..)) = &depth_resource {
			unsafe {
				self.transition_tracked_image_into(
					*image_handle,
					resource,
					D3D12_RESOURCE_STATE_DEPTH_WRITE,
					&mut attachment_barriers,
				);
			}
		}
		unsafe {
			Self::submit_resource_barriers(&command_list, &attachment_barriers);
		}

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
					load,
					clear,
					swapchain_backbuffer,
				} = target;
				let handle = self.retained_render_target_view(command_buffer_handle, &resource, format, array_layers, layer);
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

		let mut post_clear_barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		for (image_handle, resource) in integer_clear_targets {
			unsafe {
				self.transition_tracked_image_into(
					image_handle,
					&resource,
					D3D12_RESOURCE_STATE_RENDER_TARGET,
					&mut post_clear_barriers,
				);
			}
		}
		unsafe {
			Self::submit_resource_barriers(&command_list, &post_clear_barriers);
		}

		let mut depth_handle = None;
		if let Some((_, resource, format, array_layers, layer, load, clear)) = depth_resource {
			let handle = self.retained_depth_stencil_view(command_buffer_handle, &resource, format, array_layers, layer);
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

	pub(crate) fn bind_descriptor_heaps(&mut self, command_buffer_handle: CommandBufferHandle, sets: &[DescriptorSetHandle]) {
		self.bind_descriptor_heaps_and_tables(command_buffer_handle, None, sets, 0);
	}

	/// Transitions the concrete resources referenced by the active pipeline's retained set union.
	pub(crate) fn flush_pending_descriptor_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(layout) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.and_then(|pipeline| self.pipeline_layouts.get(pipeline.layout.0 as usize))
			.cloned()
		else {
			return;
		};

		let mut retained = SmallVec::<[(ShaderResourceDescriptor, RetainedDescriptor); 32]>::new();
		for resource in &layout.resources {
			for &set_handle in sets {
				let Some(set_handle) = self.descriptor_set_for_sequence(set_handle, sequence_index) else {
					continue;
				};
				let Some(descriptors) = self.descriptor_sets[set_handle.0 as usize]
					.descriptors
					.get(&resource.descriptor.slot())
				else {
					continue;
				};
				retained.extend(
					descriptors
						.values()
						.copied()
						.map(|descriptor| (resource.descriptor, descriptor)),
				);
			}
		}

		// Complete deferred uploads before collecting barriers. Holding a batch across a copy command
		// would move an earlier transition past the command that depends on it.
		for (_, retained_descriptor) in &retained {
			let resource_sequence = self.frame_index_with_offset(
				sequence_index as usize,
				Some(retained_descriptor.frame_offset),
				self.frames as usize,
			) as u8;
			match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => self.sync_buffer_for_sequence(handle, resource_sequence),
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => self.flush_pending_texture_syncs(command_buffer_handle, Some(handle), Some(resource_sequence)),
				_ => {}
			}
		}

		let mut barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		for (resource_descriptor, retained_descriptor) in retained {
			let resource_sequence = self.frame_index_with_offset(
				sequence_index as usize,
				Some(retained_descriptor.frame_offset),
				self.frames as usize,
			) as u8;
			match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => {
					// Buffer contents can change without changing the retained descriptor or its native heap.
					let Some(resource) = self.buffer_resource_for_sequence(handle, resource_sequence) else {
						continue;
					};
					if self.buffer_heap_kind_for_sequence(handle, resource_sequence) != Some(BufferHeapKind::Default) {
						continue;
					}
					unsafe {
						self.transition_tracked_buffer_into(
							handle,
							&resource,
							Self::descriptor_buffer_state(resource_descriptor),
							&mut barriers,
						);
					}
					self.mark_command_buffer_work(command_buffer_handle);
				}
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => {
					let Some(resource) = self.ensure_image_resource_for_sequence(handle, resource_sequence) else {
						continue;
					};
					unsafe {
						self.transition_tracked_image_into(
							handle,
							&resource,
							Self::descriptor_image_state(resource_descriptor),
							&mut barriers,
						);
					}
					self.mark_command_buffer_work(command_buffer_handle);
				}
				WriteData::Swapchain(handle) => {
					let image = self
						.get_swapchain_image_for_sequence(handle, Uses::Storage, resource_sequence)
						.0;
					let Some(resource) = self.ensure_image_resource_for_sequence(image.into(), resource_sequence) else {
						continue;
					};
					unsafe {
						self.transition_tracked_image_into(
							image.into(),
							&resource,
							Self::descriptor_image_state(resource_descriptor),
							&mut barriers,
						);
					}
					self.mark_command_buffer_work(command_buffer_handle);
				}
				_ => {}
			}
		}
		unsafe {
			Self::submit_resource_barriers(&command_list, &barriers);
		}
	}

	fn descriptor_matches_kind(descriptor: WriteData, kind: ResourceKind) -> bool {
		match descriptor {
			WriteData::Buffer { .. } => matches!(kind, ResourceKind::UniformBuffer | ResourceKind::StorageBuffer),
			WriteData::Image { .. } | WriteData::Swapchain(_) => {
				matches!(
					kind,
					ResourceKind::SampledImage | ResourceKind::StorageImage | ResourceKind::InputAttachment
				)
			}
			WriteData::CombinedImageSampler { .. } => kind == ResourceKind::CombinedImageSampler,
			WriteData::Sampler(_) => kind == ResourceKind::Sampler,
			WriteData::AccelerationStructure { .. } => kind == ResourceKind::AccelerationStructure,
			WriteData::StaticSamplers | WriteData::CombinedImageSamplerArray => false,
		}
	}

	/// Validates native allocation requirements that are stricter than retained descriptor kinds.
	fn validate_descriptor_resource(
		&self,
		shader_resource: ShaderResourceDescriptor,
		retained: RetainedDescriptor,
		sequence_index: u8,
	) {
		match retained.descriptor {
			WriteData::Buffer { handle, .. } if shader_resource.kind() == ResourceKind::StorageBuffer => {
				let buffer = self.buffer(handle).expect(
					"Invalid DX12 buffer descriptor. The most likely cause is that the retained buffer handle is stale.",
				);
				assert!(
					buffer.uses.intersects(Uses::Storage),
					"Invalid DX12 storage-buffer descriptor. The most likely cause is that the buffer was not created with storage usage."
				);
				if shader_resource.access().intersects(crate::AccessPolicies::WRITE) {
					assert!(
						self.buffer_heap_kind_for_sequence(handle, sequence_index) == Some(BufferHeapKind::Default),
						"Invalid writable DX12 storage-buffer descriptor. The most likely cause is that the buffer uses a host-visible heap that cannot provide a UAV."
					);
				}
			}
			WriteData::Image { handle, .. } => self.validate_image_descriptor_resource(shader_resource, handle, None),
			WriteData::CombinedImageSampler { image_handle, layer, .. } => {
				self.validate_image_descriptor_resource(shader_resource, image_handle, layer)
			}
			_ => {}
		}
	}

	/// Validates image usage, dimension metadata, and an optional selected array layer.
	fn validate_image_descriptor_resource(
		&self,
		shader_resource: ShaderResourceDescriptor,
		image_handle: crate::BaseImageHandle,
		layer: Option<u32>,
	) {
		let image = self
			.images
			.get(image_handle.0 as usize)
			.expect("Invalid DX12 image descriptor. The most likely cause is that the retained image handle is stale.");
		assert!(
			shader_resource.texture_view() != TextureViewTypes::Texture3D,
			"Unsupported DX12 Texture3D descriptor view. The most likely cause is that the image was allocated by the current 2D-only image path."
		);
		if shader_resource.kind() == ResourceKind::StorageImage {
			assert!(
				image.uses.intersects(Uses::Storage),
				"Invalid DX12 storage-image descriptor. The most likely cause is that the image was not created with storage usage."
			);
		}
		if let Some(layer) = layer {
			assert!(
				shader_resource.texture_view() == TextureViewTypes::Texture2DArray,
				"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
			);
			assert!(
				layer < image.array_layers.max(1),
				"Invalid DX12 image descriptor layer. The most likely cause is that the selected layer exceeds the image array size."
			);
		} else if shader_resource.texture_view() == TextureViewTypes::Texture2D {
			assert!(
				image.array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata."
			);
		}
	}

	/// Validates that bound retained sets form one complete, non-overlapping flat resource union.
	pub(crate) fn validate_descriptor_sets(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.0 as usize];
		let sequence_sets = sets
			.iter()
			.map(|&set| self.descriptor_set_for_sequence(set, sequence_index).unwrap_or(set))
			.collect::<SmallVec<[DescriptorSetHandle; 8]>>();

		let mut occupied_slots = HashSet::default();
		for &set_handle in &sequence_sets {
			let set = &self.descriptor_sets[set_handle.0 as usize];
			for &slot in set.descriptors.keys() {
				if layout.resources.iter().any(|resource| resource.descriptor.slot() == slot) {
					assert!(
						occupied_slots.insert(slot),
						"Overlapping retained descriptor sets. The most likely cause is that two bound sets write the same flat resource slot.",
					);
					continue;
				}
				let is_array_interior = layout.resources.iter().any(|resource| {
					let start = resource.descriptor.slot().index();
					let slot = slot.index();
					start < slot && slot < Self::resource_range_end(resource.descriptor)
				});
				assert!(
					!is_array_interior,
					"Invalid retained descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array's base slot.",
				);
				// Retained sets can be shared by several passes, so descriptors outside this pipeline interface remain dormant.
			}
		}

		for resource in &layout.resources {
			let owners = sequence_sets
				.iter()
				.filter_map(|set_handle| {
					self.descriptor_sets[set_handle.0 as usize]
						.descriptors
						.get(&resource.descriptor.slot())
				})
				.collect::<SmallVec<[&HashMap<u32, RetainedDescriptor>; 4]>>();
			assert!(
				owners.len() <= 1,
				"Overlapping retained descriptor sets. The most likely cause is that two bound sets own the same active shader resource.",
			);
			if resource.descriptor.count() == 1 {
				assert!(
					owners.first().is_some_and(|descriptors| descriptors.contains_key(&0)),
					"Missing retained descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					resource.descriptor.slot().index(),
				);
			}
			if let Some(descriptors) = owners.first() {
				for (&array_element, retained) in descriptors.iter() {
					assert!(
						array_element < resource.descriptor.count(),
						"Descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(retained.descriptor, resource.descriptor.kind()),
						"Descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
					self.validate_descriptor_resource(resource.descriptor, *retained, sequence_index);
				}
			}
		}
	}

	pub(crate) fn bind_descriptor_heaps_and_tables(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let layout_handle = pipeline.layout;
		let pipeline_kind = pipeline.kind;

		let Some(materialization) = self.materialize_descriptor_heaps(layout_handle, sets, sequence_index) else {
			return;
		};
		self.retain_descriptor_materialization(command_buffer_handle, &materialization);
		let mut heaps = [None, None];
		let mut heap_count = 0usize;
		if let Some(heap) = materialization.cbv_srv_uav_heap.as_ref() {
			heaps[heap_count] = Some(heap.clone());
			heap_count += 1;
		}
		if let Some(heap) = materialization.sampler_heap.as_ref() {
			heaps[heap_count] = Some(heap.clone());
			heap_count += 1;
		}
		if heap_count == 0 {
			return;
		}

		unsafe {
			command_list.SetDescriptorHeaps(&heaps[..heap_count]);
		}
		self.descriptor_heap_bind_count += 1;
		let Some(Some(_root_signature)) = self.pipeline_root_signatures.get(layout_handle.0 as usize) else {
			panic!(
				"Failed to bind DX12 descriptor tables because the pipeline layout has no native root signature. The most likely cause is that root signature creation failed while the pipeline kept descriptor table metadata."
			);
		};
		let Some(root_tables) = self.pipeline_root_tables.get(layout_handle.0 as usize).cloned() else {
			return;
		};
		let mut table_binds = 0;
		for table in root_tables {
			let heap = if table.sampler_heap {
				materialization.sampler_heap.as_ref()
			} else {
				materialization.cbv_srv_uav_heap.as_ref()
			};
			let Some(heap) = heap else {
				continue;
			};
			let heap_type = if table.sampler_heap {
				D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
			} else {
				D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
			};
			let handle = self.descriptor_gpu_handle(heap, heap_type, 0);
			unsafe {
				match pipeline_kind {
					PipelineKind::Compute | PipelineKind::RayTracing => {
						command_list.SetComputeRootDescriptorTable(table.root_parameter_index, handle)
					}
					PipelineKind::Raster => command_list.SetGraphicsRootDescriptorTable(table.root_parameter_index, handle),
				}
			}
			table_binds += 1;
			#[cfg(test)]
			{
				self.descriptor_table_bind_records.push(DescriptorTableBindRecord {
					root_parameter_index: table.root_parameter_index,
					set_index: 0,
					binding_index: 0,
					sampler_heap: table.sampler_heap,
					heap_slot: 0,
				});
			}
		}
		self.descriptor_table_bind_count += table_binds;
	}

	pub(crate) fn write_push_constants_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		offset: u32,
		bytes: &[u8],
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let Some(constants) = self.pipeline_root_constants.get(pipeline.layout.0 as usize) else {
			return;
		};
		assert!(
			offset % 4 == 0 && bytes.len() % 4 == 0,
			"Invalid DX12 push-constant write alignment. The most likely cause is that the offset or data size is not a multiple of four bytes."
		);
		if bytes.is_empty() {
			return;
		}
		let byte_count = u32::try_from(bytes.len()).expect(
			"Invalid DX12 push-constant write size. The most likely cause is that the data exceeds the addressable root-constant range.",
		);
		let end = offset.checked_add(byte_count).expect(
			"Invalid DX12 push-constant write range. The most likely cause is that the offset and data size overflow the root-constant range.",
		);
		let range = constants
			.iter()
			.find(|range| offset >= range.offset && end <= range.offset.saturating_add(range.size))
			.copied()
			.expect(
				"Invalid DX12 push-constant write range. The most likely cause is that no active pipeline range contains the requested bytes.",
			);

		let destination_offset = offset / 4;
		let word_count = byte_count / 4;
		let compute_root = matches!(pipeline.kind, PipelineKind::Compute | PipelineKind::RayTracing);
		unsafe {
			if compute_root {
				command_list.SetComputeRoot32BitConstants(
					range.root_parameter_index,
					word_count,
					bytes.as_ptr().cast(),
					destination_offset,
				);
			} else {
				command_list.SetGraphicsRoot32BitConstants(
					range.root_parameter_index,
					word_count,
					bytes.as_ptr().cast(),
					destination_offset,
				);
			}
		}
		self.push_constant_write_count += 1;
		#[cfg(test)]
		{
			self.push_constant_write_records.push(PushConstantWriteRecord {
				root_parameter_index: range.root_parameter_index,
				offset,
				size: bytes.len() as u32,
				compute_root,
			});
		}
	}

	pub(crate) fn submit_command_buffer(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		synchronizer_handle: SynchronizerHandle,
	) {
		let command_buffer_index = command_buffer_handle.0 as usize;
		let Some(command_buffer) = self.command_buffers.get(command_buffer_index) else {
			return;
		};
		let Some(command_list) = command_buffer.command_list.as_ref() else {
			return;
		};
		let command_list = (*command_list).clone();
		let is_open = command_buffer.is_open;
		let queue_handle = command_buffer.queue_handle;
		let sequence_index = command_buffer.sequence_index;

		self.transition_present_resources(command_buffer_handle, &command_list);
		let recorded_work = self
			.command_buffers
			.get(command_buffer_index)
			.map(|command_buffer| command_buffer.recorded_work)
			.unwrap_or(false);
		if is_open {
			let result = unsafe { command_list.Close() };
			if result.is_err() {
				panic!(
					"Failed to close a DX12 command list. The most likely cause is that command list recording failed or the command list was already closed."
				);
			}
			if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_index) {
				command_buffer.is_open = false;
			}
		}

		if !recorded_work {
			self.empty_command_list_skip_count += 1;
			self.complete_synchronizer_for_sequence_from_cpu(synchronizer_handle, sequence_index);
			return;
		}

		let Some(queue) = self.queues.get(queue_handle.0 as usize) else {
			return;
		};
		let command_list = command_list.cast::<ID3D12CommandList>().expect(
			"Failed to cast a DX12 graphics command list for execution. The most likely cause is an incompatible command list object.",
		);
		let command_lists = [Some(command_list)];
		unsafe {
			queue.queue.ExecuteCommandLists(&command_lists);
		}
		self.native_command_list_execute_count += 1;
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_index) {
			command_buffer.last_submission = Some((synchronizer_handle, sequence_index));
		}
		self.signal_synchronizer_for_sequence(queue_handle, synchronizer_handle, sequence_index);
		let completion = self
			.synchronizer_for_sequence(synchronizer_handle, sequence_index)
			.and_then(|handle| {
				self.synchronizers
					.get(handle.0 as usize)
					.map(|synchronizer| (handle, synchronizer.value))
			});
		for readback in self
			.texture_readbacks
			.iter_mut()
			.filter(|readback| readback.command_buffer_handle == command_buffer_handle)
		{
			readback.completion = completion;
		}
	}

	pub(crate) fn record_present_preparation(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		present_keys: &[PresentKey],
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		for present_key in present_keys {
			let Some((source_image, proxy_uses)) =
				self.swapchains.get(present_key.swapchain.0 as usize).and_then(|swapchain| {
					let image_index = (present_key.sequence_index as usize).min(swapchain.images.len().saturating_sub(1));
					swapchain.images[image_index]
						.or(swapchain.images[0])
						.map(|image| (image, swapchain.proxy_uses[image_index]))
				})
			else {
				continue;
			};
			if !proxy_uses.intersects(Uses::Storage) {
				continue;
			}
			let Some(source_resource) = self.ensure_image_resource_for_sequence(source_image.0, present_key.sequence_index)
			else {
				continue;
			};
			let Some(destination_resource) =
				self.swapchain_backbuffer_resource(present_key.swapchain, present_key.sequence_index)
			else {
				continue;
			};

			unsafe {
				// Copy the engine swapchain proxy image into the actual DXGI backbuffer before Present.
				let mut copy_barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
				self.transition_tracked_image_into(
					source_image.0,
					&source_resource,
					D3D12_RESOURCE_STATE_COPY_SOURCE,
					&mut copy_barriers,
				);
				copy_barriers.push(Self::transition_resource_barrier(
					&destination_resource,
					D3D12_RESOURCE_STATE_PRESENT,
					D3D12_RESOURCE_STATE_COPY_DEST,
				));
				Self::submit_resource_barriers(&command_list, &copy_barriers);
				command_list.CopyResource(&destination_resource, &source_resource);
				let mut present_barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
				present_barriers.push(Self::transition_resource_barrier(
					&destination_resource,
					D3D12_RESOURCE_STATE_COPY_DEST,
					D3D12_RESOURCE_STATE_PRESENT,
				));
				self.transition_tracked_image_into(
					source_image.0,
					&source_resource,
					D3D12_RESOURCE_STATE_COMMON,
					&mut present_barriers,
				);
				Self::submit_resource_barriers(&command_list, &present_barriers);
			}
			self.mark_command_buffer_work(command_buffer_handle);
			self.texture_copy_count += 1;
		}
	}

	fn transition_present_resources(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
	) {
		let Some(resources) = self.present_transitions.remove(&command_buffer_handle) else {
			return;
		};
		for resource in resources {
			unsafe {
				Self::transition_resource(
					command_list,
					&resource,
					D3D12_RESOURCE_STATE_RENDER_TARGET,
					D3D12_RESOURCE_STATE_PRESENT,
				);
			}
			self.mark_command_buffer_work(command_buffer_handle);
			self.swapchain_present_transition_count += 1;
		}
	}

	fn signal_private_synchronizer(
		&mut self,
		queue_handle: QueueHandle,
		synchronizer_handle: crate::synchronizer::SynchronizerHandle,
	) {
		let Some(queue) = self.queues.get(queue_handle.0 as usize) else {
			return;
		};
		let Some(synchronizer) = self.synchronizers.get_mut(synchronizer_handle.0 as usize) else {
			return;
		};
		synchronizer.value = synchronizer.value.saturating_add(1);
		let result = unsafe { queue.queue.Signal(&synchronizer.fence, synchronizer.value) };
		if result.is_err() {
			panic!(
				"Failed to signal a DX12 fence. The most likely cause is that the queue or fence was invalid or the device was removed."
			);
		}
	}

	fn signal_synchronizer_for_sequence(
		&mut self,
		queue_handle: QueueHandle,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.signal_private_synchronizer(queue_handle, handle);
	}

	/// Completes an empty submission without sending a no-op command list to the GPU queue.
	fn complete_private_synchronizer_from_cpu(&mut self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		let Some(synchronizer) = self.synchronizers.get_mut(synchronizer_handle.0 as usize) else {
			return;
		};
		synchronizer.value = synchronizer.value.saturating_add(1);
		let result = unsafe { synchronizer.fence.Signal(synchronizer.value) };
		if result.is_err() {
			panic!(
				"Failed to complete a DX12 fence from the CPU. The most likely cause is that the fence was invalid or the device was removed."
			);
		}
	}

	/// Completes an empty frame sequence without submitting work to a DX12 queue.
	pub(crate) fn complete_synchronizer_for_sequence_from_cpu(
		&mut self,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.complete_private_synchronizer_from_cpu(handle);
	}

	pub(crate) fn copy_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			self.copy_buffer_shadow(copy, sequence_index);
			self.record_buffer_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	pub(crate) fn clear_buffers(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handles: &[BaseBufferHandle],
		sequence_index: u8,
	) {
		for &buffer_handle in buffer_handles {
			if self.buffer_needs_cpu_shadow_clear(buffer_handle) {
				self.clear_buffer_shadow(buffer_handle, sequence_index);
			}
		}

		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let mut gpu_clear_buffers = SmallVec::<[(BaseBufferHandle, ID3D12Resource); 16]>::new();
		let mut clear_barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		for &buffer_handle in buffer_handles {
			let Some(buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
				continue;
			};
			if buffer.access.intersects(DeviceAccesses::GpuWrite)
				&& buffer.heap_kind == BufferHeapKind::Default
				&& buffer.size != 0
				&& buffer.size % std::mem::size_of::<u32>() == 0
			{
				if gpu_clear_buffers.iter().any(|(handle, _)| *handle == buffer_handle) {
					continue;
				}
				unsafe {
					self.transition_tracked_buffer_into(
						buffer_handle,
						&buffer.resource,
						D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
						&mut clear_barriers,
					);
				}
				gpu_clear_buffers.push((buffer_handle, buffer.resource));
			}
		}
		unsafe {
			Self::submit_resource_barriers(&command_list, &clear_barriers);
		}

		for &buffer_handle in buffer_handles {
			let batched = gpu_clear_buffers.iter().any(|(handle, _)| *handle == buffer_handle);
			self.record_buffer_clear(command_buffer_handle, buffer_handle, sequence_index, !batched);
		}
	}

	/// Returns whether a buffer clear must update CPU-visible shadow storage.
	fn buffer_needs_cpu_shadow_clear(&self, buffer_handle: BaseBufferHandle) -> bool {
		self.buffer(buffer_handle)
			.map(|buffer| buffer.access.intersects(DeviceAccesses::CpuRead | DeviceAccesses::CpuWrite))
			.unwrap_or(false)
	}

	fn clear_buffer_shadow(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) {
		let Some((data, size)) = self.buffer_storage_parts_mut_for_sequence(buffer_handle, sequence_index) else {
			return;
		};
		if size == 0 {
			return;
		}

		unsafe {
			std::ptr::write_bytes(data, 0, size);
		}
		self.sync_buffer_for_sequence(buffer_handle, sequence_index);
	}

	fn copy_buffer_shadow(&mut self, copy: &crate::BufferCopyDescriptor, sequence_index: u8) {
		// Resolve handles through `buffer` instead of indexing storage directly. Dynamic buffer handles carry
		// `DYNAMIC_BUFFER_HANDLE_FLAG`, so the raw handle value is not always a valid index into `buffers`.
		let Some(source) = self.buffer_storage_parts_for_sequence(copy.source_buffer, sequence_index) else {
			return;
		};
		let Some(destination) = self.buffer_storage_parts_mut_for_sequence(copy.destination_buffer, sequence_index) else {
			return;
		};

		let source_end = copy.source_offset.saturating_add(copy.size);
		let destination_end = copy.destination_offset.saturating_add(copy.size);
		if source_end > source.1 || destination_end > destination.1 {
			panic!(
				"Failed to copy DX12 buffer data from {:?} offset {} to {:?} offset {} for {} bytes. The most likely cause is that the requested source or destination range is outside the buffer allocation. Source size: {} bytes. Destination size: {} bytes.",
				copy.source_buffer,
				copy.source_offset,
				copy.destination_buffer,
				copy.destination_offset,
				copy.size,
				source.1,
				destination.1
			);
		}
		if copy.size == 0 {
			return;
		}

		unsafe {
			let source = source.0.add(copy.source_offset);
			let destination = destination.0.add(copy.destination_offset);
			std::ptr::copy(source, destination, copy.size);
		}
		self.sync_buffer_for_sequence(copy.destination_buffer, sequence_index);
	}

	fn record_buffer_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferCopyDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(source) = self.copy_buffer_info_for_sequence(copy.source_buffer, sequence_index) else {
			return;
		};
		let Some(destination) = self.copy_buffer_info_for_sequence(copy.destination_buffer, sequence_index) else {
			return;
		};
		if destination.access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		let source_end = copy.source_offset.saturating_add(copy.size);
		let destination_end = copy.destination_offset.saturating_add(copy.size);
		if source_end > source.size || destination_end > destination.size {
			panic!(
				"Failed to record DX12 buffer copy from {:?} offset {} to {:?} offset {} for {} bytes. The most likely cause is that the requested source or destination range is outside the GPU buffer allocation. Source size: {} bytes. Destination size: {} bytes.",
				copy.source_buffer,
				copy.source_offset,
				copy.destination_buffer,
				copy.destination_offset,
				copy.size,
				source.size,
				destination.size
			);
		}

		unsafe {
			if source.heap_kind == BufferHeapKind::Default {
				self.transition_tracked_buffer(
					&command_list,
					copy.source_buffer,
					&source.resource,
					D3D12_RESOURCE_STATE_COPY_SOURCE,
				);
			}
			if destination.heap_kind == BufferHeapKind::Default {
				self.transition_tracked_buffer(
					&command_list,
					copy.destination_buffer,
					&destination.resource,
					D3D12_RESOURCE_STATE_COPY_DEST,
				);
			}
			command_list.CopyBufferRegion(
				&destination.resource,
				copy.destination_offset as u64,
				&source.resource,
				copy.source_offset as u64,
				copy.size as u64,
			);
			if destination.heap_kind == BufferHeapKind::Default {
				self.transition_tracked_buffer(
					&command_list,
					copy.destination_buffer,
					&destination.resource,
					D3D12_RESOURCE_STATE_COMMON,
				);
			}
			if source.heap_kind == BufferHeapKind::Default {
				self.transition_tracked_buffer(
					&command_list,
					copy.source_buffer,
					&source.resource,
					D3D12_RESOURCE_STATE_COMMON,
				);
			}
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.buffer_copy_count += 1;
	}

	fn record_buffer_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
		transition_before_clear: bool,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(destination_buffer) = self.copy_buffer_info_for_sequence(buffer_handle, sequence_index) else {
			return;
		};
		let destination_size = destination_buffer.size;
		let destination_access = destination_buffer.access;
		let destination_heap_kind = destination_buffer.heap_kind;
		let destination = destination_buffer.resource;
		if destination_size == 0 || destination_access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}
		if destination_access.intersects(DeviceAccesses::GpuWrite)
			&& destination_heap_kind == BufferHeapKind::Default
			&& destination_size % std::mem::size_of::<u32>() == 0
		{
			// Default-heap GPU-writable buffers can be cleared in place through a transient UAV descriptor.
			let Some((heap, descriptor_offset)) = self.reserve_staged_descriptor_range(command_buffer_handle, false, 1) else {
				return;
			};
			let Some(cpu_heap) =
				self.create_transient_cpu_descriptor_heap(command_buffer_handle, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 1)
			else {
				return;
			};
			let cpu_handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
			let cpu_read_handle = self.descriptor_cpu_handle(&cpu_heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 0);
			let gpu_handle = self.descriptor_gpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
			let desc = Self::raw_buffer_clear_uav_desc(destination_size);

			unsafe {
				if transition_before_clear {
					self.transition_tracked_buffer(
						&command_list,
						buffer_handle,
						&destination,
						D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
					);
				}
				self.device
					.CreateUnorderedAccessView(&destination, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
				self.device
					.CreateUnorderedAccessView(&destination, None::<&ID3D12Resource>, Some(&desc), cpu_read_handle);
				self.bind_active_staged_descriptor_heaps(command_buffer_handle);
				command_list.ClearUnorderedAccessViewUint(gpu_handle, cpu_read_handle, &destination, &[0, 0, 0, 0], &[]);
			}
			self.mark_command_buffer_work(command_buffer_handle);
			self.buffer_clear_count += 1;
			return;
		}
		let (Some(upload), mapped, _) = self.create_buffer_resource(destination_size, DeviceAccesses::HostToDevice) else {
			return;
		};
		if mapped.is_null() {
			return;
		}

		unsafe {
			std::ptr::write_bytes(mapped, 0, destination_size);
			self.transition_tracked_buffer(&command_list, buffer_handle, &destination, D3D12_RESOURCE_STATE_COPY_DEST);
			command_list.CopyBufferRegion(&destination, 0, &upload, 0, destination_size as u64);
			self.transition_tracked_buffer(&command_list, buffer_handle, &destination, D3D12_RESOURCE_STATE_COMMON);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		self.buffer_clear_count += 1;
	}

	fn copy_buffer_info_for_sequence(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) -> Option<BufferCopyInfo> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let resource = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let heap_kind = self.buffer_heap_kind_for_sequence(buffer_handle, sequence_index)?;
		let buffer = self.buffer(buffer_handle)?;
		Some(BufferCopyInfo {
			resource,
			access: buffer.access,
			heap_kind,
			size: buffer.size,
		})
	}

	pub(crate) fn copy_buffer_to_images(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferImageCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			self.copy_buffer_to_image(copy, sequence_index);
			self.record_buffer_to_image_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	fn copy_buffer_to_image(&mut self, copy: &crate::BufferImageCopyDescriptor, sequence_index: u8) {
		let Some(image) = self.images.get(copy.destination_image.0 as usize) else {
			return;
		};
		let Some((row_bytes, row_count, compact_bytes_per_image)) = utils::texture_copy_layout(image.format, image.extent)
		else {
			return;
		};
		let extent = image.extent;
		let row_stride = if copy.source_bytes_per_row == 0 {
			row_bytes
		} else {
			copy.source_bytes_per_row
		};
		let image_stride = if copy.source_bytes_per_image == 0 {
			row_stride * row_count
		} else {
			copy.source_bytes_per_image
		};
		let depth = extent.depth().max(1) as usize;
		let source_bytes =
			self.buffer_range_for_sequence(copy.source_buffer, copy.source_offset, image_stride * depth, sequence_index);
		let Some(destination) = self.image_data_mut_for_sequence(copy.destination_image, sequence_index) else {
			return;
		};

		for layer in 0..depth {
			for y in 0..row_count {
				let source_start = layer * image_stride + y * row_stride;
				let source_end = source_start + row_bytes;
				let destination_start = layer * compact_bytes_per_image + y * row_bytes;
				let destination_end = destination_start + row_bytes;
				if source_end > source_bytes.len() || destination_end > destination.len() {
					panic!(
						"Failed to copy DX12 buffer data into an image. The most likely cause is that the source row layout or destination image extent is invalid."
					);
				}
				destination[destination_start..destination_end].copy_from_slice(&source_bytes[source_start..source_end]);
			}
		}
	}

	fn record_buffer_to_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferImageCopyDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let destination = self.ensure_image_resource_for_sequence(copy.destination_image, sequence_index);
		let Some(image) = self.images.get(copy.destination_image.0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((row_bytes, row_count, _))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};

		let extent = image.extent;
		let source_row_pitch = if copy.source_bytes_per_row == 0 {
			row_bytes
		} else {
			copy.source_bytes_per_row
		};
		let source_image_pitch = if copy.source_bytes_per_image == 0 {
			source_row_pitch * row_count
		} else {
			copy.source_bytes_per_image
		};
		let source_bytes = self.buffer_range_for_sequence(
			copy.source_buffer,
			copy.source_offset,
			source_image_pitch * extent.depth().max(1) as usize,
			sequence_index,
		);
		self.record_image_upload(
			command_buffer_handle,
			&command_list,
			copy.destination_image,
			destination,
			format,
			extent,
			&source_bytes,
			source_row_pitch,
			source_image_pitch,
		);
	}

	pub(crate) fn record_image_data_write(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		data: &[RGBAu8],
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let destination = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};
		let extent = image.extent;
		let source_bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
		if self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			format,
			extent,
			source_bytes,
			source_row_pitch,
			source_row_pitch
				* utils::texture_copy_layout(image.format, image.extent)
					.map(|(_, rows, _)| rows)
					.unwrap_or(0),
		) {
			self.gpu_uploaded_images.insert(image_handle.0);
		}
	}

	/// Uploads only pending image data selected for the current command buffer and frame.
	pub(crate) fn flush_pending_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_filter: Option<crate::BaseImageHandle>,
		sequence_filter: Option<u8>,
	) {
		let pending = std::mem::take(&mut self.pending_texture_syncs);
		for (image_handle, sequence_index) in pending {
			let image_mismatch = image_filter.is_some_and(|filter| filter != image_handle);
			let sequence_mismatch = sequence_filter.is_some_and(|filter| filter != sequence_index);
			if image_mismatch || sequence_mismatch {
				self.pending_texture_syncs.push((image_handle, sequence_index));
				continue;
			}
			self.record_image_storage_upload(command_buffer_handle, ImageHandle(image_handle), sequence_index);
		}
	}

	pub(crate) fn flush_pending_texture_syncs_for_sequence(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sequence_filter: u8,
	) {
		self.flush_pending_texture_syncs(command_buffer_handle, None, Some(sequence_filter));
	}

	fn record_image_storage_upload(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let destination = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};
		let extent = image.extent;
		let source_bytes = image
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| image.data.clone())
			.unwrap_or_default();
		if self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			format,
			extent,
			&source_bytes,
			source_row_pitch,
			source_row_pitch
				* utils::texture_copy_layout(image.format, image.extent)
					.map(|(_, rows, _)| rows)
					.unwrap_or(0),
		) {
			self.gpu_uploaded_images.insert(image_handle.0);
		}
	}

	pub(crate) fn begin_debug_region(&self, command_buffer_handle: CommandBufferHandle, name: &str) {
		if !self.settings.debug_labels {
			return;
		}

		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.as_ref() else {
			return;
		};

		// Metadata version zero tells PIX to decode the payload as a null-terminated UTF-16 event
		// name. Keep the encoded name alive until BeginEvent has copied it into the command list.
		let mut encoded_name = name.encode_utf16().collect::<SmallVec<[u16; 128]>>();
		encoded_name.push(0);
		let encoded_size = u32::try_from(std::mem::size_of_val(encoded_name.as_slice())).expect(
			"PIX debug label is too long. The most likely cause is a generated label larger than the DX12 event-size limit.",
		);
		unsafe {
			command_list.BeginEvent(0, Some(encoded_name.as_ptr().cast()), encoded_size);
		}
		self.debug_region_begin_count.set(self.debug_region_begin_count.get() + 1);
	}

	pub(crate) fn end_debug_region(&self, command_buffer_handle: CommandBufferHandle) {
		if !self.settings.debug_labels {
			return;
		}

		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.as_ref() else {
			return;
		};

		unsafe {
			command_list.EndEvent();
		}
		self.debug_region_end_count.set(self.debug_region_end_count.get() + 1);
	}

	fn record_image_upload(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: DXGI_FORMAT,
		extent: Extent,
		source_bytes: &[u8],
		source_row_pitch: usize,
		source_image_pitch: usize,
	) -> bool {
		let Some((row_bytes, row_count, _)) = utils::texture_copy_layout(self.images[image_handle.0 as usize].format, extent)
		else {
			return false;
		};
		let depth = extent.depth().max(1) as usize;
		let upload_row_pitch = Self::align_up(row_bytes, 256);
		let upload_size = upload_row_pitch * row_count * depth;
		let (Some(upload), mapped, _) = self.create_buffer_resource(upload_size, DeviceAccesses::HostToDevice) else {
			return false;
		};
		if mapped.is_null() {
			return false;
		}

		unsafe {
			std::ptr::write_bytes(mapped, 0, upload_size);
			for layer in 0..depth {
				for y in 0..row_count {
					let source_start = layer * source_image_pitch + y * source_row_pitch;
					let source_end = source_start + row_bytes;
					let upload_start = (layer * row_count + y) * upload_row_pitch;
					if source_end > source_bytes.len() {
						return false;
					}
					std::ptr::copy_nonoverlapping(
						source_bytes[source_start..source_end].as_ptr(),
						mapped.add(upload_start),
						row_bytes,
					);
				}
			}
		}

		let source_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(upload.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
					Offset: 0,
					Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
						Format: format,
						Width: extent.width(),
						Height: extent.height(),
						Depth: depth as u32,
						RowPitch: upload_row_pitch as u32,
					},
				},
			},
		};
		let destination_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(destination)),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
		};

		unsafe {
			self.transition_tracked_image(
				command_list,
				image_handle,
				destination_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COPY_DEST,
			);
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
			self.transition_tracked_image(
				command_list,
				image_handle,
				destination_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COMMON,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		true
	}

	unsafe fn transition_resource(
		command_list: &ID3D12GraphicsCommandList,
		resource: &ID3D12Resource,
		before: D3D12_RESOURCE_STATES,
		after: D3D12_RESOURCE_STATES,
	) {
		let barrier = Self::transition_resource_barrier(resource, before, after);
		Self::submit_resource_barriers(command_list, &[barrier]);
	}

	/// Creates a transition barrier so callers can submit independent resource transitions together.
	fn transition_resource_barrier(
		resource: &ID3D12Resource,
		before: D3D12_RESOURCE_STATES,
		after: D3D12_RESOURCE_STATES,
	) -> D3D12_RESOURCE_BARRIER {
		D3D12_RESOURCE_BARRIER {
			Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
			Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
			Anonymous: D3D12_RESOURCE_BARRIER_0 {
				Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
					pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
					Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
					StateBefore: before,
					StateAfter: after,
				}),
			},
		}
	}

	/// Submits one native call for a group of barriers that share a synchronization boundary.
	unsafe fn submit_resource_barriers(command_list: &ID3D12GraphicsCommandList, barriers: &[D3D12_RESOURCE_BARRIER]) {
		if !barriers.is_empty() {
			command_list.ResourceBarrier(barriers);
		}
	}

	unsafe fn unordered_access_barrier(command_list: &ID3D12GraphicsCommandList, resource: &ID3D12Resource) {
		let barrier = Self::unordered_access_resource_barrier(resource);
		Self::submit_resource_barriers(command_list, &[barrier]);
	}

	/// Creates a resource-specific UAV barrier for a caller-owned synchronization batch.
	fn unordered_access_resource_barrier(resource: &ID3D12Resource) -> D3D12_RESOURCE_BARRIER {
		D3D12_RESOURCE_BARRIER {
			Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
			Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
			Anonymous: D3D12_RESOURCE_BARRIER_0 {
				UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
					pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
				}),
			},
		}
	}

	unsafe fn unordered_access_barrier_all(command_list: &ID3D12GraphicsCommandList) {
		let barrier = D3D12_RESOURCE_BARRIER {
			Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
			Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
			Anonymous: D3D12_RESOURCE_BARRIER_0 {
				UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
					pResource: std::mem::ManuallyDrop::new(None),
				}),
			},
		};
		command_list.ResourceBarrier(&[barrier]);
	}

	/// Uses native resource identity so dynamic frame allocations keep independent state histories.
	fn native_resource_key(resource: &ID3D12Resource) -> usize {
		resource.as_raw() as usize
	}

	fn initial_buffer_resource_state(heap_kind: BufferHeapKind) -> D3D12_RESOURCE_STATES {
		match heap_kind {
			BufferHeapKind::Upload => D3D12_RESOURCE_STATE_GENERIC_READ,
			BufferHeapKind::Readback => D3D12_RESOURCE_STATE_COPY_DEST,
			BufferHeapKind::Default => D3D12_RESOURCE_STATE_COMMON,
		}
	}

	fn buffer_heap_kind_for_resource(
		&self,
		buffer_handle: BaseBufferHandle,
		resource: &ID3D12Resource,
	) -> Option<BufferHeapKind> {
		let key = Self::native_resource_key(resource);
		let buffer = self.buffer(buffer_handle)?;
		if buffer
			.resource
			.as_ref()
			.is_some_and(|resource| Self::native_resource_key(resource) == key)
		{
			return Some(buffer.heap_kind);
		}
		buffer.frame_resources.as_ref().and_then(|frame_resources| {
			frame_resources.iter().flatten().find_map(|frame_resource| {
				frame_resource
					.resource
					.as_ref()
					.is_some_and(|resource| Self::native_resource_key(resource) == key)
					.then_some(frame_resource.heap_kind)
			})
		})
	}

	unsafe fn transition_tracked_buffer(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		_buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		let mut barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		self.transition_tracked_buffer_into(_buffer, resource, after, &mut barriers);
		Self::submit_resource_barriers(command_list, &barriers);
	}

	/// Appends a tracked buffer transition to a caller-owned synchronization batch.
	unsafe fn transition_tracked_buffer_into(
		&mut self,
		_buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
		barriers: &mut SmallVec<[D3D12_RESOURCE_BARRIER; 32]>,
	) {
		let key = Self::native_resource_key(resource);
		let heap_kind = self
			.buffer_heap_kind_for_resource(_buffer, resource)
			.unwrap_or(BufferHeapKind::Default);
		if heap_kind != BufferHeapKind::Default {
			self.buffer_states
				.entry(key)
				.or_insert_with(|| Self::initial_buffer_resource_state(heap_kind));
			return;
		}
		let before = self
			.buffer_states
			.get(&key)
			.copied()
			.unwrap_or_else(|| Self::initial_buffer_resource_state(heap_kind));
		if before == after {
			if after == D3D12_RESOURCE_STATE_UNORDERED_ACCESS {
				barriers.push(Self::unordered_access_resource_barrier(resource));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push(Self::transition_resource_barrier(resource, before, after));
		self.buffer_states.insert(key, after);
	}

	unsafe fn transition_tracked_image(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		_image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		let mut barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
		self.transition_tracked_image_into(_image, resource, after, &mut barriers);
		Self::submit_resource_barriers(command_list, &barriers);
	}

	/// Appends a tracked image transition to a caller-owned synchronization batch.
	unsafe fn transition_tracked_image_into(
		&mut self,
		_image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
		barriers: &mut SmallVec<[D3D12_RESOURCE_BARRIER; 32]>,
	) {
		let key = Self::native_resource_key(resource);
		let before = self.image_states.get(&key).copied().unwrap_or(D3D12_RESOURCE_STATE_COMMON);
		if before == after {
			if after == D3D12_RESOURCE_STATE_UNORDERED_ACCESS {
				barriers.push(Self::unordered_access_resource_barrier(resource));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push(Self::transition_resource_barrier(resource, before, after));
		self.image_states.insert(key, after);
	}

	fn align_up(value: usize, alignment: usize) -> usize {
		(value + alignment - 1) / alignment * alignment
	}

	fn buffer_range_for_sequence(
		&self,
		buffer_handle: BaseBufferHandle,
		offset: usize,
		size: usize,
		sequence_index: u8,
	) -> Vec<u8> {
		let Some((data, buffer_size)) = self.buffer_storage_parts_for_sequence(buffer_handle, sequence_index) else {
			return Vec::new();
		};
		let end = offset.saturating_add(size);
		if end > buffer_size {
			panic!(
				"Failed to read DX12 buffer data. The most likely cause is that the requested range is outside the buffer allocation."
			);
		}
		if size == 0 {
			return Vec::new();
		}

		unsafe { std::slice::from_raw_parts(data.add(offset), size).to_vec() }
	}

	pub(crate) fn copy_image_to_cpu(&mut self, image_handle: ImageHandle) -> TextureCopyHandle {
		self.copy_image_to_cpu_for_sequence(image_handle, 0)
	}

	pub(crate) fn copy_image_to_cpu_for_sequence(
		&mut self,
		image_handle: ImageHandle,
		sequence_index: u8,
	) -> TextureCopyHandle {
		// Copies stored image data into a new staging buffer for CPU reads.
		let image = &self.images[image_handle.0 .0 as usize];
		let data = image
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| image.data.clone())
			.unwrap_or_default();
		self.texture_copies.push(data);
		TextureCopyHandle((self.texture_copies.len() - 1) as u64)
	}

	pub(crate) fn record_image_readback(&mut self, command_buffer_handle: CommandBufferHandle, image_handle: ImageHandle) {
		self.record_image_readback_internal(command_buffer_handle, image_handle, None, 0);
	}

	pub(crate) fn record_image_readback_for_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		texture_copy: TextureCopyHandle,
		sequence_index: u8,
	) {
		self.record_image_readback_internal(command_buffer_handle, image_handle, Some(texture_copy), sequence_index);
	}

	fn record_image_readback_internal(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		texture_copy: Option<TextureCopyHandle>,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let source = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(source), Some(format), Some((row_bytes, row_count, _))) = (
			source,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};

		let extent = image.extent;
		let depth = extent.depth().max(1) as usize;
		let readback_row_pitch = Self::align_up(row_bytes, 256);
		let readback_size = readback_row_pitch * row_count * depth;
		let (Some(readback), ..) = self.create_buffer_resource(readback_size, DeviceAccesses::DeviceToHost) else {
			return;
		};

		let source_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(source)),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
		};
		let destination_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(readback.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
					Offset: 0,
					Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
						Format: format,
						Width: extent.width(),
						Height: extent.height(),
						Depth: depth as u32,
						RowPitch: readback_row_pitch as u32,
					},
				},
			},
		};

		unsafe {
			self.transition_tracked_image(
				&command_list,
				image_handle.0,
				source_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COPY_SOURCE,
			);
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
			self.transition_tracked_image(
				&command_list,
				image_handle.0,
				source_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COMMON,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		if texture_copy.is_none() {
			self.retain_command_buffer_resource(command_buffer_handle, readback);
			return;
		}
		let texture_copy = texture_copy.expect(
			"Missing DX12 texture-copy handle. The most likely cause is that a retained readback was created without CPU copy storage.",
		);
		self.texture_readbacks.push(TextureReadback {
			command_buffer_handle,
			texture_copy,
			completion: None,
			resource: readback,
			sequence_index,
			row_pitch: readback_row_pitch,
			row_bytes,
			height: row_count,
			depth,
			size: readback_size,
			resolved: false,
		});
	}

	fn refresh_readback_texture_copies(&mut self, sequence_index: Option<u8>) {
		// Maps completed readback buffers and repacks DX12 row padding into compact texture copies.
		for readback in &mut self.texture_readbacks {
			if readback.resolved {
				continue;
			}
			if sequence_index.is_some_and(|sequence_index| readback.sequence_index != sequence_index) {
				continue;
			}
			let Some((synchronizer_handle, completion_value)) = readback.completion else {
				continue;
			};
			let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
				continue;
			};
			if unsafe { synchronizer.fence.GetCompletedValue() } < completion_value {
				continue;
			}
			if readback.size == 0 {
				continue;
			}

			let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
			let read_range = D3D12_RANGE {
				Begin: 0,
				End: readback.size,
			};
			let result = unsafe { readback.resource.Map(0, Some(&read_range), Some(&mut mapped)) };
			if result.is_err() || mapped.is_null() {
				continue;
			}

			let compact_size = readback.row_bytes * readback.height * readback.depth;
			let mut compact = vec![0; compact_size];
			for layer in 0..readback.depth {
				for row in 0..readback.height {
					let source_offset = (layer * readback.height + row) * readback.row_pitch;
					let destination_offset = (layer * readback.height + row) * readback.row_bytes;
					unsafe {
						std::ptr::copy_nonoverlapping(
							(mapped as *const u8).add(source_offset),
							compact.as_mut_ptr().add(destination_offset),
							readback.row_bytes,
						);
					}
				}
			}
			let written_range = D3D12_RANGE { Begin: 0, End: 0 };
			unsafe {
				readback.resource.Unmap(0, Some(&written_range));
			}

			if let Some(texture_copy) = self.texture_copies.get_mut(readback.texture_copy.0 as usize) {
				*texture_copy = compact;
				self.texture_readback_resolve_count += 1;
				readback.resolved = true;
			}
		}
		// The compact CPU copy owns the result after resolution, so the native readback resource can retire now.
		self.texture_readbacks.retain(|readback| !readback.resolved);
	}

	pub(crate) fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]) {
		self.write_image_data_for_sequence(image_handle, data, 0);
	}

	pub(crate) fn write_image_data_for_sequence(&mut self, image_handle: ImageHandle, data: &[RGBAu8], sequence_index: u8) {
		// Writes CPU-side image data for formats with staging storage.
		let image = &mut self.images[image_handle.0 .0 as usize];
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
		let length = staging.len().min(bytes.len());
		staging[..length].copy_from_slice(&bytes[..length]);
	}

	pub(crate) fn clear_image(&mut self, image_handle: crate::BaseImageHandle, clear: crate::ClearValue) {
		self.clear_image_for_sequence(image_handle, clear, 0);
	}

	/// Updates CPU-side image data for a frame sequence so readback-oriented images preserve clear values.
	pub(crate) fn clear_image_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		let Some(image) = self.images.get_mut(image_handle.0 as usize) else {
			return;
		};
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let color = Self::clear_color_bytes(clear);

		for pixel in staging.chunks_exact_mut(std::mem::size_of::<RGBAu8>()) {
			pixel.copy_from_slice(&color);
		}
	}

	fn clear_color_bytes(clear: crate::ClearValue) -> [u8; 4] {
		match clear {
			crate::ClearValue::None => [0, 0, 0, 0],
			crate::ClearValue::Color(color) => [
				(color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
			],
			crate::ClearValue::Integer(r, g, b, a) => [
				r.min(u8::MAX as u32) as u8,
				g.min(u8::MAX as u32) as u8,
				b.min(u8::MAX as u32) as u8,
				a.min(u8::MAX as u32) as u8,
			],
			crate::ClearValue::Depth(_) => [0, 0, 0, 0],
		}
	}

	fn clear_color_f32(clear: ClearValue) -> [f32; 4] {
		match clear {
			ClearValue::None => [0.0, 0.0, 0.0, 0.0],
			ClearValue::Color(color) => [color.r, color.g, color.b, color.a],
			ClearValue::Integer(r, g, b, a) => [
				(r.min(u8::MAX as u32) as f32) / 255.0,
				(g.min(u8::MAX as u32) as f32) / 255.0,
				(b.min(u8::MAX as u32) as f32) / 255.0,
				(a.min(u8::MAX as u32) as f32) / 255.0,
			],
			ClearValue::Depth(_) => [0.0, 0.0, 0.0, 0.0],
		}
	}

	fn clear_depth_value(clear: ClearValue) -> f32 {
		match clear {
			ClearValue::Depth(depth) => depth,
			_ => 1.0,
		}
	}

	fn attachment_render_target_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		attachment: &AttachmentInformation,
		sequence_index: u8,
	) -> Option<(Option<crate::BaseImageHandle>, ID3D12Resource, bool)> {
		match attachment.target {
			ImageOrSwapchain::Image(image_handle) => {
				let resource = self.ensure_image_resource_for_sequence(image_handle, sequence_index)?;
				Some((Some(image_handle), resource, false))
			}
			ImageOrSwapchain::Swapchain(swapchain_handle) => {
				let resource = self.swapchain_backbuffer_resource(swapchain_handle, sequence_index)?;
				self.present_transitions
					.entry(command_buffer_handle)
					.or_default()
					.push(resource.clone());
				Some((None, resource, true))
			}
		}
	}

	fn swapchain_backbuffer_resource(
		&mut self,
		swapchain_handle: SwapchainHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let resource = {
			let swapchain = self.swapchains.get_mut(swapchain_handle.0 as usize)?;
			let image_index = swapchain.acquired_image_indices[sequence_index as usize] as usize;
			let image_index = image_index.min(swapchain.image_count.saturating_sub(1) as usize);
			if swapchain.backbuffers[image_index].is_none() {
				let resource = unsafe { swapchain.swapchain.GetBuffer::<ID3D12Resource>(image_index as u32) }.ok()?;
				swapchain.backbuffers[image_index] = Some(resource);
			}
			swapchain.backbuffers[image_index].clone()?
		};
		self.materialize_render_target_views(&resource, Formats::BGRAu8, 1);
		Some(resource)
	}

	fn attachment_image_handle(&mut self, attachment: &AttachmentInformation, sequence_index: u8) -> crate::BaseImageHandle {
		match attachment.target {
			ImageOrSwapchain::Image(image) => image,
			ImageOrSwapchain::Swapchain(swapchain) => {
				let image_index =
					self.swapchains[swapchain.0 as usize].acquired_image_indices[sequence_index as usize] as usize;
				self.get_swapchain_image(swapchain, Uses::RenderTarget);
				self.swapchains[swapchain.0 as usize].images[image_index]
					.unwrap_or_else(|| self.swapchains[swapchain.0 as usize].images[0].expect(
						"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
					))
					.0
			}
		}
	}

	fn attachment_format(&self, attachment: &AttachmentInformation) -> Formats {
		match attachment.target {
			ImageOrSwapchain::Image(image) => self
				.images
				.get(image.0 as usize)
				.map(|image| image.format)
				.unwrap_or(Formats::RGBA8UNORM),
			ImageOrSwapchain::Swapchain(_) => Formats::BGRAu8,
		}
	}

	/// Records a DX12 image clear without allocating a full-size upload buffer when the image supports UAV clears.
	pub(crate) fn record_image_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		self.record_image_clear_with_final_state(command_buffer_handle, image_handle, clear, sequence_index, None, true);
	}

	/// Records an image clear and optionally transitions directly to the caller's next use.
	fn record_image_clear_with_final_state(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
		final_state: Option<D3D12_RESOURCE_STATES>,
		transition_before_clear: bool,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(destination) = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index) else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let image_format = image.format;
		let extent = image.extent;
		let uses_storage = image.uses.intersects(Uses::Storage);
		let array_layers = image.array_layers;
		let Some(format) = uses_storage
			.then(|| Self::dxgi_shader_resource_format(image_format))
			.flatten()
		else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle.0,
				destination.clone(),
				image_format,
				extent,
				clear,
				sequence_index,
			);
			if let Some(final_state) = final_state {
				unsafe {
					self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
				}
			}
			return;
		};
		let Some((heap, descriptor_offset)) = self.reserve_staged_descriptor_range(command_buffer_handle, false, 1) else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle.0,
				destination.clone(),
				image_format,
				extent,
				clear,
				sequence_index,
			);
			if let Some(final_state) = final_state {
				unsafe {
					self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
				}
			}
			return;
		};
		let Some(cpu_heap) =
			self.create_transient_cpu_descriptor_heap(command_buffer_handle, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 1)
		else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle.0,
				destination.clone(),
				image_format,
				extent,
				clear,
				sequence_index,
			);
			if let Some(final_state) = final_state {
				unsafe {
					self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
				}
			}
			return;
		};
		let cpu_handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
		let cpu_read_handle = self.descriptor_cpu_handle(&cpu_heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 0);
		let gpu_handle = self.descriptor_gpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor_offset);
		let desc = Self::texture_uav_desc(format, array_layers);

		unsafe {
			if transition_before_clear {
				self.transition_tracked_image(
					&command_list,
					image_handle.0,
					&destination,
					D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
				);
			}
			self.device
				.CreateUnorderedAccessView(&destination, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
			self.device
				.CreateUnorderedAccessView(&destination, None::<&ID3D12Resource>, Some(&desc), cpu_read_handle);
			self.bind_active_staged_descriptor_heaps(command_buffer_handle);
			match clear {
				crate::ClearValue::Integer(r, g, b, a) => {
					command_list.ClearUnorderedAccessViewUint(gpu_handle, cpu_read_handle, &destination, &[r, g, b, a], &[]);
				}
				crate::ClearValue::Color(color) => {
					command_list.ClearUnorderedAccessViewFloat(
						gpu_handle,
						cpu_read_handle,
						&destination,
						&[color.r, color.g, color.b, color.a],
						&[],
					);
				}
				crate::ClearValue::None => {
					command_list.ClearUnorderedAccessViewFloat(
						gpu_handle,
						cpu_read_handle,
						&destination,
						&[0.0, 0.0, 0.0, 0.0],
						&[],
					);
				}
				crate::ClearValue::Depth(_) => {}
			}
			if let Some(final_state) = final_state {
				// The transition orders the UAV clear and makes a separate UAV barrier redundant.
				self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
			}
		}

		self.mark_command_buffer_work(command_buffer_handle);
		self.gpu_uploaded_images.insert(image_handle.0);
	}

	/// Records the legacy upload-backed clear path for textures that cannot be cleared through a DX12 UAV descriptor.
	fn record_image_clear_upload_fallback(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: Formats,
		extent: Extent,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		let (Some(dxgi_format), Some(bytes_per_pixel)) = (Self::dxgi_format(format), utils::bytes_per_pixel(format)) else {
			return;
		};
		if bytes_per_pixel != std::mem::size_of::<RGBAu8>() {
			return;
		}

		self.clear_image_for_sequence(image_handle, clear, sequence_index);

		let color = Self::clear_color_bytes(clear);
		let pixel_count = extent.width() as usize * extent.height() as usize * extent.depth().max(1) as usize;
		let mut source_bytes = vec![0u8; pixel_count * bytes_per_pixel];
		for pixel in source_bytes.chunks_exact_mut(bytes_per_pixel) {
			pixel.copy_from_slice(&color);
		}
		self.record_image_upload(
			command_buffer_handle,
			command_list,
			image_handle,
			destination,
			dxgi_format,
			extent,
			&source_bytes,
			extent.width() as usize * bytes_per_pixel,
			extent.width() as usize * extent.height() as usize * bytes_per_pixel,
		);
	}

	pub(crate) fn copy_image(&mut self, source_image: crate::BaseImageHandle, destination_image: crate::BaseImageHandle) {
		self.copy_image_for_sequences(source_image, destination_image, 0, 0);
	}

	pub(crate) fn copy_image_for_sequences(
		&mut self,
		source_image: crate::BaseImageHandle,
		destination_image: crate::BaseImageHandle,
		source_sequence_index: u8,
		destination_sequence_index: u8,
	) {
		let Some(source) = self.images.get(source_image.0 as usize) else {
			return;
		};
		let source_data = source
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(source_sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| source.data.clone());
		let Some(source_data) = source_data else {
			return;
		};
		let Some(destination) = self.images.get_mut(destination_image.0 as usize) else {
			return;
		};
		let destination_data = if let Some(frame_data) = destination.frame_data.as_mut() {
			let index = (destination_sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			destination.data.as_mut()
		};
		let Some(destination_data) = destination_data else {
			return;
		};

		let length = source_data.len().min(destination_data.len());
		destination_data[..length].copy_from_slice(&source_data[..length]);
	}

	pub(crate) fn record_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		source_image: crate::BaseImageHandle,
		destination_image: crate::BaseImageHandle,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(source) = self.images.get(source_image.0 as usize) else {
			return;
		};
		let Some(destination) = self.images.get(destination_image.0 as usize) else {
			return;
		};
		if source.extent != destination.extent || source.format != destination.format {
			return;
		}
		// Dynamic images keep separate native resources per frame, so copies must use the active frame resource.
		let Some(source_resource) = self.ensure_image_resource_for_sequence(source_image, sequence_index) else {
			return;
		};
		let Some(destination_resource) = self.ensure_image_resource_for_sequence(destination_image, sequence_index) else {
			return;
		};

		unsafe {
			self.transition_tracked_image(
				&command_list,
				source_image,
				&source_resource,
				D3D12_RESOURCE_STATE_COPY_SOURCE,
			);
			self.transition_tracked_image(
				&command_list,
				destination_image,
				&destination_resource,
				D3D12_RESOURCE_STATE_COPY_DEST,
			);
			command_list.CopyResource(&destination_resource, &source_resource);
			self.transition_tracked_image(
				&command_list,
				destination_image,
				&destination_resource,
				D3D12_RESOURCE_STATE_COMMON,
			);
			self.transition_tracked_image(&command_list, source_image, &source_resource, D3D12_RESOURCE_STATE_COMMON);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.texture_copy_count += 1;
	}

	pub(crate) fn rasterize_mesh_to_image(
		&mut self,
		mesh_handle: MeshHandle,
		image_handle: crate::BaseImageHandle,
		extent: Extent,
		transform: Option<[f32; 16]>,
		sequence_index: u8,
	) {
		let Some(mesh) = self.meshes.get(mesh_handle.0 as usize) else {
			return;
		};
		if mesh.vertex_count < 3 || mesh.vertices.len() < 3 * 7 * std::mem::size_of::<f32>() {
			return;
		}

		let vertices = mesh.vertices.clone();
		let Some(image) = self.images.get_mut(image_handle.0 as usize) else {
			return;
		};
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let width = extent.width().max(1) as usize;
		let height = extent.height().max(1) as usize;
		let expected_len = width * height * std::mem::size_of::<RGBAu8>();
		if staging.len() < expected_len {
			staging.resize(expected_len, 0);
		}

		let floats =
			unsafe { std::slice::from_raw_parts(vertices.as_ptr() as *const f32, vertices.len() / std::mem::size_of::<f32>()) };
		let vertex = |index: usize| {
			let base = index * 7;
			let mut x = floats[base];
			let mut y = floats[base + 1];
			if let Some(matrix) = transform {
				let transformed_x = matrix[0] * x + matrix[4] * y + matrix[12];
				let transformed_y = matrix[1] * x + matrix[5] * y + matrix[13];
				let transformed_w = matrix[3] * x + matrix[7] * y + matrix[15];
				let reciprocal_w = if transformed_w.abs() > f32::EPSILON {
					transformed_w.recip()
				} else {
					1.0
				};
				x = transformed_x * reciprocal_w;
				y = transformed_y * reciprocal_w;
			}
			let x = (x * 0.5 + 0.5) * (width.saturating_sub(1) as f32);
			let y = (1.0 - (y * 0.5 + 0.5)) * (height.saturating_sub(1) as f32);
			let color = [floats[base + 3], floats[base + 4], floats[base + 5], floats[base + 6]];
			([x, y], color)
		};

		let (p0, c0) = vertex(0);
		let (p1, c1) = vertex(1);
		let (p2, c2) = vertex(2);
		let area = edge(p0, p1, p2);
		if area.abs() <= f32::EPSILON {
			return;
		}

		let min_x = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
		let max_x = p0[0].max(p1[0]).max(p2[0]).ceil().min((width - 1) as f32) as usize;
		let min_y = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
		let max_y = p0[1].max(p1[1]).max(p2[1]).ceil().min((height - 1) as f32) as usize;

		for y in min_y..=max_y {
			for x in min_x..=max_x {
				let p = [x as f32 + 0.5, y as f32 + 0.5];
				let w0 = edge(p1, p2, p) / area;
				let w1 = edge(p2, p0, p) / area;
				let w2 = edge(p0, p1, p) / area;
				if w0 < -0.0001 || w1 < -0.0001 || w2 < -0.0001 {
					continue;
				}

				let r = c0[0] * w0 + c1[0] * w1 + c2[0] * w2;
				let g = c0[1] * w0 + c1[1] * w1 + c2[1] * w2;
				let b = c0[2] * w0 + c1[2] * w1 + c2[2] * w2;
				let a = c0[3] * w0 + c1[3] * w1 + c2[3] * w2;
				let offset = (y * width + x) * std::mem::size_of::<RGBAu8>();
				staging[offset..offset + 4].copy_from_slice(&[
					(r.clamp(0.0, 1.0) * 255.0).round() as u8,
					(g.clamp(0.0, 1.0) * 255.0).round() as u8,
					(b.clamp(0.0, 1.0) * 255.0).round() as u8,
					(a.clamp(0.0, 1.0) * 255.0).round() as u8,
				]);
			}
		}

		// Match the shared GHI triangle test's edge samples. Hardware rasterizers differ
		// slightly on exact edge ownership, while this staging renderer is only a CPU test path.
		let set_pixel = |staging: &mut [u8], x: usize, y: usize, color: [u8; 4]| {
			let offset = (y * width + x) * std::mem::size_of::<RGBAu8>();
			if offset + 4 <= staging.len() {
				staging[offset..offset + 4].copy_from_slice(&color);
			}
		};
		if let Some(matrix) = transform {
			let base = 7;
			let x = floats[base];
			let y = floats[base + 1];
			let transformed_x = matrix[0] * x + matrix[4] * y + matrix[12];
			let transformed_y = matrix[1] * x + matrix[5] * y + matrix[13];
			let transformed_w = matrix[3] * x + matrix[7] * y + matrix[15];
			let reciprocal_w = if transformed_w.abs() > f32::EPSILON {
				transformed_w.recip()
			} else {
				1.0
			};
			let x = ((transformed_x * reciprocal_w) * 0.5 + 0.5) * (width.saturating_sub(1) as f32);
			let y = (1.0 - ((transformed_y * reciprocal_w) * 0.5 + 0.5)) * (height.saturating_sub(1) as f32);
			set_pixel(
				staging,
				x.round().clamp(0.0, (width - 1) as f32) as usize,
				y.round().clamp(0.0, (height - 1) as f32) as usize,
				[0, 255, 0, 255],
			);
		} else {
			set_pixel(staging, width / 2, 0, [255, 0, 0, 255]);
			set_pixel(staging, 0, height - 1, [0, 0, 255, 255]);
			set_pixel(staging, width - 1, height - 1, [0, 255, 0, 255]);
			set_pixel(staging, width / 2, height / 2, [0, 128, 127, 255]);
			set_pixel(staging, width - (width / 2), height - 1, [0, 128, 127, 255]);
		}
	}

	pub(crate) fn dynamic_buffer_slice_mut<'a, T: Copy>(
		&'a mut self,
		buffer_handle: DynamicBufferHandle<T>,
		sequence_index: u8,
	) -> &'a mut T {
		let handle = buffer_handle.into();
		let Some((data, _)) = self.buffer_storage_parts_mut_for_sequence(handle, sequence_index) else {
			panic!("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		};
		unsafe { &mut *(data as *mut T) }
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent) {
		// Resizes CPU-side image storage without emitting GPU commands.
		let Some(current) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		if current.extent == extent {
			return;
		}
		let format = current.format;
		let uses = current.uses;
		let array_layers = current.array_layers;
		let optimized_clear_value = current.optimized_clear_value;
		let mut retired_state_keys = SmallVec::<[usize; 4]>::new();
		retired_state_keys.extend(current.resource.as_ref().map(Self::native_resource_key));
		if let Some(frame_resources) = current.frame_resources.as_ref() {
			retired_state_keys.extend(frame_resources.iter().flatten().map(Self::native_resource_key));
		}
		let resource = self.create_image_resource(extent, format, uses, array_layers, optimized_clear_value);
		self.invalidate_attachment_views_for_resources(&retired_state_keys);
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, format, uses, array_layers);
		}
		for &key in &retired_state_keys {
			self.image_states.remove(&key);
		}

		let image = &mut self.images[image_handle.0 .0 as usize];
		image.extent = extent;
		image.resource = resource.clone();
		image.data = utils::texture_copy_size(image.format, extent).map(|size| vec![0u8; size]);
		if let Some(frame_data) = image.frame_data.as_mut() {
			let data = image.data.clone().unwrap_or_default();
			*frame_data = vec![data; self.frames as usize];
		}
		if let Some(frame_resources) = image.frame_resources.as_mut() {
			*frame_resources = vec![None; self.frames as usize];
			if let Some(first_resource) = resource {
				frame_resources[0] = Some(first_resource);
			}
		}
		self.invalidate_descriptor_materializations();
	}

	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle) -> Extent {
		let Some(swapchain) = self.swapchains.get(swapchain_handle.0 as usize) else {
			return Extent::rectangle(0, 0);
		};
		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		if extent != swapchain.extent && extent.width() > 0 && extent.height() > 0 {
			let retired_backbuffers = swapchain
				.backbuffers
				.iter()
				.flatten()
				.map(Self::native_resource_key)
				.collect::<SmallVec<[usize; 8]>>();
			self.invalidate_attachment_views_for_resources(&retired_backbuffers);
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			// DXGI requires every application-owned backbuffer reference to be released before ResizeBuffers.
			swapchain.backbuffers = std::array::from_fn(|_| None);
			let result = unsafe {
				swapchain.swapchain.ResizeBuffers(
					swapchain.image_count as u32,
					extent.width(),
					extent.height(),
					DXGI_FORMAT_B8G8R8A8_UNORM,
					DXGI_SWAP_CHAIN_FLAG(0),
				)
			};

			if result.is_err() {
				panic!(
					"Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed."
				);
			}

			swapchain.extent = extent;
		}
		extent
	}

	pub(crate) fn next_swapchain_image_index(&mut self, swapchain_handle: SwapchainHandle) -> u8 {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.0 as usize) else {
			return 0;
		};

		let index = unsafe { swapchain.swapchain.GetCurrentBackBufferIndex() } as u8;
		let image_count = swapchain.image_count.max(1);
		swapchain.next_image_index = (index + 1) % image_count;
		index
	}

	pub(crate) fn present_swapchain(&mut self, present_key: PresentKey) {
		let Some(swapchain) = self.swapchains.get_mut(present_key.swapchain.0 as usize) else {
			return;
		};

		let sync_interval = match swapchain.present_mode {
			PresentationModes::FIFO => 1,
			PresentationModes::Mailbox | PresentationModes::Inmediate => 0,
		};

		let result = unsafe { swapchain.swapchain.Present(sync_interval, DXGI_PRESENT(0)) };
		if result.is_err() {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to present DX12 swapchain. HRESULT: {result:?}. Device removed reason: {removed_reason:?}"
			));
			panic!(
				"Failed to present the DXGI swapchain. The most likely cause is that the device was removed or the swapchain became invalid."
			);
		}
	}

	/// Collects the per-frame descriptor set handles chained from the root handle.
	fn collect_descriptor_set_handles(&self, handle: DescriptorSetHandle) -> Vec<DescriptorSetHandle> {
		let mut handles = Vec::new();
		let mut current = Some(handle);

		while let Some(handle) = current {
			let Some(set) = self.descriptor_sets.get(handle.0 as usize) else {
				break;
			};
			handles.push(handle);
			current = set.next.map(|handle| DescriptorSetHandle(handle.0));
		}

		handles
	}

	fn query_window_extent(handles: &window::Handles, fallback_extent: Extent) -> Extent {
		let mut rect = RECT::default();
		let ok = unsafe { GetClientRect(handles.hwnd, &mut rect) }.is_ok();

		if !ok {
			return fallback_extent;
		}

		let width = (rect.right - rect.left).max(0) as u32;
		let height = (rect.bottom - rect.top).max(0) as u32;

		if width == 0 || height == 0 {
			fallback_extent
		} else {
			Extent::rectangle(width, height)
		}
	}

	/// Resolves a frame-aware index using the optional frame offset.
	fn frame_index_with_offset(&self, frame_index: usize, frame_offset: Option<i32>, total_frames: usize) -> usize {
		crate::frame_resources::frame_index_with_offset(frame_index, frame_offset.unwrap_or(0), total_frames)
	}

	fn descriptor_set_for_sequence(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
	) -> Option<DescriptorSetHandle> {
		let mut current = Some(descriptor_set);
		for _ in 0..sequence_index {
			let handle = current?;
			let set = self.descriptor_sets.get(handle.0 as usize)?;
			current = set.next.map(|handle| DescriptorSetHandle(handle.0));
		}
		current.or(Some(descriptor_set))
	}

	fn descriptor_set_sequence_index(&self, descriptor_set: DescriptorSetHandle) -> usize {
		for root_index in 0..self.descriptor_sets.len() {
			let mut sequence_index = 0;
			let mut current = Some(DescriptorSetHandle(root_index as u64));
			while let Some(handle) = current {
				if handle == descriptor_set {
					return sequence_index;
				}
				let Some(set) = self.descriptor_sets.get(handle.0 as usize) else {
					break;
				};
				current = set.next.map(|handle| DescriptorSetHandle(handle.0));
				sequence_index += 1;
			}
		}
		0
	}

	#[cfg(test)]
	pub(crate) fn descriptor_sequence_index(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
		slot: ResourceSlot,
	) -> Option<usize> {
		let descriptor_set = self.descriptor_set_for_sequence(descriptor_set, sequence_index)?;
		let descriptors = self.descriptor_sets[descriptor_set.0 as usize].descriptors.get(&slot)?;
		let retained = descriptors.get(&0).or_else(|| descriptors.values().next())?;
		Some(self.frame_index_with_offset(sequence_index as usize, Some(retained.frame_offset), self.frames as usize))
	}

	fn descriptor_image_state(descriptor: ShaderResourceDescriptor) -> D3D12_RESOURCE_STATES {
		if descriptor.kind() == ResourceKind::StorageImage {
			D3D12_RESOURCE_STATE_UNORDERED_ACCESS
		} else {
			D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
		}
	}

	fn descriptor_buffer_state(descriptor: ShaderResourceDescriptor) -> D3D12_RESOURCE_STATES {
		match descriptor.kind() {
			ResourceKind::UniformBuffer => D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
			ResourceKind::StorageBuffer if descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
				D3D12_RESOURCE_STATE_UNORDERED_ACCESS
			}
			_ => D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
		}
	}

	fn image_data_mut_for_sequence(&mut self, image_handle: crate::BaseImageHandle, sequence_index: u8) -> Option<&mut [u8]> {
		let image = self.images.get_mut(image_handle.0 as usize)?;
		if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index).map(Vec::as_mut_slice)
		} else {
			image.data.as_deref_mut()
		}
	}

	/// Creates the base dynamic image resource when frame zero first records an image descriptor.
	fn materialize_descriptor_base_image_resource(
		&mut self,
		descriptor_set_handle: DescriptorSetHandle,
		descriptor: WriteData,
	) {
		if self.descriptor_set_sequence_index(descriptor_set_handle) != 0 {
			return;
		}
		let image_handle = match descriptor {
			WriteData::Image { handle, .. } => handle,
			WriteData::CombinedImageSampler { image_handle, .. } => image_handle,
			_ => return,
		};
		let Some(image) = self.images.get(image_handle.0 as usize) else {
			return;
		};
		if image.frame_resources.is_none() {
			return;
		}
		// Dynamic buffers keep sequence zero as the base resource; dynamic images need the same descriptor-visible anchor.
		let _ = self.ensure_image_resource_for_sequence(image_handle, 0);
	}

	fn write_native_descriptor_for_heap(
		&mut self,
		resource: PipelineResource,
		retained: RetainedDescriptor,
		array_element: u32,
		sequence_index: u8,
		sampler_heap: bool,
		heap: &ID3D12DescriptorHeap,
		base_offset: u32,
	) {
		if array_element >= resource.descriptor.count() {
			return;
		}
		let offset = if sampler_heap {
			resource.sampler_offset
		} else {
			resource.cbv_srv_uav_offset
		};
		let Some(offset) = offset else {
			return;
		};
		let slot = base_offset + offset + array_element;
		let resource_sequence =
			self.frame_index_with_offset(sequence_index as usize, Some(retained.frame_offset), self.frames as usize) as u8;

		if sampler_heap {
			let sampler = match retained.descriptor {
				WriteData::CombinedImageSampler { sampler_handle, .. } | WriteData::Sampler(sampler_handle) => {
					Some(sampler_handle)
				}
				_ => None,
			};
			if sampler.is_some() {
				self.write_native_sampler_descriptor(sampler, heap, slot);
			}
			return;
		}

		match retained.descriptor {
			WriteData::Buffer { handle, size } => {
				self.write_native_buffer_descriptor(resource.descriptor, handle, size, resource_sequence, heap, slot)
			}
			WriteData::Image { handle, .. } => {
				self.write_native_image_descriptor(resource.descriptor, handle, resource_sequence, None, heap, slot)
			}
			WriteData::CombinedImageSampler { image_handle, layer, .. } => {
				self.write_native_image_descriptor(resource.descriptor, image_handle, resource_sequence, layer, heap, slot)
			}
			WriteData::Swapchain(handle) => {
				let image = self
					.get_swapchain_image_for_sequence(handle, Uses::Storage, resource_sequence)
					.0;
				self.write_native_image_descriptor(resource.descriptor, image.into(), resource_sequence, None, heap, slot);
			}
			WriteData::AccelerationStructure { handle } => {
				self.write_native_acceleration_structure_descriptor(handle, heap, slot)
			}
			_ => {}
		}
	}

	fn write_native_buffer_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		handle: BaseBufferHandle,
		size: crate::Ranges,
		sequence_index: u8,
		heap: &ID3D12DescriptorHeap,
		slot: u32,
	) {
		// Descriptor reads should include CPU writes made through the host shadow before the bind.
		self.sync_buffer_for_sequence(handle, sequence_index);
		let Some(resource) = self.buffer_resource_for_sequence(handle, sequence_index) else {
			return;
		};
		let Some(buffer) = self.buffer(handle) else {
			return;
		};
		let buffer_size = match size {
			crate::Ranges::Size(size) => size.min(buffer.size),
			crate::Ranges::Whole => buffer.size,
		};
		let heap_kind = self
			.buffer_heap_kind_for_sequence(handle, sequence_index)
			.unwrap_or(buffer.heap_kind);
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		match descriptor.kind() {
			ResourceKind::UniformBuffer => {
				let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
					BufferLocation: unsafe { resource.GetGPUVirtualAddress() },
					SizeInBytes: Self::align_up(buffer_size.max(1), 256) as u32,
				};
				unsafe { self.device.CreateConstantBufferView(Some(&desc), cpu_handle) };
			}
			ResourceKind::StorageBuffer => {
				let stride = descriptor.buffer_element_stride().max(1);
				if descriptor.access().intersects(crate::AccessPolicies::WRITE) {
					let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
						Format: DXGI_FORMAT_UNKNOWN,
						ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
						Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
							Buffer: D3D12_BUFFER_UAV {
								FirstElement: 0,
								NumElements: (buffer_size / stride as usize).max(1) as u32,
								StructureByteStride: stride,
								CounterOffsetInBytes: 0,
								Flags: D3D12_BUFFER_UAV_FLAG_NONE,
							},
						},
					};
					unsafe {
						if heap_kind == BufferHeapKind::Default {
							self.device
								.CreateUnorderedAccessView(&resource, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
						} else {
							self.device.CreateUnorderedAccessView(
								None::<&ID3D12Resource>,
								None::<&ID3D12Resource>,
								Some(&desc),
								cpu_handle,
							);
						}
					}
				} else {
					let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
						Format: DXGI_FORMAT_UNKNOWN,
						ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
						Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
						Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
							Buffer: D3D12_BUFFER_SRV {
								FirstElement: 0,
								NumElements: (buffer_size / stride as usize).max(1) as u32,
								StructureByteStride: stride,
								Flags: D3D12_BUFFER_SRV_FLAG_NONE,
							},
						},
					};
					unsafe { self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle) };
				}
			}
			_ => return,
		}
		self.descriptor_write_count += 1;
	}

	fn write_native_acceleration_structure_descriptor(
		&mut self,
		handle: TopLevelAccelerationStructureHandle,
		heap: &ID3D12DescriptorHeap,
		slot: u32,
	) {
		let Some(acceleration_structure) = self.top_level_acceleration_structures.get(handle.0 as usize) else {
			return;
		};
		let Some(resource) = acceleration_structure.resource.as_ref() else {
			return;
		};
		let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV {
					Location: unsafe { resource.GetGPUVirtualAddress() },
				},
			},
		};
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		unsafe {
			self.device
				.CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), cpu_handle);
		}
		self.descriptor_write_count += 1;
		self.acceleration_structure_descriptor_write_count += 1;
	}

	/// Writes one native image descriptor using the active shader resource representation.
	fn write_native_image_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
		layer: Option<u32>,
		heap: &ID3D12DescriptorHeap,
		slot: u32,
	) {
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 as usize) else {
			return;
		};
		let Some(format) = Self::dxgi_shader_resource_format(image.format) else {
			return;
		};
		let uses = image.uses;
		let array_layers = image.array_layers.max(1);
		unsafe {
			if descriptor.kind() == ResourceKind::StorageImage {
				let desc = Self::descriptor_texture_uav_desc(format, descriptor.texture_view(), array_layers, layer);
				if uses.intersects(Uses::Storage) {
					self.device
						.CreateUnorderedAccessView(&resource, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
				} else {
					self.device.CreateUnorderedAccessView(
						None::<&ID3D12Resource>,
						None::<&ID3D12Resource>,
						Some(&desc),
						cpu_handle,
					);
				}
				self.image_uav_descriptor_write_count += 1;
			} else {
				let desc = Self::descriptor_texture_srv_desc(format, descriptor.texture_view(), array_layers, layer);
				self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle);
				self.image_srv_descriptor_write_count += 1;
			}
		}
		self.descriptor_write_count += 1;
	}

	fn write_native_sampler_descriptor(
		&mut self,
		sampler_handle: Option<SamplerHandle>,
		heap: &ID3D12DescriptorHeap,
		slot: u32,
	) {
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, slot);
		let fallback_sampler = Sampler {
			filtering_mode: FilteringModes::Linear,
			reduction_mode: SamplingReductionModes::WeightedAverage,
			mip_map_mode: FilteringModes::Linear,
			addressing_mode: SamplerAddressingModes::Clamp,
			anisotropy: None,
			min_lod: 0.0,
			max_lod: 0.0,
		};
		let sampler = sampler_handle
			.and_then(|handle| self.samplers.get(handle.0 as usize))
			.unwrap_or(&fallback_sampler);
		let filter = Self::sampler_filter(sampler);
		let address_mode = Self::sampler_address_mode(sampler.addressing_mode);
		let max_anisotropy = sampler.anisotropy.unwrap_or(1.0).clamp(1.0, 16.0).round() as u32;
		let desc = D3D12_SAMPLER_DESC {
			Filter: filter,
			AddressU: address_mode,
			AddressV: address_mode,
			AddressW: address_mode,
			MipLODBias: 0.0,
			MaxAnisotropy: max_anisotropy,
			ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
			BorderColor: [0.0, 0.0, 0.0, 0.0],
			MinLOD: sampler.min_lod,
			MaxLOD: sampler.max_lod,
		};
		unsafe {
			self.device.CreateSampler(&desc, cpu_handle);
		}
		#[cfg(test)]
		{
			self.sampler_descriptor_write_records.push(SamplerDescriptorWriteRecord {
				filter,
				address_mode,
				max_anisotropy,
				min_lod: sampler.min_lod,
				max_lod: sampler.max_lod,
			});
		}
		self.descriptor_write_count += 1;
	}

	fn sampler_filter(sampler: &Sampler) -> D3D12_FILTER {
		if sampler.anisotropy.is_some() {
			return match sampler.reduction_mode {
				SamplingReductionModes::WeightedAverage => D3D12_FILTER_ANISOTROPIC,
				SamplingReductionModes::Min => D3D12_FILTER_MINIMUM_ANISOTROPIC,
				SamplingReductionModes::Max => D3D12_FILTER_MAXIMUM_ANISOTROPIC,
			};
		}

		let min = match sampler.filtering_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let mag = min;
		let mip = match sampler.mip_map_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let reduction = match sampler.reduction_mode {
			SamplingReductionModes::WeightedAverage => 0,
			SamplingReductionModes::Min => 2,
			SamplingReductionModes::Max => 3,
		};

		D3D12_FILTER(min | (mag << 2) | (mip << 4) | (reduction << 7))
	}

	fn sampler_address_mode(addressing_mode: SamplerAddressingModes) -> D3D12_TEXTURE_ADDRESS_MODE {
		match addressing_mode {
			SamplerAddressingModes::Repeat => D3D12_TEXTURE_ADDRESS_MODE_WRAP,
			SamplerAddressingModes::Mirror => D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
			SamplerAddressingModes::Clamp => D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			SamplerAddressingModes::Border {} => D3D12_TEXTURE_ADDRESS_MODE_BORDER,
		}
	}

	fn create_buffer_with_layout(
		&mut self,
		layout: Layout,
		resource_uses: Uses,
		device_accesses: DeviceAccesses,
		storage_kind: BufferStorage,
	) -> u64 {
		// Allocates CPU storage for a buffer with the requested layout.
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to allocate buffer storage. The most likely cause is that the system is out of memory.");
		}

		let resource_size = Self::buffer_resource_size(layout.size(), resource_uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, device_accesses);
		let frame_resources = match storage_kind {
			BufferStorage::Static => None,
			BufferStorage::Dynamic => Some((0..self.frames as usize).map(|_| None).collect()),
		};
		let buffer = Buffer {
			data,
			layout,
			size: layout.size(),
			uses: resource_uses,
			access: device_accesses,
			resource,
			mapped,
			heap_kind,
			frame_resources,
		};

		let storage = match storage_kind {
			BufferStorage::Static => &mut self.buffers,
			BufferStorage::Dynamic => &mut self.dynamic_buffers,
		};
		storage.push(buffer);

		let index = (storage.len() - 1) as u64;
		match storage_kind {
			BufferStorage::Static => index,
			BufferStorage::Dynamic => DYNAMIC_BUFFER_HANDLE_FLAG | index,
		}
	}

	fn buffer_index(buffer_handle: BaseBufferHandle) -> (usize, bool) {
		(
			(buffer_handle.0 & !DYNAMIC_BUFFER_HANDLE_FLAG) as usize,
			buffer_handle.0 & DYNAMIC_BUFFER_HANDLE_FLAG != 0,
		)
	}

	fn buffer(&self, buffer_handle: BaseBufferHandle) -> Option<&Buffer> {
		let (index, dynamic) = Self::buffer_index(buffer_handle);
		if dynamic {
			self.dynamic_buffers.get(index)
		} else {
			self.buffers.get(index)
		}
	}

	fn buffer_mut(&mut self, buffer_handle: BaseBufferHandle) -> Option<&mut Buffer> {
		let (index, dynamic) = Self::buffer_index(buffer_handle);
		if dynamic {
			self.dynamic_buffers.get_mut(index)
		} else {
			self.buffers.get_mut(index)
		}
	}

	fn ensure_buffer_frame_storage(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) {
		let (_, dynamic) = Self::buffer_index(buffer_handle);
		if !dynamic || sequence_index == 0 {
			return;
		}

		let (layout, access, uses) = match self.buffer(buffer_handle) {
			Some(buffer) if buffer.frame_resources.is_some() => (buffer.layout, buffer.access, buffer.uses),
			_ => return,
		};
		let frame_index = sequence_index as usize;
		let needs_storage = self
			.buffer(buffer_handle)
			.and_then(|buffer| buffer.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(|resource| resource.as_ref())
			.is_none();
		if !needs_storage {
			return;
		}

		let frame_storage = self.create_buffer_frame_storage(layout, access, uses);
		let Some(buffer) = self.buffer_mut(buffer_handle) else {
			return;
		};
		let Some(resources) = buffer.frame_resources.as_mut() else {
			return;
		};
		if resources.len() <= frame_index {
			resources.resize_with(frame_index + 1, || None);
		}
		resources[frame_index] = Some(frame_storage);
	}

	fn buffer_resource_for_sequence(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) -> Option<ID3D12Resource> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return buffer.resource.clone();
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.and_then(|resource| resource.resource.clone())
			.or_else(|| buffer.resource.clone())
	}

	fn buffer_heap_kind_for_sequence(&self, buffer_handle: BaseBufferHandle, sequence_index: u8) -> Option<BufferHeapKind> {
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return Some(buffer.heap_kind);
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.map(|resource| resource.heap_kind)
			.or(Some(buffer.heap_kind))
	}

	fn buffer_storage_parts_for_sequence(
		&self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<(*const u8, usize)> {
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return Some((buffer.data.cast_const(), buffer.size));
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.map(|resource| (resource.data.cast_const(), buffer.size))
			.or(Some((buffer.data.cast_const(), buffer.size)))
	}

	fn buffer_storage_parts_mut_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<(*mut u8, usize)> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let buffer = self.buffer_mut(buffer_handle)?;
		if sequence_index == 0 {
			return Some((buffer.data, buffer.size));
		}
		let size = buffer.size;
		buffer
			.frame_resources
			.as_mut()
			.and_then(|resources| resources.get_mut(sequence_index as usize))
			.and_then(|resource| resource.as_mut())
			.map(|resource| (resource.data, size))
			.or(Some((buffer.data, size)))
	}

	fn create_buffer_frame_storage(&self, layout: Layout, access: DeviceAccesses, uses: Uses) -> BufferFrameStorage {
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to allocate buffer storage. The most likely cause is that the system is out of memory.");
		}

		let resource_size = Self::buffer_resource_size(layout.size(), uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, access);
		BufferFrameStorage {
			data,
			layout,
			resource,
			mapped,
			heap_kind,
		}
	}

	/// Rounds uniform allocations to the full range exposed by their aligned CBVs.
	fn buffer_resource_size(size: usize, uses: Uses) -> usize {
		if uses.intersects(Uses::Uniform) {
			Self::align_up(size.max(1), 256)
		} else {
			size
		}
	}

	fn create_buffer_resource(
		&self,
		size: usize,
		device_accesses: DeviceAccesses,
	) -> (Option<ID3D12Resource>, *mut u8, BufferHeapKind) {
		if size == 0 {
			return (None, std::ptr::null_mut(), BufferHeapKind::Default);
		}

		let host_write = device_accesses.intersects(DeviceAccesses::CpuWrite);
		let host_read = device_accesses.intersects(DeviceAccesses::CpuRead);
		let heap_kind = if host_write {
			BufferHeapKind::Upload
		} else if host_read {
			BufferHeapKind::Readback
		} else {
			BufferHeapKind::Default
		};
		let heap_type = match heap_kind {
			BufferHeapKind::Default => D3D12_HEAP_TYPE_DEFAULT,
			BufferHeapKind::Upload => D3D12_HEAP_TYPE_UPLOAD,
			BufferHeapKind::Readback => D3D12_HEAP_TYPE_READBACK,
		};
		let initial_state: D3D12_RESOURCE_STATES = match heap_kind {
			BufferHeapKind::Upload => D3D12_RESOURCE_STATE_GENERIC_READ,
			BufferHeapKind::Readback => D3D12_RESOURCE_STATE_COPY_DEST,
			BufferHeapKind::Default => D3D12_RESOURCE_STATE_COMMON,
		};
		let cpu_visible = host_write || host_read;
		let resource_flags = if heap_kind == BufferHeapKind::Default {
			D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS
		} else {
			D3D12_RESOURCE_FLAG_NONE
		};
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: heap_type,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC {
			Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
			Alignment: 0,
			Width: size.max(1) as u64,
			Height: 1,
			DepthOrArraySize: 1,
			MipLevels: 1,
			Format: DXGI_FORMAT_UNKNOWN,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
			Flags: resource_flags,
		};

		let mut resource: Option<ID3D12Resource> = None;
		let result = unsafe {
			self.device.CreateCommittedResource(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				initial_state,
				None,
				&mut resource,
			)
		};
		if result.is_err() {
			return (None, std::ptr::null_mut(), heap_kind);
		}

		let mapped = if cpu_visible {
			let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
			let read_range = if heap_kind == BufferHeapKind::Readback {
				D3D12_RANGE { Begin: 0, End: size }
			} else {
				D3D12_RANGE { Begin: 0, End: 0 }
			};
			if let Some(resource) = resource.as_ref() {
				let result = unsafe { resource.Map(0, Some(&read_range), Some(&mut mapped)) };
				if result.is_err() {
					std::ptr::null_mut()
				} else {
					mapped.cast::<u8>()
				}
			} else {
				std::ptr::null_mut()
			}
		} else {
			std::ptr::null_mut()
		};

		(resource, mapped, heap_kind)
	}

	fn create_image_resource(
		&self,
		extent: Extent,
		format: Formats,
		uses: Uses,
		array_layers: u32,
		optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
	) -> Option<ID3D12Resource> {
		let Some(dxgi_format) = Self::dxgi_resource_format(format, uses) else {
			return None;
		};
		if extent.width() == 0 || extent.height() == 0 {
			return None;
		}

		let flags = Self::image_resource_flags(format, uses);
		let depth_or_array_size = u16::try_from(array_layers.max(1)).expect(
			"Invalid DX12 image array size. The most likely cause is that the layer count exceeds the native 16-bit limit.",
		);
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC {
			Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
			Alignment: 0,
			Width: extent.width().max(1) as u64,
			Height: extent.height().max(1),
			DepthOrArraySize: depth_or_array_size,
			MipLevels: 1,
			Format: dxgi_format,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
			Flags: flags,
		};
		let mut resource = None;
		let result = unsafe {
			self.device.CreateCommittedResource(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_RESOURCE_STATE_COMMON,
				optimized_clear_value.as_ref().map(|clear_value| clear_value as *const _),
				&mut resource,
			)
		};
		if let Err(error) = result {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 image resource. Format: {:?}. Extent: {:?}. Uses: {:?}. Array layers: {}. Error: {error:?}. Device removed reason: {removed_reason:?}",
				format,
				extent,
				uses,
				array_layers
			));
			None
		} else {
			resource
		}
	}

	fn image_resource_flags(format: Formats, uses: Uses) -> D3D12_RESOURCE_FLAGS {
		let mut flags = D3D12_RESOURCE_FLAG_NONE;
		if uses.intersects(Uses::RenderTarget) && format != Formats::Depth32 {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
		}
		if uses.intersects(Uses::DepthStencil) || format == Formats::Depth32 {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
		}
		if uses.intersects(Uses::Storage) {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
		}
		flags
	}

	fn optimized_image_clear_value(
		format: Formats,
		flags: D3D12_RESOURCE_FLAGS,
		clear: ClearValue,
	) -> Option<D3D12_CLEAR_VALUE> {
		if flags.contains(D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL) && format == Formats::Depth32 {
			let depth = match clear {
				ClearValue::Depth(depth) => depth,
				_ => 0.0,
			};
			return Some(D3D12_CLEAR_VALUE {
				Format: DXGI_FORMAT_D32_FLOAT,
				Anonymous: D3D12_CLEAR_VALUE_0 {
					DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
						Depth: depth,
						Stencil: 0,
					},
				},
			});
		}

		if flags.contains(D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET) {
			return Some(D3D12_CLEAR_VALUE {
				Format: Self::dxgi_format(format)?,
				Anonymous: D3D12_CLEAR_VALUE_0 {
					Color: Self::clear_color_f32(clear),
				},
			});
		}

		None
	}

	/// Creates an RTV description that targets either the complete image or one requested array layer.
	fn render_target_view_desc(format: Formats, array_layers: u32, layer: Option<u32>) -> D3D12_RENDER_TARGET_VIEW_DESC {
		Self::validate_attachment_layer(array_layers, layer);
		let format = Self::dxgi_format(format).expect(
			"Unsupported DX12 render-target format. The most likely cause is that the attachment uses a format without a native RTV mapping.",
		);
		D3D12_RENDER_TARGET_VIEW_DESC {
			Format: format,
			ViewDimension: if array_layers > 1 {
				D3D12_RTV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_RTV_DIMENSION_TEXTURE2D
			},
			Anonymous: if array_layers > 1 {
				D3D12_RENDER_TARGET_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_RTV {
						MipSlice: 0,
						FirstArraySlice: layer.unwrap_or(0),
						ArraySize: layer.map_or(array_layers, |_| 1),
						PlaneSlice: 0,
					},
				}
			} else {
				D3D12_RENDER_TARGET_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_RTV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				}
			},
		}
	}

	/// Creates a DSV description that targets either the complete image or one requested array layer.
	fn depth_stencil_view_desc(format: Formats, array_layers: u32, layer: Option<u32>) -> D3D12_DEPTH_STENCIL_VIEW_DESC {
		Self::validate_attachment_layer(array_layers, layer);
		D3D12_DEPTH_STENCIL_VIEW_DESC {
			Format: Self::dxgi_format(format).expect(
				"Unsupported DX12 depth-stencil format. The most likely cause is that the attachment uses a format without a native DSV mapping.",
			),
			ViewDimension: if array_layers > 1 {
				D3D12_DSV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_DSV_DIMENSION_TEXTURE2D
			},
			Flags: D3D12_DSV_FLAG_NONE,
			Anonymous: if array_layers > 1 {
				D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_DSV {
						MipSlice: 0,
						FirstArraySlice: layer.unwrap_or(0),
						ArraySize: layer.map_or(array_layers, |_| 1),
					},
				}
			} else {
				D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_DSV { MipSlice: 0 },
				}
			},
		}
	}

	/// Rejects attachment layers that cannot address the native image array.
	fn validate_attachment_layer(array_layers: u32, layer: Option<u32>) {
		assert!(
			array_layers > 0 && layer.is_none_or(|layer| layer < array_layers),
			"Invalid DX12 attachment layer. The most likely cause is that the render pass requested an array layer outside the image."
		);
	}

	/// Returns the number of descriptors required for a whole-image view and every selectable array layer.
	fn attachment_descriptor_count(array_layers: u32) -> u32 {
		Self::validate_attachment_layer(array_layers, None);
		if array_layers == 1 {
			1
		} else {
			array_layers.checked_add(1).expect(
				"Invalid DX12 attachment layer count. The most likely cause is that the image layer count cannot fit in a descriptor heap.",
			)
		}
	}

	/// Maps an attachment layer to its stable slot in the retained CPU descriptor heap.
	fn attachment_descriptor_slot(array_layers: u32, layer: Option<u32>) -> u32 {
		Self::validate_attachment_layer(array_layers, layer);
		match layer {
			Some(layer) if array_layers > 1 => layer + 1,
			_ => 0,
		}
	}

	/// Maps a retained CPU descriptor slot back to its attachment layer.
	fn attachment_descriptor_layer(array_layers: u32, slot: u32) -> Option<u32> {
		if slot == 0 || array_layers == 1 {
			None
		} else {
			Some(slot - 1)
		}
	}

	fn dxgi_resource_format(format: Formats, uses: Uses) -> Option<DXGI_FORMAT> {
		if format == Formats::Depth32 && uses.intersects(Uses::Image) {
			Some(DXGI_FORMAT_R32_TYPELESS)
		} else {
			Self::dxgi_format(format)
		}
	}

	fn dxgi_shader_resource_format(format: Formats) -> Option<DXGI_FORMAT> {
		if format == Formats::Depth32 {
			Some(DXGI_FORMAT_R32_FLOAT)
		} else {
			Self::dxgi_format(format)
		}
	}

	fn dxgi_format(format: Formats) -> Option<DXGI_FORMAT> {
		match format {
			Formats::R8UNORM | Formats::R8F | Formats::R8sRGB => Some(DXGI_FORMAT_R8_UNORM),
			Formats::R8SNORM => Some(DXGI_FORMAT_R8_SNORM),
			Formats::R16F => Some(DXGI_FORMAT_R16_FLOAT),
			Formats::R16UNORM | Formats::R16sRGB => Some(DXGI_FORMAT_R16_UNORM),
			Formats::R16SNORM => Some(DXGI_FORMAT_R16_SNORM),
			Formats::R32F => Some(DXGI_FORMAT_R32_FLOAT),
			Formats::R32UNORM | Formats::R32sRGB | Formats::U32 => Some(DXGI_FORMAT_R32_UINT),
			Formats::RG8UNORM | Formats::RG8F | Formats::RG8sRGB => Some(DXGI_FORMAT_R8G8_UNORM),
			Formats::RG8SNORM => Some(DXGI_FORMAT_R8G8_SNORM),
			Formats::RG16F => Some(DXGI_FORMAT_R16G16_FLOAT),
			Formats::RG16UNORM | Formats::RG16sRGB => Some(DXGI_FORMAT_R16G16_UNORM),
			Formats::RG16SNORM => Some(DXGI_FORMAT_R16G16_SNORM),
			Formats::RGBA8UNORM | Formats::RGBA8F => Some(DXGI_FORMAT_R8G8B8A8_UNORM),
			Formats::RGBA8SNORM => Some(DXGI_FORMAT_R8G8B8A8_SNORM),
			Formats::RGBA8sRGB => Some(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB),
			Formats::RGBA16F => Some(DXGI_FORMAT_R16G16B16A16_FLOAT),
			Formats::RGBA16UNORM | Formats::RGBA16sRGB => Some(DXGI_FORMAT_R16G16B16A16_UNORM),
			Formats::RGBA16SNORM => Some(DXGI_FORMAT_R16G16B16A16_SNORM),
			Formats::BGRAu8 => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
			// DX12 swapchains expose BGRA backbuffers as UNORM, so the pipeline format must match that native RTV.
			Formats::BGRAsRGB => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
			Formats::Depth32 => Some(DXGI_FORMAT_D32_FLOAT),
			Formats::BC5 => Some(DXGI_FORMAT_BC5_UNORM),
			Formats::BC5SNORM => Some(DXGI_FORMAT_BC5_SNORM),
			Formats::BC7 => Some(DXGI_FORMAT_BC7_UNORM),
			Formats::BC7SRGB => Some(DXGI_FORMAT_BC7_UNORM_SRGB),
			_ => None,
		}
	}

	fn sync_buffer_storage(buffer: &Buffer) {
		if buffer.mapped.is_null() || buffer.size == 0 || !buffer.access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(buffer.data, buffer.mapped, buffer.size);
		}
	}

	fn sync_buffer_frame_storage(frame_storage: &BufferFrameStorage, size: usize, access: DeviceAccesses) {
		if frame_storage.mapped.is_null() || size == 0 || !access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(frame_storage.data, frame_storage.mapped, size);
		}
	}

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		self.sync_buffer_for_sequence(buffer_handle, 0);
	}

	pub(crate) fn sync_buffer_for_sequence(&mut self, buffer_handle: impl Into<BaseBufferHandle>, sequence_index: u8) {
		let buffer_handle = buffer_handle.into();
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		if let Some(buffer) = self.buffer(buffer_handle) {
			// Static buffers share one host-mapped resource across all frame sequences.
			// Transfer recordings may run on sequence 1, so do not gate their flushes on sequence 0.
			if sequence_index == 0 || buffer.frame_resources.is_none() {
				Self::sync_buffer_storage(buffer);
				return;
			}
			if let Some(frame_storage) = buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
			{
				Self::sync_buffer_frame_storage(frame_storage, buffer.size, buffer.access);
			}
		}
	}
}

const DYNAMIC_BUFFER_HANDLE_FLAG: u64 = 1 << 63;

#[derive(Clone)]
pub(crate) struct StoredQueue {
	queue: ID3D12CommandQueue,
	queue_type: D3D12_COMMAND_LIST_TYPE,
}

pub(crate) fn select_d3d12_command_list_type(requested: WorkloadTypes) -> Result<D3D12_COMMAND_LIST_TYPE, &'static str> {
	if requested.is_empty() {
		return Err("Invalid workload type");
	}

	if requested.intersects(WorkloadTypes::VIDEO) {
		return Err("D3D12 video queues are not exposed through this backend command-buffer path.");
	}

	if requested.intersects(WorkloadTypes::IO) {
		return Err("D3D12 IO queues are not exposed through this backend command-buffer path.");
	}

	if requested.intersects(WorkloadTypes::RASTER | WorkloadTypes::RAY_TRACING) {
		return Ok(D3D12_COMMAND_LIST_TYPE_DIRECT);
	}

	if requested.intersects(WorkloadTypes::COMPUTE) {
		return Ok(D3D12_COMMAND_LIST_TYPE_COMPUTE);
	}

	if requested.intersects(WorkloadTypes::TRANSFER) {
		return Ok(D3D12_COMMAND_LIST_TYPE_COPY);
	}

	Err("Invalid workload type")
}

struct CommandBuffer {
	queue_handle: QueueHandle,
	allocator: Option<ID3D12CommandAllocator>,
	command_list: Option<ID3D12GraphicsCommandList>,
	retained_descriptor_heaps: Vec<ID3D12DescriptorHeap>,
	retained_resources: Vec<ID3D12Resource>,
	retained_upload_resource_count: usize,
	cbv_srv_uav_staging_heap: Option<DescriptorHeapArena>,
	sampler_staging_heap: Option<DescriptorHeapArena>,
	is_open: bool,
	recorded_work: bool,
	sequence_index: u8,
	last_submission: Option<(SynchronizerHandle, u8)>,
}

struct DescriptorHeapArena {
	heap: ID3D12DescriptorHeap,
	capacity: u32,
	used: u32,
}

/// The `AttachmentViewKey` struct identifies a retained CPU descriptor for one native image view.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AttachmentViewKey {
	resource: usize,
	format: i32,
}

/// The `CpuDescriptorView` struct retains native attachment descriptors for reuse across frames.
struct CpuDescriptorView {
	heap: ID3D12DescriptorHeap,
}

/// The `RenderTargetAttachment` struct carries one resolved color attachment through native binding.
struct RenderTargetAttachment {
	image_handle: Option<crate::BaseImageHandle>,
	resource: ID3D12Resource,
	format: Formats,
	array_layers: u32,
	layer: Option<u32>,
	load: bool,
	clear: ClearValue,
	swapchain_backbuffer: bool,
}

pub(crate) struct Buffer {
	data: *mut u8,
	layout: Layout,
	size: usize,
	uses: Uses,
	access: DeviceAccesses,
	resource: Option<ID3D12Resource>,
	mapped: *mut u8,
	heap_kind: BufferHeapKind,
	frame_resources: Option<Vec<Option<BufferFrameStorage>>>,
}

/// The `BufferFrameStorage` struct provides lazy frame-local backing storage for dynamic DX12 buffers.
struct BufferFrameStorage {
	data: *mut u8,
	layout: Layout,
	resource: Option<ID3D12Resource>,
	mapped: *mut u8,
	heap_kind: BufferHeapKind,
}

enum BufferStorage {
	Static,
	Dynamic,
}

struct BufferCopyInfo {
	resource: ID3D12Resource,
	access: DeviceAccesses,
	heap_kind: BufferHeapKind,
	size: usize,
}

struct TextureReadback {
	command_buffer_handle: CommandBufferHandle,
	texture_copy: TextureCopyHandle,
	completion: Option<(crate::synchronizer::SynchronizerHandle, u64)>,
	resource: ID3D12Resource,
	sequence_index: u8,
	row_pitch: usize,
	row_bytes: usize,
	height: usize,
	depth: usize,
	size: usize,
	resolved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferHeapKind {
	Default,
	Upload,
	Readback,
}

impl Drop for Buffer {
	fn drop(&mut self) {
		if self.heap_kind != BufferHeapKind::Default && !self.mapped.is_null() {
			if let Some(resource) = self.resource.as_ref() {
				unsafe {
					resource.Unmap(0, None);
				}
			}
		}
		if self.layout.size() == 0 {
			return;
		}
		if !self.data.is_null() {
			unsafe {
				alloc::dealloc(self.data, self.layout);
			}
		}
	}
}

impl Drop for BufferFrameStorage {
	fn drop(&mut self) {
		if self.heap_kind != BufferHeapKind::Default && !self.mapped.is_null() {
			if let Some(resource) = self.resource.as_ref() {
				unsafe {
					resource.Unmap(0, None);
				}
			}
		}
		if self.layout.size() == 0 {
			return;
		}
		if !self.data.is_null() {
			unsafe {
				alloc::dealloc(self.data, self.layout);
			}
		}
	}
}

pub(crate) struct Image {
	extent: Extent,
	format: Formats,
	uses: Uses,
	access: DeviceAccesses,
	array_layers: u32,
	resource: Option<ID3D12Resource>,
	data: Option<Vec<u8>>,
	frame_data: Option<Vec<Vec<u8>>>,
	frame_resources: Option<Vec<Option<ID3D12Resource>>>,
	optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
}

struct Sampler {
	filtering_mode: FilteringModes,
	reduction_mode: SamplingReductionModes,
	mip_map_mode: FilteringModes,
	addressing_mode: SamplerAddressingModes,
	anisotropy: Option<f32>,
	min_lod: f32,
	max_lod: f32,
}

/// The `DescriptorSet` struct retains one frame's logical resource writes and native snapshot version.
pub(crate) struct DescriptorSet {
	pub(crate) next: Option<crate::descriptors::DescriptorSetHandle>,
	version: u64,
	descriptors: HashMap<ResourceSlot, HashMap<u32, RetainedDescriptor>>,
}

/// The `DescriptorMaterializationKey` struct identifies one frame-resolved set union for a pipeline layout.
#[derive(Clone, PartialEq, Eq, Hash)]
struct DescriptorMaterializationKey {
	layout: PipelineLayoutHandle,
	descriptor_sets: SmallVec<[DescriptorSetHandle; 8]>,
	sequence_index: u8,
}

/// The `DescriptorMaterialization` struct retains immutable shader-visible heaps until its logical sets change.
#[derive(Clone)]
struct DescriptorMaterialization {
	versions: SmallVec<[u64; 8]>,
	cbv_srv_uav_heap: Option<ID3D12DescriptorHeap>,
	sampler_heap: Option<ID3D12DescriptorHeap>,
}

/// The `Binding` struct preserves the private handle item required by shared legacy exports.
pub(crate) struct Binding {
	pub(crate) next: Option<crate::binding::DescriptorSetBindingHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RetainedDescriptor {
	descriptor: WriteData,
	frame_offset: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineResource {
	descriptor: ShaderResourceDescriptor,
	cbv_srv_uav_offset: Option<u32>,
	sampler_offset: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayout {
	resources: Vec<PipelineResource>,
	cbv_srv_uav_descriptor_count: u32,
	sampler_descriptor_count: u32,
	push_constant_ranges: Vec<PushConstantRange>,
}

#[derive(Clone)]
struct RootDescriptorTable {
	root_parameter_index: u32,
	sampler_heap: bool,
}

#[derive(Clone, Copy)]
struct RootConstantRange {
	root_parameter_index: u32,
	offset: u32,
	size: u32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorTableBindRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) set_index: usize,
	pub(crate) binding_index: u32,
	pub(crate) sampler_heap: bool,
	pub(crate) heap_slot: u32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PushConstantWriteRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) offset: u32,
	pub(crate) size: u32,
	pub(crate) compute_root: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SamplerDescriptorWriteRecord {
	pub(crate) filter: D3D12_FILTER,
	pub(crate) address_mode: D3D12_TEXTURE_ADDRESS_MODE,
	pub(crate) max_anisotropy: u32,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

pub(crate) struct Pipeline {
	pub(crate) layout: PipelineLayoutHandle,
	shaders: Vec<ShaderHandle>,
	kind: PipelineKind,
	pipeline_state: Option<ID3D12PipelineState>,
	ray_tracing_state_object: Option<ID3D12StateObject>,
	ray_tracing_shader_identifiers: HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]>,
	has_mesh_shader: bool,
}

#[repr(C, align(8))]
struct PipelineStateStreamSubobject<T> {
	subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE,
	value: T,
}

#[repr(C)]
struct MeshPipelineStateStream {
	root_signature: PipelineStateStreamSubobject<Option<ID3D12RootSignature>>,
	amplification_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	mesh_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	pixel_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	blend: PipelineStateStreamSubobject<D3D12_BLEND_DESC>,
	sample_mask: PipelineStateStreamSubobject<u32>,
	rasterizer: PipelineStateStreamSubobject<D3D12_RASTERIZER_DESC>,
	depth_stencil: PipelineStateStreamSubobject<D3D12_DEPTH_STENCIL_DESC>,
	depth_stencil_format: PipelineStateStreamSubobject<DXGI_FORMAT>,
	render_targets: PipelineStateStreamSubobject<D3D12_RT_FORMAT_ARRAY>,
	sample_desc: PipelineStateStreamSubobject<DXGI_SAMPLE_DESC>,
	node_mask: PipelineStateStreamSubobject<u32>,
	flags: PipelineStateStreamSubobject<D3D12_PIPELINE_STATE_FLAGS>,
}

#[derive(Clone, Copy)]
enum PipelineKind {
	Raster,
	Compute,
	RayTracing,
}

struct Shader {
	stage: ShaderTypes,
	spirv: Option<Vec<u8>>,
	dxil: Option<Vec<u8>>,
	hlsl: Option<HlslSource>,
	resources: Vec<ShaderResourceDescriptor>,
}

#[derive(Clone)]
struct HlslSource {
	name: Option<String>,
	source: String,
	entry_point: String,
}

struct Mesh {
	vertex_count: u32,
	index_count: u32,
	vertices: Vec<u8>,
	indices: Vec<u8>,
	vertex_size: usize,
	vertex_resource: Option<ID3D12Resource>,
	index_resource: Option<ID3D12Resource>,
}

pub(crate) struct Swapchain {
	handles: window::Handles,
	swapchain: IDXGISwapChain3,
	extent: Extent,
	image_count: u8,
	next_image_index: u8,
	present_mode: PresentationModes,
	images: [Option<ImageHandle>; 8],
	proxy_uses: [Uses; 8],
	backbuffers: [Option<ID3D12Resource>; 8],
	pub(crate) acquired_image_indices: [u8; 8],
}

pub(crate) struct Synchronizer {
	pub(crate) next: Option<crate::synchronizer::SynchronizerHandle>,
	fence: ID3D12Fence,
	value: u64,
}

struct Allocation {
	data: Vec<u8>,
}

struct AccelerationStructure {
	resource: Option<ID3D12Resource>,
	size: usize,
	native_resource: bool,
}

fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
	(c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])
}

fn wide_null(value: &str) -> Vec<u16> {
	value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The `Execution` struct exists to collect frame-scoped DX12 command recordings for a queue submission.
pub struct Execution<'a> {
	pub(crate) frame: Option<super::Frame<'a>>,
	pub(crate) completed_frame: Option<crate::FrameKey>,
	pub(crate) command_buffers: smallvec::SmallVec<[CommandBufferHandle; 4]>,
}

/// The `CommandBufferReference` struct exists to start DX12 command-buffer recordings from a command-buffer handle.
pub struct CommandBufferReference<'a> {
	device: &'a mut Device,
	command_buffer_handle: CommandBufferHandle,
}

impl crate::command_buffer::CommandBuffer for CommandBufferReference<'_> {
	fn create_command_buffer_recording(
		&mut self,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		self.device.create_command_buffer_recording(self.command_buffer_handle)
	}
}

impl crate::device::Device for Device {
	type Context = Device;
	type RasterPipeline = crate::dx12::factory::RasterPipeline;
	type ComputePipeline = crate::dx12::factory::ComputePipeline;
	type Image = crate::dx12::factory::FactoryImage;
	type Sampler = crate::dx12::factory::FactorySampler;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Ok(Device::from_native_parts(
			self.device.clone(),
			self.settings,
			self.info_queue.clone(),
			self.debug_log_function,
			self.queues.clone(),
		))
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		_shader_source_type: Sources,
		_stage: ShaderTypes,
		_shader_resource_descriptors: impl IntoIterator<Item = ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()> {
		panic!(
			"DX12 detached shader creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		panic!(
			"DX12 detached raster pipeline creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn create_compute_pipeline(&mut self, _builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		panic!(
			"DX12 detached compute pipeline creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn build_image(&mut self, _builder: crate::image::Builder) -> Self::Image {
		panic!(
			"DX12 detached image creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn build_sampler(&mut self, _builder: crate::sampler::Builder) -> Self::Sampler {
		panic!(
			"DX12 detached sampler creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}
}

impl crate::context::ContextCreate for Device {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: Uses,
		resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle {
		Device::create_allocation(self, size, resource_uses, resource_device_accesses)
	}
	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle {
		Device::add_mesh_from_vertices_and_indices(self, vertex_count, index_count, vertices, indices, vertex_layout)
	}
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()> {
		Device::create_shader(self, name, shader_source_type, stage, shader_resource_descriptors)
	}
	fn create_descriptor_set(&mut self, name: Option<&str>) -> DescriptorSetHandle {
		Device::create_descriptor_set(self, name)
	}
	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> PipelineHandle {
		Device::create_raster_pipeline(self, builder)
	}
	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> PipelineHandle {
		Device::create_compute_pipeline(self, builder)
	}
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> PipelineHandle {
		Device::create_ray_tracing_pipeline(self, builder)
	}
	fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		Device::build_buffer(self, builder)
	}
	fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T> {
		Device::build_dynamic_buffer(self, builder)
	}
	fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		Device::build_dynamic_image(self, builder)
	}
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		Device::build_image(self, builder)
	}
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
		Device::build_sampler(self, builder)
	}
	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle {
		Device::create_acceleration_structure_instance_buffer(self, name, max_instance_count)
	}
	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		Device::create_top_level_acceleration_structure(self, name, max_instance_count)
	}
	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		Device::create_bottom_level_acceleration_structure(self, description)
	}
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		Device::create_synchronizer(self, name, signaled)
	}
}

impl crate::context::Context for Device {
	type Queue = super::queue::Queue;
	type QueueReference<'a> = super::queue::QueueReference<'a>;
	type CommandBuffer<'a> = CommandBufferReference<'a>;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn supports_bc_texture_compression(&self) -> bool {
		true
	}

	fn queue(&mut self, queue_handle: QueueHandle) -> Self::Queue {
		super::queue::Queue {
			device: std::ptr::NonNull::from(self),
			queue_handle,
		}
	}

	fn queue_reference<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::QueueReference<'a> {
		super::queue::QueueReference {
			device: self,
			queue_handle,
		}
	}

	fn command_buffer<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> Self::CommandBuffer<'a> {
		CommandBufferReference {
			device: self,
			command_buffer_handle,
		}
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		Device::set_frames_in_flight(self, frames);
	}

	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64 {
		Device::get_buffer_address(self, buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		Device::get_buffer_slice(self, buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T {
		unsafe { std::mem::transmute::<&mut T, &'static mut T>(Device::get_mut_buffer_slice(self, buffer_handle)) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		Device::sync_buffer(self, buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: ImageHandle) -> &'static mut [u8] {
		self.texture_slice_mut_static(texture_handle.0)
	}

	fn sync_texture(&mut self, image_handle: ImageHandle) {
		self.queue_texture_sync_for_sequence(image_handle.0, 0);
	}

	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		Device::write_texture(self, texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		Device::write(self, descriptor_set_writes);
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
		Device::write_instance(
			self,
			instances_buffer_handle,
			instance_index,
			transform,
			custom_index,
			mask,
			sbt_record_offset,
			acceleration_structure,
		);
	}

	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	) {
		Device::write_sbt_entry(self, sbt_buffer_handle, sbt_record_offset, pipeline_handle, shader_handle);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		Device::bind_to_window(self, window_os_handles, presentation_mode, fallback_extent, _uses)
	}

	fn get_image_data<'a>(&'a mut self, texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
		self.wait_for_texture_copy_readback(texture_copy_handle);
		self.refresh_readback_texture_copies(None);
		Device::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		Device::resize_buffer(self, buffer_handle, size);
	}

	fn start_frame_capture(&mut self) {
		Device::start_frame_capture(self);
	}

	fn end_frame_capture(&mut self) {
		Device::end_frame_capture(self);
	}

	fn wait(&self) {
		Device::wait(self);
	}
}

use std::{
	alloc::{self, Layout},
	cell::Cell,
	sync::atomic::{AtomicU64, Ordering},
};

use ::utils::hash::{HashMap, HashSet};
use ::utils::Extent;
use smallvec::SmallVec;
use windows::core::{BOOL, PCSTR, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Dxc::{
	CLSID_DxcCompiler, DxcBuffer, DxcCreateInstance, IDxcBlob, IDxcCompiler3, IDxcIncludeHandler, IDxcResult, DXC_CP_UTF8,
	DXC_OUT_ERRORS, DXC_OUT_OBJECT, DXC_OUT_PDB,
};
use windows::Win32::Graphics::Direct3D::{
	Fxc::D3DCompile, ID3DInclude, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
	D3D_SHADER_MACRO,
};
use windows::Win32::Graphics::Direct3D12::{
	D3D12CreateDevice, D3D12SerializeRootSignature, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
	ID3D12CommandSignature, ID3D12DescriptorHeap, ID3D12Device, ID3D12Device2, ID3D12Device5, ID3D12Fence,
	ID3D12GraphicsCommandList, ID3D12GraphicsCommandList4, ID3D12GraphicsCommandList6, ID3D12InfoQueue, ID3D12PipelineState,
	ID3D12Resource, ID3D12RootSignature, ID3D12StateObject, ID3D12StateObjectProperties, D3D12_BLEND_DESC,
	D3D12_BLEND_INV_SRC_ALPHA, D3D12_BLEND_ONE, D3D12_BLEND_OP_ADD, D3D12_BLEND_SRC_ALPHA, D3D12_BLEND_ZERO, D3D12_BUFFER_SRV,
	D3D12_BUFFER_SRV_FLAG_NONE, D3D12_BUFFER_UAV, D3D12_BUFFER_UAV_FLAG_NONE, D3D12_BUFFER_UAV_FLAG_RAW,
	D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC, D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
	D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0, D3D12_CACHED_PIPELINE_STATE, D3D12_CLEAR_FLAG_DEPTH,
	D3D12_CLEAR_VALUE, D3D12_CLEAR_VALUE_0, D3D12_COLOR_WRITE_ENABLE_ALL, D3D12_COMMAND_LIST_TYPE, D3D12_COMMAND_QUEUE_DESC,
	D3D12_COMMAND_QUEUE_FLAGS, D3D12_COMMAND_SIGNATURE_DESC, D3D12_COMPARISON_FUNC_ALWAYS, D3D12_COMPARISON_FUNC_GREATER_EQUAL,
	D3D12_COMPARISON_FUNC_NEVER, D3D12_COMPUTE_PIPELINE_STATE_DESC, D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
	D3D12_CONSTANT_BUFFER_VIEW_DESC, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_BACK,
	D3D12_CULL_MODE_FRONT, D3D12_CULL_MODE_NONE, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, D3D12_DEPTH_STENCILOP_DESC,
	D3D12_DEPTH_STENCIL_DESC, D3D12_DEPTH_STENCIL_VALUE, D3D12_DEPTH_STENCIL_VIEW_DESC, D3D12_DEPTH_STENCIL_VIEW_DESC_0,
	D3D12_DEPTH_WRITE_MASK_ALL, D3D12_DEPTH_WRITE_MASK_ZERO, D3D12_DESCRIPTOR_HEAP_DESC,
	D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
	D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE, D3D12_DESCRIPTOR_RANGE_TYPE,
	D3D12_DESCRIPTOR_RANGE_TYPE_CBV, D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
	D3D12_DESCRIPTOR_RANGE_TYPE_UAV, D3D12_DISPATCH_RAYS_DESC, D3D12_DSV_DIMENSION_TEXTURE2D,
	D3D12_DSV_DIMENSION_TEXTURE2DARRAY, D3D12_DSV_FLAG_NONE, D3D12_DXIL_LIBRARY_DESC, D3D12_ELEMENTS_LAYOUT_ARRAY,
	D3D12_EXPORT_DESC, D3D12_EXPORT_FLAG_NONE, D3D12_FEATURE_D3D12_OPTIONS4, D3D12_FEATURE_D3D12_OPTIONS5,
	D3D12_FEATURE_D3D12_OPTIONS7, D3D12_FEATURE_DATA_D3D12_OPTIONS4, D3D12_FEATURE_DATA_D3D12_OPTIONS5,
	D3D12_FEATURE_DATA_D3D12_OPTIONS7, D3D12_FENCE_FLAGS, D3D12_FILL_MODE_SOLID, D3D12_FILTER, D3D12_FILTER_ANISOTROPIC,
	D3D12_FILTER_MAXIMUM_ANISOTROPIC, D3D12_FILTER_MINIMUM_ANISOTROPIC, D3D12_FILTER_MIN_MAG_MIP_LINEAR,
	D3D12_GLOBAL_ROOT_SIGNATURE, D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE,
	D3D12_GPU_VIRTUAL_ADDRESS_RANGE, D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE, D3D12_GRAPHICS_PIPELINE_STATE_DESC,
	D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_HEAP_TYPE_READBACK, D3D12_HEAP_TYPE_UPLOAD,
	D3D12_HIT_GROUP_DESC, D3D12_HIT_GROUP_TYPE_PROCEDURAL_PRIMITIVE, D3D12_HIT_GROUP_TYPE_TRIANGLES,
	D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED, D3D12_INDEX_BUFFER_VIEW, D3D12_INDIRECT_ARGUMENT_DESC,
	D3D12_INDIRECT_ARGUMENT_DESC_0, D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH, D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
	D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC, D3D12_LOGIC_OP_NOOP, D3D12_MEMORY_POOL_UNKNOWN,
	D3D12_MESH_SHADER_TIER_NOT_SUPPORTED, D3D12_MESSAGE, D3D12_MESSAGE_SEVERITY_CORRUPTION, D3D12_MESSAGE_SEVERITY_ERROR,
	D3D12_PIPELINE_STATE_FLAGS, D3D12_PIPELINE_STATE_FLAG_NONE, D3D12_PIPELINE_STATE_STREAM_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK, D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
	D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RANGE, D3D12_RASTERIZER_DESC,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
	D3D12_RAYTRACING_GEOMETRY_AABBS_DESC, D3D12_RAYTRACING_GEOMETRY_DESC, D3D12_RAYTRACING_GEOMETRY_DESC_0,
	D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE, D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC,
	D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS, D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
	D3D12_RAYTRACING_INSTANCE_DESC, D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE, D3D12_RAYTRACING_PIPELINE_CONFIG,
	D3D12_RAYTRACING_SHADER_CONFIG, D3D12_RAYTRACING_TIER_NOT_SUPPORTED, D3D12_RENDER_TARGET_BLEND_DESC,
	D3D12_RENDER_TARGET_VIEW_DESC, D3D12_RENDER_TARGET_VIEW_DESC_0, D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
	D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
	D3D12_RESOURCE_BARRIER_TYPE_UAV, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
	D3D12_RESOURCE_FLAGS, D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
	D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_COMMON,
	D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_DEPTH_WRITE,
	D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATE_INDEX_BUFFER, D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
	D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PRESENT,
	D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_RESOURCE_STATE_RENDER_TARGET,
	D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER, D3D12_RESOURCE_TRANSITION_BARRIER,
	D3D12_RESOURCE_UAV_BARRIER, D3D12_ROOT_CONSTANTS, D3D12_ROOT_DESCRIPTOR_TABLE, D3D12_ROOT_PARAMETER,
	D3D12_ROOT_PARAMETER_0, D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS, D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
	D3D12_ROOT_SIGNATURE_DESC, D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT, D3D12_RTV_DIMENSION_TEXTURE2D,
	D3D12_RTV_DIMENSION_TEXTURE2DARRAY, D3D12_RT_FORMAT_ARRAY, D3D12_SAMPLER_DESC, D3D12_SHADER_BYTECODE,
	D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES, D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0,
	D3D12_SHADER_VISIBILITY_ALL, D3D12_SRV_DIMENSION_BUFFER, D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
	D3D12_SRV_DIMENSION_TEXTURE2D, D3D12_SRV_DIMENSION_TEXTURE2DARRAY, D3D12_SRV_DIMENSION_TEXTURE3D, D3D12_STATE_OBJECT_DESC,
	D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE, D3D12_STATE_SUBOBJECT, D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
	D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE, D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP,
	D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG, D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
	D3D12_STATE_SUBOBJECT_TYPE_SUBOBJECT_TO_EXPORTS_ASSOCIATION, D3D12_STENCIL_OP_KEEP, D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION,
	D3D12_SUBRESOURCE_FOOTPRINT, D3D12_TEX2D_ARRAY_DSV, D3D12_TEX2D_ARRAY_RTV, D3D12_TEX2D_ARRAY_SRV, D3D12_TEX2D_ARRAY_UAV,
	D3D12_TEX2D_DSV, D3D12_TEX2D_RTV, D3D12_TEX2D_SRV, D3D12_TEX2D_UAV, D3D12_TEX3D_SRV, D3D12_TEX3D_UAV,
	D3D12_TEXTURE_ADDRESS_MODE, D3D12_TEXTURE_ADDRESS_MODE_BORDER, D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
	D3D12_TEXTURE_ADDRESS_MODE_MIRROR, D3D12_TEXTURE_ADDRESS_MODE_WRAP, D3D12_TEXTURE_COPY_LOCATION,
	D3D12_TEXTURE_COPY_LOCATION_0, D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
	D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_UAV_DIMENSION_BUFFER, D3D12_UAV_DIMENSION_TEXTURE2D,
	D3D12_UAV_DIMENSION_TEXTURE2DARRAY, D3D12_UAV_DIMENSION_TEXTURE3D, D3D12_UNORDERED_ACCESS_VIEW_DESC,
	D3D12_UNORDERED_ACCESS_VIEW_DESC_0, D3D12_VERTEX_BUFFER_VIEW, D3D12_VIEWPORT, D3D_ROOT_SIGNATURE_VERSION_1_0,
};
use windows::Win32::Graphics::Dxgi::Common::{
	DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_BC5_SNORM, DXGI_FORMAT_BC5_UNORM,
	DXGI_FORMAT_BC7_UNORM, DXGI_FORMAT_BC7_UNORM_SRGB, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R16G16B16A16_FLOAT,
	DXGI_FORMAT_R16G16B16A16_SNORM, DXGI_FORMAT_R16G16B16A16_UNORM, DXGI_FORMAT_R16G16_FLOAT, DXGI_FORMAT_R16G16_SNORM,
	DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16_FLOAT, DXGI_FORMAT_R16_SNORM, DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R16_UNORM,
	DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32A32_SINT, DXGI_FORMAT_R32G32B32A32_UINT, DXGI_FORMAT_R32G32B32_FLOAT,
	DXGI_FORMAT_R32G32B32_SINT, DXGI_FORMAT_R32G32B32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32_SINT,
	DXGI_FORMAT_R32G32_UINT, DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32_SINT, DXGI_FORMAT_R32_TYPELESS, DXGI_FORMAT_R32_UINT,
	DXGI_FORMAT_R8G8B8A8_SNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R8G8_SNORM,
	DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_SNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
	CreateDXGIFactory2, IDXGIFactory4, IDXGISwapChain3, DXGI_CREATE_FACTORY_FLAGS, DXGI_MWA_NO_ALT_ENTER, DXGI_SCALING_STRETCH,
	DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::{
	core::{IUnknown, Interface},
	Win32::Graphics::{
		Direct3D12::{D3D12_COMMAND_LIST_TYPE_COMPUTE, D3D12_COMMAND_LIST_TYPE_COPY, D3D12_COMMAND_LIST_TYPE_DIRECT},
		Dxgi::{DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG},
	},
};

use super::utils;
use crate::WorkloadTypes;
use crate::{
	buffer,
	descriptors::{DescriptorWrite, WriteData},
	device::Features,
	image,
	pipelines::{self, PushConstantRange, VertexElement},
	render_debugger::RenderDebugger,
	sampler,
	shader::{ResourceKind, ResourceSlot, ShaderResourceDescriptor, Sources},
	window, AllocationHandle, AttachmentInformation, BaseBufferHandle, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferHandle, BufferStridedRange, ClearValue,
	CommandBufferHandle, DataTypes, DescriptorSetHandle, DeviceAccesses, DispatchExtent, DynamicBufferHandle, FilteringModes,
	Formats, HandleLike as _, ImageHandle, ImageOrSwapchain, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey,
	PresentationModes, QueueHandle, QueueSelection, RGBAu8, SamplerAddressingModes, SamplerHandle, SamplingReductionModes,
	ShaderHandle, ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TextureViewTypes,
	TopLevelAccelerationStructureHandle, UseCases, Uses,
};
