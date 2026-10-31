//! Draws with baked task, mesh, and fragment shaders on the GPU, and checks the image against the BESL contract.
//!
//! The shaders are baked as asset bakes bake them, with the platform generator's lowering and the platform's shader
//! compiler: MSL and the Metal compiler on macOS, GLSL and glslang on Linux. GHI draws with them into an unsigned
//! integer target. Each item the task stage keeps covers one row of it, so a lowering that loses a payload write,
//! emits the wrong number of mesh workgroups, or links a per-primitive value as an interpolated one changes the image.
//! Like the visibility pipeline's stages, both stages read buffers the host uploads in the same frame and a push
//! constant, and the task stage writes a buffer, which the host reads back.

use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		RasterizationRenderPassMode as _,
	},
	device::Device as _,
	queue::{FrameRequest, Queue as _, QueueExecution as _},
};
use utils::Extent;

use super::gpu_harness::{bake, create_baked_shader, gpu_context, upload};
use crate::shader::generator::ShaderGenerationSettings;

/// The push constant both stages read, as the visibility pipeline's task and mesh stages share theirs.
const PUSH_CONSTANT: &str = r#"
	push_constant: push_constant {
		culled_workgroup: u32,
		row_offset: u32,
	}
"#;

/// Keeps the items its lanes find marked in `keep`, compacts them into the payload through a workgroup counter, writes
/// how many it kept to `kept`, and emits one mesh workgroup per kept item. The workgroup the push constant names
/// returns early after it sets an empty grid, as the meshlet task shader does for an instance no view sees, so its
/// items never appear.
const TASK: &str = r#"
	keep: descriptor<{ type: u32[], binding: 0, access: read }>;
	kept: descriptor<{ type: u32[], binding: 2, access: write }>;
	visible_items: task_payload<u32, 32>;
	visible_count: workgroup<atomicu32>;

	main: fn () -> void {
		let lane: u32 = thread_idx();
		let workgroup: u32 = thread_position() / 32;
		if (workgroup == push_constant.culled_workgroup) {
			if (lane == 0) {
				kept[workgroup] = 0;
				set_task_mesh_output_count(0);
			}
			return;
		}
		if (lane == 0) {
			atomic_store(visible_count, 0);
		}
		workgroup_barrier();
		let item: u32 = workgroup * 32 + lane;
		if (keep[item] != 0) {
			let slot: u32 = atomic_add(visible_count, 1);
			visible_items[slot] = item;
		}
		workgroup_barrier();
		if (lane == 0) {
			let count: u32 = atomic_load(visible_count);
			kept[workgroup] = count;
			set_task_mesh_output_count(count);
		}
	}
"#;

/// Draws a quad over the row `rows` assigns the item its payload entry names, moved down by the push constant's
/// offset, with the item as a per-primitive output and a per-vertex value of one.
const MESH: &str = r#"
	rows: descriptor<{ type: u32[], binding: 1, access: read }>;
	visible_items: task_payload<u32, 32>;
	out_item: output<u32, 0, 2>;
	out_shade: vertex_output<f32, 1, 4>;

	main: fn () -> void {
		let lane: u32 = thread_idx();
		let item: u32 = visible_items[threadgroup_position()];
		let row: u32 = (rows[item] + push_constant.row_offset) % 64;
		set_mesh_output_counts(4, 2);
		if (lane < 4) {
			// Bit 0 of a corner picks its side, and bit 1 its edge of the item's row.
			let top: f32 = f32(row) / 32.0 - 1.0;
			set_mesh_vertex_position(lane, vec4f(f32(lane & 1) * 2.0 - 1.0, top + f32(lane >> 1) / 32.0, 0.5, 1.0));
			out_shade[lane] = 1.0;
		}
		if (lane < 2) {
			set_mesh_triangle(lane, vec3u(lane, 1 + lane * 2, 2));
			out_item[lane] = item;
		}
	}
"#;

/// Writes the item its primitive carries, plus 1000 for each unit of the per-vertex value.
const FRAGMENT: &str = r#"
	main: fn (pipeline_input: interface { item: primitive<u32>, shade: f32 }) -> output { id: u32 } {
		return { id: pipeline_input.item + 1000 * u32(pipeline_input.shade + 0.5) };
	}
"#;

/// The task workgroups the draw dispatches, of [`TASK_LANES`] items each.
const TASK_WORKGROUPS: u32 = 2;

