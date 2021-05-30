#include "RenderOrchestrator.h"

#undef MemoryBarrier

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>
#include "LightsRenderGroup.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/Tasks.h"
#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "UIManager.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Templates/GameApplication.h"
#include "ByteEngine/Game/CameraSystem.h"

static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -0.5f, 0.5f }, { 0.5f, 0.5f }, { 0.5f, -0.5f }, { -0.5f, -0.5f } };
//static constexpr GTSL::Vector2 SQUARE_VERTICES[] = { { -1.0f, 1.0f }, { 1.0f, 1.0f }, { 1.0f, -1.0f }, { -1.0f, -1.0f } };
static constexpr uint16 SQUARE_INDICES[] = { 0, 1, 3, 1, 2, 3 };

void StaticMeshRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
	
	GTSL::Array<MaterialSystem::MemberInfo, 8> members;
	members.EmplaceBack(&matrixUniformBufferMemberHandle, 1);
	members.EmplaceBack(&vertexBufferReferenceHandle, 1);
	members.EmplaceBack(&indexBufferReferenceHandle, 1);
	members.EmplaceBack(&materialInstance, 1);
	//members.EmplaceBack(4); //padding
	
	//TODO: MAKE A CORRECT PATH FOR DECLARING RENDER PASSES

	auto bufferHandle = materialSystem->CreateBuffer(renderSystem, MaterialSystem::MemberInfo(&staticMeshStruct, 16, members));
	materialSystem->BindBufferToName(bufferHandle, "StaticMeshRenderGroup");
}

void StaticMeshRenderManager::GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ "StaticMeshRenderGroup", AccessTypes::READ });
}

void StaticMeshRenderManager::Setup(const SetupInfo& info)
{
	auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	
	info.MaterialSystem->UpdateObjectCount(info.RenderSystem, staticMeshStruct, renderGroup->GetStaticMeshCount());

	{
		MaterialSystem::BufferIterator bufferIterator;
	
		for (uint32 p = 0; p < renderGroup->GetAddedMeshes().GetPageCount(); ++p)
		{
			for (auto e : renderGroup->GetAddedMeshes().GetPage(p))
			{
				info.MaterialSystem->UpdateIteratorMember(bufferIterator, staticMeshStruct, e.Second);
				
				info.RenderOrchestrator->AddMesh(info.RenderOrchestrator->GetStaticMeshRenderGroup(), e.First, info.RenderSystem->GetMeshMaterialHandle(e.First), e.Second, info.RenderSystem->GetMeshVertexLayout(e.First));
	
				//auto vertexBuffer = info.RenderSystem->GetVertexBufferAddress(e.First), indexBuffer = info.RenderSystem->GetIndexBufferAddress(e.First);
				////info.MaterialSystem->WriteMultiBuffer(bufferIterator, vertexBufferReferenceHandle, &vertexBuffer);
				////info.MaterialSystem->WriteMultiBuffer(bufferIterator, indexBufferReferenceHandle, &indexBuffer);
			}
		}
	
		renderGroup->ClearAddedMeshes();
	}

	{
		auto handleSize = GTSL::Math::RoundUpByPowerOf2(info.RenderSystem->GetShaderGroupHandleSize(), info.RenderSystem->GetShaderGroupHandleAlignment());
	
		MaterialSystem::BufferIterator bufferIterator;

		GTSL::MultiFor([&](uint32 i, const RenderSystem::MeshHandle& meshHandle)
		{
			auto pos = renderGroup->GetMeshTransform(i);
			
			info.MaterialSystem->UpdateIteratorMember(bufferIterator, staticMeshStruct, i);
			*info.MaterialSystem->GetMemberPointer(bufferIterator, matrixUniformBufferMemberHandle) = pos;
			*info.MaterialSystem->GetMemberPointer(bufferIterator, vertexBufferReferenceHandle) = info.RenderSystem->GetVertexBufferAddress(meshHandle);
			*info.MaterialSystem->GetMemberPointer(bufferIterator, indexBufferReferenceHandle) = info.RenderSystem->GetIndexBufferAddress(meshHandle);
			auto materialHandle = info.RenderSystem->GetMeshMaterialHandle(meshHandle);
			*info.MaterialSystem->GetMemberPointer(bufferIterator, materialInstance) = materialHandle.MaterialInstanceIndex;


			if (BE::Application::Get()->GetOption("rayTracing")) {
				info.RenderSystem->SetMeshMatrix(meshHandle, GTSL::Matrix3x4(pos));
				//info.RenderSystem->SetMeshOffset(RenderSystem::MeshHandle(index), index * 96);
				//info.RenderSystem->SetMeshOffset(RenderSystem::MeshHandle(index), index);
			}
		}, renderGroup->GetStaticMeshCount(), renderGroup->GetMeshHandles());
		
		//for (auto& e : renderGroup->GetMeshHandles())
		//{
		//
		//
		//	++index;
		//}
	}

	//if ray tracing
	//info.RenderSystem->SetMeshMatrix();
	//clear updated meshes
}

void UIRenderManager::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderOrchestrator = initializeInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
	
	//RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
	//createMaterialInfo.RenderSystem = renderSystem;
	//createMaterialInfo.GameInstance = initializeInfo.GameInstance;
	//createMaterialInfo.MaterialName = "UIMat";
	//createMaterialInfo.InstanceName = "UIMat";
	//createMaterialInfo.MaterialResourceManager = BE::Application::Get()->GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//createMaterialInfo.TextureResourceManager = BE::Application::Get()->GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//uiMaterial = renderOrchestrator->CreateMaterial(createMaterialInfo);
	//
	//square = renderSystem->CreateMesh("BE_UI_SQUARE", 0, GetUIMaterial());
	//renderSystem->UpdateMesh(square, 4, 4 * 2, 6, 2, GTSL::Array<GAL::ShaderDataType, 4>{ GAL::ShaderDataType::FLOAT2 });
	////
	//auto* meshPointer = renderSystem->GetMeshPointer(square);
	//GTSL::MemCopy(4 * 2 * 4, SQUARE_VERTICES, meshPointer);
	//meshPointer += 4 * 2 * 4;
	//GTSL::MemCopy(6 * 2, SQUARE_INDICES, meshPointer);
	//renderSystem->UpdateMesh(square);
	//renderSystem->SetWillWriteMesh(square, false);	
	//
	//GTSL::Array<MaterialSystem::MemberInfo, 8> members;
	//members.EmplaceBack(&matrixUniformBufferMemberHandle, 1);
	////members.EmplaceBack(4); //padding
	//
	////TODO: MAKE A CORRECT PATH FOR DECLARING RENDER PASSES
	//
	//auto bufferHandle = materialSystem->CreateBuffer(renderSystem, MaterialSystem::MemberInfo(&uiDataStruct, 16, members));
	//materialSystem->BindBufferToName(bufferHandle, "UIRenderGroup");
	//renderOrchestrator->AddToRenderPass("UIRenderPass", "UIRenderGroup");
}

