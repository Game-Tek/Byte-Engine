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
	GTSL::StaticMap<GTSL::StringView, uint8, 16> PRECEDENCE(16);
	PRECEDENCE.Emplace(u8"=", 1);
	PRECEDENCE.Emplace(u8"||", 2);
	PRECEDENCE.Emplace(u8"<", 7); PRECEDENCE.Emplace(u8">", 7); PRECEDENCE.Emplace(u8"<=", 7); PRECEDENCE.Emplace(u8">=", 7); PRECEDENCE.Emplace(u8"==", 7); PRECEDENCE.Emplace(u8"!=", 7);
	PRECEDENCE.Emplace(u8"+", 10); PRECEDENCE.Emplace(u8"-", 10);
	PRECEDENCE.Emplace(u8"*", 20); PRECEDENCE.Emplace(u8"/", 20); PRECEDENCE.Emplace(u8"%", 20);

	return PRECEDENCE[optor];
}

RenderOrchestrator::RenderOrchestrator(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"RenderOrchestrator"),
rayTracingSets(16, GetPersistentAllocator()), shaderHandlesDebugMap(16, GetPersistentAllocator()), shaders(16, GetPersistentAllocator()),
resources(16, GetPersistentAllocator()), dataKeys(16, GetPersistentAllocator()), dataKeysMap(32, GetPersistentAllocator()), updateKeys(16, GetPersistentAllocator()),
renderingTree(128, GetPersistentAllocator()), renderPassesMap(16, GetPersistentAllocator()), renderPasses(16), pipelines(8, GetPersistentAllocator()),
shaderGroups(16, GetPersistentAllocator()), shaderGroupsByName(16, GetPersistentAllocator()), shaderGroupInstanceByName(16, GetPersistentAllocator()), textures(16, GetPersistentAllocator()), attachments(16, GetPersistentAllocator()), elements(16, GetPersistentAllocator()), sets(16, GetPersistentAllocator()), queuedSetUpdates(1, 8, GetPersistentAllocator()), setLayoutDatas(2, GetPersistentAllocator()), pendingWrites(32, GetPersistentAllocator())
{
	auto* renderSystem = initializeInfo.ApplicationManager->GetSystem<RenderSystem>(u8"RenderSystem");

	tag = BE::Application::Get()->GetConfig()[u8"Rendering"][u8"renderTechnique"];

	//renderBuffers.EmplaceBack().BufferHandle = renderSystem->CreateBuffer(RENDER_DATA_BUFFER_PAGE_SIZE, GAL::BufferUses::STORAGE, true, true, RenderSystem::BufferHandle());

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
	tryAddDataType(u8"global", u8"vec2i", 4 * 2);
	tryAddDataType(u8"global", u8"vec2f", 4 * 2);
	tryAddDataType(u8"global", u8"vec3f", 4 * 3);
	tryAddDataType(u8"global", u8"vec4f", 4 * 4);
	tryAddDataType(u8"global", u8"u16vec2", 2 * 2);
	tryAddDataType(u8"global", u8"matrix4f", 4 * 4 * 4);
	tryAddDataType(u8"global", u8"matrix3x4f", 4 * 3 * 4);
	tryAddDataType(u8"global", u8"ptr_t", 8);
	tryAddDataType(u8"global", u8"ShaderHandle", 32);

	RegisterType(u8"global", u8"IndirectDispatchCommand", INDIRECT_DISPATCH_COMMAND_DATA);
	RegisterType(u8"global", u8"TextureReference", { { u8"uint32", u8"Instance" } });
	RegisterType(u8"global", u8"ImageReference", { { u8"uint32", u8"Instance" } });

	{
		//uint64 allocatedSize;
		//GetPersistentAllocator().Allocate(1024 * 8, 32, reinterpret_cast<void**>(&buffer[0]), &allocatedSize); //TODO: free
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
		subSetInfos.EmplaceBack(SubSetType::READ_TEXTURES, 32, &textureSubsetsHandle);
		subSetInfos.EmplaceBack(SubSetType::WRITE_TEXTURES, 32, &imagesSubsetHandle);
		subSetInfos.EmplaceBack(SubSetType::SAMPLER, 16, &samplersSubsetHandle, samplers);

		globalSetLayout = AddSetLayout(renderSystem, SetLayoutHandle(), subSetInfos);
		globalBindingsSet = AddSet(renderSystem, u8"GlobalData", globalSetLayout, subSetInfos);
	}

	{
		tryAddElement(u8"global", u8"CommonPermutation", ElementData::ElementType::SCOPE);
		RegisterType(u8"global.CommonPermutation", u8"GlobalData", GLOBAL_DATA);
		globalDataDataKey = MakeDataKey(renderSystem, u8"global.CommonPermutation", u8"GlobalData");
		globalData = AddDataNode({}, u8"GlobalData", globalDataDataKey);
		bnoise[0] = createTexture({ u8"bnoise_v_0", GetApplicationManager(), renderSystem, GetApplicationManager()->GetSystem<TextureResourceManager>(u8"TextureResourceManager") });
		bnoise[1] = createTexture({ u8"bnoise_v_1", GetApplicationManager(), renderSystem, GetApplicationManager()->GetSystem<TextureResourceManager>(u8"TextureResourceManager") });
		bnoise[2] = createTexture({ u8"bnoise_v_2", GetApplicationManager(), renderSystem, GetApplicationManager()->GetSystem<TextureResourceManager>(u8"TextureResourceManager") });
		bnoise[3] = createTexture({ u8"bnoise_v_3", GetApplicationManager(), renderSystem, GetApplicationManager()->GetSystem<TextureResourceManager>(u8"TextureResourceManager") });

		auto bwk = GetBufferWriteKey(renderSystem, globalDataDataKey);
		bwk[u8"blueNoise2D"][0] = bnoise[0];
		bwk[u8"blueNoise2D"][1] = bnoise[1];
		bwk[u8"blueNoise2D"][2] = bnoise[2];
		bwk[u8"blueNoise2D"][3] = bnoise[3];
	}

	{
		RegisterType(u8"global.CommonPermutation", u8"ViewData", VIEW_DATA);
		cameraMatricesHandle = RegisterType(u8"global.CommonPermutation", u8"CameraData", CAMERA_DATA);
		cameraDataKeyHandle = MakeDataKey(renderSystem, u8"global.CommonPermutation", u8"CameraData");
		//cameraDataNode = AddDataNode(globalData, u8"CameraData", cameraDataKeyHandle);
	}

	if constexpr (BE_DEBUG) {
		pipelineStages |= BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"debugSync"].GetBool() ? GAL::PipelineStages::ALL_GRAPHICS : GAL::PipelineStage(0);
	}

	{
		if (tag == GTSL::ShortString<16>(u8"Forward")) {
			AddAttachment(u8"Albedo", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Normal", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"WorldSpacePosition", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"ViewSpacePosition", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Lighting", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Roughness", 8, 1, GAL::ComponentType::INT, GAL::TextureType::COLOR);
			AddAttachment(u8"Shadow", 8, 1, GAL::ComponentType::INT, GAL::TextureType::COLOR);
			AddAttachment(u8"AO", 8, 4, GAL::ComponentType::INT, GAL::TextureType::COLOR);
		} else if(tag == GTSL::ShortString<16>(u8"Visibility")) {
			AddAttachment(u8"Albedo", 16, 4, GAL::ComponentType::FLOAT, GAL::TextureType::COLOR);
			AddAttachment(u8"Visibility", 32, 2, GAL::ComponentType::INT, GAL::TextureType::COLOR);
		}

		AddAttachment(u8"Depth", 32, 1, GAL::ComponentType::FLOAT, GAL::TextureType::DEPTH);
	}

	for (uint32 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
		graphicsCommandLists[f] = renderSystem->CreateCommandList(u8"Graphics Command List", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT, false);
		graphicsWorkloadHandle[f] = renderSystem->CreateWorkload(u8"Frame work", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT);
		imageAcquisitionWorkloadHandles[f] = renderSystem->CreateWorkload(u8"Swapchain Image Acquisition", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::TRANSFER);
		transferCommandList[f] = renderSystem->CreateCommandList(u8"Transfer Command List", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::TRANSFER);
	}

	const auto& config = BE::Application::Get()->GetConfig();

	auto* windowSystem = GetApplicationManager()->GetSystem<WindowSystem>(u8"WindowSystem");

	renderContext = renderSystem->CreateRenderContext(windowSystem, static_cast<GameApplication*>(BE::Application::Get())->GetWindowHandle());

	for(auto rp : config[u8"RenderOrchestrator"][u8"debugViews"]) {
		if(!attachments.Find(rp)) { BE_LOG_WARNING(u8"Tried to enable debug view for attachment ", GTSL::StringView(rp), u8", but no such attachment exists."); continue; }

		auto& dv = debugViews.EmplaceBack(rp.GetStringView());

		dv.windowHandle = windowSystem->CreateWindow(u8"debugView", rp.GetStringView(), { 1920, 1080 });
		dv.renderContext = renderSystem->CreateRenderContext(windowSystem, dv.windowHandle);

		for (uint32 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
			dv.workloadHandles[f] = renderSystem->CreateWorkload(u8"Debug view image acquisition", GAL::QueueTypes::GRAPHICS, GAL::PipelineStages::TRANSFER);
		}
	}
	
	for(auto rp : config[u8"RenderOrchestrator"][u8"renderPasses"]) {
		if(auto enabled = rp[u8"enabled"]) {
			
		}
	}

	randomB(); randomB(); randomB();
}

