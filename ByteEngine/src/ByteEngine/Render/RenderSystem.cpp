#include "RenderSystem.h"

#include <GTSL/Window.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Resources/PipelineCacheResourceManager.h"

#undef MemoryBarrier

class CameraSystem;
class RenderStaticMeshCollection;

PipelineCache RenderSystem::GetPipelineCache() const { return pipelineCaches[GTSL::Thread::ThisTreadID()]; }

void RenderSystem::CreateRayTracedMesh(const MeshHandle meshHandle)
{
	auto& mesh = meshes[meshHandle()];
	
	mesh.DerivedTypeIndex = rayTracingMeshes.Emplace();

	BE_ASSERT(mesh.DerivedTypeIndex < MAX_INSTANCES_COUNT);
}

RenderSystem::MeshHandle RenderSystem::CreateMesh(Id name, uint32 customIndex)
{
	auto meshIndex = meshes.Emplace(); auto& mesh = meshes[meshIndex];
	mesh.CustomMeshIndex = customIndex;

	return MeshHandle(meshIndex);
}

//RenderSystem::MeshHandle RenderSystem::CreateMesh(Id name, uint32 customIndex, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, MaterialInstanceHandle materialHandle)
//{
//	auto meshIndex = meshes.Emplace(); auto& mesh = meshes[meshIndex];
//	mesh.CustomMeshIndex = customIndex;
//	mesh.MaterialHandle = materialHandle;
//
//	auto meshHandle = MeshHandle(meshIndex);
//	
//	UpdateMesh(meshHandle, vertexCount, vertexSize, indexCount, indexSize);
//	return meshHandle;
//}

void RenderSystem::UpdateRayTraceMesh(const MeshHandle meshHandle)
{
	auto& mesh = meshes[meshHandle()]; auto& rayTracingMesh = rayTracingMeshes[mesh.CustomMeshIndex];
	auto& buffer = buffers[mesh.Buffer()];

	GAL::DeviceAddress meshDataAddress = 0;

	if (needsStagingBuffer) {
		auto& stagingBuffer = buffers[buffer.Staging()];
		meshDataAddress = stagingBuffer.Buffer.GetAddress(GetRenderDevice());
	}
	else {
		meshDataAddress = buffer.Buffer.GetAddress(GetRenderDevice());
	}
	
	uint32 scratchSize;
	
	{
		GAL::GeometryTriangles geometryTriangles;
		geometryTriangles.IndexType = GAL::SizeToIndexType(mesh.IndexSize);
		geometryTriangles.VertexPositionFormat = GAL::ShaderDataType::FLOAT3;
		geometryTriangles.MaxVertices = mesh.VertexCount;
		geometryTriangles.VertexData = meshDataAddress;
		geometryTriangles.IndexData = meshDataAddress + GTSL::Math::RoundUpByPowerOf2(mesh.VertexCount * mesh.VertexSize, GetBufferSubDataAlignment());
		geometryTriangles.VertexStride = mesh.VertexSize;
		geometryTriangles.FirstVertex = 0;

		GAL::Geometry geometry(geometryTriangles, GAL::GeometryFlags::OPAQUE, mesh.IndicesCount / 3, 0);

		AllocateAccelerationStructureMemory(&rayTracingMesh.AccelerationStructure, &rayTracingMesh.StructureBuffer,
			GTSL::Range<const GAL::Geometry*>(1, &geometry), &rayTracingMesh.StructureBufferAllocation, &scratchSize);
		AccelerationStructureBuildData buildData;
		buildData.ScratchBuildSize = scratchSize;
		buildData.Destination = rayTracingMesh.AccelerationStructure;

		addRayTracingInstance(geometry, buildData);
	}

	for (uint8 f = 0; f < pipelinedFrames; ++f) {
		GAL::WriteInstance(rayTracingMesh.AccelerationStructure, mesh.CustomMeshIndex, GAL::GeometryFlags::OPAQUE, GetRenderDevice(), instancesAllocation[f].Data, mesh.DerivedTypeIndex, accelerationStructureBuildDevice);
		GAL::WriteInstanceBindingTableRecordOffset(0, instancesAllocation[f].Data, mesh.DerivedTypeIndex);
	}
}

void RenderSystem::UpdateMesh(MeshHandle meshHandle, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, GTSL::Range<const GAL::ShaderDataType*> vertexLayout)
{
	auto& mesh = meshes[meshHandle()];

	mesh.VertexSize = vertexSize; mesh.VertexCount = vertexCount; mesh.IndexSize = indexSize; mesh.IndicesCount = indexCount;

	auto verticesSize = vertexCount * vertexSize; auto indecesSize = indexCount * indexSize;
	auto meshSize = GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment()) + indecesSize;

	mesh.VertexDescriptor.PushBack(vertexLayout);
	
	mesh.Buffer = CreateBuffer(meshSize, GAL::BufferUses::VERTEX | GAL::BufferUses::INDEX, true, false);
}