void UIRenderManager::GetSetupAccesses(GTSL::Array<TaskDependency, 16>& dependencies)
{
	dependencies.EmplaceBack(TaskDependency{ "UIManager", AccessTypes::READ });
	dependencies.EmplaceBack(TaskDependency{ "CanvasSystem", AccessTypes::READ });
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
		
		GTSL::Matrix4 ortho = GTSL::Math::MakeOrthoMatrix(1.0f, -1.0f, yxRatio, -yxRatio, 0, 100);
		
		//GTSL::Math::MakeOrthoMatrix(ortho, canvasSize.Width, -canvasSize.Width, canvasSize.Height, -canvasSize.Height, 0, 100);
		//GTSL::Math::MakeOrthoMatrix(ortho, 0.5f, -0.5f, 0.5f, -0.5f, 1, 100);
		
		auto& organizers = canvas.GetOrganizersTree();

		auto primitives = canvas.GetPrimitives();
		auto squares = canvas.GetSquares();

		const auto* parentOrganizer = organizers[0];

		uint32 sq = 0;
		for(auto& e : squares)
		{
			GTSL::Matrix4 trans(1.0f);

			auto location = primitives.begin()[e.PrimitiveIndex].RelativeLocation;
			auto scale = primitives.begin()[e.PrimitiveIndex].AspectRatio;
			//
			GTSL::Math::AddTranslation(trans, GTSL::Vector3(location.X(), -location.Y(), 0));
			GTSL::Math::Scale(trans, GTSL::Vector3(scale.X(), scale.Y(), 1));
			//GTSL::Math::Scale(trans, GTSL::Vector3(static_cast<float32>(canvasSize.Width), static_cast<float32>(canvasSize.Height), 1));
			//

			MaterialSystem::BufferIterator iterator;
			info.MaterialSystem->UpdateIteratorMember(iterator, uiDataStruct, sq);
			*info.MaterialSystem->GetMemberPointer(iterator, matrixUniformBufferMemberHandle) = trans * ortho;
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

	renderManagers.Initialize(16, GetPersistentAllocator());
	setupSystemsAccesses.Initialize(16, GetPersistentAllocator());
	renderingTree.Initialize(GetPersistentAllocator());
	renderingTreeMap.Initialize(16, GetPersistentAllocator());

	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = initializeInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");

	globalData = AddLayer("GlobalData", {}, LayerType::LAYER);
	cameraDataLayer = AddLayer("CameraData", "GlobalData", LayerType::LAYER);
	sceneRenderPass = AddLayer("SceneRenderPass", "CameraData", LayerType::RENDER_PASS);
	staticMeshRenderGroup = AddLayer("StaticMeshRenderGroup", "SceneRenderPass", LayerType::LAYER);
	//AddLayer("UIRenderPass", "GlobalData");
	
	// MATERIALS

	materials.Initialize(16, GetPersistentAllocator());
	materialsByName.Initialize(16, GetPersistentAllocator());
	
	texturesRefTable.Initialize(32, GetPersistentAllocator());
	pendingMaterialsPerTexture.Initialize(8, GetPersistentAllocator());
	latestLoadedTextures.Initialize(8, GetPersistentAllocator());
	 
	// MATERIALS
	
	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		onTextureInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onTextureInfoLoad", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onTextureInfoLoad>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		onTextureLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("loadTexture", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onTextureLoad>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderOrchestrator", AccessTypes::READ } };
		onShaderInfosLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onShaderInfosLoaded", Task<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onShaderInfosLoaded>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		onShadersLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onShadersLoaded", Task<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onShadersLoaded>(this), taskDependencies);
	}

	{
		GTSL::Array<TaskDependency, 1> dependencies{ { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		
		auto renderEnableHandle = initializeInfo.GameInstance->StoreDynamicTask("RO::OnRenderEnable", Task<bool>::Create<RenderOrchestrator, &RenderOrchestrator::OnRenderEnable>(this), dependencies);
		initializeInfo.GameInstance->SubscribeToEvent("Application", GameApplication::GetOnFocusGainEventHandle(), renderEnableHandle);

		auto renderDisableHandle = initializeInfo.GameInstance->StoreDynamicTask("RO::OnRenderDisable", Task<bool>::Create<RenderOrchestrator, &RenderOrchestrator::OnRenderDisable>(this), dependencies);
		initializeInfo.GameInstance->SubscribeToEvent("Application", GameApplication::GetOnFocusLossEventHandle(), renderDisableHandle);
	}
	
	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		onRenderEnable(initializeInfo.GameInstance, taskDependencies);
	}

	
	{
		GTSL::Array<MaterialSystem::SubSetInfo, 10> subSetInfos;

		{ // TEXTURES
			MaterialSystem::SubSetInfo subSetInfo;
			subSetInfo.Type = MaterialSystem::SubSetType::READ_TEXTURES;
			subSetInfo.Count = 16;
			subSetInfo.Handle = &textureSubsetsHandle;
			subSetInfos.EmplaceBack(subSetInfo);
		}

		{ // IMAGES
			MaterialSystem::SubSetInfo subSetInfo;
			subSetInfo.Type = MaterialSystem::SubSetType::WRITE_TEXTURES;
			subSetInfo.Count = 16;
			subSetInfo.Handle = &imagesSubsetHandle;
			subSetInfos.EmplaceBack(subSetInfo);
		}

		if (BE::Application::Get()->GetOption("rayTracing"))
		{
			{ //TOP LEVEL AS
				MaterialSystem::SubSetInfo subSetInfo;
				subSetInfo.Type = MaterialSystem::SubSetType::ACCELERATION_STRUCTURE;
				subSetInfo.Handle = &topLevelAsHandle;
				subSetInfo.Count = 1;
				subSetInfos.EmplaceBack(subSetInfo);
			}
		}

		materialSystem->AddSetLayout(renderSystem, "GlobalData", Id(), subSetInfos);
		materialSystem->AddSet(renderSystem, "GlobalData", "GlobalData", subSetInfos);
	}
	
	{
		GTSL::Array<MaterialSystem::MemberInfo, 2> members;
		members.EmplaceBack(&globalDataHandle, 4);

		globalDataBuffer = materialSystem->CreateBuffer(renderSystem, members);
	}

	{
		GTSL::Array<MaterialSystem::MemberInfo, 2> members;
		members.EmplaceBack(&cameraMatricesHandle, 4);

		cameraDataBuffer = materialSystem->CreateBuffer(renderSystem, members);
	}

	if (BE::Application::Get()->GetOption("rayTracing")) {
		rayTracingPipelines.Initialize(4, GetPersistentAllocator());

		auto& pipelineData = rayTracingPipelines[rayTracingPipelines.Emplace()];
		for (auto& e : pipelineData.ShaderGroups) { e.Shaders.Initialize(4, GetPersistentAllocator()); }
		
		auto* materialResorceManager = BE::Application::Get()->GetResourceManager<MaterialResourceManager>("MaterialResourceManager");

		GTSL::Vector<GAL::VulkanShader, BE::TAR> shaders(16, GetTransientAllocator());
		GTSL::Vector<Pipeline::RayTraceGroup, BE::TAR> groups(16, GetTransientAllocator());
		GTSL::Vector<Pipeline::ShaderInfo, BE::TAR> shaderInfos(16, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shadersBuffer;

		auto handleSize = renderSystem->GetShaderGroupHandleSize();
		auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupHandleAlignment());

		MaterialSystem::MemberInfo memberInfos[4][3]{};
		uint32 maxBuffers[4]{ 0 };// uint32 instances[4]{ 0 };

		auto pipelineInfo = materialResorceManager->GetRayTracePipelineInfo();

		{
			auto pipelineSize = 0u;

			for (auto& shader : pipelineInfo.Shaders)
				pipelineSize += shader.BinarySize;

			shadersBuffer.Allocate(pipelineSize, 32, GetTransientAllocator());

			materialResorceManager->LoadRayTraceShadersForPipeline(pipelineInfo, GTSL::Range<byte*>(pipelineSize, shadersBuffer.GetData())); //TODO: VIRTUAL BUFFER INTERFACE
		}
		
		uint32 offset = 0;
		
		for (uint32 i = 0; i < pipelineInfo.Shaders.GetLength(); ++i) {
			auto& rayTracingShaderInfo = pipelineInfo.Shaders[i];
			auto& shader = shaders.EmplaceBack();
			shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range<const byte*>(rayTracingShaderInfo.BinarySize, shadersBuffer.GetData() + offset));
			auto& shaderInfo = shaderInfos.EmplaceBack();
			shaderInfo.Type = rayTracingShaderInfo.ShaderType;
			shaderInfo.Shader = shader;
			offset += rayTracingShaderInfo.BinarySize;

			uint8 shaderGroup = 0xFF; Pipeline::RayTraceGroup group{};
			group.GeneralShader = Pipeline::RayTraceGroup::SHADER_UNUSED; group.ClosestHitShader = Pipeline::RayTraceGroup::SHADER_UNUSED;
			group.AnyHitShader = Pipeline::RayTraceGroup::SHADER_UNUSED; group.IntersectionShader = Pipeline::RayTraceGroup::SHADER_UNUSED;

			switch (rayTracingShaderInfo.ShaderType) {
			case GAL::ShaderType::RAY_GEN: {
				group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
				shaderGroup = GAL::RAY_GEN_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::MISS: {
				//generalShader is the index of the ray generation,miss, or callable shader from VkRayTracingPipelineCreateInfoKHR::pStages
				//in the group if the shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_GENERAL_KHR, and VK_SHADER_UNUSED_KHR otherwise.
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
				//closestHitShader is the optional index of the closest hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::ANY_HIT: {
				//anyHitShader is the optional index of the any-hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the
				//shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR,
				//and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.AnyHitShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}
			case GAL::ShaderType::INTERSECTION: {
				//intersectionShader is the index of the intersection shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::ShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
				shaderGroup = GAL::HIT_TABLE_INDEX;
				break;
			}

			default: BE_LOG_MESSAGE("Non raytracing shader found in raytracing material");
			}

			groups.EmplaceBack(group);			
		
			for (auto & shaderData = pipelineData.ShaderGroups[shaderGroup].Shaders.EmplaceBack(); auto& s : rayTracingShaderInfo.MaterialInstances) {					
				for (auto& b : s)
					shaderData.Buffers.EmplaceBack(Id(b), false);
				
				maxBuffers[shaderGroup] = GTSL::Math::Max(shaderData.Buffers.GetLength(), maxBuffers[shaderGroup]);
			}
		}

		//create buffer per shader group
		for(uint32 shaderGroupIndex = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
			auto& groupData = pipelineData.ShaderGroups[shaderGroupIndex];

			memberInfos[shaderGroupIndex][0] = MaterialSystem::MemberInfo(&groupData.ShaderHandle, 1); //shader handle
			memberInfos[shaderGroupIndex][1] = MaterialSystem::MemberInfo(&groupData.BufferBufferReferencesMemberHandle, maxBuffers[shaderGroupIndex]); //material buffer
			memberInfos[shaderGroupIndex][2] = MaterialSystem::MemberInfo(0); //padding
			
			uint32 size = alignedHandleSize + maxBuffers[shaderGroupIndex] * 4;
			uint32 alignedSize = GTSL::Math::RoundUpByPowerOf2(size, renderSystem->GetShaderGroupBaseAlignment());
			memberInfos[shaderGroupIndex][2].Count = alignedSize - size; //pad
			groupData.RoundedEntrySize = alignedSize;
			
			groupData.Buffer = materialSystem->CreateBuffer(renderSystem, MaterialSystem::MemberInfo(&groupData.EntryHandle, groupData.Shaders.GetLength(), GTSL::Range<MaterialSystem::MemberInfo*>(3, memberInfos[shaderGroupIndex])));
		}
				
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("Ray Tracing Pipeline: ");// createInfo.Name = name;
		}

		GTSL::Array<Pipeline::PipelineStateBlock, 4> pipelineStates;

		{
			Pipeline::PipelineStateBlock::RayTracingState rtInfo;
			rtInfo.Groups = groups;
			rtInfo.MaxRecursionDepth = pipelineInfo.RecursionDepth;
			pipelineStates.EmplaceBack(rtInfo);
		}
		
		pipelineData.Pipeline.InitializeRayTracePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, materialSystem->GetSetLayoutPipelineLayout(Id("GlobalData")), PipelineCache());

		GTSL::Buffer<BE::TAR> handlesBuffer; handlesBuffer.Allocate(groups.GetLength() * alignedHandleSize, 16, GetTransientAllocator());

		pipelineData.Pipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, groups.GetLength(), GTSL::Range<byte*>(handlesBuffer.GetCapacity(), handlesBuffer.GetData()));

		//buffer contains aligned handles

		
		for (uint32 shaderHandleIndex = 0, shaderGroupIndex = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
			auto& shaderGroup = pipelineData.ShaderGroups[shaderGroupIndex];

			MaterialSystem::BufferIterator iterator;

			for (uint32 s = 0; s < shaderGroup.Shaders.GetLength(); ++s) { //make index
				materialSystem->UpdateIteratorMember(iterator, shaderGroup.EntryHandle, s);

				GAL::ShaderHandle shaderHandle(handlesBuffer.GetData() + shaderHandleIndex * alignedHandleSize, handleSize, alignedHandleSize);
				materialSystem->WriteMultiBuffer(iterator, shaderGroup.ShaderHandle, &shaderHandle, 0);
				
				++shaderHandleIndex;
			}
		}
		
		for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
			materialSystem->UpdateSet2(topLevelAsHandle, 0, renderSystem->GetTopLevelAccelerationStructure(f), f);
		}
	}
}

