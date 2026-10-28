//! The `ray_tracing` module builds Vulkan ray tracing pipelines and the shader binding tables their traces read.
//!
//! The GHI shades a hit by calling the closest hit of the hit's record from the ray generation shader, so on Vulkan a
//! closest-hit shader is a callable shader, and a shader binding table is the pipeline's callable region: record `i`
//! holds the group handle of the closest hit of hit group `i`'s entry. A ray generation shader calls it with
//! `executeCallableEXT(record, 0)`, so a trace has no miss, hit, or traversal stage, and rays are found with ray
//! queries. See the ray tracing design note, `docs/develop/rendering/ray-tracing.mdx`.
//!
//! Each shader compiles once, as a pipeline library of its own, the first time a pipeline uses it with a set of
//! specialization constants. A pipeline is then only linked from libraries, which is what keeps a pipeline built again
//! with more hit groups cheap.

use std::sync::Arc;

use ash::vk;
use smallvec::SmallVec;

use crate::graphics_hardware_interface;
use crate::vulkan::device::RayTracing;
use crate::vulkan::pipeline_binaries::{PipelineBinaries, create_pipeline};
use crate::vulkan::{PipelineLayout, Shader};

/// The `Library` struct owns one compiled ray tracing pipeline library: one shader with one set of specialization
/// constants, as the only shader group.
///
/// Pipelines linked from it hold it, so it stays alive for as long as any of them, and for as long as the cache that
/// compiled it keeps it.
pub(crate) struct Library {
	/// The device and the one device function the library calls, which a cache of hundreds of libraries keeps far
	/// smaller than a whole `ash::Device` each.
	device: vk::Device,
	destroy_pipeline: vk::PFN_vkDestroyPipeline,
	pipeline: vk::Pipeline,
}

impl Drop for Library {
	fn drop(&mut self) {
		// SAFETY: Every pipeline linked from the library holds it, so none of them is alive any more, and the device
		// outlives every library its contexts and factories compiled.
		unsafe { (self.destroy_pipeline)(self.device, self.pipeline, std::ptr::null()) };
	}
}

/// The `LibraryKey` struct names what a library compiled: a shader of its factory, its specialization constants, and
/// the resource layout its descriptor mappings point into.
struct LibraryKey {
	shader: usize,
	specialization: Box<[crate::pipelines::SpecializationMapEntry]>,
	/// Every resource the ray generation shader's sets declare, which fixes where in the heaps each one lives.
	declared: Box<[crate::shader::ShaderResourceDescriptor]>,
	push_constant_size: u32,
}

/// The `Libraries` struct is the cache of the ray tracing libraries one factory or context compiled.
///
/// A closest hit takes tens of milliseconds to compile and its library microseconds to link, so pipelines rebuilt as
/// materials load compile only the closest hits they didn't use before.
#[derive(Default)]
pub(crate) struct Libraries {
	entries: Vec<(LibraryKey, Arc<Library>)>,
}

impl Libraries {
	/// Drops the cache's hold on every library. Pipelines linked from them keep theirs.
	pub(crate) fn clear(&mut self) {
		self.entries.clear();
	}
}

/// The `RayTracingPipelineState` struct is what a trace needs of a linked ray tracing pipeline: the group handle of
/// each shader, which shader binding tables copy, and the stack its shaders need.
pub(crate) struct RayTracingPipelineState {
	/// The handle of the ray generation group, then the handle of the closest hit of each hit group, in group order.
	handles: Box<[u8]>,
	handle_size: usize,
	/// The stack bytes an invocation needs: the ray generation shader's, plus the most any closest hit needs, because
	/// closest hits call nothing.
	pub(crate) stack_size: u32,
	/// The libraries the pipeline was linked from, held so they live as long as it does.
	_libraries: Box<[Arc<Library>]>,
}

impl RayTracingPipelineState {
	/// Returns how many hit groups the pipeline has.
	pub(crate) fn hit_group_count(&self) -> usize {
		self.handles.len() / self.handle_size - 1
	}

	/// Returns the group handle of the ray generation shader.
	fn ray_generation_handle(&self) -> &[u8] {
		&self.handles[..self.handle_size]
	}

