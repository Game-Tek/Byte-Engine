#include "RenderSystem.h"

#include <GTSL/Window.h>

#include "MaterialSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"

#undef MemoryBarrier

class CameraSystem;
class RenderStaticMeshCollection;

PipelineCache RenderSystem::GetPipelineCache() const { return pipelineCaches[GTSL::Thread::ThisTreadID()]; }

RenderSystem::MeshHandle RenderSystem::CreateRayTracedMesh(const CreateRayTracingMeshInfo& info)
{
	auto& localMesh = meshes[info.SharedMesh()]; BE_ASSERT(localMesh.MeshAllocation.Data, "!");

	auto verticesSize = info.VertexCount * info.VertexSize; auto indecesSize = info.IndexCount * info.IndexSize;
	auto meshSize = GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment()) + indecesSize;

	Mesh mesh; RayTracingMesh rayTracingMesh;
	
	mesh.VertexSize = info.VertexSize; mesh.VertexCount = info.VertexCount;	mesh.IndexSize = info.IndexSize; mesh.IndicesCount = info.IndexCount;
	
	{
		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = GetRenderDevice();
	
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Render System. RayTraced Mesh Buffer");
			createInfo.Name = name;
		}
	
		createInfo.Size = meshSize;
		createInfo.BufferType = BufferType::TRANSFER_DESTINATION | BufferType::BUILD_INPUT_READ_ONLY | BufferType::STORAGE | BufferType::ADDRESS;
	
		BufferLocalMemoryAllocationInfo bufferLocal;
		bufferLocal.Buffer = &mesh.Buffer;
		bufferLocal.Allocation = &mesh.MeshAllocation;
		bufferLocal.CreateInfo = &createInfo;
		AllocateLocalBufferMemory(bufferLocal);
	}

	auto meshDataBuffer = localMesh.Buffer;

	uint32 scratchSize;
	
	{
		AccelerationStructure::GeometryTriangles geometryTriangles;
		geometryTriangles.IndexType = SelectIndexType(info.IndexSize);
		geometryTriangles.VertexFormat = ShaderDataType::FLOAT3;
		geometryTriangles.MaxVertices = info.VertexCount;
		geometryTriangles.TransformData = 0;
		geometryTriangles.VertexData = meshDataBuffer.GetAddress(GetRenderDevice());
		geometryTriangles.IndexData = meshDataBuffer.GetAddress(GetRenderDevice()) + GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment());
		geometryTriangles.VertexStride = info.VertexSize;
		geometryTriangles.FirstVertex = 0;
		
		AccelerationStructure::Geometry geometry;
		geometry.Flags = GeometryFlags::OPAQUE;
		geometry.SetGeometryTriangles(geometryTriangles);
		geometry.PrimitiveCount = mesh.IndicesCount / 3;
		geometry.PrimitiveOffset = 0;
		geometries.EmplaceBack(geometry);

		AccelerationStructure::CreateInfo accelerationStructureCreateInfo;
		accelerationStructureCreateInfo.RenderDevice = GetRenderDevice();
		if constexpr (_DEBUG) { accelerationStructureCreateInfo.Name = GTSL::StaticString<64>("Render Device. Bottom Acceleration Structure"); }
		accelerationStructureCreateInfo.Geometries = GTSL::Range<AccelerationStructure::Geometry*>(1, &geometry);
		accelerationStructureCreateInfo.DeviceAddress = 0;
		accelerationStructureCreateInfo.Offset = 0;
		
		AllocateAccelerationStructureMemory(&rayTracingMesh.AccelerationStructure, &rayTracingMesh.StructureBuffer, 
			GTSL::Range<const AccelerationStructure::Geometry*>(1, &geometry), &accelerationStructureCreateInfo,
			&rayTracingMesh.StructureBufferAllocation, BuildType::GPU_LOCAL, &scratchSize);
	}

	{
		BufferCopyData bufferCopyData;
		bufferCopyData.SourceOffset = 0;
		bufferCopyData.DestinationOffset = 0;
		bufferCopyData.SourceBuffer = localMesh.Buffer;
		bufferCopyData.DestinationBuffer = mesh.Buffer;
		bufferCopyData.Size = meshSize;
		bufferCopyData.Allocation = localMesh.MeshAllocation;
		AddBufferCopy(bufferCopyData);
	}
	
	{
		AccelerationStructureBuildData buildData;
		buildData.ScratchBuildSize = scratchSize;
		buildData.Destination = rayTracingMesh.AccelerationStructure;
		buildDatas.EmplaceBack(buildData);
	}

	mesh.DerivedTypeIndex = rayTracingMeshes.Emplace(rayTracingMesh);
	auto meshHandle = addMesh(mesh);

	auto accelerationStructureAddress = rayTracingMesh.AccelerationStructure.GetAddress(GetRenderDevice());
	
	for(uint8 f = 0; f < pipelinedFrames; ++f)
	{
		auto& instance = *(static_cast<AccelerationStructure::Instance*>(instancesAllocation[f].Data) + mesh.DerivedTypeIndex);
		
		instance.Flags = GeometryInstanceFlags::OPAQUE;
		instance.AccelerationStructureAddress = accelerationStructureAddress;
		instance.Mask = 0xFF;
		instance.InstanceIndex = meshHandle();
		instance.InstanceShaderBindingTableRecordOffset = 0;
		instance.Transform = *info.Matrix;
	}
	
	BE_ASSERT(mesh.DerivedTypeIndex < MAX_INSTANCES_COUNT);
	++rayTracingInstancesCount;
	
	return meshHandle;
}

RenderSystem::MeshHandle RenderSystem::CreateMesh(Id name, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, MaterialInstanceHandle materialHandle)
{
	Mesh mesh;

	mesh.MaterialHandle = materialHandle;
	mesh.VertexSize = vertexSize; mesh.VertexCount = vertexCount; mesh.IndexSize = indexSize; mesh.IndicesCount = indexCount;

	auto verticesSize = vertexCount * vertexSize; auto indecesSize = indexCount * indexSize;
	auto meshSize = GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment()) + indecesSize;
	
	Buffer::CreateInfo createInfo;
	createInfo.RenderDevice = GetRenderDevice();
	createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::ADDRESS | BufferType::TRANSFER_SOURCE | BufferType::BUILD_INPUT_READ_ONLY | BufferType::STORAGE;
	createInfo.Size = meshSize;
	
	AllocateScratchBufferMemory(meshSize, &mesh.Buffer, createInfo, &mesh.MeshAllocation);

	return addMesh(mesh);
}

