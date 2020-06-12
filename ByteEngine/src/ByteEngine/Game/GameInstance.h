#pragma once

#include "World.h"

#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>

class GameInstance
{
public:
	GameInstance();
	virtual ~GameInstance();
	
	virtual void OnUpdate();
	
	using WorldReference = uint8;

	GTSL::FlatHashMap<class System*>::ref AddSystem(class System* system);
	
	struct CreateNewWorldInfo
	{
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.PushBack(new T());
		initWorld(index);
		return 0;
	}

	template<typename T>
	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		worlds[worldId]->DestroyWorld(destroy_info);
		worlds.Destroy(worldId);
		delete worlds[worldId];
	}

	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName) { return componentCollections.At(collectionName); }
	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName, uint64& reference)
	{
		reference = componentCollections.GetReference(collectionName);
		return componentCollections.At(collectionName);
	}
	class ComponentCollection* GetComponentCollection(const uint64 collectionReference) { return componentCollections[collectionReference]; }

	System* GetSystem(const GTSL::Id64 systemName) { return systems.At(systemName); }
	System* GetSystem(const GTSL::Id64 systemName, uint64& reference)
	{
		reference = systems.GetReference(systemName);
		return systems.At(systemName);
	}
	System* GetSystem(const uint64 systemReference) { return systems[systemReference]; }
private:
	GTSL::Vector<World*> worlds;
	
	GTSL::FlatHashMap<class System*> systems;
	GTSL::FlatHashMap<class ComponentCollection*> componentCollections;
	
	void initWorld(uint8 worldId);
};
