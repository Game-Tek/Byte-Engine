//! The Vulkan render backend.

use ash::{vk, Entry};

use crate::render_backend;

#[derive(Clone, Copy)]
pub(crate) struct Surface {
	surface: vk::SurfaceKHR,
	surface_capabilities: vk::SurfaceCapabilitiesKHR,
	surface_format: vk::SurfaceFormatKHR,
	surface_present_mode: vk::PresentModeKHR,
}

#[derive(Clone, Copy)]
pub(crate) struct Swapchain {
	swapchain: vk::SwapchainKHR,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSetLayout {
	descriptor_set_layout: vk::DescriptorSetLayout,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSet {
	descriptor_set: vk::DescriptorSet,
}

#[derive(Clone, Copy)]
pub(crate) struct PipelineLayout {
	pipeline_layout: vk::PipelineLayout,
}

#[derive(Clone, Copy)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
}

#[derive(Clone, Copy)]
pub(crate) struct CommandBuffer {
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone, Copy)]
pub(crate) struct Allocation {
	memory: vk::DeviceMemory,
}

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	buffer: vk::Buffer,
	device_address: vk::DeviceAddress,
}

#[derive(Clone, Copy)]
pub(crate) struct Synchronizer {
	fence: vk::Fence,
	semaphore: vk::Semaphore,
}

#[derive(Clone, Copy)]
pub(crate) struct Sampler {
	sampler: vk::Sampler,
}

#[derive(Clone, Copy)]
pub(crate) struct Texture {
	image: vk::Image,
}

#[derive(Clone, Copy)]
pub(crate) struct TextureView {
	image_view: vk::ImageView,
}

#[derive(Clone, Copy)]
pub(crate) struct Shader {
	shader_module: vk::ShaderModule,
	stage: vk::ShaderStageFlags
}
#[derive(Clone, Copy)]
pub(crate) struct AccelerationStructure {
	acceleration_structure: vk::AccelerationStructureKHR,
}

pub(crate) struct VulkanRenderBackend {
	entry: ash::Entry,
	instance: ash::Instance,
	debug_utils: ash::extensions::ext::DebugUtils,
	debug_utils_messenger: vk::DebugUtilsMessengerEXT,
	physical_device: vk::PhysicalDevice,
	device: ash::Device,
	queue_family_index: u32,
	queue: vk::Queue,
	swapchain: ash::extensions::khr::Swapchain,
	surface: ash::extensions::khr::Surface,
	acceleration_structure: ash::extensions::khr::AccelerationStructure,
	ray_tracing_pipeline: ash::extensions::khr::RayTracingPipeline,
	dynamic_rendering: ash::extensions::khr::DynamicRendering,
}

static mut counter: u32 = 0;

unsafe extern "system" fn vulkan_debug_utils_callback(message_severity: vk::DebugUtilsMessageSeverityFlagsEXT, message_type: vk::DebugUtilsMessageTypeFlagsEXT, p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT, _p_user_data: *mut std::ffi::c_void,) -> vk::Bool32 {
    let message = std::ffi::CStr::from_ptr((*p_callback_data).p_message);
    let severity = format!("{:?}", message_severity).to_lowercase();
    let ty = format!("{:?}", message_type).to_lowercase();
    println!("[Debug][{}][{}] {:?}", severity, ty, message);

	match message_severity {
		vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
			counter += 1;
		}
		_ => {}
	}

    vk::FALSE
}

fn to_clear_value(clear: crate::RGBA) -> vk::ClearValue {
	vk::ClearValue {
		color: vk::ClearColorValue {
			float32: [clear.r, clear.g, clear.b, clear.a],
		},
	}
}

fn texture_format_and_resource_use_to_image_layout(_texture_format: render_backend::TextureFormats, layout: render_backend::Layouts, access: Option<crate::render_backend::AccessPolicies>) -> vk::ImageLayout {
	match layout {
		render_backend::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		render_backend::Layouts::RenderTarget => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
		render_backend::Layouts::Transfer => {
			match access {
				Some(a) => {
					if a.intersects(render_backend::AccessPolicies::READ) {
						vk::ImageLayout::TRANSFER_SRC_OPTIMAL
					} else if a.intersects(render_backend::AccessPolicies::WRITE) {
						vk::ImageLayout::TRANSFER_DST_OPTIMAL
					} else {
						vk::ImageLayout::UNDEFINED
					}
				}
				None => vk::ImageLayout::UNDEFINED
			}
		}
		render_backend::Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		render_backend::Layouts::Texture => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
	}
}

fn to_load_operation(value: bool) -> vk::AttachmentLoadOp {
	if value {
		vk::AttachmentLoadOp::LOAD
	} else {
		vk::AttachmentLoadOp::CLEAR
	}
}

fn to_store_operation(value: bool) -> vk::AttachmentStoreOp {
	if value {
		vk::AttachmentStoreOp::STORE
	} else {
		vk::AttachmentStoreOp::DONT_CARE
	}
}

fn to_format(format: render_backend::TextureFormats) -> vk::Format {
	match format {
		crate::render_backend::TextureFormats::RGBAu8 => vk::Format::R8G8B8A8_UNORM,
		crate::render_backend::TextureFormats::RGBAu16 => vk::Format::R16G16B16A16_SFLOAT,
		crate::render_backend::TextureFormats::RGBAu32 => vk::Format::R32G32B32A32_SFLOAT,
		crate::render_backend::TextureFormats::RGBAf16 => vk::Format::R16G16B16A16_SFLOAT,
		crate::render_backend::TextureFormats::RGBAf32 => vk::Format::R32G32B32A32_SFLOAT,
		crate::render_backend::TextureFormats::RGBu10u10u11 => vk::Format::R16G16_S10_5_NV,
		crate::render_backend::TextureFormats::BGRAu8 => vk::Format::B8G8R8A8_SRGB,
		crate::render_backend::TextureFormats::Depth32 => vk::Format::D32_SFLOAT,
	}
}

