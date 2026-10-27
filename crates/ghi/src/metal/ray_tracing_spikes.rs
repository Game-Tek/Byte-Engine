//! Stage G0 spikes for ray-tracing pipelines on Metal 4.
//!
//! A ray-generation shader runs as a compute kernel that calls closest-hit shaders through a visible function table.
//! Each closest hit is compiled in its own library, turned into a binary function, and linked into the kernel's
//! pipeline. The stage G2 tests in `tests/rendering/ray_tracing.rs` check that this path works; the spikes left here
//! measure it. They time compiling and linking, the cost of table calls that diverge, and what sorting hits by hit
//! group saves, which stages E5 and E7 budget against. Delete each spike once a later stage's tests or benchmarks cover
//! it.
//!
//! Run them with `cargo nextest run -p byte-engine-ghi --run-ignored only ray_tracing_spikes --no-capture`.
//!
//! Metal keeps compiled code in a cache that outlives the process, so every library replaces `NONCE` in its source
//! with a number picked per run. A source that uses it in live code compiles cold on each run.

use std::ptr::NonNull;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use block2::StackBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSArray, NSRange, NSString};
use objc2_metal as mtl;
use objc2_metal::{
	MTL4ArgumentTable as _, MTL4CommandBuffer as _, MTL4CommandEncoder as _, MTL4CommandQueue as _, MTL4CommitFeedback as _,
	MTL4Compiler as _, MTL4ComputeCommandEncoder as _, MTLBuffer as _, MTLComputePipelineState as _, MTLDevice as _,
	MTLResidencySet as _, MTLVisibleFunctionTable as _,
};

/// Declarations every library of a ray-tracing pipeline repeats, so the kernel and the visible functions agree on
/// the call signature: the payload, the hit attributes, and a context of set pointers that lets a visible function
/// reach resources the kernel received as arguments.
const PRELUDE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Payload { float3 albedo; float distance; };
struct HitAttributes { uint instance_id; uint geometry_id; uint primitive_id; float2 barycentrics; };
struct RtContext { constant void* sets[8]; constant void* push_constant; };
using HitFunction = void(thread Payload&, thread const HitAttributes&, thread const RtContext&);

struct PushConstant { uint frame; };
struct Set0 { device const uint* hit_groups [[id(0)]]; device float4* results [[id(1)]]; };
struct Set1 { device const float* material_values [[id(0)]]; };
"#;

/// The ray-generation stand-in: thread `i` calls hit group `hit_groups[i]`, or miss 0 for `0xffffffff`.
///
/// The kernel never reads set 1 itself. Only closest hits do, through the context, as material hit shaders will
/// read the scene set that the surfel tracer passes along.
const RAY_GENERATION: &str = r#"
kernel void ray_generation(
	uint index [[thread_position_in_grid]],
	constant PushConstant& push_constant [[buffer(15)]],
	constant Set0& set0 [[buffer(16)]],
	constant Set1& set1 [[buffer(17)]],
	visible_function_table<HitFunction> misses [[buffer(13)]],
	visible_function_table<HitFunction> hits [[buffer(14)]]) {
	const RtContext context = { { &set0, &set1 }, &push_constant };
	// Every hit and miss overwrites the distance, but the compiler cannot know that, so the nonce stays in the
	// kernel and keeps each run's pipelines out of Metal's cache.
	Payload payload = { float3(0.0), -NONCE.0 };
	const HitAttributes hit = { index, 0, index * 3, float2(0.25, 0.5) };
	const uint group = set0.hit_groups[index];
	if (group == 0xffffffffu) {
		misses[0](payload, hit, context);
	}
	if (group != 0xffffffffu) {
		hits[group](payload, hit, context);
	}
	set0.results[index] = float4(payload.albedo, payload.distance);
}
"#;

const MISS: &str = r#"
[[visible]] void miss(thread Payload& payload, thread const HitAttributes& hit, thread const RtContext& context) {
	payload.albedo = float3(7.0, 0.0, 0.0);
	payload.distance = -2.0;
}
"#;

/// MSL 4.1, which macOS 27 adds and which reports the function-table index of a hit. objc2-metal 0.3.2 predates it,
/// so the value is spelled out.
const LANGUAGE_VERSION: mtl::MTLLanguageVersion = mtl::MTLLanguageVersion((4 << 16) + 1);

/// The `Gpu` struct holds the raw Metal 4 objects the spikes compile and submit with.
struct Gpu {
	device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
	/// The number that replaces `NONCE` in sources. It stays below 2^24, so a `float` holds it exactly.
	nonce: u32,
}

