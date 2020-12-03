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
	members[0].Count = 1;

	GTSL::Array<MaterialSystem::StructInfo, 4> structs(1);
	structs[0].Frequency = MaterialSystem::Frequency::PER_INSTANCE;
	structs[0].Handle = &staticMeshDataStructHandle;
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

	auto range = renderGroup->GetAddedObjectsRangeAndReset();
	
	info.MaterialSystem->AddObjects(info.RenderSystem, dataSet, range.Second - range.First);
	
	{
		uint32 index = 0;

		for (auto& e : positions)
		{
			auto pos = GTSL::Math::Translation(e);
			pos(2, 3) *= -1.f;
			
			*info.MaterialSystem->GetMemberPointer<GTSL::Matrix4>(matrixUniformBufferMemberHandle, index) = info.ProjectionMatrix * info.ViewMatrix * pos;

			++index;
		}
	}
}

void UIRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
	
	auto mesh = renderSystem->CreateSharedMesh("BE_UI_SQUARE", 4 * 2 * 4, 6, 2);
	
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
	//uiMaterial = materialSystem->CreateMaterial(createMaterialInfo);
	
	renderSystem->AddMeshToId(square, uiMaterial.MaterialType);

	MaterialSystem::SetInfo setInfo;

	GTSL::Array<MaterialSystem::MemberInfo, 8> members(2);
	members[0].Type = MaterialSystem::Member::DataType::MATRIX4;
	members[0].Handle = &matrixUniformBufferMemberHandle;
	members[0].Count = 1;

	members[1].Type = MaterialSystem::Member::DataType::FVEC4;
	members[1].Handle = &colorHandle;
	members[1].Count = 1;

	GTSL::Array<MaterialSystem::StructInfo, 4> structs(1);
	structs[0].Frequency = MaterialSystem::Frequency::PER_INSTANCE;
	structs[0].Handle = &uiDataStructHandle;
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

	info.MaterialSystem->AddObjects(info.RenderSystem, dataSet, comps2 - comps);
	comps = comps2;
	
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
			
			*info.MaterialSystem->GetMemberPointer<GTSL::Matrix4>(matrixUniformBufferMemberHandle, sq) = trans * ortho;
			*reinterpret_cast<GTSL::RGBA*>(info.MaterialSystem->GetMemberPointer<GTSL::Vector4>(colorHandle, sq)) = uiSystem->GetColor(e.GetColor());
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

	renderPassesFunctions.Emplace(Id("SceneRenderPass"), RenderPassFunctionType::Create<RenderOrchestrator, &RenderOrchestrator::renderScene>());
	renderPassesFunctions.Emplace(Id("UIRenderPass"), RenderPassFunctionType::Create<RenderOrchestrator, &RenderOrchestrator::renderUI>());
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
		beginRegionInfo.Name = GTSL::StaticString<64>("Graphics");
		commandBuffer.BeginRegion(beginRegionInfo);
	}

	materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle("GlobalData"), 0);
	PipelineLayout pipelineLayout = materialSystem->GET_PIPELINE_LAYOUT(SetHandle("GlobalData"));
	
	CommandBuffer::EndRegionInfo endRegionInfo;
	endRegionInfo.RenderDevice = renderSystem->GetRenderDevice();

	uint8 arp = 0xFF, apiRpSp = 0;
	
	for(uint8 erp = 0; erp < enabledRenderPasses.GetLength(); ++erp)
	{
		auto& renderPass = renderPassesMap.At(enabledRenderPasses[erp]);

		if(arp == renderPass.APIRenderPass)
		{
			commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{});
			++apiRpSp;
		}
		else
		{
			arp = renderPass.APIRenderPass;
			auto apiRenderPass = apiRenderPasses[arp].RenderPass;
			auto frameBuffer = getFrameBuffer(arp);

			CommandBuffer::BeginRenderPassInfo beginRenderPass;
			beginRenderPass.RenderDevice = renderSystem->GetRenderDevice();
			beginRenderPass.RenderPass = &apiRenderPass;
			beginRenderPass.Framebuffer = &frameBuffer;
			beginRenderPass.RenderArea = renderSystem->GetRenderExtent();
			beginRenderPass.ClearValues = GetClearValues(arp);
			commandBuffer.BeginRenderPass(beginRenderPass);

			apiRpSp = 0;

			while(apiRpSp != renderPass.APISubPass) // Render passes and subpasses respectively HAVE to be in order
			{
				commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{});
				++apiRpSp;
			}
		}

		materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle(enabledRenderPasses[erp]), 0);

		uint32 pushConstant[] = { 0, 0, 0, 0 };

		auto ppLay = materialSystem->GET_PIPELINE_LAYOUT(SetHandle(enabledRenderPasses[erp]));

		CommandBuffer::UpdatePushConstantsInfo updatePush;
		updatePush.RenderDevice = renderSystem->GetRenderDevice();
		updatePush.Size = 16;
		updatePush.Offset = 0;
		updatePush.Data = reinterpret_cast<byte*>(pushConstant);
		updatePush.PipelineLayout = &ppLay;
		updatePush.ShaderStages = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
		commandBuffer.UpdatePushConstant(updatePush);
		
		renderPassesFunctions.At(enabledRenderPasses[erp])(this, taskInfo.GameInstance, renderSystem, materialSystem, pushConstant, commandBuffer, pipelineLayout, enabledRenderPasses[erp]);
	}

	if (subPasses[arp].GetLength() > apiRpSp)
	{
		for (uint32 sp = apiRpSp; sp < subPasses[arp].GetLength() - 1; ++sp)
		{
			commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{});
		}
	}

	//WHAT IF NO RENDER PASSES?
	CommandBuffer::EndRenderPassInfo endRenderPass;
	endRenderPass.RenderDevice = renderSystem->GetRenderDevice();
	commandBuffer.EndRenderPass(endRenderPass);
	
	commandBuffer.EndRegion(endRegionInfo);
	
	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Copy render target to Swapchain");
		commandBuffer.BeginRegion(beginRegionInfo);
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
			GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
			CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
			pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
			pipelineBarrierInfo.TextureBarriers = textureBarriers;
			pipelineBarrierInfo.InitialStage = PipelineStage::ALL_GRAPHICS; //wait for //TODO: FIND CORRECT PIPELINE STAGE
			pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER; //to allow this to run
			textureBarriers[0].Texture = GetAttachmentTexture("Color");
			textureBarriers[0].CurrentLayout = TextureLayout::TRANSFER_SRC;
			textureBarriers[0].TargetLayout = TextureLayout::TRANSFER_SRC;
			textureBarriers[0].SourceAccessFlags = AccessFlags::COLOR_ATTACHMENT_WRITE;
			textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_READ;
			commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
		}

		CommandBuffer::CopyTextureToTextureInfo copyTexture;
		copyTexture.RenderDevice = renderSystem->GetRenderDevice();
		copyTexture.SourceTexture = GetAttachmentTexture("Color");
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
	renderManagers.Emplace(renderManager, systemReference);
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

	if (type & (TextureType::DEPTH | TextureType::STENCIL))
	{
		attachment.ClearValue = GTSL::RGBA(1/*depth*/, 0/*stencil*/, 0, 0);
	}
	else
	{
		attachment.ClearValue = GTSL::RGBA(0, 0, 0, 0);
	}

	attachments.Emplace(name, attachment);
}

