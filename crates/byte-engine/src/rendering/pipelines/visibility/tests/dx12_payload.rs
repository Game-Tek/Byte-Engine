//! Verifies the visibility task shader's packed meshlets on the DX12 task-to-mesh boundary.

use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	device::Device as _,
	frame::Frame as _,
	queue::{FrameRequest, Queue as _, QueueExecution as _},
};
use resource_management::{
	resources::material::SetInterface,
	shader::{
		ShaderGenerationSettings as Settings,
		besl::{
			backends::platform::{Generator, PlatformShaderLanguage},
			evaluation::ProgramEvaluation,
		},
	},
};
use utils::Extent;

use super::{WORK_HEADER, asset_program};
use crate::rendering::{
	SetLayout,
	pipelines::visibility::{
		layout::{ShaderMeshLevel, ShaderMeshletData},
		render_pass::OcclusionPhase,
		set_layouts::{OCCLUSION, SCENE},
		shader_data::{MESH_FLAG_SKINNED, ShaderMesh, ShaderViewData},
	},
};

const LANES: usize = 32;
const RECORDS: usize = LANES * 2;

/// One mesh workgroup reads its two entries as the visibility mesh shader does. It reports the lanes the
/// entries name so the test can detect missing or repeated meshlets regardless of compaction order.
const PAYLOAD_READER: &str = r#"
	use ::byte_engine::rendering::visibility::layouts::scene_records::*;

	meshlet_instances: task_payload<u32, 193>;
	seen: descriptor<{ type: u32[], binding: 0, access: write }>;

	main: fn () -> void {
		let group: u32 = threadgroup_position();
		// The DX12 mesh signature requires a position. Emit a degenerate triangle since the test reads the buffer.
		set_mesh_output_counts(3, 1);
		for (let vertex: u32 = 0; vertex < 3; vertex = vertex + 1) {
			set_mesh_vertex_position(vertex, vec4f(0.0, 0.0, 0.0, 1.0));
		}
		set_mesh_triangle(0, vec3u(0, 1, 2));
		if (group < 32) {
			let first: u32 = group_first_entry(meshlet_instances[group]);
			let entry: u32 = meshlet_instances[first];
			seen[group * 2] = entry_lane(entry) + 1;
			if ((entry & MESHLET_ENTRY_ENDS_GROUP) == 0 && first + 1 < MESHLET_PAYLOAD_WORK) {
				seen[group * 2 + 1] = entry_lane(meshlet_instances[first + 1]) + 1;
			}
		}
	}
"#;