impl Gpu {
	fn new() -> Self {
		let device = mtl::MTLCreateSystemDefaultDevice()
			.expect("No Metal device is available. The most likely cause is a machine without a GPU Metal supports.");
		let compiler = device
			.newCompilerWithDescriptor_error(&mtl::MTL4CompilerDescriptor::new())
			.expect("Metal 4 compiler creation failed. The most likely cause is an OS without Metal 4.");
		let queue = device
			.newMTL4CommandQueue()
			.expect("Metal 4 command queue creation failed. The most likely cause is an OS without Metal 4.");
		let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_micros() + 1;
		Self {
			device,
			compiler,
			queue,
			nonce,
		}
	}

	/// Compiles `PRELUDE` and `source` at [`LANGUAGE_VERSION`].
	fn library(&self, source: &str) -> Retained<ProtocolObject<dyn mtl::MTLLibrary>> {
		let options = mtl::MTLCompileOptions::new();
		options.setLanguageVersion(LANGUAGE_VERSION);
		let source = format!("{PRELUDE}{source}").replace("NONCE", &self.nonce.to_string());
		self.device
			.newLibraryWithSource_options_error(&NSString::from_str(&source), Some(&options))
			.unwrap_or_else(|error| panic!("Spike source failed to compile: {}", error.localizedDescription()))
	}

