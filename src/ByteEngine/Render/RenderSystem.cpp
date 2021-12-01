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

//RenderSystem::MeshHandle RenderSystem::CreateMesh(Id name, uint32 customIndex, uint32 vertexCount, uint32 vertexSize, const uint32 indexCount, const uint32 indexSize, MaterialInstanceHandle materialHandle)
//{
//	auto meshIndex = meshes.Emplace(); auto& mesh = meshes[meshIndex];
//	mesh.CustomMeshIndex = customIndex;
//	mesh.MaterialHandle = materialHandle;
//
//	auto meshHandle = MeshHandle(meshIndex);
//	
//	SignalMeshDataUpdate(meshHandle, vertexCount, vertexSize, indexCount, indexSize);
//	return meshHandle;
//}

RenderSystem::RenderSystem(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderSystem"),
	bufferCopyDatas{ { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() } },
	textureCopyDatas{ { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() }, { 16, GetPersistentAllocator() } },
	bottomLevelAccelerationStructures(GetPersistentAllocator()), buffers(32, GetPersistentAllocator()),
	buildDatas{ GetPersistentAllocator(), GetPersistentAllocator(), GetPersistentAllocator() }, geometries{ GetPersistentAllocator(), GetPersistentAllocator(), GetPersistentAllocator() }, pipelineCaches(16, decltype(pipelineCaches)::allocator_t()),
	textures(16, GetPersistentAllocator()), apiAllocations(128, GetPersistentAllocator())
{
	{
		//initializeInfo.ApplicationManager->AddTask(u8"RenderSystem::executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, u8"GameplayEnd", u8"RenderStart");
		//initializeInfo.ApplicationManager->AddTask(u8"RenderSystem::waitForFences", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::waitForFences>(this), actsOn, u8"RenderStart", u8"RenderStartSetup");
		initializeInfo.GameInstance->AddTask(this, u8"frameStart", &RenderSystem::frameStart, DependencyBlock(), u8"FrameStart", u8"RenderStart");
		initializeInfo.GameInstance->AddTask(this, u8"beginCommandLists", &RenderSystem::beginGraphicsCommandLists, DependencyBlock(), u8"RenderEndSetup", u8"RenderDo");
		initializeInfo.GameInstance->AddTask(this, u8"endCommandLists", &RenderSystem::renderFlush, DependencyBlock(), u8"RenderFinished", u8"RenderEnd");
		resizeHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"onResize", {}, & RenderSystem::onResize);
	}

	RenderDevice::RayTracingCapabilities rayTracingCapabilities;

	useHDR = BE::Application::Get()->GetOption(u8"hdr");
	pipelinedFrames = static_cast<uint8>(GTSL::Math::Clamp(BE::Application::Get()->GetOption(u8"buffer"), 2u, 3u));
	bool rayTracing = BE::Application::Get()->GetOption(u8"rayTracing");

	{
		RenderDevice::CreateInfo createInfo;
		createInfo.ApplicationName = GTSL::StaticString<128>(BE::Application::Get()->GetApplicationName());
		createInfo.ApplicationVersion[0] = 0; createInfo.ApplicationVersion[1] = 0; createInfo.ApplicationVersion[2] = 0;

		createInfo.Debug = BE::Application::Get()->GetOption(u8"debug");

		GTSL::StaticVector<GAL::QueueType, 5> queue_create_infos;
		GTSL::StaticVector<RenderDevice::QueueKey, 5> queueKeys;

		queue_create_infos.EmplaceBack(GAL::QueueTypes::GRAPHICS); queueKeys.EmplaceBack();
		//queue_create_infos.EmplaceBack(GAL::QueueTypes::TRANSFER); queueKeys.EmplaceBack();

		createInfo.Queues = queue_create_infos;
		createInfo.QueueKeys = queueKeys;

		GTSL::StaticVector<GTSL::Pair<RenderDevice::Extension, void*>, 8> extensions{ { RenderDevice::Extension::PIPELINE_CACHE_EXTERNAL_SYNC, nullptr } };
		extensions.EmplaceBack(RenderDevice::Extension::SWAPCHAIN_RENDERING, nullptr);
		extensions.EmplaceBack(RenderDevice::Extension::SCALAR_LAYOUT, nullptr);
		if (rayTracing) { extensions.EmplaceBack(RenderDevice::Extension::RAY_TRACING, &rayTracingCapabilities); }

		createInfo.Extensions = extensions;
		createInfo.PerformanceValidation = true;
		createInfo.SynchronizationValidation = true;
		createInfo.DebugPrintFunction = GTSL::Delegate<void(GTSL::StringView, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
		createInfo.AllocationInfo.UserData = this;
		createInfo.AllocationInfo.Allocate = GTSL::Delegate<void* (void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::allocateApiMemory>(this);
		createInfo.AllocationInfo.Reallocate = GTSL::Delegate<void* (void*, void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::reallocateApiMemory>(this);
		createInfo.AllocationInfo.Deallocate = GTSL::Delegate<void(void*, void*)>::Create<RenderSystem, &RenderSystem::deallocateApiMemory>(this);

		if (auto renderDeviceInitializationResult = renderDevice.Initialize(createInfo, GetTransientAllocator())) {
			BE_LOG_SUCCESS(u8"Started RenderDevice\n	API: Vulkan\n	GPU: ", renderDevice.GetGPUInfo().GPUName, u8"\n	Memory: ", 6, u8" GB\n	API Version: ", renderDevice.GetGPUInfo().APIVersion);
		} else {
			BE_LOG_ERROR(u8"Failed to initialize RenderDevice!\n	API: Vulkan\n	Reason: \"", renderDeviceInitializationResult.Get(), u8"\n");
		}

		graphicsQueue.Initialize(GetRenderDevice(), queueKeys[0]);
		//transferQueue.Initialize(GetRenderDevice(), queueKeys[1]);

		{
			needsStagingBuffer = true;

			auto memoryHeaps = renderDevice.GetMemoryHeaps(); GAL::VulkanRenderDevice::MemoryHeap& biggestGPUHeap = memoryHeaps[0];

			for (auto& e : memoryHeaps) {
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
			CreateBuffer(GTSL::Byte(GTSL::MegaByte(1)), GAL::BufferUses::BUILD_INPUT_READ, true, false);

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
			default:;
			}
		}
	}

	for (uint8 f = 0; f < pipelinedFrames; ++f) {
		initializeFrameResources(f);
	}

	bool pipelineCacheAvailable;
	auto* pipelineCacheManager = initializeInfo.GameInstance->GetSystem<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
	pipelineCacheManager->DoesCacheExist(pipelineCacheAvailable);

	if (pipelineCacheAvailable) {
		uint32 cacheSize = 0;
		pipelineCacheManager->GetCacheSize(cacheSize);

		GTSL::Buffer pipelineCacheBuffer(cacheSize, 32, GetTransientAllocator());

		pipelineCacheManager->GetCache(pipelineCacheBuffer);

		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
			if constexpr (_DEBUG) {
				//GTSL::StaticString<32> name(u8"Pipeline cache. Thread: "); name += i;
			}

			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, static_cast<GTSL::Range<const GTSL::byte*>>(pipelineCacheBuffer));
		}
	} else {
		for (uint8 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
			if constexpr (_DEBUG) {
				//GTSL::StaticString<32> name(u8"Pipeline cache. Thread: "); name += i;
			}

			pipelineCaches.EmplaceBack().Initialize(GetRenderDevice(), true, {});
		}
	}

	BE_LOG_MESSAGE(u8"Initialized successfully");
}

RenderSystem::~RenderSystem() {
	//graphicsQueue.Wait(GetRenderDevice()); transferQueue.Wait(GetRenderDevice());
	renderDevice.Wait();

	for (uint32 i = 0; i < pipelinedFrames; ++i) {
		freeFrameResources(i);
	}

	if (renderContext.GetHandle())
		renderContext.Destroy(&renderDevice);

	if (surface.GetHandle())
		surface.Destroy(&renderDevice);

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
			//auto* pipelineCacheResourceManager = shutdownInfo.GameInstance->GetSystem<PipelineCacheResourceManager>(u8"PipelineCacheResourceManager");
			//
			//GTSL::Buffer pipelineCacheBuffer(cacheSize, 32, GetTransientAllocator());
			//pipelineCache.GetCache(&renderDevice, pipelineCacheBuffer);
			//pipelineCacheResourceManager->WriteCache(pipelineCacheBuffer);
		}
	}
}

