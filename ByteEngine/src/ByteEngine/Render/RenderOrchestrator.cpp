#include "RenderOrchestrator.h"

#undef MemoryBarrier

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>

#include "RenderGroup.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/Tasks.h"

#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "UIManager.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/CameraSystem.h"

static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -0.5f, 0.5f }, { 0.5f, 0.5f }, { 0.5f, -0.5f }, { -0.5f, -0.5f } };
//static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -1.0f, 1.0f }, { 1.0f, 1.0f }, { 1.0f, -1.0f }, { -1.0f, -1.0f } };
static constexpr uint16 SQUARE_INDICES[] = { 0, 1, 3, 1, 2, 3 };

void StaticMeshRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");

	MaterialSystem::SetInfo setInfo;

	GTSL::Array<MaterialSystem::MemberInfo, 8> members(1);
	members[0].Type = MaterialSystem::Member::DataType::MATRIX4;
	members[0].Handle = &matrixUniformBufferMemberHandle;
	members[0].Count = 16;

	GTSL::Array<MaterialSystem::StructInfo, 4> structs(1);
	structs[0].Frequency = MaterialSystem::Frequency::PER_INSTANCE;
	structs[0].Members = members;

	setInfo.Structs = structs;
	
	dataSet = materialSystem->AddSet(renderSystem, "StaticMeshRenderGroup", "SceneRenderPass", setInfo);
	//TODO: MAKE A CORRECT PATH FOR DECLARING RENDER PASSES

	renderOrchestrator->AddToRenderPass("SceneRenderPass", "StaticMeshRenderGroup");
}

void StaticMeshRenderManager::GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ "StaticMeshRenderGroup", AccessType::READ });
}

void StaticMeshRenderManager::Setup(const SetupInfo& info)
{
	auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	auto positions = renderGroup->GetPositions();
	
	info.MaterialSystem->UpdateObjectCount(info.RenderSystem, matrixUniformBufferMemberHandle, renderGroup->GetStaticMeshes());
	
	{
		uint32 index = 0;

		for (auto& e : positions)
		{
			auto pos = GTSL::Math::Translation(e);
			pos(2, 3) *= -1.f;
			
			//*info.MaterialSystem->GetMemberPointer<GTSL::Matrix4>(matrixUniformBufferMemberHandle, index) = info.ProjectionMatrix * info.ViewMatrix * pos;

			++index;
		}
	}
}

void UIRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
	
	auto mesh = renderSystem->CreateSharedMesh("BE_UI_SQUARE", 4, 4 * 2, 6, 2);
	
	auto* meshPointer = renderSystem->GetSharedMeshPointer(mesh);
	GTSL::MemCopy(4 * 2 * 4, SQUARE_VERTICES, meshPointer);
	meshPointer += 4 * 2 * 4;
	GTSL::MemCopy(6 * 2, SQUARE_INDICES, meshPointer);
	
	square = renderSystem->CreateGPUMesh(mesh);
	
	//MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//createMaterialInfo.RenderSystem = renderSystem;
	//createMaterialInfo.GameInstance = initializeInfo.GameInstance;
	//createMaterialInfo.MaterialName = "UIMat";
	//createMaterialInfo.MaterialResourceManager = BE::Application::Get()->GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//createMaterialInfo.TextureResourceManager = BE::Application::Get()->GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//uiMaterial = materialSystem->CreateRasterMaterial(createMaterialInfo);
	
	renderSystem->AddMeshToId(square, uiMaterial.MaterialType);

	MaterialSystem::SetInfo setInfo;

	GTSL::Array<MaterialSystem::MemberInfo, 8> members(2);
	members[0].Type = MaterialSystem::Member::DataType::MATRIX4;
	members[0].Handle = &matrixUniformBufferMemberHandle;
	members[0].Count = 16;

	members[1].Type = MaterialSystem::Member::DataType::FVEC4;
	members[1].Handle = &colorHandle;
	members[1].Count = 16;

	GTSL::Array<MaterialSystem::StructInfo, 4> structs(1);
	structs[0].Frequency = MaterialSystem::Frequency::PER_INSTANCE;
	structs[0].Members = members;

	setInfo.Structs = structs;

	dataSet = materialSystem->AddSet(renderSystem, "UIRenderGroup", "UIRenderPass", setInfo);
	//TODO: MAKE A CORRECT PATH FOR DECLARING RENDER PASSES

	renderOrchestrator->AddToRenderPass("UIRenderPass", "UIRenderGroup");
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

	info.MaterialSystem->UpdateObjectCount(info.RenderSystem, matrixUniformBufferMemberHandle, comps);
	
	for (auto& ref : canvases)
	{
		auto& canvas = canvasSystem->GetCanvas(ref);
		auto canvasSize = canvas.GetExtent();

		float xyRatio = static_cast<float32>(canvasSize.Width) / static_cast<float32>(canvasSize.Height);
		float yxRatio = static_cast<float32>(canvasSize.Height) / static_cast<float32>(canvasSize.Width);
		
		GTSL::Matrix4 ortho(1.0f);
		GTSL::Math::MakeOrthoMatrix(ortho, 1.0f,
		                            -1.0f,
		                            yxRatio,
		                            -yxRatio, 0, 100);

		//GTSL::Math::MakeOrthoMatrix(ortho, canvasSize.Width, -canvasSize.Width, canvasSize.Height, -canvasSize.Height, 0, 100);
		
		//GTSL::Math::MakeOrthoMatrix(ortho, 0.5f, -0.5f, 0.5f, -0.5f, 1, 100);
		
		auto& organizers = canvas.GetOrganizersTree();

		auto primitives = canvas.GetPrimitives();
		auto squares = canvas.GetSquares();

		auto* parentOrganizer = organizers.GetRootNode();

		uint32 sq = 0;
		for(auto& e : squares)
		{
			GTSL::Matrix4 trans(1.0f);

			auto location = primitives.begin()[e.PrimitiveIndex].RelativeLocation;
			auto scale = primitives.begin()[e.PrimitiveIndex].AspectRatio;
			//
			GTSL::Math::Translate(trans, GTSL::Vector3(location.X, -location.Y, 0));
			GTSL::Math::Scale(trans, GTSL::Vector3(scale.X, scale.Y, 1));
			//GTSL::Math::Scale(trans, GTSL::Vector3(static_cast<float32>(canvasSize.Width), static_cast<float32>(canvasSize.Height), 1));
			//
			
			//*info.MaterialSystem->GetMemberPointer<GTSL::Matrix4>(matrixUniformBufferMemberHandle, sq) = trans * ortho;
			//*reinterpret_cast<GTSL::RGBA*>(info.MaterialSystem->GetMemberPointer<GTSL::Vector4>(colorHandle, sq)) = uiSystem->GetColor(e.GetColor());
			++sq;
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

	renderPassesMap.Initialize(8, GetPersistentAllocator());
	renderManagers.Initialize(16, GetPersistentAllocator());
	setupSystemsAccesses.Initialize(16, GetPersistentAllocator());

	renderPassesFunctions.Emplace(Id("SceneRenderPass")(), RenderPassFunctionType::Create<RenderOrchestrator, &RenderOrchestrator::renderScene>());
	renderPassesFunctions.Emplace(Id("UIRenderPass")(), RenderPassFunctionType::Create<RenderOrchestrator, &RenderOrchestrator::renderUI>());
	renderPassesFunctions.Emplace(Id("SceneRTRenderPass")(), RenderPassFunctionType::Create<RenderOrchestrator, &RenderOrchestrator::renderRays>());
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

	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Render");
		commandBuffer.BeginRegion(beginRegionInfo);
	}

	materialSystem->BindSet(renderSystem, commandBuffer, Id("GlobalData"));
	
	for (uint8 renderPassIndex = 0; renderPassIndex < renderPasses.GetLength();)
	{
		Id renderPassId;
		RenderPassData* renderPass;

		auto beginRenderPass = [&]()
		{
			if constexpr (_DEBUG)
			{
				CommandBuffer::BeginRegionInfo beginRegionInfo;
				beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
				GTSL::StaticString<64> name("Render Pass: "); name += renderPassId.GetString();
				beginRegionInfo.Name = name;
				commandBuffer.BeginRegion(beginRegionInfo);
			}

			switch(renderPass->PassType)
			{
				case PassType::RASTER: // Don't transition attachments as API render pass will handle transitions
					for (auto& e : renderPass->WriteAttachments) {
						updateImage(attachments.At(e.Name()), e.Layout, renderPass->PipelineStages, e.WriteAccess);
					}

					for (auto& e : renderPass->ReadAttachments) {
						updateImage(attachments.At(e.Name()), e.Layout, renderPass->PipelineStages, e.WriteAccess);
					}
					break;
				
				case PassType::COMPUTE:
				case PassType::RAY_TRACING:
					transitionImages(commandBuffer, renderSystem, renderPassId);
					break;
				
				default: break;
			}
		};

		auto canBeginRenderPass = [&]()
		{
			renderPassId = renderPasses[renderPassIndex];
			renderPass = &renderPassesMap[renderPassId()];
			++renderPassIndex;
			return renderPass->Enabled;
		};
		
		auto endRenderPass = [&]()
		{
			if constexpr (_DEBUG)
			{
				CommandBuffer::EndRegionInfo endRegionInfo;
				endRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
				commandBuffer.EndRegion(endRegionInfo);
			}
		};
		
		if (canBeginRenderPass())
		{
			beginRenderPass();

			auto doRender = [&]() { if (renderPassesFunctions.Find(renderPassId())) { renderPassesFunctions.At(renderPassId())(this, taskInfo.GameInstance, renderSystem, materialSystem, commandBuffer, renderPassId); } };
			
			switch (renderPass->PassType)
			{
			case PassType::RASTER:
			{
				CommandBuffer::BeginRenderPassInfo beginRenderPassInfo;
				beginRenderPassInfo.RenderDevice = renderSystem->GetRenderDevice();
				beginRenderPassInfo.RenderPass = apiRenderPasses[renderPass->APIRenderPass].RenderPass;
				beginRenderPassInfo.Framebuffer = getFrameBuffer(renderPass->APIRenderPass);
				beginRenderPassInfo.RenderArea = renderSystem->GetRenderExtent();
				GTSL::Array<GTSL::RGBA, 8> clearValues;
				for (uint8 i = 0; i < renderPass->WriteAttachments.GetLength(); ++i)
				{
					auto& attachment = attachments.At(renderPass->WriteAttachments[i].Name());
					clearValues.EmplaceBack(GTSL::RGBA(attachment.Type & TextureType::COLOR ? 0.0f : 1.0f, 0.0f, 0.0f, 0.0f));
				}
				beginRenderPassInfo.ClearValues = clearValues;
				commandBuffer.BeginRenderPass(beginRenderPassInfo);

				auto doRaster = [&]() {
					materialSystem->BindSet(renderSystem, commandBuffer, renderPassId);
					doRender();
				};

				doRaster();
					
				for (uint8 subPassIndex = 0; subPassIndex < subPasses[renderPass->APIRenderPass].GetLength() - 1; ++subPassIndex) {
					commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{});
					if (canBeginRenderPass()) { beginRenderPass(); doRaster(); endRenderPass(); }
				}

				CommandBuffer::EndRenderPassInfo endRenderPassInfo;
				endRenderPassInfo.RenderDevice = renderSystem->GetRenderDevice();
				commandBuffer.EndRenderPass(endRenderPassInfo);

				break;
			}

			case PassType::RAY_TRACING:
			{
				doRender();
				break;
			}
			}

			endRenderPass();
		}
	}

	{
		{
			GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
			CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
			pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
			pipelineBarrierInfo.TextureBarriers = textureBarriers;
			pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
			pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
			textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
			textureBarriers[0].CurrentLayout = TextureLayout::UNDEFINED;
			textureBarriers[0].TargetLayout = TextureLayout::TRANSFER_DST;
			textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
			textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
			commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
		}

		{
			auto& attachment = attachments.At(finalAttachment());
			
			GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
			CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
			pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
			pipelineBarrierInfo.TextureBarriers = textureBarriers;
			pipelineBarrierInfo.InitialStage = attachment.ConsumingStages;
			pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
			textureBarriers[0].Texture = attachment.Texture;
			textureBarriers[0].CurrentLayout = attachment.Layout;
			textureBarriers[0].TargetLayout = TextureLayout::TRANSFER_SRC;
			textureBarriers[0].SourceAccessFlags = accessFlagsFromStageAndAccessType(attachment.ConsumingStages, attachment.WriteAccess);
			textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_READ;
			commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);

			updateImage(attachment, TextureLayout::TRANSFER_SRC, PipelineStage::TRANSFER, false);
		}

		CommandBuffer::CopyTextureToTextureInfo copyTexture;
		copyTexture.RenderDevice = renderSystem->GetRenderDevice();
		copyTexture.SourceTexture = GetAttachmentTexture(finalAttachment);
		copyTexture.DestinationTexture = renderSystem->GetSwapchainTextures()[currentFrame];
		copyTexture.Extent = { renderSystem->GetRenderExtent().Width, renderSystem->GetRenderExtent().Height, 1 };
		copyTexture.SourceLayout = TextureLayout::TRANSFER_SRC;
		copyTexture.DestinationLayout = TextureLayout::TRANSFER_DST;
		commandBuffer.CopyTextureToTexture(copyTexture);

		{
			GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
			CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
			pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
			pipelineBarrierInfo.TextureBarriers = textureBarriers;
			pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
			pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
			textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
			textureBarriers[0].CurrentLayout = TextureLayout::TRANSFER_DST;
			textureBarriers[0].TargetLayout = TextureLayout::PRESENTATION;
			textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
			textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
			commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
		}
	}

	CommandBuffer::EndRegionInfo endRegionInfo;
	endRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
	commandBuffer.EndRegion(endRegionInfo);
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
	renderManagers.Emplace(renderManager(), systemReference);
}