fn to_shader_stage_flags(shader_type: crate::render_backend::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		crate::render_backend::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		crate::render_backend::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		crate::render_backend::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,		
	}
}

fn to_pipeline_stage_flags(stages: crate::render_backend::Stages) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(crate::render_backend::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER
	}

	if stages.contains(crate::render_backend::Stages::FRAGMENT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
	}

	if stages.contains(crate::render_backend::Stages::COMPUTE) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
	}

	if stages.contains(crate::render_backend::Stages::TRANSFER) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::TRANSFER
	}

	pipeline_stage_flags
}

fn to_access_flags(accesses: crate::render_backend::AccessPolicies, stages: crate::render_backend::Stages) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(crate::render_backend::AccessPolicies::READ) {
		if stages.intersects(crate::render_backend::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
	}

	if accesses.contains(crate::render_backend::AccessPolicies::WRITE) {
		if stages.intersects(crate::render_backend::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_WRITE
		}
	}

	access_flags
}

impl Into<vk::ShaderStageFlags> for render_backend::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		if self.intersects(render_backend::Stages::VERTEX) {
			shader_stage_flags |= vk::ShaderStageFlags::VERTEX
		}

		if self.intersects(render_backend::Stages::FRAGMENT) {
			shader_stage_flags |= vk::ShaderStageFlags::FRAGMENT
		}

		if self.intersects(render_backend::Stages::COMPUTE) {
			shader_stage_flags |= vk::ShaderStageFlags::COMPUTE
		}

		shader_stage_flags
	}
}

impl VulkanRenderBackend {
	pub fn new() -> VulkanRenderBackend {
		let entry: ash::Entry = Entry::linked();

		let application_info = vk::ApplicationInfo::default()
			.api_version(vk::make_api_version(0, 1, 3, 0));

		let layer_names = [
			#[cfg(debug_assertions)]
			std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap().as_ptr(),
			/*std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_LUNARG_api_dump\0").unwrap().as_ptr(),*/
		];

		let extension_names = [
			#[cfg(debug_assertions)]
			ash::extensions::ext::DebugUtils::NAME.as_ptr(),
			ash::extensions::khr::Surface::NAME.as_ptr(),
			ash::extensions::khr::XcbSurface::NAME.as_ptr(),
		];

		let instance_create_info = vk::InstanceCreateInfo::default()
			.application_info(&application_info)
			.enabled_layer_names(&layer_names)
			.enabled_extension_names(&extension_names)
			/* .build() */;


		let instance = unsafe { entry.create_instance(&instance_create_info, None).expect("No instance") };

		let debug_utils = ash::extensions::ext::DebugUtils::new(&entry, &instance);

		let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
			.message_severity(
				vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
					| vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
			)
			.message_type(
				vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
					| vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
					| vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
			)
			.pfn_user_callback(Some(vulkan_debug_utils_callback));

		let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).expect("Debug Utils Callback") };

		let physical_devices = unsafe { instance.enumerate_physical_devices().expect("No physical devices.") };

		let physical_device;

		{
			let best_physical_device = crate::render_system::select_by_score(physical_devices.as_slice(), |physical_device| {
				let properties = unsafe { instance.get_physical_device_properties(*physical_device) };

				let mut device_score = 0 as i64;

				device_score += match properties.device_type {
					vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
					_ => 0,
				};

				device_score += properties.limits.max_image_dimension2_d as i64;

				device_score
			});

			physical_device = *best_physical_device.expect("No physical devices.");
		}

		let queue_family_properties = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

		let queue_family_index = queue_family_properties
			.iter()
			.enumerate()
			.find_map(|(index, ref info)| {
				let supports_graphics = info.queue_flags.contains(vk::QueueFlags::GRAPHICS);
				let supports_compute = info.queue_flags.contains(vk::QueueFlags::COMPUTE);
				let supports_transfer = info.queue_flags.contains(vk::QueueFlags::TRANSFER);

				if supports_graphics && supports_compute && supports_transfer {
					Some(index as u32)
				} else {
					None
				}
			})
			.expect("No queue family index found.");

		let device_extension_names = [
			ash::extensions::khr::Swapchain::NAME.as_ptr(),
			ash::extensions::khr::DynamicRendering::NAME.as_ptr(),
		];

		let queue_create_infos = [vk::DeviceQueueCreateInfo::default()
				.queue_family_index(queue_family_index)
				.queue_priorities(&[1.0])
				/* .build() */];

		let mut buffer_device_address_features = vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true);
		let mut dynamic_rendering_features = vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);
		let mut synchronization2_features = vk::PhysicalDeviceSynchronization2FeaturesKHR::default().synchronization2(true);

		let enabled_physical_device_features = vk::PhysicalDeviceFeatures::default();

  		let device_create_info = vk::DeviceCreateInfo::default()
			.push_next(&mut buffer_device_address_features/* .build() */)
			.push_next(&mut dynamic_rendering_features/* .build() */)
			.push_next(&mut synchronization2_features/* .build() */)
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names)
			.enabled_features(&enabled_physical_device_features/* .build() */)
			/* .build() */;

		let device: ash::Device = unsafe { instance.create_device(physical_device, &device_create_info, None).expect("No device") };

		let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

		let acceleration_structure = ash::extensions::khr::AccelerationStructure::new(&instance, &device);
		let ray_tracing_pipeline = ash::extensions::khr::RayTracingPipeline::new(&instance, &device);
		let dynamic_rendering = ash::extensions::khr::DynamicRendering::new(&instance, &device);

		let swapchain = ash::extensions::khr::Swapchain::new(&instance, &device);
		let surface = ash::extensions::khr::Surface::new(&entry, &instance);

		VulkanRenderBackend { 
			entry,
			instance,
			debug_utils,
			debug_utils_messenger,
			physical_device,
			device,
			queue_family_index,
			queue,
			swapchain,
			surface,
			acceleration_structure,
			ray_tracing_pipeline,
			dynamic_rendering,
		}
	}
}