void RenderSystem::UpdateMesh(MeshHandle meshHandle)
{
	auto& mesh = meshes[meshHandle()];

	auto verticesSize = mesh.VertexSize * mesh.VertexCount; auto indecesSize = mesh.IndexSize * mesh.IndicesCount;
	auto meshSize = GTSL::Math::RoundUpByPowerOf2(verticesSize, GetBufferSubDataAlignment()) + indecesSize;

	++buffers[buffers[mesh.Buffer()].Staging()].references;
	
	BufferCopyData bufferCopyData;
	bufferCopyData.Buffer = mesh.Buffer;
	bufferCopyData.Offset = 0;
	AddBufferUpdate(bufferCopyData);
}

void RenderSystem::RenderMesh(MeshHandle handle, const uint32 instanceCount)
{
	auto& mesh = meshes[handle()]; auto buffer = buffers[mesh.Buffer()].Buffer;

	graphicsCommandBuffers[GetCurrentFrame()].BindVertexBuffer(GetRenderDevice(), buffer, mesh.VertexSize * mesh.VertexCount, 0, mesh.VertexSize);
	graphicsCommandBuffers[GetCurrentFrame()].BindIndexBuffer(GetRenderDevice(), buffer, mesh.IndexSize * mesh.IndicesCount, GTSL::Math::RoundUpByPowerOf2(mesh.VertexSize * mesh.VertexCount, GetBufferSubDataAlignment()), GAL::SizeToIndexType(mesh.IndexSize));
	graphicsCommandBuffers[GetCurrentFrame()].DrawIndexed(GetRenderDevice(), mesh.IndicesCount, instanceCount);
}

void RenderSystem::SetMeshMatrix(const MeshHandle meshHandle, const GTSL::Matrix3x4& matrix)
{
	const auto& mesh = meshes[meshHandle()];
	GAL::WriteInstanceMatrix(matrix, instancesAllocation[GetCurrentFrame()].Data, mesh.DerivedTypeIndex);
}