void RenderOrchestrator::RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const uint16 systemReference)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element != systems.end())
	
	systems.Pop(element - systems.begin());
	
	setupSystemsAccesses.Pop(element - systems.begin());
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
	
	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
}

void RenderOrchestrator::AddAttachment(RenderSystem* renderSystem, Id name, TextureFormat format, TextureUses::value_type uses, TextureType::value_type type)
{
	Attachment attachment;
	attachment.Format = format;
	attachment.Name = name;
	attachment.Type = type;
	attachment.Uses = uses;	

	if (type & TextureType::COLOR)
	{
		attachment.Uses |= TextureUses::STORAGE;
	}
	
	attachment.Layout = TextureLayout::UNDEFINED;
	attachment.WriteAccess = false;
	attachment.ConsumingStages = PipelineStage::TOP_OF_PIPE;

	attachments.Emplace(name(), attachment);
}

void RenderOrchestrator::AddPass(RenderSystem* renderSystem, GTSL::Range<const PassData*> passesData)
{
	for (uint8 passIndex = 0; passIndex < passesData.ElementCount(); ++passIndex)
	{
		finalAttachment = passesData[passIndex].ResultAttachment;
		
		switch (passesData[passIndex].PassType)
		{
		case PassType::RASTER:
		{
			uint32 lastContiguousRasterPassIndex = passIndex;
			while (lastContiguousRasterPassIndex < passesData.ElementCount() && passesData[lastContiguousRasterPassIndex].PassType == PassType::RASTER)
			{
				++lastContiguousRasterPassIndex;
			}
			auto& apiRenderPassData = apiRenderPasses[apiRenderPasses.EmplaceBack()];

			RenderPass::CreateInfo renderPassCreateInfo;
			renderPassCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
			if constexpr (_DEBUG) {
				auto name = GTSL::StaticString<32>("RenderPass");
				renderPassCreateInfo.Name = name;
			}

			GTSL::Array<Id, 8> usedAttachments;

			auto addIfNotUsed = [&](const Id name) {
				if (name() == 0) { return; }
				for (auto e : usedAttachments) { if (e == name) { return; } }
				usedAttachments.EmplaceBack(name);
			};

			for (uint8 p = passIndex; p < lastContiguousRasterPassIndex; ++p) {
				for (auto& ra : passesData[p].ReadAttachments) { addIfNotUsed(ra.Name); }
				for (auto& wa : passesData[p].WriteAttachments) { addIfNotUsed(wa.Name); }
				addIfNotUsed(passesData[p].DepthStencilAttachment.Name);
			}

			apiRenderPassData.AttachmentNames = usedAttachments;

			GTSL::Array<RenderPass::AttachmentDescriptor, 16> attachmentDescriptors;

			for (auto e : usedAttachments)
			{
				auto& attachment = attachments.At(e());

				RenderPass::AttachmentDescriptor attachmentDescriptor;
				attachmentDescriptor.Format = attachment.Format;
				attachmentDescriptor.LoadOperation = GAL::RenderTargetLoadOperations::CLEAR;
				attachmentDescriptor.StoreOperation = passesData[passIndex].ResultAttachment == e ? GAL::RenderTargetStoreOperations::STORE : GAL::RenderTargetStoreOperations::UNDEFINED;
				attachmentDescriptor.InitialLayout = TextureLayout::UNDEFINED;
				attachmentDescriptor.FinalLayout = attachment.Type & TextureType::COLOR ? TextureLayout::COLOR_ATTACHMENT : TextureLayout::DEPTH_STENCIL_ATTACHMENT; //TODO: SELECT CORRECT END LAYOUT
				attachmentDescriptors.EmplaceBack(attachmentDescriptor);
			}

			renderPassCreateInfo.RenderPassAttachments = attachmentDescriptors;

			GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
			GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences(lastContiguousRasterPassIndex);
			GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences(lastContiguousRasterPassIndex);
			GTSL::Array<GTSL::Array<uint8, 8>, 8> preserveAttachmentReferences(lastContiguousRasterPassIndex);

			subPasses.EmplaceBack();
			subPassMap.EmplaceBack();

			for (uint32 s = passIndex; s < lastContiguousRasterPassIndex; ++s, ++passIndex)
			{
				auto& renderPass = renderPassesMap.Emplace(passesData[s].Name());
				renderPasses.EmplaceBack(passesData[s].Name);
				renderPass.APIRenderPass = apiRenderPasses.GetLength() - 1;

				renderPass.PassType = PassType::RASTER;
				renderPass.PipelineStages = PipelineStage::COLOR_ATTACHMENT_OUTPUT;

				RenderPass::SubPassDescriptor subPassDescriptor;

				auto getAttachmentIndex = [&](const Id name)
				{
					for (uint8 i = 0; i < usedAttachments.GetLength(); ++i) { if (name == usedAttachments[i]) { return i; } }
					return GAL::ATTACHMENT_UNUSED;
				};

				for (auto& e : passesData[s].ReadAttachments)
				{
					RenderPass::AttachmentReference attachmentReference;
					attachmentReference.Layout = TextureLayout::SHADER_READ_ONLY;
					attachmentReference.Index = getAttachmentIndex(e.Name);

					readAttachmentReferences[s].EmplaceBack(attachmentReference);

					renderPass.ReadAttachments.EmplaceBack(AttachmentData{ e.Name, TextureLayout::SHADER_READ_ONLY, false });
				}

				subPassDescriptor.ReadColorAttachments = readAttachmentReferences[s];

				for (auto e : passesData[s].WriteAttachments)
				{
					RenderPass::AttachmentReference attachmentReference;
					attachmentReference.Layout = TextureLayout::COLOR_ATTACHMENT;
					attachmentReference.Index = getAttachmentIndex(e.Name);

					writeAttachmentReferences[s].EmplaceBack(attachmentReference);

					renderPass.WriteAttachments.EmplaceBack(AttachmentData{ e.Name, TextureLayout::COLOR_ATTACHMENT, true });
				}

				subPassDescriptor.WriteColorAttachments = writeAttachmentReferences[s];

				//{ //TODO: FIX PRESERVE
				//	auto hasToBePreserved = [&](Id name) -> bool //Determines if an attachment is read in any other later pass
				//	{
				//		uint8 nextWrite = 0, nextRead = 0xFF;
				//
				//		for (uint8 i = s + static_cast<uint8>(1); i < lastContiguousRasterPassIndex; ++i) {
				//			for (auto e : passesData[s].WriteAttachments) { if (e.Name == name) { nextWrite = i; break; } }
				//		}
				//
				//		for (uint8 i = s + static_cast<uint8>(1); i < lastContiguousRasterPassIndex; ++i) {
				//			for (auto e : passesData[s].ReadAttachments) { if (e.Name == name) { nextRead = i; break; } }
				//		}
				//
				//		return nextRead < nextWrite;
				//	};
				//
				//	GTSL::Array<Id, 16> attachmentsNotUsedThisSubPass;
				//	
				//	for (uint32 a = passIndex; a < lastContiguousRasterPassIndex; ++a) {
				//		for(auto& e : passesData[a].WriteAttachments) {
				//			if()
				//		}
				//		if (hasToBePreserved(renderPass.WriteAttachments[a].Name)) { preserveAttachmentReferences[s].EmplaceBack(a); }
				//	}
				//	for (uint32 a = 0; a < renderPass.ReadAttachments.GetLength(); ++a) {
				//		if (hasToBePreserved(renderPass.ReadAttachments[a].Name)) { preserveAttachmentReferences[s].EmplaceBack(a); }
				//	}
				//}

				subPassDescriptor.PreserveAttachments = preserveAttachmentReferences[s];

				if (passesData[s].DepthStencilAttachment.Name())
				{
					subPassDescriptor.DepthAttachmentReference.Index = getAttachmentIndex(passesData[s].DepthStencilAttachment.Name);
					subPassDescriptor.DepthAttachmentReference.Layout = TextureLayout::DEPTH_STENCIL_ATTACHMENT;

					renderPass.WriteAttachments.EmplaceBack(passesData[s].DepthStencilAttachment.Name);
				}
				else
				{
					subPassDescriptor.DepthAttachmentReference.Index = GAL::ATTACHMENT_UNUSED;
					subPassDescriptor.DepthAttachmentReference.Layout = TextureLayout::UNDEFINED;
				}

				subPassDescriptors.EmplaceBack(subPassDescriptor);

				subPasses.back().EmplaceBack();
				auto& newSubPass = subPasses.back().back();
				newSubPass.Name = passesData[s].Name;
				subPassMap.back().Emplace(passesData[s].Name(), s);
			}

			--passIndex;

			renderPassCreateInfo.SubPasses = subPassDescriptors;

			GTSL::Array<RenderPass::SubPassDependency, 16> subPassDependencies;

			for (uint8 i = 0; i < subPasses.back().GetLength() / 2; ++i)
			{
				RenderPass::SubPassDependency e;
				e.SourceSubPass = i;
				e.DestinationSubPass = i + 1;
				e.SourceAccessFlags = AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
				e.DestinationAccessFlags = 0;

				e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
				e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;
				subPassDependencies.EmplaceBack(e);
			}

			renderPassCreateInfo.SubPassDependencies = subPassDependencies;

			apiRenderPassData.RenderPass = RenderPass(renderPassCreateInfo);

			break;
		}
		case PassType::COMPUTE:
		case PassType::RAY_TRACING:
		{
			renderPasses.EmplaceBack(passesData[passIndex].Name);
			auto& renderPass = renderPassesMap.Emplace(passesData[passIndex].Name());

			renderPass.PassType = PassType::RAY_TRACING;
			renderPass.PipelineStages = PipelineStage::RAY_TRACING_SHADER;

			for (auto& e : passesData[passIndex].WriteAttachments) {
				AttachmentData attachmentData;
				attachmentData.Name = e.Name;
				attachmentData.Layout = TextureLayout::GENERAL;
				attachmentData.WriteAccess = true;
				renderPass.WriteAttachments.EmplaceBack(attachmentData);
			}

			for (auto& e : passesData[passIndex].ReadAttachments) {
				AttachmentData attachmentData;
				attachmentData.Name = e.Name;
				attachmentData.Layout = TextureLayout::SHADER_READ_ONLY;
				attachmentData.WriteAccess = false;
				renderPass.ReadAttachments.EmplaceBack(attachmentData);
			}

			break;
		}
		}
	}
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize)
{	
	auto resize = [&](Attachment& attachment) -> void
	{
		if (attachment.Allocation.Size)
		{
			renderSystem->DeallocateLocalTextureMemory(attachment.Allocation);
			attachment.Texture.Destroy(renderSystem->GetRenderDevice());
			attachment.TextureView.Destroy(renderSystem->GetRenderDevice());
			attachment.TextureSampler.Destroy(renderSystem->GetRenderDevice());
		}

		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { textureCreateInfo.Name = GTSL::StaticString<32>(attachment.Name.GetString()); }
		textureCreateInfo.Extent = { newSize.Width, newSize.Height, 1 };
		textureCreateInfo.Dimensions = Dimensions::SQUARE;
		textureCreateInfo.Format = attachment.Format;
		textureCreateInfo.MipLevels = 1;
		textureCreateInfo.Uses = attachment.Uses;
		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;

		RenderSystem::AllocateLocalTextureMemoryInfo allocateLocalTextureMemoryInfo;
		allocateLocalTextureMemoryInfo.Texture = &attachment.Texture;
		allocateLocalTextureMemoryInfo.CreateInfo = &textureCreateInfo;
		allocateLocalTextureMemoryInfo.Allocation = &attachment.Allocation;
		renderSystem->AllocateLocalTextureMemory(allocateLocalTextureMemoryInfo);

		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { textureViewCreateInfo.Name = GTSL::StaticString<32>(attachment.Name.GetString()); }
		textureViewCreateInfo.Dimensions = Dimensions::SQUARE;
		textureViewCreateInfo.Format = attachment.Format;
		textureViewCreateInfo.MipLevels = 1;
		textureViewCreateInfo.Type = attachment.Type;
		textureViewCreateInfo.Texture = attachment.Texture;
		attachment.TextureView = TextureView(textureViewCreateInfo);

		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		textureSamplerCreateInfo.Anisotropy = 0;
		if constexpr (_DEBUG) { textureSamplerCreateInfo.Name = GTSL::StaticString<32>(attachment.Name.GetString()); }
		attachment.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	};

	renderSystem->Wait();

	GTSL::ForEach(attachments, resize);

	for (auto& apiRenderPassData : apiRenderPasses)
	{
		if (apiRenderPassData.FrameBuffer.GetHandle())
		{
			apiRenderPassData.FrameBuffer.Destroy(renderSystem->GetRenderDevice());
		}

		FrameBuffer::CreateInfo framebufferCreateInfo;
		framebufferCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { framebufferCreateInfo.Name = GTSL::StaticString<32>("FrameBuffer"); }

		GTSL::Array<TextureView, 8> textureViews;

		for (auto e : apiRenderPassData.AttachmentNames) { textureViews.EmplaceBack(attachments.At(e()).TextureView); }

		framebufferCreateInfo.TextureViews = textureViews;
		framebufferCreateInfo.RenderPass = &apiRenderPassData.RenderPass;
		framebufferCreateInfo.Extent = renderSystem->GetRenderExtent();

		apiRenderPassData.FrameBuffer = FrameBuffer(framebufferCreateInfo);
	}

	uint32 i = 0;
	
	auto updateDescriptors = [&](const Attachment& attachment) {
		if (attachment.Type & TextureType::COLOR)
		{
			materialSystem->WriteSetTexture(materialSystem->GetSetHandleByName("GlobalData"), i, attachment.Texture, attachment.TextureView, TextureSampler());
			++i;
		}
	};
	
	GTSL::ForEach(attachments, updateDescriptors);
}

