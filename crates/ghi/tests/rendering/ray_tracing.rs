//! Ray tracing scenarios: bottom-level structures built once, top-level structures built every frame, rays traced
//! through them across compaction, destruction, and idle stretches, and ray tracing pipelines that run the hit group a
//! shader binding table names for each hit.
//!
//! The structure scenarios trace with [`Tracer`]: one compute thread per ray, along +z from an origin the scenario
//! picks, with an inline intersector whose report the scenario reads back. The pipeline scenarios use [`Shading`]: a ray
//! generation shader, launched with `trace_rays`, that traces the same rays and invokes the closest hit of each hit's
//! record.

use super::*;

/// The `u32` words the probe kernel writes for each ray: whether it hit, the bits of the hit distance, the instance's
/// custom index, the instance's index, the geometry index, the primitive index, the hit record, and padding.
type ProbeWords = [u32; 8];

/// The probe kernel. Each thread traces one ray along +z from its origin and writes what the intersector reports.
///
/// Only an intersector tagged `intersection_function_buffer` reports the hit record (`function_id`). Opaque geometry
/// calls no intersection function, so the default, empty function buffer is enough.
const PROBE_SHADER: &str = r#"
#include <metal_stdlib>
#include <metal_raytracing>
using namespace metal;
using namespace raytracing;

struct Resources {
	instance_acceleration_structure scene [[id(0)]];
	device const float4* origins [[id(1)]];
	device uint* hits [[id(2)]];
};

kernel void probe_main(uint index [[thread_position_in_grid]], constant Resources& resources [[buffer(16)]]) {
	intersector<instancing, triangle_data, intersection_function_buffer> trace;
	const ray probe(resources.origins[index].xyz, float3(0.0, 0.0, 1.0), 0.0, 100.0);
	const auto result = trace.intersect(probe, resources.scene, intersection_function_buffer_arguments());
	const bool hit = result.type != intersection_type::none;
	device uint* words = resources.hits + index * 8;
	words[0] = hit ? 1u : 0u;
	words[1] = as_type<uint>(hit ? result.distance : -1.0f);
	words[2] = result.user_instance_id;
	words[3] = result.instance_id;
	words[4] = result.geometry_id;
	words[5] = result.primitive_id;
	words[6] = hit ? result.function_id : 0u;
	words[7] = 0u;
}
"#;

/// The `Hit` struct is what the probe kernel reports for a ray that hit a triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
	distance: f32,
	custom_index: u32,
	instance: u32,
	geometry: u32,
	primitive: u32,
	record: u32,
}

impl Hit {
	/// Decodes one ray's words, or returns `None` for a ray that hit nothing.
	fn decode(words: ProbeWords) -> Option<Self> {
		(words[0] == 1).then(|| Self {
			distance: f32::from_bits(words[1]),
			custom_index: words[2],
			instance: words[3],
			geometry: words[4],
			primitive: words[5],
			record: words[6],
		})
	}
}

/// The `Rays` struct is one batch of probe rays: the set that names their origins and the structure they trace, and
/// the buffer their reports land in.
struct Rays {
	set: DescriptorSetHandle,
	hits: BufferHandle<[ProbeWords]>,
	count: u32,
}

impl Rays {
	/// Reads what each ray reported. Call it after the frame that traced the rays completed.
	fn read(&self, device: &mut impl ghi::context::Context) -> Vec<Option<Hit>> {
		device
			.get_buffer_slice(self.hits)
			.iter()
			.map(|words| Hit::decode(*words))
			.collect()
	}
}

/// The `Tracer` struct records the frames of a ray tracing scenario: their acceleration structure builds and the
/// probe rays that trace the structures.
struct Tracer {
	pipeline: PipelineHandle,
	layout: ghi::DescriptorSetLayout,
	queue_handle: QueueHandle,
	command_buffer: CommandBufferHandle,
	synchronizer: SynchronizerHandle,
}

impl Tracer {
	fn new(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) -> Self {
		let layout = ghi::DescriptorSetLayout::new(
			"Ray Probe",
			[
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(0),
					ghi::ResourceKind::AccelerationStructure,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(1),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(2),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::WRITE,
				),
			],
		);
		let shader = device
			.create_shader(
				Some("Ray Probe"),
				ghi::shader::Sources::MTL {
					source: PROBE_SHADER,
					entry_point: "probe_main",
				},
				ShaderTypes::Compute,
				[ghi::ShaderDescriptorSet::new(layout.clone())],
			)
			.expect("Failed to create the ray probe shader. The most likely cause is invalid MSL in the probe kernel.");
		let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
			&[],
			ShaderParameter::new(&shader, ShaderTypes::Compute),
		));
		let command_buffer = device.queue(queue_handle).create_command_buffer(None);
		Self {
			pipeline,
			layout,
			queue_handle,
			command_buffer,
			synchronizer: device.create_synchronizer(None, true),
		}
	}

	/// Creates a batch of probe rays that trace `structure` from each of `origins`, with a set and a result buffer of
	/// their own.
	fn rays(
		&self,
		device: &mut impl ghi::context::Context,
		structure: TopLevelAccelerationStructureHandle,
		origins: &[[f32; 3]],
	) -> Rays {
		let origin_buffer = device.build_buffer::<[[f32; 4]]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.name("Ray Probe Origins")
				.length(origins.len())
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		for (padded, origin) in device.get_mut_buffer_slice(origin_buffer).iter_mut().zip(origins) {
			*padded = [origin[0], origin[1], origin[2], 0.0];
		}
		device.sync_buffer(origin_buffer);
		let hits = device.build_buffer::<[ProbeWords]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.name("Ray Probe Hits")
				.length(origins.len())
				.device_accesses(DeviceAccesses::DeviceToHost),
		);
		let set = device.create_descriptor_set(None, &self.layout);
		device.write(&[
			ghi::DescriptorWrite::acceleration_structure(set, ghi::ResourceSlot::new(0), structure),
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), origin_buffer.into()),
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(2), hits.into()),
		]);
		Rays {
			set,
			hits,
			count: origins.len() as u32,
		}
	}

	/// Records frame `index` and submits it without waiting for it: the bottom-level builds, then the top-level build,
	/// then a trace of each batch of rays.
	fn frame(
		&self,
		device: &mut impl ghi::context::Context,
		index: u64,
		bottom_levels: &[BottomLevelAccelerationStructureHandle],
		top_level: Option<(TopLevelAccelerationStructureHandle, &[rt::Instance])>,
		batches: &[&Rays],
	) {
		device.queue(self.queue_handle).execute(
			Some(FrameRequest::new(index, self.synchronizer)),
			&[],
			self.synchronizer,
			|execution| {
				execution.record(self.command_buffer, |recording| {
					recording.build_bottom_level_acceleration_structures(bottom_levels);
					if let Some((structure, instances)) = top_level {
						recording.build_top_level_acceleration_structure(structure, instances);
					}
					for rays in batches {
						recording
							.bind_compute_pipeline(self.pipeline)
							.bind_descriptor_sets(&[rays.set])
							.dispatch(DispatchExtent::new(Extent::line(rays.count), Extent::line(rays.count)));
					}
				});
				[]
			},
		);
	}
}

