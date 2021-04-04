#pragma once

#include <unordered_map>
#include <GTSL/Atomic.hpp>
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
	[[nodiscard]] uint8 GetFrameIndex(int32 frameDelta) const { return static_cast<uint8>(frameDelta % pipelinedFrames); }
	uint8 GetPipelinedFrames() const { return pipelinedFrames; }

	MAKE_HANDLE(uint32, Texture);
	
	void UpdateInstanceTransform(const uint32 i, const GTSL::Matrix4& matrix4)
	{
		auto& instance = *(static_cast<AccelerationStructure::Instance*>(instancesAllocation[GetCurrentFrame()].Data) + i);
		instance.Transform = GTSL::Matrix3x4(matrix4);
	}
	
	void AllocateLocalTextureMemory(uint32 size, Texture* texture, Texture::CreateInfo& createInfo, RenderAllocation* allocation)
	{	
		Texture::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = &createInfo;
		texture->GetMemoryRequirements(&memoryRequirements);
		
		allocation->Size = memoryRequirements.MemoryRequirements.Size;
		
		testMutex.Lock();
		localMemoryAllocator.AllocateNonLinearMemory(renderDevice, &createInfo.Memory, allocation, GetPersistentAllocator());
		testMutex.Unlock();

		createInfo.Offset = allocation->Offset;
		
		texture->Initialize(createInfo);
	}
	void DeallocateLocalTextureMemory(const RenderAllocation allocation)
	{
		localMemoryAllocator.DeallocateNonLinearMemory(renderDevice, allocation);
	}

	void AllocateAccelerationStructureMemory(AccelerationStructure* accelerationStructure, Buffer* buffer, GTSL::Range<const AccelerationStructure::Geometry*> geometries, AccelerationStructure::CreateInfo* createInfo, RenderAllocation* renderAllocation, BuildType build, uint32* scratchSize)
	{
		uint32 bufferSize, memoryScratchSize;
		
		AccelerationStructure::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.BuildType = build;
		memoryRequirements.Flags = 0;
		memoryRequirements.Geometries = geometries;
		accelerationStructure->GetMemoryRequirements(memoryRequirements, &bufferSize, &memoryScratchSize);
		
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

		*scratchSize = memoryScratchSize;
	}

	struct BufferLocalMemoryAllocationInfo
	{
		Buffer* Buffer;
		Buffer::CreateInfo* CreateInfo;
		RenderAllocation* Allocation;
	};
	
	void AllocateScratchBufferMemory(uint32 size, Buffer* buffer, Buffer::CreateInfo& createInfo, RenderAllocation* allocation)
	{
		createInfo.Size = size;
		
		Buffer::GetMemoryRequirementsInfo memoryRequirements;
		memoryRequirements.RenderDevice = GetRenderDevice();
		memoryRequirements.CreateInfo = &createInfo;
		buffer->GetMemoryRequirements(&memoryRequirements);
		
		allocation->Size = memoryRequirements.MemoryRequirements.Size;
		
		testMutex.Lock();
		scratchMemoryAllocator.AllocateLinearMemory(renderDevice, &createInfo.Memory, allocation, GetPersistentAllocator());
		testMutex.Unlock();

		createInfo.Offset = allocation->Offset;
		
		buffer->Initialize(createInfo);
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

	[[nodiscard]] PipelineCache GetPipelineCache() const;

	[[nodiscard]] Texture GetSwapchainTexture() const { return swapchainTextures[imageIndex]; }

	MAKE_HANDLE(uint32, Mesh)
	
	struct CreateRayTracingMeshInfo
	{
		uint32 VertexCount, VertexSize;
		uint32 IndexCount, IndexSize;
		GTSL::Matrix3x4 Matrix;
		MeshHandle SharedMesh;
	};
	MeshHandle CreateRayTracedMesh(const CreateRayTracingMeshInfo& info);
	
	MeshHandle CreateMesh(Id name, uint32 customIndex, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, MaterialInstanceHandle materialHandle);

	MeshHandle UpdateMesh(MeshHandle meshHandle);
	
	void RenderMesh(MeshHandle handle, const uint32 instanceCount = 1);

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
	
	CommandBuffer* GetCurrentCommandBuffer() { return &graphicsCommandBuffers[currentFrameIndex]; }
	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void SetMeshMatrix(const MeshHandle meshHandle, const GTSL::Matrix4& matrix);
	
	void OnResize(GTSL::Extent2D extent) { renderArea = extent; }

	uint32 GetShaderGroupHandleSize() const { return shaderGroupHandleSize; }
	uint32 GetShaderGroupBaseAlignment() const { return shaderGroupBaseAlignment; }
	uint32 GetShaderGroupHandleAlignment() const { return shaderGroupHandleAlignment; }

	AccelerationStructure GetTopLevelAccelerationStructure(uint8 frame) const { return topLevelAccelerationStructure[frame]; }

	struct BufferAddress
	{
		BufferAddress(const uint64 address) : Address(address / MULTIPLIER)
		{
			BE_ASSERT(address < 0xFFFFFFFF, ""); BE_ASSERT(address % MULTIPLIER == 0, "");
		}
		
		BufferAddress(const void* address) : BufferAddress(reinterpret_cast<uint64>(address)) {}
		
		uint32 Address; static constexpr uint32 MULTIPLIER = 16;
	};
	
	BufferAddress GetVertexBufferAddress(MeshHandle meshHandle) const { return BufferAddress(meshes[meshHandle()].Buffer.GetAddress(GetRenderDevice())); }
	BufferAddress GetIndexBufferAddress(MeshHandle meshHandle) const { return BufferAddress(meshes[meshHandle()].Buffer.GetAddress(GetRenderDevice()) + GTSL::Math::RoundUpByPowerOf2(meshes[meshHandle()].VertexSize * meshes[meshHandle()].VertexCount, GetBufferSubDataAlignment())); }
	
	uint32 GetMeshIndex(MeshHandle meshHandle) { return meshHandle(); }
	MaterialInstanceHandle GetMeshMaterialHandle(uint32 meshHandle) { return meshes[meshHandle].MaterialHandle; }
	
	auto GetAddedMeshes() const { return addedMeshes.GetRange(); }
	void ClearAddedMeshes() { return addedMeshes.ResizeDown(0); }

	uint32 GetBufferSubDataAlignment() const { return renderDevice.GetStorageBufferBindingOffsetAlignment(); }

	void SetWindow(GTSL::Window* window) { this->window = window; }

	[[nodiscard]] TextureHandle CreateTexture(GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, TextureUses textureUses, bool updatable);
	void UpdateTexture(const TextureHandle textureHandle);
	GTSL::Range<byte*> GetTextureRange(TextureHandle textureHandle) { return GTSL::Range<byte*>(textures[textureHandle()].ScratchAllocation.Size, static_cast<byte*>(textures[textureHandle()].ScratchAllocation.Data)); }
	GTSL::Range<const byte*> GetTextureRange(TextureHandle textureHandle) const { return GTSL::Range<const byte*>(textures[textureHandle()].ScratchAllocation.Size, static_cast<const byte*>(textures[textureHandle()].ScratchAllocation.Data)); }

	Texture GetTexture(const TextureHandle textureHandle) const { return textures[textureHandle()].Texture; }
	TextureView GetTextureView(const TextureHandle textureHandle) const { return textures[textureHandle()].TextureView; }
	TextureSampler GetTextureSampler(const TextureHandle handle) const { return textures[handle()].TextureSampler; }

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	bool AcquireImage();
	void SetHasRendered(const bool state) { hasRenderTasks = state; }
private:
	GTSL::Window* window;
	
	GTSL::Mutex testMutex;

	bool hasRenderTasks = false;
	bool needsStagingBuffer = true;
	uint8 imageIndex = 0;

	uint8 pipelinedFrames = 0;
	
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;
	
	GTSL::Extent2D renderArea, lastRenderArea;

	GTSL::Array<GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> bufferCopyDatas;
	GTSL::Array<uint32, MAX_CONCURRENT_FRAMES> processedBufferCopies;
	GTSL::Array<GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference>, MAX_CONCURRENT_FRAMES> textureCopyDatas;
	//GTSL::Array<uint32, MAX_CONCURRENT_FRAMES> processedTextureCopies;
	
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

		uint32 IndicesCount, VertexCount;
		MaterialInstanceHandle MaterialHandle;
		RenderAllocation MeshAllocation;
		
		uint32 DerivedTypeIndex, CustomMeshIndex;
		uint8 IndexSize, VertexSize;
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

	AccelerationStructure topLevelAccelerationStructure[MAX_CONCURRENT_FRAMES];
	RenderAllocation topLevelAccelerationStructureAllocation[MAX_CONCURRENT_FRAMES];
	Buffer topLevelAccelerationStructureBuffer[MAX_CONCURRENT_FRAMES];

	static constexpr uint8 MAX_INSTANCES_COUNT = 16;

	uint32 topLevelStructureScratchSize;
	
	RenderAllocation instancesAllocation[MAX_CONCURRENT_FRAMES];
	Buffer instancesBuffer[MAX_CONCURRENT_FRAMES];
	
	/**
	 * \brief Pointer to the implementation for acceleration structures build.
	 * Since acc. structures can be built on the host or on the device depending on device capabilities
	 * we determine which one we are able to do and cache it.
	 */
	GTSL::FunctionPointer<void(CommandBuffer&)> buildAccelerationStructures;

	void buildAccelerationStructuresOnDevice(CommandBuffer&);
	
	uint8 currentFrameIndex = 0;

	GAL::PresentModes swapchainPresentMode;
	TextureFormat swapchainFormat;
	ColorSpace swapchainColorSpace;

	bool resize();
	
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

	uint32 shaderGroupHandleAlignment = 0, shaderGroupBaseAlignment = 0, shaderGroupHandleSize = 0;
	uint32 scratchBufferOffsetAlignment = 0;

	struct TextureComponent
	{
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;
		RenderAllocation Allocation, ScratchAllocation;

		GAL::FormatDescriptor FormatDescriptor;
		TextureUses Uses;
		Buffer ScratchBuffer;
		TextureLayout Layout;
		GTSL::Extent3D Extent;
	};
	GTSL::KeepVector<TextureComponent, BE::PersistentAllocatorReference> textures;
};
