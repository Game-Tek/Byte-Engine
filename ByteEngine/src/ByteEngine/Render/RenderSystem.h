#pragma once

#include <unordered_map>
#include <GTSL/Pair.h>
#include <GTSL/FunctionPointer.hpp>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/GameInstance.h"

#include "RendererAllocator.h"
#include "RenderTypes.h"

#include "ByteEngine/Handle.hpp"

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
		Texture* Texture;
		Texture::CreateInfo* CreateInfo;
		RenderAllocation* Allocation;
	};
	void AllocateLocalTextureMemory(AllocateLocalTextureMemoryInfo& allocationInfo)
	{		
		Texture::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = allocationInfo.CreateInfo;
		allocationInfo.Texture->GetMemoryRequirements(&memoryRequirements);
		
		allocationInfo.Allocation->Size = memoryRequirements.MemoryRequirements.Size;
		
		testMutex.Lock();
		localMemoryAllocator.AllocateTexture(renderDevice, &allocationInfo.CreateInfo->Memory, allocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		allocationInfo.CreateInfo->Offset = allocationInfo.Allocation->Offset;
		
		allocationInfo.Texture->Initialize(*allocationInfo.CreateInfo);
	}
	void DeallocateLocalTextureMemory(const RenderAllocation allocation)
	{
		localMemoryAllocator.DeallocateTexture(renderDevice, allocation);
	}

	void AllocateAccelerationStructureMemory(AccelerationStructure* accelerationStructure, Buffer* buffer, GTSL::Range<const AccelerationStructure::GeometryDescriptor*> geometryDescriptors, AccelerationStructure::CreateInfo* createInfo, RenderAllocation* renderAllocation, BuildType build)
	{
		uint32 bufferSize, scratchSize;
		
		AccelerationStructure::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.BuildType = build;
		memoryRequirements.Flags = 0;
		memoryRequirements.GeometryDescriptors = geometryDescriptors;
		memoryRequirements.IsTopLevel = geometryDescriptors[0].Type == GeometryType::INSTANCES ? true : false;
		accelerationStructure->GetMemoryRequirements(memoryRequirements, &bufferSize, &scratchSize);

		bufferSize *= 2;
		
		renderAllocation->Size = bufferSize;
		
		Buffer::CreateInfo bufferCreateInfo;
		bufferCreateInfo.RenderDevice = GetRenderDevice();
		bufferCreateInfo.BufferType = BufferType::ACCELERATION_STRUCTURE;
		bufferCreateInfo.Size = bufferSize;

		testMutex.Lock();
		localMemoryAllocator.AllocateBuffer(renderDevice, &bufferCreateInfo.Memory, renderAllocation, GetPersistentAllocator());
		testMutex.Unlock();

		Buffer::GetMemoryRequirementsInfo bufferMemoryRequirements;
		bufferMemoryRequirements.RenderDevice = GetRenderDevice();
		bufferMemoryRequirements.CreateInfo = &bufferCreateInfo;
		buffer->GetMemoryRequirements(&bufferMemoryRequirements);

		bufferCreateInfo.Offset = renderAllocation->Offset;
		buffer->Initialize(bufferCreateInfo);

		createInfo->Buffer = *buffer;

		createInfo->Offset = 0;
		createInfo->Size = bufferSize;
		accelerationStructure->Initialize(*createInfo);
	}
	
	struct BufferScratchMemoryAllocationInfo
	{
		Buffer* Buffer;
		Buffer::CreateInfo* CreateInfo;
		HostRenderAllocation* Allocation = nullptr;
	};

	struct BufferLocalMemoryAllocationInfo
	{
		Buffer* Buffer;
		Buffer::CreateInfo* CreateInfo;
		RenderAllocation* Allocation;
	};
	
	void AllocateScratchBufferMemory(BufferScratchMemoryAllocationInfo& allocationInfo)
	{
		Buffer::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = allocationInfo.CreateInfo;
		allocationInfo.Buffer->GetMemoryRequirements(&memoryRequirements);
		
		allocationInfo.Allocation->Size = memoryRequirements.MemoryRequirements.Size;
		
		testMutex.Lock();
		scratchMemoryAllocator.AllocateBuffer(renderDevice,	&allocationInfo.CreateInfo->Memory, allocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		allocationInfo.CreateInfo->Offset = allocationInfo.Allocation->Offset;
		
		allocationInfo.Buffer->Initialize(*allocationInfo.CreateInfo);
	}
	
	void DeallocateScratchBufferMemory(const HostRenderAllocation allocation)
	{
		scratchMemoryAllocator.DeallocateBuffer(renderDevice, allocation);
	}
	
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
	{
		Buffer::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = memoryAllocationInfo.CreateInfo;
		memoryAllocationInfo.Buffer->GetMemoryRequirements(&memoryRequirements);

		memoryAllocationInfo.Allocation->Size = memoryRequirements.MemoryRequirements.Size;

		testMutex.Lock();
		localMemoryAllocator.AllocateBuffer(renderDevice, &memoryAllocationInfo.CreateInfo->Memory, memoryAllocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		memoryAllocationInfo.CreateInfo->Offset = memoryAllocationInfo.Allocation->Offset;
		
		memoryAllocationInfo.Buffer->Initialize(*memoryAllocationInfo.CreateInfo);
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

	[[nodiscard]] const PipelineCache* GetPipelineCache() const;

	[[nodiscard]] GTSL::Range<const Texture*> GetSwapchainTextures() const { return swapchainTextures; }

	MAKE_HANDLE(uint32, SharedMesh)
	MAKE_HANDLE(uint32, GPUMesh)
	
	struct CreateRayTracingMeshInfo
	{
		uint32 VertexCount, VertexSize;
		uint32 IndexCount, IndexSize;
		GTSL::Matrix3x4* Matrix;
		SharedMeshHandle SharedMesh;
	};
	ComponentReference CreateRayTracedMesh(const CreateRayTracingMeshInfo& info);
	
	SharedMeshHandle CreateSharedMesh(Id name, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize);
	GPUMeshHandle CreateGPUMesh(SharedMeshHandle sharedMeshHandle);
	
	void RenderMesh(GPUMeshHandle handle, uint32 instanceCount = 1);

	byte* GetSharedMeshPointer(SharedMeshHandle sharedMesh) { return static_cast<byte*>(sharedMeshes[static_cast<uint32>(sharedMesh)].Allocation.Data); }
	
	void RenderAllMeshesForMaterial(Id material);

	void AddMeshToId(GPUMeshHandle mesh, Id material)
	{
		if(meshesByMaterial.Find(material())) //TODO: ADD MATERIALS DON'T QUERY FOR EACH MESH
		{
			meshesByMaterial.At(material()).EmplaceBack(mesh());
		}
		else
		{
			auto& e = meshesByMaterial.Emplace(material());
			e.Initialize(8, GetPersistentAllocator());
			e.EmplaceBack(mesh());
		}
	}
	
	CommandBuffer* GetCurrentCommandBuffer() { return &graphicsCommandBuffers[currentFrameIndex]; }
	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void OnResize(GTSL::Extent2D extent);

	uint32 GetShaderGroupHandleSize() const { return shaderGroupHandleSize; }
	uint32 GetShaderGroupAlignment() const { return shaderGroupAlignment; }

	AccelerationStructure GetTopLevelAccelerationStructure() const { return topLevelAccelerationStructure; }
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
	
	Queue graphicsQueue;
	Queue transferQueue;
	
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> transferCommandPools;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> transferCommandBuffers;

	struct Mesh
	{
		Buffer Buffer;
		uint32 IndicesCount;
		IndexType IndexType;	
		uint32 OffsetToIndices;
	};
	
	struct SharedMesh : Mesh
	{
		uint32 Size = 0;
		HostRenderAllocation Allocation;
	};

	struct GPUMesh : Mesh
	{
		RenderAllocation Allocation;
	};
	
	struct RayTracingMesh
	{
		Buffer MeshBuffer, StructureBuffer;
		uint32 IndicesCount;
		IndexType IndexType;

		AccelerationStructure AccelerationStructure;
		RenderAllocation MeshBufferAllocation, StructureBufferAllocation;
	};
	
	GTSL::KeepVector<SharedMesh, BE::PersistentAllocatorReference> sharedMeshes;
	GTSL::KeepVector<GPUMesh, BE::PersistentAllocatorReference> gpuMeshes;
	
	GTSL::KeepVector<RayTracingMesh, BE::PersistentAllocatorReference> rayTracingMeshes;

	GTSL::Vector<GAL::BuildAccelerationStructureInfo, BE::PersistentAllocatorReference> buildAccelerationStructureInfos;
	GTSL::Vector<GAL::BuildRange, BE::PersistentAllocatorReference> buildRanges;
	GTSL::Vector<AccelerationStructure::GeometryTriangles, BE::PersistentAllocatorReference> geometries;

	//RenderAllocation scratchBufferAllocation;
	HostRenderAllocation scratchBufferAllocation;
	Buffer accelerationStructureScratchBuffer;
	uint64 scratchBufferAddress;

	AccelerationStructure topLevelAccelerationStructure;
	HostRenderAllocation topLevelAccelerationStructureAllocation;
	Buffer topLevelAccelerationStructureBuffer;
	uint64 topLevelAccelerationStructureAddress;

	static constexpr uint8 MAX_INSTANCES_COUNT = 16;
	HostRenderAllocation instancesAllocation;
	uint64 instancesBufferAddress;
	Buffer instancesBuffer;

	uint32 instanceCount = 0;
	
	/**
	 * \brief Pointer to the implementation for acceleration structures build.
	 * Since acc. structures can be built on the host or on the device depending on device capabilities
	 * we determine which one we are able to do and cache it.
	 */
	GTSL::FunctionPointer<void()> buildAccelerationStructures;

	void buildAccelerationStructuresOnDevice();
	
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

	//GTSL::FlatHashMap<GTSL::Pair<uint64, uint64>, BE::PersistentAllocatorReference> apiAllocations;
	std::unordered_map<uint64, GTSL::Pair<uint64, uint64>> apiAllocations;
	GTSL::Mutex allocationsMutex;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;

	Vector<PipelineCache> pipelineCaches;

	GTSL::FlatHashMap<GTSL::Vector<uint32, BE::PAR>, BE::PAR> meshesByMaterial;

	uint32 shaderGroupAlignment = 0, shaderGroupHandleSize = 0;
};
