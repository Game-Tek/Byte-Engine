#include "RenderSystem.h"

#include <GTSL/Window.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"

#undef MemoryBarrier

class CameraSystem;
class RenderStaticMeshCollection;

PipelineCache RenderSystem::GetPipelineCache() const { return pipelineCaches[GTSL::Thread::ThisTreadID()]; }

//RenderSystem::MeshHandle RenderSystem::CreateMesh(Id name, uint32 customIndex, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, ShaderGroupHandle materialHandle)
//{
//	auto meshIndex = meshes.Emplace(); auto& mesh = meshes[meshIndex];
//	mesh.CustomMeshIndex = customIndex;
//	mesh.MaterialHandle = materialHandle;
//
//	auto meshHandle = MeshHandle(meshIndex);
//	
//	SignalMeshDataUpdate(meshHandle, vertexCount, vertexSize, indexCount, indexSize);
//	return meshHandle;
//}

RenderSystem::RenderSystem(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderSystem"),
	bufferCopyDatas{ { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() } },
	textureCopyDatas{ { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() } },
	accelerationStructures(16, GetPersistentAllocator()), buffers(32, GetPersistentAllocator()),
	pipelineCaches(16, decltype(pipelineCaches)::allocator_t()),
	textures(16, GetPersistentAllocator()), apiAllocations(128, GetPersistentAllocator())
{
	{
		initializeInfo.ApplicationManager->AddTask(this, u8"endCommandLists", &RenderSystem::renderFlush, DependencyBlock(), u8"FrameEnd", u8"FrameEnd");
		resizeHandle = initializeInfo.ApplicationManager->StoreDynamicTask(this, u8"onResize", {}, & RenderSystem::onResize);
	}

	RenderDevice::RayTracingCapabilities rayTracingCapabilities;

	useHDR = BE::Application::Get()->GetBoolOption(u8"hdr");
	pipelinedFrames = static_cast<uint8>(GTSL::Math::Clamp((uint32)BE::Application::Get()->GetUINTOption(u8"buffer"), 2u, 3u));
	bool rayTracing = BE::Application::Get()->GetBoolOption(u8"rayTracing");

	{
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>(BE::Application::Get()->GetApplicationName());
		createInfo.ApplicationVersion[0] = 0; createInfo.ApplicationVersion[1] = 0; createInfo.ApplicationVersion[2] = 0;

		createInfo.Debug = static_cast<bool>(BE::Application::Get()->GetUINTOption(u8"debug"));

		GTSL::StaticVector<GAL::QueueType, 5> queue_create_infos;
		GTSL::StaticVector<RenderDevice::QueueKey, 5> queueKeys;

		queue_create_infos.EmplaceBack(GAL::QueueTypes::GRAPHICS); queueKeys.EmplaceBack();
		//queue_create_infos.EmplaceBack(GAL::QueueTypes::TRANSFER); queueKeys.EmplaceBack();

		createInfo.Queues = queue_create_infos;
		createInfo.QueueKeys = queueKeys;

		GTSL::StaticVector<GTSL::Pair<RenderDevice::Extension, void*>, 8> extensions{ { RenderDevice::Extension::PIPELINE_CACHE_EXTERNAL_SYNC, nullptr } };
		extensions.EmplaceBack(RenderDevice::Extension::SWAPCHAIN_RENDERING, nullptr);
		extensions.EmplaceBack(RenderDevice::Extension::SCALAR_LAYOUT, nullptr);
		if (rayTracing) { extensions.EmplaceBack(RenderDevice::Extension::RAY_TRACING, &rayTracingCapabilities); }

		createInfo.Extensions = extensions;
		createInfo.PerformanceValidation = true;
		createInfo.SynchronizationValidation = true;
		createInfo.DebugPrintFunction = GTSL::Delegate<void(GTSL::StringView, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
		createInfo.AllocationInfo.UserData = this;
		createInfo.AllocationInfo.Allocate = GTSL::Delegate<void* (void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::allocateApiMemory>(this);
		createInfo.AllocationInfo.Reallocate = GTSL::Delegate<void* (void*, void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::reallocateApiMemory>(this);
		createInfo.AllocationInfo.Deallocate = GTSL::Delegate<void(void*, void*)>::Create<RenderSystem, &RenderSystem::deallocateApiMemory>(this);

		if (auto renderDeviceInitializationResult = renderDevice.Initialize(createInfo, GetTransientAllocator())) {
			BE_LOG_SUCCESS(u8"Started RenderDevice\n	API: Vulkan\n	GPU: ", renderDevice.GetGPUInfo().GPUName, u8"\n	Memory: ", 6, u8" GB\n	API Version: ", renderDevice.GetGPUInfo().APIVersion);
		} else {
			BE_LOG_ERROR(u8"Failed to initialize RenderDevice!\n	API: Vulkan\n	Reason: \"", renderDeviceInitializationResult.Get(), u8"\n");
		}

		graphicsQueue.Initialize(GetRenderDevice(), queueKeys[0]);

		{
			needsStagingBuffer = true;

			auto memoryHeaps = renderDevice.GetMemoryHeaps(); GAL::VulkanRenderDevice::MemoryHeap& biggestGPUHeap = memoryHeaps[0];

			for (auto& e : memoryHeaps) {
				if (e.HeapType & GAL::MemoryTypes::GPU) {
					if (e.Size > biggestGPUHeap.Size) {
						biggestGPUHeap = e;

						for (auto& mt : e.MemoryTypes) {
							if (mt & GAL::MemoryTypes::GPU && mt & GAL::MemoryTypes::HOST_COHERENT && mt & GAL::MemoryTypes::HOST_VISIBLE) {
								needsStagingBuffer = false; break;
							}
						}
					}
				}
			}
		}

		scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
		localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());

		if (rayTracing) {
			shaderGroupHandleAlignment = rayTracingCapabilities.ShaderGroupHandleAlignment;
			shaderGroupHandleSize = rayTracingCapabilities.ShaderGroupHandleSize;
			scratchBufferOffsetAlignment = rayTracingCapabilities.ScratchBuildOffsetAlignment;
			shaderGroupBaseAlignment = rayTracingCapabilities.ShaderGroupBaseAlignment;

			accelerationStructureBuildDevice = rayTracingCapabilities.BuildDevice;
		}
	}

	for (uint8 f = 0; f < pipelinedFrames; ++f) {
		initializeFrameResources(f);
	}

	bool pipelineCacheAvailable;
	auto* pipelineCacheManager = initializeInfo.ApplicationManager->GetSystem<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
	pipelineCacheManager->DoesCacheExist(pipelineCacheAvailable);

	if (pipelineCacheAvailable) {
		uint32 cacheSize = 0;
		pipelineCacheManager->GetCacheSize(cacheSize);

		GTSL::Buffer pipelineCacheBuffer(cacheSize, 32, GetTransientAllocator());

		pipelineCacheManager->GetCache(pipelineCacheBuffer);

		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, static_cast<GTSL::Range<const GTSL::byte*>>(pipelineCacheBuffer));
		}
	} else {
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, {});
		}
	}

	BE_LOG_MESSAGE(u8"Initialized successfully");
}

