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
////updateInfo.Address = 0;
////info.MaterialSystem->UpdateRenderGroupData(updateInfo);

RenderOrchestrator::RenderOrchestrator(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderOrchestrator"),
shaders(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator()), dataKeys(16, GetPersistentAllocator()),
renderingTree(128, GetPersistentAllocator()), renderPasses(16), pipelines(8, GetPersistentAllocator()), shaderGroups(16, GetPersistentAllocator()),
shaderGroupsByName(16, GetPersistentAllocator()), textures(16, GetPersistentAllocator()), attachments(16, GetPersistentAllocator()),
elements(16, GetPersistentAllocator()), sets(16, GetPersistentAllocator()), queuedSetUpdates(1, 8, GetPersistentAllocator()), setLayoutDatas(2, GetPersistentAllocator()), pendingWrites(32, GetPersistentAllocator()), shaderHandlesDebugMap(16, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

	renderBuffers.EmplaceBack().BufferHandle = renderSystem->CreateBuffer(RENDER_DATA_BUFFER_PAGE_SIZE, GAL::BufferUses::STORAGE, true, true, RenderSystem::BufferHandle());

	for (uint32 i = 0; i < renderSystem->GetPipelinedFrames(); ++i) {
		descriptorsUpdates.EmplaceBack(GetPersistentAllocator());
	}

	elements.Emplace(0, GetPersistentAllocator());

	tryAddDataType(u8"global", u8"uint8", 1);
	tryAddDataType(u8"global", u8"uint16", 2);
	tryAddDataType(u8"global", u8"uint32", 4);
	tryAddDataType(u8"global", u8"uint64", 8);
	tryAddDataType(u8"global", u8"float32", 4);
	tryAddDataType(u8"global", u8"vec2f", 4 * 2);
	tryAddDataType(u8"global", u8"vec3f", 4 * 3);
	tryAddDataType(u8"global", u8"vec4f", 4 * 4);
	tryAddDataType(u8"global", u8"matrix4f", 4 * 4 * 4);
	tryAddDataType(u8"global", u8"ptr_t", 8);
	tryAddDataType(u8"global", u8"ShaderHandle", 32);

	tryAddDataType(u8"global", u8"TextureReference", 4);
	tryAddDataType(u8"global", u8"ImageReference", 4);

	{
		uint64 allocatedSize;
		GetPersistentAllocator().Allocate(1024 * 8, 32, reinterpret_cast<void**>(&buffer[0]), &allocatedSize); //TODO: free
	}

	// MATERIALS

	onTextureInfoLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(this, u8"onTextureInfoLoad", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureInfoLoad);
	onTextureLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(this, u8"loadTexture", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureLoad);

	onShaderInfosLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(this, u8"onShaderGroupInfoLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager")), &RenderOrchestrator::onShaderInfosLoaded);
	onShaderGroupLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(this, u8"onShaderGroupLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onShadersLoaded);

	initializeInfo.ApplicationManager->AddTask(this, SETUP_TASK_NAME, &RenderOrchestrator::Setup, DependencyBlock(), u8"GameplayEnd", u8"RenderSetup");
	initializeInfo.ApplicationManager->AddTask(this, RENDER_TASK_NAME, &RenderOrchestrator::Render, DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem")), u8"Render", u8"Render");

	//{
	//	GTSL::StaticVector<TaskDependency, 1> dependencies{ { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
	//
	//	auto renderEnableHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"RenderOrchestrator::OnRenderEnable", &RenderOrchestrator::OnRenderEnable, dependencies);
	//	//initializeInfo.ApplicationManager->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusGainEventHandle(), renderEnableHandle);
	//
	//	auto renderDisableHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"RenderOrchestrator::OnRenderDisable", &RenderOrchestrator::OnRenderDisable, dependencies);
	//	//initializeInfo.ApplicationManager->SubscribeToEvent(u8"Application", GameApplication::GetOnFocusLossEventHandle(), renderDisableHandle);
	//}

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"RenderOrchestrator", AccessTypes::READ_WRITE } };
		onRenderEnable(initializeInfo.ApplicationManager, taskDependencies);
	}

	{ //sampler must be built before set layouts, as it is used as inmutable sampler
		auto& sampler = samplers.EmplaceBack();
		sampler.Initialize(renderSystem->GetRenderDevice(), 0);
	}

	{
		GTSL::StaticVector<SubSetDescriptor, 10> subSetInfos;
		subSetInfos.EmplaceBack(SubSetType::READ_TEXTURES, 16, &textureSubsetsHandle);
		subSetInfos.EmplaceBack(SubSetType::WRITE_TEXTURES, 16, &imagesSubsetHandle);
		subSetInfos.EmplaceBack(SubSetType::SAMPLER, 16, &samplersSubsetHandle, samplers);

		globalSetLayout = AddSetLayout(renderSystem, SetLayoutHandle(), subSetInfos);
		globalBindingsSet = AddSet(renderSystem, u8"GlobalData", globalSetLayout, subSetInfos);
	}

	{
		GTSL::StaticVector<MemberInfo, 4> members;
		members.EmplaceBack(&globalDataHandle, u8"uint32", u8"time");
		members.EmplaceBack(&globalDataHandle, u8"uint32", u8"blah");
		members.EmplaceBack(&globalDataHandle, u8"uint32", u8"a");
		members.EmplaceBack(&globalDataHandle, u8"uint32", u8"b");
		auto d = CreateMember(u8"global", u8"GlobalData", members);
		globalData = AddLayer(u8"GlobalData", NodeHandle());
		BindToNode(renderSystem, globalData, d);
	}

	{
		MemberHandle t;
		GTSL::StaticVector<MemberInfo, 4> members;
		members.EmplaceBack(&t, u8"matrix4f", u8"view");
		members.EmplaceBack(&t, u8"matrix4f", u8"proj");
		members.EmplaceBack(&t, u8"matrix4f", u8"viewInverse");
		members.EmplaceBack(&t, u8"matrix4f", u8"projInverse");
		cameraMatricesHandle = CreateMember(u8"global", u8"CameraData", members);
		cameraDataNode = AddLayer(u8"CameraData", globalData);
		BindToNode(renderSystem, cameraDataNode, cameraMatricesHandle);
		PrintMember(cameraMatricesHandle, renderBuffers[0].BufferHandle, renderSystem);
	}

	if constexpr (BE_DEBUG) {
		pipelineStages |= BE::Application::Get()->GetOption(u8"debugSync") ? GAL::PipelineStages::ALL_GRAPHICS : GAL::PipelineStage(0);
	}

	{
		AddAttachment(u8"Color", 8, 4, GAL::ComponentType::INT, GAL::TextureType::COLOR);
		AddAttachment(u8"Normal", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
		AddAttachment(u8"RenderDepth", 32, 1, GAL::ComponentType::FLOAT, GAL::TextureType::DEPTH);

		PassData geoRenderPass;
		geoRenderPass.PassType = PassType::RASTER;
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Color" }); //result attachment
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"Normal" });
		geoRenderPass.WriteAttachments.EmplaceBack(PassData::AttachmentReference{ u8"RenderDepth" });
		AddRenderPass(u8"SceneRenderPass", GetCameraDataLayer(), renderSystem, geoRenderPass, initializeInfo.ApplicationManager);

		RenderOrchestrator::PassData colorGrading{};
		colorGrading.PassType = RenderOrchestrator::PassType::COMPUTE;
		colorGrading.WriteAttachments.EmplaceBack(u8"Color"); //result attachment
		//auto cgrp = renderOrchestrator->AddRenderPass(u8"ColorGradingRenderPass", renderOrchestrator->GetGlobalDataLayer(), renderSystem, colorGrading, applicationManager, applicationManager->GetSystem<ShaderResourceManager>(u8"ShaderResourceManager"));
	}

	for (uint32 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		graphicsCommandLists[f] = renderSystem->CreateCommandList(u8"Command List", GAL::QueueTypes::GRAPHICS);
	}
}

