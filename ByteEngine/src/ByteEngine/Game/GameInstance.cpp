#include "GameInstance.h"

#include "ByteEngine/Application/Application.h"

#include "System.h"

static BE::PersistentAllocatorReference persistent_allocator("Game Instance");

GameInstance::GameInstance() : worlds(4, &persistent_allocator), systems(8, &persistent_allocator)
{
}

GTSL::FlatHashMap<class System*>::ref GameInstance::AddSystem(System* system)
{
	//return systems.Emplace(&persistent_allocator, GTSL::Id64(system->GetName()), system);
	return 0;
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initialize_info;
	worlds[worldId]->InitializeWorld(initialize_info);
}
