#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

#include "ByteEngine/Game/CameraSystem.h"

#include <GTSL/Math/AxisAngle.h>


#include "ByteEngine/fpfParser.h"
#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Render/LightsRenderGroup.h"
#include "ByteEngine/Render/MaterialSystem.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"
#include "ByteEngine/Render/UIManager.h"
#include "ByteEngine/Sound/AudioSystem.h"

class UIManager;
class TestSystem;

void Game::leftClick(InputManager::ActionInputEvent data)
{
	shouldFire = data.Value;
}

void Game::moveLeft(InputManager::ActionInputEvent data)
{
	moveDir.X() = -data.Value;
}

void Game::moveForward(InputManager::ActionInputEvent data)
{
	moveDir.Z() = data.Value;
}

void Game::moveBackwards(InputManager::ActionInputEvent data)
{
	moveDir.Z() = -data.Value;
}

void Game::moveRight(InputManager::ActionInputEvent data)
{
	moveDir.X() = data.Value;
}

void Game::zoom(InputManager::LinearInputEvent data)
{
	fov += data.Value * 3;
}

void Game::moveCamera(InputManager::Vector2DInputEvent data)
{
	if (GTSL::Math::Length(data.Value) > 0.2) {
		moveDir = GTSL::Vector3(data.Value.X(), 0, data.Value.Y()) * 0.5;
	}
	else
	{
		moveDir = GTSL::Vector3(0, 0, 0);
	}
}

