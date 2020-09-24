#pragma once

#include <GTSL/Pair.h>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/GameInstance.h"

#include "RendererAllocator.h"
#include "RenderTypes.h"

namespace GTSL {
	class Window;
}

class RenderSystem : public System
{
public:
	RenderSystem() : System("RenderSystem") {}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	[[nodiscard]] uint8 GetCurrentFrame() const { return currentFrameIndex; }
	void Wait();

	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
		class PipelineCacheResourceManager* PipelineCacheResourceManager;
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	struct AllocateLocalTextureMemoryInfo
	{
		Texture Texture; RenderAllocation* Allocation;
	};
	void AllocateLocalTextureMemory(const AllocateLocalTextureMemoryInfo& allocationInfo)
	{
		BE_ASSERT(testMutex.TryLock())
		DeviceMemory deviceMemory;
		
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetImageMemoryRequirements(&allocationInfo.Texture, memoryRequirements);

		allocationInfo.Allocation->Size = memoryRequirements.Size;
		
		localMemoryAllocator.AllocateTexture(renderDevice, &deviceMemory, allocationInfo.Allocation, GetPersistentAllocator());

		Texture::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = allocationInfo.Allocation->Offset;
		allocationInfo.Texture.BindToMemory(bindMemoryInfo);
		testMutex.Unlock();
	}
	void DeallocateLocalTextureMemory(const RenderAllocation allocation)
	{
		localMemoryAllocator.DeallocateTexture(renderDevice, allocation);
	}

	void AllocateScratchAccelerationStructureMemory(const AccelerationStructure accelerationStructure, HostRenderAllocation* renderAllocation)
	{
		DeviceMemory deviceMemory;

		RenderDevice::MemoryRequirements memoryRequirements;

		{
			RenderDevice::GetAccelerationStructureMemoryRequirementsInfo requirements;
			requirements.AccelerationStructure = &accelerationStructure;
			requirements.MemoryRequirements = &memoryRequirements;
			requirements.AccelerationStructureBuildType = GAL::VulkanAccelerationStructureBuildType::GPU_LOCAL;
			requirements.AccelerationStructureMemoryRequirementsType = GAL::VulkanAccelerationStructureMemoryRequirementsType::BUILD_SCRATCH;
			renderDevice.GetAccelerationStructureMemoryRequirements(requirements);
		}

		scratchMemoryAllocator.AllocateBuffer(renderDevice, &deviceMemory, memoryRequirements.Size, renderAllocation, GetPersistentAllocator());

		AccelerationStructure::BindToMemoryInfo bindToMemoryInfo;
		bindToMemoryInfo.RenderDevice = &renderDevice;
		bindToMemoryInfo.Offset = renderAllocation->Offset;
		bindToMemoryInfo.Memory = deviceMemory;
		accelerationStructure.BindToMemory(bindToMemoryInfo);
	}
	
	struct BufferScratchMemoryAllocationInfo
	{
		Buffer Buffer;
		HostRenderAllocation* Allocation = nullptr;
	};

	struct BufferLocalMemoryAllocationInfo
	{
		Buffer Buffer;
		RenderAllocation* Allocation;
	};
	
