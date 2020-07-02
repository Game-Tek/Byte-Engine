#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>


#include "RenderStaticMeshCollection.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Game/ComponentCollection.h"

class RenderStaticMeshCollection;

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	GAL::RenderDevice::CreateInfo createinfo;
	createinfo.ApplicationName = GTSL::StaticString<128>("Test");
	GTSL::Array<GAL::Queue::CreateInfo, 1> queue_create_infos(1);
	queue_create_infos[0].Capabilities = static_cast<uint8>(QueueCapabilities::GRAPHICS);
	queue_create_infos[0].QueuePriority = 1.0f;
	createinfo.QueueCreateInfos = queue_create_infos;
	createinfo.Queues = GTSL::Ranger<Queue>(1, &graphicsQueue);
	::new(&renderDevice) RenderDevice(createinfo);

	GAL::RenderContext::CreateInfo render_context_create_info;
	render_context_create_info.RenderDevice = &renderDevice;
	render_context_create_info.DesiredFramesInFlight = 2;
	render_context_create_info.PresentMode = static_cast<uint32>(PresentMode::FIFO);
	render_context_create_info.Format = (uint32)ImageFormat::BGRA_I8;
	render_context_create_info.ColorSpace = (uint32)ColorSpace::NONLINEAR_SRGB;
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
	attachment_descriptors.PushBack(GAL::AttachmentDescriptor{ (uint32)ImageFormat::BGRA_I8, GAL::RenderTargetLoadOperations::UNDEFINED, GAL::RenderTargetStoreOperations::STORE, GAL::ImageLayout::COLOR_ATTACHMENT, GAL::ImageLayout::PRESENTATION });
	render_pass_create_info.Descriptor.RenderPassColorAttachments = attachment_descriptors;
	GTSL::Array<GAL::AttachmentReference, 8> attachment_references;
	attachment_references.PushBack(GAL::AttachmentReference{ 0, GAL::ImageLayout::COLOR_ATTACHMENT });
	GTSL::Array<GAL::SubPassDescriptor, 8> sub_pass_descriptors;
	sub_pass_descriptors.PushBack(GAL::SubPassDescriptor{ GTSL::Ranger<GAL::AttachmentReference>(), attachment_references, GTSL::Ranger<uint8>(), nullptr });
	render_pass_create_info.Descriptor.SubPasses = sub_pass_descriptors;

	::new(&renderPass) RenderPass(render_pass_create_info);
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	get_images_info.SwapchainImagesFormat = (uint32)ImageFormat::BGRA_I8;
	swapchainImages = renderContext.GetImages(get_images_info);

	GraphicsPipeline::CreateInfo pipeline_create_info{};
	pipeline_create_info.RenderDevice = &renderDevice;
	pipeline_create_info.RenderPass = &renderPass;
	pipeline_create_info.BindingsPools; //safe
	pipeline_create_info.PipelineDescriptor.BlendEnable = false;
	pipeline_create_info.PipelineDescriptor.ColorBlendOperation = GAL::BlendOperation::ADD;
	pipeline_create_info.PipelineDescriptor.CullMode = GAL::CullMode::CULL_BACK;
	pipeline_create_info.PipelineDescriptor.DepthCompareOperation = GAL::CompareOperation::LESS;
	pipeline_create_info.PipelineDescriptor.RasterizationSamples = GAL::SampleCount::SAMPLE_COUNT_1;

	GTSL::String vertex_errors;
	GTSL::String fragment_errors;
	
	GTSL::StaticString<1024> vertex_shader_code{"#version 450\nlayout(location = 0) in vec3 inPos;layout(location = 1) in vec3 inNorm;layout(location = 2) in vec2 inTextPos;layout(location = 3) in vec3 inTan;layout(location = 4) in vec3 inBiTan;void main() { gl_Position = vec4(inPos, 1.0) * 0.1; }\0"};
	
	GTSL::StaticString<1024> fragment_shader_code{"#version 450\nlayout(location = 0) out vec4 outColor; void main() { outColor = vec4(1.0); }\0" };
	GTSL::Vector<byte> vertex_shader_bytecode;
	BE_ASSERT(Shader::CompileShader(vertex_shader_code, GTSL::StaticString<8>("Test\0"), GAL::ShaderType::VERTEX_SHADER, GAL::ShaderLanguage::GLSL, vertex_shader_bytecode, vertex_errors, GetTransientAllocator()), "WW");

	GTSL::Vector<byte> fragment_shader_bytecode;
	Shader::CompileShader(fragment_shader_code, GTSL::StaticString<8>("Test\0"), GAL::ShaderType::FRAGMENT_SHADER, GAL::ShaderLanguage::GLSL, fragment_shader_bytecode, fragment_errors, GetTransientAllocator());

	Shader vertex_shader(Shader::CreateInfo{ &renderDevice, vertex_shader_bytecode });
	Shader fragment_shader(Shader::CreateInfo{ &renderDevice, fragment_shader_bytecode });
	
	GTSL::Array<GraphicsPipeline::ShaderInfo, 2> shaders;
	shaders.PushBack({ GAL::ShaderType::VERTEX_SHADER, &vertex_shader });
	shaders.PushBack({ GAL::ShaderType::FRAGMENT_SHADER, &fragment_shader });
	
	pipeline_create_info.PipelineDescriptor.Stages = shaders;
	pipeline_create_info.SurfaceExtent = window_extent;
	
	GTSL::Array<GAL::ShaderDataTypes, 16> vertex{ GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3 , GAL::ShaderDataTypes::FLOAT2, GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3 };
	
	pipeline_create_info.VertexDescriptor = vertex;
	::new(&graphicsPipeline) GraphicsPipeline(pipeline_create_info);

	vertex_errors.Free(GetTransientAllocator());
	fragment_errors.Free(GetTransientAllocator());
	vertex_shader_bytecode.Free(GetTransientAllocator());
	fragment_shader_bytecode.Free(GetTransientAllocator());

	vertex_shader.Destroy(&renderDevice);
	fragment_shader.Destroy(&renderDevice);

	clearValues.EmplaceBack(0, 0, 0, 0);

	for (uint8 i = 0; i < 2; ++i)
	{
		imagesAvailable.EmplaceBack(Semaphore::CreateInfo{ &renderDevice });
		rendersFinished.EmplaceBack(Semaphore::CreateInfo{ &renderDevice });
		inFlightFences.EmplaceBack(Fence::CreateInfo{ &renderDevice });
		imagesInFlight.EmplaceBack(Fence::CreateInfo{ &renderDevice });
		
		commandBuffers.EmplaceBack(CommandBuffer::CreateInfo{ &renderDevice, true, &graphicsQueue });
		
		FrameBuffer::CreateInfo framebuffer_create_info;
		framebuffer_create_info.RenderDevice = &renderDevice;
		framebuffer_create_info.RenderPass = &renderPass;
		framebuffer_create_info.Extent = window_extent;
		framebuffer_create_info.ImageViews = GTSL::Ranger<const ImageView>(1, &swapchainImages[i]);
		framebuffer_create_info.ClearValues = clearValues;

		frameBuffers.EmplaceBack(framebuffer_create_info);
	}

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.BufferType = static_cast<uint32>(BufferType::VERTEX) | static_cast<uint32>(BufferType::INDEX);
	buffer_create_info.Size = 4 * 1024 * 1024;
	::new(&stagingMesh) Buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&stagingMesh, buffer_memory_requirements);

	auto memory_type = renderDevice.FindMemoryType(buffer_memory_requirements.MemoryTypes, (uint32)MemoryType::SHARED | (uint32)MemoryType::COHERENT);

	DeviceMemory::CreateInfo scratch_memory_create_info;
	scratch_memory_create_info.RenderDevice = &renderDevice;
	scratch_memory_create_info.Size = buffer_create_info.Size;
	scratch_memory_create_info.MemoryType = memory_type;
	::new(&mappedDeviceMemory) DeviceMemory(scratch_memory_create_info);
	mappedMemoryPointer = mappedDeviceMemory.Map(DeviceMemory::MapInfo{ &renderDevice, buffer_create_info.Size, 0 });

	stagingMesh.BindToMemory(Buffer::BindMemoryInfo{ &renderDevice, &mappedDeviceMemory, 0 });
}