void RenderSystem::buildAccelerationStructuresOnDevice(CommandList& commandBuffer)
{
	if (buildDatas[GetCurrentFrame()].GetLength()) {
		GTSL::StaticVector<GAL::BuildAccelerationStructureInfo, 8> accelerationStructureBuildInfos;
		GTSL::StaticVector<GTSL::StaticVector<GAL::Geometry, 8>, 16> geometryDescriptors;

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
		
		GTSL::StaticVector<CommandList::BarrierData, 1> barriers;
		barriers.EmplaceBack(GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::MemoryBarrier{});
		
		commandBuffer.AddPipelineBarrier(GetRenderDevice(), barriers, GetTransientAllocator());
	}
	
	buildDatas[GetCurrentFrame()].Resize(0);
	geometries[GetCurrentFrame()].Resize(0);
}

void RenderSystem::beginGraphicsCommandLists(TaskInfo taskInfo)
{	
	auto& commandBuffer = graphicsCommandBuffers[GetCurrentFrame()];

	graphicsFences[GetCurrentFrame()].Wait(GetRenderDevice());
	graphicsFences[GetCurrentFrame()].Reset(GetRenderDevice());
	
	commandBuffer.BeginRecording(GetRenderDevice());

	for (auto& e : topLevelAccelerationStructures) {
		GAL::Geometry geometry(GAL::GeometryInstances{ GetBufferDeviceAddress(e.InstancesBuffer) }, GAL::GeometryFlag(), e.ScratchSize, 0); //TODO
		geometries[GetCurrentFrame()].EmplaceBack(geometry);

		AccelerationStructureBuildData buildData;
		buildData.BuildFlags = 0;
		buildData.Destination = e.AccelerationStructures[GetCurrentFrame()];
		buildData.ScratchBuildSize = e.ScratchSize;
		buildDatas[GetCurrentFrame()].EmplaceBack(buildData);

		buildAccelerationStructures(this, commandBuffer);
	}

	{
		auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];

		for (auto& e : bufferCopyData) {
			auto& buffer = buffers[e.BufferHandle()];

			if (buffer.isMulti) {
				__debugbreak();
			} else {
				commandBuffer.CopyBuffer(GetRenderDevice(), buffer.Staging[0], e.Offset, buffer.Buffer[0], 0, buffer.Size); //TODO: offset
				--buffer.references;
			}
		}

		processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
	}

	if (auto& textureCopyData = textureCopyDatas[GetCurrentFrame()]; textureCopyData) {
		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
		GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			sourceTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format });
			destinationTextureBarriers.EmplaceBack(GAL::PipelineStages::TRANSFER, GAL::PipelineStages::FRAGMENT, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, textureCopyData[i].Format });
		}

		commandBuffer.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GetTransientAllocator());

		for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
			commandBuffer.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
		}

		commandBuffer.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GetTransientAllocator());
		textureCopyDatas[GetCurrentFrame()].Resize(0);
	}
}