void RenderOrchestrator::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void RenderOrchestrator::Setup(TaskInfo taskInfo)
{
	//if (!renderingEnabled) { return; }
	
	auto fovs = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetFieldOfViews();

	GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(fovs[0], 16.f / 9.f, 0.01f, 1000.f);

	auto cameraTransform = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetCameraTransform();

	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	
	RenderManager::SetupInfo setupInfo;
	setupInfo.GameInstance = taskInfo.GameInstance;
	setupInfo.RenderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	setupInfo.MaterialSystem = materialSystem;
	setupInfo.ProjectionMatrix = projectionMatrix;
	setupInfo.ViewMatrix = cameraTransform;
	setupInfo.RenderOrchestrator = this;
	GTSL::ForEach(renderManagers, [&](SystemHandle renderManager) { taskInfo.GameInstance->GetSystem<RenderManager>(renderManager)->Setup(setupInfo); });

	for (auto e : latestLoadedTextures) {
		for (auto b : pendingMaterialsPerTexture[e]) {
			auto& materialInstance = materials[b.MaterialIndex].MaterialInstances[b.MaterialInstanceIndex];
			if (++materialInstance.Counter == materialInstance.Target) {
				//setMaterialInstanceAsLoaded(b, materialInstance.Name);
				//taskInfo.GameInstance->DispatchEvent("MaterialSystem", GetOnMaterialInstanceLoadEventHandle(), GTSL::MoveRef(material.Name), GTSL::MoveRef(materialInstance.Name));
			}
		}
	}

	latestLoadedTextures.ResizeDown(0);
}

