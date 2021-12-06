#pragma once

#include <unordered_map>
#include <GTSL/Pair.hpp>
#include <GTSL/FunctionPointer.hpp>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Game/ApplicationManager.h"

#include "RendererAllocator.h"
#include "RenderTypes.h"

#include "ByteEngine/Handle.hpp"

#include <GAL/Vulkan/VulkanQueue.h>
#include <GTSL/Bitfield.h>

namespace GTSL {
	class Window;
}


class RenderSystem : public System
{
public:
	MAKE_HANDLE(uint32, Buffer)
	MAKE_HANDLE(uint32, Texture);
	
	explicit RenderSystem(const InitializeInfo& initializeInfo);
	~RenderSystem();
	[[nodiscard]] uint8 GetCurrentFrame() const { return currentFrameIndex; }
	[[nodiscard]] uint8 GetFrameIndex(int32 frameDelta) const { return static_cast<uint8>(frameDelta % pipelinedFrames); }
	uint8 GetPipelinedFrames() const { return pipelinedFrames; }
	GAL::FormatDescriptor GetSwapchainFormat() const { return swapchainFormat; }
	DynamicTaskHandle<GTSL::Extent2D> GetResizeHandle() const { return resizeHandle; }

	MAKE_HANDLE(uint32, CommandList);

	CommandListHandle CreateCommandList(const GTSL::StringView name, bool isSingleFrame = true) {
		uint32 index = commandLists.GetLength();
		auto& commandList = commandLists.EmplaceBack();
		commandList.CommandList.Initialize(GetRenderDevice(), name, graphicsQueue.GetQueueKey());
		commandList.Fence.Initialize(GetRenderDevice(), true);
		commandList.Semaphore.Initialize(GetRenderDevice());
		return CommandListHandle(index);
	}

	void StartCommandList(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];

		commandListData.Fence.Wait(GetRenderDevice());
		commandListData.Fence.Reset(GetRenderDevice());
		commandListData.CommandList.BeginRecording(GetRenderDevice());