/// Verifies that the real visibility task shader hands all 32 meshlets to the GPU mesh stage once. Two meshlets
/// fit in each grid, so the task has to write 16 distinct group starts into its payload.
#[test]
fn visibility_task_preserves_every_meshlet_in_a_full_dx12_workgroup() {
	let features = ghi::device::Features::new()
		.validation(true)
		.debug_log_function(|message| eprintln!("{message}"));
	let mut instance = ghi::implementation::Instance::new(features).expect("Create the DX12 test instance");
	let mut queue = None;
	let device = instance
		.create_device(
			features,
			&mut [(ghi::QueueSelection::new(ghi::WorkloadTypes::RASTER), &mut queue)],
		)
		.expect("Create a DX12 device with mesh shaders");
	let mut context = device.create_context().expect("Create the visibility test context");
	context.set_frames_in_flight(1);
	let queue = queue.expect("Select the test graphics queue");

	let (task, _) = shader(
		&mut context,
		"Visibility Task",
		crate::rendering::shader_vm_test::asset_source("visibility/meshlet-task.besl"),
		Settings::task(Extent::line(LANES as u32), 192),
		ghi::ShaderTypes::Task,
	);
	let (mesh, reader_sets) = shader(
		&mut context,
		"Visibility Payload Reader",
		PAYLOAD_READER,
		Settings::mesh(3, 1, Extent::line(1)),
		ghi::ShaderTypes::Mesh,
	);
	let pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
		&[ghi::pipelines::PushConstantRange::new(0, 7 * size_of::<u32>() as u32)],
		&[],
		&[
			ghi::pipelines::ShaderParameter::new(&task, ghi::ShaderTypes::Task),
			ghi::pipelines::ShaderParameter::new(&mesh, ghi::ShaderTypes::Mesh),
		],
		&[],
	));

	let scene_set = SCENE.create_set(&mut context, "Payload Test Scene");
	let occlusion_set = OCCLUSION.create_set(&mut context, "Payload Test Occlusion");
	let reader_layout = &reader_sets[0];
	let reader_set = reader_layout.create_set(&mut context, "Payload Test Output");
	let scene = SCENE.binder(scene_set);
	let occlusion = OCCLUSION.binder(occlusion_set);
	let output = context.build_buffer::<[u32; RECORDS]>(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.host_access(ghi::HostAccess::Read)
			.copies(ghi::Copies::PerFrame),
	);
	let view = ShaderViewData {
		view_projection: math::Matrix::identity().into(),
		near: 0.1,
		far: 100.0,
		..bytemuck::Zeroable::zeroed()
	};
	let mut mesh = ShaderMesh::new(math::Matrix::identity().into(), 0);
	mesh.set_level(0, MESH_FLAG_SKINNED);
	let sphere = [0.0, 0.0, 0.5, 1.0];
	let level = ShaderMeshLevel {
		bounding_sphere: sphere,
		meshlet_count: LANES as u32,
		..Default::default()
	};
	let meshlet = ShaderMeshletData::new(0, 0, [64, 100], sphere, sphere, [0.0, 0.0, 1.0], 2.0);
	// The first range starts after the header. Its one chunk names instance zero and meshlets 0 through 31.
	let mut work = [0u32; WORK_HEADER + 1];
	work[0] = WORK_HEADER as u32;
	let views = upload(&mut context, &[view]);
	let meshes = upload(&mut context, &[mesh]);
	let levels = upload(&mut context, &[level]);
	let meshlets = upload(&mut context, &[meshlet; LANES]);
	let work = upload(&mut context, &work);
	// The culling path is disabled. These bindings complete the task shader's occlusion set.
	let pyramid = context.build_image(ghi::image::Builder::new(ghi::Formats::R16F, ghi::Uses::Image).extent(Extent::square(1)));
	let visibility = upload(&mut context, &[0u32]);
	let pages = upload(&mut context, &[0u32]);
	context.write(&[
		scene.buffer("views", views),
		scene.buffer("meshes", meshes),
		scene.buffer("mesh_levels", levels),
		scene.buffer("meshlets", meshlets),
		scene.buffer("mesh_dispatch_work", work),
		occlusion.texture("occlusion_pyramid", pyramid),
		occlusion.buffer("occlusion_visibility", visibility),
		occlusion.buffer("page_hierarchy", pages),
		reader_layout.binder(reader_set).buffer("seen", output),
	]);

	let command_buffer = context.queue(queue).create_command_buffer(None);
	let synchronizer = context.create_synchronizer(None, true);
	context
		.queue(queue)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				let pass = recording.start_render_pass(Extent::square(1), &[]);
				let bound = pass
					.bind_raster_pipeline(pipeline)
					.bind_descriptor_sets(&[scene_set, occlusion_set, reader_set]);
				bound.write_push_constant(0, [0u32, 0, 0, 1, OcclusionPhase::Disabled as u32, 0, 0]);
				bound.dispatch_meshes(1, 1, 1);
				pass.end_render_pass();
			});
			[]
		});
	context.wait();
	assert!(
		!context.has_errors(),
		"The visibility payload dispatch triggered DX12 validation"
	);

	// Frame one takes back the output copy written by frame zero.
	let mut queue = context.queue(queue);
	let frame = queue.start_frame(1, synchronizer).frame;
	let mut seen = frame
		.read(output)
		.expect("Read back the completed mesh workgroups")
		.iter()
		.copied()
		.filter(|&lane| lane != 0)
		.collect::<Vec<_>>();
	seen.sort_unstable();
	assert!(
		seen.iter().copied().eq(1..=LANES as u32),
		"Each of the 32 meshlets must reach the mesh stage once. Read {seen:?}."
	);
}

/// Lowers the BESL stage and binds its reflected descriptor layouts to the DX12 shader.
fn shader(
	context: &mut ghi::implementation::Context,
	name: &str,
	source: &str,
	settings: Settings,
	stage: ghi::ShaderTypes,
) -> (ghi::ShaderHandle, Vec<SetLayout>) {
	let program = asset_program(source);
	let sets = ProgramEvaluation::from_program(&program)
		.expect("Reflect the payload test shader")
		.into_sets()
		.into_iter()
		.map(|set| SetLayout::new(&SetInterface::new(set, name)))
		.collect::<Vec<_>>();
	let lowered = Generator::new()
		.lower(&settings.name(name.to_owned()), &program)
		.expect("Lower the payload test shader to HLSL");
	let handle = context
		.create_shader(
			Some(name),
			ghi::shader::Sources::HLSL {
				source: &lowered.source,
				entry_point: PlatformShaderLanguage::Hlsl.entry_point(),
			},
			stage,
			sets.iter()
				.map(|set| ghi::ShaderDescriptorSet::with_used(set.layout().clone(), set.interface().used)),
		)
		.expect("Compile the payload test shader for DX12");
	(handle, sets)
}

/// Uploads fixture records as one storage buffer.
fn upload<T: bytemuck::Pod>(context: &mut ghi::implementation::Context, values: &[T]) -> ghi::BufferHandle<[T]> {
	context.build_buffer_with(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.length(values.len())
			.host_access(ghi::HostAccess::Write),
		|buffer: &mut [T]| buffer.copy_from_slice(values),
	)
}
