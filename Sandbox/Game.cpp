#include "Game.h"

#include "ByteEngine/Application/InputManager.h"

void Game::Init()
{
	GameApplication::Init();

	//GameInstance::CreateNewWorldInfo create_new_world_info;
	//create_new_world_info.Application = this;
	//menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	BE_LOG_SUCCESS("Inited Game!")

	auto mo = [&](GTSL::Vector2 a, GTSL::Vector2 b)
	{
		BE_LOG_MESSAGE("Mouse moved to: %f; %f", a.X, a.Y)
	};

	inputManagerInstance->RegisterAxisAction(GTSL::Ranger<const char>("MoveTest"), GTSL::Delegate<void(GTSL::Vector2, GTSL::Vector2)>::Create(mo));
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnNormalUpdate()
{
	GameApplication::OnNormalUpdate();
}