	/// Returns the group handle of the closest hit of hit group `group`.
	fn hit_group_handle(&self, group: usize) -> &[u8] {
		let start = (group + 1) * self.handle_size;
		&self.handles[start..start + self.handle_size]
	}
}

/// The `RayTracingPipeline` struct carries a linked Vulkan ray tracing pipeline that a factory thread built, until a
/// frame gives it a public GHI handle with [`crate::vulkan::Frame::intern_ray_tracing_pipeline`].
pub struct RayTracingPipeline {
	pub(crate) pipeline: vk::Pipeline,
	pub(crate) layout: PipelineLayout,
	pub(crate) state: RayTracingPipelineState,
}

/// Panics unless the sets of every closest-hit shader are the first sets of the ray generation shader, in the same
/// order, as the GHI requires of every backend.
fn check_hit_group_sets(shaders: &[Shader], builder: &crate::pipelines::ray_tracing::Builder) {
	let ray_generation = &shaders[builder.ray_generation.handle.0 as usize];
	for hit_group in builder.hit_groups {
		let closest_hit = &shaders[hit_group.handle.0 as usize];
		for (index, set) in closest_hit.descriptor_sets.iter().enumerate() {
			let matches = ray_generation
				.descriptor_sets
				.get(index)
				.is_some_and(|declared| declared.layout().identity() == set.layout().identity());
			assert!(
				matches,
				"Vulkan ray tracing pipeline creation failed because a closest-hit shader declares descriptor set {index} as layout `{}`, which the ray generation shader doesn't declare at that position. The most likely cause is that the closest-hit shader declares its sets in another order than the ray generation shader, or declares a set the ray generation shader leaves out. Declare the ray generation shader's first sets in the closest-hit shader, in the same order.",
				set.layout().name(),
			);
		}
	}
}

/// Creates the library of one shader of a ray tracing pipeline with the mappings of `layout`, or returns the one
/// `libraries` already holds for the same shader, constants, and layout.
fn library(
	device: &ash::Device,
	ray_tracing: &RayTracing,
	pipeline_binaries: Option<&PipelineBinaries>,
	shaders: &[Shader],
	libraries: &mut Libraries,
	layout: &PipelineLayout,
	declared: &[crate::shader::ShaderResourceDescriptor],
	parameter: &crate::pipelines::ShaderParameter,
) -> Arc<Library> {
	// Pipelines rebuilt as materials load find nearly every library cached, so the lookup copies nothing.
	let cached = libraries.entries.iter().find(|(key, _)| {
		key.shader == parameter.handle.0 as usize
			&& key.push_constant_size == layout.push_constant_size
			&& *key.specialization == *parameter.specialization_map
			&& *key.declared == *declared
	});
	if let Some((_, library)) = cached {
		return library.clone();
	}
	let key = LibraryKey {
		shader: parameter.handle.0 as usize,
		specialization: parameter.specialization_map.into(),
		declared: declared.into(),
		push_constant_size: layout.push_constant_size,
	};

	// The one group of the library is its shader, which a ray generation shader launches or calls as a callable.
	let groups = [vk::RayTracingShaderGroupCreateInfoKHR::default()
		.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
		.general_shader(0)
		.closest_hit_shader(vk::SHADER_UNUSED_KHR)
		.any_hit_shader(vk::SHADER_UNUSED_KHR)
		.intersection_shader(vk::SHADER_UNUSED_KHR)];
	let interface = library_interface();
	let parameters = std::slice::from_ref(parameter);
	let pipeline = crate::vulkan::context::with_shader_stages(shaders, parameters, layout, |stages, identified_stages| {
		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(0)
			.library_interface(&interface)
			.layout(vk::PipelineLayout::null());
		let compiled = stages.map(|stages| create_info.stages(stages));
		let identified = identified_stages.map(|stages| create_info.stages(stages));
		create_pipeline(
			pipeline_binaries,
			vk::PipelineCreateFlags2::LIBRARY_KHR,
			compiled.as_ref(),
			identified.as_ref(),
			|create_info| create_ray_tracing_pipeline(ray_tracing, create_info),
		)
	})
	.expect("Vulkan ray tracing library creation failed. The most likely cause is an invalid shader resource mapping, or a ray tracing shader whose SPIR-V the driver rejects.");

	let library = Arc::new(Library {
		device: device.handle(),
		destroy_pipeline: device.fp_v1_0().destroy_pipeline,
		pipeline,
	});
	libraries.entries.push((key, library.clone()));
	library
}

