#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Game/CameraSystem.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"

class CameraSystem;
class RenderStaticMeshCollection;

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	renderGroups.Initialize(16, GetPersistentAllocator());
	apiAllocations.Initialize(16, GetPersistentAllocator());

	{		
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>("Test");
		
		GTSL::Array<GAL::Queue::CreateInfo, 5> queue_create_infos(2);
		queue_create_infos[0].Capabilities = static_cast<uint8>(QueueCapabilities::GRAPHICS);
		queue_create_infos[0].QueuePriority = 1.0f;
		queue_create_infos[1].Capabilities = static_cast<uint8>(QueueCapabilities::TRANSFER);
		queue_create_infos[1].QueuePriority = 1.0f;
		createInfo.QueueCreateInfos = queue_create_infos;
		auto queues = GTSL::Array<Queue, 5>{ graphicsQueue, transferQueue };
		createInfo.Queues = queues;
		createInfo.Extensions = GTSL::Array<RenderDevice::Extension, 16>{ RenderDevice::Extension::PIPELINE_CACHE_EXTERNAL_SYNC };
		createInfo.DebugPrintFunction = GTSL::Delegate<void(const char*, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
		createInfo.AllocationInfo.UserData = this;
		createInfo.AllocationInfo.Allocate = GTSL::Delegate<void*(void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::allocateApiMemory>(this);
		createInfo.AllocationInfo.Reallocate = GTSL::Delegate<void*(void*, void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::reallocateApiMemory>(this);
		createInfo.AllocationInfo.Deallocate = GTSL::Delegate<void(void*, void*)>::Create<RenderSystem, &RenderSystem::deallocateApiMemory>(this);
		::new(&renderDevice) RenderDevice(createInfo);

		graphicsQueue = queues[0]; transferQueue = queues[1];
	}
	
	swapchainPresentMode = static_cast<uint32>(PresentMode::FIFO);
	swapchainColorSpace = static_cast<uint32>(ColorSpace::NONLINEAR_SRGB);
	swapchainFormat = static_cast<uint32>(TextureFormat::BGRA_I8);

	clearValues.EmplaceBack(0, 0, 0, 0);

	Surface::CreateInfo surfaceCreateInfo;
	surfaceCreateInfo.RenderDevice = &renderDevice;
	surfaceCreateInfo.Name = "Surface";
	GTSL::Window::Win32NativeHandles handles;
	initializeRenderer.Window->GetNativeHandles(&handles);
	GAL::WindowsWindowData windowsWindowData;
	windowsWindowData.InstanceHandle = GetModuleHandle(NULL);
	windowsWindowData.WindowHandle = handles.HWND;
	surfaceCreateInfo.SystemData = &handles;
	new(&surface) Surface(surfaceCreateInfo);
	
	RenderPass::CreateInfo renderPassCreateInfo;
	renderPassCreateInfo.RenderDevice = &renderDevice;
	renderPassCreateInfo.Descriptor.DepthStencilAttachmentAvailable = false;
	GTSL::Array<RenderPass::AttachmentDescriptor, 8> attachment_descriptors;
	attachment_descriptors.PushBack(RenderPass::AttachmentDescriptor{ (uint32)TextureFormat::BGRA_I8, GAL::RenderTargetLoadOperations::CLEAR, GAL::RenderTargetStoreOperations::STORE, TextureLayout::UNDEFINED, TextureLayout::PRESENTATION });
	renderPassCreateInfo.Descriptor.RenderPassColorAttachments = attachment_descriptors;

	GTSL::Array<RenderPass::AttachmentReference, 8> write_attachment_references;
	write_attachment_references.PushBack(RenderPass::AttachmentReference{ 0, TextureLayout::COLOR_ATTACHMENT });

	GTSL::Array<RenderPass::SubPassDescriptor, 8> sub_pass_descriptors;
	sub_pass_descriptors.PushBack(RenderPass::SubPassDescriptor{ GTSL::Ranger<RenderPass::AttachmentReference>(), write_attachment_references, GTSL::Ranger<uint8>(), nullptr });
	renderPassCreateInfo.Descriptor.SubPasses = sub_pass_descriptors;
	new(&renderPass) RenderPass(renderPassCreateInfo);
	
	for (uint32 i = 0; i < 2; ++i)
	{
		Semaphore::CreateInfo semaphore_create_info;
		semaphore_create_info.RenderDevice = &renderDevice;
		semaphore_create_info.Name = "ImageAvailableSemaphore";
		imageAvailableSemaphore.EmplaceBack(semaphore_create_info);
		semaphore_create_info.Name = "RenderFinishedSemaphore";
		renderFinishedSemaphore.EmplaceBack(semaphore_create_info);

		Fence::CreateInfo fence_create_info;
		fence_create_info.RenderDevice = &renderDevice;
		fence_create_info.Name = "InFlightFence";
		fence_create_info.IsSignaled = true;
		graphicsFences.EmplaceBack(fence_create_info);
		fence_create_info.Name = "TransferFence";
		fence_create_info.IsSignaled = false;
		transferFences.EmplaceBack(fence_create_info);

		{
			GTSL::StaticString<64> command_pool_name("Transfer command pool. Frame: "); command_pool_name += i;
			
			CommandPool::CreateInfo command_pool_create_info;
			command_pool_create_info.RenderDevice = &renderDevice;
			command_pool_create_info.Name = command_pool_name.begin();
			command_pool_create_info.Queue = &graphicsQueue;

			graphicsCommandPools.EmplaceBack(command_pool_create_info);
			
			GTSL::StaticString<64> command_buffer_name("Graphics command buffer. Frame: "); command_buffer_name += i;

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.IsPrimary = true;
			allocate_command_buffers_info.RenderDevice = &renderDevice;

			CommandBuffer::CreateInfo command_buffer_create_info; command_buffer_create_info.RenderDevice = &renderDevice; command_buffer_create_info.Name = command_buffer_name.begin();

			GTSL::Array<CommandBuffer::CreateInfo, 5> create_infos; create_infos.EmplaceBack(command_buffer_create_info);
			allocate_command_buffers_info.CommandBufferCreateInfos = create_infos;
			graphicsCommandBuffers.Resize(graphicsCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, graphicsCommandBuffers.begin() + i);
			graphicsCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}

		{
			GTSL::StaticString<64> command_pool_name("Transfer command pool. Frame: "); command_pool_name += i;
			
			CommandPool::CreateInfo command_pool_create_info;
			command_pool_create_info.RenderDevice = &renderDevice;
			command_pool_create_info.Name = command_pool_name.begin();
			command_pool_create_info.Queue = &transferQueue;
			transferCommandPools.EmplaceBack(command_pool_create_info);
			
			GTSL::StaticString<64> command_buffer_name("Transfer command buffer. Frame: "); command_buffer_name += i;

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.RenderDevice = &renderDevice;
			allocate_command_buffers_info.IsPrimary = true;

			CommandBuffer::CreateInfo command_buffer_create_info; command_buffer_create_info.RenderDevice = &renderDevice; command_buffer_create_info.Name = command_buffer_name.begin();
			
			GTSL::Array<CommandBuffer::CreateInfo, 5> create_infos; create_infos.EmplaceBack(command_buffer_create_info);
			allocate_command_buffers_info.CommandBufferCreateInfos = create_infos;
			transferCommandBuffers.Resize(transferCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, transferCommandBuffers.begin() + i);
			transferCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}

		bufferCopyDatas.EmplaceBack(128, GetPersistentAllocator());
		textureCopyDatas.EmplaceBack(128, GetPersistentAllocator());
	}
	
	scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());

	bool pipelineCacheAvailable;
	initializeRenderer.PipelineCacheResourceManager->DoesCacheExist(pipelineCacheAvailable);

	pipelineCaches.Initialize(7/*TODO: should be dynamic by threads*/, GetPersistentAllocator());
	
	if(pipelineCacheAvailable)
	{
		uint32 cacheSize = 0;
		initializeRenderer.PipelineCacheResourceManager->GetCacheSize(cacheSize);

		GTSL::Buffer pipelineCacheBuffer;
		pipelineCacheBuffer.Allocate(cacheSize, 32, GetPersistentAllocator());

		initializeRenderer.PipelineCacheResourceManager->GetCache(pipelineCacheBuffer);
		
		PipelineCache::CreateInfo pipelineCacheCreateInfo;
		pipelineCacheCreateInfo.RenderDevice = GetRenderDevice();
		pipelineCacheCreateInfo.ExternallySync = false;
		pipelineCacheCreateInfo.Data = pipelineCacheBuffer;

		for(uint8 i = 0; i < 7; ++i)
		{
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<32> name("Pipeline cache. Thread: "); name += i;
				pipelineCacheCreateInfo.Name = name.begin();
			}
			
			pipelineCaches.EmplaceBack(pipelineCacheCreateInfo);
		}
		
		pipelineCacheBuffer.Free(32, GetPersistentAllocator());
	}
	else
	{
		PipelineCache::CreateInfo pipelineCacheCreateInfo;
		pipelineCacheCreateInfo.RenderDevice = GetRenderDevice();
		pipelineCacheCreateInfo.ExternallySync = false;
		
		for (uint8 i = 0; i < 7; ++i)
		{
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<32> name("Pipeline cache. Thread: "); name += i;
				pipelineCacheCreateInfo.Name = name.begin();
			}
			
			pipelineCaches.EmplaceBack(pipelineCacheCreateInfo);
		}
	}
	
	BE_LOG_MESSAGE("Initialized successfully");
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