	/// Describes function `name` of `library`, specialized with `constants` as `uint` function constants 0, 1, ….
	fn function(
		&self,
		library: &ProtocolObject<dyn mtl::MTLLibrary>,
		name: &str,
		constants: &[u32],
	) -> Retained<mtl::MTL4FunctionDescriptor> {
		let library_function = mtl::MTL4LibraryFunctionDescriptor::new();
		library_function.setLibrary(Some(library));
		library_function.setName(Some(&NSString::from_str(name)));
		// SAFETY: MTL4LibraryFunctionDescriptor is a subclass of MTL4FunctionDescriptor.
		let library_function = unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(library_function) };
		if constants.is_empty() {
			return library_function;
		}
		let values = mtl::MTLFunctionConstantValues::new();
		// SAFETY: `constants` holds `constants.len()` contiguous `uint` values for the duration of the call.
		unsafe {
			values.setConstantValues_type_withRange(
				NonNull::from(constants).cast(),
				mtl::MTLDataType::UInt,
				NSRange::new(0, constants.len()),
			)
		};
		let specialized = mtl::MTL4SpecializedFunctionDescriptor::new();
		specialized.setFunctionDescriptor(Some(&library_function));
		specialized.setConstantValues(Some(&values));
		// SAFETY: MTL4SpecializedFunctionDescriptor is a subclass of MTL4FunctionDescriptor.
		unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(specialized) }
	}

	/// Compiles a visible function to machine code once, so any pipeline can link it.
	fn binary_function(
		&self,
		unique_name: &str,
		function: &mtl::MTL4FunctionDescriptor,
	) -> Result<Retained<ProtocolObject<dyn mtl::MTL4BinaryFunction>>, String> {
		self.binary_function_with(unique_name, function, mtl::MTL4BinaryFunctionOptions::PipelineIndependent)
	}

	fn binary_function_with(
		&self,
		unique_name: &str,
		function: &mtl::MTL4FunctionDescriptor,
		options: mtl::MTL4BinaryFunctionOptions,
	) -> Result<Retained<ProtocolObject<dyn mtl::MTL4BinaryFunction>>, String> {
		let descriptor = mtl::MTL4BinaryFunctionDescriptor::new();
		descriptor.setName(&NSString::from_str(unique_name));
		descriptor.setFunctionDescriptor(function);
		descriptor.setOptions(options);
		self.compiler
			.newBinaryFunctionWithDescriptor_compilerTaskOptions_error(&descriptor, None)
			.map_err(|error| error.localizedDescription().to_string())
	}

	/// Builds the ray-generation pipeline with `functions` linked, so its tables can name them.
	fn pipeline(
		&self,
		ray_generation: &mtl::MTL4FunctionDescriptor,
		functions: &[&ProtocolObject<dyn mtl::MTL4BinaryFunction>],
	) -> Result<Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>, String> {
		let descriptor = mtl::MTL4ComputePipelineDescriptor::new();
		descriptor.setComputeFunctionDescriptor(Some(ray_generation));
		descriptor.setSupportBinaryLinking(true);
		let linking = mtl::MTL4PipelineStageDynamicLinkingDescriptor::new();
		linking.setBinaryLinkedFunctions(Some(&NSArray::from_slice(functions)));
		linking.setMaxCallStackDepth(1);
		self.compiler
			.newComputePipelineStateWithDescriptor_dynamicLinkingDescriptor_compilerTaskOptions_error(
				&descriptor,
				Some(&linking),
				None,
			)
			.map_err(|error| error.localizedDescription().to_string())
	}

	/// Creates a visible function table of `pipeline` whose entry `i` calls `functions[i]`.
	fn table(
		&self,
		pipeline: &ProtocolObject<dyn mtl::MTLComputePipelineState>,
		functions: &[&ProtocolObject<dyn mtl::MTL4BinaryFunction>],
	) -> Retained<ProtocolObject<dyn mtl::MTLVisibleFunctionTable>> {
		let descriptor = mtl::MTLVisibleFunctionTableDescriptor::new();
		// SAFETY: A table of this many entries is what the dispatch indexes.
		unsafe { descriptor.setFunctionCount(functions.len()) };
		let table = pipeline.newVisibleFunctionTableWithDescriptor(&descriptor).expect(
			"Visible function table creation failed. The most likely cause is a pipeline built without binary linking.",
		);
		for (index, function) in functions.iter().enumerate() {
			let handle = pipeline.functionHandleWithBinaryFunction(function).expect(
				"Function handle lookup failed. The most likely cause is a binary function the pipeline does not link.",
			);
			// SAFETY: Every function shares the table's signature, and `index` is below the table's function count.
			unsafe { table.setFunction_atIndex(Some(&handle), index) };
		}
		table
	}

	fn buffer<T: Copy>(&self, values: &[T]) -> Retained<ProtocolObject<dyn mtl::MTLBuffer>> {
		let length = size_of_val(values).max(16);
		let buffer = self
			.device
			.newBufferWithLength_options(length, mtl::MTLResourceOptions::StorageModeShared)
			.expect("Spike buffer allocation failed. The most likely cause is that the device ran out of memory.");
		// SAFETY: The buffer is shared, at least `size_of_val(values)` bytes long, and no GPU work uses it yet.
		unsafe {
			std::ptr::copy_nonoverlapping(
				values.as_ptr().cast::<u8>(),
				buffer.contents().as_ptr().cast(),
				size_of_val(values),
			)
		};
		buffer
	}

	/// Builds an argument buffer whose entries are the GPU addresses of `buffers`, as a GHI descriptor set would.
	fn set(&self, buffers: &[&ProtocolObject<dyn mtl::MTLBuffer>]) -> Retained<ProtocolObject<dyn mtl::MTLBuffer>> {
		let addresses: Vec<u64> = buffers.iter().map(|buffer| buffer.gpuAddress()).collect();
		self.buffer(&addresses)
	}

	/// Runs `threads` threads of `pipeline` with buffers and tables bound at their indices, waits, and returns the
	/// GPU time Metal's commit feedback reports.
	fn dispatch(
		&self,
		pipeline: &ProtocolObject<dyn mtl::MTLComputePipelineState>,
		buffers: &[(usize, &ProtocolObject<dyn mtl::MTLBuffer>)],
		tables: &[(usize, &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>)],
		resident: &[&ProtocolObject<dyn mtl::MTLBuffer>],
		threads: usize,
		group_size: usize,
	) -> Duration {
		let table_descriptor = mtl::MTL4ArgumentTableDescriptor::new();
		table_descriptor.setMaxBufferBindCount(31);
		let arguments = self
			.device
			.newArgumentTableWithDescriptor_error(&table_descriptor)
			.expect("Argument table creation failed. The most likely cause is an unsupported bind count.");
		for (index, buffer) in buffers {
			// SAFETY: The buffer stays alive and resident until the dispatch completes.
			unsafe { arguments.setAddress_atIndex(buffer.gpuAddress(), *index) };
		}
		for (index, table) in tables {
			// SAFETY: The table stays alive and resident until the dispatch completes.
			unsafe { arguments.setResource_atBufferIndex(table.gpuResourceID(), *index) };
		}

		let mut allocations: Vec<&ProtocolObject<dyn mtl::MTLAllocation>> = vec![ProtocolObject::from_ref(pipeline)];
		allocations.extend(buffers.iter().map(|(_, buffer)| ProtocolObject::from_ref(*buffer)));
		allocations.extend(resident.iter().map(|buffer| ProtocolObject::from_ref(*buffer)));
		allocations.extend(tables.iter().map(|(_, table)| ProtocolObject::from_ref(*table)));
		self.submit(&allocations, |encoder| {
			encoder.setComputePipelineState(pipeline);
			encoder.setArgumentTable(Some(&arguments));
			encoder.dispatchThreads_threadsPerThreadgroup(
				mtl::MTLSize {
					width: threads,
					height: 1,
					depth: 1,
				},
				mtl::MTLSize {
					width: group_size,
					height: 1,
					depth: 1,
				},
			);
		})
	}

	/// Records `encode` into one command buffer with `allocations` resident, commits it, waits, and returns the GPU
	/// time Metal's commit feedback reports.
	fn submit(
		&self,
		allocations: &[&ProtocolObject<dyn mtl::MTLAllocation>],
		encode: impl FnOnce(&ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>),
	) -> Duration {
		let residency = self
			.device
			.newResidencySetWithDescriptor_error(&mtl::MTLResidencySetDescriptor::new())
			.expect("Residency set creation failed. The most likely cause is that the device ran out of residency sets.");
		for allocation in allocations {
			residency.addAllocation(allocation);
		}
		residency.commit();
		residency.requestResidency();

		let allocator = self
			.device
			.newCommandAllocator()
			.expect("Command allocator creation failed. The most likely cause is that the device ran out of memory.");
		let command_buffer = self
			.device
			.newCommandBuffer()
			.expect("Command buffer creation failed. The most likely cause is that the device ran out of memory.");
		command_buffer.beginCommandBufferWithAllocator(&allocator);
		command_buffer.useResidencySet(&residency);
		let encoder = command_buffer
			.computeCommandEncoder()
			.expect("Compute encoder creation failed. The most likely cause is an invalid command buffer state.");
		encode(&encoder);
		encoder.endEncoding();
		command_buffer.endCommandBuffer();

		let (sender, feedback) = std::sync::mpsc::sync_channel(1);
		let handler = StackBlock::new(move |feedback: NonNull<ProtocolObject<dyn mtl::MTL4CommitFeedback>>| {
			// SAFETY: Metal owns the feedback object for the whole call.
			let feedback = unsafe { feedback.as_ref() };
			let result = match feedback.error() {
				Some(error) => Err(error.localizedDescription().to_string()),
				None => Ok(Duration::from_secs_f64(feedback.GPUEndTime() - feedback.GPUStartTime())),
			};
			let _ = sender.try_send(result);
		});
		let options = mtl::MTL4CommitOptions::new();
		// SAFETY: Metal copies the block, and the channel it sends on outlives the wait below.
		unsafe { options.addFeedbackHandler(NonNull::from(&*handler).as_ptr()) };
		let mut command_buffers = [NonNull::from(&*command_buffer)];
		// SAFETY: The pointer addresses one retained command buffer, which outlives the call.
		unsafe {
			self.queue
				.commit_count_options(NonNull::new(command_buffers.as_mut_ptr()).unwrap(), 1, &options)
		};
		feedback
			.recv_timeout(Duration::from_secs(10))
			.expect(
				"Spike submission did not finish within 10 s. The most likely cause is a GPU fault from a bad function table call.",
			)
			.unwrap_or_else(|error| panic!("Spike submission failed on the GPU: {error}"))
	}
}

