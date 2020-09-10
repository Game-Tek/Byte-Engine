#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"
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
	
	swapchainPresentMode = PresentMode::FIFO;
	swapchainColorSpace = ColorSpace::NONLINEAR_SRGB;
	swapchainFormat = TextureFormat::BGRA_I8;
	
	Surface::CreateInfo surfaceCreateInfo;
	surfaceCreateInfo.RenderDevice = &renderDevice;
	if constexpr (_DEBUG) { surfaceCreateInfo.Name = "Surface"; }
	GTSL::Window::Win32NativeHandles handles;
	initializeRenderer.Window->GetNativeHandles(&handles);
	GAL::WindowsWindowData windowsWindowData;
	windowsWindowData.InstanceHandle = GetModuleHandle(NULL);
	windowsWindowData.WindowHandle = handles.HWND;
	surfaceCreateInfo.SystemData = &windowsWindowData;
	new(&surface) Surface(surfaceCreateInfo);

	scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
	
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
			CommandPool::CreateInfo commandPoolCreateInfo;
			commandPoolCreateInfo.RenderDevice = &renderDevice;
			
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> commandPoolName("Transfer command pool. Frame: "); commandPoolName += i;
				commandPoolCreateInfo.Name = commandPoolName.begin();
			}
			
			commandPoolCreateInfo.Queue = &graphicsQueue;

			graphicsCommandPools.EmplaceBack(commandPoolCreateInfo);

			CommandPool::AllocateCommandBuffersInfo allocateCommandBuffersInfo;
			allocateCommandBuffersInfo.IsPrimary = true;
			allocateCommandBuffersInfo.RenderDevice = &renderDevice;

			CommandBuffer::CreateInfo commandBufferCreateInfo;
			commandBufferCreateInfo.RenderDevice = &renderDevice;

			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> commandBufferName("Graphics command buffer. Frame: "); commandBufferName += i;
				commandBufferCreateInfo.Name = commandBufferName.begin();
			}

			GTSL::Array<CommandBuffer::CreateInfo, 5> createInfos; createInfos.EmplaceBack(commandBufferCreateInfo);
			allocateCommandBuffersInfo.CommandBufferCreateInfos = createInfos;
			graphicsCommandBuffers.Resize(graphicsCommandBuffers.GetLength() + 1);
			allocateCommandBuffersInfo.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, graphicsCommandBuffers.begin() + i);
			graphicsCommandPools[i].AllocateCommandBuffer(allocateCommandBuffersInfo);
		}

		{
			
			CommandPool::CreateInfo commandPoolCreateInfo;
			commandPoolCreateInfo.RenderDevice = &renderDevice;
			
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> commandPoolName("Transfer command pool. Frame: "); commandPoolName += i;
				commandPoolCreateInfo.Name = commandPoolName.begin();
			}
			
			commandPoolCreateInfo.Queue = &transferQueue;
			transferCommandPools.EmplaceBack(commandPoolCreateInfo);

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.RenderDevice = &renderDevice;
			allocate_command_buffers_info.IsPrimary = true;

			CommandBuffer::CreateInfo commandBufferCreateInfo;
			commandBufferCreateInfo.RenderDevice = &renderDevice;
			
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> commandBufferName("Transfer command buffer. Frame: "); commandBufferName += i;
				commandBufferCreateInfo.Name = commandBufferName.begin();	
			}
			
			GTSL::Array<CommandBuffer::CreateInfo, 5> createInfos; createInfos.EmplaceBack(commandBufferCreateInfo);
			allocate_command_buffers_info.CommandBufferCreateInfos = createInfos;
			transferCommandBuffers.Resize(transferCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, transferCommandBuffers.begin() + i);
			transferCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}

		bufferCopyDatas.EmplaceBack(128, GetPersistentAllocator());
		textureCopyDatas.EmplaceBack(128, GetPersistentAllocator());
	}

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

