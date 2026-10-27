//! Runs baked ray tracing shaders on the GPU and in the BESL VM, and compares what they write.
//!
//! The shaders are baked as asset bakes bake them, with the platform generator's lowering and the Metal compiler, and
//! GHI runs the baked libraries. The VM runs the same linked programs, so a lowering that changes what a shader
//! computes shows up as a difference between the two.

use std::sync::Arc;

use besl::vm::{
	AccelerationStructure, BottomLevelStructure, Buffer, DescriptorBindings, ExecutableProgram, RayTracingPipeline,
	ResourceSlot, Value,
};
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRayTracingPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _,
	},
	device::Device as _,
	queue::{FrameRequest, Queue as _, QueueExecution as _},
};

use super::{CLOSEST_HIT_FIXTURE, RAY_TRACING_LAYERS};
use crate::shader::{
	besl::backends::{msl::MSL_ENTRY_POINT, platform},
	generator::ShaderGenerationSettings,
};

/// Traces two rows of four rays from x = 0, 10, 20, and 30 at the push constant's scale of 10, along +z at twice the
/// unit length, so a hit's distance is half the depth it lies at. The first row's mask takes every instance, and the
/// second row's only instances with bit 1. Each ray shades what it finds through `invoke`, which leaves a miss's
/// payload as it is, and adds the launch width to its distance when a surface lies within 3.5 units ahead of it.
const RAY_GENERATION: &str = r#"
	main: fn (input: StageInput) -> void {
		let item: u32 = input.launch_index.x;
		let mask: u32 = 255;
		if (item >= 4) {
			mask = 2;
		}
		let origin: vec3f = vec3f(f32(item % 4) * push_constant.scale, 0.0, 0.0);
		let hit: Hit = intersect(scene, mask, origin, vec3f(0.0, 0.0, 2.0), 0.0, 100.0);
		let surface: SurfaceHit = invoke(hit, SurfaceHit(vec3f(0.25, 0.5, 0.75), -1.0));
		if (trace_occlusion(scene, mask, origin, vec3f(0.0, 0.0, 1.0), 0.0, 3.5)) {
			surface.distance = surface.distance + f32(input.launch_size.x);
		}
		surfaces[item] = surface;
	}
"#;

/// Reports the instance and geometry a hit lies on and where its ray started, without reading a set or the push
/// constant.
const INSTANCE_HIT: &str = r#"
	main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit {
		let surface: SurfaceHit = payload;
		surface.albedo = vec3f(f32(input.hit_custom_index), input.ray_origin.x, f32(input.hit_geometry_index));
		surface.distance = input.hit_distance;
		return surface;
	}
"#;

/// The slot of the scene, which the fixture's `scene` layer declares.
const SCENE: u32 = 0;

/// The slot of the surfaces the ray generation shader writes, which the `scene` layer declares too.
const SURFACES: u32 = 1;

/// The slot of the albedos, which the `materials` layer declares and only the material hit reads.
const ALBEDOS: u32 = 2;

/// The push constant's scale, which spaces the rays' origins and scales the material hit's distances.
const SCALE: f32 = 10.0;

/// The launch width, two rows of four rays.
const RAYS: u32 = 8;

/// The albedo of each geometry, which the material hit reads by geometry index.
const GEOMETRY_ALBEDOS: [[f32; 4]; 2] = [[1.0, 2.0, 3.0, 0.0], [0.5, 0.25, 0.125, 0.0]];

/// The hit group each record runs: the material hit, [`CLOSEST_HIT_FIXTURE`], for every record but the one of the
/// instance with bit 1, which runs [`INSTANCE_HIT`].
const RECORDS: [u32; 4] = [0, 0, 1, 0];

/// A triangle that no fixture ray hits.
const TRIANGLE_ASIDE: [[f32; 3]; 3] = [[50.0, 50.0, 1.0], [51.0, 50.0, 1.0], [50.0, 51.0, 1.0]];

/// A ray's surface as both sides store it: its albedo, then its distance.
type Surface = [f32; 4];

/// What `invoke` leaves for a ray that hit nothing: the payload the ray generation shader passed.
const MISS: Surface = [0.25, 0.5, 0.75, -1.0];

/// How far a GPU value may stray from the VM's, relative to values above 1, since the GPU's intersector rounds its
/// distances and barycentrics on its own.
const TOLERANCE: f32 = 1e-4;

/// The `Placement` struct places one of [`bottom_levels`] in the fixture's scene, for the GPU and the VM alike.
struct Placement {
	bottom_level: usize,
	depth_offset: f32,
	custom_index: u32,
	mask: u8,
	hit_group_offset: u32,
}

