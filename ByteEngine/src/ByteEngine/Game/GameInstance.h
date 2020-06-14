#pragma once

#include "World.h"

#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>
#include <GTSL/Pair.h>

class GameInstance : public Object
{
public:
	GameInstance();
	virtual ~GameInstance();

	const char* GetName() const override { return "GameInstance"; }
	
	virtual void OnUpdate();
	
	using WorldReference = uint8;

	template<typename T>
	T* AddSystem(const GTSL::Id64 systemName)
	{
		return *systems.Emplace(GetPersistentAllocator(), systemName, GTSL::Allocation<System>::Create<T>(GetPersistentAllocator()));
	}

	template<typename T>
	T* AddComponentCollection(const GTSL::Id64 componentCollectionName)
	{
		GTSL::Allocation<ComponentCollection> pointer = *componentCollections.Emplace(GetPersistentAllocator(), componentCollectionName, GTSL::Allocation<ComponentCollection>::Create<T>(GetPersistentAllocator()));
		initCollection(pointer); return static_cast<T*>(pointer.Data);
	}

	void DestroyComponentCollection(const uint64 collectionReference)
	{
		GTSL::Delete(componentCollections[collectionReference], GetPersistentAllocator());
	}
	
	struct CreateNewWorldInfo
	{
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.PushBack(new T());
		initWorld(index); return index;
	}

	template<typename T>
	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		destroy_info.GameInstance = this;
		worlds[worldId]->DestroyWorld(destroy_info);
		delete worlds[worldId];
		worlds.Destroy(worldId);
	}

	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName) { return componentCollections.At(collectionName); }
	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName, uint64& reference)
	{
		reference = componentCollections.GetReference(collectionName); return componentCollections.At(collectionName);
	}
	class ComponentCollection* GetComponentCollection(const uint64 collectionReference) { return componentCollections[collectionReference]; }

	class System* GetSystem(const GTSL::Id64 systemName) { return systems.At(systemName); }
	class System* GetSystem(const GTSL::Id64 systemName, uint64& reference)
	{
		reference = systems.GetReference(systemName); return systems.At(systemName);
	}
	class System* GetSystem(const uint64 systemReference) { return systems[systemReference]; }
	
private:
	GTSL::Vector<World*> worlds;
	
	GTSL::FlatHashMap<GTSL::Allocation<System>> systems;
	GTSL::FlatHashMap<GTSL::Allocation<ComponentCollection>> componentCollections;
	
	void initWorld(uint8 worldId);
	void initCollection(ComponentCollection* collection);
};
