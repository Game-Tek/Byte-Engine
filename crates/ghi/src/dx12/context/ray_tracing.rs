//! Creates a DX12 context's ray tracing pipelines, writes its shader binding tables, and records traces.
//!
//! A pipeline is a DXR state object linked from one DXIL library for the ray generation shader and one for each
//! distinct closest hit. Every library exports its entry point as `besl_main`, so the state object renames each export
//! to a name of its own. Closest hits are callable shaders, so the state object has no hit groups, and the shader
//! binding table's callable region holds one record for each hit record.

use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::core::HSTRING;

use super::*;
use crate::dx12::ray_tracing::{
	CALLABLE_OFFSET, IDENTIFIER_SIZE, MAX_TRACE_INVOCATIONS, RECORD_STRIDE, RayTracingState, ShaderBindingTable, WrittenTable,
	fill_shader_binding_table, table_size,
};

/// The name every generated shader's entry point has, which each library export renames.
const ENTRY_POINT: &str = "besl_main";

/// The export name of the ray generation shader.
const RAY_GENERATION_EXPORT: &str = "besl_ray_generation";

impl Device {
	/// Links a ray tracing pipeline from this context's shaders and returns the handle commands bind it with.
	pub fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let pipeline = self.compile_ray_tracing_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	/// Compiles a ray tracing pipeline without adding it to the context. See [`Self::compile_raster_pipeline`].
	pub(crate) fn compile_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> CompiledPipeline {
		self.require_ray_tracing("create a ray tracing pipeline");
		crate::pipelines::ray_tracing::check_hit_group_count("DX12", builder.hit_groups.len());
		let layout =
			self.ray_tracing_pipeline_layout(&builder.ray_generation, builder.hit_groups, builder.push_constant_ranges);
		let state = self.create_ray_tracing_state(&layout.root_signature, &builder);
		CompiledPipeline {
			layout,
			kind: PipelineKind::RayTracing,
			native: NativePipeline::RayTracing(Box::new(state)),
			has_mesh_shader: false,
			vertex_strides: SmallVec::new(),
		}
	}

