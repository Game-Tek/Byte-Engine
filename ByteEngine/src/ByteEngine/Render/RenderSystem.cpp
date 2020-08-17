#include "RenderSystem.h"

#include <GTSL/Window.h>
#include <Windows.h>

#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Game/CameraSystem.h"

class CameraSystem;
class RenderStaticMeshCollection;

void RenderSystem::InitializeRenderer(const InitializeRendererInfo& initializeRenderer)
{
	renderGroups.Initialize(16, GetPersistentAllocator());
	apiAllocations.Initialize(16, GetPersistentAllocator());

	{		
		RenderDevice::CreateInfo create_info;
		create_info.ApplicationName = GTSL::StaticString<128>("Test");
		GTSL::Array<GAL::Queue::CreateInfo, 5> queue_create_infos(2);
		queue_create_infos[0].Capabilities = static_cast<uint8>(QueueCapabilities::GRAPHICS);
		queue_create_infos[0].QueuePriority = 1.0f;
		queue_create_infos[1].Capabilities = static_cast<uint8>(QueueCapabilities::TRANSFER);
		queue_create_infos[1].QueuePriority = 1.0f;
		create_info.QueueCreateInfos = queue_create_infos;
		auto queues = GTSL::Array<Queue, 5>{ graphicsQueue, transferQueue };
		create_info.Queues = queues;
		create_info.DebugPrintFunction = GTSL::Delegate<void(const char*, RenderDevice::MessageSeverity)>::Create<RenderSystem, &RenderSystem::printError>(this);
		create_info.AllocationInfo.UserData = this;
		create_info.AllocationInfo.Allocate = GTSL::Delegate<void*(void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::allocateApiMemory>(this);
		create_info.AllocationInfo.Reallocate = GTSL::Delegate<void*(void*, void*, uint64, uint64)>::Create<RenderSystem, &RenderSystem::reallocateApiMemory>(this);
		create_info.AllocationInfo.Deallocate = GTSL::Delegate<void(void*, void*)>::Create<RenderSystem, &RenderSystem::deallocateApiMemory>(this);
		::new(&renderDevice) RenderDevice(create_info);

		graphicsQueue = queues[0]; transferQueue = queues[1];
	}
	
	swapchainPresentMode = static_cast<uint32>(PresentMode::FIFO);
	swapchainColorSpace = static_cast<uint32>(ColorSpace::NONLINEAR_SRGB);
	swapchainFormat = static_cast<uint32>(ImageFormat::BGRA_I8);
	
	Surface::CreateInfo surface_create_info;
	surface_create_info.RenderDevice = &renderDevice;
	surface_create_info.Name = "Surface";
	GAL::WindowsWindowData window_data;
	GTSL::Window::Win32NativeHandles native_handles;
	initializeRenderer.Window->GetNativeHandles(&native_handles);
	window_data.WindowHandle = native_handles.HWND;
	window_data.InstanceHandle = GetModuleHandleA(nullptr);
	surface_create_info.SystemData = &window_data;
	::new(&surface) Surface(surface_create_info);

	BE_ASSERT(surface.IsSupported(&renderDevice) != false, "Surface is not supported!");

	GTSL::Array<GTSL::uint32, 4> present_modes{ swapchainPresentMode };
	auto res = surface.GetSupportedPresentMode(&renderDevice, present_modes);
	if (res != 0xFFFFFFFF) { swapchainPresentMode = present_modes[res]; }

	GTSL::Array<GTSL::Pair<uint32, uint32>, 8> surface_formats{ { swapchainColorSpace, swapchainFormat } };
	res = surface.GetSupportedRenderContextFormat(&renderDevice, surface_formats);
	if (res != 0xFFFFFFFF) { swapchainColorSpace = surface_formats[res].First; swapchainFormat = surface_formats[res].Second; }

	RenderPass::CreateInfo render_pass_create_info;
	render_pass_create_info.RenderDevice = &renderDevice;
	render_pass_create_info.Descriptor.DepthStencilAttachmentAvailable = false;
	GTSL::Array<GAL::AttachmentDescriptor, 8> attachment_descriptors;
	attachment_descriptors.PushBack(GAL::AttachmentDescriptor{ (uint32)ImageFormat::BGRA_I8, GAL::RenderTargetLoadOperations::CLEAR, GAL::RenderTargetStoreOperations::STORE, GAL::ImageLayout::UNDEFINED, GAL::ImageLayout::PRESENTATION });
	render_pass_create_info.Descriptor.RenderPassColorAttachments = attachment_descriptors;

	GTSL::Array<GAL::AttachmentReference, 8> write_attachment_references;
	write_attachment_references.PushBack(GAL::AttachmentReference{ 0, GAL::ImageLayout::COLOR_ATTACHMENT });

	GTSL::Array<GAL::SubPassDescriptor, 8> sub_pass_descriptors;
	sub_pass_descriptors.PushBack(GAL::SubPassDescriptor{ GTSL::Ranger<GAL::AttachmentReference>(), write_attachment_references, GTSL::Ranger<uint8>(), nullptr });
	render_pass_create_info.Descriptor.SubPasses = sub_pass_descriptors;

	new(&renderPass) RenderPass(render_pass_create_info);
	
	RenderContext::CreateInfo render_context_create_info;
	render_context_create_info.Name = "Render System Render Context";
	render_context_create_info.RenderDevice = &renderDevice;
	render_context_create_info.DesiredFramesInFlight = 2;
	render_context_create_info.PresentMode = swapchainPresentMode;
	render_context_create_info.Format = swapchainFormat;
	render_context_create_info.ColorSpace = swapchainColorSpace;
	render_context_create_info.ImageUses = ImageUse::COLOR_ATTACHMENT;
	render_context_create_info.Surface = &surface;
	GTSL::Extent2D window_extent;
	initializeRenderer.Window->GetFramebufferExtent(window_extent);
	render_context_create_info.SurfaceArea = window_extent;
	new(&renderContext) RenderContext(render_context_create_info);

	initializeRenderer.Window->GetFramebufferExtent(renderArea);
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	get_images_info.SwapchainImagesFormat = swapchainFormat;
	get_images_info.ImageViewName = GTSL::StaticString<64>("Swapchain image view. Frame: ");
	swapchainImages = renderContext.GetImages(get_images_info);

	clearValues.EmplaceBack(0, 0, 0, 0);

	for (uint32 i = 0; i < swapchainImages.GetLength(); ++i)
	{
		Semaphore::CreateInfo semaphore_create_info;
		semaphore_create_info.RenderDevice = &renderDevice;
		semaphore_create_info.Name = "ImageAvailableSemaphore";
		imageAvailableSemaphore.EmplaceBack(semaphore_create_info);
		semaphore_create_info.Name = "RenderFinishedSemaphore";
		renderFinishedSemaphore.EmplaceBack(semaphore_create_info);

		Fence::CreateInfo fence_create_info;
		fence_create_info.RenderDevice = &renderDevice;
		fence_create_info.Name = "InFlightFence";
		fence_create_info.IsSignaled = true;
		graphicsFences.EmplaceBack(fence_create_info);
		fence_create_info.Name = "TransferFence";
		fence_create_info.IsSignaled = false;
		transferFences.EmplaceBack(fence_create_info);

		{
			GTSL::StaticString<64> command_pool_name("Transfer command pool. Frame: "); command_pool_name += i;
			
			CommandPool::CreateInfo command_pool_create_info;
			command_pool_create_info.RenderDevice = &renderDevice;
			command_pool_create_info.Name = command_pool_name.begin();
			command_pool_create_info.Queue = &graphicsQueue;

			graphicsCommandPools.EmplaceBack(command_pool_create_info);
			
			GTSL::StaticString<64> command_buffer_name("Graphics command buffer. Frame: "); command_buffer_name += i;

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.IsPrimary = true;
			allocate_command_buffers_info.RenderDevice = &renderDevice;

			CommandBuffer::CreateInfo command_buffer_create_info; command_buffer_create_info.RenderDevice = &renderDevice; command_buffer_create_info.Name = command_buffer_name.begin();

			GTSL::Array<CommandBuffer::CreateInfo, 5> create_infos; create_infos.EmplaceBack(command_buffer_create_info);
			allocate_command_buffers_info.CommandBufferCreateInfos = create_infos;
			graphicsCommandBuffers.Resize(graphicsCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, graphicsCommandBuffers.begin() + i);
			graphicsCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}

		{
			GTSL::StaticString<64> command_pool_name("Transfer command pool. Frame: "); command_pool_name += i;
			
			CommandPool::CreateInfo command_pool_create_info;
			command_pool_create_info.RenderDevice = &renderDevice;
			command_pool_create_info.Name = command_pool_name.begin();
			command_pool_create_info.Queue = &transferQueue;
			transferCommandPools.EmplaceBack(command_pool_create_info);
			
			GTSL::StaticString<64> command_buffer_name("Transfer command buffer. Frame: "); command_buffer_name += i;

			CommandPool::AllocateCommandBuffersInfo allocate_command_buffers_info;
			allocate_command_buffers_info.RenderDevice = &renderDevice;
			allocate_command_buffers_info.IsPrimary = true;

			CommandBuffer::CreateInfo command_buffer_create_info; command_buffer_create_info.RenderDevice = &renderDevice; command_buffer_create_info.Name = command_buffer_name.begin();
			
			GTSL::Array<CommandBuffer::CreateInfo, 5> create_infos; create_infos.EmplaceBack(command_buffer_create_info);
			allocate_command_buffers_info.CommandBufferCreateInfos = create_infos;
			transferCommandBuffers.Resize(transferCommandBuffers.GetLength() + 1);
			allocate_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, transferCommandBuffers.begin() + i);
			transferCommandPools[i].AllocateCommandBuffer(allocate_command_buffers_info);
		}
			
		FrameBuffer::CreateInfo framebuffer_create_info;
		framebuffer_create_info.RenderDevice = &renderDevice;
		framebuffer_create_info.RenderPass = &renderPass;
		framebuffer_create_info.Extent = window_extent;
		framebuffer_create_info.ImageViews = GTSL::Ranger<const ImageView>(1, &swapchainImages[i]);
		framebuffer_create_info.ClearValues = clearValues;

		frameBuffers.EmplaceBack(framebuffer_create_info);

		bufferCopyDatas.EmplaceBack(16, GetPersistentAllocator());
	}
	
	scratchMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Initialize(renderDevice, GetPersistentAllocator());

	PipelineCache::CreateInfo pipeline_cache_create_info;
	pipeline_cache_create_info.RenderDevice = &renderDevice;
	::new(&pipelineCache) PipelineCache(pipeline_cache_create_info);
	
	uint32 pipeline_cache_size = 0;
	pipelineCache.GetCacheSize(&renderDevice, pipeline_cache_size);

	if(pipeline_cache_size)
	{
		pipelineCacheBuffer.Allocate(pipeline_cache_size, 32, GetPersistentAllocator());
		pipelineCache.GetCache(&renderDevice, pipeline_cache_size, pipelineCacheBuffer);
	}
	
	BE_LOG_MESSAGE("Initialized successfully");
}