void RenderSystem::OnResize(TaskInfo taskInfo, const GTSL::Extent2D extent)
{
	graphicsQueue.Wait();

	BE_ASSERT(surface.IsSupported(&renderDevice) != false, "Surface is not supported!");

	GTSL::Array<PresentMode, 4> present_modes{ swapchainPresentMode };
	auto res = surface.GetSupportedPresentMode(&renderDevice, present_modes);
	if (res != 0xFFFFFFFF) { swapchainPresentMode = present_modes[res]; }

	GTSL::Array<GTSL::Pair<ColorSpace, TextureFormat>, 8> surface_formats{ { swapchainColorSpace, swapchainFormat } };
	res = surface.GetSupportedRenderContextFormat(&renderDevice, surface_formats);
	if (res != 0xFFFFFFFF) { swapchainColorSpace = surface_formats[res].First; swapchainFormat = surface_formats[res].Second; }
	
	RenderContext::RecreateInfo recreate;
	recreate.RenderDevice = GetRenderDevice();
	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Swapchain");
		recreate.Name = name.begin();
	}
	recreate.SurfaceArea = extent;
	recreate.ColorSpace = swapchainColorSpace;
	recreate.DesiredFramesInFlight = 2;
	recreate.Format = swapchainFormat;
	recreate.PresentMode = swapchainPresentMode;
	recreate.Surface = &surface;
	recreate.TextureUses = TextureUses::COLOR_ATTACHMENT | TextureUses::TRANSFER_DESTINATION;
	renderContext.Recreate(recreate);

	auto oldLength = swapchainTextures.GetLength();
	
	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	{
		RenderContext::GetTexturesInfo getTexturesInfo;
		getTexturesInfo.RenderDevice = GetRenderDevice();
		swapchainTextures = renderContext.GetTextures(getTexturesInfo);

		textureBarriers.Resize(swapchainTextures.GetLength());
		
		for (uint8 i = oldLength; i < swapchainTextures.GetLength(); ++i)
		{
			textureBarriers[i].Initialize(32, GetPersistentAllocator());
		}

		if(textureBarriers[0].GetLength())
		{
			textureBarriers[0].ResizeDown(0);
		}
		
		for(uint32 i = 0; i < swapchainTextures.GetLength(); ++i)
		{
			CommandBuffer::TextureBarrier textureBarrier;
			textureBarrier.Texture = swapchainTextures[i];
			textureBarrier.CurrentLayout = TextureLayout::UNDEFINED;
			textureBarrier.TargetLayout = TextureLayout::PRESENTATION;
			textureBarrier.SourceAccessFlags = AccessFlags::TRANSFER_READ;
			textureBarrier.DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;

			textureBarriers[0].EmplaceBack(textureBarrier);
		}
	}
	
	RenderContext::GetTextureViewsInfo getTextureViewsInfo;
	getTextureViewsInfo.RenderDevice = &renderDevice;
	GTSL::Array<TextureView::CreateInfo, 3> textureViewCreateInfos(GetFrameCount());
	{
		for (uint8 i = 0; i < GetFrameCount(); ++i)
		{
			textureViewCreateInfos[i].RenderDevice = GetRenderDevice();
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> name("Swapchain texture view. Frame: "); name += static_cast<uint16>(i); //cast to not consider it a char
				textureViewCreateInfos[i].Name = name.begin();
			}
			textureViewCreateInfos[i].Format = swapchainFormat;
		}
	}
	getTextureViewsInfo.TextureViewCreateInfos = textureViewCreateInfos;

	{
		swapchainTextureViews = renderContext.GetTextureViews(getTextureViewsInfo);
	}

	renderArea = extent;
	
	BE_LOG_MESSAGE("Resized window")
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
	for (uint32 i = 0; i < swapchainTextures.GetLength(); ++i)
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
	
	renderContext.Destroy(&renderDevice);
	surface.Destroy(&renderDevice);
	
	//DeallocateLocalTextureMemory();
	
	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : graphicsFences) { e.Destroy(&renderDevice); }
	for(auto& e : transferFences) { e.Destroy(&renderDevice); }

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

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

void RenderSystem::Wait()
{
	graphicsQueue.Wait();
	transferQueue.Wait();
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
}

void RenderSystem::renderFinish(TaskInfo taskInfo)
{
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];

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

	currentFrameIndex = (currentFrameIndex + 1) % swapchainTextureViews.GetLength();
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
			DeallocateScratchBufferMemory(bufferCopyData[i].Allocation);
		}

		for(uint32 i = 0; i < textureCopyData.GetLength(); ++i)
		{
			textureCopyData[i].SourceBuffer.Destroy(&renderDevice);
			DeallocateScratchBufferMemory(textureCopyData[i].Allocation);
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
		if (textureBarriers[currentFrameIndex].GetLength())
		{
			CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
			pipelineBarrierInfo.RenderDevice = GetRenderDevice();
			pipelineBarrierInfo.TextureBarriers = textureBarriers[currentFrameIndex];
			pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
			pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
			GetTransferCommandBuffer()->AddPipelineBarrier(pipelineBarrierInfo);
		}
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
			CommandBuffer::CopyBufferToTextureInfo copyBufferToImageInfo;
			copyBufferToImageInfo.RenderDevice = GetRenderDevice();
			copyBufferToImageInfo.DestinationTexture = &textureCopyData[i].DestinationTexture;
			copyBufferToImageInfo.Offset = { 0, 0, 0 };
			copyBufferToImageInfo.Extent = textureCopyData[i].Extent;
			copyBufferToImageInfo.SourceBuffer = &textureCopyData[i].SourceBuffer;
			copyBufferToImageInfo.TextureLayout = textureCopyData[i].Layout;
			GetTransferCommandBuffer()->CopyBufferToTexture(copyBufferToImageInfo);
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
		submit_info.WaitPipelineStages = GTSL::Array<uint32, 2>{ /*static_cast<uint32>(PipelineStage::TRANSFER)*/ };
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