	/// Compiles the pipeline's libraries and links them into a state object, with `root_signature` as the global root
	/// signature.
	fn create_ray_tracing_state(
		&mut self,
		root_signature: &ID3D12RootSignature,
		builder: &pipelines::ray_tracing::Builder,
	) -> RayTracingState {
		fn library_failure(stage: &str) -> ! {
			panic!(
				"DX12 ray tracing pipeline creation failed because the {stage} shader has no DXIL. The most likely cause is that its HLSL failed to compile, or that it was baked for another platform. Check the logged compiler errors, and bake the shader for DX12."
			)
		}
		// Hit groups that share a shader and its constants share one library.
		let mut libraries = vec![(
			HSTRING::from(RAY_GENERATION_EXPORT),
			self.shader_dxil(&builder.ray_generation)
				.unwrap_or_else(|| library_failure("ray generation")),
		)];
		let mut library_keys = Vec::<(ShaderHandle, &[pipelines::SpecializationMapEntry])>::new();
		let mut hit_group_libraries = Vec::with_capacity(builder.hit_groups.len());
		for hit_group in builder.hit_groups {
			let key = (*hit_group.handle, hit_group.specialization_map);
			let same = |(handle, map): &(ShaderHandle, &[pipelines::SpecializationMapEntry])| {
				*handle == key.0
					&& map.len() == key.1.len()
					&& map
						.iter()
						.zip(key.1)
						.all(|(a, b)| a.constant_id == b.constant_id && a.r#type == b.r#type && a.value == b.value)
			};
			let library = match library_keys.iter().position(same) {
				Some(index) => index + 1,
				None => {
					let dxil = self.shader_dxil(hit_group).unwrap_or_else(|| library_failure("closest-hit"));
					libraries.push((HSTRING::from(format!("besl_closest_hit_{}", library_keys.len())), dxil));
					library_keys.push(key);
					library_keys.len()
				}
			};
			hit_group_libraries.push(library);
		}

		let entry_point = HSTRING::from(ENTRY_POINT);
		let exports = libraries
			.iter()
			.map(|(name, _)| d3d12::D3D12_EXPORT_DESC {
				Name: PCWSTR(name.as_ptr()),
				ExportToRename: PCWSTR(entry_point.as_ptr()),
				Flags: d3d12::D3D12_EXPORT_FLAG_NONE,
			})
			.collect::<Vec<_>>();
		// Each library exports only its renamed entry point, so the helpers every library defines don't collide.
		let library_descs = libraries
			.iter()
			.zip(&exports)
			.map(|((_, dxil), export)| d3d12::D3D12_DXIL_LIBRARY_DESC {
				DXILLibrary: Self::shader_bytecode(dxil),
				NumExports: 1,
				pExports: export,
			})
			.collect::<Vec<_>>();
		// Callable parameters don't count toward the payload size, and no shader calls TraceRay or reads attributes,
		// so the smallest sizes and no recursion fit every pipeline.
		//
		// Known issue: on the RTX 3060, `CallShader` returns call data with any 16-bit member unchanged or zeroed, so
		// the BESL HLSL backend widens 16-bit payload members to 32 bits around each call (see `CALL_PAYLOAD_STRUCT`
		// in `resource-management/src/shader/besl/backends/hlsl/ray_tracing.rs`). Neither these sizes nor alignment
		// changed that. To check whether a driver or DXC update fixed it, give the `Payload` of the HLSL ray tracing
		// test shaders in `tests/rendering/ray_tracing.rs` a `float16_t3` member and run the hit group tests.
		let shader_config = d3d12::D3D12_RAYTRACING_SHADER_CONFIG {
			MaxPayloadSizeInBytes: 4,
			MaxAttributeSizeInBytes: 8,
		};
		let pipeline_config = d3d12::D3D12_RAYTRACING_PIPELINE_CONFIG {
			MaxTraceRecursionDepth: 0,
		};
		let mut global_root_signature = d3d12::D3D12_GLOBAL_ROOT_SIGNATURE {
			pGlobalRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
		};
		let subobject = |r#type, desc: *const std::ffi::c_void| d3d12::D3D12_STATE_SUBOBJECT {
			Type: r#type,
			pDesc: desc,
		};
		let mut subobjects = library_descs
			.iter()
			.map(|desc| {
				subobject(
					d3d12::D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
					(desc as *const d3d12::D3D12_DXIL_LIBRARY_DESC).cast(),
				)
			})
			.collect::<Vec<_>>();
		// Subobjects without an association apply to every export of the state object.
		subobjects.push(subobject(
			d3d12::D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
			(&shader_config as *const d3d12::D3D12_RAYTRACING_SHADER_CONFIG).cast(),
		));
		subobjects.push(subobject(
			d3d12::D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
			(&pipeline_config as *const d3d12::D3D12_RAYTRACING_PIPELINE_CONFIG).cast(),
		));
		subobjects.push(subobject(
			d3d12::D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
			(&global_root_signature as *const d3d12::D3D12_GLOBAL_ROOT_SIGNATURE).cast(),
		));
		let desc = d3d12::D3D12_STATE_OBJECT_DESC {
			Type: d3d12::D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
			NumSubobjects: subobjects.len() as u32,
			pSubobjects: subobjects.as_ptr(),
		};
		// SAFETY: Every subobject points into a value that lives until the call returns, and the call copies them.
		let state_object = unsafe { self.device.CreateStateObject::<d3d12::ID3D12StateObject>(&desc) };
		// SAFETY: The descriptor took its own clone of the root signature, which nothing reads once the call returned.
		unsafe { std::mem::ManuallyDrop::drop(&mut global_root_signature.pGlobalRootSignature) };
		let state_object = state_object.unwrap_or_else(|error| {
			self.drain_debug_messages();
			panic!(
				"DX12 ray tracing pipeline creation failed: {error:?}. The most likely cause is a shader that wasn't compiled as a DXIL library, or one whose sets the pipeline layout doesn't match. Check the logged debug layer messages, and bake ray tracing shaders with the lib_6_9 profile."
			)
		});
		self.name_object(&state_object, builder.name);
		let properties = state_object.cast::<d3d12::ID3D12StateObjectProperties>().expect(
			"DX12 ray tracing state object has no properties interface. The most likely cause is an outdated Agility SDK runtime.",
		);
		let identifier = |name: &HSTRING| -> [u8; IDENTIFIER_SIZE] {
			// SAFETY: The name is a live wide string, and the state object exports it.
			let pointer = unsafe { properties.GetShaderIdentifier(name) };
			assert!(
				!pointer.is_null(),
				"DX12 ray tracing state object exports no shader named {name}. The most likely cause is a library whose entry point isn't named {ENTRY_POINT}. Generate ray tracing shaders with the BESL HLSL generator."
			);
			// SAFETY: A non-null identifier points to IDENTIFIER_SIZE bytes the state object owns.
			unsafe { std::ptr::read_unaligned(pointer.cast::<[u8; IDENTIFIER_SIZE]>()) }
		};
		let ray_generation = identifier(&libraries[0].0);
		let library_identifiers = libraries.iter().map(|(name, _)| identifier(name)).collect::<Vec<_>>();
		// Only the ray generation shader calls closest hits, and closest hits call nothing, so the stack covers the ray
		// generation shader and the largest closest hit. The default reserves room for two callable levels.
		// SAFETY: Every name is a live wide string the state object exports.
		let stack_size = |name: &HSTRING| unsafe { properties.GetShaderStackSize(name) };
		let largest_hit = libraries[1..].iter().map(|(name, _)| stack_size(name)).max().unwrap_or(0);
		// SAFETY: No recording of this pipeline exists yet.
		unsafe { properties.SetPipelineStackSize(stack_size(&libraries[0].0) + largest_hit) };
		RayTracingState {
			state_object,
			ray_generation,
			hit_groups: hit_group_libraries
				.into_iter()
				.map(|library| library_identifiers[library])
				.collect(),
		}
	}