void RenderSystem::renderFlush(TaskInfo taskInfo) {
	auto& commandBuffer = graphicsCommandBuffers[GetCurrentFrame()];

	auto beforeFrame = uint8(currentFrameIndex - uint8(1)) % GetPipelinedFrames();
	
	for(auto& e : buffers) {
		if (e.isMulti) {
			if (e.writeMask[beforeFrame] && !e.writeMask[GetCurrentFrame()]) {
				GTSL::MemCopy(e.Size, e.StagingAllocation[beforeFrame].Data, e.StagingAllocation[GetCurrentFrame()].Data);
			}
		}
		
		e.writeMask[GetCurrentFrame()] = false;
	}
	
	commandBuffer.EndRecording(GetRenderDevice());

	{
		GTSL::StaticVector<Queue::WorkUnit, 8> workUnits;

		auto& graphicsWork = workUnits.EmplaceBack();

		graphicsWork.WaitSemaphore = &imageAvailableSemaphore[GetCurrentFrame()];

		graphicsWork.WaitPipelineStage = GAL::PipelineStages::TRANSFER;
		graphicsWork.SignalSemaphore = &renderFinishedSemaphore[GetCurrentFrame()];
		graphicsWork.CommandBuffer = &graphicsCommandBuffers[GetCurrentFrame()];		
		
		if (surface.GetHandle()) {
			graphicsWork.WaitPipelineStage |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
		}

		graphicsQueue.Submit(GetRenderDevice(), workUnits, graphicsFences[GetCurrentFrame()]);
		
		GTSL::StaticVector<GPUSemaphore*, 8> presentWaitSemaphores;

		if(surface.GetHandle()) {
			presentWaitSemaphores.EmplaceBack(&renderFinishedSemaphore[GetCurrentFrame()]);

			if(!renderContext.Present(GetRenderDevice(), presentWaitSemaphores, imageIndex, graphicsQueue)) {
				resize();
			}
		}
	}

	++currentFrameIndex %= pipelinedFrames;
}

