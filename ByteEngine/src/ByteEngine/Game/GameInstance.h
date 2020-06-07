#pragma once

#include "World.h"

class GameInstance
{
public:
	GameInstance();
	
	using WorldReference = uint8;
	
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
	}

private:
	GTSL::Vector<World*> worlds;

	void initWorld(uint8 worldId);
};
