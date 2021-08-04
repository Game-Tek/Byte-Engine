#include "RenderOrchestrator.h"

#undef MemoryBarrier

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>
#include "LightsRenderGroup.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Game/Tasks.h"
#include "StaticMeshRenderGroup.h"
#include "UIManager.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/CameraSystem.h"

static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -0.5f, 0.5f }, { 0.5f, 0.5f }, { 0.5f, -0.5f }, { -0.5f, -0.5f } };
//static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -1.0f, 1.0f }, { 1.0f, 1.0f }, { 1.0f, -1.0f }, { -1.0f, -1.0f } };
static constexpr uint16 SQUARE_INDICES[] = { 0, 1, 3, 1, 2, 3 };

StaticMeshRenderManager::StaticMeshRenderManager(const InitializeInfo& initializeInfo) : RenderManager(initializeInfo, u8"StaticMeshRenderManager"), meshes(16, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	GTSL::StaticVector<RenderOrchestrator::MemberInfo, 8> members;
	members.EmplaceBack(&matrixUniformBufferMemberHandle, 1);
	members.EmplaceBack(&vertexBufferReferenceHandle, 1);
	members.EmplaceBack(&indexBufferReferenceHandle, 1);
	members.EmplaceBack(&materialInstance, 1);
	//members.EmplaceBack(4); //padding

	staticMeshInstanceDataStruct = renderOrchestrator->MakeMember(members);
}

void StaticMeshRenderManager::GetSetupAccesses(GTSL::StaticVector<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ u8"StaticMeshRenderGroup", AccessTypes::READ });
}

void StaticMeshRenderManager::Setup(const SetupInfo& info)
{
	auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup");
			
	for (auto e : renderGroup->GetAddedMeshes()) {
		auto materialHandle = renderGroup->GetMaterialHandle(e.StaticMeshHandle);
		
		if (e.Loaded) {
			auto& meshNode = meshes[renderGroup->GetMeshIndex(e.StaticMeshHandle)];
			
			auto key = info.RenderOrchestrator->GetBufferWriteKey(info.RenderSystem, meshNode.NodeHandle, staticMeshInstanceDataStruct);
			info.RenderOrchestrator->Write(info.RenderSystem, key, matrixUniformBufferMemberHandle, renderGroup->GetMeshTransform(e.StaticMeshHandle));
			info.RenderOrchestrator->Write(info.RenderSystem, key, vertexBufferReferenceHandle, info.RenderSystem->GetVertexBufferAddress(e.MeshHandle));
			info.RenderOrchestrator->Write(info.RenderSystem, key, indexBufferReferenceHandle, info.RenderSystem->GetIndexBufferAddress(e.MeshHandle));
			info.RenderOrchestrator->Write(info.RenderSystem, key, materialInstance, materialHandle.MaterialInstanceIndex);

			if (BE::Application::Get()->GetOption(u8"rayTracing")) {
				//info.RenderOrchestrator->AddMesh(info.RenderSystem, info.RenderOrchestrator->GetSceneReference(), e.MeshHandle, materialHandle, dataKey);
			}
			
			info.RenderOrchestrator->SetNodeState(meshNode.NodeHandle, true);
		} else {
			auto& mesh = meshes.EmplaceBack();
			
			auto materialLayer = info.RenderOrchestrator->AddMaterial(info.RenderOrchestrator->GetSceneRenderPass(), materialHandle);
			info.RenderOrchestrator->SetNodeState(materialLayer, false);

			auto meshNode = info.RenderOrchestrator->AddMesh(materialLayer, e.MeshHandle, info.RenderSystem->GetMeshVertexLayout(e.MeshHandle), staticMeshInstanceDataStruct);
			info.RenderOrchestrator->SetNodeState(meshNode, false);
			
			auto dataKey = info.RenderOrchestrator->AddData(staticMeshInstanceDataStruct);
			info.RenderOrchestrator->AddData(meshNode, dataKey);

			mesh.NodeHandle = meshNode;
			mesh.StaticMeshHandle = e.StaticMeshHandle;
		}
	}
	
	renderGroup->ClearAddedMeshes();

	{
		
		for(auto& e : renderGroup->GetDirtyMeshes()) {
			auto key = info.RenderOrchestrator->GetBufferWriteKey(info.RenderSystem, meshes[renderGroup->GetMeshIndex(e)].NodeHandle, staticMeshInstanceDataStruct);
			auto pos = renderGroup->GetMeshTransform(e);
	
			//info.MaterialSystem->UpdateIteratorMember(bufferIterator, staticMeshStruct, renderGroup->GetMeshIndex(e));
			info.RenderOrchestrator->Write(info.RenderSystem, key, matrixUniformBufferMemberHandle, pos);
	
	
			if (BE::Application::Get()->GetOption(u8"rayTracing")) {
				info.RenderSystem->SetMeshMatrix(renderGroup->GetMeshHandle(e), GTSL::Matrix3x4(pos));
			}
		}
	
		renderGroup->ClearDirtyMeshes();
	}
}

void UIRenderManager::GetSetupAccesses(GTSL::StaticVector<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ u8"UIManager", AccessTypes::READ });
	dependencies.EmplaceBack(TaskDependency{ u8"CanvasSystem", AccessTypes::READ });
}

