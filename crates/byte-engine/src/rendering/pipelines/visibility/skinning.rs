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

/// The `SkinningDispatch` struct identifies one active primitive instance and its matrix-palette range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SkinningDispatch {
	pub(crate) source_vertex_base: u32,
	pub(crate) destination_vertex_base: u32,
	pub(crate) palette_base: u32,
	pub(crate) palette_count: u32,
	pub(crate) vertex_count: u32,
}

impl SkinningDispatch {
	pub(crate) const fn new(
		source_vertex_base: u32,
		destination_vertex_base: u32,
		palette_base: u32,
		palette_count: u32,
		vertex_count: u32,
	) -> Self {
		Self {
			source_vertex_base,
			destination_vertex_base,
			palette_base,
			palette_count,
			vertex_count,
		}
	}
}

/// The `SkinningPass` struct owns frame-local animation outputs and the compute state that populates them before visibility rendering.
pub(crate) struct SkinningPass {
	pipeline: crate::rendering::PipelineRef,
	descriptor_set: ghi::DescriptorSetHandle,
	matrix_palette_buffer: ghi::DynamicBufferHandle<[AffineMatrix4x3Columns; MAX_SKINNING_MATRICES]>,
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

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::ShaderGenerationSettings,
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
		assert_eq!(std::mem::size_of::<SkinnedVertex>(), 32);
		assert_eq!(std::mem::align_of::<SkinnedVertex>(), 16);
		assert_eq!(std::mem::size_of::<SkinningDispatch>(), 20);
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
}
