#pragma once

#include <unordered_map>
#include <GTSL/Pair.h>
#include <GTSL/FunctionPointer.hpp>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/GameInstance.h"

#include "RendererAllocator.h"
#include "RenderTypes.h"

#include "ByteEngine/Handle.hpp"

#include <GAL/Vulkan/VulkanQueue.h>

namespace GTSL {
	class Window;
}

MAKE_HANDLE(uint32, Buffer)

class RenderSystem : public System
{
public:
	RenderSystem() : System("RenderSystem") {}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	[[nodiscard]] uint8 GetCurrentFrame() const { return currentFrameIndex; }
	[[nodiscard]] uint8 GetFrameIndex(int32 frameDelta) const { return static_cast<uint8>(frameDelta % pipelinedFrames); }
	uint8 GetPipelinedFrames() const { return pipelinedFrames; }
	GAL::FormatDescriptor GetSwapchainFormat() const { return swapchainFormat; }

	MAKE_HANDLE(uint32, Texture);
	
	void AllocateLocalTextureMemory(uint32 size, Texture* texture, GAL::TextureUse uses, GAL::FormatDescriptor format, GTSL::Extent3D extent, GAL::Tiling tiling,
	                                GTSL::uint8 mipLevels, RenderAllocation* allocation)
	{
		GAL::MemoryRequirements memoryRequirements;
		texture->GetMemoryRequirements(GetRenderDevice(), &memoryRequirements, uses, format, extent, tiling, mipLevels);

		DeviceMemory memory;  uint32 offset = 0;
		
		testMutex.Lock();
		localMemoryAllocator.AllocateNonLinearMemory(renderDevice, &memory, allocation, memoryRequirements.Size, &offset);
		testMutex.Unlock();
		
		texture->Initialize(GetRenderDevice(), memory, offset);
	}
	void DeallocateLocalTextureMemory(const RenderAllocation allocation)
	{
		localMemoryAllocator.DeallocateNonLinearMemory(renderDevice, allocation);
	}

	void AllocateAccelerationStructureMemory(AccelerationStructure* accelerationStructure, GPUBuffer* buffer, GTSL::Range<const GAL::Geometry*> geometries, RenderAllocation* renderAllocation, uint32* scratchSize)
	{
		uint32 bufferSize, memoryScratchSize;
		accelerationStructure->GetMemoryRequirements(GetRenderDevice(), geometries, GAL::Device::GPU, 0, &bufferSize, &memoryScratchSize);
		
		AllocateScratchBufferMemory(bufferSize, GAL::BufferUses::ACCELERATION_STRUCTURE, buffer, renderAllocation);

		accelerationStructure->Initialize(GetRenderDevice(), geometries, *buffer, bufferSize, 0);

		*scratchSize = memoryScratchSize;
	}
	
	void AllocateScratchBufferMemory(uint32 size, GAL::BufferUse flags, GPUBuffer* buffer, RenderAllocation* allocation) {		
		GAL::MemoryRequirements memoryRequirements;
		buffer->GetMemoryRequirements(GetRenderDevice(), size, flags, &memoryRequirements);

		DeviceMemory memory; uint32 offset = 0;
		
		testMutex.Lock();
		scratchMemoryAllocator.AllocateLinearMemory(renderDevice, &memory, allocation, memoryRequirements.Size, &offset);
		testMutex.Unlock();
		
		buffer->Initialize(GetRenderDevice(), memoryRequirements, memory, offset);
	}
	
	void DeallocateScratchBufferMemory(const RenderAllocation allocation) {
		scratchMemoryAllocator.DeallocateLinearMemory(renderDevice, allocation);
	}
	
	void AllocateLocalBufferMemory(uint32 size, GAL::BufferUse flags, GPUBuffer* buffer, RenderAllocation* allocation) {
		GAL::MemoryRequirements memoryRequirements;
		buffer->GetMemoryRequirements(GetRenderDevice(), size, flags, &memoryRequirements);

		DeviceMemory memory; uint32 offset = 0;
		
		testMutex.Lock();
		localMemoryAllocator.AllocateLinearMemory(renderDevice, &memory, allocation, memoryRequirements.Size, &offset);
		testMutex.Unlock();
		
		buffer->Initialize(GetRenderDevice(), memoryRequirements, memory, offset);
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
		BufferHandle Buffer;
		/* Offset from start of buffer.
		 */
		uint32 Offset = 0;
	};
	void AddBufferUpdate(const BufferCopyData& bufferCopyData)
	{
		if(needsStagingBuffer)
			bufferCopyDatas[currentFrameIndex].EmplaceBack(bufferCopyData);
	}
	