void RenderSystem::SetMeshOffset(const MeshHandle meshHandle, const uint32 offset)
{
	const auto& mesh = meshes[meshHandle()];
	GAL::WriteInstanceBindingTableRecordOffset(offset, instancesAllocation[GetCurrentFrame()].Data, mesh.DerivedTypeIndex);
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

		const GTSL::Array<TaskDependency, 8> actsOn{ { u8"RenderSystem", AccessTypes::READ_WRITE } };
		initializeInfo.GameInstance->AddTask(u8"frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, u8"FrameStart", u8"RenderStart");
		initializeInfo.GameInstance->AddTask(u8"executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, u8"GameplayEnd", u8"RenderStart");
		initializeInfo.GameInstance->AddTask(u8"renderStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderStart>(this), actsOn, u8"RenderStart", u8"RenderStartSetup");
		initializeInfo.GameInstance->AddTask(u8"renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderBegin>(this), actsOn, u8"RenderEndSetup", u8"RenderDo");
		initializeInfo.GameInstance->AddTask(u8"renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFinish>(this), actsOn, u8"RenderFinished", u8"RenderEnd");
	}
	
	//apiAllocations.Initialize(128, GetPersistentAllocator());
	apiAllocations.reserve(16);

	rayTracingMeshes.Initialize(32, GetPersistentAllocator());
	meshes.Initialize(32, GetPersistentAllocator());
	buffers.Initialize(32, GetPersistentAllocator());

	textures.Initialize(32, GetPersistentAllocator());

	RenderDevice::RayTracingCapabilities rayTracingCapabilities;

	useHDR = BE::Application::Get()->GetOption(u8"hdr");
	pipelinedFrames = static_cast<uint8>(GTSL::Math::Clamp(BE::Application::Get()->GetOption(u8"buffer"), 2u, 3u));
	bool rayTracing = BE::Application::Get()->GetOption(u8"rayTracing");

	{
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>(BE::Application::Get()->GetApplicationName());
		createInfo.ApplicationVersion[0] = 0; createInfo.ApplicationVersion[1] = 0; createInfo.ApplicationVersion[2] = 0;

		createInfo.Debug = BE::Application::Get()->GetOption(u8"debug");

		GTSL::Array<GAL::QueueType, 5> queue_create_infos;
		GTSL::Array<RenderDevice::QueueKey, 5> queueKeys;
		
		queue_create_infos.EmplaceBack(GAL::QueueTypes::GRAPHICS); queueKeys.EmplaceBack();
		queue_create_infos.EmplaceBack(GAL::QueueTypes::TRANSFER); queueKeys.EmplaceBack();
		
		createInfo.Queues = queue_create_infos;
		createInfo.QueueKeys = queueKeys;

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

		BE_LOG_MESSAGE("Started Vulkan API\n	GPU: ", renderDevice.GetGPUInfo().GPUName)
		
		graphicsQueue.Initialize(GetRenderDevice(), queueKeys[0]);
		transferQueue.Initialize(GetRenderDevice(), queueKeys[1]);

		{
			needsStagingBuffer = true;

			auto memoryHeaps = renderDevice.GetMemoryHeaps(); GAL::VulkanRenderDevice::MemoryHeap& biggestGPUHeap = memoryHeaps[0];

			for (auto& e : memoryHeaps)
			{
				if (e.HeapType & GAL::MemoryTypes::GPU) {
					if (e.Size > biggestGPUHeap.Size) {
						biggestGPUHeap = e;

						for (auto& mt : e.MemoryTypes) {
							if (mt & GAL::MemoryTypes::GPU && mt & GAL::MemoryTypes::HOST_COHERENT && mt & GAL::MemoryTypes::HOST_VISIBLE) {
								needsStagingBuffer = false; break;
							}
						}
					}
				}
			}
		}

		scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
		localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());

		if (rayTracing) {
			GAL::Geometry geometry(GAL::GeometryInstances{ 0 }, GAL::GeometryFlag(), MAX_INSTANCES_COUNT, 0);

			for (uint8 f = 0; f < pipelinedFrames; ++f) {
				geometries[f].Initialize(16, GetPersistentAllocator());
				buildDatas[f].Initialize(16, GetPersistentAllocator());
				
				AllocateAccelerationStructureMemory(&topLevelAccelerationStructure[f], &topLevelAccelerationStructureBuffer[f],
					GTSL::Range<const GAL::Geometry*>(1, &geometry), &topLevelAccelerationStructureAllocation[f],
					&topLevelStructureScratchSize);

				AllocateScratchBufferMemory(MAX_INSTANCES_COUNT * GetRenderDevice()->GetAccelerationStructureInstanceSize(), GAL::BufferUses::ADDRESS | GAL::BufferUses::BUILD_INPUT_READ, &instancesBuffer[f], &instancesAllocation[f]);
				AllocateLocalBufferMemory(GTSL::Byte(GTSL::MegaByte(1)), GAL::BufferUses::ADDRESS | GAL::BufferUses::BUILD_INPUT_READ, &accelerationStructureScratchBuffer[f], &scratchBufferAllocation[f]);
			}

			shaderGroupHandleAlignment = rayTracingCapabilities.ShaderGroupHandleAlignment;
			shaderGroupHandleSize = rayTracingCapabilities.ShaderGroupHandleSize;
			scratchBufferOffsetAlignment = rayTracingCapabilities.ScratchBuildOffsetAlignment;
			shaderGroupBaseAlignment = rayTracingCapabilities.ShaderGroupBaseAlignment;

			accelerationStructureBuildDevice = rayTracingCapabilities.BuildDevice;
			
			switch (rayTracingCapabilities.BuildDevice) {
			case GAL::Device::CPU: break;
			case GAL::Device::GPU:
			case GAL::Device::GPU_OR_CPU:
				buildAccelerationStructures = decltype(buildAccelerationStructures)::Create<RenderSystem, &RenderSystem::buildAccelerationStructuresOnDevice>();
				break;
			default: ;
			}
		}
	}

	swapchainPresentMode = GAL::PresentModes::SWAP;
	swapchainColorSpace = GAL::ColorSpace::SRGB_NONLINEAR;
	swapchainFormat = GAL::FORMATS::BGRA_I8;

	for (uint32 i = 0; i < pipelinedFrames; ++i) {
		if constexpr (_DEBUG) { GTSL::StaticString<32> name(u8"Transfer semaphore. Frame: "); name += i; }
		transferDoneSemaphores[i].Initialize(GetRenderDevice());
		
		//processedTextureCopies.EmplaceBack(0);
		processedBufferCopies[i] = 0;
		
		if constexpr (_DEBUG) {
			//GTSL::StaticString<32> name("ImageAvailableSemaphore #"); name += i;
		}
		imageAvailableSemaphore[i].Initialize(GetRenderDevice());

		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name(u8"RenderFinishedSemaphore #"); name += i;
		}
		renderFinishedSemaphore[i].Initialize(GetRenderDevice());
		
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name(u8"InFlightFence #"); name += i;
		}

		graphicsFences[i].Initialize(GetRenderDevice(), true);
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name(u8"TrasferFence #"); name += i;
		}
		transferFences[i].Initialize(GetRenderDevice(), true);

		
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> commandPoolName(u8"Transfer command pool #"); commandPoolName += i;
			//commandPoolCreateInfo.Name = commandPoolName;
		}
		
		graphicsCommandBuffers[i].Initialize(GetRenderDevice(), graphicsQueue.GetQueueKey());
		
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> commandPoolName(u8"Transfer command pool #"); commandPoolName += i;
			//commandPoolCreateInfo.Name = commandPoolName;
		}
		
		transferCommandBuffers[i].Initialize(GetRenderDevice(), transferQueue.GetQueueKey());

		bufferCopyDatas[i].Initialize(64, GetPersistentAllocator());
		textureCopyDatas[i].Initialize(64, GetPersistentAllocator());
	}

	bool pipelineCacheAvailable;
	auto* pipelineCacheManager = BE::Application::Get()->GetResourceManager<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
	pipelineCacheManager->DoesCacheExist(pipelineCacheAvailable);

	pipelineCaches.Initialize(BE::Application::Get()->GetNumberOfThreads(), GetPersistentAllocator());

	if (pipelineCacheAvailable) {
		uint32 cacheSize = 0;
		pipelineCacheManager->GetCacheSize(cacheSize);

		GTSL::Buffer<BE::TAR> pipelineCacheBuffer;
		pipelineCacheBuffer.Allocate(cacheSize, 32, GetTransientAllocator());

		pipelineCacheManager->GetCache(pipelineCacheBuffer);
		
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
			if constexpr (_DEBUG) {
				GTSL::StaticString<32> name(u8"Pipeline cache. Thread: "); name += i;
			}

			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, pipelineCacheBuffer);
		}
	} else {
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i)
		{
			if constexpr (_DEBUG) {
				GTSL::StaticString<32> name(u8"Pipeline cache. Thread: "); name += i;
			}

			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, {});
		}
	}

	BE_LOG_MESSAGE("Initialized successfully");
}

void RenderSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	//graphicsQueue.Wait(GetRenderDevice()); transferQueue.Wait(GetRenderDevice());
	renderDevice.Wait();
	
	for (uint32 i = 0; i < pipelinedFrames; ++i) {
		graphicsCommandBuffers[i].Destroy(&renderDevice);
		transferCommandBuffers[i].Destroy(&renderDevice);
	}

	if(renderContext.GetHandle())
		renderContext.Destroy(&renderDevice);

	if(surface.GetHandle())
		surface.Destroy(&renderDevice);
	
	//DeallocateLocalTextureMemory();
	
	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : graphicsFences) { e.Destroy(&renderDevice); }
	for(auto& e : transferFences) { e.Destroy(&renderDevice); }

	for (auto& e : swapchainTextureViews) {
		if (e.GetVkImageView())
			e.Destroy(&renderDevice);
	}

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());

	{
		uint32 cacheSize = 0; PipelineCache pipelineCache;
		pipelineCache.Initialize(GetRenderDevice(), pipelineCaches);
		pipelineCache.GetCacheSize(GetRenderDevice(), cacheSize);

		if (cacheSize) {
			auto* pipelineCacheResourceManager = BE::Application::Get()->GetResourceManager<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
			
			GTSL::Buffer<BE::TAR> pipelineCacheBuffer;
			pipelineCacheBuffer.Allocate(cacheSize, 32, GetTransientAllocator());
			pipelineCache.GetCache(&renderDevice, pipelineCacheBuffer.GetBufferInterface());
			pipelineCacheResourceManager->WriteCache(pipelineCacheBuffer);
		}
	}
}

void RenderSystem::renderStart(TaskInfo taskInfo) {
	graphicsFences[currentFrameIndex].Wait(GetRenderDevice());
	graphicsFences[currentFrameIndex].Reset(GetRenderDevice());
}

