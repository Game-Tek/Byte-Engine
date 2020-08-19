#pragma once

#include <GTSL/Buffer.h>
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
	[[nodiscard]] uint8 GetFrameCount() const { return 2; }

	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
		class PipelineCacheResourceManager* PipelineCacheResourceManager;
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	void UpdateWindow(GTSL::Window& window);

	struct BufferScratchMemoryAllocationInfo
	{
		Buffer Buffer;

		DeviceMemory* DeviceMemory = nullptr;
		void** Data = nullptr;
		
		RenderAllocation* Allocation = nullptr;
	};

	struct BufferLocalMemoryAllocationInfo
	{
		DeviceMemory* DeviceMemory = nullptr;
		
		uint32 Size = 0;
		uint32* Offset = nullptr;
		AllocationId* AllocationId = nullptr;
	};
	void AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo)
	{
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetBufferMemoryRequirements(&allocationInfo.Buffer, memoryRequirements);
		
		scratchMemoryAllocator.AllocateBuffer(renderDevice,
			allocationInfo.DeviceMemory,
			memoryRequirements.Size,
			allocationInfo.Allocation,
			allocationInfo.Data,
			GetPersistentAllocator());
	}
	
	void DeallocateScratchBufferMemory(const RenderAllocation allocation)
	{
		scratchMemoryAllocator.DeallocateBuffer(renderDevice, allocation.Size, allocation.Offset, allocation.AllocationId);
	}
	
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
	{
		localMemoryAllocator.AllocateBuffer(renderDevice,
			memoryAllocationInfo.DeviceMemory,
			memoryAllocationInfo.Size,
			memoryAllocationInfo.Offset,
			memoryAllocationInfo.AllocationId,
			GetPersistentAllocator());
	}

	void DeallocateLocalBufferMemory(const uint32 size, const uint32 offset, const AllocationId allocId)
	{
		localMemoryAllocator.DeallocateBuffer(renderDevice, size, offset, allocId);
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
		RenderAllocation Allocation;
	};
	void AddBufferCopy(const BufferCopyData& bufferCopyData) { bufferCopyDatas[currentFrameIndex].EmplaceBack(bufferCopyData); }

	PipelineCache* GetPipelineCache() { return &pipelineCache; }

	RenderPass* GetRenderPass() { return &renderPass; }

	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void OnResize(TaskInfo taskInfo, GTSL::Extent2D extent);
	
private:
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;

	PipelineCache pipelineCache;
	GTSL::Buffer pipelineCacheBuffer;
	
	GTSL::Extent2D renderArea;
	
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> renderGroups;

	GTSL::Array<GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> bufferCopyDatas;

	RenderPass renderPass;
	GTSL::Array<ImageView, MAX_CONCURRENT_FRAMES> swapchainImages;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> imageAvailableSemaphore;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> renderFinishedSemaphore;
	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> graphicsFences;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> graphicsCommandBuffers;
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> graphicsCommandPools;
	GTSL::Array<FrameBuffer, MAX_CONCURRENT_FRAMES> frameBuffers;
	GTSL::Array<GTSL::RGBA, MAX_CONCURRENT_FRAMES> clearValues;
	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> transferFences;
	
	Queue graphicsQueue;
	Queue transferQueue;
	
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> transferCommandPools;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> transferCommandBuffers;

	uint8 currentFrameIndex = 0;

	uint32 swapchainPresentMode{ 0 };
	uint32 swapchainFormat{ 0 };
	uint32 swapchainColorSpace{ 0 };
	
	void render(TaskInfo taskInfo);
	void frameStart(TaskInfo taskInfo);
	void executeTransfers(TaskInfo taskInfo);

	void printError(const char* message, RenderDevice::MessageSeverity messageSeverity) const;
	void* allocateApiMemory(void* data, uint64 size, uint64 alignment);
	void* reallocateApiMemory(void* data, void* allocation, uint64 size, uint64 alignment);
	void deallocateApiMemory(void* data, void* allocation);

	GTSL::FlatHashMap<GTSL::Pair<uint64, uint64>, BE::PersistentAllocatorReference> apiAllocations;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;
};
