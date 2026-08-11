use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use resource_management::resources::skeleton::AffineMatrix4x3Columns;
use utils::Extent;

pub(crate) const SKINNING_WORKGROUP_SIZE: u32 = 64;
pub(crate) const MAX_SKINNED_VERTICES: usize = 65_536 * 4;
pub(crate) const MAX_SKINNING_MATRICES: usize = 65_536;

pub(crate) const SOURCE_POSITIONS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(12);
pub(crate) const SOURCE_NORMALS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(12);
pub(crate) const SOURCE_JOINTS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(2),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(8);
pub(crate) const SOURCE_WEIGHTS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(3),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(16);
pub(crate) const MATRIX_PALETTE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(4),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(48);
pub(crate) const DUAL_QUATERNION_PALETTE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(6),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(32);
pub(crate) const SKINNED_VERTICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(5),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
)
.buffer_stride(32);

/// The `SkinnedVertex` struct provides one aligned position-and-normal record for all visibility rendering stages.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SkinnedVertex {
	pub(crate) position: [f32; 4],
	pub(crate) normal: [f32; 4],
}

/// The `DualQuaternion` struct provides the aligned rigid-transform palette layout consumed by GPU skinning.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

/// The `SkinningSourceBuffers` struct groups immutable bind-pose attributes consumed by the GPU skinning pass.
#[derive(Clone, Copy)]
pub(crate) struct SkinningSourceBuffers {
	pub(crate) positions: ghi::BaseBufferHandle,
	pub(crate) normals: ghi::BaseBufferHandle,
	pub(crate) joints: ghi::BaseBufferHandle,
	pub(crate) weights: ghi::BaseBufferHandle,
}

impl SkinningSourceBuffers {
	pub(crate) const fn new(
		positions: ghi::BaseBufferHandle,
		normals: ghi::BaseBufferHandle,
		joints: ghi::BaseBufferHandle,
		weights: ghi::BaseBufferHandle,
	) -> Self {
		Self {
			positions,
			normals,
			joints,
			weights,
		}
	}
}

/// The `SkinningDispatch` struct identifies one active primitive instance and its palette range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SkinningDispatch {
	pub(crate) source_vertex_base: u32,
	pub(crate) destination_vertex_base: u32,
	pub(crate) palette_base: u32,
	pub(crate) palette_count: u32,
	pub(crate) vertex_count: u32,
	pub(crate) palette_kind: u32,
}

impl SkinningDispatch {
	pub(crate) const fn new(
		source_vertex_base: u32,
		destination_vertex_base: u32,
		palette_base: u32,
		palette_count: u32,
		vertex_count: u32,
		palette_kind: SkinningPaletteKind,
	) -> Self {
		Self {
			source_vertex_base,
			destination_vertex_base,
			palette_base,
			palette_count,
			vertex_count,
			palette_kind: palette_kind as u32,
		}
	}
}

/// The `SkinningPass` struct owns frame-local animation outputs and the compute state that populates them before visibility rendering.
pub(crate) struct SkinningPass {
	pipeline: crate::rendering::PipelineRef,
	descriptor_set: ghi::DescriptorSetHandle,
	matrix_palette_buffer: ghi::DynamicBufferHandle<[AffineMatrix4x3Columns; MAX_SKINNING_MATRICES]>,
	dual_quaternion_palette_buffer: ghi::DynamicBufferHandle<[DualQuaternion; MAX_SKINNING_MATRICES]>,
	skinned_vertices_buffer: ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]>,
}

