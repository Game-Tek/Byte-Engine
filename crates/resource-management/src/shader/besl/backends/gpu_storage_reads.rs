//! Reads storage buffers with a baked compute shader on the GPU, and checks every value against what the host wrote.
//!
//! The buffers hold the element types the engine's geometry and transform data use: `vec3f` positions, `u16` vertex
//! indices, `u8` triangle indices, a struct with a `vec2f16`, a struct that starts with a `mat4x3f`, the view record's
//! `mat4f` and `mat4x3f`, and matrix arrays held by one struct, as the simple pipeline's cameras and instance transforms
//! and the skinning joint matrices are. The host writes them in the byte layout the engine uploads, and binds them
//! with the strides reflection gives the engine. So a lowering or a binding that reads an element at the wrong
//! offset, unpacks a narrow element wrongly, or reads a matrix transposed changes a value.

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	device::Device as _,
	queue::{FrameRequest, Queue as _, QueueExecution as _},
};
use utils::Extent;

use super::gpu_harness::{bake, create_baked_shader, download, gpu_context, upload};
use crate::shader::generator::ShaderGenerationSettings;

/// Copies one element of each input to the outputs: floats to `floats`, and integers to `integers`.
const READER: &str = r#"
	Vertex: struct {
		position: vec2u,
		uv: vec2f16,
	}
	Mesh: struct {
		model: mat4x3f,
		level: u32,
		material: u32,
		surface: vec2u,
	}
	View: struct {
		view: mat4x3f,
		view_projection: mat4f,
		inverse_view: mat4x3f,
		fov: vec2f,
		near: f32,
		far: f32,
	}
	Camera: struct {
		view_projection: mat4f,
	}
	Cameras: struct {
		cameras: Camera[8],
	}
	Instances: struct {
		transforms: mat4x3f[8],
	}
	positions: descriptor<{ type: vec3f[], binding: 0, access: read }>;
	wide_indices: descriptor<{ type: u16[], binding: 1, access: read }>;
	narrow_indices: descriptor<{ type: u8[], binding: 2, access: read }>;
	vertices: descriptor<{ type: Vertex[], binding: 3, access: read }>;
	meshes: descriptor<{ type: Mesh[], binding: 4, access: read }>;
	floats: descriptor<{ type: f32[], binding: 5, access: write }>;
	integers: descriptor<{ type: u32[], binding: 6, access: write }>;
	views: descriptor<{ type: View[], binding: 7, access: read }>;
	cameras: descriptor<{ type: Cameras, binding: 8, access: read, memory: constant }>;
	instances: descriptor<{ type: Instances, binding: 9, access: read, memory: device }>;
	// The view values follow the eight floats of every item; see `ITEMS`.
	FIRST_VIEW_FLOAT: const u32 = 56;
	FIRST_CAMERA_FLOAT: const u32 = 112;
	ITEM_COUNT: const u32 = 7;

	main: fn (input: StageInput) -> void {
		let item: u32 = input.thread_id.x;
		// The last workgroup has lanes past the last item, which must not write over another item's values.
		if (item >= ITEM_COUNT) {
			return;
		}
		let position: vec3f = positions[item];
		let vertex: Vertex = vertices[item];
		let mesh: Mesh = meshes[item];
		let moved: vec3f = mesh.model * vec4f(1.0, 2.0, 3.0, 1.0);
		let view: View = views[item];
		let clip: vec4f = view.view_projection * vec4f(1.0, 2.0, 3.0, 1.0);
		let viewed: vec3f = view.view * vec4f(1.0, 2.0, 3.0, 1.0);
		// The simple pipeline reads its matrices from arrays inside one struct, not from arrays of structs.
		let camera_clip: vec4f = cameras.cameras[item].view_projection * vec4f(1.0, 2.0, 3.0, 1.0);
		let transform: mat4x3f = instances.transforms[item];
		let placed: vec3f = transform * vec4f(1.0, 2.0, 3.0, 1.0);
		floats[FIRST_CAMERA_FLOAT + item * 8] = camera_clip.x;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 1] = camera_clip.y;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 2] = camera_clip.z;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 3] = camera_clip.w;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 4] = placed.x;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 5] = placed.y;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 6] = placed.z;
		floats[FIRST_CAMERA_FLOAT + item * 8 + 7] = 7.0;
		floats[item * 8] = position.x;
		floats[item * 8 + 1] = position.y;
		floats[item * 8 + 2] = position.z;
		floats[item * 8 + 3] = f32(vertex.uv.x);
		floats[item * 8 + 4] = f32(vertex.uv.y);
		floats[item * 8 + 5] = moved.x;
		floats[item * 8 + 6] = moved.y;
		floats[item * 8 + 7] = moved.z;
		floats[FIRST_VIEW_FLOAT + item * 8] = clip.x;
		floats[FIRST_VIEW_FLOAT + item * 8 + 1] = clip.y;
		floats[FIRST_VIEW_FLOAT + item * 8 + 2] = clip.z;
		floats[FIRST_VIEW_FLOAT + item * 8 + 3] = clip.w;
		floats[FIRST_VIEW_FLOAT + item * 8 + 4] = viewed.x;
		floats[FIRST_VIEW_FLOAT + item * 8 + 5] = viewed.y;
		floats[FIRST_VIEW_FLOAT + item * 8 + 6] = viewed.z;
		floats[FIRST_VIEW_FLOAT + item * 8 + 7] = view.far;
		integers[item * 6] = u32(wide_indices[item]);
		integers[item * 6 + 1] = u32(narrow_indices[item]);
		integers[item * 6 + 2] = vertex.position.x;
		integers[item * 6 + 3] = vertex.position.y;
		integers[item * 6 + 4] = mesh.level;
		integers[item * 6 + 5] = mesh.surface.y;
	}