void RenderSystem::OnResize(TaskInfo taskInfo, const GTSL::Extent2D extent)
{
	if (extent != 0 && extent != renderArea)
	{
		graphicsQueue.Wait();

		BE_ASSERT(surface.IsSupported(&renderDevice) != false, "Surface is not supported!");

		GTSL::Array<GTSL::uint32, 4> present_modes{ swapchainPresentMode };
		auto res = surface.GetSupportedPresentMode(&renderDevice, present_modes);
		if (res != 0xFFFFFFFF) { swapchainPresentMode = present_modes[res]; }

		GTSL::Array<GTSL::Pair<uint32, uint32>, 8> surface_formats{ { swapchainColorSpace, swapchainFormat } };
		res = surface.GetSupportedRenderContextFormat(&renderDevice, surface_formats);
		if (res != 0xFFFFFFFF) { swapchainColorSpace = surface_formats[res].First; swapchainFormat = surface_formats[res].Second; }
		
		RenderContext::RecreateInfo recreate;
		recreate.RenderDevice = GetRenderDevice();
		recreate.SurfaceArea = extent;
		recreate.ColorSpace = swapchainColorSpace;
		recreate.DesiredFramesInFlight = 2;
		recreate.Format = swapchainFormat;
		recreate.PresentMode = swapchainPresentMode;
		recreate.Surface = &surface;
		recreate.ImageUses = TextureUses::COLOR_ATTACHMENT;
		renderContext.Recreate(recreate);

		for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }

		RenderContext::GetImagesInfo get_images_info;
		get_images_info.RenderDevice = &renderDevice;
		get_images_info.SwapchainImagesFormat = swapchainFormat;
		get_images_info.ImageViewName = GTSL::StaticString<64>("Swapchain image view. Frame: ");
		swapchainImages = renderContext.GetImages(get_images_info);

		for (auto& e : frameBuffers)
		{
			e.Destroy(GetRenderDevice());
		}

		frameBuffers.Resize(0);
		
		for (uint32 i = 0; i < swapchainImages.GetLength(); ++i)
		{
			FrameBuffer::CreateInfo framebuffer_create_info;
			framebuffer_create_info.RenderDevice = &renderDevice;
			framebuffer_create_info.RenderPass = &renderPass;
			framebuffer_create_info.Extent = extent;
			framebuffer_create_info.ImageViews = GTSL::Ranger<const TextureView>(1, &swapchainImages[i]);
			framebuffer_create_info.ClearValues = clearValues;

			frameBuffers.EmplaceBack(framebuffer_create_info);
		}

		renderArea = extent;
		
		BE_LOG_MESSAGE("Resized window")
	}
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	{
		const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessType::READ_WRITE } };
		initializeInfo.GameInstance->AddTask("frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, "FrameStart", "RenderStart");

		initializeInfo.GameInstance->AddTask("executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, "GameplayEnd", "RenderStart");
	}

	{
		const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessType::READ_WRITE }/*, { "MaterialSystem", AccessType::READ_WRITE }*/ };
		initializeInfo.GameInstance->AddTask("renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderSetup>(this), actsOn, "RenderStart", "FrameEnd");
	}

	{
		const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessType::READ_WRITE } };
		initializeInfo.GameInstance->AddTask("renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFinish>(this), actsOn, "RenderFinished", "RenderEnd");
	}
}

void RenderSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{	
	graphicsQueue.Wait();
	transferQueue.Wait();

	for (uint32 i = 0; i < swapchainImages.GetLength(); ++i)
	{
		CommandPool::FreeCommandBuffersInfo free_command_buffers_info;
		free_command_buffers_info.RenderDevice = &renderDevice;

		free_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, &graphicsCommandBuffers[i]);
		graphicsCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		free_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, &transferCommandBuffers[i]);
		transferCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		graphicsCommandPools[i].Destroy(&renderDevice);
		transferCommandPools[i].Destroy(&renderDevice);
	}
	
	renderPass.Destroy(&renderDevice);
	renderContext.Destroy(&renderDevice);
	surface.Destroy(&renderDevice);

	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : graphicsFences) { e.Destroy(&renderDevice); }
	for(auto& e : transferFences) { e.Destroy(&renderDevice); }

	for (auto& e : frameBuffers) { e.Destroy(&renderDevice); }
	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());

	{
		uint32 cacheSize = 0;

		PipelineCache::CreateFromMultipleInfo createPipelineCacheInfo;
		createPipelineCacheInfo.RenderDevice = GetRenderDevice();
		createPipelineCacheInfo.Caches = pipelineCaches;
		const PipelineCache pipelineCache(createPipelineCacheInfo);
		pipelineCache.GetCacheSize(GetRenderDevice(), cacheSize);

		if (cacheSize)
		{
			auto* pipelineCacheResourceManager = BE::Application::Get()->GetResourceManager<PipelineCacheResourceManager>("PipelineCacheResourceManager");
			GTSL::Buffer pipelineCacheBuffer;
			pipelineCacheBuffer.Allocate(cacheSize, 32, GetPersistentAllocator());
			pipelineCache.GetCache(&renderDevice, cacheSize, pipelineCacheBuffer);
			pipelineCacheResourceManager->WriteCache(pipelineCacheBuffer);
			pipelineCacheBuffer.Free(32, GetPersistentAllocator());
		}
	}
}

