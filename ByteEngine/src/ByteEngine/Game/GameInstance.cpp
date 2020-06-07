#include "GameInstance.h"

#include "ByteEngine/Application/Application.h"

static BE::PersistentAllocatorReference persistent_allocator("Game");

GameInstance::GameInstance() : worlds(4, &persistent_allocator)
{
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initialize_info;
	worlds[worldId]->InitializeWorld(initialize_info);
}
