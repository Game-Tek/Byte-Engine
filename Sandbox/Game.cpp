#include "Game.h"

#include "ByteEngine/Application/InputManager.h"

void Game::Init()
{
	GameApplication::Init();

	//GameInstance::CreateNewWorldInfo create_new_world_info;
	//create_new_world_info.Application = this;
	//menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	BE_LOG_SUCCESS("Inited Game: ", GetApplicationName())

	auto mo = [&](InputManager::CharacterInputEvent a)
	{
		BE_BASIC_LOG_MESSAGE("Character: ", a.Value)
	};
	const GTSL::Array<GTSL::Id64, 1> a({ GTSL::Id64("Keyboard") });
	inputManagerInstance->RegisterCharacterInputEvent("ClickTest", a, GTSL::Delegate<void(InputManager::CharacterInputEvent)>::Create(mo));
	
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnNormalUpdate()
{
	GameApplication::OnNormalUpdate();
}