void RenderSystem::renderSetup(TaskInfo taskInfo)
{
	Fence::WaitForFencesInfo waitForFencesInfo;
	waitForFencesInfo.RenderDevice = &renderDevice;
	waitForFencesInfo.Timeout = ~0ULL;
	waitForFencesInfo.WaitForAll = true;
	waitForFencesInfo.Fences = GTSL::Ranger<const Fence>(1, &graphicsFences[currentFrameIndex]);
	Fence::WaitForFences(waitForFencesInfo);
	
	Fence::ResetFencesInfo resetFencesInfo;
	resetFencesInfo.RenderDevice = &renderDevice;
	resetFencesInfo.Fences = GTSL::Ranger<const Fence>(1, &graphicsFences[currentFrameIndex]);
	Fence::ResetFences(resetFencesInfo);
	
	graphicsCommandPools[currentFrameIndex].ResetPool(&renderDevice);

	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	commandBuffer.BeginRecording({});
	CommandBuffer::BeginRenderPassInfo beginRenderPass;
	beginRenderPass.RenderDevice = GetRenderDevice();
	beginRenderPass.RenderPass = &renderPass;
	beginRenderPass.Framebuffer = &frameBuffers[currentFrameIndex];
	beginRenderPass.RenderArea = renderArea;
	beginRenderPass.ClearValues = clearValues;
	commandBuffer.BeginRenderPass(beginRenderPass);
}