	void AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo)
	{
		BE_ASSERT(testMutex.TryLock())
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetBufferMemoryRequirements(&allocationInfo.Buffer, memoryRequirements);
		
		DeviceMemory deviceMemory;
		
		scratchMemoryAllocator.AllocateBuffer(renderDevice,	&deviceMemory, memoryRequirements.Size, allocationInfo.Allocation, GetPersistentAllocator());

		Buffer::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = allocationInfo.Allocation->Offset;
		allocationInfo.Buffer.BindToMemory(bindMemoryInfo);
		testMutex.Unlock();
	}
	
	void DeallocateScratchBufferMemory(const HostRenderAllocation allocation)
	{
		scratchMemoryAllocator.DeallocateBuffer(renderDevice, allocation);
	}
	
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
	{
		BE_ASSERT(testMutex.TryLock())
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetBufferMemoryRequirements(&memoryAllocationInfo.Buffer, memoryRequirements);

		DeviceMemory deviceMemory;

		memoryAllocationInfo.Allocation->Size = memoryRequirements.Size;
		
		localMemoryAllocator.AllocateBuffer(renderDevice, &deviceMemory, memoryAllocationInfo.Allocation, GetPersistentAllocator());

		Buffer::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = memoryAllocationInfo.Allocation->Offset;
		memoryAllocationInfo.Buffer.BindToMemory(bindMemoryInfo);
		testMutex.Unlock();
	}

	void DeallocateLocalBufferMemory(const RenderAllocation renderAllocation)
	{
		localMemoryAllocator.DeallocateBuffer(renderDevice, renderAllocation);
	}
	
	RenderDevice* GetRenderDevice() { return &renderDevice; }
	const RenderDevice* GetRenderDevice() const { return &renderDevice; }
	CommandBuffer* GetTransferCommandBuffer() { return &transferCommandBuffers[currentFrameIndex]; }

	struct BufferCopyData
	{
		Buffer SourceBuffer, DestinationBuffer;
		/* Offset from start of buffer.
		 */
		uint32 SourceOffset = 0, DestinationOffset = 0;
		uint32 Size = 0;
		HostRenderAllocation Allocation;
	};
	void AddBufferCopy(const BufferCopyData& bufferCopyData) { bufferCopyDatas[currentFrameIndex].EmplaceBack(bufferCopyData); }

	struct TextureCopyData
	{
		Buffer SourceBuffer;
		Texture DestinationTexture;
		
		uint32 SourceOffset = 0;
		HostRenderAllocation Allocation;

		GTSL::Extent3D Extent;
		
		TextureLayout Layout;
	};
	void AddTextureCopy(const TextureCopyData& textureCopyData)
	{
		BE_ASSERT(testMutex.TryLock())
		textureCopyDatas[GetCurrentFrame()].EmplaceBack(textureCopyData);
		testMutex.Unlock();
	}
	
	const PipelineCache* GetPipelineCache() const;

	GTSL::Ranger<const Texture> GetSwapchainTextures() const { return swapchainTextures; }

	CommandBuffer* GetCurrentCommandBuffer() { return &graphicsCommandBuffers[currentFrameIndex]; }
	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void OnResize(TaskInfo taskInfo, GTSL::Extent2D extent);
	
private:
	GTSL::Mutex testMutex;
	
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;
	
	GTSL::Extent2D renderArea;

	GTSL::Array<GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> bufferCopyDatas;
	GTSL::Array<uint32, MAX_CONCURRENT_FRAMES> processedBufferCopies;
	GTSL::Array<GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> textureCopyDatas;
	GTSL::Array<uint32, MAX_CONCURRENT_FRAMES> processedTextureCopies;

	GTSL::Array<Texture, MAX_CONCURRENT_FRAMES> swapchainTextures;
	GTSL::Array<TextureView, MAX_CONCURRENT_FRAMES> swapchainTextureViews;
	
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> imageAvailableSemaphore;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> transferDoneSemaphores;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> renderFinishedSemaphore;
	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> graphicsFences;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> graphicsCommandBuffers;
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> graphicsCommandPools;
	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> transferFences;

	GTSL::Array<Vector<CommandBuffer::TextureBarrier>, MAX_CONCURRENT_FRAMES> textureBarriers;
	
	Queue graphicsQueue;
	Queue transferQueue;
	
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> transferCommandPools;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> transferCommandBuffers;

	uint8 currentFrameIndex = 0;

	PresentMode swapchainPresentMode;
	TextureFormat swapchainFormat;
	ColorSpace swapchainColorSpace;
	
	void renderBegin(TaskInfo taskInfo);
	void renderStart(TaskInfo taskInfo);
	void renderFinish(TaskInfo taskInfo);
	void frameStart(TaskInfo taskInfo);
	void executeTransfers(TaskInfo taskInfo);

	void printError(const char* message, RenderDevice::MessageSeverity messageSeverity) const;
	void* allocateApiMemory(void* data, uint64 size, uint64 alignment);
	void* reallocateApiMemory(void* data, void* allocation, uint64 size, uint64 alignment);
	void deallocateApiMemory(void* data, void* allocation);

	GTSL::FlatHashMap<GTSL::Pair<uint64, uint64>, BE::PersistentAllocatorReference> apiAllocations;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;

	Vector<PipelineCache> pipelineCaches;
};