void RenderOrchestrator::Setup(TaskInfo taskInfo) {
}

template<typename K, typename V, class ALLOC>
void Skim(GTSL::HashMap<K, V, ALLOC>& hash_map, auto predicate) {
	GTSL::StaticVector<uint32, 512> toSkim;
	GTSL::PairForEach(hash_map, [&](K key, V& val) { if (predicate(val)) { toSkim.EmplaceBack(key); } });
	for (auto e : toSkim) { hash_map.Remove(e); }
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

	renderSystem->StartCommandList(graphicsCommandLists[currentFrame]);

	auto& commandBuffer = *renderSystem->GetCommandList(graphicsCommandLists[currentFrame]);

	BindSet(renderSystem, commandBuffer, globalBindingsSet, GAL::ShaderStages::VERTEX | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN);

	Id resultAttachment;

	{
		auto processPendingWrite = [&](PendingWriteData& pending_write_data) {
			bool res;
			pending_write_data.FrameCountdown.Get(currentFrame, res);
			if (res) {
				GTSL::MemCopy(pending_write_data.Size, pending_write_data.WhereToRead, pending_write_data.WhereToWrite);
				return true;
			}

			return false;
		};

		Skim(pendingWrites, processPendingWrite);
	}

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
			key[u8"CameraData"][u8"view"] = viewMatrix;
			key[u8"CameraData"][u8"proj"] = projectionMatrix;
			key[u8"CameraData"][u8"viewInverse"] = GTSL::Math::Inverse(viewMatrix);
			key[u8"CameraData"][u8"projInverse"] = GTSL::Math::BuildInvertedPerspectiveMatrix(fov, aspectRatio, 0.01f, 1000.f);
		}
		else { //disable rendering for everything which depends on this view
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

		if (auto address = baseData.Address[currentFrame]) { //if node has an associated data entry, bind it
			dataStreamHandle = renderState.AddDataStream();
			le[level] = true;
			auto& setLayout = setLayoutDatas[globalSetLayout()]; address += baseData.Offset;
			commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), setLayout.PipelineLayout, dataStreamHandle() * 8, GTSL::Range(8, reinterpret_cast<const byte*>(&address)), setLayout.Stage);
		}

		switch (renderingTree.GetBetaNodeType(key)) {
		case RTT::GetTypeIndex<PipelineBindData>(): {
			const PipelineBindData& pipeline_bind_data = renderingTree.GetClass<PipelineBindData>(key);
			const auto& shaderGroup = shaderGroups[pipeline_bind_data.Handle.ShaderGroupIndex];
			uint32 pipelineIndex = 0xFFFFFFFF;
			if (shaderGroup.RasterPipelineIndex != 0xFFFFFFFF) {
				pipelineIndex = shaderGroup.RasterPipelineIndex;
			}
			else if (shaderGroup.ComputePipelineIndex != 0xFFFFFFFF) {
				pipelineIndex = shaderGroup.ComputePipelineIndex;
			}
			else if (shaderGroup.RTPipelineIndex != 0xFFFFFFFF) {
				pipelineIndex = shaderGroup.RTPipelineIndex;
			}
			else {
				BE_LOG_WARNING(u8"Pipeline bind data node with no valid pipeline reference.");
			}

			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelines[pipelineIndex].pipeline, renderState.ShaderStages);
			break;
		}
		case RTT::GetTypeIndex<DispatchData>(): {
			const DispatchData& dispatchData = renderingTree.GetClass<DispatchData>(key);
			commandBuffer.Dispatch(renderSystem->GetRenderDevice(), renderArea); //todo: change
			break;
		}
		case RTT::GetTypeIndex<RayTraceData>(): {
			const RayTraceData& rayTraceData = renderingTree.GetClass<RayTraceData>(key);
			const auto& pipelineData = pipelines[shaderGroups[rayTraceData.ShaderGroupIndex].RTPipelineIndex];
			CommandList::ShaderTableDescriptor shaderTableDescriptors[4];
			for (uint32 i = 0, offset = 0; i < 3; ++i) {
				shaderTableDescriptors[i].Entries = pipelineData.RayTracingData.ShaderGroups[i].ShaderCount;
				shaderTableDescriptors[i].EntrySize = GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
				shaderTableDescriptors[i].Address = renderSystem->GetBufferAddress(pipelineData.ShaderBindingTableBuffer) + offset;

				offset += GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
			}

			commandBuffer.TraceRays(renderSystem->GetRenderDevice(), GTSL::Range(4, shaderTableDescriptors), sizeHistory[currentFrame]);

			break;
		}
		case RTT::GetTypeIndex<MeshData>(): {
			const MeshData& meshData = renderingTree.GetClass<MeshData>(key);

			auto buffer = renderSystem->GetBuffer(meshData.Handle);

			commandBuffer.BindVertexBuffer(renderSystem->GetRenderDevice(), buffer, meshData.VertexSize * meshData.VertexCount, 0, meshData.VertexSize);
			commandBuffer.BindIndexBuffer(renderSystem->GetRenderDevice(), buffer, GTSL::Math::RoundUpByPowerOf2(meshData.VertexSize * meshData.VertexCount, 8), meshData.IndexCount, meshData.IndexType);
			commandBuffer.DrawIndexed(renderSystem->GetRenderDevice(), meshData.IndexCount, 1);
			break;
		}
		case RTT::GetTypeIndex<RenderPassData>(): {
			const RenderPassData& renderPassData = renderingTree.GetClass<RenderPassData>(key);

			transitionImages(commandBuffer, renderSystem, &renderPassData);

			switch (renderPassData.Type) {
			case PassType::RASTER: {
				updateRenderStages(GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT);

				GTSL::StaticVector<GAL::RenderPassTargetDescription, 8> renderPassTargetDescriptions;
				for (uint8 i = 0; i < renderPassData.Attachments.GetLength(); ++i) {
					if (renderPassData.Attachments[i].Access & GAL::AccessTypes::WRITE) {
						auto& e = renderPassTargetDescriptions.EmplaceBack();
						const auto& attachment = attachments.At(renderPassData.Attachments[i].Name);
						e.ClearValue = attachment.ClearColor;
						e.Start = renderPassData.Attachments[i].Layout;
						e.End = renderPassData.Attachments[i].Layout;
						e.LoadOperation = GAL::Operations::CLEAR;
						e.StoreOperation = GAL::Operations::DO;
						e.FormatDescriptor = attachment.FormatDescriptor;
						e.Texture = renderSystem->GetTexture(attachment.TextureHandle[currentFrame]);
						e.TextureView = renderSystem->GetTextureView(attachment.TextureHandle[currentFrame]);
					}
				}

				commandBuffer.BeginRenderPass(renderSystem->GetRenderDevice(), renderArea, renderPassTargetDescriptions);

				resultAttachment = renderPassData.Attachments[0].Name;
				break;
			}
			case PassType::COMPUTE: {
				updateRenderStages(GAL::ShaderStages::COMPUTE);
				break;
			}
			case PassType::RAY_TRACING: {
				updateRenderStages(GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::CALLABLE);
				break;
			}
			}

			//todo: write

			break;
		}
		}
	};

	auto endNode = [&](const uint32 key, const uint32_t level) {
		switch (renderingTree.GetBetaNodeType(key)) {
		case RTT::GetTypeIndex<RenderPassData>(): {
			auto& renderPassData = renderingTree.GetClass<RenderPassData>(key);
			if (renderPassData.Type == PassType::RASTER) {
				commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
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

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
	CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

	if (resultAttachment) {
		auto& attachment = attachments.At(resultAttachment);

		commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, attachment.AccessType,
			GAL::AccessTypes::READ, CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout[currentFrame],
			GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor } } }, GetTransientAllocator());

		updateImage(currentFrame, attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);

		commandBuffer.CopyTextureToTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachments.At(resultAttachment).TextureHandle[currentFrame]),
			*renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_SOURCE, GAL::TextureLayout::TRANSFER_DESTINATION,
			attachments.At(resultAttachment).FormatDescriptor, renderSystem->GetSwapchainFormat(),
			GTSL::Extent3D(renderSystem->GetRenderExtent()));
	}

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION,
	GAL::TextureLayout::PRESENTATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

	renderSystem->EndCommandList(graphicsCommandLists[currentFrame]);

	{
		GTSL::StaticVector<RenderSystem::CommandListHandle, 8> commandLists;

		if (BE::Application::Get()->GetOption(u8"rayTracing")) {
			commandLists.EmplaceBack(buildCommandList[currentFrame]);
		}

		commandLists.EmplaceBack(graphicsCommandLists[currentFrame]);

		renderSystem->SubmitAndPresent(commandLists);
	}
}