		beginGraphicsCommandLists(commandListData);
	}

	void DispatchBuild(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];

		for (auto& e : topLevelAccelerationStructures) {
			GAL::Geometry geometry(GAL::GeometryInstances{ GetBufferDeviceAddress(e.InstancesBuffer) }, GAL::GeometryFlag(), e.ScratchSize, 0); //TODO
			geometries[GetCurrentFrame()].EmplaceBack(geometry);

			AccelerationStructureBuildData buildData;
			buildData.BuildFlags = 0;
			buildData.Destination = e.AccelerationStructures[GetCurrentFrame()];
			buildData.ScratchBuildSize = e.ScratchSize;
			buildDatas[GetCurrentFrame()].EmplaceBack(buildData);

			buildAccelerationStructures(this, commandListData.CommandList);
		}
	}

	void EndCommandList(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];
		commandListData.CommandList.EndRecording(GetRenderDevice());
	}

	void SubmitAndPresent(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];

		GTSL::StaticVector<Queue::WorkUnit, 8> workUnits;

		auto& graphicsWork = workUnits.EmplaceBack();

		graphicsWork.WaitSemaphore = &imageAvailableSemaphore[GetCurrentFrame()];

		graphicsWork.WaitPipelineStage = GAL::PipelineStages::TRANSFER;
		graphicsWork.SignalSemaphore = &commandListData.Semaphore;
		graphicsWork.CommandBuffer = &commandListData.CommandList;

		if (surface.GetHandle()) {
			graphicsWork.WaitPipelineStage |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
		}

		graphicsQueue.Submit(GetRenderDevice(), workUnits, commandListData.Fence);

		GTSL::StaticVector<GPUSemaphore*, 8> presentWaitSemaphores;

		if (surface.GetHandle()) {
			presentWaitSemaphores.EmplaceBack(&commandListData.Semaphore);

			if (!renderContext.Present(GetRenderDevice(), presentWaitSemaphores, imageIndex, graphicsQueue)) {
				resize();
			}
		}
	}

	void AllocateLocalTextureMemory(Texture* texture, const GTSL::StringView name, GAL::TextureUse uses, GAL::FormatDescriptor format, GTSL::Extent3D extent, GAL::Tiling tiling,
	                                GTSL::uint8 mipLevels, RenderAllocation* allocation)
	{
		GAL::MemoryRequirements memoryRequirements;
		texture->GetMemoryRequirements(GetRenderDevice(), &memoryRequirements, uses, format, extent, tiling, mipLevels);

		DeviceMemory memory;  uint32 offset = 0;
		
		testMutex.Lock();
		localMemoryAllocator.AllocateNonLinearMemory(renderDevice, &memory, allocation, memoryRequirements.Size, &offset);
		testMutex.Unlock();
		
		texture->Initialize(GetRenderDevice(), name, memory, offset);
	}

	void DeallocateLocalTextureMemory(const RenderAllocation allocation) {
		localMemoryAllocator.DeallocateNonLinearMemory(renderDevice, allocation);
	}

	void AllocateAccelerationStructureMemory(AccelerationStructure* accelerationStructure, GPUBuffer* buffer, GTSL::Range<const GAL::Geometry*> geometries, RenderAllocation* renderAllocation, uint32* scratchSize)
	{
		uint32 bufferSize, memoryScratchSize;
		accelerationStructure->GetMemoryRequirements(GetRenderDevice(), geometries, GAL::Device::GPU, GAL::AccelerationStructureFlag(), &bufferSize, &memoryScratchSize);
		
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

	void DeallocateLocalBufferMemory(const RenderAllocation renderAllocation) {
		localMemoryAllocator.DeallocateLinearMemory(renderDevice, renderAllocation);
	}
	
	RenderDevice* GetRenderDevice() { return &renderDevice; }
	const RenderDevice* GetRenderDevice() const { return &renderDevice; }
	GPUBuffer GetBuffer(const RenderSystem::BufferHandle buffer_handle) const {
		return buffers[buffer_handle()].Buffer[0];
		//TODO: is multi
	}
	//CommandList* GetTransferCommandBuffer() { return &transferCommandBuffers[currentFrameIndex]; }
	
	void AddBufferUpdate(const BufferHandle buffer_handle, uint32 offset = 0)
	{
		if(needsStagingBuffer)
			bufferCopyDatas[currentFrameIndex].EmplaceBack(buffer_handle, offset);
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

	[[nodiscard]] byte* GetBufferPointer(BufferHandle bufferHandle) const {
		if (needsStagingBuffer) {
			if (buffers[bufferHandle()].isMulti) {
				return static_cast<byte*>(buffers[bufferHandle()].StagingAllocation[GetCurrentFrame()].Data);
			} else {
				return static_cast<byte*>(buffers[bufferHandle()].StagingAllocation[0].Data);
			}
		} else {
			if (buffers[bufferHandle()].isMulti) {
				return static_cast<byte*>(buffers[bufferHandle()].Allocation[GetCurrentFrame()].Data);
			} else {
				return static_cast<byte*>(buffers[bufferHandle()].Allocation[0].Data);
			}
		}
	}

	[[nodiscard]] GAL::DeviceAddress GetBufferDeviceAddress(BufferHandle bufferHandle) const {
		if (needsStagingBuffer) {
			if (buffers[bufferHandle()].isMulti) {
				return buffers[bufferHandle()].Staging[GetCurrentFrame()].GetAddress(GetRenderDevice());
			} else {
				return buffers[bufferHandle()].Staging[0].GetAddress(GetRenderDevice());
			}
		} else {
			if (buffers[bufferHandle()].isMulti) {
				return buffers[bufferHandle()].Buffer[GetCurrentFrame()].GetAddress(GetRenderDevice());
			} else {
				return buffers[bufferHandle()].Buffer[0].GetAddress(GetRenderDevice());
			}
		}
	}

	void SignalBufferWrite(const BufferHandle buffer_handle) {
		auto& buffer = buffers[buffer_handle()];

		++buffer.references;

		if(buffer.isMulti) {
			buffer.writeMask[currentFrameIndex] = true;
		}

		AddBufferUpdate(buffer_handle);
	}
	
	void DestroyBuffer(const BufferHandle handle) {
		--buffers[handle()].references;
	}
	
	CommandList* GetCommandList(const CommandListHandle handle) { return &commandLists[handle()].CommandList; }
	const CommandList* GetCommandList(const CommandListHandle handle) const { return &commandLists[handle()].CommandList; }

	[[nodiscard]] GTSL::Extent2D GetRenderExtent() const { return renderArea; }
	
	void onResize(TaskInfo, GTSL::Extent2D extent) { renderArea = extent; }

	uint32 GetShaderGroupHandleSize() const { return shaderGroupHandleSize; }
	uint32 GetShaderGroupBaseAlignment() const { return shaderGroupBaseAlignment; }
	uint32 GetShaderGroupHandleAlignment() const { return shaderGroupHandleAlignment; }

	AccelerationStructure GetTopLevelAccelerationStructure(uint32 topLevelAccelerationStructureIndex, uint8 frame) const {
		return topLevelAccelerationStructures[topLevelAccelerationStructureIndex].AccelerationStructures[frame];
	}

	uint32 GetBufferSubDataAlignment() const { return renderDevice.GetStorageBufferBindingOffsetAlignment(); }

	void SetWindow(GTSL::Window* window) { this->window = window; }

	[[nodiscard]] TextureHandle CreateTexture(GTSL::Range<const char8_t*> name, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable);
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
		return GTSL::Range(size, static_cast<const byte*>(texture.ScratchAllocation.Data));
	}

	const Texture* GetTexture(const TextureHandle textureHandle) const { return &textures[textureHandle()].Texture; }
	const TextureView* GetTextureView(const TextureHandle textureHandle) const { return &textures[textureHandle()].TextureView; }

	void OnRenderEnable(TaskInfo taskInfo, bool oldFocus);
	void OnRenderDisable(TaskInfo taskInfo, bool oldFocus);

	GTSL::Result<GTSL::Extent2D> AcquireImage();

	BufferHandle CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, bool updateable);
	void SetBufferWillWriteFromHost(BufferHandle bufferHandle, bool state);

	uint32 CreateTopLevelAccelerationStructure(uint32 estimatedMaxInstances) {
		uint32 tlasi = topLevelAccelerationStructures.GetLength();
		auto& t = topLevelAccelerationStructures.EmplaceBack();

		t.InstanceCapacity = estimatedMaxInstances;

		GAL::Geometry geometry(GAL::GeometryInstances(), GAL::GeometryFlag(), estimatedMaxInstances, 0);

		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			AllocateAccelerationStructureMemory(&t.AccelerationStructures[f], &t.AccelerationStructureBuffer[f],
				GTSL::Range(1, &geometry), &t.AccelerationStructureAllocation[f], &t.ScratchSize);

			t.AccelerationStructures[f].Initialize(&renderDevice, GTSL::Range(1, &geometry), t.AccelerationStructureBuffer[f], t.ScratchSize, 0);
		}

		t.InstancesBuffer = CreateBuffer(64 * estimatedMaxInstances, GAL::BufferUses::BUILD_INPUT_READ, true, true);

		return tlasi;
	}

	uint32 CreateBottomLevelAccelerationStructure(uint32 vertexCount, uint32 vertexSize, uint32 indexCount, GAL::IndexType indexType,  BufferHandle sourceBuffer, uint32 offset = 0) {
		uint32 blasi = bottomLevelAccelerationStructures.Emplace();

		auto& blas = bottomLevelAccelerationStructures[blasi];

		GAL::DeviceAddress meshDataAddress;

		meshDataAddress = GetBufferDeviceAddress(sourceBuffer) + offset;

		uint32 scratchSize;

		{
			GAL::GeometryTriangles geometryTriangles;
			geometryTriangles.IndexType = indexType;
			geometryTriangles.VertexPositionFormat = GAL::ShaderDataType::FLOAT3;
			geometryTriangles.MaxVertices = vertexCount;
			geometryTriangles.VertexData = meshDataAddress;
			geometryTriangles.IndexData = meshDataAddress + GTSL::Math::RoundUpByPowerOf2(vertexCount * vertexSize, GetBufferSubDataAlignment());
			geometryTriangles.VertexStride = vertexSize;
			geometryTriangles.FirstVertex = 0;

			GAL::Geometry geometry(geometryTriangles, GAL::GeometryFlags::OPAQUE, indexCount / 3, 0);

			AllocateAccelerationStructureMemory(&blas.AccelerationStructure, &blas.AccelerationStructureBuffer,
				GTSL::Range(1, &geometry), &blas.AccelerationStructureAllocation, &scratchSize);

			AccelerationStructureBuildData buildData;
			buildData.ScratchBuildSize = scratchSize;
			buildData.Destination = blas.AccelerationStructure;
			addRayTracingInstance(geometry, buildData);
		}

		return blasi;
	}

	uint32 CreateAABB(const GTSL::Matrix4& position, const GTSL::Vector3 size) {
		auto volume = CreateBuffer(sizeof(float32) * 6, GAL::BufferUses::BUILD_INPUT_READ, true, false);
		auto bufferDeviceAddress = GetBufferDeviceAddress(volume);
		auto bufferPointer = GetBufferPointer(volume);

		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 0) = -size;
		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 1) = size;

		addRayTracingInstance(GAL::Geometry(GAL::GeometryAABB(bufferDeviceAddress, sizeof(float32) * 6), {}, 1, 0), AccelerationStructureBuildData{ 0,  {}, {} });
		return 0;
	}

	uint32 AddBLASToTLAS(const uint32 tlasi, const uint32 blasi) {
		auto& tlas = topLevelAccelerationStructures[tlasi];
		auto& blas = bottomLevelAccelerationStructures[blasi];

		uint32 instanceIndex = 0;

		if(tlas.freeSlots) {
			instanceIndex = tlas.freeSlots.back();
		} else {
			instanceIndex = tlas.Instances++;
		}

		GAL::WriteInstance(blas.AccelerationStructure, instanceIndex, GAL::GeometryFlags::OPAQUE, GetRenderDevice(), GetBufferPointer(tlas.InstancesBuffer), 0, accelerationStructureBuildDevice);
		return instanceIndex;
	}

	void SetInstancePosition(uint32 instanceIndex, const GTSL::Matrix4& matrix4) {
		GAL::WriteInstanceMatrix(GTSL::Matrix3x4(matrix4), GetBufferPointer(topLevelAccelerationStructures.back().InstancesBuffer), instanceIndex);
	}

	void SetInstanceBindingTableRecordOffset(uint32 instanceIndex, const uint32 offset) {
		GAL::WriteInstanceBindingTableRecordOffset(offset, GetBufferPointer(topLevelAccelerationStructures.back().InstancesBuffer), instanceIndex);
	}

