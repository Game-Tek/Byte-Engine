//! Measurement spikes for ray tracing pipelines on Vulkan, which run closest hits as callable shaders linked from
//! pipeline libraries.
//!
//! They time what the Metal spikes in `src/metal/ray_tracing_spikes.rs` time: building a pipeline with many hit groups
//! and again with one more, and closest-hit calls that diverge against calls sorted by hit group. The closest hits
//! have the Metal spikes' shape, dependent `sin` and `cos` statements, so the numbers compare. Every shader holds a
//! number picked per run, so the driver's shader cache doesn't serve a compile from an earlier run.
//!
//! Run them with `cargo nextest run -p byte-engine-ghi --test rendering --ignore-default-filter --run-ignored only
//! ray_tracing_spikes --no-capture`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;

/// The ray generation shader: invocation `i` calls the closest hit of record `records[i]` and writes the payload it
/// leaves.
const RAY_GENERATION: &str = r#"
#version 460
#pragma shader_stage(raygen)
#extension GL_EXT_ray_tracing : require
struct Payload { vec3 albedo; float distance; };
layout(set = 0, binding = 0, std430) readonly buffer Records { uint records[]; };
layout(set = 0, binding = 1, std430) writeonly buffer Results { vec4 results[]; };
layout(location = 0) callableDataEXT Payload payload;
void main() {
	uint index = gl_LaunchIDEXT.x;
	payload = Payload(vec3(float(index) * 0.001, 0.5, 0.25), float(index));
	executeCallableEXT(records[index], 0);
	results[index] = vec4(payload.albedo, payload.distance);
}
"#;

/// Returns a closest hit of `statement_count` dependent transcendental statements, different in every shader and every
/// run. It writes its index, and the step keeps the math live without changing the distance.
fn closest_hit(index: usize, statement_count: usize, nonce: u64) -> String {
	let statements: String = (0..statement_count)
		.map(|statement| {
			let scale = 1.0 + (index * statement_count + statement) as f32 / 997.0;
			if statement % 2 == 0 {
				format!("\tv = sin(v * {scale:.6} + v.yzx);\n")
			} else {
				format!("\tv = cos(v.zxy * {scale:.6} - v);\n")
			}
		})
		.collect();
	format!(
		r#"
#version 460
#pragma shader_stage(callable)
#extension GL_EXT_ray_tracing : require
struct Payload {{ vec3 albedo; float distance; }};
layout(location = 0) callableDataInEXT Payload payload;
void main() {{
	vec3 v = payload.albedo;
{statements}	payload.albedo = vec3({index}.0, v.y * 0.0, {nonce}.0);
	payload.distance += step(1e30, abs(v.x + v.y + v.z));
}}
"#
	)
}

/// Returns a number that differs between runs, which every shader holds so that each run compiles cold.
fn run_nonce() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |time| time.as_millis() as u64 % 1_000_000)
}

/// The `Spike` struct is the ray generation shader of the spikes, the set it binds, and what records its frames.
struct Spike {
	layout: ghi::DescriptorSetLayout,
	ray_generation: ShaderHandle,
	queue_handle: QueueHandle,
	command_buffer: CommandBufferHandle,
	synchronizer: SynchronizerHandle,
	counter: CounterHandle,
}

