#include "Game.h"

#include "ByteEngine/Application/InputManager.h"

void Game::Init()
{
	GameApplication::Init();

	//GameInstance::CreateNewWorldInfo create_new_world_info;
	//create_new_world_info.Application = this;
	//menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())

	auto mo = [&](InputManager::ActionInputEvent a)
	{
		BE_BASIC_LOG_MESSAGE("Key: ", a.Value)
	};
	const GTSL::Array<GTSL::Id64, 1> a({ GTSL::Id64("W_Key") });
	inputManagerInstance->RegisterActionInputEvent("ClickTest", a, GTSL::Delegate<void(InputManager::ActionInputEvent)>::Create(mo));

	BE::PersistentAllocatorReference ss("Test");

	{
		GTSL::Vector<byte> test(16, &ss);
		
	}
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnNormalUpdate()
{
	GameApplication::OnNormalUpdate();
}