void RenderOrchestrator::ToggleRenderPass(Id renderPassName, bool enable)
{
	auto& renderPass = renderPassesMap[renderPassName()];
	switch (renderPass.PassType)
	{
		case PassType::RASTER: break;
		case PassType::COMPUTE: break;
		case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetOption("rayTracing"); break; // Enable render pass only if function is enaled in settings
		default: break;
	}
	
	renderPass.Enabled = enable;
}

#define TRANSLATE_MASK(fromVal, toVal, toVar, store) GTSL::SetBitAs(GTSL::FindFirstSetBit(fromVal), toVar & (toVal), store)

AccessFlags::value_type RenderOrchestrator::accessFlagsFromStageAndAccessType(PipelineStage::value_type stage, bool writeAccess)
{
	AccessFlags::value_type accessFlags = 0; //TODO: SWITCH FLAGS BY ATTACHMENT TYPE. E.J: COLOR, DEPTH, etc
	accessFlags |= stage & PipelineStage::COLOR_ATTACHMENT_OUTPUT ? writeAccess ? AccessFlags::COLOR_ATTACHMENT_WRITE : AccessFlags::COLOR_ATTACHMENT_READ : 0;
	accessFlags |= stage & (PipelineStage::RAY_TRACING_SHADER | PipelineStage::COMPUTE_SHADER) ? writeAccess ? AccessFlags::SHADER_WRITE : AccessFlags::SHADER_READ : 0;
	accessFlags |= stage & PipelineStage::TRANSFER ? writeAccess ? AccessFlags::TRANSFER_WRITE : AccessFlags::TRANSFER_READ : 0;
	return accessFlags;
}

