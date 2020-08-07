#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

#include "ByteEngine/Game/CameraComponentCollection.h"

void Game::Initialize()
{
	GameApplication::Initialize();

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	gameInstance = GTSL::SmartPointer<GameInstance, BE::SystemAllocatorReference>::Create<SandboxGameInstance>(systemAllocatorReference);
	sandboxGameInstance = gameInstance;

	const GTSL::Array<GTSL::Id64, 2> a({ GTSL::Id64("MouseMove") });
	inputManagerInstance->Register2DInputEvent("Move", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create<Game, &Game::move>(this));

	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	/// <summary>
	/// Push bindings only for actual shader
	/// </summary>
	MaterialResourceManager::MaterialCreateInfo material_create_info;
	material_create_info.ShaderName = "BasicMaterial";
	GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3 };
	GTSL::Array<GTSL::Array<GAL::BindingType, 8>, 8> binding_sets(1);
	binding_sets[0].EmplaceBack(GAL::BindingType::UNIFORM_BUFFER_DYNAMIC);
	material_create_info.VertexFormat = format;
	material_create_info.ShaderTypes = GTSL::Array<GAL::ShaderType, 12>{ GAL::ShaderType::VERTEX_SHADER, GAL::ShaderType::FRAGMENT_SHADER };
	GTSL::Array<GTSL::Ranger<const GAL::BindingType>, 10> b_array;
	b_array.EmplaceBack(binding_sets[0]);
	material_create_info.BindingSets = b_array;
	static_cast<MaterialResourceManager*>(GetResourceManager("MaterialResourceManager"))->CreateMaterial(material_create_info);
	
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::PostInitialize()
{
	GameApplication::PostInitialize();

	camera = static_cast<CameraComponentCollection*>(gameInstance->GetComponentCollection("CameraComponentCollection"))->AddCamera(GTSL::Vector3(0, 0, -500));
	
	auto* collection = static_cast<RenderStaticMeshCollection*>(gameInstance->GetComponentCollection("RenderStaticMeshCollection"));
	auto component = collection->AddMesh();
	collection->SetMesh(component, "Box");
	collection->SetPosition(component, GTSL::Vector3(0, 0, 0));

	auto* static_mesh_renderer = static_cast<StaticMeshRenderGroup*>(gameInstance->GetSystem("StaticMeshRenderGroup"));
	StaticMeshRenderGroup::AddStaticMeshInfo add_static_mesh_info;
	add_static_mesh_info.RenderSystem = static_cast<RenderSystem*>(gameInstance->GetSystem("RenderSystem"));
	add_static_mesh_info.GameInstance = gameInstance;
	add_static_mesh_info.ComponentReference = component;
	add_static_mesh_info.RenderStaticMeshCollection = collection;
	add_static_mesh_info.StaticMeshResourceManager = static_cast<StaticMeshResourceManager*>(GetResourceManager("StaticMeshResourceManager"));
	add_static_mesh_info.MaterialName = "BasicMaterial";
	add_static_mesh_info.MaterialResourceManager = static_cast<MaterialResourceManager*>(GetResourceManager("MaterialResourceManager"));
	static_mesh_renderer->AddStaticMesh(add_static_mesh_info);
}

void Game::OnUpdate(const OnUpdateInfo& onUpdate)
{
	GameApplication::OnUpdate(onUpdate);
}

void Game::Shutdown()
{
	GameApplication::Shutdown();
}

void Game::move(InputManager::Vector2DInputEvent data)
{
	static_cast<CameraComponentCollection*>(gameInstance->GetComponentCollection("CameraComponentCollection"))->AddCameraRotation(camera, GTSL::Quaternion());
}

Game::~Game()
{
}
