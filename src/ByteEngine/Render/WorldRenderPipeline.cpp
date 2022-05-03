#include "WorldRenderPipeline.hpp";

WorldRendererPipeline::WorldRendererPipeline(const InitializeInfo& initialize_info) : RenderPipeline(initialize_info, u8"WorldRendererPipeline"), spherePositionsAndRadius(16, GetPersistentAllocator()), instances(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator()), materials(GetPersistentAllocator()), meshToInstanceMap(16, GetPersistentAllocator()), InstanceTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"Instance")) {
	auto* renderSystem = initialize_info.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = initialize_info.ApplicationManager->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	rayTracing = BE::Application::Get()->GetBoolOption(u8"rayTracing");

	onStaticMeshInfoLoadHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnStaticMeshInfoLoad", DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager", AccessTypes::READ_WRITE), TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator", AccessTypes::READ_WRITE)), &WorldRendererPipeline::onStaticMeshInfoLoaded);

	onStaticMeshLoadHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnStaticMeshLoad", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE), TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::onStaticMeshLoaded);

	OnAddMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnAddMesh", DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"), TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")), &WorldRendererPipeline::OnAddMesh);
	GetApplicationManager()->SubscribeToEvent(u8"SMGR", StaticMeshRenderGroup::GetOnAddMeshEventHandle(), OnAddMeshTaskHandle);

	OnUpdateMeshTaskHandle = initialize_info.ApplicationManager->RegisterTask(this, u8"OnUpdateMesh", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::OnUpdateMesh);
	//GetApplicationManager()->SubscribeToEvent(u8"SMGR", StaticMeshRenderGroup::GetOnUpdateMeshEventHandle(), OnUpdateMeshTaskHandle);

	initialize_info.ApplicationManager->EnqueueScheduledTask(initialize_info.ApplicationManager->RegisterTask(this, u8"renderSetup", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::preRender, u8"RenderSetup", u8"Render"));

	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnAddPointLight"));
	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle, GTSL::Vector3>(u8"OnUpdatePointLight"));
	initialize_info.ApplicationManager->AddEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnRemovePointLight"));

	GetApplicationManager()->AddTypeSetupDependency(this, GetApplicationManager()->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")->GetStaticMeshTypeIdentifier(), OnAddMeshTaskHandle, true);
	addInstanceResourceHandle = GetApplicationManager()->AddResource(this, InstanceTypeIndentifier);
	//GetApplicationManager()->CoupleTasks(GetApplicationManager()->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")->GetOnUpdateMeshEventHandle(), OnUpdateMeshTaskHandle);
	GetApplicationManager()->AddTypeSetupDependency(this, GetApplicationManager()->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")->GetStaticMeshTypeIdentifier(), OnUpdateMeshTaskHandle, false);

	auto addLightTaskHandle = GetApplicationManager()->RegisterTask(this, u8"addPointLight", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), &WorldRendererPipeline::onAddLight);
	initialize_info.ApplicationManager->SubscribeToEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle>(u8"OnAddPointLight"), addLightTaskHandle);
	auto updateLightTaskHandle = GetApplicationManager()->RegisterTask(this, u8"updatePointLight", DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), & WorldRendererPipeline::updateLight);
	initialize_info.ApplicationManager->SubscribeToEvent(u8"WorldRendererPipeline", EventHandle<LightsRenderGroup::PointLightHandle, GTSL::Vector3, GTSL::RGB, float32>(u8"OnUpdatePointLight"), updateLightTaskHandle);

	vertexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::VERTEX | GAL::BufferUses::BUILD_INPUT_READ, true, false, {});
	indexBuffer = renderSystem->CreateBuffer(1024 * 1024 * 4, GAL::BufferUses::INDEX | GAL::BufferUses::BUILD_INPUT_READ, true, false, {});

	RenderOrchestrator::NodeHandle renderPassNodeHandle;

	renderOrchestrator->AddNotifyShaderGroupCreated(GTSL::Delegate<void(RenderOrchestrator*, RenderSystem*)>::Create<WorldRendererPipeline, &WorldRendererPipeline::onAddShaderGroup>(this));

	if (renderOrchestrator->tag == GTSL::ShortString<16>(u8"Forward")) {
		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.Attachments.EmplaceBack(u8"Color", GAL::AccessTypes::WRITE);
		geoRenderPass.Attachments.EmplaceBack(u8"Normal", GAL::AccessTypes::WRITE);
		geoRenderPass.Attachments.EmplaceBack(u8"WorldPosition", GAL::AccessTypes::WRITE);
		geoRenderPass.Attachments.EmplaceBack(u8"RenderDepth", GAL::AccessTypes::WRITE);
		renderPassNodeHandle = renderOrchestrator->AddRenderPass(u8"ForwardRenderPass", renderOrchestrator->GetGlobalDataLayer(), renderSystem, geoRenderPass);
	}
	else if (renderOrchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
		RenderOrchestrator::PassData geoRenderPass;
		geoRenderPass.PassType = RenderOrchestrator::PassType::RASTER;
		geoRenderPass.Attachments.EmplaceBack(u8"Visibility", GAL::AccessTypes::WRITE);
		geoRenderPass.Attachments.EmplaceBack(u8"RenderDepth", GAL::AccessTypes::WRITE);
		renderPassNodeHandle = renderOrchestrator->AddRenderPass(u8"VisibilityRenderPass", renderOrchestrator->GetGlobalDataLayer(), renderSystem, geoRenderPass);

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
		renderOrchestrator->CreateMember(u8"global", u8"VisibilityData", members);

		visibilityDataKey = renderOrchestrator->CreateDataKey(renderSystem, u8"global", u8"VisibilityData");
		renderPassNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"VisibilityDataLightingDataNode", visibilityDataKey);

		//pixelXY stores blocks per material that determine which pixels need to be painted with each material
		auto pielBuffer = renderOrchestrator->CreateDataKey(renderSystem, u8"global", u8"vec2s[2073600]"); //1920 * 1080

		{
			auto bwk = renderOrchestrator->GetBufferWriteKey(renderSystem, visibilityDataKey);

			const auto vertexElementsThatFitInBuffer = ((1024 * 1024 * 4) / 56u);

			bwk[u8"positionStream"] = vertexBuffer;
			bwk[u8"normalStream"] = renderSystem->MakeAddress(vertexBuffer, 12 * 1 * vertexElementsThatFitInBuffer); //todo: if buffer is updatable only address for current frame will be set
			bwk[u8"tangentStream"] = renderSystem->MakeAddress(vertexBuffer, 12 * 2 * vertexElementsThatFitInBuffer);
			bwk[u8"bitangentStream"] = renderSystem->MakeAddress(vertexBuffer, 12 * 3 * vertexElementsThatFitInBuffer);
			bwk[u8"textureCoordinatesStream"] = renderSystem->MakeAddress(vertexBuffer, 12 * 4 * vertexElementsThatFitInBuffer);
			bwk[u8"shaderGroupLength"] = 0u;
			bwk[u8"pixelBuffer"] = pielBuffer;
		}

		//Counts how many pixels each shader group uses
		RenderOrchestrator::PassData countPixelsRenderPassData;
		countPixelsRenderPassData.PassType = RenderOrchestrator::PassType::COMPUTE;
		countPixelsRenderPassData.Attachments.EmplaceBack(u8"Visibility", GAL::AccessTypes::READ);
		renderOrchestrator->AddRenderPass(u8"CountPixels", renderOrchestrator->GetGlobalDataLayer(), renderSystem, countPixelsRenderPassData);

		////Performs a prefix to build an indirect buffer defining which pixels each shader group occupies
		//RenderOrchestrator::PassData prefixSumRenderPassData;
		//prefixSumRenderPassData.PassType = RenderOrchestrator::PassType::COMPUTE;
		//renderOrchestrator->AddRenderPass(u8"PrefixSum", renderOrchestrator->GetCameraDataLayer(), renderSystem, prefixSumRenderPassData, GetApplicationManager());
		//
		////Scans the whole rendered image and stores which pixels every shader group occupies utilizing the information from the prefix sum pass
		//RenderOrchestrator::PassData selectPixelsRenderPass;
		//selectPixelsRenderPass.PassType = RenderOrchestrator::PassType::COMPUTE;
		//countPixelsRenderPassData.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Visibility" });
		//renderOrchestrator->AddRenderPass(u8"SelectPixels", renderOrchestrator->GetCameraDataLayer(), renderSystem, selectPixelsRenderPass, GetApplicationManager());
		//
		////Every participating shader group is called to paint every pixel it occupies on screen
		//RenderOrchestrator::PassData paintRenderPassData;
		//paintRenderPassData.PassType = RenderOrchestrator::PassType::RASTER;
		//paintRenderPassData.ReadAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Visibility" });
		//paintRenderPassData.WriteAttachments.EmplaceBack(RenderOrchestrator::PassData::AttachmentReference{ u8"Color" });
		//renderOrchestrator->AddRenderPass(u8"PaintPixels", renderOrchestrator->GetCameraDataLayer(), renderSystem, paintRenderPassData, GetApplicationManager());

		//renderOrchestrator->SetShaderGroupParameter(renderSystem, ShaderGroupHandle{}, u8"materialCount", 0u);
	}

	RenderOrchestrator::PassData gammaCorrectionPass;
	gammaCorrectionPass.PassType = RenderOrchestrator::PassType::COMPUTE;
	gammaCorrectionPass.Attachments.EmplaceBack(u8"Color", GAL::AccessTypes::WRITE); //result attachment
	renderOrchestrator->AddRenderPass(u8"GammaCorrection", renderOrchestrator->GetGlobalDataLayer(), renderSystem, gammaCorrectionPass);

	renderOrchestrator->CreateMember2(u8"global", u8"StaticMeshData", INSTANCE_DATA);
	meshDataBuffer = renderOrchestrator->CreateDataKey(renderSystem, u8"global", u8"StaticMeshData[8]", meshDataBuffer);

	renderOrchestrator->CreateMember2(u8"global", u8"PointLightData", POINT_LIGHT_DATA);
	renderOrchestrator->CreateMember2(u8"global", u8"LightingData", LIGHTING_DATA);

	renderPassNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"CameraData", renderOrchestrator->cameraDataKeyHandle);

	lightsDataKey = renderOrchestrator->CreateDataKey(renderSystem, u8"global", u8"LightingData");
	lightingDataNodeHandle = renderOrchestrator->AddDataNode(renderPassNodeHandle, u8"LightingDataNode", lightsDataKey);

	vertexBufferNodeHandle = renderOrchestrator->AddVertexBufferBind(renderSystem, lightingDataNodeHandle, vertexBuffer, { { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT3 }, { GAL::ShaderDataType::FLOAT2 } });
	indexBufferNodeHandle = renderOrchestrator->AddIndexBufferBind(vertexBufferNodeHandle, indexBuffer);
	meshDataNode = renderOrchestrator->AddDataNode(indexBufferNodeHandle, u8"MeshNode", meshDataBuffer, true);

	if (renderOrchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
		auto shaderGroupHandle = renderOrchestrator->CreateShaderGroup(Id(u8"VisibilityShaderGroup"));
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

	//add node
	//RenderOrchestrator::PassData pass_data;
	//pass_data.PassType = RenderOrchestrator::PassType::COMPUTE;
	//pass_data.WriteAttachments.EmplaceBack(u8"Color");
	//pass_data.ReadAttachments.EmplaceBack(u8"Normal");
	//pass_data.ReadAttachments.EmplaceBack(u8"RenderDepth");
	//auto renderPassLayerHandle = renderOrchestrator->AddRenderPass(u8"Lighting", renderOrchestrator->GetCameraDataLayer(), renderSystem, pass_data, initialize_info.ApplicationManager);
}

