#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ComponentCollection.h"
#include "ByteEngine/Debug/Assert.h"

class RenderStaticMeshCollection;

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	::new(&renderGroups) decltype(renderGroups)(16, GetPersistentAllocator());
	
	GAL::RenderDevice::CreateInfo createinfo;
	createinfo.ApplicationName = GTSL::StaticString<128>("Test");
	GTSL::Array<GAL::Queue::CreateInfo, 1> queue_create_infos(1);
	queue_create_infos[0].Capabilities = static_cast<uint8>(QueueCapabilities::GRAPHICS);
	queue_create_infos[0].QueuePriority = 1.0f;
	createinfo.QueueCreateInfos = queue_create_infos;
	createinfo.Queues = GTSL::Ranger<Queue>(1, &graphicsQueue);
	createinfo.DebugPrintFunction = GTSL::Delegate<void(const char*, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
	::new(&renderDevice) RenderDevice(createinfo);

	swapchainPresentMode = static_cast<uint32>(PresentMode::FIFO);
	swapchainColorSpace = static_cast<uint32>(ColorSpace::NONLINEAR_SRGB);
	swapchainFormat = static_cast<uint32>(ImageFormat::BGRA_I8);
	
	Surface::CreateInfo surface_create_info;
	surface_create_info.RenderDevice = &renderDevice;

	GAL::WindowsWindowData window_data;
	GTSL::Window::Win32NativeHandles native_handles;
	initializeRenderer.Window->GetNativeHandles(&native_handles);
	window_data.WindowHandle = native_handles.HWND;
	window_data.InstanceHandle = GetModuleHandleA(nullptr);

	surface_create_info.SystemData = &window_data;
	::new(&surface) Surface(surface_create_info);

	BE_ASSERT(surface.IsSupported(&renderDevice) != false, "Surface is not supported!");

	GTSL::Array<GTSL::uint32, 4> present_modes{ swapchainPresentMode };
	auto res = surface.GetSupportedPresentMode(&renderDevice, present_modes);
	if (res != 0xFFFFFFFF) { swapchainPresentMode = present_modes[res]; }

	GTSL::Array<GTSL::Pair<uint32, uint32>, 8> surface_formats{ { swapchainColorSpace, swapchainFormat } };
	res = surface.GetSupportedRenderContextFormat(&renderDevice, surface_formats);
	if (res != 0xFFFFFFFF) { swapchainColorSpace = surface_formats[res].First; swapchainFormat = surface_formats[res].Second; }

	GAL::RenderContext::CreateInfo render_context_create_info;
	render_context_create_info.RenderDevice = &renderDevice;
	render_context_create_info.DesiredFramesInFlight = 2;
	render_context_create_info.PresentMode = swapchainPresentMode;
	render_context_create_info.Format = swapchainFormat;
	render_context_create_info.ColorSpace = swapchainColorSpace;
	render_context_create_info.ImageUses = (uint32)ImageUse::COLOR_ATTACHMENT;
	render_context_create_info.Surface = &surface;
	GTSL::Extent2D window_extent;
	initializeRenderer.Window->GetFramebufferExtent(window_extent);
	render_context_create_info.SurfaceArea = window_extent;
	::new(&renderContext) RenderContext(render_context_create_info);

	initializeRenderer.Window->GetFramebufferExtent(renderArea);
	
	RenderPass::CreateInfo render_pass_create_info;
	render_pass_create_info.RenderDevice = &renderDevice;
	render_pass_create_info.Descriptor.DepthStencilAttachmentAvailable = false;
	GTSL::Array<GAL::AttachmentDescriptor, 8> attachment_descriptors;
	attachment_descriptors.PushBack(GAL::AttachmentDescriptor{ (uint32)ImageFormat::BGRA_I8, GAL::RenderTargetLoadOperations::CLEAR, GAL::RenderTargetStoreOperations::STORE, GAL::ImageLayout::UNDEFINED, GAL::ImageLayout::PRESENTATION });
	render_pass_create_info.Descriptor.RenderPassColorAttachments = attachment_descriptors;
	
	GTSL::Array<GAL::AttachmentReference, 8> write_attachment_references;
	write_attachment_references.PushBack(GAL::AttachmentReference{ 0, GAL::ImageLayout::COLOR_ATTACHMENT });
	
	GTSL::Array<GAL::SubPassDescriptor, 8> sub_pass_descriptors;
	sub_pass_descriptors.PushBack(GAL::SubPassDescriptor{ GTSL::Ranger<GAL::AttachmentReference>(), write_attachment_references, GTSL::Ranger<uint8>(), nullptr });
	render_pass_create_info.Descriptor.SubPasses = sub_pass_descriptors;

	::new(&renderPass) RenderPass(render_pass_create_info);
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	get_images_info.SwapchainImagesFormat = swapchainFormat;
	swapchainImages = renderContext.GetImages(get_images_info);

	clearValues.EmplaceBack(0, 0, 0, 0);

	for (uint8 i = 0; i < swapchainImages.GetLength(); ++i)
	{
		imageAvailableSemaphore.EmplaceBack(Semaphore::CreateInfo{ &renderDevice });
		renderFinishedSemaphore.EmplaceBack(Semaphore::CreateInfo{ &renderDevice });
		inFlightFences.EmplaceBack(Fence::CreateInfo{ &renderDevice, true });
		
		commandPools.EmplaceBack(CommandPool::CreateInfo{ &renderDevice, &graphicsQueue, true, GTSL::Ranger<CommandBuffer>(1, &commandBuffers[i]) });

		commandBuffers.Resize(swapchainImages.GetLength());
		
		FrameBuffer::CreateInfo framebuffer_create_info;
		framebuffer_create_info.RenderDevice = &renderDevice;
		framebuffer_create_info.RenderPass = &renderPass;
		framebuffer_create_info.Extent = window_extent;
		framebuffer_create_info.ImageViews = GTSL::Ranger<const ImageView>(1, &swapchainImages[i]);
		framebuffer_create_info.ClearValues = clearValues;

		frameBuffers.EmplaceBack(framebuffer_create_info);
	}

	scratchMemoryAllocator.Init(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
}

void RenderSystem::UpdateWindow(GTSL::Window& window)
{
	RenderContext::RecreateInfo recreate_info;
	recreate_info.RenderDevice = &renderDevice;
	recreate_info.DesiredFramesInFlight = swapchainImages.GetLength();
	recreate_info.PresentMode = swapchainPresentMode;
	recreate_info.ColorSpace = swapchainColorSpace;
	recreate_info.Format = swapchainFormat;
	window.GetFramebufferExtent(recreate_info.SurfaceArea);
	renderContext.Recreate(recreate_info);

	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	get_images_info.SwapchainImagesFormat = swapchainFormat;
	swapchainImages = renderContext.GetImages(get_images_info);
}

void RenderSystem::AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
{
	localMemoryAllocator.AllocateBuffer(renderDevice, memoryAllocationInfo.DeviceMemory, memoryAllocationInfo.Size, memoryAllocationInfo.Offset, GetPersistentAllocator());
}

void RenderSystem::AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo)
{
	scratchMemoryAllocator.AllocateBuffer(renderDevice, allocationInfo.DeviceMemory, allocationInfo.Size, allocationInfo.Offset, allocationInfo.Data, GetPersistentAllocator());
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	GTSL::Array<TaskDescriptor, 8> actsOn{ { "RenderSystem", AccessType::READ } };
	//initializeInfo.GameInstance->AddTask(__FUNCTION__, GTSL::Delegate<void(const GameInstance::TaskInfo&)>::Create<RenderSystem, &RenderSystem::render>(this), actsOn, "Frame");
}