/// Copies `values` into a new buffer that acceleration structure builds read.
fn geometry_buffer<T: Pod>(device: &mut impl ghi::context::Context, values: &[T]) -> BufferHandle<[T]> {
	let buffer = device.build_buffer::<[T]>(
		ghi::buffer::Builder::new(Uses::AccelerationStructureBuild)
			.length(values.len())
			.device_accesses(DeviceAccesses::HostToDevice),
	);
	device.get_mut_buffer_slice(buffer).copy_from_slice(values);
	device.sync_buffer(buffer);
	buffer
}

/// Returns the corners of a triangle in the z = 5 plane around (x, 0), which a probe ray from (x, 0, 0) hits at
/// distance 5.
fn triangle_at(x: f32) -> [[f32; 3]; 3] {
	[[x - 1.0, -1.0, 5.0], [x + 1.0, -1.0, 5.0], [x, 1.0, 5.0]]
}

/// Returns a geometry with one triangle around each x in `centers`, from packed positions and 16-bit indices.
fn triangles(device: &mut impl ghi::context::Context, centers: &[f32]) -> rt::TriangleGeometry {
	let positions = centers.iter().flat_map(|&x| triangle_at(x)).collect::<Vec<_>>();
	let indices = (0..positions.len() as u16).collect::<Vec<_>>();
	let position_buffer = geometry_buffer(device, &positions);
	let index_buffer = geometry_buffer(device, &indices);
	rt::TriangleGeometry {
		positions: BufferStridedRange::new(position_buffer.into(), 0, 12, positions.len() * 12),
		indices: index_buffer.into(),
		index_format: DataTypes::U16,
		triangle_count: centers.len() as u32,
	}
}

/// Places `bottom_level` moved by `offset`, so that its hits report `custom_index` and records from
/// `hit_group_offset`.
fn instance(
	bottom_level: BottomLevelAccelerationStructureHandle,
	offset: [f32; 3],
	custom_index: u32,
	hit_group_offset: u32,
) -> rt::Instance {
	rt::Instance {
		transform: [
			[1.0, 0.0, 0.0, offset[0]],
			[0.0, 1.0, 0.0, offset[1]],
			[0.0, 0.0, 1.0, offset[2]],
		],
		custom_index,
		mask: 0xFF,
		hit_group_offset,
		bottom_level,
	}
}

/// Asserts that `hit` lies `distance` along its ray, give or take float rounding.
fn assert_distance(hit: Option<Hit>, distance: f32) {
	let hit = hit.unwrap_or_else(|| panic!("The ray missed where it should hit at distance {distance}."));
	assert!(
		(hit.distance - distance).abs() < 1e-4,
		"The ray hit at distance {}, not {distance}.",
		hit.distance
	);
}

/// Asserts that `result` is a panic whose message calls a handle stale.
fn assert_stale(result: std::thread::Result<()>) {
	let payload = result.expect_err("A stale acceleration structure handle was accepted.");
	let message = payload
		.downcast_ref::<String>()
		.map(String::as_str)
		.or_else(|| payload.downcast_ref::<&str>().copied())
		.unwrap_or_default();
	assert!(message.contains("is stale"), "Unexpected panic: {message}");
}

/// A bottom-level structure built once keeps answering rays in later frames, though no command after its build names
/// it.
///
/// The queue lets go of storage that no recent command named after a while, so the frames after the build wait long
/// enough between them for that to happen several times over.
pub(super) fn traced_structures_stay_resident_without_rebuilds(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let tracer = Tracer::new(device, queue_handle);
	let geometry = triangles(device, &[0.0]);
	let bottom_level = device.create_bottom_level_acceleration_structure(Some("Resident Triangle"), &[geometry]);
	let top_level = device.create_top_level_acceleration_structure(Some("Resident Scene"), 1);
	let instances = [instance(bottom_level, [0.0; 3], 0, 0)];
	let rays = tracer.rays(device, top_level, &[[0.0; 3]]);

	for index in 0..4 {
		let bottom_levels: &[_] = if index == 0 {
			&[bottom_level]
		} else {
			std::thread::sleep(std::time::Duration::from_millis(300));
			&[]
		};
		tracer.frame(device, index, bottom_levels, Some((top_level, &instances)), &[&rays]);
		device.wait();
		assert_distance(rays.read(device)[0], 5.0);
	}
	assert!(!device.has_errors());
}