void RenderOrchestrator::Render(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	//renderSystem->SetHasRendered(renderingEnabled);
	//if (!renderingEnabled) { return; }
	auto renderArea = renderSystem->GetRenderExtent();
	
	if (renderArea == 0) { return; }

	if (renderSystem->AcquireImage()) { OnResize(renderSystem, materialSystem, renderArea); }
	
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();
	uint8 currentFrame = renderSystem->GetCurrentFrame();

	materialSystem->BindSet(renderSystem, commandBuffer, "GlobalData", GAL::ShaderStages::VERTEX);
	materialSystem->BindSet(renderSystem, commandBuffer, "GlobalData", GAL::ShaderStages::COMPUTE);
	materialSystem->BindSet(renderSystem, commandBuffer, "GlobalData", GAL::ShaderStages::RAY_GEN);
	
	{
		auto* cameraSystem = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem");

		GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(cameraSystem->GetFieldOfViews()[0], 16.f / 9.f, 0.01f, 1000.f);
		projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;
		
		auto viewMatrix = cameraSystem->GetCameraTransform();
		
		MaterialSystem::BufferIterator bufferIterator;

		*materialSystem->GetMemberPointer(bufferIterator, cameraMatricesHandle, 0) = viewMatrix;
		*materialSystem->GetMemberPointer(bufferIterator, cameraMatricesHandle, 1) = projectionMatrix;
		*materialSystem->GetMemberPointer(bufferIterator, cameraMatricesHandle, 2) = GTSL::Math::Inverse(viewMatrix);
		*materialSystem->GetMemberPointer(bufferIterator, cameraMatricesHandle, 3) = GTSL::Math::Inverse(projectionMatrix);
	}

	auto visitLevel = [&](auto&& self, const GTSL::Tree<Layer, BE::PAR>::Node* node) -> void {
		for (uint32 n = 0; n < node->Nodes.GetLength(); ++n) {
			const Layer& data = node->Data; IndexStreamHandle indexStream = {};

			commandBuffer.BeginRegion(renderSystem->GetRenderDevice(), static_cast<GTSL::Range<const utf8*>>(data.Name));
			
			if (data.Name()) {
				BindData(renderSystem, materialSystem, commandBuffer, materialSystem->GetBuffer(data.Name));
				if (data.Indexed) { indexStream = AddIndexStream(); }
			}
			
			switch (data.PrivateType) {
			case InternalLayerType::DISPATCH: {
				commandBuffer.Dispatch(renderSystem->GetRenderDevice(), data.Dispatch.DispatchSize);
				break;
			}

			case InternalLayerType::RAY_TRACE: {
				auto& pipelineData = rayTracingPipelines[0];

				for (auto& sg : pipelineData.ShaderGroups) {

					uint32 shaderIndex = 0;
					for (auto& shader : sg.Shaders) {
						MaterialSystem::BufferIterator iterator;

						uint32 instanceIndex = 0;
						materialSystem->UpdateIteratorMember(iterator, sg.EntryHandle, shaderIndex + instanceIndex);

						for (uint32 bufferIndex = 0; bufferIndex < shader.Buffers.GetLength(); ++bufferIndex) {
							auto& buffer = shader.Buffers[bufferIndex];
							if (!buffer.Has) {
								if (materialSystem->DoesBufferExist(buffer.Buffer)) {
									auto address = materialSystem->GetBufferAddress(renderSystem, buffer.Buffer);
									materialSystem->WriteMultiBuffer(iterator, sg.BufferBufferReferencesMemberHandle, &address, bufferIndex);
									buffer.Has = true;
								}
							}
						}

						++shaderIndex;
					}
				}

				commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelineData.Pipeline, GAL::ShaderStages::RAY_GEN);

				GTSL::Array<CommandBuffer::ShaderTableDescriptor, 4> shaderTableDescriptors;

				for (uint8 i = 0; i < 4; ++i) {
					CommandBuffer::ShaderTableDescriptor shaderTableDescriptor;

					shaderTableDescriptor.Entries = pipelineData.ShaderGroups[i].Shaders.GetLength();
					shaderTableDescriptor.EntrySize = pipelineData.ShaderGroups[i].RoundedEntrySize;
					shaderTableDescriptor.Address = materialSystem->GetBufferAddress(renderSystem, pipelineData.ShaderGroups[i].Buffer);

					shaderTableDescriptors.EmplaceBack(shaderTableDescriptor);
				}

				commandBuffer.TraceRays(renderSystem->GetRenderDevice(), shaderTableDescriptors, data.RayTrace.DispatchSize);
				break;
			}

			case InternalLayerType::MATERIAL_AND_MESHES: {
				const auto& material = materials[data.Material.MaterialHandle.MaterialIndex];
				auto& materialInstance = material.MaterialInstances[data.Material.MaterialHandle.MaterialInstanceIndex];
				
				if (materialInstance.Counter == materialInstance.Target) {					
					//UpdateIndexStream(materialInstanceIndexStream, commandBuffer, renderSystem, materialSystem);

					for (auto& meshList : data.Material.MeshInstancesPerVertexGroup) {
						commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), material.VertexGroups[meshList.VertexGroupIndex].Pipeline, renderState.ShaderStages);
						
						for (auto& e : meshList.Meshes) {
							//set per mesh params
							//UpdateIndexStream(renderGroupIndexStream, commandBuffer, renderSystem, materialSystem, meshHandle.InstanceIndex);
							renderSystem->RenderMesh(e.Handle, e.InstanceCount);
						}
					}
				}				
				
				break;
			}
				
			case InternalLayerType::RENDER_PASS: {
				if(!data.RenderPass.Enabled) { break; }

				switch (data.RenderPass.PassType) {
				case PassType::RASTER: {
					for (auto& e : data.RenderPass.WriteAttachments) {
						updateImage(attachments.At(e.Name), e.Layout, data.RenderPass.PipelineStages, GAL::AccessTypes::WRITE);
					}

					for (auto& e : data.RenderPass.ReadAttachments) {
						updateImage(attachments.At(e.Name), e.Layout, data.RenderPass.PipelineStages, GAL::AccessTypes::READ);
					}
					
					if (renderState.ActiveRenderPass != data.RenderPass.Name) {
						renderState.ShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT;

						GTSL::Array<GAL::RenderPassTargetDescription, 8> renderPassTargetDescriptions;
						for (uint8 i = 0; i < data.RenderPass.WriteAttachments.GetLength(); ++i) {
							auto& e = renderPassTargetDescriptions.EmplaceBack();
							const auto& attachment = attachments.At(data.RenderPass.WriteAttachments[i].Name);
							e.ClearValue = attachment.ClearColor;
							e.Start = data.RenderPass.WriteAttachments[i].Layout;
							e.End = data.RenderPass.WriteAttachments[i].Layout;
							e.FormatDescriptor = attachment.FormatDescriptor;
							e.Texture = renderSystem->GetTexture(attachment.TextureHandle);
						}

						commandBuffer.BeginRenderPass(renderSystem->GetRenderDevice(), apiRenderPasses[data.RenderPass.APIRenderPass].RenderPass,
							getFrameBuffer(data.RenderPass.APIRenderPass), renderArea, renderPassTargetDescriptions);
					} else {
						commandBuffer.AdvanceSubPass(renderSystem->GetRenderDevice());
					}


					break;
				}
				case PassType::COMPUTE: {
					if (renderState.ActiveRenderPassType == PassType::RASTER) {
						commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
					}
					
					renderState.ShaderStages = GAL::ShaderStages::COMPUTE;
					transitionImages(commandBuffer, renderSystem, materialSystem, LayerHandle(node));
					break;
				}
				case PassType::RAY_TRACING: {
					if (renderState.ActiveRenderPassType == PassType::RASTER) {
						commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
					}
					
					renderState.ShaderStages = GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::CALLABLE;
					transitionImages(commandBuffer, renderSystem, materialSystem, LayerHandle(node));
					break;
				}
				}

				renderState.ActiveRenderPass = data.RenderPass.Name;
				renderState.ActiveRenderPassType = data.RenderPass.PassType;
				
				break;
			}

			case InternalLayerType::LAYER: {
				break;
			}
			}

			self(self, node->Nodes[n]);

			if (data.Name()) {
				PopData();
				if (data.Indexed) { PopIndexStream(indexStream); }
			}

			commandBuffer.EndRegion(renderSystem->GetRenderDevice());
		}
	};

	for(auto& e : renderingTree)
		visitLevel(visitLevel, e);

	GTSL::Array<CommandBuffer::BarrierData, 2> swapchainToCopyBarriers;
	swapchainToCopyBarriers.EmplaceBack(CommandBuffer::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::UNDEFINED,
		GAL::TextureLayout::TRANSFER_DESTINATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
		renderSystem->GetSwapchainFormat() });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), swapchainToCopyBarriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
		
	auto& attachment = attachments.At(resultAttachment);

	GTSL::Array<CommandBuffer::BarrierData, 2> fAttachmentoToCopyBarriers;
	fAttachmentoToCopyBarriers.EmplaceBack(CommandBuffer::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle), attachment.Layout,
		GAL::TextureLayout::TRANSFER_SOURCE, attachment.WriteAccess,
		GAL::AccessTypes::READ, attachment.FormatDescriptor });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), fAttachmentoToCopyBarriers, attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, GetTransientAllocator());

	updateImage(attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);
		
	commandBuffer.CopyTextureToTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachments.At(resultAttachment).TextureHandle),
	*renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_SOURCE, GAL::TextureLayout::TRANSFER_DESTINATION, 
		attachments.At(resultAttachment).FormatDescriptor, renderSystem->GetSwapchainFormat(),
		GTSL::Extent3D(renderSystem->GetRenderExtent()));

	
	GTSL::Array<CommandBuffer::BarrierData, 2> barriers;
	barriers.EmplaceBack(CommandBuffer::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION,
		GAL::TextureLayout::PRESENTATION, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, renderSystem->GetSwapchainFormat() });
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GetTransientAllocator());
}

//TODO: FIX ACCESS TO SYSTEMS HERE

void RenderOrchestrator::AddRenderManager(GameInstance* gameInstance, const Id renderManager, const SystemHandle systemReference)
{
	systems.EmplaceBack(renderManager);

	GTSL::Array<TaskDependency, 32> dependencies;
	{
		for (uint32 i = 0; i < systems.GetLength(); ++i) {
			auto& dependency = dependencies.EmplaceBack();
			dependency.AccessedObject = systems[i];
			dependency.Access = AccessTypes::READ;
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessTypes::READ);
	dependencies.EmplaceBack("MaterialSystem", AccessTypes::READ);

	{
		GTSL::Array<TaskDependency, 32> managerDependencies;

		managerDependencies.PushBack(dependencies);
		
		GTSL::Array<TaskDependency, 16> managerSetupDependencies;

		gameInstance->GetSystem<RenderManager>(systemReference)->GetSetupAccesses(managerSetupDependencies);

		managerDependencies.PushBack(managerSetupDependencies);
		
		setupSystemsAccesses.PushBack(managerDependencies);
	}

	if (renderingEnabled)
	{
		onRenderDisable(gameInstance);
		onRenderEnable(gameInstance, dependencies);
	}
	
	renderManagers.Emplace(renderManager, systemReference);
}

void RenderOrchestrator::RemoveRenderManager(GameInstance* gameInstance, const Id renderGroupName, const SystemHandle systemReference)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element.State())
	
	systems.Pop(element.Get());
	
	setupSystemsAccesses.Pop(element.Get());

	GTSL::Array<TaskDependency, 32> dependencies;

	for (uint32 i = 0; i < systems.GetLength(); ++i)
	{
		auto& dependency = dependencies.EmplaceBack();
		dependency.AccessedObject = systems[i];
		dependency.Access = AccessTypes::READ;
	}

	dependencies.EmplaceBack("RenderSystem", AccessTypes::READ);
	dependencies.EmplaceBack("MaterialSystem", AccessTypes::READ);

	if (renderingEnabled)
	{
		onRenderDisable(gameInstance);
		onRenderEnable(gameInstance, dependencies);
	}
}

