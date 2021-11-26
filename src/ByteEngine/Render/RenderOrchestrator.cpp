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

StaticMeshRenderManager::StaticMeshRenderManager(const InitializeInfo& initializeInfo) : RenderManager(initializeInfo, u8"StaticMeshRenderManager"), meshes(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");
	
	onStaticMeshInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"StaticMeshRenderGroup::OnStaticMeshInfoLoad",
		DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager", AccessTypes::READ_WRITE),
			TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE)),
		&StaticMeshRenderManager::onStaticMeshInfoLoaded
	);
	
	onStaticMeshLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"StaticMeshRenderGroup::OnStaticMeshLoad",
		DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE),
		TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup"),
		TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")),
		&StaticMeshRenderManager::onStaticMeshLoaded);

	OnAddMesh = initializeInfo.GameInstance->StoreDynamicTask(this, u8"OnAddMesh",
		DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager"),
			TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"),
			TypedDependency<RenderSystem>(u8"RenderSystem"),
			TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")),
		&StaticMeshRenderManager::onAddMesh);
	OnUpdateMesh = initializeInfo.GameInstance->StoreDynamicTask(this, u8"OnUpdateMesh",
		DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"))
		, &StaticMeshRenderManager::updateMesh);

	GTSL::StaticVector<RenderOrchestrator::MemberInfo, 8> members;
	members.EmplaceBack(&matrixUniformBufferMemberHandle, 1, u8"Matrix4");
	members.EmplaceBack(&vertexBufferReferenceHandle, 1, u8"*");
	members.EmplaceBack(&indexBufferReferenceHandle, 1, u8"*");
	members.EmplaceBack(&materialInstance, 1, u8"uint32");

	staticMeshInstanceDataStruct = renderOrchestrator->MakeMember(u8"StaticMeshData", members);
}
	
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

