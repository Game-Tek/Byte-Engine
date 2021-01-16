#pragma once

#include <unordered_map>
#include <GTSL/Atomic.hpp>
#include <GTSL/Pair.h>
#include <GTSL/FunctionPointer.hpp>


#include "MaterialSystem.h"
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
		localMemoryAllocator.AllocateNonLinearMemory(renderDevice, &allocationInfo.CreateInfo->Memory, allocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		allocationInfo.CreateInfo->Offset = allocationInfo.Allocation->Offset;
		
		allocationInfo.Texture->Initialize(*allocationInfo.CreateInfo);
	}
	void DeallocateLocalTextureMemory(const RenderAllocation allocation)
	{
		localMemoryAllocator.DeallocateNonLinearMemory(renderDevice, allocation);
	}

	void AllocateAccelerationStructureMemory(AccelerationStructure* accelerationStructure, Buffer* buffer, GTSL::Range<const AccelerationStructure::Geometry*> geometries, AccelerationStructure::CreateInfo* createInfo, RenderAllocation* renderAllocation, BuildType build)
	{
		uint32 bufferSize, scratchSize;
		
		AccelerationStructure::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.BuildType = build;
		memoryRequirements.Flags = 0;
		memoryRequirements.Geometries = geometries;
		accelerationStructure->GetMemoryRequirements(memoryRequirements, &bufferSize, &scratchSize);
		
		renderAllocation->Size = bufferSize;
		
		Buffer::CreateInfo bufferCreateInfo;
		bufferCreateInfo.RenderDevice = GetRenderDevice();
		bufferCreateInfo.BufferType = BufferType::ACCELERATION_STRUCTURE;
		bufferCreateInfo.Size = bufferSize;

		testMutex.Lock();
		localMemoryAllocator.AllocateNonLinearMemory(renderDevice, &bufferCreateInfo.Memory, renderAllocation, GetPersistentAllocator());
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
		RenderAllocation* Allocation = nullptr;
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
		scratchMemoryAllocator.AllocateLinearMemory(renderDevice,	&allocationInfo.CreateInfo->Memory, allocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		allocationInfo.CreateInfo->Offset = allocationInfo.Allocation->Offset;
		
		allocationInfo.Buffer->Initialize(*allocationInfo.CreateInfo);
	}
	
	void DeallocateScratchBufferMemory(const RenderAllocation allocation)
	{
		scratchMemoryAllocator.DeallocateLinearMemory(renderDevice, allocation);
	}
	
	void AllocateLocalBufferMemory(BufferLocalMemoryAllocationInfo& memoryAllocationInfo)
	{
		Buffer::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = memoryAllocationInfo.CreateInfo;
		memoryAllocationInfo.Buffer->GetMemoryRequirements(&memoryRequirements);

		memoryAllocationInfo.Allocation->Size = memoryRequirements.MemoryRequirements.Size;

		testMutex.Lock();
		localMemoryAllocator.AllocateLinearMemory(renderDevice, &memoryAllocationInfo.CreateInfo->Memory, memoryAllocationInfo.Allocation, GetPersistentAllocator());
		testMutex.Unlock();

		memoryAllocationInfo.CreateInfo->Offset = memoryAllocationInfo.Allocation->Offset;
		
		memoryAllocationInfo.Buffer->Initialize(*memoryAllocationInfo.CreateInfo);
	}

	void DeallocateLocalBufferMemory(const RenderAllocation renderAllocation)
	{
		localMemoryAllocator.DeallocateLinearMemory(renderDevice, renderAllocation);
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
	void AddTextureCopy(const TextureCopyData& textureCopyData)
	{
		BE_ASSERT(testMutex.TryLock())
		textureCopyDatas[GetCurrentFrame()].EmplaceBack(textureCopyData);
		testMutex.Unlock();
	}

	[[nodiscard]] const PipelineCache* GetPipelineCache() const;

	[[nodiscard]] GTSL::Range<const Texture*> GetSwapchainTextures() const { return swapchainTextures; }

	MAKE_HANDLE(uint32, Mesh)
	
	struct CreateRayTracingMeshInfo
	{
		uint32 VertexCount, VertexSize;
		uint32 IndexCount, IndexSize;
		GTSL::Matrix3x4* Matrix;
		MeshHandle SharedMesh;
	};
	MeshHandle CreateRayTracedMesh(const CreateRayTracingMeshInfo& info);
	
	MeshHandle CreateSharedMesh(Id name, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize);
	MeshHandle CreateGPUMesh(MeshHandle sharedMeshHandle);

	void UpdateMesh(MeshHandle meshHandle);
	
	void RenderMesh(MeshHandle handle, uint32 instanceCount = 1);

	byte* GetMeshPointer(MeshHandle sharedMesh) const
	{
		const auto& mesh = meshes[sharedMesh()];
		BE_ASSERT(mesh.MeshAllocation.Data, "This mesh has no CPU accessible data!");
		return static_cast<byte*>(mesh.MeshAllocation.Data);
	}

	uint32 GetMeshSize(MeshHandle meshHandle) const
	{
		const auto& mesh = meshes[meshHandle()];
		return GTSL::Math::RoundUpByPowerOf2(mesh.VertexSize * mesh.VertexCount, GetBufferSubDataAlignment()) + mesh.IndexSize * mesh.IndicesCount;
	}
	
	void RenderAllMeshesForMaterial(Id material, MaterialSystem* materialSystem);

	void AddMeshToMaterial(MeshHandle meshHandle, MaterialHandle materialHandle)
	{
		auto& mesh = meshes[meshHandle()]; mesh.MaterialHandle = materialHandle;
		
		if(meshesByMaterial.Find(materialHandle.MaterialType())) //TODO: ADD MATERIALS DON'T QUERY FOR EACH MESH
		{
			meshesByMaterial.At(materialHandle.MaterialType()).EmplaceBack(meshHandle());
		}
		else
		{
			auto& e = meshesByMaterial.Emplace(materialHandle.MaterialType());
			e.Initialize(8, GetPersistentAllocator());
			e.EmplaceBack(meshHandle());
		}
	}
	
	CommandBuffer* GetCurrentCommandBuffer() { return &graphicsCommandBuffers[currentFrameIndex]; }
	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void OnResize(GTSL::Extent2D extent);

	uint32 GetShaderGroupHandleSize() const { return shaderGroupHandleSize; }
	uint32 GetShaderGroupBaseAlignment() const { return shaderGroupBaseAlignment; }
	uint32 GetShaderGroupAlignment() const { return shaderGroupAlignment; }

	AccelerationStructure GetTopLevelAccelerationStructure() const { return topLevelAccelerationStructure; }

	MaterialHandle GetMeshMaterialHandle(const uint32 mesh) const { return meshes[mesh].MaterialHandle; }
	Buffer GetMeshVertexBuffer(const uint32 mesh) const { return meshes[mesh].Buffer; }
	uint32 GetMeshVertexBufferSize(const uint32 mesh) const { return meshes[mesh].VertexSize * meshes[mesh].VertexCount; }
	uint32 GetMeshVertexBufferOffset(const uint32 mesh) const { return 0; }
	Buffer GetMeshIndexBuffer(const uint32 mesh) const { return meshes[mesh].Buffer; }
	uint32 GetMeshIndexBufferSize(const uint32 mesh) const { return meshes[mesh].IndexSize * meshes[mesh].IndicesCount; }
	uint32 GetMeshIndexBufferOffset(const uint32 mesh) const { return GTSL::Math::RoundUpByPowerOf2(meshes[mesh].VertexSize * meshes[mesh].VertexCount, GetBufferSubDataAlignment()); }

	auto GetAddedMeshes() const { return addedMeshes.GetRange(); }
	void ClearAddedMeshes() { return addedMeshes.ResizeDown(0); }

	uint32 GetBufferSubDataAlignment() const { return renderDevice.GetStorageBufferBindingOffsetAlignment(); }
private:	
	GTSL::Mutex testMutex;

	bool needsStagingBuffer = true;
	
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

	/**
	 * \brief Keeps track of created instances. Mesh / Material combo.
	 */
	uint32 rayTracingInstancesCount = 0;
	
	GTSL::Vector<uint32, BE::PAR> addedMeshes;
	
	Queue graphicsQueue;
	Queue transferQueue;
	
	GTSL::Array<CommandPool, MAX_CONCURRENT_FRAMES> transferCommandPools;
	GTSL::Array<CommandBuffer, MAX_CONCURRENT_FRAMES> transferCommandBuffers;

	struct Mesh
	{
		Buffer Buffer;

		uint8 IndexSize;
		uint32 IndicesCount;
		uint8 VertexSize;
		uint32 VertexCount;
		RenderAllocation MeshAllocation;
		MaterialHandle MaterialHandle;
		
		uint32 DerivedTypeIndex;
	};
	
	struct RayTracingMesh
	{
		Buffer StructureBuffer;
		RenderAllocation StructureBufferAllocation;
		AccelerationStructure AccelerationStructure;
	};
	
	GTSL::KeepVector<Mesh, BE::PersistentAllocatorReference> meshes;
	GTSL::KeepVector<RayTracingMesh, BE::PersistentAllocatorReference> rayTracingMeshes;

	MeshHandle addMesh(Mesh mesh)
	{
		MeshHandle meshHandle{ meshes.Emplace(mesh) }; addedMeshes.EmplaceBack(meshHandle());
		return meshHandle;
	}
	
	struct AccelerationStructureBuildData
	{
		uint32 ScratchBuildSize;
		AccelerationStructure Destination;
		uint32 BuildFlags = 0;
	};
	GTSL::Vector<AccelerationStructureBuildData, BE::PersistentAllocatorReference> buildDatas;
	GTSL::Vector<AccelerationStructure::Geometry, BE::PersistentAllocatorReference> geometries;

	RenderAllocation scratchBufferAllocation;
	Buffer accelerationStructureScratchBuffer;

	AccelerationStructure topLevelAccelerationStructure;
	RenderAllocation topLevelAccelerationStructureAllocation;
	Buffer topLevelAccelerationStructureBuffer;

	static constexpr uint8 MAX_INSTANCES_COUNT = 16;
	RenderAllocation instancesAllocation;
	uint64 instancesBufferAddress;
	Buffer instancesBuffer;
	
	/**
	 * \brief Pointer to the implementation for acceleration structures build.
	 * Since acc. structures can be built on the host or on the device depending on device capabilities
	 * we determine which one we are able to do and cache it.
	 */
	GTSL::FunctionPointer<void(CommandBuffer&)> buildAccelerationStructures;

	void buildAccelerationStructuresOnDevice(CommandBuffer&);
	
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

	uint32 shaderGroupAlignment = 0, shaderGroupBaseAlignment = 0, shaderGroupHandleSize = 0;
	uint32 scratchBufferOffsetAlignment = 0;
};