void RenderOrchestrator::renderScene(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp)
{	
	for (auto e : renderPassesMap.At(rp()).RenderGroups)
	{
		auto mats = materialSystem->GetMaterialHandlesForRenderGroup(e);

		materialSystem->BindSet(renderSystem, commandBuffer, e);
		
		uint32 meshIndex = 0;
		for (auto m : mats)
		{
			if (materialSystem->BindMaterial(renderSystem, commandBuffer, m))
			{
				materialSystem->BindSet(renderSystem, commandBuffer, e, meshIndex);
				renderSystem->RenderAllMeshesForMaterial(m.MaterialType);
			}
			
			++meshIndex;
		}

	}
}

void RenderOrchestrator::renderUI(GameInstance* gameInstance, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp)
{
	auto* uiRenderManager = gameInstance->GetSystem<UIRenderManager>("UIRenderManager");

	materialSystem->BindSet(renderSystem, commandBuffer, Id("UIRenderGroup"));

	auto* uiSystem = gameInstance->GetSystem<UIManager>("UIManager");
	auto* canvasSystem = gameInstance->GetSystem<CanvasSystem>("CanvasSystem");

	auto canvases = uiSystem->GetCanvases();

	for (auto& ref : canvases)
	{
		auto& canvas = canvasSystem->GetCanvas(ref);
		auto canvasSize = canvas.GetExtent();

		auto& organizers = canvas.GetOrganizersTree();

		auto primitives = canvas.GetPrimitives();
		auto squares = canvas.GetSquares();

		auto* parentOrganizer = organizers.GetRootNode();

		uint32 squareIndex = 0;
		for (auto& e : squares)
		{
			auto mat = primitives.begin()[e.PrimitiveIndex].Material;

			if (materialSystem->BindMaterial(renderSystem, commandBuffer, mat))
			{
				materialSystem->BindSet(renderSystem, commandBuffer, Id("UIRenderGroup"), squareIndex);

				renderSystem->RenderMesh(uiRenderManager->GetSquareMesh());
			}

			++squareIndex;
		}
	}
}

