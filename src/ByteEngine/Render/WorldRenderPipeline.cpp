#include "WorldRenderPipeline.hpp"

WorldRendererPipeline::WorldRendererPipeline(const InitializeInfo& initialize_info) : RenderPipeline(initialize_info, u8"WorldRendererPipeline"), spherePositionsAndRadius(16, GetPersistentAllocator()), instances(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator()), meshToInstanceMap(16, GetPersistentAllocator()), InstanceTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"Instance")) {
	auto* renderSystem = initialize_info.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = initialize_info.ApplicationManager->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	rayTracing = BE::Application::Get()->GetBoolOption(u8"rayTracing");

	onStaticMeshInfoLoadHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnStaticMeshInfoLoad", DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager", AccessTypes::READ_WRITE), TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator", AccessTypes::READ_WRITE)), &WorldRendererPipeline::onStaticMeshInfoLoaded);

	onStaticMeshLoadHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnStaticMeshLoad", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE), TypedDependency<StaticMeshSystem>(u8"StaticMeshSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::onStaticMeshLoaded);

	OnAddMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnAddMesh", DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"), TypedDependency<RenderSystem>(u8"RenderSystem")), &WorldRendererPipeline::OnAddMesh);

	OnAddRenderGroupMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnAddRenderGroupMesh", DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"), TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<StaticMeshSystem>(u8"StaticMeshSystem")), &WorldRendererPipeline::OnAddRenderGroupMesh);

	OnUpdateMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnUpdateMesh", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::OnUpdateMesh);

	OnUpdateRenderGroupMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnUpdateRenderGroupMesh", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::OnUpdateRenderGroupMesh);

	GetApplicationManager()->SetTaskReceiveOnlyLast(OnUpdateRenderGroupMeshTaskHandle);

	GetApplicationManager()->SubscribeToEvent(u8"SMGR", StaticMeshSystem::GetOnAddMeshEventHandle(), OnAddRenderGroupMeshTaskHandle);
	GetApplicationManager()->SubscribeToEvent(u8"SMGR", StaticMeshSystem::GetOnUpdateMeshEventHandle(), OnUpdateRenderGroupMeshTaskHandle);

	initialize_info.ApplicationManager->EnqueueScheduledTask(initialize_info.ApplicationManager->RegisterTask(this, u8"renderSetup", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::preRender, u8"RenderSetup", u8"Render"));

	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnAddPointLight"));
	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle, GTSL::Vector3>(u8"OnUpdatePointLight"));
	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnRemovePointLight"));

	GetApplicationManager()->AddTypeSetupDependency(this, InstanceTypeIndentifier, OnAddMeshTaskHandle, true);
	GetApplicationManager()->AddTypeSetupDependency(this, InstanceTypeIndentifier, OnUpdateMeshTaskHandle, false);

	GetApplicationManager()->AddTypeSetupDependency(this, GetApplicationManager()->GetSystem<StaticMeshSystem>(u8"StaticMeshSystem")->GetStaticMeshTypeIdentifier(), OnAddRenderGroupMeshTaskHandle, true);
	GetApplicationManager()->AddTypeSetupDependency(this, GetApplicationManager()->GetSystem<StaticMeshSystem>(u8"StaticMeshSystem")->GetStaticMeshTypeIdentifier(), OnUpdateRenderGroupMeshTaskHandle, false);

	auto addLightTaskHandle = GetApplicationManager()->RegisterTask(this, u8"addPointLight", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::onAddLight);
	initialize_info.ApplicationManager->SubscribeToEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnAddPointLight"), addLightTaskHandle);
	auto updateLightTaskHandle = GetApplicationManager()->RegisterTask(this, u8"updatePointLight", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), & WorldRendererPipeline::updateLight);
	initialize_info.ApplicationManager->SubscribeToEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle, GTSL::Vector3, GTSL::RGB, float32, float32>(u8"OnUpdatePointLight"), updateLightTaskHandle);

	sourceVertexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::VERTEX, true, {});
	destinationVertexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::VERTEX | GAL::BufferUses::BUILD_INPUT_READ, false, {});
	sourceIndexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::INDEX, true, {});
	destinationIndexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::INDEX | GAL::BufferUses::BUILD_INPUT_READ, false, {});

	RenderOrchestrator::NodeHandle renderPassNodeHandle;

	renderOrchestrator->AddNotifyShaderGroupCreated(GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)>::Create<WorldRendererPipeline, &WorldRendererPipeline::onAddShaderGroup>(this));

	if (renderTechniqueName == GTSL::ShortString<16>(u8"Forward")) {
		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.Attachments = RenderPassStructToAttachments(FORWARD_RENDERPASS_DATA);
		renderPassNodeHandle = renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"Geometry Pass", u8"ForwardRenderPass", renderSystem, geoRenderPass);
	}
	else if (renderTechniqueName == GTSL::ShortString<16>(u8"Visibility")) {
		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.Attachments.EmplaceBack(GTSL::StringView(u8"Visibility"),GTSL::StringView(u8"Visibility"), GAL::AccessTypes::WRITE);
		geoRenderPass.Attachments.EmplaceBack(GTSL::StringView(u8"Depth"),GTSL::StringView(u8"Depth"), GAL::AccessTypes::WRITE);
		renderPassNodeHandle = renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"Geometry Pass", u8"VisibilityRenderPass", renderSystem, geoRenderPass);

		GTSL::StaticVector<RenderOrchestrator::MemberInfo, 16> members;
		members.EmplaceBack(nullptr, u8"ptr_t", u8"positionStream");
		members.EmplaceBack(nullptr, u8"ptr_t", u8"normalStream");
		members.EmplaceBack(nullptr, u8"ptr_t", u8"tangentStream");
		members.EmplaceBack(nullptr, u8"ptr_t", u8"bitangentStream");
		members.EmplaceBack(nullptr, u8"ptr_t", u8"textureCoordinatesStream");
		members.EmplaceBack(nullptr, u8"uint32", u8"shaderGroupLength");
		members.EmplaceBack(nullptr, u8"uint32[256]", u8"shaderGroupUseCount");
		members.EmplaceBack(nullptr, u8"uint32[256]", u8"shaderGroupStart");
		members.EmplaceBack(nullptr, u8"IndirectDispatchCommand[256]", u8"indirectBuffer");
		members.EmplaceBack(nullptr, u8"ptr_t", u8"pixelBuffer");
		renderOrchestrator->RegisterType(u8"global", u8"VisibilityData", members);

		visibilityDataKey = renderOrchestrator->MakeDataKey(renderSystem, u8"global", u8"VisibilityData");
		renderPassNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"VisibilityDataLightingDataNode", visibilityDataKey);

		//pixelXY stores blocks per material that determine which pixels need to be painted with each material
		auto pielBuffer = renderOrchestrator->MakeDataKey(renderSystem, u8"global", u8"vec2s[2073600]"); //1920 * 1080

		{
			auto bwk = renderOrchestrator->GetBufferWriteKey(renderSystem, visibilityDataKey);

			const auto vertexElementsThatFitInBuffer = ((1024 * 1024 * 4) / 56u);

			auto bufferAddress = renderSystem->GetBufferAddress(destinationVertexBuffer);

			bwk[u8"positionStream"] = bufferAddress;
			bwk[u8"normalStream"] = bufferAddress + 12 * 1 * vertexElementsThatFitInBuffer; //todo: if buffer is updatable only address for current frame will be set
			bwk[u8"tangentStream"] = bufferAddress + 12 * 2 * vertexElementsThatFitInBuffer;
			bwk[u8"bitangentStream"] = bufferAddress + 12 * 3 * vertexElementsThatFitInBuffer;
			bwk[u8"textureCoordinatesStream"] = bufferAddress + 12 * 4 * vertexElementsThatFitInBuffer;
			bwk[u8"shaderGroupLength"] = 0u;
			bwk[u8"pixelBuffer"] = pielBuffer;
		}

		//Counts how many pixels each shader group uses
		RenderOrchestrator::PassData countPixelsRenderPassData;
		countPixelsRenderPassData.PassType = RenderOrchestrator::PassType::COMPUTE;
		countPixelsRenderPassData.Attachments.EmplaceBack(GTSL::StringView(u8"Visibility"), GTSL::StringView(u8"Visibility"), GAL::AccessTypes::READ);
		renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"CountPixels", u8"CountPixels", renderSystem, countPixelsRenderPassData);

		////Performs a prefix to build an indirect buffer defining which pixels each shader group occupies
		//RenderOrchestrator::PassData prefixSumRenderPassData;
		//prefixSumRenderPassData.PassType = RenderOrchestrator::PassType::COMPUTE;
		//renderOrchestrator->AddRenderPassNode(u8"PrefixSum", renderOrchestrator->GetCameraDataLayer(), renderSystem, prefixSumRenderPassData, GetApplicationManager());
		//
		////Scans the whole rendered image and stores which pixels every shader group occupies utilizing the information from the prefix sum pass
		//RenderOrchestrator::PassData selectPixelsRenderPass;
		//selectPixelsRenderPass.PassType = RenderOrchestrator::PassType::COMPUTE;
		//countPixelsRenderPassData.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Visibility" });
		//renderOrchestrator->AddRenderPassNode(u8"SelectPixels", renderOrchestrator->GetCameraDataLayer(), renderSystem, selectPixelsRenderPass, GetApplicationManager());
		//
		////Every participating shader group is called to paint every pixel it occupies on screen
		//RenderOrchestrator::PassData paintRenderPassData;
		//paintRenderPassData.PassType = RenderOrchestrator::PassType::RASTER;
		//paintRenderPassData.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Visibility" });
		//paintRenderPassData.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Color" });
		//renderOrchestrator->AddRenderPassNode(u8"PaintPixels", renderOrchestrator->GetCameraDataLayer(), renderSystem, paintRenderPassData, GetApplicationManager());

		//renderOrchestrator->SetShaderGroupParameter(renderSystem, ShaderGroupHandle{}, u8"materialCount", 0u);
	}

	renderOrchestrator->RegisterType(u8"global", u8"StaticMeshData", INSTANCE_DATA);
	meshDataBuffer = renderOrchestrator->MakeDataKey(renderSystem, u8"global", u8"StaticMeshData[8]", meshDataBuffer);

	renderOrchestrator->RegisterType(u8"global", u8"PointLightData", POINT_LIGHT_DATA);
	renderOrchestrator->RegisterType(u8"global", u8"LightingData", LIGHTING_DATA);

	renderPassNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"CameraData", renderOrchestrator->cameraDataKeyHandle);

	lightsDataKey = renderOrchestrator->MakeDataKey(renderSystem, u8"global", u8"LightingData");

	vertexBufferNodeHandle = renderOrchestrator->AddVertexBufferBind(renderSystem, renderPassNodeHandle, destinationVertexBuffer, { { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT2 } });
	indexBufferNodeHandle = renderOrchestrator->AddIndexBufferBind(vertexBufferNodeHandle, destinationIndexBuffer);
	meshDataNode = renderOrchestrator->AddDataNode(indexBufferNodeHandle, u8"MeshNode", meshDataBuffer, true);

	if (renderTechniqueName == GTSL::ShortString<16>(u8"Visibility")) {
		auto shaderGroupHandle = renderOrchestrator->CreateShaderGroup(u8"VisibilityShaderGroup");
		mainVisibilityPipelineNode = renderOrchestrator->AddMaterial(meshDataNode, shaderGroupHandle);
	}

	for (uint32 i = 0; i < renderSystem->GetPipelinedFrames(); ++i) {
		renderOrchestrator->buildCommandList[i] = renderSystem->CreateCommandList(u8"Acc. Struct. build", GAL::QueueTypes::COMPUTE, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD);
		renderOrchestrator->buildAccelerationStructuresWorkloadHandle[i] = renderSystem->CreateWorkload(u8"Build Acc. Structs.", GAL::QueueTypes::COMPUTE, GAL::PipelineStages::ACCELERATION_STRUCTURE_BUILD);
	}

	if (rayTracing) {
		topLevelAccelerationStructure = renderSystem->CreateTopLevelAccelerationStructure(16);

		setupDirectionShadowRenderPass(renderSystem, renderOrchestrator);
	}

	{
		renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"AO", u8"SSAO", renderSystem, { RenderPassStructToAttachments(AO_RENDERPASS_DATA), RenderOrchestrator::PassType::COMPUTE }, { { u8"Camera Data", renderOrchestrator->cameraDataKeyHandle } });
	}

	{
		auto l = {
			RenderOrchestrator::PassData::AttachmentReference{ GTSL::StringView(u8"AO"), GTSL::StringView(u8"AO"), GAL::AccessTypes::READ },
			RenderOrchestrator::PassData::AttachmentReference{ GTSL::StringView(u8"Mean"), GTSL::StringView(u8"Mean"), GAL::AccessTypes::WRITE },
			RenderOrchestrator::PassData::AttachmentReference{ GTSL::StringView(u8"Variance"), GTSL::StringView(u8"Variance"), GAL::AccessTypes::WRITE }
		};

		renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"Calculate AO Variance", u8"CalculateVariance", renderSystem, { l, RenderOrchestrator::PassType::COMPUTE }, { { u8"Camera Data", renderOrchestrator->cameraDataKeyHandle } });
	}

	{
		auto s = renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"Lighting", u8"Lighting", renderSystem, { RenderPassStructToAttachments(LIGHTING_RENDERPASS_DATA), RenderOrchestrator::PassType::COMPUTE }, { { u8"Camera Data", renderOrchestrator->cameraDataKeyHandle }, { u8"Lighting Data", lightsDataKey } });
	}

	{
		RenderOrchestrator::PassData gammaCorrectionPass;
		gammaCorrectionPass.PassType = RenderOrchestrator::PassType::COMPUTE;
		gammaCorrectionPass.Attachments.EmplaceBack(GTSL::StringView(u8"Lighting"), GTSL::StringView(u8"Lighting"), GAL::AccessTypes::WRITE); //result attachment
		auto gcrpnh = renderOrchestrator->AddRenderPassNode(renderOrchestrator->GetGlobalDataLayer(), u8"GammaCorrection", u8"GammaCorrection", renderSystem, gammaCorrectionPass);
	}
}

