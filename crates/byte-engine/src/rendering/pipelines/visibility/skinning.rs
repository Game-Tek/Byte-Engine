//! Frame-local GPU skinning: deforms bind-pose vertices with matrix or dual-quaternion palettes.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use resource_management::resources::skeleton::AffineMatrix4x3Columns;
use utils::Extent;

use super::geometry::GeometryHandles;
use crate::rendering::PipelineManagerClient;

const WORKGROUP_SIZE: u32 = 64;
pub(crate) const MAX_SKINNED_VERTICES: usize = 65_536 * 4;
pub(crate) const MAX_SKINNING_MATRICES: usize = 65_536;

const fn buffer(slot: u32, access: ghi::AccessPolicies, stride: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(ghi::ResourceSlot::new(slot), ghi::ResourceKind::StorageBuffer, access)
		.buffer_stride(stride)
}
const SOURCE_POSITIONS_BINDING: ghi::ShaderResourceDescriptor = buffer(0, ghi::AccessPolicies::READ, 12);
const SOURCE_NORMALS_BINDING: ghi::ShaderResourceDescriptor = buffer(1, ghi::AccessPolicies::READ, 12);
const SOURCE_JOINTS_BINDING: ghi::ShaderResourceDescriptor = buffer(2, ghi::AccessPolicies::READ, 8);
const SOURCE_WEIGHTS_BINDING: ghi::ShaderResourceDescriptor = buffer(3, ghi::AccessPolicies::READ, 16);
const MATRIX_PALETTE_BINDING: ghi::ShaderResourceDescriptor = buffer(4, ghi::AccessPolicies::READ, 48);
const SKINNED_VERTICES_BINDING: ghi::ShaderResourceDescriptor = buffer(5, ghi::AccessPolicies::WRITE, 32);
const DUAL_QUATERNION_PALETTE_BINDING: ghi::ShaderResourceDescriptor = buffer(6, ghi::AccessPolicies::READ, 32);

/// The `SkinnedVertex` struct is one aligned position-and-normal record read by every visibility stage after deformation.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkinnedVertex {
	pub(crate) position: [f32; 4],
	pub(crate) normal: [f32; 4],
}

/// The `DualQuaternion` struct is the aligned rigid-transform palette layout consumed by GPU skinning.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DualQuaternion {
	pub(crate) real: [f32; 4],
	pub(crate) dual: [f32; 4],
}

/// The `SkinningPaletteKind` enum selects the palette representation used by one skinning dispatch.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SkinningPaletteKind {
	#[default]
	Matrix = 0,
	DualQuaternion = 1,
}

/// The `SkinningDispatch` struct identifies one active primitive instance and its palette range; it is also the push constant.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkinningDispatch {
	pub(crate) source_vertex_base: u32,
	pub(crate) destination_vertex_base: u32,
	pub(crate) palette_base: u32,
	pub(crate) palette_count: u32,
	pub(crate) vertex_count: u32,
	pub(crate) palette_kind: u32,
}

/// The `SkinningPass` struct owns the frame-local palettes and deformed-vertex output every sink reads.
pub(crate) struct SkinningPass {
	pipeline: crate::rendering::PipelineRef,
	descriptor_set: ghi::DescriptorSetHandle,
	matrix_palette_buffer: ghi::DynamicBufferHandle<[AffineMatrix4x3Columns; MAX_SKINNING_MATRICES]>,
	dual_quaternion_palette_buffer: ghi::DynamicBufferHandle<[DualQuaternion; MAX_SKINNING_MATRICES]>,
	skinned_vertices_buffer: ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]>,
}