class PresentKey {
	Synchronizer Fence;
	uint8 ImageIndex = 0;
};

RenderSystem::~RenderSystem() {
	renderDevice.Wait();

	for (uint32 i = 0; i < pipelinedFrames; ++i) {
		freeFrameResources(i);
	}

	if (renderContext.GetHandle())
		renderContext.Destroy(&renderDevice);

	if (surface.GetHandle())
		surface.Destroy(&renderDevice);

	for (auto& e : swapchainTextureViews) {
		if (e.GetVkImageView())
			e.Destroy(&renderDevice);
	}

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());

	{
		uint32 cacheSize = 0; PipelineCache pipelineCache;
		pipelineCache.Initialize(GetRenderDevice(), pipelineCaches);
		pipelineCache.GetCacheSize(GetRenderDevice(), cacheSize);

		if (cacheSize) {
			//auto* pipelineCacheResourceManager = shutdownInfo.ApplicationManager->GetSystem<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
			//
			//GTSL::Buffer pipelineCacheBuffer(cacheSize, 32, GetTransientAllocator());
			//pipelineCache.GetCache(&renderDevice, pipelineCacheBuffer);
			//pipelineCacheResourceManager->WriteCache(pipelineCacheBuffer);
		}
	}
}

void RenderSystem::beginGraphicsCommandLists(CommandListData& command_list_data)
{
	{
		auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];

		for (auto& e : bufferCopyData) {
			auto& buffer = buffers[e.BufferHandle()];

			if (buffer.isMulti) {
				__debugbreak();
			} else {
				command_list_data.CommandList.CopyBuffer(GetRenderDevice(), buffer.Staging[0], e.Offset, buffer.Buffer[0], 0, buffer.Size); //TODO: offset
				--buffer.references;
			}
		}

		processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
		bufferCopyData.Resize(0);
	}

	if (auto& textureCopyData = textureCopyDatas[GetCurrentFrame()]; textureCopyData) {
		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			sourceTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format });
			destinationTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, GAL::PipelineStages::FRAGMENT, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, textureCopyData[i].Format });
		}

		command_list_data.CommandList.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			command_list_data.CommandList.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
		}

		command_list_data.CommandList.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GetTransientAllocator());
		textureCopyDatas[GetCurrentFrame()].Resize(0);
	}
}

