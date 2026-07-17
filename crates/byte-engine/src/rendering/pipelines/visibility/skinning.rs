use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use resource_management::{
	resources::{material, skeleton::Matrix4Columns},
	shader::generator::ShaderGenerationSettings,
	types::ShaderTypes as ResourceShaderTypes,
};
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
.buffer_stride(64);
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
	pub(crate) vertex_count: u32,
}

impl SkinningDispatch {
	pub(crate) const fn new(
		source_vertex_base: u32,
		destination_vertex_base: u32,
		palette_base: u32,
		vertex_count: u32,
	) -> Self {
		Self {
			source_vertex_base,
			destination_vertex_base,
			palette_base,
			vertex_count,
		}
	}
}

/// The `SkinningPass` struct owns frame-local animation outputs and the compute state that populates them before visibility rendering.
pub(crate) struct SkinningPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	matrix_palette_buffer: ghi::DynamicBufferHandle<[Matrix4Columns; MAX_SKINNING_MATRICES]>,
	skinned_vertices_buffer: ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]>,
}

impl SkinningPass {
	/// Creates the frame-local palette and output buffers plus the immutable compute descriptor set.
	pub(crate) fn new(context: &mut ghi::implementation::Context, sources: SkinningSourceBuffers) -> Self {
		let matrix_palette_buffer = context.build_dynamic_buffer::<[Matrix4Columns; MAX_SKINNING_MATRICES]>(
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

		let shader = crate::rendering::shader_store::create_shader(
			context,
			None,
			&crate::rendering::shader_store::ShaderSourceDescriptor {
				id: "byte-engine/rendering/visibility/skinning",
				name: "Visibility Skinning Compute Shader",
				stage: ResourceShaderTypes::Compute,
				source: crate::rendering::shader_store::ShaderSourceDefinition::Besl {
					settings: ShaderGenerationSettings::compute(Extent::line(SKINNING_WORKGROUP_SIZE)),
					main_node: create_skinning_program(),
				},
				interface: material::ShaderInterface {
					workgroup_size: Some((SKINNING_WORKGROUP_SIZE, 1, 1)),
					bindings: vec![
						material::Binding::new(0, material::BindingKind::StorageBuffer, 1, true, false),
						material::Binding::new(1, material::BindingKind::StorageBuffer, 1, true, false),
						material::Binding::new(2, material::BindingKind::StorageBuffer, 1, true, false),
						material::Binding::new(3, material::BindingKind::StorageBuffer, 1, true, false),
						material::Binding::new(4, material::BindingKind::StorageBuffer, 1, true, false),
						material::Binding::new(5, material::BindingKind::StorageBuffer, 1, false, true),
					],
				},
			},
		)
		.expect("Failed to create visibility skinning shader. The most likely cause is an incompatible packed buffer layout.");
		let pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[ghi::pipelines::PushConstantRange::new(
				0,
				std::mem::size_of::<SkinningDispatch>() as u32,
			)],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		));

		Self {
			pipeline,
			descriptor_set,
			matrix_palette_buffer,
			skinned_vertices_buffer,
		}
	}

	pub(crate) const fn matrix_palette_buffer(&self) -> ghi::DynamicBufferHandle<[Matrix4Columns; MAX_SKINNING_MATRICES]> {
		self.matrix_palette_buffer
	}

	pub(crate) const fn skinned_vertices_buffer(&self) -> ghi::DynamicBufferHandle<[SkinnedVertex; MAX_SKINNED_VERTICES]> {
		self.skinned_vertices_buffer
	}

	/// Copies a complete caller-produced palette into the active frame without allocating intermediate storage.
	pub(crate) fn write_matrix_palette(&self, frame: &mut ghi::implementation::Frame, matrices: &[Matrix4Columns]) {
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
	) {
		if dispatches.is_empty() {
			return;
		}

		let command = command_buffer.bind_compute_pipeline(self.pipeline);
		command.bind_descriptor_sets(&[self.descriptor_set]);
		for dispatch in dispatches.iter().copied().filter(|dispatch| dispatch.vertex_count != 0) {
			command.write_push_constant(0, dispatch);
			command.dispatch(ghi::DispatchExtent::new(
				Extent::line(dispatch.vertex_count),
				Extent::line(SKINNING_WORKGROUP_SIZE),
			));
		}
	}
}