impl SkinningPass {
	/// Creates the frame-local buffers over the resident bind-pose sources and requests the skinning pipeline.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		sources: GeometryHandles,
	) -> Self {
		let dynamic = |name, accesses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(accesses)
		};
		let matrix_palette_buffer = context.build_dynamic_buffer(dynamic(
			"Visibility Skinning Matrix Palette",
			ghi::DeviceAccesses::HostToDevice,
		));
		let dual_quaternion_palette_buffer = context.build_dynamic_buffer(dynamic(
			"Visibility Skinning Dual Quaternion Palette",
			ghi::DeviceAccesses::HostToDevice,
		));
		let skinned_vertices_buffer =
			context.build_dynamic_buffer(dynamic("Visibility Skinned Vertices", ghi::DeviceAccesses::DeviceOnly));
		let descriptor_set = context.create_descriptor_set(Some("Visibility Skinning Compute Set"));
		let write = |binding: ghi::ShaderResourceDescriptor, buffer| {
			ghi::DescriptorWrite::buffer(descriptor_set, binding.slot(), buffer)
		};
		context.write(&[
			write(SOURCE_POSITIONS_BINDING, sources.skinning_rest_positions.into()),
			write(SOURCE_NORMALS_BINDING, sources.skinning_rest_normals.into()),
			write(SOURCE_JOINTS_BINDING, sources.skinning_joints.into()),
			write(SOURCE_WEIGHTS_BINDING, sources.skinning_weights.into()),
			write(MATRIX_PALETTE_BINDING, matrix_palette_buffer.into()),
			write(DUAL_QUATERNION_PALETTE_BINDING, dual_quaternion_palette_buffer.into()),
			write(SKINNED_VERTICES_BINDING, skinned_vertices_buffer.into()),
		]);
		Self {
			pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/skinning.pipeline"),
			descriptor_set,
			matrix_palette_buffer,
			dual_quaternion_palette_buffer,
			skinned_vertices_buffer,
		}
	}

	pub(crate) const fn skinned_vertices_buffer(&self) -> ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]> {
		self.skinned_vertices_buffer
	}

	pub(crate) const fn pipeline(&self) -> crate::rendering::PipelineRef {
		self.pipeline
	}

	/// Uploads this frame's palettes. Empty palettes skip the sync.
	pub(crate) fn write_palettes(
		&self,
		frame: &mut ghi::implementation::Frame,
		matrices: &[AffineMatrix4x3Columns],
		dual_quaternions: &[DualQuaternion],
	) {
		if !matrices.is_empty() {
			frame.get_mut_dynamic_buffer_slice(self.matrix_palette_buffer)[..matrices.len()].copy_from_slice(matrices);
			frame.sync_buffer(self.matrix_palette_buffer);
		}
		if !dual_quaternions.is_empty() {
			frame.get_mut_dynamic_buffer_slice(self.dual_quaternion_palette_buffer)[..dual_quaternions.len()]
				.copy_from_slice(dual_quaternions);
			frame.sync_buffer(self.dual_quaternion_palette_buffer);
		}
	}

	/// Dispatches one workgroup grid per active skinned primitive.
	pub(crate) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		dispatches: &[SkinningDispatch],
		pipeline: ghi::PipelineHandle,
	) {
		use ghi::command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _};

		if dispatches.is_empty() {
			return;
		}
		let c = c.bind_compute_pipeline(pipeline);
		c.bind_descriptor_sets(&[self.descriptor_set]);
		for dispatch in dispatches.iter().filter(|dispatch| dispatch.vertex_count != 0) {
			debug_assert!(
				(dispatch.source_vertex_base + dispatch.vertex_count) as usize <= MAX_SKINNED_VERTICES
					&& (dispatch.destination_vertex_base + dispatch.vertex_count) as usize <= MAX_SKINNED_VERTICES
					&& (dispatch.palette_base + dispatch.palette_count) as usize <= MAX_SKINNING_MATRICES,
				"Skinning dispatch range exceeds its buffer. The most likely cause is corrupted primitive or skin metadata."
			);
			c.write_push_constant(0, *dispatch);
			c.dispatch(ghi::DispatchExtent::new(
				Extent::line(dispatch.vertex_count),
				Extent::line(WORKGROUP_SIZE),
			));
		}
	}
}

const RIGID_TRANSFORM_EPSILON: f32 = 1.0e-4;

/// Appends a dual-quaternion palette when every matrix is a finite proper rigid transform; otherwise appends nothing.
pub(crate) fn append_dual_quaternion_palette(matrices: &[AffineMatrix4x3Columns], output: &mut Vec<DualQuaternion>) -> bool {
	if !matrices.iter().all(is_rigid_transform) {
		return false;
	}
	output.extend(matrices.iter().map(dual_quaternion_from_rigid_transform));
	true
}

/// Checks the orthonormal basis and positive determinant required by a dual quaternion.
fn is_rigid_transform(matrix: &AffineMatrix4x3Columns) -> bool {
	if !matrix.iter().flatten().all(|value| value.is_finite()) {
		return false;
	}
	let [x, y, z, _] = matrix;
	let unit = |axis: &[f32; 3]| (dot3(*axis, *axis) - 1.0).abs() <= RIGID_TRANSFORM_EPSILON;
	let orthogonal = |left: &[f32; 3], right: &[f32; 3]| dot3(*left, *right).abs() <= RIGID_TRANSFORM_EPSILON;
	unit(x)
		&& unit(y)
		&& unit(z)
		&& orthogonal(x, y)
		&& orthogonal(x, z)
		&& orthogonal(y, z)
		&& (dot3(*x, cross3(*y, *z)) - 1.0).abs() <= RIGID_TRANSFORM_EPSILON
}