RenderOrchestrator::RenderOrchestrator(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderOrchestrator"),
	renderingTree(128, GetPersistentAllocator()), resourceNodes(16, GetPersistentAllocator()), renderPasses(16),
	pipelines(8, GetPersistentAllocator()), rayTracingPipelines(4, GetPersistentAllocator()), materials(16, GetPersistentAllocator()),
	materialsByName(16, GetPersistentAllocator()), texturesRefTable(16, GetPersistentAllocator()),
	pendingPipelinesPerTexture(4, GetPersistentAllocator()), buffers(16, GetPersistentAllocator()), attachments(16, GetPersistentAllocator()),
	sets(16, GetPersistentAllocator()), queuedSetUpdates(1, 8, GetPersistentAllocator()), setLayoutDatas(2, GetPersistentAllocator()),
	sizes(16, GetPersistentAllocator()), dataKeys(16, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");

	renderBuffers.EmplaceBack().BufferHandle = renderSystem->CreateBuffer(RENDER_DATA_BUFFER_PAGE_SIZE, GAL::BufferUses::STORAGE, true, false);

	for (uint32 i = 0; i < renderSystem->GetPipelinedFrames(); ++i) {
		descriptorsUpdates.EmplaceBack(GetPersistentAllocator());
	}

	uint32 a = 0;
	//initializeInfo.GameInstance->AddDynamicTask(this, u8"", DependencyBlock{ TypedDependency<RenderSystem>(u8"") }, &RenderOrchestrator::goCrazy<uint32>, {}, {}, GTSL::MoveRef(a));

	sizes.Emplace(u8"uint32", 4);
	sizes.Emplace(u8"uint64", 8);
	sizes.Emplace(u8"float32", 4);
	sizes.Emplace(u8"vec2f", 4 * 2);
	sizes.Emplace(u8"vec3f", 4 * 3);
	sizes.Emplace(u8"vec4f", 4 * 4);
	sizes.Emplace(u8"Matrix4", 4 * 4 * 4);
	sizes.Emplace(u8"TextureReference", 4);
	sizes.Emplace(u8"ImageReference", 4);
	sizes.Emplace(u8"*", 8);

	// MATERIALS

	onTextureInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"RenderOrchestrator::onTextureInfoLoad", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureInfoLoad);
	onTextureLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"RenderOrchestrator::loadTexture", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureLoad);

	onShaderInfosLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"RenderOrchestrator::onShaderGroupInfoLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager")),  &RenderOrchestrator::onShaderInfosLoaded);
	onShaderGroupLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(this, u8"RenderOrchestrator::onShaderGroupLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onShadersLoaded);

	initializeInfo.GameInstance->AddTask(this, SETUP_TASK_NAME, &RenderOrchestrator::Setup, DependencyBlock(), u8"GameplayEnd", u8"RenderStart");
	initializeInfo.GameInstance->AddTask(this, RENDER_TASK_NAME, &RenderOrchestrator::Render, DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem")), u8"RenderDo", u8"RenderFinished");

	//{
	//	GTSL::StaticVector<TaskDependency, 1> dependencies{ { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
	//
	//	auto renderEnableHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::OnRenderEnable", &RenderOrchestrator::OnRenderEnable, dependencies);
	//	//initializeInfo.GameInstance->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusGainEventHandle(), renderEnableHandle);
	//
	//	auto renderDisableHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"RenderOrchestrator::OnRenderDisable", &RenderOrchestrator::OnRenderDisable, dependencies);
	//	//initializeInfo.GameInstance->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusLossEventHandle(), renderDisableHandle);
	//}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onRenderEnable(initializeInfo.GameInstance, taskDependencies);
	}

	{
		GTSL::StaticVector<SubSetInfo, 10> subSetInfos;
		subSetInfos.EmplaceBack(SubSetType::READ_TEXTURES, &textureSubsetsHandle, 16);
		subSetInfos.EmplaceBack(SubSetType::WRITE_TEXTURES, &imagesSubsetHandle, 16);

		{
			MemberHandle AccelerationStructure;
			MemberHandle RayFlags, SBTRecordOffset, SBTRecordStride, MissIndex, Payload;
			MemberHandle tMin, tMax;

			GTSL::StaticVector<MemberInfo, 16> member_infos;
			member_infos.EmplaceBack(&AccelerationStructure, 1, u8"uint64");
			member_infos.EmplaceBack(&RayFlags, 1, u8"uint32"); member_infos.EmplaceBack(&SBTRecordOffset, 1, u8"uint32"); member_infos.EmplaceBack(&SBTRecordStride, 1, u8"uint32"); member_infos.EmplaceBack(&MissIndex, 1, u8"uint32"); member_infos.EmplaceBack(&Payload, 1, u8"uint32");
			member_infos.EmplaceBack(&tMin, 1, u8"float32"); member_infos.EmplaceBack(&tMax, 1, u8"float32");
		}

		globalSetLayout = AddSetLayout(renderSystem, SetLayoutHandle(), subSetInfos);
		globalBindingsSet = AddSet(renderSystem, u8"GlobalData", globalSetLayout, subSetInfos);
	}

	{
		GTSL::StaticVector<MemberInfo, 2> members;
		members.EmplaceBack(&globalDataHandle, 4, u8"uint32");
		auto d = MakeMember(u8"GlobalData", members);
		globalData = AddLayer(u8"GlobalData", NodeHandle());
		BindDataKey(globalData, MakeDataKey(d));
	}

	{
		GTSL::StaticVector<MemberInfo, 2> members;
		members.EmplaceBack(&cameraMatricesHandle, 4, u8"Matrix4");
		auto d = MakeMember(u8"CameraData", members);
		cameraDataNode = AddLayer(u8"CameraData", globalData);
		BindDataKey(cameraDataNode, MakeDataKey(d));
	}

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

	if constexpr (BE_DEBUG) {
		pipelineStages |= BE::Application::Get()->GetOption(u8"debugSync") ? GAL::PipelineStages::ALL_GRAPHICS : GAL::PipelineStage(0);
	}

	{ //BASIC SAMPLER
		auto& sampler = samplers.EmplaceBack();

		sampler.Initialize(renderSystem->GetRenderDevice(), 0);
	}

	{
		AddAttachment(u8"Color", 8, 4, GAL::ComponentType::INT, GAL::TextureType::COLOR);
		AddAttachment(u8"Position", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
		AddAttachment(u8"Normal", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
		AddAttachment(u8"RenderDepth", 32, 1, GAL::ComponentType::FLOAT, GAL::TextureType::DEPTH);

		PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Color" }); //result attachment
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Position" });
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Normal" });
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"RenderDepth" });
		AddPass(u8"SceneRenderPass", GetCameraDataLayer(), renderSystem, geoRenderPass, initializeInfo.GameInstance, initializeInfo.GameInstance->GetSystem<ShaderResourceManager>(u8"ShaderResourceManager"));

		RenderOrchestrator::PassData colorGrading{};
		colorGrading.PassType = RenderOrchestrator::PassType::COMPUTE;
		colorGrading.WriteAttachments.EmplaceBack(u8"Color"); //result attachment
		//auto cgrp = renderOrchestrator->AddPass(u8"ColorGradingRenderPass", renderOrchestrator->GetGlobalDataLayer(), renderSystem, colorGrading, applicationManager, applicationManager->GetSystem<ShaderResourceManager>(u8"ShaderResourceManager"));

		RenderOrchestrator::PassData rtRenderPass{};
		rtRenderPass.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		rtRenderPass.ReadAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Position" });
		rtRenderPass.ReadAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Normal" });
		rtRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Color" }); //result attachment

		//renderOrchestrator->ToggleRenderPass("SceneRenderPass", true);
		//renderOrchestrator->ToggleRenderPass("UIRenderPass", false);
		//renderOrchestrator->ToggleRenderPass("SceneRTRenderPass", true);
	}
}

void RenderOrchestrator::Setup(TaskInfo taskInfo) {
}

