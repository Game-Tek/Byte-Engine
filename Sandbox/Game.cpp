#include "Game.h"

#include "ByteEngine/Application/InputManager.h"

void Game::Init()
{
	GameApplication::Init();

	//GameInstance::CreateNewWorldInfo create_new_world_info;
	//create_new_world_info.Application = this;
	//menuWorld = sandboxGameInstance->CreateNewWorld<MenuWorld>(create_new_world_info);

	BE_LOG_SUCCESS("Inited Game!")

	auto mo = [&](InputManager::Vector2DInputEvent a)
	{
		BE_BASIC_LOG_MESSAGE("Mouse moved to: %f; %f", a.Value.X, a.Value.Y)
	};

	const GTSL::Array<GTSL::Id64, 1> a({ GTSL::Id64("MouseMove") });
	
	inputManagerInstance->Register2DInputEvent("MoveTest", a, GTSL::Delegate<void(InputManager::Vector2DInputEvent)>::Create(mo));
	//inputManagerInstance->Register2DInputEvent(GTSL::Ranger<const char>("MoveTest"), GTSL::Delegate<void(GTSL::Vector2, GTSL::Vector2)>::Create(mo));
	//show loading screen
	//load menu
	//show menu
	//start game
}

void Game::OnNormalUpdate()
{
	GameApplication::OnNormalUpdate();
}