/// The invocations of a task workgroup, which [`TASK`] assumes.
const TASK_LANES: u32 = 32;

/// The target's rows, one for each item of every task workgroup.
const ROWS: u32 = TASK_WORKGROUPS * TASK_LANES;

/// The target's columns, which every quad spans.
const COLUMNS: u32 = 4;

/// The task workgroup that returns early, which the push constant names.
const CULLED_WORKGROUP: u32 = 1;

/// How many rows the mesh stage moves every quad down, which the push constant carries.
const ROW_OFFSET: u32 = 7;

/// Whether the task stage keeps `item`: every item of the second workgroup, which returns before it reads them, and
/// the first workgroup's items that aren't multiples of three.
fn keeps(item: u32) -> bool {
	item >= TASK_LANES || !item.is_multiple_of(3)
}

/// The row `rows` assigns `item`: 5 is coprime with the row count, so every item gets a row of its own, out of order.
fn row_of(item: u32) -> u32 {
	item * 5 % ROWS
}

/// Returns what each row of the target holds once the draw completes: the item it shows plus the per-vertex value's
/// 1000 for a row of an item the first task workgroup keeps, and the cleared zero elsewhere.
fn expected_rows() -> Vec<u32> {
	let mut rows = vec![0; ROWS as usize];
	for item in (0..TASK_LANES).filter(|item| keeps(*item)) {
		rows[((row_of(item) + ROW_OFFSET) % ROWS) as usize] = 1000 + item;
	}
	rows
}

/// Links a standalone BESL source, after the push constant every fixture stage declares.
fn link(source: &str) -> besl::NodeReference {
	besl::compile_to_besl(&format!("{PUSH_CONSTANT}{source}"), None).expect("Expected the mesh shading fixture to link")
}

/// Verifies baked task, mesh, and fragment shaders draw what BESL specifies: the task stage's payload and mesh grid,
/// including a workgroup that returns early, and per-primitive and per-vertex values that reach the fragment stage.
#[compio::test]
async fn generated_task_mesh_and_fragment_shaders_draw_the_kept_items() {
	let task = bake(
		&link(TASK),
		ShaderGenerationSettings::task(Extent::line(TASK_LANES), TASK_LANES).name("matched_task".to_string()),
	)
	.await;
	let mesh = bake(
		&link(MESH),
		ShaderGenerationSettings::mesh(4, 2, Extent::line(4)).name("matched_mesh".to_string()),
	)
	.await;
	let fragment = bake(
		&link(FRAGMENT),
		ShaderGenerationSettings::fragment().name("matched_fragment".to_string()),
	)
	.await;

	let (rows, kept) = draw_on_the_gpu(&task, &mesh, &fragment);

	assert_eq!(
		kept,
		[(0..TASK_LANES).filter(|item| keeps(*item)).count() as u32, 0],
		"The task workgroups wrote other kept counts."
	);
	let expected = expected_rows();
	// Clip space points up on Metal and down on Vulkan, so either orientation of the rows is the same image.
	let flipped = expected.iter().rev().copied().collect::<Vec<_>>();
	assert!(
		rows == expected || rows == flipped,
		"The GPU drew rows {rows:?}, not {expected:?} in either orientation."
	);
}

