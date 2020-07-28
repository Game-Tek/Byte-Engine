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
	
	sandboxGameInstance->AddGoal("Frame");
	sandboxGameInstance->AddGoal("FrameEnd");

	auto renderer = sandboxGameInstance->AddSystem<RenderSystem>("RenderSystem");

	RenderSystem::InitializeRendererInfo initialize_renderer_info;
	initialize_renderer_info.Window = &window;
	renderer->InitializeRenderer(initialize_renderer_info);

	gameInstance->AddComponentCollection<RenderStaticMeshCollection>("RenderStaticMeshCollection");

	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	MaterialResourceManager::MaterialCreateInfo material_create_info;
	material_create_info.ShaderName = "BasicMaterial";
	GTSL::Array<uint8, 8> format{ (uint8)GAL::ShaderDataTypes::FLOAT3, (uint8)GAL::ShaderDataTypes::FLOAT3 };
	material_create_info.VertexFormat = format;
	static_cast<MaterialResourceManager*>(GetResourceManager("MaterialResourceManager"))->CreateMaterial(material_create_info);

	auto test_task = [](TaskInfo taskInfo, uint32 i)
	{
		std::cout << "Hey: " << i << std::endl;
	};

	GTSL::Array<TaskDependency, 2> dependencies{ {"RenderStaticMeshCollection", AccessType::READ} };
	
	gameInstance->AddDynamicTask("Test", GTSL::Delegate<void(TaskInfo, uint32)>::Create(test_task), dependencies, "Frame", "FrameEnd", 32u);
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