/// The fixture's instances, whose records are 0 to 3. The second structure appears twice ahead of x = 20: at depth 6
/// with mask bit 1, and in front of it at depth 2 with mask bit 0, so a ray of the second row passes the nearer one.
const PLACEMENTS: [Placement; 3] = [
	Placement {
		bottom_level: 0,
		depth_offset: 0.0,
		custom_index: 3,
		mask: 0b01,
		hit_group_offset: 0,
	},
	Placement {
		bottom_level: 1,
		depth_offset: 2.0,
		custom_index: 9,
		mask: 0b10,
		hit_group_offset: 2,
	},
	Placement {
		bottom_level: 1,
		depth_offset: -2.0,
		custom_index: 12,
		mask: 0b01,
		hit_group_offset: 3,
	},
];

/// Returns a triangle at depth `z` that a ray from `(x, 0, 0)` along +z hits at barycentrics (0.25, 0.5), which tell
/// the weights of the second and third vertices apart.
fn triangle_ahead_of(x: f32, z: f32) -> [[f32; 3]; 3] {
	[[x - 1.0, -2.0, z], [x + 3.0, -2.0, z], [x - 1.0, 2.0, z]]
}

/// Returns the fixture's bottom-level structures as the triangles of each geometry. The first holds a triangle ahead
/// of x = 0 at depth 5, and a geometry whose second triangle lies ahead of x = 10 at depth 3; the second holds one
/// triangle ahead of x = 20 at depth 4.
fn bottom_levels() -> [Vec<Vec<[[f32; 3]; 3]>>; 2] {
	[
		vec![
			vec![triangle_ahead_of(0.0, 5.0)],
			vec![TRIANGLE_ASIDE, triangle_ahead_of(10.0, 3.0)],
		],
		vec![vec![triangle_ahead_of(20.0, 4.0)]],
	]
}

/// Returns the transform that moves a structure `depth_offset` along +z.
fn transform(depth_offset: f32) -> [[f32; 4]; 3] {
	[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, depth_offset]]
}

/// Stacks the fixture layers under `source` and links the program.
fn link(source: &str) -> besl::NodeReference {
	let module = besl::parse(source).expect("Expected the ray tracing fixture to parse");
	besl::lex(besl::stack_layers(module, RAY_TRACING_LAYERS).expect("Expected the layers to stack"))
		.expect("Expected the ray tracing fixture to link")
}

/// Bakes `program` for the current platform, as asset bakes do, and returns the Metal library.
async fn bake(program: &besl::NodeReference, settings: ShaderGenerationSettings) -> Box<[u8]> {
	let name = settings.name.clone();
	platform::Generator::new()
		.lower(&settings, program)
		.unwrap_or_else(|error| panic!("Expected {name} to lower. {error}"))
		.compile()
		.await
		.unwrap_or_else(|error| panic!("Expected {name} to compile. {error}"))
		.binary
}

/// Asserts that every ray's surface in `found` matches `expected` within [`TOLERANCE`].
fn assert_surfaces_near(found: &[Surface], expected: &[Surface], source: &str) {
	assert_eq!(found.len(), expected.len(), "{source} wrote another number of surfaces.");
	for (ray, (found, expected)) in found.iter().zip(expected).enumerate() {
		let near = found
			.iter()
			.zip(expected)
			.all(|(found, expected)| (found - expected).abs() <= TOLERANCE * expected.abs().max(1.0));
		assert!(near, "{source} ray {ray} wrote {found:?}, not {expected:?}.");
	}
}