/// Converts one validated rigid matrix into the engine's xyzw dual-quaternion convention.
fn dual_quaternion_from_rigid_transform(matrix: &AffineMatrix4x3Columns) -> DualQuaternion {
	let real = quaternion_from_rotation_columns(matrix[0], matrix[1], matrix[2]);
	let translation = [matrix[3][0], matrix[3][1], matrix[3][2], 0.0];
	DualQuaternion {
		real,
		dual: quaternion_multiply(translation, real).map(|component| component * 0.5),
	}
}

/// Extracts a normalized xyzw quaternion from orthonormal rotation columns.
fn quaternion_from_rotation_columns(column0: [f32; 3], column1: [f32; 3], column2: [f32; 3]) -> [f32; 4] {
	let [m00, m10, m20] = column0;
	let [m01, m11, m21] = column1;
	let [m02, m12, m22] = column2;
	let trace = m00 + m11 + m22;
	let quaternion = if trace > 0.0 {
		let scale = (trace + 1.0).sqrt() * 2.0;
		[(m21 - m12) / scale, (m02 - m20) / scale, (m10 - m01) / scale, scale * 0.25]
	} else if m00 > m11 && m00 > m22 {
		let scale = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
		[scale * 0.25, (m01 + m10) / scale, (m02 + m20) / scale, (m21 - m12) / scale]
	} else if m11 > m22 {
		let scale = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
		[(m01 + m10) / scale, scale * 0.25, (m12 + m21) / scale, (m02 - m20) / scale]
	} else {
		let scale = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
		[(m02 + m20) / scale, (m12 + m21) / scale, scale * 0.25, (m10 - m01) / scale]
	};
	let inverse_length = dot4(quaternion, quaternion).sqrt().recip();
	quaternion.map(|component| component * inverse_length)
}