/// Reads `count` values back from a shared buffer the GPU wrote.
fn read<T: Copy>(buffer: &ProtocolObject<dyn mtl::MTLBuffer>, count: usize) -> Vec<T> {
	// SAFETY: The buffer is shared, holds at least `count` values of `T`, and the GPU work writing it completed.
	unsafe { std::slice::from_raw_parts(buffer.contents().as_ptr().cast::<T>(), count).to_vec() }
}

/// The resources every spike dispatch binds, and the hit groups each of its threads calls.
struct Bindings {
	push_constant: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	hit_groups: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	results: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	material_values: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	set0: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	set1: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	threads: usize,
}

impl Bindings {
	fn new(gpu: &Gpu, hit_groups: &[u32]) -> Self {
		let push_constant = gpu.buffer(&[3u32]);
		let hit_groups_buffer = gpu.buffer(hit_groups);
		let results = gpu.buffer(&vec![[0f32; 4]; hit_groups.len()]);
		let material_values = gpu.buffer(&[0.5f32, 0.75]);
		let set0 = gpu.set(&[&hit_groups_buffer, &results]);
		let set1 = gpu.set(&[&material_values]);
		Self {
			push_constant,
			hit_groups: hit_groups_buffer,
			results,
			material_values,
			set0,
			set1,
			threads: hit_groups.len(),
		}
	}

	/// Traces with `pipeline` and its two tables, and returns each thread's payload as albedo and distance.
	fn run(
		&self,
		gpu: &Gpu,
		pipeline: &ProtocolObject<dyn mtl::MTLComputePipelineState>,
		misses: &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>,
		hits: &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>,
	) -> Vec<[f32; 4]> {
		self.time(gpu, pipeline, &[(13, misses), (14, hits)]);
		read(&self.results, self.threads)
	}

	/// Runs `pipeline` with `tables` and returns its GPU time.
	fn time(
		&self,
		gpu: &Gpu,
		pipeline: &ProtocolObject<dyn mtl::MTLComputePipelineState>,
		tables: &[(usize, &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>)],
	) -> Duration {
		gpu.dispatch(
			pipeline,
			&[(15, &self.push_constant), (16, &self.set0), (17, &self.set1)],
			tables,
			&[&self.hit_groups, &self.results, &self.material_values],
			self.threads,
			self.threads.min(64),
		)
	}
}

/// A closest hit about the size of a material's: `STATEMENTS` dependent transcendental statements, different in every
/// function and every run. It writes `[index, material_values[0], instance_id, nonce]`, and the step keeps the math
/// live without changing the distance, since a sine or cosine is never near 1e30.
fn material_sized_closest_hit(index: usize) -> String {
	closest_hit_with_statements(index, 48)
}

