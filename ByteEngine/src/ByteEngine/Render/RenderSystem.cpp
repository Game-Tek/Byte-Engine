#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

static BE::TransientAllocatorReference tAllocator("Renderer");

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	GAL::RenderDevice::CreateInfo createinfo;
	createinfo.ApplicationName = GTSL::StaticString<128>("Test");
	GTSL::Array<GAL::Queue::CreateInfo, 1> queue_create_infos(1);
	queue_create_infos[0].Capabilities = static_cast<uint8>(GAL::QueueCapabilities::GRAPHICS);
	queue_create_infos[0].QueuePriority = 1.0f;
	createinfo.QueueCreateInfos = queue_create_infos;
	GTSL::Array<GAL::Queue*, 1> queues;
	queues.EmplaceBack(&graphicsQueue);
	createinfo.Queues = queues;
	::new(&renderDevice) RenderDevice(createinfo);

	GAL::RenderContext::CreateInfo render_context_create_info;
	render_context_create_info.RenderDevice = &renderDevice;
	render_context_create_info.DesiredFramesInFlight = 2;
	render_context_create_info.PresentMode = GAL::PresentMode::FIFO;
	GTSL::Extent2D window_extent;
	initializeRenderer.Window->GetFramebufferExtent(window_extent);
	render_context_create_info.SurfaceArea = window_extent;
	GAL::WindowsWindowData window_data;
	GTSL::Window::Win32NativeHandles native_handles;
	initializeRenderer.Window->GetNativeHandles(&native_handles);
	window_data.WindowHandle = native_handles.HWND;
	window_data.InstanceHandle = GetModuleHandleA(nullptr);
	render_context_create_info.SystemData = &window_data;
	::new(&renderContext) RenderContext(render_context_create_info);

	RenderPass::CreateInfo render_pass_create_info;
	render_pass_create_info.RenderDevice = &renderDevice;
	render_pass_create_info.Descriptor.DepthStencilAttachmentAvailable = false;
	GTSL::Array<GAL::AttachmentDescriptor, 8> attachment_descriptors;
	attachment_descriptors.PushBack(GAL::AttachmentDescriptor{ GAL::ImageFormat::RGB_I8, GAL::RenderTargetLoadOperations::UNDEFINED, GAL::RenderTargetStoreOperations::STORE, GAL::ImageLayout::UNDEFINED, GAL::ImageLayout::COLOR_ATTACHMENT });
	render_pass_create_info.Descriptor.RenderPassColorAttachments = attachment_descriptors;
	GTSL::Array<GAL::AttachmentReference, 8> attachment_references;
	attachment_references.PushBack(GAL::AttachmentReference{ 0, GAL::ImageLayout::COLOR_ATTACHMENT });
	GTSL::Array<GAL::SubPassDescriptor, 8> sub_pass_descriptors;
	sub_pass_descriptors.PushBack(GAL::SubPassDescriptor{ GTSL::Ranger<GAL::AttachmentReference>(), attachment_references, GTSL::Ranger<uint8>(), nullptr });
	render_pass_create_info.Descriptor.SubPasses = sub_pass_descriptors;

	::new(&renderPass) RenderPass(render_pass_create_info);
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	swapchainImages = renderContext.GetImages(get_images_info);

	GraphicsPipeline::CreateInfo pipeline_create_info{};
	pipeline_create_info.RenderDevice = &renderDevice;
	pipeline_create_info.RenderPass = &renderPass;
	pipeline_create_info.BindingsSets; //safe
	pipeline_create_info.PipelineDescriptor.BlendEnable = false;
	pipeline_create_info.PipelineDescriptor.ColorBlendOperation = GAL::BlendOperation::ADD;
	pipeline_create_info.PipelineDescriptor.CullMode = GAL::CullMode::CULL_BACK;
	pipeline_create_info.PipelineDescriptor.DepthCompareOperation = GAL::CompareOperation::LESS;
	pipeline_create_info.PipelineDescriptor.RasterizationSamples = GAL::SampleCount::SAMPLE_COUNT_1;

	GTSL::String vertex_errors;
	GTSL::String fragment_errors;
	
	GTSL::StaticString<1024> vertex_shader{"#version 450\nlayout(location = 0) in vec3 inPos;layout(location = 1) in vec3 inNorm;layout(location = 2) in vec2 inTextPos;layout(location = 3) in vec3 inTan;layout(location = 4) in vec3 inBiTan;void main() { gl_Position = vec4(inPos, 1.0) * 0.1; }\0"};
	
	GTSL::StaticString<1024> fragment_shader{"#version 450\nlayout(location = 0) out vec4 outColor; void main() { outColor = vec4(1.0); }\0" };
	GTSL::Vector<byte> vertex_shader_bytecode;
	BE_ASSERT(GAL::VulkanShaders::CompileShader(vertex_shader, GTSL::StaticString<8>("Test\0"), GAL::ShaderType::VERTEX_SHADER, GAL::ShaderLanguage::GLSL, vertex_shader_bytecode, vertex_errors, GetTransientAllocator()), "WW");

	GTSL::Vector<byte> fragment_shader_bytecode;
	GAL::VulkanShaders::CompileShader(fragment_shader, GTSL::StaticString<8>("Test\0"), GAL::ShaderType::FRAGMENT_SHADER, GAL::ShaderLanguage::GLSL, fragment_shader_bytecode, fragment_errors, GetTransientAllocator());
	
	GTSL::Array<GAL::ShaderInfo, 2> shaders;
	shaders.PushBack({ GAL::ShaderType::VERTEX_SHADER, GTSL::StaticString<8>("Test\0"), vertex_shader_bytecode });
	shaders.PushBack({ GAL::ShaderType::FRAGMENT_SHADER, GTSL::StaticString<8>("Test\0"), fragment_shader_bytecode });
	
	pipeline_create_info.PipelineDescriptor.Stages = shaders;
	pipeline_create_info.SurfaceExtent = window_extent;
	
	GTSL::Array<GAL::ShaderDataTypes, 16> vertex{ GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3 , GAL::ShaderDataTypes::FLOAT2, GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3 };
	
	pipeline_create_info.VertexDescriptor = vertex;
	::new(&graphicsPipeline) GraphicsPipeline(pipeline_create_info);

	vertex_errors.Free(GetTransientAllocator());
	fragment_errors.Free(GetTransientAllocator());
	vertex_shader_bytecode.Free(GetTransientAllocator());
	fragment_shader_bytecode.Free(GetTransientAllocator());
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	GTSL::Array<GTSL::Id64, 8> actsOn{ "RenderSystem" };
	initializeInfo.GameInstance->AddTask("Test", GameInstance::AccessType::READ, GTSL::Delegate<void(const GameInstance::TaskInfo&)>::Create<RenderSystem, &RenderSystem::test>(this), actsOn, "Frame");
}

void RenderSystem::Shutdown()
{
	renderContext.Destroy(&renderDevice);
}

void RenderSystem::test(const GameInstance::TaskInfo& taskInfo)
{
	//BE_LOG_SUCCESS("Test task was fired!")
	RenderContext::AcquireNextImageInfo acquire_next_image_info;
	acquire_next_image_info.RenderDevice = &renderDevice;
	acquire_next_image_info.Semaphore = &imagesAvailable[renderContext.GetCurrentImage()];
	acquire_next_image_info.Fence = &renderFinished[renderContext.GetCurrentImage()];
	renderContext.AcquireNextImage(acquire_next_image_info);

	RenderContext::PresentInfo present_info;
	present_info.RenderDevice = &renderDevice;
	present_info.Queue = &graphicsQueue;
	GAL::Semaphore* wait_semaphore = &imagesAvailable[renderContext.GetCurrentImage()];
	present_info.WaitSemaphores = GTSL::Ranger<GAL::Semaphore>(1, wait_semaphore);
	renderContext.Present(present_info);
}