impl SkinningPass {
	/// Creates frame-local buffers and requests the skinning pipeline.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		sources: SkinningSourceBuffers,
	) -> Self {
		let matrix_palette_buffer = context.build_dynamic_buffer::<[AffineMatrix4x3Columns; MAX_SKINNING_MATRICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Matrix Palette")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let dual_quaternion_palette_buffer = context.build_dynamic_buffer::<[DualQuaternion; MAX_SKINNING_MATRICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Dual Quaternion Palette")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let skinned_vertices_buffer = context.build_dynamic_buffer::<[SkinnedVertex; MAX_SKINNED_VERTICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinned Vertices")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let descriptor_set = context.create_descriptor_set(Some("Visibility Skinning Compute Set"));
		let writes = [
			ghi::DescriptorWrite::buffer(descriptor_set, SOURCE_POSITIONS_BINDING.slot(), sources.positions),
			ghi::DescriptorWrite::buffer(descriptor_set, SOURCE_NORMALS_BINDING.slot(), sources.normals),
			ghi::DescriptorWrite::buffer(descriptor_set, SOURCE_JOINTS_BINDING.slot(), sources.joints),
			ghi::DescriptorWrite::buffer(descriptor_set, SOURCE_WEIGHTS_BINDING.slot(), sources.weights),
			ghi::DescriptorWrite::buffer(descriptor_set, MATRIX_PALETTE_BINDING.slot(), matrix_palette_buffer.into()),
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				DUAL_QUATERNION_PALETTE_BINDING.slot(),
				dual_quaternion_palette_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				descriptor_set,
				SKINNED_VERTICES_BINDING.slot(),
				skinned_vertices_buffer.into(),
			),
		];
		context.write(&writes);

		let pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/skinning.pipeline");

		Self {
			pipeline,
			descriptor_set,
			matrix_palette_buffer,
			dual_quaternion_palette_buffer,
			skinned_vertices_buffer,
		}
	}

	pub(crate) const fn matrix_palette_buffer(
		&self,
	) -> ghi::DynamicBufferHandle<[AffineMatrix4x3Columns; MAX_SKINNING_MATRICES]> {
		self.matrix_palette_buffer
	}

	pub(crate) const fn skinned_vertices_buffer(&self) -> ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]> {
		self.skinned_vertices_buffer
	}

	/// Returns the asynchronously compiled pipeline required by this pass.
	pub(crate) const fn pipeline(&self) -> crate::rendering::PipelineRef {
		self.pipeline
	}

	/// Copies a complete caller-produced palette into the active frame without allocating intermediate storage.
	pub(crate) fn write_matrix_palette(&self, frame: &mut ghi::implementation::Frame, matrices: &[AffineMatrix4x3Columns]) {
		assert!(
			matrices.len() <= MAX_SKINNING_MATRICES,
			"Skinning matrix palette exceeds capacity. The most likely cause is that active skins require more than {MAX_SKINNING_MATRICES} matrices."
		);
		if matrices.is_empty() {
			return;
		}

		frame.get_mut_dynamic_buffer_slice(self.matrix_palette_buffer)[..matrices.len()].copy_from_slice(matrices);
		frame.sync_buffer(self.matrix_palette_buffer);
	}

	/// Copies a complete rigid-transform palette into the active frame without allocating intermediate storage.
	pub(crate) fn write_dual_quaternion_palette(
		&self,
		frame: &mut ghi::implementation::Frame,
		dual_quaternions: &[DualQuaternion],
	) {
		assert!(
			dual_quaternions.len() <= MAX_SKINNING_MATRICES,
			"Skinning dual-quaternion palette exceeds capacity. The most likely cause is that active skins require more than {MAX_SKINNING_MATRICES} rigid transforms."
		);
		if dual_quaternions.is_empty() {
			return;
		}

		frame.get_mut_dynamic_buffer_slice(self.dual_quaternion_palette_buffer)[..dual_quaternions.len()]
			.copy_from_slice(dual_quaternions);
		frame.sync_buffer(self.dual_quaternion_palette_buffer);
	}

	/// Dispatches one workgroup grid per active skinned primitive while retaining all job storage at the caller.
	pub(crate) fn record(
		&self,
		command_buffer: &mut ghi::implementation::CommandBufferRecording,
		dispatches: &[SkinningDispatch],
		pipeline: ghi::PipelineHandle,
	) {
		if dispatches.is_empty() {
			return;
		}

		let command = command_buffer.bind_compute_pipeline(pipeline);
		command.bind_descriptor_sets(&[self.descriptor_set]);
		for dispatch in dispatches.iter().copied().filter(|dispatch| dispatch.vertex_count != 0) {
			let source_end = (dispatch.source_vertex_base as usize)
				.checked_add(dispatch.vertex_count as usize)
				.expect("Skinning source range overflows. The most likely cause is corrupted primitive metadata.");
			let destination_end = (dispatch.destination_vertex_base as usize)
				.checked_add(dispatch.vertex_count as usize)
				.expect("Skinning destination range overflows. The most likely cause is an invalid frame-local allocation.");
			let palette_end = (dispatch.palette_base as usize)
				.checked_add(dispatch.palette_count as usize)
				.expect("Skinning palette range overflows. The most likely cause is a corrupted skin binding.");
			assert!(
				source_end <= MAX_SKINNED_VERTICES,
				"Skinning source range exceeds its buffer. The most likely cause is corrupted primitive vertex metadata."
			);
			assert!(
				destination_end <= MAX_SKINNED_VERTICES,
				"Skinning destination range exceeds its buffer. The most likely cause is an invalid frame-local allocation."
			);
			assert!(
				palette_end <= MAX_SKINNING_MATRICES,
				"Skinning palette range exceeds its buffer. The most likely cause is a corrupted skin binding."
			);
			command.write_push_constant(0, dispatch);
			command.dispatch(ghi::DispatchExtent::new(
				Extent::line(dispatch.vertex_count),
				Extent::line(SKINNING_WORKGROUP_SIZE),
			));
		}
	}
}