fn closest_hit_with_statements(index: usize, statement_count: usize) -> String {
	closest_hit_source("[[visible]] void closest_hit", index, statement_count)
}

/// The body every generated closest hit shares, behind `declaration`, the attribute, return type and name.
fn closest_hit_source(declaration: &str, index: usize, statement_count: usize) -> String {
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
{declaration}(thread Payload& payload, thread const HitAttributes& hit, thread const RtContext& context) {{
	constant Set1& set1 = *static_cast<constant Set1*>(context.sets[1]);
	float3 v = float3(set1.material_values[0], set1.material_values[1], float(hit.primitive_id));
{statements}	payload.albedo = float3({index}.0, set1.material_values[0], float(hit.instance_id));
	payload.distance = NONCE.0 + step(1e30, abs(v.x + v.y + v.z));
}}
"#
	)
}

/// S4: the cost of compiling and linking 200 material-sized hit shaders, each from its own library, and of extending
/// a 199-function pipeline with one more. The first pass compiles cold; the second repeats it on Metal's cache, as a
/// later launch would. Every thread then calls its own group, which checks a 200-entry table.
#[test]
#[ignore = "Stage G0 spike; run it explicitly."]
fn linking_two_hundred_visible_functions() {
	const COUNT: usize = 200;
	let gpu = Gpu::new();
	let sources: Vec<String> = (0..COUNT).map(material_sized_closest_hit).collect();

	for pass in ["Cold", "Cached"] {
		let start = Instant::now();
		let ray_generation = gpu.function(&gpu.library(RAY_GENERATION), "ray_generation", &[]);
		let miss = gpu
			.binary_function("miss", &gpu.function(&gpu.library(MISS), "miss", &[]))
			.unwrap();
		let baseline = gpu.pipeline(&ray_generation, &[&miss]).unwrap();
		let baseline_time = start.elapsed();

		let start = Instant::now();
		let libraries: Vec<_> = sources.iter().map(|source| gpu.library(source)).collect();
		let libraries_time = start.elapsed();

		let start = Instant::now();
		let functions: Vec<_> = libraries
			.iter()
			.enumerate()
			.map(|(index, library)| {
				gpu.binary_function(&format!("closest_hit_{index}"), &gpu.function(library, "closest_hit", &[]))
					.unwrap()
			})
			.collect();
		let binary_time = start.elapsed();

		let mut linked: Vec<&ProtocolObject<dyn mtl::MTL4BinaryFunction>> =
			functions.iter().map(|function| &**function).collect();
		linked.push(&miss);
		let start = Instant::now();
		let pipeline = gpu.pipeline(&ray_generation, &linked).unwrap();
		let link_time = start.elapsed();

		let start = Instant::now();
		let partial = gpu.pipeline(&ray_generation, &linked[1..]).unwrap();
		let partial_time = start.elapsed();
		let start = Instant::now();
		let extended = partial
			.newComputePipelineStateWithBinaryFunctions_error(&NSArray::from_slice(&[linked[0]]))
			.unwrap_or_else(|error| panic!("Pipeline extension failed: {}", error.localizedDescription()));
		let extension_time = start.elapsed();

		let start = Instant::now();
		let hits = gpu.table(&pipeline, &linked[..COUNT]);
		let table_time = start.elapsed();

		eprintln!(
			"{pass}: kernel library and a miss-only pipeline {baseline_time:?}. {COUNT} hit libraries from source \
			 {libraries_time:?} ({:?} each). Binary functions {binary_time:?} ({:?} each). Pipeline linking all {COUNT} \
			 plus a miss {link_time:?}; linking {} plus a miss {partial_time:?}; extending that by one \
			 {extension_time:?}. Filling a {COUNT}-entry table {table_time:?}.",
			libraries_time / COUNT as u32,
			binary_time / COUNT as u32,
			COUNT - 1,
		);
		drop(baseline);

		for (pipeline, hits) in [(&pipeline, hits), (&extended, gpu.table(&extended, &linked[..COUNT]))] {
			let misses = gpu.table(pipeline, &[&miss]);
			let groups: Vec<u32> = (0..COUNT as u32).collect();
			let results = Bindings::new(&gpu, &groups).run(&gpu, pipeline, &misses, &hits);
			for (index, result) in results.iter().enumerate() {
				assert_eq!(
					*result,
					[index as f32, 0.5, index as f32, gpu.nonce as f32],
					"Thread {index} ran the wrong hit group."
				);
			}
		}
	}
}

