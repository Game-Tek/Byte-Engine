#include "RenderOrchestrator.h"

#undef MemoryBarrier

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix.hpp>
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

static bool IsDelim(char32_t tst) {
	const char32_t* DELIMS = U" \n\t\r\f";
	do { // Delimiter string cannot be empty, so don't check for it.  Real code should assert on it.
		if (tst == *DELIMS)
			return true;
		++DELIMS;
	} while (*DELIMS);

	return false;
}

void d(GTSL::StringView string, GTSL::StaticVector<GTSL::StringView, 16>& tokens) {
	auto begin = string.begin();

	while (begin != string.end() && IsDelim(*begin)) { ++begin; }

	while(begin < string.end()) {
		auto tokenBegin = begin;

		do {
			++begin;
		} while (!IsDelim(*begin) && begin != string.end());

		tokens.EmplaceBack(tokenBegin, begin);

		do {
			++begin;
		} while (begin != string.end() && IsDelim(*begin));
	}
}

inline uint32 PRECEDENCE(const GTSL::StringView optor) {
	GTSL::StaticMap<Id, uint8, 16> PRECEDENCE(16);
	PRECEDENCE.Emplace(u8"=", 1);
	PRECEDENCE.Emplace(u8"||", 2);
	PRECEDENCE.Emplace(u8"<", 7); PRECEDENCE.Emplace(u8">", 7); PRECEDENCE.Emplace(u8"<=", 7); PRECEDENCE.Emplace(u8">=", 7); PRECEDENCE.Emplace(u8"==", 7); PRECEDENCE.Emplace(u8"!=", 7);
	PRECEDENCE.Emplace(u8"+", 10); PRECEDENCE.Emplace(u8"-", 10);
	PRECEDENCE.Emplace(u8"*", 20); PRECEDENCE.Emplace(u8"/", 20); PRECEDENCE.Emplace(u8"%", 20);

	return PRECEDENCE[optor];
}