void WorldRendererPipeline::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
	auto& resource = resources[Id(staticMeshInfo.GetName())];

	auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(), indicesSize = staticMeshInfo.GetIndexCount() * staticMeshInfo.GetIndexSize();

	resource.VertexSize = staticMeshInfo.GetVertexSize();
	resource.VertexCount = staticMeshInfo.VertexCount;
	resource.IndexCount = staticMeshInfo.IndexCount;
	resource.IndexType = GAL::SizeToIndexType(staticMeshInfo.IndexSize);
	resource.Interleaved = staticMeshInfo.Interleaved;

	resource.Offset = vertexComponentsPerStream; resource.IndexOffset = indicesInBuffer;

	for (uint32 i = 0; i < staticMeshInfo.GetSubMeshes().Length; ++i) {
		auto& sm = staticMeshInfo.GetSubMeshes().array[i];
		auto shaderGroupHandle = render_orchestrator->CreateShaderGroup(Id(sm.ShaderGroupName));

		if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Forward")) {
			RenderOrchestrator::NodeHandle materialNodeHandle;
			if (auto r = materials.TryEmplace(shaderGroupHandle.ShaderGroupIndex)) {
				auto materialDataNode = render_orchestrator->AddDataNode(meshDataNode, u8"MaterialNode", render_orchestrator->shaderGroups[shaderGroupHandle.ShaderGroupIndex].Buffer);
				r.Get().Node = render_orchestrator->AddMaterial(materialDataNode, shaderGroupHandle);
				materialNodeHandle = r.Get().Node;
			}
			else {
				materialNodeHandle = r.Get().Node;
			}

			resource.nodeHandle = render_orchestrator->AddMesh(materialNodeHandle, 0, resource.IndexCount, indicesInBuffer, vertexComponentsPerStream);
		}
		else if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
			if (auto r = materials.TryEmplace(shaderGroupHandle.ShaderGroupIndex)) {
				resource.nodeHandle = render_orchestrator->AddMesh(mainVisibilityPipelineNode, 0, resource.IndexCount, indicesInBuffer, vertexComponentsPerStream);
			}

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

			if (b == GAL::ShaderDataType::U16_UNORM or b == GAL::ShaderDataType::U16_UNORM2 or b == GAL::ShaderDataType::U16_UNORM3 or b == GAL::ShaderDataType::U16_UNORM4) {
				usesxNorm = true;
			}

			if (b == GAL::ShaderDataType::U16_SNORM or b == GAL::ShaderDataType::U16_SNORM2 or b == GAL::ShaderDataType::U16_SNORM3 or b == GAL::ShaderDataType::U16_SNORM4) {
				usesxNorm = true;
			}
		}

	}

	if (usesxNorm) {
		//don't always assign bounding box as scaling factor, as even if we didn't need it bounding boxes usually have little errors which would cause the mesh to be scaled incorrectly
		//even though we have the correct coordinates to begin with
		resource.ScalingFactor = staticMeshInfo.GetBoundingBox();
	}

	staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, vertexComponentsPerStream, render_system->GetBufferRange(vertexBuffer), indicesInBuffer, render_system->GetBufferRange(indexBuffer), onStaticMeshLoadHandle);

	vertexComponentsPerStream += staticMeshInfo.GetVertexCount();
	indicesInBuffer += staticMeshInfo.GetIndexCount();
}