void RenderSystem::frameStart(TaskInfo taskInfo)
{
	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	auto& textureCopyData = textureCopyDatas[GetCurrentFrame()];
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	//auto& commandBuffer = transferCommandBuffers[GetCurrentFrame()];
	//auto& commandBuffer = graphicsCommandBuffers[GetCurrentFrame()];
	
	//commandBuffer.BeginRecording(GetRenderDevice());
	
	//{
	//	auto& bufferCopyData = bufferCopyDatas[GetCurrentFrame()];
	//	
	//	for (auto& e : bufferCopyData) //TODO: What to do with multibuffers.
	//	{
	//		auto& buffer = buffers[e.Buffer()]; auto& stagingBuffer = buffers[buffer.Staging()];
	//		
	//		commandBuffer.CopyBuffer(GetRenderDevice(), stagingBuffer.Buffer, e.Offset, buffer.Buffer, 0, buffer.Size); //TODO: offset
	//		--stagingBuffer.references;
	//	}
	//
	//	processedBufferCopies[GetCurrentFrame()] = bufferCopyData.GetLength();
	//}
	//
	//if (auto & textureCopyData = textureCopyDatas[GetCurrentFrame()]; textureCopyData.GetLength())
	//{
	//	GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> sourceTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
	//	GTSL::Vector<CommandList::BarrierData, BE::TransientAllocatorReference> destinationTextureBarriers(textureCopyData.GetLength(), GetTransientAllocator());
	//
	//	for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
	//		sourceTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, textureCopyData[i].Format });
	//		destinationTextureBarriers.EmplaceBack(CommandList::TextureBarrier{ &textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, GAL::TextureLayout::SHADER_READ, GAL::AccessTypes::WRITE, GAL::AccessTypes::READ, textureCopyData[i].Format });
	//	}
	//
	//	commandBuffer.AddPipelineBarrier(GetRenderDevice(), sourceTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
	//
	//	for (uint32 i = 0; i < textureCopyData.GetLength(); ++i) {
	//		commandBuffer.CopyBufferToTexture(GetRenderDevice(), textureCopyData[i].SourceBuffer, textureCopyData[i].DestinationTexture, GAL::TextureLayout::TRANSFER_DESTINATION, textureCopyData[i].Format, textureCopyData[i].Extent);
	//	}
	//
	//	commandBuffer.AddPipelineBarrier(GetRenderDevice(), destinationTextureBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::FRAGMENT, GetTransientAllocator());
	//	textureCopyDatas[GetCurrentFrame()].Resize(0);
	//}
		
	//processedTextureCopies[GetCurrentFrame()] = textureCopyData.GetLength();

	//commandBuffer.EndRecording(GetRenderDevice());
	
	////if (bufferCopyDatas[currentFrameIndex].GetLength() || textureCopyDatas[GetCurrentFrame()].GetLength())
	////{
	//	GTSL::StaticVector<GAL::Queue::WorkUnit, 8> workUnits;
	//	auto& workUnit = workUnits.EmplaceBack();
	//	workUnit.CommandBuffer = &commandBuffer;
	//	workUnit.WaitPipelineStage = GAL::PipelineStages::TRANSFER;
	//	workUnit.SignalSemaphore = &transferDoneSemaphores[GetCurrentFrame()];
	//
	//	graphicsQueue.Submit(GetRenderDevice(), workUnits, transferFences[currentFrameIndex]);
	////}
}