RenderOrchestrator::RenderOrchestrator(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderOrchestrator"),
shaders(16, GetPersistentAllocator()), resources(16, GetPersistentAllocator()), dataKeys(16, GetPersistentAllocator()),
renderingTree(128, GetPersistentAllocator()), renderPasses(16), pipelines(8, GetPersistentAllocator()), shaderGroups(16, GetPersistentAllocator()),
shaderGroupsByName(16, GetPersistentAllocator()), textures(16, GetPersistentAllocator()), attachments(16, GetPersistentAllocator()),
elements(16, GetPersistentAllocator()), sets(16, GetPersistentAllocator()), queuedSetUpdates(1, 8, GetPersistentAllocator()), setLayoutDatas(2, GetPersistentAllocator()), pendingWrites(32, GetPersistentAllocator()), shaderHandlesDebugMap(16, GetPersistentAllocator()), rayTracingSets(16, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

	tag = BE::Application::Get()->GetStringOption(u8"renderTechnique");

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
	tryAddDataType(u8"global", u8"vec2s", 2 * 2);
	tryAddDataType(u8"global", u8"vec2u", 4 * 2);
	tryAddDataType(u8"global", u8"vec2f", 4 * 2);
	tryAddDataType(u8"global", u8"vec3f", 4 * 3);
	tryAddDataType(u8"global", u8"vec4f", 4 * 4);
	tryAddDataType(u8"global", u8"u16vec2", 2 * 2);
	tryAddDataType(u8"global", u8"matrix4f", 4 * 4 * 4);
	tryAddDataType(u8"global", u8"matrix3x4f", 4 * 3 * 4);
	tryAddDataType(u8"global", u8"ptr_t", 8);
	tryAddDataType(u8"global", u8"ShaderHandle", 32);
	tryAddDataType(u8"global", u8"IndirectDispatchCommand", 4 * 3);

	tryAddDataType(u8"global", u8"TextureReference", 4);
	tryAddDataType(u8"global", u8"ImageReference", 4);

	{
		uint64 allocatedSize;
		GetPersistentAllocator().Allocate(1024 * 8, 32, reinterpret_cast<void**>(&buffer[0]), &allocatedSize); //TODO: free
	}

	onTextureInfoLoadHandle = initializeInfo.ApplicationManager->RegisterTask(this, u8"onTextureInfoLoad", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureInfoLoad);
	onTextureLoadHandle = initializeInfo.ApplicationManager->RegisterTask(this, u8"loadTexture", DependencyBlock(TypedDependency<TextureResourceManager>(u8"TextureResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onTextureLoad);

	onShaderInfosLoadHandle = initializeInfo.ApplicationManager->RegisterTask(this, u8"onShaderGroupInfoLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager")), &RenderOrchestrator::onShaderInfosLoaded);
	onShaderGroupLoadHandle = initializeInfo.ApplicationManager->RegisterTask(this, u8"onShaderGroupLoad", DependencyBlock(TypedDependency<ShaderResourceManager>(u8"ShaderResourceManager"), TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::onShadersLoaded);

	initializeInfo.ApplicationManager->EnqueueScheduledTask(initializeInfo.ApplicationManager->RegisterTask(this, SETUP_TASK_NAME, DependencyBlock(), &RenderOrchestrator::Setup, u8"GameplayEnd", u8"RenderSetup"));
	initializeInfo.ApplicationManager->EnqueueScheduledTask(initializeInfo.ApplicationManager->RegisterTask(this, RENDER_TASK_NAME, DependencyBlock(TypedDependency<RenderSystem>(u8"RenderSystem")), &RenderOrchestrator::Render, u8"Render", u8"Render"));

	{
		const auto taskDependencies = GTSL::StaticVector<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE } };
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
		CreateMember2(u8"global", u8"GlobalData", GLOBAL_DATA);
		globalDataDataKey = CreateDataKey(renderSystem, u8"global", u8"GlobalData");
		globalData = AddDataNode({}, u8"GlobalData", globalDataDataKey);
	}

	{
		cameraMatricesHandle = CreateMember2(u8"global", u8"CameraData", CAMERA_DATA);
		cameraDataKeyHandle = CreateDataKey(renderSystem, u8"global", u8"CameraData");
		//cameraDataNode = AddDataNode(globalData, u8"CameraData", cameraDataKeyHandle);
	}

	if constexpr (BE_DEBUG) {
		pipelineStages |= BE::Application::Get()->GetBoolOption(u8"debugSync") ? GAL::PipelineStages::ALL_GRAPHICS : GAL::PipelineStage(0);
	}

	{
		if (tag == GTSL::ShortString<16>(u8"Forward")) {
			AddAttachment(u8"Color", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Normal", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"WorldPosition", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
		} else if(tag == GTSL::ShortString<16>(u8"Visibility")) {
			AddAttachment(u8"Color", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Visibility", 32, 2, GAL::ComponentType::INT, GAL::TextureType::COLOR);
		}

		AddAttachment(u8"RenderDepth", 32, 1, GAL::ComponentType::FLOAT, GAL::TextureType::DEPTH);
	}

	for (uint32 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		graphicsCommandLists[f] = renderSystem->CreateCommandList(u8"Command List", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT);
		graphicsWorkloadHandle[f] = renderSystem->CreateWorkload(u8"Frame work", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT);
		imageAcquisitionWorkloadHandles[f] = renderSystem->CreateWorkload(u8"Swpachain Image Acquisition", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::TRANSFER);
		transferCommandList[f] = renderSystem->CreateCommandList(u8"Transfer Command List", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::TRANSFER);
	}

	renderPassesGuide.EmplaceBack(u8"ForwardRenderPass");
	renderPassesGuide.EmplaceBack(u8"DirectionalShadow");
	renderPassesGuide.EmplaceBack(u8"UI");
	renderPassesGuide.EmplaceBack(u8"GammaCorrection");
}

void RenderOrchestrator::Setup(TaskInfo taskInfo) {
}

template<typename K, typename V, class ALLOC>
void Skim(GTSL::HashMap<K, V, ALLOC>& hash_map, auto predicate) {
	GTSL::StaticVector<uint64, 512> toSkim;
	GTSL::PairForEach(hash_map, [&](K key, V& val) { if (predicate(val)) { toSkim.EmplaceBack(key); } });
	for (auto e : toSkim) { hash_map.Remove(e); }
}

void RenderOrchestrator::Render(TaskInfo taskInfo, RenderSystem* renderSystem) {
	const uint8 currentFrame = renderSystem->GetCurrentFrame(); auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();

	GTSL::Extent2D renderArea = renderSystem->GetRenderExtent();

	renderSystem->Wait(graphicsWorkloadHandle[currentFrame]); // We HAVE to wait or else descriptor update fails because command list may be in use

	if (auto res = renderSystem->AcquireImage(imageAcquisitionWorkloadHandles[currentFrame]); res || sizeHistory[currentFrame] != sizeHistory[beforeFrame]) {
		OnResize(renderSystem, res.Get());
		renderArea = res.Get();
	}

	updateDescriptors(taskInfo);

	renderSystem->StartCommandList(graphicsCommandLists[currentFrame]);

	auto& commandBuffer = *renderSystem->GetCommandList(graphicsCommandLists[currentFrame]);

	BindSet(renderSystem, commandBuffer, globalBindingsSet, GAL::ShaderStages::VERTEX | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN);

	Id resultAttachment;

	{
		auto* cameraSystem = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem");

		auto fovs = cameraSystem->GetFieldOfViews();

		if (fovs.ElementCount()) {
			//SetNodeState(cameraDataNode, true); // Set state on data key, to fullfil resource counts
			auto fov = cameraSystem->GetFieldOfViews()[0]; auto aspectRatio = static_cast<float32>(renderArea.Width) / static_cast<float32>(renderArea.Height);

			float32 nearValue = 0.01f, farValue = 1000.0f;

			if constexpr (INVERSE_Z) {
				std::swap(nearValue, farValue);
			}

			GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(fov, aspectRatio, nearValue, farValue);
			projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;

			auto invertedProjectionMatrix = GTSL::Math::BuildInvertedPerspectiveMatrix(fov, aspectRatio, nearValue, farValue);
			invertedProjectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f;

			auto viewMatrix = cameraSystem->GetCameraTransform();

			auto key = GetBufferWriteKey(renderSystem, cameraDataKeyHandle);
			key[u8"view"] = viewMatrix;
			key[u8"proj"] = projectionMatrix;
			key[u8"viewInverse"] = GTSL::Math::Inverse(viewMatrix);
			key[u8"projInverse"] = invertedProjectionMatrix;
			key[u8"vp"] = projectionMatrix * viewMatrix;
			key[u8"vpInverse"] = GTSL::Math::Inverse(viewMatrix) * invertedProjectionMatrix;
			key[u8"near"] = nearValue;
			key[u8"far"] = farValue;
			key[u8"extent"] = renderArea;
		}
		else { //disable rendering for everything which depends on this view
			//SetNodeState(cameraDataNode, false);
		}
	}

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

	RenderState renderState;

	auto updateRenderStages = [&](const GAL::ShaderStage stages) {
		renderState.ShaderStages = stages;
	};

	using RTT = decltype(renderingTree);

	auto processExecutionString = [renderArea](const GTSL::StringView execution) {
		GTSL::StaticVector<GTSL::StringView, 16> tokens;

		GTSL::StaticVector<GTSL::StringView, 16> operators;

		GTSL::StaticVector<GTSL::StaticString<64>, 16> output;

		d(execution, tokens);

		while (tokens) {
			auto token = tokens.back(); tokens.PopBack();

			if (GTSL::IsNumber(token) or IsAnyOf(token, u8"windowExtent")) {
				output.EmplaceBack(token);
			}
			else { //is an operator
				while (operators && PRECEDENCE(operators.back()) > PRECEDENCE(token)) {
					output.EmplaceBack(operators.back());
					operators.PopBack();
				}

				operators.EmplaceBack(token);
			}
		}

		while (operators) {
			output.EmplaceBack(operators.back());
			operators.PopBack();
		}

		GTSL::StaticVector<GTSL::Extent3D, 8> numbers;

		//evaluate
		for (uint32 i = 0; i < output; ++i) {
			auto token = output[i];
			if (GTSL::IsNumber(token) or IsAnyOf(token, u8"windowExtent")) {
				if (token == u8"windowExtent") {
					numbers.EmplaceBack(renderArea);
				}
				else {
					numbers.EmplaceBack(GTSL::ToNumber<uint16>(token).Get());
				}
			}
			else { //operator
				auto a = numbers.back(); numbers.PopBack();

				auto b = numbers.back(); numbers.PopBack();

				switch (Hash(token)) {
				case GTSL::Hash(u8"+"): numbers.EmplaceBack(a + b); break;
				case GTSL::Hash(u8"-"): numbers.EmplaceBack(a - b); break;
				case GTSL::Hash(u8"*"): numbers.EmplaceBack(a * b); break;
				case GTSL::Hash(u8"/"): numbers.EmplaceBack(a / b); break;
				}
			}
		}

		return numbers.back();
	};

	auto runLevel = [&](const decltype(renderingTree)::Key key, const uint32_t level) -> void {
		DataStreamHandle dataStreamHandle = {};

		const auto& baseData = renderingTree.GetAlpha(key);

		printNode(key, level, false);

		switch (renderingTree.GetNodeType(key)) {
		case RTT::GetTypeIndex<LayerData>(): {
			const LayerData& layerData = renderingTree.GetClass<LayerData>(key);

			if constexpr (BE_DEBUG) {
				commandBuffer.BeginRegion(renderSystem->GetRenderDevice(), baseData.Name);
			}

			if (layerData.DataKey) {
				dataStreamHandle = renderState.AddDataStream();
				GAL::DeviceAddress address;
				const auto& dataKey = dataKeys[layerData.DataKey()];

				if (renderSystem->IsUpdatable(dataKey.Buffer)) {
					address = renderSystem->GetBufferAddress(dataKey.Buffer, renderSystem->GetCurrentFrame(), true);
				}
				else {
					address = renderSystem->GetBufferAddress(dataKey.Buffer);
				}

				auto& setLayout = setLayoutDatas[globalSetLayout()]; address += dataKey.Offset;
				commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), setLayout.PipelineLayout, dataStreamHandle() * 8, GTSL::Range(8, reinterpret_cast<const byte*>(&address)), setLayout.Stage);
			}
			break;
		}
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

			renderState.BoundPipelineIndex = pipelineIndex;

			commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelines[pipelineIndex].pipeline, renderState.ShaderStages);
			break;
		}
		case RTT::GetTypeIndex<DispatchData>(): {
			const DispatchData& dispatchData = renderingTree.GetClass<DispatchData>(key);

			const auto& execution = pipelines[renderState.BoundPipelineIndex].ExecutionString;

			auto executionExtent = processExecutionString(execution);

			commandBuffer.Dispatch(renderSystem->GetRenderDevice(), executionExtent);

			break;
		}
		case RTT::GetTypeIndex<RayTraceData>(): {
			const RayTraceData& rayTraceData = renderingTree.GetClass<RayTraceData>(key);
			const auto& pipelineData = pipelines[shaderGroups[rayTraceData.ShaderGroupIndex].RTPipelineIndex];
			CommandList::ShaderTableDescriptor shaderTableDescriptors[4];
			for (uint32 i = 0, offset = 0; i < 3; ++i) {
				shaderTableDescriptors[i].Entries = pipelineData.RayTracingData.ShaderGroups[i].ShaderCount;
				shaderTableDescriptors[i].EntrySize = GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
				shaderTableDescriptors[i].Address = GetAddress(renderSystem, pipelineData.ShaderBindingTableBuffer) + offset;

				offset += GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
			}

			auto executionExtent = processExecutionString(pipelineData.ExecutionString);

			commandBuffer.TraceRays(renderSystem->GetRenderDevice(), GTSL::Range(4, shaderTableDescriptors), executionExtent);

			break;
		}
		case RTT::GetTypeIndex<VertexBufferBindData>(): {
			const VertexBufferBindData& meshData = renderingTree.GetClass<VertexBufferBindData>(key);

			auto buffer = renderSystem->GetBuffer(meshData.Handle);

			GTSL::StaticVector<GPUBuffer, 8> buffers;

			for (uint32 i = 0; i < meshData.Offsets.GetLength(); ++i) {
				buffers.EmplaceBack(buffer);
			}

			commandBuffer.BindVertexBuffers(renderSystem->GetRenderDevice(), buffers, meshData.Offsets, meshData.VertexSize * meshData.VertexCount, meshData.VertexSize);
			break;
		}
		case RTT::GetTypeIndex<IndexBufferBindData>(): {
			const IndexBufferBindData& meshData = renderingTree.GetClass<IndexBufferBindData>(key);
			commandBuffer.BindIndexBuffer(renderSystem->GetRenderDevice(), renderSystem->GetBuffer(meshData.BufferHandle), 0, meshData.IndexCount, meshData.IndexType);
			break;
		}
		case RTT::GetTypeIndex<MeshData>(): {
			const MeshData& meshData = renderingTree.GetClass<MeshData>(key);
			commandBuffer.DrawIndexed(renderSystem->GetRenderDevice(), meshData.IndexCount, meshData.InstanceCount, meshData.InstanceIndex, meshData.IndexOffset, meshData.VertexOffset);
			break;
		}
		case RTT::GetTypeIndex<DrawData>(): {
			const DrawData& draw_data = renderingTree.GetClass<DrawData>(key);
			commandBuffer.Draw(renderSystem->GetRenderDevice(), 6, draw_data.InstanceCount);
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
						e.LoadOperation = renderPassData.Attachments[i].LoadOperation;
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
		//BE_LOG_WARNING(u8"Leaving node ", key);

		switch (renderingTree.GetNodeType(key)) {
		case RTT::GetTypeIndex<LayerData>(): {
			renderState.PopData();

			if constexpr (BE_DEBUG) {
				commandBuffer.EndRegion(renderSystem->GetRenderDevice());
			}

			break;
		}
		case RTT::GetTypeIndex<RenderPassData>(): {
			auto& renderPassData = renderingTree.GetClass<RenderPassData>(key);
			if (renderPassData.Type == PassType::RASTER) {
				commandBuffer.EndRenderPass(renderSystem->GetRenderDevice());
			}

			break;
		}
		default: break;
		}
	};

	auto onFail = [&](const uint32 nodeHandle, uint32 level) {
		return;

		switch (renderingTree.GetNodeType(nodeHandle)) {
		case decltype(renderingTree)::GetTypeIndex<LayerData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: LayerData", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<PipelineBindData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: Pipeline Bind", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<MeshData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: Mesh Data", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<VertexBufferBindData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: Vertex Buffer Bind", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<IndexBufferBindData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: Index Buffer Bind", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		case decltype(renderingTree)::GetTypeIndex<RenderPassData>(): {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: Render Pass", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		default: {
			BE_LOG_WARNING(u8"Node index: ", nodeHandle, u8", Type: null", u8", Name: ", getNode(nodeHandle).Name);
			break;
		}
		}
	};

	ForEachBeta(renderingTree, runLevel, endNode, onFail);

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
	CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

	if (resultAttachment) {
		auto& attachment = attachments.At(resultAttachment);

		commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, attachment.AccessType,
			GAL::AccessTypes::READ, CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout[currentFrame],
			GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor } } }, GetTransientAllocator());

		updateImage(currentFrame, attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);

		commandBuffer.BlitTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor, sizeHistory[currentFrame], *renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat(), GTSL::Extent3D(renderSystem->GetRenderExtent()));
	}

	commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(), GAL::TextureLayout::TRANSFER_DESTINATION,
	GAL::TextureLayout::PRESENTATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

	renderSystem->EndCommandList(graphicsCommandLists[currentFrame]);

	renderSystem->StartCommandList(transferCommandList[currentFrame]);
	renderSystem->EndCommandList(transferCommandList[currentFrame]);

	{
		GTSL::StaticVector<RenderSystem::CommandListHandle, 8> commandLists;
		GTSL::StaticVector<RenderSystem::WorkloadHandle, 8> workloads;

		commandLists.EmplaceBack(transferCommandList[renderSystem->GetCurrentFrame()]);

		workloads.EmplaceBack(buildAccelerationStructuresWorkloadHandle[renderSystem->GetCurrentFrame()]);
		if (BE::Application::Get()->GetBoolOption(u8"rayTracing")) {
		}

		workloads.EmplaceBack(imageAcquisitionWorkloadHandles[currentFrame]);
		workloads.EmplaceBack(graphicsWorkloadHandle[currentFrame]);

		commandLists.EmplaceBack(graphicsCommandLists[currentFrame]);

		renderSystem->Submit(GAL::QueueTypes::GRAPHICS, { { { transferCommandList[currentFrame] }, {}, { graphicsWorkloadHandle[currentFrame]}}, {{graphicsCommandLists[currentFrame]}, workloads, {graphicsWorkloadHandle[currentFrame]}}}, graphicsWorkloadHandle[renderSystem->GetCurrentFrame()]); // Wait on image acquisition to render maybe, //Signal grpahics workload

		renderSystem->Present({ graphicsWorkloadHandle[currentFrame] }); // Wait on graphics work to present
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
		shaderGroup.ResourceHandle = makeResource(GTSL::StringView(shader_group_name));
		addDependencyOnResource(shaderGroup.ResourceHandle); // Add dependency the pipeline itself
		shaderGroup.Buffer = MakeDataKey();
	} else {
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
		attachment.ClearColor = GTSL::RGBA(INVERSE_Z ? 0.0f: 1.0f, 0, 0, 0);
	}

	attachment.Layout[0] = GAL::TextureLayout::UNDEFINED; attachment.Layout[1] = GAL::TextureLayout::UNDEFINED; attachment.Layout[2] = GAL::TextureLayout::UNDEFINED;
	attachment.AccessType = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
	attachment.ImageIndex = imageIndex++; ++textureIndex;

	attachments.Emplace(attachmentName, attachment);
}