MaterialInstanceHandle RenderOrchestrator::CreateMaterial(const CreateMaterialInfo& info)
{
	uint32 material_size = 0;
	info.MaterialResourceManager->GetMaterialSize(info.MaterialName, material_size);

	auto materialReference = materialsByName.TryEmplace(info.MaterialName);

	uint32 materialIndex = 0xFFFFFFFF, materialInstanceIndex = 0xFFFFFFFF;
	
	if(materialReference.State())
	{
		materialIndex = materials.Emplace();
		materialReference.Get() = materialIndex;
		
		GTSL::Buffer<BE::PAR> material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
		const auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "MaterialSystem", AccessTypes::READ_WRITE }, { "RenderOrchestrator", AccessTypes::READ_WRITE } };
		MaterialResourceManager::MaterialLoadInfo material_load_info;
		material_load_info.ActsOn = acts_on;
		material_load_info.GameInstance = info.GameInstance;
		material_load_info.Name = info.MaterialName;
		material_load_info.DataBuffer = GTSL::Range<byte*>(material_buffer.GetCapacity(), material_buffer.GetData());
		auto* matLoadInfo = GTSL::New<MaterialLoadInfo>(GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), materialIndex, 0, info.TextureResourceManager);
		material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, matLoadInfo);
		material_load_info.OnMaterialLoad = Task<MaterialResourceManager::OnMaterialLoadInfo>::Create<RenderOrchestrator, &RenderOrchestrator::onMaterialLoaded>(this);
		auto materialLoadInfo = info.MaterialResourceManager->LoadMaterial(material_load_info);

		auto index = materialLoadInfo.MaterialInstances.LookFor([&](const MaterialResourceManager::MaterialInstance& materialInstance)
		{
			return materialInstance.Name == info.InstanceName;
		});

		//TODO: ERROR CHECK
		
		auto& material = materials[materialIndex];

		for(const auto& e : materialLoadInfo.Permutations)
		{
			material.VertexGroups.EmplaceBack();
			auto& descriptor = material.VertexDescriptors.EmplaceBack();
			
			for (const auto& ve : e.VertexElements) {
				descriptor.EmplaceBack(ve.Type);
			}
		}

		material.MaterialInstances.Initialize(4, GetPersistentAllocator());

		for (auto& e : materialLoadInfo.MaterialInstances) {
			auto& materialInstance = material.MaterialInstances.EmplaceBack();
			materialInstance.Name = e.Name;
		}

		materialInstanceIndex = index.Get();
	}
	else
	{
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
	attachment.Uses = 0;
	
	GAL::FormatDescriptor formatDescriptor;

	attachment.Uses |= GAL::TextureUses::ATTACHMENT;
	
	if (type == GAL::TextureType::COLOR) {		
		formatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::COLOR, 0, 1, 2, 3);
		attachment.Uses |= GAL::TextureUses::STORAGE;
		attachment.Uses |= GAL::TextureUses::TRANSFER_SOURCE;
	} else {
		formatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::DEPTH, 0, 0, 0, 0);
	}
	
	attachment.FormatDescriptor = formatDescriptor;

	attachment.Uses |= GAL::TextureUses::SAMPLE;

	attachment.ClearColor = clearColor;
	attachment.Layout = GAL::TextureLayout::UNDEFINED;
	attachment.WriteAccess = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;

	attachments.Emplace(name, attachment);
}

