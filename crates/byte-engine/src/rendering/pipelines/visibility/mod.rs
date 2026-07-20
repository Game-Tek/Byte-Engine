#[doc(hidden)]
pub mod gpu_vertex_data_manager;
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
pub(crate) const VIEWS_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(400);
// ShaderMesh array stride includes tail padding from the CPU matrix alignment; shader Mesh structs carry matching padding.
pub(crate) const MESH_DATA_BUFFER_STRIDE: u32 = if cfg!(target_os = "macos") { 96 } else { 80 };
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
pub(crate) const VERTEX_NORMALS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(3),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(12);
pub(crate) const SKINNED_VERTICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(4),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(32);
pub(crate) const VERTEX_UV_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(5),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(8);
pub(crate) const VERTEX_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(6),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
pub(crate) const PRIMITIVE_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(7),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
pub(crate) const MESHLET_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(8),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(64);
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
);
pub(crate) const MATERIAL_OFFSET_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
);
pub(crate) const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
);
pub(crate) const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
)
.buffer_stride(16);
pub(crate) const MATERIAL_XY_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1037),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
)
.buffer_stride(8);
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
	use resource_management::shader::besl::evaluation::ProgramEvaluation;

	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const VIEWS_SLOT: ResourceSlot = ResourceSlot::new(0);
	const MESH_DATA_SLOT: ResourceSlot = ResourceSlot::new(1);
	const MATERIAL_COUNT_SLOT: ResourceSlot = ResourceSlot::new(1033);
	const MATERIAL_OFFSET_SLOT: ResourceSlot = ResourceSlot::new(1034);
	const MATERIAL_OFFSET_SCRATCH_SLOT: ResourceSlot = ResourceSlot::new(1035);
	const MATERIAL_DISPATCH_SLOT: ResourceSlot = ResourceSlot::new(1036);
	const PIXEL_MAPPING_SLOT: ResourceSlot = ResourceSlot::new(1037);
	const INSTANCE_INDEX_SLOT: ResourceSlot = ResourceSlot::new(1040);
	const VERTEX_POSITIONS_SLOT: ResourceSlot = ResourceSlot::new(2);
	const SKINNED_VERTICES_SLOT: ResourceSlot = ResourceSlot::new(4);
	const VERTEX_INDICES_SLOT: ResourceSlot = ResourceSlot::new(6);
	const PRIMITIVE_INDICES_SLOT: ResourceSlot = ResourceSlot::new(7);
	const MESHLETS_SLOT: ResourceSlot = ResourceSlot::new(8);
	const FIXTURE_INSTANCE_INDEX: usize = 3;
	const FIXTURE_MESHLET_INDEX: usize = 5;
	const TASK_WORKGROUP_SIZE: u32 = 32;
	const MESH_TEST_INSTRUCTION_LIMIT: usize = 4_000_000;

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

	/// Guards the flat resource ABI used before indirect material evaluation dispatches.
	#[test]
	fn visibility_material_prepasses_reflect_reachable_flat_resources() {
		assert_reflected_resources(
			material_count_program(),
			&[
				(1, "mesh_data"),
				(1033, "material_count_buffer"),
				(1040, "instance_index_render_target"),
			],
		);
		assert_reflected_resources(
			material_offset_program(),
			&[
				(1033, "material_count_buffer"),
				(1034, "material_offset_buffer"),
				(1035, "material_offset_scratch_buffer"),
				(1036, "material_evaluation_dispatches"),
			],
		);
		assert_reflected_resources(
			pixel_mapping_program(),
			&[
				(1, "mesh_data"),
				(1035, "material_offset_scratch_buffer"),
				(1037, "pixel_mapping_buffer"),
				(1040, "instance_index_render_target"),
			],
		);
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

	/// Returns a view-projection matrix that moves identity geometry outside the horizontal clip range.
	fn horizontally_translated_matrix(translation: f32) -> [f32; 16] {
		let mut matrix = identity_matrix();
		matrix[12] = translation;
		matrix
	}

	/// Executes an exact production task main as one workgroup over consecutive meshlets.
	fn run_meshlet_task_workgroup(
		program: &ExecutableProgram,
		view_projections: &[(usize, [f32; 16])],
		selected_view_index: Option<u32>,
		center_radii: &[[f32; 4]],
		skinned: bool,
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
				.write_indexed_field("views", view_index, "inverse_view", Value::Mat4F(identity_matrix()))
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
		push_constant
			.write("instance_index", Value::U32(FIXTURE_INSTANCE_INDEX as u32))
			.expect("Failed to initialize the task instance index. The most likely cause is a drifted push constant layout.");
		if let Some(view_index) = selected_view_index {
			push_constant
				.write("view_index", Value::U32(view_index))
				.expect("Failed to initialize the task view index. The most likely cause is a drifted push constant layout.");
		}

		let mut task_outputs = TaskOutputs::new();
		let mut workgroup_state = WorkgroupState::new();
		let configs = (0..TASK_WORKGROUP_SIZE)
			.map(|lane| {
				ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_call_depth_limit(128)
					.with_thread_idx(lane)
					.with_thread_position(lane)
			})
			.collect::<Vec<_>>();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
			descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
			descriptors.bind_push_constant(&mut push_constant);
			descriptors.bind_task_outputs(&mut task_outputs);
			descriptors.bind_workgroup_state(&mut workgroup_state);

			program.run_task_workgroup(&mut descriptors, &configs).expect(
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
			task_outputs.payload_value("meshlet_indices", 0).cloned(),
		)
	}

	/// Verifies view-zero culling retains an intersecting meshlet and rejects one outside the frustum.
	#[test]
	fn visibility_task_main_emits_in_frustum_and_culls_off_frustum_meshlets() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let visible = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [0.0, 0.0, 0.5, 0.1], false);
		assert_eq!(visible, (Some(1), Some(Value::U32(FIXTURE_MESHLET_INDEX as u32))));

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
			output.payload_value("meshlet_indices", 0),
			Some(&Value::U32(FIXTURE_MESHLET_INDEX as u32))
		);
		assert_eq!(
			output.payload_value("meshlet_indices", 1),
			Some(&Value::U32(FIXTURE_MESHLET_INDEX as u32 + 2))
		);
		assert_eq!(output.payload_value("meshlet_indices", 2), None);
	}

	/// Verifies deformed geometry reaches the mesh stage even when its static meshlet bound is outside the frustum.
	#[test]
	fn visibility_task_main_bypasses_static_culling_for_skinned_meshes() {
		let program = crate::rendering::shader_vm_test::compile(visibility_task_program());
		let output = run_single_meshlet_task(&program, &[(0, identity_matrix())], None, [4.0, 0.0, 0.5, 0.1], true);
		assert_eq!(output, (Some(1), Some(Value::U32(FIXTURE_MESHLET_INDEX as u32))));
	}

	/// Verifies shadow culling selects the cascade view named by the second push constant.
	#[test]
	fn shadow_task_main_uses_selected_view_index() {
		let program = crate::rendering::shader_vm_test::compile(shadow_task_program());
		let mut view_projections: [(usize, [f32; 16]); 8] =
			std::array::from_fn(|view_index| (view_index, horizontally_translated_matrix(4.0)));
		view_projections[3].1 = identity_matrix();
		let output = run_single_meshlet_task(&program, &view_projections, Some(3), [0.0, 0.0, 0.5, 0.1], false);
		assert_eq!(output, (Some(1), Some(Value::U32(FIXTURE_MESHLET_INDEX as u32))));
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
		selected_view: Option<(usize, [f32; 16])>,
		skinned_positions: Option<[[f32; 4]; 3]>,
		expected_clip_positions: [[f32; 4]; 3],
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
		push_constant
			.write("instance_index", Value::U32(FIXTURE_INSTANCE_INDEX as u32))
			.expect("Failed to initialize the mesh instance index. The most likely cause is a drifted push constant layout.");
		if let Some((view_index, view_projection)) = selected_view {
			views
				.write_indexed_field("views", view_index, "view_projection", Value::Mat4F(view_projection))
				.expect("Failed to initialize the selected mesh view. The most likely cause is a drifted View layout.");
			push_constant
				.write("view_index", Value::U32(view_index as u32))
				.expect("Failed to initialize the shadow view index. The most likely cause is a drifted push constant layout.");
		}

		let mut out_instance_indices = buffer(&program, output_slot(0));
		let mut out_primitive_indices = buffer(&program, output_slot(1));
		let mut mesh_outputs = MeshOutputs::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_task_payload("meshlet_indices", [Value::U32(FIXTURE_MESHLET_INDEX as u32)]);
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
		);
	}

	/// Verifies that posed instances source raster positions from their frame-local deformation range.
	#[test]
	fn visibility_mesh_main_reads_skinned_positions() {
		let skinned_positions = [[2.0, 3.0, 4.0, 1.0], [5.0, 6.0, 7.0, 1.0], [8.0, 9.0, 10.0, 1.0]];
		assert_triangle_mesh_program(visibility_mesh_program(), None, Some(skinned_positions), skinned_positions);
	}

	/// Verifies shadow mesh output geometry uses the selected cascade view rather than view zero.
	#[test]
	fn shadow_mesh_main_emits_selected_view_triangle_and_metadata() {
		assert_triangle_mesh_program(
			shadow_mesh_program(),
			Some((3, horizontally_translated_matrix(2.0))),
			None,
			[[1.0, -1.0, 0.0, 1.0], [3.0, -1.0, 0.0, 1.0], [2.0, 1.0, 0.0, 1.0]],
		);
	}

	/// Creates the minimum camera data shared by the GTAO shader fixtures.
	fn gtao_views(program: &ExecutableProgram) -> besl::vm::Buffer {
		let mut views = buffer(program, VIEWS_SLOT);
		views
			.write_indexed_field("views", 0, "inverse_projection", Value::Mat4F(identity_matrix()))
			.expect("Failed to initialize the GTAO inverse projection. The most likely cause is a drifted View layout.");
		views
			.write_indexed_field("views", 0, "fov", Value::Vec2F([60.0, 60.0]))
			.expect("Failed to initialize the GTAO field of view. The most likely cause is a drifted View layout.");
		views
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
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			for coordinate in [[0, 0], [1, 0], [0, 1], [1, 1]] {
				run_at(&material_count_program, &mut descriptors, coordinate);
			}
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
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 0),
			[0, 1, 1, 0]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 2),
			[1, 1, 1, 0]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 5),
			[1, 1, 1, 0]
		);

		// Mapping reuses the scratch offsets as atomic cursors and stores one-based coordinates for later zero-sentinel checks.
		let mut pixel_mapping = buffer(&pixel_mapping_program, PIXEL_MAPPING_SLOT);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			for coordinate in [[0, 0], [1, 0], [0, 1], [1, 1]] {
				run_at(&pixel_mapping_program, &mut descriptors, coordinate);
			}
		}

		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 0), [1, 1]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 1), [2, 2]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 2), [2, 1]);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 2), 2);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 5), 3);
	}

	/// Executes the standard GTAO shader with one deterministic depth fixture.
	fn run_gtao_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut views = gtao_views(program);
		let mut depth = texture_2d(width, height, depth_texels);
		let mut output = empty_image(width, height);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
			descriptors.bind_image(ResourceSlot::new(1034), &mut output);
			run_at(program, &mut descriptors, coordinate);
		}
		rgba(&output, coordinate)
	}

	/// Verifies the standard GTAO shader's background contract and bounded foreground output.
	#[test]
	fn gtao_writes_white_for_background_and_bounded_finite_foreground() {
		let program = crate::rendering::shader_vm_test::compile(gtao_program());
		let background = run_gtao_fixture(&program, 1, 1, &[[0.0, 0.0, 0.0, 1.0]], [0, 0]);
		assert_rgba_close(background, [1.0, 1.0, 1.0, 1.0], 0.00001);

		// A recessed center surrounded by nearer depth exercises reconstruction, normal estimation, and the bounded AO integral.
		let mut foreground = [[0.35, 0.0, 0.0, 1.0]; 25];
		foreground[12] = [0.75, 0.0, 0.0, 1.0];
		let foreground = run_gtao_fixture(&program, 5, 5, &foreground, [2, 2]);
		for channel in foreground[..3].iter().copied() {
			assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
		}
		assert_eq!(foreground[3], 1.0);
	}

	/// Compiles one checked-in axis-specific GTAO blur asset.
	fn compile_gtao_blur(source: &str) -> ExecutableProgram {
		crate::rendering::shader_vm_test::compile(asset_program(source))
	}

	/// Runs one generic GTAO blur invocation for a selected specialization direction.
	fn run_gtao_blur_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		ao_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut views = gtao_views(program);
		let mut depth = texture_2d(width, height, depth_texels);
		let mut ao = texture_2d(width, height, ao_texels);
		let mut output = empty_image(width, height);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_texture(ResourceSlot::new(1033), &mut depth);
			descriptors.bind_texture(ResourceSlot::new(1034), &mut ao);
			descriptors.bind_image(ResourceSlot::new(1035), &mut output);
			run_at(program, &mut descriptors, coordinate);
		}
		rgba(&output, coordinate)
	}

	/// Verifies the two production blur assets without disturbing uniform input.
	#[test]
	fn gtao_blur_preserves_uniform_ao_and_obeys_x_y_assets() {
		let blur_x = compile_gtao_blur(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-blur-x.besl"
		)));
		let blur_y = compile_gtao_blur(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/visibility/gtao-blur-y.besl"
		)));
		let depth = [[0.5, 0.0, 0.0, 1.0]; 25];
		let uniform_ao = [[0.37, 0.0, 0.0, 1.0]; 25];
		assert_rgba_close(
			run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &uniform_ao, [2, 2]),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);
		assert_rgba_close(
			run_gtao_blur_fixture(&blur_y, 5, 5, &depth, &uniform_ao, [2, 2]),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);

		// Horizontal variation is smoothed by the X asset, while every Y sample still observes the center column.
		let directional_ao: [[f32; 4]; 25] = std::array::from_fn(|index| {
			if index % 5 == 2 {
				[1.0, 0.0, 0.0, 1.0]
			} else {
				[0.0, 0.0, 0.0, 1.0]
			}
		});
		let horizontal = run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &directional_ao, [2, 2]);
		let vertical = run_gtao_blur_fixture(&blur_y, 5, 5, &depth, &directional_ao, [2, 2]);
		assert!(
			horizontal[0] < 0.8,
			"Expected X blur to mix neighboring columns, found {horizontal:?}"
		);
		assert!(
			(vertical[0] - 1.0).abs() < 0.00001,
			"Expected Y blur to preserve the center column, found {vertical:?}"
		);
	}

	#[test]
	fn shader_meshlet_data_matches_metal_buffer_layout() {
		assert_eq!(std::mem::align_of::<super::ShaderMeshletData>(), 16);
		assert_eq!(std::mem::size_of::<super::ShaderMeshletData>(), 64);
	}
}