/// S4, cost model: cold binary-function compile time by body size, binary-function option, and the number of threads
/// compiling at once. `SPIKE_STATEMENTS` (48), `SPIKE_OPTIONS` (`independent` or `none`), `SPIKE_THREADS` (1) and
/// `SPIKE_FUNCTIONS` (64) pick the case.
#[test]
#[ignore = "Stage G0 spike; run it explicitly."]
fn binary_function_compile_cost() {
	let setting = |name: &str, default: usize| {
		std::env::var(name)
			.ok()
			.and_then(|value| value.parse().ok())
			.unwrap_or(default)
	};
	let statements = setting("SPIKE_STATEMENTS", 48);
	let threads = setting("SPIKE_THREADS", 1);
	let count = setting("SPIKE_FUNCTIONS", 64);
	let options = match std::env::var("SPIKE_OPTIONS").as_deref() {
		Ok("none") => mtl::MTL4BinaryFunctionOptions::None,
		_ => mtl::MTL4BinaryFunctionOptions::PipelineIndependent,
	};
	let gpu = Gpu::new();
	let libraries: Vec<_> = (0..count)
		.map(|index| gpu.library(&closest_hit_with_statements(index, statements)))
		.collect();

	let start = Instant::now();
	std::thread::scope(|scope| {
		for thread in 0..threads {
			let gpu = &gpu;
			let libraries = &libraries;
			scope.spawn(move || {
				for index in (thread..count).step_by(threads) {
					let function = gpu.function(&libraries[index], "closest_hit", &[]);
					gpu.binary_function_with(&format!("closest_hit_{index}"), &function, options)
						.unwrap();
				}
			});
		}
	});
	let elapsed = start.elapsed();
	eprintln!(
		"{count} binary functions of {statements} statements, {options:?}, on {threads} threads: {elapsed:?} \
		 ({:?} per function of wall time).",
		elapsed / count as u32
	);
}

/// S5: the GPU cost of reaching hit groups through a visible function table against a `switch` over the same bodies
/// inlined into the kernel, the static-dispatch variant the plan keeps in reserve. Threads either share one group,
/// share a group per SIMD group, or pick groups at random, as incoherent surfel rays will.
#[test]
#[ignore = "Stage G0 spike; run it explicitly."]
fn visible_function_calls_against_an_inlined_switch() {
	const GROUPS: usize = 25;
	let setting = |name: &str, default: usize| {
		std::env::var(name)
			.ok()
			.and_then(|value| value.parse().ok())
			.unwrap_or(default)
	};
	let statements = setting("SPIKE_STATEMENTS", 8);
	let threads = setting("SPIKE_THREADS", 1 << 20);
	let gpu = Gpu::new();

	let ray_generation = gpu.function(&gpu.library(RAY_GENERATION), "ray_generation", &[]);
	let functions: Vec<_> = (0..GROUPS)
		.map(|index| {
			let library = gpu.library(&closest_hit_with_statements(index, statements));
			gpu.binary_function(&format!("closest_hit_{index}"), &gpu.function(&library, "closest_hit", &[]))
				.unwrap()
		})
		.collect();
	let miss = gpu
		.binary_function("miss", &gpu.function(&gpu.library(MISS), "miss", &[]))
		.unwrap();
	let mut linked: Vec<&ProtocolObject<dyn mtl::MTL4BinaryFunction>> = functions.iter().map(|function| &**function).collect();
	linked.push(&miss);
	let tables_pipeline = gpu.pipeline(&ray_generation, &linked).unwrap();
	let hits = gpu.table(&tables_pipeline, &linked[..GROUPS]);
	let misses = gpu.table(&tables_pipeline, &[&miss]);

	let bodies: String = (0..GROUPS)
		.map(|index| closest_hit_source(&format!("static void closest_hit_{index}"), index, statements))
		.collect();
	let cases: String = (0..GROUPS)
		.map(|index| format!("\t\tcase {index}: closest_hit_{index}(payload, hit, context); break;\n"))
		.collect();
	let switch_source = format!(
		r#"{bodies}
kernel void switch_generation(
	uint index [[thread_position_in_grid]],
	constant PushConstant& push_constant [[buffer(15)]],
	constant Set0& set0 [[buffer(16)]],
	constant Set1& set1 [[buffer(17)]]) {{
	const RtContext context = {{ {{ &set0, &set1 }}, &push_constant }};
	Payload payload = {{ float3(0.0), -NONCE.0 }};
	const HitAttributes hit = {{ index, 0, index * 3, float2(0.25, 0.5) }};
	switch (set0.hit_groups[index]) {{
{cases}		default: break;
	}}
	set0.results[index] = float4(payload.albedo, payload.distance);
}}
"#
	);
	let switch_pipeline = gpu
		.pipeline(&gpu.function(&gpu.library(&switch_source), "switch_generation", &[]), &[])
		.unwrap();

	// A 32-bit integer hash, so random groups do not follow thread order.
	let hash = |mut value: u32| {
		value ^= value >> 16;
		value = value.wrapping_mul(0x7feb_352d);
		value ^= value >> 15;
		value = value.wrapping_mul(0x846c_a68b);
		value ^ (value >> 16)
	};
	let patterns: [(&str, Vec<u32>); 3] = [
		("one group", vec![0; threads]),
		(
			"a group per SIMD group",
			(0..threads as u32).map(|thread| (thread / 32) % GROUPS as u32).collect(),
		),
		(
			"random groups",
			(0..threads as u32).map(|thread| hash(thread) % GROUPS as u32).collect(),
		),
	];

	for (pattern, groups) in &patterns {
		let bindings = Bindings::new(&gpu, groups);
		let tables: [(usize, &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>); 2] = [(13, &misses), (14, &hits)];
		let mut table_best = Duration::MAX;
		let mut switch_best = Duration::MAX;
		// Interleave the two kernels after a warm-up, so both see the same GPU clock state.
		for round in 0..12 {
			let table_time = bindings.time(&gpu, &tables_pipeline, &tables);
			let table_results: Vec<[f32; 4]> = read(&bindings.results, threads);
			let switch_time = bindings.time(&gpu, &switch_pipeline, &[]);
			let switch_results: Vec<[f32; 4]> = read(&bindings.results, threads);
			assert!(table_results == switch_results, "The two kernels disagree for {pattern}.");
			if round >= 2 {
				table_best = table_best.min(table_time);
				switch_best = switch_best.min(switch_time);
			}
		}
		eprintln!(
			"{threads} threads, {GROUPS} groups of {statements} statements, {pattern}: function table {table_best:?}, \
			 inlined switch {switch_best:?}."
		);
	}
}

