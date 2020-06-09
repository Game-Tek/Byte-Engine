#include "Game.h"


#include "SandboxGameInstance.h"
#include "SandboxWorld.h"
#include "ByteEngine/Application/InputManager.h"

void Game::Init()
{
	GameApplication::Init();

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
	
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnNormalUpdate()
{
	GameApplication::OnNormalUpdate();
}

Game::~Game()
{
	delete sandboxGameInstance;
}