RenderSystem::MeshHandle RenderSystem::UpdateMesh(MeshHandle meshHandle)
{
	if(needsStagingBuffer)
	{
		auto& sharedMesh = meshes[meshHandle()];
		//TODO: keep mesh index, don't create another one, and simply queue the copy on another list, to avoid moving so much data aound.
		// Remember material system also has to update buffers and descriptor in consequence
		Mesh mesh(sharedMesh);		
		
		auto verticesSize = mesh.VertexSize * mesh.VertexCount; auto indecesSize = mesh.IndexSize * mesh.IndicesCount;
		auto meshSize = GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment()) + indecesSize;

		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = GetRenderDevice();
		createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::ADDRESS | BufferType::TRANSFER_DESTINATION | BufferType::STORAGE;
		createInfo.Size = meshSize;

		BufferLocalMemoryAllocationInfo bufferLocal;
		bufferLocal.CreateInfo = &createInfo;
		bufferLocal.Allocation = &mesh.MeshAllocation;
		bufferLocal.Buffer = &mesh.Buffer;
		AllocateLocalBufferMemory(bufferLocal);

		BufferCopyData bufferCopyData;
		bufferCopyData.Size = meshSize;
		bufferCopyData.DestinationBuffer = mesh.Buffer;
		bufferCopyData.DestinationOffset = 0;
		bufferCopyData.SourceBuffer = sharedMesh.Buffer;
		bufferCopyData.SourceOffset = 0;
		AddBufferCopy(bufferCopyData);

		//TODO: QUEUE STAGING BUFFER AND MEMORY DELETION
		
		return addMesh(mesh);
	}
	else //doesn't need staging buffer, has resizable BAR, is integrated graphics, etc
	{
		
	}
}

void RenderSystem::RenderMesh(MeshHandle handle, const uint32 instanceCount)
{
	auto& mesh = meshes[handle()];

	graphicsCommandBuffers[GetCurrentFrame()].BindVertexBuffer(GetRenderDevice(), mesh.Buffer, 0);
	graphicsCommandBuffers[GetCurrentFrame()].BindIndexBuffer(GetRenderDevice(), mesh.Buffer, GTSL::Math::RoundUpByPowerOf2(mesh.VertexSize * mesh.VertexCount, GetBufferSubDataAlignment()), SelectIndexType(mesh.IndexSize));
	graphicsCommandBuffers[GetCurrentFrame()].DrawIndexed(GetRenderDevice(), mesh.IndicesCount, instanceCount);
}