void RenderSystem::UpdateWindow(GTSL::Window& window)
{
	RenderContext::RecreateInfo recreate_info;
	recreate_info.RenderDevice = &renderDevice;
	recreate_info.DesiredFramesInFlight = swapchainImages.GetLength();
	recreate_info.PresentMode = swapchainPresentMode;
	recreate_info.ColorSpace = swapchainColorSpace;
	recreate_info.Format = swapchainFormat;
	window.GetFramebufferExtent(recreate_info.SurfaceArea);
	renderContext.Recreate(recreate_info);

	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }
	
	RenderContext::GetImagesInfo get_images_info;
	get_images_info.RenderDevice = &renderDevice;
	get_images_info.SwapchainImagesFormat = swapchainFormat;
	swapchainImages = renderContext.GetImages(get_images_info);
}

void RenderSystem::Initialize(const InitializeInfo& initializeInfo)
{
	const GTSL::Array<TaskDependency, 8> actsOn{ { "RenderSystem", AccessType::READ_WRITE } };
	initializeInfo.GameInstance->AddTask("frameStart",
		GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::frameStart>(this), actsOn, "FrameStart", "RenderStart");

	initializeInfo.GameInstance->AddTask("executeTransfers",
		GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::executeTransfers>(this), actsOn, "GameplayEnd", "RenderStart");

	initializeInfo.GameInstance->AddTask("render",
		GTSL::Delegate<void(TaskInfo)>::Create<RenderSystem, &RenderSystem::render>(this), actsOn, "RenderStart", "FrameEnd");
}

void RenderSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{	
	graphicsQueue.Wait();
	transferQueue.Wait();

	for (uint32 i = 0; i < swapchainImages.GetLength(); ++i)
	{
		CommandPool::FreeCommandBuffersInfo free_command_buffers_info;
		free_command_buffers_info.RenderDevice = &renderDevice;

		free_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, &graphicsCommandBuffers[i]);
		graphicsCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		free_command_buffers_info.CommandBuffers = GTSL::Ranger<CommandBuffer>(1, &transferCommandBuffers[i]);
		transferCommandPools[i].FreeCommandBuffers(free_command_buffers_info);

		graphicsCommandPools[i].Destroy(&renderDevice);
		transferCommandPools[i].Destroy(&renderDevice);
	}
	
	renderPass.Destroy(&renderDevice);
	renderContext.Destroy(&renderDevice);
	surface.Destroy(&renderDevice);

	for(auto& e : imageAvailableSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : renderFinishedSemaphore) { e.Destroy(&renderDevice); }
	for(auto& e : graphicsFences) { e.Destroy(&renderDevice); }
	for(auto& e : transferFences) { e.Destroy(&renderDevice); }

	for (auto& e : frameBuffers) { e.Destroy(&renderDevice); }
	for (auto& e : swapchainImages) { e.Destroy(&renderDevice); }

	scratchMemoryAllocator.Free(renderDevice, GetPersistentAllocator());
	localMemoryAllocator.Free(renderDevice, GetPersistentAllocator());

	uint32 cache_size = 0;
	pipelineCache.GetCacheSize(&renderDevice, cache_size);
	
	if(cache_size)
	{
		pipelineCacheBuffer.Allocate(cache_size, 32, GetPersistentAllocator());
		pipelineCache.GetCache(&renderDevice, cache_size, pipelineCacheBuffer);
	}
	else
	{
		pipelineCacheBuffer.Free(32, GetPersistentAllocator());
	}
}