void RenderSystem::renderFlush(TaskInfo taskInfo) {
	auto beforeFrame = uint8(currentFrameIndex - uint8(1)) % GetPipelinedFrames();

	++currentFrameIndex %= pipelinedFrames;
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	//auto& commandBuffer = transferCommandBuffers[GetCurrentFrame()];
	//auto& commandBuffer = graphicsCommandBuffers[GetCurrentFrame()];
	
	//commandBuffer.BeginRecording(GetRenderDevice());
	
	//{
	//	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	//	
	//	for (auto& e : bufferCopyData) //TODO: What to do with multibuffers.
	//	{
	//		auto& buffer = buffers[e.Buffer()]; auto& stagingBuffer = buffers[buffer.Staging()];
	//		
	//		commandBuffer.CopyBuffer(GetRenderDevice(), stagingBuffer.Buffer, e.Offset, buffer.Buffer, 0, buffer.Size); //TODO: offset
	//		--stagingBuffer.references;
	//	}
	//
	//	processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
	//}
	//
	//if (auto & textureCopyData = textureCopyDatas[GetCurrentFrame()]; textureCopyData.GetLength())
	//{
	//	GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
	//	GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
	//
	//	for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
	//		sourceTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, textureCopyData[i].Format });
	//		destinationTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, textureCopyData[i].Format });
	//	}
	//
	//	commandBuffer.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
	//
	//	for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
	//		commandBuffer.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
	//	}
	//
	//	commandBuffer.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::FRAGMENT, GetTransientAllocator());
	//	textureCopyDatas[GetCurrentFrame()].Resize(0);
	//}
		
	//processedTextureCopies[GetCurrentFrame()] = textureCopyData.GetLength();

	//commandBuffer.EndRecording(GetRenderDevice());
	
	////if (bufferCopyDatas[currentFrameIndex].GetLength() || textureCopyDatas[GetCurrentFrame()].GetLength())
	////{
	//	GTSL::StaticVector<GAL::Queue::WorkUnit, 8> workUnits;
	//	auto& workUnit = workUnits.EmplaceBack();
	//	workUnit.CommandBuffer = &commandBuffer;
	//	workUnit.PipelineStage = GAL::PipelineStages::TRANSFER;
	//	workUnit.SignalSemaphore = &transferDoneSemaphores[GetCurrentFrame()];
	//
	//	graphicsQueue.Submit(GetRenderDevice(), workUnits, transferFences[currentFrameIndex]);
	////}
}

