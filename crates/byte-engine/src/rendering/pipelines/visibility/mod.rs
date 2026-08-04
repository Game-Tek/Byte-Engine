#[doc(hidden)]
pub mod gpu_vertex_data_manager;
pub(crate) mod mesh_dispatch;
pub mod pipeline_manager;
#[doc(hidden)]
pub mod render_pass;
#[doc(hidden)]
pub mod resource_manager;
#[doc(hidden)]
pub mod scene_manager;
#[doc(hidden)]
pub mod shader_generator;
pub(crate) mod skinning;

pub use pipeline_manager::VisibilityPipelineManager;

/* BASE */
/// Shader binding used to access scene views.
// Every backend stores affine matrices as twelve floats; MSL reconstructs native float4x3 values when reading them.
pub(crate) const VIEW_DATA_BUFFER_STRIDE: u32 = 176;
pub(crate) const VIEWS_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VIEW_DATA_BUFFER_STRIDE);
// ShaderMesh retains an explicit 16-byte record alignment while its affine matrix occupies 48 bytes.
pub(crate) const MESH_DATA_BUFFER_STRIDE: u32 = 80;
pub(crate) const MESH_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(MESH_DATA_BUFFER_STRIDE);
pub(crate) const VERTEX_POSITIONS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(2),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(12);
/// The octahedrally encoded runtime normal element.
pub(crate) type RuntimeVertexNormal = [u16; 2];
pub(crate) const VERTEX_NORMAL_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexNormal>() as u32;
pub(crate) const VERTEX_NORMAL_SHADER_TYPE: &str = "vec2u16";
pub(crate) const VERTEX_NORMALS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(3),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_NORMAL_BUFFER_STRIDE);
pub(crate) const SKINNED_VERTICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(4),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(32);
/// The packed runtime UV element. Change this alias, its stride, and the shader storage type together to swap formats.
pub(crate) type RuntimeVertexUv = [u16; 2];
pub(crate) const VERTEX_UV_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexUv>() as u32;
pub(crate) const VERTEX_UV_SHADER_TYPE: &str = "vec2u16";
pub(crate) const VERTEX_UV_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(5),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_UV_BUFFER_STRIDE);
// HLSL reads packed narrow indices through 32-bit structured words. Metal and
// Vulkan expose their native scalar element widths directly.
pub(crate) const VERTEX_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 2 };
pub(crate) const VERTEX_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(6),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_INDEX_BUFFER_STRIDE);
pub(crate) const PRIMITIVE_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 1 };
pub(crate) const PRIMITIVE_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(7),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(PRIMITIVE_INDEX_BUFFER_STRIDE);
pub(crate) const MESHLET_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(8),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(64);
pub(crate) const MESH_DISPATCH_WORK_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1063),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(4);
pub(crate) const TEXTURES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::new(
	ghi::ResourceSlot::new(9),
	ghi::ResourceKind::CombinedImageSampler,
	MAX_BINDLESS_TEXTURES as u32,
	ghi::AccessPolicies::READ,
);

/* Visibility */
pub(crate) const MATERIAL_COUNT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_OFFSET_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(16);
pub(crate) const MATERIAL_XY_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1037),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
)
.buffer_stride(4);
pub(crate) const TRIANGLE_INDEX_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1039),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ,
);
pub(crate) const INSTANCE_ID_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1040),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ,
);

/* Material Evaluation */
const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;
const MESHLET_CULLING_TASK_GROUP_SIZE: u32 = 32;

const MAX_MESHLETS: usize = 1024 * 4;
const MAX_INSTANCES: usize = 1024;
const MAX_MATERIALS: usize = 1024;
pub(super) type ActiveMaterialMask = [u64; MAX_MATERIALS / u64::BITS as usize];
// Materials keep a small indirection table so generated shaders can use stable per-material slots,
// while the descriptor array itself is a larger scene-wide bindless texture pool.
const MAX_MATERIAL_TEXTURES: usize = 16;
const MAX_BINDLESS_TEXTURES: usize = 1024;
const MAX_LIGHTS: usize = 16;
const MAX_TRIANGLES: usize = 65536 * 4;
const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
const MAX_VERTICES: usize = 65536 * 4;
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;
pub(crate) const SHADOW_CASCADE_COUNT: usize = 4;
pub(crate) const SHADOW_MAP_RESOLUTION: u32 = 2048;
pub(crate) const MAX_CONE_SHADOWS: usize = 4;
pub(crate) const CONE_SHADOW_MAP_RESOLUTION: u32 = 1024;
/// The depth format that halves the memory used by cone-light shadow maps.
pub(crate) const CONE_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
/// The depth format retained for directional cascades and the camera depth target.
pub(crate) const DIRECTIONAL_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth32;
pub(crate) const CONE_SHADOW_VIEW_OFFSET: usize = 1 + SHADOW_CASCADE_COUNT;
pub(crate) const SHADOW_VIEW_COUNT: usize = CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOWS;

/// The `ShaderMeshletData` struct stores meshlet offsets and object-space culling bounds for GPU visibility passes.
#[derive(Copy, Clone)]
#[repr(C, align(16))]
pub(super) struct ShaderMeshletData {
	/// Base index into the vertex-index buffer.
	/// ```glsl
	/// vertex_index = mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + gl_LocalInvocationID.x];
	/// ```
	primitive_offset: u32,
	/// Base triangle index into the primitive-index buffer.
	///
	/// The stored value divides the raw index by 3 because each triangle has three indices.
	/// ```glsl
	/// triangle_index = primitive_indices.primitive_indices[(meshlet.triangle_offset + gl_LocalInvocationID.x) * 3 + 0..2]
	/// ```
	triangle_offset: u32,
	/// Number of meshlet-local primitives.
	primitive_count: u32,
	// The number of triangles in the meshlet
	triangle_count: u32,
	/// Object-space bounding sphere encoded as xyz center and w radius.
	center_radius: [f32; 4],
	/// Object-space normal-cone apex encoded as xyz apex and w cutoff.
	cone_apex_cutoff: [f32; 4],
	/// Object-space normal-cone axis encoded as xyz axis.
	cone_axis: [f32; 4],
}

#[cfg(test)]
mod tests {
	use besl::vm::{
		input_slot, output_slot, DescriptorBindings, ExecutableProgram, ExecutionConfig, MeshOutputs, ResourceSlot,
		TaskOutputs, Texture, Value, WorkgroupState,
	};
	#[cfg(target_os = "macos")]
	use resource_management::shader::besl::backends::msl::Generator as MslGenerator;
	use resource_management::shader::{
		besl::{backends::hlsl::Generator as HlslGenerator, evaluation::ProgramEvaluation},
		ShaderGenerationSettings,
	};

	use crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatchWorkItem;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const VIEWS_SLOT: ResourceSlot = ResourceSlot::new(0);
	const GTAO_PARAMETERS_SLOT: ResourceSlot = ResourceSlot::new(1);
	const MESH_DATA_SLOT: ResourceSlot = ResourceSlot::new(1);
	const MATERIAL_COUNT_SLOT: ResourceSlot = ResourceSlot::new(1033);
	const MATERIAL_OFFSET_SLOT: ResourceSlot = ResourceSlot::new(1034);
	const MATERIAL_OFFSET_SCRATCH_SLOT: ResourceSlot = ResourceSlot::new(1035);
	const MATERIAL_DISPATCH_SLOT: ResourceSlot = ResourceSlot::new(1036);
	const PIXEL_MAPPING_SLOT: ResourceSlot = ResourceSlot::new(1037);
	const INSTANCE_INDEX_SLOT: ResourceSlot = ResourceSlot::new(1040);
	const MESH_DISPATCH_WORK_SLOT: ResourceSlot = ResourceSlot::new(1063);
	const VERTEX_POSITIONS_SLOT: ResourceSlot = ResourceSlot::new(2);
	const SKINNED_VERTICES_SLOT: ResourceSlot = ResourceSlot::new(4);
	const VERTEX_INDICES_SLOT: ResourceSlot = ResourceSlot::new(6);
	const PRIMITIVE_INDICES_SLOT: ResourceSlot = ResourceSlot::new(7);
	const MESHLETS_SLOT: ResourceSlot = ResourceSlot::new(8);
	const FIXTURE_INSTANCE_INDEX: usize = 3;
	const FIXTURE_MESHLET_INDEX: usize = 5;
	const MESHLET_INSTANCE_BITS: u32 = 12;
	const TASK_WORKGROUP_SIZE: u32 = 32;
	const MESH_TEST_INSTRUCTION_LIMIT: usize = 4_000_000;
	const GTAO_WORKGROUP_WIDTH: u32 = 16;
	const GTAO_WORKGROUP_HEIGHT: u32 = 8;
	const GTAO_WORKGROUP_SIZE: usize = 128;
	const GTAO_BLUR_WORKGROUP_WIDTH: u32 = 8;
	const GTAO_BLUR_WORKGROUP_SIZE: usize = 64;
	const GTAO_PYRAMID_WORKGROUP_WIDTH: u32 = 8;
	const GTAO_PYRAMID_WORKGROUP_SIZE: usize = 64;
	const MATERIAL_COUNT_WORKGROUP_WIDTH: u32 = 8;
	const MATERIAL_COUNT_WORKGROUP_SIZE: usize = 64;
	const PIXEL_MAPPING_WORKGROUP_WIDTH: u32 = 16;
	const PIXEL_MAPPING_WORKGROUP_SIZE: usize = 256;