ShaderGroupHandle RenderOrchestrator::CreateShaderGroup(Id shader_group_name) {
	auto shaderGroupReference = shaderGroupsByName.TryEmplace(shader_group_name);

	uint32 materialIndex = 0xFFFFFFFF;

	if (shaderGroupReference.State()) {
		materialIndex = shaderGroups.Emplace();
		shaderGroupReference.Get() = materialIndex;

		ShaderLoadInfo sli(GetPersistentAllocator());
		GetApplicationManager()->GetSystem<ShaderResourceManager>(u8"ShaderResourceManager")->LoadShaderGroupInfo(GetApplicationManager(), shader_group_name, onShaderInfosLoadHandle, GTSL::MoveRef(sli));

		auto& shaderGroup = shaderGroups[materialIndex];

		if constexpr (BE_DEBUG) { shaderGroup.Name = GTSL::StringView(shader_group_name); }
		shaderGroup.ResourceHandle = makeResource();
		addDependencyOnResource(shaderGroup.ResourceHandle); // Add dependency the pipeline itself
		shaderGroup.DataKey = MakeDataKey();
	}
	else {
		auto& material = shaderGroups[shaderGroupReference.Get()];
		materialIndex = shaderGroupReference.Get();
		//auto index = material.MaterialInstances.LookFor([&](const MaterialInstance& materialInstance) {
		//	return materialInstance.Name == info.InstanceName;
		//});

		//TODO: ERROR CHECK

		//materialInstanceIndex = index.Get();
	}

	return { materialIndex };
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
	}
	else {
		attachment.FormatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::DEPTH, 0, 0, 0, 0);
		attachment.ClearColor = GTSL::RGBA(1, 0, 0, 0);
	}

	attachment.Layout[0] = GAL::TextureLayout::UNDEFINED; attachment.Layout[1] = GAL::TextureLayout::UNDEFINED; attachment.Layout[2] = GAL::TextureLayout::UNDEFINED;
	attachment.AccessType = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
	attachment.ImageIndex = imageIndex++; ++textureIndex;

	attachments.Emplace(attachmentName, attachment);
}