void RenderSystem::buildAccelerationStructuresOnDevice(CommandList& commandBuffer)
{
	if (buildDatas[GetCurrentFrame()].GetLength()) {
		GTSL::Array<GAL::BuildAccelerationStructureInfo, 8> accelerationStructureBuildInfos;
		GTSL::Array<GTSL::Array<GAL::Geometry, 8>, 16> geometryDescriptors;

		uint32 offset = 0; auto scratchBufferAddress = accelerationStructureScratchBuffer[GetCurrentFrame()].GetAddress(GetRenderDevice());
		
		for (uint32 i = 0; i < buildDatas[GetCurrentFrame()].GetLength(); ++i) {
			geometryDescriptors.EmplaceBack();
			geometryDescriptors[i].EmplaceBack(geometries[GetCurrentFrame()][i]);
			
			GAL::BuildAccelerationStructureInfo buildAccelerationStructureInfo;
			buildAccelerationStructureInfo.ScratchBufferAddress = scratchBufferAddress + offset; //TODO: ENSURE CURRENT BUILDS SCRATCH BUFFER AREN'T OVERWRITTEN ON TURN OF FRAME
			buildAccelerationStructureInfo.SourceAccelerationStructure = AccelerationStructure();
			buildAccelerationStructureInfo.DestinationAccelerationStructure = buildDatas[GetCurrentFrame()][i].Destination;
			buildAccelerationStructureInfo.Geometries = geometryDescriptors[i];
			buildAccelerationStructureInfo.Flags = buildDatas[GetCurrentFrame()][i].BuildFlags;

			accelerationStructureBuildInfos.EmplaceBack(buildAccelerationStructureInfo);
			
			offset += GTSL::Math::RoundUpByPowerOf2(buildDatas[GetCurrentFrame()][i].ScratchBuildSize, scratchBufferOffsetAlignment);
		}
		
		commandBuffer.BuildAccelerationStructure(GetRenderDevice(), accelerationStructureBuildInfos, GetTransientAllocator());
		
		GTSL::Array<CommandList::BarrierData, 1> barriers;
		barriers.EmplaceBack(CommandList::MemoryBarrier{ GAL::AccessTypes::WRITE, GAL::AccessTypes::READ });
		
		commandBuffer.AddPipelineBarrier(GetRenderDevice(), barriers, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GetTransientAllocator());
	}
	
	buildDatas[GetCurrentFrame()].Resize(0);
	geometries[GetCurrentFrame()].Resize(0);
}

bool RenderSystem::resize()
{
	if (renderArea == 0) { return false; }
	//graphicsQueue.Wait(GetRenderDevice());

	if (!surface.GetHandle()) {
		//if constexpr (_DEBUG) { surfaceCreateInfo.Name = GTSL::StaticString<32>("Surface"); }

		GAL::WindowsWindowData windowsWindowData;
		
		if constexpr (_WIN64) {
			GTSL::Window::Win32NativeHandles handles;
			window->GetNativeHandles(&handles);
			windowsWindowData.InstanceHandle = GetModuleHandle(nullptr);
			windowsWindowData.WindowHandle = handles.HWND;
		}

#if __linux__
#endif
	
		surface.Initialize(GetRenderDevice(), windowsWindowData);
	}

	Surface::SurfaceCapabilities surfaceCapabilities;
	auto isSupported = surface.IsSupported(&renderDevice, &surfaceCapabilities);

	renderArea = surfaceCapabilities.CurrentExtent;
	
	if (!isSupported) {
		BE::Application::Get()->Close(BE::Application::CloseMode::ERROR, GTSL::StaticString<64>(u8"No supported surface found!"));
	}

	auto supportedPresentModes = surface.GetSupportedPresentModes(&renderDevice);
	swapchainPresentMode = supportedPresentModes[0];

	auto supportedSurfaceFormats = surface.GetSupportedFormatsAndColorSpaces(&renderDevice);

	{
		GTSL::Pair<GAL::ColorSpace, GAL::FormatDescriptor> bestColorSpaceFormat;

		for (uint8 topScore = 0; const auto& e : supportedSurfaceFormats) {
			uint8 score = 0;
			
			if (useHDR && e.First == GAL::ColorSpace::HDR10_ST2048) {
				score += 2;
			} else {
				score += 1;
			}

			if(score > topScore) {
				bestColorSpaceFormat = e;
				topScore = score;
			}
		}

		swapchainColorSpace = bestColorSpaceFormat.First; swapchainFormat = bestColorSpaceFormat.Second;
	}	

	renderContext.InitializeOrRecreate(GetRenderDevice(), &surface, renderArea, swapchainFormat, swapchainColorSpace, GAL::TextureUses::STORAGE | GAL::TextureUses::TRANSFER_DESTINATION, swapchainPresentMode, pipelinedFrames);

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	//imageIndex = 0;

	{
		auto newSwapchainTextures = renderContext.GetTextures(GetRenderDevice());
		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			swapchainTextures[f] = newSwapchainTextures[f];
			swapchainTextureViews[f].Destroy(GetRenderDevice());

			GTSL::StaticString<64> name(u8"Swapchain ImageView "); name += f;
			
			swapchainTextureViews[f].Initialize(GetRenderDevice(), name, swapchainTextures[f], swapchainFormat, renderArea, 1);
		}
	}

	lastRenderArea = renderArea;
	
	return true;
}