void UIRenderManager::Setup(const SetupInfo& info)
{
	auto* uiSystem = info.GameInstance->GetSystem<UIManager>(u8"UIManager");
	auto* canvasSystem = info.GameInstance->GetSystem<CanvasSystem>(u8"CanvasSystem");

	float32 scale = 1.0f;
	
	//for (auto& ref : canvases)
	//{
	//	auto& canvas = canvasSystem->GetCanvas(ref);
	//	auto canvasSize = canvas.GetExtent();
	//
	//	float xyRatio = static_cast<float32>(canvasSize.Width) / static_cast<float32>(canvasSize.Height);
	//	float yxRatio = static_cast<float32>(canvasSize.Height) / static_cast<float32>(canvasSize.Width);
	//	
	//	GTSL::Matrix4 ortho = GTSL::Math::MakeOrthoMatrix(1.0f, -1.0f, yxRatio, -yxRatio, 0, 100);
	//	
	//	//GTSL::Math::MakeOrthoMatrix(ortho, canvasSize.Width, -canvasSize.Width, canvasSize.Height, -canvasSize.Height, 0, 100);
	//	//GTSL::Math::MakeOrthoMatrix(ortho, 0.5f, -0.5f, 0.5f, -0.5f, 1, 100);
	//	
	//	auto& organizers = canvas.GetOrganizersTree();
	//
	//	//auto primitives = canvas.GetPrimitives();
	//	//auto squares = canvas.GetSquares();
	//	//
	//	//const auto* parentOrganizer = organizers[0];
	//	//
	//	//uint32 sq = 0;
	//	//for(auto& e : squares)
	//	//{
	//	//	GTSL::Matrix4 trans(1.0f);
	//	//
	//	//	auto location = primitives.begin()[e.PrimitiveIndex].RelativeLocation;
	//	//	auto scale = primitives.begin()[e.PrimitiveIndex].AspectRatio;
	//	//	//
	//	//	GTSL::Math::AddTranslation(trans, GTSL::Vector3(location.X(), -location.Y(), 0));
	//	//	GTSL::Math::Scale(trans, GTSL::Vector3(scale.X(), scale.Y(), 1));
	//	//	//GTSL::Math::Scale(trans, GTSL::Vector3(static_cast<float32>(canvasSize.Width), static_cast<float32>(canvasSize.Height), 1));
	//	//	//
	//	//
	//	//	MaterialSystem::BufferIterator iterator;
	//	//	info.MaterialSystem->UpdateIteratorMember(iterator, uiDataStruct, sq);
	//	//	*info.MaterialSystem->GetMemberPointer(iterator, matrixUniformBufferMemberHandle) = trans * ortho;
	//	//	//*reinterpret_cast<GTSL::RGBA*>(info.MaterialSystem->GetMemberPointer<GTSL::Vector4>(colorHandle, sq)) = uiSystem->GetColor(e.GetColor());
	//	//	++sq;
	//	//}
	//	
	//	//auto processNode = [&](decltype(parentOrganizer) node, uint32 depth, GTSL::Matrix4 parentTransform, auto&& self) -> void
	//	//{
	//	//	GTSL::Matrix4 transform;
	//	//
	//	//	for (uint32 i = 0; i < node->Nodes.GetLength(); ++i) { self(node->Nodes[i], depth + 1, transform, self); }
	//	//
	//	//	const auto aspectRatio = organizersAspectRatio.begin()[parentOrganizer->Data];
	//	//	GTSL::Matrix4 organizerMatrix = ortho;
	//	//	GTSL::Math::Scale(organizerMatrix, { aspectRatio.X, aspectRatio.Y, 1.0f });
	//	//
	//	//	for (auto square : organizersSquares.begin()[node->Data])
	//	//	{
	//	//		primitivesPerOrganizer->begin()[square.PrimitiveIndex].AspectRatio;
	//	//	}
	//	//};
	//	//
	//	//processNode(parentOrganizer, 0, ortho, processNode);
	//}

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

RenderOrchestrator::RenderOrchestrator(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderOrchestrator"), nodesByName(64, GetPersistentAllocator()),
	systems(32, GetPersistentAllocator()), setupSystemsAccesses(16, GetPersistentAllocator()), renderManagers(16, GetPersistentAllocator()),
	renderingTree(128, GetPersistentAllocator()), internalRenderingTree(128, GetPersistentAllocator()), rayTracingPipelines(4, GetPersistentAllocator()),
	materials(16, GetPersistentAllocator()), materialsByName(16, GetPersistentAllocator()), texturesRefTable(16, GetPersistentAllocator()), latestLoadedTextures(8, GetPersistentAllocator()),
	pendingMaterialsPerTexture(4, GetPersistentAllocator()), buffers(16, GetPersistentAllocator()), sets(16, GetPersistentAllocator()),
	queuedSetUpdates(1, 8, GetPersistentAllocator()),
	setLayoutDatas(2, GetPersistentAllocator()), rasterMaterials(8, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");

	renderBuffers.EmplaceBack().BufferHandle = renderSystem->CreateBuffer(RENDER_DATA_BUFFER_PAGE_SIZE, GAL::BufferUses::STORAGE, true, false);

	for (uint32 i = 0; i < renderSystem->GetPipelinedFrames(); ++i) {
		descriptorsUpdates.EmplaceBack(GetPersistentAllocator());
	}

	// MATERIALS

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onTextureInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::onTextureInfoLoad", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onTextureInfoLoad>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onTextureLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::loadTexture", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onTextureLoad>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderOrchestrator", AccessTypes::READ } };
		onShaderInfosLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::onShaderGroupInfoLoad", Task<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, ShaderLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onShaderInfosLoaded>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onShaderGroupLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::onShaderGroupLoad", Task<ShaderResourceManager*, ShaderResourceManager::ShaderGroupInfo, GTSL::Range<byte*>, ShaderLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onShadersLoaded>(this), taskDependencies);
	}

	{
		GTSL::StaticVector<TaskDependency, 1> dependencies{ { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };

		auto renderEnableHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::OnRenderEnable", Task<bool>::Create<RenderOrchestrator, &RenderOrchestrator::OnRenderEnable>(this), dependencies);
		initializeInfo.GameInstance->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusGainEventHandle(), renderEnableHandle);

		auto renderDisableHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::OnRenderDisable", Task<bool>::Create<RenderOrchestrator, &RenderOrchestrator::OnRenderDisable>(this), dependencies);
		initializeInfo.GameInstance->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusLossEventHandle(), renderDisableHandle);
	}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onRenderEnable(initializeInfo.GameInstance, taskDependencies);
	}


	{
		GTSL::StaticVector<SubSetInfo, 10> subSetInfos;

		{ // TEXTURES
			SubSetInfo subSetInfo;
			subSetInfo.Type = SubSetType::READ_TEXTURES;
			subSetInfo.Count = 16;
			subSetInfo.Handle = &textureSubsetsHandle;
			subSetInfos.EmplaceBack(subSetInfo);
		}

		{ // IMAGES
			SubSetInfo subSetInfo;
			subSetInfo.Type = SubSetType::WRITE_TEXTURES;
			subSetInfo.Count = 16;
			subSetInfo.Handle = &imagesSubsetHandle;
			subSetInfos.EmplaceBack(subSetInfo);
		}

		if (BE::Application::Get()->GetOption(u8"rayTracing"))
		{
			{ //TOP LEVEL AS
				SubSetInfo subSetInfo;
				subSetInfo.Type = SubSetType::ACCELERATION_STRUCTURE;
				subSetInfo.Handle = &topLevelAsHandle;
				subSetInfo.Count = 1;
				subSetInfos.EmplaceBack(subSetInfo);
			}
		}

		globalSetLayout = AddSetLayout(renderSystem, SetLayoutHandle(), subSetInfos);
		globalBindingsSet = AddSet(renderSystem, u8"GlobalData", globalSetLayout, subSetInfos);
	}

	{
		GTSL::StaticVector<MemberInfo, 2> members;
		members.EmplaceBack(&globalDataHandle, 4);
		auto d = MakeMember(members);
		globalData = AddNode(u8"GlobalData", NodeHandle(), NodeType::LAYER);
		AddData(globalData, AddData(d));
	}

	{
		GTSL::StaticVector<MemberInfo, 2> members;
		members.EmplaceBack(&cameraMatricesHandle, 4);
		auto d = MakeMember(members);
		cameraDataNode = AddNode(u8"CameraData", globalData, NodeType::LAYER);
		AddData(cameraDataNode, AddData(d));
	}

	sceneRenderPass = AddNode(u8"SceneRenderPass", cameraDataNode, NodeType::RENDER_PASS);

	renderSystem->AllocateScratchBufferMemory(1024, GAL::BufferUses::VERTEX | GAL::BufferUses::INDEX, &buffer, &allocation);

	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[0] = GTSL::Vector3(0.5, 0.5, 0);
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[1] = GTSL::Vector3(0.0, 0.0, 1.0f);
	
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[2] = GTSL::Vector3(0.5, -0.5, 0);
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[3] = GTSL::Vector3(0.0, 0.0, 1.0f);
	
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[4] = GTSL::Vector3(-0.5, -0.5, 0);
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[5] = GTSL::Vector3(0.0, 0.0, 1.0f);
	
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[6] = GTSL::Vector3(-0.5, 0.5, 0);
	reinterpret_cast<GTSL::Vector3*>(allocation.Data)[7] = GTSL::Vector3(0.0, 0.0, 1.0f);
	
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[0] = 0;
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[1] = 2;
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[2] = 1;
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[3] = 0;
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[4] = 3;
	reinterpret_cast<uint16*>(reinterpret_cast<byte*>(allocation.Data) + 12 * 2 * 4)[5] = 1;
}