void RenderSystem::render(TaskInfo taskInfo)
{
	Fence::WaitForFencesInfo waitForFencesInfo;
	waitForFencesInfo.RenderDevice = &renderDevice;
	waitForFencesInfo.Timeout = ~0ULL;
	waitForFencesInfo.WaitForAll = true;
	waitForFencesInfo.Fences = GTSL::Ranger<const Fence>(1, &graphicsFences[currentFrameIndex]);
	Fence::WaitForFences(waitForFencesInfo);
	
	Fence::ResetFencesInfo resetFencesInfo;
	resetFencesInfo.RenderDevice = &renderDevice;
	resetFencesInfo.Fences = GTSL::Ranger<const Fence>(1, &graphicsFences[currentFrameIndex]);
	Fence::ResetFences(resetFencesInfo);
	
	graphicsCommandPools[currentFrameIndex].ResetPool(&renderDevice);

	auto& commandBuffer = graphicsCommandBuffers[currentFrameIndex];
	
	auto positionMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetPositionMatrices();
	auto rotationMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetRotationMatrices();
	auto fovs = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetFieldOfViews();
	
	commandBuffer.BeginRecording({});
	commandBuffer.BeginRenderPass({&renderDevice, &renderPass, &frameBuffers[currentFrameIndex], renderArea, clearValues});;
	
	GTSL::Matrix4 projectionMatrix;
	GTSL::Math::BuildPerspectiveMatrix(projectionMatrix, fovs[0], 16.f / 9.f, 0.5f, 1000.f);
	//projection_matrix(1, 1) *= -1.f;

	auto pos = positionMatrices[0];

	pos(0, 3) *= -1;
	pos(1, 3) *= -1;
	//pos(2, 3) *= -1;
	
	auto viewMatrix = rotationMatrices[0] * pos;
	auto matrix = projectionMatrix * viewMatrix;
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto& renderGroups = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->GetRenderGroups();

	GTSL::Array<BindingsSet, 32> bindingsSets;

	bindingsSets.EmplaceBack(materialSystem->globalBindingsSets[GetCurrentFrame()]);
	
	CommandBuffer::BindBindingsSetInfo globalBind;
	globalBind.RenderDevice = GetRenderDevice();
	globalBind.FirstSet = 0;
	globalBind.BoundSets = 1;
	globalBind.BindingsSets = GTSL::Ranger<const BindingsSet>(1, &materialSystem->globalBindingsSets[GetCurrentFrame()]);
	globalBind.PipelineLayout = &materialSystem->globalPipelineLayout;
	globalBind.PipelineType = PipelineType::GRAPHICS;
	commandBuffer.BindBindingsSets(globalBind);
	
	GTSL::ForEach(renderGroups, [&](MaterialSystem::RenderGroupData& renderGroupData)
	{
		bindingsSets.EmplaceBack(renderGroupData.BindingsSets[GetCurrentFrame()]);
		
		CommandBuffer::BindBindingsSetInfo renderGroupBind;
		renderGroupBind.RenderDevice = GetRenderDevice();
		renderGroupBind.FirstSet = 1;
		renderGroupBind.BoundSets = 1;
		renderGroupBind.BindingsSets = GTSL::Ranger<const BindingsSet>(1, &renderGroupData.BindingsSets[GetCurrentFrame()]);
		renderGroupBind.PipelineLayout = &renderGroupData.PipelineLayout;
		renderGroupBind.Offsets = GTSL::Array<uint32, 1>{ renderDevice.GetMinUniformBufferOffset() * GetCurrentFrame() };
		renderGroupBind.PipelineType = PipelineType::GRAPHICS;
		commandBuffer.BindBindingsSets(renderGroupBind);

		const auto renderGroup = taskInfo.GameInstance->GetSystem<StaticMeshRenderGroup>(renderGroupData.RenderGroupName);

		auto positions = renderGroup->GetPositions();

		uint32 offset = GTSL::Math::RoundUpToPowerOf2Multiple(sizeof(GTSL::Matrix4), GetRenderDevice()->GetMinUniformBufferOffset()) * GetCurrentFrame();
		BE_ASSERT(GTSL::AlignPointer(GetRenderDevice()->GetMinUniformBufferOffset(), renderGroupData.Data) == renderGroupData.Data, "Oh!");
		const auto data_pointer = static_cast<byte*>(renderGroupData.Data) + offset;

		auto pos = GTSL::Math::Translation(positions[0]); pos(2, 3) *= -1.f;
		*reinterpret_cast<GTSL::Matrix4*>(data_pointer) = projectionMatrix * viewMatrix * pos;
		
		GTSL::ForEach(renderGroupData.Instances, [&](const MaterialSystem::MaterialInstance& materialInstance)
		{
			bindingsSets.EmplaceBack(materialInstance.BindingsSets[GetCurrentFrame()]);
			
			CommandBuffer::BindBindingsSetInfo materialBind;
			materialBind.RenderDevice = GetRenderDevice();
			materialBind.FirstSet = 2;
			materialBind.BoundSets = 1;
			materialBind.BindingsSets = GTSL::Ranger<const BindingsSet>(1, &materialInstance.BindingsSets[GetCurrentFrame()]);
			materialBind.PipelineLayout = &materialInstance.PipelineLayout;
			materialBind.Offsets = GTSL::Array<uint32, 1>{ renderDevice.GetMinUniformBufferOffset() * GetCurrentFrame() }; //CHECK
			materialBind.PipelineType = PipelineType::GRAPHICS;
			commandBuffer.BindBindingsSets(materialBind);
			
			CommandBuffer::BindPipelineInfo bindPipelineInfo;
			bindPipelineInfo.RenderDevice = GetRenderDevice();
			bindPipelineInfo.PipelineType = PipelineType::GRAPHICS;
			bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
			commandBuffer.BindPipeline(bindPipelineInfo);

			renderGroup->Render(taskInfo.GameInstance, this);
			
			bindingsSets.PopBack();
		}
		);

		bindingsSets.PopBack();
	}
	);
	
	commandBuffer.EndRenderPass({ &renderDevice });
	commandBuffer.EndRecording({});

	RenderContext::AcquireNextImageInfo acquireNextImageInfo;
	acquireNextImageInfo.RenderDevice = &renderDevice;
	acquireNextImageInfo.SignalSemaphore = &imageAvailableSemaphore[currentFrameIndex];
	auto imageIndex = renderContext.AcquireNextImage(acquireNextImageInfo);
	
	BE_ASSERT(imageIndex == currentFrameIndex, "Data mismatch");
	
	Queue::SubmitInfo submitInfo;
	submitInfo.RenderDevice = &renderDevice;
	submitInfo.Fence = &graphicsFences[currentFrameIndex];
	submitInfo.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &imageAvailableSemaphore[currentFrameIndex]);
	submitInfo.SignalSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[currentFrameIndex]);
	submitInfo.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, &commandBuffer);
	GTSL::Array<uint32, 8> wps{ (uint32)GAL::PipelineStage::COLOR_ATTACHMENT_OUTPUT };
	submitInfo.WaitPipelineStages = wps;
	graphicsQueue.Submit(submitInfo);
	
	RenderContext::PresentInfo presentInfo;
	presentInfo.RenderDevice = &renderDevice;
	presentInfo.Queue = &graphicsQueue;
	presentInfo.WaitSemaphores = GTSL::Ranger<const Semaphore>(1, &renderFinishedSemaphore[currentFrameIndex]);
	presentInfo.ImageIndex = imageIndex;
	renderContext.Present(presentInfo);

	currentFrameIndex = (currentFrameIndex + 1) % swapchainImages.GetLength();
}