void RenderSystem::SetMeshMatrix(const MeshHandle meshHandle, const GTSL::Matrix4& matrix)
{
	const auto& mesh = meshes[meshHandle()];
	auto& instance = *(static_cast<AccelerationStructure::Instance*>(instancesAllocation[GetCurrentFrame()].Data) + mesh.DerivedTypeIndex);
	instance.Transform = GTSL::Matrix3x4(matrix);
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	//{
	//	GTSL::Array<TaskDependency, 1> dependencies{ { "RenderSystem", AccessTypes::READ_WRITE } };
	//	
	//	auto renderEnableHandle = initializeInfo.GameInstance->StoreDynamicTask("RS::OnRenderEnable", Task<bool>::Create<RenderSystem, &RenderSystem::OnRenderEnable>(this), dependencies);
	//	initializeInfo.GameInstance->SubscribeToEvent("Application", GameApplication::GetOnFocusGainEventHandle(), renderEnableHandle);
	//	
	//	auto renderDisableHandle = initializeInfo.GameInstance->StoreDynamicTask("RS::OnRenderDisable", Task<bool>::Create<RenderSystem, &RenderSystem::OnRenderDisable>(this), dependencies);
	//	initializeInfo.GameInstance->SubscribeToEvent("Application", GameApplication::GetOnFocusGainEventHandle(), renderDisableHandle);
	//}

	{

		const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessTypes::READ_WRITE } };
		initializeInfo.GameInstance->AddTask("frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, "FrameStart", "RenderStart");
		initializeInfo.GameInstance->AddTask("executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, "GameplayEnd", "RenderStart");
		initializeInfo.GameInstance->AddTask("renderStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderStart>(this), actsOn, "RenderStart", "RenderStartSetup");
		initializeInfo.GameInstance->AddTask("renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderBegin>(this), actsOn, "RenderEndSetup", "RenderDo");
		initializeInfo.GameInstance->AddTask("renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFinish>(this), actsOn, "RenderFinished", "RenderEnd");
	}
	
	//apiAllocations.Initialize(128, GetPersistentAllocator());
	apiAllocations.reserve(16);

	rayTracingMeshes.Initialize(32, GetPersistentAllocator());
	geometries.Initialize(32, GetPersistentAllocator());
	meshes.Initialize(32, GetPersistentAllocator());
	addedMeshes.Initialize(32, GetPersistentAllocator());

	textures.Initialize(32, GetPersistentAllocator());

	RenderDevice::RayTracingCapabilities rayTracingCapabilities;

	pipelinedFrames = BE::Application::Get()->GetOption("buffer");
	pipelinedFrames = GTSL::Math::Clamp(pipelinedFrames, (uint8)2, (uint8)3);
	bool rayTracing = BE::Application::Get()->GetOption("rayTracing");

	{
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>(BE::Application::Get()->GetApplicationName());
		createInfo.ApplicationVersion[0] = 0; createInfo.ApplicationVersion[1] = 0; createInfo.ApplicationVersion[2] = 0;

		createInfo.Debug = BE::Application::Get()->GetOption("debug");

		GTSL::Array<GAL::Queue::CreateInfo, 5> queue_create_infos(2);
		queue_create_infos[0].Capabilities = QueueCapabilities::GRAPHICS;
		queue_create_infos[0].QueuePriority = 1.0f;
		queue_create_infos[1].Capabilities = QueueCapabilities::TRANSFER;
		queue_create_infos[1].QueuePriority = 1.0f;
		createInfo.QueueCreateInfos = queue_create_infos;
		auto queues = GTSL::Array<Queue*, 5>{ &graphicsQueue, &transferQueue };
		createInfo.Queues = queues;

		GTSL::Array<GTSL::Pair<RenderDevice::Extension, void*>, 8> extensions{ { RenderDevice::Extension::PIPELINE_CACHE_EXTERNAL_SYNC, nullptr } };
		extensions.EmplaceBack(RenderDevice::Extension::SWAPCHAIN_RENDERING, nullptr);
		extensions.EmplaceBack(RenderDevice::Extension::SCALAR_LAYOUT, nullptr);
		if (rayTracing) { extensions.EmplaceBack(RenderDevice::Extension::RAY_TRACING, &rayTracingCapabilities); }

		createInfo.Extensions = extensions;
		createInfo.PerformanceValidation = true;
		createInfo.SynchronizationValidation = true;
		createInfo.DebugPrintFunction = GTSL::Delegate<void(const char*, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
		createInfo.AllocationInfo.UserData = this;
		createInfo.AllocationInfo.Allocate = GTSL::Delegate<void* (void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::allocateApiMemory>(this);
		createInfo.AllocationInfo.Reallocate = GTSL::Delegate<void* (void*, void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::reallocateApiMemory>(this);
		createInfo.AllocationInfo.Deallocate = GTSL::Delegate<void(void*, void*)>::Create<RenderSystem, &RenderSystem::deallocateApiMemory>(this);
		renderDevice.Initialize(createInfo);

		{
			needsStagingBuffer = true;

			auto memoryHeaps = renderDevice.GetMemoryHeaps(); GAL::VulkanRenderDevice::MemoryHeap& biggestGPUHeap = memoryHeaps[0];

			for (auto& e : memoryHeaps)
			{
				if (e.HeapType & GAL::MemoryType::GPU) {
					if (e.Size > biggestGPUHeap.Size) {
						biggestGPUHeap = e;

						for (auto& mt : e.MemoryTypes) {
							if (mt & GAL::MemoryType::GPU && mt & GAL::MemoryType::HOST_COHERENT && mt & GAL::MemoryType::HOST_VISIBLE) {
								needsStagingBuffer = false; break;
							}
						}
					}
				}
			}
		}

		scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
		localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());

		if (rayTracing)
		{
			buildDatas.Initialize(16, GetPersistentAllocator());

			AccelerationStructure::Geometry geometry;
			geometry.PrimitiveCount = MAX_INSTANCES_COUNT;
			geometry.Flags = 0; geometry.PrimitiveOffset = 0;
			geometry.SetGeometryInstances(AccelerationStructure::GeometryInstances{ 0 });

			AccelerationStructure::CreateInfo accelerationStructureCreateInfo;
			accelerationStructureCreateInfo.RenderDevice = GetRenderDevice();
			accelerationStructureCreateInfo.Geometries = GTSL::Range<const AccelerationStructure::Geometry*>(1, &geometry);

			for (uint32 i = 0; i < pipelinedFrames; ++i)
			{
				AllocateAccelerationStructureMemory(&topLevelAccelerationStructure[i], &topLevelAccelerationStructureBuffer[i],
					GTSL::Range<const AccelerationStructure::Geometry*>(1, &geometry), &accelerationStructureCreateInfo, &topLevelAccelerationStructureAllocation[i],
					BuildType::GPU_LOCAL, &topLevelStructureScratchSize);

				Buffer::CreateInfo buffer;
				buffer.RenderDevice = GetRenderDevice();
				buffer.Size = MAX_INSTANCES_COUNT * sizeof(AccelerationStructure::Instance);
				buffer.BufferType = BufferType::ADDRESS;

				AllocateScratchBufferMemory(MAX_INSTANCES_COUNT * sizeof(AccelerationStructure::Instance), &instancesBuffer[i], buffer, &instancesAllocation[i]);
			}

			{
				Buffer::CreateInfo buffer;
				buffer.RenderDevice = GetRenderDevice();
				buffer.Size = GTSL::Byte(GTSL::MegaByte(1));
				buffer.BufferType = BufferType::ADDRESS | BufferType::STORAGE;

				BufferLocalMemoryAllocationInfo allocationInfo;
				allocationInfo.Allocation = &scratchBufferAllocation;
				allocationInfo.CreateInfo = &buffer;
				allocationInfo.Buffer = &accelerationStructureScratchBuffer;
				AllocateLocalBufferMemory(allocationInfo);
			}

			shaderGroupAlignment = rayTracingCapabilities.ShaderGroupAlignment;
			shaderGroupHandleSize = rayTracingCapabilities.ShaderGroupHandleSize;
			scratchBufferOffsetAlignment = rayTracingCapabilities.ScratchBuildOffsetAlignment;
			shaderGroupBaseAlignment = rayTracingCapabilities.ShaderGroupBaseAlignment;

			if (rayTracingCapabilities.CanBuildOnHost)
			{
			}
			else
			{
				buildAccelerationStructures = decltype(buildAccelerationStructures)::Create<RenderSystem, &RenderSystem::buildAccelerationStructuresOnDevice>();
			}
		}
	}

	swapchainPresentMode = GAL::PresentModes::SWAP;
	swapchainColorSpace = ColorSpace::NONLINEAR_SRGB;
	swapchainFormat = TextureFormat::BGRA_I8;

	for (uint32 i = 0; i < pipelinedFrames; ++i)
	{
		{
			Semaphore::CreateInfo semaphoreCreateInfo;
			semaphoreCreateInfo.RenderDevice = GetRenderDevice();

			if constexpr (_DEBUG) { GTSL::StaticString<32> name("Transfer semaphore. Frame: "); name += i;  semaphoreCreateInfo.Name = name; }
			transferDoneSemaphores.EmplaceBack(semaphoreCreateInfo);
		}
		
		//processedTextureCopies.EmplaceBack(0);
		processedBufferCopies.EmplaceBack(0);

		Semaphore::CreateInfo semaphoreCreateInfo;
		semaphoreCreateInfo.RenderDevice = GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("ImageAvailableSemaphore #"); name += i;
			semaphoreCreateInfo.Name = name;
		}
		imageAvailableSemaphore.EmplaceBack(semaphoreCreateInfo);

		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("RenderFinishedSemaphore #"); name += i;
			semaphoreCreateInfo.Name = name;
		}
		renderFinishedSemaphore.EmplaceBack(semaphoreCreateInfo);

		Fence::CreateInfo fenceCreateInfo;
		fenceCreateInfo.RenderDevice = &renderDevice;
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("InFlightFence #"); name += i;
			fenceCreateInfo.Name = name;
		}

		fenceCreateInfo.IsSignaled = true;
		graphicsFences.EmplaceBack(fenceCreateInfo);
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("TrasferFence #"); name += i;
			fenceCreateInfo.Name = name;
		}
		transferFences.EmplaceBack(fenceCreateInfo);

		{
			CommandPool::CreateInfo commandPoolCreateInfo;
			commandPoolCreateInfo.RenderDevice = &renderDevice;
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> commandPoolName("Transfer command pool #"); commandPoolName += i;
				commandPoolCreateInfo.Name = commandPoolName;
			}
			commandPoolCreateInfo.Queue = &graphicsQueue;
			graphicsCommandPools.EmplaceBack(commandPoolCreateInfo);

			CommandPool::AllocateCommandBuffersInfo allocateCommandBuffersInfo;
			allocateCommandBuffersInfo.IsPrimary = true;
			allocateCommandBuffersInfo.RenderDevice = &renderDevice;

			CommandBuffer::CreateInfo commandBufferCreateInfo;
			commandBufferCreateInfo.RenderDevice = &renderDevice;
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> commandBufferName("Graphics command buffer #"); commandBufferName += i;
				commandBufferCreateInfo.Name = commandBufferName;
			}
			GTSL::Array<CommandBuffer::CreateInfo, 5> createInfos; createInfos.EmplaceBack(commandBufferCreateInfo);
			allocateCommandBuffersInfo.CommandBufferCreateInfos = createInfos;
			graphicsCommandBuffers.Resize(graphicsCommandBuffers.GetLength() + 1);
			allocateCommandBuffersInfo.CommandBuffers = GTSL::Range<CommandBuffer*>(1, graphicsCommandBuffers.begin() + i);
			graphicsCommandPools[i].AllocateCommandBuffer(allocateCommandBuffersInfo);
		}

		{

			CommandPool::CreateInfo commandPoolCreateInfo;
			commandPoolCreateInfo.RenderDevice = &renderDevice;
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> commandPoolName("Transfer command pool #"); commandPoolName += i;
				commandPoolCreateInfo.Name = commandPoolName;
			}
			commandPoolCreateInfo.Queue = &transferQueue;
			transferCommandPools.EmplaceBack(commandPoolCreateInfo);

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.RenderDevice = &renderDevice;
			allocate_command_buffers_info.IsPrimary = true;

			CommandBuffer::CreateInfo commandBufferCreateInfo;
			commandBufferCreateInfo.RenderDevice = &renderDevice;
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> commandBufferName("Transfer command buffer #"); commandBufferName += i;
				commandBufferCreateInfo.Name = commandBufferName;
			}
			GTSL::Array<CommandBuffer::CreateInfo, 5> createInfos; createInfos.EmplaceBack(commandBufferCreateInfo);
			allocate_command_buffers_info.CommandBufferCreateInfos = createInfos;
			transferCommandBuffers.Resize(transferCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Range<CommandBuffer*>(1, transferCommandBuffers.begin() + i);
			transferCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}

		bufferCopyDatas.EmplaceBack(64, GetPersistentAllocator());
		textureCopyDatas.EmplaceBack(64, GetPersistentAllocator());
	}

	bool pipelineCacheAvailable;
	auto* pipelineCacheManager = BE::Application::Get()->GetResourceManager<PipelineCacheResourceManager>("PipelineCacheResourceManager");
	pipelineCacheManager->DoesCacheExist(pipelineCacheAvailable);

	pipelineCaches.Initialize(BE::Application::Get()->GetNumberOfThreads(), GetPersistentAllocator());

	if (pipelineCacheAvailable)
	{
		uint32 cacheSize = 0;
		pipelineCacheManager->GetCacheSize(cacheSize);

		GTSL::Buffer<BE::TAR> pipelineCacheBuffer;
		pipelineCacheBuffer.Allocate(cacheSize, 32, GetTransientAllocator());

		pipelineCacheManager->GetCache(pipelineCacheBuffer);

		PipelineCache::CreateInfo pipelineCacheCreateInfo;
		pipelineCacheCreateInfo.RenderDevice = GetRenderDevice();
		pipelineCacheCreateInfo.ExternallySync = true;
		pipelineCacheCreateInfo.Data = pipelineCacheBuffer;
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i)
		{
			if constexpr (_DEBUG) {
				GTSL::StaticString<32> name("Pipeline cache. Thread: "); name += i;
				pipelineCacheCreateInfo.Name = name;
			}

			pipelineCaches.EmplaceBack(pipelineCacheCreateInfo);
		}
	}
	else
	{
		PipelineCache::CreateInfo pipelineCacheCreateInfo;
		pipelineCacheCreateInfo.RenderDevice = GetRenderDevice();
		pipelineCacheCreateInfo.ExternallySync = false;
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i)
		{
			if constexpr (_DEBUG) {
				GTSL::StaticString<32> name("Pipeline cache. Thread: "); name += i;
				pipelineCacheCreateInfo.Name = name;
			}

			pipelineCaches.EmplaceBack(pipelineCacheCreateInfo);
		}
	}

	BE_LOG_MESSAGE("Initialized successfully");
}

void RenderSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	graphicsQueue.Wait(GetRenderDevice()); transferQueue.Wait(GetRenderDevice());
	
	for (uint32 i = 0; i < swapchainTextures.GetLength(); ++i)
	{
		CommandPool::FreeCommandBuffersInfo free_command_buffers_info;
		free_command_buffers_info.RenderDevice = &renderDevice;

		free_command_buffers_info.CommandBuffers = GTSL::Range<CommandBuffer*>(1, &graphicsCommandBuffers[i]);
		graphicsCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		free_command_buffers_info.CommandBuffers = GTSL::Range<CommandBuffer*>(1, &transferCommandBuffers[i]);
		transferCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		graphicsCommandPools[i].Destroy(&renderDevice);
		transferCommandPools[i].Destroy(&renderDevice);
	}
	
	renderContext.Destroy(&renderDevice);
	surface.Destroy(&renderDevice);
	
	//DeallocateLocalTextureMemory();
	
	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : graphicsFences) { e.Destroy(&renderDevice); }
	for(auto& e : transferFences) { e.Destroy(&renderDevice); }

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());

	{
		uint32 cacheSize = 0;

		PipelineCache::CreateFromMultipleInfo createPipelineCacheInfo;
		createPipelineCacheInfo.RenderDevice = GetRenderDevice();
		createPipelineCacheInfo.Caches = pipelineCaches;
		const PipelineCache pipelineCache(createPipelineCacheInfo);
		pipelineCache.GetCacheSize(GetRenderDevice(), cacheSize);

		if (cacheSize)
		{
			auto* pipelineCacheResourceManager = BE::Application::Get()->GetResourceManager<PipelineCacheResourceManager>("PipelineCacheResourceManager");
			
			GTSL::Buffer<BE::TAR> pipelineCacheBuffer;
			pipelineCacheBuffer.Allocate(cacheSize, 32, GetTransientAllocator());
			pipelineCache.GetCache(&renderDevice, pipelineCacheBuffer.GetBufferInterface());
			pipelineCacheResourceManager->WriteCache(pipelineCacheBuffer);
		}
	}
}