impl Spike {
	fn new(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) -> Self {
		let layout = ghi::DescriptorSetLayout::new(
			"Spike Rays",
			[
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(0),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::READ,
				),
				ghi::ShaderResourceDescriptor::single(
					ghi::ResourceSlot::new(1),
					ghi::ResourceKind::StorageBuffer,
					ghi::AccessPolicies::WRITE,
				),
			],
		);
		// The spikes measure Vulkan, so their shaders have no MSL.
		let ray_generation = super::ray_tracing::native_shader(
			device,
			"Spike Ray Generation",
			ShaderTypes::RayGen,
			(RAY_GENERATION, "", ""),
			[ghi::ShaderDescriptorSet::new(layout.clone())],
		);
		let command_buffer = device.queue(queue_handle).create_command_buffer(None);
		Self {
			layout,
			ray_generation,
			queue_handle,
			command_buffer,
			synchronizer: device.create_synchronizer(None, true),
			counter: device.create_counter(Some("Spike Trace")),
		}
	}

	/// Creates `count` closest hits of `statements` statements each.
	fn closest_hits(&self, device: &mut impl ghi::context::Context, count: usize, statements: usize) -> Vec<ShaderHandle> {
		let nonce = run_nonce();
		(0..count)
			.map(|index| {
				super::ray_tracing::native_shader(
					device,
					"Spike Closest Hit",
					ShaderTypes::ClosestHit,
					(&closest_hit(index, statements, nonce), "", ""),
					[],
				)
			})
			.collect()
	}

	/// Creates a pipeline whose hit group `i` runs `hits[i]`, and returns it with the wall time creation took.
	fn pipeline(&self, device: &mut impl ghi::context::Context, hits: &[ShaderHandle]) -> (PipelineHandle, Duration) {
		let parameters = hits
			.iter()
			.map(|hit| ShaderParameter::new(hit, ShaderTypes::ClosestHit))
			.collect::<Vec<_>>();
		let start = Instant::now();
		let pipeline = device.create_ray_tracing_pipeline(pipelines::ray_tracing::Builder::new(
			&[],
			ShaderParameter::new(&self.ray_generation, ShaderTypes::RayGen),
			&parameters,
		));
		(pipeline, start.elapsed())
	}

	/// Traces one invocation per entry of `records` with `pipeline` over a few frames, and returns the shortest GPU
	/// time a trace took.
	fn time(
		&self,
		device: &mut impl ghi::context::Context,
		pipeline: PipelineHandle,
		groups: u32,
		records: &[u32],
	) -> Duration {
		let table = device.create_shader_binding_table(None);
		device.write_shader_binding_table(table, pipeline, &(0..groups).collect::<Vec<_>>());
		let record_buffer = device.build_buffer::<[u32]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(records.len())
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		device.get_mut_buffer_slice(record_buffer).copy_from_slice(records);
		device.sync_buffer(record_buffer);
		let results = device.build_buffer::<[[f32; 4]]>(
			ghi::buffer::Builder::new(Uses::Storage)
				.length(records.len())
				.device_accesses(DeviceAccesses::DeviceOnly),
		);
		let set = device.create_descriptor_set(None, &self.layout);
		device.write(&[
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(0), record_buffer.into()),
			ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), results.into()),
		]);

		let mut best = Duration::MAX;
		// A counter reads back once its frame's sequence comes around again, so the first frames give no sample.
		for frame in 0..16u64 {
			device.queue(self.queue_handle).execute(
				Some(FrameRequest::new(frame, self.synchronizer)),
				&[],
				self.synchronizer,
				|execution| {
					execution.record(self.command_buffer, |recording| {
						recording.counter(self.counter, |recording| {
							recording
								.bind_ray_tracing_pipeline(pipeline)
								.bind_descriptor_sets(&[set])
								.trace_rays(table, Extent::line(records.len() as u32));
						});
					});
					[]
				},
			);
			if frame >= 4
				&& let Some(time) = device.counter_duration(self.counter)
			{
				best = best.min(time);
			}
		}
		device.wait();
		assert!(!device.has_errors());
		best
	}
}

/// Reads a spike setting from the environment, or `default`.
fn setting(name: &str, default: usize) -> usize {
	std::env::var(name)
		.ok()
		.and_then(|value| value.parse().ok())
		.unwrap_or(default)
}

/// The cost of building a pipeline with 200 material-sized hit groups, which compiles each closest hit cold, and of
/// building it again with one more group, which compiles that group and links the rest.
pub(super) fn building_a_pipeline_again_compiles_only_the_new_hit_group(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let groups = setting("SPIKE_GROUPS", 200);
	let spike = Spike::new(device, queue_handle);
	let hits = spike.closest_hits(device, groups + 1, 48);
	let (_, first) = spike.pipeline(device, &hits[..groups]);
	let (_, relinked) = spike.pipeline(device, &hits[..groups]);
	let (extended, extended_time) = spike.pipeline(device, &hits);
	let records = (0..groups as u32 + 1).collect::<Vec<_>>();
	let trace = spike.time(device, extended, groups as u32 + 1, &records);
	eprintln!(
		"{groups} hit groups of 48 statements: first build {first:?}, the same pipeline again {relinked:?}, with one more \
		 group {extended_time:?}. A trace in which every invocation calls its own group took {trace:?}."
	);
}

/// The cost of closest-hit calls when neighboring invocations call different hit groups, against the same calls
/// sorted by hit group.
pub(super) fn sorted_closest_hit_calls_against_random_ones(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	const GROUPS: u32 = 25;
	let rays = setting("SPIKE_THREADS", 16_384);
	let spike = Spike::new(device, queue_handle);
	let hash = |mut value: u32| {
		value ^= value >> 16;
		value = value.wrapping_mul(0x7feb_352d);
		value ^= value >> 15;
		value = value.wrapping_mul(0x846c_a68b);
		value ^ (value >> 16)
	};
	let random = (0..rays as u32).map(|ray| hash(ray) % GROUPS).collect::<Vec<_>>();
	let mut sorted = random.clone();
	sorted.sort_unstable();

	for statements in [8, 48] {
		let hits = spike.closest_hits(device, GROUPS as usize, statements);
		let (pipeline, _) = spike.pipeline(device, &hits);
		let random_time = spike.time(device, pipeline, GROUPS, &random);
		let sorted_time = spike.time(device, pipeline, GROUPS, &sorted);
		eprintln!(
			"{rays} calls over {GROUPS} groups of {statements} statements: {random_time:?} in random order, {sorted_time:?} \
			 sorted by group."
		);
	}
}