void RenderOrchestrator::renderRays(GameInstance* gameInstance, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp)
{
	auto* cameraSystem = gameInstance->GetSystem<CameraSystem>("CameraSystem");

	GTSL::Matrix4 projectionMatrix;
	GTSL::Math::BuildPerspectiveMatrix(projectionMatrix, cameraSystem->GetFieldOfViews()[0], 16.f / 9.f, 0.5f, 1000.f);

	auto positionMatrix = cameraSystem->GetPositionMatrices()[0];
	positionMatrix(0, 3) *= -1;
	positionMatrix(1, 3) *= -1;
	
	auto viewMatrix = cameraSystem->GetRotationMatrices()[0] * positionMatrix;
	
	auto matHandle = materialSystem->GetCameraMatricesHandle();

	MaterialSystem::BufferIterator bufferIterator;
	materialSystem->MoveIterator(bufferIterator, matHandle);
	
	*materialSystem->GetMemberPointer<GTSL::Matrix4>(bufferIterator) = viewMatrix;
	materialSystem->AdvanceIterator(bufferIterator, matHandle);
	*materialSystem->GetMemberPointer<GTSL::Matrix4>(bufferIterator) = projectionMatrix; //view
	materialSystem->AdvanceIterator(bufferIterator, matHandle);
	*materialSystem->GetMemberPointer<GTSL::Matrix4>(bufferIterator) = GTSL::Math::Inverse(viewMatrix); //inv proj
	materialSystem->AdvanceIterator(bufferIterator, matHandle);
	*materialSystem->GetMemberPointer<GTSL::Matrix4>(bufferIterator) = GTSL::Math::Inverse(projectionMatrix); //inv view
	
	materialSystem->TraceRays(renderSystem->GetRenderExtent(), &commandBuffer, renderSystem);
}

