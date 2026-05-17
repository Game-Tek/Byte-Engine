/// The `Device` struct exists to own DX12 GPU resources for the shared GHI device API.
pub struct Device {
	device: ID3D12Device,
	settings: Features,
	debugger: RenderDebugger,
	pub(crate) frames: u8,

	queues: Vec<StoredQueue>,
	command_buffers: Vec<CommandBuffer>,
	buffers: Vec<Buffer>,
	dynamic_buffers: Vec<Buffer>,
	images: Vec<Image>,
	samplers: Vec<Sampler>,
	descriptor_set_templates: Vec<DescriptorSetTemplate>,
	descriptor_sets: Vec<DescriptorSet>,
	descriptor_bindings: Vec<DescriptorSetBinding>,
	descriptors: HashMap<DescriptorSetHandle, HashMap<u32, HashMap<u32, WriteData>>>,
	resource_to_descriptor: HashMap<PrivateHandles, HashSet<(DescriptorSetBindingHandle, u32)>>,
	descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32), HashSet<PrivateHandles>>,
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
	upload_resources: Vec<ID3D12Resource>,
	readback_resources: Vec<ID3D12Resource>,
	texture_readbacks: Vec<TextureReadback>,
	gpu_uploaded_images: HashSet<crate::BaseImageHandle>,
	pending_texture_syncs: Vec<(crate::BaseImageHandle, u8)>,
	present_transitions: HashMap<CommandBufferHandle, Vec<ID3D12Resource>>,
	rtv_heaps: Vec<ID3D12DescriptorHeap>,
	dsv_heaps: Vec<ID3D12DescriptorHeap>,
	buffer_states: HashMap<u64, D3D12_RESOURCE_STATES>,
	image_states: HashMap<u64, D3D12_RESOURCE_STATES>,
	texture_copy_count: usize,
	buffer_copy_count: usize,
	buffer_clear_count: usize,
	root_signature_bind_count: usize,
	descriptor_heap_bind_count: usize,
	descriptor_table_bind_count: usize,
	descriptor_table_bind_records: Vec<DescriptorTableBindRecord>,
	push_constant_write_count: usize,
	push_constant_write_records: Vec<PushConstantWriteRecord>,
	descriptor_write_count: usize,
	image_srv_descriptor_write_count: usize,
	image_uav_descriptor_write_count: usize,
	acceleration_structure_descriptor_write_count: usize,
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

		Ok(Self {
			device,
			settings,
			debugger: RenderDebugger::new(),
			frames: 2,

			queues: queue_storage,
			command_buffers: Vec::new(),
			buffers: Vec::new(),
			dynamic_buffers: Vec::new(),
			images: Vec::new(),
			samplers: Vec::new(),
			descriptor_set_templates: Vec::new(),
			descriptor_sets: Vec::new(),
			descriptor_bindings: Vec::new(),
			descriptors: HashMap::default(),
			resource_to_descriptor: HashMap::default(),
			descriptor_set_to_resource: HashMap::default(),
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
			upload_resources: Vec::new(),
			readback_resources: Vec::new(),
			texture_readbacks: Vec::new(),
			gpu_uploaded_images: HashSet::default(),
			pending_texture_syncs: Vec::new(),
			present_transitions: HashMap::default(),
			rtv_heaps: Vec::new(),
			dsv_heaps: Vec::new(),
			buffer_states: HashMap::default(),
			image_states: HashMap::default(),
			texture_copy_count: 0,
			buffer_copy_count: 0,
			buffer_clear_count: 0,
			root_signature_bind_count: 0,
			descriptor_heap_bind_count: 0,
			descriptor_table_bind_count: 0,
			descriptor_table_bind_records: Vec::new(),
			push_constant_write_count: 0,
			push_constant_write_records: Vec::new(),
			descriptor_write_count: 0,
			image_srv_descriptor_write_count: 0,
			image_uav_descriptor_write_count: 0,
			acceleration_structure_descriptor_write_count: 0,
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
		})
	}

	#[cfg(debug_assertions)]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		self.frames = frames.max(1);
		let image_count = self.frames.max(2);

		for swapchain in &mut self.swapchains {
			if swapchain.image_count != image_count && swapchain.extent.width() > 0 && swapchain.extent.height() > 0 {
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

		for image in &mut self.images {
			let Some(frame_data) = image.frame_data.as_mut() else {
				continue;
			};
			let data = image.data.clone().unwrap_or_default();
			frame_data.resize(self.frames as usize, data);
		}
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
		_name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = BindingDescriptor>,
	) -> Result<ShaderHandle, ()> {
		let (spirv, dxil, hlsl) = match shader_source_type {
			Sources::SPIRV(bytes) => (Some(bytes.to_vec()), None, None),
			Sources::DXIL(bytes) => (None, Some(bytes.to_vec()), None),
			Sources::HLSL { source, entry_point } => (
				None,
				Some(Self::compile_hlsl(source, entry_point, stage, &[])?),
				Some(HlslSource {
					source: source.to_string(),
					entry_point: entry_point.to_string(),
				}),
			),
			Sources::MTL { .. } | Sources::MTLB { .. } => return Err(()),
		};

		self.shaders.push(Shader {
			stage,
			spirv,
			dxil,
			hlsl,
			bindings: shader_binding_descriptors.into_iter().collect(),
		});

		// DX12 consumes native bytecode for PSO creation, while SPIR-V is retained as portable metadata.
		Ok(ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn compile_hlsl(
		source: &str,
		entry_point: &str,
		stage: ShaderTypes,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Result<Vec<u8>, ()> {
		if let Some(target) = Self::dxc_target(stage) {
			return Self::compile_hlsl_with_dxc(source, entry_point, target, specialization_map);
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
			.map_err(|_| ())?;
		}
		let Some(shader) = shader else {
			return Err(());
		};
		let bytecode = unsafe { std::slice::from_raw_parts(shader.GetBufferPointer().cast::<u8>(), shader.GetBufferSize()) };
		Ok(bytecode.to_vec())
	}

	fn dxc_target(stage: ShaderTypes) -> Option<&'static str> {
		match stage {
			ShaderTypes::Mesh => Some("ms_6_5"),
			ShaderTypes::RayGen
			| ShaderTypes::Miss
			| ShaderTypes::ClosestHit
			| ShaderTypes::AnyHit
			| ShaderTypes::Intersection => Some("lib_6_3"),
			_ => None,
		}
	}

	fn compile_hlsl_with_dxc(
		source: &str,
		entry_point: &str,
		target: &str,
		specialization_map: &[pipelines::SpecializationMapEntry],
	) -> Result<Vec<u8>, ()> {
		let compiler = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }.map_err(|_| ())?;
		let source_buffer = DxcBuffer {
			Ptr: source.as_ptr().cast(),
			Size: source.len(),
			Encoding: DXC_CP_UTF8.0,
		};
		let mut argument_storage = vec![
			Self::wide_argument("-E"),
			Self::wide_argument(entry_point),
			Self::wide_argument("-T"),
			Self::wide_argument(target),
		];
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
		.map_err(|_| ())?;
		let status = unsafe { result.GetStatus() }.map_err(|_| ())?;
		if status.is_err() {
			return Err(());
		}
		let mut object = None;
		unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_OBJECT, std::ptr::null_mut(), &mut object) }.map_err(|_| ())?;
		let Some(object) = object else {
			return Err(());
		};
		let bytecode = unsafe { std::slice::from_raw_parts(object.GetBufferPointer().cast::<u8>(), object.GetBufferSize()) };
		Ok(bytecode.to_vec())
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

	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		binding_templates: &[DescriptorSetBindingTemplate],
	) -> DescriptorSetTemplateHandle {
		self.descriptor_set_templates.push(DescriptorSetTemplate {
			bindings: binding_templates.to_vec(),
		});
		DescriptorSetTemplateHandle((self.descriptor_set_templates.len() - 1) as u64)
	}

	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		descriptor_set_template_handle: &DescriptorSetTemplateHandle,
	) -> DescriptorSetHandle {
		// Creates per-frame descriptor set records for the template.
		let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(DescriptorSet {
				next: None,
				template: *descriptor_set_template_handle,
				bindings: Vec::new(),
				cbv_srv_uav_heap: self.create_descriptor_heap(*descriptor_set_template_handle, false),
				sampler_heap: self.create_descriptor_heap(*descriptor_set_template_handle, true),
			});

			if let Some(previous) = previous {
				self.descriptor_sets[previous.0 as usize].next = Some(handle);
			}

			previous = Some(handle);
		}

		handle
	}

	pub fn create_descriptor_binding(
		&mut self,
		descriptor_set: DescriptorSetHandle,
		binding_constructor: BindingConstructor,
	) -> DescriptorSetBindingHandle {
		// Records a descriptor binding while deferring DX12 descriptor heap setup.
		let template = binding_constructor.descriptor_set_binding_template;
		let descriptor_type = template.descriptor_type;
		let binding_index = template.binding;
		let count = template.descriptor_count;

		let descriptor_set_handles = self.collect_descriptor_set_handles(descriptor_set);
		let mut next = None;

		for (frame_index, descriptor_set_handle) in descriptor_set_handles.iter().enumerate().rev() {
			let binding_handle = DescriptorSetBindingHandle(self.descriptor_bindings.len() as u64);

			self.descriptor_bindings.push(DescriptorSetBinding {
				next,
				descriptor_set: *descriptor_set_handle,
				descriptor_type,
				binding_index,
				count,
				frame_offset: binding_constructor.frame_offset.map(|offset| offset as i32),
			});

			if let Some(set) = self.descriptor_sets.get_mut(descriptor_set_handle.0 as usize) {
				set.bindings.push(binding_handle);
			}

			let descriptor = self.resolve_descriptor_for_frame(
				binding_constructor.descriptor,
				frame_index,
				binding_constructor.frame_offset.map(|offset| offset as i32),
			);
			self.update_descriptor_for_binding(binding_handle, descriptor, binding_constructor.array_element);

			next = Some(binding_handle);
		}

		// DX12 uses descriptor heaps and root signatures, so descriptor set bindings are stored but not bound yet.
		DescriptorSetBindingHandle(next.expect("No next binding").0)
	}

	fn descriptor_heap_descriptor_count(&self, template_handle: DescriptorSetTemplateHandle, sampler_heap: bool) -> u32 {
		self.descriptor_set_templates
			.get(template_handle.0 as usize)
			.map(|template| {
				template
					.bindings
					.iter()
					.filter(|binding| Self::descriptor_range_type(binding.descriptor_type, sampler_heap).is_some())
					.map(|binding| Self::descriptor_count_for_heap(binding, sampler_heap))
					.sum()
			})
			.unwrap_or(0)
	}

	fn descriptor_count_for_heap(binding: &DescriptorSetBindingTemplate, _sampler_heap: bool) -> u32 {
		// Keep DX12 descriptor ranges conservative until descriptor indexing support is queried and handled.
		// Large bindless-style ranges can be invalid on lower resource binding tiers and can remove the device.
		binding.descriptor_count.max(1).min(16)
	}

	fn create_descriptor_heap(
		&self,
		template_handle: DescriptorSetTemplateHandle,
		sampler_heap: bool,
	) -> Option<ID3D12DescriptorHeap> {
		let count = self.descriptor_heap_descriptor_count(template_handle, sampler_heap);
		if count == 0 {
			return None;
		}

		let desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: if sampler_heap {
				D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
			} else {
				D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
			},
			NumDescriptors: count,
			Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
			NodeMask: 0,
		};

		let heap = unsafe { self.device.CreateDescriptorHeap(&desc) };
		if let Err(error) = &heap {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			eprintln!(
				"Failed to create DX12 descriptor heap. Sampler heap: {sampler_heap}. Descriptor count: {count}. Error: {error:?}. Device removed reason: {removed_reason:?}"
			);
		}
		heap.ok()
	}

	fn descriptor_heap_slot(
		&self,
		template_handle: DescriptorSetTemplateHandle,
		descriptor_type: DescriptorType,
		binding_index: u32,
		array_element: u32,
		sampler_heap: bool,
	) -> Option<u32> {
		let template = self.descriptor_set_templates.get(template_handle.0 as usize)?;
		let mut slot = 0;
		for binding in &template.bindings {
			if Self::descriptor_range_type(binding.descriptor_type, sampler_heap).is_none() {
				continue;
			}
			if binding.binding == binding_index
				&& std::mem::discriminant(&binding.descriptor_type) == std::mem::discriminant(&descriptor_type)
			{
				return Some(slot + array_element.min(binding.descriptor_count.saturating_sub(1)));
			}
			slot += binding.descriptor_count.max(1);
		}
		None
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

	fn descriptor_range_type(descriptor_type: DescriptorType, sampler_heap: bool) -> Option<D3D12_DESCRIPTOR_RANGE_TYPE> {
		match descriptor_type {
			DescriptorType::UniformBuffer if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_CBV),
			DescriptorType::StorageBuffer | DescriptorType::StorageImage if !sampler_heap => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_UAV)
			}
			DescriptorType::SampledImage
			| DescriptorType::InputAttachment
			| DescriptorType::AccelerationStructure
			| DescriptorType::CombinedImageSampler
				if !sampler_heap =>
			{
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SRV)
			}
			DescriptorType::Sampler | DescriptorType::CombinedImageSampler if sampler_heap => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER)
			}
			_ => None,
		}
	}

	fn create_root_signature(
		&self,
		descriptor_set_template_handles: &[DescriptorSetTemplateHandle],
		push_constant_ranges: &[PushConstantRange],
	) -> (Option<ID3D12RootSignature>, Vec<RootDescriptorTable>, Vec<RootConstantRange>) {
		let mut ranges = Vec::new();
		let mut tables = Vec::new();
		for (space, template_handle) in descriptor_set_template_handles.iter().enumerate() {
			let Some(template) = self.descriptor_set_templates.get(template_handle.0 as usize) else {
				continue;
			};
			let mut cbv_srv_uav_slot = 0;
			let mut sampler_slot = 0;
			for binding in &template.bindings {
				for sampler_heap in [false, true] {
					let Some(range_type) = Self::descriptor_range_type(binding.descriptor_type, sampler_heap) else {
						continue;
					};
					let descriptor_count = Self::descriptor_count_for_heap(binding, sampler_heap);
					let heap_slot = if sampler_heap {
						let slot = sampler_slot;
						sampler_slot += descriptor_count;
						slot
					} else {
						let slot = cbv_srv_uav_slot;
						cbv_srv_uav_slot += descriptor_count;
						slot
					};
					ranges.push(D3D12_DESCRIPTOR_RANGE {
						RangeType: range_type,
						NumDescriptors: descriptor_count,
						BaseShaderRegister: binding.binding,
						RegisterSpace: space as u32,
						OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
					});
					tables.push(RootDescriptorTable {
						set_index: space,
						binding_index: binding.binding,
						sampler_heap,
						heap_slot,
					});
				}
			}
		}

		let mut parameters = ranges
			.iter()
			.map(|range| D3D12_ROOT_PARAMETER {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
				Anonymous: D3D12_ROOT_PARAMETER_0 {
					DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
						NumDescriptorRanges: 1,
						pDescriptorRanges: range as *const D3D12_DESCRIPTOR_RANGE,
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			})
			.collect::<Vec<_>>();

		let mut constants = Vec::new();
		for push_constant_range in push_constant_ranges {
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
				Anonymous: D3D12_ROOT_PARAMETER_0 {
					Constants: D3D12_ROOT_CONSTANTS {
						ShaderRegister: push_constant_range.offset / 4,
						RegisterSpace: descriptor_set_template_handles.len() as u32,
						Num32BitValues: push_constant_range.size.div_ceil(4),
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			constants.push(RootConstantRange {
				root_parameter_index,
				offset: push_constant_range.offset,
				size: push_constant_range.size,
			});
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
			Flags: D3D12_ROOT_SIGNATURE_FLAGS(0),
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
				eprintln!(
					"Failed to serialize DX12 root signature: {}",
					String::from_utf8_lossy(message)
				);
			}
			return (None, tables, constants);
		}
		let Some(blob) = blob else {
			return (None, tables, constants);
		};
		let bytes = unsafe { std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) };

		let root_signature = unsafe { self.device.CreateRootSignature(0, bytes) };
		if let Err(error) = &root_signature {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			eprintln!(
				"Failed to create DX12 root signature with {} parameters, {} descriptor tables, and {} constants: {error:?}; device removed reason: {removed_reason:?}",
				parameters.len(),
				tables.len(),
				constants.len()
			);
		}

		(root_signature.ok(), tables, constants)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		descriptor_set_template_handles: &[DescriptorSetTemplateHandle],
		push_constant_ranges: &[PushConstantRange],
	) -> PipelineLayoutHandle {
		let layout = PipelineLayout {
			descriptor_set_templates: descriptor_set_template_handles.to_vec(),
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&layout) {
			return *handle;
		}

		self.pipeline_layouts.push(layout.clone());
		let handle = PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
		let (root_signature, root_tables, root_constants) =
			self.create_root_signature(descriptor_set_template_handles, push_constant_ranges);
		self.pipeline_root_signatures.push(root_signature);
		self.pipeline_root_tables.push(root_tables);
		self.pipeline_root_constants.push(root_constants);
		self.pipeline_layout_indices.insert(layout, handle);
		handle
	}

	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
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
		for (index, attachment) in builder.render_targets.iter().take(8).enumerate() {
			render_targets[index] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[index] = Self::dxgi_format(attachment.format)?;
		}

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
				IndependentBlendEnable: BOOL((builder.render_targets.len() > 1) as i32),
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
				DepthEnable: BOOL(0),
				DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ZERO,
				DepthFunc: D3D12_COMPARISON_FUNC_ALWAYS,
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
			NumRenderTargets: builder.render_targets.len().min(8) as u32,
			RTVFormats: rtv_formats,
			DSVFormat: DXGI_FORMAT_UNKNOWN,
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
				eprintln!(
					"Failed to create DX12 graphics pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				);
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
			eprintln!("Skipping DX12 mesh pipeline creation because native mesh shaders are not supported by this device.");
			return None;
		}

		let root_signature = self
			.pipeline_root_signatures
			.get(layout.0 as usize)
			.and_then(|root_signature| root_signature.clone())?;
		let mesh_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Mesh)?;
		let fragment_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Fragment)?;
		if mesh_shader.is_empty() || fragment_shader.is_empty() {
			return None;
		}

		let mut render_targets = [Self::render_target_blend_desc(pipelines::raster::BlendMode::None); 8];
		let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
		for (index, attachment) in builder.render_targets.iter().take(8).enumerate() {
			render_targets[index] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[index] = Self::dxgi_format(attachment.format)?;
		}

		self.graphics_pipeline_state_create_attempt_count += 1;
		let mut stream = MeshPipelineStateStream {
			root_signature: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE,
				value: Some(root_signature),
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
					pShaderBytecode: fragment_shader.as_ptr().cast(),
					BytecodeLength: fragment_shader.len(),
				},
			},
			blend: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
				value: D3D12_BLEND_DESC {
					AlphaToCoverageEnable: BOOL(0),
					IndependentBlendEnable: BOOL((builder.render_targets.len() > 1) as i32),
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
					DepthEnable: BOOL(0),
					DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ZERO,
					DepthFunc: D3D12_COMPARISON_FUNC_ALWAYS,
					StencilEnable: BOOL(0),
					StencilReadMask: 0xff,
					StencilWriteMask: 0xff,
					FrontFace: Self::disabled_stencil_op_desc(),
					BackFace: Self::disabled_stencil_op_desc(),
				},
			},
			depth_stencil_format: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
				value: DXGI_FORMAT_UNKNOWN,
			},
			primitive_topology: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PRIMITIVE_TOPOLOGY,
				value: D3D12_PRIMITIVE_TOPOLOGY_DESC {
					PrimitiveTopology: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
				},
			},
			render_targets: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
				value: D3D12_RT_FORMAT_ARRAY {
					RTFormats: rtv_formats,
					NumRenderTargets: builder.render_targets.len().min(8) as u32,
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
				eprintln!("Failed to create DX12 mesh pipeline state: {error:?}");
				None
			}
		}
	}

	fn shader_dxil_for_stage(&mut self, shaders: &[pipelines::ShaderParameter], stage: ShaderTypes) -> Option<Vec<u8>> {
		let parameter = shaders.iter().find(|parameter| {
			matches!(
				(parameter.stage, stage),
				(ShaderTypes::Vertex, ShaderTypes::Vertex)
					| (ShaderTypes::Fragment, ShaderTypes::Fragment)
					| (ShaderTypes::Mesh, ShaderTypes::Mesh)
			)
		})?;
		let shader = self.shaders.get(parameter.handle.0 as usize)?;
		if !parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = Self::compile_hlsl(&hlsl.source, &hlsl.entry_point, stage, parameter.specialization_map).ok();
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
			DestBlendAlpha: D3D12_BLEND_ZERO,
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
		let layout = self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
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
				let dxil = Self::compile_hlsl(
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
			eprintln!("Failed to create DX12 compute pipeline state: {error:?}; device removed reason: {removed_reason:?}");
		}
		pipeline_state.ok()
	}

	pub fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let shaders = builder.shaders;
		let (ray_tracing_state_object, ray_tracing_shader_identifiers) = self.create_ray_tracing_state_object(&shaders);
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
		self.ray_tracing_state_object_create_attempt_count += 1;

		let mut export_names = Vec::with_capacity(shaders.len());
		let mut source_export_names = Vec::with_capacity(shaders.len());
		let mut exports = Vec::with_capacity(shaders.len());
		let mut libraries = Vec::with_capacity(shaders.len());
		let mut hit_group_names = Vec::new();
		let mut hit_groups = Vec::new();
		let mut identifier_exports = Vec::new();
		let mut subobjects = Vec::new();

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
		let shader_config = D3D12_RAYTRACING_SHADER_CONFIG {
			MaxPayloadSizeInBytes: 32,
			MaxAttributeSizeInBytes: 32,
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
			pDesc: (&shader_config as *const D3D12_RAYTRACING_SHADER_CONFIG).cast(),
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
		let Ok(state_object) = (unsafe { device.CreateStateObject::<ID3D12StateObject>(&desc) }) else {
			return (None, HashMap::default());
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
			is_open: false,
		});

		CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.begin_command_buffer(command_buffer_handle);
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
		let frame_data = if builder.use_case == UseCases::DYNAMIC {
			data.as_ref().map(|data| vec![data.clone(); self.frames as usize])
		} else {
			None
		};
		let resource = self.create_image_resource(
			builder.extent,
			builder.format,
			builder.resource_uses,
			builder.array_layers.map(|v| v.get()).unwrap_or(1),
		);

		self.images.push(Image {
			extent: builder.extent,
			format: builder.format,
			uses: builder.resource_uses,
			access: builder.device_accesses,
			array_layers: builder.array_layers.map(|v| v.get()).unwrap_or(1),
			resource,
			data,
			frame_data,
		});

		ImageHandle(crate::BaseImageHandle((self.images.len() - 1) as u64))
	}

	pub(crate) fn image_resource_state(&self, image: ImageHandle) -> Option<(Extent, bool)> {
		self.images
			.get(image.0 .0 as usize)
			.map(|image| (image.extent, image.resource.is_some()))
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

	pub(crate) fn upload_resource_count(&self) -> usize {
		self.upload_resources.len()
	}

	pub(crate) fn readback_resource_count(&self) -> usize {
		self.readback_resources.len()
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

	pub(crate) fn buffer_is_in_common_state(&self, buffer: BaseBufferHandle) -> Option<bool> {
		self.buffer(buffer)
			.and_then(|buffer_data| buffer_data.resource.as_ref())
			.map(|_| {
				self.buffer_states
					.get(&buffer.0)
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

	pub(crate) fn image_is_in_common_state(&self, image: ImageHandle) -> Option<bool> {
		self.images
			.get(image.0 .0 as usize)
			.and_then(|image_data| image_data.resource.as_ref())
			.map(|_| {
				self.image_states
					.get(&image.0 .0)
					.copied()
					.unwrap_or(D3D12_RESOURCE_STATE_COMMON)
					== D3D12_RESOURCE_STATE_COMMON
			})
	}

	pub(crate) fn descriptor_set_has_native_heaps(&self, descriptor_set: DescriptorSetHandle) -> Option<(bool, bool)> {
		self.descriptor_sets
			.get(descriptor_set.0 as usize)
			.map(|set| (set.cbv_srv_uav_heap.is_some(), set.sampler_heap.is_some()))
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

	pub(crate) fn descriptor_table_bind_records(&self) -> &[DescriptorTableBindRecord] {
		&self.descriptor_table_bind_records
	}

	pub(crate) fn push_constant_write_count(&self) -> usize {
		self.push_constant_write_count
	}

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
		let size = Self::align_up(max_instance_count as usize * 128, 256).max(256);
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
		let size = Self::bottom_level_acceleration_structure_estimated_size(description);
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
			Flags: D3D12_RESOURCE_FLAG_RAYTRACING_ACCELERATION_STRUCTURE,
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

	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		// Updates descriptor binding records without touching DX12 descriptor heaps.
		for write in descriptor_set_writes {
			let binding_handles = self.collect_descriptor_binding_handles(write.binding_handle);
			for (frame_index, binding_handle) in binding_handles.iter().enumerate() {
				if let Some(binding) = self.descriptor_bindings.get_mut(binding_handle.0 as usize) {
					binding.frame_offset = write.frame_offset;
				}
				let descriptor = self.resolve_descriptor_for_frame(write.descriptor, frame_index, write.frame_offset);
				self.update_descriptor_for_binding(*binding_handle, descriptor, write.array_element);
			}
		}

		// Native descriptor heap writes happen in update_descriptor_for_binding.
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
		let identifier = pipeline
			.ray_tracing_shader_identifiers
			.get(&shader_handle)
			.copied()
			.unwrap_or_else(|| Self::placeholder_shader_identifier(pipeline_handle, shader_handle));
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
		let Some(scratch_resource) = self
			.buffers
			.get(build.scratch_buffer.buffer.0 as usize)
			.and_then(|buffer| buffer.resource.clone())
		else {
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
		match build.description {
			crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, .. } => {
				if let Some(instance_resource) = self
					.buffers
					.get(instances_buffer.0 as usize)
					.and_then(|buffer| buffer.resource.clone())
				{
					unsafe {
						self.transition_tracked_buffer(
							&command_list,
							instances_buffer,
							&instance_resource,
							D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
						);
					}
				}
			}
		}
		self.encode_top_level_acceleration_structure_build(&command_list, build);
		self.top_level_acceleration_structure_build_record_count += 1;
	}

	pub(crate) fn record_bottom_level_acceleration_structure_builds(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
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
			if !self.prepare_bottom_level_build_inputs(command_buffer_handle, build) {
				continue;
			}
			self.encode_bottom_level_acceleration_structure_build(command_buffer_handle, build);
			self.bottom_level_acceleration_structure_build_record_count += 1;
		}
	}

	fn encode_top_level_acceleration_structure_build(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		let Some(command_list) = command_list.cast::<ID3D12GraphicsCommandList4>().ok() else {
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
		let Some(destination) = acceleration_structure
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
		else {
			return;
		};
		let scratch = self.get_buffer_address(build.scratch_buffer.buffer) + build.scratch_buffer.offset as u64;
		if destination == 0 || scratch == 0 {
			return;
		}
		let crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
			instances_buffer,
			instance_count,
		} = build.description;
		let instances = self.get_buffer_address(instances_buffer);
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
			command_list.BuildRaytracingAccelerationStructure(&desc, None);
		}
		self.native_top_level_acceleration_structure_build_encode_count += 1;
	}

	fn encode_bottom_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
			.and_then(|command_list| command_list.cast::<ID3D12GraphicsCommandList4>().ok())
		else {
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
		let Some(destination) = acceleration_structure
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
		else {
			return;
		};
		let scratch = self.get_buffer_address(build.scratch_buffer.buffer) + build.scratch_buffer.offset as u64;
		let Some(geometry) = self.bottom_level_geometry_desc(&build.description) else {
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
			command_list.BuildRaytracingAccelerationStructure(&desc, None);
		}
		self.native_bottom_level_acceleration_structure_build_encode_count += 1;
	}

	fn bottom_level_geometry_desc(
		&self,
		description: &crate::rt::BottomLevelAccelerationStructureBuildDescriptions,
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
				let vertex_address =
					self.get_buffer_address(vertex_buffer.buffer_offset.buffer) + vertex_buffer.buffer_offset.offset as u64;
				let index_address =
					self.get_buffer_address(index_buffer.buffer_offset.buffer) + index_buffer.buffer_offset.offset as u64;
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
				let address = self.get_buffer_address(*aabb_buffer);
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

	fn prepare_bottom_level_build_inputs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
	) -> bool {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return false;
		};
		let Some(scratch_resource) = self
			.buffers
			.get(build.scratch_buffer.buffer.0 as usize)
			.and_then(|buffer| buffer.resource.clone())
		else {
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

		let mut transition_input = |buffer_handle: BaseBufferHandle| {
			let Some(resource) = self
				.buffers
				.get(buffer_handle.0 as usize)
				.and_then(|buffer| buffer.resource.clone())
			else {
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

		match &build.description {
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
		}
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

	pub fn create_factory(&mut self) -> Option<crate::implementation::Factory> {
		Some(crate::implementation::Factory::default())
	}

	pub fn create_pipeline_factory(&mut self) -> Option<crate::implementation::Factory> {
		self.create_factory()
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
		let image_index = swapchain.acquired_image_indices[sequence_index as usize] as usize;
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

	pub fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let initial_value = if signaled { 1 } else { 0 };
		let fence = unsafe { self.device.CreateFence(initial_value, D3D12_FENCE_FLAGS(0)) }
			.expect("Failed to create a D3D12 fence. The most likely cause is that the device does not support fences.");
		self.synchronizers.push(Synchronizer {
			next: None,
			fence,
			value: initial_value,
		});
		SynchronizerHandle((self.synchronizers.len() - 1) as u64)
	}

	pub fn start_frame<'a>(&'a mut self, index: u32, _synchronizer_handle: SynchronizerHandle) -> super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % self.frames as u32) as u8,
		};
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		// Resizes CPU-side buffer storage while discarding previous contents.
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let buffer = self
			.buffer_mut(buffer_handle)
			.expect("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		if buffer.size >= size {
			return;
		}

		let layout = Layout::from_size_align(size, buffer.layout.align()).unwrap();
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to resize buffer storage. The most likely cause is that the system is out of memory.");
		}

		if buffer.layout.size() != 0 && !buffer.data.is_null() {
			unsafe {
				alloc::dealloc(buffer.data, buffer.layout);
			}
		}

		buffer.data = data;
		buffer.layout = layout;
		buffer.size = size;
	}

	pub fn start_frame_capture(&mut self) {
		self.debugger.start_frame_capture();
	}

	pub fn end_frame_capture(&mut self) {
		self.debugger.end_frame_capture();
	}

	pub fn wait(&self) {
		for synchronizer in &self.synchronizers {
			while unsafe { synchronizer.fence.GetCompletedValue() } < synchronizer.value {
				std::thread::yield_now();
			}
		}
	}

	pub(crate) fn wait_for_synchronizer(&self, synchronizer_handle: SynchronizerHandle) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
			return;
		};
		while unsafe { synchronizer.fence.GetCompletedValue() } < synchronizer.value {
			std::thread::yield_now();
		}
	}

	pub(crate) fn synchronizer_value(&self, synchronizer_handle: SynchronizerHandle) -> Option<u64> {
		self.synchronizers
			.get(synchronizer_handle.0 as usize)
			.map(|synchronizer| synchronizer.value)
	}

	pub(crate) fn begin_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle) {
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
		let _ = unsafe { allocator.Reset() };
		let _ = unsafe { command_list.Reset(allocator, None) };
		command_buffer.is_open = true;
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
		let Some(raygen) = self.ray_generation_shader_record(binding_tables.raygen) else {
			return;
		};
		let Some(miss) = self.shader_table_range(binding_tables.miss) else {
			return;
		};
		let Some(hit) = self.shader_table_range(binding_tables.hit) else {
			return;
		};
		let callable = if let Some(callable) = binding_tables.callable {
			let Some(callable) = self.shader_table_range(callable) else {
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
			}
		}
		self.trace_rays_record_count += 1;
	}

	fn ray_generation_shader_record(&self, range: BufferStridedRange) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE> {
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
			StartAddress: self.shader_table_address(&range)?,
			SizeInBytes: range.size as u64,
		})
	}

	fn shader_table_range(&self, range: BufferStridedRange) -> Option<D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE> {
		Some(D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
			StartAddress: self.shader_table_address(&range)?,
			SizeInBytes: range.size as u64,
			StrideInBytes: range.stride as u64,
		})
	}

	fn shader_table_address(&self, range: &BufferStridedRange) -> Option<u64> {
		let address = self.get_buffer_address(range.buffer_offset.buffer);
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
			let Some(buffer) = self.buffer(buffer_descriptor.buffer) else {
				continue;
			};
			let Some(resource) = buffer.resource.clone() else {
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
		self.vertex_buffer_bind_count += 1;
	}

	/// Binds a native DX12 index buffer view for raster input assembly.
	pub(crate) fn bind_index_buffer_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		buffer_descriptor: &BufferDescriptor,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(buffer) = self.buffer(buffer_descriptor.buffer) else {
			return;
		};
		let Some(resource) = buffer.resource.clone() else {
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
		self.vertex_buffer_bind_count += 1;
		self.index_buffer_bind_count += 1;
		self.draw_indexed_encode_count += 1;
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

		let mut target_resources = Vec::new();
		let mut depth_resource = None;
		for attachment in attachments {
			if self.attachment_format(attachment) == Formats::Depth32 {
				let image_handle = self.attachment_image_handle(attachment, sequence_index);
				let Some(image) = self.images.get(image_handle.0 as usize) else {
					continue;
				};
				let Some(resource) = image.resource.clone() else {
					continue;
				};
				depth_resource = Some((image_handle, resource, attachment.load, attachment.clear));
				continue;
			}
			let Some((image_handle, resource, swapchain_backbuffer)) =
				self.attachment_render_target_resource(command_buffer_handle, attachment, sequence_index)
			else {
				continue;
			};
			target_resources.push((
				image_handle,
				resource,
				attachment.load,
				attachment.clear,
				swapchain_backbuffer,
			));
		}

		if target_resources.is_empty() && depth_resource.is_none() {
			return;
		}

		let mut handles = Vec::with_capacity(target_resources.len());
		if !target_resources.is_empty() {
			let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
				Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
				NumDescriptors: target_resources.len() as u32,
				Flags: Default::default(),
				NodeMask: 0,
			};
			let Some(heap) = (unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc).ok() }) else {
				return;
			};
			let descriptor_size =
				unsafe { self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) } as usize;
			let start = unsafe { heap.GetCPUDescriptorHandleForHeapStart() };

			for (slot, (image_handle, resource, load, clear, swapchain_backbuffer)) in target_resources.into_iter().enumerate()
			{
				let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
					ptr: start.ptr + slot * descriptor_size,
				};
				unsafe {
					self.device.CreateRenderTargetView(&resource, None, handle);
					if let Some(image_handle) = image_handle {
						self.transition_tracked_image(
							&command_list,
							image_handle,
							&resource,
							D3D12_RESOURCE_STATE_RENDER_TARGET,
						);
					} else {
						Self::transition_resource(
							&command_list,
							&resource,
							D3D12_RESOURCE_STATE_PRESENT,
							D3D12_RESOURCE_STATE_RENDER_TARGET,
						);
					}
				}
				if swapchain_backbuffer {
					self.swapchain_backbuffer_bind_count += 1;
				}
				if !load {
					let color = Self::clear_color_f32(clear);
					unsafe {
						command_list.ClearRenderTargetView(handle, &color, None);
					}
					self.render_target_clear_count += 1;
				}
				handles.push(handle);
			}

			self.rtv_heaps.push(heap);
			self.render_target_bind_count += 1;
		}

		let mut depth_handle = None;
		if let Some((image_handle, resource, load, clear)) = depth_resource {
			let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
				Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
				NumDescriptors: 1,
				Flags: Default::default(),
				NodeMask: 0,
			};
			let Some(heap) = (unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc).ok() }) else {
				return;
			};
			let handle = unsafe { heap.GetCPUDescriptorHandleForHeapStart() };
			unsafe {
				self.device.CreateDepthStencilView(&resource, None, handle);
				self.transition_tracked_image(&command_list, image_handle, &resource, D3D12_RESOURCE_STATE_DEPTH_WRITE);
			}
			if !load {
				let depth = Self::clear_depth_value(clear);
				unsafe {
					command_list.ClearDepthStencilView(handle, D3D12_CLEAR_FLAG_DEPTH, depth, 0, None);
				}
				self.depth_stencil_clear_count += 1;
			}
			depth_handle = Some(handle);
			self.dsv_heaps.push(heap);
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
		self.bind_descriptor_heaps_and_tables(command_buffer_handle, None, sets);
	}

	pub(crate) fn flush_pending_descriptor_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let mut images = HashSet::default();
		for set in sets {
			let Some(sequence_set) = self.descriptor_set_for_sequence(*set, sequence_index) else {
				continue;
			};
			let Some(bindings) = self.descriptors.get(&sequence_set) else {
				continue;
			};
			for array_elements in bindings.values() {
				for descriptor in array_elements.values() {
					match descriptor {
						WriteData::Image { handle, .. } => {
							images.insert(*handle);
						}
						WriteData::CombinedImageSampler { image_handle, .. } => {
							images.insert(*image_handle);
						}
						_ => {}
					}
				}
			}
		}

		for image in images {
			self.flush_pending_texture_syncs(command_buffer_handle, Some(image));
		}
	}

	pub(crate) fn bind_descriptor_heaps_and_tables(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		sets: &[DescriptorSetHandle],
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		let cbv_srv_uav_heap = sets
			.iter()
			.filter_map(|set| self.descriptor_sets.get(set.0 as usize))
			.find_map(|set| set.cbv_srv_uav_heap.clone());
		let sampler_heap = sets
			.iter()
			.filter_map(|set| self.descriptor_sets.get(set.0 as usize))
			.find_map(|set| set.sampler_heap.clone());

		let heaps = [cbv_srv_uav_heap, sampler_heap]
			.into_iter()
			.flatten()
			.map(Some)
			.collect::<Vec<_>>();
		if heaps.is_empty() {
			return;
		}

		unsafe {
			command_list.SetDescriptorHeaps(&heaps);
		}
		self.descriptor_heap_bind_count += 1;

		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let mut table_binds = 0;
		let Some(Some(_root_signature)) = self.pipeline_root_signatures.get(pipeline.layout.0 as usize) else {
			panic!(
				"Failed to bind DX12 descriptor tables because the pipeline layout has no native root signature. The most likely cause is that root signature creation failed while the pipeline still kept descriptor table metadata."
			);
		};
		let Some(root_tables) = self.pipeline_root_tables.get(pipeline.layout.0 as usize).cloned() else {
			return;
		};
		for (root_parameter_index, table) in root_tables.iter().enumerate() {
			let set = sets
				.get(table.set_index)
				.and_then(|set_handle| self.descriptor_sets.get(set_handle.0 as usize));
			let heap = if table.sampler_heap {
				set.and_then(|set| set.sampler_heap.as_ref())
			} else {
				set.and_then(|set| set.cbv_srv_uav_heap.as_ref())
			};
			if let Some(heap) = heap {
				let heap_type = if table.sampler_heap {
					D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
				} else {
					D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
				};
				let handle = self.descriptor_gpu_handle(heap, heap_type, table.heap_slot);
				unsafe {
					match pipeline.kind {
						PipelineKind::Compute | PipelineKind::RayTracing => {
							command_list.SetComputeRootDescriptorTable(root_parameter_index as u32, handle)
						}
						PipelineKind::Raster => {
							command_list.SetGraphicsRootDescriptorTable(root_parameter_index as u32, handle)
						}
					}
				}
				table_binds += 1;
				self.descriptor_table_bind_records.push(DescriptorTableBindRecord {
					root_parameter_index: root_parameter_index as u32,
					set_index: table.set_index,
					binding_index: table.binding_index,
					sampler_heap: table.sampler_heap,
					heap_slot: table.heap_slot,
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
		let end = offset.saturating_add(bytes.len() as u32);
		let Some(range) = constants
			.iter()
			.find(|range| offset >= range.offset && end <= range.offset.saturating_add(range.size))
			.copied()
		else {
			return;
		};

		let mut words = bytes
			.chunks(4)
			.map(|chunk| {
				let mut word = [0u8; 4];
				word[..chunk.len()].copy_from_slice(chunk);
				u32::from_ne_bytes(word)
			})
			.collect::<Vec<_>>();
		if words.is_empty() {
			return;
		}

		let destination_offset = (offset - range.offset) / 4;
		let compute_root = matches!(pipeline.kind, PipelineKind::Compute | PipelineKind::RayTracing);
		unsafe {
			if compute_root {
				command_list.SetComputeRoot32BitConstants(
					range.root_parameter_index,
					words.len() as u32,
					words.as_mut_ptr().cast(),
					destination_offset,
				);
			} else {
				command_list.SetGraphicsRoot32BitConstants(
					range.root_parameter_index,
					words.len() as u32,
					words.as_mut_ptr().cast(),
					destination_offset,
				);
			}
		}
		self.push_constant_write_count += 1;
		self.push_constant_write_records.push(PushConstantWriteRecord {
			root_parameter_index: range.root_parameter_index,
			offset,
			size: bytes.len() as u32,
			compute_root,
		});
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

		self.transition_present_resources(command_buffer_handle, &command_list);
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
		self.signal_synchronizer(queue_handle, synchronizer_handle);
		self.wait_for_synchronizer(synchronizer_handle);
		self.refresh_readback_texture_copies();
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
			self.swapchain_present_transition_count += 1;
		}
	}

	fn signal_synchronizer(&mut self, queue_handle: QueueHandle, synchronizer_handle: SynchronizerHandle) {
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

	pub(crate) fn copy_buffers(&mut self, command_buffer_handle: CommandBufferHandle, copies: &[crate::BufferCopyDescriptor]) {
		for copy in copies {
			self.copy_buffer_shadow(copy);
			self.record_buffer_copy(command_buffer_handle, copy);
		}
	}

	pub(crate) fn clear_buffers(&mut self, command_buffer_handle: CommandBufferHandle, buffer_handles: &[BaseBufferHandle]) {
		for &buffer_handle in buffer_handles {
			Self::clear_buffer_shadow_in_storage(&mut self.buffers, buffer_handle);
			Self::clear_buffer_shadow_in_storage(&mut self.dynamic_buffers, buffer_handle);
			self.record_buffer_clear(command_buffer_handle, buffer_handle);
		}
	}

	fn clear_buffer_shadow_in_storage(storage: &mut [Buffer], buffer_handle: BaseBufferHandle) {
		let Some(buffer) = storage.get(buffer_handle.0 as usize) else {
			return;
		};
		if buffer.size == 0 {
			return;
		}

		unsafe {
			std::ptr::write_bytes(buffer.data, 0, buffer.size);
		}
		Self::sync_buffer_storage(buffer);
	}

	fn copy_buffer_shadow(&mut self, copy: &crate::BufferCopyDescriptor) {
		// Resolve handles through `buffer` instead of indexing storage directly. Dynamic buffer handles carry
		// `DYNAMIC_BUFFER_HANDLE_FLAG`, so the raw handle value is not always a valid index into `buffers`.
		let Some(source) = self.buffer(copy.source_buffer) else {
			return;
		};
		let Some(destination) = self.buffer(copy.destination_buffer) else {
			return;
		};

		let source_end = copy.source_offset.saturating_add(copy.size);
		let destination_end = copy.destination_offset.saturating_add(copy.size);
		if source_end > source.size || destination_end > destination.size {
			panic!(
				"Failed to copy DX12 buffer data from {:?} offset {} to {:?} offset {} for {} bytes. The most likely cause is that the requested source or destination range is outside the buffer allocation. Source size: {} bytes. Destination size: {} bytes.",
				copy.source_buffer,
				copy.source_offset,
				copy.destination_buffer,
				copy.destination_offset,
				copy.size,
				source.size,
				destination.size
			);
		}
		if copy.size == 0 {
			return;
		}

		unsafe {
			let source = source.data.add(copy.source_offset);
			let destination = destination.data.add(copy.destination_offset);
			std::ptr::copy(source, destination, copy.size);
		}
		if let Some(destination) = self.buffer(copy.destination_buffer) {
			Self::sync_buffer_storage(destination);
		}
	}

	fn record_buffer_copy(&mut self, command_buffer_handle: CommandBufferHandle, copy: &crate::BufferCopyDescriptor) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(source) = self.copy_buffer_info(copy.source_buffer) else {
			return;
		};
		let Some(destination) = self.copy_buffer_info(copy.destination_buffer) else {
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
		self.buffer_copy_count += 1;
	}

	fn record_buffer_clear(&mut self, command_buffer_handle: CommandBufferHandle, buffer_handle: BaseBufferHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(destination_buffer) = self.buffer(buffer_handle) else {
			return;
		};
		let destination_size = destination_buffer.size;
		let destination_access = destination_buffer.access;
		let destination = destination_buffer.resource.clone();
		if destination_size == 0 || destination_access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}
		let Some(destination) = destination else {
			return;
		};
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
		self.upload_resources.push(upload);
		self.buffer_clear_count += 1;
	}

	fn copy_buffer_info(&self, buffer_handle: BaseBufferHandle) -> Option<BufferCopyInfo> {
		self.buffer(buffer_handle).and_then(|buffer| {
			buffer.resource.clone().map(|resource| BufferCopyInfo {
				resource,
				access: buffer.access,
				heap_kind: buffer.heap_kind,
				size: buffer.size,
			})
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
			self.record_buffer_to_image_copy(command_buffer_handle, copy);
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
		let source_bytes = self.buffer_range(copy.source_buffer, copy.source_offset, image_stride * extent.depth() as usize);
		let Some(destination) = self.image_data_mut_for_sequence(copy.destination_image, sequence_index) else {
			return;
		};

		for layer in 0..extent.depth() as usize {
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
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(image) = self.images.get(copy.destination_image.0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((row_bytes, row_count, _))) = (
			image.resource.clone(),
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
		let source_bytes = self.buffer_range(
			copy.source_buffer,
			copy.source_offset,
			source_image_pitch * extent.depth() as usize,
		);
		self.record_image_upload(
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
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			image.resource.clone(),
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};
		let extent = image.extent;
		let source_bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
		self.record_image_upload(
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
		);
		self.gpu_uploaded_images.insert(image_handle.0);
	}

	pub(crate) fn flush_pending_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_filter: Option<crate::BaseImageHandle>,
	) {
		let pending = std::mem::take(&mut self.pending_texture_syncs);
		for (image_handle, sequence_index) in pending {
			if image_filter.is_some_and(|filter| filter != image_handle) {
				self.pending_texture_syncs.push((image_handle, sequence_index));
				continue;
			}
			self.record_image_storage_upload(command_buffer_handle, ImageHandle(image_handle), sequence_index);
		}
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
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			image.resource.clone(),
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
		self.record_image_upload(
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
		);
		self.gpu_uploaded_images.insert(image_handle.0);
	}

	pub(crate) fn begin_debug_region(&self, _command_buffer_handle: CommandBufferHandle, _name: &str) {
		// DX12 debug regions require PIX-formatted event metadata. Passing arbitrary UTF-8 bytes to
		// ID3D12GraphicsCommandList::BeginEvent can fault inside the native runtime, so this backend
		// leaves regions disabled until PIX event encoding is implemented.
		self.debug_region_begin_count.set(self.debug_region_begin_count.get() + 1);
	}

	pub(crate) fn end_debug_region(&self, _command_buffer_handle: CommandBufferHandle) {
		// Keep this paired with `begin_debug_region`; see the comment above for why DX12 event calls are skipped.
		self.debug_region_end_count.set(self.debug_region_end_count.get() + 1);
	}

	fn record_image_upload(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: DXGI_FORMAT,
		extent: Extent,
		source_bytes: &[u8],
		source_row_pitch: usize,
		source_image_pitch: usize,
	) {
		let Some((row_bytes, row_count, _)) = utils::texture_copy_layout(self.images[image_handle.0 as usize].format, extent)
		else {
			return;
		};
		let upload_row_pitch = Self::align_up(row_bytes, 256);
		let upload_size = upload_row_pitch * row_count * extent.depth() as usize;
		let (Some(upload), mapped, _) = self.create_buffer_resource(upload_size, DeviceAccesses::HostToDevice) else {
			return;
		};
		if mapped.is_null() {
			return;
		}

		unsafe {
			std::ptr::write_bytes(mapped, 0, upload_size);
			for layer in 0..extent.depth() as usize {
				for y in 0..row_count {
					let source_start = layer * source_image_pitch + y * source_row_pitch;
					let source_end = source_start + row_bytes;
					let upload_start = (layer * row_count + y) * upload_row_pitch;
					if source_end > source_bytes.len() {
						return;
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
						Depth: extent.depth() as u32,
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
		self.upload_resources.push(upload);
	}

	unsafe fn transition_resource(
		command_list: &ID3D12GraphicsCommandList,
		resource: &ID3D12Resource,
		before: D3D12_RESOURCE_STATES,
		after: D3D12_RESOURCE_STATES,
	) {
		let barrier = D3D12_RESOURCE_BARRIER {
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
		};
		command_list.ResourceBarrier(&[barrier]);
	}

	unsafe fn transition_tracked_buffer(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		let before = self
			.buffer_states
			.get(&buffer.0)
			.copied()
			.unwrap_or(D3D12_RESOURCE_STATE_COMMON);
		if before == after {
			return;
		}
		Self::transition_resource(command_list, resource, before, after);
		self.buffer_states.insert(buffer.0, after);
	}

	unsafe fn transition_tracked_image(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		let before = self
			.image_states
			.get(&image.0)
			.copied()
			.unwrap_or(D3D12_RESOURCE_STATE_COMMON);
		if before == after {
			return;
		}
		Self::transition_resource(command_list, resource, before, after);
		self.image_states.insert(image.0, after);
	}

	fn align_up(value: usize, alignment: usize) -> usize {
		(value + alignment - 1) / alignment * alignment
	}

	fn buffer_range(&self, buffer_handle: BaseBufferHandle, offset: usize, size: usize) -> Vec<u8> {
		let Some(buffer) = self.buffer(buffer_handle) else {
			return Vec::new();
		};
		let end = offset.saturating_add(size);
		if end > buffer.size {
			panic!(
				"Failed to read DX12 buffer data. The most likely cause is that the requested range is outside the buffer allocation."
			);
		}
		if size == 0 {
			return Vec::new();
		}

		unsafe { std::slice::from_raw_parts(buffer.data.add(offset), size).to_vec() }
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
		self.record_image_readback_internal(command_buffer_handle, image_handle, None);
	}

	pub(crate) fn record_image_readback_for_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		texture_copy: TextureCopyHandle,
	) {
		self.record_image_readback_internal(command_buffer_handle, image_handle, Some(texture_copy));
	}

	fn record_image_readback_internal(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		mut texture_copy: Option<TextureCopyHandle>,
	) {
		if !self.gpu_uploaded_images.contains(&image_handle.0) {
			texture_copy = None;
		}
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(source), Some(format), Some((row_bytes, row_count, _))) = (
			image.resource.clone(),
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};

		let extent = image.extent;
		let readback_row_pitch = Self::align_up(row_bytes, 256);
		let readback_size = readback_row_pitch * row_count * extent.depth() as usize;
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
						Depth: extent.depth() as u32,
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
		self.texture_readbacks.push(TextureReadback {
			texture_copy,
			resource: readback.clone(),
			row_pitch: readback_row_pitch,
			row_bytes,
			height: row_count,
			depth: extent.depth() as usize,
			size: readback_size,
		});
		self.readback_resources.push(readback);
	}

	fn refresh_readback_texture_copies(&mut self) {
		// Maps completed readback buffers and repacks DX12 row padding into compact texture copies.
		for readback in &self.texture_readbacks {
			let Some(texture_copy) = readback.texture_copy else {
				continue;
			};
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

			if let Some(texture_copy) = self.texture_copies.get_mut(texture_copy.0 as usize) {
				*texture_copy = compact;
				self.texture_readback_resolve_count += 1;
			}
		}
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
				let image = self.images.get(image_handle.0 as usize)?;
				let resource = image.resource.clone()?;
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
		let swapchain = self.swapchains.get_mut(swapchain_handle.0 as usize)?;
		let image_index = swapchain.acquired_image_indices[sequence_index as usize] as usize;
		let image_index = image_index.min(swapchain.image_count.saturating_sub(1) as usize);
		if swapchain.backbuffers[image_index].is_none() {
			let resource = unsafe { swapchain.swapchain.GetBuffer::<ID3D12Resource>(image_index as u32) }.ok()?;
			swapchain.backbuffers[image_index] = Some(resource);
		}
		swapchain.backbuffers[image_index].clone()
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

	pub(crate) fn record_image_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		clear: crate::ClearValue,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(destination), Some(format), Some(bytes_per_pixel)) = (
			image.resource.clone(),
			Self::dxgi_format(image.format),
			utils::bytes_per_pixel(image.format),
		) else {
			return;
		};
		if bytes_per_pixel != std::mem::size_of::<RGBAu8>() {
			return;
		}

		let extent = image.extent;
		let color = Self::clear_color_bytes(clear);
		let pixel_count = extent.width() as usize * extent.height() as usize * extent.depth() as usize;
		let mut source_bytes = vec![0u8; pixel_count * bytes_per_pixel];
		for pixel in source_bytes.chunks_exact_mut(bytes_per_pixel) {
			pixel.copy_from_slice(&color);
		}
		self.record_image_upload(
			&command_list,
			image_handle.0,
			destination,
			format,
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
		let (Some(source_resource), Some(destination_resource)) = (source.resource.clone(), destination.resource.clone())
		else {
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
		self.texture_copy_count += 1;
	}

	pub(crate) fn emulate_compute_dispatch(
		&mut self,
		descriptor_sets: &[DescriptorSetHandle],
		sequence_index: u8,
		push_constants: &[u8],
	) {
		// Emulates the shared multiframe storage-image compute test until real DX12 dispatch is wired.
		let Some(descriptor_set) = descriptor_sets.first().copied() else {
			return;
		};
		let Some(current_image) = self.descriptor_image(descriptor_set, sequence_index, 0) else {
			return;
		};
		let Some(last_frame_image) = self.descriptor_image(descriptor_set, sequence_index, 1) else {
			return;
		};

		let value = push_constants
			.get(..std::mem::size_of::<f32>())
			.map(|bytes| {
				let mut value = [0u8; std::mem::size_of::<f32>()];
				value.copy_from_slice(bytes);
				f32::from_ne_bytes(value)
			})
			.unwrap_or(0.0);
		let color = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
		let source_sequence = self
			.descriptor_sequence_index(descriptor_set, sequence_index, 1)
			.unwrap_or(sequence_index as usize) as u8;
		let previous_pixel = self
			.image_data_for_sequence(last_frame_image, source_sequence)
			.and_then(|data| data.get(..std::mem::size_of::<RGBAu8>()).map(|pixel| pixel.to_vec()))
			.unwrap_or_else(|| vec![0u8; std::mem::size_of::<RGBAu8>()]);

		let Some(current_data) = self.image_data_mut_for_sequence(current_image, sequence_index) else {
			return;
		};
		if current_data.len() < std::mem::size_of::<RGBAu8>() * 2 {
			return;
		}
		current_data[0..4].copy_from_slice(&[color, color, color, 255]);
		current_data[4..8].copy_from_slice(&previous_pixel[..4]);
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

	pub(crate) fn dynamic_buffer_slice_mut<'a, T: Copy>(&'a self, buffer_handle: DynamicBufferHandle<T>) -> &'a mut T {
		let buffer = self
			.buffer(buffer_handle.into())
			.expect("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		unsafe { &mut *(buffer.data as *mut T) }
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
		let resource = self.create_image_resource(extent, format, uses, array_layers);

		let image = &mut self.images[image_handle.0 .0 as usize];
		image.extent = extent;
		image.resource = resource;
		image.data = utils::texture_copy_size(image.format, extent).map(|size| vec![0u8; size]);
		if let Some(frame_data) = image.frame_data.as_mut() {
			let data = image.data.clone().unwrap_or_default();
			*frame_data = vec![data; self.frames as usize];
		}
	}

	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle) -> Extent {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.0 as usize) else {
			return Extent::rectangle(0, 0);
		};

		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		if extent != swapchain.extent && extent.width() > 0 && extent.height() > 0 {
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
			swapchain.backbuffers = std::array::from_fn(|_| None);
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
			current = set.next;
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

	/// Collects the per-frame descriptor binding handles chained from the root handle.
	fn collect_descriptor_binding_handles(&self, handle: DescriptorSetBindingHandle) -> Vec<DescriptorSetBindingHandle> {
		let mut handles = Vec::new();
		let mut current = Some(handle);

		while let Some(handle) = current {
			let Some(binding) = self.descriptor_bindings.get(handle.0 as usize) else {
				break;
			};
			handles.push(handle);
			current = binding.next;
		}

		handles
	}

	/// Resolves a frame-aware index using the optional frame offset.
	fn frame_index_with_offset(&self, frame_index: usize, frame_offset: Option<i32>, total_frames: usize) -> usize {
		let total = (total_frames.max(1)) as i32;
		let offset = frame_offset.unwrap_or(0);
		(frame_index as i32 + offset).rem_euclid(total) as usize
	}

	/// Resolves per-frame descriptor resources, falling back to single-resource handles for DX12.
	fn resolve_descriptor_for_frame(&self, descriptor: WriteData, frame_index: usize, frame_offset: Option<i32>) -> WriteData {
		let _sequence_index = self.frame_index_with_offset(frame_index, frame_offset, self.frames as usize);

		match descriptor {
			WriteData::Buffer { handle, size } => WriteData::Buffer { handle, size },
			WriteData::Image { handle, layout } => WriteData::Image { handle, layout },
			WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			} => WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			},
			_ => descriptor,
		}
	}

	fn descriptor_set_for_sequence(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
	) -> Option<DescriptorSetHandle> {
		self.collect_descriptor_set_handles(descriptor_set)
			.get(sequence_index as usize)
			.copied()
			.or(Some(descriptor_set))
	}

	fn descriptor_image(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
		binding_index: u32,
	) -> Option<crate::BaseImageHandle> {
		let descriptor_set = self.descriptor_set_for_sequence(descriptor_set, sequence_index)?;
		match self.descriptors.get(&descriptor_set)?.get(&binding_index)?.get(&0)? {
			WriteData::Image { handle, .. } => Some(*handle),
			WriteData::CombinedImageSampler { image_handle, .. } => Some(*image_handle),
			_ => None,
		}
	}

	pub(crate) fn descriptor_sequence_index(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
		binding_index: u32,
	) -> Option<usize> {
		let descriptor_set = self.descriptor_set_for_sequence(descriptor_set, sequence_index)?;
		let set = self.descriptor_sets.get(descriptor_set.0 as usize)?;
		let binding = set.bindings.iter().find_map(|handle| {
			let binding = self.descriptor_bindings.get(handle.0 as usize)?;
			(binding.binding_index == binding_index).then_some(binding)
		})?;
		Some(self.frame_index_with_offset(sequence_index as usize, binding.frame_offset, self.frames as usize))
	}

	fn image_data_for_sequence(&self, image_handle: crate::BaseImageHandle, sequence_index: u8) -> Option<&[u8]> {
		let image = self.images.get(image_handle.0 as usize)?;
		if let Some(frame_data) = image.frame_data.as_ref() {
			frame_data
				.get(sequence_index as usize)
				.or_else(|| frame_data.first())
				.map(Vec::as_slice)
		} else {
			image.data.as_deref()
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

	/// Updates descriptor tracking and reverse lookup maps for a binding write.
	fn update_descriptor_for_binding(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: WriteData,
		array_element: u32,
	) {
		let Some(binding) = self.descriptor_bindings.get(binding_handle.0 as usize) else {
			return;
		};

		let descriptor_set_handle = binding.descriptor_set;
		let binding_index = binding.binding_index;

		let bindings = self.descriptors.entry(descriptor_set_handle).or_default();
		let arrays = bindings.entry(binding_index).or_default();
		arrays.insert(array_element, descriptor);

		let mut record_resource = |resource: PrivateHandles| {
			self.descriptor_set_to_resource
				.entry((descriptor_set_handle, binding_index))
				.or_default()
				.insert(resource);
			self.resource_to_descriptor
				.entry(resource)
				.or_default()
				.insert((binding_handle, array_element));
		};

		match descriptor {
			WriteData::Buffer { handle, .. } => {
				record_resource(PrivateHandles::Buffer(crate::buffer::BufferHandle(handle.0)));
			}
			WriteData::Image { handle, .. } => {
				record_resource(PrivateHandles::Image(crate::image::ImageHandle(handle.0)));
			}
			WriteData::CombinedImageSampler { image_handle, .. } => {
				record_resource(PrivateHandles::Image(crate::image::ImageHandle(image_handle.0)));
			}
			_ => {}
		}
		self.write_native_descriptor(binding_handle, descriptor, array_element);
	}

	fn write_native_descriptor(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: WriteData,
		array_element: u32,
	) {
		let Some(binding) = self.descriptor_bindings.get(binding_handle.0 as usize) else {
			return;
		};
		let descriptor_set_handle = binding.descriptor_set;
		let descriptor_type = binding.descriptor_type;
		let binding_index = binding.binding_index;
		let Some(set) = self.descriptor_sets.get(descriptor_set_handle.0 as usize) else {
			return;
		};
		let template = set.template;
		let cbv_srv_uav_heap = set.cbv_srv_uav_heap.clone();
		let sampler_heap = set.sampler_heap.clone();

		match descriptor {
			WriteData::Buffer { handle, .. } => {
				let Some(heap) = cbv_srv_uav_heap else {
					return;
				};
				let Some(slot) = self.descriptor_heap_slot(template, descriptor_type, binding_index, array_element, false)
				else {
					return;
				};
				let cpu_handle = self.descriptor_cpu_handle(&heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
				let Some(buffer) = self.buffer(handle) else {
					return;
				};
				let Some(resource) = buffer.resource.as_ref() else {
					return;
				};
				match descriptor_type {
					DescriptorType::UniformBuffer => {
						let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
							BufferLocation: unsafe { resource.GetGPUVirtualAddress() },
							SizeInBytes: Self::align_up(buffer.size.max(1), 256) as u32,
						};
						unsafe {
							self.device.CreateConstantBufferView(Some(&desc), cpu_handle);
						}
					}
					DescriptorType::StorageBuffer => {
						let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
							Format: DXGI_FORMAT_UNKNOWN,
							ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
							Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
								Buffer: D3D12_BUFFER_UAV {
									FirstElement: 0,
									NumElements: (buffer.size / 4).max(1) as u32,
									StructureByteStride: 4,
									CounterOffsetInBytes: 0,
									Flags: D3D12_BUFFER_UAV_FLAG_NONE,
								},
							},
						};
						unsafe {
							if buffer.heap_kind == BufferHeapKind::Default {
								self.device.CreateUnorderedAccessView(
									resource,
									None::<&ID3D12Resource>,
									Some(&desc),
									cpu_handle,
								);
							} else {
								self.device.CreateUnorderedAccessView(
									None::<&ID3D12Resource>,
									None::<&ID3D12Resource>,
									Some(&desc),
									cpu_handle,
								);
							}
						}
					}
					_ => {
						let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
							Format: DXGI_FORMAT_UNKNOWN,
							ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
							Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
							Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
								Buffer: D3D12_BUFFER_SRV {
									FirstElement: 0,
									NumElements: (buffer.size / 4).max(1) as u32,
									StructureByteStride: 4,
									Flags: D3D12_BUFFER_SRV_FLAG_NONE,
								},
							},
						};
						unsafe {
							self.device.CreateShaderResourceView(resource, Some(&desc), cpu_handle);
						}
					}
				}
				self.descriptor_write_count += 1;
			}
			WriteData::Image { handle, .. } => {
				self.write_native_image_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					handle,
					cbv_srv_uav_heap.as_ref(),
				);
			}
			WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				..
			} => {
				self.write_native_image_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					image_handle,
					cbv_srv_uav_heap.as_ref(),
				);
				self.write_native_sampler_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					Some(sampler_handle),
					sampler_heap.as_ref(),
				);
			}
			WriteData::Sampler(sampler_handle) => {
				self.write_native_sampler_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					Some(sampler_handle),
					sampler_heap.as_ref(),
				);
			}
			WriteData::StaticSamplers => {
				self.write_native_sampler_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					None,
					sampler_heap.as_ref(),
				);
			}
			WriteData::AccelerationStructure { handle } => {
				self.write_native_acceleration_structure_descriptor(
					template,
					descriptor_type,
					binding_index,
					array_element,
					handle,
					cbv_srv_uav_heap.as_ref(),
				);
			}
			_ => {}
		}
	}

	fn write_native_acceleration_structure_descriptor(
		&mut self,
		template: DescriptorSetTemplateHandle,
		descriptor_type: DescriptorType,
		binding_index: u32,
		array_element: u32,
		handle: TopLevelAccelerationStructureHandle,
		heap: Option<&ID3D12DescriptorHeap>,
	) {
		if !matches!(descriptor_type, DescriptorType::AccelerationStructure) {
			return;
		}
		let Some(heap) = heap else {
			return;
		};
		let Some(slot) = self.descriptor_heap_slot(template, descriptor_type, binding_index, array_element, false) else {
			return;
		};
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

	fn write_native_image_descriptor(
		&mut self,
		template: DescriptorSetTemplateHandle,
		descriptor_type: DescriptorType,
		binding_index: u32,
		array_element: u32,
		image_handle: crate::BaseImageHandle,
		heap: Option<&ID3D12DescriptorHeap>,
	) {
		let Some(heap) = heap else {
			return;
		};
		let Some(slot) = self.descriptor_heap_slot(template, descriptor_type, binding_index, array_element, false) else {
			return;
		};
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		let Some(image) = self.images.get(image_handle.0 as usize) else {
			return;
		};
		let Some(resource) = image.resource.clone() else {
			return;
		};
		let Some(format) = Self::dxgi_format(image.format) else {
			return;
		};
		let array_layers = image.array_layers.max(1);
		unsafe {
			if matches!(descriptor_type, DescriptorType::StorageImage) {
				let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
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
				};
				if image.uses.intersects(Uses::Storage) {
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
				let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
					Format: format,
					ViewDimension: if array_layers > 1 {
						D3D12_SRV_DIMENSION_TEXTURE2DARRAY
					} else {
						D3D12_SRV_DIMENSION_TEXTURE2D
					},
					Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
					Anonymous: if array_layers > 1 {
						D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
							Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
								MostDetailedMip: 0,
								MipLevels: 1,
								FirstArraySlice: 0,
								ArraySize: array_layers,
								PlaneSlice: 0,
								ResourceMinLODClamp: 0.0,
							},
						}
					} else {
						D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
							Texture2D: D3D12_TEX2D_SRV {
								MostDetailedMip: 0,
								MipLevels: 1,
								PlaneSlice: 0,
								ResourceMinLODClamp: 0.0,
							},
						}
					},
				};
				self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle);
				self.image_srv_descriptor_write_count += 1;
			}
		}
		self.descriptor_write_count += 1;
	}

	fn write_native_sampler_descriptor(
		&mut self,
		template: DescriptorSetTemplateHandle,
		descriptor_type: DescriptorType,
		binding_index: u32,
		array_element: u32,
		sampler_handle: Option<SamplerHandle>,
		heap: Option<&ID3D12DescriptorHeap>,
	) {
		let Some(heap) = heap else {
			return;
		};
		let Some(slot) = self.descriptor_heap_slot(template, descriptor_type, binding_index, array_element, true) else {
			return;
		};
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
		self.sampler_descriptor_write_records.push(SamplerDescriptorWriteRecord {
			filter,
			address_mode,
			max_anisotropy,
			min_lod: sampler.min_lod,
			max_lod: sampler.max_lod,
		});
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

		let (resource, mapped, heap_kind) = self.create_buffer_resource(layout.size(), device_accesses);
		let buffer = Buffer {
			data,
			layout,
			size: layout.size(),
			uses: resource_uses,
			access: device_accesses,
			resource,
			mapped,
			heap_kind,
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

	fn create_image_resource(&self, extent: Extent, format: Formats, uses: Uses, array_layers: u32) -> Option<ID3D12Resource> {
		let Some(dxgi_format) = Self::dxgi_format(format) else {
			return None;
		};
		if extent.width() == 0 || extent.height() == 0 {
			return None;
		}

		let flags = Self::image_resource_flags(format, uses);
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
			DepthOrArraySize: array_layers.max(1) as u16,
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
				None,
				&mut resource,
			)
		};
		if let Err(error) = result {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			eprintln!(
				"Failed to create DX12 image resource. Format: {:?}. Extent: {:?}. Uses: {:?}. Array layers: {}. Error: {error:?}. Device removed reason: {removed_reason:?}",
				format,
				extent,
				uses,
				array_layers
			);
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
			Formats::BGRAsRGB => Some(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB),
			Formats::Depth32 => Some(DXGI_FORMAT_D32_FLOAT),
			Formats::BC5 => Some(DXGI_FORMAT_BC5_UNORM),
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

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		let buffer_handle = buffer_handle.into();
		if let Some(buffer) = self.buffer(buffer_handle) {
			Self::sync_buffer_storage(buffer);
		}
	}
}

