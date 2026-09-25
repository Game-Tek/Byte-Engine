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

impl Context {
	/// Allocates the Metal storage for one acceleration structure and records the scratch size its builds need.
	///
	/// Metal derives storage and scratch sizes from geometry counts and formats alone, so `sizing` carries no
	/// buffers and the structure can be allocated before any build supplies geometry.
	pub(in crate::metal) fn create_acceleration_structure(
		&mut self,
		name: Option<&str>,
		sizing: &mtl::MTLAccelerationStructureDescriptor,
	) -> u64 {
		assert!(
			self.device.supportsRaytracing(),
			"Metal acceleration structure creation failed. The most likely cause is that the selected device does not support ray tracing.",
		);

		let sizes = self.device.accelerationStructureSizesWithDescriptor(sizing);
		let structure = self
			.device
			.newAccelerationStructureWithSize(sizes.accelerationStructureSize)
			.expect("Metal acceleration structure creation failed. The most likely cause is that the device is out of memory.");

		#[cfg(debug_assertions)]
		if let Some(name) = name.filter(|_| self.settings.debug_labels) {
			structure.setLabel(Some(&NSString::from_str(name)));
		}
		#[cfg(not(debug_assertions))]
		let _ = name;

		self.acceleration_structures.push(AccelerationStructure {
			structure,
			build_scratch_size: sizes.buildScratchBufferSize,
		});

		(self.acceleration_structures.len() - 1) as u64
	}
}