/// Builds the production skinning program from one portable BESL source.
fn create_skinning_program() -> besl::NodeReference {
	build_skinning_program(super::MAX_VERTICES, MAX_SKINNING_MATRICES, MAX_SKINNED_VERTICES)
}

/// Builds a skinning program with caller-selected capacities so semantic tests can use compact buffers.
fn build_skinning_program(
	source_vertex_capacity: usize,
	matrix_capacity: usize,
	output_vertex_capacity: usize,
) -> besl::NodeReference {
	let mut root = besl::Node::root();
	let u32_type = root.get_child("u32").expect("u32 type not found in BESL root");
	let vec3f_type = root.get_child("vec3f").expect("vec3f type not found in BESL root");
	let vec4f_type = root.get_child("vec4f").expect("vec4f type not found in BESL root");
	let vec4u16_type = root.get_child("vec4u16").expect("vec4u16 type not found in BESL root");

	let skinning_matrix = root.add_child(
		besl::Node::r#struct(
			"SkinningMatrix",
			vec![
				besl::Node::member("column0", vec4f_type.clone()).into(),
				besl::Node::member("column1", vec4f_type.clone()).into(),
				besl::Node::member("column2", vec4f_type.clone()).into(),
				besl::Node::member("column3", vec4f_type.clone()).into(),
			],
		)
		.into(),
	);
	let skinned_vertex = root.add_child(
		besl::Node::r#struct(
			"SkinnedVertex",
			vec![
				besl::Node::member("position", vec4f_type.clone()).into(),
				besl::Node::member("normal", vec4f_type.clone()).into(),
			],
		)
		.into(),
	);

	root.add_children(vec![
		besl::Node::binding(
			"source_positions",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", vec3f_type.clone(), source_vertex_capacity)],
			},
			0,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"source_normals",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", vec3f_type, source_vertex_capacity)],
			},
			1,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"source_joints",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", vec4u16_type, source_vertex_capacity)],
			},
			2,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"source_weights",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", vec4f_type.clone(), source_vertex_capacity)],
			},
			3,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"matrix_palette",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", skinning_matrix, matrix_capacity)],
			},
			4,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"skinned_vertices",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("values", skinned_vertex, output_vertex_capacity)],
			},
			5,
			false,
			true,
		)
		.into(),
		besl::Node::push_constant(vec![
			besl::Node::member("source_vertex_base", u32_type.clone()).into(),
			besl::Node::member("destination_vertex_base", u32_type.clone()).into(),
			besl::Node::member("palette_base", u32_type.clone()).into(),
			besl::Node::member("vertex_count", u32_type).into(),
		])
		.into(),
	]);

	let program = besl::compile_to_besl(SKINNING_COMPUTE_BESL, Some(root))
		.expect("Failed to compile visibility skinning BESL. The most likely cause is invalid portable shader syntax.");
	program
		.get_main()
		.expect("Missing visibility skinning entry point. The most likely cause is that the BESL program does not define main.")
}

const SKINNING_COMPUTE_BESL: &str = r#"
transform_columns: fn(
	column0: vec4f,
	column1: vec4f,
	column2: vec4f,
	column3: vec4f,
	value: vec4f
) -> vec4f {
	return column0 * value.x
		+ column1 * value.y
		+ column2 * value.z
		+ column3 * value.w;
}

transform_normal: fn(column0: vec3f, column1: vec3f, column2: vec3f, normal: vec3f) -> vec3f {
	let cofactor0: vec3f = cross(column1, column2);
	let cofactor1: vec3f = cross(column2, column0);
	let cofactor2: vec3f = cross(column0, column1);
	let determinant: f32 = dot(column0, cofactor0);
	if (abs(determinant) > 0.00000001) {
		return (cofactor0 * normal.x + cofactor1 * normal.y + cofactor2 * normal.z) / determinant;
	}

	// Singular joint transforms retain their usable linear directions instead of producing NaNs.
	return column0 * normal.x + column1 * normal.y + column2 * normal.z;
}