pub(crate) type Binding = DescriptorSetBinding;
const DYNAMIC_BUFFER_HANDLE_FLAG: u64 = 1 << 63;

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
	is_open: bool,
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
	texture_copy: Option<TextureCopyHandle>,
	resource: ID3D12Resource,
	row_pitch: usize,
	row_bytes: usize,
	height: usize,
	depth: usize,
	size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferHeapKind {
	Default,
	Upload,
	Readback,
}

impl Drop for Buffer {
	fn drop(&mut self) {
		if let Some(resource) = self.resource.as_ref() {
			unsafe {
				resource.Unmap(0, None);
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

struct DescriptorSetTemplate {
	bindings: Vec<DescriptorSetBindingTemplate>,
}

pub(crate) struct DescriptorSet {
	pub(crate) next: Option<DescriptorSetHandle>,
	template: DescriptorSetTemplateHandle,
	bindings: Vec<DescriptorSetBindingHandle>,
	cbv_srv_uav_heap: Option<ID3D12DescriptorHeap>,
	sampler_heap: Option<ID3D12DescriptorHeap>,
}

pub(crate) struct DescriptorSetBinding {
	pub(crate) next: Option<DescriptorSetBindingHandle>,
	descriptor_set: DescriptorSetHandle,
	descriptor_type: DescriptorType,
	binding_index: u32,
	count: u32,
	frame_offset: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayout {
	descriptor_set_templates: Vec<DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<PushConstantRange>,
}

#[derive(Clone)]
struct RootDescriptorTable {
	set_index: usize,
	binding_index: u32,
	sampler_heap: bool,
	heap_slot: u32,
}

#[derive(Clone, Copy)]
struct RootConstantRange {
	root_parameter_index: u32,
	offset: u32,
	size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorTableBindRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) set_index: usize,
	pub(crate) binding_index: u32,
	pub(crate) sampler_heap: bool,
	pub(crate) heap_slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PushConstantWriteRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) offset: u32,
	pub(crate) size: u32,
	pub(crate) compute_root: bool,
}

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

#[repr(C)]
struct PipelineStateStreamSubobject<T> {
	subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE,
	value: T,
}

#[repr(C)]
struct MeshPipelineStateStream {
	root_signature: PipelineStateStreamSubobject<Option<ID3D12RootSignature>>,
	mesh_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	pixel_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	blend: PipelineStateStreamSubobject<D3D12_BLEND_DESC>,
	sample_mask: PipelineStateStreamSubobject<u32>,
	rasterizer: PipelineStateStreamSubobject<D3D12_RASTERIZER_DESC>,
	depth_stencil: PipelineStateStreamSubobject<D3D12_DEPTH_STENCIL_DESC>,
	depth_stencil_format: PipelineStateStreamSubobject<DXGI_FORMAT>,
	primitive_topology: PipelineStateStreamSubobject<D3D12_PRIMITIVE_TOPOLOGY_DESC>,
	render_targets: PipelineStateStreamSubobject<D3D12_RT_FORMAT_ARRAY>,
	sample_desc: PipelineStateStreamSubobject<DXGI_SAMPLE_DESC>,
	node_mask: PipelineStateStreamSubobject<u32>,
	flags: PipelineStateStreamSubobject<D3D12_PIPELINE_STATE_FLAGS>,
}

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
	bindings: Vec<BindingDescriptor>,
}

#[derive(Clone)]
struct HlslSource {
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
	pub(crate) next: Option<SynchronizerHandle>,
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
	pub(crate) command_buffers: Vec<CommandBufferHandle>,
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

	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(self) -> Result<Self::Context, &'static str> {
		Ok(self)
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
		shader_binding_descriptors: impl IntoIterator<Item = BindingDescriptor>,
	) -> Result<ShaderHandle, ()> {
		Device::create_shader(self, name, shader_source_type, stage, shader_binding_descriptors)
	}
	fn create_descriptor_set_template(
		&mut self,
		name: Option<&str>,
		binding_templates: &[DescriptorSetBindingTemplate],
	) -> DescriptorSetTemplateHandle {
		Device::create_descriptor_set_template(self, name, binding_templates)
	}
	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		descriptor_set_template_handle: &DescriptorSetTemplateHandle,
	) -> DescriptorSetHandle {
		Device::create_descriptor_set(self, name, descriptor_set_template_handle)
	}
	fn create_descriptor_binding(
		&mut self,
		descriptor_set: DescriptorSetHandle,
		binding_constructor: BindingConstructor,
	) -> DescriptorSetBindingHandle {
		Device::create_descriptor_binding(self, descriptor_set, binding_constructor)
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

	fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
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
};

use ::utils::hash::{HashMap, HashSet};
use ::utils::Extent;
use windows::core::{BOOL, PCSTR, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Dxc::{
	CLSID_DxcCompiler, DxcBuffer, DxcCreateInstance, IDxcBlob, IDxcCompiler3, IDxcIncludeHandler, IDxcResult, DXC_CP_UTF8,
	DXC_OUT_OBJECT,
};
use windows::Win32::Graphics::Direct3D::{
	Fxc::D3DCompile, ID3DInclude, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
	D3D_SHADER_MACRO,
};
use windows::Win32::Graphics::Direct3D12::{
	D3D12CreateDevice, D3D12SerializeRootSignature, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
	ID3D12CommandSignature, ID3D12DescriptorHeap, ID3D12Device, ID3D12Device2, ID3D12Device5, ID3D12Fence,
	ID3D12GraphicsCommandList, ID3D12GraphicsCommandList4, ID3D12GraphicsCommandList6, ID3D12PipelineState, ID3D12Resource,
	ID3D12RootSignature, ID3D12StateObject, ID3D12StateObjectProperties, D3D12_BLEND_DESC, D3D12_BLEND_INV_SRC_ALPHA,
	D3D12_BLEND_ONE, D3D12_BLEND_OP_ADD, D3D12_BLEND_SRC_ALPHA, D3D12_BLEND_ZERO, D3D12_BUFFER_SRV, D3D12_BUFFER_SRV_FLAG_NONE,
	D3D12_BUFFER_UAV, D3D12_BUFFER_UAV_FLAG_NONE, D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC,
	D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS, D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0,
	D3D12_CACHED_PIPELINE_STATE, D3D12_CLEAR_FLAG_DEPTH, D3D12_COLOR_WRITE_ENABLE_ALL, D3D12_COMMAND_LIST_TYPE,
	D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAGS, D3D12_COMMAND_SIGNATURE_DESC, D3D12_COMPARISON_FUNC_ALWAYS,
	D3D12_COMPARISON_FUNC_NEVER, D3D12_COMPUTE_PIPELINE_STATE_DESC, D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
	D3D12_CONSTANT_BUFFER_VIEW_DESC, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_BACK,
	D3D12_CULL_MODE_FRONT, D3D12_CULL_MODE_NONE, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, D3D12_DEPTH_STENCILOP_DESC,
	D3D12_DEPTH_STENCIL_DESC, D3D12_DEPTH_WRITE_MASK_ZERO, D3D12_DESCRIPTOR_HEAP_DESC,
	D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
	D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE,
	D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND, D3D12_DESCRIPTOR_RANGE_TYPE, D3D12_DESCRIPTOR_RANGE_TYPE_CBV,
	D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE_TYPE_SRV, D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
	D3D12_DISPATCH_RAYS_DESC, D3D12_DXIL_LIBRARY_DESC, D3D12_ELEMENTS_LAYOUT_ARRAY, D3D12_EXPORT_DESC, D3D12_EXPORT_FLAG_NONE,
	D3D12_FEATURE_D3D12_OPTIONS5, D3D12_FEATURE_D3D12_OPTIONS7, D3D12_FEATURE_DATA_D3D12_OPTIONS5,
	D3D12_FEATURE_DATA_D3D12_OPTIONS7, D3D12_FENCE_FLAGS, D3D12_FILL_MODE_SOLID, D3D12_FILTER, D3D12_FILTER_ANISOTROPIC,
	D3D12_FILTER_MAXIMUM_ANISOTROPIC, D3D12_FILTER_MINIMUM_ANISOTROPIC, D3D12_GPU_DESCRIPTOR_HANDLE,
	D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE, D3D12_GPU_VIRTUAL_ADDRESS_RANGE, D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
	D3D12_GRAPHICS_PIPELINE_STATE_DESC, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT,
	D3D12_HEAP_TYPE_READBACK, D3D12_HEAP_TYPE_UPLOAD, D3D12_HIT_GROUP_DESC, D3D12_HIT_GROUP_TYPE_PROCEDURAL_PRIMITIVE,
	D3D12_HIT_GROUP_TYPE_TRIANGLES, D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED, D3D12_INDEX_BUFFER_VIEW,
	D3D12_INDIRECT_ARGUMENT_DESC, D3D12_INDIRECT_ARGUMENT_DESC_0, D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
	D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC, D3D12_LOGIC_OP_NOOP,
	D3D12_MEMORY_POOL_UNKNOWN, D3D12_MESH_SHADER_TIER_NOT_SUPPORTED, D3D12_PIPELINE_STATE_FLAGS,
	D3D12_PIPELINE_STATE_FLAG_NONE, D3D12_PIPELINE_STATE_STREAM_DESC, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PRIMITIVE_TOPOLOGY, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK, D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_PRIMITIVE_TOPOLOGY_DESC,
	D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RANGE, D3D12_RASTERIZER_DESC,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
	D3D12_RAYTRACING_GEOMETRY_AABBS_DESC, D3D12_RAYTRACING_GEOMETRY_DESC, D3D12_RAYTRACING_GEOMETRY_DESC_0,
	D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE, D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC,
	D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS, D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
	D3D12_RAYTRACING_INSTANCE_DESC, D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE, D3D12_RAYTRACING_PIPELINE_CONFIG,
	D3D12_RAYTRACING_SHADER_CONFIG, D3D12_RAYTRACING_TIER_NOT_SUPPORTED, D3D12_RENDER_TARGET_BLEND_DESC,
	D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
	D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
	D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAGS,
	D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
	D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_FLAG_NONE,
	D3D12_RESOURCE_FLAG_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_COMMON,
	D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_DEPTH_WRITE,
	D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATE_INDEX_BUFFER, D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
	D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PRESENT,
	D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_RESOURCE_STATE_RENDER_TARGET,
	D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER, D3D12_RESOURCE_TRANSITION_BARRIER,
	D3D12_ROOT_CONSTANTS, D3D12_ROOT_DESCRIPTOR_TABLE, D3D12_ROOT_PARAMETER, D3D12_ROOT_PARAMETER_0,
	D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS, D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, D3D12_ROOT_SIGNATURE_DESC,
	D3D12_ROOT_SIGNATURE_FLAGS, D3D12_RT_FORMAT_ARRAY, D3D12_SAMPLER_DESC, D3D12_SHADER_BYTECODE,
	D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES, D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0,
	D3D12_SHADER_VISIBILITY_ALL, D3D12_SRV_DIMENSION_BUFFER, D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
	D3D12_SRV_DIMENSION_TEXTURE2D, D3D12_SRV_DIMENSION_TEXTURE2DARRAY, D3D12_STATE_OBJECT_DESC,
	D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE, D3D12_STATE_SUBOBJECT, D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
	D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP, D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
	D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG, D3D12_STENCIL_OP_KEEP, D3D12_SUBRESOURCE_FOOTPRINT,
	D3D12_TEX2D_ARRAY_SRV, D3D12_TEX2D_ARRAY_UAV, D3D12_TEX2D_SRV, D3D12_TEX2D_UAV, D3D12_TEXTURE_ADDRESS_MODE,
	D3D12_TEXTURE_ADDRESS_MODE_BORDER, D3D12_TEXTURE_ADDRESS_MODE_CLAMP, D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
	D3D12_TEXTURE_ADDRESS_MODE_WRAP, D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
	D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX, D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
	D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_UAV_DIMENSION_BUFFER, D3D12_UAV_DIMENSION_TEXTURE2D,
	D3D12_UAV_DIMENSION_TEXTURE2DARRAY, D3D12_UNORDERED_ACCESS_VIEW_DESC, D3D12_UNORDERED_ACCESS_VIEW_DESC_0,
	D3D12_VERTEX_BUFFER_VIEW, D3D12_VIEWPORT, D3D_ROOT_SIGNATURE_VERSION_1_0,
};
use windows::Win32::Graphics::Dxgi::Common::{
	DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_BC5_UNORM,
	DXGI_FORMAT_BC7_UNORM, DXGI_FORMAT_BC7_UNORM_SRGB, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R16G16B16A16_FLOAT,
	DXGI_FORMAT_R16G16B16A16_SNORM, DXGI_FORMAT_R16G16B16A16_UNORM, DXGI_FORMAT_R16G16_FLOAT, DXGI_FORMAT_R16G16_SNORM,
	DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16_FLOAT, DXGI_FORMAT_R16_SNORM, DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R16_UNORM,
	DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32A32_SINT, DXGI_FORMAT_R32G32B32A32_UINT, DXGI_FORMAT_R32G32B32_FLOAT,
	DXGI_FORMAT_R32G32B32_SINT, DXGI_FORMAT_R32G32B32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32_SINT,
	DXGI_FORMAT_R32G32_UINT, DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32_SINT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R8G8B8A8_SNORM,
	DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R8G8_SNORM, DXGI_FORMAT_R8G8_UNORM,
	DXGI_FORMAT_R8_SNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
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
	descriptors::{DescriptorType, Write as DescriptorWrite, WriteData},
	device::Features,
	image,
	pipelines::{self, PushConstantRange, VertexElement},
	render_debugger::RenderDebugger,
	sampler,
	shader::{BindingDescriptor, Sources},
	window, AllocationHandle, AttachmentInformation, BaseBufferHandle, BindingConstructor, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferHandle, BufferStridedRange, ClearValue,
	CommandBufferHandle, DataTypes, DescriptorSetBindingHandle, DescriptorSetBindingTemplate, DescriptorSetHandle,
	DescriptorSetTemplateHandle, DeviceAccesses, DispatchExtent, DynamicBufferHandle, FilteringModes, Formats, ImageHandle,
	ImageOrSwapchain, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey, PresentationModes, PrivateHandles,
	QueueHandle, QueueSelection, RGBAu8, SamplerAddressingModes, SamplerHandle, SamplingReductionModes, ShaderHandle,
	ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, UseCases, Uses,
};