void RenderOrchestrator::Setup(TaskInfo taskInfo) {
}

template<typename K, typename V, class ALLOC>
void Skim(GTSL::HashMap<K, V, ALLOC>& hash_map, auto predicate, const ALLOC& allocator) {
	GTSL::Vector<uint64, ALLOC> toSkim(8192 * 2, allocator);
	GTSL::PairForEach(hash_map, [&](K key, V& val) { if (predicate(val)) { toSkim.EmplaceBack(key); } });
	for (auto e : toSkim) { hash_map.Remove(e); }
}

inline float32 Halton(uint32 i, uint32 b) {
    float32 f = 1.0f, r = 0.0f;
 
    while (i > 0) {
        f /= static_cast<float32>(b);
        r = r + f * static_cast<float32>(i % b);
        i = static_cast<uint32>(floorf(static_cast<float32>(i) / static_cast<float32>(b)));
    }
 
    return r;
}

void RenderOrchestrator::Render(TaskInfo taskInfo, RenderSystem* renderSystem) {
	const uint8 currentFrame = renderSystem->GetCurrentFrame(); auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();

	GTSL::Extent2D renderArea = renderSystem->GetRenderExtent(renderContext);
	
	renderSystem->Wait(graphicsWorkloadHandle[currentFrame]); // We HAVE to wait or else descriptor update fails because command list may be in use

	if (auto res = renderSystem->AcquireImage(renderContext, imageAcquisitionWorkloadHandles[currentFrame], GetApplicationManager()->GetSystem<WindowSystem>(u8"WindowSystem")); res || sizeHistory[currentFrame] != sizeHistory[beforeFrame]) {
		OnResize(renderSystem, res.Get());
		renderArea = res.Get();
	}

	for(auto& dv : debugViews) {
		if (auto res = renderSystem->AcquireImage(dv.renderContext, dv.workloadHandles[currentFrame], GetApplicationManager()->GetSystem<WindowSystem>(u8"WindowSystem")); res) {
		}
	}

	updateDescriptors(taskInfo);

	GTSL::StringView resultAttachment;

	bool debugRenderNodes = BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"debugRenderNodes"].GetBool();

	{
		auto bwk = GetBufferWriteKey(renderSystem, globalDataDataKey);
		bwk[u8"frameIndex"] = frameIndex++;
		bwk[u8"elapsedTime"] = BE::Application::Get()->GetClock()->GetElapsedTime().As<float32, GTSL::Seconds>();
		bwk[u8"deltaTime"] = BE::Application::Get()->GetClock()->GetDeltaTime().As<float32, GTSL::Seconds>();
		bwk[u8"framePipelineDepth"] = static_cast<uint32>(renderSystem->GetPipelinedFrames());
		bwk[u8"random"][0] = static_cast<uint32>(randomA()); bwk[u8"random"][1] = static_cast<uint32>(randomB());
		bwk[u8"random"][2] = static_cast<uint32>(randomA()); bwk[u8"random"][3] = static_cast<uint32>(randomB());
	}

	{
		auto* cameraSystem = taskInfo.ApplicationManager->GetSystem<CameraSystem>(u8"CameraSystem");

		auto fovs = cameraSystem->GetFieldOfViews();

		if (fovs.ElementCount()) {
			//SetNodeState(cameraDataNode, true); // Set state on data key, to fullfil resource counts
			auto fov = cameraSystem->GetFieldOfViews()[0]; auto aspectRatio = static_cast<float32>(renderArea.Width) / static_cast<float32>(renderArea.Height);

			auto fExtent = GTSL::Vector2(renderArea.Width, renderArea.Height);

			float32 nearValue = 0.1f, farValue = 1000.0f;

			if constexpr (INVERSE_Z) {
				std::swap(nearValue, farValue);
			}

			uint32 jitterIndex = frameIndex % 8;

			float haltonX = 2.0f * Halton(jitterIndex + 1, 2) - 1.0f;
			float haltonY = 2.0f * Halton(jitterIndex + 1, 3) - 1.0f;
			float jitterX = (haltonX / fExtent.X());
			float jitterY = (haltonY / fExtent.Y());

			GTSL::Matrix4 projectionMatrix = GTSL::Math::BuildPerspectiveMatrix(fov, aspectRatio, nearValue, farValue);
			projectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f; // Vulkan has inverted y

			auto invertedProjectionMatrix = GTSL::Math::BuildInvertedPerspectiveMatrix(fov, aspectRatio, nearValue, farValue);
			invertedProjectionMatrix[1][1] *= API == GAL::RenderAPI::VULKAN ? -1.0f : 1.0f; // Vulkan has inverted y

			auto viewMatrix = cameraSystem->GetCameraTransform();

			auto cameraPosition = cameraSystem->GetCameraPosition(CameraSystem::CameraHandle{0});

			viewMatrix[0][3] *= -1.0f; viewMatrix[1][3] *= -1.0f; viewMatrix[2][3] *= -1.0f; // Negate coordinates to make view matrix

			auto cameraData = GetBufferWriteKey(renderSystem, cameraDataKeyHandle);
			cameraData[u8"viewHistory"][2] = cameraData[u8"viewHistory"][1];
			cameraData[u8"viewHistory"][1] = cameraData[u8"viewHistory"][0];

			auto currentView = cameraData[u8"viewHistory"][0];

			currentView[u8"view"] = viewMatrix;
			currentView[u8"proj"] = projectionMatrix;
			currentView[u8"viewInverse"] = GTSL::Math::Inverse(viewMatrix);
			currentView[u8"projInverse"] = invertedProjectionMatrix;
			currentView[u8"vp"] = projectionMatrix * viewMatrix;
			currentView[u8"vpInverse"] = GTSL::Math::Inverse(viewMatrix) * invertedProjectionMatrix;
			currentView[u8"position"] = GTSL::Vector4(cameraPosition, 1.0f);
			currentView[u8"near"] = nearValue;
			currentView[u8"far"] = farValue;
			currentView[u8"extent"] = renderArea;
			currentView[u8"extentReciprocal"] = GTSL::Vector2(1) / fExtent;
			currentView[u8"aspectRatio"] = fExtent.X() / fExtent.Y();
		}
		else { //disable rendering for everything which depends on this view
			//SetNodeState(cameraDataNode, false);
		}
	}

	for(auto& renderPassNodeHandle : renderPasses) {
		auto& renderPass = getPrivateNode<RenderPassData>(renderPassNodeHandle);

		auto bwk = GetBufferWriteKey(renderSystem, renderPass.DataKey);

		for (auto i = 0u; i < renderPass.Attachments.GetLength(); ++i) {
			const auto& e = renderPass.Attachments[i];

			if(auto a = attachments.TryGet(e.Attachment)) {
				if(GTSL::IsIn(e.Name, u8"History")) { // If attachment name is history that means we want to access the previous frames' data for that attachment
					bwk[e.Name] = a.Get().ImageIndeces[beforeFrame];
				} else {
					bwk[e.Name] = a.Get().ImageIndeces[currentFrame];
				}
			}
		}
	}

	auto processExecutionString = [renderArea](const GTSL::StringView execution) {
		GTSL::StaticVector<GTSL::StringView, 16> tokens;

		GTSL::StaticVector<GTSL::StringView, 16> operators;

		GTSL::StaticVector<GTSL::StaticString<64>, 16> output;

		d(execution, tokens);

		while (tokens) {
			auto token = tokens.back(); tokens.PopBack();

			if (GTSL::IsNumber(token) or IsAnyOf(token, u8"windowExtent", u8"localSize")) {
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
			if (GTSL::IsNumber(token) or IsAnyOf(token, u8"windowExtent", u8"localSize")) {
				if (token == u8"windowExtent") {
					numbers.EmplaceBack(renderArea);
				}
				else if (token == u8"localSize") {
					numbers.EmplaceBack(GTSL::Extent3D(32, 32, 1));
				}
				else {
					numbers.EmplaceBack(GTSL::ToNumber<uint16>(token).Get());
				}
			}
			else { //operator
				auto a = numbers.back(); numbers.PopBack();

				auto b = numbers.back(); numbers.PopBack();

				switch (GTSL::Hash(token)) {
				case GTSL::Hash(u8"+"): numbers.EmplaceBack(a + b); break;
				case GTSL::Hash(u8"-"): numbers.EmplaceBack(a - b); break;
				case GTSL::Hash(u8"*"): numbers.EmplaceBack(a * b); break;
				case GTSL::Hash(u8"/"): numbers.EmplaceBack(a / b); break;
				}
			}
		}

		return numbers.back();
	};

	uint32 counterStack[16] = { 0u };
	uint32 counterI = 0;

	if(isRenderTreeDirty) { // If render tree is dirty then every command buffer for every frame has to be updated
		if(BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"optimizeRenderTree"].GetBool()) {
			renderingTree.Optimize();
		}

		for(uint32 f = 0; f < renderSystem->GetPipelinedFrames(); ++f) {
			isCommandBufferUpdated[f] = false;
		}
	}

	if(!isCommandBufferUpdated[currentFrame] or isRenderTreeDirty) {
		if(debugRenderNodes) {
			BE_LOG_SUCCESS(u8"Started baking command buffer.");
		}

		renderSystem->StartCommandList(graphicsCommandLists[currentFrame]);

		auto& commandBuffer = *renderSystem->GetCommandList(graphicsCommandLists[currentFrame]);

		BindSet(renderSystem, commandBuffer, globalBindingsSet, GAL::ShaderStages::VERTEX | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN);

		RenderState renderState;
		uint32 lastInvalidLevel = 0xFFFFFFFF;

		auto visitNode = [&](const decltype(renderingTree)::Key key, const uint32_t level, bool enabled) -> void {
			if (!enabled || level >= lastInvalidLevel) {
				if(!enabled) { printNode(key, level, debugRenderNodes, enabled); }
				if(!enabled && level < lastInvalidLevel) { lastInvalidLevel = level; }
				return;
			}

			printNode(key, level, debugRenderNodes, enabled);

			const auto& baseData = renderingTree.GetAlpha(key);

			auto debugState = [&] {
				if(debugRenderNodes) {
					auto& shaderGroup = shaderGroups[renderState.BoundShaderGroupIndex];
					for(uint32 i = 0; auto& e : shaderGroup.PushConstantLayout) {
						auto& element = getElement(getDataKey(renderState.dataKeys[i]).Handle);
						auto dt = element.DataType;
						RTrimLast(dt, u8'[');
						dt += u8"*";
						++i;

						if(dt != GTSL::StringView(e.Type)) {
							BE_LOG_WARNING(u8"Pipeline expected push constant layout does not match current layout. Shader declared: ", e.Type, u8", but bound type is: ", dt, u8".");
						}
					}

					if(shaderGroup.PushConstantLayout.GetLength() != renderState.streamsCount) {
						BE_LOG_WARNING(u8"Bound push constant range doesn't match shader expect range.")
					}
				}
			};

			switch (renderingTree.GetNodeType(key)) {
			case RTT::GetTypeIndex<DataNode>(): {
				const auto& dataNode = renderingTree.GetClass<DataNode>(key);

				if constexpr (BE_DEBUG) {
					commandBuffer.BeginRegion(renderSystem->GetRenderDevice(), elements[getDataKey(dataNode.DataKey).Handle()].Name);
				}

				if (dataNode.DataKey) {
					if (dataNode.UseCounter) {
						++counterI;
					}

					const DataStreamHandle dataStreamHandle = renderState.AddDataStream(dataNode.DataKey);
					const auto& dataKey = getDataKey(dataNode.DataKey);

					GAL::DeviceAddress address = renderSystem->GetBufferAddress(dataKey.Buffer[1]) + dataKeysMap[dataNode.DataKey()].Second; // Get READ buffer handle

					auto& setLayout = setLayoutDatas[globalSetLayout()]; address += dataKey.Offset;
					commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), setLayout.PipelineLayout, dataStreamHandle() * 8, GTSL::Range(8, reinterpret_cast<const byte*>(&address)), setLayout.Stage);

					if(BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"debugBuffers"].GetBool()) {
						PrintMember(dataNode.DataKey, renderSystem);
					}
				}

				break;
			}
			case RTT::GetTypeIndex<PipelineBindData>(): {
				const PipelineBindData& pipeline_bind_data = renderingTree.GetClass<PipelineBindData>(key);
				const auto& shaderGroupInstance = shaderGroupInstances[pipeline_bind_data.Handle()];
				const auto& shaderGroup = shaderGroups[shaderGroupInstance.ShaderGroupIndex];
				uint32 pipelineIndex = 0xFFFFFFFF;

				if (shaderGroup.RasterPipelineIndex != 0xFFFFFFFF) {
					pipelineIndex = shaderGroup.RasterPipelineIndex;
				} else if (shaderGroup.ComputePipelineIndex != 0xFFFFFFFF) {
					pipelineIndex = shaderGroup.ComputePipelineIndex;
				} else if (shaderGroup.RTPipelineIndex != 0xFFFFFFFF) {
					pipelineIndex = shaderGroup.RTPipelineIndex;
				} else {
					BE_LOG_WARNING(u8"Pipeline bind data node with no valid pipeline reference.");
				}

				renderState.BoundPipelineIndex = pipelineIndex;
				renderState.BoundShaderGroupIndex = shaderGroupInstance.ShaderGroupIndex;

				commandBuffer.BindPipeline(renderSystem->GetRenderDevice(), pipelines[pipelineIndex].pipeline, renderState.ShaderStages);
				break;
			}
			case RTT::GetTypeIndex<DispatchData>(): {
				const DispatchData& dispatchData = renderingTree.GetClass<DispatchData>(key);

				const auto& pipeline = pipelines[renderState.BoundPipelineIndex];
				const auto& execution = pipeline.ExecutionString;

				const auto executionExtent = processExecutionString(execution);

				commandBuffer.Dispatch(renderSystem->GetRenderDevice(), executionExtent);

				break;
			}
			case RTT::GetTypeIndex<RayTraceData>(): {
				const RayTraceData& rayTraceData = renderingTree.GetClass<RayTraceData>(key);
				const auto& pipelineData = pipelines[shaderGroups[shaderGroupInstances[rayTraceData.ShaderGroupIndex].ShaderGroupIndex].RTPipelineIndex];

				CommandList::ShaderTableDescriptor shaderTableDescriptors[4];

				for (uint32 i = 0, offset = 0; i < 3; ++i) {
					shaderTableDescriptors[i].Entries = pipelineData.RayTracingData.ShaderGroups[i].ShaderCount;
					shaderTableDescriptors[i].EntrySize = GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
					shaderTableDescriptors[i].Address = renderSystem->GetBufferAddress(getDataKey(pipelineData.ShaderBindingTableBuffer).Buffer[1]) + offset;

					offset += GTSL::Math::RoundUpByPowerOf2(GetSize(pipelineData.RayTracingData.ShaderGroups[i].TableHandle), renderSystem->GetShaderGroupHandleAlignment());
				}

				const auto executionExtent = processExecutionString(pipelineData.ExecutionString);

				commandBuffer.TraceRays(renderSystem->GetRenderDevice(), GTSL::Range(4, shaderTableDescriptors), executionExtent);

				break;
			}
			case RTT::GetTypeIndex<VertexBufferBindData>(): {
				const VertexBufferBindData& meshData = renderingTree.GetClass<VertexBufferBindData>(key);
				const auto vertexBuffer = renderSystem->GetBuffer(meshData.Handle);
				GTSL::StaticVector<GPUBuffer, 8> buffers;
				for (uint32 i = 0; i < meshData.Offsets.GetLength(); ++i) {
					buffers.EmplaceBack(vertexBuffer);
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
				counterStack[counterI - 1] += meshData.InstanceCount;

				break;
			}
			case RTT::GetTypeIndex<DrawData>(): {
				const DrawData& draw_data = renderingTree.GetClass<DrawData>(key);
				commandBuffer.Draw(renderSystem->GetRenderDevice(), draw_data.VertexCount, draw_data.InstanceCount, counterStack[counterI - 1]);

				counterStack[counterI - 1] += draw_data.InstanceCount;

				break;
			}
			case RTT::GetTypeIndex<RenderPassData>(): {
				const RenderPassData& renderPassData = renderingTree.GetClass<RenderPassData>(key);

				transitionImages(commandBuffer, renderSystem, &renderPassData);

				switch (renderPassData.Type) {
				case PassType::RASTER: {
					renderState.ShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT;

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
					break;
				}
				case PassType::COMPUTE: {
					renderState.ShaderStages = GAL::ShaderStages::COMPUTE;
					break;
				}
				case PassType::RAY_TRACING: {
					renderState.ShaderStages = GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::INTERSECTION | GAL::ShaderStages::CALLABLE;
					break;
				}
				}

				resultAttachment = renderPassData.Attachments[0].Name;

				break;
			}
			}
		};

		auto endNode = [&](const uint32 key, const uint32_t level, bool enabled) {
			if (!enabled || level >= lastInvalidLevel) { return; }

			if(debugRenderNodes) {
				BE_LOG_WARNING(u8"Node: ", key, u8", Level: ", level);
			}

			switch (renderingTree.GetNodeType(key)) {
			case RTT::GetTypeIndex<DataNode>(): {
				const auto& node = getPrivateNode<DataNode>(key);

				renderState.PopData();

				if(node.UseCounter) {
					--counterI;
					counterStack[counterI] = 0u;
				}

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

		ForEachWithDisabled(renderingTree, visitNode, endNode);

		commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE,
		CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(renderContext), GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

		if (resultAttachment.GetCodepoints()) {
			auto& attachment = attachments.At(resultAttachment);

			commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, attachment.AccessType,
				GAL::AccessTypes::READ, CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout[currentFrame],
				GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor } } }, GetTransientAllocator());

			updateImage(currentFrame, attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);

			commandBuffer.BlitTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor, sizeHistory [currentFrame], *renderSystem->GetSwapchainTexture(renderContext), GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat(), GTSL::Extent3D(renderSystem->GetRenderExtent(renderContext)));
		}

		commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier	{ renderSystem->GetSwapchainTexture(renderContext), GAL::TextureLayout::TRANSFER_DESTINATION,
		GAL::TextureLayout::PRESENTATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

		for(auto& dv : debugViews) {
			commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier{ renderSystem->GetSwapchainTexture(dv.renderContext), GAL::TextureLayout::UNDEFINED, GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());

			{
				auto& attachment = attachments.At(dv.name);

				commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { attachment.ConsumingStages, GAL::PipelineStages::TRANSFER, attachment.AccessType,
					GAL::AccessTypes::READ, CommandList::TextureBarrier{ renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), attachment.Layout[currentFrame],
					GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor } } }, GetTransientAllocator());

				updateImage(currentFrame, attachment, GAL::TextureLayout::TRANSFER_SOURCE, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ);

				commandBuffer.BlitTexture(renderSystem->GetRenderDevice(), *renderSystem->GetTexture(attachment.TextureHandle[currentFrame]), GAL::TextureLayout::TRANSFER_SOURCE, attachment.FormatDescriptor, sizeHistory [currentFrame], *renderSystem->GetSwapchainTexture(dv.renderContext), GAL::TextureLayout::TRANSFER_DESTINATION, renderSystem->GetSwapchainFormat(), GTSL::Extent3D(renderSystem->GetRenderExtent(dv.renderContext)));
			}

			commandBuffer.AddPipelineBarrier(renderSystem->GetRenderDevice(), { { GAL::PipelineStages::TRANSFER, GAL::PipelineStages::TRANSFER, GAL::AccessTypes::READ, GAL::AccessTypes::WRITE, CommandList::TextureBarrier	{ renderSystem->GetSwapchainTexture(dv.renderContext), GAL::TextureLayout::TRANSFER_DESTINATION,
			GAL::TextureLayout::PRESENTATION, renderSystem->GetSwapchainFormat() } } }, GetTransientAllocator());
		}

		renderSystem->EndCommandList(graphicsCommandLists[currentFrame]);

		if(debugRenderNodes) {
			BE_LOG_SUCCESS(u8"Ended baking command buffer.");
		}

		isRenderTreeDirty = false;
		isCommandBufferUpdated[currentFrame] = true;
	}

	{
		auto processPendingWrite = [&](PendingWriteData& pending_write_data) {
			bool c = pending_write_data.FrameCountdown[currentFrame], b = pending_write_data.FrameCountdown[beforeFrame];
			
			if (c) {
				renderSystem->AddBufferUpdate(transferCommandList[currentFrame], pending_write_data.Buffer[0], pending_write_data.Buffer[1]);
				pending_write_data.FrameCountdown[beforeFrame] = false;
				pending_write_data.FrameCountdown[currentFrame] = false;
				return true;
			}
	
			//pending_write_data.FrameCountdown[currentFrame] = false;
	
			return false;
		};
	
		Skim(pendingWrites, processPendingWrite, GetPersistentAllocator());
	}

	{
		renderSystem->StartCommandList(transferCommandList[currentFrame]);
		renderSystem->EndCommandList(transferCommandList[currentFrame]);

		GTSL::StaticVector<RenderSystem::CommandListHandle, 8> commandLists;
		GTSL::StaticVector<RenderSystem::WorkloadHandle, 8> workloads;

		commandLists.EmplaceBack(transferCommandList[renderSystem->GetCurrentFrame()]);

		workloads.EmplaceBack(buildAccelerationStructuresWorkloadHandle[renderSystem->GetCurrentFrame()]);
		if (BE::Application::Get()->GetBoolOption(u8"rayTracing")) {
		}

		workloads.EmplaceBack(imageAcquisitionWorkloadHandles[currentFrame]);

		for(auto& dv : debugViews) {
			workloads.EmplaceBack(dv.workloadHandles[currentFrame]);
		}

		workloads.EmplaceBack(graphicsWorkloadHandle[currentFrame]);

		commandLists.EmplaceBack(graphicsCommandLists[currentFrame]);

		renderSystem->Submit(GAL::QueueTypes::GRAPHICS, { { { transferCommandList[currentFrame] }, {}, { graphicsWorkloadHandle[currentFrame]}}, {{graphicsCommandLists[currentFrame]}, workloads,	{graphicsWorkloadHandle[currentFrame]}}}, graphicsWorkloadHandle[renderSystem->GetCurrentFrame()]); // Wait on image acquisition to render maybe, //Signal graphics workload

		auto windowSystem = GetApplicationManager()->GetSystem<WindowSystem>(u8"WindowSystem");

		GTSL::StaticVector<RenderSystem::RenderContextHandle, 8> renderContexts;

		renderContexts.EmplaceBack(renderContext);

		for(const auto& e : debugViews) {
			renderContexts.EmplaceBack(e.renderContext);
		}

		renderSystem->Present(windowSystem, renderContexts, { graphicsWorkloadHandle[currentFrame] }); // Wait on graphics work to present
	}

	renderSystem->Wait(graphicsWorkloadHandle[currentFrame]);

	//TODO: wait on transfer work to start next frame, or else reads will be corrupted since, next frame may have started
}