void RenderSystem::renderStart(TaskInfo taskInfo)
{
	//Fence::WaitForFencesInfo waitForFencesInfo;
	//waitForFencesInfo.RenderDevice = &renderDevice;
	//waitForFencesInfo.Timeout = ~0ULL;
	//waitForFencesInfo.WaitForAll = true;
	//waitForFencesInfo.Fences = GTSL::Range<const Fence*>(1, &graphicsFences[currentFrameIndex]);
	//Fence::WaitForFences(waitForFencesInfo);

	graphicsFences[currentFrameIndex].Wait(GetRenderDevice());

	//Fence::ResetFencesInfo resetFencesInfo;
	//resetFencesInfo.RenderDevice = &renderDevice;
	//resetFencesInfo.Fences = GTSL::Range<const Fence*>(1, &graphicsFences[currentFrameIndex]);
	//Fence::ResetFences(resetFencesInfo);
	
	graphicsFences[currentFrameIndex].Reset(GetRenderDevice());
	
	graphicsCommandPools[currentFrameIndex].ResetPool(&renderDevice);
}

void RenderSystem::buildAccelerationStructuresOnDevice(CommandBuffer& commandBuffer)
{
	if (buildDatas.GetLength())
	{
		GTSL::Array<GAL::BuildAccelerationStructureInfo, 8> accelerationStructureBuildInfos;
		GTSL::Array<GTSL::Array<AccelerationStructure::Geometry, 8>, 16> geometryDescriptors;

		uint32 offset = 0;

		auto scratchBufferAddress = accelerationStructureScratchBuffer.GetAddress(GetRenderDevice());
		
		for (uint32 i = 0; i < buildDatas.GetLength(); ++i)
		{
			geometryDescriptors.EmplaceBack();
			geometryDescriptors[i].EmplaceBack(geometries[i]);
			
			GAL::BuildAccelerationStructureInfo buildAccelerationStructureInfo;
			buildAccelerationStructureInfo.ScratchBufferAddress = scratchBufferAddress + offset; //TODO: ENSURE CURRENT BUILDS SCRATCH BUFFER AREN'T OVERWRITTEN ON TURN OF FRAME
			buildAccelerationStructureInfo.SourceAccelerationStructure = AccelerationStructure();
			buildAccelerationStructureInfo.DestinationAccelerationStructure = buildDatas[i].Destination;
			buildAccelerationStructureInfo.Geometries = geometryDescriptors[i];
			buildAccelerationStructureInfo.Flags = buildDatas[i].BuildFlags;

			accelerationStructureBuildInfos.EmplaceBack(buildAccelerationStructureInfo);
			
			offset += GTSL::Math::RoundUpByPowerOf2(buildDatas[i].ScratchBuildSize, scratchBufferOffsetAlignment);
		}
		
		commandBuffer.BuildAccelerationStructure(GetRenderDevice(), accelerationStructureBuildInfos, GetTransientAllocator());
		
		GTSL::Array<CommandBuffer::BarrierData, 1> barriers;
		barriers.EmplaceBack(CommandBuffer::MemoryBarrier{ AccessFlags::ACCELERATION_STRUCTURE_WRITE, AccessFlags::ACCELERATION_STRUCTURE_READ });
		
		commandBuffer.AddPipelineBarrier(GetRenderDevice(), barriers, PipelineStage::ACCELERATION_STRUCTURE_BUILD, PipelineStage::ACCELERATION_STRUCTURE_BUILD, GetTransientAllocator());
	}
	
	buildDatas.ResizeDown(0);
	geometries.ResizeDown(0);
}