	/// Creates a shader binding table that holds no records.
	pub(crate) fn create_shader_binding_table(&mut self, name: Option<&str>) -> crate::ShaderBindingTableHandle {
		self.require_ray_tracing("create a shader binding table");
		self.shader_binding_tables.push(ShaderBindingTable {
			name: name.map(str::to_owned),
			written: None,
		});
		crate::ShaderBindingTableHandle::from_index(self.shader_binding_tables.len() as u64 - 1)
	}

	/// Returns the ray tracing state of `pipeline`, and panics, naming `operation`, when it is another kind of
	/// pipeline.
	fn ray_tracing_state(&self, pipeline: PipelineHandle, operation: &str) -> &RayTracingState {
		self.pipelines
			.get(pipeline.index() as usize)
			.and_then(|pipeline| match &pipeline.native {
				NativePipeline::RayTracing(state) => Some(state.as_ref()),
				NativePipeline::State(_) => None,
			})
			.unwrap_or_else(|| crate::pipelines::ray_tracing::not_ray_tracing_pipeline("DX12", operation))
	}

	/// Fills a new buffer with one record per entry of `hit_groups` for `pipeline`, and makes it the table's.
	///
	/// Recordings that traced with the table's old buffer keep it alive until they complete, so their traces keep their
	/// records.
	pub(crate) fn write_shader_binding_table(
		&mut self,
		table: crate::ShaderBindingTableHandle,
		pipeline: PipelineHandle,
		hit_groups: &[u32],
	) {
		self.require_ray_tracing("write a shader binding table");
		if table.index() as usize >= self.shader_binding_tables.len() {
			crate::pipelines::ray_tracing::unknown_table_write("DX12");
		}
		crate::pipelines::ray_tracing::check_records(
			"DX12",
			self.ray_tracing_state(pipeline, "write_shader_binding_table")
				.hit_groups
				.len(),
			hit_groups,
		);
		let size = table_size(hit_groups.len());
		let name = self.shader_binding_tables[table.index() as usize].name.clone();
		// The GPU reads a record each time a closest hit runs, from upload memory, which the CPU writes once.
		let resource = self.create_ray_tracing_buffer(name.as_deref(), size, D3D12_HEAP_TYPE_UPLOAD, D3D12_RESOURCE_FLAG_NONE);
		let mut mapped = std::ptr::null_mut();
		// SAFETY: Upload buffers map for writing, and the empty read range says the CPU reads nothing.
		unsafe { resource.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped)) }.expect(
			"DX12 shader binding table mapping failed. The most likely cause is that the device was removed. Check the device removed reason.",
		);
		// SAFETY: The mapping covers the buffer's `size` bytes, and only this write uses it.
		let memory = unsafe { std::slice::from_raw_parts_mut(mapped.cast::<u8>(), size as usize) };
		fill_shader_binding_table(
			self.ray_tracing_state(pipeline, "write_shader_binding_table"),
			hit_groups,
			memory,
		);
		// SAFETY: The CPU wrote every record it will, and the buffer stays mapped nowhere else.
		unsafe { resource.Unmap(0, None) };
		// SAFETY: The resource is a live buffer.
		let address = unsafe { resource.GetGPUVirtualAddress() };
		self.shader_binding_tables[table.index() as usize].written = Some(WrittenTable {
			pipeline,
			resource,
			ray_generation: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
				StartAddress: address,
				SizeInBytes: IDENTIFIER_SIZE as u64,
			},
			callable: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
				StartAddress: address + CALLABLE_OFFSET,
				SizeInBytes: RECORD_STRIDE * hit_groups.len() as u64,
				StrideInBytes: RECORD_STRIDE,
			},
		});
	}

	/// Launches the ray generation shader of `pipeline`, which the recording bound, once for each point of `extent`,
	/// with `table`'s callable region holding the closest hit of each hit record.
	pub(crate) fn trace_rays_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline: Option<PipelineHandle>,
		table: crate::ShaderBindingTableHandle,
		extent: Extent,
	) {
		let bound_pipeline = crate::pipelines::ray_tracing::bound_pipeline("DX12", pipeline);
		let shader_binding_table = self
			.shader_binding_tables
			.get(table.index() as usize)
			.unwrap_or_else(|| crate::pipelines::ray_tracing::unknown_table_trace("DX12"));
		crate::pipelines::ray_tracing::check_table_pipeline(
			"DX12",
			shader_binding_table.name.as_deref(),
			shader_binding_table.written.as_ref().map(|written| written.pipeline),
			bound_pipeline,
		);
		// The check panics for a table that was never written.
		let Some(written) = &shader_binding_table.written else {
			unreachable!()
		};
		let Some(grid @ [width, height, depth]) = crate::pipelines::ray_tracing::launch_grid(extent) else {
			return;
		};
		crate::pipelines::ray_tracing::check_launch("DX12", grid, MAX_TRACE_INVOCATIONS);
		let desc = d3d12::D3D12_DISPATCH_RAYS_DESC {
			RayGenerationShaderRecord: written.ray_generation,
			// Rays come from ray queries, so the trace needs no miss or hit group table.
			MissShaderTable: Default::default(),
			HitGroupTable: Default::default(),
			CallableShaderTable: written.callable,
			Width: width,
			Height: height,
			Depth: depth,
		};
		let resource = written.resource.clone();
		let command_list = self.command_list(command_buffer_handle);
		// SAFETY: The bound pipeline's state object made the records, the recording keeps the table's buffer alive until
		// it completes, and the regions are aligned as D3D12 requires.
		unsafe { command_list.DispatchRays(&desc) };
		self.retain_command_buffer_resource(command_buffer_handle, &resource);
		self.mark_command_buffer_work(command_buffer_handle);
	}
}