void RenderOrchestrator::AddPass(RenderSystem* renderSystem, GTSL::Range<const AttachmentInfo*> attachmentInfos, GTSL::Range<const PassData*> passesData)
{
	auto apiRenderPassName = Id("SceneRenderPass");
	
	apiRenderPassesMap.Emplace(apiRenderPassName, apiRenderPasses.GetLength());
	auto& apiRenderPassData = apiRenderPasses[apiRenderPasses.EmplaceBack()];
	
	apiRenderPassData.Name = "SceneRenderPass";

	RenderPass::CreateInfo renderPassCreateInfo;
	renderPassCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	if constexpr (_DEBUG) { renderPassCreateInfo.Name = GTSL::StaticString<32>("RenderPass"); }

	{
		GTSL::Array<RenderPass::AttachmentDescriptor, 16> attachmentDescriptors;

		for (auto e : attachmentInfos)
		{
			RenderPassAttachment renderPassAttachment;
			renderPassAttachment.Index = attachmentDescriptors.GetLength();
			apiRenderPassData.Attachments.Emplace(e.Name, renderPassAttachment);

			auto& attachment = attachments.At(e.Name);

			RenderPass::AttachmentDescriptor attachmentDescriptor;
			attachmentDescriptor.Format = attachment.Format;
			attachmentDescriptor.LoadOperation = e.Load;
			attachmentDescriptor.StoreOperation = e.Store;
			attachmentDescriptor.InitialLayout = e.StartState;
			attachmentDescriptor.FinalLayout = e.EndState;
			attachmentDescriptors.EmplaceBack(attachmentDescriptor);

			apiRenderPassData.ClearValues.EmplaceBack(attachment.ClearValue);
			
			apiRenderPassData.AttachmentNames.EmplaceBack(e.Name);
		}

		renderPassCreateInfo.RenderPassAttachments = attachmentDescriptors;
	}

	GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences(passesData.ElementCount());
	GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences(passesData.ElementCount());
	GTSL::Array<GTSL::Array<uint8, 8>, 8> preserveAttachmentReferences(passesData.ElementCount());

	subPasses.EmplaceBack();
	subPassMap.EmplaceBack();

	for (uint32 s = 0; s < passesData.ElementCount(); ++s)
	{
		apiRenderPassData.RenderPasses.EmplaceBack(static_cast<uint8>(renderPassesNames.EmplaceBack(passesData[s].Name)));

		auto& renderPass = renderPassesMap.Emplace(passesData[s].Name);
		renderPass.APIRenderPass = apiRenderPasses.GetLength() - 1;
		
		RenderPass::SubPassDescriptor subPassDescriptor;

		for (auto& e : passesData[s].ReadAttachments)
		{
			auto& renderpassAttachment = apiRenderPassData.Attachments.At(e.Name);

			renderpassAttachment.Layout = passesData[s].ReadAttachments[readAttachmentReferences[s].GetLength()].Layout;
			renderpassAttachment.Index = apiRenderPassData.Attachments.At(e.Name).Index;

			RenderPass::AttachmentReference attachmentReference;
			attachmentReference.Layout = renderpassAttachment.Layout;
			attachmentReference.Index = renderpassAttachment.Index;

			readAttachmentReferences[s].EmplaceBack(attachmentReference);
		}

		subPassDescriptor.ReadColorAttachments = readAttachmentReferences[s];

		for (auto e : passesData[s].WriteAttachments)
		{
			auto& renderpassAttachment = apiRenderPassData.Attachments.At(e.Name);

			renderpassAttachment.Layout = passesData[s].WriteAttachments[writeAttachmentReferences[s].GetLength()].Layout;
			renderpassAttachment.Index = apiRenderPassData.Attachments.At(e.Name).Index;

			RenderPass::AttachmentReference attachmentReference;
			attachmentReference.Layout = renderpassAttachment.Layout;
			attachmentReference.Index = renderpassAttachment.Index;

			writeAttachmentReferences[s].EmplaceBack(attachmentReference);
		}

		subPassDescriptor.WriteColorAttachments = writeAttachmentReferences[s];

		{
			auto isUsed = [&](Id name) -> bool //Determines if an attachment is read in any other later pass
			{ //TODO: CHECK IF ATTACHMENT IS READ AFTER IT'S LAST RESPECTIVE WRITE TO IT, ONLY THEN IT NEEDS TO BE PRESERVED. IF DONE LIKE THIS IT WILL BE PRESERVED NO MATTER IF IT IS READ LATER AFTER ANOTHER WRITE THAT OVERWRITES THE CURRENT ONE
				for (uint8 i = s + static_cast<uint8>(1); i < passesData.ElementCount(); ++i)
				{
					for (auto e : passesData[s].ReadAttachments) { if (e.Name == name) { return true; } }
					return false;
				}
			};

			for (uint32 a = 0; a < attachmentInfos.ElementCount(); ++a)
			{
				if (isUsed(attachmentInfos[a].Name)) { preserveAttachmentReferences[s].EmplaceBack(a); }
			}
		}

		subPassDescriptor.PreserveAttachments = preserveAttachmentReferences[s];

		if (passesData[s].DepthStencilAttachment.Name)
		{
			auto& attachmentInfo = apiRenderPassData.Attachments.At(passesData[s].DepthStencilAttachment.Name);

			subPassDescriptor.DepthAttachmentReference.Index = attachmentInfo.Index;
			subPassDescriptor.DepthAttachmentReference.Layout = passesData[s].DepthStencilAttachment.Layout;
		}
		else
		{
			subPassDescriptor.DepthAttachmentReference.Index = GAL::ATTACHMENT_UNUSED;
			subPassDescriptor.DepthAttachmentReference.Layout = TextureLayout::UNDEFINED;
		}

		subPassDescriptors.EmplaceBack(subPassDescriptor);

		renderPass.APISubPass = subPasses.back().GetLength();
		subPasses.back().EmplaceBack();
		auto& newSubPass = subPasses.back().back();
		newSubPass.Name = passesData[s].Name;
		subPassMap.back().Emplace(passesData[s].Name, s);
	}

	renderPassCreateInfo.SubPasses = subPassDescriptors;

	GTSL::Array<RenderPass::SubPassDependency, 16> subPassDependencies;
	
	{
		uint8 subPass = 0;

		for(uint8 i = 0; i < subPasses.back().GetLength() / 2; ++i)
		{
			RenderPass::SubPassDependency e;
			e.SourceSubPass = i;
			e.DestinationSubPass = i + 1;
		
			e.SourceAccessFlags = AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
			e.DestinationAccessFlags = 0;
		
			e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
			e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;

			subPassDependencies.EmplaceBack(e);
			
			//e.SourceSubPass = i + 1;
			//e.DestinationSubPass = i;
			//
			//e.SourceAccessFlags = AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
			//e.DestinationAccessFlags = 0;
			//
			//e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
			//e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;
			//
			//subPassDependencies.EmplaceBack(e);
		}
		
		//for (; subPass < passesData.ElementCount() - 1; ++subPass)
		//{
		//	auto& e = subPassDependencies[subPass];
		//	e.SourceSubPass = RenderPass::EXTERNAL;
		//	e.DestinationSubPass = RenderPass::EXTERNAL;
		//
		//	e.SourceAccessFlags = AccessFlags::INPUT_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_READ | AccessFlags::COLOR_ATTACHMENT_WRITE | AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ | AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;
		//	e.DestinationAccessFlags = 0;
		//
		//	e.SourcePipelineStage = PipelineStage::ALL_GRAPHICS;
		//	e.DestinationPipelineStage = PipelineStage::BOTTOM_OF_PIPE;
		//}
	}

	renderPassCreateInfo.SubPassDependencies = subPassDependencies;

	apiRenderPassData.RenderPass = RenderPass(renderPassCreateInfo);
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize)
{
	auto resize = [&](Attachment& attachment) -> void
	{
		//TODO: MAYBE HAVE A SETUP FUNCTION SO WE CAN GUARANTEE THERE'S AN ALLOCATION AND CAN JUST DEALLOCATE ALWAYS
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

	for (auto& renderPass : apiRenderPasses)
	{
		if (renderPass.FrameBuffer.GetVkFramebuffer())
		{
			renderPass.FrameBuffer.Destroy(renderSystem->GetRenderDevice());
		}

		FrameBuffer::CreateInfo framebufferCreateInfo;
		framebufferCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { framebufferCreateInfo.Name = GTSL::StaticString<32>("FrameBuffer"); }

		GTSL::Array<TextureView, 8> textureViews;

		for (auto e : renderPass.AttachmentNames) { textureViews.EmplaceBack(attachments.At(e).TextureView); }

		framebufferCreateInfo.TextureViews = textureViews;
		framebufferCreateInfo.RenderPass = &renderPass.RenderPass;
		framebufferCreateInfo.Extent = renderSystem->GetRenderExtent();

		renderPass.FrameBuffer = FrameBuffer(framebufferCreateInfo);
	}
}

void RenderOrchestrator::renderScene(GameInstance*, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 pushConstant[4], CommandBuffer commandBuffer, PipelineLayout pipelineLayout, Id rp)
{	
	for (auto e : renderPassesMap.At(rp).RenderGroups)
	{
		auto mats = materialSystem->GetMaterialHandles();

		materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle(e), 0);
		
		for (auto m : mats)
		{
			auto pipeline = materialSystem->GET_PIPELINE(m);

			CommandBuffer::BindPipelineInfo bindPipelineInfo;
			bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
			bindPipelineInfo.PipelineType = PipelineType::RASTER;
			bindPipelineInfo.Pipeline = &pipeline;
			commandBuffer.BindPipeline(bindPipelineInfo);
			
			materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle(m.MaterialType), 0);
			
			auto ppLay = materialSystem->GET_PIPELINE_LAYOUT(SetHandle(m.MaterialType));
				
			CommandBuffer::UpdatePushConstantsInfo updatePush;
			updatePush.RenderDevice = renderSystem->GetRenderDevice();
			updatePush.Size = 4;
			updatePush.Offset = 12;
			updatePush.Data = reinterpret_cast<byte*>(pushConstant);
			updatePush.PipelineLayout = &ppLay;
			updatePush.ShaderStages = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
			commandBuffer.UpdatePushConstant(updatePush);

			renderSystem->RenderAllMeshesForMaterial(m.MaterialType);

			++pushConstant[3];
		}
	}
}