/// Describes the interface every library and pipeline of this backend shares. Shaders trace no rays with
/// `traceRayEXT` and have no hit attributes, because hits come from ray queries and reach closest hits as callable
/// data, so both sizes are zero.
fn library_interface() -> vk::RayTracingPipelineInterfaceCreateInfoKHR<'static> {
	vk::RayTracingPipelineInterfaceCreateInfoKHR::default()
		.max_pipeline_ray_payload_size(0)
		.max_pipeline_ray_hit_attribute_size(0)
}

/// Creates one ray tracing pipeline or library from `create_info`.
fn create_ray_tracing_pipeline(
	ray_tracing: &RayTracing,
	create_info: &vk::RayTracingPipelineCreateInfoKHR<'_>,
) -> Result<vk::Pipeline, vk::Result> {
	// SAFETY: The create info and everything it points at live until this call returns.
	unsafe {
		ray_tracing.pipeline.create_ray_tracing_pipelines(
			vk::DeferredOperationKHR::null(),
			vk::PipelineCache::null(),
			std::slice::from_ref(create_info),
			None,
		)
	}
	.map(|pipelines| pipelines[0])
	.map_err(|(_, result)| result)
}

/// Links a ray tracing pipeline from the libraries of its shaders, compiling the libraries `libraries` doesn't hold
/// yet.
///
/// The pipeline's resource layout reserves every resource the ray generation shader's sets declare, used or not, so a
/// library compiled for one version of a pipeline fits every later version with more hit groups. It only reads device
/// state, so factories call it on compiler threads and contexts call it for their own pipelines.
pub(crate) fn build_ray_tracing_pipeline(
	device: &ash::Device,
	ray_tracing: &RayTracing,
	descriptor_heap_properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
	pipeline_binaries: Option<&PipelineBinaries>,
	shaders: &[Shader],
	libraries: &mut Libraries,
	builder: crate::pipelines::ray_tracing::Builder,
) -> RayTracingPipeline {
	let ray_generation = &shaders[builder.ray_generation.handle.0 as usize];
	assert!(
		ray_generation.stage == crate::Stages::RAYGEN,
		"Vulkan ray tracing pipeline creation failed because its ray generation shader is a {:?} shader. The most likely cause is that the shaders were passed to ray_tracing::Builder::new in the wrong order. Pass the ray generation shader first and closest-hit shaders as the hit groups.",
		ray_generation.stage,
	);
	assert!(
		!builder.hit_groups.is_empty(),
		"Vulkan ray tracing pipeline creation failed because the pipeline has no hit group. The most likely cause is that it was built before any closest-hit shader was ready. Pass at least one closest-hit shader, such as a default one for surfaces whose own shader isn't ready.",
	);
	for hit_group in builder.hit_groups {
		let stage = shaders[hit_group.handle.0 as usize].stage;
		assert!(
			stage == crate::Stages::CLOSEST_HIT,
			"Vulkan ray tracing pipeline creation failed because a hit group shader is a {stage:?} shader. The most likely cause is that a shader of another stage was passed as a hit group. Pass only closest-hit shaders as hit groups.",
		);
	}
	check_hit_group_sets(shaders, &builder);

	// The layout reserves every resource the ray generation shader's sets declare, used or not, with no stage and no
	// access of their own. Each resource then keeps the heap offset that compiled libraries point at when a later
	// version of the pipeline links a closest hit that uses more of them.
	let declared = crate::shader::flatten_declared_resources(&ray_generation.descriptor_sets)
		.into_iter()
		.map(|resource| crate::shader::ShaderResourceDescriptor {
			access: crate::AccessPolicies::NONE,
			..resource
		})
		.collect::<Box<[_]>>();
	let stage_resources = std::iter::once((crate::Stages::empty(), &*declared))
		.chain(
			std::iter::once(&builder.ray_generation)
				.chain(builder.hit_groups)
				.map(|parameter| {
					let shader = &shaders[parameter.handle.0 as usize];
					(shader.stage, shader.shader_resource_descriptors.as_slice())
				}),
		)
		.collect::<SmallVec<[_; 8]>>();
	let layout =
		crate::vulkan::build_pipeline_layout(&stage_resources, builder.push_constant_ranges, descriptor_heap_properties);

	let mut library_of = |parameter| {
		library(
			device,
			ray_tracing,
			pipeline_binaries,
			shaders,
			libraries,
			&layout,
			&declared,
			parameter,
		)
	};
	// The ray generation library comes first, so its group is group 0, and each distinct closest hit follows once.
	let mut linked = SmallVec::<[Arc<Library>; 8]>::new();
	linked.push(library_of(&builder.ray_generation));
	let groups = builder
		.hit_groups
		.iter()
		.map(|hit_group| {
			let hit = library_of(hit_group);
			match linked.iter().position(|linked| Arc::ptr_eq(linked, &hit)) {
				Some(group) => group,
				None => {
					linked.push(hit);
					linked.len() - 1
				}
			}
		})
		.collect::<SmallVec<[_; 16]>>();

	let library_pipelines = linked.iter().map(|library| library.pipeline).collect::<SmallVec<[_; 8]>>();
	let library_info = vk::PipelineLibraryCreateInfoKHR::default().libraries(&library_pipelines);
	let interface = library_interface();
	// The stack size depends on which closest hits the pipeline links, so the pipeline sets it as it binds.
	let dynamic_states = [vk::DynamicState::RAY_TRACING_PIPELINE_STACK_SIZE_KHR];
	let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
	let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
		.library_info(&library_info)
		.library_interface(&interface)
		.max_pipeline_ray_recursion_depth(0)
		.dynamic_state(&dynamic_state)
		.layout(vk::PipelineLayout::null());
	// Linking compiles nothing, so the pipeline binary cache has nothing to keep for it.
	let pipeline = create_pipeline(
		None,
		vk::PipelineCreateFlags2::empty(),
		Some(&create_info),
		None,
		|create_info| create_ray_tracing_pipeline(ray_tracing, create_info),
	)
	.expect("Vulkan ray tracing pipeline linking failed. The most likely cause is that the driver ran out of memory.");

	let handle_size = ray_tracing.shader_group_handle_size as usize;
	// SAFETY: The pipeline has one group per library, in library order.
	let group_handles = unsafe {
		ray_tracing
			.pipeline
			.get_ray_tracing_shader_group_handles(pipeline, 0, linked.len() as u32, linked.len() * handle_size)
	}
	.expect(
		"Vulkan ray tracing shader group handles are unavailable. The most likely cause is that the driver ran out of memory.",
	);
	let mut handles = Vec::with_capacity((groups.len() + 1) * handle_size);
	handles.extend_from_slice(&group_handles[..handle_size]);
	for &group in &groups {
		handles.extend_from_slice(&group_handles[group * handle_size..(group + 1) * handle_size]);
	}

	let group_stack_size = |group: usize| {
		// SAFETY: `group` is below the pipeline's group count, and every group is a general group.
		unsafe {
			ray_tracing.pipeline.get_ray_tracing_shader_group_stack_size(
				pipeline,
				group as u32,
				vk::ShaderGroupShaderKHR::GENERAL,
			)
		}
	};
	// The ray generation shader calls closest hits one at a time, and closest hits call nothing.
	let callable_stack_size = (1..linked.len()).map(group_stack_size).max().unwrap_or(0);
	let stack_size = u32::try_from(group_stack_size(0) + callable_stack_size).expect(
		"Vulkan ray tracing pipeline stack exceeds 32 bits. The most likely cause is a fault in the driver's stack size report.",
	);

	RayTracingPipeline {
		pipeline,
		layout,
		state: RayTracingPipelineState {
			handles: handles.into_boxed_slice(),
			handle_size,
			stack_size,
			_libraries: linked.into_iter().collect(),
		},
	}
}

