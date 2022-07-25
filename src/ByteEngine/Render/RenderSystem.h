#pragma once

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Game/System.hpp"

#include "RendererAllocator.h"
#include "RenderTypes.h"

#include <GAL/Vulkan/VulkanQueue.h>

#include <GTSL/Bitfield.h>
#include <GTSL/Pair.hpp>

#include "ByteEngine/Application/WindowSystem.hpp"

namespace GTSL {
	class Window;
}

class RenderSystem : public BE::System {
public:
	MAKE_HANDLE(uint32, Buffer)
	MAKE_HANDLE(uint32, Texture);
	
	explicit RenderSystem(const InitializeInfo& initializeInfo);
	~RenderSystem();
	[[nodiscard]] uint8 GetCurrentFrame() const { return currentFrameIndex; }
	[[nodiscard]] uint8 GetFrameIndex(int32 frameDelta) const { return static_cast<uint8>(frameDelta % pipelinedFrames); }
	uint8 GetPipelinedFrames() const { return pipelinedFrames; }
	GAL::FormatDescriptor GetSwapchainFormat() const { return swapchainFormat; }
	TaskHandle<GTSL::Extent2D> GetResizeHandle() const { return resizeHandle; }

	GTSL::Range<byte*> GetBufferRange(BufferHandle buffer_handle) const {
		const auto& buffer = buffers[buffer_handle()];
		return { buffer.Size, (byte*)buffer.Allocation.Data };
	}

	MAKE_HANDLE(uint32, CommandList);
	MAKE_HANDLE(uint32, Workload);
	MAKE_HANDLE(uint32, AccelerationStructure);
	MAKE_HANDLE(uint32, BLASInstance);

	struct WorkloadData {
		GTSL::StaticVector<CommandListHandle, 16> AssociatedCommandlists;
		Synchronizer Fence, Semaphore;
		GAL::PipelineStage PipelineStages;
	};
	GTSL::FixedVector<WorkloadData, BE::PAR> workloads;

	WorkloadHandle CreateWorkload(GTSL::StringView name, GAL::QueueType type, GAL::PipelineStage pipeline_stages) {
		uint32 index = workloads.Emplace();
		auto& workload = workloads[index];
		workload.Fence.Initialize(GetRenderDevice(), name, Synchronizer::Type::FENCE);
		workload.Semaphore.Initialize(GetRenderDevice(), name, Synchronizer::Type::SEMAPHORE);

		workload.PipelineStages = pipeline_stages;

		return WorkloadHandle(index);
	}

	CommandListHandle CreateCommandList(const GTSL::StringView name, GAL::QueueType type, GAL::PipelineStage pipeline_stages, bool isSingleFrame = true) {
		uint32 index = commandLists.GetLength();
		auto& commandList = commandLists.EmplaceBack(GetPersistentAllocator());
		//commandList.Fence.Initialize(GetRenderDevice(), true);
		commandList.Semaphore.Initialize(GetRenderDevice(), name, GAL::VulkanSynchronizer::Type::SEMAPHORE);
		commandList.Fence.Initialize(GetRenderDevice(), name, GAL::VulkanSynchronizer::Type::FENCE);
		commandList.Operations = type;
		commandList.PipelineStages = pipeline_stages;

		if (type & GAL::QueueTypes::GRAPHICS) {
			commandList.CommandList.Initialize(GetRenderDevice(), name, graphicsQueue.GetQueueKey(), !isSingleFrame);
		}

		if (type & GAL::QueueTypes::COMPUTE) {
			commandList.CommandList.Initialize(GetRenderDevice(), name, computeQueue.GetQueueKey(), !isSingleFrame);
		}

		if (type & GAL::QueueTypes::TRANSFER) {
			commandList.CommandList.Initialize(GetRenderDevice(), name, transferQueue.GetQueueKey(), !isSingleFrame);
		}

		return CommandListHandle(index);
	}

	void StartCommandList(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];

		if(commandListData.Fence.State()) {
			commandListData.Fence.Wait(GetRenderDevice());
			commandListData.Fence.Reset(GetRenderDevice());
		}

		commandListData.CommandList.BeginRecording(GetRenderDevice());

