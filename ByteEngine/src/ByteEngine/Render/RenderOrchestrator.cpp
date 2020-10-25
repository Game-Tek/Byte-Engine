#include "RenderOrchestrator.h"

#undef MemoryBarrier

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>

#include <ByteEngine\Render\BindingsManager.hpp>
#include "RenderGroup.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/Tasks.h"


#include "FrameManager.h"
#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "TextSystem.h"
#include "UIManager.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/CameraSystem.h"

static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -1.f, 1.f }, { 1.f, 1.f }, { 1.f, -1.f }, { -1.f, -1.f } };
static constexpr uint16 SQUARE_INDICES[] = { 0, 1, 3, 1, 2, 3 };

void StaticMeshRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");

	MaterialSystem::SetInfo setInfo;

	GTSL::Array<MaterialSystem::Member, 8> members(1);
	members[0].Type = MaterialSystem::Member::DataType::MATRIX4;
	members[0].Handle = &matrixUniformBufferMemberHandle;

	GTSL::Array<MaterialSystem::Struct, 4> structs(1);
	structs[0].Frequency = MaterialSystem::Struct::Frequency::PER_INSTANCE;
	structs[0].Members = members;

	setInfo.Structs = structs;
	
	materialSystem->AddSet("StaticMeshSet", "where", setInfo);
}

void StaticMeshRenderManager::GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ "StaticMeshRenderGroup", AccessType::READ });
}

//GTSL::ForEach(renderGroup->GetMeshesByMaterial(),
//              [&](const GTSL::Vector<uint32, BE::PersistentAllocatorReference>& e)
//              {
//	              for (auto m : e)
//	              {
//		              const auto& i = meshes[m];
//
//		              if (renderInfo.MaterialSystem->IsMaterialReady(i.Material))
//		              {
//			              auto& materialInstances = renderInfo.MaterialSystem->GetMaterialInstances();
//			              auto& materialInstance = materialInstances[i.Material.MaterialInstance];
//
//			              {
//				              auto offset = GTSL::Array<uint32, 1>{
//					              ((renderGroup->GetMeshCount() * 64) * renderInfo.CurrentFrame) + (m * 64)
//				              };
//				              renderInfo.BindingsManager->AddBinding(
//					              renderGroupInstance.BindingsSets[renderInfo.CurrentFrame], offset,
//					              PipelineType::RASTER, renderGroupInstance.PipelineLayout);
//			              }
//
//			              if (materialInstance.TextureParametersBindings.DataSize)
//			              {
//				              renderInfo.BindingsManager->AddBinding(
//					              materialInstance.TextureParametersBindings.BindingsSets[renderInfo.
//						              CurrentFrame], PipelineType::RASTER, materialInstance.PipelineLayout);
//			              }
//
//			              CommandBuffer::BindPipelineInfo bindPipelineInfo;
//			              bindPipelineInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
//			              bindPipelineInfo.PipelineType = PipelineType::RASTER;
//			              bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
//			              renderInfo.CommandBuffer->BindPipeline(bindPipelineInfo);
//
//			              CommandBuffer::BindVertexBufferInfo bindVertexInfo;
//			              bindVertexInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
//			              bindVertexInfo.Buffer = &i.Buffer;
//			              bindVertexInfo.Offset = 0;
//			              renderInfo.CommandBuffer->BindVertexBuffer(bindVertexInfo);
//			              CommandBuffer::BindIndexBufferInfo bindIndexBuffer;
//			              bindIndexBuffer.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
//			              bindIndexBuffer.Buffer = &i.Buffer;
//			              bindIndexBuffer.Offset = i.IndicesOffset;
//			              bindIndexBuffer.IndexType = i.IndexType;
//			              renderInfo.CommandBuffer->BindIndexBuffer(bindIndexBuffer);
//			              CommandBuffer::DrawIndexedInfo drawIndexedInfo;
//			              drawIndexedInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
//			              drawIndexedInfo.InstanceCount = 1;
//			              drawIndexedInfo.IndexCount = i.IndicesCount;
//			              renderInfo.CommandBuffer->DrawIndexed(drawIndexedInfo);
//
//			              if (materialInstance.TextureParametersBindings.DataSize)
//			              {
//				              renderInfo.BindingsManager->PopBindings(); //material
//			              }
//
//			              renderInfo.BindingsManager->PopBindings(); //render group
//		              }
//	              }
//              }
//);

void StaticMeshRenderManager::Setup(const SetupInfo& info)
{
	auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	auto positions = renderGroup->GetPositions();

	{
		uint32 index = 0;

		for (auto& e : positions)
		{
			auto pos = GTSL::Math::Translation(e);
			pos(2, 3) *= -1.f;
			
			*reinterpret_cast<GTSL::Matrix4*>(info.MaterialSystem->GetMemberPointer(matrixUniformBufferMemberHandle, index)) = info.ProjectionMatrix * info.ViewMatrix * pos;

			++index;
		}
	}
}

void UIRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	square = renderSystem->CreateMesh((void*)&SQUARE_VERTICES, (void*)&SQUARE_INDICES, 4 * 2 * 4, 6, 2);
}

void UIRenderManager::GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ "UIManager", AccessType::READ });
	dependencies.EmplaceBack(TaskDependency{ "CanvasSystem", AccessType::READ });
}

void UIRenderManager::Setup(const SetupInfo& info)
{
	auto* uiSystem = info.GameInstance->GetSystem<UIManager>("UIManager");
	auto* canvasSystem = info.GameInstance->GetSystem<CanvasSystem>("CanvasSystem");

	float32 scale = 1.0f;

	auto canvases = uiSystem->GetCanvases();

	//auto* data = info.MaterialSystem->GetRenderGroupDataPointer("UIRenderGroup");
	
	for (auto& ref : canvases)
	{
		auto& canvas = canvasSystem->GetCanvas(ref);
		auto canvasSize = canvas.GetExtent();

		GTSL::Matrix4 ortho;
		GTSL::Math::MakeOrthoMatrix(ortho, static_cast<float32>(canvasSize.Width) * 0.5f,
		                            static_cast<float32>(canvasSize.Width) * -0.5f,
		                            static_cast<float32>(canvasSize.Height) * 0.5f,
		                            static_cast<float32>(canvasSize.Height) * -0.5f, 1, 100);

		auto& organizers = canvas.GetOrganizersTree();
		auto organizersAspectRatio = canvas.GetOrganizersAspectRatio();
		//auto organizersSquares = canvas.GetOrganizersSquares();
		//auto primitivesPerOrganizer = canvas.GetPrimitivesPerOrganizer();

		auto primitives = canvas.GetPrimitives();
		auto squares = canvas.GetSquares();

		uint32 offset = 0;
		
		auto* parentOrganizer = organizers.GetRootNode();

		for(auto& e : squares)
		{
			GTSL::Matrix4 trans;

			auto location = primitives.begin()[e.PrimitiveIndex].RelativeLocation;
			auto scale = primitives.begin()[e.PrimitiveIndex].AspectRatio;
			
			trans = GTSL::Math::Translation(GTSL::Vector3(location.X, location.Y, 0)) * ortho;
			trans *= GTSL::Math::Scaling(GTSL::Vector3(scale.X, scale.Y, 1));

			//*static_cast<GTSL::Matrix4*>(data) = trans;
			
			offset += 16 * 4;
		}
		
		//auto processNode = [&](decltype(parentOrganizer) node, uint32 depth, GTSL::Matrix4 parentTransform, auto&& self) -> void
		//{
		//	GTSL::Matrix4 transform;
		//
		//	for (uint32 i = 0; i < node->Nodes.GetLength(); ++i) { self(node->Nodes[i], depth + 1, transform, self); }
		//
		//	const auto aspectRatio = organizersAspectRatio.begin()[parentOrganizer->Data];
		//	GTSL::Matrix4 organizerMatrix = ortho;
		//	GTSL::Math::Scale(organizerMatrix, { aspectRatio.X, aspectRatio.Y, 1.0f });
		//
		//	for (auto square : organizersSquares.begin()[node->Data])
		//	{
		//		primitivesPerOrganizer->begin()[square.PrimitiveIndex].AspectRatio;
		//	}
		//};
		//
		//processNode(parentOrganizer, 0, ortho, processNode);
	}

	//if (textSystem->GetTexts().ElementCount())
	//{
	//	int32 atlasIndex = 0;
	//	
	//	auto& text = textSystem->GetTexts()[0];
	//	auto& imageFont = textSystem->GetFont();
	//
	//	auto x = text.Position.X;
	//	auto y = text.Position.Y;
	//	
	//	byte* data = static_cast<byte*>(info.MaterialSystem->GetRenderGroupDataPointer("TextSystem"));
	//
	//	uint32 offset = 0;
	//	
	//	GTSL::Matrix4 ortho;
	//	auto renderExtent = info.RenderSystem->GetRenderExtent();
	//	GTSL::Math::MakeOrthoMatrix(ortho, static_cast<float32>(renderExtent.Width) * 0.5f, static_cast<float32>(renderExtent.Width) * -0.5f, static_cast<float32>(renderExtent.Height) * 0.5f, static_cast<float32>(renderExtent.Height) * -0.5f, 1, 100);
	//	GTSL::MemCopy(sizeof(ortho), &ortho, data + offset); offset += sizeof(ortho);
	//	GTSL::MemCopy(sizeof(uint32), &atlasIndex, data + offset); offset += sizeof(uint32); offset += sizeof(uint32) * 3;
	//	
	//	for (auto* c = text.String.begin(); c != text.String.end() - 1; c++)
	//	{
	//		auto& ch = imageFont.Characters.at(*c);
	//
	//		float xpos = x + ch.Bearing.X * scale;
	//		float ypos = y - (ch.Size.Height - ch.Bearing.Y) * scale;
	//
	//		float w = ch.Size.Width * scale;
	//		float h = ch.Size.Height * scale;
	//		
	//		// update VBO for each character
	//		float vertices[6][4] = {
	//			{ xpos,     -(ypos + h),   0.0f, 0.0f },
	//			{ xpos,     -(ypos),       0.0f, 1.0f },
	//			{ xpos + w, -(ypos),       1.0f, 1.0f },
	//
	//			{ xpos,     -(ypos + h),   0.0f, 0.0f },
	//			{ xpos + w, -(ypos),       1.0f, 1.0f },
	//			{ xpos + w, -(ypos + h),   1.0f, 0.0f }
	//		};
	//		
	//		// now advance cursors for next glyph (note that advance is number of 1/64 pixels)
	//		x += (ch.Advance >> 6) * scale; // bitshift by 6 to get value in pixels (2^6 = 64)
	//
	//		uint32 val = ch.Position.Width;
	//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
	//		val = ch.Position.Height;
	//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
	//
	//		val = ch.Size.Width;
	//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
	//		val = ch.Size.Height;
	//		GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
	//		
	//		for (uint32 v = 0; v < 6; ++v)
	//		{
	//			GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][0], data + offset); offset += sizeof(GTSL::Vector2); //vertices
	//			GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][2], data + offset); offset += sizeof(GTSL::Vector2); //uv
	//		}
	//		
	//	}
	//
	//}
	////MaterialSystem::UpdateRenderGroupDataInfo updateInfo;
	////updateInfo.RenderGroup = "TextSystem";
	////updateInfo.Data = GTSL::Range<const byte>(64, static_cast<const byte*>(nullptr));
	////updateInfo.Offset = 0;
	////info.MaterialSystem->UpdateRenderGroupData(updateInfo);
}