	struct TextureCopyData {
		GPUBuffer SourceBuffer;
		Texture DestinationTexture;
		
		uint32 SourceOffset = 0;
		RenderAllocation Allocation;

		GTSL::Extent3D Extent;
		
		GAL::TextureLayout Layout;
		GAL::FormatDescriptor Format;
	};
	void AddTextureCopy(const TextureCopyData& textureCopyData)
	{
		BE_ASSERT(testMutex.TryLock())
		textureCopyDatas[GetCurrentFrame()].EmplaceBack(textureCopyData);
		testMutex.Unlock();
	}

	[[nodiscard]] PipelineCache GetPipelineCache() const;

	[[nodiscard]] const Texture* GetSwapchainTexture() const { return &swapchainTextures[imageIndex]; }

	MAKE_HANDLE(uint32, Mesh);

	GTSL::Range<const GAL::ShaderDataType*> GetMeshVertexLayout(const MeshHandle meshHandle) const { return meshes[meshHandle()].VertexDescriptor; }

	[[nodiscard]] byte* GetBufferPointer(BufferHandle bufferHandle) const {
		if (needsStagingBuffer) {
			return static_cast<byte*>(buffers[buffers[bufferHandle()].Staging()].Allocation.Data);
		}
		else {
			return static_cast<byte*>(buffers[bufferHandle()].Allocation.Data);
		}
	}
	