"#;

/// The elements of every input, and the invocations that read them. Odd, so the last narrow element shares its word
/// with nothing.
const ITEMS: u32 = 7;

// The reader's `FIRST_VIEW_FLOAT` puts the view values after every item's first eight floats.
const _: () = assert!(ITEMS * 8 == 56);
// Its `FIRST_CAMERA_FLOAT` puts the camera values after the view values.
const _: () = assert!(ITEMS * 16 == 112);
// Its `ITEM_COUNT` stops the lanes past the last item.
const _: () = assert!(ITEMS == 7);

/// The invocations of a workgroup.
const LANES: u32 = 4;

/// The `Vertex` element as the host writes it: two words, then two half floats.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Vertex {
	position: [u32; 2],
	uv: [u16; 2],
}

/// The `Mesh` element as the host writes it: a column-major `mat4x3f`, then the words that follow it.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Mesh {
	model: [[f32; 3]; 4],
	level: u32,
	material: u32,
	surface: [u32; 2],
}

/// The `View` element as the host writes it: column-major matrices, then the words that follow them.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct View {
	view: [[f32; 3]; 4],
	view_projection: [[f32; 4]; 4],
	inverse_view: [[f32; 3]; 4],
	fov: [f32; 2],
	near: f32,
	far: f32,
}

/// The IEEE half-float bits of the small whole numbers [`ITEMS`] covers, and of one half.
const HALF_WHOLE_NUMBERS: [u16; ITEMS as usize] = [0x0000, 0x3C00, 0x4000, 0x4200, 0x4400, 0x4500, 0x4600];
const HALF_ONE_HALF: u16 = 0x3800;

fn position(item: u32) -> [f32; 3] {
	let item = item as f32;
	[item, item + 0.25, -item]
}

fn vertex(item: u32) -> Vertex {
	Vertex {
		position: [item * 7 + 1, item * 11 + 2],
		uv: [HALF_WHOLE_NUMBERS[item as usize], HALF_ONE_HALF],
	}
}

/// A mesh that moves by `(item, 2 * item, 3 * item)`, so it moves `(1, 2, 3)` to `(1 + item, 2 + 2 * item, 3 + 3 * item)`.
fn mesh(item: u32) -> Mesh {
	let offset = item as f32;
	Mesh {
		model: [
			[1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, 0.0, 1.0],
			[offset, 2.0 * offset, 3.0 * offset],
		],
		level: item + 5,
		material: 0,
		surface: [0, item * 3 + 9],
	}
}

/// A view whose matrices hold a distinct value in every entry, so a transposed or shifted read changes the product.
fn view(item: u32) -> View {
	let offset = item as f32;
	View {
		view: std::array::from_fn(|column| std::array::from_fn(|row| 10.0 + (column * 3 + row) as f32 + offset)),
		view_projection: std::array::from_fn(|column| std::array::from_fn(|row| 1.0 + (column * 4 + row) as f32 + offset)),
		inverse_view: [[0.0; 3]; 4],
		fov: [1.0, 1.0],
		near: 0.1,
		far: 100.0 + offset,
	}
}

/// Multiplies a column-major matrix of `R`-component columns by `(1, 2, 3, 1)`.
fn times_point<const R: usize>(columns: &[[f32; R]; 4]) -> [f32; R] {
	std::array::from_fn(|row| columns[0][row] + 2.0 * columns[1][row] + 3.0 * columns[2][row] + columns[3][row])
}