RenderSystem::TextureHandle RenderSystem::CreateTexture(GTSL::Range<const char8_t*> name, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, GAL::TextureUse textureUses, bool updatable)
{
	//RenderDevice::FindSupportedImageFormat findFormat;
	//findFormat.TextureTiling = TextureTiling::OPTIMAL;
	//findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	//GTSL::StaticVector<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
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
	
	AllocateLocalTextureMemory(&textureComponent.Texture, name, textureComponent.Uses, textureComponent.FormatDescriptor, extent, GAL::Tiling::OPTIMAL,
		1, &textureComponent.Allocation);
	
	textureComponent.TextureView.Initialize(GetRenderDevice(), name, textureComponent.Texture, textureComponent.FormatDescriptor, extent, 1);
	
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
		//const GTSL::StaticVector<TaskDependency, 8> actsOn{ { u8"RenderSystem", AccessTypes::READ_WRITE } };
		//taskInfo.ApplicationManager->AddTask(u8"frameStart", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, u8"FrameStart", u8"RenderStart");
		//taskInfo.ApplicationManager->AddTask(u8"executeTransfers", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, u8"GameplayEnd", u8"RenderStart");
		//taskInfo.ApplicationManager->AddTask(u8"renderSetup", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::beginGraphicsCommandLists>(this), actsOn, u8"RenderEndSetup", u8"RenderDo");
		//taskInfo.ApplicationManager->AddTask(u8"renderFinished", GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::renderFlush>(this), actsOn, u8"RenderFinished", u8"RenderEnd");

		BE_LOG_SUCCESS(u8"Enabled rendering")
	}

	//OnResize(window->GetFramebufferExtent());
}

void RenderSystem::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	if (oldFocus)
	{
		//taskInfo.ApplicationManager->RemoveTask(u8"frameStart", u8"FrameStart");
		//taskInfo.ApplicationManager->RemoveTask(u8"executeTransfers", u8"GameplayEnd");
		//taskInfo.ApplicationManager->RemoveTask(u8"waitForFences", u8"RenderStart");
		//taskInfo.ApplicationManager->RemoveTask(u8"renderSetup", u8"RenderEndSetup");
		//taskInfo.ApplicationManager->RemoveTask(u8"renderFinished", u8"RenderFinished");

		BE_LOG_SUCCESS(u8"Disabled rendering")
	}
}