void RenderOrchestrator::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void RenderOrchestrator::Setup(TaskInfo taskInfo)
{
	//if (!renderingEnabled) { return; }
	
	auto fovs = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem")->GetFieldOfViews();

	GTSL::Matrix4 projectionMatrix;

	if(fovs.ElementCount())
		GTSL::Math::BuildPerspectiveMatrix(fovs[0], 16.f / 9.f, 0.01f, 1000.f);

	auto cameraTransform = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem")->GetCameraTransform();
	
	RenderManager::SetupInfo setupInfo;
	setupInfo.GameInstance = taskInfo.ApplicationManager;
	setupInfo.RenderSystem = taskInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	setupInfo.ProjectionMatrix = projectionMatrix;
	setupInfo.ViewMatrix = cameraTransform;
	setupInfo.RenderOrchestrator = this;
	GTSL::ForEach(renderManagers, [&](SystemHandle renderManager) { taskInfo.ApplicationManager->GetSystem<RenderManager>(renderManager)->Setup(setupInfo); });

	for (auto e : latestLoadedTextures) {
		for (auto b : pendingMaterialsPerTexture[e]) {
			auto& materialInstance = materials[b.MaterialIndex].MaterialInstances[b.MaterialInstanceIndex];
			if (++materialInstance.Counter == materialInstance.Target) {
				//setMaterialInstanceAsLoaded(b, materialInstance.Name);
			}
		}
	}

	latestLoadedTextures.Resize(0);
}

void RenderOrchestrator::Render(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	//renderSystem->SetHasRendered(renderingEnabled);
	//if (!renderingEnabled) { return; }
	auto renderArea = renderSystem->GetRenderExtent();
	const uint8 currentFrame = renderSystem->GetCurrentFrame(); auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();
	
	if (renderArea == 0) { return; }

	if (renderSystem->AcquireImage() || sizeHistory[currentFrame] != renderArea || sizeHistory[currentFrame] != sizeHistory[beforeFrame]) { OnResize(renderSystem, renderArea); }

	updateDescriptors(taskInfo);
	
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();

	BindSet(renderSystem, commandBuffer, globalBindingsSet, GAL::ShaderStages::VERTEX | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN);
	
	{
		auto* cameraSystem = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem");

		GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(cameraSystem->GetFieldOfViews()[0], 16.f / 9.f, 0.01f, 1000.f);
		projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;

		auto viewMatrix = cameraSystem->GetCameraTransform();

		auto key = GetBufferWriteKey(renderSystem, cameraDataNode, cameraMatricesHandle);
		Write(renderSystem, key, cameraMatricesHandle[0], viewMatrix);
		Write(renderSystem, key, cameraMatricesHandle[1], projectionMatrix);
		Write(renderSystem, key, cameraMatricesHandle[2], GTSL::Math::Inverse(viewMatrix));
		Write(renderSystem, key, cameraMatricesHandle[3], GTSL::Math::Inverse(projectionMatrix));
	}	
	
	RenderState renderState;
	InternalNodeHandle currentNode;

	if (internalRenderingTree.IsSlotOccupied(0)) { //todo: track start
		currentNode = InternalNodeHandle(0);
	}
	
	auto runLevel = [&](auto&& self, const InternalNode& data) -> void {
		DataStreamHandle dataStreamHandle = {};
		
		currentNode = data.Next;
		
		if (!data.Enabled) {
			for (uint16 i = 0; i < data.IndirectChildren; ++i) {
				currentNode = getNode(currentNode).Next;
			}
			
			return;
		}		
		
		commandBuffer.BeginRegion(renderSystem->GetRenderDevice(), data.Name);

		if (data.Offset != 0xFFFFFFFF) {
			dataStreamHandle = renderState.AddDataStream();
			auto& setLayout = setLayoutDatas[globalSetLayout()];
			GAL::DeviceAddress bufferAddress = renderSystem->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + data.Offset;
			commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), setLayout.PipelineLayout, dataStreamHandle() * 8, GTSL::Range<const byte*>(8, reinterpret_cast<const byte*>(&bufferAddress)), setLayout.Stage);
		}

		switch (data.Type) {
		case InternalNodeType::DISPATCH: {
			commandBuffer.Dispatch(renderSystem->GetRenderDevice(), data.Dispatch.DispatchSize);
			break;
		}
		case InternalNodeType::RAY_TRACE: {
			auto& pipelineData = rayTracingPipelines[data.RayTrace.PipelineIndex];

			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelineData.Pipeline, GAL::ShaderStages::RAY_GEN);

			GTSL::StaticVector<CommandList::ShaderTableDescriptor, 4> shaderTableDescriptors;

			for (uint8 i = 0; i < 4; ++i) {
				auto& shaderTableDescriptor = shaderTableDescriptors.EmplaceBack();
				shaderTableDescriptor.Entries = pipelineData.ShaderGroups[i].ShaderCount;
				shaderTableDescriptor.EntrySize = pipelineData.ShaderGroups[i].RoundedEntrySize;
				shaderTableDescriptor.Address = renderSystem->GetBufferDeviceAddress(pipelineData.ShaderGroups[i].Buffer);
			}

			commandBuffer.TraceRays(renderSystem->GetRenderDevice(), shaderTableDescriptors, sizeHistory[currentFrame]);
			break;
		}
		case InternalNodeType::MATERIAL: {
			break;
		}
		case InternalNodeType::MATERIAL_INSTANCE: {
			auto& material = rasterMaterials[data.Material.MaterialHandle.MaterialIndex];
			auto& materialInstance = material.Instances[data.Material.MaterialHandle.MaterialInstanceIndex];

			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), materialInstance, renderState.ShaderStages);
				
			break;
		}
		case InternalNodeType::MESH: {
			renderSystem->RenderMesh(data.Mesh.Handle, 1);
			break;
		}
		case InternalNodeType::RENDER_PASS: {
			switch (data.RenderPass.Type) {
			case PassType::RASTER: {
				for (const auto& e : data.RenderPass.Attachments) {
					updateImage(attachments.At(e.Name), e.Layout, data.RenderPass.PipelineStages, e.Access);
				}

				if (!renderState.MaxAPIPass) {
					renderState.ShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT;

					GTSL::StaticVector<GAL::RenderPassTargetDescription, 8> renderPassTargetDescriptions;
					for (uint8 i = 0; i < data.RenderPass.Attachments.GetLength(); ++i) {
						if (data.RenderPass.Attachments[i].Access & GAL::AccessTypes::WRITE) {
							auto& e = renderPassTargetDescriptions.EmplaceBack();
							const auto& attachment = attachments.At(data.RenderPass.Attachments[i].Name);
							e.ClearValue = attachment.ClearColor;
							e.Start = data.RenderPass.Attachments[i].Layout;
							e.End = data.RenderPass.Attachments[i].Layout;
							e.FormatDescriptor = attachment.FormatDescriptor;
							e.Texture = renderSystem->GetTexture(attachment.TextureHandle[currentFrame]);
						}
					}

					commandBuffer.BeginRenderPass(renderSystem->GetRenderDevice(), data.RenderPass.APIRenderPass.RenderPass, data.RenderPass.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()],
						renderArea, renderPassTargetDescriptions);

					renderState.MaxAPIPass = data.RenderPass.APIRenderPass.SubPassCount;
				}
				else {
					commandBuffer.AdvanceSubPass(renderSystem->GetRenderDevice());
					++renderState.APISubPass;
				}

				break;
			}
			case PassType::COMPUTE: {
				renderState.ShaderStages = GAL::ShaderStages::COMPUTE;
				transitionImages(commandBuffer, renderSystem, data);
				break;
			}
			case PassType::RAY_TRACING: {
				renderState.ShaderStages = GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::CALLABLE;
				transitionImages(commandBuffer, renderSystem, data);
				break;
			}
			}

			break;
		}
		case InternalNodeType::LAYER: {
			break;
		}
		}

		for (uint32 i = 0; i < data.DirectChildren; ++i) {
			self(self, internalRenderingTree[currentNode()]);
		}
		
		switch (data.Type) {
		case InternalNodeType::RENDER_PASS: {
			if (data.RenderPass.Type == PassType::RASTER && renderState.MaxAPIPass - 1 == renderState.APISubPass) {
				commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
				renderState.APISubPass = 0;
				renderState.MaxAPIPass = 0;
			}
		
			break;
		}
		default: break;
		}

		commandBuffer.EndRegion(renderSystem->GetRenderDevice());
	};

	
	while (currentNode) {
		runLevel(runLevel, internalRenderingTree[currentNode()]);
	}

	GTSL::StaticVector<CommandList::BarrierData, 2> swapchainToCopyBarriers;
	swapchainToCopyBarriers.EmplaceBack(CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::UNDEFINED,
		GAL::TextureLayout::TRANSFER_DESTINATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
		renderSystem->GetSwapchainFormat() });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), swapchainToCopyBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
		
	auto& attachment = attachments.At(resultAttachment);

	GTSL::StaticVector<CommandList::BarrierData, 2> fAttachmentoToCopyBarriers;
	fAttachmentoToCopyBarriers.EmplaceBack(CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout,
		GAL::TextureLayout::TRANSFER_SOURCE, attachment.AccessType,
		GAL::AccessTypes::READ, attachment.FormatDescriptor });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), fAttachmentoToCopyBarriers, attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, GetTransientAllocator());

	updateImage(attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);
		
	commandBuffer.CopyTextureToTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachments.At(resultAttachment).TextureHandle[currentFrame]),
	*renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_SOURCE, GAL::TextureLayout::TRANSFER_DESTINATION, 
		attachments.At(resultAttachment).FormatDescriptor, renderSystem->GetSwapchainFormat(),
		GTSL::Extent3D(renderSystem->GetRenderExtent()));

	
	GTSL::StaticVector<CommandList::BarrierData, 2> barriers;
	barriers.EmplaceBack(CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION,
		GAL::TextureLayout::PRESENTATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, renderSystem->GetSwapchainFormat() });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
}