bool RenderSystem::resize()
{
	if (renderArea == 0) { return false; }
	//graphicsQueue.Wait(GetRenderDevice());

	if (!surface.GetHandle())
	{
		Surface::CreateInfo surfaceCreateInfo;
		surfaceCreateInfo.RenderDevice = &renderDevice;
		if constexpr (_DEBUG) { surfaceCreateInfo.Name = GTSL::StaticString<32>("Surface"); }

		if constexpr (_WIN64) {
			GTSL::Window::Win32NativeHandles handles;
			window->GetNativeHandles(&handles);
			surfaceCreateInfo.SystemData.InstanceHandle = GetModuleHandle(nullptr);
			surfaceCreateInfo.SystemData.WindowHandle = handles.HWND;
		}

#if __linux__
#endif
	
		surface.Initialize(surfaceCreateInfo);
	}

	Surface::SurfaceCapabilities surfaceCapabilities;
	auto isSupported = surface.IsSupported(&renderDevice, &surfaceCapabilities);

	renderArea = surfaceCapabilities.CurrentExtent;
	
	if (!isSupported) {
		BE::Application::Get()->Close(BE::Application::CloseMode::ERROR, GTSL::StaticString<64>("No supported surface found!"));
	}

	auto supportedPresentModes = surface.GetSupportedPresentModes(&renderDevice);
	swapchainPresentMode = supportedPresentModes[0];

	auto supportedSurfaceFormats = surface.GetSupportedFormatsAndColorSpaces(&renderDevice);
	swapchainColorSpace = supportedSurfaceFormats[0].First; swapchainFormat = supportedSurfaceFormats[0].Second;

	RenderContext::RecreateInfo recreate;
	recreate.RenderDevice = GetRenderDevice();
	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Swapchain");
		recreate.Name = name;
	}
	recreate.SurfaceArea = renderArea;
	recreate.ColorSpace = swapchainColorSpace;
	recreate.DesiredFramesInFlight = pipelinedFrames;
	recreate.Format = swapchainFormat;
	recreate.PresentMode = swapchainPresentMode;
	recreate.Surface = &surface;
	recreate.TextureUses = TextureUse::STORAGE | TextureUse::TRANSFER_DESTINATION;
	recreate.Queue = &graphicsQueue;
	renderContext.Recreate(recreate);

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	//imageIndex = 0;

	RenderContext::GetTexturesInfo getTexturesInfo;
	getTexturesInfo.RenderDevice = GetRenderDevice();
	swapchainTextures = renderContext.GetTextures(getTexturesInfo);

	RenderContext::GetTextureViewsInfo getTextureViewsInfo;
	getTextureViewsInfo.RenderDevice = &renderDevice;
	GTSL::Array<TextureView::CreateInfo, MAX_CONCURRENT_FRAMES> textureViewCreateInfos(MAX_CONCURRENT_FRAMES);
	{
		for (uint8 i = 0; i < MAX_CONCURRENT_FRAMES; ++i)
		{
			textureViewCreateInfos[i].RenderDevice = GetRenderDevice();
			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> name("Swapchain texture view. Frame: "); name += static_cast<uint16>(i); //cast to not consider it a char
				textureViewCreateInfos[i].Name = name;
			}
			textureViewCreateInfos[i].Format = swapchainFormat;
		}
	}
	getTextureViewsInfo.TextureViewCreateInfos = textureViewCreateInfos;

	{
		swapchainTextureViews = renderContext.GetTextureViews(getTextureViewsInfo);
	}

	lastRenderArea = renderArea;
	
	return true;
}

void RenderSystem::renderBegin(TaskInfo taskInfo)
{	
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	commandBuffer.BeginRecording({});

	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		AccelerationStructure::Geometry geometry;
		geometry.Flags = GeometryFlags::OPAQUE;
		geometry.PrimitiveCount = rayTracingInstancesCount; //TODO: WHAT HAPPENS IF MESH IS REMOVED FROM THE MIDDLE OF THE COLLECTION, maybe: keep index of highest element in the colection
		geometry.PrimitiveOffset = 0;
		geometry.SetGeometryInstances(AccelerationStructure::GeometryInstances{ instancesBuffer[GetCurrentFrame()].GetAddress(GetRenderDevice()) });
		geometries.EmplaceBack(geometry);

		AccelerationStructureBuildData buildData;
		buildData.BuildFlags = 0;
		buildData.Destination = topLevelAccelerationStructure[GetCurrentFrame()];
		buildData.ScratchBuildSize = topLevelStructureScratchSize;
		buildDatas.EmplaceBack(buildData);

		buildAccelerationStructures(this, commandBuffer);
	}
}

void RenderSystem::renderFinish(TaskInfo taskInfo)
{
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	commandBuffer.EndRecording({});

	{
		GTSL::Array<Semaphore, 8> waitSemaphores, signalSemaphores; GTSL::Array<uint32, 8> wps;

		waitSemaphores.EmplaceBack(transferDoneSemaphores[GetCurrentFrame()]);
		wps.EmplaceBack(PipelineStage::TRANSFER);
		
		if (surface.GetHandle())
		{
			waitSemaphores.EmplaceBack(imageAvailableSemaphore[currentFrameIndex]);
			wps.EmplaceBack(PipelineStage::COLOR_ATTACHMENT_OUTPUT);

			signalSemaphores.EmplaceBack(renderFinishedSemaphore[currentFrameIndex]);
		}

		Queue::SubmitInfo submitInfo;
		submitInfo.RenderDevice = &renderDevice;
		submitInfo.Fence = graphicsFences[currentFrameIndex];
		submitInfo.WaitSemaphores = waitSemaphores;
		submitInfo.SignalSemaphores = signalSemaphores;
		submitInfo.WaitPipelineStages = wps;
		submitInfo.CommandBuffers = GTSL::Range<const CommandBuffer*>(1, &commandBuffer);
		graphicsQueue.Submit(submitInfo);

		if (surface.GetHandle())
		{			
			RenderContext::PresentInfo presentInfo;
			presentInfo.RenderDevice = &renderDevice;
			presentInfo.Queue = &graphicsQueue;
			presentInfo.WaitSemaphores = signalSemaphores;
			presentInfo.ImageIndex = imageIndex;
			renderContext.Present(presentInfo);
		}
	}

	currentFrameIndex = (currentFrameIndex + 1) % pipelinedFrames;
}