void RenderSystem::AddStaticMeshes(uint32 start, uint32 end)
{
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_mesh_info;
	load_static_mesh_info.OnStaticMeshLoad = GTSL::Delegate<void(StaticMeshResourceManager::OnStaticMeshLoad)>::Create<RenderSystem, &RenderSystem::staticMeshLoaded>(this);
	load_static_mesh_info.MeshDataBuffer = GTSL::Ranger<byte>(8192, static_cast<byte*>(mappedMemoryPointer));
	load_static_mesh_info.Name = reinterpret_cast<RenderStaticMeshCollection*>(BE::Application::Get()->GetGameInstance()->GetComponentCollection("StaticMeshCollection"))->ResourceNames[start];
	static_cast<StaticMeshResourceManager*>(BE::Application::Get()->GetResourceManager()->GetSubResourceManager("StaticMeshResourceManager"))->LoadStaticMesh(load_static_mesh_info);
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	GTSL::Array<GTSL::Id64, 8> actsOn{ "RenderSystem" };
	//initializeInfo.GameInstance->AddTask(__FUNCTION__, GameInstance::AccessType::READ,
//		GTSL::Delegate<void(const GameInstance::TaskInfo&)>::Create<RenderSystem, &RenderSystem::render>(this), actsOn, "Frame");
}

