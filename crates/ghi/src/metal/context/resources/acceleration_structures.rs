use super::super::*;

/// The byte size of one instance record Metal reads while building an instance acceleration structure.
pub(in crate::metal) const INSTANCE_DESCRIPTOR_SIZE: usize =
	std::mem::size_of::<mtl::MTLIndirectAccelerationStructureInstanceDescriptor>();

/// Maps a GHI vertex-position encoding to the Metal attribute format acceleration structures read positions with.
///
/// Metal builds triangle geometry from three-component positions, so only the component encoding varies.
pub(in crate::metal) fn to_vertex_format(encoding: crate::Encodings) -> mtl::MTLAttributeFormat {
	match encoding {
		crate::Encodings::FloatingPoint => mtl::MTLAttributeFormat::Float3,
		crate::Encodings::SignedNormalized => mtl::MTLAttributeFormat::Short4Normalized,
		crate::Encodings::UnsignedNormalized | crate::Encodings::sRGB => mtl::MTLAttributeFormat::UShort4Normalized,
	}
}

/// Maps a GHI index data type to the Metal index type acceleration structures read triangle indices with.
pub(in crate::metal) fn to_index_type(data_type: crate::DataTypes) -> mtl::MTLIndexType {
	match data_type {
		crate::DataTypes::U16 => mtl::MTLIndexType::UInt16,
		crate::DataTypes::U32 | crate::DataTypes::UInt => mtl::MTLIndexType::UInt32,
		_ => panic!(
			"Metal acceleration structure index format is unsupported. The most likely cause is that a non 16 or 32-bit index type was used for ray tracing geometry.",
		),
	}
}

/// Builds the sizing descriptor for one geometry class.
///
/// Metal derives storage and scratch sizes from counts and formats alone, so this descriptor carries no buffers
/// and can be queried before any build supplies geometry.
fn sizing_descriptor(geometry: AccelerationStructureGeometry) -> Retained<mtl::MTLAccelerationStructureDescriptor> {
	match geometry {
		AccelerationStructureGeometry::Triangles {
			vertex_format,
			index_type,
			triangle_count,
		} => {
			let geometry_descriptor = mtl::MTLAccelerationStructureTriangleGeometryDescriptor::descriptor();
			geometry_descriptor.setVertexFormat(vertex_format);
			geometry_descriptor.setIndexType(index_type);
			geometry_descriptor.setTriangleCount(triangle_count);
			let descriptor = mtl::MTLPrimitiveAccelerationStructureDescriptor::descriptor();
			let geometry_descriptors = NSArray::from_retained_slice(&[Retained::into_super(geometry_descriptor)]);
			descriptor.setGeometryDescriptors(Some(&geometry_descriptors));
			Retained::into_super(descriptor)
		}
		AccelerationStructureGeometry::BoundingBoxes { bounding_box_count } => {
			let geometry_descriptor = mtl::MTLAccelerationStructureBoundingBoxGeometryDescriptor::descriptor();
			geometry_descriptor.setBoundingBoxCount(bounding_box_count);
			let descriptor = mtl::MTLPrimitiveAccelerationStructureDescriptor::descriptor();
			let geometry_descriptors = NSArray::from_retained_slice(&[Retained::into_super(geometry_descriptor)]);
			descriptor.setGeometryDescriptors(Some(&geometry_descriptors));
			Retained::into_super(descriptor)
		}
		AccelerationStructureGeometry::Instances { max_instance_count } => {
			let descriptor = mtl::MTLInstanceAccelerationStructureDescriptor::descriptor();
			descriptor.setInstanceCount(max_instance_count);
			descriptor.setInstanceDescriptorType(mtl::MTLAccelerationStructureInstanceDescriptorType::Indirect);
			Retained::into_super(descriptor)
		}
	}
}

