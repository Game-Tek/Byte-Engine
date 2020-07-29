#pragma once

#include <GTSL/Buffer.h>

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
	void Shutdown() override;
	
	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	void UpdateWindow(GTSL::Window& window);

	struct BufferScratchMemoryAllocationInfo
	{
		DeviceMemory* DeviceMemory = nullptr;
		uint32 Size = 0;
		uint32* Offset = nullptr;
		void** Data = nullptr;
	};

	struct BufferLocalMemoryAllocationInfo
	{
		DeviceMemory* DeviceMemory = nullptr;
		uint32 Size = 0;
		uint32* Offset = nullptr;
	};
	void AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo);
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo);
	
	RenderDevice* GetRenderDevice() { return &renderDevice; }
	CommandBuffer* GetTransferCommandBuffer() { return &transferCommandBuffers[index]; }

	struct BufferCopyData
	{
		Buffer SourceBuffer, DestinationBuffer;
		/* Offset from start of buffer.
		 */
		uint32 SourceOffset = 0, DestinationOffset = 0;
		uint32 Size = 0;
	};
	void AddBufferCopy(const BufferCopyData& bufferCopyData) { bufferCopyDatas.EmplaceBack(bufferCopyData); }

	PipelineCache* GetPipelineCache() { return &pipelineCache; }
	
private:
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;

	PipelineCache pipelineCache;
	GTSL::Buffer pipelineCacheBuffer;
	
	GTSL::Extent2D renderArea;
	
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> renderGroups;

	GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference> bufferCopyDatas;

	RenderPass renderPass;
	GTSL::Array<ImageView, MAX_CONCURRENT_FRAMES> swapchainImages;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> imageAvailableSemaphore;
	GTSL::Array<Semaphore, MAX_CONCURRENT_FRAMES> renderFinishedSemaphore;
	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> inFlightFences;
	
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> commandBuffers;
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> commandPools;
	
	GTSL::Array<FrameBuffer, MAX_CONCURRENT_FRAMES> frameBuffers;

	GTSL::Array<GTSL::RGBA, MAX_CONCURRENT_FRAMES> clearValues;

	GTSL::Array<Fence, MAX_CONCURRENT_FRAMES> transferFences;

	GTSL::Array<GTSL::Pair<uint32, uint32>, MAX_CONCURRENT_FRAMES> transferredMeshes;
	
	Queue graphicsQueue;
	Queue transferQueue;
	
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> transferCommandPools;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> transferCommandBuffers;

	uint8 index = 0;

	uint32 swapchainPresentMode{ 0 };
	uint32 swapchainFormat{ 0 };
	uint32 swapchainColorSpace{ 0 };
	
	void render(TaskInfo taskInfo);

	void frameStart(TaskInfo taskInfo);
	void executeTransfers(TaskInfo taskInfo);

	void printError(const char* message, RenderDevice::MessageSeverity messageSeverity) const;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;
};