/// A counting sort of ray keys by hit group: per-tile histograms, one scan over them, and a scatter, the passes a
/// sorted shading order would add between tracing and shading. Keys are below `BUCKETS`, and the ray count is a
/// multiple of `TILE`.
const SORT: &str = r#"
constant uint TILE = 256;
constant uint BUCKETS = 32;

kernel void sort_count(device const uint* keys [[buffer(0)]], device uint* tile_counts [[buffer(1)]],
	constant uint& tiles [[buffer(2)]], uint index [[thread_position_in_grid]],
	uint lane [[thread_index_in_threadgroup]], uint tile [[threadgroup_position_in_grid]]) {
	threadgroup atomic_uint histogram[BUCKETS];
	if (lane < BUCKETS) { atomic_store_explicit(&histogram[lane], 0u, memory_order_relaxed); }
	threadgroup_barrier(mem_flags::mem_threadgroup);
	atomic_fetch_add_explicit(&histogram[keys[index]], 1u, memory_order_relaxed);
	threadgroup_barrier(mem_flags::mem_threadgroup);
	if (lane < BUCKETS) { tile_counts[lane * tiles + tile] = atomic_load_explicit(&histogram[lane], memory_order_relaxed); }
}

// One threadgroup of 1024 threads turns the bucket-major tile counts into exclusive offsets.
kernel void sort_scan(device uint* tile_counts [[buffer(0)]], constant uint& count [[buffer(1)]],
	uint lane [[thread_index_in_threadgroup]], uint simd_lane [[thread_index_in_simdgroup]],
	uint simd [[simdgroup_index_in_threadgroup]]) {
	threadgroup uint simd_totals[32];
	const uint per_thread = (count + 1023) / 1024;
	const uint first = lane * per_thread;
	uint sum = 0;
	for (uint i = 0; i < per_thread; ++i) { if (first + i < count) { sum += tile_counts[first + i]; } }
	const uint prefix = simd_prefix_exclusive_sum(sum);
	if (simd_lane == 31) { simd_totals[simd] = prefix + sum; }
	threadgroup_barrier(mem_flags::mem_threadgroup);
	if (simd == 0) { simd_totals[simd_lane] = simd_prefix_exclusive_sum(simd_totals[simd_lane]); }
	threadgroup_barrier(mem_flags::mem_threadgroup);
	uint offset = simd_totals[simd] + prefix;
	for (uint i = 0; i < per_thread; ++i) {
		if (first + i < count) { const uint value = tile_counts[first + i]; tile_counts[first + i] = offset; offset += value; }
	}
}

kernel void sort_scatter(device const uint* keys [[buffer(0)]], device const uint* tile_offsets [[buffer(1)]],
	device uint* sorted_keys [[buffer(2)]], constant uint& tiles [[buffer(3)]], uint index [[thread_position_in_grid]],
	uint lane [[thread_index_in_threadgroup]], uint tile [[threadgroup_position_in_grid]]) {
	threadgroup atomic_uint ranks[BUCKETS];
	if (lane < BUCKETS) { atomic_store_explicit(&ranks[lane], 0u, memory_order_relaxed); }
	threadgroup_barrier(mem_flags::mem_threadgroup);
	const uint key = keys[index];
	const uint rank = atomic_fetch_add_explicit(&ranks[key], 1u, memory_order_relaxed);
	sorted_keys[tile_offsets[key * tiles + tile] + rank] = key;
}
"#;