RenderOrchestrator::NodeHandle RenderOrchestrator::AddRenderPass(GTSL::StringView renderPassName, NodeHandle parent_node_handle, RenderSystem* renderSystem, PassData passData) {
	GTSL::StaticVector<MemberInfo, 16> members;

	for (auto& e : passData.Attachments) {
		if(!(e.Access & GAL::AccessTypes::WRITE)) {
			members.EmplaceBack(nullptr, u8"TextureReference", GTSL::StringView(e.Name));
		} else {
			members.EmplaceBack(nullptr, u8"ImageReference", GTSL::StringView(e.Name));			
		}
	}

	auto member = CreateMember(u8"global", renderPassName, members);

	NodeHandle leftNodeHandle(0xFFFFFFFF);

	{ // Guarantee render pass order in render tree, TODO: check render pass level or else this will fail
		auto pos = renderPassesGuide.Find(Id(renderPassName));

		if(pos) {
			if(pos.Get() > 0) {
				for (uint32 i = pos.Get() - 1; i != ~0u; --i) {
					if (auto r = renderPasses2.Find(renderPassesGuide[i])) { //If render pass was already created grab it's handle to correctly place render passes, if it doesn't yet exists TODO: 
						leftNodeHandle = renderPasses2[renderPassesGuide[i]];
						break;
					}
				}
			}
		}
	}

	auto renderPassDataNode = AddDataNode(renderPassName, leftNodeHandle, parent_node_handle, member);
	NodeHandle renderPassNodeHandle = addInternalNode<RenderPassData>(Hash(renderPassName), renderPassDataNode);
	RenderPassData& renderPass = getPrivateNode<RenderPassData>(renderPassNodeHandle);

	renderPasses.Emplace(renderPassName, renderPassNodeHandle);
	renderPasses2.Emplace(renderPassName, renderPassDataNode);

	auto renderPassIndex = renderPassesInOrder.GetLength();

	renderPassesInOrder.EmplaceBack(renderPassNodeHandle);

	renderPass.ResourceHandle = makeResource(renderPassName);
	addDependencyOnResource(renderPass.ResourceHandle); //add dependency on render pass texture creation

	BindToNode(renderPassNodeHandle, renderPass.ResourceHandle);

	getNode(renderPassNodeHandle).Name = GTSL::StringView(renderPassName);

	PassType renderPassType;
	GAL::PipelineStage pipelineStage;

	NodeHandle resultHandle = renderPassNodeHandle;

	switch (passData.PassType) {
	case PassType::RASTER: {
		renderPassType = PassType::RASTER;
		pipelineStage = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;

		for (const auto& e : passData.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Access = e.Access;

			if (renderPassIndex) {
				attachmentData.LoadOperation = GAL::Operations::DO; //UNDEFINED ? 
			} else {
				attachmentData.LoadOperation = GAL::Operations::CLEAR; //UNDEFINED ?
			}

			if (e.Access & GAL::AccessTypes::READ) {
				attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
				attachmentData.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;
			} else {
				attachmentData.Layout = GAL::TextureLayout::ATTACHMENT;
				attachmentData.ConsumingStages = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;
			}

		}

		break;
	}
	case PassType::COMPUTE: {
		renderPassType = PassType::COMPUTE;
		pipelineStage = GAL::PipelineStages::COMPUTE;

		for (const auto& e : passData.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Access = e.Access;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE;

			if (e.Access & GAL::AccessTypes::READ) {
				attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			} else {
				attachmentData.Layout = GAL::TextureLayout::GENERAL;
			}

		}

		auto sgh = CreateShaderGroup(renderPassName);
		auto pipelineBindNode = addPipelineBindNode(renderPassNodeHandle, sgh);
		resultHandle = addInternalNode<DispatchData>(Hash(renderPassName), pipelineBindNode);

		break;
	}
	case PassType::RAY_TRACING: {
		renderPassType = PassType::RAY_TRACING;
		pipelineStage = GAL::PipelineStages::RAY_TRACING;

		for (const auto& e : passData.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Access = e.Access;

			attachmentData.ConsumingStages = GAL::PipelineStages::RAY_TRACING;

			if (e.Access & GAL::AccessTypes::READ) {
				attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			}
			else {
				attachmentData.Layout = GAL::TextureLayout::ATTACHMENT;
			}

		}

		break;
	}
	}

	renderPass.Type = renderPassType;
	renderPass.PipelineStages = pipelineStage;

	auto bwk = GetBufferWriteKey(renderSystem, renderPassDataNode);

	for (auto i = 0u; i < passData.Attachments.GetLength(); ++i) {
		bwk[GTSL::StringView(passData.Attachments[i].Name)] = attachments[renderPass.Attachments[i].Name].ImageIndex;
	}

	return resultHandle;
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

		attachment.Layout[currentFrame] = GAL::TextureLayout::UNDEFINED;
	};

	if (sizeHistory[currentFrame] != newSize) {
		sizeHistory[currentFrame] = newSize;
		GTSL::ForEach(attachments, resize);
	}

	for (const auto apiRenderPassData : renderPasses) {
		auto& layer = getPrivateNode<RenderPassData>(apiRenderPassData);
		signalDependencyToResource(layer.ResourceHandle);
	}
}

