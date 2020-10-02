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
#include "ByteEngine/Render/TextSystem.h"

class TestSystem;

void Game::moveLeft(InputManager::ActionInputEvent data)
{
	moveDir.X = -data.Value;
}

void Game::moveForward(InputManager::ActionInputEvent data)
{
	moveDir.Z = data.Value;
}

void Game::moveBackwards(InputManager::ActionInputEvent data)
{
	moveDir.Z = -data.Value;
}

void Game::moveRight(InputManager::ActionInputEvent data)
{
	moveDir.X = data.Value;
}

void Game::zoom(InputManager::LinearInputEvent data)
{
	fov += -(data.Value / 75);
}

void Game::Initialize()
{
	GameApplication::Initialize();

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
		MaterialResourceManager::MaterialCreateInfo materialCreateInfo;
		materialCreateInfo.ShaderName = "HydrantMat";
		materialCreateInfo.RenderGroup = "StaticMeshRenderGroup";
		materialCreateInfo.RenderPass = "MainRenderPass";
		materialCreateInfo.SubPass = "Scene";
		GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT2 };
		GTSL::Array<MaterialResourceManager::Binding, 8> binding_sets;
		binding_sets.EmplaceBack(GAL::BindingType::UNIFORM_BUFFER_DYNAMIC, GAL::ShaderStage::FRAGMENT);
		materialCreateInfo.VertexFormat = format;
		materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };

		materialCreateInfo.Textures.EmplaceBack("hydrant_Albedo");
		//materialCreateInfo.PerInstanceParameters.EmplaceBack("Color", GAL::ShaderDataType::FLOAT4);
		
		materialCreateInfo.Bindings = binding_sets;
		materialCreateInfo.DepthWrite = true;
		materialCreateInfo.DepthTest = true;
		materialCreateInfo.StencilTest = false;
		materialCreateInfo.CullMode = GAL::CullMode::CULL_BACK;
		materialCreateInfo.BlendEnable = false;
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateMaterial(materialCreateInfo);
	}

	//{
	//	MaterialResourceManager::MaterialCreateInfo materialCreateInfo;
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
	//	GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateMaterial(materialCreateInfo);
	//}//
	
	{
		MaterialResourceManager::MaterialCreateInfo materialCreateInfo;
		materialCreateInfo.ShaderName = "TvMat";
		materialCreateInfo.RenderGroup = "StaticMeshRenderGroup";
		materialCreateInfo.RenderPass = "MainRenderPass";
		materialCreateInfo.SubPass = "Scene";
		GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT2 };
		materialCreateInfo.VertexFormat = format;
		materialCreateInfo.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };

		materialCreateInfo.Textures.EmplaceBack("TV_Albedo");
		
		materialCreateInfo.DepthWrite = true;
		materialCreateInfo.DepthTest = true;
		materialCreateInfo.StencilTest = false;
		materialCreateInfo.CullMode = GAL::CullMode::CULL_BACK;
		materialCreateInfo.BlendEnable = false;
		materialCreateInfo.ColorBlendOperation = GAL::BlendOperation::ADD;
		GetResourceManager<MaterialResourceManager>("MaterialResourceManager")->CreateMaterial(materialCreateInfo);
	}
	
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::PostInitialize()
{
	GameApplication::PostInitialize();

	camera = gameInstance->GetSystem<CameraSystem>("CameraSystem")->AddCamera(GTSL::Vector3(0, 0, -250));
	
	auto* staticMeshRenderer = gameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	auto* material_system = gameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");

	//{
	//	TextureSystem::CreateTextureInfo createTextureInfo;
	//	createTextureInfo.RenderSystem = renderSystem;
	//	createTextureInfo.GameInstance = gameInstance;
	//	createTextureInfo.TextureName = "hydrant_Albedo";
	//	createTextureInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	texture = gameInstance->GetSystem<TextureSystem>("TextureSystem")->CreateTexture(createTextureInfo);
	//}

	{
		MaterialSystem::CreateMaterialInfo createMaterialInfo;
		createMaterialInfo.GameInstance = gameInstance;
		createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
		createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
		createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
		createMaterialInfo.MaterialName = "HydrantMat";
		material = material_system->CreateMaterial(createMaterialInfo);
	}
	
	{
		StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
		addStaticMeshInfo.MeshName = "hydrant";
		addStaticMeshInfo.Material = material;
		addStaticMeshInfo.GameInstance = gameInstance;
		addStaticMeshInfo.RenderSystem = renderSystem;
		addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
		const auto component = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
		staticMeshRenderer->SetPosition(component, GTSL::Vector3(0, 0, 250));
	}
	
	//{
	//	MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//	createMaterialInfo.TextureResourceManager = GetResourceManager<TextureResourceManager>("TextureResourceManager");
	//	createMaterialInfo.MaterialName = "TvMat";
	//	tvMat = material_system->CreateMaterial(createMaterialInfo);
	//}
	//
	//{
	//	StaticMeshRenderGroup::AddStaticMeshInfo addStaticMeshInfo;
	//	addStaticMeshInfo.MeshName = "TV";
	//	addStaticMeshInfo.Material = tvMat;
	//	addStaticMeshInfo.GameInstance = gameInstance;
	//	addStaticMeshInfo.RenderSystem = renderSystem;
	//	addStaticMeshInfo.StaticMeshResourceManager = GetResourceManager<StaticMeshResourceManager>("StaticMeshResourceManager");
	//	const auto component = staticMeshRenderer->AddStaticMesh(addStaticMeshInfo);
	//	staticMeshRenderer->SetPosition(component, GTSL::Vector3(200, 0, 250));
	//}//

	//{
	//	GTSL::GVector<GTSL::StaticString<64>, BE::SystemAllocatorReference> vec;
	//	vec.Initialize(6, systemAllocatorReference);
	//
	//	vec.EmplaceAt(0, "Hello");
	//	vec.EmplaceAt(1, " my name is");
	//
	//	for(uint32 i = 0; i < vec.GetGroupCount(); ++i)
	//	{
	//		BE_LOG_MESSAGE("Group ", i)
	//		
	//		for(uint32 j = 0; j < vec[i].ElementCount; ++j)
	//		{
	//			auto eg = vec[i].At(j);
	//			BE_LOG_MESSAGE("Item in Group, ", eg.First, " ", eg.Second)
	//		}
	//	}
	//
	//	vec.Clear();
	//	vec.EmplaceAt(3, " Sal.");
	//	
	//	for (uint32 i = 0; i < vec.GetGroupCount(); ++i)
	//	{
	//		BE_LOG_MESSAGE("Group ", i)
	//
	//		for (uint32 j = 0; j < vec[i].ElementCount; ++j)
	//		{
	//			auto eg = vec[i].At(j);
	//			BE_LOG_MESSAGE("Item in Group, ", eg.First, " ", eg.Second)
	//		}
	//	}
	//}
	
	//{
	//	MaterialSystem::CreateMaterialInfo createMaterialInfo;
	//	createMaterialInfo.GameInstance = gameInstance;
	//	createMaterialInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	createMaterialInfo.MaterialResourceManager = GetResourceManager<MaterialResourceManager>("MaterialResourceManager");
	//	createMaterialInfo.MaterialName = "TextMaterial";
	//	textMaterial = material_system->CreateMaterial(createMaterialInfo);
	//}

	//{
	//	TextSystem::AddTextInfo addTextInfo;
	//	addTextInfo.Position = { 450, -300 };
	//	addTextInfo.Text = "H: 97";
	//	addTextInfo.Material = textMaterial;
	//	addTextInfo.FontResourceManager = GetResourceManager<FontResourceManager>("FontResourceManager");
	//	addTextInfo.RenderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");
	//	addTextInfo.GameInstance = gameInstance;
	//	auto textComp = gameInstance->GetSystem<TextSystem>("TextSystem")->AddText(addTextInfo);
	//}

	GetResourceManager<FontResourceManager>("FontResourceManager")->GetFont(GTSL::StaticString<64>("FTLTLT"));
	
	//{
	//	LightsRenderGroup::RayTracingDirectionalLightCreateInfo info;
	//	info.Light.R() = 1.00f;
	//	info.Light.G() = 0.98f;
	//	info.Light.B() = 0.95f;
	//	info.Rotation.X = -0.785398f;
	//	info.Rotation.Y = 0;
	//	info.Rotation.Z = 0;
	//
	//	auto light = gameInstance->GetSystem<LightsRenderGroup>("LightsRenderGroup")->CreateRayTracingDirectionalLight(info);//
	//}
	
	//window.ShowMouse(false);
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
	posDelta += (data.Value - data.LastValue) * 1;

	auto rot = GTSL::Matrix4(GTSL::AxisAngle(0.f, 1.0f, 0.f, posDelta.X));
	rot *= GTSL::Matrix4(GTSL::AxisAngle(rot.GetXBasisVector(), -posDelta.Y));
	gameInstance->GetSystem<CameraSystem>("CameraSystem")->SetCameraRotation(camera, rot);
}

Game::~Game()
{
}
