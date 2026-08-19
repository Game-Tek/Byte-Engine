use super::*;

pub(crate) const SKINNING_POSITION_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_NORMAL_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_JOINTS_STRIDE: usize = std::mem::size_of::<[u16; 4]>();
pub(crate) const SKINNING_WEIGHTS_STRIDE: usize = std::mem::size_of::<[f32; 4]>();

const _: () = {
	assert!(SKINNING_POSITION_STRIDE == 12);
	assert!(SKINNING_NORMAL_STRIDE == 12);
	assert!(SKINNING_JOINTS_STRIDE == 8);
	assert!(SKINNING_WEIGHTS_STRIDE == 16);
};

/// Validates a primitive-local skinning stream against its loaded aggregate stream.
pub(crate) fn validate_skinning_primitive_stream(
	primitive: &ResourcePrimitive,
	primitive_index: usize,
	aggregate_stream: &ResourceStream,
	semantic: VertexSemantics,
	expected_stride: usize,
) -> Result<(), ()> {
	if aggregate_stream.stride != expected_stride || !aggregate_stream.size.is_multiple_of(expected_stride) {
		log::error!(
			"Skinned mesh {semantic:?} stream has invalid aggregate metadata. The most likely cause is that the mesh was baked with an incompatible vertex format; expected stride {expected_stride}, found stride {} and size {}.",
			aggregate_stream.stride,
			aggregate_stream.size
		);
		return Err(());
	}

	let Some(primitive_stream) = primitive.stream(Streams::Vertices(semantic)) else {
		log::error!(
			"Skinned primitive {primitive_index} is missing its {semantic:?} stream. The most likely cause is that the mesh was baked without complete per-primitive skinning metadata."
		);
		return Err(());
	};

	let Some(expected_size) = (primitive.vertex_count as usize).checked_mul(expected_stride) else {
		log::error!(
			"Skinned primitive {primitive_index} has invalid {semantic:?} stream metadata. The most likely cause is an overflowing vertex count in the baked mesh."
		);
		return Err(());
	};
	if primitive_stream.stride != expected_stride
		|| primitive_stream.size != expected_size
		|| !primitive_stream.offset.is_multiple_of(expected_stride)
	{
		log::error!(
			"Skinned primitive {primitive_index} has invalid {semantic:?} stream metadata. The most likely cause is that its offset, stride, or byte size does not match its {} vertices; expected vertex-aligned offset, stride {expected_stride}, and size {expected_size}, found offset {}, stride {}, and size {}.",
			primitive.vertex_count,
			primitive_stream.offset,
			primitive_stream.stride,
			primitive_stream.size
		);
		return Err(());
	}

	let Some(stream_end) = primitive_stream.offset.checked_add(primitive_stream.size) else {
		log::error!(
			"Skinned primitive {primitive_index} has an invalid {semantic:?} stream range. The most likely cause is an overflowing byte offset in the baked mesh."
		);
		return Err(());
	};
	if stream_end > aggregate_stream.size {
		log::error!(
			"Skinned primitive {primitive_index} has an out-of-bounds {semantic:?} stream range. The most likely cause is that its primitive-local range does not refer to the baked aggregate stream."
		);
		return Err(());
	}

	Ok(())
}
/// The `LoadedPrimitiveValidation` struct retains the ranges needed to validate loaded bytes without borrowing the resource.
pub(crate) struct LoadedPrimitiveValidation {
	pub(crate) vertex_count: u32,
	pub(crate) vertex_indices: Option<ResourceStream>,
	pub(crate) triangle_indices: Option<ResourceStream>,
	pub(crate) meshlets: Option<ResourceStream>,
	pub(crate) joints: Option<ResourceStream>,
	pub(crate) weights: Option<ResourceStream>,
	pub(crate) palette_len: Option<usize>,
}

/// Rejects palette-local joints that could read outside the selected skin binding.
pub(crate) fn validate_loaded_skin_joints(
	primitives: &[LoadedPrimitiveValidation],
	loaded: &resource_management::resource::ReadTargets<'_>,
) -> Result<(), ()> {
	if !primitives.iter().any(|primitive| primitive.palette_len.is_some()) {
		return Ok(());
	}
	let Some(joints) = loaded.stream("Vertex.Joints") else {
		log::error!(
			"Loaded skinned mesh data is missing its joint-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(weights) = loaded.stream("Vertex.Weights") else {
		log::error!(
			"Loaded skinned mesh data is missing its vertex-weight stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	for (primitive_index, primitive) in primitives.iter().enumerate() {
		let Some(palette_len) = primitive.palette_len else { continue };
		let Some(stream) = primitive.joints.as_ref() else {
			log::error!(
				"Skinned primitive {primitive_index} is missing its joint-index stream. The most likely cause is incomplete baked skinning metadata."
			);
			return Err(());
		};
		let Some(weight_stream) = primitive.weights.as_ref() else {
			log::error!(
				"Skinned primitive {primitive_index} is missing its vertex-weight stream. The most likely cause is incomplete baked skinning metadata."
			);
			return Err(());
		};
		let Some(stream_end) = stream.offset.checked_add(stream.size) else {
			log::error!(
				"Skinned primitive {primitive_index} joint range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(joint_bytes) = joints.buffer().get(stream.offset..stream_end) else {
			log::error!(
				"Skinned primitive {primitive_index} joint range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(weight_end) = weight_stream.offset.checked_add(weight_stream.size) else {
			log::error!(
				"Skinned primitive {primitive_index} weight range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(weight_bytes) = weights.buffer().get(weight_stream.offset..weight_end) else {
			log::error!(
				"Skinned primitive {primitive_index} weight range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		for (vertex, vertex_weights) in joint_bytes
			.chunks_exact(SKINNING_JOINTS_STRIDE)
			.zip(weight_bytes.chunks_exact(SKINNING_WEIGHTS_STRIDE))
		{
			for (lane, weight) in vertex.chunks_exact(2).zip(vertex_weights.chunks_exact(4)) {
				let joint = u16::from_ne_bytes([lane[0], lane[1]]);
				let weight = f32::from_ne_bytes([weight[0], weight[1], weight[2], weight[3]]);
				if weight > 0.0 && joint as usize >= palette_len {
					log::error!(
						"Skinned primitive {primitive_index} references joint {joint} outside its {palette_len}-matrix palette. The most likely cause is corrupted or legacy skinning data."
					);
					return Err(());
				}
			}
		}
	}
	Ok(())
}