void WorldRendererPipeline::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
	auto& resource = resources[staticMeshInfo.GetName()];

	auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(), indicesSize = staticMeshInfo.GetIndexCount() * staticMeshInfo.GetIndexSize();

	resource.VertexSize = staticMeshInfo.GetVertexSize();
	resource.VertexCount = staticMeshInfo.VertexCount;
	resource.IndexCount = staticMeshInfo.IndexCount;
	resource.IndexType = GAL::SizeToIndexType(staticMeshInfo.IndexSize);
	resource.Interleaved = staticMeshInfo.Interleaved;

	resource.VertexComponentsInStream = vertexComponentsPerStream; resource.IndicesInStream = indicesInBuffer;

	for (uint32 i = 0; i < staticMeshInfo.GetSubMeshes().Length; ++i) {
		auto& sm = staticMeshInfo.GetSubMeshes().array[i];
		auto shaderGroupHandle = render_orchestrator->CreateShaderGroup(sm.ShaderGroupName);
		resource.RenderModelHandle = shaderGroupHandle;

		if (renderTechniqueName == u8"Forward") {
			RenderOrchestrator::NodeHandle materialNodeHandle = render_orchestrator->AddMaterial(meshDataNode, shaderGroupHandle);

			resource.nodeHandle = render_orchestrator->AddMesh(materialNodeHandle, resource.IndexCount, resource.IndexCount, indicesInBuffer, vertexComponentsPerStream);
		} else if (renderTechniqueName == u8"Visibility") {
			resource.nodeHandle = render_orchestrator->AddMesh(mainVisibilityPipelineNode, 0, resource.IndexCount, indicesInBuffer, vertexComponentsPerStream);

			//TODO: add to selection buffer
			//TODO: add pipeline bind to render pixels with this material

			//render_orchestrator->AddIndirectDispatchNode();
		}
	}

	//if unorm or snorm is used to specify data, take that into account as some properties (such as positions) may need scaling as XNORM enconding is defined in the 0->1 / -1->1 range
	bool usesxNorm = false;

	for (uint32 ai = 0; ai < staticMeshInfo.GetVertexDescriptor().Length; ++ai) {
		auto& t = resource.VertexElements.EmplaceBack();

		auto& a = staticMeshInfo.GetVertexDescriptor().array[ai];
		for (uint32 bi = 0; bi < a.Length; ++bi) {
			auto& b = a.array[bi];

			t.EmplaceBack(b);

			usesxNorm = IsAnyOf(b, GAL::ShaderDataType::U16_UNORM, GAL::ShaderDataType::U16_UNORM2, GAL::ShaderDataType::U16_UNORM3, GAL::ShaderDataType::U16_UNORM4, GAL::ShaderDataType::U16_SNORM, GAL::ShaderDataType::U16_SNORM2, GAL::ShaderDataType::U16_SNORM3, GAL::ShaderDataType::U16_SNORM4);
		}
	}

	if (usesxNorm) {
		//don't always assign bounding box as scaling factor, as even if we didn't need it bounding boxes usually have little errors which would cause the mesh to be scaled incorrectly
		//even though we have the correct coordinates to begin with
		resource.ScalingFactor = staticMeshInfo.GetBoundingBox();
	}

	staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, vertexComponentsPerStream, render_system->GetBufferRange(sourceVertexBuffer), indicesInBuffer, render_system->GetBufferRange(sourceIndexBuffer), onStaticMeshLoadHandle);

	vertexComponentsPerStream += staticMeshInfo.GetVertexCount();
	indicesInBuffer += staticMeshInfo.GetIndexCount();
}