void RenderSystem::Shutdown()
{
	stagingMesh.Destroy(&renderDevice);
	deviceMesh.Destroy(&renderDevice);
	
	mappedDeviceMemory.Unmap(DeviceMemory::UnmapInfo{ &renderDevice });
	mappedDeviceMemory.Destroy(&renderDevice);
	deviceMemory.Destroy(&renderDevice);

	graphicsPipeline.Destroy(&renderDevice);
	
	renderPass.Destroy(&renderDevice);
	renderContext.Destroy(&renderDevice);

	for(auto& e : imagesAvailable) { e.Destroy(&renderDevice); }
	for(auto& e : rendersFinished) { e.Destroy(&renderDevice); }
	for(auto& e : inFlightFences) { e.Destroy(&renderDevice); }
	for(auto& e : imagesInFlight) { e.Destroy(&renderDevice); }
	
	for(auto& e : commandBuffers) { e.Destroy(&renderDevice); }
	for (auto& e : frameBuffers) { e.Destroy(&renderDevice); }

	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }
}

void RenderSystem::render(const GameInstance::TaskInfo& taskInfo)
{
	Fence::WaitForFencesInfo wait_for_fences_info;
	wait_for_fences_info.RenderDevice = &renderDevice;
	wait_for_fences_info.Timeout = ~0ULL;
	wait_for_fences_info.WaitForAll = true;
	wait_for_fences_info.Fences = GTSL::Ranger<const Fence>(1, &inFlightFences[renderContext.GetCurrentImage()]);
	Fence::WaitForFences(wait_for_fences_info);
	
	RenderContext::AcquireNextImageInfo acquire_next_image_info;
	acquire_next_image_info.RenderDevice = &renderDevice;
	acquire_next_image_info.Semaphore = &imagesAvailable[renderContext.GetCurrentImage()];
	renderContext.AcquireNextImage(acquire_next_image_info);

	Fence::ResetFencesInfo reset_fences_info;
	reset_fences_info.RenderDevice = &renderDevice;
	reset_fences_info.Fences = GTSL::Ranger<const Fence>(1, &inFlightFences[renderContext.GetCurrentImage()]);
	Fence::ResetFences(reset_fences_info);

	Queue::SubmitInfo submit_info;
	submit_info.RenderDevice = &renderDevice;
	submit_info.Fence = &inFlightFences[renderContext.GetCurrentImage()];
	submit_info.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &imagesAvailable[renderContext.GetCurrentImage()]);
	submit_info.SignalSemaphores = GTSL::Ranger<const Semaphore>(1, &rendersFinished[renderContext.GetCurrentImage()]);
	submit_info.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, &commandBuffers[renderContext.GetCurrentImage()]);
	graphicsQueue.Submit(submit_info);
	
	RenderContext::PresentInfo present_info;
	present_info.RenderDevice = &renderDevice;
	present_info.Queue = &graphicsQueue;
	present_info.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &rendersFinished[renderContext.GetCurrentImage()]);
	renderContext.Present(present_info);
}

void RenderSystem::staticMeshLoaded(StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = &renderDevice;
	buffer_create_info.BufferType = static_cast<uint32>(BufferType::VERTEX) | static_cast<uint32>(BufferType::INDEX);
	buffer_create_info.Size = onStaticMeshLoad.MeshDataBuffer.Bytes();
	::new(&deviceMesh) Buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	renderDevice.GetBufferMemoryRequirements(&stagingMesh, buffer_memory_requirements);
	auto memory_type = renderDevice.FindMemoryType(buffer_memory_requirements.MemoryTypes, (uint32)MemoryType::GPU);
	
	DeviceMemory::CreateInfo memory_create_info;
	memory_create_info.RenderDevice = &renderDevice;
	memory_create_info.Size = onStaticMeshLoad.MeshDataBuffer.Bytes();
	memory_create_info.MemoryType = memory_type;
	::new(&deviceMemory) DeviceMemory(memory_create_info);

	deviceMesh.BindToMemory(Buffer::BindMemoryInfo{ &renderDevice, &deviceMemory, 0 });
	
	commandBuffers[renderContext.GetCurrentImage()].CopyBuffers({&renderDevice, &stagingMesh, 0, &deviceMesh, 0, static_cast<uint32>(onStaticMeshLoad.MeshDataBuffer.Bytes())});
}