/// A counter around bottom-level acceleration structure builds times them, so the stage that builds a scene's meshes
/// reports the builds and the stage recorded after it doesn't, even when earlier work left an encoder open.
pub(super) fn counters_time_bottom_level_builds(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	// A grid of 2^17 separate triangles takes milliseconds to build, far longer than clearing a small image.
	let triangle_count = 1u32 << 17;
	let positions = (0..triangle_count)
		.flat_map(|index| {
			let (x, y) = ((index % 512) as f32 * 2.0, (index / 512) as f32 * 2.0);
			[[x, y, 5.0], [x + 1.0, y, 5.0], [x, y + 1.0, 5.0]]
		})
		.collect::<Vec<[f32; 3]>>();
	let indices = (0..positions.len() as u32).collect::<Vec<_>>();
	let geometry = rt::TriangleGeometry {
		positions: BufferStridedRange::new(geometry_buffer(device, &positions).into(), 0, 12, positions.len() * 12),
		indices: geometry_buffer(device, &indices).into(),
		index_format: DataTypes::U32,
		triangle_count,
	};
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear)
			.name("Counter Target")
			.extent(Extent::square(16)),
	);
	let builds = device.create_counter(Some("Builds"));
	let after = device.create_counter(Some("After"));
	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);

	// A structure builds once, so each frame builds its own. One more frame than can be in flight lets the first
	// complete.
	let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
	for frame_index in 0..frames {
		let bottom_level = device.create_bottom_level_acceleration_structure(None, &[geometry]);
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					// Earlier work leaves the compute encoder open, as a renderer's passes do.
					recording.clear_images(&[(image.into(), ClearValue::Color(RGBA::black()))]);
					recording.counter(builds, |recording| {
						recording.build_bottom_level_acceleration_structures(&[bottom_level]);
					});
					recording.counter(after, |recording| {
						recording.clear_images(&[(image.into(), ClearValue::Color(RGBA::black()))]);
					});
				});
				[]
			},
		);
		assert!(!device.has_errors());
	}
	let builds = device
		.counter_duration(builds)
		.expect("A completed frame must report its build time.");
	let after = device
		.counter_duration(after)
		.expect("A completed frame must report the clear's time.");
	assert!(
		builds > after,
		"The counter around the builds measured {builds:?}, no more than the clear after them, {after:?}. The builds' time went to a later counter."
	);
	device.wait();
}

/// Each frame in flight builds and traces its own copy of a top-level structure, and a frame that builds nothing
/// traces what the last frame of its sequence built.
pub(super) fn top_level_structures_have_a_copy_per_frame(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	device.set_frames_in_flight(2);
	let tracer = Tracer::new(device, queue_handle);
	let geometry = triangles(device, &[0.0]);
	let bottom_level = device.create_bottom_level_acceleration_structure(None, &[geometry]);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	// Frame 0 places the triangle at x = 0, and frame 1 at x = 10.
	let first = [instance(bottom_level, [0.0; 3], 100, 0)];
	let second = [instance(bottom_level, [10.0, 0.0, 0.0], 200, 0)];
	let rays = (0..4)
		.map(|_| tracer.rays(device, top_level, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]))
		.collect::<Vec<_>>();

	// The frames go out back to back, so they overlap on the GPU.
	tracer.frame(device, 0, &[bottom_level], Some((top_level, &first)), &[&rays[0]]);
	tracer.frame(device, 1, &[], Some((top_level, &second)), &[&rays[1]]);
	tracer.frame(device, 2, &[], None, &[&rays[2]]);
	tracer.frame(device, 3, &[], None, &[&rays[3]]);
	device.wait();

	let custom_indices = rays
		.iter()
		.map(|rays| {
			rays.read(device)
				.into_iter()
				.map(|hit| hit.map(|hit| hit.custom_index))
				.collect::<Vec<_>>()
		})
		.collect::<Vec<_>>();
	assert_eq!(
		custom_indices,
		[[Some(100), None], [None, Some(200)], [Some(100), None], [None, Some(200)]]
	);
	assert!(!device.has_errors());
}

/// Hits report the custom index and hit group offset of their instance in full, up to the largest value below
/// [`rt::INSTANCE_INDEX_LIMIT`], and follow the instance's transform.
pub(super) fn instances_report_custom_indices_beyond_16_bits(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let tracer = Tracer::new(device, queue_handle);
	let geometry = triangles(device, &[0.0]);
	let bottom_level = device.create_bottom_level_acceleration_structure(None, &[geometry]);
	let top_level = device.create_top_level_acceleration_structure(None, 2);
	let largest = rt::INSTANCE_INDEX_LIMIT - 1;
	// The second instance also moves 2 units along the rays, so its hits are 2 units farther.
	let instances = [
		instance(bottom_level, [0.0; 3], 70_000, 65_536),
		instance(bottom_level, [10.0, 0.0, 2.0], largest, largest),
	];
	let rays = tracer.rays(device, top_level, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]);

	tracer.frame(device, 0, &[bottom_level], Some((top_level, &instances)), &[&rays]);
	device.wait();

	let hits = rays.read(device);
	assert_eq!(
		hits.iter()
			.map(|hit| hit.map(|hit| (hit.instance, hit.custom_index, hit.record)))
			.collect::<Vec<_>>(),
		[Some((0, 70_000, 65_536)), Some((1, largest, largest))]
	);
	assert_distance(hits[0], 5.0);
	assert_distance(hits[1], 7.0);
	assert!(!device.has_errors());
}