fn quaternion_multiply(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
	let [lx, ly, lz, lw] = left;
	let [rx, ry, rz, rw] = right;
	[
		lw * rx + lx * rw + ly * rz - lz * ry,
		lw * ry - lx * rz + ly * rw + lz * rx,
		lw * rz + lx * ry - ly * rx + lz * rw,
		lw * rw - lx * rx - ly * ry - lz * rz,
	]
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
	left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn dot4(left: [f32; 4], right: [f32; 4]) -> f32 {
	left[0] * right[0] + left[1] * right[1] + left[2] * right[2] + left[3] * right[3]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};

	use super::*;
	use crate::rendering::shader_vm_test::{buffer, compile, push_constant_buffer, run_at};

	/// Parses and links the exact checked-in shader consumed by the runtime resource path.
	fn production_skinning_main() -> besl::NodeReference {
		let source = include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/skinning.besl"
		));
		besl::compile_to_besl(source, None)
			.expect(
				"Failed to compile the checked-in visibility skinning BESL. The most likely cause is invalid production shader syntax.",
			)
			.get_main()
			.expect(
				"Missing visibility skinning entry point. The most likely cause is that the checked-in shader does not define main.",
			)
	}

	/// Binds every skinning slot and runs one lane at the origin.
	fn run_skinning(program: &besl::vm::ExecutableProgram, buffers: &mut [Buffer; 7], push_constant: &mut Buffer) {
		let mut descriptors = DescriptorBindings::new();
		for (slot, buffer) in buffers.iter_mut().enumerate() {
			descriptors.bind_buffer(ResourceSlot::new(slot as u32), buffer);
		}
		descriptors.bind_push_constant(push_constant);
		run_at(program, &mut descriptors, [0, 0]);
	}

	fn write_dispatch(push_constant: &mut Buffer, dispatch: SkinningDispatch) {
		for (field, value) in [
			("source_vertex_base", dispatch.source_vertex_base),
			("destination_vertex_base", dispatch.destination_vertex_base),
			("palette_base", dispatch.palette_base),
			("palette_count", dispatch.palette_count),
			("vertex_count", dispatch.vertex_count),
			("palette_kind", dispatch.palette_kind),
		] {
			push_constant
				.write(field, Value::U32(value))
				.expect("Failed to write skinning push constant.");
		}
	}

	fn read_vec4(buffer: &Buffer, index: usize, field: &str) -> [f32; 4] {
		match buffer
			.read_indexed_field("values", index, field)
			.expect("Missing skinned output.")
		{
			Value::Vec4F(value) => value,
			_ => panic!("Skinning wrote a non-vector {field}."),
		}
	}

	#[test]
	fn skinning_host_types_match_besl_buffer_layouts() {
		assert_eq!(std::mem::size_of::<[u16; 4]>(), 8);
		assert_eq!(std::mem::size_of::<AffineMatrix4x3Columns>(), 48);
		assert_eq!(MATRIX_PALETTE_BINDING.buffer_element_stride(), 48);
		assert_eq!(std::mem::size_of::<DualQuaternion>(), 32);
		assert_eq!(std::mem::align_of::<DualQuaternion>(), 16);
		assert_eq!(DUAL_QUATERNION_PALETTE_BINDING.buffer_element_stride(), 32);
		assert_eq!(std::mem::size_of::<SkinnedVertex>(), 32);
		assert_eq!(std::mem::align_of::<SkinnedVertex>(), 16);
		assert_eq!(std::mem::size_of::<SkinningDispatch>(), 24);
	}

	#[test]
	fn rigid_palette_conversion_preserves_rotation_and_translation() {
		let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
		let matrix = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [2.0, 4.0, 6.0]];
		let mut output = Vec::new();

		assert!(append_dual_quaternion_palette(&[matrix], &mut output));
		assert_eq!(output.len(), 1);
		for (actual, expected) in output[0].real.into_iter().zip([0.0, 0.0, half_sqrt, half_sqrt]) {
			math::assert_float_eq!(actual, expected);
		}
		let recovered_translation = quaternion_multiply(
			output[0].dual.map(|component| component * 2.0),
			[-output[0].real[0], -output[0].real[1], -output[0].real[2], output[0].real[3]],
		);
		for (actual, expected) in recovered_translation[..3].iter().zip(matrix[3]) {
			math::assert_float_eq!(*actual, expected);
		}
	}

	#[test]
	fn dual_quaternion_palette_rejects_every_non_rigid_affine_form_without_partial_output() {
		let identity = resource_management::resources::skeleton::identity_affine_matrix4x3_columns();
		let mut scaled = identity;
		scaled[0][0] = 2.0;
		let mut sheared = identity;
		sheared[1][0] = 0.25;
		let mut reflected = identity;
		reflected[0][0] = -1.0;
		let mut non_finite = identity;
		non_finite[3][0] = f32::NAN;

		for matrix in [scaled, sheared, reflected, non_finite] {
			let mut output = vec![DualQuaternion::default()];
			assert!(!append_dual_quaternion_palette(&[identity, matrix], &mut output));
			assert_eq!(output, vec![DualQuaternion::default()]);
		}
	}

	/// Executes the production skinning semantics with two weighted joints and checks the deformed vertex.
	#[test]
	fn skinning_besl_vm_blends_joint_matrices_and_writes_position_and_normal() {
		let program = compile(production_skinning_main());
		let mut buffers: [Buffer; 7] = std::array::from_fn(|slot| buffer(&program, ResourceSlot::new(slot as u32)));
		let mut push_constant = push_constant_buffer(&program);
		let [positions, normals, joints, weights, palette, ..] = &mut buffers;
		positions
			.write_indexed("values", 1, Value::Vec3F([1.0, 1.0, 1.0]))
			.expect("source position");
		normals
			.write_indexed("values", 1, Value::Vec3F([0.0, 0.0, 1.0]))
			.expect("source normal");
		joints
			.write_indexed("values", 1, Value::Vec4U16([0, 1, 0, 0]))
			.expect("source joints");
		weights
			.write_indexed("values", 1, Value::Vec4F([0.5, 0.5, 0.0, 0.0]))
			.expect("source weights");
		write_translation_matrix(palette, 1, [2.0, 0.0, 0.0]);
		write_translation_matrix(palette, 2, [0.0, 4.0, 0.0]);
		write_dispatch(
			&mut push_constant,
			SkinningDispatch {
				source_vertex_base: 1,
				destination_vertex_base: 2,
				palette_base: 1,
				palette_count: 2,
				vertex_count: 1,
				palette_kind: SkinningPaletteKind::Matrix as u32,
			},
		);

		run_skinning(&program, &mut buffers, &mut push_constant);

		assert_eq!(read_vec4(&buffers[5], 2, "position"), [2.0, 3.0, 1.0, 1.0]);
		assert_eq!(read_vec4(&buffers[5], 2, "normal"), [0.0, 0.0, 1.0, 0.0]);

		// A malformed legacy joint must produce legal bind-pose output without indexing beyond the palette.
		buffers[2]
			.write_indexed("values", 1, Value::Vec4U16([2, 0, 0, 0]))
			.expect("out-of-range source joint");
		push_constant
			.write("destination_vertex_base", Value::U32(3))
			.expect("skinning destination");
		run_skinning(&program, &mut buffers, &mut push_constant);

		assert_eq!(read_vec4(&buffers[5], 3, "position"), [1.0, 1.0, 1.0, 1.0]);
	}

	/// Demonstrates that rigid dual-quaternion blending preserves radius across an opposing joint twist.
	#[test]
	fn skinning_besl_vm_dual_quaternions_preserve_twist_volume_and_handle_antipodality() {
		let program = compile(production_skinning_main());
		let mut buffers: [Buffer; 7] = std::array::from_fn(|slot| buffer(&program, ResourceSlot::new(slot as u32)));
		let mut push_constant = push_constant_buffer(&program);
		let [positions, normals, joints, weights, _, _, dual_quaternion_palette] = &mut buffers;
		positions
			.write_indexed("values", 0, Value::Vec3F([0.0, 1.0, 0.0]))
			.expect("twist source position");
		normals
			.write_indexed("values", 0, Value::Vec3F([0.0, 1.0, 0.0]))
			.expect("twist source normal");
		joints
			.write_indexed("values", 0, Value::Vec4U16([0, 1, u16::MAX, u16::MAX]))
			.expect("twist joints");
		weights
			.write_indexed("values", 0, Value::Vec4F([2.0, 2.0, 0.0, 0.0]))
			.expect("twist weights");
		let sine = 3.0_f32.sqrt() * 0.5;
		let cosine = 0.5;
		let positive = dual_quaternion_from_rigid_transform(&[
			[1.0, 0.0, 0.0],
			[0.0, cosine, sine],
			[0.0, -sine, cosine],
			[2.0, 0.0, 0.0],
		]);
		write_dual_quaternion(dual_quaternion_palette, 0, positive.real, positive.dual);
		// Negate the equivalent -60-degree transform to exercise per-vertex antipodality correction too.
		let negative = dual_quaternion_from_rigid_transform(&[
			[1.0, 0.0, 0.0],
			[0.0, cosine, -sine],
			[0.0, sine, cosine],
			[2.0, 0.0, 0.0],
		]);
		write_dual_quaternion(
			dual_quaternion_palette,
			1,
			negative.real.map(|component| -component),
			negative.dual.map(|component| -component),
		);
		write_dispatch(
			&mut push_constant,
			SkinningDispatch {
				source_vertex_base: 0,
				destination_vertex_base: 0,
				palette_base: 0,
				palette_count: 2,
				vertex_count: 1,
				palette_kind: SkinningPaletteKind::DualQuaternion as u32,
			},
		);

		run_skinning(&program, &mut buffers, &mut push_constant);

		for (actual, expected) in read_vec4(&buffers[5], 0, "position").into_iter().zip([2.0, 1.0, 0.0, 1.0]) {
			math::assert_float_eq!(actual, expected);
		}
		for (actual, expected) in read_vec4(&buffers[5], 0, "normal").into_iter().zip([0.0, 1.0, 0.0, 0.0]) {
			math::assert_float_eq!(actual, expected);
		}
	}

	/// Writes one column-major translation matrix into the compact VM palette fixture.
	fn write_translation_matrix(palette: &mut Buffer, index: usize, translation: [f32; 3]) {
		let [x, y, z] = translation;
		palette
			.write_indexed(
				"values",
				index,
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, y, z]),
			)
			.expect("Failed to write skinning matrix.");
	}

	fn write_dual_quaternion(palette: &mut Buffer, index: usize, real: [f32; 4], dual: [f32; 4]) {
		palette
			.write_indexed_field("values", index, "real", Value::Vec4F(real))
			.expect("Failed to write skinning dual-quaternion real part.");
		palette
			.write_indexed_field("values", index, "dual", Value::Vec4F(dual))
			.expect("Failed to write skinning dual-quaternion dual part.");
	}
}
