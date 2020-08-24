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
	[[nodiscard]] uint8 GetFrameCount() const { return 2; }

	struct AllocateLocalTextureMemoryInfo
	{
		Texture Texture; RenderAllocation* Allocation;
	};
	void AllocateLocalTextureMemory(const AllocateLocalTextureMemoryInfo& allocationInfo)
	{
		DeviceMemory deviceMemory;
		
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetImageMemoryRequirements(&allocationInfo.Texture, memoryRequirements);
		
		localMemoryAllocator.AllocateTexture(renderDevice, &deviceMemory, allocationInfo.Allocation, GetPersistentAllocator());

		Texture::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = allocationInfo.Allocation->Offset;
		allocationInfo.Texture.BindToMemory(bindMemoryInfo);
	}

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
		void** Data = nullptr;
		RenderAllocation* Allocation = nullptr;
	};

	struct BufferLocalMemoryAllocationInfo
	{
		Buffer Buffer;
		RenderAllocation* Allocation;
	};
	
	void AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo)
	{
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetBufferMemoryRequirements(&allocationInfo.Buffer, memoryRequirements);
		
		DeviceMemory deviceMemory;
		
		scratchMemoryAllocator.AllocateBuffer(renderDevice,	&deviceMemory, memoryRequirements.Size, allocationInfo.Allocation, allocationInfo.Data, GetPersistentAllocator());

		Buffer::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = allocationInfo.Allocation->Offset;
		allocationInfo.Buffer.BindToMemory(bindMemoryInfo);
	}
	
	void DeallocateScratchBufferMemory(const RenderAllocation allocation)
	{
		scratchMemoryAllocator.DeallocateBuffer(renderDevice, allocation);
	}
	
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
	{
		RenderDevice::MemoryRequirements memoryRequirements;
		renderDevice.GetBufferMemoryRequirements(&memoryAllocationInfo.Buffer, memoryRequirements);

		DeviceMemory deviceMemory;
		
		localMemoryAllocator.AllocateBuffer(renderDevice, &deviceMemory, memoryAllocationInfo.Allocation, GetPersistentAllocator());

		Buffer::BindMemoryInfo bindMemoryInfo;
		bindMemoryInfo.RenderDevice = GetRenderDevice();
		bindMemoryInfo.Memory = &deviceMemory;
		bindMemoryInfo.Offset = memoryAllocationInfo.Allocation->Offset;
		memoryAllocationInfo.Buffer.BindToMemory(bindMemoryInfo);
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
		RenderAllocation Allocation;
	};
	void AddBufferCopy(const BufferCopyData& bufferCopyData) { bufferCopyDatas[currentFrameIndex].EmplaceBack(bufferCopyData); }

	struct TextureCopyData
	{
		Buffer SourceBuffer;
		Texture DestinationTexture;
		
		uint32 SourceOffset = 0;
		RenderAllocation Allocation;

		GTSL::Extent3D Extent;
		
		TextureLayout Layout;
	};
	void AddTextureCopy(const TextureCopyData& textureCopyData) { textureCopyDatas[GetCurrentFrame()].EmplaceBack(textureCopyData); }
	
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
	
	GTSL::Extent2D renderArea;
	
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> renderGroups;

	GTSL::Array<GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> bufferCopyDatas;
	GTSL::Array<GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> textureCopyDatas;

	RenderPass renderPass;
	GTSL::Array<TextureView, MAX_CONCURRENT_FRAMES> swapchainImages;
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