/// A bottom-level structure holds geometries with their own buffers, offsets, strides, and index formats; hits report
/// each geometry's index in creation order, and their record is the instance's hit group offset plus that index.
pub(super) fn bottom_level_structures_hold_several_geometries(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let tracer = Tracer::new(device, queue_handle);
	let first = triangles(device, &[0.0]);
	// Triangles at x = 10 and 20, whose positions sit in 16-byte slots after one unused slot, and whose 32-bit indices
	// start after one unused index. Reading either from the buffer's start would build different triangles.
	let positions = std::iter::once([0.0; 4])
		.chain([10.0, 20.0].into_iter().flat_map(triangle_at).map(|[x, y, z]| [x, y, z, 0.0]))
		.collect::<Vec<[f32; 4]>>();
	let indices = [5u32, 0, 1, 2, 3, 4, 5];
	let position_buffer = geometry_buffer(device, &positions);
	let index_buffer = geometry_buffer(device, &indices);
	let second = rt::TriangleGeometry {
		positions: BufferStridedRange::new(position_buffer.into(), 16, 16, 6 * 16),
		indices: BufferDescriptor::from(index_buffer).offset(4),
		index_format: DataTypes::U32,
		triangle_count: 2,
	};
	let third = triangles(device, &[30.0]);
	let bottom_level = device.create_bottom_level_acceleration_structure(None, &[first, second, third]);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	let instances = [instance(bottom_level, [0.0; 3], 3, 7)];
	let rays = tracer.rays(device, top_level, &[0.0, 10.0, 20.0, 30.0, 40.0].map(|x| [x, 0.0, 0.0]));

	tracer.frame(device, 0, &[bottom_level], Some((top_level, &instances)), &[&rays]);
	device.wait();

	let hits = rays.read(device);
	assert_eq!(
		hits.iter()
			.map(|hit| hit.map(|hit| (hit.geometry, hit.primitive, hit.record)))
			.collect::<Vec<_>>(),
		[Some((0, 0, 7)), Some((1, 0, 8)), Some((1, 1, 8)), Some((2, 0, 9)), None]
	);
	for &hit in &hits[..4] {
		assert_distance(hit, 5.0);
	}
	assert!(!device.has_errors());
}

/// A bottom-level structure compacts itself a few frames after its build completes, keeps its handle, and traces
/// exactly as it did before.
pub(super) fn compacted_structures_trace_like_the_original(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	/// The quads along each side of the grid.
	const QUADS: u32 = 32;

	let tracer = Tracer::new(device, queue_handle);
	// A grid of quads in the z = 5 plane, from -16 to 16 on x and y, with two triangles per quad.
	let positions = (0..=QUADS)
		.flat_map(|y| (0..=QUADS).map(move |x| [x as f32 - 16.0, y as f32 - 16.0, 5.0]))
		.collect::<Vec<_>>();
	let indices = (0..QUADS)
		.flat_map(|y| (0..QUADS).map(move |x| y * (QUADS + 1) + x))
		.flat_map(|corner| {
			let above = corner + QUADS + 1;
			[corner, corner + 1, above + 1, corner, above + 1, above]
		})
		.collect::<Vec<_>>();
	let position_buffer = geometry_buffer(device, &positions);
	let index_buffer = geometry_buffer(device, &indices);
	let geometry = rt::TriangleGeometry {
		positions: BufferStridedRange::new(position_buffer.into(), 0, 12, positions.len() * 12),
		indices: index_buffer.into(),
		index_format: DataTypes::U32,
		triangle_count: indices.len() as u32 / 3,
	};
	let bottom_level = device.create_bottom_level_acceleration_structure(Some("Grid"), &[geometry]);
	let built_size = device.bottom_level_acceleration_structure_size(bottom_level);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	let instances = [instance(bottom_level, [0.0; 3], 0, 0)];
	// Rays through 64 quads spread over the grid, each off the quad's diagonal so that it hits one triangle.
	let origins = (0..64)
		.map(|ray| [(ray % 8) as f32 * 4.0 - 15.25, (ray / 8) as f32 * 4.0 - 15.75, 0.0])
		.collect::<Vec<_>>();
	let rays = tracer.rays(device, top_level, &origins);

	tracer.frame(device, 0, &[bottom_level], Some((top_level, &instances)), &[&rays]);
	device.wait();
	let original = rays.read(device);
	assert!(original.iter().all(Option::is_some), "A ray missed the grid: {original:?}");

	// Every frame builds the top-level structure again, so it names the compacted storage once it exists.
	let mut index = 1;
	loop {
		let compacted = device.bottom_level_acceleration_structure_size(bottom_level) < built_size;
		tracer.frame(device, index, &[], Some((top_level, &instances)), &[&rays]);
		device.wait();
		assert_eq!(
			rays.read(device),
			original,
			"The trace of frame {index} differs from the first."
		);
		if compacted {
			break;
		}
		assert!(
			index < 8,
			"The bottom-level structure kept its built size for {index} frames after its build completed."
		);
		index += 1;
	}
	let compacted_size = device.bottom_level_acceleration_structure_size(bottom_level);
	// The size was last read before the frame that traced the compacted structure first, so that frame's
	// predecessor compacted it when it started.
	println!(
		"Compaction shrank the bottom-level structure from {built_size} to {compacted_size} bytes when frame {} started.",
		index - 1
	);
	assert!(!device.has_errors());
}

/// Destroying a structure leaves it valid for the frames recorded before, and a top-level structure built from no
/// instances traces nothing.
pub(super) fn destroyed_structures_stay_valid_for_frames_in_flight(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	device.set_frames_in_flight(2);
	let tracer = Tracer::new(device, queue_handle);
	let geometry = triangles(device, &[0.0]);
	let bottom_level = device.create_bottom_level_acceleration_structure(None, &[geometry]);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	let instances = [instance(bottom_level, [0.0; 3], 0, 0)];
	let traced = tracer.rays(device, top_level, &[[0.0; 3]]);
	let emptied = tracer.rays(device, top_level, &[[0.0; 3]]);

	tracer.frame(device, 0, &[bottom_level], Some((top_level, &instances)), &[&traced]);
	// The first frame may still be running, so destroying its bottom-level structure must not take it from that frame.
	device.destroy_bottom_level_acceleration_structure(bottom_level);
	// The next frame builds the top-level structure again without the destroyed structure's instance.
	tracer.frame(device, 1, &[], Some((top_level, &[])), &[&emptied]);
	device.destroy_top_level_acceleration_structure(top_level);
	device.wait();

	assert_distance(traced.read(device)[0], 5.0);
	assert_eq!(emptied.read(device), [None]);
	assert!(!device.has_errors());
}