void RenderSystem::frameStart(TaskInfo taskInfo)
{
	//Fence::WaitForFencesInfo wait_for_fences_info;
	//wait_for_fences_info.RenderDevice = &renderDevice;
	//wait_for_fences_info.Timeout = ~0ULL;
	//wait_for_fences_info.WaitForAll = true;
	//wait_for_fences_info.Fences = GTSL::Ranger<const Fence>(1, &transferFences[currentFrameIndex]);
	//Fence::WaitForFences(wait_for_fences_info);//

	auto& copyData = bufferCopyDatas[GetCurrentFrame()];
	
	if(transferFences[currentFrameIndex].GetStatus(&renderDevice))
	{
		for(uint32 i = 0; i < copyData.GetLength(); ++i)
		{
			copyData[i].SourceBuffer.Destroy(&renderDevice);
			DeallocateScratchBufferMemory(bufferCopyDatas[currentFrameIndex][i].Allocation);
		}
		
		copyData.ResizeDown(0);

		Fence::ResetFencesInfo reset_fences_info;
		reset_fences_info.RenderDevice = &renderDevice;
		reset_fences_info.Fences = GTSL::Ranger<const Fence>(1, &transferFences[currentFrameIndex]);
		Fence::ResetFences(reset_fences_info);
	}
	
	transferCommandPools[currentFrameIndex].ResetPool(&renderDevice); //should only be done if frame is finished transferring but must also implement check in execute transfers
	//or begin command buffer complains
}

