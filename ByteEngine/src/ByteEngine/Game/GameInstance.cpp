#include "GameInstance.h"

#include "ByteEngine/Application/Application.h"

#include "System.h"

static BE::PersistentAllocatorReference persistent_allocator("Game Instance");

GameInstance::GameInstance() : worlds(4, &persistent_allocator), systems(8, &persistent_allocator)
{
}

GameInstance::~GameInstance()
{
	systems.Free(&persistent_allocator);
}

void GameInstance::OnUpdate()
{
	const GTSL::Ranger<World*> worlds_range = worlds;

	GTSL::ForEach(systems, [&](System*& system) { system->Process(worlds_range); });
}

GTSL::FlatHashMap<System*>::ref GameInstance::AddSystem(System* system)
{
	return systems.Emplace(&persistent_allocator, GTSL::Id64(system->GetName()), system);
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	worlds[worldId]->InitializeWorld(initialize_info);
}