void RenderSystem::renderBegin(TaskInfo taskInfo)
{	
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	commandBuffer.BeginRecording(GetRenderDevice());

	if (BE::Application::Get()->GetOption(u8"rayTracing")) {
		GAL::Geometry geometry(GAL::GeometryInstances{ instancesBuffer[GetCurrentFrame()].GetAddress(GetRenderDevice()) }, GAL::GeometryFlag(), rayTracingInstancesCount, 0);
		//TODO: WHAT HAPPENS IF MESH IS REMOVED FROM THE MIDDLE OF THE COLLECTION, maybe: keep index of highest element in the colection		
		geometries[GetCurrentFrame()].EmplaceBack(geometry);

		AccelerationStructureBuildData buildData;
		buildData.BuildFlags = 0;
		buildData.Destination = topLevelAccelerationStructure[GetCurrentFrame()];
		buildData.ScratchBuildSize = topLevelStructureScratchSize;
		buildDatas[GetCurrentFrame()].EmplaceBack(buildData);

		buildAccelerationStructures(this, commandBuffer);
	}
}

void RenderSystem::renderFinish(TaskInfo taskInfo)
{
	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	commandBuffer.EndRecording(GetRenderDevice());

	{
		GTSL::Array<Queue::WorkUnit, 8> workUnits; GTSL::Array<GPUSemaphore, 8> presentWaitSemaphores;

		auto& workUnit = workUnits.EmplaceBack();

		workUnit.WaitSemaphore = &transferDoneSemaphores[GetCurrentFrame()];
		workUnit.WaitPipelineStage = GAL::PipelineStages::TRANSFER;
		
		if (surface.GetHandle()) {
			auto& graphicsWork = workUnits.EmplaceBack();
			graphicsWork.WaitSemaphore = &imageAvailableSemaphore[currentFrameIndex];
			graphicsWork.WaitPipelineStage = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
			graphicsWork.SignalSemaphore = &renderFinishedSemaphore[currentFrameIndex];
			graphicsWork.CommandBuffer = &graphicsCommandBuffers[currentFrameIndex];

			presentWaitSemaphores.EmplaceBack(renderFinishedSemaphore[currentFrameIndex]);
		}

		graphicsQueue.Submit(GetRenderDevice(), workUnits, graphicsFences[currentFrameIndex]);

		if (surface.GetHandle()) {
			renderContext.Present(GetRenderDevice(), presentWaitSemaphores, imageIndex, graphicsQueue);
		}
	}

	currentFrameIndex = (currentFrameIndex + 1) % pipelinedFrames;
}

