#include "Game.h"

#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Game/RenderSystem.h"

void Game::Initialize()
{
	GameApplication::Initialize();

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())
	
	sandboxGameInstance = new SandboxGameInstance();
	
	GameInstance::CreateNewWorldInfo create_new_world_info;
	menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	auto mo = [&](InputManager::ActionInputEvent a)
	{
		BE_BASIC_LOG_MESSAGE("Key: ", a.Value)
	};
	const GTSL::Array<GTSL::Id64, 2> a({ GTSL::Id64("W_Key"), GTSL::Id64("S_Key") });
	inputManagerInstance->RegisterActionInputEvent("ClickTest", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create(mo));

	auto renderer = sandboxGameInstance->AddSystem<RenderSystem>("RenderSystem");

	RenderSystem::InitializeRendererInfo initialize_renderer_info;
	initialize_renderer_info.Window = &window;
	renderer->InitializeRenderer(initialize_renderer_info);
	
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
}

Game::~Game()
{
	delete sandboxGameInstance;
}