//TODO: FIX ACCESS TO SYSTEMS HERE

void RenderOrchestrator::AddRenderManager(ApplicationManager* gameInstance, const Id renderManager, const SystemHandle systemReference)
{
	systems.EmplaceBack(renderManager);

	GTSL::StaticVector<TaskDependency, 32> dependencies;
	{
		for (uint32 i = 0; i < systems.GetLength(); ++i) {
			auto& dependency = dependencies.EmplaceBack();
			dependency.AccessedObject = systems[i];
			dependency.Access = AccessTypes::READ;
		}
	}

	dependencies.EmplaceBack(u8"RenderSystem", AccessTypes::READ);

	{
		GTSL::StaticVector<TaskDependency, 32> managerDependencies;

		managerDependencies.PushBack(dependencies);
		
		GTSL::StaticVector<TaskDependency, 16> managerSetupDependencies;

		gameInstance->GetSystem<RenderManager>(systemReference)->GetSetupAccesses(managerSetupDependencies);

		managerDependencies.PushBack(managerSetupDependencies);
		
		setupSystemsAccesses.EmplaceBack(managerDependencies);
	}

	if (renderingEnabled)
	{
		onRenderDisable(gameInstance);
		onRenderEnable(gameInstance, dependencies);
	}
	
	renderManagers.Emplace(renderManager, systemReference);
}

void RenderOrchestrator::RemoveRenderManager(ApplicationManager* gameInstance, const Id renderGroupName, const SystemHandle systemReference)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element.State())
	
	systems.Pop(element.Get());
	
	setupSystemsAccesses.Pop(element.Get());

	GTSL::StaticVector<TaskDependency, 32> dependencies;

	for (uint32 i = 0; i < systems.GetLength(); ++i)
	{
		auto& dependency = dependencies.EmplaceBack();
		dependency.AccessedObject = systems[i];
		dependency.Access = AccessTypes::READ;
	}

	dependencies.EmplaceBack(u8"RenderSystem", AccessTypes::READ);

	if (renderingEnabled)
	{
		onRenderDisable(gameInstance);
		onRenderEnable(gameInstance, dependencies);
	}
}

MaterialInstanceHandle RenderOrchestrator::CreateMaterial(const CreateMaterialInfo& info)
{
	auto materialReference = materialsByName.TryEmplace(info.MaterialName);

	uint32 materialIndex = 0xFFFFFFFF, materialInstanceIndex = 0xFFFFFFFF;
	
	if(materialReference.State())
	{
		materialIndex = materials.Emplace(GetPersistentAllocator());
		materialReference.Get() = materialIndex;
	
		const auto acts_on = GTSL::StaticVector<TaskDependency, 16>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };

		info.ShaderResourceManager->LoadShaderGroupInfo(info.GameInstance, info.MaterialName, onShaderInfosLoadHandle, ShaderLoadInfo(GetPersistentAllocator()));
		
		auto& material = materials[materialIndex];
		material.MaterialInstances.EmplaceBack();
		
		materialInstanceIndex = 0;
	} else {
		auto& material = materials[materialReference.Get()];
		materialIndex = materialReference.Get();
		auto index = material.MaterialInstances.LookFor([&](const MaterialInstance& materialInstance)
		{
			return materialInstance.Name == info.InstanceName;
		});
		
		//TODO: ERROR CHECK

		materialInstanceIndex = index.Get();
	}
	
	return { materialIndex, materialInstanceIndex };
}

void RenderOrchestrator::AddAttachment(Id name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type, GTSL::RGBA clearColor)
{
	Attachment attachment;
	attachment.Name = name;
	attachment.Uses = GAL::TextureUse();
	
	GAL::FormatDescriptor formatDescriptor;

	attachment.Uses |= GAL::TextureUses::ATTACHMENT;
	attachment.Uses |= GAL::TextureUses::SAMPLE;
	
	if (type == GAL::TextureType::COLOR) {		
		formatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::COLOR, 0, 1, 2, 3);
		attachment.Uses |= GAL::TextureUses::STORAGE;
		attachment.Uses |= GAL::TextureUses::TRANSFER_SOURCE;
	} else {
		formatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::DEPTH, 0, 0, 0, 0);
	}
	
	attachment.FormatDescriptor = formatDescriptor;
	attachment.ClearColor = clearColor;
	attachment.Layout = GAL::TextureLayout::UNDEFINED;
	attachment.AccessType = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;

	attachments.Emplace(name, attachment);
}