void RenderOrchestrator::Render(TaskInfo taskInfo, RenderSystem* renderSystem) {
	//renderSystem->SetHasRendered(renderingEnabled);
	//if (!renderingEnabled) { return; }
	const uint8 currentFrame = renderSystem->GetCurrentFrame(); auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();

	GTSL::Extent2D renderArea = renderSystem->GetRenderExtent();

	if (auto res = renderSystem->AcquireImage(); res || sizeHistory[currentFrame] != sizeHistory[beforeFrame]) {
		OnResize(renderSystem, res.Get());
		renderArea = res.Get();
	}

	updateDescriptors(taskInfo);
	
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();

	BindSet(renderSystem, commandBuffer, globalBindingsSet, GAL::ShaderStages::VERTEX | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN);
	
	{
		auto* cameraSystem = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem");

		auto fovs = cameraSystem->GetFieldOfViews();

		if (fovs.ElementCount()) {
			SetNodeState(cameraDataNode, true);
			auto fov = cameraSystem->GetFieldOfViews()[0]; auto aspectRatio = static_cast<float32>(renderArea.Width) / static_cast<float32>(renderArea.Height);

			GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(fov, aspectRatio, 0.01f, 1000.f);
			projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;

			auto viewMatrix = cameraSystem->GetCameraTransform();

			auto key = GetBufferWriteKey(renderSystem, cameraDataNode, cameraMatricesHandle);
			Write(renderSystem, key, cameraMatricesHandle[0], viewMatrix);
			Write(renderSystem, key, cameraMatricesHandle[1], projectionMatrix);
			Write(renderSystem, key, cameraMatricesHandle[2], GTSL::Math::Inverse(viewMatrix));
			Write(renderSystem, key, cameraMatricesHandle[3], GTSL::Math::BuildInvertedPerspectiveMatrix(fov, aspectRatio, 0.01f, 1000.f));
		} else { //disable rendering for everything which depends on this view
			SetNodeState(cameraDataNode, false);
		}
	}	
	
	RenderState renderState;

	auto updateRenderStages = [&](const GAL::ShaderStage stages) {
		renderState.ShaderStages = stages;
	};

	using RTT = decltype(renderingTree);

	bool le[8]{ false };

	auto runLevel = [&](const decltype(renderingTree)::Key key, const uint32_t level) -> void {
		DataStreamHandle dataStreamHandle = {};

		const auto& baseData = renderingTree.GetBeta(key);

		if constexpr (BE_DEBUG) {
			commandBuffer.BeginRegion(renderSystem->GetRenderDevice(), baseData.Name);
		}

		if (baseData.Offset != 0xFFFFFFFF) { //if node has an associated data entry, bind it
			dataStreamHandle = renderState.AddDataStream();
			le[level] = true;
			auto& setLayout = setLayoutDatas[globalSetLayout()];
			GAL::DeviceAddress bufferAddress = renderSystem->GetBufferDeviceAddress(renderBuffers[0].BufferHandle) + baseData.Offset;
			auto buffAdd = renderSystem->GetBufferPointer(renderBuffers[0].BufferHandle) + baseData.Offset;
			commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), setLayout.PipelineLayout, dataStreamHandle() * 8, GTSL::Range(8, reinterpret_cast<const byte*>(&bufferAddress)), setLayout.Stage);
		}

		//LayerData, MaterialInstanceData, RayTraceData, DispatchData, MeshData, RenderPassData

		switch (renderingTree.GetBetaNodeType(key)) {
		case 3: {
			const DispatchData& dispatchData = renderingTree.GetClass<DispatchData>(key);

			const auto& pipelineData = pipelines[dispatchData.pipelineIndex];
			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelineData.pipeline, GAL::ShaderStages::COMPUTE);
			//commandBuffer.Dispatch(renderSystem->GetRenderDevice(), data.Dispatch.DispatchSize);
			commandBuffer.Dispatch(renderSystem->GetRenderDevice(), renderArea); //todo: change
			break;
		}
		case 2: {
			const RayTraceData& rayTraceData = renderingTree.GetClass<RayTraceData>(key);

			const auto& pipelineData = pipelines[rayTraceData.PipelineIndex];
			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelineData.pipeline, GAL::ShaderStages::RAY_GEN);
			CommandList::ShaderTableDescriptor shaderTableDescriptors[4];
			for (uint8 i = 0; i < 4; ++i) {
				//shaderTableDescriptor.Entries = pipelineData.ShaderGroups[i].ShaderCount;
				//shaderTableDescriptor.EntrySize = pipelineData.ShaderGroups[i].RoundedEntrySize;
				//shaderTableDescriptor.Address = renderSystem->GetBufferDeviceAddress(pipelineData.ShaderGroups[i].Buffer);
			}
			commandBuffer.TraceRays(renderSystem->GetRenderDevice(), GTSL::Range(4, shaderTableDescriptors), sizeHistory[currentFrame]);
			break;
		}
		case 7/*no*/: {
			break;
		}
		case 1: {
			const MaterialInstanceData& materialInstanceData = renderingTree.GetClass<MaterialInstanceData>(key);

			const auto& pipelineData = pipelines[materials[materialInstanceData.MaterialHandle.MaterialIndex].PipelineStart];
			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelineData.pipeline, renderState.ShaderStages);				
			break;
		}
		case 4: {
			const MeshData& meshData = renderingTree.GetClass<MeshData>(key);

			renderSystem->RenderMesh(meshData.Handle, 1);
			break;
		}
		case 5: {
			const RenderPassData& renderPassData = renderingTree.GetClass<RenderPassData>(key);

			switch (renderPassData.Type) {
			case PassType::RASTER: {
				if (!renderState.MaxAPIPass) {
					for (const auto& e : renderPassData.Attachments) {
						updateImage(attachments.At(e.Name), e.Layout, renderPassData.PipelineStages, e.Access);
					}

					updateRenderStages(GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT);

					GTSL::StaticVector<GAL::RenderPassTargetDescription, 8> renderPassTargetDescriptions;
					for (uint8 i = 0; i < renderPassData.Attachments.GetLength(); ++i) {
						if (renderPassData.Attachments[i].Access & GAL::AccessTypes::WRITE) {
							auto& e = renderPassTargetDescriptions.EmplaceBack();
							const auto& attachment = attachments.At(renderPassData.Attachments[i].Name);
							e.ClearValue = attachment.ClearColor;
							e.Start = renderPassData.Attachments[i].Layout;
							e.End = renderPassData.Attachments[i].Layout;
							e.FormatDescriptor = attachment.FormatDescriptor;
							e.Texture = renderSystem->GetTexture(attachment.TextureHandle[currentFrame]);
						}
					}

					commandBuffer.BeginRenderPass(renderSystem->GetRenderDevice(), renderPassData.APIRenderPass.RenderPass, renderPassData.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()],
						renderArea, renderPassTargetDescriptions);

					renderState.MaxAPIPass = renderPassData.APIRenderPass.SubPassCount;
				} else {
					commandBuffer.AdvanceSubPass(renderSystem->GetRenderDevice());
					++renderState.APISubPass;
				}

				break;
			}
			case PassType::COMPUTE: {
				updateRenderStages(GAL::ShaderStages::COMPUTE);
				transitionImages(commandBuffer, renderSystem, &renderPassData);
				break;
			}
			case PassType::RAY_TRACING: {
				updateRenderStages(GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::CALLABLE);
				transitionImages(commandBuffer, renderSystem, &renderPassData);
				break;
			}
			}

			break;
		}
		case 0: {
			break;
		}
		}
	};

	auto endNode = [&](const uint32 key, const uint32_t level) {
		switch (renderingTree.GetBetaNodeType(key)) {
		case RTT::GetTypeIndex<RenderPassData>(): {
			auto& renderPassData = renderingTree.GetClass<RenderPassData>(key);
			if (renderPassData.Type == PassType::RASTER && renderState.MaxAPIPass - 1 == renderState.APISubPass) {
				commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
				renderState.APISubPass = 0;
				renderState.MaxAPIPass = 0;
			}

			break;
		}
		default: break;
		}

		if (le[level]) {
			renderState.PopData();
			le[level] = false;
		}

		if constexpr (BE_DEBUG) {
			commandBuffer.EndRegion(renderSystem->GetRenderDevice());
		}
	};

	ForEachBeta(renderingTree, runLevel, endNode);

	auto& attachment = attachments.At(resultAttachment);

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
		CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());
	
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, attachment.AccessType,
		GAL::AccessTypes::READ, CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout,
		GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor } } }, GetTransientAllocator());

	updateImage(attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);
		
	commandBuffer.CopyTextureToTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachments.At(resultAttachment).TextureHandle[currentFrame]),
	*renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_SOURCE, GAL::TextureLayout::TRANSFER_DESTINATION, 
		attachments.At(resultAttachment).FormatDescriptor, renderSystem->GetSwapchainFormat(),
		GTSL::Extent3D(renderSystem->GetRenderExtent()));
	
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION,
		GAL::TextureLayout::PRESENTATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());
}

