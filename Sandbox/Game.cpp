#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Resources/ShaderResourceManager.h"

#include "ByteEngine/Game/CameraSystem.h"

#include "ByteEngine/fpfParser.h"
#include "ByteEngine/Render/RenderOrchestrator.h"
#include "ByteEngine/Render/UIManager.h"

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
	if (!GameApplication::Initialize()) { return false; }//

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	gameInstance = GTSL::SmartPointer<GameInstance, BE::SystemAllocatorReference>(systemAllocatorReference);
	sandboxGameInstance = gameInstance;

	GTSL::Array<Id, 2> a({ u8"MouseMove" });
	inputManagerInstance->Register2DInputEvent(u8"Move", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

	a.PopBack(); a.EmplaceBack(u8"W_Key");
	inputManagerInstance->RegisterActionInputEvent(u8"Move Forward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveForward>(this));
	a.PopBack(); a.EmplaceBack(u8"A_Key");
	inputManagerInstance->RegisterActionInputEvent(u8"Move Left", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveLeft>(this));
	a.PopBack(); a.EmplaceBack(u8"S_Key");
	inputManagerInstance->RegisterActionInputEvent(u8"Move Backward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveBackwards>(this));
	a.PopBack(); a.EmplaceBack(u8"D_Key");
	inputManagerInstance->RegisterActionInputEvent(u8"Move Right", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveRight>(this));
	a.PopBack(); a.EmplaceBack(u8"MouseWheel");
	inputManagerInstance->RegisterLinearInputEvent(u8"Zoom", a, GTSL::Delegate<void(InputManager::LinearInputEvent)>::Create<Game, &Game::zoom>(this));
	a.PopBack(); a.EmplaceBack(u8"RightStick");
	inputManagerInstance->Register2DInputEvent(u8"View", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));
	a.PopBack(); a.EmplaceBack(u8"LeftStick");
	inputManagerInstance->Register2DInputEvent(u8"Move Camera", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::moveCamera>(this));
	a.PopBack(); a.EmplaceBack(u8"LeftMouseButton"); a.EmplaceBack(u8"RightTrigger");
	inputManagerInstance->RegisterActionInputEvent(u8"Left Click", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::leftClick>(this));

	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	{
		ShaderResourceManager::ShaderGroupCreateInfo shaderGroupCreateInfo;
		shaderGroupCreateInfo.Name = u8"PlainMaterial";
		shaderGroupCreateInfo.RenderPass = u8"SceneRenderPass";

		auto& vertexShader = shaderGroupCreateInfo.Shaders.EmplaceBack();
		vertexShader.Name = u8"VertexShader";
		vertexShader.Type = GAL::ShaderType::VERTEX;
		
		ShaderResourceManager::VertexShader vertex_shader;
		vertex_shader.VertexElements.PushBack({ GAL::Pipeline::POSITION, GAL::ShaderDataType::FLOAT3 });
		vertex_shader.VertexElements.PushBack({ GAL::Pipeline::NORMAL, GAL::ShaderDataType::FLOAT3 });
		
		vertexShader.VertexShader = vertex_shader;
		
		auto& fragmentShader = shaderGroupCreateInfo.Shaders.EmplaceBack();
		ShaderResourceManager::FragmentShader fragment_shader;
		fragmentShader.Name = u8"FragmentShader";
		fragmentShader.Type = GAL::ShaderType::FRAGMENT;
		fragment_shader.WriteOperation = GAL::BlendOperation::WRITE;
		fragmentShader.FragmentShader = fragment_shader;

		{
			shaderGroupCreateInfo.MaterialInstances.EmplaceBack();
			shaderGroupCreateInfo.MaterialInstances.back().Name = u8"plainMaterial";
		}
		
		GetResourceManager<ShaderResourceManager>(u8"ShaderResourceManager")->CreateShaderGroup(shaderGroupCreateInfo);
	}
	
	//show loading screen//
	//load menu
	//show menu
	//start game

	return true;
}

void Game::PostInitialize()
{
	//BE_LOG_LEVEL(BE::Logger::VerbosityLevel::WARNING);
	
	GameApplication::PostInitialize();

	{
		auto* cameraSystem = gameInstance->GetSystem<CameraSystem>(u8"CameraSystem");
	
		camera = cameraSystem->AddCamera(GTSL::Vector3(0, 0.5, -2));
		fov = cameraSystem->GetFieldOfView(camera);
	}
	
	
	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup");
	auto* renderOrchestrator = gameInstance->GetSystem<RenderOrchestrator>(u8"RenderOrchestrator");
	auto* renderSystem = gameInstance->GetSystem<RenderSystem>(u8"RenderSystem");
	//auto* audioSystem = gameInstance->GetSystem<AudioSystem>("AudioSystem");
	
	//{
	//	RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = renderSystem;
	//	createMaterialInfo.ShaderResourceManager = GetResourceManager<ShaderResourceManager>("ShaderResourceManager");
	//	createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	createMaterialInfo.MaterialName = "HydrantMat";
	//	createMaterialInfo.InstanceName = "tvMat";
	//	tvMaterialInstance = renderOrchestrator->CreateMaterial(createMaterialInfo);
	//}
	
	{
		RenderOrchestrator::CreateMaterialInfo createMaterialInfo;
		createMaterialInfo.GameInstance = gameInstance;
		createMaterialInfo.RenderSystem = renderSystem;
		createMaterialInfo.ShaderResourceManager = GetResourceManager<ShaderResourceManager>(u8"ShaderResourceManager");
		createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>(u8"TextureResourceManager");
		createMaterialInfo.MaterialName = u8"PlainMaterial";
		createMaterialInfo.InstanceName = u8"plainMaterial";
		plainMaterialInstance = renderOrchestrator->CreateMaterial(createMaterialInfo);
	}
	
	//audioEmitter = audioSystem->CreateAudioEmitter();
	//audioListener = audioSystem->CreateAudioListener();
	//audioSystem->SetAudioListener(audioListener);
	//audioSystem->BindAudio(audioEmitter, "gunshot");
	//audioSystem->SetLooping(audioEmitter, true)
	
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
	//}//
	
	//{		
	//	StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
	//	addStaticMeshInfo.MeshName = "TV";
	//	addStaticMeshInfo.Material = tvMaterialInstance;
	//	addStaticMeshInfo.GameInstance = gameInstance;
	//	addStaticMeshInfo.RenderSystem = renderSystem;
	//	addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
	//	tv = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
	//
	//	GTSL::Math::SetTranslation(staticMeshRenderer->GetTransformation(tv), { 0, 0, 1 });
	//	
	//	//auto tv2 = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
	//	//GTSL::Math::SetTranslation(staticMeshRenderer->GetTransformation(tv2), { 0, 1, 1 });
	//}
	//
	{		
		StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
		addStaticMeshInfo.MeshName = u8"plane";
		addStaticMeshInfo.Material = plainMaterialInstance;
		addStaticMeshInfo.GameInstance = gameInstance;
		addStaticMeshInfo.RenderSystem = renderSystem;
		addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>(u8"StaticMeshResourceManager");
		plane = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
		
		auto position = staticMeshRenderer->GetMeshPosition(plane);
		staticMeshRenderer->SetPosition(plane, { 0, 0, 0 });
		
		GTSL::Math::SetRotation(staticMeshRenderer->GetTransformation(plane), GTSL::Rotator(-GTSL::Math::PI / 2, 0, 0));
		////GTSL::Math::SetRotation(staticMeshRenderer->GetTransformation(plane), GTSL::AxisAngle(1, 0, 0, GTSL::Math::PI / 2));
		////GTSL::Math::SetRotation(staticMeshRenderer->GetTransformation(plane), GTSL::Quaternion(0.707, 0, 0, 0.707));
		GTSL::Math::AddScale(staticMeshRenderer->GetTransformation(plane), { 2, 2, 2 });//
	}

	
	//{
	//	auto* uiManager = gameInstance->GetSystem<UIManager>("UIManager");
	//
	//	uiManager->AddColor("sandboxRed", { 0.9607f, 0.2588f, 0.2588f, 1.0f });
	//	uiManager->AddColor("sandboxYellow", { 0.9607f, 0.7843f, 0.2588f, 1.0f });
	//	uiManager->AddColor("sandboxGreen", { 0.2882f, 0.9507f, 0.4588f, 1.0f });
	//	
	//	auto* canvasSystem = gameInstance->GetSystem<CanvasSystem>("CanvasSystem");
	//	auto canvas = canvasSystem->CreateCanvas("MainCanvas");
	//	canvasSystem->SetExtent(canvas, { 1280, 720 });
	//
	//	uiManager->AddCanvas(canvas);
	//
	//	auto organizerComp = canvasSystem->AddOrganizer(canvas, "TopBar");
	//	canvasSystem->SetAspectRatio(organizerComp, { 2, 0.06f });
	//	canvasSystem->SetAlignment(organizerComp, Alignment::RIGHT);
	//	canvasSystem->SetPosition(organizerComp, { 0, 0.96f });
	//	canvasSystem->SetSizingPolicy(organizerComp, SizingPolicy::SET_ASPECT_RATIO);
	//	canvasSystem->SetScalingPolicy(organizerComp, ScalingPolicy::FROM_SCREEN);
	//	canvasSystem->SetSpacingPolicy(organizerComp, SpacingPolicy::PACK);
	//
	//	auto minimizeButtonComp = canvasSystem->AddSquare();
	//	canvasSystem->SetColor(minimizeButtonComp, "sandboxGreen");
	//	canvasSystem->SetMaterial(minimizeButtonComp, buttonMaterial);
	//	canvasSystem->AddToOrganizer(organizerComp, minimizeButtonComp);
	//	
	//	auto toggleButtonComp = canvasSystem->AddSquare();
	//	canvasSystem->SetColor(toggleButtonComp, "sandboxYellow");
	//	canvasSystem->SetMaterial(toggleButtonComp, buttonMaterial);
	//	canvasSystem->AddToOrganizer(organizerComp, toggleButtonComp);
	//
	//	auto closeButtonComp = canvasSystem->AddSquare();
	//	canvasSystem->SetColor(closeButtonComp, "sandboxRed");
	//	canvasSystem->SetMaterial(closeButtonComp, buttonMaterial);
	//	canvasSystem->AddToOrganizer(organizerComp, closeButtonComp);
	//}
	
	//{
	//	MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	createMaterialInfo.ShaderResourceManager = GetResourceManager<ShaderResourceManager>("ShaderResourceManager");
	//	createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	createMaterialInfo.MaterialName = "TvMat";
	//	tvMat = material_system->CreateMaterial(createMaterialInfo);
	//}//
	
	//{
	//	auto* lightsRenderGroup = gameInstance->GetSystem<LightsRenderGroup>("LightsRenderGroup");
	//	auto light = lightsRenderGroup->CreateDirectionalLight();
	//	lightsRenderGroup->SetColor(light, { 1.0f, 0.98f, 0.98f, 1.0f });
	//	lightsRenderGroup->SetRotation(light, { -0.785398f, 0.0f, 0.0f });
	//	auto pointLight = lightsRenderGroup->CreatePointLight();
	//	lightsRenderGroup->SetRadius(pointLight, 1);
	//}
}

void Game::OnUpdate(const OnUpdateInfo& onUpdate)
{
	//auto* material_system = gameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	//auto* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//auto* audioSystem = gameInstance->GetSystem<AudioSystem>("AudioSystem");
	//
	auto deltaSeconds = GetClock()->GetDeltaTime().As<float32, GTSL::Seconds>();
	//
	//if (shouldFire)
	//{
	//	inputManagerInstance->SetInputDeviceParameter(controller, "HighEndVibration", 1.0f);
	//	audioSystem->PlayAudio(audioEmitter);
	//	shouldFire = false;
	//} else {
	//	inputManagerInstance->SetInputDeviceParameter(controller, "HighEndVibration", GTSL::Math::Interp(0, inputManagerInstance->GetInputDeviceParameter(controller, "HighEndVibration"), deltaSeconds, 2));
	//}
	//
	GameApplication::OnUpdate(onUpdate);
	//
	auto* cameraSystem = gameInstance->GetSystem<CameraSystem>(u8"CameraSystem");
	//
	auto cameraDirection = GTSL::Quaternion(GTSL::Rotator(0, -posDelta.X(), 0));
	auto dir = cameraDirection * moveDir;
	//
	//
	auto camPos = GTSL::Math::Interp(cameraSystem->GetCameraPosition(camera) + dir, cameraSystem->GetCameraPosition(camera), deltaSeconds, 1);
	//
	//audioSystem->SetPosition(audioListener, camPos);
	//audioSystem->SetOrientation(audioListener, cameraDirection);
	cameraSystem->SetCameraPosition(camera, camPos);
	cameraSystem->SetFieldOfView(camera, GTSL::Math::DegreesToRadians(GTSL::Math::Interp(fov, GTSL::Math::RadiansToDegrees(cameraSystem->GetFieldOfView(camera)), deltaSeconds, 18.0f)));
	//
	//auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	//
	//auto hydrantPos = GTSL::Vector3(0, GTSL::Math::Sine(GetClock()->GetElapsedTime().As<float32, GTSL::Seconds>()) / 4, 2);
	//
	////staticMeshRenderer->SetPosition(hydrant, hydrantPos);
	////staticMeshRenderer->SetPosition(tv, GTSL::Vector3(0, 0, 0));
}

void Game::Shutdown()
{
	GameApplication::Shutdown();
}

void Game::move(InputManager::Vector2DInputEvent data)
{
	//posDelta += (data.Value - data.LastValue) * 2;
	data.Value.X() *= -1;
	posDelta = GTSL::Math::Wrap(posDelta + data.Value * 0.005f, GTSL::Vector2(GTSL::Math::PI));
	
	//auto rot = GTSL::Matrix4(GTSL::AxisAngle(0.f, 1.0f, 0.f, posDelta.X()));//inMesh->mFaces[face].mIndices[index]
	auto rot = GTSL::Matrix4(GTSL::Rotator(0, posDelta.X(), 0));
	rot *= GTSL::Matrix4(GTSL::AxisAngle(GTSL::Vector3(rot.GetXBasisVector()), posDelta.Y()));
	
	//auto rot = GTSL::Quaternion(GTSL::AxisAngle(0.f, 1.0f, 0.f, 0));
	gameInstance->GetSystem<CameraSystem>(u8"CameraSystem")->SetCameraRotation(camera, rot);
}

Game::~Game()
{
}