void RenderOrchestrator::ToggleRenderPass(NodeHandle renderPassName, bool enable)
{
	if (renderPassName) {
		auto& renderPassNode = getPrivateNode<RenderPassData>(renderPassName);

		switch (renderPassNode.Type) {
		case PassType::RASTER: break;
		case PassType::COMPUTE: break;
		case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetBoolOption(u8"rayTracing"); break; // Enable render pass only if function is enaled in settings
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

	GTSL::HashMap<Id, StructElement, BE::TAR> parameters(8, GetTransientAllocator());

	GTSL::StaticVector<GTSL::StaticVector<GAL::Pipeline::VertexElement, 8>, 8> vertexStreams;
	struct ShaderBundleData {
		GTSL::StaticVector<uint32, 8> Shaders;
		GAL::ShaderStage Stage;
		uint32 PipelineIndex = 0;
		GTSL::ShortString<32> Set;
		bool Transparency = false;
	};
	GTSL::StaticVector<ShaderBundleData, 4> shaderBundles;
	GTSL::StaticVector<MemberInfo, 16> members;
	GTSL::KeyMap<uint64, BE::TAR> loadedShadersMap(8, GetTransientAllocator()); //todo: differentiate hash from hash + name, since a different hash could be interpreted as a different shader, when in reality it functionally represents the same shader but with different code

	GTSL::StaticString<64> executionString;

	shaderGroupNotify(this, renderSystem);

	for (uint8 ai = 0;  auto& a : shader_group_info.VertexElements) {
		auto& stream = vertexStreams.EmplaceBack();

		for (auto& b : a) {
			GAL::ShaderDataType type;

			switch (Hash(b.Type)) {
			case GTSL::Hash(u8"vec2f"): type = GAL::ShaderDataType::FLOAT2; break;
			case GTSL::Hash(u8"vec3f"): type = GAL::ShaderDataType::FLOAT3; break;
			case GTSL::Hash(u8"vec4f"): type = GAL::ShaderDataType::FLOAT4; break;
			}

			stream.EmplaceBack(GAL::Pipeline::VertexElement{ GTSL::ShortString<32>(b.Name.c_str()), type, ai++ });
		}
	}

	for (uint32 offset = 0, si = 0; si < shader_group_info.Shaders; offset += shader_group_info.Shaders[si].Size, ++si) {
		const auto& s = shader_group_info.Shaders[si];

		if (Contains(PermutationManager::ShaderTag(u8"Domain", u8"World"), s.Tags.GetRange())) {
			if (!Contains(PermutationManager::ShaderTag(u8"RenderTechnique", tag), s.Tags.GetRange())) {
				continue;
			}
		}

		//if shader doesn't contain tag don't use it, tags are used to filter shaders usually based on render technique used

		if (auto shader = shaders.TryEmplace(s.Hash)) {
			shader.Get().Shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range(s.Size, shaderLoadInfo.Buffer.GetData() + offset));
			shader.Get().Type = s.Type;
			shader.Get().Name = s.Name;
			loadedShadersMap.Emplace(s.Hash);
		}

		if (auto executionExists = GTSL::Find(s.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Execution"; })) {
			executionString = executionExists.Get()->Second;
		}

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
			if(auto r = GTSL::Find(s.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Set"; })) {
				sb.Set = GTSL::StringView(r.Get()->Second);
			}

		} else {
			switch (s.Type) {
			case GAL::ShaderType::VERTEX: break;
			case GAL::ShaderType::FRAGMENT: break; //todo: set transparency
			case GAL::ShaderType::COMPUTE: break;
			case GAL::ShaderType::TASK: break;
			case GAL::ShaderType::MESH: break;
			case GAL::ShaderType::RAY_GEN: break;
			case GAL::ShaderType::CLOSEST_HIT: break;
			case GAL::ShaderType::ANY_HIT: break;
			case GAL::ShaderType::INTERSECTION: break;
			case GAL::ShaderType::MISS: break;
			case GAL::ShaderType::CALLABLE: break;
			default: ;
			}
		}
	}

	for (uint32 pi = 0; const auto & p : shader_group_info.Parameters) {
		parameters.Emplace(Id(p.Name), p.Type, p.Name, p.Value);
		members.EmplaceBack(MemberInfo{ &sg.ParametersHandles.Emplace(Id(p.Name)), p.Type, p.Name });
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

			Id renderPass = u8"ForwardRenderPass";

			if (auto r = GTSL::Find(shader_group_info.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"RenderPass"; })) {
				renderPass = Id(GTSL::StringView(r.Get()->Second));
			}

			bool transparency = false;

			if (auto r = GTSL::Find(shader_group_info.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Transparency"; })) {
				transparency = r.Get()->Second == GTSL::StringView(u8"True");
			}

			//BUG: if shader group gets processed before render pass it will fail
			const auto& renderPassNode = getPrivateNode<RenderPassData>(renderPasses[renderPass]);

			for (const auto& writeAttachment : renderPassNode.Attachments) {
				if (writeAttachment.Access & GAL::AccessTypes::WRITE) {
					auto& attachment = attachments.At(writeAttachment.Name);
					auto& attachmentState = att.EmplaceBack();
					attachmentState.BlendEnable = transparency; attachmentState.FormatDescriptor = attachment.FormatDescriptor;
				}
			}

			context.Attachments = att;
			pipelineStates.EmplaceBack(context);

			if (!transparency) {
				GAL::Pipeline::PipelineStateBlock::DepthState depth;
				depth.CompareOperation = INVERSE_Z ? GAL::CompareOperation::GREATER : GAL::CompareOperation::LESS;
				pipelineStates.EmplaceBack(depth);
			}

			GAL::Pipeline::PipelineStateBlock::RasterState rasterState;
			rasterState.CullMode = GAL::CullMode::CULL_NONE;
			rasterState.WindingOrder = GAL::WindingOrder::COUNTER_CLOCKWISE;
			pipelineStates.EmplaceBack(rasterState);

			GAL::Pipeline::PipelineStateBlock::ViewportState viewportState;
			viewportState.ViewportCount = 1;
			pipelineStates.EmplaceBack(viewportState);

			auto& vertexState = pipelineStates.EmplaceBack(GAL::Pipeline::PipelineStateBlock::VertexState{});

			GTSL::StaticVector<GTSL::Range<const GAL::Pipeline::VertexElement*>, 8> vertexStreamsRanges;

			for(auto& e : vertexStreams) { vertexStreamsRanges.EmplaceBack(e); }

			vertexState.Vertex.VertexStreams = vertexStreamsRanges;

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

			pipeline.ExecutionString = executionString;

			pipeline.pipeline.InitializeComputePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}

		if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT)) {
			if (!BE::Application::Get()->GetBoolOption(u8"rayTracing")) { continue; }

			if(auto r = rayTracingSets.TryEmplace(Id(e.Set), 0xFFFFFFFFu)) {
				sg.RTPipelineIndex = pipelines.Emplace(GetPersistentAllocator());
				r.Get() = sg.RTPipelineIndex;
			} else {
				sg.RTPipelineIndex = r.Get();
			}

			e.PipelineIndex = sg.RTPipelineIndex;

			auto& pipelineData = pipelines[e.PipelineIndex];

			//add newly loaded shaders to new pipeline update
			for (auto s : e.Shaders) {
				pipelineData.Shaders.EmplaceBack(shader_group_info.Shaders[s].Hash);
			}

			GTSL::Sort(pipelineData.Shaders, [&](uint64 a, uint64 b) { return shaders[a].Type > shaders[b].Type; });

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

			pipelineData.ExecutionString = executionString;

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
		sg.Buffer = CreateDataKey(renderSystem, scope, u8"ShaderParametersData", sg.Buffer);

		for (uint8 ii = 0; auto & i : shader_group_info.Instances) { //TODO: check parameters against stored layout to check if everything is still compatible
			for (uint32 pi = 0; auto & p : i.Parameters) {
				GTSL::StaticString<32> parameterValue;

				//if shader instance has specialized value for param, use that, else, fallback to shader group default value for param
				if (p.Second) {
					parameterValue = p.Second;
				} else {
					parameterValue = parameters[Id(p.First)].DefaultValue;
				}

				switch (Hash(parameters[Id(p.First)].Type)) {
				case GTSL::Hash(u8"TextureReference"): {
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.ApplicationManager;
					createTextureInfo.TextureResourceManager = taskInfo.ApplicationManager->GetSystem<TextureResourceManager>(u8"TextureResourceManager");
					createTextureInfo.TextureName = parameterValue;
					auto textureReference = createTexture(createTextureInfo);

					GetBufferWriteKey(renderSystem, sg.Buffer)[p.First] = textureReference;

					for (auto& e : shaderBundles) {
						addPendingResourceToTexture(Id(parameterValue), sg.ResourceHandle);
					}

					break;
				}
				case GTSL::Hash(u8"ImageReference"): {
					auto textureReference = attachments.TryGet(Id(parameterValue));

					if (textureReference) {
						uint32 textureComponentIndex = textureReference.Get().ImageIndex;

						GetBufferWriteKey(renderSystem, sg.Buffer)[p.First] = textureComponentIndex;
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

		PrintMember(sg.Buffer, renderSystem);
	}

	for (auto& e : shaderBundles) {
		if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE)) {
			if (!BE::Application::Get()->GetBoolOption(u8"rayTracing")) { continue; }
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
			pipelineData.ShaderBindingTableBuffer = CreateDataKey(renderSystem, GTSL::StaticString<128>(u8"global") << u8"." << GTSL::StringView(shader_group_info.Name), u8"ShaderTableData", pipelineData.ShaderBindingTableBuffer, GAL::BufferUses::SHADER_BINDING_TABLE);

			auto bWK = GetBufferWriteKey(renderSystem, pipelineData.ShaderBindingTableBuffer);

			for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
				auto& groupData = rtPipelineData.ShaderGroups[shaderGroupIndex];
				auto table = bWK[tables[shaderGroupIndex].Name];
				for (uint32 i = 0; i < groupData.ShaderCount; ++i, ++shaderCount) {
					table[u8"shaderHandle"] = shaderGroupHandlesBuffer[shaderCount];
					table[u8"materialData"] = GetAddress(renderSystem, sg.Buffer); //todo: wrong

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
		}
	}
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo) {

	if (auto t = textures.TryEmplace(Id(createTextureInfo.TextureName))) {
		t.Get().Index = textureIndex++;
		auto textureLoadInfo = TextureLoadInfo(RenderAllocation());
		createTextureInfo.TextureResourceManager->LoadTextureInfo(createTextureInfo.GameInstance, Id(createTextureInfo.TextureName), onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));
		t.Get().Resource = makeResource(createTextureInfo.TextureName);
		return t.Get().Index;
	}
	else {
		return t.Get().Index;
	}
}

void RenderOrchestrator::onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem* renderSystem,
	TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	GTSL::StaticString<128> name(u8"Texture resource: "); name += GTSL::Range(textureInfo.GetName());

	loadInfo.TextureHandle = renderSystem->CreateTexture(name, textureInfo.Format, textureInfo.Extent, GAL::TextureUses::SAMPLE | GAL::TextureUses::ATTACHMENT, true);

	auto dataBuffer = renderSystem->GetTextureRange(loadInfo.TextureHandle);

	resourceManager->LoadTexture(taskInfo.ApplicationManager, textureInfo, dataBuffer, onTextureLoadHandle, GTSL::MoveRef(loadInfo));
}

void RenderOrchestrator::onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, RenderSystem* renderSystem, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	renderSystem->UpdateTexture(transferCommandList[renderSystem->GetCurrentFrame()], loadInfo.TextureHandle);

	auto& texture = textures[textureInfo.GetName()];

	for(uint8 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		WriteBinding(renderSystem, textureSubsetsHandle, loadInfo.TextureHandle, texture.Index, f);
	}

	signalDependencyToResource(texture.Resource);
}

#include "ByteEngine/MetaStruct.hpp"

//using VisibilityData = meta_struct<member<"shaderGroupLength", uint32>> ;

#define REGISTER_TASK(name) name = GetApplicationManager()->RegisterTask(this, u8"name", name##_DEPENDENCIES, &WorldRendererPipeline::OnUpdateMesh);