impl render_backend::RenderBackend for VulkanRenderBackend {
	fn get_log_count(&self) -> u32 {
		unsafe { counter }
	}

	fn create_descriptor_set_layout(&self, bindings: &[render_backend::DescriptorSetLayoutBinding]) -> render_backend::DescriptorSetLayout {
		let mut ll = Vec::new();

		let descriptor_set_layout_bindings = bindings
			.iter()
			.map(|binding| {
				let b = vk::DescriptorSetLayoutBinding::default()
					.binding(binding.binding)
					.descriptor_type(match binding.descriptor_type {
						render_backend::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
						render_backend::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
						render_backend::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
						render_backend::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
						render_backend::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
					})
					.descriptor_count(binding.descriptor_count)
					.stage_flags(binding.stage_flags.into());

				if let Some(immutable_samplers) = &binding.immutable_samplers {
					let l = immutable_samplers.iter().map(|sampler| sampler.vulkan_sampler.sampler).collect::<Vec<_>>();
					let b = b.immutable_samplers(unsafe { std::slice::from_raw_parts(l.as_ptr(), l.len()) }); // WARNING: Don't how else to return a slice of l which gets stored in an outer scope, which should be safe since l is not dropped.
					ll.push(l);
					b
				} else {
					b
				}
			})
			.collect::<Vec<_>>();

		let descriptor_set_layout_create_info = vk::DescriptorSetLayoutCreateInfo::default()
			.bindings(&descriptor_set_layout_bindings)
			/* .build() */;

		let descriptor_set_layout = unsafe { self.device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None).expect("No descriptor set layout") };