void RenderSystem::renderFinish(TaskInfo taskInfo)
{
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	CommandBuffer::EndRenderPassInfo endRenderPass;
	endRenderPass.RenderDevice = GetRenderDevice();
	commandBuffer.EndRenderPass(endRenderPass);
	commandBuffer.EndRecording({});

	RenderContext::AcquireNextImageInfo acquireNextImageInfo;
	acquireNextImageInfo.RenderDevice = &renderDevice;
	acquireNextImageInfo.SignalSemaphore = &imageAvailableSemaphore[currentFrameIndex];
	auto imageIndex = renderContext.AcquireNextImage(acquireNextImageInfo);

	//BE_ASSERT(imageIndex == currentFrameIndex, "Data mismatch");

	Queue::SubmitInfo submitInfo;
	submitInfo.RenderDevice = &renderDevice;
	submitInfo.Fence = &graphicsFences[currentFrameIndex];
	submitInfo.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &imageAvailableSemaphore[currentFrameIndex]);
	submitInfo.SignalSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[currentFrameIndex]);
	submitInfo.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, &commandBuffer);
	GTSL::Array<uint32, 8> wps{ (uint32)PipelineStage::COLOR_ATTACHMENT_OUTPUT };
	submitInfo.WaitPipelineStages = wps;
	graphicsQueue.Submit(submitInfo);

	RenderContext::PresentInfo presentInfo;
	presentInfo.RenderDevice = &renderDevice;
	presentInfo.Queue = &graphicsQueue;
	presentInfo.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[currentFrameIndex]);
	presentInfo.ImageIndex = imageIndex;
	renderContext.Present(presentInfo);

	currentFrameIndex = (currentFrameIndex + 1) % swapchainImages.GetLength();
}