void RenderOrchestrator::AddPass(Id name, Id parent, RenderSystem* renderSystem, MaterialSystem* materialSystem, PassData passData)
{
	Layer::RenderPassData* renderPass;

	{
		auto searchResult = renderingTreeMap.TryGet(name); LayerHandle layerHandle;

		if (searchResult.State()) {
			layerHandle = searchResult.Get();
			renderPass = &searchResult.Get()->Data.RenderPass;
		}
		else {
			layerHandle = AddLayer(name, parent, LayerType::RENDER_PASS);
			renderPass = &layerHandle->Data.RenderPass;
		}
		
		renderPasses.EmplaceBack(layerHandle);
	}
	
	GTSL::StaticMap<Id, uint32, 16> attachmentReadsPerPass;

	resultAttachment = passData.ResultAttachment;
	
	attachmentReadsPerPass.At(resultAttachment) = 0xFFFFFFFF; //set result attachment last read as "infinte" so it will always be stored

	{
		auto& finalAttachment = attachments.At(resultAttachment);
		finalAttachment.FormatDescriptor = GAL::FORMATS::BGRA_I8;
	}

	renderPass->Name = name;
	
	switch (passData.PassType) {
	case PassType::RASTER: {
		//for(auto e : renderPasses) {
		//	if(e->Data.RenderPass.PassType == PassType::RASTER) {
		//		
		//	}
		//}
		
		//uint32 contiguousRasterPassCount = passIndex;
		//while (contiguousRasterPassCount < passesData.ElementCount() && passesData[contiguousRasterPassCount].PassType == PassType::RASTER) {
		//	++contiguousRasterPassCount;
		//}
		//	
		//uint32 lastContiguousRasterPassIndex = contiguousRasterPassCount - 1;
		
		auto& apiRenderPassData = apiRenderPasses.EmplaceBack();
			
		if constexpr (_DEBUG) {
			auto name = GTSL::StaticString<32>("RenderPass");
			//renderPassCreateInfo.Name = name;
		}

		GTSL::Array<Id, 16> renderPassUsedAttachments; GTSL::Array<GTSL::Array<Id, 16>, 16> usedAttachmentsPerSubPass;

		usedAttachmentsPerSubPass.EmplaceBack();
		
		for (auto& ra : passData.ReadAttachments) {
			if (!renderPassUsedAttachments.Find(ra.Name).State()) { renderPassUsedAttachments.EmplaceBack(ra.Name); }
			if (!usedAttachmentsPerSubPass[0].Find(ra.Name).State()) { usedAttachmentsPerSubPass[0].EmplaceBack(ra.Name); }
		}
		
		for (auto& wa : passData.WriteAttachments) {
			if (!renderPassUsedAttachments.Find(wa.Name).State()) { renderPassUsedAttachments.EmplaceBack(wa.Name); }
			if (!usedAttachmentsPerSubPass[0].Find(wa.Name).State()) { usedAttachmentsPerSubPass[0].EmplaceBack(wa.Name); }
		}

		GTSL::Array<GAL::RenderPassTargetDescription, 16> attachmentDescriptors;

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

		GTSL::Array<RenderPass::SubPassDescriptor, 8> subPassDescriptors;
		GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> readAttachmentReferences;
		GTSL::Array<GTSL::Array<RenderPass::AttachmentReference, 8>, 8> writeAttachmentReferences;
		GTSL::Array<GTSL::Array<uint8, 8>, 8> preserveAttachmentReferences;

		GAL::AccessType sourceAccessFlags = GAL::AccessTypes::READ, destinationAccessFlags = GAL::AccessTypes::READ;
		GAL::PipelineStage sourcePipelineStages = GAL::PipelineStages::TOP_OF_PIPE, destinationPipelineStages = GAL::PipelineStages::TOP_OF_PIPE;
			
		subPasses.EmplaceBack();

		for (uint32 s = 0; s < 1/*contiguousRasterPassCount*/; ++s) {
			readAttachmentReferences.EmplaceBack();
			writeAttachmentReferences.EmplaceBack();
			preserveAttachmentReferences.EmplaceBack();
			
			//auto& renderPass = renderPassesMap.Emplace(passesData[passIndex].Name);
			//renderPasses.EmplaceBack(passesData[passIndex].Name);
			renderPass->APIRenderPass = apiRenderPasses.GetLength() - 1;

			renderPass->PassType = PassType::RASTER;
			renderPass->PipelineStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;

			RenderPass::SubPassDescriptor subPassDescriptor;

			auto getAttachmentIndex = [&](const Id name) {
				auto res = renderPassUsedAttachments.Find(name); return res.State() ? res.Get() : GAL::ATTACHMENT_UNUSED;
			};
			
			for (const auto& e : passData.ReadAttachments) {
				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Index = getAttachmentIndex(e.Name);
				attachmentReference.Layout = GAL::TextureLayout::SHADER_READ;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationAccessFlags = GAL::AccessTypes::READ;
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
					destinationAccessFlags = GAL::AccessTypes::READ;
					destinationPipelineStages |= GAL::PipelineStages::EARLY_FRAGMENT_TESTS | GAL::PipelineStages::LATE_FRAGMENT_TESTS;
				}
				
				auto& attachmentData = renderPass->ReadAttachments.EmplaceBack();
				attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::SHADER_READ; attachmentData.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
				readAttachmentReferences[s].EmplaceBack(attachmentReference);
			}

			subPassDescriptor.ReadAttachments = readAttachmentReferences[s];

			for (const auto& e : passData.WriteAttachments) {
				RenderPass::AttachmentReference attachmentReference;
				attachmentReference.Layout = GAL::TextureLayout::ATTACHMENT;
				attachmentReference.Index = getAttachmentIndex(e.Name);

				writeAttachmentReferences[s].EmplaceBack(attachmentReference);
				auto& attachmentData = renderPass->WriteAttachments.EmplaceBack();
				attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::ATTACHMENT; attachmentData.ConsumingStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				
				if (attachments.At(e.Name).FormatDescriptor.Type == GAL::TextureType::COLOR) {
					destinationAccessFlags = GAL::AccessTypes::WRITE;
					destinationPipelineStages |= GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
				} else {
					destinationAccessFlags = GAL::AccessTypes::WRITE;
					destinationPipelineStages |= GAL::PipelineStages::EARLY_FRAGMENT_TESTS | GAL::PipelineStages::LATE_FRAGMENT_TESTS;
				}
			}

			subPassDescriptor.WriteAttachments = writeAttachmentReferences[s];

			for (auto b : renderPassUsedAttachments) {
				if (!usedAttachmentsPerSubPass[s].Find(b).State()) { // If attachment is not used this sub pass
					if (attachmentReadsPerPass.At(b) > s) // And attachment is read after this pass
						preserveAttachmentReferences[s].EmplaceBack(getAttachmentIndex(b));
				}
			}
			
			subPassDescriptor.PreserveAttachments = preserveAttachmentReferences[s];

			subPassDescriptors.EmplaceBack(subPassDescriptor);

			subPasses.back().EmplaceBack();
			auto& newSubPass = subPasses.back().back();
			newSubPass.Name = name;

			sourceAccessFlags = destinationAccessFlags;
			sourcePipelineStages = destinationPipelineStages;
		}

		GTSL::Array<RenderPass::SubPassDependency, 16> subPassDependencies;

		for (uint8 i = 0; i < subPasses.back().GetLength() / 2; ++i) {
			RenderPass::SubPassDependency e;
			e.SourcePipelineStage = sourcePipelineStages;
			e.DestinationPipelineStage = destinationPipelineStages;
			
			e.SourceSubPass = i;
			e.DestinationSubPass = i + 1;
			e.SourceAccessType = sourceAccessFlags;
			e.DestinationAccessType = destinationAccessFlags;

			subPassDependencies.EmplaceBack(e);
		}

		apiRenderPassData.UsedAttachments = renderPassUsedAttachments;
			
		apiRenderPassData.RenderPass.Initialize(renderSystem->GetRenderDevice(), attachmentDescriptors, subPassDescriptors, subPassDependencies);

		break;
	}
	case PassType::COMPUTE: {
		renderPass->PassType = PassType::COMPUTE;
		renderPass->PipelineStages = GAL::PipelineStages::COMPUTE_SHADER;

		for (auto& e : passData.WriteAttachments) {
			auto& attachmentData = renderPass->WriteAttachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::GENERAL;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE_SHADER;
		}

		for (auto& e : passData.ReadAttachments) {
			auto& attachmentData = renderPass->ReadAttachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE_SHADER;
		}

		break;
	}
	case PassType::RAY_TRACING: {
		renderPass->PassType = PassType::RAY_TRACING;
		renderPass->PipelineStages = GAL::PipelineStages::RAY_TRACING_SHADER;

		for (auto& e : passData.WriteAttachments) {
			auto& attachmentData = renderPass->WriteAttachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::GENERAL;
			attachmentData.ConsumingStages = GAL::PipelineStages::RAY_TRACING_SHADER;
		}

		for (auto& e : passData.ReadAttachments) {
			auto& attachmentData = renderPass->ReadAttachments.EmplaceBack();
			attachmentData.Name = e.Name;
			attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			attachmentData.ConsumingStages = GAL::PipelineStages::RAY_TRACING_SHADER;
		}

		break;
	}
	}
	
	//for (uint8 rp = 0; rp < renderPasses.GetLength(); ++rp) {
	//	auto& renderPass = renderPassesMap.At(renderPasses[rp]);
	//	
	//	//GTSL::Array<MaterialSystem::SubSetInfo, 8> subSets;
	//	//subSets.EmplaceBack(MaterialSystem::SubSetInfo{ MaterialSystem::SubSetType::READ_TEXTURES, &renderPass.ReadAttachmentsHandle, 16 });
	//	//subSets.EmplaceBack(MaterialSystem::SubSetInfo{ MaterialSystem::SubSetType::WRITE_TEXTURES, &renderPass.WriteAttachmentsHandle, 16 });
	//	//renderPass.AttachmentsSetHandle = materialSystem->AddSet(renderSystem, renderPasses[rp], "RenderPasses", subSets);
	//
	//	GTSL::Array<MaterialSystem::MemberInfo, 2> members;
	//
	//	MaterialSystem::MemberInfo memberInfo;
	//	memberInfo.Handle = &renderPass.AttachmentsIndicesHandle;
	//	memberInfo.Type = MaterialSystem::Member::DataType::UINT32;
	//	memberInfo.Count = 16; //TODO: MAKE NUMBER OF MEMBERS
	//	members.EmplaceBack(memberInfo);
	//	
	//	renderPass.BufferHandle = materialSystem->CreateBuffer(renderSystem, members);
	//}
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, MaterialSystem* materialSystem, const GTSL::Extent2D newSize)
{
	//pendingDeleteFrames = renderSystem->GetPipelinedFrames();
	
	auto resize = [&](Attachment& attachment) -> void {
		if(attachment.TextureHandle) {
			attachment.TextureHandle = renderSystem->CreateTexture(attachment.FormatDescriptor, newSize, attachment.Uses, false);
		} else {
			attachment.TextureHandle = renderSystem->CreateTexture(attachment.FormatDescriptor, newSize, attachment.Uses, false);
			attachment.ImageIndex = imageIndex++;
		}

		if(attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {
			materialSystem->WriteSetTexture(renderSystem, imagesSubsetHandle, attachment.TextureHandle, attachment.ImageIndex);
		}
	};

	GTSL::ForEach(attachments, resize);

	for (auto& apiRenderPassData : apiRenderPasses)
	{
		if (apiRenderPassData.FrameBuffer.GetHandle()) {
			apiRenderPassData.FrameBuffer.Destroy(renderSystem->GetRenderDevice());
		}

		GTSL::Array<TextureView, 16> textureViews;
		for (auto e : apiRenderPassData.UsedAttachments) { textureViews.EmplaceBack(renderSystem->GetTextureView(attachments.At(e).TextureHandle)); }

		apiRenderPassData.FrameBuffer.Initialize(renderSystem->GetRenderDevice(), apiRenderPassData.RenderPass, newSize, textureViews);
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

void RenderOrchestrator::ToggleRenderPass(Id renderPassName, bool enable)
{
	auto renderPassSearch = GetLayer(renderPassName);

	if (renderPassSearch) {

		auto& renderPass = renderPassSearch->RenderPass;
		
		switch (renderPass.PassType) {
		case PassType::RASTER: break;
		case PassType::COMPUTE: break;
		case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetOption("rayTracing"); break; // Enable render pass only if function is enaled in settings
		default: break;
		}

		renderPass.Enabled = enable;
	} else {
		BE_LOG_WARNING("Tried to ", enable ? "enable" : "disable", " render pass ", renderPassName.GetString(), " which does not exist.");
	}
}

void RenderOrchestrator::UpdateIndexStream(IndexStreamHandle indexStreamHandle, CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, uint32 value)
{
	renderState.IndexStreams[indexStreamHandle()] = value;

	materialSystem->PushConstant(renderSystem, commandBuffer, renderState.PipelineLayout, 64ull + (indexStreamHandle() * 4),
		GTSL::Range<const byte*>(4, reinterpret_cast<const byte*>(&renderState.IndexStreams[indexStreamHandle()])));
}

void RenderOrchestrator::BindData(const RenderSystem* renderSystem, const MaterialSystem* materialSystem, CommandBuffer commandBuffer, Buffer buffer)
{
	GAL::DeviceAddress bufferAddress = 0;

	if (buffer.GetVkBuffer()) {
		bufferAddress = buffer.GetAddress(renderSystem->GetRenderDevice());
	}
	
	RenderSystem::BufferAddress dbufferAddress(bufferAddress);

	renderState.PipelineLayout = "GlobalData";

	materialSystem->PushConstant(renderSystem, commandBuffer, renderState.PipelineLayout, renderState.Offset,
		GTSL::Range<const byte*>(4, reinterpret_cast<const byte*>(&dbufferAddress)));

	renderState.Offset += 4;
}

void RenderOrchestrator::onRenderEnable(GameInstance* gameInstance, const GTSL::Range<const TaskDependency*> dependencies)
{
	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
}

void RenderOrchestrator::onRenderDisable(GameInstance* gameInstance)
{
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderDo");
}

void RenderOrchestrator::OnRenderEnable(TaskInfo taskInfo, bool oldFocus)
{
	//if (!oldFocus)
	//{
	//	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	//
	//	for (uint32 i = 0; i < dependencies.GetLength(); ++i)
	//	{
	//		dependencies[i].AccessedObject = systems[i];
	//		dependencies[i].Access = AccessTypes::READ;
	//	}
	//
	//	dependencies.EmplaceBack("RenderSystem", AccessTypes::READ);
	//	dependencies.EmplaceBack("MaterialSystem", AccessTypes::READ);
	//
	//	onRenderEnable(taskInfo.GameInstance, dependencies);
	//	BE_LOG_SUCCESS("Enabled rendering")
	//}

	renderingEnabled = true;
}

void RenderOrchestrator::OnRenderDisable(TaskInfo taskInfo, bool oldFocus)
{
	renderingEnabled = false;
}

void RenderOrchestrator::renderUI(GameInstance* gameInstance, RenderSystem* renderSystem, MaterialSystem* materialSystem, CommandBuffer commandBuffer, Id rp)
{
	auto* uiRenderManager = gameInstance->GetSystem<UIRenderManager>("UIRenderManager");

	//materialSystem->BindSet(renderSystem, commandBuffer, Id("UIRenderGroup"));

	auto* uiSystem = gameInstance->GetSystem<UIManager>("UIManager");
	auto* canvasSystem = gameInstance->GetSystem<CanvasSystem>("CanvasSystem");

	auto canvases = uiSystem->GetCanvases();
	auto renderGroupIndexStream = AddIndexStream();
	BindData(renderSystem, materialSystem, commandBuffer, materialSystem->GetBuffer("UIRenderGroup"));

	for (auto& ref : canvases) {
		auto& canvas = canvasSystem->GetCanvas(ref);
		auto canvasSize = canvas.GetExtent();

		auto& organizers = canvas.GetOrganizersTree();

		auto primitives = canvas.GetPrimitives();
		auto squares = canvas.GetSquares();

		const auto* parentOrganizer = organizers[0];

		commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), materials[uiRenderManager->GetUIMaterial().MaterialIndex].VertexGroups[0].Pipeline, GAL::ShaderStages::VERTEX);

		for (auto& e : squares) {
			//UpdateIndexStream(renderGroupIndexStream, commandBuffer, renderSystem, materialSystem);
			renderSystem->RenderMesh(uiRenderManager->GetSquareMesh());
		}
	}
	
	PopData();
	PopIndexStream(renderGroupIndexStream);
}

void RenderOrchestrator::transitionImages(CommandBuffer commandBuffer, RenderSystem* renderSystem, MaterialSystem* materialSystem, const LayerHandle renderPassLayerHandle)
{
	GTSL::Array<CommandBuffer::BarrierData, 16> barriers;
	
	auto& renderPass = GetLayer(renderPassLayerHandle)->RenderPass;

	GAL::PipelineStage initialStage;
	
	auto buildTextureBarrier = [&](const Layer::AttachmentData& attachmentData, GAL::PipelineStage attachmentStages, GAL::AccessType access)
	{
		auto& attachment = attachments.At(attachmentData.Name);

		CommandBuffer::TextureBarrier textureBarrier;
		textureBarrier.Texture = renderSystem->GetTexture(attachment.TextureHandle);
		textureBarrier.CurrentLayout = attachment.Layout;
		textureBarrier.Format = attachment.FormatDescriptor;
		textureBarrier.TargetLayout = attachmentData.Layout;
		textureBarrier.SourceAccess = attachment.WriteAccess;
		textureBarrier.DestinationAccess = access;
		barriers.EmplaceBack(textureBarrier);

		initialStage |= attachment.ConsumingStages;
		
		updateImage(attachment, attachmentData.Layout, renderPass.PipelineStages, access);
	};
	
	for (auto& e : renderPass.ReadAttachments) { buildTextureBarrier(e, e.ConsumingStages, GAL::AccessTypes::READ); }
	for (auto& e : renderPass.WriteAttachments) { buildTextureBarrier(e, e.ConsumingStages, GAL::AccessTypes::WRITE); }
	
	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, initialStage, renderPass.PipelineStages, GetTransientAllocator());
}

