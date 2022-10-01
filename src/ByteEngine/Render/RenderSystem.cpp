#include "RenderSystem.h"

#include <GTSL/Window.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/WindowSystem.hpp"
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
	accelerationStructures(16, GetPersistentAllocator()), buffers(32, GetPersistentAllocator()),
	pipelineCaches(16, decltype(pipelineCaches)::allocator_t()),
	textures(16, GetPersistentAllocator()), apiAllocations(128, GetPersistentAllocator()), workloads(16, GetPersistentAllocator())
{
	{
		initializeInfo.ApplicationManager->EnqueueScheduledTask(initializeInfo.ApplicationManager->RegisterTask(this, u8"endCommandLists", DependencyBlock(), &RenderSystem::renderFlush, u8"FrameEnd", u8"FrameEnd"));
		resizeHandle = initializeInfo.ApplicationManager->RegisterTask(this, u8"onResize", {}, & RenderSystem::onResize);
	}

	RenderDevice::RayTracingCapabilities rayTracingCapabilities;

	auto config = BE::Application::Get()->GetConfig()[u8"Rendering"];

	useHDR = config[u8"hdr"].GetBool();
	pipelinedFrames = static_cast<uint8>(GTSL::Math::Clamp(static_cast<uint32>(config[u8"buffer"].GetUint()), 2u, 3u));
	bool rayTracing = config[u8"rayTracing"].GetBool();

	{
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>(BE::Application::Get()->GetApplicationName());
		createInfo.ApplicationVersion[0] = 0; createInfo.ApplicationVersion[1] = 0; createInfo.ApplicationVersion[2] = 0;

		createInfo.Debug = static_cast<bool>(BE::Application::Get()->GetUINTOption(u8"debug"));

		GTSL::StaticVector<GAL::QueueType, 5> queue_create_infos;
		GTSL::StaticVector<RenderDevice::QueueKey, 5> queueKeys;

		queue_create_infos.EmplaceBack(GAL::QueueTypes::GRAPHICS); queueKeys.EmplaceBack();
		queue_create_infos.EmplaceBack(GAL::QueueTypes::COMPUTE); queueKeys.EmplaceBack();
		queue_create_infos.EmplaceBack(GAL::QueueTypes::TRANSFER); queueKeys.EmplaceBack();

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

		graphicsQueue.Initialize(GetRenderDevice(), queueKeys[0]); computeQueue.Initialize(GetRenderDevice(), queueKeys[1]); transferQueue.Initialize(GetRenderDevice(), queueKeys[2]);

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

void RenderSystem::renderFlush(TaskInfo taskInfo) {
	auto beforeFrame = uint8(currentFrameIndex - uint8(1)) % GetPipelinedFrames();

	++currentFrameIndex %= pipelinedFrames;
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

void RenderSystem::UpdateTexture(const CommandListHandle command_list_handle, const TextureHandle textureHandle)
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
	AddTextureCopy(command_list_handle, textureCopyData);
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

GTSL::Result<GTSL::Extent2D> RenderSystem::AcquireImage(const RenderContextHandle render_context_handle, const WorkloadHandle workload_handle, WindowSystem* window_system)
{
	bool result = false;

	auto& renderContext = renderx[render_context_handle()];

	if(!renderContext.renderContext.GetHandle()) {
		resize(window_system,render_context_handle); result = true;
	}

	const auto acquireResult = renderContext.renderContext.AcquireNextImage(&renderDevice, &workloads[workload_handle()].Semaphore);

	renderContext.imageIndex = acquireResult.Get();

	switch (acquireResult.State()) {
	case GAL::VulkanRenderContext::AcquireState::OK: break;
	case GAL::VulkanRenderContext::AcquireState::SUBOPTIMAL:
	case GAL::VulkanRenderContext::AcquireState::BAD: resize(window_system,render_context_handle); result = true; break;
	}

	if (renderContext.lastRenderArea != renderContext.renderArea) { renderContext.lastRenderArea = renderContext.renderArea; result = true; }
	
	return { GTSL::MoveRef(renderContext.renderArea), result };
}

void RenderSystem::resize(WindowSystem* window_system, const RenderContextHandle render_context_handle) {
	auto& renderContext = renderx[render_context_handle()];

	if (!renderContext.surface.GetHandle()) {
		renderContext.surface.Initialize(GetRenderDevice(), *BE::Application::Get()->GetSystemApplication(), window_system->GetWindow(renderContext.windowHandle));
	}

	Surface::SurfaceCapabilities surfaceCapabilities;
	auto isSupported = renderContext.surface.IsSupported(&renderDevice, &surfaceCapabilities);

	renderContext.renderArea = surfaceCapabilities.CurrentExtent;

	if (!isSupported) {
		BE::Application::Get()->Close(BE::Application::CloseMode::ERROR, GTSL::StaticString<64>(u8"No supported surface found!"));
	}

	auto supportedPresentModes = renderContext.surface.GetSupportedPresentModes(&renderDevice);
	swapchainPresentMode = supportedPresentModes[0];

	auto supportedSurfaceFormats = renderContext.surface.GetSupportedFormatsAndColorSpaces(&renderDevice);

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

	renderContext.renderContext.InitializeOrRecreate(GetRenderDevice(), graphicsQueue, &renderContext.surface, renderContext.renderArea, swapchainFormat, swapchainColorSpace, GAL::TextureUses::STORAGE | GAL::TextureUses::TRANSFER_DESTINATION, swapchainPresentMode, pipelinedFrames);	

	for (auto& e : renderContext.swapchainTextureViews) { e.Destroy(&renderDevice); }

	//imageIndex = 0; keep index of last acquired image

	{
		auto newSwapchainTextures = renderContext.renderContext.GetTextures(GetRenderDevice());
		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			renderContext.swapchainTextures[f] = newSwapchainTextures[f];
			renderContext.swapchainTextureViews[f].Destroy(GetRenderDevice());

			GTSL::StaticString<64> name(u8"Swapchain ImageView "); name += f;

			renderContext.swapchainTextureViews[f].Initialize(GetRenderDevice(), name, renderContext.swapchainTextures[f], swapchainFormat, renderContext.renderArea, 1);
		}
	}
}

RenderSystem::BufferHandle RenderSystem::CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, BufferHandle buffer_handle) {
	auto doBuffer = [&] {
		auto& buffer = buffers[buffer_handle()];

		if (size > buffer.Size) {
			if (buffer.Buffer.GetVkBuffer()) {
				buffer.Buffer.Destroy(GetRenderDevice());

				if (willWriteFromHost && needsStagingBuffer) {
					DeallocateLocalBufferMemory(buffer.Allocation);
				} else {
					DeallocateLocalBufferMemory(buffer.Allocation);
				}
			}

			if (willWriteFromHost && needsStagingBuffer) {
				flags |= GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE;
				AllocateScratchBufferMemory(size, flags, &buffer.Buffer, &buffer.Allocation);
			} else {
				flags |= GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_DESTINATION;
				AllocateLocalBufferMemory(size, flags, &buffer.Buffer, &buffer.Allocation);
			}

			buffer.Addresses = buffer.Buffer.GetAddress(GetRenderDevice());

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
	
	buffer.Flags = flags;
	++buffer.references;

	doBuffer();

	return buffer_handle;
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