RenderOrchestrator::NodeHandle RenderOrchestrator::AddRenderPass(GTSL::StringView renderPassName, NodeHandle parent, RenderSystem* renderSystem, PassData passData, ApplicationManager* am) {
	NodeHandle renderPassNodeHandle = addNode(Id(renderPassName), parent, NodeType::RENDER_PASS);
	InternalNodeHandle internalNodeHandle = addInternalNode<RenderPassData>(Hash(renderPassName), renderPassNodeHandle, parent);
	RenderPassData& renderPass = getPrivateNode<RenderPassData>(internalNodeHandle);

	renderPasses.Emplace(renderPassName, renderPassNodeHandle, internalNodeHandle);
	renderPassesInOrder.EmplaceBack(internalNodeHandle);

	renderPass.ResourceHandle = makeResource();
	addDependencyOnResource(renderPass.ResourceHandle); //add dependency on render pass texture creation

	BindToNode(internalNodeHandle, renderPass.ResourceHandle);

	getNode(internalNodeHandle).Name = GTSL::StringView(renderPassName);

	Id resultAttachment;

	if (passData.WriteAttachments.GetLength())
		resultAttachment = passData.WriteAttachments[0].Name;

	{
		auto& finalAttachment = attachments.At(resultAttachment);
		finalAttachment.FormatDescriptor = GAL::FORMATS::BGRA_I8;
	}

	switch (passData.PassType) {
	case PassType::RASTER: {
		renderPass.Type = PassType::RASTER;
		renderPass.PipelineStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;

		for (const auto& e : passData.ReadAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::SHADER_READ; attachmentData.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
			attachmentData.Access = GAL::AccessTypes::READ;
		}

		for (const auto& e : passData.WriteAttachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();
			attachmentData.Name = e.Name; attachmentData.Layout = GAL::TextureLayout::ATTACHMENT; attachmentData.ConsumingStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
			attachmentData.Access = GAL::AccessTypes::WRITE;
		}

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

		auto dispatchNodeHandle = addInternalNode<DispatchData>(Hash(renderPassName), renderPassNodeHandle, parent);

		auto loadComputeShader = [&]() {
			//auto shaderLoadInfo = ShaderLoadInfo(GetPersistentAllocator());
			//shaderLoadInfo.handle = dispatchNodeHandle;
			//srm->LoadShaderGroupInfo(am, renderPassName, onShaderInfosLoadHandle, GTSL::MoveRef(shaderLoadInfo));
			//
			//getNode(dispatchNodeHandle).Name = GTSL::StringView(renderPassName);

			return Id(renderPassName);
		};

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

	GTSL::StaticVector<MemberInfo, 16> members;

	for (auto& e : passData.WriteAttachments) {
		members.EmplaceBack(&renderPass.RenderTargetReferences, u8"ImageReference", GTSL::StringView(e.Name));
	}

	auto member = CreateMember(u8"global", renderPassName, members);
	BindToNode(renderSystem, renderPassNodeHandle, member);
	auto bwk = GetBufferWriteKey(renderSystem, renderPassNodeHandle, renderPass.RenderTargetReferences);
	for (auto i = 0u; i < passData.WriteAttachments.GetLength(); ++i) {
		bwk[renderPassName][GTSL::StringView(passData.WriteAttachments[i].Name)] = attachments[renderPass.Attachments[i].Name].ImageIndex;
	}

	PrintMember(member, renderBuffers[0].BufferHandle, renderSystem);

	return renderPassNodeHandle;
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize)
{
	//pendingDeleteFrames = renderSystem->GetPipelinedFrames();

	auto currentFrame = renderSystem->GetCurrentFrame();
	auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();

	auto resize = [&](Attachment& attachment) -> void {
		GTSL::StaticString<64> name(u8"Attachment: "); name += GTSL::StringView(attachment.Name);

		attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(name, attachment.FormatDescriptor, newSize, attachment.Uses, false, attachment.TextureHandle[currentFrame]);

		if (attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {  //if attachment is of type color (not depth), write image descriptor
			WriteBinding(renderSystem, imagesSubsetHandle, attachment.TextureHandle[currentFrame], attachment.ImageIndex, currentFrame);
		}

		WriteBinding(renderSystem, textureSubsetsHandle, attachment.TextureHandle[currentFrame], attachment.ImageIndex, currentFrame);
	};

	if (sizeHistory[currentFrame] != newSize) {
		sizeHistory[currentFrame] = newSize;
		GTSL::ForEach(attachments, resize);
	}

	for (const auto apiRenderPassData : renderPasses) {
		auto& layer = getPrivateNode<RenderPassData>(apiRenderPassData.Second);
		signalDependencyToResource(layer.ResourceHandle);
	}
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
	}
	else {
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
		textureBarrier.CurrentLayout = attachment.Layout[renderSystem->GetCurrentFrame()];
		textureBarrier.Format = attachment.FormatDescriptor;
		textureBarrier.TargetLayout = attachmentData.Layout;
		barriers.EmplaceBack(initialStage, renderPass->PipelineStages, attachment.AccessType, access, textureBarrier);

		initialStage |= attachment.ConsumingStages;

		updateImage(renderSystem->GetCurrentFrame(), attachment, attachmentData.Layout, renderPass->PipelineStages, access);
	};

	for (auto& e : renderPass->Attachments) { buildTextureBarrier(e, e.ConsumingStages, e.Access); }

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), barriers, GetTransientAllocator());
}

//TODO: GRANT CONTINUITY TO ALLOCATED PIPELINES PER SHADER GROUP