RenderSystem::TextureHandle RenderSystem::CreateTexture(GTSL::Range<const char8_t*> name, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable, TextureHandle texture_handle)
{
	auto doTexture = [&](TextureComponent& texture) {
		const auto textureSize = extent.Width * extent.Height * extent.Depth * formatDescriptor.GetSize();

		if (updatable && needsStagingBuffer) {
			AllocateScratchBufferMemory(textureSize, GAL::BufferUses::TRANSFER_SOURCE, &texture.ScratchBuffer, &texture.ScratchAllocation);
		}

		AllocateLocalTextureMemory(&texture.Texture, name, texture.Uses, texture.FormatDescriptor, extent, GAL::Tiling::OPTIMAL, 1, &texture.Allocation);
		texture.TextureView.Initialize(GetRenderDevice(), name, texture.Texture, texture.FormatDescriptor, extent, 1);
	};

	if(texture_handle) {
		auto& texture = textures[texture_handle()];

		if(extent != texture.Extent) {
			if(texture.Texture.GetVkImage()) {
				texture.Texture.Destroy(GetRenderDevice());
				DeallocateLocalBufferMemory(texture.Allocation);

				if (texture.ScratchAllocation.Data) {
					DeallocateScratchBufferMemory(texture.ScratchAllocation);
				}
			}

			if(texture.TextureView.GetVkImageView()) {
				texture.TextureView.Destroy(GetRenderDevice());
			}

			doTexture(texture);
		}

		return texture_handle;
	}

	const auto textureIndex = textures.Emplace();

	auto& texture = textures[textureIndex];
	
	texture.Extent = extent;	
	texture.FormatDescriptor = formatDescriptor;
	texture.Uses = textureUses;
	if (updatable) { texture.Uses |= GAL::TextureUses::TRANSFER_DESTINATION; }
	texture.Layout = GAL::TextureLayout::UNDEFINED;	

	doTexture(texture);

	return TextureHandle(textureIndex);
}

void RenderSystem::UpdateTexture(const TextureHandle textureHandle)
{
	const auto& texture = textures[textureHandle()];

	TextureCopyData textureCopyData;
	textureCopyData.Layout = texture.Layout;
	textureCopyData.Extent = texture.Extent;
	textureCopyData.Allocation = texture.Allocation;
	textureCopyData.DestinationTexture = texture.Texture;
	textureCopyData.SourceOffset = 0;
	textureCopyData.SourceBuffer = texture.ScratchBuffer;
	textureCopyData.Format = texture.FormatDescriptor;
	AddTextureCopy(textureCopyData);
	
	//TODO: QUEUE BUFFER DELETION
}

void RenderSystem::OnRenderEnable(TaskInfo taskInfo, bool oldFocus)
{
	if(!oldFocus) {
		BE_LOG_SUCCESS(u8"Enabled rendering")
	}

	//OnResize(window->GetFramebufferExtent());
}

void RenderSystem::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	if (oldFocus) {
		BE_LOG_SUCCESS(u8"Disabled rendering")
	}
}

GTSL::Result<GTSL::Extent2D> RenderSystem::AcquireImage()
{
	bool result = false;
	
	if(!surface.GetHandle()) {
		resize(); result = true;
	}

	const auto acquireResult = renderContext.AcquireNextImage(&renderDevice, &imageAvailableSemaphore[GetCurrentFrame()]);

	imageIndex = acquireResult.Get();

	switch (acquireResult.State()) {
	case GAL::VulkanRenderContext::AcquireState::OK: break;
	case GAL::VulkanRenderContext::AcquireState::SUBOPTIMAL:
	case GAL::VulkanRenderContext::AcquireState::BAD: resize(); result = true; break;
	}

	if (lastRenderArea != renderArea) { lastRenderArea = renderArea; result = true; }
	
	return { GTSL::MoveRef(renderArea), result };
}

