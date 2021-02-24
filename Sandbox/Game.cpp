#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

#include "ByteEngine/Game/CameraSystem.h"

#include <GTSL/Math/AxisAngle.h>

#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Render/LightsRenderGroup.h"
#include "ByteEngine/Render/MaterialSystem.h"
#include "ByteEngine/Render/StaticMeshRenderGroup.h"
#include "ByteEngine/Render/UIManager.h"
#include "ByteEngine/Resources/TextRendering.h"

class UIManager;
class TestSystem;

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
	fov += -(data.Value / 75);
}

bool Game::Initialize()
{
	if (!GameApplication::Initialize()) { return false; }

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	gameInstance = GTSL::SmartPointer<GameInstance, BE::SystemAllocatorReference>::Create<SandboxGameInstance>(systemAllocatorReference);
	sandboxGameInstance = gameInstance;

	GTSL::Array<GTSL::Id64, 2> a({ GTSL::Id64("MouseMove"), "RightStick" });
	inputManagerInstance->Register2DInputEvent("Move", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

	a.PopBack(); a.PopBack(); a.EmplaceBack("W_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Forward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveForward>(this));
	a.PopBack(); a.EmplaceBack("A_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Left", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveLeft>(this));
	a.PopBack(); a.EmplaceBack("S_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Backward", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveBackwards>(this));
	a.PopBack(); a.EmplaceBack("D_Key");
	inputManagerInstance->RegisterActionInputEvent("Move Right", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create<Game, &Game::moveRight>(this));
	a.PopBack(); a.EmplaceBack("MouseWheel");
	inputManagerInstance->RegisterLinearInputEvent("Zoom", a, GTSL::Delegate<void(InputManager::LinearInputEvent)>::Create<Game, &Game::zoom>(this));
	a.PopBack(); a.EmplaceBack("LeftStick");
	inputManagerInstance->Register2DInputEvent("View", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

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
	//	materialCreateInfo.ShaderName = "TvMat";
	//	materialCreateInfo.RenderGroup = "StaticMeshRenderGroup";
	//	materialCreateInfo.RenderPass = "SceneRenderPass";
	//	GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT2 };
	//	GTSL::Array<MaterialResourceManager::Binding, 8> binding_sets;
	//	binding_sets.EmplaceBack(GAL::BindingType::UNIFORM_BUFFER_DYNAMIC, GAL::ShaderStage::FRAGMENT);
	//	materialCreateInfo.VertexFormat = format;
	//	materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };
	//	materialCreateInfo.Textures.EmplaceBack("TV_Albedo");
	//	//materialCreateInfo.PerInstanceParameters.EmplaceBack("Color", GAL::ShaderDataType::FLOAT4);
	//	materialCreateInfo.Bindings = binding_sets;
	//	materialCreateInfo.DepthWrite = true;
	//	materialCreateInfo.DepthTest = true;
	//	materialCreateInfo.StencilTest = false;
	//	materialCreateInfo.CullMode = GAL::CullMode::CULL_BACK;
	//	materialCreateInfo.BlendEnable = false;
	//	materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
	//	GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRasterMaterial(materialCreateInfo);
	//}
	
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
	//}//
	
	{
		MaterialResourceManager::RayTraceMaterialCreateInfo materialCreateInfo;
		materialCreateInfo.Type = GAL::ShaderType::RAY_GEN;
		materialCreateInfo.ShaderName = "RayGen";
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRayTraceMaterial(materialCreateInfo);
	}

	{
		MaterialResourceManager::RayTraceMaterialCreateInfo materialCreateInfo;
		materialCreateInfo.Type = GAL::ShaderType::CLOSEST_HIT;
		materialCreateInfo.ShaderName = "ClosestHit";
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRayTraceMaterial(materialCreateInfo);
	}

	{
		MaterialResourceManager::RayTraceMaterialCreateInfo materialCreateInfo;
		materialCreateInfo.Type = GAL::ShaderType::MISS;
		materialCreateInfo.ShaderName = "Miss";
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateRayTraceMaterial(materialCreateInfo);
	}
	
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::PostInitialize()
{
	//BE_LOG_LEVEL(BE::Logger::VerbosityLevel::WARNING);
	
	GameApplication::PostInitialize();

	camera = gameInstance->GetSystem<CameraSystem>("CameraSystem")->AddCamera(GTSL::Vector3(0, 0, -250));
	
	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	auto* material_system = gameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	auto* fontResourceManager = GetResourceManager<FontResourceManager>("FontResourceManager");

	//FaceTree faceTree(GetPersistentAllocator());
	//faceTree.MakeFromPaths(fontResourceManager->GetFont(GTSL::StaticString<32>("FTLTLT")), GetPersistentAllocator());
	//faceTree.RenderChar({ 256, 256 }, 65, GetPersistentAllocator());

	//{
	//	StaticMeshRenderGroup::AddRayTracedStaticMeshInfo addStaticMeshInfo;
	//	addStaticMeshInfo.MeshName = "hydrant";
	//	addStaticMeshInfo.Material = material;
	//	addStaticMeshInfo.GameInstance = gameInstance;
	//	addStaticMeshInfo.RenderSystem = renderSystem;
	//	addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
	//	const auto component = staticMeshRenderer->AddRayTracedStaticMesh(addStaticMeshInfo);
	//}//
	
	{
		MaterialSystem::CreateMaterialInfo createMaterialInfo;
		createMaterialInfo.GameInstance = gameInstance;
		createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
		createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
		createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
		createMaterialInfo.MaterialName = "HydrantMat";
		material = material_system->CreateMaterial(createMaterialInfo);
	}

	//{
	//	MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//	createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	createMaterialInfo.MaterialName = "UIMat";
	//	buttonMaterial = material_system->CreateRasterMaterial(createMaterialInfo);
	//}

	auto hydrantMaterialInstance = material_system->GetMaterialHandle("hydrantMat");
	auto tvMaterialInstance = material_system->GetMaterialHandle("tvMat");
	
	{
		StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
		addStaticMeshInfo.MeshName = "hydrant";
		addStaticMeshInfo.Material = hydrantMaterialInstance;
		addStaticMeshInfo.GameInstance = gameInstance;
		addStaticMeshInfo.RenderSystem = renderSystem;
		addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
		hydrant = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
		//staticMeshRenderer->SetPosition(box, GTSL::Vector3(0, 0, 250));
	}

	{
		StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
		addStaticMeshInfo.MeshName = "TV";
		addStaticMeshInfo.Material = tvMaterialInstance;
		addStaticMeshInfo.GameInstance = gameInstance;
		addStaticMeshInfo.RenderSystem = renderSystem;
		addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
		tv = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
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
	//	auto& canvasRef = canvasSystem->GetCanvas(canvas);
	//	canvasRef.SetExtent({ 1280, 720 });//
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
	//	tvMat = material_system->CreateMaterial(createMaterialInfo);//
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
	
	GameApplication::OnUpdate(onUpdate);

	gameInstance->GetSystem<CameraSystem>("CameraSystem")->AddCameraPosition(camera, GTSL::Vector3(moveDir * 10));
	gameInstance->GetSystem<CameraSystem>("CameraSystem")->SetFieldOfView(camera, GTSL::Math::DegreesToRadians(fov));

	auto r = GTSL::Math::Sine(GetClock()->GetElapsedTime() / 1000000.0f);
	auto g = GTSL::Math::Sine(90.f + GetClock()->GetElapsedTime() / 1000000.0f);
	auto b = GTSL::Math::Sine(180.f + GetClock()->GetElapsedTime() / 1000000.0f);

	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	staticMeshRenderer->SetPosition(hydrant, GTSL::Vector3(0, GTSL::Math::Sine(GetClock()->GetElapsedTime() / 100000.0f) * 25, 250));
	staticMeshRenderer->SetPosition(tv, GTSL::Vector3(GTSL::Math::Sine(GetClock()->GetElapsedTime() / 100000.0f) * 20 + 200, 0, 250));

	//auto r = 1.0f;
	//auto g = 1.0f;
	//auto b = 1.0f;
	
	GTSL::RGBA color(r, g, b, 1.0);
	material_system->SetDynamicMaterialParameter(material, GAL::ShaderDataType::FLOAT4, "Color", &color);
}

void Game::Shutdown()
{
	GameApplication::Shutdown();
}

void Game::move(InputManager::Vector2DInputEvent data)
{
	posDelta += (data.Value - data.LastValue) * 2;

	auto rot = GTSL::Matrix4(GTSL::AxisAngle(0.f, 1.0f, 0.f, posDelta.X()));
	rot *= GTSL::Matrix4(GTSL::AxisAngle(rot.GetXBasisVector(), -posDelta.Y()));
	gameInstance->GetSystem<CameraSystem>("CameraSystem")->SetCameraRotation(camera, rot);
}

Game::~Game()
{
}