/// Verifies baked ray generation and closest-hit shaders write on the GPU what the VM writes for the same scene: hits
/// through both hit groups, an instance that a mask skips for a farther one, misses whose payload `invoke` leaves as
/// it is, and an occlusion test, with a ray direction twice the unit length.
#[compio::test]
async fn generated_ray_tracing_shaders_match_the_vm() {
	let ray_generation = link(RAY_GENERATION);
	let hit_groups = [link(CLOSEST_HIT_FIXTURE), link(INSTANCE_HIT)];
	let baked_ray_generation = bake(
		&ray_generation,
		ShaderGenerationSettings::ray_generation().name("matched_ray_generation".to_string()),
	)
	.await;
	let baked_material_hit = bake(
		&hit_groups[0],
		ShaderGenerationSettings::closest_hit().name("matched_material_hit".to_string()),
	)
	.await;
	let baked_instance_hit = bake(
		&hit_groups[1],
		ShaderGenerationSettings::closest_hit().name("matched_instance_hit".to_string()),
	)
	.await;

	let vm = trace_in_the_vm(&ray_generation, &hit_groups);
	let gpu = trace_on_the_gpu(&baked_ray_generation, &[&baked_material_hit, &baked_instance_hit]);

	// The material hit adds the hit's albedo times its distance to the barycentrics and the primitive index, and
	// reports the distance times the scale plus the ray direction's 2.
	let expected = [
		// Geometry 0 of the first instance, at distance 2.5.
		[2.75, 5.5, 7.5, 27.0],
		// The second triangle of geometry 1, at distance 1.5, which also lies within the occlusion test's 3.5 units.
		[1.0, 0.875, 1.1875, 25.0],
		// The nearer copy of the second structure, at distance 1, within the occlusion test too.
		[1.25, 2.5, 3.0, 20.0],
		// Nothing lies ahead of x = 30, and the second row's mask skips the first instance.
		MISS,
		MISS,
		MISS,
		// The instance hit: the farther copy of the second structure, at distance 3, passing the nearer one.
		[9.0, 20.0, 0.0, 3.0],
		MISS,
	];
	assert_surfaces_near(&vm, &expected, "The VM");
	assert_surfaces_near(&gpu, &vm, "The GPU");
}

/// Traces the fixture's rays through `ray_generation` and `hit_groups` in the VM and returns each ray's surface.
fn trace_in_the_vm(ray_generation: &besl::NodeReference, hit_groups: &[besl::NodeReference]) -> Vec<Surface> {
	let compile =
		|program: &besl::NodeReference| ExecutableProgram::compile(program.clone()).expect("Expected the VM to compile");
	let ray_generation = compile(ray_generation);
	let hit_groups = hit_groups.iter().map(compile).collect::<Vec<_>>();

	let surfaces_layout = ray_generation
		.buffer_layout(ResourceSlot::new(SURFACES))
		.expect("Expected the surfaces buffer")
		.clone();
	let mut surfaces = Buffer::new_array(surfaces_layout, RAYS as usize).expect("Expected surface storage");
	// Only the material hit reads the albedos, so only its program describes them.
	let albedos_layout = hit_groups[0]
		.buffer_layout(ResourceSlot::new(ALBEDOS))
		.expect("Expected the albedos buffer")
		.clone();
	let mut albedos = Buffer::new_array(albedos_layout, GEOMETRY_ALBEDOS.len()).expect("Expected albedo storage");
	for (geometry, albedo) in GEOMETRY_ALBEDOS.into_iter().enumerate() {
		albedos
			.write_array_element(geometry, Value::Vec4F(albedo))
			.expect("Expected to write an albedo");
	}
	let mut push_constant = Buffer::new(
		ray_generation
			.push_constant_layout()
			.expect("Expected the push constant")
			.clone(),
	);
	push_constant
		.write("scale", Value::F32(SCALE))
		.expect("Expected to write the scale");

	let bottom_levels = bottom_levels().map(|geometries| Arc::new(BottomLevelStructure::new(geometries)));
	let scene = AccelerationStructure::new(
		PLACEMENTS
			.iter()
			.map(|placement| besl::vm::Instance {
				transform: transform(placement.depth_offset),
				custom_index: placement.custom_index,
				mask: placement.mask,
				hit_group_offset: placement.hit_group_offset,
				bottom_level: bottom_levels[placement.bottom_level].clone(),
			})
			.collect(),
	);
	let pipeline = RayTracingPipeline::new(ray_generation, hit_groups).expect("Expected a valid VM pipeline");

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_acceleration_structure(ResourceSlot::new(SCENE), &scene);
	descriptors.bind_buffer(ResourceSlot::new(SURFACES), &mut surfaces);
	descriptors.bind_buffer(ResourceSlot::new(ALBEDOS), &mut albedos);
	descriptors.bind_push_constant(&mut push_constant);
	pipeline
		.trace_rays(&mut descriptors, &RECORDS, [RAYS, 1, 1])
		.expect("Expected the VM trace to run");
	drop(descriptors);

	(0..RAYS as usize)
		.map(|ray| {
			let (Ok(Value::Vec3F(albedo)), Ok(Value::F32(distance))) = (
				surfaces.read_array_member(ray, "albedo"),
				surfaces.read_array_member(ray, "distance"),
			) else {
				panic!("Expected the VM to write a surface for ray {ray}");
			};
			[albedo[0], albedo[1], albedo[2], distance]
		})
		.collect()
}