/// A destroyed structure's handle is rejected, even after a new structure takes its place.
pub(super) fn stale_structure_handles_are_rejected(device: &mut impl ghi::context::Context, _: QueueHandle) {
	use std::panic::{AssertUnwindSafe, catch_unwind};

	let geometry = triangles(device, &[0.0]);
	let destroyed = device.create_bottom_level_acceleration_structure(None, &[geometry]);
	device.destroy_bottom_level_acceleration_structure(destroyed);
	let replacement = device.create_bottom_level_acceleration_structure(None, &[geometry]);
	assert_ne!(replacement, destroyed);
	assert!(device.bottom_level_acceleration_structure_size(replacement) > 0);

	assert_stale(catch_unwind(AssertUnwindSafe(|| {
		device.bottom_level_acceleration_structure_size(destroyed);
	})));
	assert_stale(catch_unwind(AssertUnwindSafe(|| {
		device.destroy_bottom_level_acceleration_structure(destroyed);
	})));

	let top_level = device.create_top_level_acceleration_structure(None, 1);
	device.destroy_top_level_acceleration_structure(top_level);
	assert_stale(catch_unwind(AssertUnwindSafe(|| {
		device.destroy_top_level_acceleration_structure(top_level);
	})));
}

/// Declarations every library of the pipeline scenarios repeats, so the ray generation kernels and the closest hits
/// agree on how a closest hit is called: the payload it fills, the hit it runs for, and the addresses of the kernel's
/// descriptor sets, through which a closest hit reaches resources. Metal doesn't check that they agree.
macro_rules! pipeline_prelude {
	() => {
		r#"
#include <metal_stdlib>
#include <metal_raytracing>
using namespace metal;
using namespace raytracing;

struct Payload { uint group; uint value; };
struct HitAttributes { uint record; uint custom_index; uint geometry; uint primitive; };
struct SetAddresses { constant void* sets[2]; };
using ClosestHit = void(thread Payload&, thread const HitAttributes&, thread const SetAddresses&);

struct Rays {
	instance_acceleration_structure scene [[id(0)]];
	device const float4* origins [[id(1)]];
	device Payload* payloads [[id(2)]];
};
struct Materials { device const uint* values [[id(0)]]; };
"#
	};
}

/// The ray generation kernel of the hit group scenarios. Each thread traces one ray along +z from its origin with an
/// inline intersector, runs the closest hit of the record the intersector reports through the shader binding table at
/// buffer 14, and writes the payload. It binds the materials set at buffer 17 for its closest hits without using it.
const RAY_GENERATION: &str = concat!(
	pipeline_prelude!(),
	r#"
kernel void ray_generation(
	uint index [[thread_position_in_grid]],
	visible_function_table<ClosestHit> hits [[buffer(14)]],
	constant Rays& rays [[buffer(16)]],
	constant Materials& materials [[buffer(17)]]) {
	intersector<instancing, triangle_data, intersection_function_buffer> trace;
	const ray probe(rays.origins[index].xyz, float3(0.0, 0.0, 1.0), 0.0, 100.0);
	const auto result = trace.intersect(probe, rays.scene, intersection_function_buffer_arguments());
	Payload payload = { 0xffffffffu, 0u };
	if (result.type != intersection_type::none) {
		const HitAttributes hit = { result.function_id, result.user_instance_id, result.geometry_id, result.primitive_id };
		const SetAddresses sets = { { &rays, &materials } };
		hits[result.function_id](payload, hit, sets);
	}
	rays.payloads[index] = payload;
}
"#
);

/// A closest hit that reports its specialization constant 0 and the hit's record.
const TAGGED_HIT: &str = concat!(
	pipeline_prelude!(),
	r#"
constant uint tag [[function_constant(0)]];

[[visible]] void closest_hit(thread Payload& payload, thread const HitAttributes& hit, thread const SetAddresses& sets) {
	payload.group = tag;
	payload.value = hit.record;
}
"#
);

/// A closest hit with the name of [`TAGGED_HIT`]'s function, from another library, as every generated closest hit
/// has. It reports 30 and the hit's record.
const OTHER_HIT: &str = concat!(
	pipeline_prelude!(),
	r#"
[[visible]] void closest_hit(thread Payload& payload, thread const HitAttributes& hit, thread const SetAddresses& sets) {
	payload.group = 30u;
	payload.value = hit.record;
}
"#
);

/// A closest hit that reports 40 and the material value of the hit's record, from the set the ray generation kernel
/// binds without using.
const MATERIAL_HIT: &str = concat!(
	pipeline_prelude!(),
	r#"
[[visible]] void closest_hit(thread Payload& payload, thread const HitAttributes& hit, thread const SetAddresses& sets) {
	constant Materials& materials = *static_cast<constant Materials*>(sets.sets[1]);
	payload.group = 40u;
	payload.value = materials.values[hit.record];
}
"#
);