MaterialInstanceHandle RenderOrchestrator::CreateMaterial(const CreateMaterialInfo& info) {
	auto materialReference = materialsByName.TryEmplace(info.MaterialName);

	uint32 materialIndex = 0xFFFFFFFF, materialInstanceIndex = 0xFFFFFFFF;
	
	if(materialReference.State()) {
		materialIndex = materials.Emplace(GetPersistentAllocator());
		materialReference.Get() = materialIndex;

		auto pipelineStart = pipelines.Emplace(GetPersistentAllocator());
		pipelines[pipelineStart].ResourceHandle = makeResource();

		bindDataKey(pipelines[pipelineStart].ResourceHandle, MakeDataKey());
		addDependencyCount(pipelines[pipelineStart].ResourceHandle); // Add dependency the pipeline itself

		ShaderLoadInfo sli(GetPersistentAllocator());
		sli.PipelineStart = pipelineStart;
		info.ShaderResourceManager->LoadShaderGroupInfo(info.GameInstance, info.MaterialName, onShaderInfosLoadHandle, GTSL::MoveRef(sli));

		auto& material = materials[materialIndex];
		material.MaterialInstances.EmplaceBack();
		material.PipelineStart = pipelineStart;
		material.Name = info.MaterialName;
		
		materialInstanceIndex = 0;
	} else {
		auto& material = materials[materialReference.Get()];
		materialIndex = materialReference.Get();
		auto index = material.MaterialInstances.LookFor([&](const MaterialInstance& materialInstance) {
			return materialInstance.Name == info.InstanceName;
		});
		
		//TODO: ERROR CHECK

		materialInstanceIndex = index.Get();
	}
	
	return { materialIndex, materialInstanceIndex };
}

void RenderOrchestrator::AddAttachment(Id attachmentName, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type) {
	Attachment attachment;
	attachment.Name = attachmentName;
	attachment.Uses = GAL::TextureUse();

	attachment.Uses |= GAL::TextureUses::ATTACHMENT;
	attachment.Uses |= GAL::TextureUses::SAMPLE;
	
	if (type == GAL::TextureType::COLOR) {		
		attachment.FormatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::COLOR, 0, 1, 2, 3);
		attachment.Uses |= GAL::TextureUses::STORAGE;
		attachment.Uses |= GAL::TextureUses::TRANSFER_SOURCE;
		attachment.ClearColor = GTSL::RGBA(0, 0, 0, 0);
	} else {
		attachment.FormatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::DEPTH, 0, 0, 0, 0);
		attachment.ClearColor = GTSL::RGBA(1, 0, 0, 0);
	}
	
	attachment.Layout = GAL::TextureLayout::UNDEFINED;
	attachment.AccessType = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;

	attachments.Emplace(attachmentName, attachment);
}