/// Draws with the baked shaders on a new GPU device, with validation, and returns the value each row of the target
/// holds, which must be the same in every pixel of the row, and the counts the task workgroups wrote.
fn draw_on_the_gpu(task: &[u8], mesh: &[u8], fragment: &[u8]) -> (Vec<u32>, Vec<u32>) {
	let (_instance, _device, mut context, queue_handle) =
		gpu_context(ghi::device::Features::new().validation(true), ghi::WorkloadTypes::RASTER);

	use ghi::context::{Context as _, ContextCreate as _};
	// The task stage reads `keep` and writes `kept`, and the mesh stage reads `rows`, so each declares only its own.
	let storage = |slot: u32, access: ghi::AccessPolicies| {
		ghi::ShaderResourceDescriptor::single(ghi::ResourceSlot::new(slot), ghi::ResourceKind::StorageBuffer, access)
	};
	let layout = ghi::DescriptorSetLayout::new(
		"Mesh Shading",
		[
			storage(0, ghi::AccessPolicies::READ),
			storage(1, ghi::AccessPolicies::READ),
			storage(2, ghi::AccessPolicies::WRITE),
		],
	);
	let task = create_baked_shader(
		&mut context,
		"Matched Task",
		task,
		ghi::ShaderTypes::Task,
		Some(Extent::line(TASK_LANES)),
		[ghi::ShaderDescriptorSet::with_used(layout.clone(), 0b101)],
	);
	let mesh = create_baked_shader(
		&mut context,
		"Matched Mesh",
		mesh,
		ghi::ShaderTypes::Mesh,
		Some(Extent::line(4)),
		[ghi::ShaderDescriptorSet::with_used(layout.clone(), 0b010)],
	);
	let fragment = create_baked_shader(
		&mut context,
		"Matched Fragment",
		fragment,
		ghi::ShaderTypes::Fragment,
		None,
		[ghi::ShaderDescriptorSet::with_used(layout.clone(), 0)],
	);
	let pipeline = context.create_raster_pipeline(
		ghi::pipelines::raster::Builder::new(
			&[ghi::pipelines::PushConstantRange::new(0, 2 * size_of::<u32>() as u32)],
			&[],
			&[
				ghi::pipelines::ShaderParameter::new(&task, ghi::ShaderTypes::Task),
				ghi::pipelines::ShaderParameter::new(&mesh, ghi::ShaderTypes::Mesh),
				ghi::pipelines::ShaderParameter::new(&fragment, ghi::ShaderTypes::Fragment),
			],
			&[ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::U32)],
		)
		.cull_mode(ghi::pipelines::raster::CullMode::None),
	);
	let extent = Extent::rectangle(COLUMNS, ROWS);
	let target = context.build_image(
		ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::TransferSource)
			.extent(extent)
			.device_accesses(ghi::DeviceAccesses::DeviceToHost),
	);

	let keep = upload(
		&mut context,
		ghi::Uses::Storage,
		&(0..ROWS).map(|item| u32::from(keeps(item))).collect::<Vec<_>>(),
	);
	let rows = upload(&mut context, ghi::Uses::Storage, &(0..ROWS).map(row_of).collect::<Vec<_>>());
	let kept = context.build_buffer::<[u32]>(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.length(TASK_WORKGROUPS as usize)
			.device_accesses(ghi::DeviceAccesses::DeviceToHost),
	);
	let set = context.create_descriptor_set(None, &layout);
	context.write(&[
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(0), keep.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), rows.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(2), kept.into()),
	]);

	let command_buffer = context.queue(queue_handle).create_command_buffer(None);
	let synchronizer = context.create_synchronizer(None, true);
	let mut readback = None;
	context
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				let attachments = [ghi::AttachmentInformation::new(
					target,
					ghi::Layouts::RenderTarget,
					ghi::LoadOp::Clear(ghi::ClearValue::Integer(0, 0, 0, 0)),
					ghi::StoreOp::Store,
				)];
				let pass = recording.start_render_pass(extent, &attachments);
				let bound = pass.bind_raster_pipeline(pipeline).bind_descriptor_sets(&[set]);
				bound.write_push_constant(0, [CULLED_WORKGROUP, ROW_OFFSET]);
				bound.dispatch_meshes(TASK_WORKGROUPS, 1, 1);
				pass.end_render_pass();
				readback = Some(
					recording
						.transfer_texture(target.into())
						.expect("Failed to read back the target. The most likely cause is an unsupported transfer source."),
				);
			});
			[]
		});
	context.wait();
	assert!(
		!context.has_errors(),
		"The mesh shading draw broke a GPU validation rule. The most likely cause is a lowering that links the stages wrongly; the validation messages above name it."
	);

	let kept = context.get_buffer_slice(kept).to_vec();
	let pixels = context
		.get_image_data(readback.expect("The recording must have queued the readback."))
		.expect("Failed to map the readback. The most likely cause is incomplete GPU readback.");
	let rows = (0..ROWS as usize)
		.map(|row| {
			let values = (0..COLUMNS as usize)
				.map(|column| {
					let offset = row * pixels.bytes_per_row + column * 4;
					u32::from_le_bytes(pixels.bytes[offset..offset + 4].try_into().unwrap())
				})
				.collect::<Vec<_>>();
			assert!(
				values.iter().all(|value| *value == values[0]),
				"Row {row} holds {values:?}, but every quad spans the whole row."
			);
			values[0]
		})
		.collect();
	(rows, kept)
}
