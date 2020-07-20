#pragma once
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/GameInstance.h"

#include <GTSL/DataSizes.h>

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
	
private:
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;
	
	GTSL::Extent2D renderArea;
	
	GTSL::Vector<GTSL::Id64, BE::PersistentAllocatorReference> renderGroups;

	RenderPass renderPass;
	GTSL::Array<ImageView, 3> swapchainImages;
	GTSL::Array<Semaphore, 3> imageAvailableSemaphore;
	GTSL::Array<Semaphore, 3> renderFinishedSemaphore;
	GTSL::Array<Fence, 3> inFlightFences;
	
	GTSL::Array<CommandBuffer, 3> commandBuffers;
	GTSL::Array<CommandPool, 3> commandPools;
	
	GTSL::Array<CommandPool, 3> transferCommandPools;
	GTSL::Array<CommandBuffer, 3> transferCommandBuffers;
	
	GTSL::Array<FrameBuffer, 3> frameBuffers;

	GTSL::Array<GTSL::RGBA, 3> clearValues;

	Queue graphicsQueue;

	uint8 index = 0;

	uint32 swapchainPresentMode{ 0 };
	uint32 swapchainFormat{ 0 };
	uint32 swapchainColorSpace{ 0 };
	
	void render(const GameInstance::TaskInfo& taskInfo);

	void printError(const char* message, RenderDevice::MessageSeverity messageSeverity) const;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;
};