void RenderOrchestrator::Initialize(const InitializeInfo& initializeInfo)
{
	systems.Initialize(32, GetPersistentAllocator());
	
	{
		const GTSL::Array<TaskDependency, 4> dependencies{ { CLASS_NAME, AccessType::READ_WRITE } };
		initializeInfo.GameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
		initializeInfo.GameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
	}

	renderManagers.Initialize(16, GetPersistentAllocator());
	setupSystemsAccesses.Initialize(16, GetPersistentAllocator());
	renderSystemsAccesses.Initialize(16, GetPersistentAllocator());
}

void RenderOrchestrator::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void RenderOrchestrator::Setup(TaskInfo taskInfo)
{
	auto positionMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetPositionMatrices();
	auto rotationMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetRotationMatrices();
	auto fovs = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetFieldOfViews();

	GTSL::Matrix4 projectionMatrix;
	GTSL::Math::BuildPerspectiveMatrix(projectionMatrix, fovs[0], 16.f / 9.f, 0.5f, 1000.f);

	auto cameraPosition = positionMatrices[0];

	cameraPosition(0, 3) *= -1;
	cameraPosition(1, 3) *= -1;

	auto viewMatrix = rotationMatrices[0] * cameraPosition;
	auto matrix = projectionMatrix * viewMatrix;
	
	RenderManager::SetupInfo setupInfo;
	setupInfo.GameInstance = taskInfo.GameInstance;
	setupInfo.RenderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	setupInfo.MaterialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	setupInfo.ProjectionMatrix = projectionMatrix;
	setupInfo.ViewMatrix = viewMatrix;
	GTSL::ForEach(renderManagers, [&](uint16 renderManager) { taskInfo.GameInstance->GetSystem<RenderManager>(renderManager)->Setup(setupInfo); });
}

