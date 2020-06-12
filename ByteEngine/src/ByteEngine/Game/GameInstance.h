#pragma once

#include "World.h"

#include <GTSL/FlatHashMap.h>

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

private:
	GTSL::Vector<World*> worlds;
	
	GTSL::FlatHashMap<class System*> systems;
	
	void initWorld(uint8 worldId);
};
