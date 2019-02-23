#include "GameInstance.h"

#include "World.h"

GameInstance::GameInstance() : ActiveWorld(new World())
{
	
}

GameInstance::~GameInstance()
{
}

void GameInstance::OnUpdate()
{
	ActiveWorld->OnUpdate();
}