private:
	GTSL::Window* window;
	
	GTSL::Mutex testMutex;
	
	bool needsStagingBuffer = true;
	uint8 imageIndex = 0;

	uint8 pipelinedFrames = 0;

	bool useHDR = false;
	
	RenderDevice renderDevice;
	Surface surface;
	RenderContext renderContext;
	
	GTSL::Extent2D renderArea, lastRenderArea;

	struct BufferCopyData {
		BufferHandle BufferHandle; uint32 Offset = 0;
	};
	GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference> bufferCopyDatas[MAX_CONCURRENT_FRAMES];
	uint32 processedBufferCopies[MAX_CONCURRENT_FRAMES];
	GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference> textureCopyDatas[MAX_CONCURRENT_FRAMES];
	
	Texture swapchainTextures[MAX_CONCURRENT_FRAMES];
	TextureView swapchainTextureViews[MAX_CONCURRENT_FRAMES];
	
	GPUSemaphore imageAvailableSemaphore[MAX_CONCURRENT_FRAMES];
	
	GAL::VulkanQueue graphicsQueue;
	bool breakOnError = true;
	DynamicTaskHandle<GTSL::Extent2D> resizeHandle;

	struct BufferData {
		GPUBuffer Buffer[MAX_CONCURRENT_FRAMES];
		uint32 Size = 0, Counter = 0;
		GAL::BufferUse Flags;
		uint32 references = 0;
		bool isMulti = false;
		GTSL::Bitfield<MAX_CONCURRENT_FRAMES> writeMask;
		GPUBuffer Staging[MAX_CONCURRENT_FRAMES];
		RenderAllocation Allocation[MAX_CONCURRENT_FRAMES];
		RenderAllocation StagingAllocation[MAX_CONCURRENT_FRAMES];
	};
	GTSL::FixedVector<BufferData, BE::PAR> buffers;
	
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

	struct TopLevelAccelerationStructure {
		AccelerationStructure AccelerationStructures[MAX_CONCURRENT_FRAMES];
		RenderAllocation AccelerationStructureAllocation[MAX_CONCURRENT_FRAMES];
		GPUBuffer AccelerationStructureBuffer[MAX_CONCURRENT_FRAMES];
		uint32 ScratchSize = 0, InstanceCapacity = 0;
		BufferHandle InstancesBuffer;
		GTSL::StaticVector<uint32, 8> freeSlots;
		uint32 Instances = 0;
	};
	GTSL::StaticVector<TopLevelAccelerationStructure, 8> topLevelAccelerationStructures;

	struct BottomLevelAccelerationStructure {
		GPUBuffer AccelerationStructureBuffer;
		RenderAllocation AccelerationStructureAllocation;
		AccelerationStructure AccelerationStructure;
	};
	GTSL::FixedVector<BottomLevelAccelerationStructure, BE::PersistentAllocatorReference> bottomLevelAccelerationStructures;

	struct CommandListData {
		CommandList CommandList;
		Fence Fence;
		GPUSemaphore Semaphore;
	};
	GTSL::StaticVector<CommandListData, 8> commandLists;

	GAL::Device accelerationStructureBuildDevice;

	void addRayTracingInstance(GAL::Geometry geometry, AccelerationStructureBuildData buildData) {
		//++rayTracingInstancesCount;

		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			geometries[f].EmplaceBack(geometry);
			buildDatas[f].EmplaceBack(buildData);
		}
	}
	
	/**
	 * \brief Pointer to the implementation for acceleration structures build.
	 * Since acc. structures can be built on the host or on the device depending on device capabilities
	 * we determine which one we are able to do and cache it.
	 */
	GTSL::FunctionPointer<void(CommandList&)> buildAccelerationStructures;

	void buildAccelerationStructuresOnDevice(CommandList&);
	
	uint8 currentFrameIndex = 0;

	GAL::PresentModes swapchainPresentMode;
	GAL::FormatDescriptor swapchainFormat;
	GAL::ColorSpace swapchainColorSpace;

	void resize();
	
	void beginGraphicsCommandLists(CommandListData& command_list_data);
	void renderFlush(TaskInfo taskInfo);
	void executeTransfers(TaskInfo taskInfo);

	void printError(const GTSL::StringView message, RenderDevice::MessageSeverity messageSeverity) const;
	void* allocateApiMemory(void* data, uint64 size, uint64 alignment);
	void* reallocateApiMemory(void* data, void* allocation, uint64 size, uint64 alignment);
	void deallocateApiMemory(void* data, void* allocation);

	//GTSL::StaticMap<uint64, GTSL::StaticVector<GAL::ShaderDataType, 8>, 8> vertexFormats;
	
	GTSL::Mutex allocationsMutex;
	GTSL::HashMap<uint64, GTSL::Pair<uint64, uint64>, BE::PersistentAllocatorReference> apiAllocations;
	
	ScratchMemoryAllocator scratchMemoryAllocator;
	LocalMemoryAllocator localMemoryAllocator;

	GTSL::StaticVector<PipelineCache, 32> pipelineCaches;

	uint32 shaderGroupHandleAlignment = 0, shaderGroupBaseAlignment = 0, shaderGroupHandleSize = 0;
	uint32 scratchBufferOffsetAlignment = 0;

	struct TextureComponent {
		Texture Texture;
		TextureView TextureView;
		RenderAllocation Allocation, ScratchAllocation;

		GAL::FormatDescriptor FormatDescriptor;
		GAL::TextureUse Uses;
		GPUBuffer ScratchBuffer;
		GAL::TextureLayout Layout;
		GTSL::Extent3D Extent;
	};
	GTSL::FixedVector<TextureComponent, BE::PersistentAllocatorReference> textures;

	void initializeFrameResources(const uint8 frameIndex);
	void freeFrameResources(const uint8 frameIndex);
};