/// S5, sorting: what the three sort passes cost for the ray budget, and what shading costs in the sorted order,
/// against shading in random order. Each pass is its own submission, so the sum leaves out the barriers a frame
/// would put between them.
#[test]
#[ignore = "Stage G0 spike; run it explicitly."]
fn sorting_rays_by_hit_group_before_shading() {
	const GROUPS: usize = 25;
	const TILE: usize = 256;
	const BUCKETS: usize = 32;
	let setting = |name: &str, default: usize| {
		std::env::var(name)
			.ok()
			.and_then(|value| value.parse().ok())
			.unwrap_or(default)
	};
	let statements = setting("SPIKE_STATEMENTS", 48);
	let rays = setting("SPIKE_THREADS", 16_384);
	let tiles = rays / TILE;
	let gpu = Gpu::new();

	let ray_generation = gpu.function(&gpu.library(RAY_GENERATION), "ray_generation", &[]);
	let functions: Vec<_> = (0..GROUPS)
		.map(|index| {
			let library = gpu.library(&closest_hit_with_statements(index, statements));
			gpu.binary_function(&format!("closest_hit_{index}"), &gpu.function(&library, "closest_hit", &[]))
				.unwrap()
		})
		.collect();
	let miss = gpu
		.binary_function("miss", &gpu.function(&gpu.library(MISS), "miss", &[]))
		.unwrap();
	let mut linked: Vec<&ProtocolObject<dyn mtl::MTL4BinaryFunction>> = functions.iter().map(|function| &**function).collect();
	linked.push(&miss);
	let shading = gpu.pipeline(&ray_generation, &linked).unwrap();
	let hits = gpu.table(&shading, &linked[..GROUPS]);
	let misses = gpu.table(&shading, &[&miss]);

	let sort = gpu.library(SORT);
	let count = gpu.pipeline(&gpu.function(&sort, "sort_count", &[]), &[]).unwrap();
	let scan = gpu.pipeline(&gpu.function(&sort, "sort_scan", &[]), &[]).unwrap();
	let scatter = gpu.pipeline(&gpu.function(&sort, "sort_scatter", &[]), &[]).unwrap();

	let hash = |mut value: u32| {
		value ^= value >> 16;
		value = value.wrapping_mul(0x7feb_352d);
		value ^= value >> 15;
		value = value.wrapping_mul(0x846c_a68b);
		value ^ (value >> 16)
	};
	let random: Vec<u32> = (0..rays as u32).map(|ray| hash(ray) % GROUPS as u32).collect();
	let keys = gpu.buffer(&random);
	let tile_counts = gpu.buffer(&vec![0u32; BUCKETS * tiles]);
	let sorted_keys = gpu.buffer(&vec![0u32; rays]);
	let tiles_value = gpu.buffer(&[tiles as u32]);
	let count_value = gpu.buffer(&[(BUCKETS * tiles) as u32]);
	let unsorted = Bindings::new(&gpu, &random);
	let tables: [(usize, &ProtocolObject<dyn mtl::MTLVisibleFunctionTable>); 2] = [(13, &misses), (14, &hits)];

	let mut best = [Duration::MAX; 5];
	for round in 0..12 {
		let times = [
			gpu.dispatch(
				&count,
				&[(0, &keys), (1, &tile_counts), (2, &tiles_value)],
				&[],
				&[],
				rays,
				TILE,
			),
			gpu.dispatch(&scan, &[(0, &tile_counts), (1, &count_value)], &[], &[], 1024, 1024),
			gpu.dispatch(
				&scatter,
				&[(0, &keys), (1, &tile_counts), (2, &sorted_keys), (3, &tiles_value)],
				&[],
				&[],
				rays,
				TILE,
			),
			Bindings::new(&gpu, &read::<u32>(&sorted_keys, rays)).time(&gpu, &shading, &tables),
			unsorted.time(&gpu, &shading, &tables),
		];
		if round >= 2 {
			for (best, time) in best.iter_mut().zip(times) {
				*best = (*best).min(time);
			}
		}
	}

	let sorted: Vec<u32> = read(&sorted_keys, rays);
	let mut expected = random.clone();
	expected.sort_unstable();
	assert_eq!(sorted, expected, "The counting sort did not order the keys.");
	let [count_time, scan_time, scatter_time, sorted_time, unsorted_time] = best;
	eprintln!(
		"{rays} rays, {GROUPS} groups of {statements} statements: count {count_time:?}, scan {scan_time:?}, scatter \
		 {scatter_time:?}, shading in sorted order {sorted_time:?}; total {:?} against {unsorted_time:?} shading in \
		 random order.",
		count_time + scan_time + scatter_time + sorted_time
	);
}