/// A ray generation kernel that counts its invocations, writes each one's launch index at its own position, and
/// reports the launch size.
const LAUNCH: &str = concat!(
	pipeline_prelude!(),
	r#"
struct Launches {
	device atomic_uint* count [[id(0)]];
	device uint4* size [[id(1)]];
	device uint4* points [[id(2)]];
};

kernel void launch(
	uint3 index [[thread_position_in_grid]],
	uint3 size [[threads_per_grid]],
	constant Launches& launches [[buffer(16)]]) {
	atomic_fetch_add_explicit(launches.count, 1u, memory_order_relaxed);
	launches.points[index.x + size.x * (index.y + size.y * index.z)] = uint4(index, 1u);
	if (all(index == 0u)) {
		*launches.size = uint4(size, 0u);
	}
}
"#
);

/// The payload a hit group scenario reads back for each ray: the group its closest hit reports and the value it
/// wrote, or [`MISS`] and 0 for a ray that hit nothing.
type Payload = [u32; 2];

/// The payload group of a ray that hit nothing.
const MISS: u32 = u32::MAX;

/// The `Shading` struct records the frames of the hit group scenarios: one ray per thread, which the ray generation
/// kernel traces and shades with the closest hit of the hit's record.
struct Shading {
	rays_layout: ghi::DescriptorSetLayout,
	materials_layout: ghi::DescriptorSetLayout,
	ray_generation: ShaderHandle,
	queue_handle: QueueHandle,
	command_buffer: CommandBufferHandle,
	synchronizer: SynchronizerHandle,
}

/// The `ShadedRays` struct is one batch of rays of the hit group scenarios: the sets the ray generation kernel binds,
/// and the buffer their payloads land in.
struct ShadedRays {
	sets: [DescriptorSetHandle; 2],
	payloads: BufferHandle<[Payload]>,
	count: u32,
}

impl ShadedRays {
	/// Reads each ray's payload. Call it after the frame that traced the rays completed.
	fn read(&self, device: &mut impl ghi::context::Context) -> Vec<Payload> {
		device.get_buffer_slice(self.payloads).to_vec()
	}
}

impl Shading {
	fn new(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) -> Self {
		let rays_layout = ghi::DescriptorSetLayout::new(
			"Shaded Rays",
			[
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(0),
					ghi::ResourceKind::AccelerationStructure,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(1),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(2),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::WRITE,
				),
			],
		);
		let materials_layout = ghi::DescriptorSetLayout::new(
			"Materials",
			[ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(0),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::READ,
			)],
		);
		let ray_generation = device
			.create_shader(
				Some("Ray Generation"),
				ghi::shader::Sources::MTL {
					source: RAY_GENERATION,
					entry_point: "ray_generation",
				},
				ShaderTypes::RayGen,
				[
					ghi::ShaderDescriptorSet::new(rays_layout.clone()),
					ghi::ShaderDescriptorSet::with_used(materials_layout.clone(), 0),
				],
			)
			.expect("Failed to create the ray generation shader. The most likely cause is invalid MSL in the kernel.");
		let command_buffer = device.queue(queue_handle).create_command_buffer(None);
		Self {
			rays_layout,
			materials_layout,
			ray_generation,
			queue_handle,
			command_buffer,
			synchronizer: device.create_synchronizer(None, true),
		}
	}

	/// Creates a closest-hit shader from `source`. A shader that reads materials declares both of the kernel's sets
	/// and uses only the second; any other declares none.
	fn closest_hit(
		&self,
		device: &mut impl ghi::context::Context,
		name: &str,
		source: &str,
		reads_materials: bool,
	) -> ShaderHandle {
		let sets = if reads_materials {
			vec![
				ghi::ShaderDescriptorSet::with_used(self.rays_layout.clone(), 0),
				ghi::ShaderDescriptorSet::new(self.materials_layout.clone()),
			]
		} else {
			Vec::new()
		};
		device
			.create_shader(
				Some(name),
				ghi::shader::Sources::MTL {
					source,
					entry_point: "closest_hit",
				},
				ShaderTypes::ClosestHit,
				sets,
			)
			.expect("Failed to create a closest-hit shader. The most likely cause is invalid MSL in the shader.")
	}

	/// Creates a ray tracing pipeline that launches the fixture's kernel with `hit_groups`.
	fn pipeline(&self, device: &mut impl ghi::context::Context, hit_groups: &[ShaderParameter]) -> PipelineHandle {
		device.create_ray_tracing_pipeline(
			pipelines::ray_tracing::Builder::new(
				&[],
				ShaderParameter::new(&self.ray_generation, ShaderTypes::RayGen),
				hit_groups,
			)
			.name("Hit Groups"),
		)
	}

	/// Creates a batch of rays that trace `structure` from each of `origins`, whose closest hits can read
	/// `material_values` by record.
	fn rays(
		&self,
		device: &mut impl ghi::context::Context,
		structure: TopLevelAccelerationStructureHandle,
		origins: &[[f32; 3]],
		material_values: &[u32],
	) -> ShadedRays {
		let origin_buffer = device.build_buffer::<[[f32; 4]]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(origins.len())
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		for (padded, origin) in device.get_mut_buffer_slice(origin_buffer).iter_mut().zip(origins) {
			*padded = [origin[0], origin[1], origin[2], 0.0];
		}
		device.sync_buffer(origin_buffer);
		let material_buffer = device.build_buffer::<[u32]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(material_values.len())
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		device.get_mut_buffer_slice(material_buffer).copy_from_slice(material_values);
		device.sync_buffer(material_buffer);
		let payloads = device.build_buffer::<[Payload]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(origins.len())
				.device_accesses(DeviceAccesses::DeviceToHost),
		);
		let rays_set = device.create_descriptor_set(None, &self.rays_layout);
		let materials_set = device.create_descriptor_set(None, &self.materials_layout);
		device.write(&[
			ghi::DescriptorWrite::acceleration_structure(rays_set, ghi::ResourceSlot::new(0), structure),
			ghi::DescriptorWrite::buffer(rays_set, ghi::ResourceSlot::new(1), origin_buffer.into()),
			ghi::DescriptorWrite::buffer(rays_set, ghi::ResourceSlot::new(2), payloads.into()),
			ghi::DescriptorWrite::buffer(materials_set, ghi::ResourceSlot::new(0), material_buffer.into()),
		]);
		ShadedRays {
			sets: [rays_set, materials_set],
			payloads,
			count: origins.len() as u32,
		}
	}

	/// Records frame `index` and submits it without waiting for it: the bottom-level builds, the top-level build, and
	/// a trace of each batch of rays with the pipeline and the shader binding table of `shading`.
	fn frame(
		&self,
		device: &mut impl ghi::context::Context,
		index: u64,
		bottom_levels: &[BottomLevelAccelerationStructureHandle],
		top_level: (TopLevelAccelerationStructureHandle, &[rt::Instance]),
		shading: (PipelineHandle, ShaderBindingTableHandle),
		batches: &[&ShadedRays],
	) {
		let (pipeline, table) = shading;
		device.queue(self.queue_handle).execute(
			Some(FrameRequest::new(index, self.synchronizer)),
			&[],
			self.synchronizer,
			|execution| {
				execution.record(self.command_buffer, |recording| {
					recording.build_bottom_level_acceleration_structures(bottom_levels);
					recording.build_top_level_acceleration_structure(top_level.0, top_level.1);
					for rays in batches {
						recording
							.bind_ray_tracing_pipeline(pipeline)
							.bind_descriptor_sets(&rays.sets)
							.trace_rays(table, Extent::line(rays.count));
					}
				});
				[]
			},
		);
	}
}