	/// Parses the checked-in BESL source that production baking consumes.
	fn asset_program(source: &str) -> besl::NodeReference {
		besl::lex(
			besl::parse(source)
				.expect("Failed to parse a visibility shader asset. The most likely cause is invalid checked-in BESL source."),
		)
		.expect("Failed to link a visibility shader asset. The most likely cause is an invalid shader declaration.")
		.get_main()
		.expect("Missing visibility shader main. The most likely cause is that a checked-in BESL asset is incomplete.")
	}

	fn material_count_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/material-count.besl"
		)))
	}

	fn material_offset_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/material-offset.besl"
		)))
	}

	fn pixel_mapping_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/pixel-mapping.besl"
		)))
	}

	fn visibility_fragment_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/visibility-fragment.besl"
		)))
	}

	fn visibility_task_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/visibility-task.besl"
		)))
	}

	fn shadow_task_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/shadow-task.besl"
		)))
	}

	fn gtao_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao.besl"
		)))
	}

	fn gtao_depth_pyramid_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-depth-pyramid.besl"
		)))
	}

	fn gtao_upscale_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-upscale.besl"
		)))
	}

	fn gtao_blur_x_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-blur-x.besl"
		)))
	}

	/// Guards the complete GTAO interface persisted beside the native shader artifact.
	#[test]
	fn gtao_reflects_compact_view_and_linear_hierarchy_resources() {
		assert_reflected_resources(
			gtao_program(),
			&[
				(0, "gtao_view"),
				(1, "gtao_parameters"),
				(1033, "depth_pyramid"),
				(1034, "ao_output"),
			],
		);
		assert_reflected_resources(
			gtao_depth_pyramid_program(),
			&[
				(0, "gtao_view"),
				(1033, "source_depth"),
				(1034, "reduced_depth_1"),
				(1035, "reduced_depth_2"),
				(1036, "reduced_depth_3"),
			],
		);
		assert_reflected_resources(
			gtao_blur_x_program(),
			&[(1033, "linear_depth"), (1034, "ao_source"), (1035, "ao_output")],
		);
		assert_reflected_resources(
			gtao_upscale_program(),
			&[
				(0, "gtao_view"),
				(1033, "visibility_depth"),
				(1034, "ao_source"),
				(1035, "ao_output"),
				(1036, "low_resolution_depth"),
			],
		);
	}

	/// Verifies a BESL prepass exposes only its reachable flat resources.
	fn assert_reflected_resources(program: besl::NodeReference, expected: &[(u32, &str)]) {
		let evaluation = ProgramEvaluation::from_main(&program)
			.expect("Failed to reflect a visibility prepass. The most likely cause is an invalid BESL resource graph.");
		let reflected = evaluation
			.bindings()
			.iter()
			.map(|binding| (binding.slot, binding.name.as_str()))
			.collect::<Vec<_>>();
		assert_eq!(reflected, expected);
	}

	/// Verifies the visibility fragment preserves the mesh-stage identifiers consumed by later compute passes.
	#[test]
	fn visibility_fragment_main_forwards_primitive_and_instance_identifiers() {
		let program = crate::rendering::shader_vm_test::compile(visibility_fragment_program());
		let mut instance_input = besl::vm::Buffer::new(
			program
				.input_layout(0)
				.expect("Missing visibility instance input. The most likely cause is a drifted fragment interface.")
				.clone(),
		);
		let mut primitive_input = besl::vm::Buffer::new(
			program
				.input_layout(1)
				.expect("Missing visibility primitive input. The most likely cause is a drifted fragment interface.")
				.clone(),
		);
		let mut primitive_output = besl::vm::Buffer::new(
			program
				.output_layout(0)
				.expect("Missing visibility primitive output. The most likely cause is a drifted fragment interface.")
				.clone(),
		);
		let mut instance_output = besl::vm::Buffer::new(
			program
				.output_layout(1)
				.expect("Missing visibility instance output. The most likely cause is a drifted fragment interface.")
				.clone(),
		);
		instance_input
			.write("in_instance_index", Value::U32(37))
			.expect("Failed to initialize the visibility instance input. The most likely cause is a drifted input type.");
		primitive_input
			.write("in_primitive_index", Value::U32(0x0102_03ab))
			.expect("Failed to initialize the visibility primitive input. The most likely cause is a drifted input type.");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot(0), &mut instance_input);
			descriptors.bind_buffer(input_slot(1), &mut primitive_input);
			descriptors.bind_buffer(output_slot(0), &mut primitive_output);
			descriptors.bind_buffer(output_slot(1), &mut instance_output);
			program.run_main(&mut descriptors).expect(
				"Failed to execute the visibility fragment shader. The most likely cause is missing interface support in the BESL VM.",
			);
		}

		assert_eq!(
			primitive_output
				.read("out_primitive_index")
				.expect("Failed to read the visibility primitive output. The most likely cause is a drifted output layout."),
			Value::U32(0x0102_03ab)
		);
		assert_eq!(
			instance_output
				.read("out_instance_id")
				.expect("Failed to read the visibility instance output. The most likely cause is a drifted output layout."),
			Value::U32(37)
		);
	}

	/// Returns a column-major identity matrix in the BESL VM representation.
	fn identity_matrix() -> [f32; 16] {
		[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
	}

	/// Returns a column-major affine identity matrix in the BESL VM representation.
	fn identity_affine_matrix() -> [f32; 12] {
		[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
	}

	/// Returns a view-projection matrix that moves identity geometry outside the horizontal clip range.
	fn horizontally_translated_matrix(translation: f32) -> [f32; 16] {
		let mut matrix = identity_matrix();
		matrix[12] = translation;
		matrix
	}

	/// Packs the production task payload without allowing its meshlet and instance indices to diverge.
	fn meshlet_instance(meshlet_index: u32, instance_index: u32) -> u32 {
		meshlet_index | (instance_index << MESHLET_INSTANCE_BITS)
	}

	/// Executes an exact production task main as one workgroup over consecutive meshlets.
	fn run_meshlet_task_workgroup(
		program: &ExecutableProgram,
		view_projections: &[(usize, [f32; 16])],
		selected_view_index: Option<u32>,
		center_radii: &[[f32; 4]],
		skinned: bool,
	) -> TaskOutputs {
		run_meshlet_task_workgroup_at(program, view_projections, selected_view_index, center_radii, skinned, 0)
	}

	/// Executes one exact production task workgroup at its global dispatch position.
	fn run_meshlet_task_workgroup_at(
		program: &ExecutableProgram,
		view_projections: &[(usize, [f32; 16])],
		selected_view_index: Option<u32>,
		center_radii: &[[f32; 4]],
		skinned: bool,
		workgroup_index: u32,
	) -> TaskOutputs {
		assert!(
			!center_radii.is_empty(),
			"Missing task meshlet fixtures. The most likely cause is a test invoking a workgroup without any task lanes."
		);
		let meshlet_count = u32::try_from(center_radii.len())
			.expect("Task meshlet fixture is too large. The most likely cause is an invalid test case.");
		assert!(
			meshlet_count <= TASK_WORKGROUP_SIZE,
			"Task meshlet fixture exceeds one workgroup. The most likely cause is a test supplying more meshlets than the production payload can address."
		);
		let mut views = buffer(program, VIEWS_SLOT);
		for (view_index, view_projection) in view_projections.iter().copied() {
			views
				.write_indexed_field("views", view_index, "view_projection", Value::Mat4F(view_projection))
				.expect("Failed to initialize a task view. The most likely cause is a drifted View layout.");
			views
				.write_indexed_field("views", view_index, "inverse_view", Value::Mat4x3F(identity_affine_matrix()))
				.expect("Failed to initialize a task inverse view. The most likely cause is a drifted View layout.");
		}

		let mut meshes = buffer(program, MESH_DATA_SLOT);
		meshes
			.write_indexed_field(
				"meshes",
				FIXTURE_INSTANCE_INDEX,
				"model",
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
			)
			.expect("Failed to initialize a task mesh transform. The most likely cause is a drifted Mesh layout.");
		for (field, value) in [
			("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
			("meshlet_count", meshlet_count),
			("skinned_base_vertex_index", if skinned { 0 } else { u32::MAX }),
		] {
			meshes
				.write_indexed_field("meshes", FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
				.expect("Failed to initialize a task mesh field. The most likely cause is a drifted Mesh layout.");
		}

		let mut meshlets = buffer(program, MESHLETS_SLOT);
		for (meshlet_offset, center_radius) in center_radii.iter().copied().enumerate() {
			let meshlet_index = FIXTURE_MESHLET_INDEX + meshlet_offset;
			meshlets
				.write_indexed_field("meshlets", meshlet_index, "center_radius", Value::Vec4F(center_radius))
				.expect("Failed to initialize a task meshlet bound. The most likely cause is a drifted Meshlet layout.");
			// A cutoff above one disables cone rejection so each fixture isolates frustum and skinning behavior.
			meshlets
				.write_indexed_field(
					"meshlets",
					meshlet_index,
					"cone_apex_cutoff",
					Value::Vec4F([0.0, 0.0, 0.0, 2.0]),
				)
				.expect("Failed to disable task cone culling. The most likely cause is a drifted Meshlet layout.");
		}

		let push_constant_layout = program
			.push_constant_layout()
			.expect("Missing task push constants. The most likely cause is that the production task main no longer uses them.")
			.clone();
		let mut push_constant = besl::vm::Buffer::new(push_constant_layout);
		let view_index = selected_view_index.unwrap_or(0);
		push_constant
			.write("work_item_base", Value::U32(0))
			.expect("Failed to initialize the task work base. The most likely cause is a drifted push constant layout.");
		push_constant
			.write("view_index", Value::U32(view_index))
			.expect("Failed to initialize the task view index. The most likely cause is a drifted push constant layout.");
		let mut mesh_dispatch_work = buffer(program, MESH_DISPATCH_WORK_SLOT);
		let packed_work = MeshDispatchWorkItem::new(FIXTURE_INSTANCE_INDEX as u32, 0).packed();
		mesh_dispatch_work
			.write_indexed("items", workgroup_index as usize, Value::U32(packed_work))
			.expect("Failed to initialize compact mesh dispatch work. The most likely cause is a drifted work-item layout.");

		let mut task_outputs = TaskOutputs::new();
		let mut workgroup_state = WorkgroupState::new();
		let configs = (0..TASK_WORKGROUP_SIZE)
			.map(|lane| {
				ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_call_depth_limit(128)
					.with_thread_idx(lane)
					.with_thread_position(workgroup_index * TASK_WORKGROUP_SIZE + lane)
			})
			.collect::<Vec<_>>();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
			descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
			descriptors.bind_buffer(MESH_DISPATCH_WORK_SLOT, &mut mesh_dispatch_work);
			descriptors.bind_push_constant(&mut push_constant);
			descriptors.bind_task_outputs(&mut task_outputs);
			descriptors.bind_workgroup_state(&mut workgroup_state);

			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute a production task workgroup. The most likely cause is missing task synchronization support or an invalid fixture binding.",
			);
		}

		task_outputs
	}

	/// Executes one lane of an exact production task main with one meshlet.
	fn run_single_meshlet_task(
		program: &ExecutableProgram,
		view_projections: &[(usize, [f32; 16])],
		selected_view_index: Option<u32>,
		center_radius: [f32; 4],
		skinned: bool,
	) -> (Option<u32>, Option<Value>) {
		let task_outputs =
			run_meshlet_task_workgroup(program, view_projections, selected_view_index, &[center_radius], skinned);
		(
			task_outputs.mesh_output_count(),
			task_outputs.payload_value("meshlet_instances", 0).cloned(),
		)
	}

	/// Verifies view-zero culling retains an intersecting meshlet and rejects one outside the frustum.
	#[test]
	fn visibility_task_main_emits_in_frustum_and_culls_off_frustum_meshlets() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let visible = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [0.0, 0.0, 0.5, 0.1], false);
		assert_eq!(
			visible,
			(
				Some(1),
				Some(Value::U32(meshlet_instance(
					FIXTURE_MESHLET_INDEX as u32,
					FIXTURE_INSTANCE_INDEX as u32,
				)))
			)
		);

		let culled = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [4.0, 0.0, 0.5, 0.1], false);
		assert_eq!(culled, (Some(0), None));
	}

	/// Verifies workgroup barriers and atomics compact visible meshlets in lane order before publishing the final count.
	#[test]
	fn visibility_task_workgroup_compacts_mixed_meshlets_in_lane_order() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let output = run_meshlet_task_workgroup(
			&program,
			&[(0, identity_matrix())],
			None,
			&[[0.0, 0.0, 0.5, 0.1], [4.0, 0.0, 0.5, 0.1], [0.5, 0.0, 0.5, 0.1]],
			false,
		);

		assert_eq!(output.mesh_output_count(), Some(2));
		assert_eq!(
			output.payload_value("meshlet_instances", 0),
			Some(&Value::U32(meshlet_instance(
				FIXTURE_MESHLET_INDEX as u32,
				FIXTURE_INSTANCE_INDEX as u32,
			)))
		);
		assert_eq!(
			output.payload_value("meshlet_instances", 1),
			Some(&Value::U32(meshlet_instance(
				FIXTURE_MESHLET_INDEX as u32 + 2,
				FIXTURE_INSTANCE_INDEX as u32,
			)))
		);
		assert_eq!(output.payload_value("meshlet_instances", 2), None);
	}

	/// Verifies visibility culling reads the work item selected by the global dispatch position.
	#[test]
	fn visibility_task_main_selects_later_batched_workgroup() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let output =
			run_meshlet_task_workgroup_at(&program, &[(0, identity_matrix())], None, &[[0.0, 0.0, 0.5, 0.1]], false, 1);

		assert_eq!(output.mesh_output_count(), Some(1));
		assert_eq!(
			output.payload_value("meshlet_instances", 0),
			Some(&Value::U32(meshlet_instance(
				FIXTURE_MESHLET_INDEX as u32,
				FIXTURE_INSTANCE_INDEX as u32,
			)))
		);
	}

	/// Verifies deformed geometry reaches the mesh stage even when its static meshlet bound is outside the frustum.
	#[test]
	fn visibility_task_main_bypasses_static_culling_for_skinned_meshes() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let output = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [4.0, 0.0, 0.5, 0.1], true);
		assert_eq!(
			output,
			(
				Some(1),
				Some(Value::U32(meshlet_instance(
					FIXTURE_MESHLET_INDEX as u32,
					FIXTURE_INSTANCE_INDEX as u32,
				)))
			)
		);
	}

	/// Verifies shadow culling selects the cascade view named by the second push constant.
	#[test]
	fn shadow_task_main_uses_selected_view_index() {
		let program = crate::rendering::shader_vm_test::compile(shadow_task_program());
		let mut view_projections: [(usize, [f32; 16]); 8] =
			std::array::from_fn(|view_index| (view_index, horizontally_translated_matrix(4.0)));
		view_projections[3].1 = identity_matrix();
		let output = run_single_meshlet_task(&program, &view_projections, Some(3), [0.0, 0.0, 0.5, 0.1], false);
		assert_eq!(
			output,
			(
				Some(1),
				Some(Value::U32(meshlet_instance(
					FIXTURE_MESHLET_INDEX as u32,
					FIXTURE_INSTANCE_INDEX as u32,
				)))
			)
		);
	}

	/// Verifies later object workgroups select their own compact work item from global thread positions.
	#[test]
	fn shadow_task_main_selects_later_batched_workgroup() {
		let program = crate::rendering::shader_vm_test::compile(shadow_task_program());
		let output = run_meshlet_task_workgroup_at(
			&program,
			&[(3, identity_matrix())],
			Some(3),
			&[[0.0, 0.0, 0.5, 0.1]],
			false,
			1,
		);

		assert_eq!(output.mesh_output_count(), Some(1));
		assert_eq!(
			output.payload_value("meshlet_instances", 0),
			Some(&Value::U32(meshlet_instance(
				FIXTURE_MESHLET_INDEX as u32,
				FIXTURE_INSTANCE_INDEX as u32,
			)))
		);
	}

	/// Builds the exact production visibility mesh main for VM execution.
	fn visibility_mesh_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/visibility-mesh.besl"
		)))
	}

	/// Builds the exact production shadow mesh main for VM execution.
	fn shadow_mesh_program() -> besl::NodeReference {
		asset_program(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/shadow-mesh.besl"
		)))
	}

	/// Compiles the checked-in visibility and shadow stages against the packed Metal affine ABI.
	#[cfg(target_os = "macos")]
	#[test]
	fn production_visibility_and_shadow_mesh_stages_compile_with_packed_metal_matrices() {
		let stages = [
			(
				"visibility-task",
				visibility_task_program(),
				ShaderGenerationSettings::task(utils::Extent::line(32), 32),
			),
			(
				"shadow-task",
				shadow_task_program(),
				ShaderGenerationSettings::task(utils::Extent::line(32), 32),
			),
			(
				"visibility-mesh",
				visibility_mesh_program(),
				ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
			),
			(
				"shadow-mesh",
				shadow_mesh_program(),
				ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
			),
		];

		for (name, main, settings) in stages {
			let source = MslGenerator::new()
				.generate(&settings, &main)
				.unwrap_or_else(|()| panic!("Failed to lower production {name} BESL to MSL."));
			assert!(source.contains("_besl_packed_float4x3 model"));
			assert!(source.contains("_besl_load_mat4x3("));
			assert!(
				!source.contains("mul("),
				"Production {name} MSL must use Metal's native multiplication operator."
			);
			resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, name)
				.unwrap_or_else(|error| panic!("Failed to compile production {name} MSL: {error}"));
		}
	}

	/// Compiles the production Material Count shader so subgroup lowering stays valid on Metal.
	#[cfg(target_os = "macos")]
	#[test]
	fn production_material_count_stage_compiles_with_metal_subgroups() {
		let source = MslGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(MATERIAL_COUNT_WORKGROUP_WIDTH)),
				&material_count_program(),
			)
			.expect("Failed to lower production Material Count BESL to MSL. The most likely cause is invalid subgroup source.");
		assert!(source.contains("_besl_subgroup_ballot("));
		assert!(source.contains("_besl_subgroup_broadcast_u32("));
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, "visibility-material-count")
			.expect(
				"Failed to compile production Material Count MSL. The most likely cause is invalid Metal subgroup lowering.",
			);
	}

	/// Compiles the production Pixel Mapping shader so its established-key fast path stays valid on Metal.
	#[cfg(target_os = "macos")]
	#[test]
	fn production_pixel_mapping_stage_compiles_with_metal() {
		let source = MslGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::square(PIXEL_MAPPING_WORKGROUP_WIDTH)),
				&pixel_mapping_program(),
			)
			.expect("Failed to lower production Pixel Mapping BESL to MSL. The most likely cause is invalid atomic source.");
		assert!(source.contains("atomic_load_explicit(&"));
		assert!(source.contains("_besl_atomic_compare_exchange("));
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, "visibility-pixel-mapping")
			.expect("Failed to compile production Pixel Mapping MSL. The most likely cause is invalid Metal atomic lowering.");
	}

	/// Creates one identity-transformed triangle meshlet in the production visibility buffer layouts.
	fn mesh_triangle_buffers(
		program: &ExecutableProgram,
	) -> (
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
	) {
		let mut views = buffer(program, VIEWS_SLOT);
		views
			.write_indexed_field("views", 0, "view_projection", Value::Mat4F(identity_matrix()))
			.expect("Failed to initialize the mesh view. The most likely cause is a drifted View layout.");

		let mut meshes = buffer(program, MESH_DATA_SLOT);
		meshes
			.write_indexed_field(
				"meshes",
				FIXTURE_INSTANCE_INDEX,
				"model",
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
			)
			.expect("Failed to initialize the mesh model matrix. The most likely cause is a drifted Mesh layout.");
		for (field, value) in [
			("base_vertex_index", 0),
			("base_primitive_index", 0),
			("base_triangle_index", 0),
			("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
			("meshlet_count", 1),
			("skinned_base_vertex_index", u32::MAX),
		] {
			meshes
				.write_indexed_field("meshes", FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
				.expect("Failed to initialize a mesh offset. The most likely cause is a drifted Mesh layout.");
		}

		let mut positions = buffer(program, VERTEX_POSITIONS_SLOT);
		for (index, position) in [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
			positions
				.write_indexed("positions", index, Value::Vec3F(position))
				.expect("Failed to initialize a mesh vertex. The most likely cause is a drifted position layout.");
		}
		let skinned_vertices = buffer(program, SKINNED_VERTICES_SLOT);

		let mut vertex_indices = buffer(program, VERTEX_INDICES_SLOT);
		let mut primitive_indices = buffer(program, PRIMITIVE_INDICES_SLOT);
		for (index, value) in [0, 1, 2].into_iter().enumerate() {
			vertex_indices
				.write_indexed("vertex_indices", index, Value::U16(value))
				.expect("Failed to initialize a vertex index. The most likely cause is a drifted index layout.");
			primitive_indices
				.write_indexed("primitive_indices", index, Value::U8(value as u8))
				.expect("Failed to initialize a triangle index. The most likely cause is a drifted primitive layout.");
		}

		let mut meshlets = buffer(program, MESHLETS_SLOT);
		for (field, value) in [
			("primitive_offset", 0),
			("triangle_offset", 0),
			("primitive_count", 3),
			("triangle_count", 1),
		] {
			meshlets
				.write_indexed_field("meshlets", FIXTURE_MESHLET_INDEX, field, Value::U32(value))
				.expect("Failed to initialize a meshlet field. The most likely cause is a drifted Meshlet layout.");
		}

		(
			views,
			meshes,
			positions,
			skinned_vertices,
			vertex_indices,
			primitive_indices,
			meshlets,
		)
	}

	/// Executes one production mesh main and verifies its complete one-triangle output contract.
	fn assert_triangle_mesh_program(
		program: besl::NodeReference,
		selected_view: Option<(usize, [f32; 16], u32)>,
		skinned_positions: Option<[[f32; 4]; 3]>,
		expected_clip_positions: [[f32; 4]; 3],
		expected_render_target_array_index: Option<u32>,
	) {
		let program = crate::rendering::shader_vm_test::compile(program);
		let (
			mut views,
			mut meshes,
			mut positions,
			mut skinned_vertices,
			mut vertex_indices,
			mut primitive_indices,
			mut meshlets,
		) = mesh_triangle_buffers(&program);
		if let Some(skinned_positions) = skinned_positions {
			const SKINNED_BASE_VERTEX: usize = 7;
			meshes
				.write_indexed_field(
					"meshes",
					FIXTURE_INSTANCE_INDEX,
					"skinned_base_vertex_index",
					Value::U32(SKINNED_BASE_VERTEX as u32),
				)
				.expect("Failed to select skinned mesh vertices. The most likely cause is a drifted Mesh layout.");
			for (index, position) in skinned_positions.into_iter().enumerate() {
				skinned_vertices
					.write_indexed_field("vertices", SKINNED_BASE_VERTEX + index, "position", Value::Vec4F(position))
					.expect(
						"Failed to initialize a skinned mesh vertex. The most likely cause is a drifted SkinnedVertex layout.",
					);
			}
		}
		let push_constant_layout = program
			.push_constant_layout()
			.expect(
				"Missing mesh push constant layout. The most likely cause is that the production mesh main no longer uses it.",
			)
			.clone();
		let mut push_constant = besl::vm::Buffer::new(push_constant_layout);
		let (view_index, render_target_array_index) =
			if let Some((view_index, view_projection, render_target_array_index)) = selected_view {
				views
					.write_indexed_field("views", view_index, "view_projection", Value::Mat4F(view_projection))
					.expect("Failed to initialize the selected mesh view. The most likely cause is a drifted View layout.");
				(view_index as u32, render_target_array_index)
			} else {
				(0, 0)
			};
		push_constant
			.write("work_item_base", Value::U32(0))
			.expect("Failed to initialize the mesh work base. The most likely cause is a drifted push constant layout.");
		push_constant
			.write("view_index", Value::U32(view_index))
			.expect("Failed to initialize the mesh view index. The most likely cause is a drifted push constant layout.");
		push_constant
			.write("render_target_array_index", Value::U32(render_target_array_index))
			.expect("Failed to initialize the mesh target layer. The most likely cause is a drifted push constant layout.");

		let mut out_instance_indices = buffer(&program, output_slot(0));
		let mut out_primitive_indices = buffer(&program, output_slot(1));
		let mut mesh_outputs = MeshOutputs::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_task_payload(
				"meshlet_instances",
				[Value::U32(meshlet_instance(
					FIXTURE_MESHLET_INDEX as u32,
					FIXTURE_INSTANCE_INDEX as u32,
				))],
			);
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
			descriptors.bind_buffer(VERTEX_POSITIONS_SLOT, &mut positions);
			descriptors.bind_buffer(SKINNED_VERTICES_SLOT, &mut skinned_vertices);
			descriptors.bind_buffer(VERTEX_INDICES_SLOT, &mut vertex_indices);
			descriptors.bind_buffer(PRIMITIVE_INDICES_SLOT, &mut primitive_indices);
			descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
			descriptors.bind_buffer(output_slot(0), &mut out_instance_indices);
			descriptors.bind_buffer(output_slot(1), &mut out_primitive_indices);
			descriptors.bind_push_constant(&mut push_constant);
			descriptors.bind_mesh_outputs(&mut mesh_outputs);

			// Mesh invocations share their capture just as lanes in one production mesh workgroup share output arrays.
			for thread_idx in 0..3 {
				let config = ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_call_depth_limit(128)
					.with_thread_idx(thread_idx)
					.with_threadgroup_position(0);
				program.run_main_with_config(&mut descriptors, &config).expect(
					"Failed to execute a production mesh shader with the BESL VM. The most likely cause is missing mesh intrinsic support or an invalid fixture binding.",
				);
			}
		}

		assert_eq!(mesh_outputs.vertex_count(), 3);
		assert_eq!(mesh_outputs.primitive_count(), 1);
		for (index, expected) in expected_clip_positions.into_iter().enumerate() {
			let actual = mesh_outputs
				.vertex_position(index)
				.expect("Missing mesh vertex output. The most likely cause is that a mesh invocation did not write its lane.");
			assert_rgba_close(actual, expected, 0.00001);
		}
		assert_eq!(mesh_outputs.triangle(0), Some([0, 1, 2]));
		if let Some(expected_render_target_array_index) = expected_render_target_array_index {
			assert_eq!(
				mesh_outputs.render_target_array_index(0),
				Some(expected_render_target_array_index)
			);
		}
		assert_eq!(
			read_u32(&out_instance_indices, "out_instance_index", 0),
			FIXTURE_INSTANCE_INDEX as u32
		);
		assert_eq!(
			read_u32(&out_primitive_indices, "out_primitive_index", 0),
			(FIXTURE_MESHLET_INDEX as u32) << 8
		);
	}

	/// Verifies visibility mesh output geometry and metadata through the BESL VM.
	#[test]
	fn visibility_mesh_main_emits_identity_triangle_and_metadata() {
		assert_triangle_mesh_program(
			visibility_mesh_program(),
			None,
			None,
			[[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
			None,
		);
	}

	/// Verifies that posed instances source raster positions from their frame-local deformation range.
	#[test]
	fn visibility_mesh_main_reads_skinned_positions() {
		let skinned_positions = [[2.0, 3.0, 4.0, 1.0], [5.0, 6.0, 7.0, 1.0], [8.0, 9.0, 10.0, 1.0]];
		assert_triangle_mesh_program(
			visibility_mesh_program(),
			None,
			Some(skinned_positions),
			skinned_positions,
			None,
		);
	}

	/// Verifies shadow mesh output keeps the selected view independent from the target texture-array layer.
	#[test]
	fn shadow_mesh_main_emits_selected_view_triangle_and_metadata() {
		assert_triangle_mesh_program(
			shadow_mesh_program(),
			Some((7, horizontally_translated_matrix(2.0), 2)),
			None,
			[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
			Some(2),
		);
	}

	/// Creates compact camera data for one square GTAO shader fixture.
	fn gtao_view_data(program: &ExecutableProgram, width: u32, height: u32) -> besl::vm::Buffer {
		let near = 0.1f32;
		let far = 100.0f32;
		let projection = math::projection_matrix(60.0, width as f32 / height as f32, near, far);
		let projection_x = projection[0];
		let projection_y = projection[5];
		let width = width as f32;
		let height = height as f32;
		let mut view = buffer(program, VIEWS_SLOT);
		for (member, value) in [
			(
				"pixel_to_ray_mul",
				Value::Vec2F([2.0 / (width * projection_x), -2.0 / (height * projection_y)]),
			),
			(
				"pixel_to_ray_add",
				Value::Vec2F([(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y]),
			),
			("projection_pixels_y", Value::F32(height * projection_y * 0.5)),
			("view_z_sign", Value::F32(1.0)),
			("depth_unproject_numerator", Value::F32(near * far / (far - near))),
			("depth_unproject_denominator_offset", Value::F32(near / (far - near))),
		] {
			view.write(member, value)
				.expect("Failed to initialize compact GTAO view data. The most likely cause is a drifted FastGTAOView layout.");
		}
		view
	}

	/// Creates the default runtime controls used by production GTAO VM fixtures.
	fn gtao_parameters_data(program: &ExecutableProgram) -> besl::vm::Buffer {
		let mut parameters = buffer(program, GTAO_PARAMETERS_SLOT);
		for (member, value) in [
			("radius", Value::F32(1.0)),
			("samples_per_ray", Value::U32(4)),
			("radial_rays", Value::U32(6)),
		] {
			parameters.write(member, value).expect(
				"Failed to initialize GTAO runtime parameters. The most likely cause is a changed production parameter layout.",
			);
		}
		parameters
	}

	/// Reconstructs the positive fixture distance encoded by one reversed device depth.
	fn gtao_fixture_linear_depth(depth: f32) -> f32 {
		if depth == 0.0 {
			return 0.0;
		}
		let near = 0.1f32;
		let far = 100.0f32;
		let range = far - near;
		(near * far / range) / (depth + near / range)
	}

	/// Encodes positive view-space distance as the reversed device depth used by the GTAO fixture camera.
	fn gtao_fixture_device_depth(linear_depth: f32) -> f32 {
		let near = 0.1f32;
		let far = 100.0f32;
		let range = far - near;
		(near * far / (range * linear_depth) - near / range).clamp(0.0, 1.0)
	}

	/// Reduces one positive-linear-depth image while ignoring zero-valued background texels.
	fn reduce_nearest_nonzero_depth(source: &[[f32; 4]], width: u32, height: u32) -> (Vec<[f32; 4]>, u32, u32) {
		let reduced_width = width.div_ceil(2).max(1);
		let reduced_height = height.div_ceil(2).max(1);
		let mut reduced = vec![[0.0, 0.0, 0.0, 1.0]; (reduced_width * reduced_height) as usize];

		for y in 0..reduced_height {
			for x in 0..reduced_width {
				let mut nearest = 0.0f32;
				for offset_y in 0..2 {
					for offset_x in 0..2 {
						let source_x = (x * 2 + offset_x).min(width - 1);
						let source_y = (y * 2 + offset_y).min(height - 1);
						let depth = source[(source_y * width + source_x) as usize][0];
						if depth != 0.0 && (nearest == 0.0 || depth < nearest) {
							nearest = depth;
						}
					}
				}
				reduced[(y * reduced_width + x) as usize][0] = nearest;
			}
		}

		(reduced, reduced_width, reduced_height)
	}

	/// Executes GTAO over a flat floor whose pixel footprint crosses the former absolute normal cutoff.
	fn run_gtao_floor_fixture(program: &ExecutableProgram, camera_height: f32, coordinate: [u32; 2]) -> [f32; 4] {
		const EXTENT: u32 = 64;
		let projection = math::projection_matrix(60.0, 1.0, 0.1, 100.0);
		let ray_mul_y = -2.0 / (EXTENT as f32 * projection[5]);
		let ray_add_y = (1.0 - 1.0 / EXTENT as f32) / projection[5];
		let mut linear_depth = vec![[0.0, 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize];

		for y in 0..EXTENT {
			let ray_y = y as f32 * ray_mul_y + ray_add_y;
			if ray_y >= 0.0 {
				continue;
			}
			let depth = (0.0 - camera_height) / ray_y;
			if !(0.1..=100.0).contains(&depth) {
				continue;
			}
			for x in 0..EXTENT {
				let index = (y * EXTENT + x) as usize;
				linear_depth[index][0] = depth;
			}
		}

		let (linear_depth_1, width_1, height_1) = reduce_nearest_nonzero_depth(&linear_depth, EXTENT, EXTENT);
		let (linear_depth_2, width_2, height_2) = reduce_nearest_nonzero_depth(&linear_depth_1, width_1, height_1);
		let mut view = gtao_view_data(program, EXTENT, EXTENT);
		let mut parameters = gtao_parameters_data(program);
		let mut depth_pyramid = texture_2d(
			EXTENT * 2,
			EXTENT * 2,
			&vec![[0.0, 0.0, 0.0, 1.0]; (EXTENT * 2 * EXTENT * 2) as usize],
		);
		depth_pyramid.add_mip(texture_2d(EXTENT, EXTENT, &linear_depth));
		depth_pyramid.add_mip(texture_2d(width_1, height_1, &linear_depth_1));
		depth_pyramid.add_mip(texture_2d(width_2, height_2, &linear_depth_2));
		let mut output = empty_image(EXTENT, EXTENT);
		let group_base = [
			coordinate[0] / GTAO_WORKGROUP_WIDTH * GTAO_WORKGROUP_WIDTH,
			coordinate[1] / GTAO_WORKGROUP_HEIGHT * GTAO_WORKGROUP_HEIGHT,
		];
		let configs: [ExecutionConfig; GTAO_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([
					group_base[0] + lane % GTAO_WORKGROUP_WIDTH,
					group_base[1] + lane / GTAO_WORKGROUP_WIDTH,
				])
		});
		let mut workgroup = WorkgroupState::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut view);
			descriptors.bind_buffer(GTAO_PARAMETERS_SLOT, &mut parameters);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth_pyramid);
			descriptors.bind_image(ResourceSlot::new(1034), &mut output);
			descriptors.bind_workgroup_state(&mut workgroup);
			program
				.run_workgroup(&mut descriptors, &configs)
				.expect("Failed to execute the floor-normal GTAO fixture. The most likely cause is invalid shared-cache synchronization.");
		}
		rgba(&output, coordinate)
	}

	/// Reads one unsigned scalar from an indexed visibility buffer member.
	fn read_u32(buffer: &besl::vm::Buffer, member: &str, index: usize) -> u32 {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM u32 array element. The most likely cause is a drifted visibility buffer layout.")
		{
			Value::U32(value) => value,
			value => panic!(
				"Unexpected visibility buffer value: {value:?}. The most likely cause is a drifted material buffer type."
			),
		}
	}

	/// Reads one dispatch tuple from an indexed visibility buffer member.
	fn read_vec4u(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u32; 4] {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM vec4u array element. The most likely cause is a drifted visibility buffer layout.")
		{
			Value::Vec4U(value) => value,
			value => panic!(
				"Unexpected visibility dispatch value: {value:?}. The most likely cause is a drifted dispatch buffer type."
			),
		}
	}

	/// Reads one packed pixel coordinate from the visibility mapping buffer.
	fn read_vec2u16(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u16; 2] {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM vec2u16 array element. The most likely cause is a drifted pixel mapping layout.")
		{
			Value::Vec2U16(value) => value,
			value => panic!(
				"Unexpected visibility pixel mapping value: {value:?}. The most likely cause is a drifted mapping buffer type."
			),
		}
	}

	/// Exercises the production material prepasses as one stateful VM pipeline.
	#[test]
	fn visibility_material_compute_pipeline_counts_offsets_and_maps_valid_pixels() {
		let material_count_program = crate::rendering::shader_vm_test::compile(material_count_program());
		let material_offset_program = crate::rendering::shader_vm_test::compile(material_offset_program());
		let pixel_mapping_program = crate::rendering::shader_vm_test::compile(pixel_mapping_program());

		// Three visible instances span two materials; the fourth texel is the renderer's empty-pixel sentinel.
		let mut mesh_data = buffer(&material_count_program, MESH_DATA_SLOT);
		for (mesh_index, material_index) in [(0, 2), (1, 5), (2, 2)] {
			mesh_data
				.write_indexed_field("meshes", mesh_index, "material_index", Value::U32(material_index))
				.expect("Failed to initialize a VM mesh. The most likely cause is a drifted Mesh buffer layout.");
		}

		let mut instance_indices = Texture::new(2, 2)
			.expect("Failed to create the visibility index fixture. The most likely cause is an invalid test extent.");
		for (coordinate, instance_index) in [([0, 0], 0), ([1, 0], 1), ([0, 1], u32::MAX), ([1, 1], 2)] {
			instance_indices
				.write_u32(coordinate, instance_index)
				.expect("Failed to initialize a visibility index texel. The most likely cause is an invalid coordinate.");
		}

		let mut material_counts = buffer(&material_count_program, MATERIAL_COUNT_SLOT);
		{
			let mut workgroup = WorkgroupState::new();
			let configs: [ExecutionConfig; MATERIAL_COUNT_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
				let lane = lane as u32;
				ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_thread_idx(lane)
					.with_thread_id([lane % MATERIAL_COUNT_WORKGROUP_WIDTH, lane / MATERIAL_COUNT_WORKGROUP_WIDTH])
			});
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			material_count_program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the production material-count workgroup. The most likely cause is broken tile synchronization or invalid histogram storage.",
			);
		}

		assert_eq!(read_u32(&material_counts, "material_count", 2), 2);
		assert_eq!(read_u32(&material_counts, "material_count", 5), 1);
		assert_eq!(read_u32(&material_counts, "material_count", 0), 0);

		// The offset pass converts sparse counts into exclusive offsets and one indirect dispatch tuple per material.
		let mut material_offsets = buffer(&material_offset_program, MATERIAL_OFFSET_SLOT);
		let mut material_offset_scratch = buffer(&material_offset_program, MATERIAL_OFFSET_SCRATCH_SLOT);
		let mut material_dispatches = buffer(&material_offset_program, MATERIAL_DISPATCH_SLOT);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_buffer(MATERIAL_OFFSET_SLOT, &mut material_offsets);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(MATERIAL_DISPATCH_SLOT, &mut material_dispatches);
			run_at(&material_offset_program, &mut descriptors, [0, 0]);
		}

		assert_eq!(read_u32(&material_offsets, "material_offset", 2), 0);
		assert_eq!(read_u32(&material_offsets, "material_offset", 5), 2);
		assert_eq!(read_u32(&material_offsets, "material_offset", 6), 3);
		assert_eq!(read_u32(&material_counts, "material_count", 2), 0);
		assert_eq!(read_u32(&material_counts, "material_count", 5), 0);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 0),
			[0, 1, 1, 0]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 2),
			[1, 1, 1, 2]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 5),
			[1, 1, 1, 1]
		);

		// Mapping reuses the scratch offsets as atomic cursors and stores one-based coordinates for later zero-sentinel checks.
		let mut pixel_mapping = buffer(&pixel_mapping_program, PIXEL_MAPPING_SLOT);
		{
			let mut workgroup = WorkgroupState::new();
			let configs: [ExecutionConfig; PIXEL_MAPPING_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
				let lane = lane as u32;
				ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_thread_idx(lane)
					.with_thread_id([lane % PIXEL_MAPPING_WORKGROUP_WIDTH, lane / PIXEL_MAPPING_WORKGROUP_WIDTH])
			});
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			pixel_mapping_program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the production pixel-mapping workgroup. The most likely cause is broken tile reservation or synchronization.",
			);
		}

		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 0), [1, 1]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 1), [2, 2]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 2), [2, 1]);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 2), 2);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 5), 3);
	}

	/// Verifies a coherent tile reuses its established local key while preserving every pixel mapping.
	#[test]
	fn pixel_mapping_load_fast_path_preserves_coherent_tile_mappings() {
		let program = crate::rendering::shader_vm_test::compile(pixel_mapping_program());
		let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
		mesh_data
			.write_indexed_field("meshes", 0, "material_index", Value::U32(7))
			.expect(
				"Failed to initialize the coherent Pixel Mapping mesh. The most likely cause is a drifted Mesh buffer layout.",
			);
		let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
		let mut instance_indices = Texture::new(PIXEL_MAPPING_WORKGROUP_WIDTH, PIXEL_MAPPING_WORKGROUP_WIDTH)
			.expect("Failed to create the coherent Pixel Mapping fixture. The most likely cause is an invalid test extent.");
		for lane in 0..PIXEL_MAPPING_WORKGROUP_SIZE {
			instance_indices
				.write_u32(
					[
						(lane % PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u32,
						(lane / PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u32,
					],
					0,
				)
				.expect("Failed to initialize a coherent Pixel Mapping texel. The most likely cause is an invalid coordinate.");
		}

		let mut pixel_mapping = buffer(&program, PIXEL_MAPPING_SLOT);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; PIXEL_MAPPING_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_thread_idx(lane)
				.with_thread_id([lane % PIXEL_MAPPING_WORKGROUP_WIDTH, lane / PIXEL_MAPPING_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the coherent Pixel Mapping workgroup. The most likely cause is broken established-key reservation.",
			);
		}

		let mut seen = [[false; PIXEL_MAPPING_WORKGROUP_WIDTH as usize]; PIXEL_MAPPING_WORKGROUP_WIDTH as usize];
		for mapping_index in 0..PIXEL_MAPPING_WORKGROUP_SIZE {
			let coordinate = read_vec2u16(&pixel_mapping, "pixel_mapping", mapping_index);
			assert!(
				coordinate[0] > 0
					&& coordinate[0] <= PIXEL_MAPPING_WORKGROUP_WIDTH as u16
					&& coordinate[1] > 0
					&& coordinate[1] <= PIXEL_MAPPING_WORKGROUP_WIDTH as u16,
				"Pixel Mapping returned an invalid coherent-tile coordinate. The most likely cause is that the fast path reused a local rank."
			);
			let x = (coordinate[0] - 1) as usize;
			let y = (coordinate[1] - 1) as usize;
			assert!(
				!seen[y][x],
				"Pixel Mapping duplicated a coherent-tile coordinate. The most likely cause is that the fast path reused a local rank."
			);
			seen[y][x] = true;
		}
		assert!(
			seen.into_iter().flatten().all(|coordinate| coordinate),
			"Pixel Mapping omitted a coherent-tile coordinate. The most likely cause is that the fast path skipped a local rank."
		);
		assert_eq!(
			read_u32(&material_offset_scratch, "material_offset_scratch", 7),
			PIXEL_MAPPING_WORKGROUP_SIZE as u32,
			"Pixel Mapping advanced the coherent material cursor incorrectly. The most likely cause is that the fast path did not reserve each pixel exactly once."
		);
	}

	/// Verifies tile-local reservations preserve mappings when distinct materials exceed the bounded histogram.
	#[test]
	fn pixel_mapping_tile_reservation_preserves_overflowed_materials() {
		let program = crate::rendering::shader_vm_test::compile(pixel_mapping_program());
		let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
		let mut material_offset_scratch = buffer(&program, MATERIAL_OFFSET_SCRATCH_SLOT);
		for material_index in 0..33 {
			mesh_data
				.write_indexed_field("meshes", material_index, "material_index", Value::U32(material_index as u32))
				.expect("Failed to initialize a VM mesh. The most likely cause is a drifted Mesh buffer layout.");
			material_offset_scratch
				.write_indexed("material_offset_scratch", material_index, Value::U32(material_index as u32))
				.expect("Failed to initialize a material mapping offset. The most likely cause is a drifted scratch layout.");
		}

		let mut instance_indices = Texture::new(PIXEL_MAPPING_WORKGROUP_WIDTH, PIXEL_MAPPING_WORKGROUP_WIDTH)
			.expect("Failed to create the pixel-mapping overflow fixture. The most likely cause is an invalid extent.");
		for lane in 0..PIXEL_MAPPING_WORKGROUP_SIZE {
			let instance_index = if lane < 33 { lane as u32 } else { u32::MAX };
			instance_indices
				.write_u32(
					[
						(lane % PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u32,
						(lane / PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u32,
					],
					instance_index,
				)
				.expect("Failed to initialize a mapping texel. The most likely cause is an invalid coordinate.");
		}

		let mut pixel_mapping = buffer(&program, PIXEL_MAPPING_SLOT);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; PIXEL_MAPPING_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_thread_idx(lane)
				.with_thread_id([lane % PIXEL_MAPPING_WORKGROUP_WIDTH, lane / PIXEL_MAPPING_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the overflowing pixel-mapping workgroup. The most likely cause is broken fallback reservation.",
			);
		}

		for material_index in 0..33 {
			let expected_coordinate = [
				(material_index % PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
				(material_index / PIXEL_MAPPING_WORKGROUP_WIDTH as usize) as u16 + 1,
			];
			assert_eq!(
				read_vec2u16(&pixel_mapping, "pixel_mapping", material_index),
				expected_coordinate,
				"Unexpected coordinate for material {material_index}. The most likely cause is a dropped tile reservation."
			);
			assert_eq!(
				read_u32(&material_offset_scratch, "material_offset_scratch", material_index),
				material_index as u32 + 1,
				"Unexpected cursor for material {material_index}. The most likely cause is a duplicated tile reservation."
			);
		}
	}

	/// Verifies a tile with more unique materials than histogram slots preserves every count through the overflow path.
	#[test]
	fn material_count_tile_histogram_preserves_overflowed_materials() {
		let program = crate::rendering::shader_vm_test::compile(material_count_program());
		let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
		for material_index in 0..33 {
			mesh_data
				.write_indexed_field("meshes", material_index, "material_index", Value::U32(material_index as u32))
				.expect("Failed to initialize a VM mesh. The most likely cause is a drifted Mesh buffer layout.");
		}

		let mut instance_indices = Texture::new(8, 8)
			.expect("Failed to create the histogram fixture. The most likely cause is an invalid test extent.");
		for lane in 0..MATERIAL_COUNT_WORKGROUP_SIZE {
			instance_indices
				.write_u32(
					[
						(lane % MATERIAL_COUNT_WORKGROUP_WIDTH as usize) as u32,
						(lane / MATERIAL_COUNT_WORKGROUP_WIDTH as usize) as u32,
					],
					(lane % 33) as u32,
				)
				.expect("Failed to initialize a histogram texel. The most likely cause is an invalid coordinate.");
		}

		let mut material_counts = buffer(&program, MATERIAL_COUNT_SLOT);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; MATERIAL_COUNT_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_thread_idx(lane)
				.with_thread_id([lane % MATERIAL_COUNT_WORKGROUP_WIDTH, lane / MATERIAL_COUNT_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the overflowing material-count workgroup. The most likely cause is broken safe-overflow handling.",
			);
		}

		for material_index in 0..33 {
			let expected = if material_index < 31 { 2 } else { 1 };
			assert_eq!(
				read_u32(&material_counts, "material_count", material_index),
				expected,
				"Unexpected count for material {material_index}. The most likely cause is a dropped or duplicated tile-histogram entry."
			);
		}
	}

	/// Verifies subgroup aggregation retains every pixel in a coherent Material Count tile.
	#[test]
	fn material_count_subgroup_aggregation_counts_a_coherent_tile_once_per_partition() {
		let program = crate::rendering::shader_vm_test::compile(material_count_program());
		let mut mesh_data = buffer(&program, MESH_DATA_SLOT);
		mesh_data
			.write_indexed_field("meshes", 0, "material_index", Value::U32(7))
			.expect(
				"Failed to initialize the coherent Material Count mesh. The most likely cause is a drifted Mesh buffer layout.",
			);
		let mut instance_indices = Texture::new(8, 8)
			.expect("Failed to create the coherent Material Count fixture. The most likely cause is an invalid test extent.");
		for lane in 0..MATERIAL_COUNT_WORKGROUP_SIZE {
			instance_indices
				.write_u32(
					[
						(lane % MATERIAL_COUNT_WORKGROUP_WIDTH as usize) as u32,
						(lane / MATERIAL_COUNT_WORKGROUP_WIDTH as usize) as u32,
					],
					0,
				)
				.expect(
					"Failed to initialize a coherent Material Count texel. The most likely cause is an invalid coordinate.",
				);
		}

		let mut material_counts = buffer(&program, MATERIAL_COUNT_SLOT);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; MATERIAL_COUNT_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_thread_idx(lane)
				.with_thread_id([lane % MATERIAL_COUNT_WORKGROUP_WIDTH, lane / MATERIAL_COUNT_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Coherent Material Count execution failed. The most likely cause is broken subgroup aggregation or tile-histogram flushing.",
			);
		}

		assert_eq!(
			read_u32(&material_counts, "material_count", 7),
			MATERIAL_COUNT_WORKGROUP_SIZE as u32
		);
	}

	/// Executes the standard GTAO shader with one deterministic depth fixture.
	fn run_gtao_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		run_gtao_fixture_with_parameters(program, width, height, depth_texels, coordinate, 1.0, 6, 8)
	}

	/// Executes GTAO with explicit runtime controls so parameter reads are covered by the VM fixture.
	fn run_gtao_fixture_with_parameters(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		coordinate: [u32; 2],
		radius: f32,
		samples_per_ray: u32,
		radial_rays: u32,
	) -> [f32; 4] {
		let mut view = gtao_view_data(program, width, height);
		let mut parameters = gtao_parameters_data(program);
		parameters
			.write("radius", Value::F32(radius))
			.expect("GTAO fixture radius should match the production parameter type.");
		parameters
			.write("samples_per_ray", Value::U32(samples_per_ray))
			.expect("GTAO fixture samples should match the production parameter type.");
		parameters
			.write("radial_rays", Value::U32(radial_rays))
			.expect("GTAO fixture rays should match the production parameter type.");
		let linear_depth_texels = depth_texels
			.iter()
			.map(|texel| [gtao_fixture_linear_depth(texel[0]), 0.0, 0.0, 1.0])
			.collect::<Vec<_>>();
		let pyramid_1_extent = [(width / 2).max(1), (height / 2).max(1)];
		let pyramid_2_extent = [(width / 4).max(1), (height / 4).max(1)];
		let pyramid_1 = texture_2d(
			pyramid_1_extent[0],
			pyramid_1_extent[1],
			&vec![[0.0, 0.0, 0.0, 1.0]; (pyramid_1_extent[0] * pyramid_1_extent[1]) as usize],
		);
		let pyramid_2 = texture_2d(
			pyramid_2_extent[0],
			pyramid_2_extent[1],
			&vec![[0.0, 0.0, 0.0, 1.0]; (pyramid_2_extent[0] * pyramid_2_extent[1]) as usize],
		);
		let mut depth_pyramid = texture_2d(
			width * 2,
			height * 2,
			&vec![[0.0, 0.0, 0.0, 1.0]; (width * 2 * height * 2) as usize],
		);
		depth_pyramid.add_mip(texture_2d(width, height, &linear_depth_texels));
		depth_pyramid.add_mip(pyramid_1);
		depth_pyramid.add_mip(pyramid_2);
		let mut output = empty_image(width, height);
		let group_base = [
			coordinate[0] / GTAO_WORKGROUP_WIDTH * GTAO_WORKGROUP_WIDTH,
			coordinate[1] / GTAO_WORKGROUP_HEIGHT * GTAO_WORKGROUP_HEIGHT,
		];
		let configs: [ExecutionConfig; GTAO_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([
					group_base[0] + lane % GTAO_WORKGROUP_WIDTH,
					group_base[1] + lane / GTAO_WORKGROUP_WIDTH,
				])
		});
		let mut workgroup = WorkgroupState::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut view);
			descriptors.bind_buffer(GTAO_PARAMETERS_SLOT, &mut parameters);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth_pyramid);
			descriptors.bind_image(ResourceSlot::new(1034), &mut output);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the production GTAO workgroup. The most likely cause is broken cache synchronization or an invalid fixture binding.",
			);
		}
		rgba(&output, coordinate)
	}

	/// Runs the production GTAO shader with uniform coarse levels so a fixture can isolate hierarchical sampling.
	fn run_gtao_hierarchical_fixture(program: &ExecutableProgram, coarse_linear_depth: f32) -> [f32; 4] {
		const EXTENT: u32 = 129;
		const CENTER: [u32; 2] = [64, 64];
		let linear_depth_texels = vec![[gtao_fixture_linear_depth(0.35), 0.0, 0.0, 1.0]; (EXTENT * EXTENT) as usize];

		let mut view = gtao_view_data(program, EXTENT, EXTENT);
		let mut parameters = gtao_parameters_data(program);
		let mut depth_pyramid = texture_2d(
			EXTENT * 2,
			EXTENT * 2,
			&vec![[0.0, 0.0, 0.0, 1.0]; (EXTENT * 2 * EXTENT * 2) as usize],
		);
		depth_pyramid.add_mip(texture_2d(EXTENT, EXTENT, &linear_depth_texels));
		depth_pyramid.add_mip(texture_2d(
			EXTENT / 2,
			EXTENT / 2,
			&vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 64 * 64],
		));
		depth_pyramid.add_mip(texture_2d(
			EXTENT / 4,
			EXTENT / 4,
			&vec![[coarse_linear_depth, 0.0, 0.0, 1.0]; 32 * 32],
		));
		let mut output = empty_image(EXTENT, EXTENT);
		let configs: [ExecutionConfig; GTAO_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([
					CENTER[0] + lane % GTAO_WORKGROUP_WIDTH,
					CENTER[1] + lane / GTAO_WORKGROUP_WIDTH,
				])
		});
		let mut workgroup = WorkgroupState::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut view);
			descriptors.bind_buffer(GTAO_PARAMETERS_SLOT, &mut parameters);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth_pyramid);
			descriptors.bind_image(ResourceSlot::new(1034), &mut output);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the hierarchical GTAO fixture. The most likely cause is broken shared-cache addressing.",
			);
		}
		rgba(&output, CENTER)
	}

	/// Verifies each production depth-pyramid texel keeps the nearest nonzero linear depth in its source footprint.
	#[test]
	fn gtao_depth_pyramid_reduces_odd_extents_to_nearest_linear_depth() {
		let program = crate::rendering::shader_vm_test::compile(gtao_depth_pyramid_program());
		let mut source = texture_2d(
			3,
			3,
			&[
				[0.0, 0.0, 0.0, 1.0],
				[0.2, 0.0, 0.0, 1.0],
				[0.3, 0.0, 0.0, 1.0],
				[0.4, 0.0, 0.0, 1.0],
				[0.9, 0.0, 0.0, 1.0],
				[0.5, 0.0, 0.0, 1.0],
				[0.6, 0.0, 0.0, 1.0],
				[0.7, 0.0, 0.0, 1.0],
				[0.8, 0.0, 0.0, 1.0],
			],
		);
		let mut reduced_1 = empty_image(1, 1);
		let mut reduced_2 = empty_image(1, 1);
		let mut reduced_3 = empty_image(1, 1);
		let mut view = gtao_view_data(&program, 3, 3);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; GTAO_PYRAMID_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([lane % GTAO_PYRAMID_WORKGROUP_WIDTH, lane / GTAO_PYRAMID_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut view);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut source);
			descriptors.bind_image(ResourceSlot::new(1034), &mut reduced_1);
			descriptors.bind_image(ResourceSlot::new(1035), &mut reduced_2);
			descriptors.bind_image(ResourceSlot::new(1036), &mut reduced_3);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute the fused GTAO depth pyramid. The most likely cause is broken shared reduction synchronization or an invalid fixture binding.",
			);
		}

		let nearest = [gtao_fixture_linear_depth(0.9), 0.0, 0.0, 1.0];
		assert_rgba_close(rgba(&reduced_1, [0, 0]), nearest, 0.00001);
		assert_rgba_close(rgba(&reduced_2, [0, 0]), nearest, 0.00001);
		assert_rgba_close(rgba(&reduced_3, [0, 0]), nearest, 0.00001);
	}

	/// Verifies distant GTAO steps consume conservative hierarchy levels instead of always fetching full-resolution depth.
	#[test]
	fn gtao_uses_depth_pyramid_for_distant_samples() {
		let program = crate::rendering::shader_vm_test::compile(gtao_program());
		let empty_coarse_depth = run_gtao_hierarchical_fixture(&program, 0.0);
		let occupied_coarse_depth = run_gtao_hierarchical_fixture(&program, gtao_fixture_linear_depth(0.4));

		assert!(
			occupied_coarse_depth[0] < empty_coarse_depth[0],
			"Expected populated coarse depth to increase distant occlusion, found empty={empty_coarse_depth:?} and occupied={occupied_coarse_depth:?}. The most likely cause is that GTAO stopped selecting hierarchy levels."
		);
	}

	/// Verifies the standard GTAO shader's background contract and recessed-foreground response.
	#[test]
	fn gtao_writes_white_for_background_and_expected_recessed_foreground_ao() {
		let program = crate::rendering::shader_vm_test::compile(gtao_program());
		let background = run_gtao_fixture(&program, 1, 1, &[[0.0, 0.0, 0.0, 1.0]], [0, 0]);
		assert_rgba_close(background, [1.0, 1.0, 1.0, 1.0], 0.00001);

		// A recessed center surrounded by nearer depth exercises reconstruction,
		// normal estimation, and the adaptive bounded AO integral.
		let mut foreground_depth = [[0.75, 0.0, 0.0, 1.0]; 25];
		foreground_depth[12] = [0.35, 0.0, 0.0, 1.0];
		let foreground = run_gtao_fixture(&program, 5, 5, &foreground_depth, [2, 2]);
		assert_rgba_close(foreground, [0.8315444, 0.8315444, 0.8315444, 1.0], 0.00001);

		let disabled = run_gtao_fixture_with_parameters(&program, 5, 5, &foreground_depth, [2, 2], 0.0, 1, 2);
		assert_rgba_close(disabled, [1.0, 1.0, 1.0, 1.0], 0.00001);
	}

	/// Verifies flat-floor normals remain valid when their world-space finite differences become very small.
	#[test]
	fn gtao_floor_has_no_scale_dependent_normal_seam() {
		let program = crate::rendering::shader_vm_test::compile(gtao_program());
		let larger_floor = run_gtao_floor_fixture(&program, 0.1, [32, 63]);
		let scaled_floor = run_gtao_floor_fixture(&program, 0.06, [32, 63]);

		assert!(
			(larger_floor[0] - scaled_floor[0]).abs() < 0.0005,
			"Expected geometrically identical floors to preserve AO across world scales, got large={} and scaled={}. The most likely cause is a scale-dependent normal fallback.",
			larger_floor[0],
			scaled_floor[0]
		);
	}

	/// Compiles one checked-in axis-specific GTAO blur asset.
	fn compile_gtao_blur(source: &str) -> ExecutableProgram {
		crate::rendering::shader_vm_test::compile(asset_program(source))
	}

	/// Runs one complete GTAO blur workgroup and reads the selected output pixel.
	fn run_gtao_blur_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		ao_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut depth = texture_2d(width, height, depth_texels);
		let mut ao = texture_2d(width, height, ao_texels);
		let mut output = empty_image(width, height);
		let mut workgroup = WorkgroupState::new();
		let configs: [ExecutionConfig; GTAO_BLUR_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([lane % GTAO_BLUR_WORKGROUP_WIDTH, lane / GTAO_BLUR_WORKGROUP_WIDTH])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
			descriptors.bind_texture(ResourceSlot::new(1034), &mut ao);
			descriptors.bind_image(ResourceSlot::new(1035), &mut output);
			descriptors.bind_workgroup_state(&mut workgroup);
			program.run_workgroup(&mut descriptors, &configs).expect(
				"Failed to execute a production GTAO blur workgroup. The most likely cause is broken shared-tile synchronization or an invalid fixture binding.",
			);
		}
		rgba(&output, coordinate)
	}

	/// Runs the production depth-aware upscale workgroup and reads one full-resolution output pixel.
	fn run_gtao_upscale_fixture(
		program: &ExecutableProgram,
		full_extent: [u32; 2],
		device_depth_texels: &[[f32; 4]],
		low_extent: [u32; 2],
		linear_depth_texels: &[[f32; 4]],
		ao_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut view = gtao_view_data(program, low_extent[0], low_extent[1]);
		let mut device_depth = texture_2d(full_extent[0], full_extent[1], device_depth_texels);
		let mut linear_depth = texture_2d(low_extent[0], low_extent[1], linear_depth_texels);
		let mut ao = texture_2d(low_extent[0], low_extent[1], ao_texels);
		let mut output = empty_image(full_extent[0], full_extent[1]);
		let mut workgroup = WorkgroupState::new();
		let group_base = [
			coordinate[0] / GTAO_BLUR_WORKGROUP_WIDTH * GTAO_BLUR_WORKGROUP_WIDTH,
			coordinate[1] / GTAO_BLUR_WORKGROUP_WIDTH * GTAO_BLUR_WORKGROUP_WIDTH,
		];
		let configs: [ExecutionConfig; GTAO_BLUR_WORKGROUP_SIZE] = std::array::from_fn(|lane| {
			let lane = lane as u32;
			ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane)
				.with_thread_id([
					group_base[0] + lane % GTAO_BLUR_WORKGROUP_WIDTH,
					group_base[1] + lane / GTAO_BLUR_WORKGROUP_WIDTH,
				])
		});
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut view);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut device_depth);
			descriptors.bind_texture(ResourceSlot::new(1034), &mut ao);
			descriptors.bind_image(ResourceSlot::new(1035), &mut output);
			descriptors.bind_texture(ResourceSlot::new(1036), &mut linear_depth);
			descriptors.bind_workgroup_state(&mut workgroup);
			program
				.run_workgroup(&mut descriptors, &configs)
				.expect("Failed to execute the production GTAO upscale workgroup. The most likely cause is an invalid reconstruction binding.");
		}
		rgba(&output, coordinate)
	}

	/// Verifies the half-resolution horizontal denoiser preserves uniform AO and smooths its axis.
	#[test]
	fn gtao_half_resolution_blur_preserves_uniform_ao_and_smooths_horizontally() {
		let blur_x = compile_gtao_blur(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-blur-x.besl"
		)));
		let depth = [[0.5, 0.0, 0.0, 1.0]; 25];
		let uniform_ao = [[0.37, 0.0, 0.0, 1.0]; 25];
		assert_rgba_close(
			run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &uniform_ao, [2, 2]),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);

		// Horizontal variation must be reduced before the final reconstruction stage.
		let directional_ao: [[f32; 4]; 25] = std::array::from_fn(|index| {
			if index % 5 == 2 {
				[1.0, 0.0, 0.0, 1.0]
			} else {
				[0.0, 0.0, 0.0, 1.0]
			}
		});
		let horizontal = run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &directional_ao, [2, 2]);
		assert!(
			horizontal[0] < 0.8,
			"Expected X blur to mix neighboring columns, found {horizontal:?}"
		);
	}

	/// Verifies full-resolution reconstruction preserves uniform input and rejects AO across depth discontinuities.
	#[test]
	fn gtao_upscale_is_depth_aware_and_preserves_uniform_ao() {
		let upscale = crate::rendering::shader_vm_test::compile(gtao_upscale_program());
		let full_extent = [7, 5];
		let low_extent = [4, 3];
		let uniform_device_depth = vec![[0.5, 0.0, 0.0, 1.0]; 35];
		let uniform_linear_depth = vec![[gtao_fixture_linear_depth(0.5), 0.0, 0.0, 1.0]; 12];
		let uniform_ao = vec![[0.37, 0.0, 0.0, 1.0]; 12];
		assert_rgba_close(
			run_gtao_upscale_fixture(
				&upscale,
				full_extent,
				&uniform_device_depth,
				low_extent,
				&uniform_linear_depth,
				&uniform_ao,
				[6, 4],
			),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);

		let full_extent = [8, 8];
		let low_extent = [4, 4];
		let device_depth: [[f32; 4]; 64] = std::array::from_fn(|index| {
			let x = index % full_extent[0] as usize;
			[if x < 4 { 0.7 } else { 0.3 }, 0.0, 0.0, 1.0]
		});
		let linear_depth: [[f32; 4]; 16] = std::array::from_fn(|index| {
			let x = index % low_extent[0] as usize;
			[gtao_fixture_linear_depth(if x < 2 { 0.7 } else { 0.3 }), 0.0, 0.0, 1.0]
		});
		let ao: [[f32; 4]; 16] = std::array::from_fn(|index| {
			let x = index % low_extent[0] as usize;
			[if x < 2 { 0.2 } else { 0.8 }, 0.0, 0.0, 1.0]
		});
		let left = run_gtao_upscale_fixture(&upscale, full_extent, &device_depth, low_extent, &linear_depth, &ao, [3, 3]);
		let right = run_gtao_upscale_fixture(&upscale, full_extent, &device_depth, low_extent, &linear_depth, &ao, [4, 3]);
		assert!(
			left[0] < 0.3 && right[0] > 0.7,
			"Expected reconstruction to preserve the AO edge, found left={left:?} and right={right:?}. The most likely cause is missing low-resolution depth rejection."
		);
	}

	#[test]
	fn shader_meshlet_data_matches_metal_buffer_layout() {
		assert_eq!(std::mem::align_of::<super::ShaderMeshletData>(), 16);
		assert_eq!(std::mem::size_of::<super::ShaderMeshletData>(), 64);
	}
}