void RenderSystem::frameStart(TaskInfo taskInfo)
{
	transferFences[GetCurrentFrame()].Wait(GetRenderDevice());

	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];

	GTSL::Array<uint32, 32> buffersToDelete;

	IndexedForEach(buffers, [&](const uint32 index, BufferData& e) {
		if (!e.references) {
			auto destroyBuffer = [&](BufferData& buffer) {
				buffer.Buffer.Destroy(GetRenderDevice());
				DeallocateLocalBufferMemory(buffer.Allocation);
				++buffer.references; //TODO: remove, there to avoid loop trying to delete chained buffers which will be already flagged to be deleted

				if (buffer.Staging != BufferHandle()) {
					auto& stagingBuffer = buffers[buffer.Staging()];
					stagingBuffer.Buffer.Destroy(GetRenderDevice());
					DeallocateScratchBufferMemory(stagingBuffer.Allocation);
					buffersToDelete.EmplaceBack(buffer.Staging());
					++stagingBuffer.references;
				}

				buffersToDelete.EmplaceBack(index);
			};

			if (e.Next() != 0xFFFFFFFF) {
				BufferHandle nextBufferHandle = e.Next;
				for (uint8 f = 1; f < pipelinedFrames; ++f) {
					auto& otherBuffer = buffers[nextBufferHandle()];
					auto currentHandle = nextBufferHandle;
					nextBufferHandle = otherBuffer.Next;
					destroyBuffer(otherBuffer);
				}
			}

			destroyBuffer(e);
		}
		});

	for (auto e : buffersToDelete)
		buffers.Pop(e);
	
	//if(transferFences[currentFrameIndex].GetStatus(&renderDevice))
	{		
		bufferCopyData.Pop(0, processedBufferCopies[GetCurrentFrame()]);
		
		transferFences[currentFrameIndex].Reset(GetRenderDevice());
	}
	
	//should only be done if frame is finished transferring but must also implement check in execute transfers
	//or begin command buffer complains
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	auto& commandBuffer = transferCommandBuffers[GetCurrentFrame()];
	
	commandBuffer.BeginRecording(GetRenderDevice());
	
	{
		auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
		
		for (auto& e : bufferCopyData) //TODO: What to do with multibuffers.
		{
			auto& buffer = buffers[e.Buffer()]; auto& stagingBuffer = buffers[buffer.Staging()];
			
			commandBuffer.CopyBuffer(GetRenderDevice(), stagingBuffer.Buffer, e.Offset, buffer.Buffer, 0, buffer.Size); //TODO: offset
			--stagingBuffer.references;
		}

		processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
	}
	
	if (auto & textureCopyData = textureCopyDatas[GetCurrentFrame()]; textureCopyData.GetLength())
	{

		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());
		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), textureCopyData.GetLength(), GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			sourceTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, textureCopyData[i].Format });
			destinationTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, textureCopyData[i].Format });
		}

		commandBuffer.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			commandBuffer.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
		}

		commandBuffer.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::FRAGMENT, GetTransientAllocator());
		textureCopyDatas[GetCurrentFrame()].Resize(0);
	}
		
	//processedTextureCopies[GetCurrentFrame()] = textureCopyData.GetLength();

	commandBuffer.EndRecording(GetRenderDevice());
	
	//if (bufferCopyDatas[currentFrameIndex].GetLength() || textureCopyDatas[GetCurrentFrame()].GetLength())
	//{
		GTSL::Array<GAL::Queue::WorkUnit, 8> workUnits;
		auto& workUnit = workUnits.EmplaceBack();
		workUnit.CommandBuffer = &commandBuffer;
		workUnit.WaitPipelineStage = GAL::PipelineStages::TRANSFER;
		workUnit.SignalSemaphore = &transferDoneSemaphores[GetCurrentFrame()];
	
		transferQueue.Submit(GetRenderDevice(), workUnits, transferFences[currentFrameIndex]);
	//}
}

RenderSystem::TextureHandle RenderSystem::CreateTexture(GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable)
{
	//RenderDevice::FindSupportedImageFormat findFormat;
	//findFormat.TextureTiling = TextureTiling::OPTIMAL;
	//findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	//GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	//findFormat.Candidates = candidates;
	//auto supportedFormat = renderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

	//GAL::Texture::ConvertTextureFormat(textureInfo.Format, GAL::TextureFormat::RGBA_I8, textureInfo.Extent, GTSL::AlignedPointer<byte, 16>(buffer.begin()), 1);

	static uint32 index = 0;
	
	TextureComponent textureComponent;

	textureComponent.Extent = extent;
	
	textureComponent.FormatDescriptor = formatDescriptor;

	textureComponent.Uses = textureUses;
	if (updatable) { textureComponent.Uses |= GAL::TextureUses::TRANSFER_DESTINATION; }

	textureComponent.Layout = GAL::TextureLayout::UNDEFINED;

	const auto textureSize = extent.Width * extent.Height * extent.Depth * formatDescriptor.GetSize();
	
	if (updatable && needsStagingBuffer) {	
		AllocateScratchBufferMemory(textureSize, GAL::BufferUses::TRANSFER_SOURCE, &textureComponent.ScratchBuffer, &textureComponent.ScratchAllocation);
	}
	
	AllocateLocalTextureMemory(textureSize, &textureComponent.Texture, textureComponent.Uses, textureComponent.FormatDescriptor, extent, GAL::Tiling::OPTIMAL,
		1, &textureComponent.Allocation);

	auto textureViewName = GTSL::StaticString<64>(u8"nnn"); textureViewName += index++;
	
	textureComponent.TextureView.Initialize(GetRenderDevice(), textureViewName, textureComponent.Texture, textureComponent.FormatDescriptor, extent, 1);
	textureComponent.TextureSampler.Initialize(GetRenderDevice(), 0);
	
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
	textureCopyData.Format = texture.FormatDescriptor;
	AddTextureCopy(textureCopyData);
	
	//TODO: QUEUE BUFFER DELETION
}