/// Creates a bottom-level structure with one geometry for each x of `centers`, each a triangle that a ray from
/// (x, 0, 0) hits, so the geometry a ray hits picks the record it reports.
fn geometry_per_triangle(device: &mut impl ghi::context::Context, centers: &[f32]) -> BottomLevelAccelerationStructureHandle {
	let geometries = centers.iter().map(|&x| triangles(device, &[x])).collect::<Vec<_>>();
	device.create_bottom_level_acceleration_structure(None, &geometries)
}

/// A hit runs the hit group that the shader binding table maps its record to, whichever instance and geometry it
/// hit: two groups that share a shader with different specialization constants, and a third whose shader names its
/// function the same in another library.
pub(super) fn invoke_runs_the_hit_group_of_the_record_the_intersector_reports(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let shading = Shading::new(device, queue_handle);
	let tagged = shading.closest_hit(device, "Tagged Hit", TAGGED_HIT, false);
	let other = shading.closest_hit(device, "Other Hit", OTHER_HIT, false);
	let ten = [pipelines::SpecializationMapEntry::new(0, 10u32)];
	let twenty = [pipelines::SpecializationMapEntry::new(0, 20u32)];
	let pipeline = shading.pipeline(
		device,
		&[
			ShaderParameter::new(&tagged, ShaderTypes::ClosestHit).with_specialization_map(&ten),
			ShaderParameter::new(&tagged, ShaderTypes::ClosestHit).with_specialization_map(&twenty),
			ShaderParameter::new(&other, ShaderTypes::ClosestHit),
		],
	);
	let bottom_level = geometry_per_triangle(device, &[0.0, 4.0, 8.0]);
	let top_level = device.create_top_level_acceleration_structure(None, 2);
	// The second instance sits 10 units up, and its geometries report records 3 to 5.
	let instances = [
		instance(bottom_level, [0.0; 3], 0, 0),
		instance(bottom_level, [0.0, 10.0, 0.0], 1, 3),
	];
	let table = device.create_shader_binding_table(Some("Hit Group Records"));
	device.write_shader_binding_table(table, pipeline, &[2, 0, 1, 1, 2, 0]);
	let origins = [
		[0.0, 0.0, 0.0],
		[4.0, 0.0, 0.0],
		[8.0, 0.0, 0.0],
		[0.0, 10.0, 0.0],
		[4.0, 10.0, 0.0],
		[8.0, 10.0, 0.0],
		[20.0, 0.0, 0.0],
	];
	let rays = shading.rays(device, top_level, &origins, &[0]);

	shading.frame(
		device,
		0,
		&[bottom_level],
		(top_level, &instances),
		(pipeline, table),
		&[&rays],
	);
	device.wait();

	assert_eq!(
		rays.read(device),
		[[30, 0], [10, 1], [20, 2], [20, 3], [30, 4], [10, 5], [MISS, 0]]
	);
	assert!(!device.has_errors());
}

/// A closest hit reads resources of a set that the ray generation shader declares and binds without using.
pub(super) fn closest_hit_reads_sets_the_ray_generation_shader_does_not_use(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let shading = Shading::new(device, queue_handle);
	let material = shading.closest_hit(device, "Material Hit", MATERIAL_HIT, true);
	let pipeline = shading.pipeline(device, &[ShaderParameter::new(&material, ShaderTypes::ClosestHit)]);
	let bottom_level = geometry_per_triangle(device, &[0.0, 4.0, 8.0]);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	let instances = [instance(bottom_level, [0.0; 3], 0, 0)];
	let table = device.create_shader_binding_table(None);
	device.write_shader_binding_table(table, pipeline, &[0, 0, 0]);
	let rays = shading.rays(device, top_level, &[0.0, 4.0, 8.0].map(|x| [x, 0.0, 0.0]), &[700, 1100, 1300]);

	shading.frame(
		device,
		0,
		&[bottom_level],
		(top_level, &instances),
		(pipeline, table),
		&[&rays],
	);
	device.wait();

	assert_eq!(rays.read(device), [[40, 700], [40, 1100], [40, 1300]]);
	assert!(!device.has_errors());
}