safe_normalize: fn(value: vec3f, fallback: vec3f) -> vec3f {
	let length_squared: f32 = dot(value, value);
	if (length_squared > 0.00000001) {
		return value * inversesqrt(length_squared);
	}
	return fallback;
}

main: fn () -> void {
	let local_vertex_index: u32 = thread_id().x;
	if (local_vertex_index >= push_constant.vertex_count) {
		return;
	}

	let source_vertex_index: u32 = push_constant.source_vertex_base + local_vertex_index;
	let source_position: vec3f = source_positions.values[source_vertex_index];
	let source_normal: vec3f = source_normals.values[source_vertex_index];
	let joints: vec4u16 = source_joints.values[source_vertex_index];
	let weights: vec4f = source_weights.values[source_vertex_index];
	let matrix0: SkinningMatrix = matrix_palette.values[push_constant.palette_base + u32(joints.x)];
	let matrix1: SkinningMatrix = matrix_palette.values[push_constant.palette_base + u32(joints.y)];
	let matrix2: SkinningMatrix = matrix_palette.values[push_constant.palette_base + u32(joints.z)];
	let matrix3: SkinningMatrix = matrix_palette.values[push_constant.palette_base + u32(joints.w)];
	let total_weight: f32 = weights.x + weights.y + weights.z + weights.w;

	let fallback_normal: vec3f = safe_normalize(source_normal, vec3f(0.0, 0.0, 1.0));
	let position: vec4f = vec4f(source_position.x, source_position.y, source_position.z, 1.0);
	let normal: vec3f = fallback_normal;
	if (total_weight > 0.00000001) {
		let inverse_total_weight: f32 = 1.0 / total_weight;
		let column0: vec4f = (
			matrix0.column0 * weights.x
			+ matrix1.column0 * weights.y
			+ matrix2.column0 * weights.z
			+ matrix3.column0 * weights.w
		) * inverse_total_weight;
		let column1: vec4f = (
			matrix0.column1 * weights.x
			+ matrix1.column1 * weights.y
			+ matrix2.column1 * weights.z
			+ matrix3.column1 * weights.w
		) * inverse_total_weight;
		let column2: vec4f = (
			matrix0.column2 * weights.x
			+ matrix1.column2 * weights.y
			+ matrix2.column2 * weights.z
			+ matrix3.column2 * weights.w
		) * inverse_total_weight;
		let column3: vec4f = (
			matrix0.column3 * weights.x
			+ matrix1.column3 * weights.y
			+ matrix2.column3 * weights.z
			+ matrix3.column3 * weights.w
		) * inverse_total_weight;

		position = transform_columns(
			column0,
			column1,
			column2,
			column3,
			vec4f(source_position.x, source_position.y, source_position.z, 1.0)
		);
		normal = safe_normalize(
			transform_normal(
				vec3f(column0.x, column0.y, column0.z),
				vec3f(column1.x, column1.y, column1.z),
				vec3f(column2.x, column2.y, column2.z),
				source_normal
			),
			fallback_normal
		);
	}

	let destination_vertex_index: u32 = push_constant.destination_vertex_base + local_vertex_index;
	// Store the complete visibility vertex in one operation so the buffer observes its declared BESL struct layout.
	skinned_vertices.values[destination_vertex_index] = SkinnedVertex(
		vec4f(position.x, position.y, position.z, 1.0),
		vec4f(normal.x, normal.y, normal.z, 0.0)
	);
}
"#;
#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ResourceSlot, Value};
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::{ShaderGenerationSettings, ShaderGenerator as _},
	};

	use super::*;
	use crate::rendering::shader_vm_test::{buffer, compile, run_at};

	#[test]
	fn skinning_host_types_match_besl_buffer_layouts() {
		assert_eq!(std::mem::size_of::<[u16; 4]>(), 8);
		assert_eq!(std::mem::size_of::<Matrix4Columns>(), 64);
		assert_eq!(std::mem::size_of::<SkinnedVertex>(), 32);
		assert_eq!(std::mem::align_of::<SkinnedVertex>(), 16);
		assert_eq!(std::mem::size_of::<SkinningDispatch>(), 16);
	}

	/// Verifies the one production BESL program lowers through every supported graphics backend.
	#[test]
	fn skinning_besl_lowers_to_every_backend() {
		let main = create_skinning_program();
		let settings = ShaderGenerationSettings::compute(Extent::line(SKINNING_WORKGROUP_SIZE));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to lower visibility skinning BESL to GLSL.");
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to lower visibility skinning BESL to HLSL.");
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to lower visibility skinning BESL to MSL.");

		assert!(glsl.contains("layout(local_size_x=64,local_size_y=1,local_size_z=1) in;"));
		assert!(hlsl.contains("[numthreads(64, 1, 1)]"));
		assert!(msl.contains("// besl-threadgroup-size:64,1,1"));
		for source in [&glsl, &hlsl, &msl] {
			assert!(source.contains("cofactor0") && source.contains("determinant"));
			assert!(source.contains("destination_vertex_base"));
			assert!(source.contains("skinned_vertices"));
		}
		assert!(glsl.contains("u16vec4"));
		assert!(hlsl.contains("uint16_t4"));
		assert!(msl.contains("packed_ushort4"));
	}

	/// Executes the production skinning semantics with two weighted joints and checks the deformed vertex.
	#[test]
	fn skinning_besl_vm_blends_joint_matrices_and_writes_position_and_normal() {
		let program = compile(build_skinning_program(4, 4, 4));
		let mut positions = buffer(&program, ResourceSlot::new(0));
		let mut normals = buffer(&program, ResourceSlot::new(1));
		let mut joints = buffer(&program, ResourceSlot::new(2));
		let mut weights = buffer(&program, ResourceSlot::new(3));
		let mut palette = buffer(&program, ResourceSlot::new(4));
		let mut output = buffer(&program, ResourceSlot::new(5));
		let mut push_constant = Buffer::new(
			program
				.push_constant_layout()
				.expect("Missing skinning push constant layout.")
				.clone(),
		);

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
	}

	/// Writes one column-major translation matrix into the compact VM palette fixture.
	fn write_translation_matrix(palette: &mut Buffer, index: usize, translation: [f32; 3]) {
		for (field, value) in [
			("column0", [1.0, 0.0, 0.0, 0.0]),
			("column1", [0.0, 1.0, 0.0, 0.0]),
			("column2", [0.0, 0.0, 1.0, 0.0]),
			("column3", [translation[0], translation[1], translation[2], 1.0]),
		] {
			palette
				.write_indexed_field("values", index, field, Value::Vec4F(value))
				.expect("Failed to write skinning matrix.");
		}
	}

	#[test]
	fn skinning_besl_msl_compiles_for_metal() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		if !ghi::implementation::USES_METAL {
			return;
		}

		let main = create_skinning_program();
		let msl = MSLShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::line(SKINNING_WORKGROUP_SIZE)),
				&main,
			)
			.expect("Failed to lower visibility skinning BESL to MSL.");
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the skinning compute shader test.");
		let mut queue = None;
		let mut context = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::COMPUTE), &mut queue)],
			)
			.expect("Expected a Metal device for the skinning compute shader test.")
			.create_context()
			.expect("Expected a Metal context for the skinning compute shader test.");

		let shader = context.create_shader(
			Some("Visibility Skinning BESL Compute Shader Test"),
			ghi::shader::Sources::MTL {
				source: &msl,
				entry_point: "besl_main",
			},
			ghi::ShaderTypes::Compute,
			[
				SOURCE_POSITIONS_BINDING,
				SOURCE_NORMALS_BINDING,
				SOURCE_JOINTS_BINDING,
				SOURCE_WEIGHTS_BINDING,
				MATRIX_PALETTE_BINDING,
				SKINNED_VERTICES_BINDING,
			],
		);

		assert!(
			shader.is_ok(),
			"Expected generated visibility skinning MSL to compile for Metal."
		);
	}
}