void RenderOrchestrator::onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager* materialResourceManager,
	GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo)
{
	uint32 totalSize = 0;

	for (auto e : shaderInfos) { totalSize += e.Size; }

	shaderLoadInfo.Buffer.Allocate(totalSize, 8, GetPersistentAllocator());

	materialResourceManager->LoadShaders(taskInfo.GameInstance, shaderInfos, onShadersLoadHandle, shaderLoadInfo.Buffer.GetRange(), GTSL::MoveRef(shaderLoadInfo));
}

void RenderOrchestrator::onShadersLoaded(TaskInfo taskInfo, MaterialResourceManager*,
                                         GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer,
                                         ShaderLoadInfo shaderLoadInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	shaderLoadInfo.Component;

	Pipeline pipeline;
	//ComputePipeline::CreateInfo createInfo;
	//createInfo.RenderDevice = renderSystem->GetRenderDevice();
	//createInfo.PipelineLayout;
	//createInfo.ShaderInfo.Blob = GTSL::Range<const byte*>(shaders[0].Size, shaderLoadInfo.Buffer.GetData());
	//createInfo.ShaderInfo.Type = ShaderType::COMPUTE;
	//pipeline.Initialize(createInfo);
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo)
{
	auto component = textureIndex++;

	pendingMaterialsPerTexture.EmplaceAt(component, GetPersistentAllocator());
	pendingMaterialsPerTexture[component].Initialize(4, GetPersistentAllocator());

	texturesRefTable.Emplace(createTextureInfo.TextureName, component);

	auto textureLoadInfo = TextureLoadInfo(component, createTextureInfo.RenderSystem, RenderAllocation());

	createTextureInfo.TextureResourceManager->LoadTextureInfo(createTextureInfo.GameInstance, createTextureInfo.TextureName, onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));

	return component;
}

