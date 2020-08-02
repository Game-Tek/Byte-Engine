#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"
#include <iostream>

void Game::Initialize()
{
	GameApplication::Initialize();

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	gameInstance = GTSL::SmartPointer<GameInstance, BE::SystemAllocatorReference>::Create<SandboxGameInstance>(systemAllocatorReference);
	sandboxGameInstance = gameInstance;

	auto mo = [&](InputManager::ActionInputEvent a)
	{
		//BE_BASIC_LOG_MESSAGE("Key: ", a.Value)
	};
	const GTSL::Array<GTSL::Id64, 2> a({ GTSL::Id64("RightHatButton"), GTSL::Id64("S_Key") });
	inputManagerInstance->RegisterActionInputEvent("ClickTest", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create(mo));

	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	/// <summary>
	/// Push bindings only for actual shader
	/// </summary>
	MaterialResourceManager::MaterialCreateInfo material_create_info;
	material_create_info.ShaderName = "BasicMaterial";
	GTSL::Array<GAL::ShaderDataType, 8> format{ GAL::ShaderDataType::FLOAT3, GAL::ShaderDataType::FLOAT3 };
	GTSL::Array<GTSL::Array<uint8, 8>, 8> binding_sets(1);
	binding_sets[0].EmplaceBack(static_cast<uint8>(GAL::BindingType::UNIFORM_BUFFER_DYNAMIC));
	material_create_info.VertexFormat = format;
	material_create_info.ShaderTypes = GTSL::Array<uint8, 12>{ (uint8)GAL::ShaderType::VERTEX_SHADER, (uint8)GAL::ShaderType::FRAGMENT_SHADER };
	GTSL::Array<GTSL::Ranger<const uint8>, 10> b_array;
	b_array.EmplaceBack(binding_sets[0]);
	material_create_info.BindingSets = b_array;
	static_cast<MaterialResourceManager*>(GetResourceManager("MaterialResourceManager"))->CreateMaterial(material_create_info);
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnUpdate(const OnUpdateInfo& onUpdate)
{
	GameApplication::OnUpdate(onUpdate);
}

void Game::Shutdown()
{
	GameApplication::Shutdown();
}

Game::~Game()
{
}