void RenderOrchestrator::AddPass(Id name, NodeHandle parent, RenderSystem* renderSystem, PassData passData)
{	
	uint32 currentPassIndex = renderPassesInOrder.GetLength();
	
	NodeHandle renderPassNode = AddNode(name, parent, NodeType::RENDER_PASS);
	InternalNode::RenderPassData& renderPass = getNode2(renderPassNode).RenderPass;
	auto t = getNode(renderPassNode).InternalSiblings.front().InternalNode;
	renderPasses.Emplace(name, t);
	renderPassesInOrder.EmplaceBack(t);
	getNode2(renderPassNode).Enabled = true;
	
	
	if(passData.WriteAttachments.GetLength())
		resultAttachment = passData.WriteAttachments[0].Name;

	GTSL::StaticMap<Id, uint32, 16> attachmentsRead;
	
	attachmentsRead.Emplace(resultAttachment, 0xFFFFFFFF); //set result attachment last read as "infinte" so it will always be stored

	for (uint32 i = renderPassesInOrder.GetLength() - 1; i < renderPassesInOrder.GetLength(); --i) {
		auto& rp = getNode(renderPassesInOrder[i]).RenderPass;
		for(auto& e : rp.Attachments) {
			if(e.Access & GAL::AccessTypes::READ) {
				if(!attachmentsRead.Find(e.Name)) {
					attachmentsRead.Emplace(e.Name, i);
				}
			}			
		}		
	}
	
	{
		auto& finalAttachment = attachments.At(resultAttachment);
		finalAttachment.FormatDescriptor = GAL::FORMATS::BGRA_I8;
	}
	
	switch (passData.PassType) {
	case PassType::RASTER: {	
		//uint32 contiguousRasterPassCount = passIndex;
		//while (contiguousRasterPassCount < passesData.ElementCount() && passesData[contiguousRasterPassCount].PassType == PassType::RASTER) {
		//	++contiguousRasterPassCount;
		//}
		//	
		//uint32 lastContiguousRasterPassIndex = contiguousRasterPassCount - 1;
			
		if constexpr (_DEBUG) {
			auto name = GTSL::StaticString<32>(u8"RenderPass");
			//renderPassCreateInfo.Name = name;
		}

		GTSL::StaticVector<Id, 16> renderPassUsedAttachments; GTSL::StaticVector<GTSL::StaticVector<Id, 16>, 16> usedAttachmentsPerSubPass;

		usedAttachmentsPerSubPass.EmplaceBack();
		
		for (auto& ra : passData.ReadAttachments) {
			if (!renderPassUsedAttachments.Find(ra.Name).State()) { renderPassUsedAttachments.EmplaceBack(ra.Name); }
			if (!usedAttachmentsPerSubPass[0].Find(ra.Name).State()) { usedAttachmentsPerSubPass[0].EmplaceBack(ra.Name); }
		}
		
		for (auto& wa : passData.WriteAttachments) {
			if (!renderPassUsedAttachments.Find(wa.Name).State()) { renderPassUsedAttachments.EmplaceBack(wa.Name); }
			if (!usedAttachmentsPerSubPass[0].Find(wa.Name).State()) { usedAttachmentsPerSubPass[0].EmplaceBack(wa.Name); }
		}

		GTSL::StaticVector<GAL::RenderPassTargetDescription, 16> attachmentDescriptors;

		for (auto e : renderPassUsedAttachments) {
			auto& attachment = attachments.At(e);

			auto& attachmentDescriptor = attachmentDescriptors.EmplaceBack();
			attachmentDescriptor.FormatDescriptor = attachment.FormatDescriptor;
			attachmentDescriptor.LoadOperation = GAL::Operations::CLEAR;
			attachmentDescriptor.StoreOperation = GAL::Operations::DO;
			//if(attachmentReadsPerPass[0].At(e) > lastContiguousRasterPassIndex) {
			//	attachmentDescriptor.StoreOperation = GAL::Operations::DO;
			//} else {
			//	attachmentDescriptor.StoreOperation = GAL::Operations::UNDEFINED;
			//}
			attachmentDescriptor.Start = GAL::TextureLayout::UNDEFINED;
			attachmentDescriptor.End = GAL::TextureLayout::ATTACHMENT; //TODO: SELECT CORRECT END LAYOUT
		}

		GTSL::StaticVector<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
		GTSL::StaticVector<GTSL::StaticVector<RenderPass::AttachmentReference, 16>, 8> attachmentReferences;
		GTSL::StaticVector<GTSL::StaticVector<uint8, 8>, 8> preserveAttachmentReferences;

		GAL::AccessType sourceAccessFlags = GAL::AccessTypes::READ, destinationAccessFlags = GAL::AccessTypes::READ;
		GAL::PipelineStage sourcePipelineStages = GAL::PipelineStages::TOP_OF_PIPE, destinationPipelineStages = GAL::PipelineStages::TOP_OF_PIPE;

		for (uint32 s = 0; s < 1/*contiguousRasterPassCount*/; ++s) {
			attachmentReferences.EmplaceBack();
			preserveAttachmentReferences.EmplaceBack();

			renderPass.Type = PassType::RASTER;
			renderPass.PipelineStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;

			RenderPass::SubPassDescriptor subPassDescriptor;

			auto getAttachmentIndex = [&](const Id name) {
				auto res = renderPassUsedAttachments.Find(name); return res.State() ? res.Get() : GAL::ATTACHMENT_UNUSED;
			};
			
			for (const auto& e : passData.ReadAttachments) {
				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Index = getAttachmentIndex(e.Name);
				attachmentReference.Layout = GAL::TextureLayout::SHADER_READ;
				attachmentReference.Access = GAL::AccessTypes::READ;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationAccessFlags = GAL::AccessTypes::READ;
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
					destinationAccessFlags = GAL::AccessTypes::READ;
					destinationPipelineStages |= GAL::PipelineStages::EARLY_FRAGMENT_TESTS | GAL::PipelineStages::LATE_FRAGMENT_TESTS;
				}
				
				auto& attachmentData = renderPass.Attachments.EmplaceBack();
				attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::SHADER_READ; attachmentData.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
				attachmentData.Access = GAL::AccessTypes::READ;
				attachmentReferences[s].EmplaceBack(attachmentReference);
			}

			for (const auto& e : passData.WriteAttachments) {
				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Layout = GAL::TextureLayout::ATTACHMENT;
				attachmentReference.Index = getAttachmentIndex(e.Name);
				attachmentReference.Access = GAL::AccessTypes::WRITE;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationAccessFlags = GAL::AccessTypes::WRITE;
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
					destinationAccessFlags = GAL::AccessTypes::WRITE;
					destinationPipelineStages |= GAL::PipelineStages::EARLY_FRAGMENT_TESTS | GAL::PipelineStages::LATE_FRAGMENT_TESTS;
				}

				auto& attachmentData = renderPass.Attachments.EmplaceBack();
				attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::ATTACHMENT; attachmentData.ConsumingStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				attachmentData.Access = GAL::AccessTypes::WRITE;
				attachmentReferences[s].EmplaceBack(attachmentReference);
			}

			subPassDescriptor.Attachments = attachmentReferences[s];

			for (auto b : renderPassUsedAttachments) {
				if (!usedAttachmentsPerSubPass[s].Find(b).State()) { // If attachment is not used this sub pass
					if (attachmentsRead.At(b) > currentPassIndex + s) { // And attachment is read after this pass
						preserveAttachmentReferences[s].EmplaceBack(getAttachmentIndex(b));
					}
				}
			}
			
			subPassDescriptor.PreserveAttachments = preserveAttachmentReferences[s];

			subPassDescriptors.EmplaceBack(subPassDescriptor);
			renderPass.APIRenderPass.APISubPass = 0;
			renderPass.APIRenderPass.SubPassCount++;

			sourceAccessFlags = destinationAccessFlags;
			sourcePipelineStages = destinationPipelineStages;
		}

		GTSL::StaticVector<RenderPass::SubPassDependency, 16> subPassDependencies;

		for (uint8 i = 0; i < subPassDescriptors.GetLength() / 2; ++i) {
			RenderPass::SubPassDependency e;
			e.SourcePipelineStage = sourcePipelineStages;
			e.DestinationPipelineStage = destinationPipelineStages;
			
			e.SourceSubPass = i;
			e.DestinationSubPass = i + 1;
			e.SourceAccessType = sourceAccessFlags;
			e.DestinationAccessType = destinationAccessFlags;

			subPassDependencies.EmplaceBack(e);
		}

		//for(uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f)
		renderPass.APIRenderPass.RenderPass.Initialize(renderSystem->GetRenderDevice(), attachmentDescriptors, subPassDescriptors, subPassDependencies);
		
		break;
	}
	case PassType::COMPUTE: {
		renderPass.Type = PassType::COMPUTE;
		renderPass.PipelineStages = GAL::PipelineStages::COMPUTE;

		for (auto& e : passData.WriteAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::GENERAL;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE;
		}

		for (auto& e : passData.ReadAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE;
		}

		break;
	}
	case PassType::RAY_TRACING: {
		renderPass.Type = PassType::RAY_TRACING;
		renderPass.PipelineStages = GAL::PipelineStages::RAY_TRACING;

		for (auto& e : passData.ReadAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			attachmentData.ConsumingStages = GAL::PipelineStages::RAY_TRACING;
		}
		
		for (auto& e : passData.WriteAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::GENERAL;
			attachmentData.ConsumingStages = GAL::PipelineStages::RAY_TRACING;
		}

		break;
	}
	}
		
	//GTSL::StaticVector<MaterialSystem::SubSetInfo, 8> subSets;
	//subSets.EmplaceBack(MaterialSystem::SubSetInfo{ MaterialSystem::SubSetType::READ_TEXTURES, &renderPass.ReadAttachmentsHandle, 16 });
	//subSets.EmplaceBack(MaterialSystem::SubSetInfo{ MaterialSystem::SubSetType::WRITE_TEXTURES, &renderPass.WriteAttachmentsHandle, 16 });
	//renderPass.AttachmentsSetHandle = materialSystem->AddSet(renderSystem, renderPasses[rp], "RenderPasses", subSets);
	
	GTSL::StaticVector<MemberInfo, 16> members;
	members.EmplaceBack(&renderPass.RenderTargetReferences, 16);
	
	AddData(renderPassNode, AddData(MakeMember(members)));

	for(auto bwk = GetBufferWriteKey(renderSystem, renderPassNode, renderPass.RenderTargetReferences); bwk < renderPass.Attachments.GetLength(); ++bwk) {
		Write(renderSystem, bwk, renderPass.RenderTargetReferences, attachments[renderPass.Attachments[bwk].Name].ImageIndex);
	}
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize)
{
	//pendingDeleteFrames = renderSystem->GetPipelinedFrames();

	auto currentFrame = renderSystem->GetCurrentFrame();
	auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();
	
	auto resize = [&](Attachment& attachment) -> void {
		if(attachment.TextureHandle[currentFrame]) {
			//destroy texture
			attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(attachment.FormatDescriptor, newSize, attachment.Uses, false);
		} else {
			attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(attachment.FormatDescriptor, newSize, attachment.Uses, false);
			attachment.ImageIndex = imageIndex++;
		}

		if(attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {
			WriteBinding(renderSystem, imagesSubsetHandle, attachment.TextureHandle[currentFrame], attachment.ImageIndex);
		}
	};

	auto resizeEqual = [&](Attachment& attachment) -> void {
		if(attachment.TextureHandle[currentFrame]) { /*destroy*/ }
		
		attachment.TextureHandle[currentFrame] = attachment.TextureHandle[beforeFrame];

		if(attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {
			WriteBinding(renderSystem, imagesSubsetHandle, attachment.TextureHandle[currentFrame], attachment.ImageIndex);
		}
	};

	if (sizeHistory[currentFrame] != newSize) {
		sizeHistory[currentFrame] = newSize;
		
		if(sizeHistory[currentFrame] == sizeHistory[beforeFrame]) {
			GTSL::ForEach(attachments, resizeEqual);
		} else {
			GTSL::ForEach(attachments, resize);
		}		
	}

	for (auto apiRenderPassData : renderPasses) {
		auto& layer = getNode(apiRenderPassData);

		if (layer.RenderPass.Type == PassType::RASTER) {
			if (layer.RenderPass.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].GetHandle())
				layer.RenderPass.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].Destroy(renderSystem->GetRenderDevice());

			GTSL::StaticVector<TextureView, 16> textureViews;
			for (auto e : layer.RenderPass.Attachments) {
				textureViews.EmplaceBack(renderSystem->GetTextureView(attachments.At(e.Name).TextureHandle[currentFrame]));
			}

			layer.RenderPass.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].Initialize(renderSystem->GetRenderDevice(), layer.RenderPass.APIRenderPass.RenderPass, newSize, textureViews);
		}
	}

	//for (auto rp : renderPasses) {
	//	auto& renderPass = rp->Data.RenderPass;
	//
	//	MaterialSystem::BufferIterator bufferIterator; uint8 attachmentIndex = 0;
	//	
	//	for (uint8 r = 0; r < renderPass.ReadAttachments.GetLength(); ++r) {
	//		auto& attachment = attachments.At(renderPass.ReadAttachments[r].Name);
	//		auto name = attachment.Name;
	//		materialSystem->WriteMultiBuffer(bufferIterator, renderPass.AttachmentsIndicesHandle, &attachment.ImageIndex, attachmentIndex++);
	//	}
	//	
	//	for (uint8 w = 0; w < renderPass.WriteAttachments.GetLength(); ++w) {
	//		auto& attachment = attachments.At(renderPass.WriteAttachments[w].Name);
	//		auto name = attachment.Name;
	//		if (attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {
	//			materialSystem->WriteMultiBuffer(bufferIterator, renderPass.AttachmentsIndicesHandle, &attachment.ImageIndex, attachmentIndex++);
	//		}
	//	}
	//}
}