void RenderSystem::executeTransfers(TaskInfo taskInfo)
{
	CommandBuffer::BeginRecordingInfo begin_recording_info;
	begin_recording_info.RenderDevice = &renderDevice;
	begin_recording_info.PrimaryCommandBuffer = &transferCommandBuffers[currentFrameIndex];
	transferCommandBuffers[currentFrameIndex].BeginRecording(begin_recording_info);
	
	for(auto& e : bufferCopyDatas[currentFrameIndex])
	{
		CommandBuffer::CopyBuffersInfo copy_buffers_info;
		copy_buffers_info.RenderDevice = &renderDevice;
		copy_buffers_info.Destination = &e.DestinationBuffer;
		copy_buffers_info.DestinationOffset = e.DestinationOffset;
		copy_buffers_info.Source = &e.SourceBuffer;
		copy_buffers_info.SourceOffset = e.SourceOffset;
		copy_buffers_info.Size = e.Size;
		GetTransferCommandBuffer()->CopyBuffers(copy_buffers_info);
	}

	CommandBuffer::EndRecordingInfo end_recording_info;
	end_recording_info.RenderDevice = &renderDevice;
	transferCommandBuffers[currentFrameIndex].EndRecording(end_recording_info);
	
	if (bufferCopyDatas[currentFrameIndex].GetLength())
	{
		Queue::SubmitInfo submit_info;
		submit_info.RenderDevice = &renderDevice;
		submit_info.Fence = &transferFences[currentFrameIndex];
		submit_info.CommandBuffers = GTSL::Ranger<const CommandBuffer>(1, &transferCommandBuffers[currentFrameIndex]);
		submit_info.WaitPipelineStages = GTSL::Array<uint32, 2>{ static_cast<uint32>(GAL::PipelineStage::TRANSFER) };
		transferQueue.Submit(submit_info);
	}
}