/// A trace runs the ray generation shader exactly once for each point of its launch size and reports that size,
/// whatever its shape: a height or a depth of 0 counts as 1, and a width of 0 runs nothing.
pub(super) fn trace_rays_launches_exactly_the_requested_grid(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let layout = ghi::DescriptorSetLayout::new(
		"Launches",
		[
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(0),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE,
			),
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(1),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::WRITE,
			),
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(2),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::WRITE,
			),
		],
	);
	let launch = device
		.create_shader(
			Some("Launch"),
			ghi::shader::Sources::MTL {
				source: LAUNCH,
				entry_point: "launch",
			},
			ShaderTypes::RayGen,
			[ghi::ShaderDescriptorSet::new(layout.clone())],
		)
		.expect("Failed to create the launch shader. The most likely cause is invalid MSL in the kernel.");
	let hit = device
		.create_shader(
			None,
			ghi::shader::Sources::MTL {
				source: TAGGED_HIT,
				entry_point: "closest_hit",
			},
			ShaderTypes::ClosestHit,
			[],
		)
		.expect("Failed to create the closest-hit shader. The most likely cause is invalid MSL in the shader.");
	let tag = [pipelines::SpecializationMapEntry::new(0, 0u32)];
	let pipeline = device.create_ray_tracing_pipeline(pipelines::ray_tracing::Builder::new(
		&[],
		ShaderParameter::new(&launch, ShaderTypes::RayGen),
		&[ShaderParameter::new(&hit, ShaderTypes::ClosestHit).with_specialization_map(&tag)],
	));
	let table = device.create_shader_binding_table(None);
	device.write_shader_binding_table(table, pipeline, &[0]);

	// A row that doesn't fill its last threadgroup, a rectangle and a box whose sides aren't multiples of a tile, and
	// a launch with no width.
	let extents = [
		Extent::line(100),
		Extent::rectangle(9, 7),
		Extent::new(5, 3, 2),
		Extent::line(0),
	];
	let launches = extents.map(|extent| {
		let points = (extent.width() * extent.height().max(1) * extent.depth().max(1)) as usize;
		let count = device.build_buffer::<[u32]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(1)
				.device_accesses(DeviceAccesses::DeviceToHost),
		);
		let size = device.build_buffer::<[[u32; 4]]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(1)
				.device_accesses(DeviceAccesses::DeviceToHost),
		);
		let indices = device.build_buffer::<[[u32; 4]]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(points.max(1))
				.device_accesses(DeviceAccesses::DeviceToHost),
		);
		let set = device.create_descriptor_set(None, &layout);
		device.write(&[
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(0), count.into()),
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), size.into()),
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(2), indices.into()),
		]);
		(extent, set, count, size, indices)
	});

	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	device
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				for (extent, set, ..) in &launches {
					recording
						.bind_ray_tracing_pipeline(pipeline)
						.bind_descriptor_sets(&[*set])
						.trace_rays(table, *extent);
				}
			});
			[]
		});
	device.wait();

	for (extent, _, count, size, indices) in launches {
		let (width, height, depth) = (extent.width(), extent.height().max(1), extent.depth().max(1));
		let expected = if width == 0 { 0 } else { width * height * depth };
		assert_eq!(
			device.get_buffer_slice(count)[0],
			expected,
			"Invocations of a launch of {extent:?}."
		);
		if expected == 0 {
			assert_eq!(device.get_buffer_slice(size)[0], [0; 4], "A launch with no width ran.");
			continue;
		}
		assert_eq!(device.get_buffer_slice(size)[0], [width, height, depth, 0]);
		let written = device.get_buffer_slice(indices);
		for z in 0..depth {
			for y in 0..height {
				for x in 0..width {
					assert_eq!(written[(x + width * (y + height * z)) as usize], [x, y, z, 1]);
				}
			}
		}
	}
	assert!(!device.has_errors());
}

/// A pipeline created again with one more hit group runs the new group once the shader binding table is written for
/// it, and a trace recorded before that write keeps the records it was recorded with.
pub(super) fn rebuilt_pipeline_with_more_hit_groups_invokes_the_new_group(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let shading = Shading::new(device, queue_handle);
	let tagged = shading.closest_hit(device, "Tagged Hit", TAGGED_HIT, false);
	let other = shading.closest_hit(device, "Other Hit", OTHER_HIT, false);
	let ten = [pipelines::SpecializationMapEntry::new(0, 10u32)];
	let first = shading.pipeline(
		device,
		&[ShaderParameter::new(&tagged, ShaderTypes::ClosestHit).with_specialization_map(&ten)],
	);
	let bottom_level = geometry_per_triangle(device, &[0.0, 4.0]);
	let top_level = device.create_top_level_acceleration_structure(None, 1);
	let instances = [instance(bottom_level, [0.0; 3], 0, 0)];
	let table = device.create_shader_binding_table(None);
	device.write_shader_binding_table(table, first, &[0, 0]);
	let origins = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
	let before = shading.rays(device, top_level, &origins, &[0]);
	let after = shading.rays(device, top_level, &origins, &[0]);

	shading.frame(
		device,
		0,
		&[bottom_level],
		(top_level, &instances),
		(first, table),
		&[&before],
	);
	// The first group keeps its shader and constants, and the new group sits after it.
	let second = shading.pipeline(
		device,
		&[
			ShaderParameter::new(&tagged, ShaderTypes::ClosestHit).with_specialization_map(&ten),
			ShaderParameter::new(&other, ShaderTypes::ClosestHit),
		],
	);
	device.write_shader_binding_table(table, second, &[0, 1]);
	shading.frame(device, 1, &[], (top_level, &instances), (second, table), &[&after]);
	device.wait();

	assert_eq!(before.read(device), [[10, 0], [10, 1]]);
	assert_eq!(after.read(device), [[10, 0], [30, 1]]);
	assert!(!device.has_errors());
}