void RenderOrchestrator::ToggleRenderPass(NodeHandle renderPassName, bool enable)
{
	if (renderPassName) {
		auto& renderPass = getNode(getNode(renderPassName).InternalSiblings.back().InternalNode);
		
		switch (renderPass.RenderPass.Type) {
		case PassType::RASTER: break;
		case PassType::COMPUTE: break;
		case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetOption(u8"rayTracing"); break; // Enable render pass only if function is enaled in settings
		default: break;
		}

		renderPass.Enabled = enable;
	} else {
		BE_LOG_WARNING("Tried to ", enable ? "enable" : "disable", " a render pass which does not exist.");
	}
}

void RenderOrchestrator::onRenderEnable(ApplicationManager* gameInstance, const GTSL::Range<const TaskDependency*> dependencies)
{
	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, u8"GameplayEnd", u8"RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, u8"RenderDo", u8"RenderFinished");
}

void RenderOrchestrator::onRenderDisable(ApplicationManager* gameInstance)
{
	gameInstance->RemoveTask(SETUP_TASK_NAME, u8"GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, u8"RenderDo");
}

void RenderOrchestrator::OnRenderEnable(TaskInfo taskInfo, bool oldFocus)
{
	//if (!oldFocus)
	//{
	//	GTSL::StaticVector<TaskDependency, 32> dependencies(systems.GetLength());
	//
	//	for (uint32 i = 0; i < dependencies.GetLength(); ++i)
	//	{
	//		dependencies[i].AccessedObject = systems[i];
	//		dependencies[i].Access = AccessTypes::READ;
	//	}
	//
	//	dependencies.EmplaceBack("RenderSystem", AccessTypes::READ);
	//
	//	onRenderEnable(taskInfo.ApplicationManager, dependencies);
	//	BE_LOG_SUCCESS("Enabled rendering")
	//}

	renderingEnabled = true;
}

void RenderOrchestrator::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	renderingEnabled = false;
}