void RenderSystem::resize() {
	if (!surface.GetHandle()) {
		surface.Initialize(GetRenderDevice(), *BE::Application::Get()->GetSystemApplication(), *window);
	}

	Surface::SurfaceCapabilities surfaceCapabilities;
	auto isSupported = surface.IsSupported(&renderDevice, &surfaceCapabilities);

	renderArea = surfaceCapabilities.CurrentExtent;

	if (!isSupported) {
		BE::Application::Get()->Close(BE::Application::CloseMode::ERROR, GTSL::StaticString<64>(u8"No supported surface found!"));
	}

	auto supportedPresentModes = surface.GetSupportedPresentModes(&renderDevice);
	swapchainPresentMode = supportedPresentModes[0];

	auto supportedSurfaceFormats = surface.GetSupportedFormatsAndColorSpaces(&renderDevice);

	{
		GTSL::Pair<GAL::ColorSpaces, GAL::FormatDescriptor> bestColorSpaceFormat;

		for (uint8 topScore = 0; const auto & e : supportedSurfaceFormats) {
			uint8 score = 0;

			if (useHDR && e.First == GAL::ColorSpaces::HDR10_ST2048) {
				score += 5;
			}
			else {
				if (e.Second.ColorSpace == GAL::ColorSpaces::SRGB_NONLINEAR) {
					score += 2;
				}
				else {
					score += 3;
				}
			}

			if (score > topScore) {
				bestColorSpaceFormat = e;
				topScore = score;
			}
		}

		swapchainColorSpace = bestColorSpaceFormat.First; swapchainFormat = bestColorSpaceFormat.Second;
	}

	renderContext.InitializeOrRecreate(GetRenderDevice(), graphicsQueue, &surface, renderArea, swapchainFormat, swapchainColorSpace, GAL::TextureUses::STORAGE | GAL::TextureUses::TRANSFER_DESTINATION, swapchainPresentMode, pipelinedFrames);	

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	//imageIndex = 0; keep index of last acquired image

	{
		auto newSwapchainTextures = renderContext.GetTextures(GetRenderDevice());
		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			swapchainTextures[f] = newSwapchainTextures[f];
			swapchainTextureViews[f].Destroy(GetRenderDevice());

			GTSL::StaticString<64> name(u8"Swapchain ImageView "); name += f;

			swapchainTextureViews[f].Initialize(GetRenderDevice(), name, swapchainTextures[f], swapchainFormat, renderArea, 1);
		}
	}
}

RenderSystem::BufferHandle RenderSystem::CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, bool updateable, BufferHandle buffer_handle) {
	auto doBuffer = [&] {
		auto& buffer = buffers[buffer_handle()];

		if (buffer.Size < size) {
			auto frames = updateable ? GetPipelinedFrames() : 1;

			for (uint8 f = 0; f < frames; ++f) {
				if (buffer.Buffer[f].GetVkBuffer()) {
					buffer.Buffer[f].Destroy(GetRenderDevice());
					DeallocateLocalBufferMemory(buffer.Allocation[f]);
				}

				if (willWriteFromHost) {
					if (needsStagingBuffer) { //create staging buffer
						if (buffer.Staging[f].GetVkBuffer()) {
							buffer.Staging[f].Destroy(GetRenderDevice());
							DeallocateLocalBufferMemory(buffer.StagingAllocation[f]);
						}

						AllocateScratchBufferMemory(size, flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE, &buffer.Staging[f], &buffer.StagingAllocation[f]);
						buffer.StagingAddresses[f] = buffer.Staging[f].GetAddress(GetRenderDevice());

						flags |= GAL::BufferUses::TRANSFER_DESTINATION;
					}
				}

				AllocateLocalBufferMemory(size, flags | GAL::BufferUses::ADDRESS, &buffer.Buffer[f], &buffer.Allocation[f]);
				buffer.Addresses[f] = buffer.Buffer[f].GetAddress(GetRenderDevice());
			}			

			buffer.Size = size;
		}
	};

	if(buffer_handle) {
		doBuffer();
		return buffer_handle;
	}

	GTSL::Max(&size, 1024u); //force buffers to have a minimum size, so we always allocate and have valid data

	uint32 bufferIndex = buffers.Emplace(); auto& buffer = buffers[bufferIndex];

	buffer_handle = BufferHandle(bufferIndex);

	buffer.isMulti = updateable;
	buffer.Flags = flags;
	++buffer.references;

	doBuffer();

	return buffer_handle;
}