bool Game::Initialize()
{
	if (!GameApplication::Initialize()) { return false; }

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	gameInstance = GTSL::SmartPointer<GameInstance, BE::SystemAllocatorReference>::Create<SandboxGameInstance>(systemAllocatorReference);
	sandboxGameInstance = gameInstance;

	GTSL::Array<Id, 2> a({ "MouseMove" });
	inputManagerInstance->Register2DInputEvent("Move", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

	a.PopBack(); a.EmplaceBack("W_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Forward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveForward>(this));
	a.PopBack(); a.EmplaceBack("A_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Left", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveLeft>(this));
	a.PopBack(); a.EmplaceBack("S_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Backward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveBackwards>(this));
	a.PopBack(); a.EmplaceBack("D_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Right", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveRight>(this));
	a.PopBack(); a.EmplaceBack("MouseWheel");
	inputManagerInstance->RegisterLinearInputEvent("Zoom", a, GTSL::Delegate<void(InputManager::LinearInputEvent)>::Create<Game, &Game::zoom>(this));
	a.PopBack(); a.EmplaceBack("RightStick");
	inputManagerInstance->Register2DInputEvent("View", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

	a.PopBack(); a.EmplaceBack("LeftStick");//
	inputManagerInstance->Register2DInputEvent("Move Camera", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::moveCamera>(this));
	
	a.PopBack(); a.EmplaceBack("LeftMouseButton"); a.EmplaceBack("RightTrigger");
	inputManagerInstance->RegisterActionInputEvent("Left Click", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::leftClick>(this));

	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	{
		MaterialResourceManager::RasterMaterialCreateInfo materialCreateInfo;
		materialCreateInfo.ShaderName = "HydrantMat";
		materialCreateInfo.RenderGroup = "StaticMeshRenderGroup";
		materialCreateInfo.RenderPass = "SceneRenderPass";
		GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT2 };
		materialCreateInfo.VertexFormat = format;
		materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };
		
		materialCreateInfo.Parameters.EmplaceBack("albedo", MaterialResourceManager::ParameterType::TEXTURE_REFERENCE);
		
		materialCreateInfo.DepthWrite = true;
		materialCreateInfo.DepthTest = true;
		materialCreateInfo.StencilTest = false;
		materialCreateInfo.CullMode = GAL::CullMode::CULL_BACK;
		materialCreateInfo.BlendEnable = false;
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;

		{
			materialCreateInfo.MaterialInstances.EmplaceBack();
			materialCreateInfo.MaterialInstances.back().Name = "hydrantMat";
			materialCreateInfo.MaterialInstances.back().Parameters.EmplaceBack();
			materialCreateInfo.MaterialInstances.back().Parameters.back().First = "albedo";
			materialCreateInfo.MaterialInstances.back().Parameters.back().Second.TextureReference = "hydrant_Albedo";
		}

		{
			materialCreateInfo.MaterialInstances.EmplaceBack();
			materialCreateInfo.MaterialInstances.back().Name = "tvMat";
			materialCreateInfo.MaterialInstances.back().Parameters.EmplaceBack();
			materialCreateInfo.MaterialInstances.back().Parameters.back().First = "albedo";
			materialCreateInfo.MaterialInstances.back().Parameters.back().Second.TextureReference = "TV_Albedo";
		}
		
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRasterMaterial(materialCreateInfo);
	}
	
	//{
	//	MaterialResourceManager::RasterMaterialCreateInfo materialCreateInfo;
	//	materialCreateInfo.ShaderName = "TextMaterial";
	//	materialCreateInfo.RenderGroup = "TextSystem";
	//	materialCreateInfo.RenderPass = "MainRenderPass";
	//	materialCreateInfo.SubPass = "Text";
	//	GTSL::Array<GAL::ShaderDataType, 8> format;
	//	materialCreateInfo.VertexFormat = format;
	//	materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };
	//	materialCreateInfo.DepthWrite = false;
	//	materialCreateInfo.DepthTest = false;
	//	materialCreateInfo.StencilTest = false;
	//	materialCreateInfo.CullMode = GAL::CullMode::CULL_BACK;
	//	materialCreateInfo.BlendEnable = true;
	//	materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
	//	GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRasterMaterial(materialCreateInfo);
	//}
	
	//{
	//	MaterialResourceManager::RasterMaterialCreateInfo materialCreateInfo{};
	//	materialCreateInfo.ShaderName = "UIMat";
	//	materialCreateInfo.RenderGroup = "UIRenderGroup";
	//	materialCreateInfo.RenderPass = "UIRenderPass";
	//	GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT2 };
	//	materialCreateInfo.VertexFormat = format;
	//	materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };
	//	materialCreateInfo.DepthWrite = true;
	//	materialCreateInfo.DepthTest = true;
	//	materialCreateInfo.StencilTest = false;
	//	materialCreateInfo.CullMode = GAL::CullMode::CULL_NONE;
	//	materialCreateInfo.BlendEnable = false;
	//	materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
	//	GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRasterMaterial(materialCreateInfo);
	//}
	
	{
		MaterialResourceManager::RayTracePipelineCreateInfo pipelineCreateInfo;
		pipelineCreateInfo.RecursionDepth = 3;
		pipelineCreateInfo.Payload.EmplaceBack(MaterialResourceManager::ParameterType::FVEC4);
		pipelineCreateInfo.PipelineName = "ScenePipeline";

		{
			auto& shader = pipelineCreateInfo.Shaders.EmplaceBack();
			shader.ShaderName = "RayGen";
			shader.Type = GAL::ShaderType::RAY_GEN;
			shader.MaterialInstances.EmplaceBack();
		}

		{
			auto& shader = pipelineCreateInfo.Shaders.EmplaceBack();
			shader.ShaderName = "ClosestHit";
			shader.Type = GAL::ShaderType::CLOSEST_HIT;
			auto& hydrantInstance = shader.MaterialInstances.EmplaceBack();
			hydrantInstance.EmplaceBack("StaticMeshRenderGroup");
			hydrantInstance.EmplaceBack("HydrantMat");
			auto& tvInstance = shader.MaterialInstances.EmplaceBack();
			tvInstance.EmplaceBack("StaticMeshRenderGroup");
			tvInstance.EmplaceBack("HydrantMat");
		}

		{
			auto& shader = pipelineCreateInfo.Shaders.EmplaceBack();
			shader.ShaderName = "Miss";
			shader.Type = GAL::ShaderType::MISS;
			shader.MaterialInstances.EmplaceBack();
		}
		
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRayTracePipeline(pipelineCreateInfo);
	}
	
	//show loading screen
	//load menu
	//show menu//
	//start game

	return true;
}

void Game::PostInitialize()
{
	//BE_LOG_LEVEL(BE::Logger::VerbosityLevel::WARNING);
	
	GameApplication::PostInitialize();

	{
		auto* cameraSystem = gameInstance->GetSystem<CameraSystem>("CameraSystem");

		camera = cameraSystem->AddCamera(GTSL::Vector3(0, 0, -250));
		fov = cameraSystem->GetFieldOfView(camera);
	}
	
	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	auto* renderOrchestrator = gameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");
	auto* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* audioSystem = gameInstance->GetSystem<AudioSystem>("AudioSystem");
	
	{
		RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
		createMaterialInfo.GameInstance = gameInstance;
		createMaterialInfo.RenderSystem = renderSystem;
		createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
		createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
		createMaterialInfo.MaterialName = "HydrantMat";
		material = renderOrchestrator->CreateMaterial(createMaterialInfo);
	}

	audioEmitter = audioSystem->CreateAudioEmitter();
	audioListener = audioSystem->CreateAudioListener();
	audioSystem->SetAudioListener(audioListener);
	audioSystem->BindAudio(audioEmitter, "gunshot");
	//audioSystem->SetLooping(audioEmitter, true);

	auto hydrantMaterialInstance = renderOrchestrator->GetMaterialHandle("hydrantMat");
	auto tvMaterialInstance = renderOrchestrator->GetMaterialHandle("tvMat");
	
	{
		StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
		addStaticMeshInfo.MeshName = "hydrant";
		addStaticMeshInfo.Material = hydrantMaterialInstance;
		addStaticMeshInfo.GameInstance = gameInstance;
		addStaticMeshInfo.RenderSystem = renderSystem;
		addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
		hydrant = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
	}
	
	//{
	//	auto fpfString = GTSL::StaticString<512>(R"(class AudioFile { uint32 FrameCount } class AudioFormat { uint32 KHz uint32 BitDepth AudioFile[] AudioFiles }
	//		{ AudioFormat[] audioFormats { { 48000, 16, { { 1400 }, { 1500 } } }, { 41000, 32, { { 1200 }, { 750 } } } } })");
	//
	//	FileDescription<BE::SystemAllocatorReference> fileDescription;
	//	auto result = BuildFileDescription(fpfString, systemAllocatorReference, fileDescription);
	//
	//	ParseState<BE::SystemAllocatorReference> parseState;
	//	StartParse(fileDescription, parseState, fpfString, systemAllocatorReference);
	//
	//	struct AudioFile { uint32 FrameCount; };
	//	struct AudioFormat { uint32 KHz, BitDepth; GTSL::Array<AudioFile, 8> AudioFiles; };
	//
	//	GTSL::Array<AudioFormat, 8> audioFormats;
	//	
	//	uint32 audioFormatIndex = 0;
	//
	//	GoToArray(fileDescription, parseState, "audioFormats", audioFormatIndex);
	//	while (GoToIndex(fileDescription, parseState, audioFormatIndex)) {
	//		AudioFormat audioFormat;
	//
	//		GetVariable(fileDescription, parseState, "KHz", audioFormat.KHz);
	//		GetVariable(fileDescription, parseState, "BitDepth", audioFormat.BitDepth);
	//
	//		uint32 audioFileIndex = 0;
	//
	//		GoToArray(fileDescription, parseState, "AudioFiles", audioFileIndex);
	//		while (GoToIndex(fileDescription, parseState, audioFileIndex)) {
	//			AudioFile audioFile;
	//			
	//			GetVariable(fileDescription, parseState, "FrameCount", audioFile.FrameCount);
	//
	//			audioFormat.AudioFiles.EmplaceBack(audioFile);
	//			
	//			++audioFileIndex;
	//		}
	//		
	//		audioFormats.EmplaceBack(audioFormat);
	//		
	//		++audioFormatIndex;
	//	}
	//
	//	uint32 t = 0;
	//}
	
	//{
	//	StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
	//	addStaticMeshInfo.MeshName = "TV";
	//	addStaticMeshInfo.Material = tvMaterialInstance;
	//	addStaticMeshInfo.GameInstance = gameInstance;
	//	addStaticMeshInfo.RenderSystem = renderSystem;
	//	addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
	//	tv = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
	//}

	
	//{
	//	auto* uiManager = gameInstance->GetSystem<UIManager>("UIManager");
	//
	//	uiManager->AddColor("sandboxRed", { 0.9607f, 0.2588f, 0.2588f, 1.0f });
	//	uiManager->AddColor("sandboxYellow", { 0.9607f, 0.7843f, 0.2588f, 1.0f });
	//	uiManager->AddColor("sandboxGreen", { 0.2882f, 0.9507f, 0.4588f, 1.0f });
	//	
	//	auto* canvasSystem = gameInstance->GetSystem<CanvasSystem>("CanvasSystem");
	//	auto canvas = canvasSystem->CreateCanvas("MainCanvas");
	//	auto& canvasRef = canvasSystem->GetCanvas(canvas);
	//	canvasRef.SetExtent({ 1280, 720 });
	//
	//	uiManager->AddCanvas(canvas);
	//
	//	auto organizerComp = canvasRef.AddOrganizer("TopBar");
	//	canvasRef.SetOrganizerAspectRatio(organizerComp, { 2, 0.06f });
	//	canvasRef.SetOrganizerAlignment(organizerComp, Alignment::RIGHT);
	//	canvasRef.SetOrganizerPosition(organizerComp, { 0, 0.96f });
	//	//canvasRef.SetOrganizerPosition(organizerComp, { 0, 0 });
	//	canvasRef.SetOrganizerSizingPolicy(organizerComp, SizingPolicy::SET_ASPECT_RATIO);
	//	canvasRef.SetOrganizerScalingPolicy(organizerComp, ScalingPolicy::FROM_SCREEN);
	//	canvasRef.SetOrganizerSpacingPolicy(organizerComp, SpacingPolicy::PACK);
	//
	//	auto minimizeButtonComp = canvasRef.AddSquare();
	//	canvasRef.SetSquareMaterial(minimizeButtonComp, buttonMaterial);
	//	canvasRef.SetSquareColor(minimizeButtonComp, "sandboxGreen");
	//	canvasRef.AddSquareToOrganizer(organizerComp, minimizeButtonComp);
	//	
	//	auto toggleButtonComp = canvasRef.AddSquare();
	//	canvasRef.SetSquareColor(toggleButtonComp, "sandboxYellow");
	//	canvasRef.SetSquareMaterial(toggleButtonComp, buttonMaterial);
	//	canvasRef.AddSquareToOrganizer(organizerComp, toggleButtonComp);
	//
	//	auto closeButtonComp = canvasRef.AddSquare();
	//	canvasRef.SetSquareColor(closeButtonComp, "sandboxRed");
	//	canvasRef.SetSquareMaterial(closeButtonComp, buttonMaterial);
	//	canvasRef.AddSquareToOrganizer(organizerComp, closeButtonComp);
	//}
	
	//{
	//	MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//	createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	createMaterialInfo.MaterialName = "TvMat";
	//	tvMat = material_system->CreateMaterial(createMaterialInfo);
	//}
	
	{
		auto* lightsRenderGroup = gameInstance->GetSystem<LightsRenderGroup>("LightsRenderGroup");
		auto light = lightsRenderGroup->CreateDirectionalLight();
		lightsRenderGroup->SetLightColor(light, { 1.0f, 0.98f, 0.98f, 1.0f });
		lightsRenderGroup->SetLightRotation(light, { -0.785398f, 0.0f, 0.0f });
	}
}

void Game::OnUpdate(const OnUpdateInfo& onUpdate)
{
	auto* material_system = gameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto* audioSystem = gameInstance->GetSystem<AudioSystem>("AudioSystem");

	if (shouldFire)
	{
		audioSystem->PlayAudio(audioEmitter);
		shouldFire = false;
	}
	
	GameApplication::OnUpdate(onUpdate);

	auto rotationMatrix = GTSL::Quaternion(GTSL::AxisAngle(0.f, 1.0f, 0.f, posDelta.X()));
	auto dir = rotationMatrix * (moveDir * 50);

	auto deltaSeconds = GetClock()->GetDeltaTime().As<float32, GTSL::Seconds>();
	
	auto* cameraSystem = gameInstance->GetSystem<CameraSystem>("CameraSystem");
	audioSystem->SetPosition(audioListener, cameraSystem->GetCameraPosition(camera) + dir);
	audioSystem->SetOrientation(audioListener, rotationMatrix);
	cameraSystem->SetCameraPosition(camera, GTSL::Math::Interp(cameraSystem->GetCameraPosition(camera) + dir, cameraSystem->GetCameraPosition(camera), deltaSeconds, 10));
	cameraSystem->SetFieldOfView(camera, GTSL::Math::DegreesToRadians(GTSL::Math::Interp(fov, GTSL::Math::RadiansToDegrees(cameraSystem->GetFieldOfView(camera)), deltaSeconds, 18.0f)));
	
	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");

	auto hydrantPos = GTSL::Vector3(0, GTSL::Math::Sine(GetClock()->GetElapsedTime() * 0.000009f) * 25, 250);
	
	staticMeshRenderer->SetPosition(hydrant, hydrantPos);
	//staticMeshRenderer->SetPosition(tv, GTSL::Vector3(GTSL::Math::Sine(GetClock()->GetElapsedTime() * 0.000009f) * 25 + 200, 0, 250));

	renderSystem->UpdateInstanceTransform(0, GTSL::Matrix4(hydrantPos));
}

void Game::Shutdown()
{
	GameApplication::Shutdown();
}

void Game::move(InputManager::Vector2DInputEvent data)
{
	//posDelta += (data.Value - data.LastValue) * 2;
	posDelta += data.Value * 0.005f;
	posDelta = GTSL::Math::Modulo(posDelta, GTSL::Math::PI * 2.0f);

	auto rot = GTSL::Matrix4(GTSL::AxisAngle(0.f, 1.0f, 0.f, posDelta.X()));
	rot *= GTSL::Matrix4(GTSL::AxisAngle(GTSL::Vector3(rot.GetXBasisVector()), -posDelta.Y()));
	gameInstance->GetSystem<CameraSystem>("CameraSystem")->SetCameraRotation(camera, rot);
}

Game::~Game()
{
}