void RenderOrchestrator::transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const InternalNode& renderPass)
{
	GTSL::StaticVector<CommandList::BarrierData, 16> barriers;

	GAL::PipelineStage initialStage;
	
	auto buildTextureBarrier = [&](const AttachmentData& attachmentData, GAL::PipelineStage attachmentStages, GAL::AccessType access) {
		auto& attachment = attachments.At(attachmentData.Name);

		CommandList::TextureBarrier textureBarrier;
		textureBarrier.Texture = renderSystem->GetTexture(attachment.TextureHandle[renderSystem->GetCurrentFrame()]);
		textureBarrier.CurrentLayout = attachment.Layout;
		textureBarrier.Format = attachment.FormatDescriptor;
		textureBarrier.TargetLayout = attachmentData.Layout;
		textureBarrier.SourceAccess = attachment.AccessType;
		textureBarrier.DestinationAccess = access;
		barriers.EmplaceBack(textureBarrier);

		initialStage |= attachment.ConsumingStages;
		
		updateImage(attachment, attachmentData.Layout, renderPass.RenderPass.PipelineStages, access);
	};
	
	for (auto& e : renderPass.RenderPass.Attachments) { buildTextureBarrier(e, e.ConsumingStages, e.Access); }
	
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, initialStage, renderPass.RenderPass.PipelineStages, GetTransientAllocator());
}

void RenderOrchestrator::onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager* materialResourceManager,
	ShaderResourceManager::ShaderGroupInfo shader_group_info, ShaderLoadInfo shaderLoadInfo)
{
	shaderLoadInfo.Buffer.Allocate(shader_group_info.Size, 8);

	materialResourceManager->LoadShaderGroup(taskInfo.ApplicationManager, shader_group_info, onShaderGroupLoadHandle, GTSL::Range<byte*>(shader_group_info.Size, shaderLoadInfo.Buffer.GetData()), GTSL::MoveRef(shaderLoadInfo));
}