RenderModelHandle RenderOrchestrator::CreateShaderGroup(GTSL::StringView shader_group_instance_name) {
	auto shaderGroupReference = shaderGroupInstanceByName.TryEmplace(shader_group_instance_name);

	uint32 id = 0xFFFFFFFF;

	if (shaderGroupReference.State()) {
		id = shaderGroupInstances.GetLength();
		shaderGroupReference.Get() = id;

		ShaderLoadInfo sli(GetPersistentAllocator());
		GetApplicationManager()->GetSystem<ShaderResourceManager>(u8"ShaderResourceManager")->LoadShaderGroupInfo(GetApplicationManager(), Id(shader_group_instance_name), onShaderInfosLoadHandle, GTSL::MoveRef(sli));

		auto& shaderGroupInstance = shaderGroupInstances.EmplaceBack();
		
		shaderGroupInstance.Resource = makeResource(GTSL::StringView(shader_group_instance_name));
		addDependencyOnResource(shaderGroupInstance.Resource); // Add dependency the pipeline itself
		shaderGroupInstance.DataKey = MakeDataKey();
		shaderGroupInstance.Name = shader_group_instance_name;
		shaderGroupInstance.UpdateKey = CreateUpdateKey();
	} else {
		auto& material = shaderGroups[shaderGroupReference.Get()];
		id = shaderGroupReference.Get();
	}

	return RenderModelHandle(id);
}