void RenderSystem::printError(const char* message, const RenderDevice::MessageSeverity messageSeverity) const
{
	switch (messageSeverity)
	{
	case RenderDevice::MessageSeverity::MESSAGE: /*BE_LOG_MESSAGE(message);*/ break;
	case RenderDevice::MessageSeverity::WARNING: BE_LOG_WARNING(message); break;
	case RenderDevice::MessageSeverity::ERROR:   BE_LOG_ERROR(message); break;
	default: break;
	}
}

void* RenderSystem::allocateApiMemory(void* data, const uint64 size, const uint64 alignment)
{
	void* allocation; uint64 allocated_size;
	GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);
	apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
	return allocation;
}

void* RenderSystem::reallocateApiMemory(void* data, void* oldAllocation, uint64 size, uint64 alignment)
{
	void* allocation; uint64 allocated_size;
	
	if(oldAllocation)
	{
		const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(oldAllocation));
		
		GetPersistentAllocator().Allocate(size, old_alloc.Second, &allocation, &allocated_size);
		apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
		
		GTSL::MemCopy(old_alloc.First, oldAllocation, allocation);
		
		GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
		apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
		return allocation;
	}

	if (size)
	{
		GetPersistentAllocator().Allocate(size, alignment, &allocation, &allocated_size);
		apiAllocations.Emplace(reinterpret_cast<uint64>(allocation), size, alignment);
		return allocation;
	}
	
	const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(oldAllocation));
	GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, oldAllocation);
	apiAllocations.Remove(reinterpret_cast<uint64>(oldAllocation));
	return nullptr;
}

void RenderSystem::deallocateApiMemory(void* data, void* allocation)
{
	if (data)
	{
		const auto old_alloc = apiAllocations.At(reinterpret_cast<uint64>(allocation));
		GetPersistentAllocator().Deallocate(old_alloc.First, old_alloc.Second, allocation);
		apiAllocations.Remove(reinterpret_cast<uint64>(allocation));
	}
}