GTSL::Result<GTSL::Extent2D> RenderSystem::AcquireImage()
{
	bool result = false;
	
	if(!surface.GetHandle()) {
		resize(); result = true;
	}

	const auto acquireResult = renderContext.AcquireNextImage(&renderDevice, &imageAvailableSemaphore[GetCurrentFrame()]);

	imageIndex = acquireResult.Get();

	switch (acquireResult.State()) {
	case GAL::VulkanRenderContext::AcquireState::OK: break;
	case GAL::VulkanRenderContext::AcquireState::SUBOPTIMAL:
	case GAL::VulkanRenderContext::AcquireState::BAD: resize(); result = true; break;
	}

	if (lastRenderArea != renderArea) { lastRenderArea = renderArea; result = true; }
	
	return { GTSL::MoveRef(renderArea), result };
}

void RenderSystem::resize() {
	if (!surface.GetHandle()) {
		surface.Initialize(GetRenderDevice(), BE::Application::Get()->GetApplication(), *window);
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

		for (uint8 topScore = 0; const auto & e : supportedSurfaceFormats) {
			uint8 score = 0;

			if (useHDR && e.First == GAL::ColorSpace::HDR10_ST2048) {
				score += 2;
			}
			else {
				score += 1;
			}

			if (score > topScore) {
				bestColorSpaceFormat = e;
				topScore = score;
			}
		}

		swapchainColorSpace = bestColorSpaceFormat.First; swapchainFormat = bestColorSpaceFormat.Second;
	}

	renderContext.InitializeOrRecreate(GetRenderDevice(), graphicsQueue, &surface, renderArea, swapchainFormat, swapchainColorSpace, GAL::TextureUses::STORAGE | GAL::TextureUses::TRANSFER_DESTINATION, swapchainPresentMode, pipelinedFrames);

	for (auto& e : swapchainTextureViews) { e.Destroy(&renderDevice); }

	//imageIndex = 0; keep index of last acquired image

	{
		auto newSwapchainTextures = renderContext.GetTextures(GetRenderDevice());
		for (uint8 f = 0; f < pipelinedFrames; ++f) {
			swapchainTextures[f] = newSwapchainTextures[f];
			swapchainTextureViews[f].Destroy(GetRenderDevice());

			GTSL::StaticString<64> name(u8"Swapchain ImageView "); name += f;

			swapchainTextureViews[f].Initialize(GetRenderDevice(), name, swapchainTextures[f], swapchainFormat, renderArea, 1);
		}
	}
}

RenderSystem::BufferHandle RenderSystem::CreateBuffer(uint32 size, GAL::BufferUse flags, bool willWriteFromHost, bool updateable)
{
	uint32 bufferIndex = buffers.Emplace(); auto& buffer = buffers[bufferIndex];

	buffer.isMulti = updateable;
	buffer.Size = size; buffer.Flags = flags;
	++buffer.references;

	auto frames = updateable ? GetPipelinedFrames() : 1;
	
	for (uint8 f = 0; f < frames; ++f) {
		if (willWriteFromHost) {
			if (needsStagingBuffer) { //create staging buffer			
				AllocateScratchBufferMemory(size, flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE,
					&buffer.Staging[f], &buffer.StagingAllocation[f]);

				flags |= GAL::BufferUses::TRANSFER_DESTINATION;
			}
		}

		AllocateLocalBufferMemory(size, flags | GAL::BufferUses::ADDRESS, &buffer.Buffer[f], &buffer.Allocation[f]);
	}

	return BufferHandle(bufferIndex);
}