void RenderOrchestrator::AddAttachment(GTSL::StringView attachment_name, uint8 bitDepth, uint8 componentCount, GAL::ComponentType compType, GAL::TextureType type) {
	Attachment attachment;
	attachment.Name = attachment_name;
	attachment.Uses = GAL::TextureUse();

	attachment.Uses |= GAL::TextureUses::ATTACHMENT;
	attachment.Uses |= GAL::TextureUses::SAMPLE;

	if (type == GAL::TextureType::COLOR) {
		attachment.FormatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::COLOR, 0, componentCount >= 2 ? 1 : 0, componentCount >= 3 ? 2 : 0, componentCount >= 4 ? 3 : 0);
		attachment.Uses |= GAL::TextureUses::STORAGE;
		attachment.Uses |= GAL::TextureUses::TRANSFER_SOURCE;
		attachment.ClearColor = GTSL::RGBA(1, 1, 1, 1);
	}
	else {
		attachment.FormatDescriptor = GAL::FormatDescriptor(compType, componentCount, bitDepth, GAL::TextureType::DEPTH, 0, 0, 0, 0);
		attachment.ClearColor = GTSL::RGBA(INVERSE_Z ? 0.0f: 1.0f, 0, 0, 0);
	}

	attachment.Layout[0] = GAL::TextureLayout::UNDEFINED; attachment.Layout[1] = GAL::TextureLayout::UNDEFINED; attachment.Layout[2] = GAL::TextureLayout::UNDEFINED;
	attachment.AccessType = GAL::AccessTypes::READ;
	attachment.ConsumingStages = GAL::PipelineStages::TOP_OF_PIPE;

	for(uint32 f = 0; f < 2; ++f) {
		attachment.ImageIndeces[f] = imageIndex++;
		++textureIndex;
	}

	attachments.Emplace(attachment_name, attachment);
}