void RenderOrchestrator::onShaderInfosLoaded(TaskInfo taskInfo, ShaderResourceManager* materialResourceManager,
	ShaderResourceManager::ShaderGroupInfo shader_group_info, ShaderLoadInfo shaderLoadInfo)
{
	uint32 size = 0;

	for (auto& s : shader_group_info.Shaders) { size += s.Size; }

	shaderLoadInfo.Buffer.Allocate(size, 16);
	shaderLoadInfo.Buffer.AddBytes(size);

	materialResourceManager->LoadShaderGroup(taskInfo.ApplicationManager, shader_group_info, onShaderGroupLoadHandle, shaderLoadInfo.Buffer.GetRange(), GTSL::MoveRef(shaderLoadInfo));
}

void RenderOrchestrator::onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem* renderSystem, ShaderResourceManager::ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo)
{
	if constexpr (BE_DEBUG) {
		//if (!shader_group_info.Valid) {
		//	BE_LOG_ERROR(u8"Tried to load shader group ", shader_group_info.Name, u8" which is not valid. Will use stand in shader. ", BE::FIX_OR_CRASH_STRING);
		//	return;
		//}
	}

	auto& sg = shaderGroups[shaderGroupsByName[Id(shader_group_info.Name)]];

	addScope(u8"global", shader_group_info.Name);

	GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock, 32> pipelineStates;

	GTSL::StaticMap<Id, StructElement, 8> parameters;

	MemberHandle textureReferences[8];

	GTSL::Vector<GAL::Pipeline::VertexElement, BE::TAR> vertexElements(GetTransientAllocator());
	struct ShaderBundleData {
		GTSL::StaticVector<uint32, 8> Shaders;
		GAL::ShaderStage Stage;
		uint32 PipelineIndex = 0;
	};
	GTSL::StaticVector<ShaderBundleData, 4> shaderBundles;
	GTSL::StaticVector<MemberInfo, 16> members;
	GTSL::KeyMap<uint64, BE::TAR> loadedShadersMap(8, GetTransientAllocator()); //todo: differentiate hash from hash + name, since a different hash could be interpreted as a different shader, when in reality it functionally represents the same shader but with different code

	for (auto& e : shader_group_info.VertexElements) {
		GAL::ShaderDataType type;

		switch (Hash(e.Type)) {
		case GTSL::Hash(u8"vec2f"): type = GAL::ShaderDataType::FLOAT2; break;
		case GTSL::Hash(u8"vec3f"): type = GAL::ShaderDataType::FLOAT3; break;
		case GTSL::Hash(u8"vec4f"): type = GAL::ShaderDataType::FLOAT4; break;
		}

		vertexElements.EmplaceBack(GAL::Pipeline::VertexElement{ GTSL::ShortString<32>(e.Name.c_str()), type });
	}

	for (uint32 offset = 0, si = 0; const auto & s : shader_group_info.Shaders) {
		if (auto shader = shaders.TryEmplace(s.Hash)) {
			shader.Get().Shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range(s.Size, shaderLoadInfo.Buffer.GetData() + offset));
			shader.Get().Type = s.Type;
			shader.Get().Name = s.Name;
		}

		loadedShadersMap.Emplace(s.Hash);

		offset += s.Size;

		bool foundGroup = false;
		auto shaderStageFlag = GAL::ShaderTypeToShaderStageFlag(s.Type);

		for (auto& e : shaderBundles) {
			if (e.Stage & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT) && shaderStageFlag & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT)) {
				e.Shaders.EmplaceBack(si);
				e.Stage |= shaderStageFlag;
				foundGroup = true;
				break;
			}

			if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION) && shaderStageFlag & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION)) {
				e.Shaders.EmplaceBack(si);
				e.Stage |= shaderStageFlag;
				foundGroup = true;
				break;
			}
		}

		if (!foundGroup) {
			auto& sb = shaderBundles.EmplaceBack();
			sb.Shaders.EmplaceBack(si);
			sb.Stage = shaderStageFlag;
		}

		++si;
	}

	for (uint32 pi = 0; const auto & p : shader_group_info.Parameters) {
		parameters.Emplace(Id(p.Name), p.Type, p.Name, p.Value);
		members.EmplaceBack(MemberInfo{ &shaderGroups[shaderLoadInfo.MaterialIndex].ParametersHandles.Emplace(Id(p.Name)), p.Type, p.Name });
	}

	for (auto& e : shaderBundles) {
		GTSL::Vector<GPUPipeline::ShaderInfo, BE::TAR> shaderInfos(8, GetTransientAllocator());

		if (e.Stage & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT)) {
			if (sg.RasterPipelineIndex == 0xFFFFFFFF) { //if no pipeline already exists for this stage, create one
				sg.RasterPipelineIndex = pipelines.Emplace(GetPersistentAllocator());
			}

			e.PipelineIndex = sg.RasterPipelineIndex;

			for (auto s : e.Shaders) {
				auto& shaderInfo = shaderInfos.EmplaceBack();
				auto& shader = shaders[shader_group_info.Shaders[s].Hash];
				shaderInfo.Type = shader.Type;
				shaderInfo.Shader = shader.Shader;
				//shaderInfo.Blob = GTSL::Range(shader_group_info.Shaders[s].Size, shaderLoadInfo.Buffer.GetData() + offset);
			}

			GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock::RenderContext::AttachmentState, 8> att;

			GAL::Pipeline::PipelineStateBlock::RenderContext context;

			const auto& renderPassNode = getPrivateNodeFromPublicHandle<RenderPassData>(GetSceneRenderPass());

			for (const auto& writeAttachment : renderPassNode.Attachments) {
				if (writeAttachment.Access & GAL::AccessTypes::WRITE) {
					auto& attachment = attachments.At(writeAttachment.Name);
					auto& attachmentState = att.EmplaceBack();
					attachmentState.BlendEnable = false; attachmentState.FormatDescriptor = attachment.FormatDescriptor;
				}
			}

			context.Attachments = att;
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

			pipelines[e.PipelineIndex].pipeline.InitializeRasterPipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}

		if (e.Stage & GAL::ShaderStages::COMPUTE) {
			if (sg.ComputePipelineIndex == 0xFFFFFFFF) { //if no pipeline already exists for this stage, create one
				sg.ComputePipelineIndex = pipelines.Emplace(GetPersistentAllocator());
			}

			e.PipelineIndex = sg.ComputePipelineIndex;

			auto& pipeline = pipelines[e.PipelineIndex];

			for (auto s : e.Shaders) {
				auto& shaderInfo = shaderInfos.EmplaceBack();
				auto& shader = shaders[shader_group_info.Shaders[s].Hash];
				shaderInfo.Type = shader.Type;
				shaderInfo.Shader = shader.Shader;
				//shaderInfo.Blob = GTSL::Range(shader_group_info.Shaders[s].Size, shaderLoadInfo.Buffer.GetData() + offset);
			}

			sg.ComputePipelineIndex = e.PipelineIndex;
			pipeline.pipeline.InitializeComputePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}

		if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT)) {
			if (!BE::Application::Get()->GetOption(u8"rayTracing")) { continue; }

			if (rayTracingPipelineIndex == 0xFFFFFFFF) { //if no pipeline already exists for this stage, create one
				sg.RTPipelineIndex = pipelines.Emplace(GetPersistentAllocator());
				rayTracingPipelineIndex = sg.RTPipelineIndex;
			}
			else {
				sg.RTPipelineIndex = rayTracingPipelineIndex;
			}

			e.PipelineIndex = sg.RTPipelineIndex;

			auto& pipelineData = pipelines[e.PipelineIndex];

			//add newly loaded shaders to new pipeline update
			for (auto s : e.Shaders) {
				pipelineData.Shaders.EmplaceBack(shader_group_info.Shaders[s].Hash);
			}

			GTSL::Sort(pipelineData.Shaders, [&](uint64 a, uint64 b) {
				return shaders[a].Type > shaders[b].Type;
				});

			//add already loaded shaders to shaderInfos, todo: use pipeline libraries to accumulate state properly
			for (auto s : pipelineData.Shaders) {
				auto& shaderInfo = shaderInfos.EmplaceBack();
				auto& shader = shaders[s];
				shaderInfo.Type = shader.Type;
				shaderInfo.Shader = shader.Shader;
				BE_LOG_MESSAGE(static_cast<uint16>(shader.Type));
				//shaderInfo.Blob = GTSL::Range(shader_group_info.Shaders[s].Size, shaderLoadInfo.Buffer.GetData() + offset);
			}

			GTSL::Vector<GPUPipeline::RayTraceGroup, BE::TAR> rayTracingGroups(16, GetTransientAllocator());

			GPUPipeline::PipelineStateBlock::RayTracingState rayTracePipelineState;
			rayTracePipelineState.MaxRecursionDepth = 1;

			for (uint32 i = 0; i < pipelineData.Shaders; ++i) {
				auto& shaderInfo = shaders[pipelineData.Shaders[i]];

				GPUPipeline::RayTraceGroup group; uint8 shaderGroup = 0xFF;

				switch (shaderInfo.Type) {
				case GAL::ShaderType::RAY_GEN:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					shaderGroup = GAL::RAY_GEN_TABLE_INDEX;
					GTSL::Max(&rayTracePipelineState.MaxRecursionDepth, static_cast<uint8>(1));
					break;
				case GAL::ShaderType::MISS:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					shaderGroup = GAL::MISS_TABLE_INDEX;
					break;
				case GAL::ShaderType::CALLABLE:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					shaderGroup = GAL::CALLABLE_TABLE_INDEX;
					break;
				case GAL::ShaderType::CLOSEST_HIT:
					group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
					shaderGroup = GAL::HIT_TABLE_INDEX;
					break;
				case GAL::ShaderType::ANY_HIT:
					group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.AnyHitShader = i;
					shaderGroup = GAL::HIT_TABLE_INDEX;
					break;
				case GAL::ShaderType::INTERSECTION:
					group.ShaderGroup = GAL::ShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
					shaderGroup = GAL::HIT_TABLE_INDEX;
					break;
				default: BE_LOG_MESSAGE(u8"Non raytracing shader found in raytracing material");
				}

				rayTracingGroups.EmplaceBack(group);

				if (loadedShadersMap.Find(pipelineData.Shaders[i])) { //only increment shader count when a new shader is added (not when updated since the shader is already there)
					++pipelineData.RayTracingData.ShaderGroups[shaderGroup].ShaderCount;
				}
			}

			rayTracePipelineState.Groups = rayTracingGroups;
			pipelineStates.EmplaceBack(rayTracePipelineState);

			auto oldPipeline = pipelineData.pipeline;

			pipelineData.pipeline.InitializeRayTracePipeline(renderSystem->GetRenderDevice(), pipelineData.pipeline, pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());

			if (oldPipeline.GetHandle()) { //TODO: defer deletion
				oldPipeline.Destroy(renderSystem->GetRenderDevice());
			}
		}

		signalDependencyToResource(sg.ResourceHandle); //add ref count for pipeline load itself, todo: do we signal even when we are doing a pipeline update?
	}

	if (!sg.Loaded) {
		sg.Loaded = true;

		GTSL::StaticString<64> scope(u8"global"); scope << u8"." << GTSL::StringView(shader_group_info.Name);

		auto materialDataMember = CreateMember(scope, u8"ShaderParametersData", members);
		sg.Buffer = CreateBuffer(renderSystem, materialDataMember);
		sg.DataKey = MakeDataKey(renderSystem, sg.Buffer, sg.DataKey);

		for (uint8 ii = 0; auto & i : shader_group_info.Instances) { //TODO: check parameters against stored layout to check if everything is still compatible
			for (uint32 pi = 0; auto & p : i.Parameters) {
				Id parameterValue;

				//if shader instance has specialized value for param, use that, else, fallback to shader group default value for param
				if (p.Second) {
					parameterValue = Id(p.Second);
				}
				else {
					parameterValue = Id(parameters[Id(p.First)].DefaultValue);
				}

				switch (Hash(parameters[Id(p.First)].Type)) {
				case GTSL::Hash(u8"TextureReference"): {
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.ApplicationManager;
					createTextureInfo.TextureResourceManager = taskInfo.ApplicationManager->GetSystem<TextureResourceManager>(u8"TextureResourceManager");
					createTextureInfo.TextureName = static_cast<GTSL::StringView>(parameterValue);
					auto textureReference = createTexture(createTextureInfo);

					GetBufferWriteKey(renderSystem, sg.Buffer, textureReferences[pi])[textureReferences[pi]] = textureReference;

					for (auto& e : shaderBundles) {
						addPendingResourceToTexture(parameterValue, sg.ResourceHandle);
					}

					break;
				}
				case GTSL::Hash(u8"ImageReference"): {
					auto textureReference = attachments.TryGet(parameterValue);

					if (textureReference) {
						uint32 textureComponentIndex = textureReference.Get().ImageIndex;

						GetBufferWriteKey(renderSystem, sg.Buffer, textureReferences[pi])[textureReferences[pi]] = textureComponentIndex;
					}
					else {
						BE_LOG_WARNING(u8"Default parameter value of ", GTSL::StringView(parameterValue), u8" for shader group ", shader_group_info.Name, u8" parameter ", p.First, u8" could not be found.");
					}

					break;
				}
				}

				++pi;
			}

			++ii;
		}
	}

	for (auto& e : shaderBundles) {
		if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE)) {
			if (!BE::Application::Get()->GetOption(u8"rayTracing")) { continue; }
			auto& pipelineData = pipelines[e.PipelineIndex]; auto& rtPipelineData = pipelineData.RayTracingData;

			GTSL::Vector<GAL::ShaderHandle, BE::TAR> shaderGroupHandlesBuffer(e.Shaders.GetLength(), GetTransientAllocator());
			pipelineData.pipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, pipelineData.Shaders.GetLength(), shaderGroupHandlesBuffer);
			GTSL::StaticVector<MemberInfo, 8> tablePerGroup[4];

			for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
				auto& groupData = rtPipelineData.ShaderGroups[shaderGroupIndex];
				for (uint32 i = 0; i < groupData.ShaderCount; ++i, ++shaderCount) {
					auto& entry = rtPipelineData.ShaderGroups[shaderGroupIndex].Instances.EmplaceBack();
					tablePerGroup[shaderGroupIndex].EmplaceBack(&entry.ShaderHandle, u8"ShaderHandle", u8"shaderHandle");
					tablePerGroup[shaderGroupIndex].EmplaceBack(&entry.Elements.EmplaceBack(), u8"ptr_t", u8"materialData");
				}
			}

			GTSL::StaticVector<MemberInfo, 4> tables{
				{ &rtPipelineData.ShaderGroups[0].TableHandle, tablePerGroup[0], u8"RayGenTable", u8"rayGenTable", renderSystem->GetShaderGroupBaseAlignment()},
				{ &rtPipelineData.ShaderGroups[1].TableHandle, tablePerGroup[1], u8"ClosestHitTable", u8"closestHitTable", renderSystem->GetShaderGroupBaseAlignment()},
				{ &rtPipelineData.ShaderGroups[2].TableHandle, tablePerGroup[2], u8"MissTable", u8"missTable", renderSystem->GetShaderGroupBaseAlignment()},
				{ &rtPipelineData.ShaderGroups[3].TableHandle, tablePerGroup[3], u8"CallableTable", u8"callableTable", renderSystem->GetShaderGroupBaseAlignment()},
			};
			auto sbtMemeber = CreateMember(GTSL::StaticString<128>(u8"global") << u8"." << GTSL::StringView(shader_group_info.Name), u8"ShaderTableData", tables);
			pipelineData.ShaderBindingTableBuffer = renderSystem->CreateBuffer(GetSize(sbtMemeber), GAL::BufferUses::SHADER_BINDING_TABLE, true, false, pipelineData.ShaderBindingTableBuffer);

			auto bWK = GetBufferWriteKey(renderSystem, pipelineData.ShaderBindingTableBuffer, sbtMemeber);

			for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
				auto& groupData = rtPipelineData.ShaderGroups[shaderGroupIndex];
				auto table = bWK[groupData.TableHandle];
				for (uint32 i = 0; i < groupData.ShaderCount; ++i, ++shaderCount) {
					table[groupData.Instances[i].ShaderHandle] = shaderGroupHandlesBuffer[shaderCount];
					table[groupData.Instances[i].Elements[0]] = GAL::DeviceAddress(0); //todo: check updatability of buffer

					uint64 shaderHandleHash = 0; GTSL::StaticString<128> string(u8"S.H: "); string += shader_group_info.Name; string << u8", "; string += shaders[pipelineData.Shaders[shaderCount]].Name << u8": ";

					for (uint32 j = 0; j < 4; ++j) {
						uint64 val = reinterpret_cast<uint64*>(&shaderGroupHandlesBuffer[shaderCount])[j];
						if (j) { string += U'-'; } GTSL::ToString(string, val);
					}

					shaderHandleHash = quickhash64({ 32, reinterpret_cast<byte*>(&shaderGroupHandlesBuffer[shaderCount]) });

					BE_LOG_MESSAGE(string);

					if(auto r = shaderHandlesDebugMap.TryEmplace(shaderHandleHash, shaders[pipelineData.Shaders[shaderCount]].Name); !r) {
						BE_LOG_ERROR(u8"Could not emplace ", string);
					}
				}
			}

			PrintMember(sbtMemeber, pipelineData.ShaderBindingTableBuffer, renderSystem);
		}
	}
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo) {

	if (auto t = textures.TryEmplace(Id(createTextureInfo.TextureName))) {
		t.Get().Index = textureIndex++;
		auto textureLoadInfo = TextureLoadInfo(RenderAllocation());
		createTextureInfo.TextureResourceManager->LoadTextureInfo(createTextureInfo.GameInstance, Id(createTextureInfo.TextureName), onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));
		t.Get().Resource = makeResource();
		return t.Get().Index;
	}
	else {
		return t.Get().Index;
	}
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

	auto& texture = textures[textureInfo.Name];

	for(uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		WriteBinding(renderSystem, textureSubsetsHandle, loadInfo.TextureHandle, texture.Index, f);
	}

	signalDependencyToResource(texture.Resource);
}