void RenderSystem::SetBufferWillWriteFromHost(BufferHandle bufferHandle, bool state)
{
	auto& buffer = buffers[bufferHandle()];

	if (buffer.isMulti) {
		__debugbreak();
	}
	
	if(state) {		
		if(!buffer.Staging[0].GetVkBuffer()) {//if will write from host and we have no buffer
			if (needsStagingBuffer) {
				AllocateScratchBufferMemory(buffer.Size, buffer.Flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE | GAL::BufferUses::STORAGE,
					&buffer.Staging[0], &buffer.StagingAllocation[0]);
			}
		}

		//if will write from host and we have buffer, do nothing
	} else {
		if (buffer.Staging[0].GetVkBuffer()) { //if won't write from host and we have a buffer
			if (needsStagingBuffer) {
				--buffer.references; //todo: what
			}
		}

		//if won't write from host and we have no buffer, do nothing
	}
}

void RenderSystem::printError(GTSL::StringView message, const RenderDevice::MessageSeverity messageSeverity) const {
	bool breakeablelogLevel = false;

	switch (messageSeverity) {
	//case RenderDevice::MessageSeverity::MESSAGE: BE_LOG_MESSAGE(message) break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message); break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); breakeablelogLevel = true; break;
	default: break;
	}

	if(breakOnError && breakeablelogLevel) {
		__debugbreak();
	}
}

void* RenderSystem::allocateApiMemory(void* data, const uint64 size, const uint64 alignment) {
	void* allocation; uint64 allocated_size;
	GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);

	{
		GTSL::Lock lock(allocationsMutex);		
		apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair(size, alignment));
	}

	return allocation;
}

void* RenderSystem::reallocateApiMemory(void* data, void* oldAllocation, uint64 size, uint64 alignment) {
	void* allocation; uint64 allocated_size;

	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		old_alloc = apiAllocations[reinterpret_cast<uint64>(oldAllocation)];
	}
	
	GetPersistentAllocator().Allocate(size, old_alloc.Second, &allocation, &allocated_size);
	apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair(size, alignment));
	
	GTSL::MemCopy(old_alloc.First, oldAllocation, allocation);
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
	
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
	}
	
	return allocation;
}

void RenderSystem::deallocateApiMemory(void* data, void* allocation) {
	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		old_alloc = apiAllocations[reinterpret_cast<uint64>(allocation)];
	}
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, allocation);
	
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.Remove(reinterpret_cast<uint64>(allocation));
	}
}

void RenderSystem::initializeFrameResources(const uint8 frame_index) {
	processedBufferCopies[frame_index] = 0;

	imageAvailableSemaphore[frame_index].Initialize(GetRenderDevice(), GAL::VulkanSynchronizer::Type::SEMAPHORE);

	fences[frame_index].Initialize(GetRenderDevice(), GAL::VulkanSynchronizer::Type::FENCE, true);
}

void RenderSystem::freeFrameResources(const uint8 frameIndex) {
	imageAvailableSemaphore[frameIndex].Destroy(&renderDevice);
	fences[frameIndex].Destroy(GetRenderDevice());
}