RenderOrchestrator::NodeHandle RenderOrchestrator::AddRenderPassNode(NodeHandle parent_node_handle, GTSL::StringView instance_name, GTSL::StringView render_pass_name, RenderSystem* renderSystem, PassData pass_data, const GTSL::Range<const ND*> innner) {
	GTSL::StaticVector<MemberInfo, 16> members;

	for (auto& e : pass_data.Attachments) {
		if(!(e.Access & GAL::AccessTypes::WRITE)) {
			members.EmplaceBack(nullptr, u8"TextureReference", GTSL::StringView(e.Name));
		} else {
			members.EmplaceBack(nullptr, u8"ImageReference", GTSL::StringView(e.Name));			
		}
	}

	CreateScope(u8"global", render_pass_name);

	const auto scope = GTSL::StaticString<64>(u8"global") + u8"." + render_pass_name;

	auto member = RegisterType(scope, u8"RenderPassData", members);

	NodeHandle leftNodeHandle(0xFFFFFFFF);

	const auto dataKey = MakeDataKey(renderSystem, scope, u8"RenderPassData");
	const auto renderPassDataNode = AddDataNode(leftNodeHandle, parent_node_handle, dataKey);
	const auto renderPassNodeHandleResult = addInternalNode<RenderPassData>(GTSL::Hash(instance_name), renderPassDataNode);

	if(!renderPassNodeHandleResult) { return renderPassNodeHandleResult.Get(); }

	NodeHandle renderPassNodeHandle = renderPassNodeHandleResult.Get();

	RenderPassData& renderPass = getPrivateNode<RenderPassData>(renderPassNodeHandle);
	renderPass.DataKey = dataKey;

	auto renderPassIndex = renderPasses.GetLength();
	renderPassesMap.Emplace(render_pass_name, renderPassIndex);
	renderPasses.EmplaceBack(renderPassNodeHandle);

	renderPass.ResourceHandle = makeResource(render_pass_name);
	addDependencyOnResource(renderPass.ResourceHandle); //add dependency on render pass texture creation

	BindResourceToNode(renderPassNodeHandle, renderPass.ResourceHandle);

	getNode(renderPassNodeHandle).Name = GTSL::StringView(instance_name);

	PassType renderPassType = PassType::RASTER;
	GAL::PipelineStage pipelineStage;

	NodeHandle resultHandle = renderPassNodeHandle;

	switch (pass_data.PassType) {
	case PassType::RASTER: {
		renderPassType = PassType::RASTER;
		pipelineStage = GAL::PipelineStages::COLOR_ATTACHMENT_OUTPUT;

		for (const auto& e : pass_data.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Attachment = e.Attachment;
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

		for (const auto& e : pass_data.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Attachment = e.Attachment;
			attachmentData.Access = e.Access;
			attachmentData.ConsumingStages = GAL::PipelineStages::COMPUTE;

			if (e.Access & GAL::AccessTypes::READ) {
				attachmentData.Layout = GAL::TextureLayout::SHADER_READ;
			} else {
				attachmentData.Layout = GAL::TextureLayout::GENERAL;
			}
		}

		auto sgh = CreateShaderGroup(render_pass_name);
		resultHandle = AddMaterial(renderPassNodeHandle, sgh);

		for(const auto e : innner) {
			resultHandle = AddDataNode(resultHandle, e.Name, e.DKH);
		}

		resultHandle = addInternalNode<DispatchData>(GTSL::Hash(render_pass_name), resultHandle).Get();

		break;
	}
	case PassType::RAY_TRACING: {
		renderPassType = PassType::RAY_TRACING;
		pipelineStage = GAL::PipelineStages::RAY_TRACING;

		for (const auto& e : pass_data.Attachments) {
			auto& attachmentData = renderPass.Attachments.EmplaceBack();

			attachmentData.Name = e.Name;
			attachmentData.Attachment = e.Attachment;
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

	auto bwk = GetBufferWriteKey(renderSystem, renderPass.DataKey);

	for (auto i = 0u; i < pass_data.Attachments.GetLength(); ++i) {
		const auto& attachmentReference = pass_data.Attachments[i];

		AddNodeDependency(renderPassNodeHandle);

		if(auto a = attachments.TryGet(attachmentReference.Attachment)) {
			if(GTSL::IsIn(attachmentReference.Name, u8"History")) { // Set image reference as attachment for previous hit since history means we access previous frame's data
				bwk[pass_data.Attachments[i].Name] = a.Get().ImageIndeces[(renderSystem->GetCurrentFrame() - 1) % renderSystem->GetPipelinedFrames()];
			} else {
				bwk[pass_data.Attachments[i].Name] = a.Get().ImageIndeces[(renderSystem->GetCurrentFrame() - 1) % renderSystem->GetPipelinedFrames()];				
			}

			FulfillNodeDependency(renderPassNodeHandle);
		} else {
			BE_LOG_WARNING(u8"Render pass: ", render_pass_name, u8", references attachment: ", attachmentReference.Name, u8", which does not exist. Render pass will be disabled.");
		}
	}

	return resultHandle;
}

void RenderOrchestrator::OnResize(RenderSystem* renderSystem, const GTSL::Extent2D newSize)
{
	//pendingDeleteFrames = renderSystem->GetPipelinedFrames();

	auto currentFrame = renderSystem->GetCurrentFrame();
	auto beforeFrame = uint8(currentFrame - uint8(1)) % renderSystem->GetPipelinedFrames();

	auto resize = [&](Attachment& attachment) -> void {
		GTSL::StaticString<64> name; name += GTSL::StringView(attachment.Name); GTSL::ToString(name, currentFrame);

		attachment.TextureHandle[currentFrame] = renderSystem->CreateTexture(name, attachment.FormatDescriptor, newSize, attachment.Uses, false, attachment.TextureHandle[currentFrame]);

		for(uint32 i = 0; i < 2u; ++i) {
			for(uint32 f = 0; f < GTSL::Math::Clamp(2u, 0u, frameIndex + 1u); ++f) {
				if (attachment.FormatDescriptor.Type == GAL::TextureType::COLOR) {  //if attachment is of type color (not depth), write image descriptor
					WriteBinding(renderSystem, imagesSubsetHandle, attachment.TextureHandle[f], attachment.ImageIndeces[f], i);
				}

				WriteBinding(renderSystem, textureSubsetsHandle, attachment.TextureHandle[f], attachment.ImageIndeces[f], i);
			}
		}

		attachment.Layout[currentFrame] = GAL::TextureLayout::UNDEFINED;
	};

	if (sizeHistory[currentFrame] != newSize) {
		sizeHistory[currentFrame] = newSize;
		GTSL::ForEach(attachments, resize);
	}

	for (const auto apiRenderPassData : renderPasses) {
		auto& layer = getPrivateNode<RenderPassData>(apiRenderPassData);
		signalDependencyToResource(layer.ResourceHandle);
		setRenderTreeAsDirty(apiRenderPassData);
	}
}

void RenderOrchestrator::ToggleRenderPass(NodeHandle renderPassName, bool enable)
{
	if (!renderPassName) { BE_LOG_WARNING(u8"Tried to ", enable ? u8"enable" : u8"disable", u8" a render pass which does not exist."); return; }

	auto& renderPassNode = getPrivateNode<RenderPassData>(renderPassName);

	switch (renderPassNode.Type) {
	case PassType::RASTER: break;
	case PassType::COMPUTE: break;
	case PassType::RAY_TRACING: enable = enable && BE::Application::Get()->GetBoolOption(u8"rayTracing"); break; // Enable render pass only if function is enaled in settings
	default: break;
	}

	SetNodeState(renderPassName, enable);
}

void RenderOrchestrator::onRenderEnable(ApplicationManager* gameInstance, const GTSL::Range<const TaskDependency*> dependencies) {
	//gameInstance->AddTask(SETUP_TASK_NAME, &RenderOrchestrator::Setup, DependencyBlock(), u8"GameplayEnd", u8"RenderStart");
	//gameInstance->AddTask(RENDER_TASK_NAME, &RenderOrchestrator::Render, DependencyBlock(), u8"RenderDo", u8"RenderFinished");
}

void RenderOrchestrator::onRenderDisable(ApplicationManager* gameInstance) {
	//gameInstance->RemoveTask(SETUP_TASK_NAME, u8"GameplayEnd");
	//gameInstance->RemoveTask(RENDER_TASK_NAME, u8"RenderDo");
}

void RenderOrchestrator::OnRenderEnable(TaskInfo taskInfo, bool oldFocus) {
	renderingEnabled = true;
}

void RenderOrchestrator::OnRenderDisable(TaskInfo taskInfo, bool oldFocus) {
	renderingEnabled = false;
}

void RenderOrchestrator::transitionImages(CommandList commandBuffer, RenderSystem* renderSystem, const RenderPassData* renderPass)
{
	GTSL::StaticVector<CommandList::BarrierData, 16> barriers;

	GAL::PipelineStage initialStage;

	auto buildTextureBarrier = [&](const AttachmentData& attachmentData, GAL::PipelineStage attachmentStages, GAL::AccessType access) {
		auto& attachment = attachments.At(attachmentData.Attachment);

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
	shaderLoadInfo.Buffer.PushBytes(size);

	auto shaderGroupWasEmplaced = shaderGroupsByName.TryEmplace(shader_group_info.Name, ~0u);

	if(shaderGroupWasEmplaced) {
		auto shaderGroupIndex = shaderGroups.Emplace();
		shaderGroupWasEmplaced.Get() = shaderGroupIndex;
		auto& shaderGroup = shaderGroups[shaderGroupIndex];

		shaderGroup.Name = shader_group_info.Name;
		shaderGroup.Buffer = MakeDataKey();
		shaderGroup.Resource = makeResource(shader_group_info.Name);

		for(auto& e : shader_group_info.Instances) {
			if(!shaderGroupInstanceByName.Find(e.Name)) { continue; }
			auto& instance = shaderGroupInstances[shaderGroupInstanceByName[e.Name]];
			instance.Name = e.Name;
			instance.ShaderGroupIndex = shaderGroupIndex;
			addDependencyOnResource(shaderGroup.Resource, instance.Resource);
		}
	}

	materialResourceManager->LoadShaderGroup(taskInfo.ApplicationManager, GTSL::MoveRef(shader_group_info), onShaderGroupLoadHandle, shaderLoadInfo.Buffer.GetRange(), GTSL::MoveRef(shaderLoadInfo));
}

void RenderOrchestrator::onShadersLoaded(TaskInfo taskInfo, ShaderResourceManager*, RenderSystem* renderSystem, ShaderResourceManager::ShaderGroupInfo shader_group_info, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo)
{
	if constexpr (BE_DEBUG) {
		bool valid = true;

		for(auto& e : shader_group_info.Shaders) { //If any shader is size zero, then shader group cannot be used.
			if(e.Size == 0u) { valid = false; break; }
		}

		if (!valid) {
			BE_LOG_ERROR(u8"Tried to load shader group ", shader_group_info.Name, u8" which is not valid. ", BE::FIX_OR_CRASH_STRING);
			return;
		}
	}

	auto& shaderGroup = shaderGroups[shaderGroupsByName[shader_group_info.Name]];

	if(shaderGroup.Loaded) { return; }

	addScope(u8"global", shader_group_info.Name);

	GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock, 32> pipelineStates;

	GTSL::HashMap<GTSL::StringView, StructElement, BE::TAR> parameters(8, GetTransientAllocator());

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

	// DEBUGGING ONLY: whether to check for integrity in shader member definitions, etc
	const auto debugShaders = BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"inspectShaders"].GetBool();

	// Construct vertex stream description for pipeline from shader group info.
	for (uint8 ai = 0; auto& a : shader_group_info.VertexElements) {
		auto& stream = vertexStreams.EmplaceBack();

		for (auto& b : a) {
			GAL::ShaderDataType type;

			switch (GTSL::Hash(b.Type)) {
			case GTSL::Hash(u8"vec2f"): type = GAL::ShaderDataType::FLOAT2; break;
			case GTSL::Hash(u8"vec3f"): type = GAL::ShaderDataType::FLOAT3; break;
			case GTSL::Hash(u8"vec4f"): type = GAL::ShaderDataType::FLOAT4; break;
			}

			stream.EmplaceBack(GAL::Pipeline::VertexElement{ GTSL::ShortString<32>(b.Name.c_str()), type, ai++ });
		}
	}

	for (uint32 offset = 0, si = 0; si < shader_group_info.Shaders; offset += shader_group_info.Shaders[si].Size, ++si) {
		const auto& shaderInfo = shader_group_info.Shaders[si];

		//TODO: cheap hack to filter shaders by render technique
		if (Contains(shaderInfo.Tags.GetRange(), PermutationManager::ShaderTag(u8"Domain", u8"World"))) {
			if (!Contains(shaderInfo.Tags.GetRange(), PermutationManager::ShaderTag(u8"RenderTechnique", tag)) && !Contains(shader_group_info.Tags, PermutationManager::ShaderTag(u8"RenderTechnique", tag))) {
				BE_LOG_WARNING(u8"Ignoring shader: ", shaderInfo.Name, u8" because it does not feature needed tag: ", tag);
				continue;
			}
		}

		if (auto res = shaders.TryEmplace(shaderInfo.Hash)) {
			auto& shader = res.Get();

			shader.Shader.Initialize(renderSystem->GetRenderDevice(), GTSL::Range(shaderInfo.Size, shaderLoadInfo.Buffer.GetData() + offset));
			shader.Type = shaderInfo.Type;
			shader.Name = shaderInfo.Name;
			loadedShadersMap.Emplace(shaderInfo.Hash);

			if(shaderInfo.DebugData){ // Check if shader symbols match active runtime symbols
				auto json = GTSL::JSON(shaderInfo.DebugData, GetPersistentAllocator());
				
				for(auto jsonStruct : json[u8"structs"]) {
					auto structName = jsonStruct[u8"name"].GetStringView();

					auto handle = tryGetDataTypeHandle(structName);

					if(!handle) {
						if(debugShaders) {
							BE_LOG_WARNING(u8"Could not find compatible shader declared symbol: ", structName);
						}

						continue;
					}

					auto& element = getElement(handle.Get());

					for(auto c : jsonStruct[u8"members"]) {
						auto memberSearchResult = tryGetDataTypeHandle(handle.Get(), c[u8"name"]);

						if(!memberSearchResult) {
							if(debugShaders) {
								BE_LOG_WARNING(u8"Shader symbol ", structName, u8", has member: ", c[u8"name"], u8", which matching renderer symbol doesn't.");
							}
						}
					}

					for(auto& e : element.children) {
						auto& f = getElement(e.Handle);
						if(f.Type != ElementData::ElementType::MEMBER) { continue; }

						[&]() {
							for(auto c : jsonStruct[u8"members"]) {
								if(c[f.Name]) {
									return;
								}
							}

							if(debugShaders) {
								BE_LOG_WARNING(u8"Renderer symbol ", element.Name, u8", has member: ", f.Name, u8", which matching shader symbol doesn't.");
							}
						};
					}
				}

				if(!shaderGroup.PushConstantLayout) {
					for(auto e : json[u8"pushConstant"][u8"members"]) {
						shaderGroup.PushConstantLayout.EmplaceBack(e[u8"type"], e[u8"name"]);
					}
				}
			}
		}

		if (auto executionExists = GTSL::Find(shaderInfo.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Execution"; })) {
			executionString = executionExists.Get()->Second;
		}

		bool foundGroup = false;
		auto shaderStageFlag = GAL::ShaderTypeToShaderStageFlag(shaderInfo.Type);

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
			if(auto r = GTSL::Find(shaderInfo.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Set"; })) {
				sb.Set = GTSL::StringView(r.Get()->Second);
			}

		} else {
			switch (shaderInfo.Type) {
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

	GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock::SpecializationData::SpecializationEntry, 8> specializationEntries;
	GTSL::StaticBuffer<1024> specializationData;

	{
		auto& debugEntry = specializationEntries.EmplaceBack();
		debugEntry.Size = 4; debugEntry.Offset = 0u; debugEntry.ID = 0u;
		specializationData.AllocateStructure<uint32>(BE::Application::Get()->GetConfig()[u8"RenderOrchestrator"][u8"debugShaders"].GetBool());

		for (uint32 pi = 0; const auto & p : shader_group_info.Parameters) {
			{
				parameters.Emplace(p.Name, p.Type, p.Name, p.Value);
				members.EmplaceBack(MemberInfo{ &shaderGroup.ParametersHandles.Emplace(Id(p.Name)), p.Type, p.Name });				
			}
		}

		auto& specializations = pipelineStates.EmplaceBack(GAL::Pipeline::PipelineStateBlock::SpecializationData{});

		specializations.Specialization.Entries = specializationEntries;
		specializations.Specialization.Data = specializationData.GetRange();
	}

	for (auto& e : shaderBundles) {
		GTSL::Vector<GPUPipeline::ShaderInfo, BE::TAR> shaderInfos(8, GetTransientAllocator());

		if (e.Stage & (GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT)) {
			if (shaderGroup.RasterPipelineIndex == 0xFFFFFFFF) { //if no pipeline already exists for this stage, create one
				shaderGroup.RasterPipelineIndex = pipelines.Emplace(GetPersistentAllocator());
			}

			e.PipelineIndex = shaderGroup.RasterPipelineIndex;

			for (auto s : e.Shaders) {
				auto& shaderInfo = shaderInfos.EmplaceBack();
				auto& shader = shaders[shader_group_info.Shaders[s].Hash];
				shaderInfo.Type = shader.Type;
				shaderInfo.Shader = shader.Shader;
			}

			GTSL::StaticVector<GAL::Pipeline::PipelineStateBlock::RenderContext::AttachmentState, 8> attachmentStates;

			GAL::Pipeline::PipelineStateBlock::RenderContext context;

			GTSL::StaticString<128> renderPass{ u8"ForwardRenderPass" };

			if (auto r = GTSL::Find(shader_group_info.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"RenderPass"; })) {
				renderPass = GTSL::StringView(r.Get()->Second);
			}

			bool transparency = false;

			if (auto r = GTSL::Find(shader_group_info.Tags, [&](const PermutationManager::ShaderTag& tag) { return static_cast<GTSL::StringView>(tag.First) == u8"Transparency"; })) {
				transparency = r.Get()->Second == GTSL::StringView(u8"true");
			}

			//BUG: if shader group gets processed before render pass it will fail
			const auto& renderPassNode = getPrivateNode<RenderPassData>(renderPasses[renderPassesMap[renderPass]]);

			for (const auto& writeAttachment : renderPassNode.Attachments) {
				if (writeAttachment.Access & GAL::AccessTypes::WRITE) {
					auto& attachment = attachments.At(writeAttachment.Name);
					auto& attachmentState = attachmentStates.EmplaceBack();
					attachmentState.BlendEnable = transparency; attachmentState.FormatDescriptor = attachment.FormatDescriptor;
				}
			}

			context.Attachments = attachmentStates;
			pipelineStates.EmplaceBack(context);

			if (!transparency) {
				GAL::Pipeline::PipelineStateBlock::DepthState depth;
				depth.CompareOperation = INVERSE_Z ? GAL::CompareOperation::GREATER : GAL::CompareOperation::LESS;
				pipelineStates.EmplaceBack(depth);
			}

			GAL::Pipeline::PipelineStateBlock::RasterState rasterState;
			rasterState.CullMode = GAL::CullMode::CULL_BACK;
			rasterState.WindingOrder = GAL::WindingOrder::CLOCKWISE;
			pipelineStates.EmplaceBack(rasterState);

			GAL::Pipeline::PipelineStateBlock::ViewportState viewportState;
			viewportState.ViewportCount = 1;
			pipelineStates.EmplaceBack(viewportState);

			auto& vertexState = pipelineStates.EmplaceBack(GAL::Pipeline::PipelineStateBlock::VertexState{});

			GTSL::StaticVector<GTSL::Range<const GAL::Pipeline::VertexElement*>, 8> vertexStreamsRanges;

			for(auto& vs : vertexStreams) { vertexStreamsRanges.EmplaceBack(vs); }

			vertexState.Vertex.VertexStreams = vertexStreamsRanges;

			pipelines[e.PipelineIndex].pipeline.InitializeRasterPipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}

		if (e.Stage & GAL::ShaderStages::COMPUTE) {
			if (shaderGroup.ComputePipelineIndex == 0xFFFFFFFF) { //if no pipeline already exists for this stage, create one
				shaderGroup.ComputePipelineIndex = pipelines.Emplace(GetPersistentAllocator());
			}

			e.PipelineIndex = shaderGroup.ComputePipelineIndex;

			auto& pipeline = pipelines[e.PipelineIndex];

			for (auto s : e.Shaders) {
				auto& shaderInfo = shaderInfos.EmplaceBack();
				auto& shader = shaders[shader_group_info.Shaders[s].Hash];
				shaderInfo.Type = shader.Type;
				shaderInfo.Shader = shader.Shader;
			}

			pipeline.ExecutionString = executionString;

			pipeline.pipeline.InitializeComputePipeline(renderSystem->GetRenderDevice(), pipelineStates, shaderInfos, setLayoutDatas[globalSetLayout()].PipelineLayout, renderSystem->GetPipelineCache());
		}

		if (e.Stage & (GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT)) {
			if (!BE::Application::Get()->GetBoolOption(u8"rayTracing")) { continue; }

			if(auto r = rayTracingSets.TryEmplace(Id(e.Set), 0xFFFFFFFFu)) {
				shaderGroup.RTPipelineIndex = pipelines.Emplace(GetPersistentAllocator());
				r.Get() = shaderGroup.RTPipelineIndex;
			} else {
				shaderGroup.RTPipelineIndex = r.Get();
			}

			e.PipelineIndex = shaderGroup.RTPipelineIndex;

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
				//shaderInfo.Blob = GTSL::Range(shader_group_info.Shaders[s].Size, shaderLoadInfo.Buffer.GetData() + offset);
			}

			GTSL::Vector<GPUPipeline::RayTraceGroup, BE::TAR> rayTracingGroups(16, GetTransientAllocator());

			GPUPipeline::PipelineStateBlock::RayTracingState rayTracePipelineState;
			rayTracePipelineState.MaxRecursionDepth = 1;

			for (uint32 i = 0; i < pipelineData.Shaders; ++i) {
				auto& shaderInfo = shaders[pipelineData.Shaders[i]];

				GPUPipeline::RayTraceGroup group; uint8 rtShaderGroupIndex = 0xFF;

				switch (shaderInfo.Type) {
				case GAL::ShaderType::RAY_GEN:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					rtShaderGroupIndex = GAL::RAY_GEN_TABLE_INDEX;
					GTSL::Max(&rayTracePipelineState.MaxRecursionDepth, static_cast<uint8>(1));
					break;
				case GAL::ShaderType::MISS:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					rtShaderGroupIndex = GAL::MISS_TABLE_INDEX;
					break;
				case GAL::ShaderType::CALLABLE:
					group.ShaderGroup = GAL::ShaderGroupType::GENERAL; group.GeneralShader = i;
					rtShaderGroupIndex = GAL::CALLABLE_TABLE_INDEX;
					break;
				case GAL::ShaderType::CLOSEST_HIT:
					group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
					rtShaderGroupIndex = GAL::HIT_TABLE_INDEX;
					break;
				case GAL::ShaderType::ANY_HIT:
					group.ShaderGroup = GAL::ShaderGroupType::TRIANGLES; group.AnyHitShader = i;
					rtShaderGroupIndex = GAL::HIT_TABLE_INDEX;
					break;
				case GAL::ShaderType::INTERSECTION:
					group.ShaderGroup = GAL::ShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
					rtShaderGroupIndex = GAL::HIT_TABLE_INDEX;
					break;
				default: BE_LOG_MESSAGE(u8"Non raytracing shader found in raytracing material");
				}

				rayTracingGroups.EmplaceBack(group);

				if (loadedShadersMap.Find(pipelineData.Shaders[i])) { // Only increment shader count when a new shader is added (not when updated since the shader is already there)
					++pipelineData.RayTracingData.ShaderGroups[rtShaderGroupIndex].ShaderCount;
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

		signalDependencyToResource(shaderGroup.Resource); //add ref count for pipeline load itself, todo: do we signal even when we are doing a pipeline update?

		for(auto& k : shader_group_info.Instances) {
			if(!shaderGroupInstanceByName.Find(k.Name)) { continue; }
			auto& instance = shaderGroupInstances[shaderGroupInstanceByName[k.Name]];
			signalDependencyToResource(instance.Resource); //add ref count for pipeline load itself, todo: do we signal even when we are doing a pipeline update?
		}
	}

	if (!shaderGroup.Loaded) {
		shaderGroup.Loaded = true;

		GTSL::StaticString<64> scope(u8"global"); scope << u8"." << GTSL::StringView(shader_group_info.Name);

		auto materialDataMember = RegisterType(scope, u8"ShaderParametersData", members);
		shaderGroup.Buffer = MakeDataKey(renderSystem, scope, u8"ShaderParametersData[4]", shaderGroup.Buffer); // Create shader group data in array, with an element for each instance

		auto bwk = GetBufferWriteKey(renderSystem, shaderGroup.Buffer);

		for (uint8 ii = 0; auto & i : shader_group_info.Instances) { //TODO: check parameters against stored layout to check if everything is still compatible
			if(!shaderGroupInstanceByName.Find(i.Name)) { continue; }
			auto& instance = shaderGroupInstances[shaderGroupInstanceByName[i.Name]];

			WriteUpdateKey(renderSystem, instance.UpdateKey, uint32(ii));

			CopyDataKey(instance.DataKey, shaderGroup.Buffer, instance.Name == u8"BlurV" ? 8 : 0);

			auto instanceElement = bwk[ii];

			for (uint32 pi = 0; auto & p : i.Parameters) {
				GTSL::StaticString<32> parameterValue;

				const auto& parameter = parameters[p.First];

				// If shader group instance has specialized value for parameter, use that, else, fallback to shader group default value for parameter.
				if (p.Second) {
					parameterValue = p.Second;
				} else {
					parameterValue = parameter.DefaultValue;
				}

				switch (GTSL::Hash(parameter.Type)) {
				case GTSL::Hash(u8"vec2u"): {
					struct vec2u { uint32 x, y; };

					vec2u vec = { GTSL::ToNumber<uint32>({1, 1, parameterValue.c_str()}).Get(), GTSL::ToNumber<uint32>({1, 1, parameterValue.c_str() + 3}).Get() };

					instanceElement[parameter.Name] = vec;
					break;
				}
				case GTSL::Hash(u8"TextureReference"): {
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.ApplicationManager;
					createTextureInfo.TextureResourceManager = taskInfo.ApplicationManager->GetSystem<TextureResourceManager>(u8"TextureResourceManager");
					createTextureInfo.TextureName = parameterValue;
					auto textureReference = createTexture(createTextureInfo);

					instanceElement[p.First] = textureReference;

					for (auto& e : shaderBundles) {
						addPendingResourceToTexture(parameterValue, instance.Resource);
					}

					break;
				}
				case GTSL::Hash(u8"ImageReference"): {
					if (auto textureReference = attachments.TryGet(parameterValue)) {
						uint32 textureComponentIndex = textureReference.Get().ImageIndeces[0]; // TODO: noo
						instanceElement[p.First] = textureComponentIndex;
					} else {
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
			if (!BE::Application::Get()->GetBoolOption(u8"rayTracing")) { continue; }

			auto& pipeline = pipelines[e.PipelineIndex]; auto& rtPipelineData = pipeline.RayTracingData;

			GTSL::Vector<GAL::ShaderHandle, BE::TAR> shaderGroupHandlesBuffer(e.Shaders.GetLength(), GetTransientAllocator());
			pipeline.pipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, pipeline.Shaders.GetLength(), shaderGroupHandlesBuffer);
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
			RegisterType(GTSL::StaticString<128>(u8"global") << u8"." << GTSL::StringView(shader_group_info.Name), u8"ShaderTableData", tables);
			pipeline.ShaderBindingTableBuffer = MakeDataKey(renderSystem, GTSL::StaticString<128>(u8"global") << u8"." << GTSL::StringView(shader_group_info.Name), u8"ShaderTableData", pipeline.ShaderBindingTableBuffer, GAL::BufferUses::SHADER_BINDING_TABLE);

			auto bWK = GetBufferWriteKey(renderSystem, pipeline.ShaderBindingTableBuffer);

			for (uint32 shaderGroupIndex = 0, shaderCount = 0; shaderGroupIndex < 4; ++shaderGroupIndex) {
				auto& groupData = rtPipelineData.ShaderGroups[shaderGroupIndex];
				for (uint32 i = 0; i < groupData.ShaderCount; ++i, ++shaderCount) {
					auto table = bWK[tables[shaderGroupIndex].Name];
					table[u8"shaderHandle"] = shaderGroupHandlesBuffer[shaderCount];

					uint64 shaderHandleHash = 0;

					shaderHandleHash = quickhash64({ 32, reinterpret_cast<byte*>(&shaderGroupHandlesBuffer[shaderCount]) });

					if(auto r = shaderHandlesDebugMap.TryEmplace(shaderHandleHash, shaders[pipeline.Shaders[shaderCount]].Name); !r) {
						BE_LOG_ERROR(u8"Could not emplace ");
					}
				}
			}
		}
	}
}

uint32 RenderOrchestrator::createTexture(const CreateTextureInfo& createTextureInfo) {

	if (auto t = textures.TryEmplace(createTextureInfo.TextureName)) {
		t.Get().Index = textureIndex++; ++imageIndex;
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