void RenderSystem::Shutdown()
{
	uint8 i = 0;
	for (auto& e : commandPools) { e.FreeCommandBuffers({ &renderDevice, GTSL::Ranger<CommandBuffer>(1, &commandBuffers[i]) }); ++i; }
	for (auto& e : commandPools) { e.Destroy(&renderDevice); }
	
	renderPass.Destroy(&renderDevice);
	renderContext.Destroy(&renderDevice);
	surface.Destroy(&renderDevice);

	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : inFlightFences) { e.Destroy(&renderDevice); }

	for (auto& e : frameBuffers) { e.Destroy(&renderDevice); }
	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
}

void RenderSystem::render(const TaskInfo& taskInfo)
{
	auto& command_buffer = commandBuffers[index];
	
	Fence::WaitForFencesInfo wait_for_fences_info;
	wait_for_fences_info.RenderDevice = &renderDevice;
	wait_for_fences_info.Timeout = ~0ULL;
	wait_for_fences_info.WaitForAll = true;
	wait_for_fences_info.Fences = GTSL::Ranger<const Fence>(1, &inFlightFences[index]);
	Fence::WaitForFences(wait_for_fences_info);

	Fence::ResetFencesInfo reset_fences_info;
	reset_fences_info.RenderDevice = &renderDevice;
	reset_fences_info.Fences = GTSL::Ranger<const Fence>(1, &inFlightFences[index]);
	Fence::ResetFences(reset_fences_info);
	
	commandPools[index].ResetPool(&renderDevice);
	
	command_buffer.BeginRecording({});
	command_buffer.BeginRenderPass({&renderDevice, &renderPass, &frameBuffers[index], renderArea, clearValues});;
	command_buffer.EndRenderPass({&renderDevice});
	command_buffer.EndRecording({});

	RenderContext::AcquireNextImageInfo acquire_next_image_info;
	acquire_next_image_info.RenderDevice = &renderDevice;
	acquire_next_image_info.Semaphore = &imageAvailableSemaphore[index];
	auto image_index = renderContext.AcquireNextImage(acquire_next_image_info);
	
	Queue::SubmitInfo submit_info;
	submit_info.RenderDevice = &renderDevice;
	submit_info.Fence = &inFlightFences[index];
	submit_info.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &imageAvailableSemaphore[index]);
	submit_info.SignalSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[index]);
	submit_info.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, &commandBuffers[index]);
	GTSL::Array<uint32, 8> wps{ (uint32)GAL::PipelineStage::COLOR_ATTACHMENT_OUTPUT };
	submit_info.WaitPipelineStages = wps;
	graphicsQueue.Submit(submit_info);
	
	RenderContext::PresentInfo present_info;
	present_info.RenderDevice = &renderDevice;
	present_info.Queue = &graphicsQueue;
	present_info.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[index]);
	present_info.ImageIndex = image_index;
	renderContext.Present(present_info);

	index = (index + 1) % swapchainImages.GetLength();
}

void RenderSystem::printError(const char* message, const RenderDevice::MessageSeverity messageSeverity) const
{
	switch (messageSeverity)
	{
	case RenderDevice::MessageSeverity::MESSAGE: BE_LOG_MESSAGE(message); break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message); break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); break;
	default: break;
	}
}