/// The `ShaderBindingTable` struct keeps the records of a shader binding table as the buffer a trace reads them from.
pub(crate) struct ShaderBindingTable {
	pub(crate) name: Option<String>,
	/// The pipeline the last write was for and the table it filled, or `None` before the first write.
	///
	/// Each write fills a new buffer, and the buffer it replaces lives until the frames recorded before the write
	/// complete, so traces recorded before a write keep their records.
	pub(crate) written: Option<WrittenTable>,
}

/// The `WrittenTable` struct is the buffer one write of a shader binding table filled, and the regions of it a trace
/// passes.
pub(crate) struct WrittenTable {
	pub(crate) pipeline: graphics_hardware_interface::PipelineHandle,
	pub(crate) buffer: vk::Buffer,
	pub(crate) allocation: graphics_hardware_interface::AllocationHandle,
	pub(crate) ray_generation: vk::StridedDeviceAddressRegionKHR,
	pub(crate) callable: vk::StridedDeviceAddressRegionKHR,
}

/// The `TableLayout` struct is where the records of a shader binding table go in its buffer: the ray generation
/// record first, then the callable region, which holds one record per hit record.
pub(crate) struct TableLayout {
	/// The bytes between two records, and the size of the ray generation region, which holds one.
	pub(crate) stride: u64,
	/// Where the callable region starts.
	pub(crate) callable_offset: u64,
	/// The alignment of the start of each region.
	pub(crate) base_alignment: u64,
	/// The bytes the buffer holds, with room to align its start.
	pub(crate) size: u64,
}