	void AddVolume(const GTSL::Matrix3x4& position, const GTSL::Vector3 size) {
		auto volume = CreateBuffer(sizeof(float32) * 6, GAL::BufferUses::BUILD_INPUT_READ, true, false);
		auto* bufferPointer = GetBufferPointer(volume);

		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 0) = -size;
		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 1) = size;

		addRayTracingInstance(GAL::Geometry(GAL::GeometryAABB(reinterpret_cast<GAL::DeviceAddress>(bufferPointer), sizeof(float32) * 6), {}, 1, 0), AccelerationStructureBuildData{ 0,  {}, {} });
	}
	
	void CreateRayTracedMesh(const MeshHandle meshHandle);
	
	MeshHandle CreateMesh(Id name, uint32 customIndex, const MaterialInstanceHandle materialInstanceHandle);
	//MeshHandle CreateMesh(Id name, uint32 customIndex, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, MaterialInstanceHandle materialHandle);

	void UpdateRayTraceMesh(const MeshHandle meshHandle);
	void UpdateMesh(MeshHandle meshHandle, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, GTSL::Range<const GAL::ShaderDataType*> vertexLayout);
	void UpdateMesh(MeshHandle meshHandle);
	void SetWillWriteMesh(MeshHandle meshHandle, bool willUpdate) {
		SetBufferWillWriteFromHost(meshes[meshHandle()].Buffer, willUpdate);
	}
	
	void RenderMesh(MeshHandle handle, const uint32 instanceCount = 1);
	
	void DestroyBuffer(const BufferHandle handle) {
		--buffers[handle()].references;
	}

	void DestroyMesh(MeshHandle meshHandle)
	{
		DestroyBuffer(meshes[meshHandle()].Buffer);
	}
	
	byte* GetMeshPointer(MeshHandle sharedMesh) const {
		const auto& mesh = meshes[sharedMesh()];
		return GetBufferPointer(mesh.Buffer);
	}

	uint32 GetMeshSize(MeshHandle meshHandle) const
	{
		const auto& mesh = meshes[meshHandle()];
		return GTSL::Math::RoundUpByPowerOf2(mesh.VertexSize * mesh.VertexCount, GetBufferSubDataAlignment()) + mesh.IndexSize * mesh.IndicesCount;
	}
	
	CommandBuffer* GetCurrentCommandBuffer() { return &graphicsCommandBuffers[currentFrameIndex]; }
	const CommandBuffer* GetCurrentCommandBuffer() const { return &graphicsCommandBuffers[currentFrameIndex]; }
	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }

	void SetMeshMatrix(const MeshHandle meshHandle, const GTSL::Matrix3x4& matrix);
	void SetMeshOffset(const MeshHandle meshHandle, const uint32 offset);
	
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
	
	BufferAddress GetVertexBufferAddress(MeshHandle meshHandle) const { return BufferAddress(buffers[meshes[meshHandle()].Buffer()].Buffer.GetAddress(GetRenderDevice())); }
	BufferAddress GetIndexBufferAddress(MeshHandle meshHandle) const { return BufferAddress(buffers[meshes[meshHandle()].Buffer()].Buffer.GetAddress(GetRenderDevice()) + GTSL::Math::RoundUpByPowerOf2(meshes[meshHandle()].VertexSize * meshes[meshHandle()].VertexCount, GetBufferSubDataAlignment())); }
	
	MaterialInstanceHandle GetMeshMaterialHandle(MeshHandle meshHandle) { return meshes[meshHandle()].MaterialHandle; }

	uint32 GetBufferSubDataAlignment() const { return renderDevice.GetStorageBufferBindingOffsetAlignment(); }

	void SetWindow(GTSL::Window* window) { this->window = window; }

	[[nodiscard]] TextureHandle CreateTexture(GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable);
	void UpdateTexture(const TextureHandle textureHandle);

	//TODO: SELECT DATA POINTER BASED ON STAGING BUFFER NECESSITY
	
	GTSL::Range<byte*> GetTextureRange(TextureHandle textureHandle) {
		const auto& texture = textures[textureHandle()];
		uint32 size = texture.Extent.Width * texture.Extent.Depth * texture.Extent.Height;
		size *= texture.FormatDescriptor.GetSize();
		return GTSL::Range<byte*>(size, static_cast<byte*>(texture.ScratchAllocation.Data));
	}
	
	GTSL::Range<const byte*> GetTextureRange(TextureHandle textureHandle) const {
		const auto& texture = textures[textureHandle()];
		uint32 size = texture.Extent.Width * texture.Extent.Depth * texture.Extent.Height;
		size *= texture.FormatDescriptor.GetSize();
		return GTSL::Range<const byte*>(size, static_cast<const byte*>(texture.ScratchAllocation.Data));
	}

	const Texture* GetTexture(const TextureHandle textureHandle) const { return &textures[textureHandle()].Texture; }
	TextureView GetTextureView(const TextureHandle textureHandle) const { return textures[textureHandle()].TextureView; }
	TextureSampler GetTextureSampler(const TextureHandle handle) const { return textures[handle()].TextureSampler; }

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	bool AcquireImage();
	void SetHasRendered(const bool state) { hasRenderTasks = state; }

	BufferHandle CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, bool updateable);
	void SetBufferWillWriteFromHost(BufferHandle bufferHandle, bool state);