const RIGID_TRANSFORM_EPSILON: f32 = 1.0e-4;

/// Appends a dual-quaternion palette when every matrix represents a finite proper rigid transform.
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
	let squared_length = |value: &[f32; 3]| dot3(*value, *value);
	let determinant = dot3(*x, cross3(*y, *z));

	(squared_length(x) - 1.0).abs() <= RIGID_TRANSFORM_EPSILON
		&& (squared_length(y) - 1.0).abs() <= RIGID_TRANSFORM_EPSILON
		&& (squared_length(z) - 1.0).abs() <= RIGID_TRANSFORM_EPSILON
		&& dot3(*x, *y).abs() <= RIGID_TRANSFORM_EPSILON
		&& dot3(*x, *z).abs() <= RIGID_TRANSFORM_EPSILON
		&& dot3(*y, *z).abs() <= RIGID_TRANSFORM_EPSILON
		&& (determinant - 1.0).abs() <= RIGID_TRANSFORM_EPSILON
}

/// Converts one validated rigid matrix into the engine's xyzw dual-quaternion convention.
fn dual_quaternion_from_rigid_transform(matrix: &AffineMatrix4x3Columns) -> DualQuaternion {
	let real = quaternion_from_rotation_columns(matrix[0], matrix[1], matrix[2]);
	let translation = [matrix[3][0], matrix[3][1], matrix[3][2], 0.0];
	let mut dual = quaternion_multiply(translation, real);
	for component in &mut dual {
		*component *= 0.5;
	}
	DualQuaternion { real, dual }
}