void WorldRendererPipeline::onStaticMeshLoaded(TaskInfo taskInfo, RenderSystem* render_system, StaticMeshRenderGroup* render_group, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo) {
	auto& res = resources[Id(staticMeshInfo.GetName())];

	auto commandListHandle = render_orchestrator->buildCommandList[render_system->GetCurrentFrame()];

	render_system->UpdateBuffer(commandListHandle, vertexBuffer); render_system->UpdateBuffer(commandListHandle, indexBuffer);
	render_orchestrator->AddVertices(vertexBufferNodeHandle, staticMeshInfo.GetVertexCount());
	render_orchestrator->AddIndices(indexBufferNodeHandle, staticMeshInfo.GetIndexCount());

	if (rayTracing) {
		res.BLAS = render_system->CreateBottomLevelAccelerationStructure(staticMeshInfo.VertexCount, 12/*todo: use actual position stride*/, staticMeshInfo.IndexCount, GAL::SizeToIndexType(staticMeshInfo.IndexSize), vertexBuffer, indexBuffer, res.Offset * 12/*todo: use actual position coordinate element size*/, res.IndexOffset);
		pendingBuilds.EmplaceBack(res.BLAS);
	}

	for (auto e : res.Instances) {
		AddMeshInstance(render_system, render_orchestrator, e, staticMeshInfo.GetName(), 0);
		*spherePositionsAndRadius.GetPointer<3>(e()) = staticMeshInfo.BoundingRadius;
	}

	res.Loaded = true;

	GTSL::StaticVector<GTSL::Range<const GAL::ShaderDataType*>, 8> r;

	for (auto& e : res.VertexElements) {
		r.EmplaceBack(e.GetRange());
	}
}

