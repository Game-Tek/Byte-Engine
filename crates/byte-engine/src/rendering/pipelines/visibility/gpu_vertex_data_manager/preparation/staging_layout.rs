use super::*;

/// Reserves one contiguous range in an already size-checked owned backing.
pub(crate) fn take_range(cursor: &mut usize, size: usize) -> std::ops::Range<usize> {
	let start = *cursor;
	*cursor += size;
	start..*cursor
}

/// Reserves one GPU-copy-compatible aligned range in a size-checked staging lease.
pub(crate) fn take_range_aligned(cursor: &mut usize, size: usize, alignment: usize) -> std::ops::Range<usize> {
	*cursor = cursor.next_multiple_of(alignment);
	take_range(cursor, size)
}
pub(crate) const RESOURCE_MESHLET_STRIDE: usize = 52;
pub(crate) const NORMAL_F32_SOURCE_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const UV_F16_SOURCE_STRIDE: usize = std::mem::size_of::<RuntimeVertexUv>();
pub(crate) const UV_F32_SOURCE_STRIDE: usize = std::mem::size_of::<[f32; 2]>();

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UvSourceFormat {
	F16,
	F32,
}

/// Octahedrally encodes one unit vector into two UNORM16 components.
pub(crate) fn encode_octahedral_unit_vector(vector: (f32, f32, f32)) -> RuntimeUnitVector {
	let length = vector.0.abs() + vector.1.abs() + vector.2.abs();
	if !length.is_finite() || length == 0.0 {
		return [32768, 32768];
	}

	let mut x = vector.0 / length;
	let mut y = vector.1 / length;
	let z = vector.2 / length;
	if z < 0.0 {
		let previous_x = x;
		let sign_x = if previous_x < 0.0 { -1.0 } else { 1.0 };
		let sign_y = if y < 0.0 { -1.0 } else { 1.0 };
		x = (1.0 - y.abs()) * sign_x;
		y = (1.0 - previous_x.abs()) * sign_y;
	}
	[encode_unorm16(x * 0.5 + 0.5), encode_unorm16(y * 0.5 + 0.5)]
}

/// Converts vec3f normals to octahedral runtime storage in transfer staging memory.
pub(crate) fn pack_f32_normals(source: &[u8], destination: &mut [u8], vertex_count: usize) {
	for index in 0..vertex_count {
		let offset = index * NORMAL_F32_SOURCE_STRIDE;
		let component = |component_offset| {
			f32::from_ne_bytes(
				source[offset + component_offset..offset + component_offset + 4]
					.try_into()
					.expect("A validated f32 normal component is four bytes."),
			)
		};
		let encoded = encode_octahedral_unit_vector((component(0), component(4), component(8)));
		let destination_offset = index * VERTEX_NORMAL_BUFFER_STRIDE as usize;
		destination[destination_offset..destination_offset + 2].copy_from_slice(&encoded[0].to_ne_bytes());
		destination[destination_offset + 2..destination_offset + 4].copy_from_slice(&encoded[1].to_ne_bytes());
	}
}

/// Encodes one UV component using the visibility pipeline's UNORM conversion policy.
fn encode_unorm16(value: f32) -> u16 {
	(value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

/// Converts an f32 UV stream to half-float transfer storage without clamping sampler coordinates.
pub(crate) fn pack_f32_uvs(source: &[u8], destination: &mut [u8], vertex_count: usize) {
	for index in 0..vertex_count {
		let source_offset = index * UV_F32_SOURCE_STRIDE;
		let destination_offset = index * UV_F16_SOURCE_STRIDE;
		let u = f32::from_ne_bytes(
			source[source_offset..source_offset + 4]
				.try_into()
				.expect("A validated f32 UV is four bytes."),
		);
		let v = f32::from_ne_bytes(
			source[source_offset + 4..source_offset + 8]
				.try_into()
				.expect("A validated f32 UV is four bytes."),
		);
		destination[destination_offset..destination_offset + 2]
			.copy_from_slice(&half::f16::from_f32(u).to_bits().to_ne_bytes());
		destination[destination_offset + 2..destination_offset + 4]
			.copy_from_slice(&half::f16::from_f32(v).to_bits().to_ne_bytes());
	}
}