		{
			GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> barriers(commandListData.bufferCopyDatas.GetLength(), GetTransientAllocator());

			for (auto& e : commandListData.bufferCopyDatas) {
				commandListData.CommandList.CopyBuffer(GetRenderDevice(), buffers[e.SourceBufferHandle()].Buffer, e.SourceOffset, buffers[e.DestinationBufferHandle()].Buffer, e.DestinationOffset, buffers[e.SourceBufferHandle()].Size);

				//auto& barrier = barriers.EmplaceBack(GAL::PipelineStages::TRANSFER, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::BufferBarrier{ &buffer.Buffer, buffer.Size });
			}

			commandListData.CommandList.AddPipelineBarrier(GetRenderDevice(), barriers, GetTransientAllocator());

			commandListData.bufferCopyDatas.Resize(0);
		}

		if (auto& textureCopyData = commandListData.textureCopyDatas; textureCopyData) {
			GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
			GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());

			for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
				sourceTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, commandListData.PipelineStages, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format });
				destinationTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, commandListData.PipelineStages, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, textureCopyData[i].Format });
			}

			commandListData.CommandList.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GetTransientAllocator());

			for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
				commandListData.CommandList.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
			}

			commandListData.CommandList.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GetTransientAllocator());
		}

		commandListData.textureCopyDatas.Resize(0);
	}

	void DispatchBuild(const CommandListHandle command_list_handle, const GTSL::Range<const AccelerationStructureHandle*> handles) {
		if(!handles.ElementCount()) { return; }

		auto& commandListData = commandLists[command_list_handle()];

		GTSL::StaticVector<GAL::AccelerationStructureBuildInfo, 8> buildDatas;
		GTSL::StaticVector<GAL::Geometry, 8> geometries;

		for (auto handle : handles) {
			auto& buildData = buildDatas.EmplaceBack();

			if (accelerationStructures[handle()].isTop) {
				const auto& as = accelerationStructures[handle()];
				auto& tlas = accelerationStructures[handle()].TopLevel;

				buildData.DestinationAccelerationStructure = tlas.AccelerationStructures;
				buildData.ScratchBufferAddress = GetBufferAddress(as.ScratchBuffer);

				commandListData.CommandList.CopyBuffer(GetRenderDevice(), buffers[tlas.SourceInstancesBuffer()].Buffer, buffers[tlas.DestinationInstancesBuffer()].Buffer, GetBufferSize(tlas.DestinationInstancesBuffer));

				geometries.EmplaceBack(GAL::GeometryInstances{ GetBufferAddress(tlas.DestinationInstancesBuffer) }, GAL::GeometryFlag(), as.PrimitiveCount, 0);

				buildData.Geometries = geometries;
			} else {
				const auto& as = accelerationStructures[handle()];
				const auto& blas = accelerationStructures[handle()].BottomLevel;

				buildData.DestinationAccelerationStructure = blas.AccelerationStructure;
				buildData.ScratchBufferAddress = GetBufferAddress(as.ScratchBuffer);
				
				geometries.EmplaceBack(GAL::Geometry{ GAL::GeometryTriangles{ GAL::ShaderDataType::FLOAT3, GAL::IndexType::UINT16, static_cast<uint8>(blas.VertexSize), GetBufferAddress(blas.VertexBuffer) + blas.VertexByteOffset, GetBufferAddress(blas.IndexBuffer) + blas.IndexBufferByteOffset, 0, blas.VertexCount }, GAL::GeometryFlags::OPAQUE, as.PrimitiveCount, 0 });

				buildData.Geometries = geometries;
			}
		}

		switch (accelerationStructureBuildDevice) {
		case GAL::Device::CPU: break;
		case GAL::Device::GPU:
		case GAL::Device::GPU_OR_CPU: {
			commandListData.CommandList.BuildAccelerationStructure(GetRenderDevice(), buildDatas, GetTransientAllocator());
			break;
		}
		default:;
		}

		GTSL::StaticVector<CommandList::BarrierData, 1> barriers;
		barriers.EmplaceBack(GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::PipelineStages::RAY_TRACING, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::MemoryBarrier{});
		commandListData.CommandList.AddPipelineBarrier(GetRenderDevice(), barriers, GetTransientAllocator());
	}


	void StagingCopy(const CommandListHandle command_list, const BufferHandle handle) {
		commandLists[command_list()].CommandList.CopyBuffer(GetRenderDevice(), buffers[handle()].Buffer, buffers[handle()].Buffer, buffers[handle()].Size);
	}

	void EndCommandList(const CommandListHandle command_list_handle) {
		auto& commandListData = commandLists[command_list_handle()];
		commandListData.CommandList.EndRecording(GetRenderDevice());
	}

	void Wait(WorkloadHandle workload_handle) {
		auto& workloadData = workloads[workload_handle()];

		if (workloadData.Fence.State()) {
			workloadData.Fence.Wait(GetRenderDevice());
			workloadData.Fence.Reset(GetRenderDevice());
		}
	}

	struct WorkUnit {
		GTSL::Range<const CommandListHandle*> CommandListHandles;
		GTSL::Range<const WorkloadHandle*> WaitWorkloadHandles, SignalWorkloadHandles;
	};

	void Submit(const GAL::QueueType queue_type, const GTSL::Range<const WorkUnit*> work_units, const WorkloadHandle workload_handle) {
		GTSL::StaticVector<Queue::WorkUnit<Synchronizer>, 8> workUnits;
		GTSL::StaticVector<GTSL::StaticVector<const GAL::CommandList*, 8>, 4> command_listses;
		GTSL::StaticVector<GTSL::StaticVector<Queue::WorkUnit<Synchronizer>::SynchronizerOperationInfo, 8>, 4> waitOperations, signalOperations;

		for (uint32 wui = 0; wui < work_units.ElementCount(); ++wui) {
			auto& wu = work_units[wui];
			auto& workUnit = workUnits.EmplaceBack(); auto& cl = command_listses.EmplaceBack(); auto& wo = waitOperations.EmplaceBack(); auto& so = signalOperations.EmplaceBack();

			for(auto& e : wu.WaitWorkloadHandles) {
				auto& workload = workloads[e()];
				wo.EmplaceBack(&workload.Semaphore, workload.PipelineStages);
			}

			for (auto& e : wu.SignalWorkloadHandles) {
				auto& workload = workloads[e()];
				so.EmplaceBack(&workload.Semaphore, workload.PipelineStages);
			}

			for (auto& e : wu.CommandListHandles) {
				auto& c = commandLists[e()];
				cl.EmplaceBack(&c.CommandList);
			}

			workUnit.CommandLists = cl;
			workUnit.Signal = so;
			workUnit.Wait = wo;
		}

		auto& workload = workloads[workload_handle()];

		if (queue_type & GAL::QueueTypes::GRAPHICS) {
			graphicsQueue.Submit(GetRenderDevice(), workUnits, workload.Fence);
		}

		if (queue_type & GAL::QueueTypes::COMPUTE) {
			computeQueue.Submit(GetRenderDevice(), workUnits, workload.Fence);
		}

		if(queue_type & GAL::QueueTypes::TRANSFER) {
			transferQueue.Submit(GetRenderDevice(), workUnits, workload.Fence);
		}
	}

	void Present(WindowSystem* window_system, const GTSL::Range<const WorkloadHandle*> wait_workload_handles) {
		GTSL::StaticVector<Synchronizer*, 8> waitSemaphores;

		for(auto e : wait_workload_handles) {
			waitSemaphores.EmplaceBack(&workloads[e()].Semaphore);
		}

		if (surface.GetHandle()) {
			if (!renderContext.Present(GetRenderDevice(), waitSemaphores, imageIndex, graphicsQueue)) {
				resize(window_system);
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
		return buffers[buffer_handle()].Buffer;
		//TODO: is multi
	}
	//CommandList* GetTransferCommandBuffer() { return &transferCommandBuffers[currentFrameIndex]; }
	
	void AddBufferUpdate(CommandListHandle command_list_handle, const BufferHandle source_buffer_handle, const BufferHandle destination_buffer_handle, uint32 source_offset = 0, uint32 destination_offset = 0) {
		auto& commandList = commandLists[command_list_handle()];
		if(needsStagingBuffer)
			commandList.bufferCopyDatas.EmplaceBack(source_buffer_handle, destination_buffer_handle, source_offset, destination_offset);
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
	void AddTextureCopy(CommandListHandle command_list_handle, const TextureCopyData& textureCopyData) {
		BE_ASSERT(testMutex.TryLock());
		auto& commandList = commandLists[command_list_handle()];
		commandList.textureCopyDatas.EmplaceBack(textureCopyData);
		testMutex.Unlock();
	}

	[[nodiscard]] PipelineCache GetPipelineCache() const;

	[[nodiscard]] const Texture* GetSwapchainTexture() const { return &swapchainTextures[imageIndex]; }

	[[nodiscard]] byte* GetBufferPointer(BufferHandle bufferHandle) const {
		return static_cast<byte*>(buffers[bufferHandle()].Allocation.Data);
	}

	[[nodiscard]] GAL::DeviceAddress GetBufferAddress(BufferHandle bufferHandle) const {
		return buffers[bufferHandle()].Addresses;
	}

	uint32 GetBufferSize(const BufferHandle buffer_handle) const {
		return buffers[buffer_handle()].Size;
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

	AccelerationStructure GetTopLevelAccelerationStructure(AccelerationStructureHandle topLevelAccelerationStructureIndex) const {
		return accelerationStructures[topLevelAccelerationStructureIndex()].TopLevel.AccelerationStructures;
	}

	GAL::DeviceAddress GetTopLevelAccelerationStructureAddress(AccelerationStructureHandle topLevelAccelerationStructureIndex) const {
		return accelerationStructures[topLevelAccelerationStructureIndex()].TopLevel.AccelerationStructures.GetAddress(GetRenderDevice());
	}

	uint32 GetBufferSubDataAlignment() const { return renderDevice.GetStorageBufferBindingOffsetAlignment(); }

	[[nodiscard]] TextureHandle CreateTexture(GTSL::Range<const char8_t*> name, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable, TextureHandle texture_handle = TextureHandle());

	void UpdateTexture(const CommandListHandle command_list_handle, const TextureHandle textureHandle);
	
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

	GTSL::Result<GTSL::Extent2D> AcquireImage(const WorkloadHandle workload_handle, WindowSystem* window_system);

	BufferHandle CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, const BufferHandle buffer_handle);

	AccelerationStructureHandle CreateTopLevelAccelerationStructure(uint32 estimatedMaxInstances) {
		uint32 tlasi = accelerationStructures.Emplace(true);
		auto& as = accelerationStructures[tlasi];
		auto& t = accelerationStructures[tlasi].TopLevel;

		GAL::Geometry geometry(GAL::GeometryInstances(), GAL::GeometryFlag(), estimatedMaxInstances, 0);

		uint32 size;

		t.AccelerationStructures.GetMemoryRequirements(GetRenderDevice(), GTSL::Range(1, &geometry), accelerationStructureBuildDevice, GAL::AccelerationStructureFlags::PREFER_FAST_TRACE, &size, &as.ScratchSize);

		AllocateLocalBufferMemory(size, GAL::BufferUses::ACCELERATION_STRUCTURE, &t.AccelerationStructureBuffer, &t.AccelerationStructureAllocation);
		t.AccelerationStructures.Initialize(&renderDevice, true, t.AccelerationStructureBuffer, size, 0);

		t.SourceInstancesBuffer = CreateBuffer(64 * estimatedMaxInstances, GAL::BufferUses::BUILD_INPUT_READ, true, t.SourceInstancesBuffer);
		t.DestinationInstancesBuffer = CreateBuffer(64 * estimatedMaxInstances, GAL::BufferUses::BUILD_INPUT_READ, false, t.DestinationInstancesBuffer);
		as.ScratchBuffer = CreateBuffer(1024 * 1204, GAL::BufferUses::BUILD_INPUT_READ | GAL::BufferUses::STORAGE, false, as.ScratchBuffer);

		return AccelerationStructureHandle{ tlasi };
	}

	AccelerationStructureHandle CreateBottomLevelAccelerationStructure(uint32 vertexCount, uint32 vertexSize, uint32 indexCount, GAL::IndexType indexType,  BufferHandle vertex_buffer_handle, BufferHandle index_buffer_handle, uint32 vertex_buffer_byte_offset = 0, uint32 index_buffer_byte_offset = 0, bool willUpdate = false, bool willRebuild = false, bool isOpaque = true) {
		uint32 blasi = accelerationStructures.Emplace(false);

		auto& as = accelerationStructures[blasi];
		auto& blas = accelerationStructures[blasi].BottomLevel;

		blas.VertexCount = vertexCount; blas.VertexSize = vertexSize; blas.VertexBuffer = vertex_buffer_handle; blas.IndexBuffer = index_buffer_handle;
		as.PrimitiveCount = indexCount / 3; blas.VertexByteOffset = vertex_buffer_byte_offset; blas.IndexBufferByteOffset = index_buffer_byte_offset;

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
		blas.AccelerationStructure.GetMemoryRequirements(GetRenderDevice(), GTSL::Range(1, &geometry), GAL::Device::GPU, acceleration_structure_flag, &bufferSize, &as.ScratchSize);
		AllocateLocalBufferMemory(bufferSize, GAL::BufferUses::ACCELERATION_STRUCTURE, &blas.AccelerationStructureBuffer, &blas.AccelerationStructureAllocation);
		blas.AccelerationStructure.Initialize(GetRenderDevice(), false, blas.AccelerationStructureBuffer, bufferSize, 0);

		as.ScratchBuffer = CreateBuffer(1024 * 1204, GAL::BufferUses::BUILD_INPUT_READ | GAL::BufferUses::STORAGE, true, as.ScratchBuffer);

		return AccelerationStructureHandle{ blasi };
	}

	uint32 CreateAABB(const GTSL::Matrix4& position, const GTSL::Vector3 size) {
		//auto volume = CreateBuffer(sizeof(float32) * 6, GAL::BufferUses::BUILD_INPUT_READ, true, false);
		//auto bufferDeviceAddress = GetBufferAddress(volume);
		//auto bufferPointer = GetBufferPointer(volume);
		//
		//*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 0) = -size;
		//*(reinterpret_cast<GTSL::Vector3*>(bufferPointer) + 1) = size;

		//addRayTracingInstance(GAL::Geometry(GAL::GeometryAABB(bufferDeviceAddress, sizeof(float32) * 6), {}, 1, 0), AccelerationStructureBuildData{ 0,  {}, {} });
		return 0;
	}

	BLASInstanceHandle AddBLASToTLAS(const AccelerationStructureHandle tlash, const AccelerationStructureHandle blash, uint32 instance_custom_index, BLASInstanceHandle instance_handle) {
		auto& tlas = accelerationStructures[tlash()].TopLevel;

		uint32 instanceIndex = 0;

		if (instance_handle) {
			instanceIndex = instance_handle();
		} else {
			if (tlas.freeSlots) {
				instanceIndex = tlas.freeSlots.back();
			}
			else {
				instanceIndex = accelerationStructures[tlash()].PrimitiveCount++;
				//TODO: check need resize
			}
		}

		if (blash) {
			const auto& blas = accelerationStructures[blash()].BottomLevel;
			GAL::WriteInstance(blas.AccelerationStructure, instanceIndex, GAL::GeometryFlags::OPAQUE, GetRenderDevice(), GetBufferPointer(tlas.SourceInstancesBuffer), instance_custom_index, accelerationStructureBuildDevice);
		}

		return BLASInstanceHandle(instanceIndex);
	}

#define BE_LOG_IF(cond, text) if(cond) { BE_LOG_WARNING(text); return; }

	void SetInstancePosition(AccelerationStructureHandle topLevel, BLASInstanceHandle instance_handle, const GTSL::Matrix3x4& matrix4) {
		BE_LOG_IF(!static_cast<bool>(instance_handle), u8"TlAS instance handle is invalid.");
		GAL::WriteInstanceMatrix(matrix4, GetBufferPointer(accelerationStructures[topLevel()].TopLevel.SourceInstancesBuffer), instance_handle());
	}

	void SetAccelerationStructureInstanceIndex(AccelerationStructureHandle topLevel, BLASInstanceHandle instance_handle, uint32 custom_index) {
		GAL::WriteInstanceIndex(custom_index, GetBufferPointer(accelerationStructures[topLevel()].TopLevel.SourceInstancesBuffer), instance_handle());
	}

	void SetInstanceBindingTableRecordOffset(AccelerationStructureHandle topLevel, BLASInstanceHandle instance_handle, const uint32 offset) {
		BE_LOG_IF(instance_handle, u8"TlAS instance handle is invalid.");
		GAL::WriteInstanceBindingTableRecordOffset(offset, GetBufferPointer(accelerationStructures[topLevel()].TopLevel.SourceInstancesBuffer), instance_handle());
	}

private:	
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
		BufferHandle SourceBufferHandle, DestinationBufferHandle; uint32 SourceOffset = 0, DestinationOffset = 0;
	};
	
	Texture swapchainTextures[MAX_CONCURRENT_FRAMES];
	TextureView swapchainTextureViews[MAX_CONCURRENT_FRAMES];
	
	GAL::VulkanQueue graphicsQueue, computeQueue, transferQueue;

	bool breakOnError = false;
	TaskHandle<GTSL::Extent2D> resizeHandle;

	struct BufferData {
		uint32 Size = 0, Counter = 0;
		GAL::BufferUse Flags;
		uint32 references = 0;
		GPUBuffer Buffer;
		RenderAllocation Allocation;
		GAL::DeviceAddress Addresses;
	};
	GTSL::FixedVector<BufferData, BE::PAR> buffers;

	struct AccelerationStructureData {
		bool isTop = false;
		GTSL::uint32 PrimitiveCount = 0;
		BufferHandle ScratchBuffer;
		uint32 ScratchSize;

		struct TopLevelAccelerationStructure {
			AccelerationStructure AccelerationStructures;
			RenderAllocation AccelerationStructureAllocation;
			GPUBuffer AccelerationStructureBuffer;

			BufferHandle SourceInstancesBuffer, DestinationInstancesBuffer;

			GTSL::StaticVector<uint32, 8> freeSlots;
		};

		struct BottomLevelAccelerationStructure {
			GPUBuffer AccelerationStructureBuffer;
			RenderAllocation AccelerationStructureAllocation;
			AccelerationStructure AccelerationStructure;
			BufferHandle VertexBuffer, IndexBuffer;
			GTSL::uint32 VertexCount, VertexSize;
			uint32 VertexByteOffset, IndexBufferByteOffset;
		};

		union {
			TopLevelAccelerationStructure TopLevel;
			BottomLevelAccelerationStructure BottomLevel;
		};

		AccelerationStructureData(bool isTopLevel) : isTop(isTopLevel) {
			if (isTop) {
				::new(&TopLevel) TopLevelAccelerationStructure();
			} else {
				::new(&BottomLevel) BottomLevelAccelerationStructure();
			}
		}

		AccelerationStructureData(AccelerationStructureData&& other) : isTop(other.isTop), PrimitiveCount(other.PrimitiveCount) {
			if(isTop) {
				GTSL::Move(&other.TopLevel, &TopLevel);
			} else {
				GTSL::Move(&other.BottomLevel, &BottomLevel);				
			}
		}

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
		CommandListData(const BE::PAR& allocator) : bufferCopyDatas(allocator), textureCopyDatas(allocator) {}

		CommandList CommandList;
		//Fence Fence;
		Synchronizer Semaphore, Fence;
		GAL::QueueType Operations;
		GAL::PipelineStage PipelineStages;
		GTSL::Vector<BufferCopyData, BE::PersistentAllocatorReference> bufferCopyDatas;
		GTSL::Vector<TextureCopyData, BE::PersistentAllocatorReference> textureCopyDatas;
	};
	GTSL::StaticVector<CommandListData, 8> commandLists;

	GAL::Device accelerationStructureBuildDevice;
	
	uint8 currentFrameIndex = 0;

	GAL::PresentModes swapchainPresentMode;
	GAL::FormatDescriptor swapchainFormat;
	GAL::ColorSpaces swapchainColorSpace;

	void resize(WindowSystem* window_system);
	
	void renderFlush(TaskInfo taskInfo);

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
};

class RenderManager : public BE::System
{
public:
	RenderManager(const InitializeInfo& initializeInfo, const char8_t* name) : System(initializeInfo, name) {}

	struct SetupInfo {
		ApplicationManager* GameInstance;
		RenderSystem* RenderSystem;
		//RenderState* RenderState;
		GTSL::Matrix4 ViewMatrix, ProjectionMatrix;
	};
};