private:
	GTSL::Window* window;
	
	GTSL::Mutex testMutex;

	bool hasRenderTasks = false;
	bool needsStagingBuffer = true;
	uint8 imageIndex = 0;

	uint8 pipelinedFrames = 0;

	bool useHDR = false;
	
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;
	
	GTSL::Extent2D renderArea, lastRenderArea;

	GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference> bufferCopyDatas[MAX_CONCURRENT_FRAMES];
	uint32 processedBufferCopies[MAX_CONCURRENT_FRAMES];
	GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference> textureCopyDatas[MAX_CONCURRENT_FRAMES];
	//GTSL::Array<uint32, MAX_CONCURRENT_FRAMES> processedTextureCopies;
	
	Texture swapchainTextures[MAX_CONCURRENT_FRAMES];
	TextureView swapchainTextureViews[MAX_CONCURRENT_FRAMES];
	
	GPUSemaphore imageAvailableSemaphore[MAX_CONCURRENT_FRAMES];
	GPUSemaphore transferDoneSemaphores[MAX_CONCURRENT_FRAMES];
	GPUSemaphore renderFinishedSemaphore[MAX_CONCURRENT_FRAMES];
	Fence graphicsFences[MAX_CONCURRENT_FRAMES];
	Fence transferFences[MAX_CONCURRENT_FRAMES];
	
	CommandBuffer graphicsCommandBuffers[MAX_CONCURRENT_FRAMES];
	CommandBuffer transferCommandBuffers[MAX_CONCURRENT_FRAMES];
	
	GAL::VulkanQueue graphicsQueue;
	GAL::VulkanQueue transferQueue;
	GAL::Device accelerationStructureBuildDevice;

	struct Mesh
	{
		BufferHandle Buffer;
		uint32 IndicesCount, VertexCount;
		MaterialInstanceHandle MaterialHandle;
		uint32 DerivedTypeIndex, CustomMeshIndex;
		uint8 IndexSize, VertexSize;
		GTSL::Array<GAL::ShaderDataType, 20> VertexDescriptor;
	};
	
	struct RayTracingMesh {
		GPUBuffer StructureBuffer;
		RenderAllocation StructureBufferAllocation;
		AccelerationStructure AccelerationStructure;
	};
	
	GTSL::KeepVector<Mesh, BE::PersistentAllocatorReference> meshes;
	GTSL::KeepVector<RayTracingMesh, BE::PersistentAllocatorReference> rayTracingMeshes;

	struct Buffer
	{
		GPUBuffer Buffer; uint32 Size = 0, Counter = 0;
		GAL::BufferUse Flags;
		uint32 references = 0;
		BufferHandle Staging, Next;
		RenderAllocation Allocation;
	};
	GTSL::KeepVector<Buffer, BE::PAR> buffers;
	
	struct AccelerationStructureBuildData
	{
		uint32 ScratchBuildSize;
		AccelerationStructure Destination;
		uint32 BuildFlags = 0;
	};
	GTSL::Vector<AccelerationStructureBuildData, BE::PersistentAllocatorReference> buildDatas[MAX_CONCURRENT_FRAMES];
	GTSL::Vector<GAL::Geometry, BE::PersistentAllocatorReference> geometries[MAX_CONCURRENT_FRAMES];

	RenderAllocation scratchBufferAllocation[MAX_CONCURRENT_FRAMES];
	GPUBuffer accelerationStructureScratchBuffer[MAX_CONCURRENT_FRAMES];

	AccelerationStructure topLevelAccelerationStructure[MAX_CONCURRENT_FRAMES];
	RenderAllocation topLevelAccelerationStructureAllocation[MAX_CONCURRENT_FRAMES];
	GPUBuffer topLevelAccelerationStructureBuffer[MAX_CONCURRENT_FRAMES];

	/**
	* \brief Keeps track of created instances. Mesh / Material combo.
	*/
	uint32 rayTracingInstancesCount = 0;

	void addRayTracingInstance(GAL::Geometry geometry, AccelerationStructureBuildData buildData) {
		++rayTracingInstancesCount;

		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			geometries[f].EmplaceBack(geometry);
			buildDatas[f].EmplaceBack(buildData);
		}
	}
	
	static constexpr uint8 MAX_INSTANCES_COUNT = 16;

	uint32 topLevelStructureScratchSize;
	
	RenderAllocation instancesAllocation[MAX_CONCURRENT_FRAMES];
	GPUBuffer instancesBuffer[MAX_CONCURRENT_FRAMES];
	
	/**
	 * \brief Pointer to the implementation for acceleration structures build.
	 * Since acc. structures can be built on the host or on the device depending on device capabilities
	 * we determine which one we are able to do and cache it.
	 */
	GTSL::FunctionPointer<void(CommandBuffer&)> buildAccelerationStructures;

	void buildAccelerationStructuresOnDevice(CommandBuffer&);
	
	uint8 currentFrameIndex = 0;

	GAL::PresentModes swapchainPresentMode;
	GAL::FormatDescriptor swapchainFormat;
	GAL::ColorSpace swapchainColorSpace;

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

//	GTSL::StaticMap<uint64, GTSL::Array<GAL::ShaderDataType, 8>, 8> vertexFormats;
	
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
		GAL::TextureUse Uses;
		GPUBuffer ScratchBuffer;
		GAL::TextureLayout Layout;
		GTSL::Extent3D Extent;
	};
	GTSL::KeepVector<TextureComponent, BE::PersistentAllocatorReference> textures;
};