impl TableLayout {
	/// Lays out a table of `records` records for the device's handle size and alignments.
	pub(crate) fn new(ray_tracing: &RayTracing, records: usize) -> Self {
		let base_alignment = u64::from(ray_tracing.shader_group_base_alignment);
		let stride = u64::from(ray_tracing.shader_group_handle_size)
			.next_multiple_of(u64::from(ray_tracing.shader_group_handle_alignment));
		let callable_offset = stride.next_multiple_of(base_alignment);
		// A table without records still has its ray generation record, and the start of the buffer may need aligning.
		Self {
			stride,
			callable_offset,
			base_alignment,
			size: callable_offset + stride * records.max(1) as u64 + base_alignment,
		}
	}
}

/// Panics unless every entry of `hit_groups` names a hit group of the pipeline `state` describes.
pub(crate) fn check_hit_groups(state: &RayTracingPipelineState, hit_groups: &[u32]) {
	for (record, &group) in hit_groups.iter().enumerate() {
		assert!(
			(group as usize) < state.hit_group_count(),
			"Vulkan shader binding table write failed because record {record} names hit group {group}, and the pipeline has {} hit groups. The most likely cause is that the records were made for another version of the pipeline, with more hit groups. Write the table for the pipeline whose hit groups the records name.",
			state.hit_group_count(),
		);
	}
}

/// Writes the records of a table that `layout` lays out into `memory`, which holds [`TableLayout::size`] bytes at
/// `address`, and returns the regions a trace passes. Record `i` runs hit group `hit_groups[i]`, which
/// [`check_hit_groups`] checked.
pub(crate) fn fill_shader_binding_table(
	state: &RayTracingPipelineState,
	layout: &TableLayout,
	hit_groups: &[u32],
	memory: &mut [u8],
	address: vk::DeviceAddress,
) -> (vk::StridedDeviceAddressRegionKHR, vk::StridedDeviceAddressRegionKHR) {
	// Regions start at a multiple of the base alignment, so the records start at the first aligned address.
	let start = address.next_multiple_of(layout.base_alignment) - address;
	let records = &mut memory[start as usize..];
	records[..state.handle_size].copy_from_slice(state.ray_generation_handle());
	for (record, &group) in hit_groups.iter().enumerate() {
		let offset = (layout.callable_offset + layout.stride * record as u64) as usize;
		records[offset..offset + state.handle_size].copy_from_slice(state.hit_group_handle(group as usize));
	}
	let base = address + start;
	(
		vk::StridedDeviceAddressRegionKHR::default()
			.device_address(base)
			.stride(layout.stride)
			.size(layout.stride),
		vk::StridedDeviceAddressRegionKHR::default()
			.device_address(base + layout.callable_offset)
			.stride(layout.stride)
			.size(layout.stride * hit_groups.len() as u64),
	)
}
