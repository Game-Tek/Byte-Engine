#include "GameInstance.h"

#include "ByteEngine/Application/Application.h"

#include "System.h"

static BE::PersistentAllocatorReference persistent_allocator("Game Instance");

GameInstance::GameInstance() : worlds(4, &persistent_allocator), systems(8, persistent_allocator), componentCollections(64, persistent_allocator)
{
}

GameInstance::~GameInstance()
{
	systems.Free(persistent_allocator);
	componentCollections.Free(persistent_allocator);
}

void GameInstance::OnUpdate()
{
	const GTSL::Ranger<World*> worlds_range = worlds;

	ForEach(systems, [&](GTSL::Allocation<System>& system) { system->Process(worlds_range); });
	//GTSL::ForEach(componentCollections, [&](ComponentCollection*& collection) { collection->Process(worlds_range); });
}

void GameInstance::initWorld(const uint8 worldId)
{
	World::InitializeInfo initialize_info;
	initialize_info.GameInstance = this;
	worlds[worldId]->InitializeWorld(initialize_info);
}

void GameInstance::initCollection(ComponentCollection* collection)
{
	//collection->Initialize();
}