void RenderSystem::frameStart(TaskInfo taskInfo)
{
	//Fence::WaitForFencesInfo wait_for_fences_info;
	//wait_for_fences_info.RenderDevice = &renderDevice;
	//wait_for_fences_info.Timeout = ~0ULL;
	//wait_for_fences_info.WaitForAll = true;
	//wait_for_fences_info.Fences = GTSL::Ranger<const Fence>(1, &transferFences[currentFrameIndex]);
	//Fence::WaitForFences(wait_for_fences_info);//

	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];
	
	if(transferFences[currentFrameIndex].GetStatus(&renderDevice))
	{
		for(uint32 i = 0; i < bufferCopyData.GetLength(); ++i)
		{
			bufferCopyData[i].SourceBuffer.Destroy(&renderDevice);
			DeallocateScratchBufferMemory(bufferCopyDatas[currentFrameIndex][i].Allocation);
		}

		for(uint32 i = 0; i < textureCopyData.GetLength(); ++i)
		{
			textureCopyData[i].SourceBuffer.Destroy(&renderDevice);
			DeallocateScratchBufferMemory(bufferCopyDatas[currentFrameIndex][i].Allocation);
		}
		
		bufferCopyData.ResizeDown(0);
		textureCopyData.ResizeDown(0);

		Fence::ResetFencesInfo reset_fences_info;
		reset_fences_info.RenderDevice = &renderDevice;
		reset_fences_info.Fences = GTSL::Ranger<const Fence>(1, &transferFences[currentFrameIndex]);
		Fence::ResetFences(reset_fences_info);
	}
	
	transferCommandPools[currentFrameIndex].ResetPool(&renderDevice); //should only be done if frame is finished transferring but must also implement check in execute transfers
	//or begin command buffer complains
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	CommandBuffer::BeginRecordingInfo beginRecordingInfo;
	beginRecordingInfo.RenderDevice = &renderDevice;
	beginRecordingInfo.PrimaryCommandBuffer = &transferCommandBuffers[currentFrameIndex];
	transferCommandBuffers[currentFrameIndex].BeginRecording(beginRecordingInfo);
	
	for(auto& e : bufferCopyDatas[currentFrameIndex])
	{
		CommandBuffer::CopyBuffersInfo copy_buffers_info;
		copy_buffers_info.RenderDevice = &renderDevice;
		copy_buffers_info.Destination = &e.DestinationBuffer;
		copy_buffers_info.DestinationOffset = e.DestinationOffset;
		copy_buffers_info.Source = &e.SourceBuffer;
		copy_buffers_info.SourceOffset = e.SourceOffset;
		copy_buffers_info.Size = e.Size;
		GetTransferCommandBuffer()->CopyBuffers(copy_buffers_info);
	}

	{
		auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];
		
		GTSL::Vector<CommandBuffer::TextureBarrier, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());
		GTSL::Vector<CommandBuffer::TextureBarrier, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());

		
		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i)
		{
			sourceTextureBarriers[i].Texture = textureCopyData[i].DestinationTexture;
			sourceTextureBarriers[i].SourceAccessFlags = 0;
			sourceTextureBarriers[i].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
			sourceTextureBarriers[i].CurrentLayout = TextureLayout::UNDEFINED;
			sourceTextureBarriers[i].TargetLayout = TextureLayout::TRANSFER_DST;

			destinationTextureBarriers[i].Texture = textureCopyData[i].DestinationTexture;
			destinationTextureBarriers[i].SourceAccessFlags = AccessFlags::TRANSFER_WRITE;
			destinationTextureBarriers[i].DestinationAccessFlags = AccessFlags::SHADER_READ;
			destinationTextureBarriers[i].CurrentLayout = TextureLayout::TRANSFER_DST;
			destinationTextureBarriers[i].TargetLayout = TextureLayout::SHADER_READ_ONLY;
		}


		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = GetRenderDevice();
		pipelineBarrierInfo.TextureBarriers = sourceTextureBarriers;
		pipelineBarrierInfo.InitialStage = PipelineStage::TOP_OF_PIPE;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GetTransferCommandBuffer()->AddPipelineBarrier(pipelineBarrierInfo);

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i)
		{
			CommandBuffer::CopyBufferToImageInfo copyBufferToImageInfo;
			copyBufferToImageInfo.RenderDevice = GetRenderDevice();
			copyBufferToImageInfo.DestinationImage = &textureCopyData[i].DestinationTexture;
			copyBufferToImageInfo.Offset = { 0, 0, 0 };
			copyBufferToImageInfo.Extent = textureCopyData[i].Extent;
			copyBufferToImageInfo.SourceBuffer = &textureCopyData[i].SourceBuffer;
			copyBufferToImageInfo.TextureLayout = textureCopyData[i].Layout;
			GetTransferCommandBuffer()->CopyBufferToImage(copyBufferToImageInfo);
		}
			
		pipelineBarrierInfo.TextureBarriers = destinationTextureBarriers;
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::FRAGMENT_SHADER;
		GetTransferCommandBuffer()->AddPipelineBarrier(pipelineBarrierInfo);
	}
	
	CommandBuffer::EndRecordingInfo endRecordingInfo;
	endRecordingInfo.RenderDevice = &renderDevice;
	GetTransferCommandBuffer()->EndRecording(endRecordingInfo);
	
	if (bufferCopyDatas[currentFrameIndex].GetLength() || textureCopyDatas[GetCurrentFrame()].GetLength())
	{
		Queue::SubmitInfo submit_info;
		submit_info.RenderDevice = &renderDevice;
		submit_info.Fence = &transferFences[currentFrameIndex];
		submit_info.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, GetTransferCommandBuffer());
		submit_info.WaitPipelineStages = GTSL::Array<uint32, 2>{ static_cast<uint32>(PipelineStage::TRANSFER) };
		transferQueue.Submit(submit_info);
	}
}

void RenderSystem::printError(const char* message, const RenderDevice::MessageSeverity messageSeverity) const
{
	switch (messageSeverity)
	{
	case RenderDevice::MessageSeverity::MESSAGE: /*BE_LOG_MESSAGE(message);*/ break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message); break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); break;
	default: break;
	}
}

void* RenderSystem::allocateApiMemory(void* data, const uint64 size, const uint64 alignment)
{
	void* allocation; uint64 allocated_size;
	GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);
	apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
	return allocation;
}

void* RenderSystem::reallocateApiMemory(void* data, void* oldAllocation, uint64 size, uint64 alignment)
{
	void* allocation; uint64 allocated_size;
	
	const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(oldAllocation));
	
	GetPersistentAllocator().Allocate(size, old_alloc.Second, &allocation, &allocated_size);
	apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
	
	GTSL::MemCopy(old_alloc.First, oldAllocation, allocation);
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
	apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
	return allocation;
}

void RenderSystem::deallocateApiMemory(void* data, void* allocation)
{
	const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(allocation));
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, allocation);
	apiAllocations.Remove(reinterpret_cast<uint64>(allocation));
}