		render_backend::DescriptorSetLayout {
			vulkan_descriptor_set_layout: DescriptorSetLayout {
				descriptor_set_layout,
			},
		}
	}

	fn create_descriptor_set(&self, descriptor_set_layout: &render_backend::DescriptorSetLayout, bindings: &[render_backend::DescriptorSetLayoutBinding]) -> render_backend::DescriptorSet {
		let pool_sizes = bindings
			.iter()
			.map(|binding| {
				vk::DescriptorPoolSize::default()
					.ty(match binding.descriptor_type {
						render_backend::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
						render_backend::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
						render_backend::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
						render_backend::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
						render_backend::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
					})
					.descriptor_count(binding.descriptor_count)
					/* .build() */
			})
			.collect::<Vec<_>>();

		let descriptor_pool_create_info = vk::DescriptorPoolCreateInfo::default()
			.max_sets(3)
			.pool_sizes(&pool_sizes);

		let descriptor_pool = unsafe { self.device.create_descriptor_pool(&descriptor_pool_create_info, None).expect("No descriptor pool") };

		let descriptor_set_layouts = [unsafe { descriptor_set_layout.vulkan_descriptor_set_layout.descriptor_set_layout }];

		let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
			.descriptor_pool(descriptor_pool)
			.set_layouts(&descriptor_set_layouts)
			/* .build() */;

		let descriptor_set = unsafe { self.device.allocate_descriptor_sets(&descriptor_set_allocate_info).expect("No descriptor set") };

		render_backend::DescriptorSet {
			vulkan_descriptor_set: DescriptorSet {
				descriptor_set: descriptor_set[0],
			},
		}
	}

	fn write_descriptors(&self, descriptor_set_writes: &[render_backend::DescriptorSetWrite]) {
		for descriptor_set_write in descriptor_set_writes {
			let mut buffers: Vec<vk::DescriptorBufferInfo> = Vec::new();
			let mut images: Vec<vk::DescriptorImageInfo> = Vec::new();

			let descriptor_type = match descriptor_set_write.descriptor_type {
				render_backend::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
				render_backend::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
				render_backend::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
				render_backend::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
				render_backend::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
			};

			let write_info = vk::WriteDescriptorSet::default()
				.dst_set(unsafe { descriptor_set_write.descriptor_set.vulkan_descriptor_set.descriptor_set })
				.dst_binding(descriptor_set_write.binding)
				.dst_array_element(descriptor_set_write.array_element)
				.descriptor_type(descriptor_type)
				;

			let write_info = match descriptor_set_write.descriptor_info {
				render_backend::DescriptorInfo::Buffer { buffer, offset, range } => {
					let a = vk::DescriptorBufferInfo::default()
						.buffer(unsafe { buffer.vulkan_buffer.buffer })
						.offset(offset)
						.range(range);
					buffers.push(a);
					write_info.buffer_info(&buffers)
				},
				render_backend::DescriptorInfo::Texture { texture, format, layout } => {
					let a = vk::DescriptorImageInfo::default()
					.image_layout(texture_format_and_resource_use_to_image_layout(format, layout, None))
					.image_view(unsafe { texture.vulkan_texture_view.image_view });
					images.push(a);
					write_info.image_info(&images)
				},
				_ => panic!("Invalid descriptor info"),
			};

			unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
		}

	}

	fn create_command_buffer(&self) -> crate::render_backend::CommandBuffer {
		let command_pool_create_info = vk::CommandPoolCreateInfo::default()
			.flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
			.queue_family_index(self.queue_family_index)
			/* .build() */;

		let command_pool = unsafe { self.device.create_command_pool(&command_pool_create_info, None).expect("No command pool") };

		let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
			.command_pool(command_pool)
			.level(vk::CommandBufferLevel::PRIMARY)
			.command_buffer_count(1)
			/* .build() */;

		let command_buffer = unsafe { self.device.allocate_command_buffers(&command_buffer_allocate_info).expect("No command buffer") };

		crate::render_backend::CommandBuffer {
			vulkan_command_buffer: CommandBuffer {
				command_buffer: command_buffer[0],
			},
		}
	}

	fn create_pipeline_layout(&self, descriptor_set_layouts: &[crate::render_backend::DescriptorSetLayout]) -> crate::render_backend::PipelineLayout {
		let push_constant_ranges = [vk::PushConstantRange::default().size(64).offset(0).stage_flags(vk::ShaderStageFlags::VERTEX)];
		let set_layouts = descriptor_set_layouts.iter().map(|set_layout| unsafe { set_layout.vulkan_descriptor_set_layout.descriptor_set_layout }).collect::<Vec<_>>();

  		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&set_layouts)
			.push_constant_ranges(&push_constant_ranges)
			/* .build() */;

		let pipeline_layout = unsafe { self.device.create_pipeline_layout(&pipeline_layout_create_info, None).expect("No pipeline layout") };

		crate::render_backend::PipelineLayout {
			vulkan_pipeline_layout: PipelineLayout {
				pipeline_layout,
			},
		}
	}

	fn create_shader(&self, stage: crate::render_backend::ShaderTypes, shader: &[u8]) -> crate::render_backend::Shader {
		let shader_module_create_info = vk::ShaderModuleCreateInfo::default()
			.code(unsafe { shader.align_to::<u32>().1 })
			/* .build() */;

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).expect("No shader module") };

		crate::render_backend::Shader {
			vulkan_shader: Shader {
				shader_module,
				stage: to_shader_stage_flags(stage),
			},
		}
	}

	fn create_pipeline(&self, pipeline_layout: &render_backend::PipelineLayout, shaders: &[render_backend::Shader]) -> crate::render_backend::Pipeline {
		let stages = shaders
			.iter()
			.map(|shader| {
				vk::PipelineShaderStageCreateInfo::default()
					.stage(unsafe { shader.vulkan_shader.stage })
					.module(unsafe { shader.vulkan_shader.shader_module })
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					/* .build() */
			})
			.collect::<Vec<_>>();

		let viewports = [vk::Viewport::default()
			.x(0.0)
			.y(9.0)
			.width(16.0)
			.height(-9.0)
			.min_depth(0.0)
			.max_depth(1.0)
			/* .build() */];

		let scissors = [vk::Rect2D::default()
			.offset(vk::Offset2D { x: 0, y: 0 })
			.extent(vk::Extent2D { width: 16, height: 9 })
			/* .build() */];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors)
			/* .build() */;

		let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
			.color_attachment_formats(&[vk::Format::R8G8B8A8_UNORM])
			.depth_attachment_format(vk::Format::UNDEFINED)
			/* .build() */;

		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
			.dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR])
			/* .build() */;

		let attachments = [
			vk::PipelineColorBlendAttachmentState::default()
				.color_write_mask(vk::ColorComponentFlags::RGBA)
				.blend_enable(false)
				.src_color_blend_factor(vk::BlendFactor::ONE)
				.dst_color_blend_factor(vk::BlendFactor::ZERO)
				.color_blend_op(vk::BlendOp::ADD)
				.src_alpha_blend_factor(vk::BlendFactor::ONE)
				.dst_alpha_blend_factor(vk::BlendFactor::ZERO)
				.alpha_blend_op(vk::BlendOp::ADD)
				/* .build() */,
		];

		let color_blend_state = &vk::PipelineColorBlendStateCreateInfo::default()
			.logic_op_enable(false)
			.logic_op(vk::LogicOp::COPY)
			.attachments(&attachments)
			.blend_constants([0.0, 0.0, 0.0, 0.0])
			/* .build() */;

		let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
			.sample_shading_enable(false)
			.rasterization_samples(vk::SampleCountFlags::TYPE_1)
			.min_sample_shading(1.0)
			.alpha_to_coverage_enable(false)
			.alpha_to_one_enable(false)
			/* .build() */;

		let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
			.depth_test_enable(false)
			.depth_write_enable(false)
			.depth_compare_op(vk::CompareOp::ALWAYS)
			.depth_bounds_test_enable(false)
			.stencil_test_enable(false)
			.front(vk::StencilOpState::default()/* .build() */)
			.back(vk::StencilOpState::default()/* .build() */)
			/* .build() */;

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false)
			/* .build() */;

		let vertex_input_attribute_descriptions = [
			vk::VertexInputAttributeDescription::default()
				.binding(0)
				.location(0)
				.format(vk::Format::R32G32B32_SFLOAT)
				.offset(0)
				/* .build() */,
			vk::VertexInputAttributeDescription::default()
				.binding(0)
				.location(1)
				.format(vk::Format::R32G32B32A32_SFLOAT)
				.offset(12)
				/* .build() */,
		];

		let vertex_binding_descriptions = [
			vk::VertexInputBindingDescription::default()
				.binding(0)
				.stride(28)
				.input_rate(vk::VertexInputRate::VERTEX)
				/* .build() */,
		];

		let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
			.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
			.vertex_binding_descriptions(&vertex_binding_descriptions)
			/* .build() */;

		let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
			.depth_clamp_enable(false)
			.rasterizer_discard_enable(false)
			.polygon_mode(vk::PolygonMode::FILL)
			.cull_mode(vk::CullModeFlags::NONE)
			.front_face(vk::FrontFace::CLOCKWISE)
			.depth_bias_enable(false)
			.depth_bias_constant_factor(0.0)
			.depth_bias_clamp(0.0)
			.depth_bias_slope_factor(0.0)
			.line_width(1.0);

		let mut pipeline_create_infos = [vk::GraphicsPipelineCreateInfo::default()
			.push_next(&mut rendering_info)
			.layout(unsafe { pipeline_layout.vulkan_pipeline_layout.pipeline_layout })
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
			.color_blend_state(&color_blend_state)
			.depth_stencil_state(&depth_stencil_state)
			.dynamic_state(&dynamic_state)
			.input_assembly_state(&input_assembly_state)
			.vertex_input_state(&vertex_input_state)
			.rasterization_state(&rasterization_state)
			.viewport_state(&viewport_state)
			.multisample_state(&multisample_state)
			.stages(&stages)
			/* .build() */
		];

		pipeline_create_infos[0].p_viewport_state = &viewport_state;

		let pipeline = unsafe { self.device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None).expect("No pipeline") };

		crate::render_backend::Pipeline {
			vulkan_pipeline: Pipeline {
				pipeline: pipeline[0],
			},
		}
	}

	fn allocate_memory(&self, size: usize, device_accesses: crate::render_system::DeviceAccesses) -> crate::render_backend::Allocation {
		// get memory types
		let memory_properties = unsafe { self.instance.get_physical_device_memory_properties(self.physical_device) };

		let memory_type_index = memory_properties
			.memory_types
			.iter()
			.enumerate()
			.find_map(|(index, memory_type)| {
				let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

				memory_property_flags |= if device_accesses.contains(crate::render_system::DeviceAccesses::CpuRead) { vk::MemoryPropertyFlags::HOST_VISIBLE } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(crate::render_system::DeviceAccesses::CpuWrite) { vk::MemoryPropertyFlags::HOST_COHERENT } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(crate::render_system::DeviceAccesses::GpuRead) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(crate::render_system::DeviceAccesses::GpuWrite) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };

				let memory_type = memory_type.property_flags.contains(memory_property_flags);

				if memory_type {
					Some(index as u32)
				} else {
					None
				}
			})
			.expect("No memory type index found.");

		let mut memory_allocate_flags_info = vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);

		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(memory_type_index)
			.push_next(&mut memory_allocate_flags_info)
			/* .build() */;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		let mut mapped_memory = std::ptr::null_mut();

		if device_accesses.intersects(crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::CpuWrite) {
			mapped_memory = unsafe { self.device.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty()).expect("No mapped memory") as *mut u8 };
		}

		crate::render_backend::Allocation {
			vulkan_allocation: Allocation{
				memory,
			},
			pointer: mapped_memory,
		}
	}

	fn get_allocation_pointer(&self, allocation: &crate::render_backend::Allocation) -> *mut u8 {
		allocation.pointer
	}

	fn create_buffer(&self, size: usize, resource_uses: render_backend::Uses) -> crate::render_backend::MemoryBackedResourceCreationResult<crate::render_backend::Buffer> {
		let buffer_create_info = vk::BufferCreateInfo::default()
			.size(size as u64)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(
				if resource_uses.contains(render_backend::Uses::Vertex) { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_backend::Uses::Index) { vk::BufferUsageFlags::INDEX_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_backend::Uses::Uniform) { vk::BufferUsageFlags::UNIFORM_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS /*We allways use this feature so include constantly*/
			)
			/* .build() */;

		let buffer = unsafe { self.device.create_buffer(&buffer_create_info, None).expect("No buffer") };

		let memory_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

		crate::render_backend::MemoryBackedResourceCreationResult{
			resource: crate::render_backend::Buffer {
				vulkan_buffer: Buffer {
					buffer,
					device_address: 0,
				},
			},
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
		}
	}

	fn create_texture(&self, extent: crate::Extent, format: crate::render_backend::TextureFormats, resource_uses: render_backend::Uses, device_accesses: crate::render_system::DeviceAccesses, _access_policies: crate::render_backend::AccessPolicies, mip_levels: u32) -> crate::render_backend::MemoryBackedResourceCreationResult<crate::render_backend::Texture> {
		let image_type_from_extent = |extent: crate::Extent| {
			if extent.depth > 1 {
				vk::ImageType::TYPE_3D
			} else if extent.height > 1 {
				vk::ImageType::TYPE_2D
			} else {
				vk::ImageType::TYPE_1D
			}
		};

		let image_create_info = vk::ImageCreateInfo::default()
			.image_type(image_type_from_extent(extent))
			.format(to_format(format))
			.extent(vk::Extent3D{
				width: extent.width,
				height: extent.height,
				depth: 1,
			})
			.mip_levels(mip_levels)
			.array_layers(1)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(if !device_accesses.intersects(crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::CpuWrite) { vk::ImageTiling::OPTIMAL } else { vk::ImageTiling::LINEAR })
			.usage(
				if resource_uses.intersects(render_backend::Uses::Texture) { vk::ImageUsageFlags::SAMPLED } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_backend::Uses::RenderTarget) { vk::ImageUsageFlags::COLOR_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_backend::Uses::DepthStencil) { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_backend::Uses::TransferSource) { vk::ImageUsageFlags::TRANSFER_SRC } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_backend::Uses::TransferDestination) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
			)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED)
			/* .build() */;

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };

		let memory_requirements = unsafe { self.device.get_image_memory_requirements(image) };

		crate::render_backend::MemoryBackedResourceCreationResult {
			resource: crate::render_backend::Texture {
				vulkan_texture: Texture{
					image,
				},
			},
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
		}
	}

	fn create_sampler(&self) -> render_backend::Sampler {
		let sampler_create_info = vk::SamplerCreateInfo::default()
			.mag_filter(vk::Filter::NEAREST)
			.min_filter(vk::Filter::NEAREST)
			.mipmap_mode(vk::SamplerMipmapMode::NEAREST)
			.address_mode_u(vk::SamplerAddressMode::REPEAT)
			.address_mode_v(vk::SamplerAddressMode::REPEAT)
			.address_mode_w(vk::SamplerAddressMode::REPEAT)
			.anisotropy_enable(false)
			.max_anisotropy(1.0)
			.compare_enable(false)
			.compare_op(vk::CompareOp::ALWAYS)
			.min_lod(0.0)
			.max_lod(0.0)
			.mip_lod_bias(0.0)
			.unnormalized_coordinates(false)
			/* .build() */;

		let sampler = unsafe { self.device.create_sampler(&sampler_create_info, None).expect("No sampler") };

		render_backend::Sampler {
			vulkan_sampler: Sampler {
				sampler,
			},
		}
	}

	fn get_image_subresource_layout(&self, texture: &render_backend::Texture, mip_level: u32) -> render_backend::ImageSubresourceLayout {
		let image_subresource = vk::ImageSubresource {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			mip_level,
			array_layer: 0,
		};

		let image_subresource_layout = unsafe { self.device.get_image_subresource_layout(texture.vulkan_texture.image, image_subresource) };

		render_backend::ImageSubresourceLayout {
			offset: image_subresource_layout.offset,
			size: image_subresource_layout.size,
			row_pitch: image_subresource_layout.row_pitch,
			array_pitch: image_subresource_layout.array_pitch,
			depth_pitch: image_subresource_layout.depth_pitch,
		}
	}

	fn bind_buffer_memory(&self, memory: crate::render_backend::Memory, resource_creation_info: &crate::render_backend::MemoryBackedResourceCreationResult<crate::render_backend::Buffer>) {
		unsafe { self.device.bind_buffer_memory(resource_creation_info.resource.vulkan_buffer.buffer, memory.allocation.vulkan_allocation.memory, memory.offset as u64).expect("No buffer memory binding") };
	}

	fn bind_texture_memory(&self, memory: crate::render_backend::Memory, resource_creation_info: &crate::render_backend::MemoryBackedResourceCreationResult<crate::render_backend::Texture>) {
		unsafe { self.device.bind_image_memory(resource_creation_info.resource.vulkan_texture.image, memory.allocation.vulkan_allocation.memory, memory.offset as u64).expect("No image memory binding") };
		//let image_view = unsafe { self.device.create_image_view(&image_view_create_info, None).expect("No image view") };
		//resource_creation_info.resource.vulkan_texture.image_view = image_view;
	}

	fn create_synchronizer(&self, signaled: bool) -> crate::render_backend::Synchronizer {
		let fence_create_info = vk::FenceCreateInfo::default()
			.flags(vk::FenceCreateFlags::empty() | if signaled { vk::FenceCreateFlags::SIGNALED } else { vk::FenceCreateFlags::empty() })
			/* .build() */;

		let fence = unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") };

		let semaphore_create_info = vk::SemaphoreCreateInfo::default()
			/* .build() */;

		let semaphore = unsafe { self.device.create_semaphore(&semaphore_create_info, None).expect("No semaphore") };

		crate::render_backend::Synchronizer {
			vulkan_synchronizer: Synchronizer {
				fence,
				semaphore,
			},
		}
	}

	fn create_texture_view(&self, texture: &crate::render_backend::Texture, format: render_backend::TextureFormats, _mip_levels: u32) -> crate::render_backend::TextureView {
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(unsafe { texture.vulkan_texture.image })
			.view_type(
				vk::ImageViewType::TYPE_2D
			)
			.format(to_format(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: 0,
				layer_count: 1,
			})
			/* .build() */;

		let image_view = unsafe { self.device.create_image_view(&image_view_create_info, None).expect("No image view") };

		crate::render_backend::TextureView {
			vulkan_texture_view: TextureView {
				image_view,
			},
		}
	}

	fn get_surface_properties(&self, surface: &crate::render_backend::Surface) -> crate::render_backend::SurfaceProperties {
		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface.vulkan_surface.surface).expect("No surface capabilities") };

		crate::render_backend::SurfaceProperties {
			extent: crate::Extent {
				width: surface_capabilities.current_extent.width,
				height: surface_capabilities.current_extent.height,
				depth: 1,
			},
		}
	}

	fn create_surface(&self, window_os_handles: crate::window_system::WindowOsHandles) -> crate::render_backend::Surface {
		let xcb_surface_create_info = vk::XcbSurfaceCreateInfoKHR::default()
			.connection(window_os_handles.xcb_connection)
			.window(window_os_handles.xcb_window);

		let xcb_surface = ash::extensions::khr::XcbSurface::new(&self.entry, &self.instance);

		let surface = unsafe { xcb_surface.create_xcb_surface(&xcb_surface_create_info, None).expect("No surface") };

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let surface_format = unsafe { self.surface.get_physical_device_surface_formats(self.physical_device, surface).expect("No surface formats") };

		let surface_format: vk::SurfaceFormatKHR = surface_format
			.iter()
			.find(|format| {
				format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
			})
			.expect("No surface format").to_owned();

		let surface_present_modes = unsafe { self.surface.get_physical_device_surface_present_modes(self.physical_device, surface).expect("No surface present modes") };

		let surface_present_mode: vk::PresentModeKHR = surface_present_modes
			.iter()
			.find(|present_mode| {
				**present_mode == vk::PresentModeKHR::FIFO
			})
			.expect("No surface present mode").to_owned();

		let _surface_resolution = surface_capabilities.current_extent;

		crate::render_backend::Surface {
			vulkan_surface: Surface {
				surface,
				surface_capabilities,
				surface_format,
				surface_present_mode,
			},
		}
	}

	fn create_swapchain(&self, surface: &crate::render_backend::Surface, _extent: crate::Extent, buffer_count: u32) -> crate::render_backend::Swapchain {
		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface.vulkan_surface.surface).expect("No surface capabilities") };

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.surface(unsafe { surface.vulkan_surface.surface })
			.min_image_count(buffer_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(surface_capabilities.current_extent)
			.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(surface_capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(unsafe { surface.vulkan_surface.surface_present_mode })
			.image_array_layers(1)
			.clipped(true);

		let swapchain_loader = ash::extensions::khr::Swapchain::new(&self.instance, &self.device);

		let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None).expect("No swapchain") };

		crate::render_backend::Swapchain {
			vulkan_swapchain: Swapchain {
				swapchain,
			},
		}
	}

	fn get_swapchain_images(&self, swapchain: &crate::render_backend::Swapchain) -> Vec<crate::render_backend::Texture> {
		let swapchain_images = unsafe { self.swapchain.get_swapchain_images(unsafe { swapchain.vulkan_swapchain.swapchain }).expect("No swapchain images") };

		swapchain_images.iter().map(|a| crate::render_backend::Texture {
			vulkan_texture: Texture {
				image: *a,
			},
		}).collect::<Vec<_>>()
	}

	fn begin_command_buffer_recording(&self, command_buffer: &crate::render_backend::CommandBuffer) {
		let command_buffer_begin_info = vk::CommandBufferBeginInfo::default()
			.flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
			/* .build() */;

		unsafe { self.device.begin_command_buffer(command_buffer.vulkan_command_buffer.command_buffer, &command_buffer_begin_info).expect("No command buffer begin") };
	}

	fn end_command_buffer_recording(&self, command_buffer: &crate::render_backend::CommandBuffer) {
		unsafe { self.device.end_command_buffer(command_buffer.vulkan_command_buffer.command_buffer).expect("No command buffer end") };
	}

	fn start_render_pass(&self, command_buffer: &crate::render_backend::CommandBuffer, extent: crate::Extent, attachments: &[crate::render_backend::AttachmentInformation]) {
		let render_area = vk::Rect2D::default()
			.offset(vk::Offset2D::default().x(0).y(0)/* .build() */)
			.extent(vk::Extent2D::default().width(extent.width).height(extent.height)/* .build() */)
			/* .build() */;

		let color_attchments = attachments.iter().map(|attachment| {
			vk::RenderingAttachmentInfo::default()
				.image_view(unsafe { attachment.texture_view.vulkan_texture_view.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.resource_use, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear.expect("No clear value")))
				/* .build() */
		}).collect::<Vec<_>>();

		let depth_attachment = attachments.iter().find(|attachment| attachment.format == render_backend::TextureFormats::Depth32).map(|attachment| {
			vk::RenderingAttachmentInfo::default()
				.image_view(unsafe { attachment.texture_view.vulkan_texture_view.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.resource_use, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear.expect("No clear value")))
				/* .build() */
		}).or(Some(vk::RenderingAttachmentInfo::default())).unwrap();

		let rendering_info = vk::RenderingInfoKHR::default()
			.color_attachments(color_attchments.as_slice())
			.depth_attachment(&depth_attachment)
			.render_area(render_area)
			.layer_count(1)
			/* .build() */;

		let viewports = [
			vk::Viewport {
				x: 0.0,
				y: extent.height as f32,
				width: extent.width as f32,
				height: -(extent.height as f32),
				min_depth: 0.0,
				max_depth: 1.0,
			}
		];

		unsafe { self.dynamic_rendering.cmd_begin_rendering(command_buffer.vulkan_command_buffer.command_buffer, &rendering_info); }
		unsafe { self.device.cmd_set_viewport(command_buffer.vulkan_command_buffer.command_buffer, 0, &viewports); }
		unsafe { self.device.cmd_set_scissor(command_buffer.vulkan_command_buffer.command_buffer, 0, &[render_area]); }
	}

	fn end_render_pass(&self, command_buffer: &crate::render_backend::CommandBuffer) {
		unsafe { self.dynamic_rendering.cmd_end_rendering(command_buffer.vulkan_command_buffer.command_buffer); }
	}

	fn bind_shader(&self, _command_buffer: &crate::render_backend::CommandBuffer, _shader: &crate::render_backend::Shader) {
		panic!("Not implemented")
	}

	fn bind_pipeline(&self, command_buffer: &crate::render_backend::CommandBuffer, pipeline: &crate::render_backend::Pipeline) {
		unsafe { self.device.cmd_bind_pipeline(command_buffer.vulkan_command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline.vulkan_pipeline.pipeline); }
	}

	fn write_to_push_constant(&self, command_buffer: &render_backend::CommandBuffer, pipeline_layout: &render_backend::PipelineLayout, offset: u32, data: &[u8]) {
		unsafe { self.device.cmd_push_constants(command_buffer.vulkan_command_buffer.command_buffer, pipeline_layout.vulkan_pipeline_layout.pipeline_layout, vk::ShaderStageFlags::VERTEX, offset, data); }
	}

	fn bind_descriptor_set(&self, command_buffer: &render_backend::CommandBuffer, pipeline_layout: &render_backend::PipelineLayout, descriptor_set: &render_backend::DescriptorSet, index: u32) {
		unsafe { self.device.cmd_bind_descriptor_sets(command_buffer.vulkan_command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline_layout.vulkan_pipeline_layout.pipeline_layout, index, &[descriptor_set.vulkan_descriptor_set.descriptor_set], &[]); }
	}

	fn bind_vertex_buffer(&self, command_buffer: &crate::render_backend::CommandBuffer, buffer: &crate::render_backend::Buffer) {
		unsafe { self.device.cmd_bind_vertex_buffers(command_buffer.vulkan_command_buffer.command_buffer, 0, &[buffer.vulkan_buffer.buffer], &[0 as u64]); }
	}

	fn bind_index_buffer(&self, command_buffer: &crate::render_backend::CommandBuffer, buffer: &crate::render_backend::Buffer) {
		unsafe { self.device.cmd_bind_index_buffer(command_buffer.vulkan_command_buffer.command_buffer, buffer.vulkan_buffer.buffer, 0, vk::IndexType::UINT32); }
	}

	fn draw_indexed(&self, command_buffer: &crate::render_backend::CommandBuffer, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
		unsafe { self.device.cmd_draw_indexed(command_buffer.vulkan_command_buffer.command_buffer, index_count, instance_count, first_index, vertex_offset, first_instance); }
	}

	fn execute_barriers(&self, command_buffer: &crate::render_backend::CommandBuffer, barriers: &[crate::render_backend::BarrierDescriptor]) {
		let mut image_memory_barriers = Vec::new();

		for barrier in barriers {
			match barrier.barrier {
				// crate::render_backend::Barrier::BufferBarrier(buffer_barrier) => {
				// 	let buffer_memory_barrier = vk::BufferMemoryBarrier2KHR::default()
				// 		.src_stage_mask(to_pipeline_stage_flags(buffer_barrier.source_stage))
				// 		.dst_stage_mask(to_pipeline_stage_flags(buffer_barrier.destination_stage))
				// 		.src_access_mask(to_access_flags(buffer_barrier.source_access))
				// 		.dst_access_mask(to_access_flags(buffer_barrier.destination_access))
				// 		.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				// 		.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				// 		.buffer(unsafe { buffer_barrier.buffer.vulkan_buffer.buffer })
				// 		.offset(0)
				// 		.size(vk::WHOLE_SIZE)
				// 		/* .build() */;

				// 	unsafe { self.device.cmd_pipeline_barrier2(command_buffer.vulkan_command_buffer.command_buffer, &vk::DependencyInfo::default().buffer_memory_barriers(&[buffer_memory_barrier])) };
				// },
				crate::render_backend::Barrier::Texture(texture_barrier) => {
					let image_memory_barrier = if let Some(source) = barrier.source {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(texture_format_and_resource_use_to_image_layout(crate::render_backend::TextureFormats::RGBAu8, source.layout, Some(source.access)))
							.src_stage_mask(to_pipeline_stage_flags(source.stage))
							.src_access_mask(to_access_flags(source.access, source.stage))
						} else {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(vk::ImageLayout::UNDEFINED)
							.src_stage_mask(vk::PipelineStageFlags2::empty())
							.src_access_mask(vk::AccessFlags2KHR::empty())
						}
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(texture_format_and_resource_use_to_image_layout(crate::render_backend::TextureFormats::RGBAu8, barrier.destination.layout, Some(barrier.destination.access)))
						.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
						.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(unsafe { texture_barrier.vulkan_texture.image })
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: vk::ImageAspectFlags::COLOR,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
						/* .build() */;

					image_memory_barriers.push(image_memory_barrier);
				},
			}
		}

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		unsafe { self.device.cmd_pipeline_barrier2(command_buffer.vulkan_command_buffer.command_buffer, &dependency_info) };
	}

	fn copy_textures(&self, command_buffer: &crate::render_backend::CommandBuffer, copies: &[crate::render_backend::TextureCopy]) {
		let regions = copies.iter().map(|copy| {
			vk::ImageCopy2::default()
				.src_subresource(vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1)
					/* .build() */
				)
				.src_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
				.dst_subresource(vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1)
					/* .build() */
				)
				.dst_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
				.extent(vk::Extent3D::default().width(copy.extent.width).height(copy.extent.height).depth(1)/* .build() */)
				/* .build() */
		}).collect::<Vec<_>>();

		for copy in copies {
			let copy_image_info = vk::CopyImageInfo2::default()
				.src_image(unsafe { copy.source.vulkan_texture.image })
				.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
				.dst_image(unsafe { copy.destination.vulkan_texture.image })
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions)
				/* .build() */;

			unsafe { self.device.cmd_copy_image2(command_buffer.vulkan_command_buffer.command_buffer, &copy_image_info); }
		}
	}

	fn execute(&self, command_buffer: &crate::render_backend::CommandBuffer, wait_for: Option<&crate::render_backend::Synchronizer>, signal: Option<&crate::render_backend::Synchronizer>, execution_completion: &crate::render_backend::Synchronizer) {
		let command_buffers = [unsafe { command_buffer.vulkan_command_buffer.command_buffer }];

		let command_buffer_infos = [
			vk::CommandBufferSubmitInfo::default()
				.command_buffer(
					command_buffers[0]
				)
				/* .build() */
		];

		let wait_semaphores = if let Some(wait_for) = wait_for {
			vec![
				vk::SemaphoreSubmitInfo::default()
					.semaphore(unsafe { wait_for.vulkan_synchronizer.semaphore })
					.stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
					/* .build() */
			]
		} else {
			vec![]
		};

		let signal_semaphores = if let Some(signal) = signal {
			vec![
				vk::SemaphoreSubmitInfo::default()
					.semaphore(unsafe { signal.vulkan_synchronizer.semaphore })
				/* .build() */
			]
		} else {
			vec![]
		};

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores)
			/* .build() */;

		unsafe { self.device.queue_submit2(self.queue, &[submit_info], execution_completion.vulkan_synchronizer.fence); }
	}

	fn acquire_swapchain_image(&self, swapchain: &crate::render_backend::Swapchain, image_available: &crate::render_backend::Synchronizer) -> (u32, render_backend::SwapchainStates) {
		let acquisition_result = unsafe { self.swapchain.acquire_next_image(swapchain.vulkan_swapchain.swapchain, u64::MAX, image_available.vulkan_synchronizer.semaphore, vk::Fence::null()) };

		if let Ok((index, state)) = acquisition_result {
			if !state {
				(index, render_backend::SwapchainStates::Ok)
			} else {
				(index, render_backend::SwapchainStates::Suboptimal)
			}
		} else {
			(0, render_backend::SwapchainStates::Invalid)
		}
	}

	fn present(&self, swapchain: &crate::render_backend::Swapchain, wait_for: &crate::render_backend::Synchronizer, image_index: u32) {
		let swapchains = [unsafe { swapchain.vulkan_swapchain.swapchain }];
		let wait_semaphores = [unsafe { wait_for.vulkan_synchronizer.semaphore }];
		let image_indices = [image_index];

  		let present_info = vk::PresentInfoKHR::default()
			.swapchains(&swapchains)
			.wait_semaphores(&wait_semaphores)
			.image_indices(&image_indices);

		unsafe { self.swapchain.queue_present(self.queue, &present_info); }
	}

	fn wait(&self, synchronizer: &crate::render_backend::Synchronizer) {
		unsafe { self.device.wait_for_fences(&[synchronizer.vulkan_synchronizer.fence], true, u64::MAX).expect("No fence wait"); }
		unsafe { self.device.reset_fences(&[synchronizer.vulkan_synchronizer.fence]).expect("No fence reset"); }
	}
}