RenderOrchestrator::NodeHandle RenderOrchestrator::AddPass(GTSL::StringView renderPassName, NodeHandle parent, RenderSystem* renderSystem, PassData passData, ApplicationManager* am, ShaderResourceManager* srm) {	
	uint32 currentPassIndex = renderPassesInOrder.GetLength();
	
	NodeHandle renderPassNodeHandle = addNode(Id(renderPassName), parent, NodeType::RENDER_PASS);
	InternalNodeHandle internalNodeHandle = addInternalNode<RenderPassData>(Hash(renderPassName), renderPassNodeHandle, parent, InternalNodeType::RENDER_PASS);
	RenderPassData& renderPass = getPrivateNode<RenderPassData>(internalNodeHandle);

	renderPasses.Emplace(renderPassName, renderPassNodeHandle, internalNodeHandle);
	renderPassesInOrder.EmplaceBack(internalNodeHandle);

	renderPass.ResourceHandle = makeResource();
	addDependencyCount(renderPass.ResourceHandle); //add dependency on render pass texture creation

	bindResourceToNode(internalNodeHandle, renderPass.ResourceHandle);

	getInternalNode(internalNodeHandle).Name = GTSL::StringView(renderPassName);

	if(passData.WriteAttachments.GetLength())
		resultAttachment = passData.WriteAttachments[0].Name;

	GTSL::StaticMap<Id, uint32, 16> attachmentsRead;
	
	attachmentsRead.Emplace(resultAttachment, 0xFFFFFFFF); //set result attachment last read as "infinte" so it will always be stored

	for (uint32 i = renderPassesInOrder.GetLength() - 1; i < renderPassesInOrder.GetLength(); --i) {
		auto& rp = getPrivateNode<RenderPassData>(renderPassesInOrder[i]);
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
			
		if constexpr (BE_DEBUG) {
			//auto name = GTSL::StaticString<32>(u8"RenderPass");
			//renderPassCreateInfo.Name = renderPassName;
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
				destinationAccessFlags = GAL::AccessTypes::READ;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
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
				destinationAccessFlags = GAL::AccessTypes::WRITE;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
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

		auto dispatchNodeHandle = addInternalNode<DispatchData>(Hash(renderPassName), renderPassNodeHandle, parent, InternalNodeType::DISPATCH);

		auto loadComputeShader = [&]() -> uint32 {
			auto pipelineIndex = pipelines.Emplace(GetPersistentAllocator());

			const auto acts_on = GTSL::StaticVector<TaskDependency, 16>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };

			auto shaderLoadInfo = ShaderLoadInfo(GetPersistentAllocator());
			shaderLoadInfo.handle = dispatchNodeHandle;
			shaderLoadInfo.PipelineStart = pipelineIndex;
			srm->LoadShaderGroupInfo(am, renderPassName, onShaderInfosLoadHandle, GTSL::MoveRef(shaderLoadInfo));

			getInternalNode(dispatchNodeHandle).Name = GTSL::StringView(renderPassName);

			pipelines[pipelineIndex].ResourceHandle = makeResource();
			addDependencyCount(pipelines[pipelineIndex].ResourceHandle);
			bindResourceToNode(dispatchNodeHandle, pipelines[pipelineIndex].ResourceHandle);

			bindDataKey(pipelines[pipelineIndex].ResourceHandle, MakeDataKey());

			return pipelineIndex;
		};

		getPrivateNode<DispatchData>(dispatchNodeHandle).pipelineIndex = loadComputeShader();

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
	members.EmplaceBack(&renderPass.RenderTargetReferences, 16, u8"ImageReference");
	
	BindDataKey(renderPassNodeHandle, MakeDataKey(MakeMember(u8"RenderPassData", members)));

	for(auto bwk = GetBufferWriteKey(renderSystem, renderPassNodeHandle, renderPass.RenderTargetReferences); bwk < renderPass.Attachments.GetLength(); ++bwk) {
		Write(renderSystem, bwk, renderPass.RenderTargetReferences, attachments[renderPass.Attachments[bwk].Name].ImageIndex);
	}

	return renderPassNodeHandle;
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize)
{
	//pendingDeleteFrames = renderSystem->GetPipelinedFrames();

	auto currentFrame = renderSystem->GetCurrentFrame();
	auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();
	
	auto resize = [&](Attachment& attachment) -> void {
		GTSL::StaticString<64> name(u8"Attachment: "); name += GTSL::StringView(attachment.Name);

		if(attachment.TextureHandle[currentFrame]) {
			//destroy texture
			attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(name, attachment.FormatDescriptor, newSize, attachment.Uses, false);
		} else {
			attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(name, attachment.FormatDescriptor, newSize, attachment.Uses, false);
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

	for (const auto apiRenderPassData : renderPasses) {
		auto& layer = getPrivateNode<RenderPassData>(apiRenderPassData.Second);

		if (layer.Type == PassType::RASTER) {
			if (layer.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].GetHandle())
				layer.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].Destroy(renderSystem->GetRenderDevice());

			GTSL::StaticVector<TextureView, 16> textureViews;
			for (auto& e : layer.Attachments) {
				textureViews.EmplaceBack(renderSystem->GetTextureView(attachments.At(e.Name).TextureHandle[currentFrame]));
			}

			layer.APIRenderPass.FrameBuffer[renderSystem->GetCurrentFrame()].Initialize(renderSystem->GetRenderDevice(), layer.APIRenderPass.RenderPass, newSize, textureViews);
		}

		signalDependencyToResource(layer.ResourceHandle);
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
		auto& renderPassNode = getPrivateNodeFromPublicHandle<RenderPassData>(renderPassName);
		
		switch (renderPassNode.Type) {
		case PassType::RASTER: break;
		case PassType::COMPUTE: break;
		case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetOption(u8"rayTracing"); break; // Enable render pass only if function is enaled in settings
		default: break;
		}

		SetNodeState(renderPassName, enable); //TODO: enable only if resource is not impeding activation
	} else {
		BE_LOG_WARNING(u8"Tried to ", enable ? u8"enable" : u8"disable", u8" a render pass which does not exist.");
	}
}

void RenderOrchestrator::onRenderEnable(ApplicationManager* gameInstance, const GTSL::Range<const TaskDependency*> dependencies)
{
	//gameInstance->AddTask(SETUP_TASK_NAME, &RenderOrchestrator::Setup, DependencyBlock(), u8"GameplayEnd", u8"RenderStart");
	//gameInstance->AddTask(RENDER_TASK_NAME, &RenderOrchestrator::Render, DependencyBlock(), u8"RenderDo", u8"RenderFinished");
}

void RenderOrchestrator::onRenderDisable(ApplicationManager* gameInstance)
{
	//gameInstance->RemoveTask(SETUP_TASK_NAME, u8"GameplayEnd");
	//gameInstance->RemoveTask(RENDER_TASK_NAME, u8"RenderDo");
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

void RenderOrchestrator::transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const RenderPassData* renderPass)
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
		barriers.EmplaceBack(initialStage, renderPass->PipelineStages, attachment.AccessType, access, textureBarrier);

		initialStage |= attachment.ConsumingStages;

		if constexpr (BE_DEBUG) {
			initialStage |= pipelineStages;
		}
		
		updateImage(attachment, attachmentData.Layout, renderPass->PipelineStages, access);
	};
	
	for (auto& e : renderPass->Attachments) { buildTextureBarrier(e, e.ConsumingStages, e.Access); }
	
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, GetTransientAllocator());
}