void RenderSystem::OnRenderEnable(TaskInfo taskInfo, bool oldFocus)
{
	if(!oldFocus)
	{
		const GTSL::Array<TaskDependency, 8> actsOn{ { u8"RenderSystem", AccessTypes::READ_WRITE } };
		taskInfo.GameInstance->AddTask(u8"frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, u8"FrameStart", u8"RenderStart");

		taskInfo.GameInstance->AddTask(u8"executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, u8"GameplayEnd", u8"RenderStart");
	
		taskInfo.GameInstance->AddTask(u8"renderStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderStart>(this), actsOn, u8"RenderStart", u8"RenderStartSetup");
		taskInfo.GameInstance->AddTask(u8"renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderBegin>(this), actsOn, u8"RenderEndSetup", u8"RenderDo");
	
		taskInfo.GameInstance->AddTask(u8"renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFinish>(this), actsOn, u8"RenderFinished", u8"RenderEnd");

		BE_LOG_SUCCESS("Enabled rendering")
	}

	OnResize(window->GetFramebufferExtent());
}

void RenderSystem::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	if (oldFocus)
	{
		taskInfo.GameInstance->RemoveTask(u8"frameStart", u8"FrameStart");
		taskInfo.GameInstance->RemoveTask(u8"executeTransfers", u8"GameplayEnd");
		taskInfo.GameInstance->RemoveTask(u8"renderStart", u8"RenderStart");
		taskInfo.GameInstance->RemoveTask(u8"renderSetup", u8"RenderEndSetup");
		taskInfo.GameInstance->RemoveTask(u8"renderFinished", u8"RenderFinished");

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
	} else {
		resize(); result = true; AcquireImage();
	}

	if (lastRenderArea != renderArea) { resize(); result = true; }
	
	return result;
}

BufferHandle RenderSystem::CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, bool updateable)
{
	uint32 bufferIndex = 0;

	bufferIndex = buffers.Emplace(); auto& buffer = buffers[bufferIndex];

	buffer.Size = size; buffer.Flags = flags;
	++buffer.references;

	if (willWriteFromHost) {
		if (needsStagingBuffer) { //create staging buffer
			auto stagingBufferIndex = buffers.Emplace(); auto& stagingBuffer = buffers[stagingBufferIndex];

			++stagingBuffer.references;
			AllocateScratchBufferMemory(size, flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE,
				&stagingBuffer.Buffer, &stagingBuffer.Allocation);

			buffer.Staging = BufferHandle(stagingBufferIndex);

			flags |= GAL::BufferUses::TRANSFER_DESTINATION;
		}
	}

	AllocateLocalBufferMemory(size, flags | GAL::BufferUses::ADDRESS, &buffer.Buffer, &buffer.Allocation);
	
	if (updateable) {		
		uint32 lastBuffer = bufferIndex;
		
		for (uint8 f = 1; f < pipelinedFrames; ++f) {
			auto nextBufferIndex = buffers.Emplace(); auto& nextBuffer = buffers[nextBufferIndex];

			if (needsStagingBuffer) { //create staging buffer
				auto stagingBufferIndex = buffers.Emplace(); auto& stagingBuffer = buffers[stagingBufferIndex];

				++stagingBuffer.references;
				AllocateScratchBufferMemory(size, flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE,
					&stagingBuffer.Buffer, &stagingBuffer.Allocation);

				nextBuffer.Staging = BufferHandle(stagingBufferIndex);

				flags |= GAL::BufferUses::TRANSFER_DESTINATION;
			}
			
			AllocateLocalBufferMemory(size, flags | GAL::BufferUses::ADDRESS, &nextBuffer.Buffer, &nextBuffer.Allocation);

			buffers[lastBuffer].Next = BufferHandle(nextBufferIndex);
			lastBuffer = nextBufferIndex;
		}		
	}	

	return BufferHandle(bufferIndex);
}

void RenderSystem::SetBufferWillWriteFromHost(BufferHandle bufferHandle, bool state)
{
	auto& buffer = buffers[bufferHandle()];
	
	if(state) {
		if(!buffer.Staging) {//if will write from host and we have no buffer
			if (needsStagingBuffer) {
				auto stagingBufferIndex = buffers.Emplace(); auto& stagingBuffer = buffers[stagingBufferIndex];

				AllocateScratchBufferMemory(buffer.Size, buffer.Flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE | GAL::BufferUses::STORAGE,
					&stagingBuffer.Buffer, &stagingBuffer.Allocation);

				buffer.Staging = BufferHandle(stagingBufferIndex);
			}
		}

		//if will write from host and we have buffer, do nothing
	} else {
		if (buffer.Staging) { //if won't write from host and we have a buffer
			if (needsStagingBuffer) {
				auto& stagingBuffer = buffers[buffer.Staging()];
				--stagingBuffer.references;
			}
		}

		//if won't write from host and we have no buffer, do nothing
	}
}

void RenderSystem::printError(const char* message, const RenderDevice::MessageSeverity messageSeverity) const
{
	switch (messageSeverity)
	{
	case RenderDevice::MessageSeverity::MESSAGE: BE_LOG_MESSAGE(message) break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message) break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); break;
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