/// Traces the fixture's rays with the baked shaders on a new GPU device and returns each ray's surface.
fn trace_on_the_gpu(ray_generation: &[u8], hit_groups: &[&[u8]]) -> Vec<Surface> {
	let features = ghi::device::Features::new().validation(true).ray_tracing(true);
	// The context drops before the device, and the device before the instance.
	let mut instance = ghi::implementation::Instance::new(features)
		.expect("Failed to create the GHI instance. The most likely cause is that no Metal device is available.");
	let mut queue_handle = None;
	let device = instance
		.create_device(
			features,
			&mut [(
				ghi::QueueSelection::new(ghi::WorkloadTypes::COMPUTE | ghi::WorkloadTypes::TRANSFER),
				&mut queue_handle,
			)],
		)
		.expect("Failed to create the GHI device. The most likely cause is a GPU without compute queue support.");
	let mut context = device
		.create_context()
		.expect("Failed to create the GHI context. The most likely cause is unavailable command support.");
	let queue_handle = queue_handle.expect("GHI device creation must fill the requested queue handle.");
	trace_with(&mut context, queue_handle, ray_generation, hit_groups)
}

/// Builds the fixture's scene on `context`, traces its rays with the baked shaders, and reads back their surfaces.
fn trace_with(
	context: &mut impl ghi::context::Context,
	queue_handle: ghi::QueueHandle,
	ray_generation: &[u8],
	hit_groups: &[&[u8]],
) -> Vec<Surface> {
	assert!(
		context.supports_ray_tracing(),
		"The test device cannot trace rays. The most likely cause is a GPU without ray tracing support. Run the test on a GPU that supports it."
	);

	// The layers declare the scene and the surfaces in set 0, and the albedos in set 1.
	let scene_layout = ghi::DescriptorSetLayout::new(
		"Scene",
		[
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(SCENE),
				ghi::ResourceKind::AccelerationStructure,
				ghi::AccessPolicies::READ,
			),
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(SURFACES),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::WRITE,
			),
		],
	);
	let materials_layout = ghi::DescriptorSetLayout::new(
		"Materials",
		[ghi::ShaderResourceDescriptor::single(
			ghi::ResourceSlot::new(ALBEDOS),
			ghi::ResourceKind::StorageBuffer,
			ghi::AccessPolicies::READ,
		)],
	);
	// Every shader takes both sets; each marks only the resources it reads or writes as used.
	let ray_generation = create_shader(
		context,
		"Matched Ray Generation",
		ray_generation,
		ghi::ShaderTypes::RayGen,
		[
			ghi::ShaderDescriptorSet::new(scene_layout.clone()),
			ghi::ShaderDescriptorSet::with_used(materials_layout.clone(), 0),
		],
	);
	let material_hit = create_shader(
		context,
		"Matched Material Hit",
		hit_groups[0],
		ghi::ShaderTypes::ClosestHit,
		[
			ghi::ShaderDescriptorSet::with_used(scene_layout.clone(), 0),
			ghi::ShaderDescriptorSet::new(materials_layout.clone()),
		],
	);
	let instance_hit = create_shader(
		context,
		"Matched Instance Hit",
		hit_groups[1],
		ghi::ShaderTypes::ClosestHit,
		[
			ghi::ShaderDescriptorSet::with_used(scene_layout.clone(), 0),
			ghi::ShaderDescriptorSet::with_used(materials_layout.clone(), 0),
		],
	);
	let push_constant_ranges = [ghi::pipelines::PushConstantRange::new(0, size_of::<f32>() as u32)];
	let hit_group_parameters = [
		ghi::ShaderParameter::new(&material_hit, ghi::ShaderTypes::ClosestHit),
		ghi::ShaderParameter::new(&instance_hit, ghi::ShaderTypes::ClosestHit),
	];
	let pipeline = context.create_ray_tracing_pipeline(
		ghi::pipelines::ray_tracing::Builder::new(
			&push_constant_ranges,
			ghi::ShaderParameter::new(&ray_generation, ghi::ShaderTypes::RayGen),
			&hit_group_parameters,
		)
		.name("Matched Ray Tracing"),
	);
	let table = context.create_shader_binding_table(Some("Matched Records"));
	context.write_shader_binding_table(table, pipeline, &RECORDS);

	let bottom_levels = bottom_levels().map(|geometries| {
		let geometries = geometries
			.iter()
			.map(|triangles| triangle_geometry(context, triangles))
			.collect::<Vec<_>>();
		context.create_bottom_level_acceleration_structure(None, &geometries)
	});
	let instances = PLACEMENTS
		.iter()
		.map(|placement| ghi::rt::Instance {
			transform: transform(placement.depth_offset),
			custom_index: placement.custom_index,
			mask: placement.mask,
			hit_group_offset: placement.hit_group_offset,
			bottom_level: bottom_levels[placement.bottom_level],
		})
		.collect::<Vec<_>>();
	let top_level = context.create_top_level_acceleration_structure(None, instances.len() as u32);

	let albedos = context.build_buffer::<[[f32; 4]]>(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.length(GEOMETRY_ALBEDOS.len())
			.device_accesses(ghi::DeviceAccesses::HostToDevice),
	);
	context.get_mut_buffer_slice(albedos).copy_from_slice(&GEOMETRY_ALBEDOS);
	context.sync_buffer(albedos);
	let surfaces = context.build_buffer::<[Surface]>(
		ghi::buffer::Builder::new(ghi::Uses::Storage)
			.length(RAYS as usize)
			.device_accesses(ghi::DeviceAccesses::DeviceToHost),
	);
	let scene_set = context.create_descriptor_set(None, &scene_layout);
	let materials_set = context.create_descriptor_set(None, &materials_layout);
	context.write(&[
		ghi::DescriptorWrite::acceleration_structure(scene_set, ghi::ResourceSlot::new(SCENE), top_level),
		ghi::DescriptorWrite::buffer(scene_set, ghi::ResourceSlot::new(SURFACES), surfaces.into()),
		ghi::DescriptorWrite::buffer(materials_set, ghi::ResourceSlot::new(ALBEDOS), albedos.into()),
	]);

	let command_buffer = context.queue(queue_handle).create_command_buffer(None);
	let synchronizer = context.create_synchronizer(None, true);
	context
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				recording.build_bottom_level_acceleration_structures(&bottom_levels);
				recording.build_top_level_acceleration_structure(top_level, &instances);
				let traced = recording
					.bind_ray_tracing_pipeline(pipeline)
					.bind_descriptor_sets(&[scene_set, materials_set]);
				traced.write_push_constant(0, SCALE);
				traced.trace_rays(table, utils::Extent::line(RAYS));
			});
			[]
		});
	context.wait();
	assert!(
		!context.has_errors(),
		"The GPU trace reported validation errors. The most likely cause is a baked shader that reads a resource its pipeline doesn't bind."
	);

	context.get_buffer_slice(surfaces).to_vec()
}