//TODO: GRANT CONTINUITY TO ALLOCATED PIPELINES PER SHADER GROUP

void RenderOrchestrator::onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager* materialResourceManager,
	ShaderResourceManager::ShaderGroupInfo shader_group_info, ShaderLoadInfo shaderLoadInfo)
{
	shaderLoadInfo.Buffer.Allocate(shader_group_info.Size, 8);

	materialResourceManager->LoadShaderGroup(taskInfo.ApplicationManager, shader_group_info, onShaderGroupLoadHandle, GTSL::Range<byte*>(shader_group_info.Size, shaderLoadInfo.Buffer.GetData()), GTSL::MoveRef(shaderLoadInfo));
}

void RenderOrchestrator::onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem* renderSystem,
                                         ShaderResourceManager::ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer,
                                         ShaderLoadInfo shaderLoadInfo)
{
	if constexpr (BE_DEBUG) {
		if (!shader_group_info.Valid) {
			BE_LOG_ERROR(u8"Tried to load shader group ", shader_group_info.Name, u8" which is not valid. Will use stand in shader. ", BE::FIX_OR_CRASH_STRING);
			return;
		}
	}
	
	GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock, 32> pipelineStates;

	GTSL::StaticMap<Id, GTSL::ShortString<128>, 4> parametersTypes;
	GTSL::StaticMap<Id, GTSL::ShortString<128>, 4> parametersDefaultValues;

	MemberHandle textureReferences[8];

	GTSL::StaticVector<MemberInfo, 16> members;

	for (uint8 i = 0; i < shader_group_info.Parameters; ++i) {
		members.EmplaceBack(&textureReferences[i], 1u, Id(shader_group_info.Parameters[i].Type));
		parametersTypes.Emplace(Id(shader_group_info.Parameters[i].Name), shader_group_info.Parameters[i].Type);
		parametersDefaultValues.Emplace(Id(shader_group_info.Parameters[i].Name), shader_group_info.Parameters[i].Value);
	}

	for (uint8 instance_index = 0; instance_index < shader_group_info.Instances; ++instance_index) {
		GTSL::StaticString<64> structName; structName += shader_group_info.Name; structName += u8"ParametersData";
		UpdateDataKey(pipelines[shaderLoadInfo.PipelineStart + instance_index].ResourceHandle, MakeMember(GTSL::StringView(structName), members));
	}

	for (uint8 instance_index = 0; instance_index < shader_group_info.Instances; ++instance_index) {
		auto& instance = shader_group_info.Instances[instance_index];
		for (uint32 p = 0; p < instance.Parameters; ++p) {
			Id parameterValue;

			//if shader instance has specialized value for param, use that, else, fallback to shader group default value for param
			if (instance.Parameters[p].Second) {
				parameterValue = Id(instance.Parameters[p].Second);
			} else {
				parameterValue = Id(parametersDefaultValues[Id(instance.Parameters[p].First)]);
			}

			switch (Hash(parametersTypes[Id(instance.Parameters[p].First)])) {
			case GTSL::Hash(u8"TextureReference"): {
				uint32 textureComponentIndex;

				auto textureReference = texturesRefTable.TryGet(parameterValue);

				if (!textureReference) {
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.ApplicationManager;
					createTextureInfo.TextureResourceManager = taskInfo.ApplicationManager->GetSystem<TextureResourceManager>(u8"TextureResourceManager");
					createTextureInfo.TextureName = static_cast<GTSL::StringView>(parameterValue);
					textureReference.Get() = createTexture(createTextureInfo);
					textureComponentIndex = textureReference.Get();
				} else {
					textureComponentIndex = textureReference.Get();
				}

				Write(renderSystem, GetBufferWriteKey(renderSystem, pipelines[shaderLoadInfo.PipelineStart + instance_index].ResourceHandle, textureReferences[p]), textureReferences[p], textureComponentIndex);

				addPendingPipelineToTexture(textureComponentIndex, shaderLoadInfo.PipelineStart);

				break;
			}
			case GTSL::Hash(u8"ImageReference"): {
				auto textureReference = attachments.TryGet(parameterValue);

				if (textureReference) {
					uint32 textureComponentIndex = textureReference.Get().ImageIndex;

					Write(renderSystem, GetBufferWriteKey(renderSystem, pipelines[shaderLoadInfo.PipelineStart + instance_index].ResourceHandle, textureReferences[p]), textureReferences[p], textureComponentIndex);
				} else {
					BE_LOG_WARNING(u8"Default parameter value of ", GTSL::StringView(parameterValue), u8" for shader group ", shader_group_info.Name, u8" parameter ", instance.Parameters[p].First, u8" could not be found.");
				}

				break;
			}
			}
		}
	}

	if(shader_group_info.Stages & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::MESH)) {
		auto materialIndex = shaderLoadInfo.MaterialIndex;

		auto& materialData = materials[materialIndex];
		materialData.Name = Id(shader_group_info.Name);
		
		GTSL::StaticVector<::Pipeline::ShaderInfo, 8> shaderInfos;
		GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> vertexElements;
		GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock::RenderContext::AttachmentState, 8> att;

		for (uint32 offset = 0; const auto& s : shader_group_info.Shaders) {
			auto& shaderInfo = shaderInfos.EmplaceBack();
			shaderInfo.Type = s.Type;
			shaderInfo.Blob = GTSL::Range(s.Size, shaderLoadInfo.Buffer.GetData() + offset);
			shaderInfo.Shader.Initialize(renderSystem->GetRenderDevice(), shaderInfo.Blob);

			offset += s.Size;

			switch (s.Type) {
			case GAL::ShaderType::VERTEX: {
				for (auto& e : s.VertexShader.VertexElements) {
					vertexElements.EmplaceBack(e);
				}

				break;
			}
			}
		}

		GAL::Pipeline::PipelineStateBlock::RenderContext context;

		for (const auto& writeAttachment : getPrivateNode<RenderPassData>(renderPasses.At(Id(shader_group_info.RenderPass)).Second).Attachments) {
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

		for (uint8 materialInstanceIndex = 0; materialInstanceIndex < shader_group_info.Instances; ++materialInstanceIndex) {
			auto& rasterMaterialInstanceData = pipelines[shaderLoadInfo.PipelineStart + materialInstanceIndex];
			rasterMaterialInstanceData.pipeline.InitializeRasterPipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}
	} else if (shader_group_info.Stages & (GAL::ShaderStages::COMPUTE)) {
		GTSL::StaticVector<::Pipeline::ShaderInfo, 1> shaderInfos;
				
		for (uint32 offset = 0; auto& s : shader_group_info.Shaders) {
			auto& shaderInfo = shaderInfos.EmplaceBack();
			shaderInfo.Type = s.Type;
			shaderInfo.Shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range(s.Size, shaderLoadInfo.Buffer.GetData() + offset));;
			shaderInfo.Blob = GTSL::Range(s.Size, shaderLoadInfo.Buffer.GetData() + offset);

			offset += s.Size;
		}

		auto& pipeline = pipelines[shaderLoadInfo.PipelineStart];
		pipeline.pipeline.InitializeComputePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
	} else  if (shader_group_info.Stages & (GAL::ShaderStages::RAY_GEN)) {
		auto rtPipelineIndex = rayTracingPipelines.Emplace();
		auto& pipelineData = pipelines[shaderLoadInfo.PipelineStart];
		auto& rtPipelineData = rayTracingPipelines[rtPipelineIndex];
		
		GTSL::Vector<::Pipeline::RayTraceGroup, BE::TAR> groups(16, GetTransientAllocator());
		GTSL::Vector<::Pipeline::ShaderInfo, BE::TAR> shaderInfos(16, GetTransientAllocator());
		
		auto handleSize = renderSystem->GetShaderGroupHandleSize();
		auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupHandleAlignment());

		::Pipeline::PipelineStateBlock::RayTracingState rtInfo;
		rtInfo.Groups = groups;
		rtInfo.MaxRecursionDepth = 0;
		
		uint32 offset = 0;//
		
		for (uint32 i = 0; i < shader_group_info.Shaders.GetLength(); ++i) {
			auto& rayTracingShaderInfo = shader_group_info.Shaders[i];
			
			{
				auto& shader = shaderInfos.EmplaceBack();
				shader.Type = rayTracingShaderInfo.Type;
				shader.Blob = GTSL::Range(rayTracingShaderInfo.Size, buffer.begin() + offset);
				shader.Shader.Initialize(renderSystem->GetRenderDevice(), shader.Blob);

				offset += rayTracingShaderInfo.Size;
			}
		
			uint8 shaderGroup = 0xFF; ::Pipeline::RayTraceGroup group{};
			group.GeneralShader = ::Pipeline::RayTraceGroup::SHADER_UNUSED; group.ClosestHitShader = ::Pipeline::RayTraceGroup::SHADER_UNUSED;
			group.AnyHitShader = ::Pipeline::RayTraceGroup::SHADER_UNUSED; group.IntersectionShader = ::Pipeline::RayTraceGroup::SHADER_UNUSED;
		
			switch (rayTracingShaderInfo.Type) {
			case GAL::ShaderType::RAY_GEN: {
				group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
				shaderGroup = GAL::RAY_GEN_TABLE_INDEX;

				GTSL::Max(&rtInfo.MaxRecursionDepth, rayTracingShaderInfo.RayGenShader.Recursion);
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
			default: BE_LOG_MESSAGE(u8"Non raytracing shader found in raytracing material");
			}
		
			groups.EmplaceBack(group);

			++rtPipelineData.ShaderGroups[shaderGroup].ShaderCount;

			rayTracingMaterials.Emplace((uint64)0/*todo: implement*/ << 32 | shaderLoadInfo.MaterialIndex, RTMI{rtPipelineIndex, shaderGroup, 0/*todo: implement*/});
		}

		//{
		//	auto renderPassLayerHandle = addNode<RenderPassData>(Id(shader_group_info.RenderPass), cameraDataNode, NodeType::RENDER_PASS);
		//
		//	rayTraceNode = addNode<RayTraceData>(Id(shader_group_info.Name), renderPassLayerHandle, NodeType::RAY_TRACE);
		//	getPrivateNodeFromPublicHandle<RayTraceData>(rayTraceNode).PipelineIndex = rtPipelineIndex;
		//}

		pipelineStates.EmplaceBack(rtInfo);
		
		pipelineData.pipeline.InitializeRayTracePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());

		GTSL::Vector<GAL::ShaderHandle, BE::TAR> shaderGroupHandlesBuffer(shaderInfos.GetLength(), GetTransientAllocator());
		
		pipelineData.pipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, groups.GetLength(), shaderGroupHandlesBuffer);

		//create buffer per shader group
		for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
			auto& groupData = rtPipelineData.ShaderGroups[shaderGroupIndex];

			//GTSL::StaticVector<MemberInfo, 2> membersA{ { &groupData.MaterialDataHandle, 1, u8"*"}, { &groupData.ObjectDataHandle, 1, u8"*" } };
			//GTSL::StaticVector<MemberInfo, 2> members{ { &groupData.ShaderHandle, 1, u8"ShaderHandle" }, { &groupData.ShaderEntryMemberHandle, 16, membersA, u8"*" } };
			//GTSL::StaticVector<MemberInfo, 2> elements{ MemberInfo(&groupData.ShaderGroupDataHandle, groupData.ShaderCount, members, renderSystem->GetShaderGroupBaseAlignment()) };
			//
			//groupData.Buffer = CreateBuffer(renderSystem, elements);
			//materialData.DataKey = MakeDataKey()
			
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

	signalDependencyToResource(pipelines[shaderLoadInfo.PipelineStart].ResourceHandle); //add ref count for pipeline load itself
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo)
{
	auto component = textureIndex++;

	pendingPipelinesPerTexture.EmplaceAt(component, GetPersistentAllocator());

	texturesRefTable.Emplace(Id(createTextureInfo.TextureName), component);

	auto textureLoadInfo = TextureLoadInfo(component, RenderAllocation());

	createTextureInfo.TextureResourceManager->LoadTextureInfo(createTextureInfo.GameInstance, Id(createTextureInfo.TextureName), onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));

	return component;
}

void RenderOrchestrator::onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem* renderSystem,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	GTSL::StaticString<128> name(u8"Texture resource: "); name += GTSL::Range<const char8_t*>(textureInfo.Name);

	loadInfo.TextureHandle = renderSystem->CreateTexture(name, textureInfo.Format, textureInfo.Extent, GAL::TextureUses::SAMPLE | GAL::TextureUses::ATTACHMENT, true);

	auto dataBuffer = renderSystem->GetTextureRange(loadInfo.TextureHandle);

	resourceManager->LoadTexture(taskInfo.ApplicationManager, textureInfo, dataBuffer, onTextureLoadHandle, GTSL::MoveRef(loadInfo));
}

void RenderOrchestrator::onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem* renderSystem,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{	
	renderSystem->UpdateTexture(loadInfo.TextureHandle);

	WriteBinding(renderSystem, textureSubsetsHandle, loadInfo.TextureHandle, loadInfo.Component);

	for (auto e : pendingPipelinesPerTexture[loadInfo.Component]) {
		auto& l = pipelines[e];
		signalDependencyToResource(pipelines[e].ResourceHandle);
	}
}