void RenderOrchestrator::Render(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();
	uint8 currentFrame = renderSystem->GetCurrentFrame();
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	
	BindingsManager<BE::TAR> bindingsManager(GetTransientAllocator(), renderSystem, renderSystem->GetCurrentCommandBuffer());

	//TODO: AUTO ADD BINDINGS BY SCOPE

	auto* frameManager = taskInfo.GameInstance->GetSystem<FrameManager>("FrameManager");

	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Graphics");
		commandBuffer.BeginRegion(beginRegionInfo);
	}

	CommandBuffer::EndRegionInfo endRegionInfo;
	endRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
	
	for (uint8 rp = 0; rp < frameManager->GetRenderPassCount(); ++rp)
	{
		auto renderPass = frameManager->GetRenderPass(rp);
		auto frameBuffer = frameManager->GetFrameBuffer(rp);

		CommandBuffer::BeginRenderPassInfo beginRenderPass;
		beginRenderPass.RenderDevice = renderSystem->GetRenderDevice();
		beginRenderPass.RenderPass = &renderPass;
		beginRenderPass.Framebuffer = &frameBuffer;
		beginRenderPass.RenderArea = renderSystem->GetRenderExtent();
		beginRenderPass.ClearValues = frameManager->GetClearValues(rp);
		commandBuffer.BeginRenderPass(beginRenderPass);

		auto renderPassName = frameManager->GetRenderPassName(rp);
		
		for (uint8 sp = 0; sp < frameManager->GetSubPassCount(rp); ++sp)
		{
			auto subPassName = frameManager->GetSubPassName(rp, sp);
			
			if (sp < frameManager->GetSubPassCount(rp) - 1) { commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{}); }
		}

		CommandBuffer::EndRenderPassInfo endRenderPass;
		endRenderPass.RenderDevice = renderSystem->GetRenderDevice();
		commandBuffer.EndRenderPass(endRenderPass);
	}

	commandBuffer.EndRegion(endRegionInfo);

	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Copy render target to Swapchain");
		commandBuffer.BeginRegion(beginRegionInfo);
	}
	
	{
		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
		textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
		textureBarriers[0].CurrentLayout = TextureLayout::UNDEFINED;
		textureBarriers[0].TargetLayout = TextureLayout::TRANSFER_DST;
		textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
		textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
		pipelineBarrierInfo.TextureBarriers = textureBarriers;
		commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
	}

	CommandBuffer::CopyTextureToTextureInfo copyTexture;
	copyTexture.RenderDevice = renderSystem->GetRenderDevice();
	copyTexture.SourceTexture = frameManager->GetAttachmentTexture("Color");
	copyTexture.DestinationTexture = renderSystem->GetSwapchainTextures()[currentFrame];
	copyTexture.Extent = { renderSystem->GetRenderExtent().Width, renderSystem->GetRenderExtent().Height, 1 };
	copyTexture.SourceLayout = TextureLayout::TRANSFER_SRC;
	copyTexture.DestinationLayout = TextureLayout::TRANSFER_DST;
	commandBuffer.CopyTextureToTexture(copyTexture);

	{
		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
		textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
		textureBarriers[0].CurrentLayout = TextureLayout::TRANSFER_DST;
		textureBarriers[0].TargetLayout = TextureLayout::PRESENTATION;
		textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
		textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
		pipelineBarrierInfo.TextureBarriers = textureBarriers;
		commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
	}

	commandBuffer.EndRegion(endRegionInfo);
	
	bindingsManager.PopBindings();
}

void RenderOrchestrator::AddRenderManager(GameInstance* gameInstance, const Id renderManager, const uint16 systemReference)
{
	systems.EmplaceBack(renderManager);
	
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderDo");

	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	{
		for (uint32 i = 0; i < dependencies.GetLength(); ++i)
		{
			dependencies[i].AccessedObject = systems[i];
			dependencies[i].Access = AccessType::READ;
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessType::READ);
	dependencies.EmplaceBack("MaterialSystem", AccessType::READ);
	dependencies.EmplaceBack("FrameManager", AccessType::READ);

	{
		GTSL::Array<TaskDependency, 32> managerDependencies;

		managerDependencies.PushBack(dependencies);
		
		GTSL::Array<TaskDependency, 16> managerSetupDependencies;

		gameInstance->GetSystem<RenderManager>(systemReference)->GetSetupAccesses(managerSetupDependencies);

		managerDependencies.PushBack(managerSetupDependencies);
		
		setupSystemsAccesses.PushBack(managerDependencies);
	}
	

	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
	renderManagers.Emplace(renderManager, systemReference);
}

void RenderOrchestrator::RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const uint16 systemReference)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element != systems.end())
	
	systems.Pop(element - systems.begin());
	
	setupSystemsAccesses.Pop(element - systems.begin());
	renderSystemsAccesses.Pop(element - systems.begin());
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderDo");

	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	{
		for(uint32 i = 0; i < dependencies.GetLength(); ++i)
		{
			dependencies[i].AccessedObject = systems[i];
			dependencies[i].Access = AccessType::READ;
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessType::READ);
	dependencies.EmplaceBack("MaterialSystem", AccessType::READ);
	dependencies.EmplaceBack("FrameManager", AccessType::READ);
	
	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
}
