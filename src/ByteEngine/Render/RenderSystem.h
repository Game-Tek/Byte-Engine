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

	void ResizeBuffer(BufferHandle buffer_handle, uint32 size) {
		auto& buffer = buffers[buffer_handle()];
		if(buffer.Size < size) {
			if(buffer.isMulti) {
				__debugbreak();
			} else {
				if(buffer.Buffer[0].GetVkBuffer()) {
					buffer.Buffer[0].Destroy(GetRenderDevice());
					DeallocateLocalBufferMemory(buffer.Allocation[0]);
				}

				AllocateLocalBufferMemory(size, buffer.Flags, &buffer.Buffer[0], &buffer.Allocation[0]);

				if(needsStagingBuffer) {
					if(buffer.Staging[0].GetVkBuffer()) {
						buffer.Staging[0].Destroy(GetRenderDevice());
						DeallocateLocalBufferMemory(buffer.StagingAllocation[0]);
					}

					AllocateScratchBufferMemory(size, buffer.Flags, &buffer.Staging[0], &buffer.StagingAllocation[0]);
				}
			}
		}
	}

	MAKE_HANDLE(uint32, CommandList);
	MAKE_HANDLE(uint32, AccelerationStructure);
	MAKE_HANDLE(uint32, BLASInstance);

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

	void DispatchBuild(const CommandListHandle command_list_handle, const GTSL::Range<const AccelerationStructureHandle*> handles) {
		auto& commandListData = commandLists[command_list_handle()];

		GTSL::StaticVector<GAL::AccelerationStructureBuildInfo, 8> build_datas;
		GTSL::StaticVector<GAL::Geometry, 8> geometries;

		if (accelerationStructures[handles[0]()].isTop) {
			auto& buildData = build_datas.EmplaceBack();
			buildData.DestinationAccelerationStructure = accelerationStructures[handles[0]()].TopLevel.AccelerationStructures[GetCurrentFrame()];
			buildData.ScratchBufferAddress = GetBufferDeviceAddress(accelerationStructures[handles[0]()].TopLevel.ScratchBuffer);

			for (auto e : handles) {
				auto& as = accelerationStructures[e()];
				auto& tlas = accelerationStructures[e()].TopLevel;
				geometries.EmplaceBack(GAL::GeometryInstances{ GetBufferDeviceAddress(tlas.InstancesBuffer) }, GAL::GeometryFlag(), as.PrimitiveCount, 0);
			}

			buildData.Geometries = geometries;
		} else {
			auto& buildData = build_datas.EmplaceBack();

			for (auto e : handles) {
				auto& as = accelerationStructures[e()];
				auto& blas = accelerationStructures[e()].BottomLevel;
				auto address = GetBufferDeviceAddress(blas.DataBuffer);
				geometries.EmplaceBack(GAL::Geometry{ GAL::GeometryTriangles{ GAL::ShaderDataType::FLOAT3, GAL::IndexType::UINT16, static_cast<uint8>(blas.VertexSize), address, address + GTSL::Math::RoundUpByPowerOf2(blas.VertexCount * blas.VertexSize, GetBufferSubDataAlignment()), 0, blas.VertexCount}, GAL::GeometryFlags::OPAQUE, as.PrimitiveCount, 0});
			}

			buildData.Geometries = geometries;
		}

		switch (accelerationStructureBuildDevice) {
		case GAL::Device::CPU: break;
		case GAL::Device::GPU:
		case GAL::Device::GPU_OR_CPU: {
			commandListData.CommandList.BuildAccelerationStructure(GetRenderDevice(), build_datas, GetTransientAllocator());
			break;
		}
		default:;
		}

		GTSL::StaticVector<CommandList::BarrierData, 1> barriers;
		barriers.EmplaceBack(GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::PipelineStages::RAY_TRACING, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::MemoryBarrier{});
		commandListData.CommandList.AddPipelineBarrier(GetRenderDevice(), barriers, GetTransientAllocator());
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

	AccelerationStructure GetTopLevelAccelerationStructure(AccelerationStructureHandle topLevelAccelerationStructureIndex, uint8 frame) const {
		return accelerationStructures[topLevelAccelerationStructureIndex()].TopLevel.AccelerationStructures[frame];
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

	AccelerationStructureHandle CreateTopLevelAccelerationStructure(uint32 estimatedMaxInstances) {
		uint32 tlasi = accelerationStructures.Emplace();
		auto& t = accelerationStructures[tlasi].TopLevel;
		
		accelerationStructures[tlasi].PrimitiveCount = estimatedMaxInstances;

		GAL::Geometry geometry(GAL::GeometryInstances(), GAL::GeometryFlag(), estimatedMaxInstances, 0);

		uint32 size;

		t.AccelerationStructures[0].GetMemoryRequirements(GetRenderDevice(), GTSL::Range(1, &geometry), accelerationStructureBuildDevice, GAL::AccelerationStructureFlags::PREFER_FAST_TRACE, &size, &t.ScratchSize);

		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			AllocateLocalBufferMemory(size, GAL::BufferUses::ACCELERATION_STRUCTURE, &t.AccelerationStructureBuffer[f], &t.AccelerationStructureAllocation[f]);
			t.AccelerationStructures[f].Initialize(&renderDevice, true, t.AccelerationStructureBuffer[f], t.ScratchSize, 0);
		}

		t.InstancesBuffer = CreateBuffer(64 * estimatedMaxInstances, GAL::BufferUses::BUILD_INPUT_READ, true, true);
		t.ScratchBuffer = CreateBuffer(1024 * 1204, GAL::BufferUses::BUILD_INPUT_READ, true, false);

		return AccelerationStructureHandle{ tlasi };
	}

	AccelerationStructureHandle CreateBottomLevelAccelerationStructure(uint32 vertexCount, uint32 vertexSize, uint32 indexCount, GAL::IndexType indexType,  BufferHandle sourceBuffer, bool willUpdate = false, bool willRebuild = false, bool isOpaque = true, uint32 offset = 0) {
		uint32 blasi = accelerationStructures.Emplace();

		auto& blas = accelerationStructures[blasi].BottomLevel;

		blas.VertexCount = vertexCount; blas.VertexSize = vertexSize; blas.DataBuffer = sourceBuffer;
		accelerationStructures[blasi].PrimitiveCount = indexCount / 3;

		GAL::GeometryTriangles geometryTriangles; //todo: add buffer references, so it can't be deleted while blas build consumes it
		geometryTriangles.IndexType = indexType;
		geometryTriangles.VertexPositionFormat = GAL::ShaderDataType::FLOAT3;
		geometryTriangles.MaxVertices = vertexCount;
		geometryTriangles.VertexData = GAL::DeviceAddress();
		geometryTriangles.IndexData = GAL::DeviceAddress();
		geometryTriangles.VertexStride = vertexSize;
		geometryTriangles.FirstVertex = 0;

		GAL::GeometryFlag geometry_flags; geometry_flags |= isOpaque ? GAL::GeometryFlags::OPAQUE : 0;
		GAL::Geometry geometry(geometryTriangles, geometry_flags, indexCount / 3, 0);

		GAL::AccelerationStructureFlag acceleration_structure_flag;
		acceleration_structure_flag |= !willRebuild ? GAL::AccelerationStructureFlags::ALLOW_COMPACTION : 0;
		acceleration_structure_flag |= !willUpdate ? GAL::AccelerationStructureFlags::ALLOW_COMPACTION : 0;
		acceleration_structure_flag |= willUpdate or willRebuild ? GAL::AccelerationStructureFlags::PREFER_FAST_BUILD : 0;
		acceleration_structure_flag |= willUpdate ? GAL::AccelerationStructureFlags::ALLOW_UPDATE : 0;
		acceleration_structure_flag |= !willUpdate and !willRebuild ? GAL::AccelerationStructureFlags::PREFER_FAST_TRACE : 0;

		uint32 bufferSize;
		blas.AccelerationStructure.GetMemoryRequirements(GetRenderDevice(), GTSL::Range(1, &geometry), GAL::Device::GPU, acceleration_structure_flag, &bufferSize, &blas.ScratchSize);
		AllocateLocalBufferMemory(bufferSize, GAL::BufferUses::ACCELERATION_STRUCTURE, &blas.AccelerationStructureBuffer, &blas.AccelerationStructureAllocation);
		blas.AccelerationStructure.Initialize(GetRenderDevice(), false, blas.AccelerationStructureBuffer, bufferSize, 0);

		return AccelerationStructureHandle{ blasi };
	}

	uint32 CreateAABB(const GTSL::Matrix4& position, const GTSL::Vector3 size) {
		auto volume = CreateBuffer(sizeof(float32) * 6, GAL::BufferUses::BUILD_INPUT_READ, true, false);
		auto bufferDeviceAddress = GetBufferDeviceAddress(volume);
		auto bufferPointer = GetBufferPointer(volume);

		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 0) = -size;
		*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 1) = size;

		//addRayTracingInstance(GAL::Geometry(GAL::GeometryAABB(bufferDeviceAddress, sizeof(float32) * 6), {}, 1, 0), AccelerationStructureBuildData{ 0,  {}, {} });
		return 0;
	}

	BLASInstanceHandle AddBLASToTLAS(const AccelerationStructureHandle tlash, const AccelerationStructureHandle blash) {
		auto& tlas = accelerationStructures[tlash()].TopLevel;
		auto& blas = accelerationStructures[blash()].BottomLevel;

		uint32 instanceIndex = 0;

		if(tlas.freeSlots) {
			instanceIndex = tlas.freeSlots.back();
		} else {
			instanceIndex = tlas.Instances++;
		}

		GAL::WriteInstance(blas.AccelerationStructure, instanceIndex, GAL::GeometryFlags::OPAQUE, GetRenderDevice(), GetBufferPointer(tlas.InstancesBuffer), 0, accelerationStructureBuildDevice);

		return BLASInstanceHandle(instanceIndex);
	}

	void SetInstancePosition(AccelerationStructureHandle topLevel, BLASInstanceHandle instance_handle, const GTSL::Matrix4& matrix4) {
		GAL::WriteInstanceMatrix(GTSL::Matrix3x4(matrix4), GetBufferPointer(accelerationStructures[topLevel()].TopLevel.InstancesBuffer), instance_handle());
	}

	void SetInstanceBindingTableRecordOffset(AccelerationStructureHandle topLevel, BLASInstanceHandle instance_handle, const uint32 offset) {
		GAL::WriteInstanceBindingTableRecordOffset(offset, GetBufferPointer(accelerationStructures[topLevel()].TopLevel.InstancesBuffer), instance_handle());
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

	struct AccelerationStructureData {
		AccelerationStructureData() {}

		AccelerationStructureData(AccelerationStructureData&& other) : isTop(other.isTop), PrimitiveCount(other.PrimitiveCount) {
			if(isTop) {
				GTSL::Move(&other.TopLevel, &TopLevel);
			} else {
				GTSL::Move(&other.BottomLevel, &BottomLevel);				
			}
		}

		bool isTop = false;
		GTSL::uint32 PrimitiveCount = 0;

		union {
			struct TopLevelAccelerationStructure {
				AccelerationStructure AccelerationStructures[MAX_CONCURRENT_FRAMES];
				RenderAllocation AccelerationStructureAllocation[MAX_CONCURRENT_FRAMES];
				GPUBuffer AccelerationStructureBuffer[MAX_CONCURRENT_FRAMES];
				uint32 ScratchSize = 0;
				BufferHandle InstancesBuffer;
				GTSL::StaticVector<uint32, 8> freeSlots;
				uint32 Instances = 0;
				BufferHandle ScratchBuffer;
			} TopLevel;

			struct BottomLevelAccelerationStructure {
				GPUBuffer AccelerationStructureBuffer;
				RenderAllocation AccelerationStructureAllocation;
				AccelerationStructure AccelerationStructure;
				uint32 ScratchSize;
				BufferHandle DataBuffer;
				GTSL::uint32 VertexCount, VertexSize;
			} BottomLevel;
		};

		~AccelerationStructureData() {
			if(isTop) {
				GTSL::Destroy(TopLevel);
			} else {
				GTSL::Destroy(BottomLevel);
			}
		}
	};
	GTSL::FixedVector<AccelerationStructureData, BE::PAR> accelerationStructures;

	struct CommandListData {
		CommandList CommandList;
		Fence Fence;
		GPUSemaphore Semaphore;
	};
	GTSL::StaticVector<CommandListData, 8> commandLists;

	GAL::Device accelerationStructureBuildDevice;
	
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
