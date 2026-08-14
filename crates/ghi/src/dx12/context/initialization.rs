//! DX12 device operations for initialization.

use super::*;

impl Device {
	const NATIVE_16_BIT_SHADER_OPS_UNAVAILABLE: &str = "DX12 native 16-bit shader types are unavailable. The most likely cause is a GPU or driver that does not report Native16BitShaderOpsSupported.";
	const WAVE_OPS_UNAVAILABLE: &str = "DX12 wave operations are unavailable. The most likely cause is that the selected GPU or driver does not report D3D12 WaveOps support required by Material Count.";

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
		if !Self::query_wave_ops_support(&device) {
			return Err(Self::WAVE_OPS_UNAVAILABLE);
		}
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
	pub(crate) fn from_native_parts(
		device: ID3D12Device,
		settings: Features,
		info_queue: Option<ID3D12InfoQueue>,
		debug_log_function: fn(&str),
		queues: Vec<StoredQueue>,
	) -> Self {
		let native_16_bit_shader_ops_supported = Self::query_native_16_bit_shader_ops_support(&device);
		let descriptor_handle_increment_sizes = [
			unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) },
			unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER) },
			unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) },
			unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_DSV) },
		];
		Self {
			device,
			descriptor_handle_increment_sizes,
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
			retained_clear_uav_descriptors: HashMap::default(),
			clear_uav_descriptor_pages: Vec::new(),
			free_clear_uav_descriptor_slots: Vec::new(),
			buffer_states: HashMap::default(),
			image_states: HashMap::default(),
			render_target_view_allocation_count: 0,
			depth_stencil_view_allocation_count: 0,
			texture_copy_count: 0,
			buffer_copy_count: 0,
			buffer_clear_count: 0,
			clear_descriptor_copy_call_count: 0,
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

	pub(crate) fn log_debug_message(&self, message: impl AsRef<str>) {
		(self.debug_log_function)(message.as_ref());
	}

	pub(crate) fn log_dx12_error(&self, message: impl AsRef<str>) {
		self.log_debug_message(message);
		self.debug_log_count.fetch_add(10, Ordering::Relaxed);
		self.drain_debug_messages();
	}

	pub(crate) fn drain_debug_messages(&self) {
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
		self.invalidate_clear_uav_descriptors_for_resources(&retired_image_state_keys);
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
		self.invalidate_clear_uav_descriptors_for_resources(&retired_buffer_state_keys);
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

	pub(crate) fn compile_hlsl(
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
	pub(crate) fn dxc_target(stage: ShaderTypes, native_16_bit_types: bool) -> Option<&'static str> {
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
	pub(crate) fn hlsl_uses_native_16_bit_types(source: &str) -> bool {
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

	pub(crate) fn compile_hlsl_with_dxc(
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
		// Every declared slot is materialized before binding, so DXC can optimize under the fully-bound resource contract.
		argument_storage.push(Self::wide_argument("-all_resources_bound"));
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

	pub(crate) fn hlsl_debug_artifacts_enabled(&self) -> bool {
		// Shader PDBs are valuable when the DX12 debug layer is active, but they make normal startup pay filesystem and
		// embedded-debug compilation costs for every generated shader.
		self.settings.validation || self.settings.gpu_validation
	}

	pub(crate) fn hlsl_dxil_cache_path(
		source: &str,
		entry_point: &str,
		target: &str,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Option<std::path::PathBuf> {
		// Version 4 uses DXC's official IDxcCompiler argument for the fully-bound resource contract.
		let mut hash = Self::fnv64(b"byte-engine-dx12-dxil-cache-v4");
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

	pub(crate) fn write_hlsl_dxil_cache(path: &std::path::Path, bytecode: &[u8]) {
		let Some(directory) = path.parent() else {
			return;
		};
		if std::fs::create_dir_all(directory).is_err() {
			return;
		}
		// Best-effort cache writes keep shader compilation correctness independent of filesystem availability.
		let _ = std::fs::write(path, bytecode);
	}

	pub(crate) fn fnv64(bytes: &[u8]) -> u64 {
		let mut hash = 0xcbf29ce484222325;
		Self::fnv64_update(&mut hash, bytes);
		hash
	}

	pub(crate) fn fnv64_update_text(hash: &mut u64, text: &str) {
		Self::fnv64_update(hash, &(text.len() as u64).to_le_bytes());
		Self::fnv64_update(hash, text.as_bytes());
	}

	pub(crate) fn fnv64_update(hash: &mut u64, bytes: &[u8]) {
		for byte in bytes {
			*hash ^= u64::from(*byte);
			*hash = hash.wrapping_mul(0x100000001b3);
		}
	}

	pub(crate) fn write_shader_debug_files(
		&self,
		name: Option<&str>,
		entry_point: &str,
		target: &str,
		source: &str,
		result: &IDxcResult,
	) {
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

	pub(crate) fn shader_debug_hlsl_path(name: Option<&str>, entry_point: &str, target: &str) -> Option<std::path::PathBuf> {
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

	pub(crate) fn sanitize_shader_debug_name(name: &str) -> String {
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

	pub(crate) fn dxc_error_output(result: &IDxcResult) -> String {
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

	pub(crate) fn log_hlsl_compile_error(&self, source: &str, entry_point: &str, target: &str, reason: &str) {
		self.log_dx12_error(format!(
			"Failed to compile DX12 HLSL shader. Entry point: {entry_point}. Target: {target}. Reason: {reason}\n--- HLSL source ---\n{source}\n--- End HLSL source ---"
		));
	}

	pub(crate) fn wide_argument(argument: &str) -> Vec<u16> {
		argument.encode_utf16().chain(std::iter::once(0)).collect()
	}

	pub(crate) fn hlsl_specialization_macro_storage(
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

	pub(crate) fn push_hlsl_bool_specialization_macro(
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

	pub(crate) fn push_hlsl_i32_specialization_macro(
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

	pub(crate) fn push_hlsl_u32_specialization_macro(
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

	pub(crate) fn push_hlsl_f32_specialization_macro(
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

	pub(crate) fn push_hlsl_specialization_macro_text(
		names: &mut Vec<std::ffi::CString>,
		values: &mut Vec<std::ffi::CString>,
		constant_id: u32,
		value: &str,
	) -> Result<(), ()> {
		names.push(std::ffi::CString::new(format!("SPEC_CONSTANT_{constant_id}")).map_err(|_| ())?);
		values.push(std::ffi::CString::new(value).map_err(|_| ())?);
		Ok(())
	}

	pub(crate) fn push_hlsl_specialization_macro_vector(
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
}