void WorldRendererPipeline::onStaticMeshLoaded(TaskInfo taskInfo, RenderSystem* render_system, StaticMeshSystem* render_group, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
	auto& res = resources[staticMeshInfo.GetName()];

	auto commandListHandle = render_orchestrator->buildCommandList[render_system->GetCurrentFrame()];

	render_system->AddBufferUpdate(commandListHandle, sourceVertexBuffer, destinationVertexBuffer); render_system->AddBufferUpdate(commandListHandle, sourceIndexBuffer, destinationIndexBuffer);
	render_orchestrator->AddVertices(vertexBufferNodeHandle, staticMeshInfo.GetVertexCount());
	render_orchestrator->AddIndices(indexBufferNodeHandle, staticMeshInfo.GetIndexCount());

	if (rayTracing) {
		res.BLAS = render_system->CreateBottomLevelAccelerationStructure(staticMeshInfo.VertexCount, 12/*todo: use actual position stride*/, staticMeshInfo.IndexCount, GAL::SizeToIndexType(staticMeshInfo.IndexSize), destinationVertexBuffer, destinationIndexBuffer, res.VertexComponentsInStream * 12/*todo: use actual position coordinate element size*/, res.IndicesInStream * 2);
		pendingBlasUpdates.EmplaceBack(res.BLAS);
	}

	for (auto e : res.Instances) {
		AddMeshInstance(Id(staticMeshInfo.GetName()), e);
		*spherePositionsAndRadius.GetPointer<3>(e()) = staticMeshInfo.BoundingRadius;
	}

	res.Loaded = true;

	GTSL::StaticVector<GTSL::Range<const GAL::ShaderDataType*>, 8> r;

	for (auto& e : res.VertexElements) {
		r.EmplaceBack(e.GetRange());
	}
}

void WorldRendererPipeline::OnAddRenderGroupMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, StaticMeshSystem* static_mesh_render_group, StaticMeshSystem::StaticMeshHandle static_mesh_handle, GTSL::StaticString<64> resourceName) {
	auto resource = resources.TryEmplace(GTSL::StringView(resourceName));

	const auto instanceIndex = instances.Emplace();
	auto instanceHandle = GetApplicationManager()->MakeHandle<InstanceHandle>(InstanceTypeIndentifier, instanceIndex);
	meshToInstanceMap.Emplace(static_mesh_handle, instanceHandle);

	if (resource) { // If resource isn't already loaded 
		static_mesh_resource_manager->LoadStaticMeshInfo(task_info.ApplicationManager, resourceName, onStaticMeshInfoLoadHandle);
	} else {
		if (resource.Get().Loaded) {
			AddMeshInstance(Id(resourceName), instanceHandle);
		}
	}

	resource.Get().Instances.EmplaceBack(instanceHandle);
}