impl Context {
	/// Allocates the Metal storage for one acceleration structure and records the sizes it was built for.
	fn create_acceleration_structure(&mut self, name: Option<&str>, geometry: AccelerationStructureGeometry) -> u64 {
		assert!(
			self.device.supportsRaytracing(),
			"Metal acceleration structure creation failed. The most likely cause is that the selected device does not support ray tracing.",
		);

		let sizes = self
			.device
			.accelerationStructureSizesWithDescriptor(&sizing_descriptor(geometry));
		let structure = self
			.device
			.newAccelerationStructureWithSize(sizes.accelerationStructureSize)
			.expect("Metal acceleration structure creation failed. The most likely cause is that the device is out of memory.");

		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			if let Some(name) = name {
				structure.setLabel(Some(&NSString::from_str(name)));
			}
		}
		#[cfg(not(debug_assertions))]
		let _ = name;

		self.acceleration_structures.push(AccelerationStructure {
			structure,
			geometry,
			build_scratch_size: sizes.buildScratchBufferSize,
		});

		(self.acceleration_structures.len() - 1) as u64
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		// Instance records are written by the host and read by the acceleration-structure build, so they live in
		// shared storage rather than behind a staging copy the caller would have to synchronize.
		let buffer = self.create_buffer_resource(
			name,
			max_instance_count as usize * INSTANCE_DESCRIPTOR_SIZE,
			crate::Uses::AccelerationStructureBuild,
			crate::DeviceAccesses::HostToDevice,
		);
		let mut creator = self.buffers.creator();

		creator.add(buffer);

		creator.into()
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let index = self.create_acceleration_structure(
			name,
			AccelerationStructureGeometry::Instances {
				max_instance_count: max_instance_count as usize,
			},
		);

		graphics_hardware_interface::TopLevelAccelerationStructureHandle(index)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let geometry = match description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_position_encoding,
				triangle_count,
				index_format,
				..
			} => AccelerationStructureGeometry::Triangles {
				vertex_format: to_vertex_format(vertex_position_encoding),
				index_type: to_index_type(index_format),
				triangle_count: triangle_count as usize,
			},
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				AccelerationStructureGeometry::BoundingBoxes {
					bounding_box_count: transform_count as usize,
				}
			}
		};
		let index = self.create_acceleration_structure(None, geometry);

		graphics_hardware_interface::BottomLevelAccelerationStructureHandle(index)
	}

	pub fn write_instance(
		&mut self,
		instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		let structure = &self.acceleration_structures[acceleration_structure.0 as usize];
		// A Metal instance record names its bottom-level structure by GPU resource handle, so the structure must be
		// resident while the build reads the record; the build command marks it.
		let acceleration_structure_id = structure.structure.gpuResourceID();
		let buffer = self.buffers.get_single(instances_buffer_handle).expect(
			"Metal acceleration structure instance buffer is missing. The most likely cause is that its handle came from another context.",
		);
		let offset = instance_index * INSTANCE_DESCRIPTOR_SIZE;

		assert!(
			offset + INSTANCE_DESCRIPTOR_SIZE <= buffer.size,
			"Metal acceleration structure instance is out of bounds. The most likely cause is that the instance buffer was created for fewer instances. instance_index={instance_index}, instance_capacity={}",
			buffer.size / INSTANCE_DESCRIPTOR_SIZE,
		);

		// GHI transforms are row-major 3x4 matrices; Metal reads them as four packed float3 columns.
		let columns = std::array::from_fn(|column| mtl::MTLPackedFloat3 {
			x: transform[0][column],
			y: transform[1][column],
			z: transform[2][column],
		});
		let descriptor = mtl::MTLIndirectAccelerationStructureInstanceDescriptor {
			transformationMatrix: mtl::MTLPackedFloat4x3 { columns },
			options: mtl::MTLAccelerationStructureInstanceOptions::empty(),
			mask: mask as u32,
			intersectionFunctionTableOffset: sbt_record_offset as u32,
			userID: custom_index as u32,
			accelerationStructureID: acceleration_structure_id,
		};

		// SAFETY: The write stays inside the shared instance buffer, whose bounds were checked above.
		unsafe {
			buffer
				.pointer
				.add(offset)
				.cast::<mtl::MTLIndirectAccelerationStructureInstanceDescriptor>()
				.write_unaligned(descriptor);
		}
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: graphics_hardware_interface::PipelineHandle,
		_shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		// Metal has no shader binding table: a ray-tracing pipeline dispatches its ray-generation function directly
		// and resolves hits through the acceleration structure, so binding-table records carry no backend state.
	}
}