void RenderSystem::SetBufferWillWriteFromHost(BufferHandle bufferHandle, bool state)
{
	auto& buffer = buffers[bufferHandle()];

	if (buffer.isMulti) {
		__debugbreak();
	}
	
	if(state) {		
		if(!buffer.Staging[0].GetVkBuffer()) {//if will write from host and we have no buffer
			if (needsStagingBuffer) {
				AllocateScratchBufferMemory(buffer.Size, buffer.Flags | GAL::BufferUses::ADDRESS | GAL::BufferUses::TRANSFER_SOURCE | GAL::BufferUses::STORAGE,
					&buffer.Staging[0], &buffer.StagingAllocation[0]);
			}
		}

		//if will write from host and we have buffer, do nothing
	} else {
		if (buffer.Staging[0].GetVkBuffer()) { //if won't write from host and we have a buffer
			if (needsStagingBuffer) {
				--buffer.references; //todo: what
			}
		}

		//if won't write from host and we have no buffer, do nothing
	}
}

void RenderSystem::printError(GTSL::StringView message, const RenderDevice::MessageSeverity messageSeverity) const {
	bool breakeablelogLevel = false;

	switch (messageSeverity) {
	//case RenderDevice::MessageSeverity::MESSAGE: BE_LOG_MESSAGE(message) break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message); break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); breakeablelogLevel = true; break;
	default: break;
	}

	if(breakOnError && breakeablelogLevel) {
		__debugbreak();
	}
}

void* RenderSystem::allocateApiMemory(void* data, const uint64 size, const uint64 alignment) {
	void* allocation; uint64 allocated_size;
	GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);

	{
		GTSL::Lock lock(allocationsMutex);		
		apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair(size, alignment));
	}

	return allocation;
}

void* RenderSystem::reallocateApiMemory(void* data, void* oldAllocation, uint64 size, uint64 alignment) {
	void* allocation; uint64 allocated_size;

	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		old_alloc = apiAllocations[reinterpret_cast<uint64>(oldAllocation)];
	}
	
	GetPersistentAllocator().Allocate(size, old_alloc.Second, &allocation, &allocated_size);
	apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), GTSL::Pair(size, alignment));
	
	GTSL::MemCopy(old_alloc.First, oldAllocation, allocation);
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
	
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
	}
	
	return allocation;
}

void RenderSystem::deallocateApiMemory(void* data, void* allocation) {
	GTSL::Pair<uint64, uint64> old_alloc;
	
	{
		GTSL::Lock lock(allocationsMutex);
		old_alloc = apiAllocations[reinterpret_cast<uint64>(allocation)];
	}
	
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, allocation);
	
	{
		GTSL::Lock lock(allocationsMutex);
		apiAllocations.Remove(reinterpret_cast<uint64>(allocation));
	}
}

void RenderSystem::initializeFrameResources(const uint8 frame_index) {	
	if constexpr (_DEBUG) { GTSL::StaticString<32> name(u8"Transfer semaphore. Frame: "); name += frame_index; }
	processedBufferCopies[frame_index] = 0;

	if constexpr (_DEBUG) {
		//GTSL::StaticString<32> name("ImageAvailableSemaphore #"); name += i;
	}
	imageAvailableSemaphore[frame_index].Initialize(GetRenderDevice());

	if constexpr (_DEBUG) {
		//GTSL::StaticString<32> renderFinishedSe(u8"RenderFinishedSemaphore #"); renderFinishedSe += i;
	}
	renderFinishedSemaphore[frame_index].Initialize(GetRenderDevice());

	if constexpr (_DEBUG) {
		//GTSL::StaticString<32> name(u8"InFlightFence #"); name += i;
	}

	graphicsFences[frame_index].Initialize(GetRenderDevice(), true);

	graphicsCommandBuffers[frame_index].Initialize(GetRenderDevice(), graphicsQueue.GetQueueKey());

	for(auto& e : topLevelAccelerationStructures) {
		e.AccelerationStructures[frame_index];
	}
}

void RenderSystem::freeFrameResources(const uint8 frameIndex) {
	graphicsCommandBuffers[frameIndex].Destroy(&renderDevice);

	imageAvailableSemaphore[frameIndex].Destroy(&renderDevice);
	renderFinishedSemaphore[frameIndex].Destroy(&renderDevice);
	graphicsFences[frameIndex].Destroy(&renderDevice);
}