/// Extracts a normalized xyzw quaternion from orthonormal rotation columns.
fn quaternion_from_rotation_columns(column0: [f32; 3], column1: [f32; 3], column2: [f32; 3]) -> [f32; 4] {
	let m00 = column0[0];
	let m01 = column1[0];
	let m02 = column2[0];
	let m10 = column0[1];
	let m11 = column1[1];
	let m12 = column2[1];
	let m20 = column0[2];
	let m21 = column1[2];
	let m22 = column2[2];
	let trace = m00 + m11 + m22;

	let mut quaternion = if trace > 0.0 {
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
	for component in &mut quaternion {
		*component *= inverse_length;
	}
	quaternion
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
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::{ShaderGenerationSettings, ShaderGenerator},
	};

	use super::*;
	use crate::rendering::shader_vm_test::{buffer, compile, push_constant_buffer, run_at};

	/// Parses and links the exact checked-in shader consumed by the runtime resource path.
	fn production_skinning_main() -> besl::NodeReference {
		let source = include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/skinning.besl"
		));
		let program = besl::compile_to_besl(source, None).expect(
			"Failed to compile the checked-in visibility skinning BESL. The most likely cause is invalid production shader syntax.",
		);
		program.get_main().expect(
			"Missing visibility skinning entry point. The most likely cause is that the checked-in shader does not define main.",
		)
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

	/// Verifies the production dual-quaternion path lowers across every supported BESL backend.
	#[compio::test]
	async fn skinning_besl_lowers_for_all_backends_and_compiles_with_metal() {
		let main = production_skinning_main();
		let settings = ShaderGenerationSettings::compute(Extent::line(SKINNING_WORKGROUP_SIZE));
		GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Visibility skinning should lower to GLSL.");
		HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Visibility skinning should lower to HLSL.");
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Visibility skinning should lower to MSL.");
		assert!(msl.contains("struct DualQuaternion"));

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-dual-quaternion-skinning",
		)
		.await
		.expect("Visibility dual-quaternion skinning should compile with Metal.");
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
		let mut positions = buffer(&program, ResourceSlot::new(0));
		let mut normals = buffer(&program, ResourceSlot::new(1));
		let mut joints = buffer(&program, ResourceSlot::new(2));
		let mut weights = buffer(&program, ResourceSlot::new(3));
		let mut palette = buffer(&program, ResourceSlot::new(4));
		let mut output = buffer(&program, ResourceSlot::new(5));
		let mut dual_quaternion_palette = buffer(&program, ResourceSlot::new(6));
		let mut push_constant = push_constant_buffer(&program);

		positions
			.write_indexed("values", 1, Value::Vec3F([1.0, 1.0, 1.0]))
			.expect("Failed to write source position.");
		normals
			.write_indexed("values", 1, Value::Vec3F([0.0, 0.0, 1.0]))
			.expect("Failed to write source normal.");
		joints
			.write_indexed("values", 1, Value::Vec4U16([0, 1, 0, 0]))
			.expect("Failed to write source joints.");
		weights
			.write_indexed("values", 1, Value::Vec4F([0.5, 0.5, 0.0, 0.0]))
			.expect("Failed to write source weights.");
		write_translation_matrix(&mut palette, 1, [2.0, 0.0, 0.0]);
		write_translation_matrix(&mut palette, 2, [0.0, 4.0, 0.0]);

		for (field, value) in [
			("source_vertex_base", 1),
			("destination_vertex_base", 2),
			("palette_base", 1),
			("palette_count", 2),
			("vertex_count", 1),
			("palette_kind", SkinningPaletteKind::Matrix as u32),
		] {
			push_constant
				.write(field, Value::U32(value))
				.expect("Failed to write skinning push constant.");
		}

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(ResourceSlot::new(0), &mut positions);
			descriptors.bind_buffer(ResourceSlot::new(1), &mut normals);
			descriptors.bind_buffer(ResourceSlot::new(2), &mut joints);
			descriptors.bind_buffer(ResourceSlot::new(3), &mut weights);
			descriptors.bind_buffer(ResourceSlot::new(4), &mut palette);
			descriptors.bind_buffer(ResourceSlot::new(5), &mut output);
			descriptors.bind_buffer(ResourceSlot::new(6), &mut dual_quaternion_palette);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&program, &mut descriptors, [0, 0]);
		}

		assert_eq!(
			output
				.read_indexed_field("values", 2, "position")
				.expect("Missing skinned position."),
			Value::Vec4F([2.0, 3.0, 1.0, 1.0])
		);
		assert_eq!(
			output
				.read_indexed_field("values", 2, "normal")
				.expect("Missing skinned normal."),
			Value::Vec4F([0.0, 0.0, 1.0, 0.0])
		);

		// A malformed legacy joint must produce legal bind-pose output without indexing beyond the palette.
		joints
			.write_indexed("values", 1, Value::Vec4U16([2, 0, 0, 0]))
			.expect("Failed to write an out-of-range source joint.");
		push_constant
			.write("destination_vertex_base", Value::U32(3))
			.expect("Failed to update the skinning destination.");
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(ResourceSlot::new(0), &mut positions);
			descriptors.bind_buffer(ResourceSlot::new(1), &mut normals);
			descriptors.bind_buffer(ResourceSlot::new(2), &mut joints);
			descriptors.bind_buffer(ResourceSlot::new(3), &mut weights);
			descriptors.bind_buffer(ResourceSlot::new(4), &mut palette);
			descriptors.bind_buffer(ResourceSlot::new(5), &mut output);
			descriptors.bind_buffer(ResourceSlot::new(6), &mut dual_quaternion_palette);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&program, &mut descriptors, [0, 0]);
		}
		assert_eq!(
			output
				.read_indexed_field("values", 3, "position")
				.expect("Missing fallback skinned position."),
			Value::Vec4F([1.0, 1.0, 1.0, 1.0])
		);
	}

	/// Demonstrates that rigid dual-quaternion blending preserves radius across an opposing joint twist.
	#[test]
	fn skinning_besl_vm_dual_quaternions_preserve_twist_volume_and_handle_antipodality() {
		let program = compile(production_skinning_main());
		let mut positions = buffer(&program, ResourceSlot::new(0));
		let mut normals = buffer(&program, ResourceSlot::new(1));
		let mut joints = buffer(&program, ResourceSlot::new(2));
		let mut weights = buffer(&program, ResourceSlot::new(3));
		let mut matrix_palette = buffer(&program, ResourceSlot::new(4));
		let mut output = buffer(&program, ResourceSlot::new(5));
		let mut dual_quaternion_palette = buffer(&program, ResourceSlot::new(6));
		let mut push_constant = push_constant_buffer(&program);

		positions
			.write_indexed("values", 0, Value::Vec3F([0.0, 1.0, 0.0]))
			.expect("Failed to write twist source position.");
		normals
			.write_indexed("values", 0, Value::Vec3F([0.0, 1.0, 0.0]))
			.expect("Failed to write twist source normal.");
		joints
			.write_indexed("values", 0, Value::Vec4U16([0, 1, u16::MAX, u16::MAX]))
			.expect("Failed to write twist joints.");
		weights
			.write_indexed("values", 0, Value::Vec4F([2.0, 2.0, 0.0, 0.0]))
			.expect("Failed to write twist weights.");
		let sine = 3.0_f32.sqrt() * 0.5;
		let cosine = 0.5;
		let positive = dual_quaternion_from_rigid_transform(&[
			[1.0, 0.0, 0.0],
			[0.0, cosine, sine],
			[0.0, -sine, cosine],
			[2.0, 0.0, 0.0],
		]);
		write_dual_quaternion(&mut dual_quaternion_palette, 0, positive.real, positive.dual);
		// Negate the equivalent -60-degree transform to exercise per-vertex antipodality correction too.
		let negative = dual_quaternion_from_rigid_transform(&[
			[1.0, 0.0, 0.0],
			[0.0, cosine, -sine],
			[0.0, sine, cosine],
			[2.0, 0.0, 0.0],
		]);
		write_dual_quaternion(
			&mut dual_quaternion_palette,
			1,
			negative.real.map(|component| -component),
			negative.dual.map(|component| -component),
		);
		for (field, value) in [
			("source_vertex_base", 0),
			("destination_vertex_base", 0),
			("palette_base", 0),
			("palette_count", 2),
			("vertex_count", 1),
			("palette_kind", SkinningPaletteKind::DualQuaternion as u32),
		] {
			push_constant
				.write(field, Value::U32(value))
				.expect("Failed to write dual-quaternion skinning push constant.");
		}

		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(0), &mut positions);
		descriptors.bind_buffer(ResourceSlot::new(1), &mut normals);
		descriptors.bind_buffer(ResourceSlot::new(2), &mut joints);
		descriptors.bind_buffer(ResourceSlot::new(3), &mut weights);
		descriptors.bind_buffer(ResourceSlot::new(4), &mut matrix_palette);
		descriptors.bind_buffer(ResourceSlot::new(5), &mut output);
		descriptors.bind_buffer(ResourceSlot::new(6), &mut dual_quaternion_palette);
		descriptors.bind_push_constant(&mut push_constant);
		run_at(&program, &mut descriptors, [0, 0]);

		let Value::Vec4F(position) = output
			.read_indexed_field("values", 0, "position")
			.expect("Missing dual-quaternion skinned position.")
		else {
			panic!("Dual-quaternion skinning wrote a non-vector position.");
		};
		let Value::Vec4F(normal) = output
			.read_indexed_field("values", 0, "normal")
			.expect("Missing dual-quaternion skinned normal.")
		else {
			panic!("Dual-quaternion skinning wrote a non-vector normal.");
		};
		for (actual, expected) in position.into_iter().zip([2.0, 1.0, 0.0, 1.0]) {
			math::assert_float_eq!(actual, expected);
		}
		for (actual, expected) in normal.into_iter().zip([0.0, 1.0, 0.0, 0.0]) {
			math::assert_float_eq!(actual, expected);
		}
	}

	/// Writes one column-major translation matrix into the compact VM palette fixture.
	fn write_translation_matrix(palette: &mut Buffer, index: usize, translation: [f32; 3]) {
		palette
			.write_indexed(
				"values",
				index,
				Value::Mat4x3F([
					1.0,
					0.0,
					0.0,
					0.0,
					1.0,
					0.0,
					0.0,
					0.0,
					1.0,
					translation[0],
					translation[1],
					translation[2],
				]),
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