void RenderOrchestrator::transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, Id renderPassId)
{
	GTSL::Array<CommandBuffer::TextureBarrier, 16> textureBarriers;
	
	auto& renderPass = renderPassesMap.At(renderPassId());

	uint32 initialStage = 0;
	
	auto buildTextureBarrier = [&](const AttachmentData& attachmentData)
	{
		auto& attachment = attachments.At(attachmentData.Name());

		CommandBuffer::TextureBarrier textureBarrier;
		textureBarrier.Texture = attachment.Texture;
		textureBarrier.CurrentLayout = attachment.Layout;
		textureBarrier.TargetLayout = attachmentData.Layout;
		textureBarrier.SourceAccessFlags = accessFlagsFromStageAndAccessType(attachment.ConsumingStages, attachment.WriteAccess);
		textureBarrier.DestinationAccessFlags = accessFlagsFromStageAndAccessType(renderPass.PipelineStages, attachmentData.WriteAccess);
		textureBarriers.EmplaceBack(textureBarrier);

		initialStage |= attachment.ConsumingStages;
		
		updateImage(attachment, attachmentData.Layout, renderPass.PipelineStages, attachmentData.WriteAccess);
	};
	
	for (auto& e : renderPass.WriteAttachments) { buildTextureBarrier(e); }
	for (auto& e : renderPass.ReadAttachments) { buildTextureBarrier(e); }

	CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
	pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
	pipelineBarrierInfo.TextureBarriers = textureBarriers;
	pipelineBarrierInfo.InitialStage = initialStage;
	pipelineBarrierInfo.FinalStage = renderPass.PipelineStages;
	
	commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
}