void RenderOrchestrator::renderUI(GameInstance* gameInstance, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 pushConstant[4], CommandBuffer commandBuffer, PipelineLayout pipelineLayout, Id rp)
{
	auto* uiRenderManager = gameInstance->GetSystem<UIRenderManager>("UIRenderManager");

	materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle("UIRenderGroup"), 0);

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

		for (auto& e : squares)
		{
			auto mat = primitives.begin()[e.PrimitiveIndex].Material;
			auto pipeline = materialSystem->GET_PIPELINE(mat);

			if (pipeline.GetVkPipeline())
			{
				CommandBuffer::BindPipelineInfo bindPipelineInfo;
				bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
				bindPipelineInfo.PipelineType = PipelineType::RASTER;
				bindPipelineInfo.Pipeline = &pipeline;
				commandBuffer.BindPipeline(bindPipelineInfo);

				materialSystem->BIND_SET(renderSystem, commandBuffer, SetHandle(mat.MaterialType), 0);

				auto ppLay = materialSystem->GET_PIPELINE_LAYOUT(SetHandle(mat.MaterialType));

				CommandBuffer::UpdatePushConstantsInfo updatePush;
				updatePush.RenderDevice = renderSystem->GetRenderDevice();
				updatePush.Size = 4;
				updatePush.Offset = 12;
				updatePush.Data = reinterpret_cast<byte*>(&pushConstant[3]);
				updatePush.PipelineLayout = &ppLay;
				updatePush.ShaderStages = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
				commandBuffer.UpdatePushConstant(updatePush);

				renderSystem->RenderMesh(uiRenderManager->GetSquareMesh());
			}

			++pushConstant[3];
		}
	}
}