void RenderOrchestrator::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{
	auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem"); auto* renderSystem = loadInfo->RenderSystem;

	auto materialIndex = loadInfo->Component; auto& materialData = materials[materialIndex];

	materialData.Name = onMaterialLoadInfo.ResourceName;
	materialData.RenderGroup = onMaterialLoadInfo.RenderGroup; materialData.Parameters = onMaterialLoadInfo.Parameters;

	{
		GTSL::Array<MaterialSystem::MemberInfo, 16> materialParameters;

		for (auto& e : materialData.Parameters) {
			materialData.ParametersHandles.Emplace(e.Name);

			MaterialSystem::Member::DataType memberType;

			switch (e.Type)
			{
			case MaterialResourceManager::ParameterType::UINT32: memberType = MaterialSystem::Member::DataType::UINT32; break;
			case MaterialResourceManager::ParameterType::FVEC4: memberType = MaterialSystem::Member::DataType::FVEC4; break;
			case MaterialResourceManager::ParameterType::TEXTURE_REFERENCE: memberType = MaterialSystem::Member::DataType::UINT32; break;
			case MaterialResourceManager::ParameterType::BUFFER_REFERENCE: memberType = MaterialSystem::Member::DataType::UINT64; break;
			}

			materialParameters.EmplaceBack(&materialData.ParametersHandles.At(e.Name), 1);
		}

		materialData.BufferHandle = materialSystem->CreateBuffer(renderSystem, MaterialSystem::MemberInfo(&materialData.MaterialInstancesMemberHandle, 8, materialParameters));
		materialSystem->BindBufferToName(materialData.BufferHandle, Id(onMaterialLoadInfo.ResourceName));
	}

	GTSL::Array<GAL::VulkanShader, 16> shaders;

	{
		uint32 offset = 0;
		
		for (auto& e : onMaterialLoadInfo.Shaders) {
			auto& shader = shaders.EmplaceBack();
			shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range<const byte*>(e.Size, loadInfo->Buffer.GetData() + offset));
			offset += e.Size;
		}
	}

	//if constexpr (_DEBUG) {
	//	GTSL::StaticString<64> name("Raster pipeline. Material: "); name += onMaterialLoadInfo.ResourceName();
	//	createInfo.Name = name;
	//}

	GTSL::Array<GAL::Pipeline::PipelineStateBlock, 16> pipelineStates;
	GTSL::Array<GAL::Pipeline::PipelineStateBlock::RenderContext::AttachmentState, 8> att;
	
	{
		GAL::Pipeline::PipelineStateBlock::RenderContext context;

		for (const auto & writeAttachment : GetLayer(onMaterialLoadInfo.RenderPass)->RenderPass.WriteAttachments) {
			auto& attachment = attachments.At(writeAttachment.Name);
			auto& attachmentState = att.EmplaceBack();
			attachmentState.BlendEnable = false; attachmentState.FormatDescriptor = attachment.FormatDescriptor;
		}
		
		context.Attachments = att;
		context.RenderPass = static_cast<const GAL::RenderPass*>(getAPIRenderPass(onMaterialLoadInfo.RenderPass));
		context.SubPassIndex = getAPISubPassIndex(onMaterialLoadInfo.RenderPass);
		pipelineStates.EmplaceBack(context);
	}

	{
		GAL::Pipeline::PipelineStateBlock::DepthState depth;
		depth.CompareOperation = GAL::CompareOperation::LESS;
		pipelineStates.EmplaceBack(depth);
	}

	{
		GAL::Pipeline::PipelineStateBlock::RasterState rasterState;
		rasterState.CullMode = GAL::CullMode::CULL_BACK;
		rasterState.WindingOrder = GAL::WindingOrder::CLOCKWISE;
		pipelineStates.EmplaceBack(rasterState);
	}

	{
		GAL::Pipeline::PipelineStateBlock::ViewportState viewportState;
		viewportState.ViewportCount = 1;
		pipelineStates.EmplaceBack(viewportState);
	}

	GTSL::Array<Pipeline::ShaderInfo, 8> shaderInfos;

	for (uint32 i = 0, offset = 0; i < onMaterialLoadInfo.Shaders.GetLength(); ++i) {
		auto& shaderInfo = shaderInfos.EmplaceBack();
		shaderInfo.Type = onMaterialLoadInfo.Shaders[i].Type;
		shaderInfo.Shader = shaders[i];
		shaderInfo.Blob = GTSL::Range<const byte*>(onMaterialLoadInfo.Shaders[i].Size, loadInfo->Buffer.GetData() + offset);

		offset += onMaterialLoadInfo.Shaders[i].Size;
	}

	auto& vertexState = pipelineStates.EmplaceBack(GAL::Pipeline::PipelineStateBlock::VertexState{});
	
	for (uint8 permutationIndex = 0; permutationIndex < onMaterialLoadInfo.Permutations.GetLength(); ++permutationIndex)
	{
		const auto& permutationInfo = onMaterialLoadInfo.Permutations[permutationIndex];
		auto& permutationData = materialData.VertexGroups[permutationIndex];

		GTSL::Array<Pipeline::VertexElement, 10> vertexDescriptor;

		vertexDescriptor.PushBack({ GAL::Pipeline::POSITION, 0, false, GAL::ShaderDataType::FLOAT3 });
		vertexDescriptor.PushBack({ GAL::Pipeline::NORMAL, 0, false, GAL::ShaderDataType::FLOAT3 });
		vertexDescriptor.PushBack({ GAL::Pipeline::TANGENT, 0, false, GAL::ShaderDataType::FLOAT3 });
		vertexDescriptor.PushBack({ GAL::Pipeline::BITANGENT, 0, false, GAL::ShaderDataType::FLOAT3 });
		vertexDescriptor.PushBack({ GAL::Pipeline::TEXTURE_COORDINATES, 0, false, GAL::ShaderDataType::FLOAT2 });

		for (auto& e : permutationInfo.VertexElements) {

			auto vertexElement = vertexDescriptor.LookFor([&](const Pipeline::VertexElement& vertexElement)
				{
					return vertexElement.Index == e.Index && vertexElement.Identifier == e.VertexAttribute;
				});

			vertexDescriptor[vertexElement.Get()].Enabled = vertexElement.State();
		}

		vertexState.Vertex.VertexDescriptor = vertexDescriptor;
		
		permutationData.Pipeline.InitializeRasterPipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, materialSystem->GetSetLayoutPipelineLayout(Id("GlobalData")), renderSystem->GetPipelineCache());
	}
	
	for (uint8 materialInstanceIndex = 0; materialInstanceIndex < onMaterialLoadInfo.MaterialInstances.GetLength(); ++materialInstanceIndex)
	{
		const auto& materialInstanceInfo = onMaterialLoadInfo.MaterialInstances[materialInstanceIndex];
		auto& materialInstanceData = materialData.MaterialInstances[materialInstanceIndex];
		
		//UpdateObjectCount(renderSystem, material., material.InstanceCount); //assuming every material uses the same set instance, not index

		MaterialInstanceHandle materialInstanceHandle{ materialIndex, materialInstanceIndex };
		
		for (auto& resourceMaterialInstanceParameter : materialInstanceInfo.Parameters)
		{
			auto materialParameter = materialData.Parameters.LookFor([&](const MaterialResourceManager::Parameter& parameter) { return parameter.Name == resourceMaterialInstanceParameter.First; }); //get parameter description from name

			BE_ASSERT(materialParameter.State(), "No parameter by that name found. Data must be invalid");

			if (materialData.Parameters[materialParameter.Get()].Type == MaterialResourceManager::ParameterType::TEXTURE_REFERENCE) //if parameter is texture reference, load texture
			{
				uint32 textureComponentIndex;

				auto textureReference = texturesRefTable.TryGet(resourceMaterialInstanceParameter.Second.TextureReference);

				if (!textureReference.State()) {
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.GameInstance;
					createTextureInfo.TextureResourceManager = loadInfo->TextureResourceManager;
					createTextureInfo.TextureName = resourceMaterialInstanceParameter.Second.TextureReference;
					createTextureInfo.MaterialHandle = materialInstanceHandle;
					auto textureComponent = createTexture(createTextureInfo);

					addPendingMaterialToTexture(textureComponent, materialInstanceHandle);

					textureComponentIndex = textureComponent;
				} else {
					textureComponentIndex = textureReference.Get();
					++materialInstanceData.Counter; //since we up the target for every texture, up the counter for every already existing texture
				}

				++materialInstanceData.Target;

				MaterialSystem::BufferIterator bufferIterator;
				materialSystem->UpdateIteratorMember(bufferIterator, materialData.MaterialInstancesMemberHandle, materialInstanceIndex);
				materialSystem->WriteMultiBuffer(bufferIterator, materialData.ParametersHandles.At(resourceMaterialInstanceParameter.First), &textureComponentIndex);
			}
		}
	}
	
	GTSL::Delete(loadInfo, GetPersistentAllocator());
}

void RenderOrchestrator::onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	loadInfo.TextureHandle = loadInfo.RenderSystem->CreateTexture(textureInfo.Format, textureInfo.Extent, GAL::TextureUses::SAMPLE | GAL::TextureUses::ATTACHMENT, true);

	auto dataBuffer = loadInfo.RenderSystem->GetTextureRange(loadInfo.TextureHandle);

	resourceManager->LoadTexture(taskInfo.GameInstance, textureInfo, dataBuffer, onTextureLoadHandle, GTSL::MoveRef(loadInfo));
}

void RenderOrchestrator::onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	
	loadInfo.RenderSystem->UpdateTexture(loadInfo.TextureHandle);

	materialSystem->WriteSetTexture(loadInfo.RenderSystem, textureSubsetsHandle, loadInfo.TextureHandle, loadInfo.Component);
	
	latestLoadedTextures.EmplaceBack(loadInfo.Component);
}