/// Verifies a baked compute shader reads every element of the geometry stream types at the offset and with the value
/// the host wrote, with the strides reflection gives the engine.
#[compio::test]
async fn generated_compute_shaders_read_geometry_stream_elements() {
	let builtins = besl::Builtins::new();
	let mut program =
		besl::compile_to_besl(READER, besl::Program::new(&builtins)).expect("Expected the storage reader fixture to link");
	let reader = bake(
		&mut program,
		ShaderGenerationSettings::compute(Extent::line(LANES)).name("storage_reader".to_string()),
	)
	.await;
	let bindings = crate::shader::besl::evaluation::collect_bindings(&program, &program.root())
		.expect("Expected the storage reader fixture's bindings to reflect");

	let (_instance, _device, mut context, queue_handle) =
		gpu_context(ghi::device::Features::new().validation(true), ghi::WorkloadTypes::COMPUTE);
	use ghi::context::{Context as _, ContextCreate as _};
	// Bind every buffer with the stride reflection reports, as the engine's set layouts do.
	let layout = ghi::DescriptorSetLayout::new(
		"Storage Reads",
		bindings.iter().map(|binding| {
			let access = if binding.write {
				ghi::AccessPolicies::WRITE
			} else {
				ghi::AccessPolicies::READ
			};
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(binding.slot),
				ghi::ResourceKind::StorageBuffer,
				access,
			)
			.buffer_stride(
				binding
					.buffer_stride
					.expect("Expected every storage buffer binding to reflect a stride"),
			)
		}),
	);
	let shader = create_baked_shader(
		&mut context,
		"Storage Reader",
		&reader,
		ghi::ShaderTypes::Compute,
		Some(Extent::line(LANES)),
		[ghi::ShaderDescriptorSet::new(layout.clone())],
	);
	let pipeline = context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[],
		ghi::pipelines::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	));

	let items = 0..ITEMS;
	let positions = upload(
		&mut context,
		ghi::Uses::Storage,
		&items.clone().map(position).collect::<Vec<_>>(),
	);
	let wide = upload(
		&mut context,
		ghi::Uses::Storage,
		&items.clone().map(|item| 1000 + item as u16 * 3).collect::<Vec<_>>(),
	);
	let narrow = upload(
		&mut context,
		ghi::Uses::Storage,
		&items.clone().map(|item| 200 + item as u8).collect::<Vec<_>>(),
	);
	let vertices = upload(
		&mut context,
		ghi::Uses::Storage,
		&items.clone().map(vertex).collect::<Vec<_>>(),
	);
	let meshes = upload(&mut context, ghi::Uses::Storage, &items.clone().map(mesh).collect::<Vec<_>>());
	// One `Cameras` and one `Instances` value, whose array element `item` the invocation of that item reads.
	let cameras = upload(
		&mut context,
		ghi::Uses::Storage,
		&[std::array::from_fn::<_, 8, _>(|item| view(item as u32).view_projection)],
	);
	let instances = upload(
		&mut context,
		ghi::Uses::Storage,
		&[std::array::from_fn::<_, 8, _>(|item| view(item as u32).view)],
	);
	let views = upload(&mut context, ghi::Uses::Storage, &items.clone().map(view).collect::<Vec<_>>());
	let output = |context: &mut ghi::implementation::Context, length: u32| {
		context.build_buffer::<[u32]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.length(length as usize)
				.host_access(ghi::HostAccess::Read),
		)
	};
	let floats = output(&mut context, ITEMS * 24);
	let integers = output(&mut context, ITEMS * 6);
	let set = context.create_descriptor_set(None, &layout);
	context.write(&[
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(0), positions.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), wide.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(2), narrow.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(3), vertices.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(4), meshes.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(5), floats.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(6), integers.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(7), views.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(8), cameras.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(9), instances.into()),
	]);

	let command_buffer = context.queue(queue_handle).create_command_buffer(None);
	let synchronizer = context.create_synchronizer(None, true);
	context
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				let recording = recording.bind_compute_pipeline(pipeline);
				recording.bind_descriptor_sets(&[set]);
				recording.dispatch(ghi::DispatchExtent::new(Extent::line(ITEMS), Extent::line(LANES)));
			});
			[]
		});
	context.wait();
	assert!(
		!context.has_errors(),
		"The storage reads broke a GPU validation rule. The most likely cause is a binding whose stride disagrees with the shader; the validation messages above name it."
	);

	let floats = download(&mut context, floats)
		.into_iter()
		.map(f32::from_bits)
		.collect::<Vec<_>>();
	let integers = download(&mut context, integers);
	for item in items {
		let [x, y, z] = position(item);
		let offset = item as f32;
		assert_eq!(
			floats[item as usize * 8..][..8],
			[x, y, z, offset, 0.5, 1.0 + offset, 2.0 + 2.0 * offset, 3.0 + 3.0 * offset],
			"Item {item} read the wrong positions, half floats, or matrix."
		);
		let view = view(item);
		let [clip_x, clip_y, clip_z, clip_w] = times_point(&view.view_projection);
		let [view_x, view_y, view_z] = times_point(&view.view);
		assert_eq!(
			floats[(ITEMS + item) as usize * 8..][..8],
			[clip_x, clip_y, clip_z, clip_w, view_x, view_y, view_z, view.far],
			"Item {item} read the wrong view matrices or the words after them."
		);
		assert_eq!(
			floats[(2 * ITEMS + item) as usize * 8..][..8],
			[clip_x, clip_y, clip_z, clip_w, view_x, view_y, view_z, 7.0],
			"Item {item} read the wrong matrix from an array inside a struct buffer."
		);
		let vertex = vertex(item);
		let mesh = mesh(item);
		assert_eq!(
			integers[item as usize * 6..][..6],
			[
				1000 + item * 3,
				200 + item,
				vertex.position[0],
				vertex.position[1],
				mesh.level,
				mesh.surface[1]
			],
			"Item {item} read the wrong narrow indices or struct words."
		);
	}
}