void RenderOrchestrator::onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*,
                                         ShaderResourceManager::ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer,
                                         ShaderLoadInfo shaderLoadInfo)
{
	auto* renderSystem = taskInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	
	if (!shader_group_info.Valid)
		__debugbreak();

	GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock, 32> pipelineStates;

	auto materialIndex = shaderLoadInfo.Component;
	
	auto& materialData = materials[materialIndex];
	materialData.Name = Id(shader_group_info.Name);
	
	if(shader_group_info.Stages & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::MESH)) {
		auto& rasterMaterialData = rasterMaterials.EmplaceAt(materialIndex, GetPersistentAllocator());		

		getNode2(getNodeByName({ materialIndex, 0 })).Enabled = true;
		
		GTSL::StaticVector<Pipeline::ShaderInfo, 8> shaderInfos;
		GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> vertexElements;
		GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock::RenderContext::AttachmentState, 8> att;

		for (uint32 offset = 0; const auto& s : shader_group_info.Shaders) {
			auto& shaderInfo = shaderInfos.EmplaceBack();
			shaderInfo.Type = s.Type;
			shaderInfo.Blob = GTSL::Range<const byte*>(s.Size, shaderLoadInfo.Buffer.GetData() + offset);
			shaderInfo.Shader.Initialize(renderSystem->GetRenderDevice(), shaderInfo.Blob);

			offset += s.Size;

			switch (s.Type) {
			case GAL::ShaderType::VERTEX: {
				for (auto& e : s.VertexShader.VertexElements) {
					vertexElements.EmplaceBack(e);
				}
					
				break;
			}
			case GAL::ShaderType::TESSELLATION_CONTROL: break;
			case GAL::ShaderType::TESSELLATION_EVALUATION: break;
			case GAL::ShaderType::GEOMETRY: break;
			case GAL::ShaderType::FRAGMENT: break;
			case GAL::ShaderType::COMPUTE: break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::CALLABLE: break;
			default: ;
			}
		}
		
		GAL::Pipeline::PipelineStateBlock::RenderContext context;

		for (const auto& writeAttachment : getNode(renderPasses.At(Id(shader_group_info.RenderPass))).RenderPass.Attachments) {
			if (writeAttachment.Access & GAL::AccessTypes::WRITE) {
				auto& attachment = attachments.At(writeAttachment.Name);
				auto& attachmentState = att.EmplaceBack();
				attachmentState.BlendEnable = false; attachmentState.FormatDescriptor = attachment.FormatDescriptor;
			}
		}

		context.Attachments = att;
		context.RenderPass = static_cast<const GAL::RenderPass*>(getAPIRenderPass(Id(shader_group_info.RenderPass)));
		context.SubPassIndex = getAPISubPassIndex(Id(shader_group_info.RenderPass));
		pipelineStates.EmplaceBack(context);
		
		GAL::Pipeline::PipelineStateBlock::DepthState depth;
		depth.CompareOperation = GAL::CompareOperation::LESS;
		pipelineStates.EmplaceBack(depth);
		
		GAL::Pipeline::PipelineStateBlock::RasterState rasterState;
		rasterState.CullMode = GAL::CullMode::CULL_BACK;
		rasterState.WindingOrder = GAL::WindingOrder::CLOCKWISE;
		pipelineStates.EmplaceBack(rasterState);
		
		GAL::Pipeline::PipelineStateBlock::ViewportState viewportState;
		viewportState.ViewportCount = 1;
		pipelineStates.EmplaceBack(viewportState);
		
		auto& vertexState = pipelineStates.EmplaceBack(GAL::Pipeline::PipelineStateBlock::VertexState{});
		vertexState.Vertex.VertexDescriptor = vertexElements;

		for (uint8 materialInstanceIndex = 0; materialInstanceIndex < 1; ++materialInstanceIndex) {
			auto& rasterMaterialInstanceData = rasterMaterialData.Instances.EmplaceBack();
			auto& materialInstanceData = materialData.MaterialInstances[materialInstanceIndex];
			rasterMaterialInstanceData.InitializeRasterPipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}
	} else if (shader_group_info.Stages & (GAL::ShaderStages::COMPUTE)) {
		GTSL::StaticVector<Pipeline::ShaderInfo, 1> shaderInfos;
				
		for (uint32 offset = 0; auto& s : shader_group_info.Shaders) {
			auto& shaderInfo = shaderInfos.EmplaceBack();
			shaderInfo.Type = s.Type;
			shaderInfo.Shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range<const byte*>(s.Size, shaderLoadInfo.Buffer.GetData() + offset));;
			shaderInfo.Blob = GTSL::Range<const byte*>(s.Size, shaderLoadInfo.Buffer.GetData() + offset);

			offset += s.Size;
		}		
		
		Pipeline pipeline;
		pipeline.InitializeComputePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
	} else  if (shader_group_info.Stages & (GAL::ShaderStages::RAY_GEN)) {
		auto pipelineIndex = rayTracingPipelines.Emplace();
		auto& pipelineData = rayTracingPipelines[pipelineIndex];
		
		GTSL::Vector<Pipeline::RayTraceGroup, BE::TAR> groups(16, GetTransientAllocator());
		GTSL::Vector<Pipeline::ShaderInfo, BE::TAR> shaderInfos(16, GetTransientAllocator());
		
		auto handleSize = renderSystem->GetShaderGroupHandleSize();
		auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupHandleAlignment());

		GTSL::StaticVector<Pipeline::PipelineStateBlock, 4> pipelineStates;

		Pipeline::PipelineStateBlock::RayTracingState rtInfo;
		rtInfo.Groups = groups;
		rtInfo.MaxRecursionDepth = 0;
		
		uint32 offset = 0;
		
		for (uint32 i = 0; i < shader_group_info.Shaders.GetLength(); ++i) {
			auto& rayTracingShaderInfo = shader_group_info.Shaders[i];
			
			{
				auto& shader = shaderInfos.EmplaceBack();
				shader.Type = rayTracingShaderInfo.Type;
				shader.Blob = GTSL::Range<const byte*>(rayTracingShaderInfo.Size, buffer.begin() + offset);
				shader.Shader.Initialize(renderSystem->GetRenderDevice(), shader.Blob);

				offset += rayTracingShaderInfo.Size;
			}
		
			uint8 shaderGroup = 0xFF; Pipeline::RayTraceGroup group{};
			group.GeneralShader = Pipeline::RayTraceGroup::SHADER_UNUSED; group.ClosestHitShader = Pipeline::RayTraceGroup::SHADER_UNUSED;
			group.AnyHitShader = Pipeline::RayTraceGroup::SHADER_UNUSED; group.IntersectionShader = Pipeline::RayTraceGroup::SHADER_UNUSED;
		
			switch (rayTracingShaderInfo.Type) {
			case GAL::ShaderType::RAY_GEN: {
				group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
				shaderGroup = GAL::RAY_GEN_TABLE_INDEX;

				rtInfo.MaxRecursionDepth = GTSL::Math::Max(rayTracingShaderInfo.RayGenShader.Recursion, rtInfo.MaxRecursionDepth);
				break;
			}
			case GAL::ShaderType::MISS: {
				group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
				shaderGroup = GAL::MISS_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::CALLABLE: {
				group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
				shaderGroup = GAL::CALLABLE_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::CLOSEST_HIT: {
				group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::ANY_HIT: {
				group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.AnyHitShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::INTERSECTION: {
				group.ShaderGroup = GAL::ShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}		
			default: BE_LOG_MESSAGE("Non raytracing shader found in raytracing material");
			}
		
			groups.EmplaceBack(group);

			++pipelineData.ShaderGroups[shaderGroup].ShaderCount;

			rayTracingMaterials.Emplace((uint64)0/*todo: implement*/ << 32 | materialIndex, RTMI{pipelineIndex, shaderGroup, 0/*todo: implement*/});
		}

		{
			auto renderPassLayerHandle = AddNode(Id(shader_group_info.RenderPass), cameraDataNode, NodeType::RENDER_PASS);
			SetNodeState(renderPassLayerHandle, true);

			rayTraceNode = AddNode(Id(shader_group_info.Name), renderPassLayerHandle, NodeType::RAY_TRACE);
			getNode2(rayTraceNode).RayTrace.PipelineIndex = pipelineIndex;
		}

		pipelineStates.EmplaceBack(rtInfo);
		
		pipelineData.Pipeline.InitializeRayTracePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());

		GTSL::Vector<GAL::ShaderHandle, BE::TAR> shaderGroupHandlesBuffer(shaderInfos.GetLength(), GetTransientAllocator());
		
		pipelineData.Pipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, groups.GetLength(), shaderGroupHandlesBuffer);

		//create buffer per shader group
		for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
			auto& groupData = pipelineData.ShaderGroups[shaderGroupIndex];

			GTSL::StaticVector<MemberInfo, 2> membersA{ { &groupData.MaterialDataHandle, 1 }, { &groupData.ObjectDataHandle, 1 } };
			GTSL::StaticVector<MemberInfo, 2> members{ { &groupData.ShaderHandle, 1 }, { &groupData.ShaderEntryMemberHandle, 16, membersA } };			
			GTSL::StaticVector<MemberInfo, 2> elements{ MemberInfo(&groupData.ShaderGroupDataHandle, groupData.ShaderCount, members, renderSystem->GetShaderGroupBaseAlignment()) };
			
			groupData.Buffer = CreateBuffer(renderSystem, elements);			
			//materialData.DataKey = AddData()
			
			for (auto bWK = GetBufferWriteKey(renderSystem, groupData.Buffer, groupData.ShaderEntryMemberHandle); bWK < groupData.ShaderCount; ++shaderCount, ++bWK) {
				Write(renderSystem, bWK, groupData.ShaderHandle, shaderGroupHandlesBuffer[shaderCount]);
			}
		}		
		
		for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
			WriteBinding(topLevelAsHandle, 0, renderSystem->GetTopLevelAccelerationStructure(f), f);
		}
		
		for (auto& s : shaderInfos) { s.Shader.Destroy(renderSystem->GetRenderDevice()); }
	} else {
		__debugbreak();
	}
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo)
{
	auto component = textureIndex++;

	pendingMaterialsPerTexture.EmplaceAt(component, GetPersistentAllocator());
	//pendingMaterialsPerTexture[component].Initialize(4, GetPersistentAllocator());

	texturesRefTable.Emplace(createTextureInfo.TextureName, component);

	auto textureLoadInfo = TextureLoadInfo(component, createTextureInfo.RenderSystem, RenderAllocation());

	createTextureInfo.TextureResourceManager->LoadTextureInfo(createTextureInfo.GameInstance, createTextureInfo.TextureName, onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));

	return component;
}

void RenderOrchestrator::onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	loadInfo.TextureHandle = loadInfo.RenderSystem->CreateTexture(textureInfo.Format, textureInfo.Extent, GAL::TextureUses::SAMPLE | GAL::TextureUses::ATTACHMENT, true);

	auto dataBuffer = loadInfo.RenderSystem->GetTextureRange(loadInfo.TextureHandle);

	resourceManager->LoadTexture(taskInfo.ApplicationManager, textureInfo, dataBuffer, onTextureLoadHandle, GTSL::MoveRef(loadInfo));
}

void RenderOrchestrator::onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{	
	loadInfo.RenderSystem->UpdateTexture(loadInfo.TextureHandle);

	WriteBinding(loadInfo.RenderSystem, textureSubsetsHandle, loadInfo.TextureHandle, loadInfo.Component);
	
	latestLoadedTextures.EmplaceBack(loadInfo.Component);
}

//uint32 textureComponentIndex;
//
//auto textureReference = texturesRefTable.TryGet(resourceMaterialInstanceParameter.Second.TextureReference);
//
//if (!textureReference.State()) {
//	CreateTextureInfo createTextureInfo;
//	createTextureInfo.RenderSystem = renderSystem;
//	createTextureInfo.ApplicationManager = taskInfo.ApplicationManager;
//	createTextureInfo.TextureResourceManager = loadInfo->TextureResourceManager;
//	createTextureInfo.TextureName = resourceMaterialInstanceParameter.Second.TextureReference;
//	createTextureInfo.MaterialHandle = materialInstanceHandle;
//	auto textureComponent = createTexture(createTextureInfo);
//
//	addPendingMaterialToTexture(textureComponent, materialInstanceHandle);
//
//	textureComponentIndex = textureComponent;
//}
//else {
//	textureComponentIndex = textureReference.Get();
//	++materialInstanceData.Counter; //since we up the target for every texture, up the counter for every already existing texture
//}
//
//++materialInstanceData.Target;