void RenderSystem::frameStart(TaskInfo taskInfo)
{
	transferFences[GetCurrentFrame()].Wait(GetRenderDevice());

	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];
	
	//if(transferFences[currentFrameIndex].GetStatus(&renderDevice))
	{
		//for(uint32 i = 0; i < processedBufferCopies[GetCurrentFrame()]; ++i)
		//{
		//	bufferCopyData[i].SourceBuffer.Destroy(&renderDevice);
		//	DeallocateScratchBufferMemory(bufferCopyData[i].Allocation);
		//}

		//for(uint32 i = 0; i < processedTextureCopies[GetCurrentFrame()]; ++i)
		//{
		//	textureCopyData[i].SourceBuffer.Destroy(&renderDevice);
		//	DeallocateScratchBufferMemory(textureCopyData[i].Allocation);
		//}
		
		bufferCopyData.Pop(0, processedBufferCopies[GetCurrentFrame()]);
		//textureCopyData.Pop(0, processedTextureCopies[GetCurrentFrame()]);
		//triangleDatas.Pop(0, processedAccelerationStructureBuilds[GetCurrentFrame()]);

		Fence::ResetFencesInfo reset_fences_info;
		reset_fences_info.RenderDevice = &renderDevice;
		reset_fences_info.Fences = GTSL::Range<const Fence*>(1, &transferFences[currentFrameIndex]);
		Fence::ResetFences(reset_fences_info);
	}
	
	transferCommandPools[currentFrameIndex].ResetPool(&renderDevice); //should only be done if frame is finished transferring but must also implement check in execute transfers
	//or begin command buffer complains
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	auto& commandBuffer = transferCommandBuffers[GetCurrentFrame()];
	
	CommandBuffer::BeginRecordingInfo beginRecordingInfo;
	beginRecordingInfo.RenderDevice = &renderDevice;
	commandBuffer.BeginRecording(beginRecordingInfo);
	
	{
		auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
		
		for (auto& e : bufferCopyData)
		{
			CommandBuffer::CopyBuffersInfo copy_buffers_info;
			copy_buffers_info.RenderDevice = &renderDevice;
			copy_buffers_info.Destination = e.DestinationBuffer;
			copy_buffers_info.DestinationOffset = e.DestinationOffset;
			copy_buffers_info.Source = e.SourceBuffer;
			copy_buffers_info.SourceOffset = e.SourceOffset;
			copy_buffers_info.Size = e.Size;
			commandBuffer.CopyBuffers(copy_buffers_info);
		}

		processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
	}
	
	{
		auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];

		if (textureCopyData.GetLength())
		{

			GTSL::Vector<CommandBuffer::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());
			GTSL::Vector<CommandBuffer::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());

			for (uint32 i = 0; i < textureCopyData.GetLength(); ++i)
			{
				sourceTextureBarriers.EmplaceBack(CommandBuffer::TextureBarrier{ textureCopyData[i].DestinationTexture, TextureLayout::UNDEFINED, TextureLayout::TRANSFER_DST, 0, AccessFlags::TRANSFER_WRITE, TextureType::COLOR });
				destinationTextureBarriers.EmplaceBack(CommandBuffer::TextureBarrier{ textureCopyData[i].DestinationTexture, TextureLayout::TRANSFER_DST, TextureLayout::SHADER_READ_ONLY, AccessFlags::TRANSFER_WRITE, AccessFlags::SHADER_READ, TextureType::COLOR });
			}

			commandBuffer.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, PipelineStage::TRANSFER, PipelineStage::TRANSFER, GetTransientAllocator());

			for (uint32 i = 0; i < textureCopyData.GetLength(); ++i)
			{
				CommandBuffer::CopyBufferToTextureInfo copyBufferToImageInfo;
				copyBufferToImageInfo.RenderDevice = GetRenderDevice();
				copyBufferToImageInfo.DestinationTexture = textureCopyData[i].DestinationTexture;
				copyBufferToImageInfo.Offset = { 0, 0, 0 };
				copyBufferToImageInfo.Extent = textureCopyData[i].Extent;
				copyBufferToImageInfo.SourceBuffer = textureCopyData[i].SourceBuffer;
				copyBufferToImageInfo.TextureLayout = TextureLayout::TRANSFER_DST;// textureCopyData[i].Layout;
				commandBuffer.CopyBufferToTexture(copyBufferToImageInfo);
			}

			commandBuffer.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, PipelineStage::TRANSFER, PipelineStage::ALL_GRAPHICS, GetTransientAllocator());
			textureCopyDatas[GetCurrentFrame()].ResizeDown(0);
		}
			
		//processedTextureCopies[GetCurrentFrame()] = textureCopyData.GetLength();
	}

	
	CommandBuffer::EndRecordingInfo endRecordingInfo;
	endRecordingInfo.RenderDevice = &renderDevice;
	commandBuffer.EndRecording(endRecordingInfo);
	
	//if (bufferCopyDatas[currentFrameIndex].GetLength() || textureCopyDatas[GetCurrentFrame()].GetLength())
	//{
		Queue::SubmitInfo submit_info;
		submit_info.RenderDevice = &renderDevice;
		submit_info.Fence = transferFences[currentFrameIndex];
		submit_info.CommandBuffers = GTSL::Range<const CommandBuffer*>(1, &commandBuffer);
		submit_info.WaitPipelineStages = GTSL::Array<uint32, 2>{ PipelineStage::TRANSFER };
		submit_info.SignalSemaphores = GTSL::Array<Semaphore, 1>{ transferDoneSemaphores[GetCurrentFrame()] };
		transferQueue.Submit(submit_info);
	//}
}

RenderSystem::TextureHandle RenderSystem::CreateTexture(GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, TextureUses textureUses, bool updatable)
{
	//RenderDevice::FindSupportedImageFormat findFormat;
	//findFormat.TextureTiling = TextureTiling::OPTIMAL;
	//findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	//GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	//findFormat.Candidates = candidates;
	//auto supportedFormat = renderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

	//GAL::Texture::ConvertTextureFormat(textureInfo.Format, GAL::TextureFormat::RGBA_I8, textureInfo.Extent, GTSL::AlignedPointer<byte, 16>(buffer.begin()), 1);

	TextureComponent textureComponent;

	textureComponent.Extent = extent;
	
	textureComponent.FormatDescriptor = formatDescriptor;
	auto format = static_cast<TextureFormat>(GAL::FormatToVkFomat(GAL::MakeFormatFromFormatDescriptor(formatDescriptor)));

	auto textureDimensions = GAL::VulkanDimensionsFromExtent(extent);

	textureComponent.Uses = textureUses;
	if (updatable) { textureComponent.Uses |= TextureUse::TRANSFER_DESTINATION; }
	
	Texture::CreateInfo textureCreateInfo;
	textureCreateInfo.RenderDevice = GetRenderDevice();
	if constexpr (_DEBUG) {
		GTSL::StaticString<64> name("Texture.");
		textureCreateInfo.Name = name;
	}

	textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
	textureCreateInfo.Uses = textureComponent.Uses;
	textureCreateInfo.Dimensions = textureDimensions;
	textureCreateInfo.Format = format;
	textureCreateInfo.Extent = extent;
	textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
	textureCreateInfo.MipLevels = 1;

	textureComponent.Layout = TextureLayout::UNDEFINED;

	auto textureSize = extent.Width * extent.Height * extent.Depth * formatDescriptor.GetSize();
	
	if (updatable && needsStagingBuffer)
	{
		Buffer::CreateInfo scratchBufferCreateInfo;
		scratchBufferCreateInfo.RenderDevice = GetRenderDevice();
		scratchBufferCreateInfo.BufferType = BufferType::TRANSFER_SOURCE;
		
		AllocateScratchBufferMemory(textureSize, &textureComponent.ScratchBuffer, scratchBufferCreateInfo, &textureComponent.ScratchAllocation);
	}
	
	AllocateLocalTextureMemory(textureSize, &textureComponent.Texture, textureCreateInfo, &textureComponent.Allocation);
	
	TextureView::CreateInfo textureViewCreateInfo;
	textureViewCreateInfo.RenderDevice = GetRenderDevice();
	if constexpr (_DEBUG) {
		GTSL::StaticString<64> name("Texture view.");
		textureViewCreateInfo.Name = name;
	}

	textureViewCreateInfo.Type = TextureAspectToVkImageAspectFlags(formatDescriptor.Type);
	textureViewCreateInfo.Dimensions = textureDimensions;
	textureViewCreateInfo.Format = format;
	textureViewCreateInfo.Texture = textureComponent.Texture;
	textureViewCreateInfo.MipLevels = 1;

	textureComponent.TextureView = TextureView(textureViewCreateInfo);
	
	TextureSampler::CreateInfo textureSamplerCreateInfo;
	textureSamplerCreateInfo.RenderDevice = GetRenderDevice();
	if constexpr (_DEBUG) {
		GTSL::StaticString<64> name("Texture sampler.");
		textureSamplerCreateInfo.Name = name;
	}

	textureSamplerCreateInfo.Anisotropy = 0;

	textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	
	auto textureIndex = textures.Emplace(textureComponent);

	return TextureHandle(textureIndex);
}