/// Creates a shader from a baked Metal library, whose entry point every generated shader names alike.
fn create_shader(
	context: &mut impl ghi::context::Context,
	name: &str,
	binary: &[u8],
	stage: ghi::ShaderTypes,
	sets: impl IntoIterator<Item = ghi::ShaderDescriptorSet>,
) -> ghi::ShaderHandle {
	context
		.create_shader(
			Some(name),
			ghi::shader::Sources::MTLB {
				binary,
				entry_point: MSL_ENTRY_POINT,
				threadgroup_size: None,
			},
			stage,
			sets,
		)
		.unwrap_or_else(|()| {
			panic!("Failed to create shader {name}. The most likely cause is a Metal library without `{MSL_ENTRY_POINT}`.")
		})
}

/// Uploads `triangles` as one geometry: packed positions, three per triangle, and 16-bit indices.
fn triangle_geometry(context: &mut impl ghi::context::Context, triangles: &[[[f32; 3]; 3]]) -> ghi::rt::TriangleGeometry {
	let positions = triangles.concat();
	let indices = (0..positions.len() as u16).collect::<Vec<_>>();
	let position_buffer = geometry_buffer(context, &positions);
	let index_buffer = geometry_buffer(context, &indices);
	ghi::rt::TriangleGeometry {
		positions: ghi::BufferStridedRange::new(position_buffer.into(), 0, 12, positions.len() * 12),
		indices: index_buffer.into(),
		index_format: ghi::DataTypes::U16,
		triangle_count: triangles.len() as u32,
	}
}

/// Copies `values` into a new buffer that acceleration structure builds read.
fn geometry_buffer<T: bytemuck::Pod>(context: &mut impl ghi::context::Context, values: &[T]) -> ghi::BufferHandle<[T]> {
	let buffer = context.build_buffer::<[T]>(
		ghi::buffer::Builder::new(ghi::Uses::AccelerationStructureBuild)
			.length(values.len())
			.device_accesses(ghi::DeviceAccesses::HostToDevice),
	);
	context.get_mut_buffer_slice(buffer).copy_from_slice(values);
	context.sync_buffer(buffer);
	buffer
}