WorldRendererPipeline::WorldRendererPipeline(const InitializeInfo& initialize_info) : RenderPipeline(initialize_info, u8"WorldRendererPipeline"), meshes(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator()), spherePositionsAndRadius(16, GetPersistentAllocator()) {
	auto* renderSystem = initialize_info.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");
	auto* renderOrchestrator = initialize_info.ApplicationManager->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");

	rayTracing = BE::Application::Get()->GetOption(u8"rayTracing");

	onStaticMeshInfoLoadHandle = initialize_info.ApplicationManager->StoreDynamicTask(this, u8"OnStaticMeshInfoLoad",
		DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager", AccessTypes::READ_WRITE),
			TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE)),
		&WorldRendererPipeline::onStaticMeshInfoLoaded
	);

	onStaticMeshLoadHandle = initialize_info.ApplicationManager->StoreDynamicTask(this, u8"OnStaticMeshLoad",
		DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem", AccessTypes::READ_WRITE),
			TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup"),
			TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")),
		&WorldRendererPipeline::onStaticMeshLoaded);

	OnAddMesh = initialize_info.ApplicationManager->StoreDynamicTask(this, u8"OnAddMesh",
		DependencyBlock(TypedDependency<StaticMeshResourceManager>(u8"StaticMeshResourceManager"),
			TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"),
			TypedDependency<RenderSystem>(u8"RenderSystem"),
			TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup")),
		&WorldRendererPipeline::onAddMesh);

	OnUpdateMesh = initialize_info.ApplicationManager->StoreDynamicTask(this, u8"OnUpdateMesh",
		DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator"))
		, &WorldRendererPipeline::updateMesh);

	initialize_info.ApplicationManager->AddTask(this, u8"renderSetup", &WorldRendererPipeline::preRender, DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem"), TypedDependency<RenderOrchestrator>(u8"RenderOrchestrator")), u8"RenderSetup", u8"Render");

	GTSL::StaticVector<RenderOrchestrator::MemberInfo, 8> members;
	members.EmplaceBack(&matrixUniformBufferMemberHandle, u8"matrix4f", u8"transform");
	members.EmplaceBack(&vertexBufferReferenceHandle, u8"ptr_t", u8"vertexBuffer");
	members.EmplaceBack(&indexBufferReferenceHandle, u8"ptr_t", u8"indexBuffer");
	members.EmplaceBack(&materialInstance, u8"uint32", u8"materialInstanceIndex");

	staticMeshInstanceDataStruct = renderOrchestrator->CreateMember(u8"global", u8"StaticMeshData", members);
	meshDataBuffer = renderOrchestrator->CreateBuffer(renderSystem, staticMeshInstanceDataStruct);

	if (rayTracing) {
		for (uint32 i = 0; i < renderSystem->GetPipelinedFrames(); ++i) {
			renderOrchestrator->buildCommandList[i] = renderSystem->CreateCommandList(u8"Acceleration structure build command list", GAL::QueueTypes::COMPUTE);
		}

		topLevelAccelerationStructure = renderSystem->CreateTopLevelAccelerationStructure(16);

		//add node
		RenderOrchestrator::PassData pass_data;
		pass_data.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		pass_data.WriteAttachments.EmplaceBack(u8"Color");
		auto renderPassLayerHandle = renderOrchestrator->AddRenderPass(u8"RayTraceRenderPass", renderOrchestrator->GetCameraDataLayer(), renderSystem, pass_data, initialize_info.ApplicationManager);

		auto rayTraceShaderGroupHandle = renderOrchestrator->CreateShaderGroup(u8"rayTraceShaderGroup");
		renderOrchestrator->addPipelineBindNode(renderPassLayerHandle, renderOrchestrator->GetCameraDataLayer(), rayTraceShaderGroupHandle); //TODO:
		auto rayTraceNode = renderOrchestrator->addRayTraceNode(renderPassLayerHandle, renderOrchestrator->GetCameraDataLayer(), rayTraceShaderGroupHandle); //TODO:

		RenderOrchestrator::MemberHandle traceRayParameters, staticMeshDatas;

		//r.AccelerationStructure r.RayFlags r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, r.tMin, r.tMax
		GTSL::StaticVector<RenderOrchestrator::MemberInfo, 8> traceRayParameterDataMembers{ { &Acc, u8"uint64", u8"accelerationStructure" }, { &RayFlags, u8"uint32", u8"rayFlags" }, {&RecordOffset, u8"uint32", u8"recordOffset"}, {&RecordStride, u8"uint32", u8"recordStride"}, {&MissIndex, u8"uint32", u8"missIndex"}, {&tMin, u8"float32", u8"tMin"}, {&tMax, u8"float32", u8"tMax"} };
		auto traceRayParameterDataHandle = renderOrchestrator->CreateMember(u8"global", u8"TraceRayParameterData", traceRayParameterDataMembers);
		GTSL::StaticVector<RenderOrchestrator::MemberInfo, 8> rayTraceDataMembers{ { &traceRayParameters, u8"TraceRayParameterData", u8"traceRayParameters"}, {&staticMeshDatas, u8"StaticMeshData*", u8"staticMeshes"} };
		auto rayTraceDataMember = renderOrchestrator->CreateMember(u8"global", u8"RayTraceData", rayTraceDataMembers);
		renderOrchestrator->BindToNode(renderSystem, rayTraceNode, rayTraceDataMember);

		auto bwk = renderOrchestrator->GetBufferWriteKey(renderSystem, rayTraceNode, rayTraceDataMember);
		bwk[traceRayParameters][Acc] = topLevelAccelerationStructure;
		bwk[traceRayParameters][RayFlags] = 0u;
		bwk[traceRayParameters][RecordOffset] = 0u;
		bwk[traceRayParameters][RecordStride] = 0u;
		bwk[traceRayParameters][MissIndex] = 0u;
		bwk[traceRayParameters][tMin] = 0.0f;
		bwk[traceRayParameters][tMax] = 100.0f;
		bwk[staticMeshDatas] = meshDataBuffer;
	}

	//for (uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		//WriteBinding(topLevelAsHandle, 0, renderSystem->GetTopLevelAccelerationStructure(f), f);
	//}
}