void RenderSystem::UpdateTexture(const TextureHandle textureHandle)
{
	const auto& texture = textures[textureHandle()];

	TextureCopyData textureCopyData;
	textureCopyData.Layout = texture.Layout;
	textureCopyData.Extent = texture.Extent;
	textureCopyData.Allocation = texture.Allocation;
	textureCopyData.DestinationTexture = texture.Texture;
	textureCopyData.SourceOffset = 0;
	textureCopyData.SourceBuffer = texture.ScratchBuffer;
	AddTextureCopy(textureCopyData);
	
	//TODO: QUEUE BUFFER DELETION
}

void RenderSystem::OnRenderEnable(TaskInfo taskInfo, bool oldFocus)
{
	if(!oldFocus)
	{
		const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessTypes::READ_WRITE } };
		taskInfo.GameInstance->AddTask("frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, "FrameStart", "RenderStart");

		taskInfo.GameInstance->AddTask("executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, "GameplayEnd", "RenderStart");
	
		taskInfo.GameInstance->AddTask("renderStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderStart>(this), actsOn, "RenderStart", "RenderStartSetup");
		taskInfo.GameInstance->AddTask("renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderBegin>(this), actsOn, "RenderEndSetup", "RenderDo");
	
		taskInfo.GameInstance->AddTask("renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFinish>(this), actsOn, "RenderFinished", "RenderEnd");

		BE_LOG_SUCCESS("Enabled rendering")
	}

	OnResize(window->GetFramebufferExtent());
}

void RenderSystem::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	if (oldFocus)
	{
		taskInfo.GameInstance->RemoveTask("frameStart", "FrameStart");
		taskInfo.GameInstance->RemoveTask("executeTransfers", "GameplayEnd");
		taskInfo.GameInstance->RemoveTask("renderStart", "RenderStart");
		taskInfo.GameInstance->RemoveTask("renderSetup", "RenderEndSetup");
		taskInfo.GameInstance->RemoveTask("renderFinished", "RenderFinished");

		BE_LOG_SUCCESS("Disabled rendering")
	}
}

bool RenderSystem::AcquireImage()
{
	bool result = false;
	
	if(surface.GetHandle()) {
		auto acquireResult = renderContext.AcquireNextImage(&renderDevice, imageAvailableSemaphore[currentFrameIndex]);

		imageIndex = acquireResult.Get();

		switch (acquireResult.State())
		{
		case GAL::VulkanRenderContext::AcquireState::OK: break;
		case GAL::VulkanRenderContext::AcquireState::SUBOPTIMAL:
		case GAL::VulkanRenderContext::AcquireState::BAD: resize(); result = true; break;
		default:;
		}
	}
	else
	{
		resize(); result = true; AcquireImage();
	}

	if (lastRenderArea != renderArea) { resize(); result = true; }
	
	return result;
}

void RenderSystem::printError(const char* message, const RenderDevice::MessageSeverity messageSeverity) const
{
	switch (messageSeverity)
	{
	case RenderDevice::MessageSeverity::MESSAGE: BE_LOG_MESSAGE(message) break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message) break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); GAL_DEBUG_BREAK; break;
	default: break;
	}
}

void* RenderSystem::allocateApiMemory(void* data, const uint64 size, const uint64 alignment)
{
	void* allocation; uint64 allocated_size;
	GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);
	//apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
	{
		GTSL::Lock lock(allocationsMutex);

		BE_ASSERT(!apiAllocations.contains(reinterpret_cast<uint64>(allocation)), "")
		apiAllocations.emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair<uint64, uint64>(size, alignment));
	}
	return allocation;
}

void* RenderSystem::reallocateApiMemory(void* data, void* oldAllocation, uint64 size, uint64 alignment)
{
	void* allocation; uint64 allocated_size;

	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		//const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(oldAllocation));
		old_alloc = apiAllocations.at(reinterpret_cast<uint64>(oldAllocation));
	}
	
	GetPersistentAllocator().Allocate(size, old_alloc.Second, &allocation, &allocated_size);
	//apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
	apiAllocations.emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair<uint64, uint64>(size, alignment));
	
	GTSL::MemCopy(old_alloc.First, oldAllocation, allocation);
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
	//apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.erase(reinterpret_cast<uint64>(oldAllocation));
	}
	
	return allocation;
}

void RenderSystem::deallocateApiMemory(void* data, void* allocation)
{
	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		old_alloc = apiAllocations.at(reinterpret_cast<uint64>(allocation));
		//const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(allocation));
	}
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, allocation);
	
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.erase(reinterpret_cast<uint64>(allocation));
		//apiAllocations.Remove(reinterpret_cast<uint64>(allocation));
	}
}