void WorldRendererPipeline::OnAddMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, StaticMeshRenderGroup* static_mesh_render_group, StaticMeshRenderGroup::StaticMeshHandle static_mesh_handle, Id resourceName) {
	const auto instanceIndex = instances.Emplace();
	const auto instanceHandle = GetApplicationManager()->MakeHandle<InstanceHandle>(InstanceTypeIndentifier, instanceIndex, static_mesh_handle);
	meshToInstanceMap.Emplace(static_mesh_handle, instanceHandle);
	auto resource = resources.TryEmplace(resourceName);

	spherePositionsAndRadius.EmplaceBack(0, 0, 0, 0);
	auto& instance = instances[instanceIndex];

	if (rayTracing) {
		instance.InstanceHandle = render_system->AddBLASToTLAS(topLevelAccelerationStructure, resource.Get().BLAS, 0, instance.InstanceHandle); // Custom instance index will be set later
	}

	if (resource) { // If resource isn't already loaded 
		//resource.Get().Index = prefixSum.EmplaceBack(0);
		//prefixSumGuide.EmplaceBack(resourceName);
		static_mesh_resource_manager->LoadStaticMeshInfo(task_info.ApplicationManager, resourceName, onStaticMeshInfoLoadHandle);
	}
	else {
		if (resource.Get().Loaded) {
			AddMeshInstance(render_system, render_orchestrator, instanceHandle, resourceName, 0);
		}
	}

	resource.Get().Instances.EmplaceBack(instanceHandle);
}